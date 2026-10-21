//! 主界面：启动信息、看板数据、勾选、撤销、外观

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use super::{EVENT_INBOX_CHANGED, glass_supported, notify_changes, poisoned, with_service};
use crate::agent::AgentReply;
use crate::config::{AppConfig, Appearance};
use crate::db;
use crate::desktop::vibrancy;
use crate::error::{AppError, AppResult};
use crate::service::{Board, ChangeSet, DayCompletion, HeatDay};
use crate::state::AppState;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Bootstrap {
    config: AppConfig,
    data_dir: String,
    dev_build: bool,
    business_date: String,
    schema_version: u32,
    tables: Vec<String>,
    glass_supported: bool,
    desktop_error: Option<String>,
    onboarding_needed: bool,
    model_configured: bool,
}

#[tauri::command]
pub fn get_bootstrap(state: State<'_, AppState>) -> AppResult<Bootstrap> {
    let _maintenance = state
        .maintenance
        .try_read()
        .map_err(|_| AppError::Invalid("正在恢复数据，请稍后再试".into()))?;
    let config = state.config.lock().map_err(poisoned)?.clone();
    let conn = state.db.lock().map_err(poisoned)?;
    let service = crate::service::Service::new(&conn, state.clock.as_ref(), &config)?;
    Ok(Bootstrap {
        business_date: service.today().to_string(),
        schema_version: db::schema_version(&conn)?,
        tables: db::list_tables(&conn)?,
        data_dir: state.paths.root.to_string_lossy().into_owned(),
        dev_build: state.dev,
        glass_supported: glass_supported(&state),
        desktop_error: state.desktop_error.lock().map_err(poisoned)?.clone(),
        onboarding_needed: !config.onboarding_completed
            && !conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM inbox WHERE status='done')",
                [],
                |r| r.get::<_, bool>(0),
            )?,
        model_configured: !config.model.chat.base_url.trim().is_empty()
            && !config.model.chat.model.trim().is_empty()
            && state
                .secrets
                .get(&config.model.chat)?
                .is_some_and(|key| !key.trim().is_empty()),
        config,
    })
}

#[tauri::command]
pub fn complete_onboarding(app: AppHandle, state: State<'_, AppState>) -> AppResult<()> {
    let _guard = state
        .maintenance
        .try_read()
        .map_err(|_| AppError::Invalid("正在恢复数据，请稍后再试".into()))?;
    let next = {
        let mut config = state.config.lock().map_err(poisoned)?;
        let mut next = config.clone();
        next.onboarding_completed = true;
        next.save(&state.paths.config())?;
        *config = next.clone();
        next
    };
    super::notify_config(&app, &next);
    Ok(())
}

#[tauri::command]
pub fn get_board(state: State<'_, AppState>) -> AppResult<Board> {
    let _maintenance = state
        .maintenance
        .try_read()
        .map_err(|_| AppError::Invalid("正在恢复数据，请稍后再试".into()))?;
    with_service(&state, |svc| svc.board())
}

#[tauri::command]
pub fn get_heatmap(state: State<'_, AppState>) -> AppResult<Vec<HeatDay>> {
    let _maintenance = state
        .maintenance
        .try_read()
        .map_err(|_| AppError::Invalid("正在恢复数据，请稍后再试".into()))?;
    with_service(&state, |svc| svc.heatmap())
}

#[tauri::command]
pub fn get_day_completions(
    state: State<'_, AppState>,
    date: String,
) -> AppResult<Vec<DayCompletion>> {
    let _maintenance = state
        .maintenance
        .try_read()
        .map_err(|_| AppError::Invalid("正在恢复数据，请稍后再试".into()))?;
    with_service(&state, |svc| svc.day_completions(&date))
}

/// 勾选 / 取消勾选今日任务：界面唯一的直接写操作
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskDoneResult {
    #[serde(flatten)]
    changes: ChangeSet,
    parent_questions: Vec<String>,
}

#[tauri::command]
pub fn set_task_done(
    app: AppHandle,
    state: State<'_, AppState>,
    id: i64,
    version: i64,
    done: bool,
) -> AppResult<TaskDoneResult> {
    let _maintenance = state
        .maintenance
        .try_read()
        .map_err(|_| AppError::Invalid("正在恢复数据，请稍后再试".into()))?;
    let result = with_service(&state, |svc| {
        let before: Vec<_> = svc
            .parent_completion_candidates()?
            .into_iter()
            .map(|n| n.id)
            .collect();
        let changes = svc.set_task_done(id, version, done)?;
        let parent_questions = svc
            .parent_completion_candidates()?
            .into_iter()
            .filter(|n| !before.contains(&n.id))
            .map(|n| {
                format!(
                    "「{}」的子项都已完成，要把它也标为完成吗？可以在项目模式说：完成「{}」。",
                    n.title, n.title
                )
            })
            .collect();
        Ok(TaskDoneResult {
            changes,
            parent_questions,
        })
    })?;
    notify_changes(&app, &result.changes);
    Ok(result)
}

/// 撤销按钮：直接由 Rust 撤销最近一组用户操作，不经过模型
#[tauri::command]
pub fn undo_last(app: AppHandle, state: State<'_, AppState>) -> AppResult<AgentReply> {
    let _maintenance = state
        .maintenance
        .try_read()
        .map_err(|_| AppError::Invalid("正在恢复数据，请稍后再试".into()))?;
    let (summary, changes) = with_service(&state, |svc| svc.undo_last())?;
    notify_changes(&app, &changes);
    let _ = app.emit(EVENT_INBOX_CHANGED, ());
    Ok(AgentReply::done(
        "",
        format!("知了，已撤销：{summary}。"),
        vec![],
        changes,
    ))
}

/// 修改外观设置：保存、应用毛玻璃，并通知所有窗口刷新
#[tauri::command]
pub fn set_appearance(
    app: AppHandle,
    state: State<'_, AppState>,
    appearance: Appearance,
) -> AppResult<AppConfig> {
    let _maintenance = state
        .maintenance
        .try_read()
        .map_err(|_| AppError::Invalid("正在恢复数据，请稍后再试".into()))?;
    appearance.validate()?;
    let config = {
        let mut cfg = state.config.lock().map_err(poisoned)?;
        let mut next = cfg.clone();
        next.appearance = appearance;
        next.save(&state.paths.config())?;
        *cfg = next.clone();
        next
    };
    let supported = glass_supported(&state);
    for win in tauri::Manager::webview_windows(&app).values() {
        vibrancy::apply(win, config.appearance.theme, supported);
    }
    super::notify_config(&app, &config);
    Ok(config)
}

/// 前端渲染好第一帧后再显示主窗口，避免白屏闪烁
#[tauri::command]
pub fn window_ready(window: tauri::WebviewWindow) -> AppResult<()> {
    if window.label() == "main" {
        crate::updater::mark_ready();
        if !window
            .state::<AppState>()
            .config
            .lock()
            .map_err(super::poisoned)?
            .show_main_on_startup()
        {
            return Ok(());
        }
    }
    crate::desktop::memory::show(&window)
        .and_then(|_| window.set_focus())
        .map_err(|e| AppError::Invalid(format!("显示窗口失败：{e}")))
}
