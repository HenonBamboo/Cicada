//! 任务：分区查询、勾选、新增

use chrono::NaiveDate;
use rusqlite::{OptionalExtension, Row};

use super::oplog::{self, Source};
use super::write::Writer;
use super::{ChangeSet, DATE_FMT, Service, TaskBoard, TaskLink, TaskView};
use crate::error::{AppError, AppResult};

const TASK_COLUMNS: &str = "t.id, t.title, t.planned_date, t.status, t.due_at, t.remind_at, t.note, \
     t.node_id, t.version, r.kind, r.params, t.created_at, t.done_at";

struct RawTask {
    view: TaskView,
    node_id: Option<i64>,
    recur_kind: Option<String>,
    recur_params: Option<String>,
}

fn read_task(r: &Row<'_>) -> rusqlite::Result<RawTask> {
    let status: String = r.get(3)?;
    Ok(RawTask {
        view: TaskView {
            id: r.get(0)?,
            title: r.get(1)?,
            planned_date: r.get(2)?,
            done: status == "done",
            delay_days: 0,
            due_at: r.get(4)?,
            remind_at: r.get(5)?,
            note: r.get(6)?,
            created_at: r.get(11)?,
            done_at: r.get(12)?,
            recur: None,
            link: None,
            version: r.get(8)?,
        },
        node_id: r.get(7)?,
        recur_kind: r.get(9)?,
        recur_params: r.get(10)?,
    })
}

/// 重复规则的简短说明
pub(super) fn recur_label(kind: &str, params: &str) -> String {
    let p: serde_json::Value = serde_json::from_str(params).unwrap_or_default();
    let n = |k: &str| p.get(k).and_then(|v| v.as_i64()).unwrap_or(0);
    const WEEK: [&str; 7] = ["一", "二", "三", "四", "五", "六", "日"];
    match kind {
        "daily" => "每天".into(),
        "weekdays" => "工作日".into(),
        "weekly" => {
            let w = n("weekday").clamp(1, 7) as usize;
            format!("每周{}", WEEK[w - 1])
        }
        "monthly" => format!("每月 {} 日", n("day")),
        "yearly" => format!("每年 {} 月 {} 日", n("month"), n("day")),
        _ => "重复".into(),
    }
}

/// 勾选前读取的任务状态
struct TaskState {
    status: String,
    planned: String,
    done_date: Option<String>,
    version: i64,
}

fn parse_date(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s, DATE_FMT).ok()
}

impl Service<'_> {
    fn query_tasks(&self, filter: &str, args: &[&dyn rusqlite::ToSql]) -> AppResult<Vec<TaskView>> {
        let sql = format!(
            "SELECT {TASK_COLUMNS} FROM tasks t LEFT JOIN recur_rules r ON r.id = t.recur_rule_id WHERE {filter}"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let raws = stmt
            .query_map(args, read_task)?
            .collect::<Result<Vec<_>, _>>()?;
        let today = self.today();
        let mut out = Vec::with_capacity(raws.len());
        for raw in raws {
            let mut v = raw.view;
            if let (Some(k), Some(p)) = (&raw.recur_kind, &raw.recur_params) {
                v.recur = Some(recur_label(k, p));
            }
            if let Some(node_id) = raw.node_id {
                v.link = self
                    .node_path(node_id)?
                    .map(|path| TaskLink { node_id, path });
            }
            if let (false, Some(d)) = (v.done, parse_date(&v.planned_date)) {
                v.delay_days = (today - d).num_days().max(0);
            }
            out.push(v);
        }
        Ok(out)
    }

    /// 任务四个分区。延期天数由计划日期推算，"之后"的任务到了当天自然进入"今日"。
    pub fn task_board(&self) -> AppResult<TaskBoard> {
        let today = self.today().format(DATE_FMT).to_string();
        Ok(TaskBoard {
            today: self.query_tasks(
                "t.status = 'open' AND t.planned_date = ?1 ORDER BY t.remind_at IS NULL, t.remind_at, t.id",
                &[&today],
            )?,
            legacy: self.query_tasks(
                "t.status = 'open' AND t.planned_date < ?1 ORDER BY t.planned_date, t.id",
                &[&today],
            )?,
            done: self.query_tasks(
                "t.status = 'done' AND t.done_date = ?1 ORDER BY t.done_at DESC, t.id",
                &[&today],
            )?,
            upcoming: self.query_tasks(
                "t.status = 'open' AND t.planned_date > ?1 ORDER BY t.planned_date, t.id",
                &[&today],
            )?,
        })
    }

    /// 勾选 / 取消勾选今日任务（唯一的手动写操作，docs/产品设计.md 第 2 节）。
    /// 关联项目节点同步改变，完成只记一条 completions（7.4），整体一个事务并写 op_log。
    pub fn set_task_done(&self, id: i64, version: i64, done: bool) -> AppResult<ChangeSet> {
        let today = self.today().format(DATE_FMT).to_string();
        let now = self.now_str();
        let row = self
            .conn
            .query_row(
                "SELECT status, planned_date, done_date, version FROM tasks WHERE id = ?1",
                [id],
                |r| {
                    Ok(TaskState {
                        status: r.get(0)?,
                        planned: r.get(1)?,
                        done_date: r.get(2)?,
                        version: r.get(3)?,
                    })
                },
            )
            .optional()?;
        let Some(TaskState {
            status,
            planned,
            done_date,
            version: current_version,
            ..
        }) = row
        else {
            return Err(AppError::Invalid("这件事已经不存在了".into()));
        };
        if current_version != version {
            return Err(AppError::Invalid(
                "这件事刚刚有变化，已为你刷新，请再试一次".into(),
            ));
        }
        let is_done = status == "done";
        if is_done == done {
            return Ok(ChangeSet::default());
        }
        if done && planned > today {
            return Err(AppError::Invalid("还没到计划日期的任务不能勾选".into()));
        }
        if !done && done_date.as_deref() != Some(today.as_str()) {
            return Err(AppError::Invalid("只能取消今天完成的任务".into()));
        }

        let tx = self.conn.unchecked_transaction()?;
        let mut w = Writer::new(&tx, oplog::new_group_id(), Source::Manual, now, today);
        w.set_task_done(id, done, |n| self.project_title_of(n))?;
        let changes = w.changes;
        tx.commit()?;
        Ok(changes)
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

    fn version(conn: &Connection, id: i64) -> i64 {
        conn.query_row("SELECT version FROM tasks WHERE id = ?1", [id], |r| {
            r.get(0)
        })
        .unwrap()
    }

    fn titles(v: &[TaskView]) -> Vec<&str> {
        v.iter().map(|t| t.title.as_str()).collect()
    }

    #[test]
    fn 四个分区() {
        let conn = db();
        let clock = clock("2026-10-17 10:00:00");
        let svc = Service::new(&conn, &clock, &AppConfig::default()).unwrap();
        add(&conn, "今天", "2026-10-17");
        add(&conn, "三天前", "2026-10-14");
        add(&conn, "明天", "2026-10-18");
        let done = add(&conn, "做完了", "2026-10-17");
        svc.set_task_done(done, 1, true).unwrap();

        let b = svc.task_board().unwrap();
        assert_eq!(titles(&b.today), ["今天"]);
        assert_eq!(titles(&b.legacy), ["三天前"]);
        assert_eq!(b.legacy[0].delay_days, 3);
        assert_eq!(titles(&b.done), ["做完了"]);
        assert_eq!(b.today[0].created_at, NOW);
        assert_eq!(b.today[0].done_at, None);
        assert_eq!(b.done[0].done_at.as_deref(), Some("2026-10-17T10:00:00"));
        assert_eq!(titles(&b.upcoming), ["明天"]);
    }

    #[test]
    fn 之后的任务到了当天进入今日() {
        let conn = db();
        add(&conn, "明天的事", "2026-10-18");
        let config = AppConfig::default();

        let c1 = clock("2026-10-17 23:59:00");
        let b = Service::new(&conn, &c1, &config)
            .unwrap()
            .task_board()
            .unwrap();
        assert_eq!(titles(&b.upcoming), ["明天的事"]);
        assert!(b.today.is_empty());

        let c2 = clock("2026-10-18 00:00:00");
        let b = Service::new(&conn, &c2, &config)
            .unwrap()
            .task_board()
            .unwrap();
        assert_eq!(titles(&b.today), ["明天的事"]);
        assert!(b.upcoming.is_empty());
    }

    #[test]
    fn 已完成区只显示今天完成的() {
        let conn = db();
        let config = AppConfig::default();
        let id = add(&conn, "昨天完成", "2026-10-16");
        let c1 = clock("2026-10-16 20:00:00");
        Service::new(&conn, &c1, &config)
            .unwrap()
            .set_task_done(id, 1, true)
            .unwrap();
        let c2 = clock("2026-10-17 08:00:00");
        let b = Service::new(&conn, &c2, &config)
            .unwrap()
            .task_board()
            .unwrap();
        assert!(b.done.is_empty());
    }

    #[test]
    fn 勾选写入完成记录和操作日志_取消后删除完成记录() {
        let conn = db();
        let clock = clock("2026-10-17 10:00:00");
        let svc = Service::new(&conn, &clock, &AppConfig::default()).unwrap();
        let id = add(&conn, "跑步", "2026-10-17");

        let ch = svc.set_task_done(id, 1, true).unwrap();
        assert_eq!(ch.tasks, [id]);
        assert_eq!(
            count(
                &conn,
                "SELECT COUNT(*) FROM completions WHERE kind = 'task'"
            ),
            1
        );
        assert_eq!(
            count(&conn, "SELECT COUNT(*) FROM op_log WHERE source = 'manual'"),
            2 // 任务 + 完成记录
        );
        assert_eq!(version(&conn, id), 2);

        svc.set_task_done(id, 2, false).unwrap();
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM completions"), 0);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM op_log"), 4);
    }

    #[test]
    fn 版本不一致时拒绝() {
        let conn = db();
        let clock = clock("2026-10-17 10:00:00");
        let svc = Service::new(&conn, &clock, &AppConfig::default()).unwrap();
        let id = add(&conn, "跑步", "2026-10-17");
        assert!(svc.set_task_done(id, 9, true).is_err());
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM op_log"), 0);
    }

    #[test]
    fn 不能勾选之后的任务_不能取消往日完成() {
        let conn = db();
        let config = AppConfig::default();
        let future = add(&conn, "明天", "2026-10-18");
        let c = clock("2026-10-17 10:00:00");
        assert!(
            Service::new(&conn, &c, &config)
                .unwrap()
                .set_task_done(future, 1, true)
                .is_err()
        );

        let old = add(&conn, "昨天", "2026-10-16");
        let c1 = clock("2026-10-16 10:00:00");
        Service::new(&conn, &c1, &config)
            .unwrap()
            .set_task_done(old, 1, true)
            .unwrap();
        assert!(
            Service::new(&conn, &c, &config)
                .unwrap()
                .set_task_done(old, 2, false)
                .is_err()
        );
    }

    #[test]
    fn 遗留任务可以勾选() {
        let conn = db();
        let clock = clock("2026-10-17 10:00:00");
        let svc = Service::new(&conn, &clock, &AppConfig::default()).unwrap();
        let id = add(&conn, "拖了很久", "2026-10-01");
        svc.set_task_done(id, 1, true).unwrap();
        let b = svc.task_board().unwrap();
        assert_eq!(titles(&b.done), ["拖了很久"]);
        let d: String = conn
            .query_row("SELECT business_date FROM completions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(d, "2026-10-17");
    }

    #[test]
    fn 关联任务勾选同步节点且只记一次() {
        let conn = db();
        let clock = clock("2026-10-17 10:00:00");
        let svc = Service::new(&conn, &clock, &AppConfig::default()).unwrap();
        conn.execute(
            "INSERT INTO nodes (depth, title, project_status, created_at, updated_at) VALUES (1, '知了', 'active', ?1, ?1)",
            [NOW],
        )
        .unwrap();
        let p = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO nodes (parent_id, depth, title, created_at, updated_at) VALUES (?1, 2, '写文档', ?2, ?2)",
            params![p, NOW],
        )
        .unwrap();
        let n = conn.last_insert_rowid();
        let id = add(&conn, "写文档", "2026-10-17");
        conn.execute("UPDATE tasks SET node_id = ?1 WHERE id = ?2", [n, id])
            .unwrap();

        let b = svc.task_board().unwrap();
        assert_eq!(b.today[0].link.as_ref().unwrap().path, "知了");

        let ch = svc.set_task_done(id, 1, true).unwrap();
        assert_eq!(ch.nodes, [n, p]);
        let status: String = conn
            .query_row("SELECT status FROM nodes WHERE id = ?1", [n], |r| r.get(0))
            .unwrap();
        assert_eq!(status, "done");
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM completions"), 1);
        assert_eq!(
            count(
                &conn,
                "SELECT COUNT(*) FROM completions WHERE kind = 'node' AND project_title = '知了'"
            ),
            1
        );
        // 任务、节点和父节点进度作为同一组操作记录
        assert_eq!(
            count(&conn, "SELECT COUNT(DISTINCT group_id) FROM op_log"),
            1
        );
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM op_log"), 4); // 任务、节点、父节点进度、完成记录

        svc.set_task_done(id, 2, false).unwrap();
        let status: String = conn
            .query_row("SELECT status FROM nodes WHERE id = ?1", [n], |r| r.get(0))
            .unwrap();
        assert_eq!(status, "open");
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM completions"), 0);
    }

    #[test]
    fn 重复规则说明() {
        assert_eq!(recur_label("daily", "{}"), "每天");
        assert_eq!(recur_label("weekly", r#"{"weekday":1}"#), "每周一");
        assert_eq!(recur_label("monthly", r#"{"day":15}"#), "每月 15 日");
    }
}
