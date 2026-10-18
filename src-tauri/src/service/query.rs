//! 只读查询：供模型的 *_query、summarize 工具使用。只返回需要的条目，不把全部数据交给模型（4.3）

use serde::Serialize;

use super::{DATE_FMT, Service};
use crate::error::AppResult;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TaskBrief {
    pub id: i64,
    pub title: String,
    pub planned_date: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub done_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remind_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct TaskFilter {
    /// open / done / all
    pub status: Option<String>,
    pub date_from: Option<String>,
    pub date_to: Option<String>,
    pub keyword: Option<String>,
    pub limit: Option<i64>,
}

pub const QUERY_LIMIT: i64 = 50;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Summary {
    pub from: String,
    pub to: String,
    pub completed_count: i64,
    pub completed: Vec<SummaryItem>,
    pub open_count: i64,
    pub overdue: Vec<TaskBrief>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SummaryItem {
    pub date: String,
    pub title: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
}

impl Service<'_> {
    /// 按状态、日期范围（计划日期，已完成的按完成日期）、关键词查询任务
    pub fn query_tasks_brief(&self, f: &TaskFilter) -> AppResult<Vec<TaskBrief>> {
        let mut sql = String::from(
            "SELECT id, title, planned_date, status, done_date, due_at, remind_at, note FROM tasks WHERE 1 = 1",
        );
        let mut args: Vec<String> = Vec::new();
        let status = f.status.as_deref().unwrap_or("all");
        match status {
            "open" => sql.push_str(" AND status = 'open'"),
            "done" => sql.push_str(" AND status = 'done'"),
            _ => {}
        }
        let date_col = if status == "done" {
            "done_date"
        } else {
            "planned_date"
        };
        if let Some(d) = &f.date_from {
            args.push(d.clone());
            sql.push_str(&format!(" AND {date_col} >= ?{}", args.len()));
        }
        if let Some(d) = &f.date_to {
            args.push(d.clone());
            sql.push_str(&format!(" AND {date_col} <= ?{}", args.len()));
        }
        if let Some(k) = f
            .keyword
            .as_deref()
            .map(str::trim)
            .filter(|k| !k.is_empty())
        {
            args.push(format!("%{}%", k.replace(['%', '_'], "")));
            sql.push_str(&format!(
                " AND (title LIKE ?{0} OR note LIKE ?{0})",
                args.len()
            ));
        }
        let limit = f.limit.unwrap_or(QUERY_LIMIT).clamp(1, QUERY_LIMIT);
        sql.push_str(&format!(" ORDER BY status, planned_date, id LIMIT {limit}"));
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(args.iter()), |r| {
                Ok(TaskBrief {
                    id: r.get(0)?,
                    title: r.get(1)?,
                    planned_date: r.get(2)?,
                    status: r.get(3)?,
                    done_date: r.get(4)?,
                    due_at: r.get(5)?,
                    remind_at: r.get(6)?,
                    note: r.get(7)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// 一段时间的完成情况与当前未完成的事，供模型写总结
    pub fn summary_data(&self, from: &str, to: &str) -> AppResult<Summary> {
        let today = self.today().format(DATE_FMT).to_string();
        let mut stmt = self.conn.prepare(
            "SELECT business_date, title, kind, project_title FROM completions
             WHERE business_date >= ?1 AND business_date <= ?2 ORDER BY business_date, id LIMIT 200",
        )?;
        let completed = stmt
            .query_map([from, to], |r| {
                Ok(SummaryItem {
                    date: r.get(0)?,
                    title: r.get(1)?,
                    kind: r.get(2)?,
                    project: r.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let completed_count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM completions WHERE business_date >= ?1 AND business_date <= ?2",
            [from, to],
            |r| r.get(0),
        )?;
        let open_count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM tasks WHERE status = 'open' AND planned_date <= ?1",
            [&today],
            |r| r.get(0),
        )?;
        let overdue = self.query_tasks_brief(&TaskFilter {
            status: Some("open".into()),
            date_to: Some(
                (self.today() - chrono::Days::new(1))
                    .format(DATE_FMT)
                    .to_string(),
            ),
            limit: Some(20),
            ..TaskFilter::default()
        })?;
        Ok(Summary {
            from: from.into(),
            to: to.into(),
            completed_count,
            completed,
            open_count,
            overdue,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::*;
    use super::*;
    use crate::config::AppConfig;
    use rusqlite::params;

    #[test]
    fn 按状态日期关键词查询() {
        let conn = db();
        let clock = clock("2026-10-17 10:00:00");
        let svc = Service::new(&conn, &clock, &AppConfig::default()).unwrap();
        for (t, d) in [
            ("周会", "2026-10-17"),
            ("产品评审会", "2026-10-18"),
            ("买菜", "2026-10-17"),
        ] {
            conn.execute(
                "INSERT INTO tasks (title, planned_date, created_at, updated_at) VALUES (?1, ?2, 'x', 'x')",
                params![t, d],
            )
            .unwrap();
        }
        let f = |k: &str| TaskFilter {
            keyword: Some(k.into()),
            ..TaskFilter::default()
        };
        assert_eq!(svc.query_tasks_brief(&f("会")).unwrap().len(), 2);
        let today_only = TaskFilter {
            status: Some("open".into()),
            date_from: Some("2026-10-17".into()),
            date_to: Some("2026-10-17".into()),
            ..TaskFilter::default()
        };
        assert_eq!(svc.query_tasks_brief(&today_only).unwrap().len(), 2);
        // 通配符不会被当成匹配一切
        assert!(svc.query_tasks_brief(&f("%")).unwrap().len() == 3);
    }

    #[test]
    fn 总结数据() {
        let conn = db();
        let clock = clock("2026-10-17 10:00:00");
        let svc = Service::new(&conn, &clock, &AppConfig::default()).unwrap();
        conn.execute(
            "INSERT INTO completions (business_date, kind, task_id, title, completed_at) VALUES ('2026-10-15', 'task', 1, '跑步', 'x')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO tasks (title, planned_date, created_at, updated_at) VALUES ('预约体检', '2026-10-08', 'x', 'x')",
            [],
        )
        .unwrap();
        let s = svc.summary_data("2026-10-12", "2026-10-17").unwrap();
        assert_eq!(s.completed_count, 1);
        assert_eq!(s.overdue[0].title, "预约体检");
        assert_eq!(s.open_count, 1);
    }
}
