//! 暴露给前端的 Tauri 命令。只做参数转换，业务规则在 service / agent 层。

pub mod agent;
pub mod automation;
pub mod backup;
pub mod board;
pub mod data;
pub mod desktop;
pub mod preferences;
pub mod prompt_settings;
pub mod settings;
pub mod update;

use std::sync::atomic::Ordering;

use tauri::{AppHandle, Emitter, Manager};

use crate::error::{AppError, AppResult};
use crate::service::{ChangeSet, Service};
use crate::state::AppState;

pub const EVENT_CONFIG_CHANGED: &str = "config-changed";
/// 业务数据变更：所有窗口收到后重新读取，并高亮改动的条目。
pub const EVENT_DATA_CHANGED: &str = "data-changed";
/// 收件箱有变化（数量、状态）
pub const EVENT_INBOX_CHANGED: &str = "inbox-changed";
/// 模型回复的流式片段
pub const EVENT_AGENT_DELTA: &str = "agent-delta";

pub(crate) fn poisoned<T>(_: T) -> AppError {
    AppError::Invalid("内部状态异常，请重启知了".into())
}

/// 在数据库锁内构造业务服务并执行
pub(crate) fn with_service<T>(
    state: &AppState,
    f: impl FnOnce(&Service<'_>) -> AppResult<T>,
) -> AppResult<T> {
    let config = state.config.lock().map_err(poisoned)?.clone();
    let conn = state.db.lock().map_err(poisoned)?;
    let service = Service::new(&conn, state.clock.as_ref(), &config)?;
    f(&service)
}

pub(crate) fn notify_changes(app: &AppHandle, changes: &ChangeSet) {
    if !changes.tasks.is_empty() || !changes.nodes.is_empty() {
        let _ = app.emit(EVENT_DATA_CHANGED, changes);
        crate::scheduler::wake_copy(app);
    }
}

/// 子窗口只需要外观；模型设置与主窗口流式回复不广播到其它窗口。
pub(crate) fn notify_config(app: &AppHandle, config: &crate::config::AppConfig) {
    let _ = app.emit_to("main", EVENT_CONFIG_CHANGED, config);
    for (label, window) in app.webview_windows() {
        if label != "main" {
            let _ = window.emit(
                EVENT_CONFIG_CHANGED,
                serde_json::json!({"appearance":config.appearance}),
            );
        }
    }
}

#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Delta<'a> {
    pub request_id: &'a str,
    pub text: &'a str,
}

/// 在当前作用域里用应用状态构造代理环境 `$ctx`：流式片段、数据变更转成事件。
/// 用宏而不是闭包，是为了让 async 命令的 Future 保持 Send，且不跨 await 持有锁
#[macro_export]
macro_rules! agent_ctx {
    ($ctx:ident, $app:expr, $state:expr) => {
        let __config = $state
            .config
            .lock()
            .map_err($crate::commands::poisoned)?
            .clone();
        let __app = $app.clone();
        let __app2 = $app.clone();
        let __on_delta = move |id: &str, text: &str| {
            let _ = tauri::Emitter::emit_to(
                &__app,
                "main",
                $crate::commands::EVENT_AGENT_DELTA,
                $crate::commands::Delta {
                    request_id: id,
                    text,
                },
            );
        };
        let __on_changes =
            move |c: &$crate::service::ChangeSet| $crate::commands::notify_changes(&__app2, c);
        let $ctx = $crate::agent::Ctx {
            db: &$state.db,
            clock: $state.clock.as_ref(),
            config: __config,
            prompts_dir: $state.paths.prompts(),
            inbox_dir: $state.paths.inbox(),
            secrets: $state.secrets.as_ref(),
            on_delta: &__on_delta,
            on_changes: &__on_changes,
            #[cfg(test)]
            test_model: None,
        };
    };
}

pub(crate) fn glass_supported(state: &AppState) -> bool {
    state.glass_supported.load(Ordering::Relaxed)
}
