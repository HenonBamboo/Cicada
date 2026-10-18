//! 撤销（docs/产品设计.md 4.5）。
//!
//! - 只撤销用户操作（对话、手动勾选），按时间倒序一组一组撤销，支持连续撤销；
//! - 系统操作（日切等）和撤销本身不会成为撤销对象；
//! - 按 op_log 快照还原；某个对象之后又被改过（版本号不一致）时拒绝撤销。

use std::collections::HashSet;

use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;

use super::oplog::{self, Entry, Source};
use super::write::Writer;
use super::{ChangeSet, DATE_FMT, Service};
use crate::error::{AppError, AppResult};

/// 允许还原的表
const TABLES: &[&str] = &["tasks", "nodes", "completions", "recur_rules", "stickies"];

struct LogRow {
    entity: String,
    entity_id: i64,
    before: Option<Value>,
    after: Option<Value>,
    summary: String,
    op: String,
}

fn table_name(entity: &str) -> AppResult<&'static str> {
    TABLES
        .iter()
        .copied()
        .find(|t| *t == entity)
        .ok_or_else(|| AppError::Invalid(format!("无法撤销的数据类型：{entity}")))
}

fn columns(conn: &Connection, table: &str) -> AppResult<Vec<String>> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let cols = stmt
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(cols)
}

fn to_sql(v: &Value) -> SqlValue {
    match v {
        Value::Null => SqlValue::Null,
        Value::Bool(b) => SqlValue::Integer(*b as i64),
        Value::Number(n) => n
            .as_i64()
            .map(SqlValue::Integer)
            .or_else(|| n.as_f64().map(SqlValue::Real))
            .unwrap_or(SqlValue::Null),
        Value::String(s) => SqlValue::Text(s.clone()),
        other => SqlValue::Text(other.to_string()),
    }
}

fn same_content(a: &Value, b: &Value, table: &str) -> bool {
    let mut a = a.clone();
    let mut b = b.clone();
    for value in [&mut a, &mut b] {
        if let Some(obj) = value.as_object_mut() {
            obj.remove("version");
            if table == "tasks" {
                for field in ["postponed_days", "reminded_at", "updated_at"] {
                    obj.remove(field);
                }
            } else if table == "recur_rules" {
                for field in ["last_generated_date", "updated_at"] {
                    obj.remove(field);
                }
            }
        }
    }
    a == b
}

/// 把一行恢复成快照内容。版本号只增不减，避免旧的确认请求意外匹配
fn restore_row(
    conn: &Connection,
    table: &str,
    id: i64,
    snapshot: &Value,
    current: Option<&Value>,
) -> AppResult<()> {
    let cols = columns(conn, table)?;
    let obj = snapshot
        .as_object()
        .ok_or_else(|| AppError::Invalid("操作日志快照已损坏".into()))?;
    let has_version = cols.iter().any(|c| c == "version");
    let next_version = || {
        let cur = current.and_then(|c| c["version"].as_i64()).unwrap_or(0);
        let old = obj.get("version").and_then(|v| v.as_i64()).unwrap_or(0);
        cur.max(old) + 1
    };
    let mut names = Vec::new();
    let mut values = Vec::new();
    for c in cols.iter().filter(|c| c.as_str() != "id") {
        if c == "version" {
            continue;
        }
        if let Some(v) = obj.get(c) {
            names.push(c.clone());
            let preserve = (table == "recur_rules" && c == "last_generated_date")
                || (table == "tasks"
                    && c == "reminded_at"
                    && current.is_some_and(|cur| cur.get("remind_at") == obj.get("remind_at")));
            values.push(to_sql(if preserve {
                current.and_then(|cur| cur.get(c)).unwrap_or(v)
            } else {
                v
            }));
        }
    }
    if has_version {
        names.push("version".into());
        values.push(SqlValue::Integer(next_version()));
    }
    if current.is_some() {
        let sets: Vec<String> = names
            .iter()
            .enumerate()
            .map(|(i, n)| format!("{n} = ?{}", i + 2))
            .collect();
        let sql = format!("UPDATE {table} SET {} WHERE id = ?1", sets.join(", "));
        let mut params: Vec<SqlValue> = vec![SqlValue::Integer(id)];
        params.extend(values);
        conn.execute(&sql, rusqlite::params_from_iter(params))?;
    } else {
        let marks: Vec<String> = (0..=names.len()).map(|i| format!("?{}", i + 1)).collect();
        let sql = format!(
            "INSERT INTO {table} (id, {}) VALUES ({})",
            names.join(", "),
            marks.join(", ")
        );
        let mut params: Vec<SqlValue> = vec![SqlValue::Integer(id)];
        params.extend(values);
        conn.execute(&sql, rusqlite::params_from_iter(params))?;
    }
    Ok(())
}

impl Service<'_> {
    /// 最近一组可撤销的用户操作。`exclude` 为当前正在提交的请求
    fn undo_target(&self, conn: &Connection, exclude: Option<&str>) -> AppResult<Option<String>> {
        Ok(conn
            .query_row(
                "SELECT group_id FROM op_log
                 WHERE source IN ('chat', 'manual') AND undone_at IS NULL AND group_id != ?1
                 GROUP BY group_id ORDER BY MAX(id) DESC LIMIT 1",
                [exclude.unwrap_or("")],
                |r| r.get(0),
            )
            .optional()?)
    }

    fn group_summary(conn: &Connection, group: &str) -> AppResult<String> {
        let mut stmt = conn.prepare(
            "SELECT summary FROM op_log WHERE group_id = ?1 AND source IN ('chat', 'manual')
             AND op NOT LIKE 'completion.%' AND op != 'node.touch'
             AND NOT (op = 'node.set_done' AND summary = '同步关联的项目节点') ORDER BY id",
        )?;
        let parts = stmt
            .query_map([group], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(if parts.is_empty() {
            "上一次操作".into()
        } else {
            parts.join("、")
        })
    }

    /// 撤销预览：将要撤销的操作说明；没有可撤销的操作时为 None
    pub fn undo_preview(&self) -> AppResult<Option<String>> {
        match self.undo_target(self.conn, None)? {
            Some(g) => Ok(Some(Self::group_summary(self.conn, &g)?)),
            None => Ok(None),
        }
    }

    /// 在已有事务中撤销最近一组用户操作，返回被撤销的说明
    pub(super) fn undo_in(&self, w: &mut Writer<'_>) -> AppResult<String> {
        let conn = w.conn;
        let group = self
            .undo_target(conn, Some(&w.group))?
            .ok_or_else(|| AppError::Invalid("没有可以撤销的操作了".into()))?;
        let summary = Self::group_summary(conn, &group)?;

        let mut stmt = conn.prepare(
            "SELECT entity, entity_id, before, after, summary, op FROM op_log
             WHERE group_id = ?1 AND source IN ('chat', 'manual') ORDER BY id DESC",
        )?;
        let rows = stmt
            .query_map([&group], |r| {
                let parse =
                    |s: Option<String>| s.and_then(|s| serde_json::from_str::<Value>(&s).ok());
                Ok(LogRow {
                    entity: r.get(0)?,
                    entity_id: r.get::<_, Option<i64>>(1)?.unwrap_or(0),
                    before: parse(r.get(2)?),
                    after: parse(r.get(3)?),
                    summary: r.get(4)?,
                    op: r.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let mut restored = HashSet::new();
        for row in &rows {
            let table = table_name(&row.entity)?;
            let current = oplog::row_json(conn, table, row.entity_id)?;
            // 冲突检测：当前状态必须仍是这组操作留下的样子
            let unchanged = match (&row.after, &current) {
                (Some(a), Some(c)) => {
                    let previous_undo: Option<String> = conn
                        .query_row(
                            "SELECT CASE WHEN source = 'undo' THEN after END FROM op_log
                         WHERE entity = ?1 AND entity_id = ?2 ORDER BY id DESC LIMIT 1",
                            params![table, row.entity_id],
                            |r| r.get(0),
                        )
                        .optional()?
                        .flatten();
                    let restored_version = restored.contains(&(table, row.entity_id))
                        || previous_undo
                            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
                            .as_ref()
                            == Some(c);
                    let only_system_since: bool = conn.query_row(
                        "SELECT EXISTS(SELECT 1 FROM op_log WHERE entity=?1 AND entity_id=?2 AND source='system' AND id>(SELECT MAX(id) FROM op_log WHERE group_id=?3 AND entity=?1 AND entity_id=?2))
                         AND NOT EXISTS(SELECT 1 FROM op_log WHERE entity=?1 AND entity_id=?2 AND source IN ('chat','manual') AND id>(SELECT MAX(id) FROM op_log WHERE group_id=?3 AND entity=?1 AND entity_id=?2))",
                        params![table,row.entity_id,group], |r|r.get(0))?;
                    same_content(a, c, table)
                        && (a.get("version") == c.get("version")
                            || restored_version
                            || only_system_since)
                }
                (None, None) => true,
                _ => false,
            };
            if !unchanged {
                let what = row
                    .before
                    .as_ref()
                    .or(row.after.as_ref())
                    .and_then(|v| v["title"].as_str().map(str::to_string))
                    .unwrap_or_else(|| "相关数据".into());
                return Err(AppError::Invalid(format!(
                    "「{what}」之后又被改动过，不能撤销「{summary}」"
                )));
            }
            match &row.before {
                None => {
                    if table == "recur_rules" {
                        let mut stmt =
                            conn.prepare("SELECT id FROM tasks WHERE recur_rule_id=?1")?;
                        let tasks = stmt
                            .query_map([row.entity_id], |r| r.get::<_, i64>(0))?
                            .collect::<Result<Vec<_>, _>>()?;
                        drop(stmt);
                        for task in tasks {
                            let before = oplog::row_json(conn, "tasks", task)?;
                            conn.execute("UPDATE tasks SET recur_rule_id=NULL,version=version+1,updated_at=?2 WHERE id=?1",params![task,w.now])?;
                            oplog::record(
                                conn,
                                Entry {
                                    group_id: &w.group,
                                    source: Source::Undo,
                                    op: "undo.recur.detach",
                                    entity: "tasks",
                                    entity_id: Some(task),
                                    before,
                                    after: oplog::row_json(conn, "tasks", task)?,
                                    summary: "撤销重复规则，保留系统生成的任务".into(),
                                    created_at: &w.now,
                                },
                            )?;
                            if !w.changes.tasks.contains(&task) {
                                w.changes.tasks.push(task);
                            }
                        }
                    }
                    conn.execute(
                        &format!("DELETE FROM {table} WHERE id = ?1"),
                        [row.entity_id],
                    )?;
                }
                Some(b) => restore_row(conn, table, row.entity_id, b, current.as_ref())?,
            }
            if table == "tasks" && row.before.is_some() {
                conn.execute("UPDATE tasks SET postponed_days=MAX(0,CAST(julianday(?2)-julianday(planned_date) AS INTEGER)) WHERE id=?1 AND status='open'",params![row.entity_id,w.today])?;
            }
            restored.insert((table, row.entity_id));
            let after = oplog::row_json(conn, table, row.entity_id)?;
            oplog::record(
                conn,
                Entry {
                    group_id: &w.group,
                    source: Source::Undo,
                    op: &format!("undo.{}", row.op),
                    entity: table,
                    entity_id: Some(row.entity_id),
                    before: current,
                    after,
                    summary: format!("撤销：{}", row.summary),
                    created_at: &w.now,
                },
            )?;
            match table {
                "tasks" if !w.changes.tasks.contains(&row.entity_id) => {
                    w.changes.tasks.push(row.entity_id)
                }
                "nodes" if !w.changes.nodes.contains(&row.entity_id) => {
                    w.changes.nodes.push(row.entity_id)
                }
                "recur_rules" => {
                    let mut stmt = conn.prepare("SELECT id FROM tasks WHERE recur_rule_id=?1")?;
                    let tasks = stmt
                        .query_map([row.entity_id], |r| r.get::<_, i64>(0))?
                        .collect::<Result<Vec<_>, _>>()?;
                    drop(stmt);
                    for task in tasks {
                        w.touch_task(task);
                    }
                }
                _ => {}
            }
        }
        conn.execute(
            "UPDATE op_log SET undone_at = ?2 WHERE group_id = ?1 AND source IN ('chat', 'manual')",
            params![group, w.now],
        )?;
        Ok(summary)
    }

    /// 撤销按钮：不经过模型，直接撤销最近一组用户操作
    pub fn undo_last(&self) -> AppResult<(String, ChangeSet)> {
        let tx = self.conn.unchecked_transaction()?;
        let today = self.today().format(DATE_FMT).to_string();
        let mut w = Writer::new(
            &tx,
            oplog::new_group_id(),
            Source::Undo,
            self.now_str(),
            today,
        );
        let summary = self.undo_in(&mut w)?;
        let changes = w.changes;
        tx.commit()?;
        Ok((summary, changes))
    }
}

#[cfg(test)]
mod tests {
    use super::super::changes::{Change, TaskRef};
    use super::super::testing::*;
    use super::*;
    use crate::config::AppConfig;

    const NOW: &str = "2026-10-17T09:00:00";

    fn add_plan(title: &str) -> Vec<Change> {
        vec![Change::TaskAdd {
            title: title.into(),
            planned_date: "2026-10-17".into(),
            due_at: None,
            remind_at: None,
            note: None,
        }]
    }

    fn apply(svc: &Service<'_>, plan: &[Change], group: &str) {
        let v = svc.validate_plan(plan).unwrap();
        svc.apply_plan(plan, &v.pins, group, Source::Chat).unwrap();
    }

    #[test]
    fn 连续撤销按时间倒序() {
        let conn = db();
        let clock = clock("2026-10-17 10:00:00");
        let svc = Service::new(&conn, &clock, &AppConfig::default()).unwrap();
        apply(&svc, &add_plan("买菜"), "g1");
        apply(&svc, &add_plan("跑步"), "g2");
        assert_eq!(svc.undo_preview().unwrap().unwrap(), "添加「跑步」");

        let (s, ch) = svc.undo_last().unwrap();
        assert_eq!(s, "添加「跑步」");
        assert_eq!(ch.tasks.len(), 1);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM tasks"), 1);

        svc.undo_last().unwrap();
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM tasks"), 0);
        assert!(svc.undo_last().is_err()); // 没有可撤销的了
        assert!(svc.undo_preview().unwrap().is_none());
    }

    #[test]
    fn 撤销删除恢复原任务与完成记录() {
        let conn = db();
        let clock = clock("2026-10-17 10:00:00");
        let svc = Service::new(&conn, &clock, &AppConfig::default()).unwrap();
        conn.execute(
            "INSERT INTO tasks (title, planned_date, created_at, updated_at) VALUES ('跑步', '2026-10-17', ?1, ?1)",
            [NOW],
        )
        .unwrap();
        let id = conn.last_insert_rowid();
        svc.set_task_done(id, 1, true).unwrap();
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM completions"), 1);

        // 撤销勾选：任务回到未完成，完成记录消失
        svc.undo_last().unwrap();
        assert_eq!(
            count(&conn, "SELECT COUNT(*) FROM tasks WHERE status = 'open'"),
            1
        );
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM completions"), 0);

        // 撤销删除：原编号、原标题回来，版本号继续递增
        let plan = vec![Change::TaskDelete {
            task: TaskRef::Id(id),
        }];
        apply(&svc, &plan, "del");
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM tasks"), 0);
        svc.undo_last().unwrap();
        let (title, version): (String, i64) = conn
            .query_row(
                "SELECT title, version FROM tasks WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(title, "跑步");
        assert!(version > 3);
    }

    #[test]
    fn 之后又被改过时拒绝撤销() {
        let conn = db();
        let clock = clock("2026-10-17 10:00:00");
        let svc = Service::new(&conn, &clock, &AppConfig::default()).unwrap();
        apply(&svc, &add_plan("买菜"), "g1");
        let id: i64 = conn
            .query_row("SELECT id FROM tasks", [], |r| r.get(0))
            .unwrap();
        // 系统操作改了它（模拟日切）：不会成为撤销对象，但会让撤销失去前提
        conn.execute(
            "UPDATE tasks SET version = version + 1, title = '买菜（系统改过）' WHERE id = ?1",
            [id],
        )
        .unwrap();
        let err = svc.undo_last().unwrap_err().to_string();
        assert!(err.contains("改动过"), "{err}");
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM tasks"), 1);
    }

    #[test]
    fn 系统操作和撤销本身不会被撤销() {
        let conn = db();
        let clock = clock("2026-10-17 10:00:00");
        let svc = Service::new(&conn, &clock, &AppConfig::default()).unwrap();
        apply(&svc, &add_plan("买菜"), "g1");
        conn.execute(
            "INSERT INTO op_log (group_id, source, op, entity, summary, created_at) VALUES ('sys', 'system', 'day_cut', 'tasks', '日切', ?1)",
            [NOW],
        )
        .unwrap();
        // 最近的是系统操作，但撤销目标仍是用户的"添加「买菜」"
        assert_eq!(svc.undo_preview().unwrap().unwrap(), "添加「买菜」");
        svc.undo_last().unwrap();
        // 撤销留下的 undo 记录不会被再次撤销
        assert!(svc.undo_preview().unwrap().is_none());
    }

    #[test]
    fn 对话里的撤销作为计划的一步() {
        let conn = db();
        let clock = clock("2026-10-17 10:00:00");
        let svc = Service::new(&conn, &clock, &AppConfig::default()).unwrap();
        apply(&svc, &add_plan("买菜"), "g1");
        let plan = vec![Change::Undo];
        let v = svc.validate_plan(&plan).unwrap();
        assert_eq!(v.preview[0].text, "撤销：添加「买菜」");
        svc.apply_plan(&plan, &v.pins, "g2", Source::Chat).unwrap();
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM tasks"), 0);
    }
}
