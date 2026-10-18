//! 隐藏、最小化或处于后台的窗口请求 WebView2 降低内存占用，不暂停脚本或销毁页面。
use tauri::{WebviewWindow, WindowEvent};

pub fn show(win: &WebviewWindow) -> tauri::Result<()> {
    // 先请求恢复 Normal，减少低内存页面重新显示时的换页等待。
    set_target(win, true);
    win.show()?;
    refresh(win);
    Ok(())
}

pub fn hide(win: &WebviewWindow) -> tauri::Result<()> {
    win.hide()?;
    refresh(win);
    Ok(())
}

pub fn observe(win: &WebviewWindow) {
    let window = win.clone();
    win.on_window_event(move |event| {
        // 包括前端原生 minimize() 和任务栏还原，不增加定时轮询。
        if matches!(event, WindowEvent::Resized(_) | WindowEvent::Focused(_)) {
            refresh(&window);
        }
    });
}

fn refresh(win: &WebviewWindow) {
    #[cfg(windows)]
    {
        let window = win.clone();
        if let Err(error) = win.run_on_main_thread(move || {
            // 到主线程后再读实际状态，避免迟到的隐藏请求覆盖已显示状态。
            let (Ok(visible), Ok(minimized), Ok(focused)) = (
                window.is_visible(),
                window.is_minimized(),
                window.is_focused(),
            ) else {
                return;
            };
            // WebView2 Low 不暂停脚本；窗口不可见、最小化或处于后台时均可使用。
            set_target_on_main_thread(&window, visible && !minimized && focused);
        }) {
            eprintln!("Window memory dispatch: {error}");
        }
    }
}

fn set_target(win: &WebviewWindow, normal: bool) {
    #[cfg(windows)]
    {
        let window = win.clone();
        if let Err(error) = win.run_on_main_thread(move || {
            set_target_on_main_thread(&window, normal);
        }) {
            eprintln!("Window memory dispatch: {error}");
        }
    }
}

#[cfg(windows)]
fn set_target_on_main_thread(win: &WebviewWindow, normal: bool) {
    if let Err(error) = win.with_webview(move |platform| {
        use webview2_com::Microsoft::Web::WebView2::Win32::{
            COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW,
            COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL, ICoreWebView2_19,
        };
        use windows_core::Interface;

        let result = (|| -> windows_core::Result<()> {
            // COM 对象只在 WebView 所在主线程中使用。
            let core = unsafe { platform.controller().CoreWebView2()? };
            let webview = match core.cast::<ICoreWebView2_19>() {
                Ok(webview) => webview,
                // 较旧 Runtime 不支持该优化，保留默认策略。
                Err(error) if error.code() == windows_core::HRESULT(0x80004002_u32 as i32) => {
                    return Ok(());
                }
                Err(error) => return Err(error),
            };
            let target = if normal {
                COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL
            } else {
                COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW
            };
            let mut current = COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL;
            unsafe {
                webview.MemoryUsageTargetLevel(&mut current)?;
                if current != target {
                    webview.SetMemoryUsageTargetLevel(target)?;
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            eprintln!("WebView2 memory policy: {error}");
        }
    }) {
        eprintln!("WebView2 memory dispatch: {error}");
    }
}
