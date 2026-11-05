//! 窗口尺寸模式：正常（320×160）/ 缩小（320×24，仅保留标题栏）。
//!
//! - 手动：标题栏 `─`（缩小）/ `⤢`（恢复）按钮切换 —— `toggle_window_mode` 命令
//! - 自动循环：正常模式空闲 3 分钟 → 缩小；缩小持续 20 分钟 → 恢复；如此往复
//! - 重置：新建页签、刷新活动页签、手动恢复 —— 都会回到正常大小并重新起算 3 分钟
//!
//! 状态（模式 + 当前模式的进入时刻 `since`）由 `Mutex` 保护，`Condvar` 驱动定时线程：
//! 每次模式切换都更新 `since` 并 notify，线程按 `since + 该模式超时` 重新算 deadline。
use std::sync::{Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, LogicalSize, PhysicalPosition, WebviewWindow};

use crate::tabs::{self, WINDOW_HEIGHT, WINDOW_WIDTH};

/// 缩小模式下仅保留的标题栏高度（与页签栏一致）
const TITLE_BAR_HEIGHT: f64 = 24.0;
/// 正常模式空闲多久后自动缩小
const IDLE_TIMEOUT: Duration = Duration::from_secs(3 * 60);
/// 缩小模式持续多久后自动恢复
const COMPACT_TIMEOUT: Duration = Duration::from_secs(20 * 60);

/// 实际生效的超时：默认 `fallback`，环境变量 `name`（秒）可覆盖（调试/测试用）
fn env_timeout(name: &str, fallback: Duration) -> Duration {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|&secs| secs > 0)
        .map(Duration::from_secs)
        .unwrap_or(fallback)
}

/// 正常 → 缩小的空闲超时（默认 180s，`FULLME_IDLE_SECS` 可覆盖）
fn idle_timeout() -> Duration {
    env_timeout("FULLME_IDLE_SECS", IDLE_TIMEOUT)
}

/// 缩小 → 正常的持续超时（默认 1200s，`FULLME_COMPACT_SECS` 可覆盖）
fn compact_timeout() -> Duration {
    env_timeout("FULLME_COMPACT_SECS", COMPACT_TIMEOUT)
}
/// 发给页签栏前端的模式事件名
pub const MODE_EVENT: &str = "window-mode-updated";

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Compact,
}

/// 发给前端的模式快照
#[derive(Clone, Copy, serde::Serialize)]
pub struct ModeSnapshot {
    pub compact: bool,
}

struct State {
    mode: Mode,
    /// 当前模式的进入时刻：正常 = 空闲起点，缩小 = 缩小起点；每次切换刷新
    since: Instant,
}

struct Timer {
    state: Mutex<State>,
    cv: Condvar,
}

fn timer() -> &'static Timer {
    static TIMER: OnceLock<Timer> = OnceLock::new();
    TIMER.get_or_init(|| Timer {
        state: Mutex::new(State {
            mode: Mode::Normal,
            since: Instant::now(),
        }),
        cv: Condvar::new(),
    })
}

fn lock_state() -> MutexGuard<'static, State> {
    timer().state.lock().unwrap_or_else(|e| e.into_inner())
}

fn mode_name(mode: Mode) -> &'static str {
    match mode {
        Mode::Normal => "正常",
        Mode::Compact => "缩小",
    }
}

/// 把窗口贴到当前显示器工作区右下角（避开菜单栏 / Dock）。
///
/// 高度按模式取（正常 160 / 缩小 24），所以缩小/恢复后重新调用即可让窗口
/// 重新贴住右下角 —— 若只改尺寸不改位置，顶边不动、底边会上移而悬空。
pub fn anchor_bottom_right(window: &WebviewWindow, mode: Mode) -> Result<(), String> {
    let height = match mode {
        Mode::Normal => WINDOW_HEIGHT,
        Mode::Compact => TITLE_BAR_HEIGHT,
    };
    // 优先取窗口当前所在显示器（拖到副屏后仍贴副屏右下角），失败退回主屏
    let monitor = window
        .current_monitor()
        .map_err(|e| e.to_string())?
        .or_else(|| window.primary_monitor().ok().flatten())
        .ok_or_else(|| "无法识别显示器".to_string())?;
    let work_area = monitor.work_area();
    let scale = monitor.scale_factor();
    let width_px = (WINDOW_WIDTH * scale).round() as i32;
    let height_px = (height * scale).round() as i32;
    let x = work_area.position.x + work_area.size.width as i32 - width_px;
    let y = work_area.position.y + work_area.size.height as i32 - height_px;
    window
        .set_position(PhysicalPosition::new(x, y))
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// 调整窗口尺寸（Normal 时顺带校准各页签子 webview 的布局）
fn apply(app: &AppHandle, mode: Mode) {
    let Some(window) = tabs::main_window(app) else {
        flog!(
            "fullme: [mode] 找不到主窗口，无法切换为{}模式",
            mode_name(mode)
        );
        return;
    };
    let height = match mode {
        Mode::Normal => WINDOW_HEIGHT,
        Mode::Compact => TITLE_BAR_HEIGHT,
    };
    if let Err(e) = window.set_size(LogicalSize::new(WINDOW_WIDTH, height)) {
        flog!("fullme: [mode] 设置窗口尺寸失败: {e}");
        return;
    }
    // 注意：set_size 异步生效，此处只记录目标尺寸（物理实测需等下一拍）
    flog!(
        "fullme: [mode] 窗口 -> {}模式 ({}x{})",
        mode_name(mode),
        WINDOW_WIDTH,
        height
    );
    // 尺寸变了，重新贴右下角（否则缩小后底边上移、窗口悬空）
    if let Err(e) = anchor_bottom_right(&window, mode) {
        flog!("fullme: [mode] 贴右下角失败: {e}");
    }
    if mode == Mode::Normal {
        tabs::reposition_tabs();
    }
}

/// 把模式推给页签栏前端（按钮图标 / 提示随之切换）
fn emit(app: &AppHandle, mode: Mode) {
    if let Some(window) = tabs::main_window(app) {
        let _ = window.emit(
            MODE_EVENT,
            ModeSnapshot {
                compact: mode == Mode::Compact,
            },
        );
    }
}

/// 切换到指定模式：刷新 `since`（正常 = 重新起算空闲，缩小 = 重新起算 20 分钟）
pub fn set_mode(app: &AppHandle, mode: Mode) {
    {
        let t = timer();
        let mut st = lock_state();
        st.mode = mode;
        st.since = Instant::now();
        t.cv.notify_all();
    }
    apply(app, mode);
    emit(app, mode);
    flog!("fullme: [mode] 切换为{}模式", mode_name(mode));
}

pub fn current_mode() -> Mode {
    lock_state().mode
}

/// 恢复正常大小并重新起算空闲计时（新建页签 / 刷新 / 手动恢复时调用）
pub fn restore(app: &AppHandle) {
    set_mode(app, Mode::Normal);
}

/// 标题栏按钮：正常 ↔ 缩小
#[tauri::command]
pub fn toggle_window_mode(app: AppHandle) {
    let target = match current_mode() {
        Mode::Normal => Mode::Compact,
        Mode::Compact => Mode::Normal,
    };
    set_mode(&app, target);
}

/// 模式循环监听线程：正常空闲满 3 分钟 → 缩小；缩小满 20 分钟 → 恢复；如此往复
///
/// deadline = `since + 当前模式超时`；期间任何 `set_mode`（新页签/刷新/手动）
/// 都会刷新 `since` 并 notify，线程醒来按新 deadline 重新等待。
pub fn spawn_idle_watch(app: AppHandle) {
    std::thread::spawn(move || loop {
        let t = timer();
        let mut st = lock_state();
        let timeout = match st.mode {
            Mode::Normal => idle_timeout(),
            Mode::Compact => compact_timeout(),
        };
        let deadline = st.since + timeout;
        let now = Instant::now();
        if deadline > now {
            // 未到期：睡到 deadline；被重置会提前醒来重算
            drop(t.cv.wait_timeout(st, deadline - now));
        } else {
            // 到期：切到另一模式。先释放锁再动窗口/发事件，避免锁内做平台调用
            let prev = st.mode;
            let next = match prev {
                Mode::Normal => Mode::Compact,
                Mode::Compact => Mode::Normal,
            };
            st.mode = next;
            st.since = Instant::now();
            drop(st);
            apply(&app, next);
            emit(&app, next);
            flog!(
                "fullme: [mode] {}模式持续 {}s，自动切为{}模式",
                mode_name(prev),
                timeout.as_secs(),
                mode_name(next)
            );
        }
    });
}
