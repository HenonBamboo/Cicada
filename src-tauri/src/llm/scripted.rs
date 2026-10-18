//! 测试用的脚本化模型：按顺序返回预先写好的回复，并记录收到的请求

use std::collections::VecDeque;
use std::sync::Mutex;

use serde_json::{Value, json};

use super::{ChatRequest, ChatResponse, LlmError, Message, StopReason, ToolCall};

pub enum Step {
    /// 调用一个或多个工具
    Tools(Vec<(&'static str, Value)>),
    /// 最终回复文字
    Say(&'static str),
    Fail(LlmError),
}

#[derive(Default)]
pub struct ScriptedModel {
    steps: Mutex<VecDeque<Step>>,
    /// 每次请求的系统提示词与消息，便于断言
    pub seen: Mutex<Vec<(String, Vec<Message>)>>,
}

impl ScriptedModel {
    pub fn new(steps: Vec<Step>) -> Self {
        Self {
            steps: Mutex::new(steps.into()),
            seen: Mutex::default(),
        }
    }

    pub fn chat(
        &self,
        req: &ChatRequest<'_>,
        on_text: &(dyn Fn(&str) + Send + Sync),
    ) -> Result<ChatResponse, LlmError> {
        self.seen
            .lock()
            .unwrap()
            .push((req.system.to_string(), req.messages.to_vec()));
        let step = self
            .steps
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Step::Say("（脚本已用完）"));
        match step {
            Step::Fail(e) => Err(e),
            Step::Say(t) => {
                on_text(t);
                Ok(ChatResponse {
                    text: t.into(),
                    tool_calls: vec![],
                    stop: StopReason::Done,
                    raw: json!({ "role": "assistant", "content": t }),
                })
            }
            Step::Tools(calls) => {
                let n = self.seen.lock().unwrap().len();
                let tool_calls: Vec<ToolCall> = calls
                    .into_iter()
                    .enumerate()
                    .map(|(i, (name, args))| ToolCall {
                        id: format!("c{n}_{i}"),
                        name: name.into(),
                        arguments: Some(args),
                    })
                    .collect();
                Ok(ChatResponse {
                    text: String::new(),
                    raw: json!({ "role": "assistant", "content": null, "tool_calls": tool_calls }),
                    tool_calls,
                    stop: StopReason::ToolUse,
                })
            }
        }
    }
}
