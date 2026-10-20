//! 项目与节点的树结构查询。

use std::collections::HashMap;

use rusqlite::OptionalExtension;

use super::{Progress, ProjectNode, Service};
use crate::error::AppResult;

struct Flat {
    id: i64,
    parent_id: Option<i64>,
    depth: i64,
    title: String,
    done: bool,
    note: Option<String>,
    due_at: Option<String>,
    created_at: String,
    done_at: Option<String>,
    project_status: Option<String>,
}

fn build(id: i64, flat: &HashMap<i64, Flat>, kids: &HashMap<i64, Vec<i64>>) -> ProjectNode {
    let f = &flat[&id];
    let children: Vec<ProjectNode> = kids
        .get(&id)
        .map(|ids| ids.iter().map(|k| build(*k, flat, kids)).collect())
        .unwrap_or_default();
    let progress = (!children.is_empty()).then(|| Progress {
        done: children.iter().filter(|c| c.done).count() as u32,
        total: children.len() as u32,
    });
    ProjectNode {
        id,
        title: f.title.clone(),
        depth: f.depth,
        done: f.done,
        note: f.note.clone(),
        due_at: f.due_at.clone(),
        created_at: f.created_at.clone(),
        done_at: f.done_at.clone(),
        project_status: f.project_status.clone(),
        progress,
        children,
    }
}

impl Service<'_> {
    /// 全部项目的树。进行中在前，已完成其次，已归档最后（界面折叠在底部）
    pub fn project_tree(&self) -> AppResult<Vec<ProjectNode>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, parent_id, depth, title, status, project_status, note, due_at, created_at, done_at FROM nodes
             ORDER BY sort_order, id",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(Flat {
                    id: r.get(0)?,
                    parent_id: r.get(1)?,
                    depth: r.get(2)?,
                    title: r.get(3)?,
                    done: r.get::<_, String>(4)? == "done",
                    project_status: r.get(5)?,
                    note: r.get(6)?,
                    due_at: r.get(7)?,
                    created_at: r.get(8)?,
                    done_at: r.get(9)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let mut roots = Vec::new();
        let mut kids: HashMap<i64, Vec<i64>> = HashMap::new();
        for f in &rows {
            match f.parent_id {
                Some(p) => kids.entry(p).or_default().push(f.id),
                None => roots.push(f.id),
            }
        }
        let flat: HashMap<i64, Flat> = rows.into_iter().map(|f| (f.id, f)).collect();
        let rank = |id: &i64| match flat[id].project_status.as_deref() {
            Some("archived") => 2,
            Some("done") => 1,
            _ => 0,
        };
        roots.sort_by_key(|id| rank(id));
        Ok(roots.iter().map(|id| build(*id, &flat, &kids)).collect())
    }

    /// 节点及其祖先的标题，从项目开始排列
    fn ancestry(&self, node_id: i64) -> AppResult<Vec<String>> {
        let mut stmt = self.conn.prepare_cached(
            "WITH RECURSIVE up(id, parent_id, title, depth) AS (
               SELECT id, parent_id, title, depth FROM nodes WHERE id = ?1
               UNION ALL
               SELECT n.id, n.parent_id, n.title, n.depth FROM nodes n JOIN up ON n.id = up.parent_id
             )
             SELECT title FROM up ORDER BY depth",
        )?;
        let titles = stmt
            .query_map([node_id], |r| r.get(0))?
            .collect::<Result<Vec<String>, _>>()?;
        Ok(titles)
    }

    /// 关联任务旁显示的位置：节点所在的项目与上级，如「知了 › 阶段 0」。节点不存在时为空
    pub(super) fn node_path(&self, node_id: i64) -> AppResult<Option<String>> {
        let mut titles = self.ancestry(node_id)?;
        if titles.is_empty() {
            return Ok(None);
        }
        if titles.len() > 1 {
            titles.pop(); // 去掉节点本身
        }
        Ok(Some(titles.join(" › ")))
    }

    /// 节点所属项目的标题（写入完成记录快照用）
    pub(super) fn project_title_of(&self, node_id: i64) -> AppResult<Option<String>> {
        Ok(self.ancestry(node_id)?.into_iter().next())
    }

    #[allow(dead_code)] // 供 node_* 工具校验
    pub(super) fn node_exists(&self, node_id: i64) -> AppResult<bool> {
        Ok(self
            .conn
            .query_row("SELECT 1 FROM nodes WHERE id = ?1", [node_id], |_| Ok(()))
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

    fn node(
        conn: &Connection,
        parent: Option<i64>,
        depth: i64,
        title: &str,
        done: bool,
        ps: Option<&str>,
    ) -> i64 {
        conn.execute(
            "INSERT INTO nodes (parent_id, depth, title, status, project_status, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
            params![parent, depth, title, if done { "done" } else { "open" }, ps, NOW],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    #[test]
    fn 三层树与进度() {
        let conn = db();
        let clock = clock("2026-10-17 10:00:00");
        let svc = Service::new(&conn, &clock, &AppConfig::default()).unwrap();
        let old = node(&conn, None, 1, "搬家", true, Some("archived"));
        node(&conn, Some(old), 2, "打包", true, None);
        let p = node(&conn, None, 1, "知了", false, Some("active"));
        let s0 = node(&conn, Some(p), 2, "阶段 0", false, None);
        node(&conn, Some(s0), 3, "主界面", true, None);
        let leaf = node(&conn, Some(s0), 3, "设置页", false, None);
        node(&conn, Some(p), 2, "阶段 1", false, None);

        let tree = svc.project_tree().unwrap();
        assert_eq!(tree[0].title, "知了"); // 已归档排在最后
        assert_eq!(tree[0].created_at, NOW);
        assert_eq!(tree[0].done_at, None);
        assert_eq!(tree[1].title, "搬家");
        assert_eq!(tree[0].progress, Some(Progress { done: 0, total: 2 }));
        assert_eq!(
            tree[0].children[0].progress,
            Some(Progress { done: 1, total: 2 })
        );
        assert_eq!(tree[0].children[1].progress, None);

        assert_eq!(svc.node_path(leaf).unwrap().unwrap(), "知了 › 阶段 0");
        assert_eq!(svc.node_path(s0).unwrap().unwrap(), "知了");
        assert_eq!(svc.project_title_of(leaf).unwrap().unwrap(), "知了");
        assert_eq!(svc.node_path(9999).unwrap(), None);
    }
}
