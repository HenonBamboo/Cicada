//! 操作日志：每次写操作记录前后快照，按 group_id 成组撤销（docs/产品设计.md 4.5）

use rusqlite::types::ValueRef;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Map, Value};

use crate::error::AppResult;

/// 操作来源，对应 op_log.source
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// 对话或模型工具操作。
    Chat,
    /// 手动勾选
    Manual,
    /// 日切、重复任务、报告等后台任务
    #[allow(dead_code)]
    System,
    /// 撤销留下的记录，本身不可再撤销
    Undo,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Chat => "chat",
            Source::Manual => "manual",
            Source::System => "system",
            Source::Undo => "undo",
        }
    }
}

pub fn new_group_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// 把一行数据读成 JSON 对象，作为 op_log 的快照。表名只能是内部常量。
pub fn row_json(conn: &Connection, table: &'static str, id: i64) -> AppResult<Option<Value>> {
    let sql = format!("SELECT * FROM {table} WHERE id = ?1");
    let mut stmt = conn.prepare(&sql)?;
    let names: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
    let row = stmt
        .query_row([id], |r| {
            let mut obj = Map::new();
            for (i, name) in names.iter().enumerate() {
                let v = match r.get_ref(i)? {
                    ValueRef::Null => Value::Null,
                    ValueRef::Integer(n) => Value::from(n),
                    ValueRef::Real(f) => Value::from(f),
                    ValueRef::Text(t) => Value::from(String::from_utf8_lossy(t).into_owned()),
                    ValueRef::Blob(_) => Value::Null,
                };
                obj.insert(name.clone(), v);
            }
            Ok(Value::Object(obj))
        })
        .optional()?;
    Ok(row)
}

pub struct Entry<'a> {
    pub group_id: &'a str,
    pub source: Source,
    pub op: &'a str,
    pub entity: &'static str,
    pub entity_id: Option<i64>,
    pub before: Option<Value>,
    pub after: Option<Value>,
    pub summary: String,
    pub created_at: &'a str,
}

pub fn record(conn: &Connection, e: Entry<'_>) -> AppResult<()> {
    conn.execute(
        "INSERT INTO op_log (group_id, source, op, entity, entity_id, before, after, summary, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            e.group_id,
            e.source.as_str(),
            e.op,
            e.entity,
            e.entity_id,
            e.before.map(|v| v.to_string()),
            e.after.map(|v| v.to_string()),
            e.summary,
            e.created_at,
        ],
    )?;
    Ok(())
}
