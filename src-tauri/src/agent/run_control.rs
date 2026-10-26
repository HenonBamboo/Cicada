//! 运行中的输入与取消信号。取消和最终提交共用阶段锁，停止成功后不能再写入计划。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tokio::sync::Notify;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StopState {
    Stopping,
    Stopped,
    Committing,
    Finished,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Running,
    Cancelled,
    Committing,
    Finished,
}

struct RunState {
    phase: Mutex<Phase>,
    cancelled: Notify,
}

#[derive(Default)]
pub struct RunControl {
    active: Mutex<HashMap<String, Arc<RunState>>>,
}

impl RunControl {
    pub fn start(&self, id: &str) -> Option<RunGuard<'_>> {
        let mut active = self.active.lock().unwrap_or_else(|e| e.into_inner());
        if active.contains_key(id) {
            return None;
        }
        let state = Arc::new(RunState {
            phase: Mutex::new(Phase::Running),
            cancelled: Notify::new(),
        });
        active.insert(id.into(), state.clone());
        Some(RunGuard {
            owner: self,
            id: id.into(),
            state,
        })
    }

    pub fn stop(&self, id: &str) -> StopState {
        let active = self.active.lock().unwrap_or_else(|e| e.into_inner());
        let Some(state) = active.get(id) else {
            return StopState::Finished;
        };
        let mut phase = state.phase.lock().unwrap_or_else(|e| e.into_inner());
        match *phase {
            Phase::Running | Phase::Cancelled => {
                *phase = Phase::Cancelled;
                // notify_one 保留许可，允许停止发生在等待 future 首次轮询之前。
                state.cancelled.notify_one();
                StopState::Stopping
            }
            Phase::Committing => StopState::Committing,
            Phase::Finished => StopState::Finished,
        }
    }
}

pub struct RunGuard<'a> {
    owner: &'a RunControl,
    id: String,
    state: Arc<RunState>,
}

impl RunGuard<'_> {
    pub fn is_cancelled(&self) -> bool {
        *self.state.phase.lock().unwrap_or_else(|e| e.into_inner()) == Phase::Cancelled
    }

    pub async fn cancelled(&self) {
        while !self.is_cancelled() {
            self.state.cancelled.notified().await;
        }
    }

    /// 此后只允许同步的最终落盘，不能再 await；已开始落盘时停止请求必须如实返回。
    pub fn begin_finish(&self) -> bool {
        let mut phase = self.state.phase.lock().unwrap_or_else(|e| e.into_inner());
        if *phase != Phase::Running {
            return false;
        }
        *phase = Phase::Committing;
        true
    }

    /// 封闭所有早退路径；避免网络失败刚排队时接受的停止信号被丢失。
    pub fn finish(&self) -> bool {
        let mut phase = self.state.phase.lock().unwrap_or_else(|e| e.into_inner());
        let cancelled = *phase == Phase::Cancelled;
        *phase = Phase::Finished;
        cancelled
    }
}

impl Drop for RunGuard<'_> {
    fn drop(&mut self) {
        let mut active = self.owner.active.lock().unwrap_or_else(|e| e.into_inner());
        *self.state.phase.lock().unwrap_or_else(|e| e.into_inner()) = Phase::Finished;
        active.remove(&self.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn 停止先于等待也能唤醒且不能提交() {
        let control = RunControl::default();
        let run = control.start("request").unwrap();
        assert!(control.start("request").is_none());
        assert_eq!(control.stop("request"), StopState::Stopping);
        tokio::time::timeout(std::time::Duration::from_millis(100), run.cancelled())
            .await
            .unwrap();
        assert!(!run.begin_finish());
        drop(run);
        assert!(control.start("request").is_some());
    }

    #[test]
    fn 已开始提交时不误报停止成功() {
        let control = RunControl::default();
        let run = control.start("request").unwrap();
        assert!(run.begin_finish());
        assert_eq!(control.stop("request"), StopState::Committing);
        assert!(!run.is_cancelled());
    }
}
