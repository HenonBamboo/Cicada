//! 用户设置 config.json（不含 API Key），见 docs/产品设计.md 第 10 节

use std::path::Path;

use chrono::Local;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::clock::parse_cutoff;
use crate::error::{AppError, AppResult};
use crate::llm::{Endpoint, ProviderId};

pub const CONFIG_VERSION: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeChoice {
    System,
    Light,
    Dark,
    Paper,
    Glass,
    Forest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scale {
    Small,
    Medium,
    Large,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Appearance {
    pub theme: ThemeChoice,
    /// 自定义强调色 `#RRGGBB`，None 表示使用主题默认值
    pub accent: Option<String>,
    pub font_size: Scale,
    pub radius: Scale,
    pub motion: bool,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::Light,
            accent: None,
            font_size: Scale::Medium,
            radius: Scale::Medium,
            motion: true,
        }
    }
}

impl Appearance {
    pub fn validate(&self) -> AppResult<()> {
        if let Some(c) = &self.accent {
            let ok = c.len() == 7
                && c.starts_with('#')
                && c[1..].chars().all(|ch| ch.is_ascii_hexdigit());
            if !ok {
                return Err(AppError::Invalid(format!("强调色格式不正确：{c}")));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WeekStart {
    Monday,
    Sunday,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TimeSettings {
    /// 日切时间 `HH:MM`
    pub day_cutoff: String,
    pub week_start: WeekStart,
}

impl Default for TimeSettings {
    fn default() -> Self {
        Self {
            day_cutoff: "00:00".into(),
            week_start: WeekStart::Monday,
        }
    }
}

/// 模型设置（docs/产品设计.md 第 10 节）。API Key 不在这里，见 secrets.rs
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ModelSettings {
    /// 对话模型，需要支持工具调用
    pub chat: Endpoint,
    /// 识图模型；为空时用对话模型识图
    pub vision: Option<Endpoint>,
}

/// 日切、回顾和报告等后台行为设置。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AutomationSettings {
    pub weekly_report: bool,
    pub monthly_report: bool,
    pub yearly_report: bool,
    pub review_enabled: bool,
    pub review_time: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DesktopSettings {
    pub quick_shortcut: String,
    pub main_shortcut: String,
    pub sticky_shortcut: String,
    pub autostart: bool,
    pub show_stickies_on_startup: bool,
    pub close_to_tray: bool,
}
impl Default for DesktopSettings {
    fn default() -> Self {
        Self {
            quick_shortcut: "Ctrl+Alt+Space".into(),
            main_shortcut: "Ctrl+Alt+Z".into(),
            sticky_shortcut: "Ctrl+Alt+S".into(),
            autostart: !cfg!(debug_assertions),
            show_stickies_on_startup: false,
            close_to_tray: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StickySettings {
    pub color: String,
    pub opacity: f64,
    pub auto_hide: bool,
}
impl Default for StickySettings {
    fn default() -> Self {
        Self {
            color: "theme".into(),
            opacity: 0.9,
            auto_hide: false,
        }
    }
}
impl StickySettings {
    pub fn validate(&self) -> AppResult<()> {
        if !["theme", "amber", "pink", "blue", "green"].contains(&self.color.as_str())
            || !self.opacity.is_finite()
            || !(0.55..=1.0).contains(&self.opacity)
        {
            return Err(AppError::Invalid("便利贴颜色或透明度不正确".into()));
        }
        Ok(())
    }
}

impl Default for AutomationSettings {
    fn default() -> Self {
        Self {
            weekly_report: true,
            monthly_report: true,
            yearly_report: true,
            review_enabled: true,
            review_time: "21:30".into(),
        }
    }
}

impl Default for ModelSettings {
    fn default() -> Self {
        Self {
            chat: Endpoint {
                provider: ProviderId::Deepseek,
                base_url: ProviderId::Deepseek.default_base_url().into(),
                model: "deepseek-chat".into(),
            },
            vision: None,
        }
    }
}

impl ModelSettings {
    pub fn validate(&self) -> AppResult<()> {
        for ep in std::iter::once(&self.chat).chain(self.vision.as_ref()) {
            let url = ep.base_url.trim();
            if !url.is_empty() {
                let parsed = reqwest::Url::parse(url)
                    .map_err(|_| AppError::Invalid(format!("Base URL 格式不正确：{url}")))?;
                if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
                    return Err(AppError::Invalid(
                        "Base URL 须使用 HTTP 或 HTTPS，并包含主机名".into(),
                    ));
                }
            }
            if ep.model.chars().count() > 120 {
                return Err(AppError::Invalid("模型名称太长".into()));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppConfig {
    pub version: u32,
    pub appearance: Appearance,
    pub time: TimeSettings,
    pub model: ModelSettings,
    pub automation: AutomationSettings,
    pub desktop: DesktopSettings,
    pub sticky: StickySettings,
    pub onboarding_completed: bool,
    /// 新版本增加但当前版本不认识的设置，原样保留。
    #[serde(flatten)]
    pub rest: Map<String, Value>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            appearance: Appearance::default(),
            time: TimeSettings::default(),
            model: ModelSettings::default(),
            automation: AutomationSettings::default(),
            desktop: DesktopSettings::default(),
            sticky: StickySettings::default(),
            onboarding_completed: false,
            rest: Map::new(),
        }
    }
}

impl AppConfig {
    pub fn validate(&self) -> AppResult<()> {
        self.appearance.validate()?;
        parse_cutoff(&self.time.day_cutoff)?;
        parse_cutoff(&self.automation.review_time)?;
        self.model.validate()?;
        self.sticky.validate()?;
        Ok(())
    }

    /// 读取配置；不存在则写入默认配置。
    /// 文件损坏或内容不合法时，把原文件改名保留，再使用默认配置，不让应用起不来。
    pub fn load_or_create(path: &Path) -> AppResult<Self> {
        if !path.exists() {
            let cfg = Self::default();
            cfg.save(path)?;
            return Ok(cfg);
        }
        let text = std::fs::read_to_string(path)?;
        let parsed = serde_json::from_str::<Self>(&text)
            .map_err(AppError::from)
            .and_then(|mut cfg| {
                let mut migrated = false;
                if cfg.version < 2 {
                    // v1 的发布默认会同时唤起便利贴，且 Alt+Space 在 Windows 上常被系统工具占用。
                    // 这是预发布配置的一次性默认值迁移；用户之后仍可在设置中重新选择。
                    if cfg.desktop.quick_shortcut == "Alt+Space" {
                        cfg.desktop.quick_shortcut = "Ctrl+Alt+Space".into();
                    }
                    cfg.desktop.show_stickies_on_startup = false;
                    migrated = true;
                }
                if cfg.version < CONFIG_VERSION {
                    cfg.version = CONFIG_VERSION;
                    migrated = true;
                }
                cfg.validate()?;
                if migrated {
                    cfg.save(path)?;
                }
                Ok(cfg)
            });
        match parsed {
            Ok(cfg) => Ok(cfg),
            Err(_) => {
                let stamp = Local::now().format("%Y%m%d-%H%M%S");
                std::fs::rename(path, path.with_extension(format!("json.broken-{stamp}")))?;
                let cfg = Self::default();
                cfg.save(path)?;
                Ok(cfg)
            }
        }
    }

    /// 先写临时文件再替换，避免写到一半断电导致配置损坏
    pub fn save(&self, path: &Path) -> AppResult<()> {
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self)?)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 首次启动写入默认配置() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.json");
        let cfg = AppConfig::load_or_create(&path).unwrap();
        assert_eq!(cfg, AppConfig::default());
        assert!(path.exists());
    }

    #[test]
    fn 保存后读取一致且保留未知字段() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.json");
        std::fs::write(
            &path,
            r##"{"version":1,"appearance":{"theme":"paper","accent":"#336699"},"future":{"x":1}}"##,
        )
        .unwrap();
        let mut cfg = AppConfig::load_or_create(&path).unwrap();
        assert_eq!(cfg.version, CONFIG_VERSION);
        assert_eq!(cfg.desktop.quick_shortcut, "Ctrl+Alt+Space");
        assert!(!cfg.desktop.show_stickies_on_startup);
        assert_eq!(cfg.appearance.theme, ThemeChoice::Paper);
        assert_eq!(cfg.appearance.font_size, Scale::Medium); // 缺省字段取默认值
        assert!(!cfg.onboarding_completed);
        cfg.onboarding_completed = true;
        cfg.appearance.theme = ThemeChoice::Forest;
        cfg.desktop.quick_shortcut = "Ctrl+Shift+K".into();
        cfg.desktop.show_stickies_on_startup = false;
        cfg.desktop.close_to_tray = false;
        cfg.sticky.color = "pink".into();
        cfg.sticky.opacity = 0.75;
        cfg.sticky.auto_hide = true;
        cfg.automation.weekly_report = false;
        cfg.time.day_cutoff = "04:00".into();
        cfg.save(&path).unwrap();
        let again = AppConfig::load_or_create(&path).unwrap();
        assert_eq!(again.appearance.theme, ThemeChoice::Forest);
        assert_eq!(again, cfg);
        assert!(crate::desktop::shortcuts::parse_shortcuts(&again.desktop).is_ok());
        let mut duplicate = again.desktop.clone();
        duplicate.main_shortcut = duplicate.quick_shortcut.clone();
        assert!(crate::desktop::shortcuts::parse_shortcuts(&duplicate).is_err());
        assert_eq!(again.rest.get("future"), Some(&serde_json::json!({"x": 1})));
    }

    #[test]
    fn 损坏的配置被保留并重置() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.json");
        std::fs::write(&path, "{ not json").unwrap();
        let cfg = AppConfig::load_or_create(&path).unwrap();
        assert_eq!(cfg, AppConfig::default());
        let broken = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(Result::ok)
            .any(|e| e.file_name().to_string_lossy().contains("broken"));
        assert!(broken, "原文件应改名保留");
    }

    #[test]
    fn 校验强调色与日切时间() {
        let mut cfg = AppConfig::default();
        cfg.appearance.accent = Some("#12abEF".into());
        assert!(cfg.validate().is_ok());
        cfg.appearance.accent = Some("red".into());
        assert!(cfg.validate().is_err());
        cfg.appearance.accent = None;
        cfg.time.day_cutoff = "25:00".into();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn 模型地址允许_http_和_https() {
        let mut cfg = AppConfig::default();
        for url in [
            "https://example.com/v1",
            "http://example.com/v1",
            "http://localhost:8080/v1",
            "http://127.0.0.1:8080/v1",
            "http://127.0.0.2:8080/v1",
            "http://[::1]:8080/v1",
        ] {
            cfg.model.chat.base_url = url.into();
            assert!(cfg.validate().is_ok(), "应允许 {url}");
        }
        for url in ["ftp://example.com/v1", "not-a-url", "http://"] {
            cfg.model.chat.base_url = url.into();
            assert!(cfg.validate().is_err(), "应拒绝 {url}");
        }
    }

    #[test]
    fn v2_远程_http_配置升级时保留地址() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.json");
        let mut cfg = AppConfig::default();
        cfg.version = 2;
        cfg.model.chat.base_url = "http://example.com/v1".into();
        std::fs::write(&path, serde_json::to_string(&cfg).unwrap()).unwrap();

        let migrated = AppConfig::load_or_create(&path).unwrap();
        assert_eq!(migrated.version, CONFIG_VERSION);
        assert_eq!(migrated.model.chat.base_url, "http://example.com/v1");
        assert_eq!(migrated.model.chat.model, "deepseek-chat");
    }
}
