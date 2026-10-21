//! 大模型代理（docs/产品设计.md 4.4）：
//!
//! 1. 登记：输入先写入收件箱，获得请求 ID；
//! 2. 理解意图：调用模型，查询工具立即执行，写工具只生成变更计划；
//! 3. 校验整份计划；
//! 4. 危险操作保存为"待确认"，本轮暂停（追问同理）；
//! 5. 原子提交（只有这一步持有写事务，等待模型和用户时不占用写锁）；
//! 6. 返回结果，发数据变更事件。

mod clarify;
mod context;
mod intent;
mod project_tools;
mod recur_tools;
mod reply;
mod report_tools;
pub mod tools;

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use base64::Engine;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::clock::Clock;
use crate::config::AppConfig;
use crate::error::{AppError, AppResult};
use crate::llm::{ChatRequest, LlmError, Message, Model, Part, StopReason};
use crate::prompts::{self, PromptId};
use crate::secrets::SecretStore;
use crate::service::changes::{Change, Pin, PreviewItem};
use crate::service::inbox::{NewPending, REASON_UNCONFIGURED, Status};
use crate::service::oplog::Source;
use crate::service::{ChangeSet, Service};

pub use reply::{AgentReply, ReplyKind};

/// 固定执行约束供运行与设置页预览共用，不写入可编辑提示词。
pub(crate) fn runtime_instructions() -> &'static str {
    intent::INSTRUCTIONS
}

pub(crate) fn tool_specs(mode: &str) -> Vec<crate::llm::ToolSpec> {
    intent::specs(tools::specs_for(mode))
}

/// 一轮里最多调用模型的次数，防止工具调用死循环
const MAX_ROUNDS: usize = 8;
/// 待确认 / 待追问的有效期
const PENDING_MINUTES: i64 = 10;
/// 网络失败的重试间隔（秒）与次数
pub const RETRY_DELAYS: [i64; 5] = [30, 60, 120, 300, 600];
pub const MAX_ATTEMPTS: i64 = 6;
pub const MAX_INPUT_CHARS: usize = 20_000;
pub const MAX_IMAGES: usize = 4;
pub const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_IMAGE_TOTAL_BYTES: usize = 20 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Task,
    Project,
}

impl Mode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Task => "task",
            Self::Project => "project",
        }
    }
}

pub struct Input {
    pub source: &'static str,
    pub mode: Mode,
    pub text: String,
    /// data URL
    pub images: Vec<String>,
}

/// 代理运行需要的环境。数据库锁只在同步片段里短暂持有，绝不跨越 await
pub struct Ctx<'a> {
    pub db: &'a Mutex<Connection>,
    pub clock: &'a dyn Clock,
    pub config: AppConfig,
    pub prompts_dir: PathBuf,
    pub inbox_dir: PathBuf,
    pub secrets: &'a dyn SecretStore,
    /// 流式文字（请求 ID, 片段）
    pub on_delta: &'a (dyn Fn(&str, &str) + Send + Sync),
    /// 数据变更（通知所有窗口刷新）
    pub on_changes: &'a (dyn Fn(&ChangeSet) + Send + Sync),
    /// 测试时注入脚本化模型
    #[cfg(test)]
    pub test_model: Option<&'a Model>,
}

/// 追问时保存的上下文，用户回答后接着处理
#[derive(Serialize, Deserialize)]
struct ClarifyContext {
    messages: Vec<Message>,
    call_id: String,
    plan: Vec<Change>,
    #[serde(default)]
    checklist: intent::IntentChecklist,
    #[serde(default)]
    reads: intent::ReadScope,
    #[serde(default)]
    source: String,
    #[serde(default)]
    pins: Vec<Pin>,
    #[serde(default)]
    snapshots: Option<Vec<serde_json::Value>>,
    #[serde(default)]
    entity_options: Vec<clarify::PendingOption>,
}

/// 待确认时保存的上下文
#[derive(Serialize, Deserialize)]
struct ConfirmContext {
    text: String,
    #[serde(default)]
    query_only: bool,
    #[serde(default)]
    snapshots: Option<Vec<serde_json::Value>>,
}

enum ModelRef<'a> {
    Owned(Model),
    #[cfg_attr(not(test), allow(dead_code))]
    Borrowed(&'a Model),
}

impl ModelRef<'_> {
    fn get(&self) -> &Model {
        match self {
            Self::Owned(m) => m,
            Self::Borrowed(m) => m,
        }
    }
}

fn is_yes(t: &str) -> bool {
    let t = t.trim().trim_end_matches(['。', '！', '!', '.', '，', ',']);
    matches!(
        t,
        "确认"
            | "确定"
            | "是"
            | "是的"
            | "好"
            | "好的"
            | "可以"
            | "行"
            | "对"
            | "嗯"
            | "删吧"
            | "确认删除"
            | "执行"
            | "ok"
            | "OK"
    )
}

fn is_no(t: &str) -> bool {
    let t = t.trim().trim_end_matches(['。', '！', '!', '.']);
    matches!(
        t,
        "算了" | "取消" | "不要" | "不用了" | "不" | "不用" | "否" | "别删"
    )
}

fn now_plus_minutes(clock: &dyn Clock, m: i64) -> String {
    (clock.now() + chrono::Duration::minutes(m))
        .format(crate::service::DATETIME_FMT)
        .to_string()
}

impl Ctx<'_> {
    /// 在数据库锁内执行一段业务操作
    fn svc<T>(&self, f: impl FnOnce(&Service<'_>) -> AppResult<T>) -> AppResult<T> {
        let conn = self
            .db
            .lock()
            .map_err(|_| AppError::Invalid("内部状态异常，请重启知了".into()))?;
        let svc = Service::new(&conn, self.clock, &self.config)?;
        f(&svc)
    }

    fn chat_model(&self) -> Result<ModelRef<'_>, LlmError> {
        #[cfg(test)]
        if let Some(m) = self.test_model {
            return Ok(ModelRef::Borrowed(m));
        }
        let ep = &self.config.model.chat;
        let key = self
            .secrets
            .get(ep)
            .ok()
            .flatten()
            .ok_or(LlmError::NotConfigured)?;
        Model::new(ep, key).map(ModelRef::Owned)
    }

    fn vision_model(&self) -> Result<ModelRef<'_>, LlmError> {
        match &self.config.model.vision {
            Some(ep) => {
                #[cfg(test)]
                if let Some(m) = self.test_model {
                    return Ok(ModelRef::Borrowed(m));
                }
                let key = self
                    .secrets
                    .get(ep)
                    .ok()
                    .flatten()
                    .ok_or(LlmError::NotConfigured)?;
                Model::new(ep, key).map(ModelRef::Owned)
            }
            None => self.chat_model(),
        }
    }

    /// 系统提示词：可编辑的提示词 + 程序生成的说明。时间按输入登记时理解（4.6）
    fn system_prompt(&self, received_at: &str, business_date: &str, mode: &str) -> String {
        prompts::system_prompt(
            &self.prompts_dir,
            &self.config,
            received_at,
            business_date,
            mode,
        )
    }
}

fn decoded_len(data: &str) -> Option<usize> {
    let padding = data
        .as_bytes()
        .iter()
        .rev()
        .take_while(|&&b| b == b'=')
        .count();
    data.len()
        .checked_add(3)?
        .checked_div(4)?
        .checked_mul(3)?
        .checked_sub(padding)
}

fn image_parts(images: &[String]) -> AppResult<Vec<(&str, &str)>> {
    if images.len() > MAX_IMAGES {
        return Err(AppError::Invalid(format!(
            "一次最多添加 {MAX_IMAGES} 张图片"
        )));
    }
    let mut total = 0usize;
    images
        .iter()
        .map(|url| {
            let Some(rest) = url.strip_prefix("data:") else {
                return Err(AppError::Invalid("图片格式不支持".into()));
            };
            let Some((meta, data)) = rest.split_once(',') else {
                return Err(AppError::Invalid("图片格式不支持".into()));
            };
            let Some(media_type) = meta.strip_suffix(";base64") else {
                return Err(AppError::Invalid("图片格式不支持".into()));
            };
            if !matches!(
                media_type,
                "image/png" | "image/jpeg" | "image/webp" | "image/gif"
            ) || data.is_empty()
            {
                return Err(AppError::Invalid("图片格式不支持".into()));
            }
            let size =
                decoded_len(data).ok_or_else(|| AppError::Invalid("图片数据已损坏".into()))?;
            if size > MAX_IMAGE_BYTES {
                return Err(AppError::Invalid(format!(
                    "单张图片不能超过 {} MiB",
                    MAX_IMAGE_BYTES / 1024 / 1024
                )));
            }
            total = total
                .checked_add(size)
                .ok_or_else(|| AppError::Invalid("图片总大小过大".into()))?;
            if total > MAX_IMAGE_TOTAL_BYTES {
                return Err(AppError::Invalid(format!(
                    "图片总大小不能超过 {} MiB",
                    MAX_IMAGE_TOTAL_BYTES / 1024 / 1024
                )));
            }
            Ok((media_type, data))
        })
        .collect()
}

pub(crate) fn validate_input(text: &str, images: &[String]) -> AppResult<()> {
    if text.trim().is_empty() && images.is_empty() {
        return Err(AppError::Invalid("请输入内容或选择图片".into()));
    }
    if text.chars().count() > MAX_INPUT_CHARS {
        return Err(AppError::Invalid(format!(
            "文字不能超过 {MAX_INPUT_CHARS} 字"
        )));
    }
    image_parts(images)?;
    Ok(())
}

pub(crate) fn save_images(dir: &Path, id: &str, images: &[String]) -> AppResult<Vec<String>> {
    if images.is_empty() {
        return Ok(Vec::new());
    }
    let id = uuid::Uuid::parse_str(id)
        .map_err(|_| AppError::Invalid("请求编号无效".into()))?
        .to_string();
    let root =
        std::fs::canonicalize(dir).map_err(|_| AppError::Invalid("图片暂存目录不可用".into()))?;
    let parts = image_parts(images)?;
    let mut paths = Vec::with_capacity(parts.len());
    for (i, (media_type, data)) in parts.into_iter().enumerate() {
        let bytes = match base64::engine::general_purpose::STANDARD.decode(data.as_bytes()) {
            Ok(bytes) => bytes,
            Err(_) => {
                drop_images(&root, &paths);
                return Err(AppError::Invalid("图片数据已损坏".into()));
            }
        };
        let ext = media_type
            .trim_start_matches("image/")
            .replace("jpeg", "jpg");
        let path = root.join(format!("{id}-{i}.{ext}"));
        let mut file = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => file,
            Err(error) => {
                drop_images(&root, &paths);
                return Err(error.into());
            }
        };
        paths.push(path.to_string_lossy().into_owned());
        if let Err(error) = std::io::Write::write_all(&mut file, &bytes) {
            drop(file);
            drop_images(&root, &paths);
            return Err(error.into());
        }
    }
    Ok(paths)
}

fn inbox_file(root: &Path, stored: &str) -> AppResult<PathBuf> {
    let root =
        std::fs::canonicalize(root).map_err(|_| AppError::Invalid("图片暂存目录不可用".into()))?;
    let stored = Path::new(stored);
    if !stored.is_absolute() {
        return Err(AppError::Invalid("暂存图片路径无效".into()));
    }
    let link_meta = std::fs::symlink_metadata(stored)
        .map_err(|_| AppError::Invalid("暂存的图片已被清理".into()))?;
    if link_meta.file_type().is_symlink() || !link_meta.file_type().is_file() {
        return Err(AppError::Invalid("暂存图片路径无效".into()));
    }
    let path = std::fs::canonicalize(stored)
        .map_err(|_| AppError::Invalid("暂存的图片已被清理".into()))?;
    if path == root || !path.starts_with(&root) {
        return Err(AppError::Invalid("暂存图片路径无效".into()));
    }
    Ok(path)
}

fn load_images(root: &Path, paths: &[String]) -> AppResult<Vec<Part>> {
    if paths.len() > MAX_IMAGES {
        return Err(AppError::Invalid("暂存图片数量过多，请重新上传".into()));
    }
    let mut total = 0u64;
    paths
        .iter()
        .map(|stored| {
            let path = inbox_file(root, stored)?;
            let size = std::fs::metadata(&path)?.len();
            if size > MAX_IMAGE_BYTES as u64 {
                return Err(AppError::Invalid("暂存图片过大，请重新上传".into()));
            }
            total = total
                .checked_add(size)
                .ok_or_else(|| AppError::Invalid("暂存图片总大小过大".into()))?;
            if total > MAX_IMAGE_TOTAL_BYTES as u64 {
                return Err(AppError::Invalid("暂存图片总大小过大，请重新上传".into()));
            }
            let bytes =
                std::fs::read(&path).map_err(|_| AppError::Invalid("暂存的图片已被清理".into()))?;
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            let media_type = match ext.as_str() {
                "jpg" | "jpeg" => "image/jpeg",
                "png" => "image/png",
                "webp" => "image/webp",
                "gif" => "image/gif",
                _ => return Err(AppError::Invalid("暂存图片格式无效".into())),
            };
            Ok(Part::Image {
                media_type: media_type.into(),
                data: base64::engine::general_purpose::STANDARD.encode(bytes),
            })
        })
        .collect()
}

/// 处理结束后删除暂存图片（图片只识别，不长期保存）
pub(crate) fn drop_images(root: &Path, paths: &[String]) {
    for stored in paths {
        if let Ok(path) = inbox_file(root, stored) {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// 新的一句输入
#[cfg(test)]
pub async fn submit(ctx: &Ctx<'_>, input: Input) -> AppResult<AgentReply> {
    submit_with_id(ctx, input, None).await
}

/// 主窗口从收件箱打开的追问/确认，明确续答该请求（可能来自快速记录或便签）。
pub async fn respond_to(
    ctx: &Ctx<'_>,
    input: Input,
    request_id: Option<String>,
    reply_to: &str,
) -> AppResult<AgentReply> {
    validate_input(&input.text, &input.images)?;
    ctx.svc(|s| s.pending_expire())?;
    let pending = ctx
        .svc(|s| s.pending_get(reply_to))?
        .ok_or_else(|| AppError::Invalid("待处理内容已失效，请重新输入".into()))?;
    let item = ctx.svc(|s| s.inbox_get(reply_to))?;
    if item.mode == input.mode.as_str() {
        if pending.kind == "clarify" {
            return answer(ctx, reply_to, input.text).await;
        }
        if input.images.is_empty() && is_yes(&input.text) {
            return confirm(ctx, reply_to).await;
        }
        if is_no(&input.text) {
            return cancel(ctx, reply_to);
        }
    }
    cancel(ctx, reply_to)?;
    submit_with_id(ctx, input, request_id).await
}

pub async fn submit_with_id(
    ctx: &Ctx<'_>,
    input: Input,
    request_id: Option<String>,
) -> AppResult<AgentReply> {
    validate_input(&input.text, &input.images)?;
    let id = match request_id {
        Some(id) => uuid::Uuid::parse_str(&id)
            .map_err(|_| AppError::Invalid("请求编号无效".into()))?
            .to_string(),
        None => uuid::Uuid::new_v4().to_string(),
    };
    if ctx.svc(|s| s.inbox_exists(&id))? {
        return process(ctx, &id).await;
    }
    ctx.svc(|s| s.pending_expire())?;
    // 有待确认 / 待追问时，这句话可能是对它的回应
    if let Some(p) = ctx.svc(|s| s.pending_active(input.source))? {
        let pending_mode = ctx.svc(|s| s.inbox_get(&p.request_id))?.mode;
        if pending_mode != input.mode.as_str() {
            cancel(ctx, &p.request_id)?;
        } else if p.kind == "confirm" {
            if input.images.is_empty() && is_yes(&input.text) {
                return confirm(ctx, &p.request_id).await;
            }
            if is_no(&input.text) {
                return cancel(ctx, &p.request_id);
            }
            // 说了别的：视为取消原来的确认，处理新的输入
            cancel(ctx, &p.request_id)?;
        } else {
            return answer(ctx, &p.request_id, input.text).await;
        }
    }

    ctx.svc(|s| {
        if !s.inbox_exists(&id)? {
            let paths = save_images(&ctx.inbox_dir, &id, &input.images)?;
            if let Err(error) =
                s.inbox_register(&id, input.source, input.mode.as_str(), &input.text, &paths)
            {
                drop_images(&ctx.inbox_dir, &paths);
                return Err(error);
            }
        }
        Ok(())
    })?;
    process(ctx, &id).await
}

/// 处理收件箱里的一条（新输入、离线恢复、配置好模型后）
pub async fn process(ctx: &Ctx<'_>, id: &str) -> AppResult<AgentReply> {
    cleanup(ctx)?;
    ctx.svc(|s| s.pending_expire())?;
    let item = ctx.svc(|s| s.inbox_get(id))?;
    if item.status == Status::Awaiting {
        let p = ctx
            .svc(|s| s.pending_get(id))?
            .ok_or_else(|| AppError::Invalid("待确认内容已失效，请重新输入".into()))?;
        return if p.kind == "confirm" {
            Ok(AgentReply::confirm(
                id,
                serde_json::from_str(p.preview.as_deref().unwrap_or("[]"))?,
                false,
            ))
        } else {
            let mut reply = AgentReply::clarify(
                id,
                item.result_summary.unwrap_or_else(|| "请补充说明".into()),
                serde_json::from_str(p.preview.as_deref().unwrap_or("[]"))?,
            );
            let context: ClarifyContext =
                serde_json::from_str(p.context.as_deref().unwrap_or("{}"))?;
            reply.entity_options = context
                .entity_options
                .iter()
                .filter(|choice| {
                    choice.expires_at.as_str() > now_plus_minutes(ctx.clock, 0).as_str()
                })
                .map(clarify::PendingOption::view)
                .collect();
            Ok(reply)
        };
    }
    if item.status_reason.as_deref() == Some("images_expired") {
        return Ok(AgentReply::error(
            id,
            "暂存图片已清理，请重新上传图片。".into(),
        ));
    }
    let result = run(ctx, id, None).await;
    if let Err(e) = &result {
        ctx.svc(|s| {
            if s.inbox_get(id)?.status == Status::Processing {
                s.inbox_set(id, Status::Failed, None, Some(&e.to_string()), None)?;
            }
            Ok(())
        })?;
    }
    result
}

async fn run(
    ctx: &Ctx<'_>,
    id: &str,
    resume: Option<(ClarifyContext, String, Option<clarify::PendingOption>)>,
) -> AppResult<AgentReply> {
    let item = ctx.svc(|s| s.inbox_get(id))?;
    if item.status == Status::Done {
        return Ok(AgentReply::info(
            id,
            item.result_summary
                .unwrap_or_else(|| "这件事已经处理过了。".into()),
        ));
    }
    if !ctx.svc(|s| s.inbox_claim(id))? {
        return Ok(AgentReply::info(id, "这条输入正在处理中。".into()));
    }

    let chat = match ctx.chat_model() {
        Ok(m) => m,
        Err(LlmError::NotConfigured) => {
            ctx.svc(|s| s.inbox_set(id, Status::Queued, Some(REASON_UNCONFIGURED), None, None))?;
            return Ok(AgentReply::queued_unconfigured(id));
        }
        Err(e) => return fail(ctx, id, &item.image_paths, &e.to_string()),
    };

    // 识图：先把图片转成文字，结果缓存，重试时不再识别
    let mut user_text = item.content.clone();
    let mut intent_source = item.content.clone();
    if !item.image_paths.is_empty() {
        let described = match &item.vision_text {
            Some(t) => t.clone(),
            None => {
                let images = match load_images(&ctx.inbox_dir, &item.image_paths) {
                    Ok(i) => i,
                    Err(e) => return fail(ctx, id, &item.image_paths, &e.to_string()),
                };
                let vision = match ctx.vision_model() {
                    Ok(m) => m,
                    Err(e) => return fail(ctx, id, &item.image_paths, &e.to_string()),
                };
                let prompt = prompts::render_single(
                    &ctx.prompts_dir,
                    PromptId::Vision,
                    &ctx.config,
                    &item.received_at,
                    &item.business_date,
                    "图片识别",
                );
                let msgs = [Message::User {
                    text: if item.content.is_empty() {
                        "请识别这张图片。".into()
                    } else {
                        item.content.clone()
                    },
                }];
                let req = ChatRequest {
                    system: &prompt,
                    messages: &msgs,
                    images: &images,
                    tools: &[],
                };
                match vision.get().chat(&req, &|_| {}).await {
                    Ok(r) => {
                        ctx.svc(|s| s.inbox_set_vision(id, &r.text))?;
                        r.text
                    }
                    Err(e) => return model_error(ctx, id, &item.image_paths, e),
                }
            }
        };
        user_text = format!(
            "{}\n\n（以下是图片识别出的内容）\n{described}",
            item.content
        );
        intent_source = format!("{}\n{described}", item.content);
    }

    let (mut messages, mut plan, mut checklist, mut reads) = match resume {
        Some((mut c, answer, selection)) => {
            if c.reads.has_unbound_selection()
                || c.entity_options
                    .iter()
                    .any(|option| option.intent_id.is_empty())
                || selection
                    .as_ref()
                    .is_some_and(|option| c.checklist.selection_intent(&option.intent_id).is_err())
            {
                return fail(
                    ctx,
                    id,
                    &item.image_paths,
                    "旧版暂存选择缺少原意图关联，没有执行任何改动，请重新输入完整要求。",
                );
            }
            if !c.plan.is_empty() {
                let current = ctx.svc(|s| s.plan_snapshots(&c.pins));
                if !c
                    .snapshots
                    .as_ref()
                    .zip(current.as_ref().ok())
                    .is_some_and(|(old, new)| clarify::same_snapshots(old, new))
                {
                    return fail(
                        ctx,
                        id,
                        &item.image_paths,
                        "追问期间相关内容已变化，或旧版暂存计划无法安全续答；没有执行改动，请重新输入完整要求。",
                    );
                }
            }
            if let Some(choice) = &selection {
                let current = ctx.svc(|s| s.plan_snapshots(&choice.pins));
                if !current
                    .as_ref()
                    .ok()
                    .is_some_and(|new| clarify::same_snapshots(&choice.snapshots, new))
                {
                    return fail(
                        ctx,
                        id,
                        &item.image_paths,
                        "选项中的内容已经变化，没有执行任何改动，请重新查询后选择。",
                    );
                }
                if let Err(error) = c.checklist.selection_context(&choice.intent_id, &answer) {
                    return fail(
                        ctx,
                        id,
                        &item.image_paths,
                        &format!("{error}，没有执行任何改动。"),
                    );
                }
                c.reads.select(&choice.intent_id, &choice.targets);
            }
            if !c.source.is_empty() {
                intent_source = c.source;
            }
            intent_source.push('\n');
            intent_source.push_str(&answer);
            let mut m = c.messages;
            m.push(Message::ToolResult {
                call_id: c.call_id,
                content: match selection {
                    Some(choice) => serde_json::json!({
                        "answer": answer, "selectedTargets": choice.targets,
                        "intent_id": choice.intent_id,
                        "instruction": "这些是用户为此原意图明确选择的实体，只限定该 intent_id；其它独立意图继续各自处理。选择文字已登记为context；不要重新创建或改绑原意图，不要重新执行旧操作，仍须查询当前状态后继续原意图。"
                    }).to_string(),
                    None => format!("用户回答：{answer}\n程序提示：用 intent_checklist(append=true) 只补充本次回答的原文意图，已登记的原始要求会保留；新增独立操作不能标为context。"),
                },
                is_error: false,
            });
            // 完整对话保留，但回答后已有目标仍须重新查询当前状态。
            (m, c.plan, c.checklist, c.reads.restart())
        }
        None => (
            vec![Message::User { text: user_text }],
            Vec::new(),
            intent::IntentChecklist::default(),
            intent::ReadScope::default(),
        ),
    };

    let mut system = ctx.system_prompt(&item.received_at, &item.business_date, &item.mode);
    system.push_str(runtime_instructions());
    system.push_str(&context::render(ctx.svc(|s| s.conversation_context(id))?));
    if let Some(project_id) = item.target_project_id.filter(|_| item.mode == "project") {
        let project = ctx.svc(|s| {
            Ok(s.query_project_nodes(None)?
                .into_iter()
                .find(|n| n.id == project_id && n.depth == 1))
        })?;
        let Some(project) = project else {
            return fail(
                ctx,
                id,
                &item.image_paths,
                "便利贴绑定的项目已经不存在，请从主窗口选择项目后重试。",
            );
        };
        system.push_str(&format!("\n本次输入来自项目便利贴，未明确指定其它项目时默认作用于以下宿主绑定项目：{}。仍须 project_query 查证，再操作默认项目及其子项。\n", serde_json::json!({"id": project.id, "title": project.title})));
    }
    let specs = tool_specs(&item.mode);
    let mut final_text = String::new();
    let mut finished = false;
    let mut failures = intent::FailureTracker::default();
    for _ in 0..MAX_ROUNDS {
        let req = ChatRequest {
            system: &system,
            messages: &messages,
            images: &[],
            tools: &specs,
        };
        let resp = match chat
            .get()
            // 工具中间文字可能含内部定位或未提交的完成声明，校验前不直出给界面。
            .chat(&req, &|_| {})
            .await
        {
            Ok(r) => r,
            Err(e) => return model_error(ctx, id, &item.image_paths, e),
        };
        match resp.stop {
            StopReason::Refusal => {
                return fail(ctx, id, &item.image_paths, "模型拒绝处理这个请求。");
            }
            StopReason::MaxTokens => {
                return fail(
                    ctx,
                    id,
                    &item.image_paths,
                    "回复太长被截断了，请换个简短的说法。",
                );
            }
            _ => {}
        }
        messages.push(Message::Assistant {
            raw: resp.raw.clone(),
        });
        if resp.tool_calls.is_empty() {
            if let Some(missing) = checklist.incomplete(plan.len(), &intent_source) {
                if failures.repeated(&format!("coverage:{missing}")) {
                    return fail(
                        ctx,
                        id,
                        &item.image_paths,
                        "模型未能完整整理本次要求，没有执行任何改动。请重新说明需要处理的内容。",
                    );
                }
                messages.push(Message::User {
                    text: format!("程序校验：{missing} 当前没有提交任何改动。"),
                });
                continue;
            }
            let visible_text = if plan.is_empty() {
                resp.text.clone()
            } else {
                checklist.query_answers()
            };
            if reads.has_internal_reference(&visible_text) {
                if failures.repeated("reply:internal-reference") {
                    return fail(
                        ctx,
                        id,
                        &item.image_paths,
                        "模型未能用名称和时间准确说明本次内容，没有执行任何改动。请重新说明需要处理的内容。",
                    );
                }
                messages.push(Message::User { text: "回复及intent_resolve的evidence不能使用内部编号或第几条定位，请改用任务标题、计划日期、提醒时间或完整项目路径。若已有查询结论含编号，请重新intent_resolve修正evidence；不要重复调用已经成功的写工具。".into() });
                continue;
            }
            final_text = resp.text;
            if plan.is_empty() && !final_text.trim().is_empty() {
                (ctx.on_delta)(id, &clean_reply(&final_text));
            }
            finished = true;
            break;
        }
        let calls = resp.tool_calls;
        for (i, call) in calls.iter().enumerate() {
            let before = plan.len();
            let mut owner = None;
            let prepared = if !specs.iter().any(|spec| spec.name == call.name) {
                Err(format!(
                    "当前模式不能使用 {}，请用 Ctrl+Tab 切换模式。",
                    call.name
                ))
            } else if intent::is_write(&call.name) {
                checklist
                    .prepare(call, &reads, &intent_source)
                    .map(|(call, id)| {
                        owner = Some(id);
                        call
                    })
            } else {
                Ok(call.clone())
            };
            let outcome = match prepared {
                Err(error) => tools::Outcome::Error(error),
                Ok(call) if call.name == "intent_checklist" => {
                    match checklist.declare(call.arguments.as_ref(), &intent_source, plan.len()) {
                        Ok(text) => tools::Outcome::Ok(text),
                        Err(error) => tools::Outcome::Error(error),
                    }
                }
                Ok(call) if call.name == "intent_resolve" => {
                    match checklist.resolve(call.arguments.as_ref(), &reads) {
                        Ok(text) => tools::Outcome::Ok(text),
                        Err(error) => tools::Outcome::Error(error),
                    }
                }
                Ok(call) if call.name == "report_get" => report_tools::handle(ctx, &call),
                Ok(call) if call.name == "ask_clarify" => {
                    ctx.svc(|s| Ok(clarify::handle(s, &call, &reads, &checklist)))?
                }
                Ok(call) => ctx.svc(|s| {
                    Ok(tools::handle_mode(
                        s,
                        &call,
                        &mut plan,
                        &item.business_date,
                        &item.mode,
                    ))
                })?,
            };
            if let tools::Outcome::Ok(content) = &outcome {
                reads.observe(&call.name, content);
                if let Some(owner) = owner {
                    checklist.record(&owner, before, plan.len());
                }
            }
            match outcome {
                tools::Outcome::Ok(content) => messages.push(Message::ToolResult {
                    call_id: call.id.clone(),
                    content,
                    is_error: false,
                }),
                tools::Outcome::Error(content) => {
                    if failures.repeated(&format!("{}:{content}", call.name)) {
                        let reason = if call.name.starts_with("intent_")
                            || content.contains("意图")
                            || content.contains("原文")
                        {
                            "模型未能完整整理本次要求"
                        } else if call.name == "ask_clarify" {
                            "模型未能生成可辨认的选择内容"
                        } else {
                            "模型未能准确匹配并处理当前内容"
                        };
                        return fail(
                            ctx,
                            id,
                            &item.image_paths,
                            &format!("{reason}，没有执行任何改动。请用名称和日期重新说明。"),
                        );
                    }
                    messages.push(Message::ToolResult {
                        call_id: call.id.clone(),
                        content,
                        is_error: true,
                    });
                }
                tools::Outcome::Clarify {
                    question,
                    options,
                    mut entity_options,
                } => {
                    // 同一轮里后面的调用不再执行，但每个调用都要有结果
                    for rest in &calls[i + 1..] {
                        messages.push(Message::ToolResult {
                            call_id: rest.id.clone(),
                            content: "未执行：先等用户回答追问".into(),
                            is_error: true,
                        });
                    }
                    let (pins, snapshots) = ctx.svc(|s| {
                        let pins = s.validate_plan(&plan)?.pins;
                        let snapshots = s.plan_snapshots(&pins)?;
                        Ok((pins, snapshots))
                    })?;
                    let expires = now_plus_minutes(ctx.clock, PENDING_MINUTES);
                    for choice in &mut entity_options {
                        choice.expires_at = expires.clone();
                    }
                    let option_views = entity_options
                        .iter()
                        .map(clarify::PendingOption::view)
                        .collect();
                    let context = ClarifyContext {
                        messages,
                        call_id: call.id.clone(),
                        plan,
                        checklist,
                        reads,
                        source: intent_source,
                        pins,
                        snapshots: Some(snapshots),
                        entity_options,
                    };
                    let json = serde_json::to_string(&context)?;
                    let options_json = serde_json::to_string(&options)?;
                    ctx.svc(|s| {
                        s.pending_save(NewPending {
                            request_id: id,
                            kind: "clarify",
                            plan: "[]",
                            versions: "[]",
                            context: Some(&json),
                            preview: Some(&options_json),
                            expires_at: &expires,
                        })?;
                        s.inbox_set(id, Status::Awaiting, Some("clarify"), None, Some(&question))
                    })?;
                    let mut reply = AgentReply::clarify(id, question, options);
                    reply.entity_options = option_views;
                    return Ok(reply);
                }
            }
        }
    }
    if !finished {
        return fail(
            ctx,
            id,
            &item.image_paths,
            "模型未能在本次处理中完成解析，没有执行任何改动。请用名称和日期重新说明。",
        );
    }
    let text = if plan.is_empty() {
        final_text
    } else {
        checklist.query_answers()
    };
    finish(ctx, id, &item.image_paths, plan, text).await
}

async fn answer(ctx: &Ctx<'_>, id: &str, text: String) -> AppResult<AgentReply> {
    let p = ctx
        .svc(|s| s.pending_get(id))?
        .ok_or_else(|| AppError::Invalid("这个问题已经过期了，请重新说一遍".into()))?;
    let context: ClarifyContext = serde_json::from_str(p.context.as_deref().unwrap_or("{}"))?;
    ctx.svc(|s| {
        s.pending_delete(id)?;
        s.inbox_set(id, Status::Queued, None, None, None)
    })?;
    run(ctx, id, Some((context, text, None))).await
}

/// 结构化选项的独立入口：一次性值不进入输入正文、历史或模型消息。
pub async fn respond_selection(
    ctx: &Ctx<'_>,
    reply_to: &str,
    selection_token: &str,
) -> AppResult<AgentReply> {
    ctx.svc(|s| s.pending_expire())?;
    // 在同一个数据库锁内校验和消费，重复点击不能重新把已完成请求置回排队。
    let (context, choice) = ctx.svc(|s| {
        let pending = s
            .pending_get(reply_to)?
            .filter(|p| p.kind == "clarify")
            .ok_or_else(|| AppError::Invalid("这个选项已经失效，请重新查询后选择".into()))?;
        if s.inbox_get(reply_to)?.status != Status::Awaiting {
            return Err(AppError::Invalid(
                "这个选项已经处理过了，请勿重复选择".into(),
            ));
        }
        let mut context: ClarifyContext =
            serde_json::from_str(pending.context.as_deref().unwrap_or("{}"))?;
        let choice = context
            .entity_options
            .iter()
            .find(|option| option.value == selection_token)
            .cloned()
            .ok_or_else(|| AppError::Invalid("这个选项不属于当前问题，请重新选择".into()))?;
        if choice.expires_at.as_str() <= now_plus_minutes(ctx.clock, 0).as_str() {
            return Err(AppError::Invalid(
                "这个选项已经过期，请重新查询后选择".into(),
            ));
        }
        context.entity_options.clear();
        s.pending_delete(reply_to)?;
        s.inbox_set(reply_to, Status::Queued, None, None, None)?;
        Ok((context, choice))
    })?;
    let text = choice.label.clone();
    run(ctx, reply_to, Some((context, text, Some(choice)))).await
}

async fn finish(
    ctx: &Ctx<'_>,
    id: &str,
    images: &[String],
    plan: Vec<Change>,
    text: String,
) -> AppResult<AgentReply> {
    if plan.is_empty() {
        let text = if text.trim().is_empty() {
            "知了。".to_string()
        } else {
            clean_reply(&text)
        };
        ctx.svc(|s| s.inbox_set(id, Status::Done, None, None, Some(&text)))?;
        drop_images(&ctx.inbox_dir, images);
        return Ok(AgentReply::info(id, text));
    }
    let validated = match ctx.svc(|s| s.validate_plan(&plan)) {
        Ok(v) => v,
        Err(e) => return fail(ctx, id, images, &format!("没有执行：{e}")),
    };
    if validated.dangerous {
        return ask_confirm(
            ctx,
            id,
            &plan,
            &validated.pins,
            &validated.preview,
            &text,
            false,
        );
    }
    commit(
        ctx,
        id,
        images,
        &plan,
        &validated.pins,
        &validated.preview,
        &text,
        None,
    )
}

fn ask_confirm(
    ctx: &Ctx<'_>,
    id: &str,
    plan: &[Change],
    pins: &[Pin],
    preview: &[PreviewItem],
    text: &str,
    changed: bool,
) -> AppResult<AgentReply> {
    let plan_json = serde_json::to_string(plan)?;
    let pins_json = serde_json::to_string(pins)?;
    let preview_json = serde_json::to_string(preview)?;
    let context = serde_json::to_string(&ConfirmContext {
        text: text.into(),
        query_only: true,
        snapshots: Some(ctx.svc(|s| s.plan_snapshots(pins))?),
    })?;
    let expires = now_plus_minutes(ctx.clock, PENDING_MINUTES);
    ctx.svc(|s| {
        s.pending_save(NewPending {
            request_id: id,
            kind: "confirm",
            plan: &plan_json,
            versions: &pins_json,
            context: Some(&context),
            preview: Some(&preview_json),
            expires_at: &expires,
        })?;
        s.inbox_set(id, Status::Awaiting, Some("confirm"), None, None)
    })?;
    Ok(AgentReply::confirm(id, preview.to_vec(), changed))
}

fn commit(
    ctx: &Ctx<'_>,
    id: &str,
    images: &[String],
    plan: &[Change],
    pins: &[Pin],
    preview: &[PreviewItem],
    text: &str,
    expected: Option<&[serde_json::Value]>,
) -> AppResult<AgentReply> {
    // 写入结果完全由实际通过校验并提交的操作生成，模型的结束语不能虚报或泄露内部编号。
    let lead = if preview.is_empty() {
        "知了，当前内容已经符合要求。".to_string()
    } else {
        "知了，已完成以下操作。".to_string()
    };
    let result = ctx.svc(|s| {
        let item = s.inbox_get(id)?;
        if item.status == Status::Done {
            return Ok(Err(item
                .result_summary
                .unwrap_or_else(|| "这件事已经处理过了。".into())));
        }
        if let Some(expected) = expected {
            let current = s.plan_snapshots(pins)?;
            if expected.len() != current.len()
                || expected.iter().zip(&current).any(|(old, new)| {
                    let entity = old["entity"].as_str().unwrap_or("");
                    let old = &old["row"];
                    let new = &new["row"];
                    old["created_at"] != new["created_at"]
                        || (new["version"].as_i64() <= old["version"].as_i64()
                            && !clarify::same_row(entity, old, new))
                })
            {
                return Err(AppError::Invalid(
                    "待确认的内容已经被替换，没有执行任何改动，请重新查询".into(),
                ));
            }
        }
        let before: Vec<_> = s
            .parent_completion_candidates()?
            .into_iter()
            .map(|n| n.id)
            .collect();
        let applied = s.apply_plan(plan, pins, id, Source::Chat)?;
        let mut lead = lead.clone();
        if !text.trim().is_empty() {
            lead.push_str("\n\n查询结果：\n");
            lead.push_str(&clean_reply(text));
        }
        if !applied.already {
            for parent in s
                .parent_completion_candidates()?
                .into_iter()
                .filter(|n| !before.contains(&n.id))
            {
                lead.push_str(&format!(
                    "\n「{}」的子项都已完成，要把它也标为完成吗？可以在项目模式说：完成「{}」。",
                    parent.title, parent.title
                ));
            }
        }
        s.pending_delete(id)?;
        let summary = if preview.is_empty() {
            lead.clone()
        } else {
            format!(
                "{lead}\n{}",
                preview
                    .iter()
                    .map(|p| format!("- {}", p.text))
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        };
        s.inbox_set(id, Status::Done, None, None, Some(&summary))?;
        Ok(Ok((applied, lead)))
    });
    match result {
        Ok(Err(summary)) => Ok(AgentReply::info(id, summary)),
        Ok(Ok((applied, lead))) => {
            drop_images(&ctx.inbox_dir, images);
            if !applied.already {
                (ctx.on_changes)(&applied.changes);
            }
            Ok(AgentReply::done(
                id,
                lead,
                preview.to_vec(),
                applied.changes,
            ))
        }
        // 确认之前相关任务有变化：重新校验并重新预览，不覆盖用户的改动
        Err(AppError::Conflict(_)) => match ctx.svc(|s| s.validate_plan(plan)) {
            Ok(v) => ask_confirm(ctx, id, plan, &v.pins, &v.preview, text, true),
            Err(e) => fail(
                ctx,
                id,
                images,
                &format!("相关的任务已经变了，没有执行：{e}"),
            ),
        },
        Err(e) => fail(ctx, id, images, &format!("没有执行：{e}")),
    }
}

fn clean_reply(text: &str) -> String {
    text.trim_matches(|c: char| c.is_whitespace() || matches!(c, '\u{feff}' | '\u{200b}'))
        .to_string()
}

/// 确认按钮或回复"确认"：直接由 Rust 执行，不再经过模型
pub async fn confirm(ctx: &Ctx<'_>, id: &str) -> AppResult<AgentReply> {
    ctx.svc(|s| s.pending_expire())?;
    let item = ctx.svc(|s| s.inbox_get(id))?;
    if item.status == Status::Done {
        return Ok(AgentReply::info(
            id,
            item.result_summary
                .unwrap_or_else(|| "这件事已经处理过了。".into()),
        ));
    }
    let p = ctx
        .svc(|s| s.pending_get(id))?
        .filter(|p| p.kind == "confirm")
        .ok_or_else(|| AppError::Invalid("这个确认已经过期了，请重新说一遍".into()))?;
    let plan: Vec<Change> = serde_json::from_str(&p.plan)?;
    let pins: Vec<Pin> = serde_json::from_str(&p.versions)?;
    let preview: Vec<PreviewItem> = serde_json::from_str(p.preview.as_deref().unwrap_or("[]"))?;
    let context = serde_json::from_str::<ConfirmContext>(p.context.as_deref().unwrap_or("{}"))?;
    let item = ctx.svc(|s| s.inbox_get(id))?;
    let Some(snapshots) = context.snapshots else {
        return fail(
            ctx,
            id,
            &item.image_paths,
            "旧版暂存的确认缺少安全快照，没有执行任何改动，请重新输入要求。",
        );
    };
    let text = if context.query_only {
        context.text
    } else {
        String::new()
    };
    commit(
        ctx,
        id,
        &item.image_paths,
        &plan,
        &pins,
        &preview,
        &text,
        Some(&snapshots),
    )
}

pub fn cancel(ctx: &Ctx<'_>, id: &str) -> AppResult<AgentReply> {
    let item = ctx.svc(|s| {
        let item = s.inbox_get(id)?;
        if item.status == Status::Processing {
            return Err(AppError::Invalid("正在处理，请稍后再取消".into()));
        }
        if item.status == Status::Done {
            return Ok(item);
        }
        s.pending_delete(id)?;
        s.inbox_set(id, Status::Done, Some("cancelled"), None, Some("已取消"))?;
        s.inbox_get(id)
    })?;
    drop_images(&ctx.inbox_dir, &item.image_paths);
    Ok(AgentReply::info(
        id,
        item.result_summary.unwrap_or_else(|| "已取消".into()),
    ))
}

/// 放弃收件箱里的一条
pub fn discard(ctx: &Ctx<'_>, id: &str) -> AppResult<()> {
    let item = ctx.svc(|s| {
        let item = s.inbox_get(id)?;
        if item.status == Status::Processing {
            return Err(AppError::Invalid("正在处理，请稍后再放弃".into()));
        }
        if item.status == Status::Done {
            return Ok(item);
        }
        s.pending_delete(id)?;
        s.inbox_set(id, Status::Done, Some("discarded"), None, Some("已放弃"))?;
        s.inbox_get(id)
    })?;
    drop_images(&ctx.inbox_dir, &item.image_paths);
    Ok(())
}

fn model_error(ctx: &Ctx<'_>, id: &str, images: &[String], e: LlmError) -> AppResult<AgentReply> {
    if e.retryable() {
        let msg = e.to_string();
        let again = ctx.svc(|s| s.inbox_retry_later(id, &msg, MAX_ATTEMPTS, &RETRY_DELAYS))?;
        if again {
            return Ok(AgentReply::queued_offline(id, msg));
        }
        return Ok(AgentReply::error(id, format!("多次重试仍然失败：{msg}")));
    }
    if e == LlmError::NotConfigured {
        ctx.svc(|s| s.inbox_set(id, Status::Queued, Some(REASON_UNCONFIGURED), None, None))?;
        return Ok(AgentReply::queued_unconfigured(id));
    }
    fail(ctx, id, images, &e.to_string())
}

fn fail(ctx: &Ctx<'_>, id: &str, _images: &[String], msg: &str) -> AppResult<AgentReply> {
    let msg = if clarify::has_internal_reference(msg) {
        "当前内容无法准确定位，没有执行任何改动，请用名称和日期重新说明。"
    } else {
        msg
    };
    ctx.svc(|s| {
        s.pending_delete(id)?;
        s.inbox_set(id, Status::Failed, None, Some(msg), None)
    })?;
    Ok(AgentReply::error(id, msg.into()))
}

/// 清理时只允许操作当前收件箱目录内的普通文件。
pub fn cleanup(ctx: &Ctx<'_>) -> AppResult<bool> {
    let paths = ctx.svc(|s| s.inbox_cleanup())?;
    let changed = !paths.is_empty();
    for path in paths {
        let remove_reference = match inbox_file(&ctx.inbox_dir, &path) {
            Ok(file) => match std::fs::remove_file(file) {
                Ok(()) => true,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => true,
                Err(_) => false,
            },
            // 恢复自其他电脑的绝对路径或越界路径只从记录中丢弃，绝不操作该文件。
            Err(_) => true,
        };
        if remove_reference {
            ctx.svc(|s| s.inbox_image_removed(&path))?;
        }
    }
    Ok(changed)
}

#[cfg(test)]
mod tests;
