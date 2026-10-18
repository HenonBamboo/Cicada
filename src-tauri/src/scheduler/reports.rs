//! 周/月/年报告：已结束业务周期、完成快照、每次补一份、文件先落地后建索引。
use super::ai::{generate, poisoned};
use crate::config::{AppConfig, WeekStart};
use crate::error::{AppError, AppResult};
use crate::paths::DataPaths;
use crate::prompts::PromptId;
use crate::service::oplog;
use crate::state::AppState;
use chrono::{Datelike, Days, Months, NaiveDate};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashSet;
use std::fs;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportMetadata {
    pub kind: String,
    pub period: String,
    pub start_date: String,
    pub end_date: String,
    pub generated_at: String,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportEntry {
    pub id: i64,
    pub kind: String,
    pub period: String,
    pub start_date: String,
    pub end_date: String,
    pub generated_at: String,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportDocument {
    pub entry: ReportEntry,
    pub markdown: String,
}
#[derive(Debug, Clone)]
struct Period {
    kind: &'static str,
    period: String,
    start: NaiveDate,
    end: NaiveDate,
}

fn period_at(day: NaiveDate, kind: &'static str, week_start: WeekStart) -> AppResult<Period> {
    let start = match kind {
        "weekly" => day.checked_sub_days(Days::new(match week_start {
            WeekStart::Monday => day.weekday().num_days_from_monday() as u64,
            WeekStart::Sunday => day.weekday().num_days_from_sunday() as u64,
        })),
        "monthly" => day.with_day(1),
        _ => NaiveDate::from_ymd_opt(day.year(), 1, 1),
    }
    .ok_or_else(|| AppError::Invalid("报告日期超出范围".into()))?;
    let next = match kind {
        "weekly" => start.checked_add_days(Days::new(7)),
        "monthly" => start.checked_add_months(Months::new(1)),
        _ => start.checked_add_months(Months::new(12)),
    }
    .ok_or_else(|| AppError::Invalid("报告日期超出范围".into()))?;
    // 周日开始时，选周内星期四的 ISO 周号；精确起止日期保存在 metadata。
    let period = match kind {
        "weekly" => {
            let anchor = start
                .checked_add_days(Days::new(4))
                .ok_or_else(|| AppError::Invalid("报告日期超出范围".into()))?;
            format!(
                "{}-W{:02}",
                anchor.iso_week().year(),
                anchor.iso_week().week()
            )
        }
        "monthly" => start.format("%Y-%m").to_string(),
        _ => start.format("%Y").to_string(),
    };
    Ok(Period {
        kind,
        period,
        start,
        end: next
            .pred_opt()
            .ok_or_else(|| AppError::Invalid("报告日期超出范围".into()))?,
    })
}

fn entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<ReportEntry> {
    Ok(ReportEntry {
        id: row.get(0)?,
        kind: row.get(1)?,
        period: row.get(2)?,
        start_date: row.get(3)?,
        end_date: row.get(4)?,
        generated_at: row.get(5)?,
    })
}
pub fn list(conn: &Connection) -> AppResult<Vec<ReportEntry>> {
    let mut stmt = conn.prepare("SELECT id,kind,period,start_date,end_date,generated_at FROM reports ORDER BY end_date DESC,kind,id DESC")?;
    Ok(stmt.query_map([], entry)?.collect::<Result<Vec<_>, _>>()?)
}
pub fn read(conn: &Connection, paths: &DataPaths, id: i64) -> AppResult<ReportDocument> {
    read_dir(conn, &paths.reports(), id)
}
pub fn read_dir(
    conn: &Connection,
    reports_dir: &std::path::Path,
    id: i64,
) -> AppResult<ReportDocument> {
    let report = conn
        .query_row(
            "SELECT id,kind,period,start_date,end_date,generated_at FROM reports WHERE id=?1",
            [id],
            entry,
        )
        .optional()?
        .ok_or_else(|| AppError::Invalid("报告不存在".into()))?;
    if !["weekly", "monthly", "yearly"].contains(&report.kind.as_str())
        || !report
            .period
            .chars()
            .all(|c| c.is_ascii_digit() || c == '-' || c == 'W')
    {
        return Err(AppError::Invalid("报告路径无效".into()));
    }
    let path = reports_dir
        .join(&report.kind)
        .join(format!("{}.md", report.period));
    let canonical = path.canonicalize()?;
    if !canonical.starts_with(reports_dir.canonicalize()?) {
        return Err(AppError::Invalid("报告路径超出数据目录".into()));
    }
    let content = fs::read_to_string(canonical)?;
    let markdown = if content.starts_with("<!-- cicada-report ") {
        content
            .split_once('\n')
            .map(|(_, body)| body)
            .unwrap_or("")
            .trim_start()
            .to_string()
    } else {
        content
    };
    Ok(ReportDocument {
        entry: report,
        markdown,
    })
}

fn next_missing(
    conn: &Connection,
    config: &AppConfig,
    today: NaiveDate,
) -> AppResult<Option<Period>> {
    let earliest: Option<String> = conn.query_row(
        "SELECT MIN(d) FROM (SELECT business_date AS d FROM completions UNION ALL SELECT substr(created_at,1,10) FROM tasks UNION ALL SELECT substr(created_at,1,10) FROM nodes UNION ALL SELECT value FROM meta WHERE key='report_history_start')", [], |r| r.get(0))?;
    let first = earliest
        .and_then(|s| NaiveDate::parse_from_str(&s, "%Y-%m-%d").ok())
        .unwrap_or(today);
    conn.execute(
        "INSERT OR IGNORE INTO meta(key,value) VALUES('report_history_start',?1)",
        [first.to_string()],
    )?;
    let existing: HashSet<(String, String)> = list(conn)?
        .into_iter()
        .map(|r| (r.kind, r.period))
        .collect();
    let mut candidates = Vec::new();
    for (kind, enabled) in [
        ("weekly", config.automation.weekly_report),
        ("monthly", config.automation.monthly_report),
        ("yearly", config.automation.yearly_report),
    ] {
        if !enabled {
            continue;
        }
        let mut p = period_at(first, kind, config.time.week_start)?;
        while p.end < today {
            if !existing.contains(&(kind.to_string(), p.period.clone())) {
                candidates.push(p);
                break;
            }
            p = period_at(
                p.end
                    .succ_opt()
                    .ok_or_else(|| AppError::Invalid("报告日期超出范围".into()))?,
                kind,
                config.time.week_start,
            )?;
        }
    }
    candidates.sort_by_key(|p| (p.end, p.kind));
    Ok(candidates.into_iter().next())
}

fn context(conn: &Connection, period: &Period) -> AppResult<String> {
    let mut stmt = conn.prepare("SELECT business_date,kind,title,project_title,task_id IS NOT NULL AND node_id IS NOT NULL FROM completions WHERE business_date>=?1 AND business_date<=?2 ORDER BY business_date,id")?;
    let rows = stmt.query_map(params![period.start.to_string(),period.end.to_string()], |r| Ok(json!({
        "date": r.get::<_,String>(0)?, "kind": r.get::<_,String>(1)?, "title": r.get::<_,String>(2)?, "project": r.get::<_,Option<String>>(3)?, "linkedTask":r.get::<_,bool>(4)?
    })))?.collect::<Result<Vec<_>,_>>()?;
    Ok(json!({"kind":period.kind,"startDate":period.start.to_string(),"endDate":period.end.to_string(),"completedCount":rows.len(),"completions":rows}).to_string())
}
fn index(conn: &Connection, metadata: &ReportMetadata) -> AppResult<()> {
    let tx = conn.unchecked_transaction()?;
    let inserted = tx.execute("INSERT OR IGNORE INTO reports(kind,period,start_date,end_date,path,generated_at) VALUES(?1,?2,?3,?4,?5,?6)", params![metadata.kind,metadata.period,metadata.start_date,metadata.end_date,format!("reports/{}/{}.md",metadata.kind,metadata.period),metadata.generated_at])?;
    if inserted > 0 {
        let id = tx.last_insert_rowid();
        oplog::record(
            &tx,
            oplog::Entry {
                group_id: &oplog::new_group_id(),
                source: oplog::Source::System,
                op: "report_generate",
                entity: "reports",
                entity_id: Some(id),
                before: None,
                after: Some(serde_json::to_value(metadata)?),
                summary: format!("生成报告 {}", metadata.period),
                created_at: &metadata.generated_at,
            },
        )?;
    }
    tx.commit()?;
    Ok(())
}

/// 一次最多生成一份。失败不写成功索引，调用方按冷却间隔重试。
pub async fn catch_up(state: &AppState) -> AppResult<usize> {
    let _maintenance = state.maintenance.read().await;
    let config = state.config.lock().map_err(poisoned)?.clone();
    let today = state
        .clock
        .business_date(crate::clock::parse_cutoff(&config.time.day_cutoff)?);
    let pending = {
        let conn = state.db.lock().map_err(poisoned)?;
        next_missing(&conn, &config, today)?
    };
    let Some(period) = pending else {
        return Ok(0);
    };
    let path = state
        .paths
        .reports()
        .join(period.kind)
        .join(format!("{}.md", period.period));
    // 文件已完成、索引提交前中断：从文件补索引，不再调用模型。
    if path.is_file() {
        let content = fs::read_to_string(&path)?;
        let header = content
            .lines()
            .next()
            .and_then(|line| line.strip_prefix("<!-- cicada-report "))
            .and_then(|line| line.strip_suffix(" -->"));
        let metadata = if let Some(header) = header {
            serde_json::from_str::<ReportMetadata>(header)?
        } else {
            ReportMetadata {
                kind: period.kind.into(),
                period: period.period.clone(),
                start_date: period.start.to_string(),
                end_date: period.end.to_string(),
                generated_at: state
                    .clock
                    .now()
                    .format(crate::service::DATETIME_FMT)
                    .to_string(),
            }
        };
        if metadata.kind != period.kind || metadata.period != period.period {
            return Err(AppError::Invalid("报告元数据与文件名不一致".into()));
        }
        let conn = state.db.lock().map_err(poisoned)?;
        index(&conn, &metadata)?;
        return Ok(1);
    }
    let input = {
        let conn = state.db.lock().map_err(poisoned)?;
        context(&conn, &period)?
    };
    let markdown = generate(state, &config, PromptId::Report, input).await?;
    let metadata = ReportMetadata {
        kind: period.kind.into(),
        period: period.period.clone(),
        start_date: period.start.to_string(),
        end_date: period.end.to_string(),
        generated_at: state
            .clock
            .now()
            .format(crate::service::DATETIME_FMT)
            .to_string(),
    };
    let content = format!(
        "<!-- cicada-report {} -->\n\n{}\n",
        serde_json::to_string(&metadata)?,
        markdown
    );
    fs::create_dir_all(
        path.parent()
            .ok_or_else(|| AppError::Invalid("报告目录无效".into()))?,
    )?;
    let temp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    fs::write(&temp, content)?;
    // 调度唯一入口负责串行调用；同名文件存在时保持已生成报告不变。
    if path.exists() {
        fs::remove_file(&temp)?;
        return Ok(0);
    }
    fs::rename(&temp, &path)?;
    let conn = state.db.lock().map_err(poisoned)?;
    index(&conn, &metadata)?;
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }
    #[test]
    fn 周起始日与跨年周期() {
        let mon = period_at(d("2026-01-01"), "weekly", WeekStart::Monday).unwrap();
        assert_eq!(
            (mon.start, mon.end, mon.period.as_str()),
            (d("2025-12-29"), d("2026-01-04"), "2026-W01")
        );
        let sun = period_at(d("2026-01-01"), "weekly", WeekStart::Sunday).unwrap();
        assert_eq!(
            (sun.start, sun.end, sun.period.as_str()),
            (d("2025-12-28"), d("2026-01-03"), "2026-W01")
        );
        let leap = period_at(d("2024-02-29"), "monthly", WeekStart::Monday).unwrap();
        assert_eq!(leap.end, d("2024-02-29"));
        let year = period_at(d("2025-12-31"), "yearly", WeekStart::Monday).unwrap();
        assert_eq!((year.start, year.end), (d("2025-01-01"), d("2025-12-31")));
    }
}
