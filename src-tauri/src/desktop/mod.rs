//! 窗口、托盘、快捷键与系统通知。

pub mod memory;
pub mod notifications;
pub mod shortcuts;
pub mod tray;
pub mod vibrancy;
pub mod windows;

use tauri::{AppHandle, Manager};

pub const MAIN_WINDOW: &str = "main";

/// 单实例：再次启动时唤起已有主窗口
pub fn focus_main(app: &AppHandle) {
    if let Some(win) = app.get_webview_window(MAIN_WINDOW) {
        let _ = win.unminimize();
        let _ = memory::show(&win);
        let _ = win.set_focus();
    }
}
