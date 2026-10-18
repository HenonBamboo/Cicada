//! 业务日切与启动补跑。系统写入不进入用户撤销栈。
use super::oplog::{self, Entry, Source};
use super::write::Writer;
use super::{ChangeSet, DATE_FMT, Service};
use crate::error::{AppError, AppResult};
use chrono::{Days, NaiveDate, NaiveDateTime, NaiveTime};
use rusqlite::{OptionalExtension, params};

#[derive(Debug)]
pub struct RolloverResult {
    pub changes: ChangeSet,
    pub business_date: String,
    pub processed_dates: Vec<String>,
}

fn date(value: &str) -> AppResult<NaiveDate> {
    NaiveDate::parse_from_str(value, DATE_FMT)
        .map_err(|_| AppError::Invalid(format!("日切日期无效：{value}")))
}

impl Service<'_> {
    pub fn rollover(&self) -> AppResult<RolloverResult> {
        let today = self.today();
        let today_text = today.format(DATE_FMT).to_string();
        let tx = self.conn.unchecked_transaction()?;
        let last: Option<String> = tx
            .query_row(
                "SELECT value FROM meta WHERE key='last_rollover_date'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        let initial:String=tx.query_row("SELECT MIN(d) FROM (SELECT ?1 AS d UNION ALL SELECT substr(created_at,1,10) FROM tasks UNION ALL SELECT substr(created_at,1,10) FROM recur_rules)",[&today_text],|r|r.get(0))?;
        let mut next = match last.as_deref() {
            Some(last) => date(last)?.checked_add_days(Days::new(1)),
            None => Some(date(&initial)?),
        };
        let mut result = RolloverResult {
            changes: ChangeSet::default(),
            business_date: today_text.clone(),
            processed_dates: vec![],
        };
        while let Some(day) = next.filter(|d| *d <= today) {
            let business = day.format(DATE_FMT).to_string();
            let mut w = Writer::new(
                &tx,
                format!("rollover:{business}"),
                Source::System,
                self.now_str(),
                business.clone(),
            );
            let mut stmt=tx.prepare("SELECT id,planned_date,postponed_days FROM tasks WHERE status='open' AND planned_date<?1")?;
            let tasks = stmt
                .query_map([&business], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, i64>(2)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            drop(stmt);
            for (id, planned, old) in tasks {
                let days = (day - date(&planned)?).num_days().max(0);
                if days != old {
                    w.update_task(
                        id,
                        &[("postponed_days", Some(days.to_string()))],
                        format!("日切：未完成任务滚入 {business}，延期 {days} 天"),
                    )?;
                }
            }
            if let Some(previous) = day.checked_sub_days(Days::new(1)) {
                let previous = previous.format(DATE_FMT).to_string();
                let mut stmt =
                    tx.prepare("SELECT id FROM tasks WHERE status='done' AND done_date=?1")?;
                let completed = stmt
                    .query_map([previous], |r| r.get::<_, i64>(0))?
                    .collect::<Result<Vec<_>, _>>()?;
                drop(stmt);
                for id in completed {
                    let snapshot = oplog::row_json(&tx, "tasks", id)?;
                    oplog::record(
                        &tx,
                        Entry {
                            group_id: &w.group,
                            source: Source::System,
                            op: "task.archive",
                            entity: "tasks",
                            entity_id: Some(id),
                            before: snapshot.clone(),
                            after: snapshot,
                            summary: "日切：完成任务归档，保留完成历史".into(),
                            created_at: &w.now,
                        },
                    )?;
                    w.changes.tasks.push(id);
                }
            }
            self.generate_recurrences(&mut w, day)?;
            tx.execute("INSERT INTO meta(key,value) VALUES('last_rollover_date',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[&business])?;
            oplog::record(
                &tx,
                Entry {
                    group_id: &w.group,
                    source: Source::System,
                    op: "day.rollover",
                    entity: "meta",
                    entity_id: None,
                    before: None,
                    after: Some(serde_json::json!({"businessDate":business})),
                    summary: format!("完成 {business} 日切"),
                    created_at: &w.now,
                },
            )?;
            result.changes.tasks.extend(w.changes.tasks);
            result.changes.nodes.extend(w.changes.nodes);
            result.processed_dates.push(business);
            next = day.checked_add_days(Days::new(1));
        }
        // 当天刚新增的规则也能生成首条；游标保证每次调度不会重复生成。
        let mut w = Writer::new(
            &tx,
            format!("recur:{today_text}:{}", uuid::Uuid::new_v4()),
            Source::System,
            self.now_str(),
            today_text.clone(),
        );
        self.generate_recurrences(&mut w, today)?;
        result.changes.tasks.extend(w.changes.tasks);
        result.changes.nodes.extend(w.changes.nodes);
        result.changes.tasks.sort_unstable();
        result.changes.tasks.dedup();
        result.changes.nodes.sort_unstable();
        result.changes.nodes.dedup();
        tx.commit()?;
        Ok(result)
    }

    fn generate_recurrences(&self, w: &mut Writer<'_>, through: NaiveDate) -> AppResult<()> {
        for rule in self.query_recur_rules(None)? {
            super::recur::validate(&rule)?;
            let mut next = match rule.last_generated_date.as_deref() {
                Some(last) => date(last)?.checked_add_days(Days::new(1)),
                None => {
                    let created =
                        NaiveDateTime::parse_from_str(&rule.created_at, super::DATETIME_FMT)
                            .map_err(|_| AppError::Invalid("重复规则创建时间无效".into()))?;
                    Some(
                        date(&rule.start_date)?
                            .max(crate::clock::business_date(created, self.cutoff)),
                    )
                }
            };
            let mut advanced = false;
            while let Some(day) = next.filter(|d| *d <= through) {
                if super::recur::matches(&rule, day) {
                    let day_text = day.format(DATE_FMT).to_string();
                    let exists:bool=w.conn.query_row("SELECT EXISTS(SELECT 1 FROM tasks WHERE recur_rule_id=?1 AND planned_date=?2)",params![rule.id,day_text],|r|r.get(0))?;
                    if !exists {
                        let reminder = rule
                            .remind_time
                            .as_ref()
                            .map(|t| -> AppResult<String> {
                                let time = NaiveTime::parse_from_str(t, "%H:%M").map_err(|_| {
                                    AppError::Invalid("重复规则提醒时间无效".into())
                                })?;
                                let reminder_day = if time < self.cutoff {
                                    day.checked_add_days(Days::new(1)).ok_or_else(|| {
                                        AppError::Invalid("提醒日期超出范围".into())
                                    })?
                                } else {
                                    day
                                };
                                Ok(format!("{}T{t}:00", reminder_day.format(DATE_FMT)))
                            })
                            .transpose()?;
                        let task = w.add_task(
                            &rule.title,
                            &day_text,
                            None,
                            reminder.as_deref(),
                            rule.note.as_deref(),
                        )?;
                        w.update_task(
                            task,
                            &[("recur_rule_id", Some(rule.id.to_string()))],
                            format!("按重复规则生成 {day_text} 任务"),
                        )?;
                    }
                }
                advanced = true;
                next = day.checked_add_days(Days::new(1));
            }
            if advanced {
                let before = oplog::row_json(w.conn, "recur_rules", rule.id)?;
                w.conn.execute("UPDATE recur_rules SET last_generated_date=?2,updated_at=?3,version=version+1 WHERE id=?1",params![rule.id,through.format(DATE_FMT).to_string(),w.now])?;
                w.log(
                    "recur.generated",
                    "recur_rules",
                    rule.id,
                    before,
                    "推进重复任务生成日期".into(),
                )?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use crate::service::changes::{Change, TaskRef};
    use crate::service::recur::RecurChange;
    use crate::service::testing::{clock, db};
    use serde_json::json;

    fn apply(s: &Service<'_>, plan: &[Change]) {
        let v = s.validate_plan(plan).unwrap();
        s.apply_plan(
            plan,
            &v.pins,
            &uuid::Uuid::new_v4().to_string(),
            Source::Chat,
        )
        .unwrap();
    }
    fn rule(title: &str, kind: &str, p: serde_json::Value) -> Change {
        Change::Recur {
            change: RecurChange::Add {
                title: title.into(),
                kind: kind.into(),
                params: p,
                remind_time: Some("09:00".into()),
                note: None,
                start_date: "2028-02-28".into(),
                end_date: None,
            },
        }
    }
    fn update(id: i64, title: &str) -> Change {
        Change::Recur {
            change: RecurChange::Update {
                id,
                title: Some(title.into()),
                kind: None,
                params: None,
                remind_time: None,
                note: None,
                start_date: None,
                end_date: None,
                active: None,
            },
        }
    }

    #[test]
    fn 五种重复规则闰日补跑幂等及规则修改删除撤销() {
        let db = db();
        let config = AppConfig::default();
        let c = clock("2028-02-28 08:00:00");
        let s = Service::new(&db, &c, &config).unwrap();
        apply(
            &s,
            &[
                rule("每天", "daily", json!({})),
                rule("工作日", "weekdays", json!({})),
                rule("每周一", "weekly", json!({"weekday":1})),
                rule("每月29", "monthly", json!({"day":29})),
                rule("闰日", "yearly", json!({"month":2,"day":29})),
            ],
        );
        assert!(
            s.validate_plan(&[rule("错误", "yearly", json!({"month":2,"day":30}))])
                .is_err()
        );
        let daily = s.query_recur_rules(Some("每天")).unwrap()[0].id;
        let change = update(daily, "每天改名");
        let old = s.validate_plan(&[change.clone()]).unwrap();
        s.rollover().unwrap();
        assert!(matches!(
            s.apply_plan(&[change.clone()], &old.pins, "stale-rule", Source::Chat),
            Err(AppError::Conflict(_))
        ));
        apply(&s, &[change]);
        let later = clock("2028-03-01 08:00:00");
        let s = Service::new(&db, &later, &config).unwrap();
        let run = s.rollover().unwrap();
        assert_eq!(run.processed_dates, ["2028-02-29", "2028-03-01"]);
        let total: i64 = db
            .query_row("SELECT COUNT(*) FROM tasks", [], |r| r.get(0))
            .unwrap();
        assert_eq!(total, 9);
        assert!(s.rollover().unwrap().changes.tasks.is_empty());
        apply(&s, &[Change::Undo]);
        assert_eq!(s.query_recur_rules(Some("每天")).unwrap()[0].title, "每天");
        assert_eq!(
            s.query_recur_rules(Some("每天")).unwrap()[0]
                .last_generated_date
                .as_deref(),
            Some("2028-03-01")
        );
        assert!(s.rollover().unwrap().changes.tasks.is_empty());
        let monthly = s.query_recur_rules(Some("每月29")).unwrap()[0].id;
        let deletion = Change::Recur {
            change: RecurChange::Delete { id: monthly },
        };
        assert!(s.validate_plan(&[deletion.clone()]).unwrap().dangerous);
        apply(&s, &[deletion]);
        assert_eq!(s.query_recur_rules(None).unwrap().len(), 4);
        apply(&s, &[Change::Undo]);
        assert_eq!(s.query_recur_rules(None).unwrap().len(), 5);
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM tasks", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            9
        );
    }

    #[test]
    fn 自定义日切补跑归档延期且系统更新不阻断用户撤销() {
        let db = db();
        let mut config = AppConfig::default();
        config.time.day_cutoff = "04:00".into();
        let c = clock("2026-10-17 02:30:00");
        let s = Service::new(&db, &c, &config).unwrap();
        let add = |title: &str| Change::TaskAdd {
            title: title.into(),
            planned_date: "2026-10-16".into(),
            due_at: None,
            remind_at: Some("2026-10-17T03:00:00".into()),
            note: None,
        };
        apply(
            &s,
            &[
                add("未完"),
                add("完成"),
                Change::Recur {
                    change: RecurChange::Add {
                        title: "日切前提醒".into(),
                        kind: "daily".into(),
                        params: json!({}),
                        remind_time: Some("03:00".into()),
                        note: None,
                        start_date: "2026-10-16".into(),
                        end_date: None,
                    },
                },
            ],
        );
        let tasks = s.task_board().unwrap().today;
        let first = tasks.iter().find(|t| t.title == "未完").unwrap();
        let second = tasks.iter().find(|t| t.title == "完成").unwrap();
        s.set_task_done(second.id, second.version, true).unwrap();
        apply(
            &s,
            &[Change::TaskUpdate {
                task: TaskRef::Id(first.id),
                title: Some("改名".into()),
                planned_date: None,
                due_at: None,
                remind_at: None,
                note: None,
            }],
        );
        s.rollover().unwrap();
        assert_eq!(db.query_row("SELECT remind_at FROM tasks WHERE recur_rule_id IS NOT NULL AND planned_date='2026-10-16'",[],|r|r.get::<_,String>(0)).unwrap(),"2026-10-17T03:00:00");
        let later = clock("2026-10-19 05:00:00");
        let s = Service::new(&db, &later, &config).unwrap();
        let run = s.rollover().unwrap();
        assert_eq!(
            run.processed_dates,
            ["2026-10-17", "2026-10-18", "2026-10-19"]
        );
        assert_eq!(s.task_board().unwrap().legacy[0].delay_days, 3);
        assert!(s.task_board().unwrap().done.is_empty());
        assert_eq!(s.day_completions("2026-10-16").unwrap().len(), 1);
        db.execute(
            "UPDATE tasks SET reminded_at='2026-10-19T05:00:00' WHERE id=?1",
            [first.id],
        )
        .unwrap();
        apply(&s, &[Change::Undo]);
        let task = &s.task_board().unwrap().legacy[0];
        assert_eq!(task.title, "未完");
        assert_eq!(task.delay_days, 3);
        assert_eq!(
            db.query_row(
                "SELECT postponed_days FROM tasks WHERE id=?1",
                [first.id],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            3
        );
        assert_eq!(
            db.query_row(
                "SELECT reminded_at FROM tasks WHERE id=?1",
                [first.id],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "2026-10-19T05:00:00"
        );
        assert!(s.rollover().unwrap().processed_dates.is_empty());
        let reminder = |time: &str| Change::TaskUpdate {
            task: TaskRef::Id(first.id),
            title: None,
            planned_date: None,
            due_at: None,
            remind_at: Some(time.into()),
            note: None,
        };
        apply(&s, &[reminder("2026-10-17T03:00:00")]);
        assert!(
            db.query_row(
                "SELECT reminded_at IS NOT NULL FROM tasks WHERE id=?1",
                [first.id],
                |r| r.get::<_, bool>(0)
            )
            .unwrap()
        );
        apply(&s, &[reminder("2026-10-19T06:00:00")]);
        assert!(
            db.query_row(
                "SELECT reminded_at IS NULL FROM tasks WHERE id=?1",
                [first.id],
                |r| r.get::<_, bool>(0)
            )
            .unwrap()
        );
    }
}
