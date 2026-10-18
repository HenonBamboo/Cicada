//! 变更计划（docs/产品设计.md 4.4）。
//!
//! 模型的写工具不直接改数据，而是生成一份计划：校验 → 必要时确认 → 原子提交。
//! 提交时按版本号重新校验，相关对象变化过就放弃提交，交给上层重新预览。

use std::collections::{HashMap, HashSet};

use chrono::{NaiveDate, NaiveDateTime};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};

use super::oplog::Source;
use super::project_changes::{ProjectChange, ProjectValidation};
use super::recur::{RecurChange, RecurValidation};
use super::write::Writer;
use super::{ChangeSet, DATE_FMT, DATETIME_FMT, Service};
use crate::error::{AppError, AppResult};

/// 任务引用：已有任务的编号，或本计划中前面新建的任务（"$1" 表示第 1 条新建）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TaskRef {
    Id(i64),
    Temp(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Change {
    Recur {
        change: RecurChange,
    },
    Project {
        change: ProjectChange,
    },
    TaskAdd {
        title: String,
        planned_date: String,
        due_at: Option<String>,
        remind_at: Option<String>,
        note: Option<String>,
    },
    /// 字段为 None 表示不改；Some("") 表示清空
    TaskUpdate {
        task: TaskRef,
        title: Option<String>,
        planned_date: Option<String>,
        due_at: Option<String>,
        remind_at: Option<String>,
        note: Option<String>,
    },
    TaskDelete {
        task: TaskRef,
    },
    TaskSetDone {
        task: TaskRef,
        done: bool,
    },
    Undo,
}

/// 涉及的已有对象及其版本号，提交前据此判断是否有变化
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pin {
    #[serde(default = "task_entity")]
    pub entity: String,
    pub id: i64,
    pub version: i64,
}

fn task_entity() -> String {
    "tasks".into()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewItem {
    /// 图标名，与前端 Icon 一致
    pub icon: String,
    pub text: String,
    pub danger: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Validated {
    pub pins: Vec<Pin>,
    pub preview: Vec<PreviewItem>,
    /// 需要用户确认：有删除、影响超过 3 条已有任务、或取消往日的完成
    pub dangerous: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Applied {
    pub changes: ChangeSet,
    /// 同一请求已经提交过（重试时不重复执行）
    pub already: bool,
}

/// 影响多少条已有任务时需要确认
pub const CONFIRM_THRESHOLD: usize = 3;

struct TaskInfo {
    title: String,
    version: i64,
    status: String,
    done_date: Option<String>,
}

pub fn valid_date(s: &str) -> bool {
    NaiveDate::parse_from_str(s, DATE_FMT).is_ok()
}

pub fn valid_datetime(s: &str) -> bool {
    NaiveDateTime::parse_from_str(s, DATETIME_FMT).is_ok()
}

fn check_title(t: &str) -> AppResult<()> {
    let n = t.trim().chars().count();
    if n == 0 {
        return Err(AppError::Invalid("任务标题不能为空".into()));
    }
    if n > 200 {
        return Err(AppError::Invalid("任务标题太长了（最多 200 字）".into()));
    }
    Ok(())
}

fn check_opt(
    field: &str,
    v: &Option<String>,
    check: fn(&str) -> bool,
    hint: &str,
) -> AppResult<()> {
    match v.as_deref() {
        Some(s) if !s.is_empty() && !check(s) => Err(AppError::Invalid(format!(
            "{field}格式不正确：{s}（应为 {hint}）"
        ))),
        _ => Ok(()),
    }
}

fn short_date(today: &str, d: &str) -> String {
    let (Ok(t), Ok(x)) = (
        NaiveDate::parse_from_str(today, DATE_FMT),
        NaiveDate::parse_from_str(d, DATE_FMT),
    ) else {
        return d.to_string();
    };
    match (x - t).num_days() {
        0 => "今天".into(),
        1 => "明天".into(),
        2 => "后天".into(),
        _ => format!("{} 月 {} 日", x.format("%-m"), x.format("%-d")),
    }
}

fn item(icon: &str, text: String, danger: bool) -> PreviewItem {
    PreviewItem {
        icon: icon.into(),
        text,
        danger,
    }
}

impl Service<'_> {
    fn task_info(&self, id: i64) -> AppResult<Option<TaskInfo>> {
        Ok(self
            .conn
            .query_row(
                "SELECT title, version, status, done_date FROM tasks WHERE id = ?1",
                [id],
                |r| {
                    Ok(TaskInfo {
                        title: r.get(0)?,
                        version: r.get(1)?,
                        status: r.get(2)?,
                        done_date: r.get(3)?,
                    })
                },
            )
            .optional()?)
    }

    /// 校验整份计划，生成预览与版本号。任何一步不合法，整份计划作废
    pub fn validate_plan(&self, plan: &[Change]) -> AppResult<Validated> {
        let today = self.today().format(DATE_FMT).to_string();
        let mut pins: Vec<Pin> = Vec::new();
        let mut preview = Vec::new();
        let mut touched: HashSet<i64> = HashSet::new();
        let mut deleted: HashSet<i64> = HashSet::new();
        let mut temp_titles: Vec<String> = Vec::new();
        let mut dangerous = false;
        let mut projects = ProjectValidation::new(self)?;
        let mut recurring = RecurValidation::new(self)?;

        // 解析引用，返回任务标题；已有任务记录版本号
        let mut resolve = |r: &TaskRef,
                           temp_titles: &[String],
                           deleted: &HashSet<i64>|
         -> AppResult<(Option<i64>, String, Option<TaskInfo>)> {
            match r {
                TaskRef::Temp(t) => {
                    let n: usize = t
                        .trim_start_matches('$')
                        .parse()
                        .map_err(|_| AppError::Invalid(format!("无法识别的任务引用：{t}")))?;
                    let title = temp_titles
                        .get(n.wrapping_sub(1))
                        .ok_or_else(|| AppError::Invalid(format!("引用了不存在的新建任务：{t}")))?;
                    Ok((None, title.clone(), None))
                }
                TaskRef::Id(id) => {
                    if deleted.contains(id) {
                        return Err(AppError::Invalid(format!(
                            "任务 #{id} 在这次操作中已被删除"
                        )));
                    }
                    let info = self.task_info(*id)?.ok_or_else(|| {
                        AppError::Invalid(format!("任务 #{id} 不存在，请先查询再操作"))
                    })?;
                    if !pins.iter().any(|p| p.id == *id) {
                        pins.push(Pin {
                            entity: "tasks".into(),
                            id: *id,
                            version: info.version,
                        });
                    }
                    Ok((Some(*id), info.title.clone(), Some(info)))
                }
            }
        };

        for c in plan {
            match c {
                Change::Recur { change } => recurring.change(self, change)?,
                Change::Project { change } => projects.change(self, change)?,
                Change::TaskAdd {
                    title,
                    planned_date,
                    due_at,
                    remind_at,
                    ..
                } => {
                    check_title(title)?;
                    if !valid_date(planned_date) {
                        return Err(AppError::Invalid(format!(
                            "计划日期格式不正确：{planned_date}"
                        )));
                    }
                    check_opt("截止时间", due_at, valid_datetime, "YYYY-MM-DDTHH:MM:SS")?;
                    check_opt("提醒时间", remind_at, valid_datetime, "YYYY-MM-DDTHH:MM:SS")?;
                    let mut text = format!("添加「{}」", title.trim());
                    let mut extra = Vec::new();
                    if planned_date != &today {
                        extra.push(short_date(&today, planned_date));
                    }
                    if let Some(r) = remind_at.as_deref().filter(|s| !s.is_empty()) {
                        extra.push(format!("{} 提醒", &r[11..16]));
                    }
                    if !extra.is_empty() {
                        text.push_str(&format!("（{}）", extra.join("，")));
                    }
                    preview.push(item("plus", text, false));
                    temp_titles.push(title.trim().to_string());
                }
                Change::TaskUpdate {
                    task,
                    title,
                    planned_date,
                    due_at,
                    remind_at,
                    ..
                } => {
                    let (id, name, _) = resolve(task, &temp_titles, &deleted)?;
                    if let Some(t) = title {
                        check_title(t)?;
                    }
                    if let Some(d) = planned_date
                        && !valid_date(d)
                    {
                        return Err(AppError::Invalid(format!("计划日期格式不正确：{d}")));
                    }
                    check_opt("截止时间", due_at, valid_datetime, "YYYY-MM-DDTHH:MM:SS")?;
                    check_opt("提醒时间", remind_at, valid_datetime, "YYYY-MM-DDTHH:MM:SS")?;
                    let mut parts = Vec::new();
                    if let Some(t) = title {
                        parts.push(format!("改名为「{}」", t.trim()));
                    }
                    if let Some(d) = planned_date {
                        parts.push(format!("改到{}", short_date(&today, d)));
                    }
                    if let Some(r) = remind_at {
                        parts.push(if r.is_empty() {
                            "取消提醒".into()
                        } else {
                            format!("{} 提醒", &r[11..16])
                        });
                    }
                    if let Some(d) = due_at {
                        parts.push(if d.is_empty() {
                            "去掉截止时间".into()
                        } else {
                            format!("截止{}", short_date(&today, &d[..10]))
                        });
                    }
                    if parts.is_empty() {
                        parts.push("修改备注".into());
                    }
                    preview.push(item(
                        "edit",
                        format!("「{name}」{}", parts.join("，")),
                        false,
                    ));
                    if let Some(id) = id {
                        touched.insert(id);
                    }
                }
                Change::TaskDelete { task } => {
                    let (id, name, _) = resolve(task, &temp_titles, &deleted)?;
                    preview.push(item("trash", format!("删除「{name}」"), true));
                    dangerous = true;
                    if let Some(id) = id {
                        touched.insert(id);
                        deleted.insert(id);
                    }
                }
                Change::TaskSetDone { task, done } => {
                    let (id, name, info) = resolve(task, &temp_titles, &deleted)?;
                    if let Some(info) = &info {
                        let is_done = info.status == "done";
                        if is_done == *done {
                            continue; // 已经是目标状态，不算改动
                        }
                        // 取消往日的完成会改变历史统计，需要确认（4.5）
                        if !done && info.done_date.as_deref().is_some_and(|d| d != today) {
                            dangerous = true;
                            preview.push(item(
                                "undo",
                                format!(
                                    "把 {} 完成的「{name}」改回未完成",
                                    info.done_date.clone().unwrap_or_default()
                                ),
                                true,
                            ));
                        } else {
                            preview.push(item(
                                "check",
                                format!("{}「{name}」", if *done { "完成" } else { "取消完成" }),
                                false,
                            ));
                        }
                    } else {
                        preview.push(item("check", format!("完成「{name}」"), false));
                    }
                    if let Some(id) = id {
                        touched.insert(id);
                        projects.task_completion(self, id, *done)?;
                    }
                }
                Change::Undo => {
                    let summary = self
                        .undo_preview()?
                        .ok_or_else(|| AppError::Invalid("没有可以撤销的操作了".into()))?;
                    preview.push(item("undo", format!("撤销：{summary}"), false));
                }
            }
        }
        for p in &pins {
            projects.task_link(self, p.id)?;
        }
        touched.extend(projects.touched_tasks.iter().copied());
        touched.extend(recurring.touched_tasks.iter().copied());
        if touched.len() + projects.touched.len() + recurring.touched.len() > CONFIRM_THRESHOLD {
            dangerous = true;
        }
        dangerous |= projects.dangerous;
        dangerous |= recurring.dangerous;
        preview.extend(projects.preview);
        preview.extend(recurring.preview);
        for p in projects.pins.into_iter().chain(recurring.pins) {
            if !pins
                .iter()
                .any(|pin| pin.entity == p.entity && pin.id == p.id)
            {
                pins.push(p);
            }
        }
        Ok(Validated {
            pins,
            preview,
            dangerous,
        })
    }

    /// 原子提交：同一 group 只提交一次；版本号不一致时返回 Conflict，什么都不写
    pub fn apply_plan(
        &self,
        plan: &[Change],
        pins: &[Pin],
        group: &str,
        source: Source,
    ) -> AppResult<Applied> {
        let tx = self.conn.unchecked_transaction()?;
        let done_before: Option<i64> = tx
            .query_row(
                "SELECT 1 FROM op_log WHERE group_id = ?1 LIMIT 1",
                [group],
                |r| r.get(0),
            )
            .optional()?;
        if done_before.is_some() {
            return Ok(Applied {
                changes: ChangeSet::default(),
                already: true,
            });
        }
        for p in pins {
            let table = match p.entity.as_str() {
                "tasks" => "tasks",
                "nodes" => "nodes",
                "recur_rules" => "recur_rules",
                _ => return Err(AppError::Invalid("未知的版本校验对象".into())),
            };
            let v: Option<i64> = tx
                .query_row(
                    &format!("SELECT version FROM {table} WHERE id = ?1"),
                    [p.id],
                    |r| r.get(0),
                )
                .optional()?;
            if v != Some(p.version) {
                return Err(AppError::Conflict(
                    "相关的任务刚刚有变化，请重新确认".into(),
                ));
            }
        }

        let today = self.today().format(DATE_FMT).to_string();
        let mut w = Writer::new(&tx, group.to_string(), source, self.now_str(), today);
        let mut temp: HashMap<usize, i64> = HashMap::new();
        let mut node_temp: HashMap<usize, i64> = HashMap::new();
        let resolve = |r: &TaskRef, temp: &HashMap<usize, i64>| -> AppResult<i64> {
            match r {
                TaskRef::Id(id) => Ok(*id),
                TaskRef::Temp(t) => t
                    .trim_start_matches('$')
                    .parse::<usize>()
                    .ok()
                    .and_then(|n| temp.get(&n).copied())
                    .ok_or_else(|| AppError::Invalid(format!("无法识别的任务引用：{t}"))),
            }
        };
        let nonempty =
            |s: &Option<String>| s.as_deref().filter(|x| !x.is_empty()).map(str::to_string);

        for c in plan {
            match c {
                Change::Recur { change } => w.recur_change(change)?,
                Change::Project { change } => w.project_change(change, &mut node_temp)?,
                Change::TaskAdd {
                    title,
                    planned_date,
                    due_at,
                    remind_at,
                    note,
                } => {
                    let id = w.add_task(
                        title.trim(),
                        planned_date,
                        nonempty(due_at).as_deref(),
                        nonempty(remind_at).as_deref(),
                        nonempty(note).as_deref(),
                    )?;
                    temp.insert(temp.len() + 1, id);
                }
                Change::TaskUpdate {
                    task,
                    title,
                    planned_date,
                    due_at,
                    remind_at,
                    note,
                } => {
                    let id = resolve(task, &temp)?;
                    let name = w.task_title(id)?;
                    let mut fields: Vec<(&'static str, Option<String>)> = Vec::new();
                    if let Some(t) = title {
                        fields.push(("title", Some(t.trim().to_string())));
                    }
                    if let Some(d) = planned_date {
                        fields.push(("planned_date", Some(d.clone())));
                    }
                    for (col, v) in [("due_at", due_at), ("remind_at", remind_at), ("note", note)] {
                        if let Some(v) = v {
                            fields.push((col, (!v.is_empty()).then(|| v.clone())));
                        }
                    }
                    w.update_task(id, &fields, format!("修改「{name}」"))?;
                }
                Change::TaskDelete { task } => {
                    let id = resolve(task, &temp)?;
                    w.delete_task(id)?;
                }
                Change::TaskSetDone { task, done } => {
                    let id = resolve(task, &temp)?;
                    w.set_task_done(id, *done, |n| self.project_title_of(n))?;
                }
                Change::Undo => {
                    self.undo_in(&mut w)?;
                }
            }
        }
        let changes = w.changes;
        tx.commit()?;
        Ok(Applied {
            changes,
            already: false,
        })
    }

    /// 这个请求是否已经提交过
    pub fn has_group(&self, group: &str) -> AppResult<bool> {
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM op_log WHERE group_id = ?1 LIMIT 1",
                [group],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::*;
    use super::*;
    use crate::config::AppConfig;
    use rusqlite::{Connection, params};

    const NOW: &str = "2026-10-17T09:00:00";

    fn add(conn: &Connection, title: &str, planned: &str) -> i64 {
        conn.execute(
            "INSERT INTO tasks (title, planned_date, created_at, updated_at) VALUES (?1, ?2, ?3, ?3)",
            params![title, planned, NOW],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn task_add(title: &str, date: &str) -> Change {
        Change::TaskAdd {
            title: title.into(),
            planned_date: date.into(),
            due_at: None,
            remind_at: None,
            note: None,
        }
    }

    #[test]
    fn 新增并引用刚建的任务() {
        let conn = db();
        let clock = clock("2026-10-17 10:00:00");
        let svc = Service::new(&conn, &clock, &AppConfig::default()).unwrap();
        let plan = vec![
            task_add("买菜", "2026-10-17"),
            Change::TaskSetDone {
                task: TaskRef::Temp("$1".into()),
                done: true,
            },
        ];
        let v = svc.validate_plan(&plan).unwrap();
        assert!(!v.dangerous);
        assert_eq!(v.preview[0].text, "添加「买菜」");
        let a = svc.apply_plan(&plan, &v.pins, "g1", Source::Chat).unwrap();
        assert_eq!(a.changes.tasks.len(), 1);
        assert_eq!(
            count(&conn, "SELECT COUNT(*) FROM tasks WHERE status = 'done'"),
            1
        );
        // 同一请求再次提交不会重复执行
        let again = svc.apply_plan(&plan, &v.pins, "g1", Source::Chat).unwrap();
        assert!(again.already);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM tasks"), 1);
    }

    #[test]
    fn 不存在的编号使整份计划作废() {
        let conn = db();
        let clock = clock("2026-10-17 10:00:00");
        let svc = Service::new(&conn, &clock, &AppConfig::default()).unwrap();
        let plan = vec![
            task_add("买菜", "2026-10-17"),
            Change::TaskDelete {
                task: TaskRef::Id(99),
            },
        ];
        assert!(svc.validate_plan(&plan).is_err());
        let bad_date = vec![task_add("买菜", "明天")];
        assert!(svc.validate_plan(&bad_date).is_err());
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM tasks"), 0);
    }

    #[test]
    fn 删除和批量修改需要确认() {
        let conn = db();
        let clock = clock("2026-10-17 10:00:00");
        let svc = Service::new(&conn, &clock, &AppConfig::default()).unwrap();
        let ids: Vec<i64> = (0..4)
            .map(|i| add(&conn, &format!("事{i}"), "2026-10-17"))
            .collect();
        let del = vec![Change::TaskDelete {
            task: TaskRef::Id(ids[0]),
        }];
        assert!(svc.validate_plan(&del).unwrap().dangerous);

        let three: Vec<Change> = ids[..3]
            .iter()
            .map(|id| Change::TaskUpdate {
                task: TaskRef::Id(*id),
                title: None,
                planned_date: Some("2026-10-18".into()),
                due_at: None,
                remind_at: None,
                note: None,
            })
            .collect();
        assert!(!svc.validate_plan(&three).unwrap().dangerous);
        let mut four = three.clone();
        four.push(Change::TaskSetDone {
            task: TaskRef::Id(ids[3]),
            done: true,
        });
        assert!(svc.validate_plan(&four).unwrap().dangerous);
    }

    #[test]
    fn 版本变化时拒绝提交且不写数据() {
        let conn = db();
        let clock = clock("2026-10-17 10:00:00");
        let svc = Service::new(&conn, &clock, &AppConfig::default()).unwrap();
        let id = add(&conn, "开会", "2026-10-17");
        let plan = vec![Change::TaskDelete {
            task: TaskRef::Id(id),
        }];
        let v = svc.validate_plan(&plan).unwrap();
        // 确认之前，任务被手动勾选了
        svc.set_task_done(id, 1, true).unwrap();
        let err = svc
            .apply_plan(&plan, &v.pins, "g", Source::Chat)
            .unwrap_err();
        assert!(matches!(err, AppError::Conflict(_)));
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM tasks"), 1);
    }

    #[test]
    fn 中途失败整体回滚() {
        let conn = db();
        let clock = clock("2026-10-17 10:00:00");
        let svc = Service::new(&conn, &clock, &AppConfig::default()).unwrap();
        // 跳过校验直接提交：第二步引用不存在的任务，第一步也不能留下
        let plan = vec![
            task_add("买菜", "2026-10-17"),
            Change::TaskSetDone {
                task: TaskRef::Id(42),
                done: true,
            },
        ];
        assert!(svc.apply_plan(&plan, &[], "g", Source::Chat).is_err());
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM tasks"), 0);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM op_log"), 0);
    }

    #[test]
    fn 修改字段与清空() {
        let conn = db();
        let clock = clock("2026-10-17 10:00:00");
        let svc = Service::new(&conn, &clock, &AppConfig::default()).unwrap();
        let id = add(&conn, "开会", "2026-10-17");
        conn.execute(
            "UPDATE tasks SET remind_at = '2026-10-17T15:00:00' WHERE id = ?1",
            [id],
        )
        .unwrap();
        let plan = vec![Change::TaskUpdate {
            task: TaskRef::Id(id),
            title: Some("产品评审会".into()),
            planned_date: Some("2026-10-18".into()),
            due_at: None,
            remind_at: Some(String::new()),
            note: None,
        }];
        let v = svc.validate_plan(&plan).unwrap();
        assert_eq!(
            v.preview[0].text,
            "「开会」改名为「产品评审会」，改到明天，取消提醒"
        );
        svc.apply_plan(&plan, &v.pins, "g", Source::Chat).unwrap();
        let (t, d, r): (String, String, Option<String>) = conn
            .query_row(
                "SELECT title, planned_date, remind_at FROM tasks WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            (t.as_str(), d.as_str(), r),
            ("产品评审会", "2026-10-18", None)
        );
    }
}
