//! 更新只接受启动或主动检查得到的已签名候选，不接受前端传入路径、版本或 URL。
use crate::{
    error::{AppError, AppResult},
    state::AppState,
    updater::{self, UpdateState, UpdateStatusView, UpdateView},
};
use tauri::{AppHandle, State};

#[tauri::command]
pub fn get_update(state: State<'_, UpdateState>) -> AppResult<Option<UpdateView>> {
    updater::get(&state)
}

#[tauri::command]
pub fn get_update_status(state: State<'_, UpdateState>) -> AppResult<UpdateStatusView> {
    updater::get_status(&state)
}

#[tauri::command]
pub async fn check_update(
    app: AppHandle,
    state: State<'_, UpdateState>,
) -> AppResult<UpdateStatusView> {
    updater::check_now(&app, &state).await
}

#[tauri::command]
pub async fn install_update(
    app: AppHandle,
    update: State<'_, UpdateState>,
    state: State<'_, AppState>,
) -> AppResult<()> {
    let _guard = state
        .maintenance
        .try_write()
        .map_err(|_| AppError::Invalid("还有任务正在处理，请稍后再更新".into()))?;
    updater::install(&app, &update).await
}
