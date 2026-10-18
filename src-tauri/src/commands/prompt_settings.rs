//! 提示词设置。ID 白名单决定路径，预览复用实际请求构造，工具定义仍由程序提供。
use super::poisoned;
use crate::{
    error::{AppError, AppResult},
    prompts::{self, PromptId, PromptView},
    state::AppState,
};
use serde::Serialize;
use tauri::State;

#[tauri::command]
pub fn get_prompts(state: State<'_, AppState>) -> AppResult<Vec<PromptView>> {
    let _guard = state
        .maintenance
        .try_read()
        .map_err(|_| AppError::Invalid("正在恢复数据，请稍后再试".into()))?;
    PromptId::ALL
        .into_iter()
        .map(|id| prompts::view(&state.paths.prompts(), id))
        .collect()
}

#[tauri::command]
pub fn save_prompt(
    state: State<'_, AppState>,
    id: String,
    content: String,
) -> AppResult<PromptView> {
    let _guard = state
        .maintenance
        .try_read()
        .map_err(|_| AppError::Invalid("正在恢复数据，请稍后再试".into()))?;
    prompts::save(&state.paths.prompts(), PromptId::parse(&id)?, &content)
}

#[tauri::command]
pub fn reset_prompt(state: State<'_, AppState>, id: String) -> AppResult<PromptView> {
    let _guard = state
        .maintenance
        .try_read()
        .map_err(|_| AppError::Invalid("正在恢复数据，请稍后再试".into()))?;
    prompts::reset(&state.paths.prompts(), PromptId::parse(&id)?)
}

#[derive(Serialize)]
pub struct PromptPreview {
    pub content: String,
    pub tools: String,
}

#[tauri::command]
pub fn preview_prompt(state: State<'_, AppState>, id: String) -> AppResult<PromptPreview> {
    let _guard = state
        .maintenance
        .try_read()
        .map_err(|_| AppError::Invalid("正在恢复数据，请稍后再试".into()))?;
    let id = PromptId::parse(&id)?;
    let config = state.config.lock().map_err(poisoned)?.clone();
    let now = state
        .clock
        .now()
        .format(crate::service::DATETIME_FMT)
        .to_string();
    let date = state
        .clock
        .business_date(crate::clock::parse_cutoff(&config.time.day_cutoff)?)
        .to_string();
    let dir = state.paths.prompts();
    if matches!(id, PromptId::Vision | PromptId::Greeting | PromptId::Report) {
        return Ok(PromptPreview {
            content: prompts::render_single(
                &dir,
                id,
                &config,
                &now,
                &date,
                if id == PromptId::Vision {
                    "图片识别"
                } else {
                    "只读总结"
                },
            ),
            tools: String::new(),
        });
    }
    let mode = if id == PromptId::Project {
        "project"
    } else {
        "task"
    };
    Ok(PromptPreview {
        content: prompts::system_prompt(&dir, &config, &now, &date, mode),
        tools: serde_json::to_string_pretty(&crate::agent::tools::specs_for(mode))?,
    })
}
