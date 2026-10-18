//! 项目变更的只读预演。虚拟树用于校验同一轮的新建引用、移动与删除。
use std::collections::{HashMap, HashSet};

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use super::Progress;
use super::Service;
use super::changes::{Pin, PreviewItem, TaskRef, valid_date, valid_datetime};
use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ProjectChange {
    Add {
        title: String,
        note: Option<String>,
        due_at: Option<String>,
    },
    NodeAdd {
        parent: TaskRef,
        title: String,
        note: Option<String>,
        due_at: Option<String>,
    },
    Update {
        node: TaskRef,
        title: Option<String>,
        note: Option<String>,
        due_at: Option<String>,
    },
    Move {
        node: TaskRef,
        parent: TaskRef,
    },
    Delete {
        node: TaskRef,
    },
    SetDone {
        node: TaskRef,
        done: bool,
    },
    SetStatus {
        project: TaskRef,
        status: String,
    },
    Schedule {
        node: TaskRef,
        planned_date: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeBrief {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub depth: i64,
    pub title: String,
    pub status: String,
    pub project_status: Option<String>,
    pub note: Option<String>,
    pub due_at: Option<String>,
    pub done_at: Option<String>,
    pub version: i64,
    pub progress: Option<Progress>,
}

impl Service<'_> {
    pub fn query_project_nodes(&self, keyword: Option<&str>) -> AppResult<Vec<NodeBrief>> {
        let mut stmt = self.conn.prepare(
            "SELECT id,parent_id,depth,title,status,project_status,note,due_at,done_at,version,
             (SELECT COUNT(*) FROM nodes c WHERE c.parent_id=nodes.id),
             (SELECT COUNT(*) FROM nodes c WHERE c.parent_id=nodes.id AND c.status='done')
             FROM nodes WHERE ?1 IS NULL OR instr(title,?1)>0 OR instr(COALESCE(note,''),?1)>0 ORDER BY depth,sort_order,id")?;
        Ok(stmt
            .query_map([keyword], |r| {
                Ok(NodeBrief {
                    id: r.get(0)?,
                    parent_id: r.get(1)?,
                    depth: r.get(2)?,
                    title: r.get(3)?,
                    status: r.get(4)?,
                    project_status: r.get(5)?,
                    note: r.get(6)?,
                    due_at: r.get(7)?,
                    done_at: r.get(8)?,
                    version: r.get(9)?,
                    progress: match r.get::<_, u32>(10)? {
                        0 => None,
                        total => Some(Progress {
                            done: r.get(11)?,
                            total,
                        }),
                    },
                })
            })?
            .collect::<Result<Vec<_>, _>>()?)
    }

    pub fn parent_completion_candidates(&self) -> AppResult<Vec<NodeBrief>> {
        let nodes = self.query_project_nodes(None)?;
        let by_id: HashMap<_, _> = nodes.iter().map(|n| (n.id, n)).collect();
        Ok(nodes
            .iter()
            .filter(|n| {
                let mut ancestor = *n;
                while let Some(parent) = ancestor.parent_id.and_then(|id| by_id.get(&id)) {
                    ancestor = parent;
                }
                n.status == "open" && ancestor.project_status.as_deref() != Some("archived") && {
                    let children: Vec<_> =
                        nodes.iter().filter(|c| c.parent_id == Some(n.id)).collect();
                    !children.is_empty() && children.iter().all(|c| c.status == "done")
                }
            })
            .cloned()
            .collect())
    }
}

pub(super) struct ProjectValidation {
    nodes: HashMap<i64, NodeBrief>,
    original: HashMap<i64, NodeBrief>,
    created: usize,
    pub pins: Vec<Pin>,
    pub preview: Vec<PreviewItem>,
    pub dangerous: bool,
    pub touched: HashSet<i64>,
    pub touched_tasks: HashSet<i64>,
}

fn invalid(message: impl Into<String>) -> AppError {
    AppError::Invalid(message.into())
}

fn title_valid(title: &str) -> AppResult<()> {
    if title.trim().is_empty() || title.trim().chars().count() > 200 {
        return Err(invalid("标题须为 1 到 200 个字"));
    }
    Ok(())
}

fn due_valid(due: &Option<String>) -> AppResult<()> {
    if due
        .as_deref()
        .is_some_and(|d| !d.is_empty() && !valid_datetime(d))
    {
        return Err(invalid("截止时间格式应为 YYYY-MM-DDTHH:MM:SS"));
    }
    Ok(())
}

impl ProjectValidation {
    pub fn new(svc: &Service<'_>) -> AppResult<Self> {
        let nodes: HashMap<_, _> = svc
            .query_project_nodes(None)?
            .into_iter()
            .map(|n| (n.id, n))
            .collect();
        Ok(Self {
            original: nodes.clone(),
            nodes,
            created: 0,
            pins: vec![],
            preview: vec![],
            dangerous: false,
            touched: HashSet::new(),
            touched_tasks: HashSet::new(),
        })
    }

    fn resolve(&self, node: &TaskRef) -> AppResult<i64> {
        let id = match node {
            TaskRef::Id(id) => *id,
            TaskRef::Temp(t) => -t
                .strip_prefix('$')
                .and_then(|v| v.parse::<i64>().ok())
                .filter(|v| *v > 0)
                .ok_or_else(|| invalid("节点引用应为 $1、$2 等"))?,
        };
        if !self.nodes.contains_key(&id) {
            return Err(invalid(format!("项目节点 {node:?} 不存在或已在本轮删除")));
        }
        Ok(id)
    }

    fn descendants(&self, id: i64) -> Vec<i64> {
        let mut out = vec![id];
        let mut next = 0;
        while next < out.len() {
            let p = out[next];
            out.extend(
                self.nodes
                    .values()
                    .filter(|n| n.parent_id == Some(p))
                    .map(|n| n.id),
            );
            next += 1;
        }
        out
    }

    fn pin(&mut self, id: i64) {
        if let Some(n) = self.original.get(&id) {
            if !self.pins.iter().any(|p| p.entity == "nodes" && p.id == id) {
                self.pins.push(Pin {
                    entity: "nodes".into(),
                    id,
                    version: n.version,
                });
            }
        }
    }

    fn ancestors(&mut self, mut id: i64) {
        loop {
            self.pin(id);
            match self.nodes.get(&id).and_then(|n| n.parent_id) {
                Some(p) => id = p,
                None => break,
            }
        }
    }

    fn mark(&mut self, id: i64) {
        self.pin(id);
        if id > 0 {
            self.touched.insert(id);
        }
    }

    fn show(&mut self, icon: &str, text: String, danger: bool) {
        self.preview.push(PreviewItem {
            icon: icon.into(),
            text,
            danger,
        });
        self.dangerous |= danger;
    }

    fn linked(&mut self, svc: &Service<'_>, ids: &[i64], verb: Option<&str>) -> AppResult<()> {
        for node in ids.iter().filter(|id| **id > 0) {
            let mut stmt = svc
                .conn
                .prepare("SELECT id,title,version,status FROM tasks WHERE node_id=?1")?;
            let rows = stmt
                .query_map([node], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, String>(3)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            for (id, title, version, status) in rows {
                if !self.pins.iter().any(|p| p.entity == "tasks" && p.id == id) {
                    self.pins.push(Pin {
                        entity: "tasks".into(),
                        id,
                        version,
                    });
                }
                if let Some(verb) = verb {
                    let affected = match verb {
                        "删除" => true,
                        "同步取消完成" => {
                            let latest: Option<i64> = svc.conn.query_row("SELECT task_id FROM completions WHERE node_id=?1 ORDER BY id DESC LIMIT 1", [node], |r| r.get(0)).optional()?.flatten();
                            status == "done" && latest == Some(id)
                        }
                        _ => status == "open",
                    };
                    if affected {
                        self.touched_tasks.insert(id);
                    }
                    if status == "open" && affected {
                        self.show("link", format!("{verb}关联任务「{title}」"), verb == "删除");
                    } else if verb == "删除" {
                        self.show(
                            "link",
                            format!("保留已完成任务「{title}」及完成历史，解除节点关联"),
                            false,
                        );
                    } else if affected {
                        self.show("link", format!("{verb}关联任务「{title}」"), false);
                    }
                }
            }
        }
        Ok(())
    }

    pub fn task_link(&mut self, svc: &Service<'_>, task: i64) -> AppResult<()> {
        let node: Option<i64> = svc
            .conn
            .query_row("SELECT node_id FROM tasks WHERE id=?1", [task], |r| {
                r.get(0)
            })
            .optional()?
            .flatten();
        if let Some(id) = node {
            self.ancestors(id);
        }
        Ok(())
    }

    pub fn task_completion(&mut self, svc: &Service<'_>, task: i64, done: bool) -> AppResult<()> {
        let node: Option<i64> = svc
            .conn
            .query_row("SELECT node_id FROM tasks WHERE id=?1", [task], |r| {
                r.get(0)
            })
            .optional()?
            .flatten();
        if let Some(id) = node {
            if !self.nodes.contains_key(&id) {
                return Err(invalid("关联节点已在本轮删除，无法继续完成关联任务"));
            }
            self.set_done(svc, id, done)?;
        }
        Ok(())
    }

    pub fn change(&mut self, svc: &Service<'_>, c: &ProjectChange) -> AppResult<()> {
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
                title_valid(title)?;
                due_valid(due_at)?;
                let parent = if let ProjectChange::NodeAdd { parent, .. } = c {
                    Some(self.resolve(parent)?)
                } else {
                    None
                };
                let depth = parent.map(|p| self.nodes[&p].depth + 1).unwrap_or(1);
                if depth > 3 {
                    return Err(invalid("项目最多三层，不能在第三层继续添加节点"));
                }
                if let Some(p) = parent {
                    self.ancestors(p);
                }
                self.created += 1;
                let id = -(self.created as i64);
                self.nodes.insert(
                    id,
                    NodeBrief {
                        id,
                        parent_id: parent,
                        depth,
                        title: title.trim().into(),
                        status: "open".into(),
                        project_status: parent.is_none().then(|| "active".into()),
                        note: note.clone(),
                        due_at: due_at.clone(),
                        done_at: None,
                        version: 1,
                        progress: None,
                    },
                );
                self.show(
                    "plus",
                    format!(
                        "添加{}「{}」",
                        if parent.is_none() { "项目" } else { "节点" },
                        title.trim()
                    ),
                    false,
                );
            }
            ProjectChange::Update {
                node,
                title,
                note,
                due_at,
            } => {
                let id = self.resolve(node)?;
                if let Some(t) = title {
                    title_valid(t)?;
                }
                due_valid(due_at)?;
                self.mark(id);
                self.ancestors(id);
                let n = self.nodes.get_mut(&id).unwrap();
                let old = n.title.clone();
                if let Some(t) = title {
                    n.title = t.trim().into();
                }
                if let Some(t) = note {
                    n.note = (!t.is_empty()).then(|| t.clone());
                }
                if let Some(t) = due_at {
                    n.due_at = (!t.is_empty()).then(|| t.clone());
                }
                self.show("edit", format!("修改节点「{old}」"), false);
            }
            ProjectChange::Move { node, parent } => {
                let id = self.resolve(node)?;
                let parent = self.resolve(parent)?;
                if self.nodes[&id].parent_id.is_none() {
                    return Err(invalid("项目根节点不能移动到其他节点下"));
                }
                let descendants = self.descendants(id);
                if descendants.contains(&parent) {
                    return Err(invalid("不能把节点移入自身或自己的子节点"));
                }
                let delta = self.nodes[&parent].depth + 1 - self.nodes[&id].depth;
                if descendants.iter().any(|n| self.nodes[n].depth + delta > 3) {
                    return Err(invalid("移动后超过项目最多三层的限制"));
                }
                self.ancestors(id);
                self.ancestors(parent);
                for d in &descendants {
                    self.mark(*d);
                    self.nodes.get_mut(d).unwrap().depth += delta;
                }
                let name = self.nodes[&id].title.clone();
                let target = self.nodes[&parent].title.clone();
                self.nodes.get_mut(&id).unwrap().parent_id = Some(parent);
                self.linked(svc, &descendants, None)?;
                self.show(
                    "folder",
                    format!("将「{name}」及其子节点移动到「{target}」"),
                    false,
                );
            }
            ProjectChange::Delete { node } => {
                let id = self.resolve(node)?;
                let descendants = self.descendants(id);
                self.ancestors(id);
                self.linked(svc, &descendants, Some("删除"))?;
                for d in descendants {
                    self.mark(d);
                    let n = self.nodes.remove(&d).unwrap();
                    self.show(
                        "trash",
                        format!(
                            "删除{}「{}」",
                            if n.depth == 1 { "项目" } else { "节点" },
                            n.title
                        ),
                        true,
                    );
                }
            }
            ProjectChange::SetDone { node, done } => {
                let id = self.resolve(node)?;
                self.set_done(svc, id, *done)?;
            }
            ProjectChange::SetStatus { project, status } => {
                if !matches!(status.as_str(), "active" | "done" | "archived") {
                    return Err(invalid("项目状态必须为 active、done 或 archived"));
                }
                let id = self.resolve(project)?;
                if self.nodes[&id].depth != 1 {
                    return Err(invalid("只有项目根节点可以设置项目状态"));
                }
                self.mark(id);
                if status != "archived" {
                    self.set_done(svc, id, status == "done")?;
                }
                self.nodes.get_mut(&id).unwrap().project_status = Some(status.clone());
                let label = match status.as_str() {
                    "active" => "进行中",
                    "done" => "已完成",
                    _ => "已归档",
                };
                self.show(
                    "folder",
                    format!("将项目「{}」设为{label}", self.nodes[&id].title),
                    false,
                );
            }
            ProjectChange::Schedule { node, planned_date } => {
                if !valid_date(planned_date) {
                    return Err(invalid("计划日期格式应为 YYYY-MM-DD"));
                }
                let id = self.resolve(node)?;
                if self.nodes[&id].status == "done" {
                    return Err(invalid("已完成节点不能安排今日任务，请先取消完成"));
                }
                self.mark(id);
                self.ancestors(id);
                self.linked(svc, &[id], Some("调整计划日期"))?;
                self.show(
                    "calendar",
                    format!(
                        "将「{}」安排到 {planned_date}（已有未完成关联任务时复用）",
                        self.nodes[&id].title
                    ),
                    false,
                );
            }
        }
        Ok(())
    }

    fn set_done(&mut self, svc: &Service<'_>, id: i64, done: bool) -> AppResult<()> {
        if (self.nodes[&id].status == "done") == done {
            return Ok(());
        }
        self.ancestors(id);
        self.mark(id);
        self.linked(
            svc,
            &[id],
            Some(if done {
                "同步完成"
            } else {
                "同步取消完成"
            }),
        )?;
        let n = &self.nodes[&id];
        let prior = if id > 0 && !done {
            let today = svc.today().format(super::DATE_FMT).to_string();
            svc.conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM completions WHERE node_id=?1 AND business_date!=?2)",
                params![id, today],
                |r| r.get::<_, bool>(0),
            )?
        } else {
            false
        };
        self.show(
            "check",
            format!(
                "{}节点「{}」",
                if done { "完成" } else { "取消完成" },
                n.title
            ),
            prior,
        );
        let n = self.nodes.get_mut(&id).unwrap();
        n.status = if done { "done" } else { "open" }.into();
        if n.depth == 1 {
            n.project_status = Some(if done { "done" } else { "active" }.into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use crate::service::changes::Change;
    use crate::service::oplog::Source;
    use crate::service::testing::{clock, db};

    fn change(c: ProjectChange) -> Change {
        Change::Project { change: c }
    }
    fn reference(n: usize) -> TaskRef {
        TaskRef::Temp(format!("${n}"))
    }
    fn add(parent: Option<TaskRef>, title: &str) -> Change {
        change(match parent {
            Some(parent) => ProjectChange::NodeAdd {
                parent,
                title: title.into(),
                note: None,
                due_at: None,
            },
            None => ProjectChange::Add {
                title: title.into(),
                note: None,
                due_at: None,
            },
        })
    }
    fn apply(svc: &Service<'_>, plan: &[Change]) {
        let v = svc.validate_plan(plan).unwrap();
        svc.apply_plan(
            plan,
            &v.pins,
            &uuid::Uuid::new_v4().to_string(),
            Source::Chat,
        )
        .unwrap();
    }
    fn id(svc: &Service<'_>, name: &str) -> i64 {
        svc.query_project_nodes(Some(name)).unwrap()[0].id
    }
    fn count(svc: &Service<'_>, table: &str) -> i64 {
        svc.conn
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn 三层新建引用与移动限制() {
        let db = db();
        let clock = clock("2026-10-18 10:00:00");
        let svc = Service::new(&db, &clock, &AppConfig::default()).unwrap();
        let mut plan = vec![
            add(None, "项目"),
            add(Some(reference(1)), "阶段"),
            add(Some(reference(2)), "叶子"),
        ];
        apply(&svc, &plan);
        plan.push(add(Some(reference(3)), "第四层"));
        assert!(svc.validate_plan(&plan).is_err());
        let stage = id(&svc, "阶段");
        let leaf = id(&svc, "叶子");
        assert!(
            svc.validate_plan(&[change(ProjectChange::Move {
                node: TaskRef::Id(stage),
                parent: TaskRef::Id(leaf)
            })])
            .is_err()
        );
        apply(&svc, &[add(None, "另一项目")]);
        apply(
            &svc,
            &[change(ProjectChange::Move {
                node: TaskRef::Id(stage),
                parent: TaskRef::Id(id(&svc, "另一项目")),
            })],
        );
        assert_eq!(svc.query_project_nodes(Some("叶子")).unwrap()[0].depth, 3);
    }

    #[test]
    fn 安排复用双向完成只计一次并可撤销() {
        let db = db();
        let clock = clock("2026-10-18 10:00:00");
        let svc = Service::new(&db, &clock, &AppConfig::default()).unwrap();
        apply(&svc, &[add(None, "项目"), add(Some(reference(1)), "工作")]);
        let node = id(&svc, "工作");
        let schedule = change(ProjectChange::Schedule {
            node: TaskRef::Id(node),
            planned_date: "2026-10-18".into(),
        });
        apply(&svc, &[schedule.clone()]);
        apply(&svc, &[schedule]);
        assert_eq!(count(&svc, "tasks"), 1);
        apply(
            &svc,
            &[change(ProjectChange::SetDone {
                node: TaskRef::Id(node),
                done: true,
            })],
        );
        assert_eq!(count(&svc, "completions"), 1);
        assert_eq!(svc.task_board().unwrap().done.len(), 1);
        assert_eq!(svc.parent_completion_candidates().unwrap().len(), 1);
        apply(&svc, &[Change::Undo]);
        assert_eq!(count(&svc, "completions"), 0);
        let task = svc.task_board().unwrap().today[0].clone();
        svc.set_task_done(task.id, task.version, true).unwrap();
        assert_eq!(
            svc.query_project_nodes(Some("工作")).unwrap()[0].status,
            "done"
        );
        apply(
            &svc,
            &[change(ProjectChange::SetDone {
                node: TaskRef::Id(node),
                done: true,
            })],
        );
        assert_eq!(count(&svc, "completions"), 1);
    }

    #[test]
    fn 删除项目保留完成快照并可完整撤销() {
        let db = db();
        let clock = clock("2026-10-18 10:00:00");
        let svc = Service::new(&db, &clock, &AppConfig::default()).unwrap();
        apply(
            &svc,
            &[
                add(None, "项目"),
                add(Some(reference(1)), "完成项"),
                add(Some(reference(1)), "未完项"),
            ],
        );
        let root = id(&svc, "项目");
        let done = id(&svc, "完成项");
        let open = id(&svc, "未完项");
        apply(
            &svc,
            &[
                change(ProjectChange::Schedule {
                    node: TaskRef::Id(done),
                    planned_date: "2026-10-18".into(),
                }),
                change(ProjectChange::SetDone {
                    node: TaskRef::Id(done),
                    done: true,
                }),
                change(ProjectChange::Schedule {
                    node: TaskRef::Id(open),
                    planned_date: "2026-10-18".into(),
                }),
            ],
        );
        let before = svc.day_completions("2026-10-18").unwrap();
        let sticky = svc.sticky_create("project", Some(root)).unwrap();
        let deletion = change(ProjectChange::Delete {
            node: TaskRef::Id(root),
        });
        let v = svc.validate_plan(&[deletion.clone()]).unwrap();
        assert!(v.dangerous);
        assert!(v.preview.iter().any(|p| p.text.contains("关联任务")));
        apply(&svc, &[deletion]);
        assert_eq!(count(&svc, "nodes"), 0);
        assert_eq!(count(&svc, "tasks"), 1);
        assert!(svc.stickies().unwrap().is_empty());
        assert_eq!(svc.day_completions("2026-10-18").unwrap(), before);
        apply(&svc, &[Change::Undo]);
        assert_eq!(count(&svc, "nodes"), 3);
        assert_eq!(count(&svc, "tasks"), 2);
        assert_eq!(svc.sticky_get(sticky.id).unwrap(), sticky);
        assert_eq!(svc.day_completions("2026-10-18").unwrap(), before);
    }

    #[test]
    fn 确认期间新增子节点和关联任务会冲突() {
        let db = db();
        let clock = clock("2026-10-18 10:00:00");
        let svc = Service::new(&db, &clock, &AppConfig::default()).unwrap();
        apply(&svc, &[add(None, "项目"), add(Some(reference(1)), "工作")]);
        let root = id(&svc, "项目");
        let node = id(&svc, "工作");
        let plan = [change(ProjectChange::Delete {
            node: TaskRef::Id(root),
        })];
        let v = svc.validate_plan(&plan).unwrap();
        apply(&svc, &[add(Some(TaskRef::Id(node)), "新子项")]);
        assert!(matches!(
            svc.apply_plan(&plan, &v.pins, "stale-tree", Source::Chat),
            Err(AppError::Conflict(_))
        ));
        let v = svc.validate_plan(&plan).unwrap();
        apply(
            &svc,
            &[change(ProjectChange::Schedule {
                node: TaskRef::Id(node),
                planned_date: "2026-10-18".into(),
            })],
        );
        assert!(matches!(
            svc.apply_plan(&plan, &v.pins, "stale-link", Source::Chat),
            Err(AppError::Conflict(_))
        ));
    }

    #[test]
    fn 两个关联任务同步两个节点时要求批量确认() {
        let db = db();
        let clock = clock("2026-10-18 10:00:00");
        let svc = Service::new(&db, &clock, &AppConfig::default()).unwrap();
        apply(
            &svc,
            &[
                add(None, "项目"),
                add(Some(reference(1)), "工作甲"),
                add(Some(reference(1)), "工作乙"),
            ],
        );
        apply(
            &svc,
            &[
                change(ProjectChange::Schedule {
                    node: TaskRef::Id(id(&svc, "工作甲")),
                    planned_date: "2026-10-18".into(),
                }),
                change(ProjectChange::Schedule {
                    node: TaskRef::Id(id(&svc, "工作乙")),
                    planned_date: "2026-10-18".into(),
                }),
            ],
        );
        let tasks = svc.task_board().unwrap().today;
        let plan: Vec<_> = tasks
            .iter()
            .map(|t| Change::TaskSetDone {
                task: TaskRef::Id(t.id),
                done: true,
            })
            .collect();
        let v = svc.validate_plan(&plan).unwrap();
        assert!(v.dangerous);
        assert!(
            v.preview
                .iter()
                .any(|p| p.text.contains("完成节点「工作甲」"))
        );
        assert!(
            v.preview
                .iter()
                .any(|p| p.text.contains("完成节点「工作乙」"))
        );
        assert_eq!(v.pins.iter().filter(|p| p.entity == "tasks").count(), 2);
    }

    #[test]
    fn 更新跨父移动归档进度和撤销保持一致() {
        let db = db();
        let clock = clock("2026-10-18 10:00:00");
        let svc = Service::new(&db, &clock, &AppConfig::default()).unwrap();
        apply(
            &svc,
            &[
                add(None, "甲项目"),
                add(None, "乙项目"),
                add(Some(reference(1)), "原节点"),
            ],
        );
        let a = id(&svc, "甲项目");
        let b = id(&svc, "乙项目");
        let node = id(&svc, "原节点");
        apply(
            &svc,
            &[
                change(ProjectChange::Update {
                    node: TaskRef::Id(node),
                    title: Some("新节点".into()),
                    note: Some("备注".into()),
                    due_at: Some("2026-10-20T18:00:00".into()),
                }),
                change(ProjectChange::Move {
                    node: TaskRef::Id(node),
                    parent: TaskRef::Id(b),
                }),
                change(ProjectChange::SetDone {
                    node: TaskRef::Id(node),
                    done: true,
                }),
            ],
        );
        let n = svc.query_project_nodes(Some("新节点")).unwrap().remove(0);
        assert_eq!(n.parent_id, Some(b));
        assert_eq!(n.note.as_deref(), Some("备注"));
        assert_eq!(
            svc.query_project_nodes(Some("乙项目")).unwrap()[0].progress,
            Some(Progress { done: 1, total: 1 })
        );
        apply(
            &svc,
            &[change(ProjectChange::SetStatus {
                project: TaskRef::Id(b),
                status: "archived".into(),
            })],
        );
        assert!(svc.parent_completion_candidates().unwrap().is_empty());
        assert_eq!(svc.project_tree().unwrap().last().unwrap().id, b);
        apply(&svc, &[Change::Undo]);
        assert_eq!(svc.parent_completion_candidates().unwrap()[0].id, b);
        apply(&svc, &[Change::Undo]);
        let n = svc.query_project_nodes(Some("原节点")).unwrap().remove(0);
        assert_eq!(n.parent_id, Some(a));
        assert_eq!(n.status, "open");
        assert_eq!(count(&svc, "completions"), 0);
    }
}
