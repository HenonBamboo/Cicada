//! 输入框、确认 / 取消、收件箱。所有模型调用都在 Rust 端完成

use tauri::{AppHandle, Emitter, State, WebviewWindow};

use super::{EVENT_INBOX_CHANGED, with_service};
use crate::agent::{self, AgentReply, Input, Mode};
use crate::agent_ctx;
use crate::error::{AppError, AppResult};
use crate::service::inbox::InboxItem;
use crate::state::AppState;

fn inbox_changed(app: &AppHandle) {
    let _ = app.emit(EVENT_INBOX_CHANGED, ());
}

fn conversation_source(label: &str) -> AppResult<&'static str> {
    match label {
        "main" => Ok("main"),
        "quick" => Ok("quick"),
        _ => Err(AppError::Invalid("该窗口不能处理对话".into())),
    }
}

fn ensure_source_access(caller: &str, source: &str) -> AppResult<()> {
    if conversation_source(caller)? == "main" || source == "quick" {
        Ok(())
    } else {
        Err(AppError::Invalid("快速记录只能处理本入口的输入".into()))
    }
}

/// 快速窗口不得借请求 ID 读取、续答或修改主窗口 / 便利贴的请求。
fn ensure_request_access(state: &AppState, caller: &str, id: &str) -> AppResult<()> {
    conversation_source(caller)?;
    if caller == "main" {
        return Ok(());
    }
    with_service(state, |s| {
        if s.inbox_exists(id)? {
            ensure_source_access(caller, &s.inbox_get(id)?.source)?;
        }
        Ok(())
    })
}

/// 输入框提交：来源与流式回复目标取自宿主窗口。
#[tauri::command]
pub async fn submit_input(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
    text: String,
    mode: Mode,
    images: Vec<String>,
    request_id: Option<String>,
    reply_to: Option<String>,
    selection_token: Option<String>,
) -> AppResult<AgentReply> {
    let _maintenance = state.maintenance.read().await;
    let source = conversation_source(window.label())?;
    if let Some(id) = request_id.as_deref() {
        ensure_request_access(&state, source, id)?;
    }
    if let Some(id) = reply_to.as_deref() {
        ensure_request_access(&state, source, id)?;
    }
    agent_ctx!(ctx, app, state, source);
    let input = Input {
        source,
        mode,
        text,
        images,
    };
    let r = if let Some(token) = selection_token {
        let reply_to = reply_to
            .ok_or_else(|| crate::error::AppError::Invalid("请选择当前追问中的选项".into()))?;
        if !input.images.is_empty() {
            return Err(crate::error::AppError::Invalid(
                "选择追问选项时不能附带图片".into(),
            ));
        }
        agent::respond_selection(&ctx, &reply_to, &token).await
    } else if let Some(reply_to) = reply_to {
        agent::respond_to(&ctx, input, request_id, &reply_to).await
    } else {
        agent::submit_with_id(&ctx, input, request_id).await
    };
    inbox_changed(&app);
    r
}

/// 确认按钮：直接执行已保存的计划，不再经过模型
#[tauri::command]
pub async fn confirm_pending(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
    request_id: String,
) -> AppResult<AgentReply> {
    let _maintenance = state.maintenance.read().await;
    ensure_request_access(&state, window.label(), &request_id)?;
    agent_ctx!(ctx, app, state, window.label().to_owned());
    let r = agent::confirm(&ctx, &request_id).await;
    inbox_changed(&app);
    r
}

#[tauri::command]
pub fn cancel_pending(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
    request_id: String,
) -> AppResult<AgentReply> {
    let _maintenance = state
        .maintenance
        .try_read()
        .map_err(|_| crate::error::AppError::Invalid("正在恢复备份，请稍后再试".into()))?;
    ensure_request_access(&state, window.label(), &request_id)?;
    agent_ctx!(ctx, app, state, window.label().to_owned());
    let r = agent::cancel(&ctx, &request_id);
    inbox_changed(&app);
    r
}

/// 实时停止独立于确认取消，也不等待正在执行的模型或数据维护写锁。
#[tauri::command]
pub fn stop_input(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
    request_ids: Vec<String>,
) -> AppResult<agent::run_control::StopState> {
    use agent::run_control::StopState;
    if request_ids.is_empty() || request_ids.len() > 2 {
        return Err(crate::error::AppError::Invalid("待停止的输入无效".into()));
    }
    let ids = request_ids
        .iter()
        .map(|id| {
            uuid::Uuid::parse_str(id)
                .map(|id| id.to_string())
                .map_err(|_| crate::error::AppError::Invalid("待停止的输入无效".into()))
        })
        .collect::<AppResult<Vec<_>>>()?;
    // Validate every supplied reference before stopping any one of them.
    for id in &ids {
        ensure_request_access(&state, window.label(), id)?;
    }
    for id in &ids {
        let active = state.agent_runs.stop(id);
        if active != StopState::Finished {
            return Ok(active);
        }
    }
    let _maintenance = state
        .maintenance
        .try_read()
        .map_err(|_| crate::error::AppError::Invalid("正在恢复备份，请稍后再试".into()))?;
    agent_ctx!(ctx, app, state);
    let result = agent::stop_running(&ctx, &ids);
    inbox_changed(&app);
    result
}

#[tauri::command]
pub fn list_inbox(window: WebviewWindow, state: State<'_, AppState>) -> AppResult<Vec<InboxItem>> {
    let source = conversation_source(window.label())?;
    let _maintenance = state
        .maintenance
        .try_read()
        .map_err(|_| crate::error::AppError::Invalid("正在恢复备份，请稍后再试".into()))?;
    with_service(&state, |s| {
        let mut items = s.inbox_open()?;
        if source == "quick" {
            items.retain(|item| item.source == source);
        }
        Ok(items)
    })
}

/// 输入历史按实际调用窗口校验来源，子窗口不能读取主窗口的历史。
#[tauri::command]
pub fn list_input_history(
    window: WebviewWindow,
    state: State<'_, AppState>,
    mode: String,
    source: Option<String>,
) -> AppResult<Vec<String>> {
    let _maintenance = state
        .maintenance
        .try_read()
        .map_err(|_| crate::error::AppError::Invalid("正在恢复备份，请稍后再试".into()))?;
    let expected = if window.label() == "main" {
        "main"
    } else if window.label() == "quick" {
        "quick"
    } else {
        super::desktop::sticky_id(window.label())?;
        "sticky"
    };
    let source = source.as_deref().unwrap_or("main");
    if source != expected {
        return Err(crate::error::AppError::Invalid(
            "该窗口不能读取其它入口的输入历史".into(),
        ));
    }
    with_service(&state, |s| s.input_history(&mode, source))
}

/// 收件箱里的"重试"
#[tauri::command]
pub async fn retry_inbox(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
    request_id: String,
) -> AppResult<AgentReply> {
    let _maintenance = state.maintenance.read().await;
    ensure_request_access(&state, window.label(), &request_id)?;
    agent_ctx!(ctx, app, state, window.label().to_owned());
    let r = agent::retry(&ctx, &request_id).await;
    inbox_changed(&app);
    r
}

/// 收件箱里的"放弃"：只处理待处理的输入，不是对任务数据的编辑
#[tauri::command]
pub fn discard_inbox(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
    request_id: String,
) -> AppResult<()> {
    let _maintenance = state
        .maintenance
        .try_read()
        .map_err(|_| crate::error::AppError::Invalid("正在恢复备份，请稍后再试".into()))?;
    ensure_request_access(&state, window.label(), &request_id)?;
    agent_ctx!(ctx, app, state, window.label().to_owned());
    agent::discard(&ctx, &request_id)?;
    inbox_changed(&app);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 快速对话只允许本来源_主窗口可以接续所有入口() {
        assert_eq!(conversation_source("quick").unwrap(), "quick");
        assert!(conversation_source("sticky-1").is_err());
        for source in ["main", "quick", "sticky"] {
            assert!(ensure_source_access("main", source).is_ok());
            assert_eq!(
                ensure_source_access("quick", source).is_ok(),
                source == "quick"
            );
        }
    }
}
