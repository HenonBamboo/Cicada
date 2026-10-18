//! 完整数据备份：一致性数据库快照、清单校验和可回退恢复。
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use chrono::{Datelike, Days, NaiveDate, NaiveDateTime, Weekday};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zip::write::SimpleFileOptions;

use crate::clock::{Clock, parse_cutoff};
use crate::config::AppConfig;
use crate::error::{AppError, AppResult};
use crate::{db, paths::DataPaths};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupEntry {
    pub name: String,
    pub created_at: String,
    pub kind: String,
    pub size_bytes: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    app_version: String,
    schema_version: u32,
    created_at: String,
    kind: String,
    files: BTreeMap<String, String>,
}

fn invalid(message: impl ToString) -> AppError {
    AppError::Invalid(format!("备份处理失败：{}", message.to_string()))
}

struct WorkDir(PathBuf);
impl WorkDir {
    fn new(root: &Path) -> AppResult<Self> {
        let path = root.join(format!(".backup-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }
}
impl Drop for WorkDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn collect(root: &Path, dir: &Path, files: &mut BTreeMap<String, Vec<u8>>) -> AppResult<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        if ty.is_symlink() {
            return Err(invalid("不支持备份符号链接"));
        }
        if ty.is_dir() {
            collect(root, &entry.path(), files)?;
        } else if ty.is_file() {
            let name = entry
                .path()
                .strip_prefix(root)
                .map_err(invalid)?
                .to_string_lossy()
                .replace('\\', "/");
            files.insert(name, fs::read(entry.path())?);
        }
    }
    Ok(())
}

pub fn create(
    paths: &DataPaths,
    conn: &Connection,
    clock: &dyn Clock,
    kind: &str,
) -> AppResult<BackupEntry> {
    let work = WorkDir::new(&paths.backups())?;
    let snapshot = work.0.join("cicada.db");
    db::snapshot_to(conn, &snapshot)?;
    let mut files = BTreeMap::new();
    files.insert("cicada.db".into(), fs::read(snapshot)?);
    files.insert("config.json".into(), fs::read(paths.config())?);
    collect(&paths.root, &paths.reports(), &mut files)?;
    collect(&paths.root, &paths.prompts(), &mut files)?;
    if files.len() >= 10_000
        || files.values().map(|data| data.len() as u64).sum::<u64>() > 512 * 1024 * 1024
    {
        return Err(invalid("备份超过文件数量或 512 MB 大小限制"));
    }
    let manifest = Manifest {
        app_version: env!("CARGO_PKG_VERSION").into(),
        schema_version: db::schema_version(conn)?,
        created_at: clock.now().format("%Y-%m-%dT%H:%M:%S").to_string(),
        kind: kind.into(),
        files: files
            .iter()
            .map(|(name, bytes)| (name.clone(), format!("{:x}", Sha256::digest(bytes))))
            .collect(),
    };
    let name = format!(
        "{}-{kind}-{}.zip",
        clock.now().format("%Y%m%d-%H%M%S"),
        uuid::Uuid::new_v4()
    );
    let temporary = work.0.join("backup.zip");
    let mut writer = zip::ZipWriter::new(File::create(&temporary)?);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, data) in files {
        writer.start_file(name, options).map_err(invalid)?;
        writer.write_all(&data)?;
    }
    writer
        .start_file("manifest.json", options)
        .map_err(invalid)?;
    writer.write_all(&serde_json::to_vec_pretty(&manifest)?)?;
    writer.finish().map_err(invalid)?.sync_all()?;
    fs::rename(temporary, paths.backups().join(&name))?;
    let entry = BackupEntry {
        size_bytes: fs::metadata(paths.backups().join(&name))?.len(),
        name,
        created_at: manifest.created_at,
        kind: kind.into(),
    };
    // 清理失败不否定已成功落盘的备份。
    if let Ok(entries) = list(paths) {
        for automatic in [true, false] {
            let keep = if automatic { 30 } else { 10 };
            let existing_to_keep = keep - usize::from((entry.kind == "auto") == automatic);
            for old in entries
                .iter()
                .filter(|e| (e.kind == "auto") == automatic && e.name != entry.name)
                .skip(existing_to_keep)
            {
                let _ = fs::remove_file(paths.backups().join(&old.name));
            }
        }
    }
    Ok(entry)
}

pub fn daily(
    paths: &DataPaths,
    conn: &Connection,
    config: &AppConfig,
    clock: &dyn Clock,
) -> AppResult<()> {
    let today = clock
        .business_date(parse_cutoff(&config.time.day_cutoff)?)
        .to_string();
    let last: Option<String> = conn
        .query_row(
            "SELECT value FROM meta WHERE key='last_backup_date'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    if last.as_deref() != Some(&today) {
        create(paths, conn, clock, "auto")?;
        conn.execute("INSERT INTO meta(key,value) VALUES('last_backup_date',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [&today])?;
    }
    Ok(())
}

pub fn list(paths: &DataPaths) -> AppResult<Vec<BackupEntry>> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(paths.backups())? {
        let entry = entry?;
        if !entry.file_type()?.is_file() || entry.path().extension().is_none_or(|e| e != "zip") {
            continue;
        }
        let parsed = (|| -> AppResult<Manifest> {
            let mut archive = zip::ZipArchive::new(File::open(entry.path())?).map_err(invalid)?;
            let file = archive.by_name("manifest.json").map_err(invalid)?;
            if file.size() > 2 * 1024 * 1024 {
                return Err(invalid("清单过大"));
            }
            Ok(serde_json::from_reader(file)?)
        })();
        if let Ok(m) = parsed {
            entries.push(BackupEntry {
                name: entry.file_name().to_string_lossy().into(),
                created_at: m.created_at,
                kind: m.kind,
                size_bytes: entry.metadata()?.len(),
            });
        }
    }
    entries.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| b.name.cmp(&a.name))
    });
    Ok(entries)
}

fn safe_name(name: &str) -> bool {
    !name.contains(['\\', ':', '\0'])
        && !name.split('/').any(|s| {
            let stem = s.split('.').next().unwrap_or_default().to_ascii_uppercase();
            let reserved = ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
                || (stem.starts_with("COM") || stem.starts_with("LPT"))
                    && stem.len() == 4
                    && matches!(stem.as_bytes()[3], b'1'..=b'9');
            s.is_empty() || s == "." || s == ".." || s.ends_with(['.', ' ']) || reserved
        })
        && Path::new(name)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
        && (name == "cicada.db"
            || name == "config.json"
            || name.starts_with("reports/")
            || name.starts_with("prompts/"))
}

fn extract(path: &Path, target: &Path) -> AppResult<Manifest> {
    let mut archive = zip::ZipArchive::new(File::open(path)?).map_err(invalid)?;
    if archive.len() > 10_000 {
        return Err(invalid("备份文件过多"));
    }
    let manifest: Manifest = {
        let file = archive.by_name("manifest.json").map_err(invalid)?;
        if file.size() > 2 * 1024 * 1024 {
            return Err(invalid("清单过大"));
        }
        serde_json::from_reader(file)?
    };
    if manifest.schema_version > db::LATEST_VERSION {
        return Err(invalid("备份来自更新版本，请先升级知了"));
    }
    if !manifest.files.contains_key("cicada.db") || !manifest.files.contains_key("config.json") {
        return Err(invalid("备份缺少数据库或配置"));
    }
    let mut seen = BTreeSet::new();
    let mut total = 0u64;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(invalid)?;
        let name = file.name().to_string();
        if !seen.insert(name.to_lowercase()) {
            return Err(invalid("备份包含重复路径"));
        }
        if name == "manifest.json" {
            continue;
        }
        if !safe_name(&name)
            || file.is_dir()
            || file.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000)
        {
            return Err(invalid("备份包含不安全路径"));
        }
        total = total.saturating_add(file.size());
        if total > 512 * 1024 * 1024 {
            return Err(invalid("备份解压大小超过 512 MB"));
        }
        let expected = manifest
            .files
            .get(&name)
            .ok_or_else(|| invalid("文件不在清单中"))?;
        let mut data = Vec::new();
        (&mut file)
            .take(512 * 1024 * 1024 + 1)
            .read_to_end(&mut data)?;
        if data.len() as u64 != file.size() || format!("{:x}", Sha256::digest(&data)) != *expected {
            return Err(invalid(format!("文件校验失败：{name}")));
        }
        let dest = target.join(&name);
        fs::create_dir_all(dest.parent().ok_or_else(|| invalid("路径无效"))?)?;
        fs::write(dest, data)?;
    }
    if seen.len() != manifest.files.len() + 1 {
        return Err(invalid("清单文件不完整"));
    }
    Ok(manifest)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReportMetadata {
    kind: String,
    period: String,
    start_date: NaiveDate,
    end_date: NaiveDate,
    generated_at: NaiveDateTime,
}

fn rebuild_reports(conn: &Connection, root: &Path, generated: &str) -> AppResult<()> {
    conn.execute("DELETE FROM reports", [])?;
    for kind in ["weekly", "monthly", "yearly"] {
        let dir = root.join("reports").join(kind);
        fs::create_dir_all(&dir)?;
        for file in fs::read_dir(dir)? {
            let file = file?;
            let path = file.path();
            if path.extension().is_none_or(|s| s != "md") {
                continue;
            }
            let period = path.file_stem().unwrap_or_default().to_string_lossy();
            let start = match kind {
                "weekly" => period.split_once("-W").and_then(|(y, w)| {
                    NaiveDate::from_isoywd_opt(y.parse().ok()?, w.parse().ok()?, Weekday::Mon)
                }),
                "monthly" => NaiveDate::parse_from_str(&format!("{period}-01"), "%Y-%m-%d").ok(),
                _ => NaiveDate::parse_from_str(&format!("{period}-01-01"), "%Y-%m-%d").ok(),
            }
            .ok_or_else(|| invalid(format!("报告文件名无效：{}", path.display())))?;
            let next = match kind {
                "weekly" => start.checked_add_days(Days::new(7)),
                "monthly" => start.checked_add_months(chrono::Months::new(1)),
                _ => start.checked_add_months(chrono::Months::new(12)),
            }
            .and_then(|d| d.checked_sub_days(Days::new(1)))
            .ok_or_else(|| invalid("报告日期超出范围"))?;
            let content = fs::read_to_string(&path)?;
            let first_line = content.lines().next().unwrap_or_default().trim();
            let (start, end, generated_at) =
                if let Some(metadata) = first_line.strip_prefix("<!-- cicada-report ") {
                    let json = metadata
                        .strip_suffix(" -->")
                        .ok_or_else(|| invalid("报告元数据格式无效"))?;
                    let metadata: ReportMetadata = serde_json::from_str(json)
                        .map_err(|e| invalid(format!("报告元数据无效：{e}")))?;
                    let dates_match = if kind == "weekly" {
                        matches!(metadata.start_date.weekday(), Weekday::Mon | Weekday::Sun)
                            && metadata
                                .end_date
                                .signed_duration_since(metadata.start_date)
                                .num_days()
                                == 6
                            && metadata.end_date.iso_week() == next.iso_week()
                    } else {
                        metadata.start_date == start && metadata.end_date == next
                    };
                    if metadata.kind != kind || metadata.period != period || !dates_match {
                        return Err(invalid(format!(
                            "报告元数据与文件周期不一致：{}",
                            path.display()
                        )));
                    }
                    (
                        metadata.start_date,
                        metadata.end_date,
                        metadata
                            .generated_at
                            .format(crate::service::DATETIME_FMT)
                            .to_string(),
                    )
                } else {
                    (start, next, generated.to_string())
                };
            conn.execute("INSERT INTO reports(kind,period,start_date,end_date,path,generated_at) VALUES(?1,?2,?3,?4,?5,?6)", rusqlite::params![kind,period,start.to_string(),end.to_string(),format!("reports/{kind}/{period}.md"),generated_at])?;
        }
    }
    Ok(())
}

pub fn restore(
    paths: &DataPaths,
    conn: &mut Connection,
    config: &mut AppConfig,
    clock: &dyn Clock,
    name: &str,
) -> AppResult<()> {
    if name.contains(['/', '\\', ':'])
        || !name.ends_with(".zip")
        || Path::new(name).file_name().is_none()
    {
        return Err(invalid("备份名称无效"));
    }
    let source = paths.backups().join(name);
    if fs::symlink_metadata(&source)?.file_type().is_symlink() {
        return Err(invalid("不支持链接备份"));
    }
    let work = WorkDir::new(&paths.backups())?;
    let staged = work.0.join("new");
    fs::create_dir(&staged)?;
    let manifest = extract(&source, &staged)?;
    let next: AppConfig = serde_json::from_slice(&fs::read(staged.join("config.json"))?)?;
    next.validate()?;
    {
        let mut restored = db::open(&staged.join("cicada.db"))?;
        if db::schema_version(&restored)? != manifest.schema_version {
            return Err(invalid("数据库版本与清单不一致"));
        }
        let integrity: String = restored.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
        if integrity != "ok" {
            return Err(invalid("数据库完整性检查失败"));
        }
        db::migrate(&mut restored, |_, _| Ok(()))?;
        rebuild_reports(&restored, &staged, &manifest.created_at)?;
        crate::service::Service::new(&restored, clock, &next)?.inbox_recover()?;
        restored.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
    }
    fs::create_dir_all(staged.join("prompts"))?;
    create(paths, conn, clock, "pre-restore")?;
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
    let old = work.0.join("old");
    fs::create_dir(&old)?;
    let placeholder = Connection::open_in_memory()?;
    drop(std::mem::replace(conn, placeholder));
    let mut moved = Vec::new();
    let mut installed = Vec::new();
    let result = (|| -> AppResult<Connection> {
        for item in ["cicada.db", "reports", "config.json", "prompts"] {
            fs::rename(paths.root.join(item), old.join(item))?;
            moved.push(item);
            fs::rename(staged.join(item), paths.root.join(item))?;
            installed.push(item);
        }
        db::open(&paths.db())
    })();
    match result {
        Ok(restored) => {
            *conn = restored;
            *config = next;
            Ok(())
        }
        Err(error) => {
            // 将失败的新文件移回暂存区，原文件保持可恢复。
            for item in installed.iter().rev() {
                if let Err(e) = fs::rename(paths.root.join(item), staged.join(item)) {
                    let recovery = work.0.clone();
                    std::mem::forget(work);
                    return Err(invalid(format!(
                        "恢复失败且回退受阻：{error}；{e}。原数据保存在 {}",
                        recovery.display()
                    )));
                }
            }
            for item in moved.iter().rev() {
                if let Err(e) = fs::rename(old.join(item), paths.root.join(item)) {
                    let recovery = work.0.clone();
                    std::mem::forget(work);
                    return Err(invalid(format!(
                        "恢复失败且回退受阻：{error}；{e}。原数据保存在 {}",
                        recovery.display()
                    )));
                }
            }
            *conn = db::open(&paths.db())?;
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ThemeChoice;

    #[test]
    fn daily_backups_and_retention_are_independent() {
        let temp = tempfile::tempdir().unwrap();
        let paths = DataPaths::new(temp.path(), true);
        paths.ensure().unwrap();
        let mut conn = db::open(&paths.db()).unwrap();
        db::migrate(&mut conn, |_, _| Ok(())).unwrap();
        let mut config = AppConfig::default();
        config.time.day_cutoff = "04:00".into();
        config.save(&paths.config()).unwrap();
        let start = crate::service::testing::clock("2026-10-18 04:00:00").0;

        // 连续驻留三天，每日启动/定时调用多次；日切前仍属于前一业务日。
        for day in 0..3 {
            let clock = crate::clock::FixedClock(start + chrono::Duration::days(day));
            daily(&paths, &conn, &config, &clock).unwrap();
            daily(&paths, &conn, &config, &clock).unwrap();
            let before_next_cutoff =
                crate::clock::FixedClock(clock.0 + chrono::Duration::hours(23));
            daily(&paths, &conn, &config, &before_next_cutoff).unwrap();
        }
        assert_eq!(list(&paths).unwrap().len(), 3);

        // 超过保留数量后，两组独立保留；操作前备份与手动备份共用十份配额。
        for day in 3..32 {
            let clock = crate::clock::FixedClock(start + chrono::Duration::days(day));
            daily(&paths, &conn, &config, &clock).unwrap();
        }
        for index in 0..12 {
            let clock = crate::clock::FixedClock(
                start + chrono::Duration::days(32) + chrono::Duration::seconds(index),
            );
            let kind = match index % 3 {
                0 => "manual",
                1 => "pre-migration",
                _ => "pre-restore",
            };
            create(&paths, &conn, &clock, kind).unwrap();
        }
        let entries = list(&paths).unwrap();
        let automatic: Vec<_> = entries
            .iter()
            .filter(|entry| entry.kind == "auto")
            .collect();
        let other: Vec<_> = entries
            .iter()
            .filter(|entry| entry.kind != "auto")
            .collect();
        assert_eq!(automatic.len(), 30);
        assert_eq!(other.len(), 10);
        assert_eq!(
            automatic.last().unwrap().created_at,
            (start + chrono::Duration::days(2))
                .format("%Y-%m-%dT%H:%M:%S")
                .to_string()
        );
        assert_eq!(
            other.last().unwrap().created_at,
            (start + chrono::Duration::days(32) + chrono::Duration::seconds(2))
                .format("%Y-%m-%dT%H:%M:%S")
                .to_string()
        );
    }

    #[test]
    fn backup_roundtrip_and_corrupt_archive_preserves_current_data() {
        // 全部文件和 SQLite 连接都位于隔离目录，不初始化应用或凭据管理器。
        let temp = tempfile::tempdir().unwrap();
        let paths = DataPaths::new(temp.path(), true);
        paths.ensure().unwrap();
        let clock = crate::service::testing::clock("2026-10-18 10:00:00");
        let mut conn = db::open(&paths.db()).unwrap();
        db::migrate(&mut conn, |_, _| Ok(())).unwrap();
        let mut config = AppConfig::default();
        config.appearance.theme = ThemeChoice::Paper;
        config.time.day_cutoff = "04:00".into();
        config.save(&paths.config()).unwrap();
        conn.execute(
            "INSERT INTO tasks(id,title,planned_date,created_at,updated_at) VALUES(1,'备份时的任务','2026-10-18','2026-10-18T10:00:00','2026-10-18T10:00:00')",
            [],
        ).unwrap();
        let prompt = paths.prompts().join("task.md");
        let report = paths.reports().join("monthly/2026-10.md");
        let weekly_report = paths.reports().join("weekly/2026-W42.md");
        fs::write(&prompt, "备份时的提示词").unwrap();
        fs::write(&report, "# 十月报告\n备份时的内容").unwrap();
        fs::write(&weekly_report, "<!-- cicada-report {\"kind\":\"weekly\",\"period\":\"2026-W42\",\"startDate\":\"2026-10-11\",\"endDate\":\"2026-10-17\",\"generatedAt\":\"2026-10-18T08:30:00\"} -->\n# 周报").unwrap();
        let saved_config = config.clone();
        let saved = create(&paths, &conn, &clock, "manual").unwrap();

        conn.execute("UPDATE tasks SET title='当前任务' WHERE id=1", [])
            .unwrap();
        config.appearance.theme = ThemeChoice::Forest;
        config.time.day_cutoff = "00:00".into();
        config.save(&paths.config()).unwrap();
        fs::write(&prompt, "当前提示词").unwrap();
        fs::write(&report, "当前报告").unwrap();
        let current_config = config.clone();
        let current_config_bytes = fs::read(paths.config()).unwrap();

        // 重新打包得到 CRC 正确但与 manifest SHA-256 不一致的 ZIP，确实覆盖清单校验。
        let corrupt_name = "corrupt-checksum.zip";
        {
            let mut original =
                zip::ZipArchive::new(File::open(paths.backups().join(&saved.name)).unwrap())
                    .unwrap();
            let mut corrupt =
                zip::ZipWriter::new(File::create(paths.backups().join(corrupt_name)).unwrap());
            for index in 0..original.len() {
                let mut file = original.by_index(index).unwrap();
                let mut bytes = Vec::new();
                file.read_to_end(&mut bytes).unwrap();
                if file.name() == "prompts/task.md" {
                    bytes = b"tampered prompt".to_vec();
                }
                corrupt
                    .start_file(file.name(), SimpleFileOptions::default())
                    .unwrap();
                corrupt.write_all(&bytes).unwrap();
            }
            corrupt.finish().unwrap();
        }
        let error = restore(&paths, &mut conn, &mut config, &clock, corrupt_name).unwrap_err();
        assert!(
            error.to_string().contains("文件校验失败：prompts/task.md"),
            "{error}"
        );
        let title = |conn: &Connection| -> String {
            conn.query_row("SELECT title FROM tasks WHERE id=1", [], |r| r.get(0))
                .unwrap()
        };
        assert_eq!(title(&conn), "当前任务");
        assert_eq!(config, current_config);
        assert_eq!(fs::read(paths.config()).unwrap(), current_config_bytes);
        assert_eq!(fs::read_to_string(&prompt).unwrap(), "当前提示词");
        assert_eq!(fs::read_to_string(&report).unwrap(), "当前报告");

        restore(&paths, &mut conn, &mut config, &clock, &saved.name).unwrap();
        assert_eq!(title(&conn), "备份时的任务");
        assert_eq!(config, saved_config);
        let disk_config: AppConfig =
            serde_json::from_slice(&fs::read(paths.config()).unwrap()).unwrap();
        assert_eq!(disk_config, saved_config);
        assert_eq!(fs::read_to_string(&prompt).unwrap(), "备份时的提示词");
        assert_eq!(
            fs::read_to_string(&report).unwrap(),
            "# 十月报告\n备份时的内容"
        );
        let indexed: (String, String, String) = conn.query_row(
            "SELECT start_date,end_date,path FROM reports WHERE kind='monthly' AND period='2026-10'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        ).unwrap();
        assert_eq!(
            indexed,
            (
                "2026-10-01".into(),
                "2026-10-31".into(),
                "reports/monthly/2026-10.md".into()
            )
        );
        let weekly_dates: (String, String, String) = conn.query_row(
            "SELECT start_date,end_date,generated_at FROM reports WHERE kind='weekly' AND period='2026-W42'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        ).unwrap();
        assert_eq!(
            weekly_dates,
            (
                "2026-10-11".into(),
                "2026-10-17".into(),
                "2026-10-18T08:30:00".into()
            )
        );
        assert!(
            list(&paths)
                .unwrap()
                .iter()
                .any(|entry| entry.kind == "pre-restore")
        );
    }
}
