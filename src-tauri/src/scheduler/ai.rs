//! 后台只读文本生成：不暴露工具，不持有数据库或配置锁等待网络。
use crate::config::AppConfig;
use crate::error::{AppError, AppResult};
use crate::llm::{ChatRequest, Message, Model, StopReason};
use crate::prompts::{self, PromptId};
use crate::state::AppState;

pub(super) fn poisoned<T>(_: T) -> AppError {
    AppError::Invalid("内部状态异常，请重启知了".into())
}

pub(super) async fn generate(
    state: &AppState,
    config: &AppConfig,
    prompt: PromptId,
    context: String,
) -> AppResult<String> {
    let endpoint = &config.model.chat;
    let key = state
        .secrets
        .get(endpoint)?
        .filter(|key| !key.trim().is_empty())
        .ok_or_else(|| AppError::Invalid("还没有配置模型".into()))?;
    let model = Model::new(endpoint, key).map_err(|e| AppError::Invalid(e.to_string()))?;
    let now = state.clock.now();
    let today = state
        .clock
        .business_date(crate::clock::parse_cutoff(&config.time.day_cutoff)?);
    let mut system = prompts::render_single(
        &state.paths.prompts(),
        prompt,
        config,
        &now.format(crate::service::DATETIME_FMT).to_string(),
        &today.to_string(),
        "只读总结",
    );
    if prompt == PromptId::Greeting {
        // 更新不改写用户提示词；界面格式与事实边界由程序补充。
        system.push_str(super::greetings::COPY_RULES);
    }
    let messages = [Message::User { text: context }];
    let request_id = uuid::Uuid::new_v4().to_string();
    // 流式连接持续有心跳也必须结束，避免长期占用维护读锁阻塞数据恢复。
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(90),
        model.chat_tracked(
            &ChatRequest {
                system: &system,
                messages: &messages,
                images: &[],
                tools: &[],
            },
            &|_| {},
            crate::usage::UsageContext {
                db: &state.db,
                clock: state.clock.as_ref(),
                config,
                endpoint,
                purpose: if prompt == PromptId::Greeting {
                    "ui_copy"
                } else {
                    "report"
                },
                request_id: &request_id,
                notify: Some(&state.usage_changed),
            },
        ),
    )
    .await
    .map_err(|_| AppError::Invalid("后台文本生成超时，请稍后重试".into()))?
    .map_err(|e| AppError::Invalid(e.to_string()))?;
    if response.stop != StopReason::Done
        || !response.tool_calls.is_empty()
        || response.text.trim().is_empty()
    {
        return Err(AppError::Invalid(
            "模型没有返回完整的文本，请稍后重试".into(),
        ));
    }
    Ok(response.text.trim().to_string())
}
