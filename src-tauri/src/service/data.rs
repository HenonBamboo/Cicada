//! 可移植业务数据。只接受本应用的版本化格式，追加导入，不读取导入文件中的路径。
use std::collections::{BTreeMap, HashMap};

use chrono::{NaiveDate, NaiveDateTime};
use rusqlite::{Connection, params_from_iter, types::Value as SqlValue};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::{DATE_FMT, DATETIME_FMT, Service, oplog};
use crate::error::{AppError, AppResult};

const TABLES: [&str; 4] = ["nodes", "recur_rules", "tasks", "completions"];
const MAX_BYTES: usize = 10 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Transfer {
    format: String,
    version: u32,
    tables: BTreeMap<String, Vec<Map<String, Value>>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportFile {
    pub file_name: String,
    pub mime_type: String,
    pub content: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub tasks: usize,
    pub nodes: usize,
    pub recur_rules: usize,
    pub completions: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationLog {
    id: i64,
    source: String,
    operation: String,
    entity: String,
    summary: String,
    created_at: String,
    undone_at: Option<String>,
}

fn invalid(s: &str) -> AppError {
    AppError::Invalid(format!("数据导入失败：{s}"))
}

fn markdown_text(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\\' | '`' | '*' | '_' | '[' | ']' | '#' | '!' | '|' => {
                out.push('\\');
                out.push(c);
            }
            '\r' | '\n' => out.push(' '),
            _ => out.push(c),
        }
    }
    out
}

fn columns(conn: &Connection, table: &str) -> AppResult<Vec<(String, String)>> {
    Ok(conn
        .prepare(&format!("PRAGMA table_info({table})"))?
        .query_map([], |r| Ok((r.get(1)?, r.get(2)?)))?
        .collect::<Result<_, _>>()?)
}

fn insert(conn: &Connection, table: &str, row: &Map<String, Value>) -> AppResult<()> {
    // 表名来自 TABLES；字段只能来自当前数据库结构，绝不使用导入内容拼 SQL。
    let cols = columns(conn, table)?;
    if row.len() != cols.len() || cols.iter().any(|(n, _)| !row.contains_key(n)) {
        return Err(invalid("字段不符合当前导出格式"));
    }
    let values = cols
        .iter()
        .map(|(name, kind)| match (&row[name], kind.as_str()) {
            (Value::Null, _) => Ok(SqlValue::Null),
            (Value::String(s), "TEXT") if s.len() <= 100_000 => Ok(SqlValue::Text(s.clone())),
            (Value::Number(n), "INTEGER") => n
                .as_i64()
                .map(SqlValue::Integer)
                .ok_or_else(|| invalid("整数超出范围")),
            _ => Err(invalid("字段类型无效")),
        })
        .collect::<AppResult<Vec<_>>>()?;
    let names = cols
        .iter()
        .map(|(n, _)| n.as_str())
        .collect::<Vec<_>>()
        .join(",");
    let marks = vec!["?"; cols.len()].join(",");
    conn.execute(
        &format!("INSERT INTO {table} ({names}) VALUES ({marks})"),
        params_from_iter(values),
    )?;
    Ok(())
}

fn validate_row(table: &str, row: &Map<String, Value>) -> AppResult<()> {
    if !row
        .get("id")
        .and_then(Value::as_i64)
        .is_some_and(|v| v > 0 && v < i64::MAX / 2)
    {
        return Err(invalid("记录编号无效"));
    }
    if !row
        .get("title")
        .and_then(Value::as_str)
        .is_some_and(|s| !s.trim().is_empty() && s.chars().count() <= 200)
    {
        return Err(invalid("标题须为1到200字"));
    }
    if table != "completions"
        && !row
            .get("version")
            .and_then(Value::as_i64)
            .is_some_and(|n| n > 0 && n < i64::MAX / 2)
    {
        return Err(invalid("记录版本无效"));
    }
    for (key, value) in row {
        if value.is_null() {
            continue;
        }
        if key.ends_with("_date") {
            let s = value.as_str().ok_or_else(|| invalid("日期类型无效"))?;
            if s.len() != 10 || NaiveDate::parse_from_str(s, DATE_FMT).is_err() {
                return Err(invalid("日期无效"));
            }
        } else if key.ends_with("_at") {
            let s = value.as_str().ok_or_else(|| invalid("时间类型无效"))?;
            if s.len() != 19 || NaiveDateTime::parse_from_str(s, DATETIME_FMT).is_err() {
                return Err(invalid("时间无效"));
            }
        }
    }
    if table == "tasks" && ((row["status"] == "done") != !row["done_date"].is_null()) {
        return Err(invalid("任务完成日期不一致"));
    }
    Ok(())
}

impl Transfer {
    pub fn parse(content: &str) -> AppResult<Self> {
        if content.len() > MAX_BYTES {
            return Err(invalid("文件超过10MB"));
        }
        let mut data: Self =
            serde_json::from_str(content).map_err(|_| invalid("仅支持知了导出的JSON业务数据"))?;
        if data.format != "cicada-business-data"
            || data.version != 1
            || data.tables.len() != 4
            || TABLES.iter().any(|t| !data.tables.contains_key(*t))
        {
            return Err(invalid("文件格式或版本不支持"));
        }
        if data.tables.values().map(Vec::len).sum::<usize>() > 50_000 {
            return Err(invalid("记录数超过50000"));
        }
        data.tables
            .get_mut("nodes")
            .unwrap()
            .sort_by_key(|r| r.get("depth").and_then(Value::as_i64).unwrap_or(0));
        // 独立数据库先校验所有类型、CHECK、唯一约束及外键；失败不接触用户数据。
        let mut check = Connection::open_in_memory()?;
        check.pragma_update(None, "foreign_keys", "ON")?;
        crate::db::migrate(&mut check, |_, _| Ok(()))?;
        for table in TABLES {
            for row in &data.tables[table] {
                insert(&check, table, row).map_err(|_| invalid("字段、引用或唯一约束无效"))?;
                validate_row(table, row)?;
            }
        }
        let bad: bool = check.query_row("SELECT EXISTS(SELECT 1 FROM nodes n JOIN nodes p ON n.parent_id=p.id WHERE n.depth!=p.depth+1) OR EXISTS(SELECT 1 FROM tasks t JOIN nodes n ON t.node_id=n.id WHERE t.status='open' AND n.status='done') OR EXISTS(SELECT 1 FROM nodes WHERE (status='done')!=(done_at IS NOT NULL) OR (project_status='done' AND status!='done')) OR EXISTS(SELECT 1 FROM completions WHERE task_id=0 OR node_id=0 OR (kind='task' AND task_id IS NULL) OR (kind='node' AND node_id IS NULL))", [], |r|r.get(0))?;
        if bad {
            return Err(invalid("项目层级或任务节点状态不一致"));
        }
        let config = crate::config::AppConfig::default();
        let clock = crate::clock::SystemClock;
        for rule in Service::new(&check, &clock, &config)?.query_recur_rules(None)? {
            super::recur::validate(&rule)?;
        }
        Ok(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::testing::{clock, count, db};

    #[test]
    fn 数据往返追加保持关联且无效导入不改数据() {
        let conn = db();
        let clock = clock("2026-10-18 10:00:00");
        let config = crate::config::AppConfig::default();
        let s = Service::new(&conn, &clock, &config).unwrap();
        conn.execute_batch("INSERT INTO nodes(id,depth,title,status,project_status,created_at,updated_at,done_at) VALUES(1,1,'项目','done','done','2026-10-18T09:00:00','2026-10-18T09:00:00','2026-10-18T09:00:00'); INSERT INTO tasks(id,title,planned_date,status,node_id,created_at,updated_at,done_at,done_date) VALUES(1,'任务','2026-10-18','done',1,'2026-10-18T09:00:00','2026-10-18T09:00:00','2026-10-18T09:00:00','2026-10-18'); INSERT INTO completions(business_date,kind,task_id,node_id,title,completed_at) VALUES('2026-10-18','task',1,1,'任务','2026-10-18T09:00:00'),('2026-10-17','task',99,NULL,'已删除任务','2026-10-17T09:00:00'); INSERT INTO recur_rules(title,kind,start_date,created_at,updated_at) VALUES('每日任务','daily','2026-10-18','2026-10-18T09:00:00','2026-10-18T09:00:00');").unwrap();
        let json = s.export_data("json").unwrap().content;
        let result = s.import_data(Transfer::parse(&json).unwrap()).unwrap();
        assert_eq!(
            (
                result.tasks,
                result.nodes,
                result.recur_rules,
                result.completions
            ),
            (1, 1, 1, 2)
        );
        assert_eq!(
            count(
                &conn,
                "SELECT count(*) FROM tasks WHERE id=100 AND node_id=2"
            ),
            1
        );
        assert_eq!(
            count(
                &conn,
                "SELECT count(*) FROM completions WHERE task_id=100 AND node_id=2"
            ),
            1
        );
        assert_eq!(
            count(&conn, "SELECT count(*) FROM completions WHERE task_id<0"),
            1
        );
        assert_eq!(count(&conn, "SELECT count(*) FROM tasks WHERE id=1"), 1);
        // 负编号历史仍能再次导出、校验；不与未来自增记录绑定。
        Transfer::parse(&s.export_data("json").unwrap().content).unwrap();
        let baseline = s.export_data("json").unwrap().content;
        for mutation in 0..4 {
            let mut invalid: Value = serde_json::from_str(&json).unwrap();
            match mutation {
                0 => invalid["tables"]["tasks"][0]["node_id"] = 999.into(),
                1 => invalid["tables"]["tasks"][0]["planned_date"] = "2026-02-30".into(),
                2 => invalid["tables"]["nodes"][0]["depth"] = 3.into(),
                _ => invalid["tables"]["tasks"][0]["path"] = "../../config.json".into(),
            }
            assert!(Transfer::parse(&invalid.to_string()).is_err());
            assert_eq!(s.export_data("json").unwrap().content, baseline);
        }
        assert!(s.export_data("markdown").unwrap().content.contains("任务"));
        assert_eq!(s.operation_log().unwrap().len(), 1);
    }
}

impl Service<'_> {
    fn transfer(&self) -> AppResult<Transfer> {
        let mut tables = BTreeMap::new();
        for table in TABLES {
            let ids = self
                .conn
                .prepare(&format!("SELECT id FROM {table} ORDER BY id"))?
                .query_map([], |r| r.get::<_, i64>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            let mut rows = vec![];
            for id in ids {
                if let Some(Value::Object(row)) = oplog::row_json(self.conn, table, id)? {
                    rows.push(row);
                }
            }
            tables.insert(table.to_string(), rows);
        }
        Ok(Transfer {
            format: "cicada-business-data".into(),
            version: 1,
            tables,
        })
    }

    pub fn export_data(&self, format: &str) -> AppResult<ExportFile> {
        let data = self.transfer()?;
        let (extension, mime, content) = match format {
            "json" => (
                "json",
                "application/json",
                serde_json::to_string_pretty(&data)?,
            ),
            "markdown" => {
                let mut out = String::from(
                    "# 知了数据导出\n\n包含任务、项目、重复规则和完成历史。导入请使用 JSON 文件；完整迁移请使用备份恢复。\n",
                );
                for (table, label) in [
                    ("nodes", "项目与节点"),
                    ("tasks", "任务"),
                    ("recur_rules", "重复规则"),
                    ("completions", "完成历史"),
                ] {
                    out.push_str(&format!("\n## {label}\n\n"));
                    for row in &data.tables[table] {
                        let title = markdown_text(row["title"].as_str().unwrap_or(""));
                        let date = row
                            .get("planned_date")
                            .or_else(|| row.get("business_date"))
                            .or_else(|| row.get("start_date"))
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        let mark = if row.get("status").and_then(Value::as_str) == Some("done") {
                            "x"
                        } else {
                            " "
                        };
                        let indent = "  ".repeat(
                            row.get("depth")
                                .and_then(Value::as_u64)
                                .unwrap_or(1)
                                .saturating_sub(1)
                                .min(2) as usize,
                        );
                        out.push_str(&format!("{indent}- [{mark}] {title} {date}\n"));
                        let fields = [
                            ("id", "编号"),
                            ("parent_id", "父节点"),
                            ("node_id", "关联节点"),
                            ("project_status", "项目状态"),
                            ("due_at", "截止"),
                            ("remind_at", "提醒"),
                            ("kind", "类型"),
                            ("params", "规则参数"),
                            ("remind_time", "提醒时间"),
                            ("end_date", "结束日期"),
                            ("project_title", "项目"),
                        ];
                        let details = fields
                            .iter()
                            .filter_map(|(key, label)| {
                                row.get(*key).filter(|v| !v.is_null()).map(|v| {
                                    format!(
                                        "{label}：{}",
                                        markdown_text(
                                            v.as_str()
                                                .map(str::to_owned)
                                                .unwrap_or_else(|| v.to_string())
                                                .as_str()
                                        )
                                    )
                                })
                            })
                            .collect::<Vec<_>>()
                            .join("；");
                        if !details.is_empty() {
                            out.push_str(&format!("{indent}  {details}\n"));
                        }
                        if let Some(note) = row.get("note").and_then(Value::as_str) {
                            for line in note.lines() {
                                out.push_str(&format!("{indent}  > {}\n", markdown_text(line)));
                            }
                        }
                    }
                }
                ("md", "text/markdown", out)
            }
            _ => return Err(AppError::Invalid("仅支持Markdown或JSON导出".into())),
        };
        Ok(ExportFile {
            file_name: format!("知了-{}.{}", self.today().format(DATE_FMT), extension),
            mime_type: mime.into(),
            content,
        })
    }

    pub fn import_data(&self, mut data: Transfer) -> AppResult<ImportResult> {
        let result = ImportResult {
            tasks: data.tables["tasks"].len(),
            nodes: data.tables["nodes"].len(),
            recur_rules: data.tables["recur_rules"].len(),
            completions: data.tables["completions"].len(),
        };
        let tx = self.conn.unchecked_transaction()?;
        let mut maps: HashMap<&str, HashMap<i64, i64>> = HashMap::new();
        for table in TABLES {
            let mut max: i64 = tx.query_row(
                &format!("SELECT MAX(COALESCE((SELECT MAX(id) FROM {table}),0),COALESCE((SELECT MAX(entity_id) FROM op_log WHERE entity='{table}'),0))"),
                [],
                |r| r.get(0),
            )?;
            if table == "tasks" || table == "nodes" {
                let field = if table == "tasks" {
                    "task_id"
                } else {
                    "node_id"
                };
                let historic_max: i64 = tx.query_row(
                    &format!("SELECT COALESCE(MAX({field}),0) FROM completions"),
                    [],
                    |r| r.get(0),
                )?;
                max = max.max(historic_max);
            }
            let mut map = HashMap::new();
            for (index, row) in data.tables[table].iter().enumerate() {
                let id = max
                    .checked_add(index as i64 + 1)
                    .ok_or_else(|| invalid("编号已满"))?;
                map.insert(row["id"].as_i64().ok_or_else(|| invalid("编号无效"))?, id);
            }
            maps.insert(table, map);
        }
        // 历史可以保留已删除对象；用独立负编号避免未来的新任务误关联这些快照。
        let mut historic = HashMap::<(&str, i64), i64>::new();
        let mut next_historic: i64 = tx.query_row(
            "SELECT MIN(0,COALESCE(MIN(task_id),0),COALESCE(MIN(node_id),0)) FROM completions",
            [],
            |r| r.get(0),
        )?;
        for table in TABLES {
            for row in data.tables.get_mut(table).unwrap() {
                let old_id = row["id"].as_i64().unwrap();
                row.insert("id".into(), maps[table][&old_id].into());
                for (field, target) in [
                    ("parent_id", "nodes"),
                    ("node_id", "nodes"),
                    ("recur_rule_id", "recur_rules"),
                    ("task_id", "tasks"),
                ] {
                    let Some(old) = row.get(field).and_then(Value::as_i64) else {
                        continue;
                    };
                    let id = if let Some(id) = maps[target].get(&old) {
                        *id
                    } else if table == "completions" {
                        if let Some(id) = historic.get(&(target, old)) {
                            *id
                        } else {
                            next_historic = next_historic
                                .checked_sub(1)
                                .ok_or_else(|| invalid("历史编号已满"))?;
                            historic.insert((target, old), next_historic);
                            next_historic
                        }
                    } else {
                        return Err(invalid("关联对象不存在"));
                    };
                    row.insert(field.into(), id.into());
                }
                if table == "completions" {
                    row.insert("op_group".into(), Value::Null);
                }
                insert(&tx, table, row)?;
            }
        }
        oplog::record(
            &tx,
            oplog::Entry {
                group_id: &oplog::new_group_id(),
                source: oplog::Source::System,
                op: "data.import",
                entity: "data",
                entity_id: None,
                before: None,
                after: None,
                summary: format!(
                    "导入{}条任务、{}个项目节点、{}条重复规则、{}条完成历史",
                    result.tasks, result.nodes, result.recur_rules, result.completions
                ),
                created_at: &self.now_str(),
            },
        )?;
        tx.commit()?;
        Ok(result)
    }

    pub fn operation_log(&self) -> AppResult<Vec<OperationLog>> {
        Ok(self.conn.prepare("SELECT id,source,op,entity,summary,created_at,undone_at FROM op_log ORDER BY id DESC LIMIT 100")?
            .query_map([], |r| Ok(OperationLog {id:r.get(0)?,source:r.get(1)?,operation:r.get(2)?,entity:r.get(3)?,summary:r.get::<_,String>(4)?.chars().take(500).collect(),created_at:r.get(5)?,undone_at:r.get(6)?}))?
            .collect::<Result<_,_>>()?)
    }
}
