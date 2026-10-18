//! 数据目录布局，见 docs/产品设计.md 11.1

use std::path::{Path, PathBuf};

use crate::error::AppResult;

/// 正式版数据目录名
pub const RELEASE_DIR: &str = "Cicada";
/// 开发版（debug 构建）数据目录名，与正式数据隔离
pub const DEV_DIR: &str = "Cicada-dev";

#[derive(Debug, Clone)]
pub struct DataPaths {
    pub root: PathBuf,
}

impl DataPaths {
    /// `base` 为 `%APPDATA%`
    pub fn new(base: &Path, dev: bool) -> Self {
        Self {
            root: base.join(if dev { DEV_DIR } else { RELEASE_DIR }),
        }
    }

    pub fn db(&self) -> PathBuf {
        self.root.join("cicada.db")
    }
    pub fn config(&self) -> PathBuf {
        self.root.join("config.json")
    }
    pub fn reports(&self) -> PathBuf {
        self.root.join("reports")
    }
    pub fn prompts(&self) -> PathBuf {
        self.root.join("prompts")
    }
    pub fn inbox(&self) -> PathBuf {
        self.root.join("inbox")
    }
    pub fn logs(&self) -> PathBuf {
        self.root.join("logs")
    }
    pub fn backups(&self) -> PathBuf {
        self.root.join("backups")
    }

    /// 创建全部目录（已存在则跳过）
    pub fn ensure(&self) -> AppResult<()> {
        let reports = self.reports();
        for dir in [
            self.root.clone(),
            reports.join("weekly"),
            reports.join("monthly"),
            reports.join("yearly"),
            self.prompts(),
            self.inbox(),
            self.logs(),
            self.backups(),
        ] {
            std::fs::create_dir_all(dir)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 开发版与正式版目录隔离() {
        let base = Path::new("C:/AppData");
        assert!(DataPaths::new(base, false).root.ends_with("Cicada"));
        assert!(DataPaths::new(base, true).root.ends_with("Cicada-dev"));
    }

    #[test]
    fn 创建全部目录() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = DataPaths::new(tmp.path(), false);
        paths.ensure().unwrap();
        paths.ensure().unwrap(); // 可重复调用
        for sub in [
            "reports/weekly",
            "reports/monthly",
            "reports/yearly",
            "prompts",
            "inbox",
            "logs",
            "backups",
        ] {
            assert!(paths.root.join(sub).is_dir(), "{sub} 不存在");
        }
    }
}
