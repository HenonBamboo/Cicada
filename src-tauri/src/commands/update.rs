//! 更新只接受启动检查留下的已签名候选，不接受前端传入路径、版本或 URL。
use crate::{
    error::{AppError, AppResult},
    state::AppState,
    updater::{self, UpdateState, UpdateView},
};
use tauri::{AppHandle, State};

#[tauri::command]
pub fn get_update(state: State<'_, UpdateState>) -> AppResult<Option<UpdateView>> {
    updater::get(&state)
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
