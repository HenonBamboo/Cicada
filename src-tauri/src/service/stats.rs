//! 热力图与完成清单，数据来自 completions（docs/产品设计.md 第 6 节）

use chrono::{Days, NaiveDate};

use super::{DATE_FMT, DayCompletion, HeatDay, Service};
use crate::error::{AppError, AppResult};

/// 热力图覆盖的天数：53 周，保证从某个周一开始能铺满近一年
pub const HEAT_DAYS: u64 = 53 * 7;

impl Service<'_> {
    /// 近一年每天的完成数（只返回有记录的日子）
    pub fn heatmap(&self) -> AppResult<Vec<HeatDay>> {
        let today = self.today();
        let from = today
            .checked_sub_days(Days::new(HEAT_DAYS))
            .unwrap_or(today);
        let mut stmt = self.conn.prepare(
            "SELECT business_date,
                    SUM(kind = 'task'),
                    SUM(kind = 'node')
             FROM completions
             WHERE business_date > ?1 AND business_date <= ?2
             GROUP BY business_date ORDER BY business_date",
        )?;
        let days = stmt
            .query_map(
                [
                    from.format(DATE_FMT).to_string(),
                    today.format(DATE_FMT).to_string(),
                ],
                |r| {
                    Ok(HeatDay {
                        date: r.get(0)?,
                        tasks: r.get(1)?,
                        nodes: r.get(2)?,
                    })
                },
            )?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(days)
    }

    /// 某一天的完成清单（点击热力图时在回复区显示）
    pub fn day_completions(&self, date: &str) -> AppResult<Vec<DayCompletion>> {
        NaiveDate::parse_from_str(date, DATE_FMT)
            .map_err(|_| AppError::Invalid(format!("日期格式不正确：{date}")))?;
        let mut stmt = self.conn.prepare(
            "SELECT kind, title, project_title FROM completions
             WHERE business_date = ?1 ORDER BY completed_at, id",
        )?;
        let list = stmt
            .query_map([date], |r| {
                Ok(DayCompletion {
                    kind: r.get(0)?,
                    title: r.get(1)?,
                    project_title: r.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(list)
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::*;
    use super::*;
    use crate::config::AppConfig;
    use rusqlite::params;

    #[test]
    fn 按天汇总并分开任务与项目() {
        let conn = db();
        let clock = clock("2026-10-17 10:00:00");
        let svc = Service::new(&conn, &clock, &AppConfig::default()).unwrap();
        let add = |date: &str, kind: &str, title: &str| {
            conn.execute(
                "INSERT INTO completions (business_date, kind, task_id, title, completed_at) VALUES (?1, ?2, 1, ?3, ?1)",
                params![date, kind, title],
            )
            .unwrap();
        };
        add("2026-10-17", "task", "跑步");
        add("2026-10-17", "node", "主界面");
        add("2026-10-17", "task", "买猫粮");
        add("2026-10-01", "task", "交房租");
        add("2025-01-01", "task", "太久以前");

        let heat = svc.heatmap().unwrap();
        assert_eq!(heat.len(), 2);
        assert_eq!(
            heat[1],
            HeatDay {
                date: "2026-10-17".into(),
                tasks: 2,
                nodes: 1
            }
        );
        let list = svc.day_completions("2026-10-17").unwrap();
        assert_eq!(list.len(), 3);
        assert!(svc.day_completions("昨天").is_err());
    }
}
