//! 设置：模型配置（API Key 只写入凭据管理器、从不返回给前端）、连接测试、打开文件夹

use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::{AppHandle, State};

use super::poisoned;
use crate::config::ModelSettings;
use crate::error::{AppError, AppResult};
use crate::llm::{ChatRequest, Endpoint, LlmError, Message, Model, ToolSpec};
use crate::secrets::same_credential_scope;
use crate::state::AppState;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EndpointView {
    #[serde(flatten)]
    endpoint: Endpoint,
    /// 凭据管理器里是否已有这个服务商的 Key（不返回 Key 本身）
    has_key: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelSettingsView {
    chat: EndpointView,
    vision: Option<EndpointView>,
}

fn view(state: &AppState, m: &ModelSettings) -> ModelSettingsView {
    let v = |ep: &Endpoint| EndpointView {
        has_key: state
            .secrets
            .get(ep)
            .ok()
            .flatten()
            .is_some_and(|key| !key.trim().is_empty()),
        endpoint: ep.clone(),
    };
    ModelSettingsView {
        chat: v(&m.chat),
        vision: m.vision.as_ref().map(v),
    }
}

#[tauri::command]
pub fn get_model_settings(state: State<'_, AppState>) -> AppResult<ModelSettingsView> {
    let _maintenance = state
        .maintenance
        .try_read()
        .map_err(|_| AppError::Invalid("正在恢复数据，请稍后再试".into()))?;
    let m = state.config.lock().map_err(poisoned)?.model.clone();
    Ok(view(&state, &m))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveModel {
    chat: Endpoint,
    vision: Option<Endpoint>,
    /// 新填写的 Key；为空表示不修改
    chat_key: Option<String>,
    vision_key: Option<String>,
}

/// 保存模型设置。Key 只经过这一次，写入 Windows 凭据管理器后不再离开 Rust
#[tauri::command]
pub fn save_model_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    input: SaveModel,
) -> AppResult<ModelSettingsView> {
    let _maintenance = state
        .maintenance
        .try_read()
        .map_err(|_| AppError::Invalid("正在恢复数据，请稍后再试".into()))?;
    let trim = |ep: Endpoint| Endpoint {
        provider: ep.provider,
        base_url: ep.base_url.trim().trim_end_matches('/').to_string(),
        model: ep.model.trim().to_string(),
    };
    let model = ModelSettings {
        chat: trim(input.chat),
        vision: input.vision.map(trim),
    };
    model.validate()?;
    if model
        .vision
        .as_ref()
        .is_some_and(|ep| same_credential_scope(ep, &model.chat))
        && let (Some(chat), Some(vision)) = (input.chat_key.as_deref(), input.vision_key.as_deref())
        && !chat.trim().is_empty()
        && !vision.trim().is_empty()
        && chat.trim() != vision.trim()
    {
        return Err(AppError::Invalid(
            "同一服务商共用一份 API Key，请填写相同的 Key".into(),
        ));
    }
    for (ep, key) in [
        (Some(&model.chat), &input.chat_key),
        (model.vision.as_ref(), &input.vision_key),
    ] {
        if let (Some(ep), Some(k)) = (ep, key.as_deref().map(str::trim).filter(|k| !k.is_empty())) {
            state.secrets.set(ep, k)?;
        }
    }
    let config = {
        let mut cfg = state.config.lock().map_err(poisoned)?;
        let mut next = cfg.clone();
        next.model = model.clone();
        next.save(&state.paths.config())?;
        *cfg = next.clone();
        next
    };
    super::notify_config(&app, &config);
    // 之前因为没配置模型而暂存的输入，现在可以处理了
    state.wake.notify_one();
    Ok(view(&state, &model))
}

#[tauri::command]
pub fn clear_api_key(state: State<'_, AppState>, endpoint: Endpoint) -> AppResult<()> {
    let _maintenance = state
        .maintenance
        .try_read()
        .map_err(|_| AppError::Invalid("正在恢复数据，请稍后再试".into()))?;
    state.secrets.delete(&endpoint)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestResult {
    ok: bool,
    latency_ms: u64,
    /// 模型是否按要求调用了工具（知了依赖工具调用改数据）
    tool_calls: bool,
    message: String,
}

/// 连接测试：发一个很小的请求，并检查是否支持工具调用
#[tauri::command]
pub async fn test_model(state: State<'_, AppState>, target: String) -> AppResult<TestResult> {
    let _maintenance = state.maintenance.read().await;
    let ep = {
        let cfg = state.config.lock().map_err(poisoned)?;
        match target.as_str() {
            "vision" => cfg
                .model
                .vision
                .clone()
                .unwrap_or_else(|| cfg.model.chat.clone()),
            _ => cfg.model.chat.clone(),
        }
    };
    let key = state
        .secrets
        .get(&ep)?
        .filter(|key| !key.trim().is_empty())
        .ok_or_else(|| AppError::Invalid("还没有填写这个接口地址的 API Key".into()))?;
    let model = match Model::new(&ep, key) {
        Ok(m) => m,
        Err(e) => return Ok(fail(0, &e)),
    };
    let tools = [ToolSpec {
        name: "ping".into(),
        description: "连接测试。收到测试消息时调用一次。".into(),
        parameters: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
    }];
    let vision = target == "vision";
    let msgs = [Message::User {
        text: if vision {
            "这是一次连接测试，请回复连接正常。"
        } else {
            "这是一次连接测试，请调用 ping 工具。"
        }
        .into(),
    }];
    let req = ChatRequest {
        system: if vision {
            "你是连接测试助手，请简短回答。"
        } else {
            "你是连接测试助手。收到测试消息时调用 ping 工具，不要输出其他内容。"
        },
        messages: &msgs,
        images: &[],
        tools: if target == "vision" { &[] } else { &tools },
    };
    let start = Instant::now();
    let r = model.chat(&req, &|_| {}).await;
    let ms = start.elapsed().as_millis() as u64;
    Ok(match r {
        Ok(resp) => {
            let tool_calls = resp.tool_calls.iter().any(|c| c.name == "ping");
            TestResult {
                ok: true,
                latency_ms: ms,
                tool_calls,
                message: if vision {
                    "连接正常（尚未验证图片识别）".into()
                } else if tool_calls {
                    "连接正常".into()
                } else {
                    "连接正常，但模型没有调用工具，可能不支持工具调用".into()
                },
            }
        }
        Err(e) => fail(ms, &e),
    })
}

fn fail(ms: u64, e: &LlmError) -> TestResult {
    TestResult {
        ok: false,
        latency_ms: ms,
        tool_calls: false,
        message: e.to_string(),
    }
}

/// 打开数据目录或提示词文件夹
#[tauri::command]
pub fn open_folder(state: State<'_, AppState>, which: String) -> AppResult<()> {
    let path = match which.as_str() {
        "prompts" => state.paths.prompts(),
        "backups" => state.paths.backups(),
        _ => state.paths.root.clone(),
    };
    std::process::Command::new("explorer")
        .arg(&path)
        .spawn()
        .map_err(|e| AppError::Invalid(format!("打开文件夹失败：{e}")))?;
    Ok(())
}
