//! 模型服务商适配（docs/产品设计.md 4.1、10）。
//!
//! 只在 Rust 端调用模型；前端不发模型请求，也不接触 API Key。
//! 两种协议：OpenAI 兼容（DeepSeek、通义千问、Kimi、智谱、自定义）和 Claude Messages API。

mod claude;
mod openai;
#[cfg(test)]
pub mod scripted;
mod sse;

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use claude::ClaudeClient;
pub use openai::OpenAiClient;

/// 单次请求超时。流式请求只要持续有数据就不会触发
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(90);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderId {
    Deepseek,
    Qwen,
    Kimi,
    Zhipu,
    Claude,
    Custom,
}

impl ProviderId {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Deepseek => "deepseek",
            Self::Qwen => "qwen",
            Self::Kimi => "kimi",
            Self::Zhipu => "zhipu",
            Self::Claude => "claude",
            Self::Custom => "custom",
        }
    }

    pub fn default_base_url(self) -> &'static str {
        match self {
            Self::Deepseek => "https://api.deepseek.com/v1",
            Self::Qwen => "https://dashscope.aliyuncs.com/compatible-mode/v1",
            Self::Kimi => "https://api.moonshot.cn/v1",
            Self::Zhipu => "https://open.bigmodel.cn/api/paas/v4",
            Self::Claude => "https://api.anthropic.com/v1",
            Self::Custom => "",
        }
    }
}

/// 一个可调用的模型端点（Key 另外从凭据管理器读取）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Endpoint {
    pub provider: ProviderId,
    pub base_url: String,
    pub model: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Part {
    Text(String),
    /// base64 图片
    Image {
        media_type: String,
        data: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// 解析后的参数；模型给出的 JSON 无法解析时为 None
    pub arguments: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    /// JSON Schema
    pub parameters: Value,
}

/// 对话消息。助手消息保存服务商原样返回的内容（raw），下一轮原样回传，
/// 这样 Claude 的思考块等字段不会丢失
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Message {
    User {
        text: String,
    },
    Assistant {
        raw: Value,
    },
    ToolResult {
        call_id: String,
        content: String,
        is_error: bool,
    },
}

pub struct ChatRequest<'a> {
    pub system: &'a str,
    pub messages: &'a [Message],
    /// 只用于第一条用户消息之前附带的图片（识图）
    pub images: &'a [Part],
    pub tools: &'a [ToolSpec],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    Done,
    ToolUse,
    /// 输出被截断：本轮的工具调用不可执行
    MaxTokens,
    /// 模型拒绝
    Refusal,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChatResponse {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
    pub stop: StopReason,
    /// 助手消息原文，下一轮放回 messages
    pub raw: Value,
}

#[derive(Debug, Clone, thiserror::Error, PartialEq)]
pub enum LlmError {
    #[error("还没有配置模型")]
    NotConfigured,
    #[error("网络连接失败：{0}")]
    Network(String),
    #[error("模型服务暂时不可用（{status}）")]
    Unavailable { status: u16 },
    #[error("API Key 无效或没有权限（{status}）")]
    Auth { status: u16 },
    #[error("请求被拒绝（{status}）：{message}")]
    Rejected { status: u16, message: String },
    #[error("模型返回的内容无法解析：{0}")]
    BadResponse(String),
}

impl LlmError {
    /// 网络、超时、限流、服务端错误可以稍后重试；其余重试也没用
    pub fn retryable(&self) -> bool {
        matches!(self, Self::Network(_) | Self::Unavailable { .. })
    }

    fn from_status(status: u16, body: &str) -> Self {
        match status {
            401 | 403 => Self::Auth { status },
            408 | 409 | 429 | 500..=599 => Self::Unavailable { status },
            _ => Self::Rejected {
                status,
                message: error_message(body),
            },
        }
    }
}

impl From<reqwest::Error> for LlmError {
    fn from(e: reqwest::Error) -> Self {
        // reqwest 的错误信息不含请求头，不会带出 API Key
        Self::Network(e.without_url().to_string())
    }
}

/// 从错误响应里取一句人能看懂的话，避免把整段 JSON 显示给用户
fn error_message(body: &str) -> String {
    let v: Value = serde_json::from_str(body).unwrap_or_default();
    let msg = v
        .pointer("/error/message")
        .or_else(|| v.get("message"))
        .and_then(|m| m.as_str())
        .unwrap_or(body);
    msg.chars().take(200).collect()
}

fn http_client() -> Result<reqwest::Client, LlmError> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(LlmError::from)
}

/// 可调用的模型。用枚举分发，测试时换成脚本化的假模型
pub enum Model {
    OpenAi(OpenAiClient),
    Claude(ClaudeClient),
    #[cfg(test)]
    Scripted(scripted::ScriptedModel),
}

impl Model {
    pub fn new(endpoint: &Endpoint, api_key: String) -> Result<Self, LlmError> {
        if endpoint.base_url.trim().is_empty() || endpoint.model.trim().is_empty() {
            return Err(LlmError::NotConfigured);
        }
        let http = http_client()?;
        Ok(match endpoint.provider {
            ProviderId::Claude => Self::Claude(ClaudeClient::new(http, endpoint, api_key)),
            _ => Self::OpenAi(OpenAiClient::new(http, endpoint, api_key)),
        })
    }

    /// 发起一次对话。on_text 收到流式文字片段
    pub async fn chat(
        &self,
        req: &ChatRequest<'_>,
        on_text: &(dyn Fn(&str) + Send + Sync),
    ) -> Result<ChatResponse, LlmError> {
        match self {
            Self::OpenAi(c) => c.chat(req, on_text).await,
            Self::Claude(c) => c.chat(req, on_text).await,
            #[cfg(test)]
            Self::Scripted(m) => m.chat(req, on_text),
        }
    }
}

/// 把 data URL（data:image/png;base64,xxxx）拆成图片片段
pub fn image_from_data_url(url: &str) -> Option<Part> {
    let rest = url.strip_prefix("data:")?;
    let (meta, data) = rest.split_once(',')?;
    let media_type = meta.strip_suffix(";base64")?;
    if !matches!(
        media_type,
        "image/png" | "image/jpeg" | "image/webp" | "image/gif"
    ) || data.is_empty()
    {
        return None;
    }
    Some(Part::Image {
        media_type: media_type.to_string(),
        data: data.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 错误分类() {
        assert!(LlmError::from_status(503, "").retryable());
        assert!(LlmError::from_status(429, "").retryable());
        assert!(!LlmError::from_status(401, "").retryable());
        let e = LlmError::from_status(400, r#"{"error":{"message":"bad model"}}"#);
        assert_eq!(
            e,
            LlmError::Rejected {
                status: 400,
                message: "bad model".into()
            }
        );
    }

    #[test]
    fn 解析图片_data_url() {
        let p = image_from_data_url("data:image/png;base64,AAAA").unwrap();
        assert_eq!(
            p,
            Part::Image {
                media_type: "image/png".into(),
                data: "AAAA".into()
            }
        );
        assert!(image_from_data_url("data:text/plain;base64,AAAA").is_none());
        assert!(image_from_data_url("http://x").is_none());
    }

    #[test]
    fn 未填写端点时视为未配置() {
        let ep = Endpoint {
            provider: ProviderId::Custom,
            base_url: "".into(),
            model: "m".into(),
        };
        assert!(matches!(
            Model::new(&ep, "k".into()),
            Err(LlmError::NotConfigured)
        ));
    }
}
