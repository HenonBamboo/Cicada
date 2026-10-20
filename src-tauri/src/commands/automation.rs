//! 已有问候语和报告只读入口。
use super::poisoned;
use crate::error::{AppError, AppResult};
use crate::scheduler::{greetings, reports};
use crate::state::AppState;
use tauri::State;

#[tauri::command]
pub fn get_ui_copy(state: State<'_, AppState>) -> AppResult<greetings::UiCopyView> {
    let _guard = state
        .maintenance
        .try_read()
        .map_err(|_| AppError::Invalid("正在恢复数据，请稍后再试".into()))?;
    Ok(greetings::UiCopyView::from(&greetings::current(&state)?))
}

#[tauri::command]
pub fn get_greeting(state: State<'_, AppState>) -> AppResult<greetings::GreetingView> {
    let _guard = state
        .maintenance
        .try_read()
        .map_err(|_| AppError::Invalid("正在恢复数据，请稍后再试".into()))?;
    greetings::current(&state)
}

#[tauri::command]
pub fn list_reports(state: State<'_, AppState>) -> AppResult<Vec<reports::ReportEntry>> {
    let _guard = state
        .maintenance
        .try_read()
        .map_err(|_| AppError::Invalid("正在恢复数据，请稍后再试".into()))?;
    let conn = state.db.lock().map_err(poisoned)?;
    reports::list(&conn)
}

#[tauri::command]
pub fn read_report(state: State<'_, AppState>, id: i64) -> AppResult<reports::ReportDocument> {
    let _guard = state
        .maintenance
        .try_read()
        .map_err(|_| AppError::Invalid("正在恢复数据，请稍后再试".into()))?;
    let conn = state.db.lock().map_err(poisoned)?;
    reports::read(&conn, &state.paths, id)
}
