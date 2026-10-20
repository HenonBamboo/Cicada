//! 收件箱：所有输入先登记在这里，再处理（docs/产品设计.md 4.6）。
//! 收件箱是"待处理的输入"，不是业务数据，不写 op_log。

use rusqlite::{OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

use super::{DATE_FMT, Service};
use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// 已暂存：等待处理（离线、未配置模型）
    Queued,
    Processing,
    /// 待确认：危险操作的确认，或等待回答追问
    Awaiting,
    Done,
    Failed,
}

impl Status {
    fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Processing => "processing",
            Self::Awaiting => "awaiting",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }
    fn parse(s: &str) -> Self {
        match s {
            "processing" => Self::Processing,
            "awaiting" => Self::Awaiting,
            "done" => Self::Done,
            "failed" => Self::Failed,
            _ => Self::Queued,
        }
    }
}

/// 暂存原因
pub const REASON_UNCONFIGURED: &str = "unconfigured";
pub const REASON_OFFLINE: &str = "offline";

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InboxItem {
    pub id: String,
    pub source: String,
    pub mode: String,
    pub content: String,
    pub image_paths: Vec<String>,
    pub received_at: String,
    pub business_date: String,
    pub status: Status,
    pub status_reason: Option<String>,
    pub attempts: i64,
    pub last_error: Option<String>,
    pub result_summary: Option<String>,
    pub target_project_id: Option<i64>,
    #[serde(skip)]
    pub vision_text: Option<String>,
}

fn read(r: &Row<'_>) -> rusqlite::Result<InboxItem> {
    let images: String = r.get(4)?;
    Ok(InboxItem {
        id: r.get(0)?,
        source: r.get(1)?,
        mode: r.get(2)?,
        content: r.get(3)?,
        image_paths: serde_json::from_str(&images).unwrap_or_default(),
        received_at: r.get(5)?,
        business_date: r.get(6)?,
        status: Status::parse(&r.get::<_, String>(7)?),
        status_reason: r.get(8)?,
        attempts: r.get(9)?,
        last_error: r.get(10)?,
        result_summary: r.get(11)?,
        vision_text: r.get(12)?,
        target_project_id: r.get(13)?,
    })
}

const COLS: &str = "id, source, mode, content, image_paths, received_at, business_date, status, status_reason, \
                    attempts, last_error, result_summary, vision_text, target_project_id";

/// 已完成输入的有限上下文；不包含内部编号或图片文件路径。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationContextItem {
    pub content: String,
    pub vision_text: Option<String>,
    pub result_summary: String,
    pub mode: String,
    pub source: String,
    pub received_at: String,
}

/// 待确认 / 待追问的计划
#[derive(Debug, Clone, PartialEq)]
pub struct Pending {
    pub request_id: String,
    pub kind: String,
    pub plan: String,
    pub versions: String,
    pub context: Option<String>,
    pub preview: Option<String>,
}

pub struct NewPending<'a> {
    pub request_id: &'a str,
    pub kind: &'a str,
    pub plan: &'a str,
    pub versions: &'a str,
    pub context: Option<&'a str>,
    pub preview: Option<&'a str>,
    pub expires_at: &'a str,
}

impl Service<'_> {
    /// 待追问计划的已有对象快照；版本相同但 ID 被复用时，完整行仍能识别变化。
    pub(crate) fn plan_snapshots(
        &self,
        pins: &[super::changes::Pin],
    ) -> AppResult<Vec<serde_json::Value>> {
        pins.iter()
            .map(|pin| {
                let entity = match pin.entity.as_str() {
                    "tasks" => "tasks",
                    "nodes" => "nodes",
                    "recur_rules" => "recur_rules",
                    _ => return Err(AppError::Invalid("未知的计划引用对象".into())),
                };
                let row = super::oplog::row_json(self.conn, entity, pin.id)?
                    .ok_or_else(|| AppError::Conflict("相关内容已经变化，请重新输入".into()))?;
                Ok(serde_json::json!({"entity":entity,"id":pin.id,"row":row}))
            })
            .collect()
    }

    /// 登记一条输入，记下当时的时间和业务日期（之后处理时按这个理解"今天 / 明天"）
    pub fn inbox_register(
        &self,
        id: &str,
        source: &str,
        mode: &str,
        content: &str,
        images: &[String],
    ) -> AppResult<()> {
        self.inbox_register_with_project(id, source, mode, content, images, None)
    }

    /// 默认项目与输入在同一条 INSERT 中登记，绑定失败不能留下无归属的可执行请求。
    pub fn inbox_register_with_project(
        &self,
        id: &str,
        source: &str,
        mode: &str,
        content: &str,
        images: &[String],
        target_project: Option<i64>,
    ) -> AppResult<()> {
        if let Some(project) = target_project {
            let exists: bool = self.conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM nodes WHERE id=?1 AND depth=1)",
                [project],
                |r| r.get(0),
            )?;
            if source != "sticky" || mode != "project" || !exists {
                return Err(AppError::Invalid(
                    "便利贴绑定的项目已经不存在或来源无效".into(),
                ));
            }
        }
        let now = self.now_str();
        let today = self.today().format(DATE_FMT).to_string();
        self.conn.execute(
            "INSERT INTO inbox (id, source, mode, content, image_paths, received_at, business_date, status, updated_at,target_project_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'queued', ?6,?8)",
            params![id, source, mode, content, serde_json::to_string(images)?, now, today,target_project],
        )?;
        Ok(())
    }

    pub fn inbox_get(&self, id: &str) -> AppResult<InboxItem> {
        self.conn
            .query_row(
                &format!("SELECT {COLS} FROM inbox WHERE id = ?1"),
                [id],
                read,
            )
            .optional()?
            .ok_or_else(|| AppError::Invalid("这条记录已经不存在了".into()))
    }

    /// 绑定来源由宿主窗口确认的默认项目，不改写用户原文。
    pub fn inbox_bind_project(&self, id: &str, project: i64) -> AppResult<()> {
        let exists: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM nodes WHERE id=?1 AND depth=1)",
            [project],
            |r| r.get(0),
        )?;
        if !exists {
            return Err(AppError::Invalid("便利贴绑定的项目已经不存在了".into()));
        }
        self.conn.execute(
            "UPDATE inbox SET target_project_id=?2 WHERE id=?1 AND status='queued' AND source='sticky'",
            params![id, project],
        )?;
        Ok(())
    }

    /// 所有入口和模式共享最近六轮；只读取本次输入之前、24 小时内成功结束的请求。
    pub fn conversation_context(&self, id: &str) -> AppResult<Vec<ConversationContextItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT content,vision_text,result_summary,mode,source,received_at FROM inbox
             WHERE status='done' AND status_reason IS NULL AND result_summary IS NOT NULL
             AND rowid < (SELECT rowid FROM inbox WHERE id=?1)
             AND received_at >= strftime('%Y-%m-%dT%H:%M:%S',(SELECT received_at FROM inbox WHERE id=?1),'-24 hours')
             AND received_at <= (SELECT received_at FROM inbox WHERE id=?1)
             ORDER BY received_at DESC,rowid DESC LIMIT 6",
        )?;
        Ok(stmt
            .query_map([id], |r| {
                Ok(ConversationContextItem {
                    content: r.get(0)?,
                    vision_text: r.get(1)?,
                    result_summary: r.get(2)?,
                    mode: r.get(3)?,
                    source: r.get(4)?,
                    received_at: r.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?)
    }

    /// 新到旧的输入历史，最多读取五十条；保留 Markdown 原文和未发送草稿的职责在前端。
    pub fn input_history(&self, mode: &str, source: &str) -> AppResult<Vec<String>> {
        if !matches!(mode, "task" | "project") || !matches!(source, "main" | "quick" | "sticky") {
            return Err(AppError::Invalid("输入历史的来源或模式无效".into()));
        }
        let mut stmt = self.conn.prepare(
            "SELECT content FROM inbox WHERE mode=?1 AND source=?2 AND trim(content)!=''
             ORDER BY received_at DESC,rowid DESC LIMIT 50",
        )?;
        let mut items = stmt
            .query_map(params![mode, source], |r| r.get(0))?
            .collect::<Result<Vec<String>, _>>()?;
        items.retain(|text| !text.trim().is_empty());
        items.dedup();
        Ok(items)
    }

    pub fn inbox_exists(&self, id: &str) -> AppResult<bool> {
        Ok(self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM inbox WHERE id = ?1)",
            [id],
            |r| r.get(0),
        )?)
    }

    /// 在同一个数据库锁内领取请求，防止手动和后台重试重复调用模型。
    pub fn inbox_claim(&self, id: &str) -> AppResult<bool> {
        Ok(self.conn.execute(
            "UPDATE inbox SET status = 'processing', status_reason = NULL, last_error = NULL, updated_at = ?2
             WHERE id = ?1 AND status IN ('queued', 'failed')",
            params![id, self.now_str()],
        )? == 1)
    }

    /// 仅在启动或恢复数据库后调用：上次中断的处理重新进入队列。
    pub fn inbox_recover(&self) -> AppResult<()> {
        self.conn.execute(
            "UPDATE inbox SET status = 'done', result_summary = COALESCE(result_summary, '已完成上次请求'),
             updated_at = ?1 WHERE status != 'done' AND EXISTS(SELECT 1 FROM op_log WHERE group_id = inbox.id)",
            [self.now_str()],
        )?;
        self.conn.execute(
            "UPDATE inbox SET status = 'queued', status_reason = 'offline', next_attempt_at = ?1,
             last_error = '上次处理被中断，正在恢复', updated_at = ?1 WHERE status = 'processing'",
            [self.now_str()],
        )?;
        self.conn.execute(
            "UPDATE inbox SET status_reason = 'offline', next_attempt_at = ?1
             WHERE status = 'queued' AND status_reason IS NULL",
            [self.now_str()],
        )?;
        Ok(())
    }

    /// 图片只保留七天；已完成记录只保留三十天。文件删除由代理限定目录处理。
    pub fn inbox_cleanup(&self) -> AppResult<Vec<String>> {
        let cutoff = (self.clock.now() - chrono::Duration::days(7))
            .format(super::DATETIME_FMT)
            .to_string();
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLS} FROM inbox WHERE image_paths != '[]' AND status != 'processing' AND (status = 'done' OR received_at <= ?1)"
        ))?;
        let items = stmt
            .query_map([cutoff], read)?
            .collect::<Result<Vec<_>, _>>()?;
        let mut paths = Vec::new();
        for item in items {
            paths.extend(item.image_paths);
            if item.status != Status::Done {
                self.pending_delete(&item.id)?;
                self.inbox_set(
                    &item.id,
                    Status::Failed,
                    Some("images_expired"),
                    Some("暂存图片已超过 7 天并清理，请重新上传图片"),
                    None,
                )?;
            }
        }
        let retention = (self.clock.now() - chrono::Duration::days(30))
            .format(super::DATETIME_FMT)
            .to_string();
        self.conn.execute(
            "DELETE FROM inbox WHERE status = 'done' AND updated_at <= ?1 AND image_paths = '[]'",
            [retention],
        )?;
        Ok(paths)
    }

    pub fn inbox_image_removed(&self, path: &str) -> AppResult<()> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLS} FROM inbox WHERE image_paths != '[]'"
        ))?;
        let items = stmt.query_map([], read)?.collect::<Result<Vec<_>, _>>()?;
        for mut item in items {
            if item.image_paths.iter().any(|p| p == path) {
                item.image_paths.retain(|p| p != path);
                self.conn.execute(
                    "UPDATE inbox SET image_paths = ?2 WHERE id = ?1",
                    params![item.id, serde_json::to_string(&item.image_paths)?],
                )?;
            }
        }
        Ok(())
    }

    pub fn inbox_set(
        &self,
        id: &str,
        status: Status,
        reason: Option<&str>,
        error: Option<&str>,
        result: Option<&str>,
    ) -> AppResult<()> {
        self.conn.execute(
            "UPDATE inbox SET status = ?2, status_reason = ?3, last_error = ?4,
             result_summary = COALESCE(?5, result_summary), updated_at = ?6 WHERE id = ?1",
            params![id, status.as_str(), reason, error, result, self.now_str()],
        )?;
        Ok(())
    }

    /// 网络类失败：记一次尝试，安排下次重试时间；超过次数标为失败。返回是否还会重试
    pub fn inbox_retry_later(
        &self,
        id: &str,
        error: &str,
        max_attempts: i64,
        delays_secs: &[i64],
    ) -> AppResult<bool> {
        let attempts: i64 =
            self.conn
                .query_row("SELECT attempts FROM inbox WHERE id = ?1", [id], |r| {
                    r.get(0)
                })?;
        let attempts = attempts + 1;
        if attempts >= max_attempts {
            self.conn.execute(
                "UPDATE inbox SET status = 'failed', status_reason = ?2, attempts = ?3, last_error = ?4,
                 next_attempt_at = NULL, updated_at = ?5 WHERE id = ?1",
                params![id, REASON_OFFLINE, attempts, error, self.now_str()],
            )?;
            return Ok(false);
        }
        let delay = delays_secs
            .get((attempts - 1) as usize)
            .or(delays_secs.last())
            .copied()
            .unwrap_or(60);
        let next = (self.clock.now() + chrono::Duration::seconds(delay))
            .format(super::DATETIME_FMT)
            .to_string();
        self.conn.execute(
            "UPDATE inbox SET status = 'queued', status_reason = ?2, attempts = ?3, last_error = ?4,
             next_attempt_at = ?5, updated_at = ?6 WHERE id = ?1",
            params![id, REASON_OFFLINE, attempts, error, next, self.now_str()],
        )?;
        Ok(true)
    }

    pub fn inbox_set_vision(&self, id: &str, text: &str) -> AppResult<()> {
        self.conn.execute(
            "UPDATE inbox SET vision_text = ?2 WHERE id = ?1",
            params![id, text],
        )?;
        Ok(())
    }

    /// 还没处理完的输入（界面上的"收件箱 n 条"）
    pub fn inbox_open(&self) -> AppResult<Vec<InboxItem>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLS} FROM inbox WHERE status IN ('queued', 'processing', 'awaiting', 'failed')
             ORDER BY received_at, id"
        ))?;
        let items = stmt.query_map([], read)?.collect::<Result<Vec<_>, _>>()?;
        Ok(items)
    }

    /// 到了重试时间的离线暂存
    #[cfg(test)]
    pub fn inbox_due(&self) -> AppResult<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT id FROM inbox WHERE status = 'queued' AND status_reason = ?1
             AND next_attempt_at IS NOT NULL AND next_attempt_at <= ?2 ORDER BY received_at",
        )?;
        let ids = stmt
            .query_map(params![REASON_OFFLINE, self.now_str()], |r| r.get(0))?
            .collect::<Result<Vec<String>, _>>()?;
        Ok(ids)
    }

    pub fn inbox_ready(&self) -> AppResult<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT id FROM inbox WHERE status = 'queued' AND ((status_reason IS NULL AND source IN ('sticky','quick')) OR status_reason = 'unconfigured'
             OR (status_reason = 'offline' AND next_attempt_at <= ?1)) ORDER BY received_at, id",
        )?;
        Ok(stmt
            .query_map([self.now_str()], |r| r.get(0))?
            .collect::<Result<Vec<_>, _>>()?)
    }

    /// 因未配置模型而暂存的输入（配置好模型后处理）
    #[cfg(test)]
    pub fn inbox_unconfigured(&self) -> AppResult<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT id FROM inbox WHERE status = 'queued' AND status_reason = ?1 ORDER BY received_at",
        )?;
        let ids = stmt
            .query_map([REASON_UNCONFIGURED], |r| r.get(0))?
            .collect::<Result<Vec<String>, _>>()?;
        Ok(ids)
    }

    pub fn pending_save(&self, p: NewPending<'_>) -> AppResult<()> {
        self.conn.execute(
            "DELETE FROM pending_plans WHERE request_id = ?1",
            [p.request_id],
        )?;
        self.conn.execute(
            "INSERT INTO pending_plans (request_id, kind, plan, versions, context, preview, created_at, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![p.request_id, p.kind, p.plan, p.versions, p.context, p.preview, self.now_str(), p.expires_at],
        )?;
        Ok(())
    }

    pub fn pending_get(&self, request_id: &str) -> AppResult<Option<Pending>> {
        Ok(self
            .conn
            .query_row(
                "SELECT request_id, kind, plan, versions, context, preview FROM pending_plans WHERE request_id = ?1",
                [request_id],
                |r| {
                    Ok(Pending {
                        request_id: r.get(0)?,
                        kind: r.get(1)?,
                        plan: r.get(2)?,
                        versions: r.get(3)?,
                        context: r.get(4)?,
                        preview: r.get(5)?,
                    })
                },
            )
            .optional()?)
    }

    /// 某个来源窗口当前未过期的待确认 / 待追问（只看最近一条）
    pub fn pending_active(&self, source: &str) -> AppResult<Option<Pending>> {
        let id: Option<String> = self
            .conn
            .query_row(
                "SELECT p.request_id FROM pending_plans p JOIN inbox i ON i.id = p.request_id
                 WHERE i.source = ?1 AND i.status = 'awaiting' AND p.expires_at > ?2
                 ORDER BY p.created_at DESC LIMIT 1",
                params![source, self.now_str()],
                |r| r.get(0),
            )
            .optional()?;
        match id {
            Some(id) => self.pending_get(&id),
            None => Ok(None),
        }
    }

    pub fn pending_delete(&self, request_id: &str) -> AppResult<()> {
        self.conn.execute(
            "DELETE FROM pending_plans WHERE request_id = ?1",
            [request_id],
        )?;
        Ok(())
    }

    /// 过期的待确认：主窗口里的过期后视为取消；其余来源保留在收件箱里等用户处理
    pub fn pending_expire(&self) -> AppResult<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT p.request_id FROM pending_plans p JOIN inbox i ON i.id = p.request_id
             WHERE i.source = 'main' AND p.expires_at <= ?1",
        )?;
        let ids = stmt
            .query_map([self.now_str()], |r| r.get(0))?
            .collect::<Result<Vec<String>, _>>()?;
        for id in &ids {
            self.pending_delete(id)?;
            self.inbox_set(
                id,
                Status::Done,
                Some("expired"),
                None,
                Some("已过期，未执行"),
            )?;
        }
        Ok(ids)
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::*;
    use super::*;
    use crate::config::AppConfig;

    #[test]
    fn 上下文共享跨入口模式且限定六轮一天_历史保留原文() {
        let conn = db();
        let clock = clock("2026-10-20 10:00:00");
        let svc = Service::new(&conn, &clock, &AppConfig::default()).unwrap();
        svc.inbox_register("old", "main", "task", "过早记录", &[])
            .unwrap();
        svc.inbox_set("old", Status::Done, None, None, Some("旧结果"))
            .unwrap();
        conn.execute(
            "UPDATE inbox SET received_at='2026-10-19T09:59:59' WHERE id='old'",
            [],
        )
        .unwrap();
        for n in 0..8 {
            let source = ["main", "quick", "sticky"][n % 3];
            let mode = if n % 2 == 0 { "task" } else { "project" };
            let id = format!("r{n}");
            svc.inbox_register(&id, source, mode, &format!("原文{n}"), &[])
                .unwrap();
            svc.inbox_set(&id, Status::Done, None, None, Some(&format!("结果{n}")))
                .unwrap();
        }
        svc.inbox_set_vision("r7", "识图中的补充内容").unwrap();
        let raw = "# 项目\n- **交周报**\n  详细说明";
        svc.inbox_register("current", "main", "project", raw, &[])
            .unwrap();
        let context = svc.conversation_context("current").unwrap();
        assert_eq!(context.len(), 6);
        assert_eq!(context[0].content, "原文7");
        assert_eq!(context[0].vision_text.as_deref(), Some("识图中的补充内容"));
        assert_eq!(context[5].content, "原文2");
        assert!(
            context
                .iter()
                .any(|item| item.source == "quick" && item.mode == "task")
        );
        assert_eq!(svc.inbox_get("current").unwrap().content, raw);
        svc.inbox_register("duplicate", "main", "project", raw, &[])
            .unwrap();
        assert_eq!(
            svc.input_history("project", "main")
                .unwrap()
                .iter()
                .filter(|t| t.as_str() == raw)
                .count(),
            1
        );
        assert!(svc.input_history("task", "unknown").is_err());
    }

    #[test]
    fn 无效项目不能留下未绑定的排队输入() {
        let conn = db();
        let clock = clock("2026-10-20 10:00:00");
        let svc = Service::new(&conn, &clock, &AppConfig::default()).unwrap();
        assert!(
            svc.inbox_register_with_project(
                "invalid",
                "sticky",
                "project",
                "添加工作",
                &[],
                Some(404)
            )
            .is_err()
        );
        assert!(!svc.inbox_exists("invalid").unwrap());
    }

    #[test]
    fn 登记_重试_失败() {
        let conn = db();
        let clock = clock("2026-10-17 23:30:00");
        let svc = Service::new(&conn, &clock, &AppConfig::default()).unwrap();
        svc.inbox_register("r1", "main", "task", "明天交周报", &[])
            .unwrap();
        let it = svc.inbox_get("r1").unwrap();
        assert_eq!(it.business_date, "2026-10-17");
        assert_eq!(it.status, Status::Queued);

        assert!(svc.inbox_retry_later("r1", "断网", 3, &[30, 60]).unwrap());
        assert!(svc.inbox_due().unwrap().is_empty()); // 还没到时间
        assert!(svc.inbox_retry_later("r1", "断网", 3, &[30, 60]).unwrap());
        assert!(!svc.inbox_retry_later("r1", "断网", 3, &[30, 60]).unwrap());
        assert_eq!(svc.inbox_get("r1").unwrap().status, Status::Failed);
        assert_eq!(svc.inbox_open().unwrap().len(), 1);
    }

    #[test]
    fn 到时间的重试与未配置的暂存() {
        let conn = db();
        let c1 = clock("2026-10-17 10:00:00");
        let svc = Service::new(&conn, &c1, &AppConfig::default()).unwrap();
        svc.inbox_register("a", "quick", "task", "x", &[]).unwrap();
        svc.inbox_register("b", "main", "task", "y", &[]).unwrap();
        svc.inbox_retry_later("a", "断网", 5, &[30]).unwrap();
        svc.inbox_set("b", Status::Queued, Some(REASON_UNCONFIGURED), None, None)
            .unwrap();
        assert_eq!(svc.inbox_unconfigured().unwrap(), ["b"]);

        let c2 = clock("2026-10-17 10:01:00");
        let later = Service::new(&conn, &c2, &AppConfig::default()).unwrap();
        assert_eq!(later.inbox_due().unwrap(), ["a"]);
    }

    #[test]
    fn 待确认按来源取最近一条_过期后取消() {
        let conn = db();
        let c1 = clock("2026-10-17 10:00:00");
        let svc = Service::new(&conn, &c1, &AppConfig::default()).unwrap();
        svc.inbox_register("r1", "main", "task", "删掉开会", &[])
            .unwrap();
        svc.inbox_set("r1", Status::Awaiting, None, None, None)
            .unwrap();
        svc.pending_save(NewPending {
            request_id: "r1",
            kind: "confirm",
            plan: "[]",
            versions: "[]",
            context: None,
            preview: None,
            expires_at: "2026-10-17T10:10:00",
        })
        .unwrap();
        assert_eq!(
            svc.pending_active("main").unwrap().unwrap().request_id,
            "r1"
        );
        assert!(svc.pending_active("quick").unwrap().is_none());

        let c2 = clock("2026-10-17 10:11:00");
        let later = Service::new(&conn, &c2, &AppConfig::default()).unwrap();
        assert!(later.pending_active("main").unwrap().is_none());
        assert_eq!(later.pending_expire().unwrap(), ["r1"]);
        assert_eq!(later.inbox_get("r1").unwrap().status, Status::Done);
    }
}
