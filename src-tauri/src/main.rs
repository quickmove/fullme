//! fullme —— 右上角 320x160 多页签小浏览器（内容区 320x136）
//!
//! - `fullme [url]...`：首次启动打开窗口并创建页签；已在运行时转发给现有实例追加页签
//! - 页签池默认 3 个，FIFO：池满时先关闭最旧页签
//! - 平台：macOS / Windows（Unix 与 Windows 分支以 cfg 分隔，见 spawn_daemon）
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[macro_use]
mod log;
mod tabs;

use std::io::Read;
#[cfg(unix)]
use std::os::unix::io::AsRawFd;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use tauri::{AppHandle, Manager, PhysicalPosition};

use tabs::{add_tab, WINDOW_WIDTH};

const USAGE: &str = "\
fullme —— 右上角 320x160 多页签小浏览器

用法:
  fullme [url]...   打开窗口并创建页签；窗口已存在时追加为新页签
                    页签池默认 3 个，超出时 FIFO 关闭最旧页签

选项:
  -h, --help        显示本帮助
  -V, --version     显示版本
";

/// 取出位置参数（跳过 `-` 开头的选项）
fn positional_urls<I: IntoIterator<Item = String>>(args: I) -> Vec<String> {
    args.into_iter().filter(|a| !a.starts_with('-')).collect()
}

// ---------------------------------------------------------------------------
// 启动器 / 守护子进程：
// `fullme [url]` 作为启动器运行 —— spawn 一个全新的自己（exec，不经 fork 污染）
// 承载 GUI，等子进程就绪（写出 1 字节）或退出（转发给现有实例）后立即返回。
// 行为与 `open [url]` 一致：同步等待命令结束的调用方（终端 shell、
// TinTin++ `#system` 等）不会被常驻 GUI 进程阻塞。
//
// 注意：不能在本进程内 fork 后继续跑 Tauri —— 已加载 AppKit/WebKit 的进程
// fork 后主线程 CFRunLoop 派发会失效（实测 run_on_main_thread / add_child
// 永久卡死），必须 exec 出全新进程。
// ---------------------------------------------------------------------------

/// 启动锁（仅 Unix）：串行化「检查无实例 → 子进程绑定单实例 socket」的窗口期。
/// Windows 无需此锁：单实例插件用原子的 `CreateMutexW` + 事件窗口轮询，天然无竞态。
#[cfg(unix)]
struct LaunchLock(std::fs::File);

#[cfg(unix)]
impl LaunchLock {
    fn acquire() -> std::io::Result<Self> {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .open("/tmp/com_fullme_app_launch.lock")?;
        // 阻塞获取；持有进程退出时由内核自动释放
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(Self(file))
    }
}

#[cfg(unix)]
impl Drop for LaunchLock {
    fn drop(&mut self) {
        unsafe { libc::flock(self.0.as_raw_fd(), libc::LOCK_UN) };
    }
}

/// 启动器：spawn 守护子进程，等它就绪后返回
fn spawn_daemon() -> std::io::Result<()> {
    #[cfg(unix)]
    let _lock = LaunchLock::acquire()?;

    let mut cmd = Command::new(std::env::current_exe()?);
    cmd.args(std::env::args().skip(1))
        .env("FULLME_DAEMON", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped()) // 就绪握手管道
        .stderr(Stdio::null()); // 丢弃系统噪音（macOS view-bridge / IMK 等）

    // Unix：脱离终端 —— Ctrl+C、关闭终端都不影响守护进程
    #[cfg(unix)]
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    // Windows：不分配控制台（避免黑框）+ 独立进程组（父终端信号不波及）
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }

    let mut child = cmd.spawn()?;
    wait_ready(&mut child);
    Ok(())
}

/// 等子进程写出就绪字节（首实例：单实例已接管、窗口已创建）
/// 或退出（已转发给现有实例）。
/// 跨平台实现：reader 线程 + 带超时的 channel 等待（Windows 管道无超时 API）。
fn wait_ready(child: &mut Child) {
    let Some(stdout) = child.stdout.take() else {
        return;
    };
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut stdout = stdout;
        let mut buf = [0u8; 1];
        let _ = tx.send(stdout.read(&mut buf));
    });
    match rx.recv_timeout(Duration::from_secs(10)) {
        Ok(Ok(0)) => {
            let _ = child.wait(); // 子进程已退出（转发场景），回收
        }
        Ok(Ok(_)) => {} // 就绪：子进程继续常驻
        Ok(Err(e)) => eprintln!("fullme: 读取就绪信号失败: {e}"),
        Err(_) => eprintln!("fullme: 等待子进程就绪超时"),
    }
}

/// 守护子进程：向启动器写就绪字节
fn signal_ready() {
    if std::env::var_os("FULLME_DAEMON").is_none() {
        return;
    }
    use std::io::Write;
    let mut out = std::io::stdout();
    let _ = out.write_all(b"1");
    let _ = out.flush();
    // Unix：把 stdout 指向 /dev/null（启动器退出后管道已无人读取）
    #[cfg(unix)]
    if let Ok(null) = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/null")
    {
        unsafe { libc::dup2(null.as_raw_fd(), 1); }
    }
    // Windows：无需摘除 —— 应用启动后不再写 stdout（系统噪音走 stderr→null）
}

/// 显示主窗口但不抢焦点（保持调用方如 TinTin++ 在前台）
fn reveal_window(app: &AppHandle) {
    if let Some(window) = tabs::main_window(app) {
        let _ = window.unminimize();
        let _ = window.show();
        // 注意：不调用 set_focus() —— tao 的 set_focus 会 activateIgnoringOtherApps
    } else {
        flog!(
            "fullme: [callback] 找不到主窗口, windows={:?}",
            {
                use tauri::Manager;
                app.windows().keys().collect::<Vec<_>>()
            }
        );
    }
}

/// 定位到主显示器工作区右上角（避开菜单栏 / Dock），并置顶
fn position_top_right(window: &tauri::WebviewWindow) -> Result<(), String> {
    let monitor = window
        .primary_monitor()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "无法识别主显示器".to_string())?;
    let work_area = monitor.work_area();
    let scale = monitor.scale_factor();
    let width = (WINDOW_WIDTH * scale).round() as i32;
    let x = work_area.position.x + work_area.size.width as i32 - width;
    let y = work_area.position.y;
    window
        .set_position(PhysicalPosition::new(x, y))
        .map_err(|e| e.to_string())?;
    window.set_always_on_top(true).map_err(|e| e.to_string())?;
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "-h" || a == "--help") {
        print!("{USAGE}");
        return;
    }
    if args.iter().any(|a| a == "-V" || a == "--version") {
        println!("fullme {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    let is_daemon = std::env::var_os("FULLME_DAEMON").is_some();
    let is_foreground = std::env::var_os("FULLME_FOREGROUND").is_some();

    if !is_daemon && !is_foreground {
        // 启动器模式：spawn 守护子进程承载 GUI，本进程立即返回（行为同 `open`）。
        // FULLME_FOREGROUND=1 可跳过（调试：本进程直接跑 GUI，调用方会阻塞）
        if let Err(e) = spawn_daemon() {
            eprintln!("fullme: 启动失败: {e}");
            std::process::exit(1);
        }
        return;
    }

    // 守护子进程的 stderr 已指向 /dev/null（或 Windows null），panic 信息改写日志文件
    std::panic::set_hook(Box::new(|info| {
        flog!("panic: {info}");
    }));

    let started = std::time::Instant::now();
    let builder = tauri::Builder::default();
    // 打开时不激活本应用：窗口照常出现，但前台/键盘焦点保持在调用方（如 TinTin++）。
    // 仅 macOS 需要显式关闭；Windows 后台进程本就无法抢占前台
    #[cfg(target_os = "macos")]
    let builder = builder.activate_ignoring_other_apps(false);
    builder
        // 单实例：第二次执行 fullme 时，参数转发给已运行实例并退出本次进程
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            flog!("fullme: [callback] 收到第二个实例参数: {:?}", argv);
            reveal_window(app);
            for url in positional_urls(argv.into_iter().skip(1)) {
                if let Err(e) = add_tab(app, &url) {
                    flog!("fullme: 打开 {url} 失败: {e}");
                }
            }
        }))
        .invoke_handler(tauri::generate_handler![
            tabs::get_tabs,
            tabs::switch_tab,
            tabs::close_tab,
            tabs::reload_active,
            tabs::quit_app,
        ])
        .setup(move |app| {
            flog!("fullme: [t+{:?}] setup 开始", started.elapsed());

            // 单实例已接管（Unix socket / Windows 命名互斥锁），通知启动器可返回
            signal_ready();

            let window = app
                .get_webview_window("main")
                .ok_or_else(|| "找不到主窗口".to_string())?;
            position_top_right(&window)?;
            tabs::set_main_window(&window);
            flog!("fullme: [t+{:?}] 窗口就绪", started.elapsed());

            let urls = positional_urls(std::env::args().skip(1));
            let mut opened = 0usize;
            for url in urls {
                match add_tab(app.handle(), &url) {
                    Ok(()) => opened += 1,
                    Err(e) => flog!("fullme: 打开 {url} 失败: {e}"),
                }
            }
            if opened == 0 {
                if let Err(e) = add_tab(app.handle(), "about:blank") {
                    flog!("fullme: {e}");
                }
            }
            flog!("fullme: [t+{:?}] 页签就绪", started.elapsed());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running fullme");
}
