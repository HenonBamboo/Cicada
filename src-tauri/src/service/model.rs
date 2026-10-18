//! 返回给界面的只读视图结构（serde 使用 camelCase，与前端 src/lib/types.ts 对应）

use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TaskLink {
    pub node_id: i64,
    /// 节点所在位置，如「知了 › 阶段 0」
    pub path: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TaskView {
    pub id: i64,
    pub title: String,
    pub planned_date: String,
    pub done: bool,
    /// 遗留任务延期了几天（计划日期到今天的天数）
    pub delay_days: i64,
    pub due_at: Option<String>,
    pub remind_at: Option<String>,
    pub note: Option<String>,
    /// 重复规则的简短说明，如「每天」
    pub recur: Option<String>,
    pub link: Option<TaskLink>,
    pub version: i64,
}

/// 任务区四个分区（docs/产品设计.md 5.2）
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TaskBoard {
    pub today: Vec<TaskView>,
    pub legacy: Vec<TaskView>,
    pub done: Vec<TaskView>,
    pub upcoming: Vec<TaskView>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub done: u32,
    pub total: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectNode {
    pub id: i64,
    pub title: String,
    pub depth: i64,
    pub done: bool,
    /// 只有项目（第 1 层）有：active / done / archived
    pub project_status: Option<String>,
    /// 直接子节点的完成进度；没有子节点时为空
    pub progress: Option<Progress>,
    pub children: Vec<ProjectNode>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Board {
    pub business_date: String,
    pub tasks: TaskBoard,
    pub projects: Vec<ProjectNode>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HeatDay {
    pub date: String,
    pub tasks: u32,
    pub nodes: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DayCompletion {
    pub kind: String,
    pub title: String,
    pub project_title: Option<String>,
}

/// 一次写操作改动了哪些条目，随 data-changed 事件发给所有窗口，用于刷新和高亮
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChangeSet {
    pub tasks: Vec<i64>,
    pub nodes: Vec<i64>,
}
