//! 应用启动：数据目录、配置、数据库，汇总为全局状态

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use rusqlite::Connection;

use crate::clock::{Clock, SystemClock};
use crate::config::AppConfig;
use crate::db;
use crate::error::AppResult;
use crate::paths::DataPaths;
use crate::secrets::{KeyringStore, SecretStore};

pub struct AppState {
    pub paths: DataPaths,
    /// debug 构建：使用独立数据目录
    pub dev: bool,
    pub config: Mutex<AppConfig>,
    pub db: Mutex<Connection>,
    pub clock: Arc<dyn Clock>,
    /// 当前系统是否支持毛玻璃（Mica），窗口创建后探测
    pub glass_supported: AtomicBool,
    /// API Key 存储（Windows 凭据管理器）
    pub secrets: Box<dyn SecretStore>,
    /// 唤醒后台收件箱处理（例如刚配置好模型）
    pub wake: tokio::sync::Notify,
    pub usage_changed: tokio::sync::Notify,
    pub agent_runs: crate::agent::run_control::RunControl,
    /// 数据恢复持写锁；普通业务操作持读锁。
    pub maintenance: tokio::sync::RwLock<()>,
    pub desktop_error: Mutex<Option<String>>,
}

impl AppState {
    /// `base` 为 `%APPDATA%`。首次运行时创建目录、默认配置和数据库。
    pub fn init(
        base: &Path,
        dev: bool,
        clock: Arc<dyn Clock>,
        secrets: Box<dyn SecretStore>,
    ) -> AppResult<Self> {
        let paths = DataPaths::new(base, dev);
        paths.ensure()?;
        let config = AppConfig::load_or_create(&paths.config())?;
        crate::prompts::write_reference(&paths.prompts())?;
        let mut conn = db::open(&paths.db())?;
        db::migrate(&mut conn, |conn, _| {
            crate::backup::create(&paths, conn, clock.as_ref(), "pre-migration").map(|_| ())
        })?;
        #[cfg(debug_assertions)]
        if dev {
            let svc = crate::service::Service::new(&conn, clock.as_ref(), &config)?;
            let now = clock.now().format(crate::service::DATETIME_FMT).to_string();
            db::dev_seed::seed_if_empty(&conn, svc.today(), &now)?;
        }
        // 日常备份失败不应让整个应用失去恢复入口；迁移前备份仍由上面的
        // migrate 回调强制执行，失败时会阻止迁移。
        let backup_warning = crate::backup::daily(&paths, &conn, &config, clock.as_ref())
            .err()
            .map(|error| format!("自动备份失败，本次仍可继续使用：{error}"));
        Ok(Self {
            paths,
            dev,
            config: Mutex::new(config),
            db: Mutex::new(conn),
            clock,
            glass_supported: AtomicBool::new(false),
            secrets,
            wake: tokio::sync::Notify::new(),
            usage_changed: tokio::sync::Notify::new(),
            agent_runs: crate::agent::run_control::RunControl::default(),
            maintenance: tokio::sync::RwLock::new(()),
            desktop_error: Mutex::new(backup_warning),
        })
    }

    pub fn system(base: &Path) -> AppResult<Self> {
        let dev = cfg!(debug_assertions);
        Self::init(
            base,
            dev,
            Arc::new(SystemClock),
            Box::new(KeyringStore::new(dev)),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::FixedClock;
    use chrono::NaiveDateTime;

    fn clock() -> Arc<dyn Clock> {
        let t = NaiveDateTime::parse_from_str("2026-10-17 10:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
        Arc::new(FixedClock(t))
    }

    #[test]
    fn 首次启动创建目录配置和数据库() {
        let tmp = tempfile::tempdir().unwrap();
        let state = AppState::init(
            tmp.path(),
            false,
            clock(),
            Box::new(crate::secrets::MemoryStore::default()),
        )
        .unwrap();
        assert!(state.paths.config().is_file());
        assert!(state.paths.db().is_file());
        let conn = state.db.lock().unwrap();
        assert_eq!(db::schema_version(&conn).unwrap(), db::LATEST_VERSION);
        assert_eq!(db::list_tables(&conn).unwrap().len(), 12);
    }

    #[test]
    fn 开发版不碰正式数据目录() {
        let tmp = tempfile::tempdir().unwrap();
        AppState::init(
            tmp.path(),
            true,
            clock(),
            Box::new(crate::secrets::MemoryStore::default()),
        )
        .unwrap();
        assert!(tmp.path().join("Cicada-dev").is_dir());
        assert!(!tmp.path().join("Cicada").exists());
    }

    #[test]
    fn 再次启动沿用已有数据() {
        let tmp = tempfile::tempdir().unwrap();
        {
            let state = AppState::init(
                tmp.path(),
                false,
                clock(),
                Box::new(crate::secrets::MemoryStore::default()),
            )
            .unwrap();
            state
                .db
                .lock()
                .unwrap()
                .execute("INSERT INTO meta (key, value) VALUES ('k', 'v')", [])
                .unwrap();
        }
        let state = AppState::init(
            tmp.path(),
            false,
            clock(),
            Box::new(crate::secrets::MemoryStore::default()),
        )
        .unwrap();
        let v: String = state
            .db
            .lock()
            .unwrap()
            .query_row("SELECT value FROM meta WHERE key = 'k'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, "v");
    }
}
