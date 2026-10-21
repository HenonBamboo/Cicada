use crate::commands::{EVENT_DATA_CHANGED, poisoned, with_service};
use crate::desktop::notifications;
use crate::error::AppResult;
use crate::state::AppState;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

pub fn spawn_automation(app: AppHandle) {
    app.manage(super::CopyWake(tokio::sync::Notify::new()));
    let local_app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut startup = true;
        loop {
            if tick(&local_app, startup).await.is_ok() {
                startup = false;
            }
            tokio::time::sleep(Duration::from_secs(15)).await;
        }
    });
    tauri::async_runtime::spawn(async move {
        let mut report_retry = Instant::now();
        loop {
            let state = app.state::<AppState>();
            let visible = app.get_webview_window("main").is_some_and(|w| {
                w.is_visible().unwrap_or(false) && !w.is_minimized().unwrap_or(false)
            });
            if visible && let Ok(greeting) = super::greetings::get(&state).await {
                let _ = app.emit("greeting-changed", &greeting);
                let _ = app.emit(
                    "ui-copy-changed",
                    super::greetings::UiCopyView::from(&greeting),
                );
            }
            if Instant::now() >= report_retry {
                match super::reports::catch_up(&state).await {
                    Ok(n) => {
                        if n > 0 {
                            let _ = app.emit("reports-changed", ());
                        }
                    }
                    Err(_) => report_retry = Instant::now() + Duration::from_secs(15 * 60),
                }
            }
            let wake = app.state::<super::CopyWake>();
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(60)) => {},
                _ = wake.0.notified() => {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
            }
        }
    });
}

async fn tick(app: &AppHandle, startup: bool) -> AppResult<()> {
    let state = app.state::<AppState>();
    let _maintenance = state.maintenance.read().await;
    let config = state.config.lock().map_err(poisoned)?.clone();
    // 每个实际运行的业务日只备份一次；离线日期只补业务，不伪造过去的备份。
    let backup_result = {
        let conn = state.db.lock().map_err(poisoned)?;
        crate::backup::daily(&state.paths, &conn, &config, state.clock.as_ref())
    };
    let rollover = with_service(&state, |svc| svc.rollover())?;
    if !rollover.processed_dates.is_empty()
        || !rollover.changes.tasks.is_empty()
        || !rollover.changes.nodes.is_empty()
    {
        // 即使只有完成区归档也要刷新业务日期与热力图。
        let _ = app.emit(EVENT_DATA_CHANGED, &rollover.changes);
    }
    with_service(&state, |svc| {
        let reminders = svc.due_reminders()?;
        if !reminders.is_empty() {
            if startup && reminders.len() > 1 {
                let titles = reminders
                    .iter()
                    .take(5)
                    .map(|r| r.title.as_str())
                    .collect::<Vec<_>>()
                    .join("、");
                notifications::notify(
                    app,
                    "知了 · 错过的提醒",
                    &format!(
                        "有 {} 条提醒：{}{}",
                        reminders.len(),
                        titles,
                        if reminders.len() > 5 { "等" } else { "" }
                    ),
                )?;
                svc.acknowledge_reminders(&reminders)?;
            } else {
                for reminder in reminders {
                    notifications::notify(app, "知了 · 提醒", &reminder.title)?;
                    svc.acknowledge_reminders(&[reminder])?;
                }
            }
        }
        if let Some((date, count)) = svc.review_due(&config.automation)? {
            notifications::review(app, &format!("今天完成了 {count} 件，明天要做什么？"))?;
            svc.acknowledge_review(&date)?;
        }
        Ok(())
    })?;
    backup_result.map(|_| ())
}
