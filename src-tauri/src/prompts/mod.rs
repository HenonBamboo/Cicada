//! 提示词管理（docs/产品设计.md 4.7）。
//!
//! - 内置默认版本编译进程序；
//! - %APPDATA%\Cicada\prompts\ 下有同名文件时优先使用（用户修改后的版本）；
//! - 提示词里的 {{变量}} 在调用前由程序填充；
//! - 工具定义、数据上下文、安全校验由程序生成，不在可编辑的提示词里。

use std::path::Path;

use crate::config::{AppConfig, WeekStart};
use crate::error::{AppError, AppResult};
use chrono::Datelike;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptId {
    Base,
    Task,
    Project,
    Vision,
    Summary,
    Report,
    Greeting,
}

impl PromptId {
    pub fn parse(id: &str) -> AppResult<Self> {
        Self::ALL
            .into_iter()
            .find(|p| p.key() == id)
            .ok_or_else(|| AppError::Invalid("提示词不存在".into()))
    }

    pub fn key(self) -> &'static str {
        self.file_name().trim_end_matches(".md")
    }
    pub const ALL: [PromptId; 7] = [
        Self::Base,
        Self::Task,
        Self::Project,
        Self::Vision,
        Self::Summary,
        Self::Report,
        Self::Greeting,
    ];

    pub fn file_name(self) -> &'static str {
        match self {
            Self::Base => "base.md",
            Self::Task => "task.md",
            Self::Project => "project.md",
            Self::Vision => "vision.md",
            Self::Summary => "summary.md",
            Self::Report => "report.md",
            Self::Greeting => "greeting.md",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Base => "通用角色",
            Self::Task => "任务模式",
            Self::Project => "项目模式",
            Self::Vision => "图片识别",
            Self::Summary => "总结",
            Self::Report => "周报 / 月报 / 年报",
            Self::Greeting => "问候语",
        }
    }

    pub fn default_text(self) -> &'static str {
        match self {
            Self::Base => include_str!("defaults/base.md"),
            Self::Task => include_str!("defaults/task.md"),
            Self::Project => include_str!("defaults/project.md"),
            Self::Vision => include_str!("defaults/vision.md"),
            Self::Summary => include_str!("defaults/summary.md"),
            Self::Report => include_str!("defaults/report.md"),
            Self::Greeting => include_str!("defaults/greeting.md"),
        }
    }
}

/// 读取提示词：用户文件优先，读不到或为空时使用默认版本
pub fn load(dir: &Path, id: PromptId) -> String {
    match std::fs::read_to_string(dir.join(id.file_name())) {
        Ok(s) if !s.trim().is_empty() => s,
        _ => id.default_text().to_string(),
    }
}

/// 填充 {{变量}}。不认识的变量原样保留，方便用户发现写错
pub fn render(template: &str, vars: &[(&str, &str)]) -> String {
    let mut out = template.to_string();
    for (k, v) in vars {
        out = out.replace(&format!("{{{{{k}}}}}"), v);
    }
    out
}

const README: &str = "\
# 知了 · 提示词

这里存放你调整过的提示词。知了每次调用模型前都会先看这里：
有同名文件就用你的版本，没有就用内置的默认版本。

调整方法：
1. 打开 defaults 文件夹，找到想改的提示词，例如 task.md（任务模式）；
2. 把它复制到当前文件夹（和这个 README 放在一起），再修改；
3. 下一次对话起生效。想恢复默认，删掉你复制的那个文件即可。

文件对照：
base.md 通用角色 · task.md 任务模式 · project.md 项目模式 · vision.md 图片识别
summary.md 总结 · report.md 周报/月报/年报 · greeting.md 问候语

可用的变量（会被自动替换）：
{{now}} 当前时间　{{weekday}} 星期　{{business_date}} 业务日期
{{day_cutoff}} 日切时间　{{week_start}} 每周起始日　{{mode}} 当前模式

注意：改提示词只影响模型怎么理解和回答；能改哪些数据始终由程序校验，
提示词写错了也不会越权修改数据。defaults 文件夹每次启动都会被重新写入，请不要直接修改它。
";

/// 写出默认版本供参考，并附上说明。defaults/ 每次启动覆盖，用户文件不受影响
pub fn write_reference(dir: &Path) -> AppResult<()> {
    let defaults = dir.join("defaults");
    std::fs::create_dir_all(&defaults)?;
    for id in PromptId::ALL {
        // 升级前保存用户修改所依据的旧默认，不覆盖用户文本。
        let baseline = dir.join(".versions").join(id.file_name());
        if dir.join(id.file_name()).is_file() && !baseline.exists() {
            let old = std::fs::read_to_string(defaults.join(id.file_name()))
                .unwrap_or_else(|_| id.default_text().into());
            std::fs::create_dir_all(dir.join(".versions"))?;
            atomic_write(&baseline, &old)?;
        }
        std::fs::write(defaults.join(id.file_name()), id.default_text())?;
    }
    std::fs::write(dir.join("README.md"), README)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 用户文件优先_删除后恢复默认() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(
            load(tmp.path(), PromptId::Task),
            PromptId::Task.default_text()
        );
        std::fs::write(tmp.path().join("task.md"), "我的任务提示词").unwrap();
        assert_eq!(load(tmp.path(), PromptId::Task), "我的任务提示词");
        std::fs::write(tmp.path().join("task.md"), "  \n").unwrap();
        assert_eq!(
            load(tmp.path(), PromptId::Task),
            PromptId::Task.default_text()
        );
        std::fs::remove_file(tmp.path().join("task.md")).unwrap();
        assert_eq!(
            load(tmp.path(), PromptId::Task),
            PromptId::Task.default_text()
        );
    }

    #[test]
    fn 填充变量_未知变量保留() {
        let s = render(
            "今天 {{business_date}}，{{unknown}}",
            &[("business_date", "2026-10-17")],
        );
        assert_eq!(s, "今天 2026-10-17，{{unknown}}");
    }

    #[test]
    fn 写出参考文件不覆盖用户文件() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("base.md"), "自定义").unwrap();
        write_reference(tmp.path()).unwrap();
        assert!(tmp.path().join("defaults/greeting.md").is_file());
        assert!(tmp.path().join("README.md").is_file());
        assert_eq!(load(tmp.path(), PromptId::Base), "自定义");
    }

    #[test]
    fn 默认提示词都不为空() {
        for id in PromptId::ALL {
            assert!(!id.default_text().trim().is_empty(), "{}", id.title());
        }
    }
}

/// 与代理请求共用的完整系统提示词。
pub fn system_prompt(
    dir: &Path,
    config: &AppConfig,
    received_at: &str,
    business_date: &str,
    mode: &str,
) -> String {
    const WEEK: [&str; 7] = ["周一", "周二", "周三", "周四", "周五", "周六", "周日"];
    let weekday = chrono::NaiveDate::parse_from_str(business_date, crate::service::DATE_FMT)
        .map(|d| WEEK[d.weekday().num_days_from_monday() as usize])
        .unwrap_or("");
    let now = received_at.replace('T', " ");
    let now = now.get(..16).unwrap_or(&now);
    let mode_name = if mode == "project" {
        "项目模式"
    } else {
        "任务模式"
    };
    let week_start = match config.time.week_start {
        WeekStart::Monday => "周一",
        WeekStart::Sunday => "周日",
    };
    let vars = [
        ("now", now),
        ("weekday", weekday),
        ("business_date", business_date),
        ("day_cutoff", config.time.day_cutoff.as_str()),
        ("week_start", week_start),
        ("mode", mode_name),
    ];

    let mode_prompt = if mode == "project" {
        PromptId::Project
    } else {
        PromptId::Task
    };
    [
        load(dir, PromptId::Base),
        load(dir, mode_prompt),
        load(dir, PromptId::Summary),
    ]
    .iter()
    .map(|p| render(p, &vars))
    .collect::<Vec<_>>()
    .join("\n\n")
}

#[derive(Serialize)]
pub struct Variable {
    pub name: &'static str,
    pub description: &'static str,
}

fn variables() -> Vec<Variable> {
    [
        ("now", "当前时间"),
        ("weekday", "星期"),
        ("business_date", "业务日期"),
        ("day_cutoff", "日切时间"),
        ("week_start", "每周起始日"),
        ("mode", "当前模式"),
    ]
    .into_iter()
    .map(|(name, description)| Variable { name, description })
    .collect()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptView {
    pub id: &'static str,
    pub title: &'static str,
    pub content: String,
    pub default_content: &'static str,
    pub modified: bool,
    pub default_updated: bool,
    pub unknown_variables: Vec<String>,
    pub variables: Vec<Variable>,
}

fn unknown_variables(text: &str) -> Vec<String> {
    let known = variables();
    let mut result = Vec::new();
    let mut remaining = text;
    while let Some((_, tail)) = remaining.split_once("{{") {
        let Some((name, rest)) = tail.split_once("}}") else {
            break;
        };
        if !known.iter().any(|v| v.name == name) && !result.iter().any(|v| v == name) {
            result.push(name.to_string());
        }
        remaining = rest;
    }
    result
}

pub fn view(dir: &Path, id: PromptId) -> AppResult<PromptView> {
    let content = load(dir, id);
    let modified = content != id.default_text();
    let baseline = dir.join(".versions").join(id.file_name());
    let default_updated =
        modified && baseline.is_file() && std::fs::read_to_string(baseline)? != id.default_text();
    Ok(PromptView {
        id: id.key(),
        title: id.title(),
        unknown_variables: unknown_variables(&content),
        content,
        default_content: id.default_text(),
        modified,
        default_updated,
        variables: variables(),
    })
}

fn atomic_write(path: &Path, content: &str) -> AppResult<()> {
    use std::io::Write;
    let parent = path
        .parent()
        .ok_or_else(|| AppError::Invalid("提示词路径无效".into()))?;
    let io_error = |action: &str, error: std::io::Error| {
        AppError::Invalid(format!(
            "提示词文件{action}失败（{}）：{error}",
            path.display()
        ))
    };
    std::fs::create_dir_all(parent).map_err(|error| io_error("创建目录", error))?;
    let tmp = parent.join(format!(".prompt-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> AppResult<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
            .map_err(|error| io_error("创建临时文件", error))?;
        file.write_all(content.as_bytes())
            .map_err(|error| io_error("写入", error))?;
        file.sync_all().map_err(|error| io_error("同步", error))?;
        drop(file);
        replace_file(&tmp, path).map_err(|error| io_error("替换", error))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

#[cfg(windows)]
fn replace_file(from: &Path, to: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    let from: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
    let to: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: 两个路径缓冲区均为有效且以 NUL 结尾的 UTF-16 字符串。
    if unsafe {
        MoveFileExW(
            from.as_ptr(),
            to.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn replace_file(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::rename(from, to)
}

pub fn save(dir: &Path, id: PromptId, content: &str) -> AppResult<PromptView> {
    if content.trim().is_empty() || content.len() > 128 * 1024 {
        return Err(AppError::Invalid(
            "提示词不能为空，且不能超过 128 KB".into(),
        ));
    }
    if content == id.default_text() {
        return reset(dir, id);
    }
    let baseline = dir.join(".versions").join(id.file_name());
    if !baseline.exists() {
        atomic_write(&baseline, id.default_text())?;
    }
    atomic_write(&dir.join(id.file_name()), content)?;
    view(dir, id)
}

pub fn reset(dir: &Path, id: PromptId) -> AppResult<PromptView> {
    for path in [
        dir.join(id.file_name()),
        dir.join(".versions").join(id.file_name()),
    ] {
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    view(dir, id)
}

pub fn render_single(
    dir: &Path,
    id: PromptId,
    config: &AppConfig,
    now: &str,
    date: &str,
    mode: &str,
) -> String {
    let weekday = chrono::NaiveDate::parse_from_str(date, crate::service::DATE_FMT)
        .map(|d| {
            ["周一", "周二", "周三", "周四", "周五", "周六", "周日"]
                [d.weekday().num_days_from_monday() as usize]
        })
        .unwrap_or("");
    render(
        &load(dir, id),
        &[
            ("now", now),
            ("weekday", weekday),
            ("business_date", date),
            ("day_cutoff", &config.time.day_cutoff),
            (
                "week_start",
                match config.time.week_start {
                    WeekStart::Monday => "周一",
                    WeekStart::Sunday => "周日",
                },
            ),
            ("mode", mode),
        ],
    )
}

#[cfg(test)]
mod settings_tests {
    use super::*;
    #[test]
    fn 编辑恢复与默认升级保留用户文本() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(PromptId::parse("../config").is_err());
        write_reference(tmp.path()).unwrap();
        let saved = save(tmp.path(), PromptId::Task, "我的规则 {{now}} {{oops}}").unwrap();
        assert_eq!(saved.unknown_variables, vec!["oops"]);
        assert!(saved.modified);
        std::fs::write(tmp.path().join(".versions/task.md"), "旧默认").unwrap();
        write_reference(tmp.path()).unwrap();
        assert!(view(tmp.path(), PromptId::Task).unwrap().default_updated);
        assert_eq!(
            load(tmp.path(), PromptId::Task),
            "我的规则 {{now}} {{oops}}"
        );
        assert!(!reset(tmp.path(), PromptId::Task).unwrap().modified);
        assert!(save(tmp.path(), PromptId::Task, " ").is_err());
    }
}
