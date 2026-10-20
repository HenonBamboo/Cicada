//! 便利贴和快速记录的窄接口。窗口身份来自宿主，不能由前端指定其它窗口ID。
use super::{EVENT_INBOX_CHANGED, notify_changes, poisoned, with_service};
use crate::agent::Mode;
use crate::desktop::windows;
use crate::error::{AppError, AppResult};
use crate::service::stickies::{StickyPatch, StickyView};
use crate::service::{ChangeSet, ProjectNode, TaskBoard};
use crate::state::AppState;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};

pub fn sticky_id(label: &str) -> AppResult<i64> {
    label
        .strip_prefix("sticky-")
        .and_then(|s| s.parse::<i64>().ok())
        .filter(|n| *n > 0)
        .ok_or_else(|| AppError::Invalid("该窗口不能操作便利贴".into()))
}
fn maintenance(state: &AppState) -> AppResult<tokio::sync::RwLockReadGuard<'_, ()>> {
    state
        .maintenance
        .try_read()
        .map_err(|_| AppError::Invalid("正在恢复数据，请稍后再试".into()))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopBootstrap {
    appearance: crate::config::Appearance,
    glass_supported: bool,
}
#[tauri::command]
pub fn get_desktop_bootstrap(state: State<'_, AppState>) -> AppResult<DesktopBootstrap> {
    let _guard = maintenance(&state)?;
    Ok(DesktopBootstrap {
        appearance: state.config.lock().map_err(poisoned)?.appearance.clone(),
        glass_supported: super::glass_supported(&state),
    })
}
#[tauri::command]
pub fn get_sticky_state(
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> AppResult<StickyView> {
    let _guard = maintenance(&state)?;
    let id = sticky_id(window.label())?;
    with_service(&state, |s| s.sticky_get(id))
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StickyBoard {
    business_date: String,
    tasks: TaskBoard,
    project: Option<ProjectNode>,
}
#[tauri::command]
pub fn get_sticky_board(
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> AppResult<StickyBoard> {
    let _guard = maintenance(&state)?;
    let id = sticky_id(window.label())?;
    with_service(&state, |s| {
        let sticky = s.sticky_get(id)?;
        let board = s.board()?;
        let mut tasks = if sticky.kind == "today" {
            board.tasks
        } else {
            TaskBoard::default()
        };
        tasks.upcoming.clear();
        Ok(StickyBoard {
            business_date: board.business_date,
            tasks,
            project: board
                .projects
                .into_iter()
                .find(|p| Some(p.id) == sticky.node_id),
        })
    })
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AppearancePatch {
    color: Option<String>,
    opacity: Option<f64>,
    pinned: Option<bool>,
    locked: Option<bool>,
    auto_hide: Option<bool>,
}
#[tauri::command]
pub fn update_sticky(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
    patch: AppearancePatch,
) -> AppResult<StickyView> {
    let _guard = maintenance(&state)?;
    let id = sticky_id(window.label())?;
    let view = with_service(&state, |s| {
        s.sticky_update(
            id,
            StickyPatch {
                color: patch.color,
                opacity: patch.opacity,
                pinned: patch.pinned,
                locked: patch.locked,
                auto_hide: patch.auto_hide,
                ..Default::default()
            },
        )
    })?;
    windows::sync_sticky(&app, &view)?;
    Ok(view)
}
#[tauri::command]
pub fn close_sticky(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> AppResult<()> {
    let _guard = maintenance(&state)?;
    let id = sticky_id(window.label())?;
    let view = with_service(&state, |s| {
        s.sticky_update(
            id,
            StickyPatch {
                visible: Some(false),
                ..Default::default()
            },
        )
    })?;
    windows::sync_sticky(&app, &view)
}
#[tauri::command]
pub async fn create_sticky(
    app: AppHandle,
    state: State<'_, AppState>,
    kind: String,
    node_id: Option<i64>,
    hide_main: Option<bool>,
) -> AppResult<StickyView> {
    let _guard = maintenance(&state)?;
    let view = with_service(&state, |s| {
        if let Some(view) = s
            .stickies()?
            .into_iter()
            .find(|v| v.kind == kind && v.node_id == node_id)
        {
            s.sticky_update(
                view.id,
                StickyPatch {
                    visible: Some(true),
                    ..Default::default()
                },
            )
        } else {
            s.sticky_create(&kind, node_id)
        }
    })?;
    windows::sync_sticky(&app, &view)?;
    if hide_main.unwrap_or(false) {
        if let Some(main) = app.get_webview_window("main") {
            crate::desktop::memory::hide(&main).map_err(|e| AppError::Invalid(e.to_string()))?;
        }
    }
    Ok(view)
}
#[tauri::command]
pub fn set_sticky_task_done(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
    id: i64,
    version: i64,
    done: bool,
) -> AppResult<ChangeSet> {
    let _guard = maintenance(&state)?;
    let sticky = sticky_id(window.label())?;
    let (changes, questions) = with_service(&state, |s| {
        if s.sticky_get(sticky)?.kind != "today" {
            return Err(AppError::Invalid("项目便利贴只读".into()));
        }
        let before: Vec<_> = s
            .parent_completion_candidates()?
            .into_iter()
            .map(|n| n.id)
            .collect();
        let changes = s.set_task_done(id, version, done)?;
        let questions: Vec<_> = s
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
        Ok((changes, questions))
    })?;
    notify_changes(&app, &changes);
    if !questions.is_empty() {
        let message = questions.join("\n");
        let _ = app.emit_to("main", "desktop-message", &message);
        let _ = crate::desktop::notifications::notify(&app, "知了", &message);
    }
    Ok(changes)
}
#[tauri::command]
pub fn open_main(app: AppHandle) {
    crate::desktop::focus_main(&app);
}
#[tauri::command]
pub fn show_quick_capture(app: AppHandle) -> AppResult<()> {
    windows::show_quick(&app)
}
#[tauri::command]
pub fn hide_quick_capture(app: AppHandle) -> AppResult<()> {
    windows::hide_quick(&app)
}

/// 只登记输入，网络处理交给已有后台队列。失败保留输入，成功后快速窗口即隐藏。
#[tauri::command]
pub fn submit_capture(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
    text: String,
    mode: Mode,
    images: Vec<String>,
    request_id: String,
) -> AppResult<String> {
    let _guard = maintenance(&state)?;
    crate::agent::validate_input(&text, &images)?;
    let id = uuid::Uuid::parse_str(&request_id)
        .map_err(|_| AppError::Invalid("请求编号无效".into()))?
        .to_string();
    let (source, target_project) = if window.label() == "quick" {
        ("quick", None)
    } else {
        let sticky = with_service(&state, |s| s.sticky_get(sticky_id(window.label())?))?;
        (
            "sticky",
            if matches!(mode, Mode::Project) {
                sticky.node_id
            } else {
                None
            },
        )
    };
    let mode = match mode {
        Mode::Task => "task",
        Mode::Project => "project",
    };
    with_service(&state, |s| {
        if s.inbox_exists(&id)? {
            let existing = s.inbox_get(&id)?;
            if existing.source != source
                || existing.content != text
                || existing.mode != mode
                || existing.target_project_id != target_project
            {
                return Err(AppError::Invalid("请求编号已用于其它输入".into()));
            }
        } else {
            let paths = crate::agent::save_images(&state.paths.inbox(), &id, &images)?;
            if let Err(e) =
                s.inbox_register_with_project(&id, source, mode, &text, &paths, target_project)
            {
                crate::agent::drop_images(&state.paths.inbox(), &paths);
                return Err(e);
            }
        }
        Ok(())
    })?;
    if source == "quick" {
        windows::hide_quick(&app)?;
    }
    let _ = app.emit_to("main", EVENT_INBOX_CHANGED, ());
    state.wake.notify_one();
    Ok(id)
}
