//! 系统托盘，保留应用在主窗口关闭后的入口。
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Listener, Manager};

use crate::commands::with_service;
use crate::error::{AppError, AppResult};
use crate::service::stickies::StickyPatch;
use crate::state::AppState;

fn menu_error(error: tauri::Error) -> AppError {
    AppError::Invalid(format!("更新托盘失败：{error}"))
}

fn projects(app: &AppHandle) -> AppResult<Vec<(i64, String)>> {
    let state = app.state::<AppState>();
    let _guard = state
        .maintenance
        .try_read()
        .map_err(|_| AppError::Invalid("正在恢复数据，请稍后再试".into()))?;
    with_service(&state, |svc| {
        Ok(svc
            .project_tree()?
            .into_iter()
            .map(|project| (project.id, project.title))
            .collect())
    })
}

/// 调用前已经释放数据库锁；菜单变更只在主线程进行。
fn update_projects(
    app: &AppHandle,
    menu: &Submenu<tauri::Wry>,
    projects: Vec<(i64, String)>,
) -> AppResult<()> {
    while menu.remove_at(0).map_err(menu_error)?.is_some() {}
    if projects.is_empty() {
        let empty = MenuItem::with_id(app, "project-empty", "暂无项目", false, None::<&str>)
            .map_err(menu_error)?;
        menu.append(&empty).map_err(menu_error)?;
    }
    for (id, title) in projects {
        // 原生菜单把 & 解释为助记符，转义后显示完整项目名称。
        let item = MenuItem::with_id(
            app,
            format!("project-sticky-{id}"),
            title.replace('&', "&&"),
            true,
            None::<&str>,
        )
        .map_err(menu_error)?;
        menu.append(&item).map_err(menu_error)?;
    }
    Ok(())
}

fn open_project(app: &AppHandle, id: i64) -> AppResult<()> {
    let state = app.state::<AppState>();
    let _guard = state
        .maintenance
        .try_read()
        .map_err(|_| AppError::Invalid("正在恢复数据，请稍后再试".into()))?;
    let view = with_service(&state, |svc| {
        if let Some(view) = svc
            .stickies()?
            .into_iter()
            .find(|view| view.kind == "project" && view.node_id == Some(id))
        {
            svc.sticky_update(
                view.id,
                StickyPatch {
                    visible: Some(true),
                    ..Default::default()
                },
            )
        } else {
            svc.sticky_create("project", Some(id))
        }
    })?;
    super::windows::sync_sticky(app, &view)
}

fn report_error(app: &AppHandle, error: AppError) {
    let _ = app.emit("desktop-error", error.to_string());
    super::focus_main(app);
}

pub fn init(app: &AppHandle) -> AppResult<()> {
    let open =
        MenuItem::with_id(app, "open", "打开知了", true, None::<&str>).map_err(menu_error)?;
    let stickies = MenuItem::with_id(app, "stickies", "显示 / 隐藏便利贴", true, None::<&str>)
        .map_err(menu_error)?;
    let unlock = MenuItem::with_id(app, "unlock", "解锁所有便利贴", true, None::<&str>)
        .map_err(menu_error)?;
    let project_menu =
        Submenu::with_id(app, "project-stickies", "新建项目便利贴", true).map_err(menu_error)?;
    update_projects(app, &project_menu, projects(app)?)?;
    let quick =
        MenuItem::with_id(app, "quick", "快速记录", true, None::<&str>).map_err(menu_error)?;
    let settings =
        MenuItem::with_id(app, "settings", "设置", true, None::<&str>).map_err(menu_error)?;
    let separator = PredefinedMenuItem::separator(app).map_err(menu_error)?;
    let exit =
        MenuItem::with_id(app, "exit", "退出知了", true, None::<&str>).map_err(menu_error)?;
    let menu = Menu::with_items(
        app,
        &[
            &open,
            &stickies,
            &unlock,
            &project_menu,
            &quick,
            &settings,
            &separator,
            &exit,
        ],
    )
    .map_err(menu_error)?;
    let icon = tauri::include_image!("icons/16x16.png");
    TrayIconBuilder::with_id("cicada")
        .icon(icon)
        .tooltip("知了")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            let result = match event.id.as_ref() {
                "open" => {
                    super::focus_main(app);
                    Ok(())
                }
                "stickies" => {
                    let handle = app.clone();
                    tauri::async_runtime::spawn_blocking(move || {
                        if let Err(error) = super::windows::toggle_stickies(&handle) {
                            report_error(&handle, error);
                        }
                    });
                    Ok(())
                }
                "unlock" => {
                    let handle = app.clone();
                    tauri::async_runtime::spawn_blocking(move || {
                        if let Err(error) = super::windows::unlock_stickies(&handle) {
                            report_error(&handle, error);
                        }
                    });
                    Ok(())
                }
                "quick" => super::windows::show_quick(app),
                "settings" => {
                    super::focus_main(app);
                    let _ = app.emit("main-action", "settings");
                    Ok(())
                }
                "exit" => {
                    app.exit(0);
                    Ok(())
                }
                id if id.starts_with("project-sticky-") => {
                    if let Some(id) = id
                        .strip_prefix("project-sticky-")
                        .and_then(|id| id.parse::<i64>().ok())
                    {
                        let handle = app.clone();
                        // Windows WebView 创建不能阻塞原生菜单事件线程。
                        tauri::async_runtime::spawn_blocking(move || {
                            if let Err(error) = open_project(&handle, id) {
                                report_error(&handle, error);
                            }
                        });
                    }
                    Ok(())
                }
                _ => Ok(()),
            };
            if let Err(error) = result {
                report_error(app, error);
            }
        })
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                super::focus_main(tray.app_handle());
            }
        })
        .build(app)
        .map_err(menu_error)?;
    let events = app.clone();
    app.listen("data-changed", move |_| {
        let handle = events.clone();
        let menu = project_menu.clone();
        tauri::async_runtime::spawn_blocking(move || {
            if let Ok(items) = projects(&handle) {
                let app = handle.clone();
                let _ = handle.run_on_main_thread(move || {
                    if let Err(error) = update_projects(&app, &menu, items) {
                        report_error(&app, error);
                    }
                });
            }
        });
    });
    Ok(())
}
