//! Claude Messages API（POST /v1/messages，流式 SSE）。
//!
//! - 默认模型 claude-opus-5-5；思考由模型自适应（不传 thinking 参数）；
//! - 助手消息的内容块（含思考块及其签名）原样保存，下一轮原样回传；
//! - 工具开启 eager_input_streaming，参数 JSON 由我们自己校验；
//! - stop_reason 为 max_tokens / refusal 时不执行本轮工具；
//! - 官方地址上的新模型默认开启服务端回退（拒绝时由服务端换模型重试）。

use crate::usage::TokenUsage;
use serde_json::{Map, Value, json};

use super::sse::for_each_event;
use super::{ChatRequest, ChatResponse, Endpoint, LlmError, Message, Part, StopReason, ToolCall};

const API_VERSION: &str = "2023-06-01";
const MAX_TOKENS: u32 = 16_000;
/// 支持 fallbacks: "default" 的模型
const FALLBACK_MODELS: &[&str] = &[
    "claude-fable-5-1",
    "claude-opus-5-5",
    "claude-opus-5",
    "claude-sonnet-5-5",
];
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";

pub struct ClaudeClient {
    http: reqwest::Client,
    url: String,
    model: String,
    api_key: String,
    fallback: bool,
}

impl ClaudeClient {
    pub fn new(http: reqwest::Client, ep: &Endpoint, api_key: String) -> Self {
        let base = ep.base_url.trim_end_matches('/');
        let model = ep.model.trim().to_string();
        Self {
            http,
            url: format!("{base}/messages"),
            fallback: base.starts_with("https://api.anthropic.com")
                && FALLBACK_MODELS.contains(&model.as_str()),
            model,
            api_key,
        }
    }

    pub async fn chat(
        &self,
        req: &ChatRequest<'_>,
        on_text: &(dyn Fn(&str) + Send + Sync),
        on_usage: &(dyn Fn(TokenUsage) + Send + Sync),
    ) -> Result<ChatResponse, LlmError> {
        let mut body = build_body(&self.model, req);
        let mut rb = self
            .http
            .post(&self.url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", API_VERSION);
        if self.fallback {
            body["fallbacks"] = json!("default");
            rb = rb.header("anthropic-beta", FALLBACK_BETA);
        }
        let resp = rb.json(&body).send().await?;
        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            let text = resp.text().await.unwrap_or_default();
            return Err(LlmError::from_status(status, &text));
        }
        let mut acc = Accumulator::default();
        for_each_event(resp, |data| {
            let result = acc.push(data, on_text);
            if let Some(usage) = acc.usage {
                on_usage(usage);
            }
            result
        })
        .await?;
        acc.finish()
    }
}

pub(super) fn build_body(model: &str, req: &ChatRequest<'_>) -> Value {
    // 连续的工具结果要合并到同一条 user 消息里
    let mut messages: Vec<Value> = Vec::new();
    let mut images_used = false;
    for m in req.messages {
        match m {
            Message::User { text } => {
                let mut content: Vec<Value> = Vec::new();
                if !images_used && !req.images.is_empty() {
                    images_used = true;
                    for p in req.images {
                        if let Part::Image { media_type, data } = p {
                            content.push(json!({
                                "type": "image",
                                "source": { "type": "base64", "media_type": media_type, "data": data }
                            }));
                        }
                    }
                }
                content.push(json!({ "type": "text", "text": text }));
                messages.push(json!({ "role": "user", "content": content }));
            }
            Message::Assistant { raw } => messages.push(raw.clone()),
            Message::ToolResult {
                call_id,
                content,
                is_error,
            } => {
                let block = json!({
                    "type": "tool_result", "tool_use_id": call_id, "content": content, "is_error": is_error
                });
                let last_is_results = messages.last().is_some_and(|l| {
                    l["role"] == "user" && l["content"][0]["type"] == "tool_result"
                });
                if last_is_results {
                    if let Some(arr) = messages
                        .last_mut()
                        .and_then(|l| l["content"].as_array_mut())
                    {
                        arr.push(block);
                    }
                } else {
                    messages.push(json!({ "role": "user", "content": [block] }));
                }
            }
        }
    }
    let mut body = json!({
        "model": model,
        "max_tokens": MAX_TOKENS,
        "stream": true,
        "system": req.system,
        "messages": messages,
    });
    if !req.tools.is_empty() {
        body["tools"] = req
            .tools
            .iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.parameters,
                    "eager_input_streaming": true,
                })
            })
            .collect();
    }
    body
}

/// 流式内容块：起始块 + 增量
#[derive(Default)]
pub(super) struct Accumulator {
    blocks: Vec<Value>,
    /// tool_use 参数的 JSON 片段，按块下标累积
    partial: Vec<String>,
    text: String,
    stop_reason: Option<String>,
    usage: Option<TokenUsage>,
    input_tokens: Option<u64>,
    cache_read_tokens: u64,
    cache_write_tokens: u64,
}

impl Accumulator {
    pub(super) fn push(
        &mut self,
        data: &str,
        on_text: &(dyn Fn(&str) + Send + Sync),
    ) -> Result<(), LlmError> {
        let v: Value =
            serde_json::from_str(data).map_err(|e| LlmError::BadResponse(e.to_string()))?;
        let kind = v["type"].as_str().unwrap_or_default();
        if matches!(kind, "message_start" | "message_delta") {
            let usage = if kind == "message_start" {
                v.pointer("/message/usage")
            } else {
                v.get("usage")
            };
            if let Some(value) = usage.filter(|u| u.is_object()) {
                if let Some(n) = value.get("input_tokens").and_then(Value::as_u64) {
                    self.input_tokens = Some(n);
                }
                if let Some(n) = value.get("cache_read_input_tokens").and_then(Value::as_u64) {
                    self.cache_read_tokens = n;
                }
                if let Some(n) = value
                    .get("cache_creation_input_tokens")
                    .and_then(Value::as_u64)
                {
                    self.cache_write_tokens = n;
                }
                let input = self.input_tokens.and_then(|n| {
                    n.checked_add(self.cache_read_tokens)?
                        .checked_add(self.cache_write_tokens)
                });
                // message_start 的输出计数只是初始快照，缺少后续用量时不能冒充最终结果。
                let output = if kind == "message_start" {
                    None
                } else {
                    value
                        .get("output_tokens")
                        .and_then(Value::as_u64)
                        .or_else(|| self.usage.and_then(|u| u.output_tokens))
                };
                self.usage = Some(TokenUsage {
                    input_tokens: input,
                    output_tokens: output,
                    total_tokens: input.and_then(|n| n.checked_add(output?)),
                });
            }
        }
        match v["type"].as_str().unwrap_or_default() {
            "content_block_start" => {
                let idx = v["index"].as_u64().unwrap_or(0) as usize;
                while self.blocks.len() <= idx {
                    self.blocks.push(Value::Null);
                    self.partial.push(String::new());
                }
                self.blocks[idx] = v["content_block"].clone();
            }
            "content_block_delta" => {
                let idx = v["index"].as_u64().unwrap_or(0) as usize;
                let Some(block) = self.blocks.get_mut(idx) else {
                    return Ok(());
                };
                let d = &v["delta"];
                match d["type"].as_str().unwrap_or_default() {
                    "text_delta" => {
                        let t = d["text"].as_str().unwrap_or_default();
                        append(block, "text", t);
                        self.text.push_str(t);
                        on_text(t);
                    }
                    "input_json_delta" => {
                        self.partial[idx].push_str(d["partial_json"].as_str().unwrap_or_default());
                    }
                    "thinking_delta" => append(
                        block,
                        "thinking",
                        d["thinking"].as_str().unwrap_or_default(),
                    ),
                    "signature_delta" => append(
                        block,
                        "signature",
                        d["signature"].as_str().unwrap_or_default(),
                    ),
                    _ => {}
                }
            }
            "message_delta" => {
                if let Some(r) = v.pointer("/delta/stop_reason").and_then(|r| r.as_str()) {
                    self.stop_reason = Some(r.to_string());
                }
            }
            "error" => {
                let msg = v
                    .pointer("/error/message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("未知错误");
                let kind = v
                    .pointer("/error/type")
                    .and_then(|m| m.as_str())
                    .unwrap_or_default();
                return Err(if kind == "overloaded_error" || kind == "api_error" {
                    LlmError::Unavailable { status: 529 }
                } else {
                    LlmError::Rejected {
                        status: 200,
                        message: msg.chars().take(200).collect(),
                    }
                });
            }
            _ => {}
        }
        Ok(())
    }

    pub(super) fn finish(mut self) -> Result<ChatResponse, LlmError> {
        if self.stop_reason.is_none() {
            return Err(LlmError::Network("回复中断，未收到完整结果".into()));
        }
        let mut tool_calls = Vec::new();
        for (i, block) in self.blocks.iter_mut().enumerate() {
            if block["type"] != "tool_use" {
                continue;
            }
            let partial = self.partial[i].trim();
            let args = if partial.is_empty() {
                Some(Value::Object(Map::new()))
            } else {
                serde_json::from_str::<Value>(partial)
                    .ok()
                    .filter(|v| v.is_object())
            };
            // 回传时 input 必须是对象
            block["input"] = args.clone().unwrap_or_else(|| Value::Object(Map::new()));
            tool_calls.push(ToolCall {
                id: block["id"].as_str().unwrap_or_default().to_string(),
                name: block["name"].as_str().unwrap_or_default().to_string(),
                arguments: args,
            });
        }
        let stop = match self.stop_reason.as_deref() {
            Some("max_tokens") => StopReason::MaxTokens,
            Some("refusal") => StopReason::Refusal,
            Some("tool_use") => StopReason::ToolUse,
            _ if !tool_calls.is_empty() => StopReason::ToolUse,
            _ => StopReason::Done,
        };
        let blocks: Vec<Value> = self.blocks.into_iter().filter(|b| !b.is_null()).collect();
        Ok(ChatResponse {
            text: self.text,
            tool_calls,
            stop,
            raw: json!({ "role": "assistant", "content": blocks }),
        })
    }
}

fn append(block: &mut Value, key: &str, s: &str) {
    let cur = block[key].as_str().unwrap_or_default().to_string();
    block[key] = Value::String(cur + s);
}

#[cfg(test)]
mod tests {
    use super::super::ToolSpec;
    use super::*;

    fn noop(_: &str) {}

    #[test]
    fn 用量累计快照与缓存不重复计数() {
        let mut acc = Accumulator::default();
        acc.push(r#"{"type":"message_start","message":{"usage":{"input_tokens":10,"cache_read_input_tokens":20,"cache_creation_input_tokens":5,"output_tokens":1}}}"#, &noop).unwrap();
        assert!(!acc.usage.unwrap().complete());
        acc.push(
            r#"{"type":"message_delta","usage":{"output_tokens":6}}"#,
            &noop,
        )
        .unwrap();
        acc.push(
            r#"{"type":"message_delta","usage":{"output_tokens":8}}"#,
            &noop,
        )
        .unwrap();
        assert_eq!(
            acc.usage,
            Some(TokenUsage {
                input_tokens: Some(35),
                output_tokens: Some(8),
                total_tokens: Some(43)
            })
        );
    }

    #[test]
    fn 累积思考_文字与工具调用并保留原文() {
        let mut acc = Accumulator::default();
        let events = [
            r#"{"type":"message_start","message":{"id":"m"}}"#,
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"想一想"}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"sig"}}"#,
            r#"{"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}"#,
            r#"{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"好"}}"#,
            r#"{"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"tu1","name":"task_add","input":{}}}"#,
            r#"{"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{\"title\":"}}"#,
            r#"{"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"\"买菜\"}"}}"#,
            r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"}}"#,
            r#"{"type":"message_stop"}"#,
        ];
        for e in events {
            acc.push(e, &noop).unwrap();
        }
        let r = acc.finish().unwrap();
        assert_eq!(r.text, "好");
        assert_eq!(r.stop, StopReason::ToolUse);
        assert_eq!(r.tool_calls[0].arguments, Some(json!({"title": "买菜"})));
        let content = &r.raw["content"];
        assert_eq!(content[0]["signature"], "sig");
        assert_eq!(content[0]["thinking"], "想一想");
        assert_eq!(content[2]["input"]["title"], "买菜");
    }

    #[test]
    fn 截断时不执行工具() {
        let mut acc = Accumulator::default();
        for e in [
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"t","name":"x","input":{}}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"a\":"}}"#,
            r#"{"type":"message_delta","delta":{"stop_reason":"max_tokens"}}"#,
        ] {
            acc.push(e, &noop).unwrap();
        }
        let r = acc.finish().unwrap();
        assert_eq!(r.stop, StopReason::MaxTokens);
        assert_eq!(r.tool_calls[0].arguments, None);
    }

    #[test]
    fn 工具结果合并到同一条用户消息() {
        let msgs = [
            Message::User { text: "hi".into() },
            Message::Assistant {
                raw: json!({"role":"assistant","content":[]}),
            },
            Message::ToolResult {
                call_id: "a".into(),
                content: "1".into(),
                is_error: false,
            },
            Message::ToolResult {
                call_id: "b".into(),
                content: "2".into(),
                is_error: true,
            },
        ];
        let tools = [ToolSpec {
            name: "t".into(),
            description: "d".into(),
            parameters: json!({"type":"object"}),
        }];
        let body = build_body(
            "claude-opus-5-5",
            &ChatRequest {
                system: "s",
                messages: &msgs,
                images: &[],
                tools: &tools,
            },
        );
        assert_eq!(body["system"], "s");
        assert_eq!(body["messages"].as_array().unwrap().len(), 3);
        assert_eq!(body["messages"][2]["content"][1]["tool_use_id"], "b");
        assert_eq!(body["messages"][2]["content"][1]["is_error"], true);
        assert_eq!(body["tools"][0]["input_schema"]["type"], "object");
        assert_eq!(body["tools"][0]["eager_input_streaming"], true);
    }

    #[test]
    fn 只有官方地址上的新模型开启回退() {
        let http = reqwest::Client::new();
        let ep = |url: &str, model: &str| Endpoint {
            provider: super::super::ProviderId::Claude,
            base_url: url.into(),
            model: model.into(),
        };
        assert!(
            ClaudeClient::new(
                http.clone(),
                &ep("https://api.anthropic.com/v1", "claude-opus-5-5"),
                "k".into()
            )
            .fallback
        );
        assert!(
            !ClaudeClient::new(
                http.clone(),
                &ep("https://api.anthropic.com/v1", "claude-haiku-5-5"),
                "k".into()
            )
            .fallback
        );
        assert!(
            !ClaudeClient::new(
                http,
                &ep("https://proxy.example/v1", "claude-opus-5-5"),
                "k".into()
            )
            .fallback
        );
    }
}
