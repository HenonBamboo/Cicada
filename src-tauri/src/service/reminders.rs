//! 提醒与每日回顾的持久去重；系统操作不进入用户撤销栈。
use chrono::NaiveTime;
use rusqlite::{OptionalExtension, params};

use super::Service;
use super::oplog::{self, Entry, Source};
use crate::config::AutomationSettings;
use crate::error::AppResult;

pub struct Reminder {
    pub id: i64,
    pub title: String,
    pub version: i64,
}

impl Service<'_> {
    pub fn due_reminders(&self) -> AppResult<Vec<Reminder>> {
        let mut stmt = self.conn.prepare("SELECT id,title,version FROM tasks WHERE status='open' AND reminded_at IS NULL AND remind_at IS NOT NULL AND remind_at<=?1 ORDER BY remind_at,id")?;
        Ok(stmt
            .query_map([self.now_str()], |r| {
                Ok(Reminder {
                    id: r.get(0)?,
                    title: r.get(1)?,
                    version: r.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?)
    }

    /// 只有成功提交系统通知后才记回执；时间戳不改变业务 version。
    pub fn acknowledge_reminders(&self, reminders: &[Reminder]) -> AppResult<()> {
        let tx = self.conn.unchecked_transaction()?;
        let now = self.now_str();
        let group = oplog::new_group_id();
        for reminder in reminders {
            let before = oplog::row_json(&tx, "tasks", reminder.id)?;
            let changed = tx.execute("UPDATE tasks SET reminded_at=?1 WHERE id=?2 AND version=?3 AND status='open' AND reminded_at IS NULL", params![now, reminder.id, reminder.version])?;
            if changed > 0 {
                oplog::record(
                    &tx,
                    Entry {
                        group_id: &group,
                        source: Source::System,
                        op: "task.reminded",
                        entity: "tasks",
                        entity_id: Some(reminder.id),
                        before,
                        after: oplog::row_json(&tx, "tasks", reminder.id)?,
                        summary: "已发送任务提醒".into(),
                        created_at: &now,
                    },
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn review_due(&self, settings: &AutomationSettings) -> AppResult<Option<(String, i64)>> {
        if !settings.review_enabled {
            return Ok(None);
        }
        let now = self.clock.now();
        let at = NaiveTime::parse_from_str(&settings.review_time, "%H:%M")
            .map_err(|_| crate::error::AppError::Invalid("每日回顾时间格式不正确".into()))?;
        if now.time() < at {
            return Ok(None);
        }
        let date = now.date().to_string();
        let last: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM meta WHERE key='last_review_date'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        if last.as_deref().is_some_and(|d| d >= date.as_str()) {
            return Ok(None);
        }
        let count = self.conn.query_row(
            "SELECT COUNT(*) FROM completions WHERE business_date=?1",
            [self.today().to_string()],
            |r| r.get(0),
        )?;
        Ok(Some((date, count)))
    }

    pub fn acknowledge_review(&self, date: &str) -> AppResult<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("INSERT INTO meta(key,value) VALUES('last_review_date',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [date])?;
        oplog::record(
            &tx,
            Entry {
                group_id: &oplog::new_group_id(),
                source: Source::System,
                op: "review.notify",
                entity: "meta",
                entity_id: None,
                before: None,
                after: None,
                summary: "已发送每日回顾".into(),
                created_at: &self.now_str(),
            },
        )?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use crate::service::testing::{clock, db};

    #[test]
    fn 到点提醒和回顾只发送一次且完成任务不提醒() {
        let db = db();
        db.execute("INSERT INTO tasks(title,planned_date,remind_at,created_at,updated_at) VALUES('开会','2026-10-18','2026-10-18T21:30:00','2026-10-18T10:00:00','2026-10-18T10:00:00')", []).unwrap();
        let config = AppConfig::default();
        let early = clock("2026-10-18 21:29:00");
        let svc = Service::new(&db, &early, &config).unwrap();
        assert!(svc.due_reminders().unwrap().is_empty());
        assert!(svc.review_due(&config.automation).unwrap().is_none());
        let due = clock("2026-10-18 21:30:00");
        let svc = Service::new(&db, &due, &config).unwrap();
        let reminders = svc.due_reminders().unwrap();
        assert_eq!(reminders.len(), 1);
        svc.acknowledge_reminders(&reminders).unwrap();
        assert!(svc.due_reminders().unwrap().is_empty());
        let (date, count) = svc.review_due(&config.automation).unwrap().unwrap();
        assert_eq!(count, 0);
        svc.acknowledge_review(&date).unwrap();
        assert!(svc.review_due(&config.automation).unwrap().is_none());
        db.execute(
            "UPDATE tasks SET reminded_at=NULL,status='done',done_at='2026-10-18T21:30:00'",
            [],
        )
        .unwrap();
        assert!(svc.due_reminders().unwrap().is_empty());
    }
}
