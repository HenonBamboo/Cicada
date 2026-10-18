//! 备份设置命令。
use tauri::{AppHandle, Emitter, Manager, State};

use super::{EVENT_DATA_CHANGED, EVENT_INBOX_CHANGED, poisoned};
use crate::backup::{self, BackupEntry};
use crate::error::AppResult;
use crate::state::AppState;

#[tauri::command]
pub fn list_backups(state: State<'_, AppState>) -> AppResult<Vec<BackupEntry>> {
    backup::list(&state.paths)
}

#[tauri::command]
pub async fn create_backup(state: State<'_, AppState>) -> AppResult<BackupEntry> {
    let _maintenance = state.maintenance.write().await;
    let _config = state.config.lock().map_err(poisoned)?;
    let conn = state.db.lock().map_err(poisoned)?;
    backup::create(&state.paths, &conn, state.clock.as_ref(), "manual")
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreResult {
    needs_api_key: bool,
}

#[tauri::command]
pub async fn restore_backup(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
) -> AppResult<RestoreResult> {
    let _maintenance = state.maintenance.write().await;
    let mut config = state.config.lock().map_err(poisoned)?;
    let mut conn = state.db.lock().map_err(poisoned)?;
    backup::restore(
        &state.paths,
        &mut conn,
        &mut config,
        state.clock.as_ref(),
        &name,
    )?;
    let needs_api_key = std::iter::once(&config.model.chat)
        .chain(config.model.vision.as_ref())
        .any(|ep| state.secrets.get(ep).ok().flatten().is_none());
    let restored_config = config.clone();
    drop(conn);
    drop(config);
    drop(_maintenance);
    for window in app.webview_windows().values() {
        crate::desktop::vibrancy::apply(
            window,
            restored_config.appearance.theme,
            super::glass_supported(&state),
        );
    }
    super::notify_config(&app, &restored_config);
    crate::desktop::windows::restore_stickies(&app)?;
    if let Err(e) = crate::desktop::shortcuts::set_shortcuts(&app, &restored_config.desktop)
        .and_then(|_| {
            crate::desktop::shortcuts::set_autostart(&app, restored_config.desktop.autostart)
        })
    {
        *state.desktop_error.lock().map_err(poisoned)? = Some(e.to_string());
        let _ = app.emit_to("main", "desktop-error", e.to_string());
    }
    let _ = app.emit(
        EVENT_DATA_CHANGED,
        serde_json::json!({"tasks":[],"nodes":[]}),
    );
    let _ = app.emit(EVENT_INBOX_CHANGED, ());
    state.wake.notify_one();
    Ok(RestoreResult { needs_api_key })
}
