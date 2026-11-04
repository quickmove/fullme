//! 页签池：默认容量 3，FIFO —— 池满时关闭最旧页签，新页签入池并置为活动页签。
//!
//! 窗口结构：
//! - 主 webview（窗口自带）渲染顶部 24px 页签栏
//! - 每个页签是一个 `add_child` 出来的子 webview，占据 y=24 以下 240x96 的内容区，
//!   通过 show/hide 切换，互不重叠
use std::sync::{Mutex, OnceLock};

use serde::Serialize;
use tauri::webview::{PageLoadEvent, WebviewBuilder};
use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Webview, WebviewUrl, WebviewWindow,
};
use url::Url;

/// 页签池默认容量
pub const POOL_SIZE: usize = 3;
/// 窗口逻辑尺寸
pub const WINDOW_WIDTH: f64 = 320.0;
pub const WINDOW_HEIGHT: f64 = 160.0;
/// 页签栏高度（主 webview 内容）
const TAB_BAR_HEIGHT: f64 = 24.0;
/// 子 webview（内容区）相对窗口的纵向偏移与高度
const CONTENT_TOP: f64 = TAB_BAR_HEIGHT;
const CONTENT_HEIGHT: f64 = WINDOW_HEIGHT - TAB_BAR_HEIGHT;

/// 发给前端的页签信息
#[derive(Clone, Serialize)]
pub struct TabInfo {
    pub id: u64,
    pub title: String,
    pub url: String,
}

/// 发给前端的完整快照
#[derive(Clone, Serialize)]
pub struct TabsSnapshot {
    pub tabs: Vec<TabInfo>,
    pub active: Option<u64>,
}

struct Tab {
    id: u64,
    title: String,
    url: String,
    webview: Webview,
}

pub struct TabManager {
    /// 入池顺序（最旧在前），FIFO 依据
    tabs: Vec<Tab>,
    active: Option<u64>,
    next_id: u64,
}

impl Default for TabManager {
    fn default() -> Self {
        Self {
            tabs: Vec::new(),
            active: None,
            next_id: 0,
        }
    }
}

impl TabManager {
    fn snapshot(&self) -> TabsSnapshot {
        TabsSnapshot {
            tabs: self
                .tabs
                .iter()
                .map(|t| TabInfo {
                    id: t.id,
                    title: t.title.clone(),
                    url: t.url.clone(),
                })
                .collect(),
            active: self.active,
        }
    }

    fn index_of(&self, id: u64) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }
}

type Guard<'a> = std::sync::MutexGuard<'a, TabManager>;

/// 页签池全局单例
static TAB_MANAGER: OnceLock<Mutex<TabManager>> = OnceLock::new();

/// 主窗口全局引用：单实例回调等非 setup 上下文无需按 label 查找
static MAIN_WINDOW: OnceLock<WebviewWindow> = OnceLock::new();

pub fn set_main_window(window: &WebviewWindow) {
    let _ = MAIN_WINDOW.set(window.clone());
}

pub fn main_window(app: &AppHandle) -> Option<WebviewWindow> {
    MAIN_WINDOW
        .get()
        .cloned()
        .or_else(|| app.get_webview_window("main"))
}

fn lock_app() -> Guard<'static> {
    TAB_MANAGER
        .get_or_init(|| Mutex::new(TabManager::default()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// 把当前页签状态推给页签栏前端
pub fn emit_tabs_updated(app: &AppHandle) {
    let snapshot = lock_app().snapshot();
    if let Some(window) = main_window(app) {
        let _ = window.emit("tabs-updated", snapshot);
    }
}

/// 补全 URL：无 scheme 时默认 https://
pub fn normalize_url(input: &str) -> String {
    let s = input.trim();
    if s.is_empty() {
        return "about:blank".to_string();
    }
    const KNOWN_SCHEMES: [&str; 6] = ["about:", "data:", "file:", "http:", "https:", "mailto:"];
    if s.contains("://") || KNOWN_SCHEMES.iter().any(|scheme| s.starts_with(scheme)) {
        s.to_string()
    } else {
        format!("https://{s}")
    }
}

fn title_from_url(url: &Url) -> String {
    if url.scheme() == "about" {
        return "blank".to_string();
    }
    let host = url.host_str().unwrap_or(url.as_str());
    host.strip_prefix("www.").unwrap_or(host).to_string()
}

fn title_from_url_str(url: &str) -> String {
    Url::parse(url)
        .map(|u| title_from_url(&u))
        .unwrap_or_else(|_| url.to_string())
}

/// 页面加载 / 标题变化时更新页签元信息，变化则通知前端
fn update_tab_meta(app: &AppHandle, label: &str, title: Option<&str>, url: Option<&str>) {
    let changed = {
        let mut guard = lock_app();
        let mgr = &mut *guard;
        match mgr.tabs.iter_mut().find(|t| t.webview.label() == label) {
            Some(tab) => {
                let mut changed = false;
                if let Some(title) = title {
                    let title = title.trim();
                    if !title.is_empty() && tab.title != title {
                        tab.title = title.to_string();
                        changed = true;
                    }
                }
                if let Some(url) = url {
                    if url != tab.url {
                        tab.url = url.to_string();
                        changed = true;
                    }
                }
                changed
            }
            None => false,
        }
    };
    if changed {
        emit_tabs_updated(app);
    }
}

/// 打开一个新页签；池满（默认 3）时 FIFO 关闭最旧页签
pub fn add_tab(app: &AppHandle, raw_url: &str) -> Result<(), String> {
    let url = normalize_url(raw_url);
    let parsed = Url::parse(&url).map_err(|e| format!("无效 URL {url}: {e}"))?;

    let window = main_window(app).ok_or_else(|| "找不到主窗口".to_string())?;
    let host_window = window.as_ref().window();

    let mut guard = lock_app();
    let mgr = &mut *guard;

    // FIFO：池满时先关最旧的
    while mgr.tabs.len() >= POOL_SIZE {
        let oldest = mgr.tabs.remove(0);
        let _ = oldest.webview.close();
        if mgr.active == Some(oldest.id) {
            mgr.active = None;
        }
        flog!(
            "fullme: 池已满，FIFO 关闭最旧页签 #{} ({})",
            oldest.id,
            oldest.url
        );
    }

    let id = mgr.next_id;
    mgr.next_id += 1;
    let label = format!("tab-{id}");
    let initial_title = title_from_url(&parsed);

    let title_app = app.clone();
    let load_app = app.clone();
    let builder = WebviewBuilder::new(label, WebviewUrl::External(parsed))
        .on_document_title_changed(move |webview, title| {
            update_tab_meta(&title_app, webview.label(), Some(title.as_str()), None);
        })
        .on_page_load(move |webview, payload| {
            let url = payload.url().to_string();
            let title = matches!(payload.event(), PageLoadEvent::Started)
                .then(|| title_from_url_str(&url));
            update_tab_meta(&load_app, webview.label(), title.as_deref(), Some(&url));
        });

    let webview = host_window
        .add_child(
            builder,
            LogicalPosition::new(0.0, CONTENT_TOP),
            LogicalSize::new(WINDOW_WIDTH, CONTENT_HEIGHT),
        )
        .map_err(|e| format!("创建页签失败: {e}"))?;

    // 切换到新页签：隐藏其余，显示当前
    for tab in mgr.tabs.iter() {
        let _ = tab.webview.hide();
    }
    let _ = webview.show();

    mgr.tabs.push(Tab {
        id,
        title: initial_title,
        url: url.clone(),
        webview,
    });
    mgr.active = Some(id);
    drop(guard);

    flog!("fullme: 打开 {url} -> 页签 #{id} (共 {} 个)", {
        lock_app().tabs.len()
    });
    emit_tabs_updated(app);
    Ok(())
}

/// 切换活动页签
#[tauri::command]
pub fn switch_tab(app: AppHandle, id: u64) -> Result<(), String> {
    let mut guard = lock_app();
    let mgr = &mut *guard;
    let idx = mgr
        .index_of(id)
        .ok_or_else(|| format!("页签 #{id} 不存在"))?;
    if mgr.active == Some(id) {
        return Ok(());
    }
    if let Some(current) = mgr.active.and_then(|a| mgr.index_of(a)) {
        let _ = mgr.tabs[current].webview.hide();
    }
    let _ = mgr.tabs[idx].webview.show();
    mgr.active = Some(id);
    drop(guard);
    emit_tabs_updated(&app);
    Ok(())
}

/// 关闭指定页签；关闭的是活动页签时激活相邻页签
#[tauri::command]
pub fn close_tab(app: AppHandle, id: u64) -> Result<(), String> {
    let mut guard = lock_app();
    let mgr = &mut *guard;
    let idx = mgr
        .index_of(id)
        .ok_or_else(|| format!("页签 #{id} 不存在"))?;
    let removed = mgr.tabs.remove(idx);
    let _ = removed.webview.close();
    flog!("fullme: 关闭页签 #{} ({})", removed.id, removed.url);

    if mgr.active == Some(id) {
        if mgr.tabs.is_empty() {
            mgr.active = None;
        } else {
            let next = idx.min(mgr.tabs.len() - 1);
            let next_id = mgr.tabs[next].id;
            let _ = mgr.tabs[next].webview.show();
            mgr.active = Some(next_id);
        }
    }
    drop(guard);
    emit_tabs_updated(&app);
    Ok(())
}

/// 刷新活动页签
#[tauri::command]
pub fn reload_active() -> Result<(), String> {
    let guard = lock_app();
    let id = guard
        .active
        .ok_or_else(|| "没有活动页签".to_string())?;
    let tab = guard
        .tabs
        .iter()
        .find(|t| t.id == id)
        .ok_or_else(|| "没有活动页签".to_string())?;
    tab.webview.reload().map_err(|e| format!("刷新失败: {e}"))
}

/// 退出应用
#[tauri::command]
pub fn quit_app(app: AppHandle) {
    app.exit(0);
}

/// 供前端初始化 / 校准页签栏
#[tauri::command]
pub fn get_tabs() -> TabsSnapshot {
    lock_app().snapshot()
}
