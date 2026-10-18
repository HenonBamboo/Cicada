//! OpenAI 兼容接口（/chat/completions，流式）：DeepSeek、通义千问、Kimi、智谱及自定义服务

use serde_json::{Value, json};

use super::sse::for_each_event;
use super::{ChatRequest, ChatResponse, Endpoint, LlmError, Message, Part, StopReason, ToolCall};

pub struct OpenAiClient {
    http: reqwest::Client,
    url: String,
    model: String,
    api_key: String,
}

impl OpenAiClient {
    pub fn new(http: reqwest::Client, ep: &Endpoint, api_key: String) -> Self {
        Self {
            http,
            url: format!("{}/chat/completions", ep.base_url.trim_end_matches('/')),
            model: ep.model.trim().to_string(),
            api_key,
        }
    }

    pub async fn chat(
        &self,
        req: &ChatRequest<'_>,
        on_text: &(dyn Fn(&str) + Send + Sync),
    ) -> Result<ChatResponse, LlmError> {
        let body = build_body(&self.model, req);
        let resp = self
            .http
            .post(&self.url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await?;
        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            let text = resp.text().await.unwrap_or_default();
            return Err(LlmError::from_status(status, &text));
        }
        let mut acc = Accumulator::default();
        for_each_event(resp, |data| acc.push(data, on_text)).await?;
        acc.finish()
    }
}

pub(super) fn build_body(model: &str, req: &ChatRequest<'_>) -> Value {
    let mut messages = vec![json!({ "role": "system", "content": req.system })];
    let mut images_used = false;
    for m in req.messages {
        messages.push(match m {
            Message::User { text } if !images_used && !req.images.is_empty() => {
                images_used = true;
                let mut content: Vec<Value> = req
                    .images
                    .iter()
                    .filter_map(|p| match p {
                        Part::Image { media_type, data } => Some(json!({
                            "type": "image_url",
                            "image_url": { "url": format!("data:{media_type};base64,{data}") }
                        })),
                        Part::Text(_) => None,
                    })
                    .collect();
                content.push(json!({ "type": "text", "text": text }));
                json!({ "role": "user", "content": content })
            }
            Message::User { text } => json!({ "role": "user", "content": text }),
            Message::Assistant { raw } => raw.clone(),
            Message::ToolResult {
                call_id, content, ..
            } => json!({ "role": "tool", "tool_call_id": call_id, "content": content }),
        });
    }
    let mut body = json!({ "model": model, "stream": true, "messages": messages });
    if !req.tools.is_empty() {
        body["tools"] = req
            .tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": { "name": t.name, "description": t.description, "parameters": t.parameters }
                })
            })
            .collect();
    }
    body
}

#[derive(Default)]
struct PartialCall {
    id: String,
    name: String,
    arguments: String,
}

/// 把流式片段累积成完整回复
#[derive(Default)]
pub(super) struct Accumulator {
    text: String,
    calls: Vec<PartialCall>,
    finish: Option<String>,
}

impl Accumulator {
    pub(super) fn push(
        &mut self,
        data: &str,
        on_text: &(dyn Fn(&str) + Send + Sync),
    ) -> Result<(), LlmError> {
        if data.trim() == "[DONE]" {
            return Ok(());
        }
        let v: Value =
            serde_json::from_str(data).map_err(|e| LlmError::BadResponse(e.to_string()))?;
        if let Some(err) = v.get("error") {
            let msg = err
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("未知错误");
            return Err(LlmError::Rejected {
                status: 200,
                message: msg.chars().take(200).collect(),
            });
        }
        // 部分服务商最后会发一条只有用量、没有 choices 的片段
        let Some(choice) = v.pointer("/choices/0") else {
            return Ok(());
        };
        let delta = &choice["delta"];
        if let Some(t) = delta.get("content").and_then(|c| c.as_str())
            && !t.is_empty()
        {
            self.text.push_str(t);
            on_text(t);
        }
        if let Some(calls) = delta.get("tool_calls").and_then(|c| c.as_array()) {
            for c in calls {
                let idx = c.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
                while self.calls.len() <= idx {
                    self.calls.push(PartialCall::default());
                }
                let slot = &mut self.calls[idx];
                if let Some(id) = c.get("id").and_then(|s| s.as_str()) {
                    slot.id = id.to_string();
                }
                if let Some(n) = c.pointer("/function/name").and_then(|s| s.as_str()) {
                    slot.name.push_str(n);
                }
                if let Some(a) = c.pointer("/function/arguments").and_then(|s| s.as_str()) {
                    slot.arguments.push_str(a);
                }
            }
        }
        if let Some(f) = choice.get("finish_reason").and_then(|f| f.as_str()) {
            self.finish = Some(f.to_string());
        }
        Ok(())
    }

    pub(super) fn finish(self) -> Result<ChatResponse, LlmError> {
        if self.finish.is_none() {
            return Err(LlmError::Network("回复中断，未收到完整结果".into()));
        }
        let stop = match self.finish.as_deref() {
            Some("length") => StopReason::MaxTokens,
            Some("content_filter") => StopReason::Refusal,
            _ if !self.calls.is_empty() => StopReason::ToolUse,
            _ => StopReason::Done,
        };
        let tool_calls: Vec<ToolCall> = self
            .calls
            .iter()
            .enumerate()
            .map(|(i, c)| ToolCall {
                id: if c.id.is_empty() {
                    format!("call_{i}")
                } else {
                    c.id.clone()
                },
                name: c.name.clone(),
                arguments: parse_args(&c.arguments),
            })
            .collect();
        let mut raw = json!({ "role": "assistant", "content": self.text });
        if !tool_calls.is_empty() {
            // 只有工具调用时 content 用 null，部分服务商不接受空字符串
            if self.text.is_empty() {
                raw["content"] = Value::Null;
            }
            raw["tool_calls"] = tool_calls
                .iter()
                .zip(&self.calls)
                .map(|(t, c)| {
                    json!({
                        "id": t.id,
                        "type": "function",
                        "function": { "name": t.name, "arguments": if c.arguments.is_empty() { "{}" } else { &c.arguments } }
                    })
                })
                .collect();
        }
        Ok(ChatResponse {
            text: self.text,
            tool_calls,
            stop,
            raw,
        })
    }
}

/// 空参数视为 {}；不是 JSON 对象时返回 None，由调用方回报错误
fn parse_args(s: &str) -> Option<Value> {
    if s.trim().is_empty() {
        return Some(json!({}));
    }
    serde_json::from_str::<Value>(s)
        .ok()
        .filter(|v| v.is_object())
}

#[cfg(test)]
mod tests {
    use super::super::ToolSpec;
    use super::*;

    fn noop(_: &str) {}

    #[test]
    fn 累积文字与工具调用() {
        let mut acc = Accumulator::default();
        let chunks = [
            r#"{"choices":[{"delta":{"content":"好的"}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":"task_add","arguments":"{\"tit"}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"le\":\"买菜\"}"}}]}}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
            r#"{"choices":[],"usage":{"total_tokens":10}}"#,
            "[DONE]",
        ];
        for c in chunks {
            acc.push(c, &noop).unwrap();
        }
        let r = acc.finish().unwrap();
        assert_eq!(r.text, "好的");
        assert_eq!(r.stop, StopReason::ToolUse);
        assert_eq!(r.tool_calls[0].name, "task_add");
        assert_eq!(r.tool_calls[0].arguments, Some(json!({"title": "买菜"})));
        assert_eq!(
            r.raw["tool_calls"][0]["function"]["arguments"],
            "{\"title\":\"买菜\"}"
        );
    }

    #[test]
    fn 参数不是合法_json_时为空() {
        let mut acc = Accumulator::default();
        acc.push(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":"x","arguments":"{bad"}}]},"finish_reason":"tool_calls"}]}"#,
            &noop,
        )
        .unwrap();
        assert_eq!(acc.finish().unwrap().tool_calls[0].arguments, None);
    }

    #[test]
    fn 截断与错误() {
        let mut acc = Accumulator::default();
        acc.push(
            r#"{"choices":[{"delta":{"content":"x"},"finish_reason":"length"}]}"#,
            &noop,
        )
        .unwrap();
        assert_eq!(acc.finish().unwrap().stop, StopReason::MaxTokens);
        let mut acc = Accumulator::default();
        assert!(acc.push(r#"{"error":{"message":"quota"}}"#, &noop).is_err());
    }

    #[test]
    fn 请求体带系统提示词_图片与工具() {
        let msgs = [
            Message::User {
                text: "看图".into(),
            },
            Message::Assistant {
                raw: json!({"role":"assistant","content":"","tool_calls":[]}),
            },
            Message::ToolResult {
                call_id: "c1".into(),
                content: "ok".into(),
                is_error: false,
            },
        ];
        let images = [Part::Image {
            media_type: "image/png".into(),
            data: "AAA".into(),
        }];
        let tools = [ToolSpec {
            name: "t".into(),
            description: "d".into(),
            parameters: json!({"type":"object"}),
        }];
        let body = build_body(
            "m",
            &ChatRequest {
                system: "你是知了",
                messages: &msgs,
                images: &images,
                tools: &tools,
            },
        );
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][1]["content"][0]["type"], "image_url");
        assert_eq!(body["messages"][1]["content"][1]["text"], "看图");
        assert_eq!(body["messages"][3]["role"], "tool");
        assert_eq!(body["tools"][0]["function"]["name"], "t");
    }
}
