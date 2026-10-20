//! 项目事务写入；所有显式与联动变化均保存可撤销快照。
use std::collections::HashMap;

use rusqlite::{OptionalExtension, params};

use super::changes::TaskRef;
use super::oplog::{self, Entry};
use super::project_changes::ProjectChange;
use super::write::Writer;
use crate::error::{AppError, AppResult};

fn resolve(node: &TaskRef, temp: &HashMap<usize, i64>) -> AppResult<i64> {
    match node {
        TaskRef::Id(id) => Ok(*id),
        TaskRef::Temp(t) => t
            .strip_prefix('$')
            .and_then(|v| v.parse().ok())
            .and_then(|n| temp.get(&n).copied())
            .ok_or_else(|| AppError::Invalid("新建节点引用无效".into())),
    }
}

impl Writer<'_> {
    fn unique_node_title(
        &self,
        parent: Option<i64>,
        title: &str,
        except: Option<i64>,
    ) -> AppResult<()> {
        let mut stmt = self
            .conn
            .prepare("SELECT id,title FROM nodes WHERE parent_id IS ?1")?;
        let target = super::project_changes::normalized_title(title);
        for row in stmt.query_map([parent], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })? {
            let (id, existing) = row?;
            if Some(id) != except && super::project_changes::normalized_title(&existing) == target {
                return Err(AppError::Invalid(
                    "同一位置已经有同名项目或节点，未重复添加".into(),
                ));
            }
        }
        Ok(())
    }
    fn changed_node(&mut self, id: i64) {
        if !self.changes.nodes.contains(&id) {
            self.changes.nodes.push(id);
        }
    }

    pub(super) fn touch_node_ancestors(&mut self, id: i64) -> AppResult<()> {
        let mut node = Some(id);
        while let Some(id) = node {
            let before = oplog::row_json(self.conn, "nodes", id)?;
            let Some(before) = before else { break };
            node = before["parent_id"].as_i64();
            self.conn.execute(
                "UPDATE nodes SET version=version+1,updated_at=?2 WHERE id=?1",
                params![id, self.now],
            )?;
            self.log(
                "node.touch",
                "nodes",
                id,
                Some(before),
                "更新项目关联和进度".into(),
            )?;
            self.changed_node(id);
        }
        Ok(())
    }

    pub(super) fn touch_node_parents(&mut self, id: i64) -> AppResult<()> {
        let parent = self
            .conn
            .query_row("SELECT parent_id FROM nodes WHERE id=?1", [id], |r| {
                r.get(0)
            })?;
        if let Some(parent) = parent {
            self.touch_node_ancestors(parent)?;
        }
        Ok(())
    }

    fn project_name(&self, id: i64) -> AppResult<Option<String>> {
        Ok(self.conn.query_row("WITH RECURSIVE up(id,parent_id,title) AS (SELECT id,parent_id,title FROM nodes WHERE id=?1 UNION ALL SELECT n.id,n.parent_id,n.title FROM nodes n JOIN up ON n.id=up.parent_id) SELECT title FROM up WHERE parent_id IS NULL",[id],|r|r.get(0)).optional()?)
    }

    fn node_name(&self, id: i64) -> AppResult<String> {
        self.conn
            .query_row("SELECT title FROM nodes WHERE id=?1", [id], |r| r.get(0))
            .optional()?
            .ok_or_else(|| AppError::Invalid(format!("节点 #{id} 不存在")))
    }

    fn descendants(&self, id: i64) -> AppResult<Vec<(i64, i64)>> {
        let mut stmt=self.conn.prepare("WITH RECURSIVE tree(id,depth) AS (SELECT id,depth FROM nodes WHERE id=?1 UNION ALL SELECT n.id,n.depth FROM nodes n JOIN tree ON n.parent_id=tree.id) SELECT id,depth FROM tree ORDER BY depth DESC,id")?;
        Ok(stmt
            .query_map([id], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<Vec<_>, _>>()?)
    }

    fn remove_logged(
        &mut self,
        entity: &'static str,
        id: i64,
        op: &str,
        summary: String,
    ) -> AppResult<()> {
        let before = oplog::row_json(self.conn, entity, id)?;
        self.conn
            .execute(&format!("DELETE FROM {entity} WHERE id=?1"), [id])?;
        oplog::record(
            self.conn,
            Entry {
                group_id: &self.group,
                source: self.source,
                op,
                entity,
                entity_id: Some(id),
                before,
                after: None,
                summary,
                created_at: &self.now,
            },
        )?;
        Ok(())
    }

    fn set_node_done(&mut self, id: i64, done: bool) -> AppResult<()> {
        let before = oplog::row_json(self.conn, "nodes", id)?
            .ok_or_else(|| AppError::Invalid("节点不存在".into()))?;
        if (before["status"] == "done") == done {
            return Ok(());
        }
        let linked: Option<i64> = if done {
            self.conn
                .query_row(
                    "SELECT id FROM tasks WHERE node_id=?1 AND status='open' LIMIT 1",
                    [id],
                    |r| r.get(0),
                )
                .optional()?
        } else {
            self.conn.query_row("SELECT t.id FROM tasks t JOIN completions c ON c.task_id=t.id AND c.node_id=?1 WHERE t.node_id=?1 AND t.status='done' ORDER BY c.id DESC LIMIT 1",[id],|r|r.get(0)).optional()?
        };
        if let Some(task) = linked {
            let project = self.project_name(id)?;
            self.set_task_done(task, done, |_| Ok(project.clone()))?;
            return Ok(());
        }
        self.conn.execute("UPDATE nodes SET status=?2,done_at=?3,project_status=CASE WHEN depth=1 THEN ?4 ELSE NULL END,version=version+1,updated_at=?5 WHERE id=?1",params![id,if done{"done"}else{"open"},done.then_some(&self.now),if done{"done"}else{"active"},self.now])?;
        let title = before["title"].as_str().unwrap_or_default().to_string();
        self.log(
            "node.set_done",
            "nodes",
            id,
            Some(before),
            format!("{}「{title}」", if done { "完成" } else { "取消完成" }),
        )?;
        self.changed_node(id);
        if done {
            let project = self.project_name(id)?;
            self.conn.execute("INSERT INTO completions(business_date,kind,node_id,title,project_title,completed_at,op_group) VALUES(?1,'node',?2,?3,?4,?5,?6)",params![self.today,id,title,project,self.now,self.group])?;
            self.log(
                "completion.add",
                "completions",
                self.conn.last_insert_rowid(),
                None,
                "记录完成".into(),
            )?;
        } else {
            let completion: Option<i64> = self
                .conn
                .query_row(
                    "SELECT id FROM completions WHERE node_id=?1 ORDER BY id DESC LIMIT 1",
                    [id],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(cid) = completion {
                self.remove_logged(
                    "completions",
                    cid,
                    "completion.delete",
                    "删除完成记录".into(),
                )?;
            }
        }
        self.touch_node_parents(id)?;
        Ok(())
    }

    pub(super) fn project_change(
        &mut self,
        c: &ProjectChange,
        temp: &mut HashMap<usize, i64>,
    ) -> AppResult<()> {
        match c {
            ProjectChange::Add {
                title,
                note,
                due_at,
            }
            | ProjectChange::NodeAdd {
                title,
                note,
                due_at,
                ..
            } => {
                let parent = if let ProjectChange::NodeAdd { parent, .. } = c {
                    Some(resolve(parent, temp)?)
                } else {
                    None
                };
                let depth = match parent {
                    Some(p) => {
                        self.conn
                            .query_row("SELECT depth+1 FROM nodes WHERE id=?1", [p], |r| {
                                r.get::<_, i64>(0)
                            })?
                    }
                    None => 1,
                };
                if depth > 3 {
                    return Err(AppError::Invalid("项目最多三层".into()));
                }
                self.unique_node_title(parent, title, None)?;
                self.conn.execute("INSERT INTO nodes(parent_id,depth,title,project_status,note,due_at,sort_order,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,(SELECT COALESCE(MAX(sort_order),-1)+1 FROM nodes WHERE parent_id IS ?1),?7,?7)",params![parent,depth,title.trim(),parent.is_none().then_some("active"),note.as_deref().filter(|v|!v.is_empty()),due_at.as_deref().filter(|v|!v.is_empty()),self.now])?;
                let id = self.conn.last_insert_rowid();
                temp.insert(temp.len() + 1, id);
                self.log(
                    "node.add",
                    "nodes",
                    id,
                    None,
                    format!("添加「{}」", title.trim()),
                )?;
                self.changed_node(id);
                if let Some(p) = parent {
                    self.touch_node_ancestors(p)?;
                }
            }
            ProjectChange::Update {
                node,
                title,
                note,
                due_at,
            } => {
                let id = resolve(node, temp)?;
                let before = oplog::row_json(self.conn, "nodes", id)?;
                let name = self.node_name(id)?;
                if let Some(title) = title {
                    let parent: Option<i64> = self.conn.query_row(
                        "SELECT parent_id FROM nodes WHERE id=?1",
                        [id],
                        |r| r.get(0),
                    )?;
                    self.unique_node_title(parent, title, Some(id))?;
                }
                self.conn.execute("UPDATE nodes SET title=COALESCE(?2,title),note=CASE WHEN ?3 IS NULL THEN note ELSE NULLIF(?3,'') END,due_at=CASE WHEN ?4 IS NULL THEN due_at ELSE NULLIF(?4,'') END,version=version+1,updated_at=?5 WHERE id=?1",params![id,title.as_deref().map(str::trim),note,due_at,self.now])?;
                self.log(
                    "node.update",
                    "nodes",
                    id,
                    before,
                    format!("修改「{name}」"),
                )?;
                self.changed_node(id);
                let parent: Option<i64> =
                    self.conn
                        .query_row("SELECT parent_id FROM nodes WHERE id=?1", [id], |r| {
                            r.get(0)
                        })?;
                if let Some(p) = parent {
                    self.touch_node_ancestors(p)?;
                }
            }
            ProjectChange::Move { node, parent } => {
                let id = resolve(node, temp)?;
                let parent = resolve(parent, temp)?;
                self.unique_node_title(Some(parent), &self.node_name(id)?, Some(id))?;
                let descendants = self.descendants(id)?;
                let (old_parent, depth): (Option<i64>, i64) = self.conn.query_row(
                    "SELECT parent_id,depth FROM nodes WHERE id=?1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?;
                if old_parent.is_none() || descendants.iter().any(|(n, _)| *n == parent) {
                    return Err(AppError::Invalid(
                        "节点不能移入自身或子节点，项目根节点不能移动".into(),
                    ));
                }
                let new_depth: i64 = self.conn.query_row(
                    "SELECT depth+1 FROM nodes WHERE id=?1",
                    [parent],
                    |r| r.get(0),
                )?;
                let delta = new_depth - depth;
                if descendants.iter().any(|(_, d)| d + delta > 3) {
                    return Err(AppError::Invalid("移动后超过三层".into()));
                }
                for (node, depth) in descendants {
                    let before = oplog::row_json(self.conn, "nodes", node)?;
                    self.conn.execute("UPDATE nodes SET depth=?2,parent_id=CASE WHEN id=?3 THEN ?4 ELSE parent_id END,version=version+1,updated_at=?5 WHERE id=?1",params![node,depth+delta,id,parent,self.now])?;
                    self.log("node.move", "nodes", node, before, "移动项目节点".into())?;
                    self.changed_node(node);
                }
                if let Some(p) = old_parent {
                    self.touch_node_ancestors(p)?;
                }
                self.touch_node_ancestors(parent)?;
            }
            ProjectChange::Delete { node } => {
                let id = resolve(node, temp)?;
                let parent: Option<i64> =
                    self.conn
                        .query_row("SELECT parent_id FROM nodes WHERE id=?1", [id], |r| {
                            r.get(0)
                        })?;
                for (node, _) in self.descendants(id)? {
                    let mut stmt = self
                        .conn
                        .prepare("SELECT id,status FROM tasks WHERE node_id=?1 ORDER BY id")?;
                    let tasks = stmt
                        .query_map([node], |r| {
                            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
                        })?
                        .collect::<Result<Vec<_>, _>>()?;
                    drop(stmt);
                    for (task, status) in tasks {
                        if status == "open" {
                            self.delete_task(task)?;
                        } else {
                            self.update_task(
                                task,
                                &[("node_id", None)],
                                "解除已完成任务的节点关联，保留历史".into(),
                            )?;
                        }
                    }
                    // 显式保存便签快照，避免外键级联绕过撤销日志；逆序撤销先恢复节点再恢复便签。
                    let mut stmt = self
                        .conn
                        .prepare("SELECT id FROM stickies WHERE node_id=?1")?;
                    let stickies = stmt
                        .query_map([node], |r| r.get::<_, i64>(0))?
                        .collect::<Result<Vec<_>, _>>()?;
                    drop(stmt);
                    for sticky in stickies {
                        self.remove_logged(
                            "stickies",
                            sticky,
                            "sticky.delete",
                            "关闭被删除项目的便利贴".into(),
                        )?;
                    }
                    let name = self.node_name(node)?;
                    self.remove_logged("nodes", node, "node.delete", format!("删除「{name}」"))?;
                    self.changed_node(node);
                }
                if let Some(p) = parent {
                    self.touch_node_ancestors(p)?;
                }
            }
            ProjectChange::SetDone { node, done } => {
                self.set_node_done(resolve(node, temp)?, *done)?
            }
            ProjectChange::SetStatus { project, status } => {
                let id = resolve(project, temp)?;
                if status != "archived" {
                    self.set_node_done(id, status == "done")?;
                }
                let before = oplog::row_json(self.conn, "nodes", id)?;
                self.conn.execute("UPDATE nodes SET project_status=?2,version=version+1,updated_at=?3 WHERE id=?1 AND depth=1",params![id,status,self.now])?;
                self.log("project.status", "nodes", id, before, "修改项目状态".into())?;
                self.changed_node(id);
            }
            ProjectChange::Schedule { node, planned_date } => {
                let id = resolve(node, temp)?;
                let status: String =
                    self.conn
                        .query_row("SELECT status FROM nodes WHERE id=?1", [id], |r| r.get(0))?;
                if status == "done" {
                    return Err(AppError::Invalid("已完成节点不能安排今日任务".into()));
                }
                let task: Option<i64> = self
                    .conn
                    .query_row(
                        "SELECT id FROM tasks WHERE node_id=?1 AND status='open'",
                        [id],
                        |r| r.get(0),
                    )
                    .optional()?;
                if let Some(task) = task {
                    self.update_task(
                        task,
                        &[("planned_date", Some(planned_date.clone()))],
                        "调整关联任务的计划日期".into(),
                    )?;
                } else {
                    let (title, note, due): (String, Option<String>, Option<String>) =
                        self.conn.query_row(
                            "SELECT title,note,due_at FROM nodes WHERE id=?1",
                            [id],
                            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                        )?;
                    let task =
                        self.add_task(&title, planned_date, due.as_deref(), None, note.as_deref())?;
                    self.update_task(
                        task,
                        &[("node_id", Some(id.to_string()))],
                        "关联项目节点".into(),
                    )?;
                }
                self.touch_node_ancestors(id)?;
            }
        }
        Ok(())
    }
}
