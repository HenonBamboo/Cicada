//! 统一业务服务层（docs/产品设计.md 4.5）
//!
//! 任务与项目的所有规则只在这里实现：分区、联动、完成记录、版本校验、op_log。
//! 模型工具（agent）、手动勾选（commands）、定时任务（scheduler）都只调用本层，
//! 不直接写数据库。

pub mod changes;
pub mod data;
pub mod inbox;
pub mod model;
pub mod oplog;
pub mod project_changes;
mod project_write;
mod projects;
pub mod query;
pub mod recur;
pub mod reminders;
mod rollover;
mod stats;
pub mod stickies;
mod tasks;
mod undo;
mod write;

use chrono::{NaiveDate, NaiveTime};
use rusqlite::Connection;

use crate::clock::{Clock, parse_cutoff};
use crate::config::AppConfig;
use crate::error::AppResult;

pub use model::*;
pub use rollover::RolloverResult;

pub const DATE_FMT: &str = "%Y-%m-%d";
pub const DATETIME_FMT: &str = "%Y-%m-%dT%H:%M:%S";

pub struct Service<'a> {
    conn: &'a Connection,
    clock: &'a dyn Clock,
    cutoff: NaiveTime,
    sticky_defaults: crate::config::StickySettings,
}

impl<'a> Service<'a> {
    pub fn new(conn: &'a Connection, clock: &'a dyn Clock, config: &AppConfig) -> AppResult<Self> {
        Ok(Self {
            conn,
            clock,
            cutoff: parse_cutoff(&config.time.day_cutoff)?,
            sticky_defaults: config.sticky.clone(),
        })
    }

    /// 当前业务日期。所有"今天"都从这里取。
    pub fn today(&self) -> NaiveDate {
        self.clock.business_date(self.cutoff)
    }

    fn now_str(&self) -> String {
        self.clock.now().format(DATETIME_FMT).to_string()
    }

    /// 主界面需要的全部数据
    pub fn board(&self) -> AppResult<Board> {
        Ok(Board {
            business_date: self.today().format(DATE_FMT).to_string(),
            tasks: self.task_board()?,
            projects: self.project_tree()?,
        })
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use crate::clock::FixedClock;
    use chrono::NaiveDateTime;

    pub fn clock(s: &str) -> FixedClock {
        FixedClock(NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap())
    }

    pub fn db() -> Connection {
        let mut conn = crate::db::open_in_memory().unwrap();
        crate::db::migrate(&mut conn, |_, _| Ok(())).unwrap();
        conn
    }

    pub fn count(conn: &Connection, sql: &str) -> i64 {
        conn.query_row(sql, [], |r| r.get(0)).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;

    #[test]
    fn 今天按设置的日切时间计算() {
        let conn = db();
        let clock = clock("2026-10-18 03:00:00");
        let mut config = AppConfig::default();
        let today = |c: &AppConfig| Service::new(&conn, &clock, c).unwrap().today().to_string();
        assert_eq!(today(&config), "2026-10-18");
        config.time.day_cutoff = "04:00".into();
        assert_eq!(today(&config), "2026-10-17");
    }
}
