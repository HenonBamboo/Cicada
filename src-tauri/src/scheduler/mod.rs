//! 后台定时任务：短周期本地调度与模型生成分开，网络等待不阻塞提醒。

mod ai;
pub mod greetings;
mod inbox;
pub mod reports;
mod timer;

pub use inbox::spawn_inbox_worker;
pub use timer::spawn_automation;

pub struct CopyWake(pub tokio::sync::Notify);

pub fn wake_copy(app: &tauri::AppHandle) {
    use tauri::Manager;
    if let Some(wake) = app.try_state::<CopyWake>() {
        wake.0.notify_one();
    }
}
