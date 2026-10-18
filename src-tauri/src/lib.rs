pub mod agent;
pub mod backup;
pub mod clock;
#[path = "../command_manifest.rs"]
mod command_manifest;
pub mod commands;
pub mod config;
pub mod db;
pub mod desktop;
pub mod error;
pub mod llm;
pub mod paths;
pub mod prompts;
pub mod scheduler;
pub mod secrets;
pub mod service;
pub mod state;

use std::sync::atomic::Ordering;
use std::time::Duration;

use tauri::{Emitter, Manager};

use crate::commands::{
    agent as agent_cmd, automation, backup as backup_cmd, board, data, desktop as desktop_cmd,
    preferences, prompt_settings, settings,
};
use crate::desktop::{MAIN_WINDOW, vibrancy};
use crate::state::AppState;

/// 前端没有按时报告就绪时，最多等这么久就显示主窗口
const SHOW_FALLBACK: Duration = Duration::from_secs(3);

fn report_startup_failure(error: &tauri::Error) {
    let app_data = std::env::var_os("APPDATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("%APPDATA%"));
    let data_dir = app_data.join("Cicada");
    let backups = data_dir.join("backups");
    let message = format!(
        "知了无法启动。\r\n\r\n{error}\r\n\r\n数据目录：{}\r\n备份目录：{}\r\n\r\n请先保留整个数据目录。需要恢复时，可将 Cicada 文件夹改名后重新启动，再从设置的数据页恢复原备份。",
        data_dir.display(),
        backups.display()
    );
    let _ = std::fs::create_dir_all(&data_dir);
    let _ = std::fs::write(data_dir.join("startup-error.txt"), &message);
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
        let mut wide: Vec<u16> = message.encode_utf16().collect();
        wide.push(0);
        let mut title: Vec<u16> = "知了启动失败".encode_utf16().collect();
        title.push(0);
        // SAFETY: 两个 UTF-16 缓冲区均以 NUL 结尾，并在调用期间保持有效。
        unsafe {
            MessageBoxW(
                std::ptr::null_mut(),
                wide.as_ptr(),
                title.as_ptr(),
                MB_OK | MB_ICONERROR,
            );
        }
    }
    #[cfg(not(windows))]
    eprintln!("{message}");
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let result = tauri::Builder::default()
        // 单实例插件必须最先注册
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            desktop::focus_main(app);
        }))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(
            tauri_plugin_autostart::Builder::new()
                .app_name(if cfg!(debug_assertions) {
                    "Cicada-dev"
                } else {
                    "Cicada"
                })
                .build(),
        )
        .setup(|app| {
            // Windows 上 data_dir 即 %APPDATA%（Roaming）
            let state = AppState::system(&app.path().data_dir()?)?;
            let theme = state.config.lock().map(|c| c.appearance.theme).ok();
            if let Some(win) = app.get_webview_window(MAIN_WINDOW) {
                if let Some(theme) = theme {
                    let supported = vibrancy::probe(&win, theme);
                    state.glass_supported.store(supported, Ordering::Relaxed);
                }
                std::thread::spawn(move || {
                    std::thread::sleep(SHOW_FALLBACK);
                    if !win.is_visible().unwrap_or(true) {
                        let _ = desktop::memory::show(&win);
                    }
                });
            }
            app.manage(state);
            desktop::windows::init(app.handle())?;
            desktop::tray::init(app.handle())?;
            if let Err(error) = desktop::shortcuts::init(app.handle()) {
                if let Ok(mut slot) = app.state::<AppState>().desktop_error.lock() {
                    *slot = Some(error.to_string());
                }
                let _ = app.emit_to("main", "desktop-error", error.to_string());
            }
            desktop::notifications::init(app.handle())?;
            scheduler::spawn_inbox_worker(app.handle().clone());
            scheduler::spawn_automation(app.handle().clone());
            Ok(())
        })
        .invoke_handler(|invoke| {
            if !command_manifest::allowed(
                invoke.message.webview_ref().label(),
                invoke.message.command(),
            ) {
                invoke.resolver.reject("该窗口没有调用此命令的权限");
                return true;
            }
            let handler: fn(tauri::ipc::Invoke<tauri::Wry>) -> bool = tauri::generate_handler![
                board::get_bootstrap,
                board::complete_onboarding,
                board::get_board,
                board::get_heatmap,
                board::get_day_completions,
                board::set_task_done,
                board::undo_last,
                board::set_appearance,
                board::window_ready,
                agent_cmd::submit_input,
                agent_cmd::confirm_pending,
                agent_cmd::cancel_pending,
                agent_cmd::list_inbox,
                agent_cmd::retry_inbox,
                agent_cmd::discard_inbox,
                settings::get_model_settings,
                preferences::get_preferences,
                preferences::save_preferences,
                prompt_settings::get_prompts,
                prompt_settings::save_prompt,
                prompt_settings::reset_prompt,
                prompt_settings::preview_prompt,
                data::export_data,
                data::import_data,
                data::list_operation_log,
                settings::save_model_settings,
                settings::clear_api_key,
                settings::test_model,
                settings::open_folder,
                backup_cmd::list_backups,
                backup_cmd::create_backup,
                backup_cmd::restore_backup,
                automation::get_greeting,
                automation::list_reports,
                automation::read_report,
                desktop_cmd::get_desktop_bootstrap,
                desktop_cmd::get_sticky_state,
                desktop_cmd::get_sticky_board,
                desktop_cmd::update_sticky,
                desktop_cmd::close_sticky,
                desktop_cmd::create_sticky,
                desktop_cmd::set_sticky_task_done,
                desktop_cmd::open_main,
                desktop_cmd::show_quick_capture,
                desktop_cmd::hide_quick_capture,
                desktop_cmd::submit_capture,
            ];
            handler(invoke)
        })
        .run(tauri::generate_context!());
    if let Err(error) = result {
        report_startup_failure(&error);
        std::process::exit(1);
    }
}
