//! 设置页数据操作；前端仅传文件内容，不传文件系统路径。
use tauri::{AppHandle, Emitter, State};

use super::{EVENT_DATA_CHANGED, poisoned, with_service};
use crate::{
    backup,
    error::AppResult,
    service::data::{ExportFile, ImportResult, OperationLog, Transfer},
    state::AppState,
};

#[tauri::command]
pub async fn export_data(state: State<'_, AppState>, format: String) -> AppResult<ExportFile> {
    let _maintenance = state.maintenance.read().await;
    with_service(&state, |s| s.export_data(&format))
}

#[tauri::command]
pub async fn import_data(
    app: AppHandle,
    state: State<'_, AppState>,
    content: String,
) -> AppResult<ImportResult> {
    let data = Transfer::parse(&content)?;
    let _maintenance = state.maintenance.write().await;
    let config = state.config.lock().map_err(poisoned)?;
    let conn = state.db.lock().map_err(poisoned)?;
    backup::create(&state.paths, &conn, state.clock.as_ref(), "pre-import")?;
    let result =
        crate::service::Service::new(&conn, state.clock.as_ref(), &config)?.import_data(data)?;
    drop(conn);
    drop(config);
    drop(_maintenance);
    let _ = app.emit(
        EVENT_DATA_CHANGED,
        serde_json::json!({"tasks":[],"nodes":[]}),
    );
    Ok(result)
}

#[tauri::command]
pub async fn list_operation_log(state: State<'_, AppState>) -> AppResult<Vec<OperationLog>> {
    let _maintenance = state.maintenance.read().await;
    with_service(&state, |s| s.operation_log())
}
