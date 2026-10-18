//! 重复任务工具；仅生成计划，写入由统一服务执行。
use super::tools::{Outcome, s, spec};
use crate::llm::{ToolCall, ToolSpec};
use crate::service::Service;
use crate::service::changes::Change;
use crate::service::recur::RecurChange;
use serde_json::{Value, json};

pub fn specs() -> Vec<ToolSpec> {
    let fields = json!({"title":{"type":"string"},"kind":{"type":"string","enum":["daily","weekdays","weekly","monthly","yearly"]},"params":{"type":"object","description":"weekly: {weekday:1..7}，monthly: {day:1..31}，yearly: {month:1..12,day:1..31}；daily/weekdays:{}"},"remind_time":{"type":"string","description":"HH:MM；空字符串取消提醒"},"note":{"type":"string"},"start_date":{"type":"string","description":"YYYY-MM-DD，默认输入时的业务日"},"end_date":{"type":"string","description":"YYYY-MM-DD；空字符串清空"}});
    let mut update = fields.clone();
    update["id"] = json!({"type":"string","description":"recur_query返回的规则编号"});
    update["active"] = json!({"type":"boolean"});
    vec![
        spec(
            "recur_add",
            "添加重复任务规则，按日切生成任务。工作日为周一至周五。",
            fields,
            &["title", "kind"],
        ),
        spec(
            "recur_update",
            "修改重复规则，只传要改字段；不改变已经生成的任务。",
            update,
            &["id"],
        ),
        spec(
            "recur_delete",
            "删除重复规则，需要确认；保留已经生成的任务。",
            json!({"id":{"type":"string"}}),
            &["id"],
        ),
        spec(
            "recur_query",
            "查询重复规则的编号、频率、时间和启用状态。",
            json!({"keyword":{"type":"string"}}),
            &[],
        ),
    ]
}

fn id(args: &Value) -> Result<i64, String> {
    args.get("id")
        .and_then(|v| {
            v.as_i64().or_else(|| {
                v.as_str()
                    .and_then(|s| s.trim_start_matches('#').parse().ok())
            })
        })
        .filter(|id| *id > 0)
        .ok_or_else(|| "需要有效的重复规则编号".into())
}

pub fn handle(svc: &Service<'_>, call: &ToolCall, plan: &mut Vec<Change>, today: &str) -> Outcome {
    let Some(args) = call.arguments.as_ref().filter(|v| v.is_object()) else {
        return Outcome::Error("参数必须是JSON对象".into());
    };
    let Some(definition) = specs().into_iter().find(|s| s.name == call.name) else {
        return Outcome::Error("未知的重复任务工具".into());
    };
    for (key, value) in args.as_object().unwrap() {
        let Some(schema) = definition.parameters["properties"].get(key) else {
            return Outcome::Error(format!("未知参数：{key}"));
        };
        let valid = match schema["type"].as_str() {
            Some("string") => value.is_string() || (key == "id" && value.as_i64().is_some()),
            Some("boolean") => value.is_boolean(),
            Some("object") => value.is_object(),
            _ => false,
        };
        if !valid {
            return Outcome::Error(format!("参数 {key} 的类型不正确"));
        }
    }
    if call.name == "recur_update" && args.as_object().unwrap().keys().all(|k| k == "id") {
        return Outcome::Error("请提供要修改的重复规则字段".into());
    }
    if call.name == "recur_query" {
        return match svc.query_recur_rules(s(args, "keyword").as_deref()) {
            Ok(r) => Outcome::Ok(serde_json::to_string(&r).unwrap_or_default()),
            Err(e) => Outcome::Error(e.to_string()),
        };
    }
    let parse = || -> Result<RecurChange, String> {
        Ok(match call.name.as_str() {
            "recur_add" => RecurChange::Add {
                title: s(args, "title").ok_or("缺少title")?,
                kind: s(args, "kind").ok_or("缺少kind")?,
                params: args.get("params").cloned().unwrap_or_else(|| json!({})),
                remind_time: s(args, "remind_time"),
                note: s(args, "note"),
                start_date: s(args, "start_date").unwrap_or_else(|| today.into()),
                end_date: s(args, "end_date"),
            },
            "recur_update" => RecurChange::Update {
                id: id(args)?,
                title: s(args, "title"),
                kind: s(args, "kind"),
                params: args.get("params").cloned(),
                remind_time: s(args, "remind_time"),
                note: s(args, "note"),
                start_date: s(args, "start_date"),
                end_date: s(args, "end_date"),
                active: match args.get("active") {
                    Some(v) => Some(v.as_bool().ok_or("active必须为布尔值")?),
                    None => None,
                },
            },
            "recur_delete" => RecurChange::Delete { id: id(args)? },
            _ => return Err("未知的重复任务工具".into()),
        })
    };
    let c = match parse() {
        Ok(c) => Change::Recur { change: c },
        Err(e) => return Outcome::Error(e),
    };
    let mut trial = plan.clone();
    trial.push(c.clone());
    if let Err(e) = svc.validate_plan(&trial) {
        return Outcome::Error(format!("没有计入：{e}"));
    }
    plan.push(c);
    Outcome::Ok("已计入重复规则变更计划，校验及确认后统一执行。".into())
}
