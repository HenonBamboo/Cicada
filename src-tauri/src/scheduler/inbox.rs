//! 收件箱后台处理（docs/产品设计.md 4.6）：
//! 定时检查到了重试时间的离线暂存；配置好模型后处理之前暂存的输入；
//! 处理结果通过 Windows 通知告知。删除、批量修改只会到"待确认"，不会自动执行。

use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager};

use crate::agent::{self, AgentReply, ReplyKind};
use crate::agent_ctx;
use crate::commands::{EVENT_INBOX_CHANGED, with_service};
use crate::error::AppResult;
use crate::state::AppState;

const TICK: Duration = Duration::from_secs(30);

pub fn spawn_inbox_worker(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        {
            let state = app.state::<AppState>();
            let _maintenance = state.maintenance.read().await;
            let _ = with_service(&state, |s| s.inbox_recover());
        }
        loop {
            {
                let state = app.state::<AppState>();
                tokio::select! {
                    _ = state.wake.notified() => {}
                    _ = tokio::time::sleep(TICK) => {}
                }
            }
            let _ = run_once(&app).await;
        }
    });
}

async fn run_once(app: &AppHandle) -> AppResult<()> {
    let state = app.state::<AppState>();
    let _maintenance = state.maintenance.read().await;
    agent_ctx!(ctx, app, state);
    let cleaned = agent::cleanup(&ctx)?;
    let (expired, ids) = with_service(&state, |s| {
        let expired = s.pending_expire()?;
        let ids = s.inbox_ready()?;
        Ok((expired, ids))
    })?;
    if !cleaned && expired.is_empty() && ids.is_empty() {
        return Ok(());
    }
    for id in ids {
        if let Ok(reply) = agent::process(&ctx, &id).await {
            notify(app, &reply);
        }
    }
    let _ = app.emit(EVENT_INBOX_CHANGED, ());
    Ok(())
}

/// 后台处理完成后通知用户；仍在暂存的不打扰
fn notify(app: &AppHandle, r: &AgentReply) {
    let body = match r.kind {
        ReplyKind::Unconfigured | ReplyKind::Offline => return,
        ReplyKind::Confirm | ReplyKind::Clarify => format!("{} 打开知了处理。", r.lead),
        _ => r.lead.clone(),
    };
    let title = if matches!(r.kind, ReplyKind::Confirm | ReplyKind::Clarify) {
        "知了 · 需要处理"
    } else {
        "知了"
    };
    let _ = crate::desktop::notifications::notify(app, title, &body);
}
