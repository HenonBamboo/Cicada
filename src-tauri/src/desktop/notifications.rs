//! 系统提醒；调度与去重由 scheduler 负责。
use tauri::{AppHandle, Emitter};
use tauri_plugin_notification::NotificationExt;

use crate::error::{AppError, AppResult};

/// 启动时注册一次。Windows 插件仅报告屏幕上 Toast 的点击，通知中心点击不在其支持范围内。
pub fn init(app: &AppHandle) -> AppResult<()> {
    let handle = app.clone();
    app.notification()
        .on_action(move |action| {
            let app = handle.clone();
            let review = action.notification().is_some_and(|n| n.id() == 6001);
            let pending = action
                .notification()
                .and_then(|n| n.title())
                .is_some_and(|t| t == "知了 · 需要处理");
            let _ = handle.run_on_main_thread(move || {
                if review {
                    let _ = super::windows::show_quick(&app);
                } else {
                    super::focus_main(&app);
                    if pending {
                        let _ = app.emit_to("main", "main-action", "inbox");
                    }
                }
            });
        })
        .map_err(|error| AppError::Invalid(format!("注册通知点击处理失败：{error}")))
}

pub fn review(app: &AppHandle, body: &str) -> AppResult<()> {
    app.notification()
        .builder()
        .id(6001)
        .title("知了 · 每日回顾")
        .body(body)
        .show()
        .map_err(|error| AppError::Invalid(format!("发送回顾通知失败：{error}")))
}

/// 提交通知。成功表示交给系统通知插件，系统勿扰或通知权限仍可能阻止实际显示。
pub fn notify(app: &AppHandle, title: &str, body: &str) -> AppResult<()> {
    app.notification()
        .builder()
        .title(title)
        .body(body)
        .show()
        .map_err(|error| AppError::Invalid(format!("发送系统通知失败：{error}")))
}
