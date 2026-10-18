//! 回复区的一轮回复（docs/产品设计.md 3.4），与前端 src/lib/types.ts 的 AgentReply 对应

use serde::Serialize;

use crate::service::ChangeSet;
use crate::service::changes::PreviewItem;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ReplyKind {
    /// 已执行，可撤销
    Done,
    /// 查询结果、总结、拒绝等，没有改数据
    Info,
    /// 危险操作的预览，等待确认
    Confirm,
    /// 追问
    Clarify,
    /// 已暂存：未配置模型
    Unconfigured,
    /// 已暂存：网络问题，稍后自动重试
    Offline,
    Error,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentReply {
    pub request_id: String,
    pub kind: ReplyKind,
    pub status: Option<String>,
    pub lead: String,
    pub items: Vec<PreviewItem>,
    /// 追问的可选回答
    pub options: Vec<String>,
    pub changes: ChangeSet,
}

impl AgentReply {
    fn new(id: &str, kind: ReplyKind, status: Option<&str>, lead: String) -> Self {
        Self {
            request_id: id.into(),
            kind,
            status: status.map(str::to_string),
            lead,
            items: vec![],
            options: vec![],
            changes: ChangeSet::default(),
        }
    }

    pub fn done(id: &str, lead: String, items: Vec<PreviewItem>, changes: ChangeSet) -> Self {
        Self {
            items,
            changes,
            ..Self::new(id, ReplyKind::Done, None, lead)
        }
    }

    pub fn info(id: &str, lead: String) -> Self {
        Self::new(id, ReplyKind::Info, None, lead)
    }

    pub fn confirm(id: &str, items: Vec<PreviewItem>, changed: bool) -> Self {
        let lead = if changed {
            "相关内容刚刚有变化，按最新情况重新列了一遍，确认要这样做吗？"
        } else {
            "确认要这样做吗？"
        };
        Self {
            items,
            ..Self::new(id, ReplyKind::Confirm, Some("需要确认"), lead.into())
        }
    }

    pub fn clarify(id: &str, question: String, options: Vec<String>) -> Self {
        Self {
            options,
            ..Self::new(id, ReplyKind::Clarify, Some("需要你选一下"), question)
        }
    }

    pub fn queued_unconfigured(id: &str) -> Self {
        Self::new(
            id,
            ReplyKind::Unconfigured,
            Some("已暂存"),
            "还没有配置模型，这句话先存进收件箱，配置好后自动处理。".into(),
        )
    }

    pub fn queued_offline(id: &str, why: String) -> Self {
        Self::new(
            id,
            ReplyKind::Offline,
            Some("已暂存"),
            format!("暂时连不上模型（{why}），已存进收件箱，恢复后自动处理，再通知你。"),
        )
    }

    pub fn error(id: &str, msg: String) -> Self {
        Self::new(id, ReplyKind::Error, Some("没有完成"), msg)
    }
}
