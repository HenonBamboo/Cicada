//! 非模型设置；只接受对应分组，避免覆盖凭据与其它配置。
use super::poisoned;
use crate::config::{AppConfig, AutomationSettings, DesktopSettings, StickySettings, TimeSettings};
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Preferences {
    time: TimeSettings,
    automation: AutomationSettings,
    desktop: DesktopSettings,
    sticky: StickySettings,
}
impl From<&AppConfig> for Preferences {
    fn from(config: &AppConfig) -> Self {
        Self {
            time: config.time.clone(),
            automation: config.automation.clone(),
            desktop: config.desktop.clone(),
            sticky: config.sticky.clone(),
        }
    }
}
#[tauri::command]
pub fn get_preferences(state: State<'_, AppState>) -> AppResult<Preferences> {
    Ok(Preferences::from(&*state.config.lock().map_err(poisoned)?))
}

#[tauri::command]
pub async fn save_preferences(
    app: AppHandle,
    state: State<'_, AppState>,
    preferences: Preferences,
) -> AppResult<Preferences> {
    let _guard = state.maintenance.write().await;
    let old = state.config.lock().map_err(poisoned)?.clone();
    let mut next = old.clone();
    next.time = preferences.time;
    next.automation = preferences.automation;
    next.desktop = preferences.desktop;
    next.sticky = preferences.sticky;
    next.validate()?;
    crate::desktop::shortcuts::parse_shortcuts(&next.desktop)?;
    let shortcuts_changed = old.desktop.quick_shortcut != next.desktop.quick_shortcut
        || old.desktop.main_shortcut != next.desktop.main_shortcut
        || old.desktop.sticky_shortcut != next.desktop.sticky_shortcut;
    let autostart_changed = old.desktop.autostart != next.desktop.autostart;
    if shortcuts_changed {
        crate::desktop::shortcuts::set_shortcuts(&app, &next.desktop)?;
    }
    let mut autostart_applied = false;
    let commit = (|| {
        if autostart_changed {
            crate::desktop::shortcuts::set_autostart(&app, next.desktop.autostart)?;
            autostart_applied = true;
        }
        next.save(&state.paths.config())
    })();
    if let Err(error) = commit {
        let rollback_keys = if shortcuts_changed {
            crate::desktop::shortcuts::set_shortcuts(&app, &old.desktop)
        } else {
            Ok(())
        };
        let rollback_startup = if autostart_applied {
            crate::desktop::shortcuts::set_autostart(&app, old.desktop.autostart)
        } else {
            Ok(())
        };
        if rollback_keys.is_err() || rollback_startup.is_err() {
            return Err(AppError::Invalid(format!(
                "{error}；系统设置回退失败，请重启知了后重试"
            )));
        }
        return Err(error);
    }
    *state.config.lock().map_err(poisoned)? = next.clone();
    let mut desktop_error = state.desktop_error.lock().map_err(poisoned)?;
    if desktop_error.as_ref().is_some_and(|error| {
        (shortcuts_changed
            && (error.contains("快捷键")
                || error.starts_with("无法替换快捷键")
                || error.starts_with("全局快捷键需要")
                || error.starts_with("三个全局快捷键不能")))
            || (autostart_changed
                && (error.starts_with("读取开机启动状态失败")
                    || error.starts_with("设置开机启动失败")))
    }) {
        *desktop_error = None;
    }
    drop(desktop_error);
    // 默认颜色只影响新建；透明度和贴边隐藏立即应用到已有便利贴。
    let updated = if old.sticky.opacity != next.sticky.opacity
        || old.sticky.auto_hide != next.sticky.auto_hide
    {
        super::with_service(&state, |svc| {
            svc.stickies()?
                .into_iter()
                .map(|sticky| {
                    svc.sticky_update(
                        sticky.id,
                        crate::service::stickies::StickyPatch {
                            opacity: (old.sticky.opacity != next.sticky.opacity)
                                .then_some(next.sticky.opacity),
                            auto_hide: (old.sticky.auto_hide != next.sticky.auto_hide)
                                .then_some(next.sticky.auto_hide),
                            ..Default::default()
                        },
                    )
                })
                .collect::<AppResult<Vec<_>>>()
        })?
    } else {
        Vec::new()
    };
    drop(_guard);
    super::notify_config(&app, &next);
    for sticky in updated {
        if app
            .get_webview_window(&format!("sticky-{}", sticky.id))
            .is_some()
        {
            crate::desktop::windows::sync_sticky(&app, &sticky)?;
        }
    }
    // 日切或周起始日改变后，前端立即重读业务日期与统计范围。
    let _ = app.emit(
        super::EVENT_DATA_CHANGED,
        serde_json::json!({"tasks":[],"nodes":[]}),
    );
    if let Ok(greeting) = crate::scheduler::greetings::current(&state) {
        let _ = app.emit("greeting-changed", greeting);
    }
    state.wake.notify_one();
    Ok(Preferences::from(&next))
}
