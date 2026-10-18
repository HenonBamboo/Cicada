//! 事务内的写操作。每一次行变化（包括完成记录）都写入 op_log，撤销时据此还原。
//! 手动勾选、对话的变更计划、撤销都通过这里写数据。

use rusqlite::{Connection, OptionalExtension, params};

use super::ChangeSet;
use super::oplog::{self, Entry, Source};
use crate::error::{AppError, AppResult};

pub(super) struct Writer<'t> {
    pub conn: &'t Connection,
    pub group: String,
    pub source: Source,
    pub now: String,
    pub today: String,
    pub changes: ChangeSet,
}

impl<'t> Writer<'t> {
    pub fn new(
        conn: &'t Connection,
        group: String,
        source: Source,
        now: String,
        today: String,
    ) -> Self {
        Self {
            conn,
            group,
            source,
            now,
            today,
            changes: ChangeSet::default(),
        }
    }

    pub(super) fn log(
        &self,
        op: &str,
        entity: &'static str,
        id: i64,
        before: Option<serde_json::Value>,
        summary: String,
    ) -> AppResult<()> {
        let after = oplog::row_json(self.conn, entity, id)?;
        oplog::record(
            self.conn,
            Entry {
                group_id: &self.group,
                source: self.source,
                op,
                entity,
                entity_id: Some(id),
                before,
                after,
                summary,
                created_at: &self.now,
            },
        )
    }

    pub(super) fn touch_task(&mut self, id: i64) {
        if !self.changes.tasks.contains(&id) {
            self.changes.tasks.push(id);
        }
    }

    pub fn task_title(&self, id: i64) -> AppResult<String> {
        self.conn
            .query_row("SELECT title FROM tasks WHERE id = ?1", [id], |r| r.get(0))
            .optional()?
            .ok_or_else(|| AppError::Invalid(format!("任务 #{id} 不存在")))
    }

    pub fn add_task(
        &mut self,
        title: &str,
        planned: &str,
        due_at: Option<&str>,
        remind_at: Option<&str>,
        note: Option<&str>,
    ) -> AppResult<i64> {
        self.conn.execute(
            "INSERT INTO tasks (title, planned_date, due_at, remind_at, note, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
            params![title, planned, due_at, remind_at, note, self.now],
        )?;
        let id = self.conn.last_insert_rowid();
        self.log("task.add", "tasks", id, None, format!("添加「{title}」"))?;
        self.touch_task(id);
        Ok(id)
    }

    /// 修改任务字段。`fields` 为 (列名, 新值)，列名只能是内部常量
    pub fn update_task(
        &mut self,
        id: i64,
        fields: &[(&'static str, Option<String>)],
        summary: String,
    ) -> AppResult<()> {
        if fields.is_empty() {
            return Ok(());
        }
        let before = oplog::row_json(self.conn, "tasks", id)?;
        let mut fields = fields.to_vec();
        if fields.iter().any(|(key, value)| {
            *key == "remind_at"
                && value.as_deref() != before.as_ref().and_then(|row| row["remind_at"].as_str())
        }) {
            fields.push(("reminded_at", None));
        }
        let sets: Vec<String> = fields
            .iter()
            .enumerate()
            .map(|(i, (col, _))| format!("{col} = ?{}", i + 3))
            .collect();
        let sql = format!(
            "UPDATE tasks SET {}, updated_at = ?2, version = version + 1 WHERE id = ?1",
            sets.join(", ")
        );
        let mut values: Vec<&dyn rusqlite::ToSql> = vec![&id, &self.now];
        for (_, v) in &fields {
            values.push(v);
        }
        self.conn.execute(&sql, values.as_slice())?;
        self.log("task.update", "tasks", id, before, summary)?;
        self.touch_task(id);
        Ok(())
    }

    pub fn delete_task(&mut self, id: i64) -> AppResult<()> {
        let title = self.task_title(id)?;
        let before = oplog::row_json(self.conn, "tasks", id)?;
        let node = before.as_ref().and_then(|v| v["node_id"].as_i64());
        self.conn.execute("DELETE FROM tasks WHERE id = ?1", [id])?;
        oplog::record(
            self.conn,
            Entry {
                group_id: &self.group,
                source: self.source,
                op: "task.delete",
                entity: "tasks",
                entity_id: Some(id),
                before,
                after: None,
                summary: format!("删除「{title}」"),
                created_at: &self.now,
            },
        )?;
        self.touch_task(id);
        if let Some(node) = node {
            self.touch_node_ancestors(node)?;
        }
        Ok(())
    }

    /// 标记完成 / 未完成。关联节点同步改变，完成只记一条 completions（docs/产品设计.md 7.4）
    pub fn set_task_done(
        &mut self,
        id: i64,
        done: bool,
        project_of: impl Fn(i64) -> AppResult<Option<String>>,
    ) -> AppResult<()> {
        let (status, node_id, title): (String, Option<i64>, String) = self
            .conn
            .query_row(
                "SELECT status, node_id, title FROM tasks WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?
            .ok_or_else(|| AppError::Invalid(format!("任务 #{id} 不存在")))?;
        if (status == "done") == done {
            return Ok(());
        }

        let before = oplog::row_json(self.conn, "tasks", id)?;
        if done {
            self.conn.execute(
                "UPDATE tasks SET status = 'done', done_at = ?2, done_date = ?3, updated_at = ?2,
                 version = version + 1 WHERE id = ?1",
                params![id, self.now, self.today],
            )?;
        } else {
            self.conn.execute(
                "UPDATE tasks SET status = 'open', done_at = NULL, done_date = NULL, updated_at = ?2,
                 version = version + 1 WHERE id = ?1",
                params![id, self.now],
            )?;
        }
        let verb = if done { "完成" } else { "取消完成" };
        self.log(
            "task.set_done",
            "tasks",
            id,
            before,
            format!("{verb}「{title}」"),
        )?;
        self.touch_task(id);

        // 关联节点同步
        let node = match node_id {
            Some(n) => self
                .conn
                .query_row("SELECT status FROM nodes WHERE id = ?1", [n], |r| {
                    r.get::<_, String>(0)
                })
                .optional()?
                .map(|s| (n, s)),
            None => None,
        };
        if let Some((nid, node_status)) = &node
            && (node_status == "done") != done
        {
            let before = oplog::row_json(self.conn, "nodes", *nid)?;
            self.conn.execute(
                "UPDATE nodes SET status = ?2, done_at = ?3, updated_at = ?4, version = version + 1,
                 project_status = CASE WHEN depth=1 THEN CASE WHEN ?2='done' THEN 'done' ELSE 'active' END ELSE NULL END WHERE id = ?1",
                params![nid, if done { "done" } else { "open" }, done.then_some(&self.now), self.now],
            )?;
            self.log(
                "node.set_done",
                "nodes",
                *nid,
                before,
                "同步关联的项目节点".into(),
            )?;
            self.changes.nodes.push(*nid);
        }
        if let Some((nid, _)) = &node {
            self.touch_node_parents(*nid)?;
        }

        // 完成记录：一件事只记一条；取消完成时删除（都记入 op_log 以便撤销）
        if done {
            let (kind, node_ref, project) = match &node {
                Some((nid, _)) => ("node", Some(*nid), project_of(*nid)?),
                None => ("task", None, None),
            };
            self.conn.execute(
                "INSERT INTO completions (business_date, kind, task_id, node_id, title, project_title, completed_at, op_group)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![self.today, kind, id, node_ref, title, project, self.now, self.group],
            )?;
            let cid = self.conn.last_insert_rowid();
            self.log(
                "completion.add",
                "completions",
                cid,
                None,
                "记录完成".into(),
            )?;
        } else {
            // 取消的是最近一次完成记录（可能在今天，也可能是确认过的往日记录）
            let cid: Option<i64> = self
                .conn
                .query_row(
                    "SELECT id FROM completions WHERE task_id = ?1 ORDER BY business_date DESC, id DESC LIMIT 1",
                    [id],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(cid) = cid {
                let before = oplog::row_json(self.conn, "completions", cid)?;
                self.conn
                    .execute("DELETE FROM completions WHERE id = ?1", [cid])?;
                oplog::record(
                    self.conn,
                    Entry {
                        group_id: &self.group,
                        source: self.source,
                        op: "completion.delete",
                        entity: "completions",
                        entity_id: Some(cid),
                        before,
                        after: None,
                        summary: "删除完成记录".into(),
                        created_at: &self.now,
                    },
                )?;
            }
        }
        Ok(())
    }
}
