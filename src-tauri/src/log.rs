//! 极简文件日志：
//! - macOS/Linux：~/Library/Logs/fullme.log
//! - Windows：%LOCALAPPDATA%\fullme.log
//!
//! 守护子进程的 stderr 指向 null（macOS 系统噪音如 view-bridge/IMK 会刷屏），
//! 应用自身的日志通过本模块直接写入日志文件。
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

static SINK: OnceLock<Option<Mutex<std::fs::File>>> = OnceLock::new();

fn sink() -> Option<&'static Mutex<std::fs::File>> {
    SINK.get_or_init(|| {
        #[cfg(windows)]
        let path = PathBuf::from(std::env::var("LOCALAPPDATA").ok()?).join("fullme.log");
        #[cfg(not(windows))]
        let path = {
            let home = std::env::var("HOME").ok()?;
            PathBuf::from(home).join("Library/Logs").join("fullme.log")
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).ok()?;
        }
        let file = OpenOptions::new().create(true).append(true).open(path).ok()?;
        Some(Mutex::new(file))
    })
    .as_ref()
}

/// 写入一行日志
pub fn line(msg: &str) {
    if let Some(sink) = sink() {
        if let Ok(mut file) = sink.lock() {
            let _ = writeln!(file, "{msg}");
            let _ = file.flush();
        }
    }
}

/// `flog!("打开 {url} -> 页签 #{id}")` —— 与 eprintln 用法一致
macro_rules! flog {
    ($($arg:tt)*) => {
        crate::log::line(&format!($($arg)*))
    };
}
