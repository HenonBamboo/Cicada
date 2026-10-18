//! 重复规则的查询、只读计划校验与事务写入。
use super::changes::{Pin, PreviewItem};
use super::oplog::{self, Entry};
use super::write::Writer;
use super::{DATE_FMT, Service};
use crate::error::{AppError, AppResult};
use chrono::{Datelike, NaiveDate, NaiveTime};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum RecurChange {
    Add {
        title: String,
        kind: String,
        params: Value,
        remind_time: Option<String>,
        note: Option<String>,
        start_date: String,
        end_date: Option<String>,
    },
    Update {
        id: i64,
        title: Option<String>,
        kind: Option<String>,
        params: Option<Value>,
        remind_time: Option<String>,
        note: Option<String>,
        start_date: Option<String>,
        end_date: Option<String>,
        active: Option<bool>,
    },
    Delete {
        id: i64,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecurRule {
    pub id: i64,
    pub title: String,
    pub kind: String,
    pub params: Value,
    pub remind_time: Option<String>,
    pub note: Option<String>,
    pub start_date: String,
    pub end_date: Option<String>,
    pub active: bool,
    pub last_generated_date: Option<String>,
    pub version: i64,
    pub created_at: String,
}

impl Service<'_> {
    pub fn query_recur_rules(&self, keyword: Option<&str>) -> AppResult<Vec<RecurRule>> {
        let mut stmt=self.conn.prepare("SELECT id,title,kind,params,remind_time,note,start_date,end_date,active,last_generated_date,version,created_at FROM recur_rules WHERE ?1 IS NULL OR instr(title,?1)>0 ORDER BY id")?;
        Ok(stmt
            .query_map([keyword], |r| {
                Ok(RecurRule {
                    id: r.get(0)?,
                    title: r.get(1)?,
                    kind: r.get(2)?,
                    params: serde_json::from_str(&r.get::<_, String>(3)?).unwrap_or(Value::Null),
                    remind_time: r.get(4)?,
                    note: r.get(5)?,
                    start_date: r.get(6)?,
                    end_date: r.get(7)?,
                    active: r.get(8)?,
                    last_generated_date: r.get(9)?,
                    version: r.get(10)?,
                    created_at: r.get(11)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?)
    }
}

fn invalid(s: &str) -> AppError {
    AppError::Invalid(s.into())
}
pub(super) fn validate(rule: &RecurRule) -> AppResult<()> {
    if rule.title.trim().is_empty() || rule.title.chars().count() > 200 {
        return Err(invalid("重复任务标题须为1到200字"));
    }
    let p = &rule.params;
    let int = |k: &str| p.get(k).and_then(Value::as_u64);
    let valid = match rule.kind.as_str() {
        "daily" | "weekdays" => p.is_object(),
        "weekly" => int("weekday").is_some_and(|d| (1..=7).contains(&d)),
        "monthly" => int("day").is_some_and(|d| (1..=31).contains(&d)),
        "yearly" => match (int("month"), int("day")) {
            (Some(m), Some(d)) if m <= 12 && d <= 31 => {
                NaiveDate::from_ymd_opt(2000, m as u32, d as u32).is_some()
            }
            _ => false,
        },
        _ => false,
    };
    if !valid {
        return Err(invalid(
            "重复规则参数无效；周使用weekday 1到7，月使用day，年使用month和day",
        ));
    }
    let start = NaiveDate::parse_from_str(&rule.start_date, DATE_FMT)
        .map_err(|_| invalid("开始日期格式应为YYYY-MM-DD"))?;
    if let Some(end) = rule.end_date.as_deref() {
        let end = NaiveDate::parse_from_str(end, DATE_FMT)
            .map_err(|_| invalid("结束日期格式应为YYYY-MM-DD"))?;
        if end < start {
            return Err(invalid("结束日期不能早于开始日期"));
        }
    }
    if let Some(t) = rule.remind_time.as_deref() {
        if t.len() != 5 || NaiveTime::parse_from_str(t, "%H:%M").is_err() {
            return Err(invalid("提醒时间应为HH:MM"));
        }
    }
    Ok(())
}

pub(super) fn matches(rule: &RecurRule, day: NaiveDate) -> bool {
    let date = day.format(DATE_FMT).to_string();
    if !rule.active
        || date < rule.start_date
        || rule.end_date.as_ref().is_some_and(|end| date > *end)
    {
        return false;
    }
    let p = |k: &str| rule.params.get(k).and_then(Value::as_u64).unwrap_or(0) as u32;
    match rule.kind.as_str() {
        "daily" => true,
        "weekdays" => day.weekday().number_from_monday() <= 5,
        "weekly" => day.weekday().number_from_monday() == p("weekday"),
        "monthly" => day.day() == p("day"),
        "yearly" => day.month() == p("month") && day.day() == p("day"),
        _ => false,
    }
}

pub(super) struct RecurValidation {
    rules: HashMap<i64, RecurRule>,
    original: HashMap<i64, RecurRule>,
    pub pins: Vec<Pin>,
    pub preview: Vec<PreviewItem>,
    pub touched: HashSet<i64>,
    pub touched_tasks: HashSet<i64>,
    pub dangerous: bool,
}
impl RecurValidation {
    pub fn new(s: &Service<'_>) -> AppResult<Self> {
        let rules: HashMap<_, _> = s
            .query_recur_rules(None)?
            .into_iter()
            .map(|r| (r.id, r))
            .collect();
        Ok(Self {
            original: rules.clone(),
            rules,
            pins: vec![],
            preview: vec![],
            touched: HashSet::new(),
            touched_tasks: HashSet::new(),
            dangerous: false,
        })
    }
    pub fn change(&mut self, svc: &Service<'_>, c: &RecurChange) -> AppResult<()> {
        let mut rule = match c {
            RecurChange::Add {
                title,
                kind,
                params,
                remind_time,
                note,
                start_date,
                end_date,
            } => RecurRule {
                id: 0,
                title: title.trim().into(),
                kind: kind.clone(),
                params: params.clone(),
                remind_time: remind_time.clone().filter(|s| !s.is_empty()),
                note: note.clone(),
                start_date: start_date.clone(),
                end_date: end_date.clone().filter(|s| !s.is_empty()),
                active: true,
                last_generated_date: None,
                version: 1,
                created_at: svc.now_str(),
            },
            RecurChange::Update { id, .. } | RecurChange::Delete { id } => {
                let r = self
                    .rules
                    .get(id)
                    .cloned()
                    .ok_or_else(|| invalid("重复规则不存在或已在本轮删除"))?;
                if self.touched.insert(*id) {
                    self.pins.push(Pin {
                        entity: "recur_rules".into(),
                        id: *id,
                        version: self.original[id].version,
                    });
                }
                r
            }
        };
        if let RecurChange::Update {
            title,
            kind,
            params,
            remind_time,
            note,
            start_date,
            end_date,
            active,
            ..
        } = c
        {
            if let Some(v) = title {
                rule.title = v.trim().into();
            }
            if let Some(v) = kind {
                rule.kind = v.clone();
            }
            if let Some(v) = params {
                rule.params = v.clone();
            }
            if let Some(v) = remind_time {
                rule.remind_time = (!v.is_empty()).then(|| v.clone());
            }
            if let Some(v) = note {
                rule.note = (!v.is_empty()).then(|| v.clone());
            }
            if let Some(v) = start_date {
                rule.start_date = v.clone();
            }
            if let Some(v) = end_date {
                rule.end_date = (!v.is_empty()).then(|| v.clone());
            }
            if let Some(v) = active {
                rule.active = *v;
            }
        }
        validate(&rule)?;
        let (verb, danger) = match c {
            RecurChange::Add { .. } => ("添加", false),
            RecurChange::Update { .. } => ("修改", false),
            RecurChange::Delete { .. } => ("删除", true),
        };
        self.preview.push(PreviewItem {
            icon: if danger { "trash" } else { "repeat" }.into(),
            text: format!(
                "{verb}重复规则「{}」（{}）",
                rule.title,
                super::tasks::recur_label(&rule.kind, &rule.params.to_string())
            ),
            danger,
        });
        self.dangerous |= danger;
        if let RecurChange::Delete { id } = c {
            let mut stmt = svc
                .conn
                .prepare("SELECT id,title,version FROM tasks WHERE recur_rule_id=?1")?;
            for row in stmt.query_map([id], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })? {
                let (id, title, version) = row?;
                self.pins.push(Pin {
                    entity: "tasks".into(),
                    id,
                    version,
                });
                self.touched_tasks.insert(id);
                self.preview.push(PreviewItem {
                    icon: "link".into(),
                    text: format!("保留已生成任务「{title}」，解除重复规则关联"),
                    danger: false,
                });
            }
            self.rules.remove(id);
        } else if rule.id != 0 {
            self.rules.insert(rule.id, rule);
        }
        Ok(())
    }
}

impl Writer<'_> {
    pub(super) fn recur_change(&mut self, c: &RecurChange) -> AppResult<()> {
        match c {
            RecurChange::Add {
                title,
                kind,
                params,
                remind_time,
                note,
                start_date,
                end_date,
            } => {
                self.conn.execute("INSERT INTO recur_rules(title,kind,params,remind_time,note,start_date,end_date,created_at,updated_at) VALUES(?1,?2,?3,NULLIF(?4,''),NULLIF(?5,''),?6,NULLIF(?7,''),?8,?8)",params![title.trim(),kind,params.to_string(),remind_time,note,start_date,end_date,self.now])?;
                let id = self.conn.last_insert_rowid();
                self.log(
                    "recur.add",
                    "recur_rules",
                    id,
                    None,
                    format!("添加重复规则「{title}」"),
                )?;
            }
            RecurChange::Update {
                id,
                title,
                kind,
                params: parameters,
                remind_time,
                note,
                start_date,
                end_date,
                active,
            } => {
                let before = oplog::row_json(self.conn, "recur_rules", *id)?;
                self.conn.execute("UPDATE recur_rules SET title=COALESCE(?2,title),kind=COALESCE(?3,kind),params=COALESCE(?4,params),remind_time=CASE WHEN ?5 IS NULL THEN remind_time ELSE NULLIF(?5,'') END,note=CASE WHEN ?6 IS NULL THEN note ELSE NULLIF(?6,'') END,start_date=COALESCE(?7,start_date),end_date=CASE WHEN ?8 IS NULL THEN end_date ELSE NULLIF(?8,'') END,active=COALESCE(?9,active),updated_at=?10,version=version+1 WHERE id=?1",params![id,title,kind,parameters.as_ref().map(Value::to_string),remind_time,note,start_date,end_date,active,self.now])?;
                self.log(
                    "recur.update",
                    "recur_rules",
                    *id,
                    before,
                    "修改重复规则".into(),
                )?;
                // 任务区从规则读取频率标签；只通知刷新，不修改已生成任务。
                let mut stmt = self
                    .conn
                    .prepare("SELECT id FROM tasks WHERE recur_rule_id=?1")?;
                let tasks = stmt
                    .query_map([id], |r| r.get::<_, i64>(0))?
                    .collect::<Result<Vec<_>, _>>()?;
                drop(stmt);
                for task in tasks {
                    self.touch_task(task);
                }
            }
            RecurChange::Delete { id } => {
                let mut stmt = self
                    .conn
                    .prepare("SELECT id FROM tasks WHERE recur_rule_id=?1")?;
                let tasks = stmt
                    .query_map([id], |r| r.get::<_, i64>(0))?
                    .collect::<Result<Vec<_>, _>>()?;
                drop(stmt);
                for task in tasks {
                    self.update_task(
                        task,
                        &[("recur_rule_id", None)],
                        "解除重复规则关联，保留任务".into(),
                    )?;
                }
                let before = oplog::row_json(self.conn, "recur_rules", *id)?;
                self.conn
                    .execute("DELETE FROM recur_rules WHERE id=?1", [id])?;
                oplog::record(
                    self.conn,
                    Entry {
                        group_id: &self.group,
                        source: self.source,
                        op: "recur.delete",
                        entity: "recur_rules",
                        entity_id: Some(*id),
                        before,
                        after: None,
                        summary: "删除重复规则".into(),
                        created_at: &self.now,
                    },
                )?;
            }
        }
        Ok(())
    }
}
