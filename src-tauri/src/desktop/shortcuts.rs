//! 全局快速记录快捷键，以及已有自启动设置的系统同步。
use std::sync::Mutex;
use tauri::{AppHandle, Manager};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::commands::poisoned;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

#[derive(Clone, Copy)]
enum ShortcutAction {
    Quick,
    Main,
    Stickies,
}

impl ShortcutAction {
    const ALL: [Self; 3] = [Self::Quick, Self::Main, Self::Stickies];

    fn label(self) -> &'static str {
        match self {
            Self::Quick => "快速记录",
            Self::Main => "主窗口",
            Self::Stickies => "便利贴",
        }
    }
}

#[derive(Clone, Copy)]
struct ShortcutSlot {
    action: ShortcutAction,
    key: Shortcut,
    registered: bool,
}

#[derive(Default)]
struct Registered(Mutex<Vec<ShortcutSlot>>);

pub fn init(app: &AppHandle) -> AppResult<()> {
    app.manage(Registered::default());
    let config = app
        .state::<AppState>()
        .config
        .lock()
        .map_err(poisoned)?
        .desktop
        .clone();
    let keys = parse_shortcuts(&config)?;
    *app.state::<Registered>().0.lock().map_err(poisoned)? = ShortcutAction::ALL
        .into_iter()
        .zip(keys.iter().copied())
        .map(|(action, key)| ShortcutSlot {
            action,
            key,
            registered: false,
        })
        .collect();

    // 启动时逐项注册。某个组合被占用时，其余入口仍应可用。
    let mut errors = Vec::new();
    for (index, key) in keys.into_iter().enumerate() {
        match register(app, key) {
            Ok(()) => {
                if let Ok(mut slots) = app.state::<Registered>().0.lock() {
                    slots[index].registered = true;
                }
            }
            Err(error) => errors.push(format!(
                "{}快捷键{}",
                ShortcutAction::ALL[index].label(),
                error
            )),
        }
    }
    if let Err(error) = set_autostart(app, config.autostart) {
        errors.push(error.to_string());
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(AppError::Invalid(errors.join("；")))
    }
}

/// 先注册新组合再移除旧组合，冲突时保留原有可用快捷键。
pub fn parse_shortcuts(config: &crate::config::DesktopSettings) -> AppResult<Vec<Shortcut>> {
    let mut values = Vec::new();
    for value in [
        &config.quick_shortcut,
        &config.main_shortcut,
        &config.sticky_shortcut,
    ] {
        let key: Shortcut = value
            .parse()
            .map_err(|e| AppError::Invalid(format!("快捷键格式不正确：{e}")))?;
        if key.mods.is_empty() {
            return Err(AppError::Invalid(
                "全局快捷键需要包含 Ctrl、Alt、Shift 或 Win 修饰键".into(),
            ));
        }
        if values.iter().any(|old: &Shortcut| old.id() == key.id()) {
            return Err(AppError::Invalid("三个全局快捷键不能使用相同组合".into()));
        }
        values.push(key);
    }
    Ok(values)
}

fn register(app: &AppHandle, key: Shortcut) -> AppResult<()> {
    app.global_shortcut()
        .on_shortcut(key, |app, key, event| {
            if event.state != ShortcutState::Pressed {
                return;
            }
            let action = app.state::<Registered>().0.lock().ok().and_then(|slots| {
                slots
                    .iter()
                    .find(|slot| slot.registered && slot.key.id() == key.id())
                    .map(|slot| slot.action)
            });
            let app = app.clone();
            tauri::async_runtime::spawn_blocking(move || {
                let result = match action {
                    Some(ShortcutAction::Quick) => super::windows::show_quick(&app),
                    Some(ShortcutAction::Main) => {
                        if let Some(win) = app.get_webview_window(super::MAIN_WINDOW) {
                            if win.is_visible().unwrap_or(false)
                                && !win.is_minimized().unwrap_or(false)
                            {
                                super::memory::hide(&win)
                                    .map_err(|e| AppError::Invalid(e.to_string()))
                            } else {
                                super::focus_main(&app);
                                Ok(())
                            }
                        } else {
                            Ok(())
                        }
                    }
                    Some(ShortcutAction::Stickies) => super::windows::toggle_stickies(&app),
                    _ => Ok(()),
                };
                if let Err(error) = result {
                    let _ =
                        tauri::Emitter::emit_to(&app, "main", "desktop-error", error.to_string());
                }
            });
        })
        .map_err(|e| AppError::Invalid(format!("无法注册快捷键，可能已被占用：{e}")))
}

pub fn set_shortcuts(app: &AppHandle, config: &crate::config::DesktopSettings) -> AppResult<()> {
    let next = parse_shortcuts(config)?;
    let registered = app.state::<Registered>();
    let mut previous = registered.0.lock().map_err(poisoned)?;
    let changed: Vec<_> = if previous.len() == ShortcutAction::ALL.len() {
        next.iter()
            .enumerate()
            .filter(|(index, key)| previous[*index].key.id() != key.id())
            .map(|(index, key)| (index, *key))
            .collect()
    } else {
        next.iter().copied().enumerate().collect()
    };
    for (position, (_, key)) in changed.iter().enumerate() {
        if let Err(error) = register(app, *key) {
            for (_, key) in &changed[..position] {
                let _ = app.global_shortcut().unregister(*key);
            }
            return Err(error);
        }
    }
    let removed: Vec<_> = if previous.len() == ShortcutAction::ALL.len() {
        changed
            .iter()
            .filter_map(|(index, _)| previous[*index].registered.then_some(previous[*index].key))
            .collect()
    } else {
        Vec::new()
    };
    for (position, key) in removed.iter().enumerate() {
        if let Err(error) = app.global_shortcut().unregister(*key) {
            for key in &removed[..position] {
                let _ = register(app, *key);
            }
            for (_, key) in &changed {
                let _ = app.global_shortcut().unregister(*key);
            }
            return Err(AppError::Invalid(format!("无法替换快捷键：{error}")));
        }
    }
    if previous.len() == ShortcutAction::ALL.len() {
        for (index, key) in changed {
            previous[index].key = key;
            previous[index].registered = true;
        }
    } else {
        *previous = ShortcutAction::ALL
            .into_iter()
            .zip(next)
            .map(|(action, key)| ShortcutSlot {
                action,
                key,
                registered: true,
            })
            .collect();
    }
    Ok(())
}

pub fn set_autostart(app: &AppHandle, enabled: bool) -> AppResult<()> {
    let manager = app.autolaunch();
    let current = manager
        .is_enabled()
        .map_err(|e| AppError::Invalid(format!("读取开机启动状态失败：{e}")))?;
    if current == enabled {
        return Ok(());
    }
    let result = if enabled {
        manager.enable()
    } else {
        manager.disable()
    };
    result.map_err(|e| AppError::Invalid(format!("设置开机启动失败：{e}")))
}
