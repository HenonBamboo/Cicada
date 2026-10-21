//! 实体选择由当前查询生成标签，内部引用只保存在待追问上下文中。
use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    intent::{IntentChecklist, ReadScope},
    reply::EntityOption,
    tools::{self, Outcome},
};
use crate::llm::ToolCall;
use crate::service::{Service, changes::Pin};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Target {
    pub kind: String,
    pub id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingOption {
    #[serde(default)]
    pub(super) intent_id: String,
    pub(super) value: String,
    pub(super) label: String,
    pub(super) targets: Vec<Target>,
    pub(super) pins: Vec<Pin>,
    pub(super) snapshots: Vec<Value>,
    #[serde(default)]
    pub(super) expires_at: String,
}

impl PendingOption {
    pub(super) fn view(&self) -> EntityOption {
        EntityOption {
            label: self.label.clone(),
            value: self.value.clone(),
        }
    }
}

/// 系统通知回执不改变业务 version，也不改变实体身份。
pub(super) fn same_row(entity: &str, old: &Value, new: &Value) -> bool {
    if entity != "tasks" {
        return old == new;
    }
    match (old.as_object(), new.as_object()) {
        (Some(old), Some(new)) => {
            old.iter()
                .filter(|(key, _)| key.as_str() != "reminded_at")
                .all(|(key, value)| new.get(key) == Some(value))
                && new
                    .keys()
                    .filter(|key| key.as_str() != "reminded_at")
                    .all(|key| old.contains_key(key))
        }
        _ => old == new,
    }
}

pub(super) fn same_snapshots(old: &[Value], new: &[Value]) -> bool {
    old.len() == new.len()
        && old.iter().zip(new).all(|(old, new)| {
            old["entity"] == new["entity"]
                && old["id"] == new["id"]
                && same_row(
                    old["entity"].as_str().unwrap_or(""),
                    &old["row"],
                    &new["row"],
                )
        })
}

/// 禁止将数据库定位标识包装成界面中的“第几条”。日期和时间不匹配这里。
pub(super) fn has_internal_reference(text: &str) -> bool {
    for (start, _) in text.match_indices("编号") {
        let prefix = &text[..start];
        let internal = prefix
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_alphanumeric())
            || ["任务", "项目", "节点", "内部"]
                .iter()
                .any(|word| prefix.ends_with(word));
        if internal
            && text[start + "编号".len()..]
                .trim_start_matches([' ', '：', ':', '#'])
                .starts_with(|c: char| c.is_ascii_digit())
        {
            return true;
        }
    }
    let chars: Vec<_> = text.chars().collect();
    for (i, character) in chars.iter().enumerate() {
        if *character == '第' && chars.get(i + 1).is_some_and(char::is_ascii_digit) {
            let mut end = i + 1;
            while chars
                .get(end)
                .is_some_and(|c| c.is_ascii_digit() || matches!(c, '、' | ',' | '，' | ' '))
            {
                end += 1;
            }
            if chars
                .get(end)
                .is_some_and(|c| matches!(c, '条' | '项' | '号' | '个'))
            {
                let suffix: String = chars[end..].iter().collect();
                if !["个工作日", "个月", "个季度"]
                    .iter()
                    .any(|time| suffix.starts_with(time))
                {
                    return true;
                }
            }
        }
        if *character == '$' && chars.get(i + 1).is_some_and(char::is_ascii_digit) {
            return true;
        }
    }
    for (start, _) in text.match_indices('#') {
        if text[start + 1..].starts_with(|c: char| c.is_ascii_digit())
            && ["任务", "项目", "节点"]
                .iter()
                .any(|word| text[..start].trim_end().ends_with(word))
        {
            return true;
        }
    }
    let lowercase = text.to_ascii_lowercase();
    lowercase.match_indices("id").any(|(start, _)| {
        let boundary = lowercase[..start]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_alphanumeric());
        let rest = lowercase[start + 2..].trim_start();
        boundary
            && rest
                .strip_prefix(['=', ':', '：'])
                .is_some_and(|value| value.trim_start().starts_with(|c: char| c.is_ascii_digit()))
    })
}

fn label(reads: &ReadScope, target: &Target) -> Result<String, String> {
    let row = reads
        .row(&target.kind, target.id)
        .ok_or("请先查询当前任务或项目，再生成实体选项")?;
    if target.kind == "task" {
        return row
            .get("display_label")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| "任务查询缺少可读定位，请重新查询".into());
    }
    let mut names = Vec::new();
    let mut current = Some(target.id);
    let mut visited = HashSet::new();
    while let Some(id) = current {
        if !visited.insert(id) {
            return Err("项目路径有循环，请重新查询".into());
        }
        let node = reads
            .row("node", id)
            .ok_or("项目路径不完整，请重新查询项目上下文")?;
        names.push(
            node.get("title")
                .and_then(Value::as_str)
                .ok_or("项目缺少名称")?
                .to_string(),
        );
        current = node.get("parentId").and_then(Value::as_i64);
    }
    names.reverse();
    let mut text = format!("「{}」", names.join(" › "));
    if let Some(due) = row
        .get("dueAt")
        .and_then(Value::as_str)
        .filter(|d| !d.is_empty())
    {
        text.push_str(&format!("（截止 {}）", due.replace('T', " ")));
    }
    Ok(text)
}

fn options(
    svc: &Service<'_>,
    args: &Value,
    reads: &ReadScope,
    checklist: &IntentChecklist,
) -> Result<Vec<PendingOption>, String> {
    let Some(raw) = args.get("entity_options") else {
        return Ok(vec![]);
    };
    if raw.is_null() || raw.as_array().is_some_and(Vec::is_empty) {
        return Ok(vec![]);
    }
    let choices = raw
        .as_array()
        .filter(|a| !a.is_empty() && a.len() <= 5)
        .ok_or("entity_options 需要1到5个实体选项")?;
    let intent_id = tools::s(args, "intent_id")
        .filter(|id| !id.is_empty())
        .ok_or("实体追问需要原 intent_id，不能将选择作用于整轮其它要求")?;
    checklist.selection_intent(&intent_id)?;
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    for choice in choices {
        let targets = choice
            .get("targets")
            .and_then(Value::as_array)
            .filter(|a| !a.is_empty() && a.len() <= 50)
            .ok_or("每个实体选项需要1到50个targets")?;
        let mut selected = Vec::new();
        let mut pins = Vec::new();
        let mut labels = Vec::new();
        for raw in targets {
            let kind = raw
                .get("kind")
                .and_then(Value::as_str)
                .filter(|k| matches!(*k, "task" | "node"))
                .ok_or("实体kind需要task或node")?;
            let id = raw
                .get("id")
                .and_then(|v| {
                    v.as_i64()
                        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
                })
                .filter(|id| *id > 0)
                .ok_or("实体选择需要当前查询返回的有效定位")?;
            let target = Target {
                kind: kind.into(),
                id,
            };
            if selected.contains(&target) {
                return Err("同一个实体选项不能重复列出同一内容".into());
            }
            let row = reads
                .row(kind, id)
                .ok_or("实体尚未经过本轮查询，请先查询当前内容")?;
            labels.push(label(reads, &target)?);
            pins.push(Pin {
                entity: if kind == "task" { "tasks" } else { "nodes" }.into(),
                id,
                version: row
                    .get("version")
                    .and_then(Value::as_i64)
                    .ok_or("查询缺少版本信息，请重新查询")?,
            });
            selected.push(target);
        }
        let snapshots = svc
            .plan_snapshots(&pins)
            .map_err(|_| "相关内容已经变化，请重新查询再生成选项")?;
        for (target, snapshot) in selected.iter().zip(&snapshots) {
            let queried = reads.row(&target.kind, target.id).unwrap();
            let actual = &snapshot["row"];
            if actual["version"] != queried["version"]
                || actual["title"] != queried["title"]
                || actual["created_at"]
                    != queried[if target.kind == "task" {
                        "created_at"
                    } else {
                        "createdAt"
                    }]
            {
                return Err("查询后内容已经变化，请重新查询再生成选项".into());
            }
        }
        let mut identity: Vec<_> = selected
            .iter()
            .map(|t| format!("{}:{}", t.kind, t.id))
            .collect();
        identity.sort();
        if !seen.insert(identity) {
            return Err("实体选项不能重复".into());
        }
        result.push(PendingOption {
            intent_id: intent_id.clone(),
            value: uuid::Uuid::new_v4().to_string(),
            label: labels.join("；"),
            targets: selected,
            pins,
            snapshots,
            expires_at: String::new(),
        });
    }
    // 同名同日任务仍可用录入时间辨别，绝不用数据库编号补标签。
    let labels: Vec<_> = result.iter().map(|c| c.label.clone()).collect();
    for choice in &mut result {
        if labels
            .iter()
            .filter(|label| **label == choice.label)
            .count()
            > 1
        {
            let created: Vec<_> = choice
                .snapshots
                .iter()
                .filter_map(|s| s["row"]["created_at"].as_str())
                .map(|s| s.replace('T', " "))
                .collect();
            choice
                .label
                .push_str(&format!("（录入 {}）", created.join("、")));
        }
    }
    if result
        .iter()
        .map(|c| &c.label)
        .collect::<HashSet<_>>()
        .len()
        != result.len()
    {
        return Err(
            "候选内容的名称和时间完全相同，请追问可区分的日期或备注，不要使用内部编号".into(),
        );
    }
    Ok(result)
}

pub(super) fn handle(
    svc: &Service<'_>,
    call: &ToolCall,
    reads: &ReadScope,
    checklist: &IntentChecklist,
) -> Outcome {
    let Some(args) = call.arguments.as_ref() else {
        return Outcome::Error("追问需要JSON参数".into());
    };
    let choices = match options(svc, args, reads, checklist) {
        Ok(choices) => choices,
        Err(error) => return Outcome::Error(error),
    };
    let question = tools::s(args, "question")
        .filter(|q| !q.is_empty())
        .unwrap_or_else(|| "你指的是哪一项？".into());
    let plain: Vec<String> = args
        .get("options")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .take(5)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    if choices.is_empty()
        && (reads.has_internal_reference(&question)
            || plain.iter().any(|s| reads.has_internal_reference(s)))
    {
        return Outcome::Error("不能让用户按内部编号选择；请用entity_options.targets，由程序生成名称、计划日期和提醒时间选项".into());
    }
    Outcome::Clarify {
        question: if choices.is_empty() {
            question
        } else {
            "请选择要处理的内容。".into()
        },
        options: if choices.is_empty() { plain } else { vec![] },
        entity_options: choices,
    }
}
