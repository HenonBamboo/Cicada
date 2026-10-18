//! 项目模式的工具定义和参数转换。写入仍走统一变更计划。
use serde_json::{Value, json};

use super::tools::{Outcome, normalize_time, s, spec};
use crate::llm::{ToolCall, ToolSpec};
use crate::service::Service;
use crate::service::changes::{Change, TaskRef};
use crate::service::project_changes::ProjectChange;

fn reference(args: &Value, key: &str) -> Result<TaskRef, String> {
    match args.get(key) {
        Some(Value::Number(n)) => n.as_i64().filter(|id| *id > 0).map(TaskRef::Id),
        Some(Value::String(v)) if v.starts_with('$') => Some(TaskRef::Temp(v.clone())),
        Some(Value::String(v)) => v
            .trim()
            .trim_start_matches('#')
            .parse::<i64>()
            .ok()
            .filter(|id| *id > 0)
            .map(TaskRef::Id),
        _ => None,
    }
    .ok_or_else(|| format!("{key} 需要有效的节点编号"))
}

pub fn specs() -> Vec<ToolSpec> {
    let id = json!({"type":"string","description":"项目或节点编号，或本轮新建项目/节点的 $1、$2 引用（统一按新建顺序编号）"});
    let title = json!({"type":"string","description":"简短标题"});
    let date = json!({"type":"string","description":"截止时间 YYYY-MM-DD HH:MM；空字符串清空"});
    vec![
        spec(
            "project_add",
            "新建第一层项目。",
            json!({"title":title,"note":{"type":"string"},"due_at":date}),
            &["title"],
        ),
        spec(
            "node_add",
            "在指定父节点下新建子节点，含项目最多三层。",
            json!({"parent_id":id,"title":title,"note":{"type":"string"},"due_at":date}),
            &["parent_id", "title"],
        ),
        spec(
            "node_update",
            "修改节点标题、备注或截止时间；只传要改的字段。",
            json!({"id":id,"title":title,"note":{"type":"string"},"due_at":date}),
            &["id"],
        ),
        spec(
            "node_move",
            "移动节点及其子树到另一个父节点，不能循环或超过三层。",
            json!({"id":id,"parent_id":id}),
            &["id", "parent_id"],
        ),
        spec(
            "node_delete",
            "删除节点及子树，程序先确认；未完成关联任务一并删除，完成历史保留。",
            json!({"id":id}),
            &["id"],
        ),
        spec(
            "node_set_done",
            "完成或取消完成节点，同时同步关联任务。不要未经用户同意自动完成父节点。",
            json!({"id":id,"done":{"type":"boolean"}}),
            &["id", "done"],
        ),
        spec(
            "project_set_status",
            "设置项目状态：进行中、已完成、已归档。",
            json!({"id":id,"status":{"type":"string","enum":["active","done","archived"]}}),
            &["id", "status"],
        ),
        spec(
            "node_schedule_today",
            "将节点安排为今日任务，已有未完成关联任务时复用。",
            json!({"id":id}),
            &["id"],
        ),
        spec(
            "project_query",
            "查询项目节点及其父子关系、状态、进度所需数据。修改前查询准确编号。",
            json!({"keyword":{"type":"string","description":"可选标题关键词"}}),
            &[],
        ),
    ]
}

pub fn handle(svc: &Service<'_>, call: &ToolCall, plan: &mut Vec<Change>, today: &str) -> Outcome {
    let Some(args) = call.arguments.as_ref().filter(|a| a.is_object()) else {
        return Outcome::Error("参数需要 JSON 对象".into());
    };
    let definition = specs().into_iter().find(|s| s.name == call.name);
    let Some(definition) = definition else {
        return Outcome::Error("未知的项目工具".into());
    };
    let properties = &definition.parameters["properties"];
    for (key, value) in args.as_object().unwrap() {
        let Some(schema) = properties.get(key) else {
            return Outcome::Error(format!("未知参数：{key}"));
        };
        let valid = match schema["type"].as_str() {
            Some("boolean") => value.is_boolean(),
            Some("string") => {
                value.is_string()
                    || (matches!(key.as_str(), "id" | "parent_id") && value.as_i64().is_some())
            }
            _ => false,
        };
        if !valid {
            return Outcome::Error(format!("参数 {key} 的类型不正确"));
        }
    }
    if call.name == "node_update"
        && !["title", "note", "due_at"]
            .iter()
            .any(|key| args.get(key).is_some())
    {
        return Outcome::Error("请提供要修改的标题、备注或截止时间".into());
    }
    if call.name == "project_query" {
        return match svc.query_project_nodes(s(args, "keyword").as_deref()) {
            Ok(nodes) => Outcome::Ok(serde_json::to_string(&nodes).unwrap_or_default()),
            Err(e) => Outcome::Error(e.to_string()),
        };
    }
    let parsed = (|| -> Result<ProjectChange, String> {
        let title = || {
            s(args, "title")
                .filter(|t| !t.is_empty())
                .ok_or_else(|| "缺少 title".to_string())
        };
        let due = || {
            s(args, "due_at")
                .map(|v| normalize_time(&v, today))
                .transpose()
        };
        Ok(match call.name.as_str() {
            "project_add" => ProjectChange::Add {
                title: title()?,
                note: s(args, "note"),
                due_at: due()?,
            },
            "node_add" => ProjectChange::NodeAdd {
                parent: reference(args, "parent_id")?,
                title: title()?,
                note: s(args, "note"),
                due_at: due()?,
            },
            "node_update" => ProjectChange::Update {
                node: reference(args, "id")?,
                title: s(args, "title"),
                note: s(args, "note"),
                due_at: due()?,
            },
            "node_move" => ProjectChange::Move {
                node: reference(args, "id")?,
                parent: reference(args, "parent_id")?,
            },
            "node_delete" => ProjectChange::Delete {
                node: reference(args, "id")?,
            },
            "node_set_done" => ProjectChange::SetDone {
                node: reference(args, "id")?,
                done: args
                    .get("done")
                    .and_then(Value::as_bool)
                    .ok_or("done 必须为布尔值")?,
            },
            "project_set_status" => ProjectChange::SetStatus {
                project: reference(args, "id")?,
                status: s(args, "status").ok_or("缺少 status")?,
            },
            "node_schedule_today" => ProjectChange::Schedule {
                node: reference(args, "id")?,
                planned_date: today.into(),
            },
            _ => return Err("未知的项目工具".into()),
        })
    })();
    let change = match parsed {
        Ok(change) => change,
        Err(e) => return Outcome::Error(e),
    };
    let new_node = matches!(
        change,
        ProjectChange::Add { .. } | ProjectChange::NodeAdd { .. }
    );
    let mut trial = plan.clone();
    trial.push(Change::Project { change });
    if let Err(e) = svc.validate_plan(&trial) {
        return Outcome::Error(format!("没有计入：{e}"));
    }
    let note = if new_node {
        let n = trial
            .iter()
            .filter(|c| {
                matches!(
                    c,
                    Change::Project {
                        change: ProjectChange::Add { .. } | ProjectChange::NodeAdd { .. }
                    }
                )
            })
            .count();
        format!("本次新建节点引用为 ${n}。")
    } else {
        String::new()
    };
    *plan = trial;
    Outcome::Ok(format!(
        "已计入计划。{note}全部变更在本轮结束后统一提交；父节点不会自动完成。"
    ))
}
