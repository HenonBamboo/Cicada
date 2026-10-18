//! 毛玻璃主题：Windows 11 使用 Mica；不支持的系统由前端降级为浅色主题

use tauri::WebviewWindow;

use crate::config::ThemeChoice;

/// 探测系统是否支持 Mica，并按当前主题设置窗口。返回是否支持。
pub fn probe(window: &WebviewWindow, theme: ThemeChoice) -> bool {
    let supported = enable(window);
    if supported && theme != ThemeChoice::Glass {
        disable(window);
    }
    supported
}

/// 切换主题时调用
pub fn apply(window: &WebviewWindow, theme: ThemeChoice, supported: bool) {
    // 辅助窗口使用原型中的半透明背景，不能叠加主窗口的 Mica 底板。
    if !supported || window.label() != super::MAIN_WINDOW {
        return;
    }
    if theme == ThemeChoice::Glass {
        enable(window);
    } else {
        disable(window);
    }
}

#[cfg(target_os = "windows")]
fn enable(window: &WebviewWindow) -> bool {
    // 毛玻璃主题是浅色调
    window_vibrancy::apply_mica(window, Some(false)).is_ok()
}

#[cfg(target_os = "windows")]
fn disable(window: &WebviewWindow) {
    let _ = window_vibrancy::clear_mica(window);
}

#[cfg(not(target_os = "windows"))]
fn enable(_window: &WebviewWindow) -> bool {
    false
}

#[cfg(not(target_os = "windows"))]
fn disable(_window: &WebviewWindow) {}
