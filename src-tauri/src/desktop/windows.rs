//! 便利贴与预创建的快速记录窗口。窗口几何使用逻辑像素持久化。
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use tauri::{
    AppHandle, Emitter, Listener, LogicalPosition, LogicalSize, Manager, PhysicalPosition,
    WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent,
};

use crate::commands::{poisoned, with_service};
use crate::error::{AppError, AppResult};
use crate::service::stickies::{StickyPatch, StickyView};
use crate::state::AppState;

pub const QUICK_WINDOW: &str = "quick";

#[derive(Clone, Copy, PartialEq)]
struct Geometry {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

#[derive(Clone, Copy)]
enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

struct Tracking {
    geometry: Geometry,
    saved: Geometry,
    changed: Instant,
    hidden: Option<(Edge, PhysicalPosition<i32>)>,
    auto_hide: bool,
    locked: bool,
}

#[derive(Default)]
struct WindowState(Mutex<HashMap<i64, Tracking>>, AtomicBool);

fn window_error(error: impl std::fmt::Display) -> AppError {
    AppError::Invalid(format!("窗口操作失败：{error}"))
}

fn geometry(win: &WebviewWindow) -> AppResult<Geometry> {
    let scale = win.scale_factor().map_err(window_error)?;
    let pos = win
        .outer_position()
        .map_err(window_error)?
        .to_logical::<i32>(scale);
    let size = win
        .inner_size()
        .map_err(window_error)?
        .to_logical::<u32>(scale);
    Ok(Geometry {
        x: pos.x,
        y: pos.y,
        width: size.width,
        height: size.height,
    })
}

/// 在 AppState 注册之后调用；不访问自启动设置。
pub fn init(app: &AppHandle) -> AppResult<()> {
    app.manage(WindowState::default());
    if app.get_webview_window(QUICK_WINDOW).is_none() {
        let quick = WebviewWindowBuilder::new(
            app,
            QUICK_WINDOW,
            WebviewUrl::App("index.html?window=quick".into()),
        )
        .title("知了 · 快速记录")
        .inner_size(640.0, 132.0)
        .resizable(false)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .skip_taskbar(true)
        .always_on_top(true)
        .visible(false)
        .focused(false)
        .center()
        .build()
        .map_err(window_error)?;
        super::memory::observe(&quick);
        let win = quick.clone();
        quick.on_window_event(move |event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = super::memory::hide(&win);
            }
        });
        super::memory::hide(&quick).map_err(window_error)?;
    }
    if let Some(main) = app.get_webview_window(super::MAIN_WINDOW) {
        super::memory::observe(&main);
        let handle = app.clone();
        main.on_window_event(move |event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let close_to_tray = handle
                    .state::<AppState>()
                    .config
                    .lock()
                    .map(|config| config.desktop.close_to_tray)
                    .unwrap_or(true);
                if !close_to_tray {
                    handle.exit(0);
                    return;
                }
                if let Some(win) = handle.get_webview_window(super::MAIN_WINDOW) {
                    let _ = super::memory::hide(&win);
                }
            }
        });
    }
    with_service(&app.state::<AppState>(), |svc| svc.sticky_ensure_default())?;
    let show = !app
        .state::<AppState>()
        .config
        .lock()
        .map_err(poisoned)?
        .show_main_on_startup();
    if show {
        // 用户选择只显示便利贴时，至少保留一张可见便签作为入口。
        with_service(&app.state::<AppState>(), |svc| {
            let all = svc.stickies()?;
            if !all.iter().any(|sticky| sticky.visible) {
                if let Some(today) = all.iter().find(|sticky| sticky.kind == "today") {
                    svc.sticky_update(
                        today.id,
                        StickyPatch {
                            visible: Some(true),
                            ..Default::default()
                        },
                    )?;
                } else {
                    svc.sticky_create("today", None)?;
                }
            }
            Ok(())
        })?;
        restore_stickies(app)?;
    } else {
        // 仅抑制本次启动显示，不覆盖每张便签原有的显示状态。
        app.state::<WindowState>().1.store(true, Ordering::Relaxed);
    }
    let events = app.clone();
    app.listen("data-changed", move |_| {
        let handle = events.clone();
        tauri::async_runtime::spawn(async move {
            let _ = reconcile_stickies(&handle);
        });
    });
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(250)).await;
            // 原生窗口调用统一在主线程执行，避免持布局锁等待主线程时与窗口命令互锁。
            let app = handle.clone();
            let _ = handle.run_on_main_thread(move || {
                let _ = update_geometry(&app);
            });
        }
    });
    Ok(())
}

pub fn show_quick(app: &AppHandle) -> AppResult<()> {
    let win = app
        .get_webview_window(QUICK_WINDOW)
        .ok_or_else(|| window_error("快速记录窗口尚未就绪"))?;
    win.center().map_err(window_error)?;
    super::memory::show(&win).map_err(window_error)?;
    win.set_focus().map_err(window_error)?;
    let _ = win.emit("quick-opened", ());
    Ok(())
}

pub fn hide_quick(app: &AppHandle) -> AppResult<()> {
    if let Some(win) = app.get_webview_window(QUICK_WINDOW) {
        super::memory::hide(&win).map_err(window_error)?;
    }
    Ok(())
}

pub fn restore_stickies(app: &AppHandle) -> AppResult<()> {
    app.state::<WindowState>().1.store(false, Ordering::Relaxed);
    let state = app.state::<AppState>();
    let _guard = state
        .maintenance
        .try_read()
        .map_err(|_| window_error("正在恢复数据"))?;
    let stickies = with_service(&state, |svc| svc.stickies())?;
    for (label, win) in app.webview_windows() {
        if let Some(id) = label
            .strip_prefix("sticky-")
            .and_then(|s| s.parse::<i64>().ok())
        {
            if !stickies.iter().any(|sticky| sticky.id == id) {
                win.destroy().map_err(window_error)?;
            }
        }
    }
    for sticky in stickies {
        sync_sticky(app, &sticky)?;
    }
    Ok(())
}

fn reconcile_stickies(app: &AppHandle) -> AppResult<()> {
    let suppressed = app.state::<WindowState>().1.load(Ordering::Relaxed);
    let state = app.state::<AppState>();
    let Ok(_guard) = state.maintenance.try_read() else {
        return Ok(());
    };
    let stickies = with_service(&state, |svc| svc.stickies())?;
    for (label, win) in app.webview_windows() {
        if let Some(id) = label
            .strip_prefix("sticky-")
            .and_then(|s| s.parse::<i64>().ok())
        {
            if !stickies.iter().any(|sticky| sticky.id == id) {
                win.destroy().map_err(window_error)?;
                app.state::<WindowState>()
                    .0
                    .lock()
                    .map_err(poisoned)?
                    .remove(&id);
            }
        }
    }
    for sticky in stickies {
        if !suppressed
            && sticky.visible
            && app
                .get_webview_window(&format!("sticky-{}", sticky.id))
                .is_none()
        {
            sync_sticky(app, &sticky)?;
        }
    }
    Ok(())
}

/// 应用业务服务返回的窗口状态。配置颜色、透明度由前端从数据中读取。
pub fn sync_sticky(app: &AppHandle, sticky: &StickyView) -> AppResult<()> {
    let label = format!("sticky-{}", sticky.id);
    let existing = app.get_webview_window(&label);
    if existing.is_none() && !sticky.visible {
        return Ok(());
    }
    let win = if let Some(win) = existing {
        win
    } else {
        let win = WebviewWindowBuilder::new(
            app,
            &label,
            WebviewUrl::App(format!("index.html?window=sticky&id={}", sticky.id).into()),
        )
        .title("知了 · 便利贴")
        .inner_size(sticky.width as f64, sticky.height as f64)
        .min_inner_size(200.0, 160.0)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .skip_taskbar(true)
        .visible(false)
        .focused(false)
        .build()
        .map_err(window_error)?;
        let handle = app.clone();
        let id = sticky.id;
        super::memory::observe(&win);
        win.on_window_event(move |event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let state = handle.state::<AppState>();
                if let Ok(_guard) = state.maintenance.try_read() {
                    if let Ok(view) = with_service(&state, |svc| {
                        svc.sticky_update(
                            id,
                            StickyPatch {
                                visible: Some(false),
                                ..Default::default()
                            },
                        )
                    }) {
                        let _ = sync_sticky(&handle, &view);
                    }
                }
            }
        });
        win
    };
    win.set_size(LogicalSize::new(sticky.width as f64, sticky.height as f64))
        .map_err(window_error)?;
    if let (Some(x), Some(y)) = (sticky.x, sticky.y) {
        win.set_position(LogicalPosition::new(x as f64, y as f64))
            .map_err(window_error)?;
        // 显示器移除后保证至少一块可操作区域还在屏幕中。
        let pos = win.outer_position().map_err(window_error)?;
        let reachable = win
            .available_monitors()
            .map_err(window_error)?
            .iter()
            .any(|m| {
                let a = m.work_area();
                pos.x >= a.position.x - sticky.width as i32 + 40
                    && pos.y >= a.position.y
                    && pos.x < a.position.x + a.size.width as i32 - 40
                    && pos.y < a.position.y + a.size.height as i32 - 40
            });
        if !reachable {
            win.center().map_err(window_error)?;
        }
    } else {
        win.center().map_err(window_error)?;
    }
    win.set_always_on_top(sticky.pinned).map_err(window_error)?;
    win.set_ignore_cursor_events(sticky.locked)
        .map_err(window_error)?;
    if sticky.visible {
        super::memory::show(&win).map_err(window_error)?;
    } else {
        super::memory::hide(&win).map_err(window_error)?;
    }
    let g = geometry(&win)?;
    app.state::<WindowState>()
        .0
        .lock()
        .map_err(poisoned)?
        .insert(
            sticky.id,
            Tracking {
                geometry: g,
                saved: g,
                changed: Instant::now(),
                hidden: None,
                auto_hide: sticky.auto_hide,
                locked: sticky.locked,
            },
        );
    let _ = app.emit("sticky-changed", ());
    Ok(())
}

pub fn toggle_stickies(app: &AppHandle) -> AppResult<()> {
    let state = app.state::<AppState>();
    let _guard = state
        .maintenance
        .try_read()
        .map_err(|_| window_error("正在恢复数据"))?;
    let suppressed = app.state::<WindowState>().1.load(Ordering::Relaxed);
    let views = with_service(&state, |svc| {
        let views = svc.stickies()?;
        let visible = suppressed || !views.iter().any(|s| s.visible);
        views
            .into_iter()
            .map(|s| {
                svc.sticky_update(
                    s.id,
                    StickyPatch {
                        visible: Some(visible),
                        ..Default::default()
                    },
                )
            })
            .collect::<AppResult<Vec<_>>>()
    })?;
    app.state::<WindowState>().1.store(false, Ordering::Relaxed);
    for view in views {
        sync_sticky(app, &view)?;
    }
    Ok(())
}

pub fn unlock_stickies(app: &AppHandle) -> AppResult<()> {
    let state = app.state::<AppState>();
    let _guard = state
        .maintenance
        .try_read()
        .map_err(|_| window_error("正在恢复数据"))?;
    let views = with_service(&state, |svc| {
        svc.stickies()?
            .into_iter()
            .filter(|s| s.locked)
            .map(|s| {
                svc.sticky_update(
                    s.id,
                    StickyPatch {
                        locked: Some(false),
                        ..Default::default()
                    },
                )
            })
            .collect::<AppResult<Vec<_>>>()
    })?;
    for view in views {
        sync_sticky(app, &view)?;
    }
    Ok(())
}

fn update_geometry(app: &AppHandle) -> AppResult<()> {
    let state = app.state::<AppState>();
    let Ok(_guard) = state.maintenance.try_read() else {
        return Ok(());
    };
    let tracking = app.state::<WindowState>();
    let mut entries = tracking.0.lock().map_err(poisoned)?;
    for (id, entry) in entries.iter_mut() {
        let Some(win) = app.get_webview_window(&format!("sticky-{id}")) else {
            continue;
        };
        if !win.is_visible().unwrap_or(false) {
            continue;
        }
        let monitor = if let Some((_, anchor)) = entry.hidden {
            // 隐藏后大部分窗体位于屏幕外，不能再按窗体面积选择相邻显示器。
            win.available_monitors()
                .map_err(window_error)?
                .into_iter()
                .find(|monitor| {
                    let area = monitor.work_area();
                    anchor.x >= area.position.x
                        && anchor.x < area.position.x + area.size.width as i32
                        && anchor.y >= area.position.y
                        && anchor.y < area.position.y + area.size.height as i32
                })
        } else {
            win.current_monitor().map_err(window_error)?
        };
        let Some(monitor) = monitor else {
            continue;
        };
        let a = monitor.work_area();
        let scale = win.scale_factor().map_err(window_error)?;
        let size = win.outer_size().map_err(window_error)?;
        let cursor = win.cursor_position().map_err(window_error)?;
        if let Some((edge, anchor)) = entry.hidden {
            let at_edge = match edge {
                Edge::Left => {
                    cursor.x >= a.position.x as f64
                        && cursor.x < (a.position.x + 8) as f64
                        && cursor.y >= anchor.y as f64
                        && cursor.y < (anchor.y + size.height as i32) as f64
                }
                Edge::Right => {
                    cursor.x >= (a.position.x + a.size.width as i32 - 8) as f64
                        && cursor.x < (a.position.x + a.size.width as i32) as f64
                        && cursor.y >= anchor.y as f64
                        && cursor.y < (anchor.y + size.height as i32) as f64
                }
                Edge::Top => {
                    cursor.y >= a.position.y as f64
                        && cursor.y < (a.position.y + 8) as f64
                        && cursor.x >= anchor.x as f64
                        && cursor.x < (anchor.x + size.width as i32) as f64
                }
                Edge::Bottom => {
                    cursor.y >= (a.position.y + a.size.height as i32 - 8) as f64
                        && cursor.y < (a.position.y + a.size.height as i32) as f64
                        && cursor.x >= anchor.x as f64
                        && cursor.x < (anchor.x + size.width as i32) as f64
                }
            };
            if at_edge {
                win.set_position(anchor).map_err(window_error)?;
                entry.hidden = None;
                entry.changed = Instant::now();
            }
            continue;
        }
        let now = geometry(&win)?;
        if now != entry.geometry {
            entry.geometry = now;
            entry.changed = Instant::now();
            continue;
        }
        if entry.changed.elapsed() < Duration::from_millis(600) {
            continue;
        }
        let mut pos = win.outer_position().map_err(window_error)?;
        let right = a.position.x + a.size.width as i32 - size.width as i32;
        let bottom = a.position.y + a.size.height as i32 - size.height as i32;
        let margin = (12.0 * scale) as i32;
        let edge = if (pos.x - a.position.x).abs() <= margin {
            pos.x = a.position.x;
            Some(Edge::Left)
        } else if (pos.x - right).abs() <= margin {
            pos.x = right;
            Some(Edge::Right)
        } else if (pos.y - a.position.y).abs() <= margin {
            pos.y = a.position.y;
            Some(Edge::Top)
        } else if (pos.y - bottom).abs() <= margin {
            pos.y = bottom;
            Some(Edge::Bottom)
        } else {
            None
        };
        if edge.is_some() {
            win.set_position(pos).map_err(window_error)?;
        }
        let actual = geometry(&win)?;
        if actual != entry.saved {
            with_service(&state, |svc| {
                svc.sticky_update(
                    *id,
                    StickyPatch {
                        x: Some(actual.x),
                        y: Some(actual.y),
                        width: Some(actual.width),
                        height: Some(actual.height),
                        ..Default::default()
                    },
                )
            })?;
            entry.saved = actual;
            entry.geometry = actual;
        }
        let inside = cursor.x >= pos.x as f64
            && cursor.x < (pos.x + size.width as i32) as f64
            && cursor.y >= pos.y as f64
            && cursor.y < (pos.y + size.height as i32) as f64;
        if entry.auto_hide
            && !entry.locked
            && size.width <= a.size.width
            && size.height <= a.size.height
            && !inside
            && !win.is_focused().unwrap_or(false)
        {
            if let Some(edge) = edge {
                let hidden = match edge {
                    Edge::Left => {
                        PhysicalPosition::new(a.position.x - size.width as i32 + 6, pos.y)
                    }
                    Edge::Right => {
                        PhysicalPosition::new(a.position.x + a.size.width as i32 - 6, pos.y)
                    }
                    Edge::Top => {
                        PhysicalPosition::new(pos.x, a.position.y - size.height as i32 + 6)
                    }
                    Edge::Bottom => {
                        PhysicalPosition::new(pos.x, a.position.y + a.size.height as i32 - 6)
                    }
                };
                win.set_position(hidden).map_err(window_error)?;
                entry.hidden = Some((edge, pos));
            }
        }
    }
    Ok(())
}
