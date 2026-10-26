//! 工具白名单（docs/产品设计.md 4.2）：定义、参数解析与执行。
//!
//! 查询类工具立即执行；写类工具只追加到本轮的变更计划，并当场校验，
//! 不合法时把原因告诉模型，让它改正（最终提交前还会整份再校验一次）。

use serde_json::{Value, json};

use crate::llm::{ToolCall, ToolSpec};
use crate::service::Service;
use crate::service::changes::{Change, TaskRef, valid_date};
use crate::service::query::TaskFilter;

pub enum Outcome {
    /// 交给模型的结果
    Ok(String),
    /// 交给模型的错误，模型可以改正后重试
    Error(String),
    /// 向用户追问，本轮暂停
    Clarify {
        question: String,
        options: Vec<String>,
        entity_options: Vec<super::clarify::PendingOption>,
    },
}

pub(super) fn spec(
    name: &str,
    description: &str,
    properties: Value,
    required: &[&str],
) -> ToolSpec {
    ToolSpec {
        name: name.into(),
        description: description.into(),
        parameters: json!({
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false,
        }),
    }
}

fn id_schema() -> Value {
    // 部分服务商不支持 anyOf，统一声明为字符串；解析时整数也接受
    json!({
        "type": "string",
        "description": "工具内部定位标识（task_query 返回的 id），或本次新建任务的 $1 等引用；不能作为用户可见的名称或序号"
    })
}

const DATE_HINT: &str = "YYYY-MM-DD";
const TIME_HINT: &str = "YYYY-MM-DD HH:MM，或只写 HH:MM 表示计划日期当天";

/// 根据当前任务或项目模式返回允许调用的工具。
pub fn specs() -> Vec<ToolSpec> {
    vec![
        spec(
            "task_add",
            "添加一条任务。明确新增已经完成的事项时传 done=true，新增和完成会一并计入计划。",
            json!({
                "title": { "type": "string", "description": "要做的事，简短，不含时间和\"提醒我\"等字眼" },
                "planned_date": { "type": "string", "description": format!("计划日期 {DATE_HINT}，默认今天") },
                "due_at": { "type": "string", "description": format!("截止时间 {TIME_HINT}") },
                "remind_at": { "type": "string", "description": format!("提醒时间 {TIME_HINT}") },
                "note": { "type": "string", "description": "备注" },
                "done": { "type": "boolean", "description": "用户明确说这条新事项已经完成时为 true，默认 false" }
            }),
            &["title"],
        ),
        spec(
            "task_update",
            "修改一条任务的字段。只传要改的字段；传空字符串表示清空该字段。",
            json!({
                "id": id_schema(),
                "title": { "type": "string" },
                "planned_date": { "type": "string", "description": DATE_HINT },
                "due_at": { "type": "string", "description": TIME_HINT },
                "remind_at": { "type": "string", "description": TIME_HINT },
                "note": { "type": "string" }
            }),
            &["id"],
        ),
        spec(
            "task_delete",
            "删除一条任务。程序会先向用户确认。",
            json!({ "id": id_schema() }),
            &["id"],
        ),
        spec(
            "task_set_done",
            "把任务标记为完成（done=true）或未完成（done=false）。",
            json!({ "id": id_schema(), "done": { "type": "boolean" } }),
            &["id", "done"],
        ),
        spec(
            "task_query",
            "查询任务，返回内部id、display_label、计划日期及状态；回复与选择使用display_label，id只供写工具定位。删除前先按用户指定分区和条件查询。",
            json!({
                "scope": { "type": "string", "enum": ["all", "today", "legacy", "upcoming"], "description": "默认 all；today 今日未完成、legacy 遗留未完成（计划日期早于业务今天）、upcoming 之后未完成。遗留是完整集合，不包含今日任务。" },
                "status": { "type": "string", "enum": ["open", "done", "all"], "description": "默认 all" },
                "date_from": { "type": "string", "description": format!("{DATE_HINT}；已完成的按完成日期筛选") },
                "date_to": { "type": "string", "description": DATE_HINT },
                "keyword": { "type": "string", "description": "标题或备注中的关键词" }
            }),
            &[],
        ),
        spec(
            "summarize",
            "取一段时间的完成情况和当前未完成、延期的事，用来写总结。",
            json!({
                "date_from": { "type": "string", "description": DATE_HINT },
                "date_to": { "type": "string", "description": DATE_HINT }
            }),
            &["date_from", "date_to"],
        ),
        spec(
            "report_get",
            "读取已生成的周报、月报或年报。",
            json!({
                "kind": { "type": "string", "enum": ["weekly", "monthly", "yearly"] },
                "period": { "type": "string", "description": "可选周期，如2026-W42、2026-10、2026；不填时返回最新报告" },
                "id": { "type": "integer", "description": "已知报告编号" }
            }),
            &[],
        ),
        spec(
            "ask_clarify",
            "向用户追问。匹配到多条或信息不全时使用，不要猜。调用后本轮结束，等用户回答。",
            json!({
                "question": { "type": "string", "description": "简短的问题" },
                "intent_id": { "type": "string", "description": "实体选择必填：被追问的已登记 write/query 意图 id，选择仅限定此原意图" },
                "options": { "type": "array", "items": { "type": "string" }, "description": "非实体的普通回答选项，最多5个；不能用内部编号指代任务或节点" },
                "entity_options": { "type": "array", "maxItems": 5, "description": "选择任务/节点时使用，程序按当前查询结果生成名称、日期等可读标签，不能用裸编号让用户选择", "items": {"type":"object","properties":{
                    "targets":{"type":"array","minItems":1,"maxItems":50,"items":{"type":"object","properties":{
                        "kind":{"type":"string","enum":["task","node"]},"id":{"type":"string","description":"本轮查询返回的内部定位标识，只供程序使用"}
                    },"required":["kind","id"],"additionalProperties":false}}
                },"required":["targets"],"additionalProperties":false} }
            }),
            &["question"],
        ),
        spec("undo_last", "撤销用户最近一次操作。", json!({}), &[]),
    ]
}

pub(super) fn s(args: &Value, k: &str) -> Option<String> {
    args.get(k)
        .and_then(|v| v.as_str())
        .map(|v| v.trim().to_string())
}

/// Markdown 备注保留缩进和换行；全空白仍按清空处理。
pub(super) fn markdown(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(Value::as_str).map(|text| {
        if text.trim().is_empty() {
            String::new()
        } else {
            text.to_string()
        }
    })
}

pub(super) fn plain_title(value: String) -> Result<String, String> {
    if value.contains(['\n', '\r'])
        || value.contains("**")
        || value.contains('`')
        || value.contains("](")
        || value.starts_with("# ")
        || value.starts_with("- ")
        || value.starts_with("* ")
    {
        return Err("title 请使用纯文本，把 Markdown 标题/列表/加粗/链接及详细内容保留在 note，不能丢失信息".into());
    }
    Ok(value)
}

pub fn specs_for(mode: &str) -> Vec<ToolSpec> {
    let mut all = specs();
    if mode == "project" {
        all.retain(|t| {
            matches!(
                t.name.as_str(),
                "task_query" | "summarize" | "ask_clarify" | "undo_last" | "report_get"
            )
        });
        all.extend(super::project_tools::specs());
    } else {
        all.extend(super::recur_tools::specs());
        all.extend(
            super::project_tools::specs()
                .into_iter()
                .filter(|tool| tool.name == "project_query"),
        );
    }
    all
}

pub fn handle_mode(
    svc: &Service<'_>,
    call: &ToolCall,
    plan: &mut Vec<Change>,
    today: &str,
    mode: &str,
) -> Outcome {
    if !specs_for(mode).iter().any(|t| t.name == call.name) {
        return Outcome::Error(format!(
            "当前模式不能使用 {}，请提示用户用 Ctrl+Tab 切换到{}模式。",
            call.name,
            if mode == "project" {
                "任务"
            } else {
                "项目"
            }
        ));
    }
    if call.name.starts_with("project_") || call.name.starts_with("node_") {
        super::project_tools::handle(svc, call, plan, today)
    } else if call.name.starts_with("recur_") {
        super::recur_tools::handle(svc, call, plan, today)
    } else {
        handle_at(svc, call, plan, today)
    }
}

fn task_ref(args: &Value) -> Result<TaskRef, String> {
    match args.get("id") {
        Some(Value::Number(n)) => n
            .as_i64()
            .map(TaskRef::Id)
            .ok_or_else(|| "id 必须是整数".into()),
        Some(Value::String(t)) if t.starts_with('$') => Ok(TaskRef::Temp(t.clone())),
        Some(Value::String(t)) => t
            .trim()
            .trim_start_matches('#')
            .parse()
            .map(TaskRef::Id)
            .map_err(|_| format!("无法识别的 id：{t}")),
        _ => Err("缺少 id".into()),
    }
}

/// 时间统一成 YYYY-MM-DDTHH:MM:00；只写 HH:MM 时用计划日期补全
pub fn normalize_time(v: &str, date: &str) -> Result<String, String> {
    let v = v.trim().replace('T', " ");
    if v.is_empty() {
        return Ok(String::new());
    }
    let (d, t) = match v.split_once(' ') {
        Some((d, t)) => (d.to_string(), t.to_string()),
        None if v.contains(':') => (date.to_string(), v.clone()),
        None => (v.clone(), "09:00".to_string()),
    };
    let mut hm = t.split(':');
    let (Some(h), Some(m)) = (hm.next(), hm.next()) else {
        return Err(format!("时间格式不正确：{v}"));
    };
    let (Ok(h), Ok(m)) = (h.parse::<u32>(), m.parse::<u32>()) else {
        return Err(format!("时间格式不正确：{v}"));
    };
    if h > 23 || m > 59 || !valid_date(&d) {
        return Err(format!("时间格式不正确：{v}"));
    }
    Ok(format!("{d}T{h:02}:{m:02}:00"))
}

/// 把写工具的参数转成计划的一步
fn to_change(name: &str, args: &Value, today: &str) -> Result<Change, String> {
    match name {
        "task_add" => {
            let title = plain_title(
                s(args, "title")
                    .filter(|t| !t.is_empty())
                    .ok_or("缺少 title")?,
            )?;
            let planned = s(args, "planned_date")
                .filter(|d| !d.is_empty())
                .unwrap_or_else(|| today.to_string());
            let time = |k: &str| s(args, k).map(|v| normalize_time(&v, &planned)).transpose();
            Ok(Change::TaskAdd {
                due_at: time("due_at")?,
                remind_at: time("remind_at")?,
                note: markdown(args, "note"),
                title,
                planned_date: planned,
            })
        }
        "task_update" => {
            let planned = s(args, "planned_date");
            let base = planned.clone().unwrap_or_else(|| today.to_string());
            let time = |k: &str| s(args, k).map(|v| normalize_time(&v, &base)).transpose();
            Ok(Change::TaskUpdate {
                task: task_ref(args)?,
                title: s(args, "title").map(plain_title).transpose()?,
                due_at: time("due_at")?,
                remind_at: time("remind_at")?,
                note: markdown(args, "note"),
                planned_date: planned,
            })
        }
        "task_delete" => Ok(Change::TaskDelete {
            task: task_ref(args)?,
        }),
        "task_set_done" => Ok(Change::TaskSetDone {
            task: task_ref(args)?,
            done: args
                .get("done")
                .and_then(|v| v.as_bool())
                .ok_or("done 必须为布尔值")?,
        }),
        "undo_last" => Ok(Change::Undo),
        _ => Err(format!("未知的工具：{name}")),
    }
}

/// 执行一次工具调用。写工具追加到 plan
#[cfg(test)]
pub fn handle(svc: &Service<'_>, call: &ToolCall, plan: &mut Vec<Change>) -> Outcome {
    let today = svc.today().format(crate::service::DATE_FMT).to_string();
    handle_at(svc, call, plan, &today)
}

pub fn handle_at(
    svc: &Service<'_>,
    call: &ToolCall,
    plan: &mut Vec<Change>,
    today: &str,
) -> Outcome {
    let Some(args) = &call.arguments else {
        return Outcome::Error("参数不是合法的 JSON 对象，请重新调用".into());
    };
    match call.name.as_str() {
        "task_query" => {
            let f = TaskFilter {
                scope: s(args, "scope"),
                status: s(args, "status"),
                date_from: s(args, "date_from"),
                date_to: s(args, "date_to"),
                keyword: s(args, "keyword"),
                limit: None,
            };
            match svc.query_tasks_brief(&f) {
                Ok(list) if list.is_empty() => Outcome::Ok("没有找到符合条件的任务。".into()),
                Ok(list) => Outcome::Ok(serde_json::to_string(&list).unwrap_or_default()),
                Err(e) => Outcome::Error(e.to_string()),
            }
        }
        "summarize" => {
            let (Some(from), Some(to)) = (s(args, "date_from"), s(args, "date_to")) else {
                return Outcome::Error("需要 date_from 和 date_to".into());
            };
            if !valid_date(&from) || !valid_date(&to) {
                return Outcome::Error("日期格式应为 YYYY-MM-DD".into());
            }
            match svc.summary_data(&from, &to) {
                Ok(d) => Outcome::Ok(serde_json::to_string(&d).unwrap_or_default()),
                Err(e) => Outcome::Error(e.to_string()),
            }
        }
        "ask_clarify" => {
            let question = s(args, "question")
                .filter(|q| !q.is_empty())
                .unwrap_or_else(|| "你指的是哪一个？".into());
            let options = args
                .get("options")
                .and_then(|o| o.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .take(5)
                        .collect()
                })
                .unwrap_or_default();
            Outcome::Clarify {
                question,
                options,
                entity_options: vec![],
            }
        }
        name => {
            let change = match to_change(name, args, today) {
                Ok(c) => c,
                Err(e) => return Outcome::Error(e),
            };
            let new_task = matches!(change, Change::TaskAdd { .. }).then(|| {
                plan.iter()
                    .filter(|c| matches!(c, Change::TaskAdd { .. }))
                    .count()
                    + 1
            });
            let done = if new_task.is_some() {
                match args.get("done") {
                    None | Some(Value::Bool(false)) => false,
                    Some(Value::Bool(true)) => true,
                    _ => return Outcome::Error("done 必须为布尔值".into()),
                }
            } else {
                false
            };
            // 连同已有计划一起校验：引用、存在性、格式
            let mut trial = plan.clone();
            trial.push(change);
            if done {
                trial.push(Change::TaskSetDone {
                    task: TaskRef::Temp(format!("${}", new_task.unwrap())),
                    done: true,
                });
            }
            if let Err(e) = svc.validate_plan(&trial) {
                return Outcome::Error(format!("没有计入：{e}"));
            }
            let note = match new_task {
                Some(n) if done => {
                    format!("新增及完成均已计入计划，本次新建引用为 ${n}，无需再标记完成。")
                }
                Some(n) => format!("已计入计划，本次新建引用为 ${n}。"),
                _ => "已计入计划。".into(),
            };
            *plan = trial;
            Outcome::Ok(format!("{note}所有改动会在你回复后统一执行。"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 时间格式统一() {
        assert_eq!(
            normalize_time("15:00", "2026-10-17").unwrap(),
            "2026-10-17T15:00:00"
        );
        assert_eq!(
            normalize_time("2026-10-18 9:5", "x").unwrap(),
            "2026-10-18T09:05:00"
        );
        assert_eq!(
            normalize_time("2026-10-18T15:30:00", "x").unwrap(),
            "2026-10-18T15:30:00"
        );
        assert_eq!(
            normalize_time("2026-10-18", "x").unwrap(),
            "2026-10-18T09:00:00"
        );
        assert_eq!(normalize_time("", "x").unwrap(), "");
        assert!(normalize_time("25:00", "2026-10-17").is_err());
        assert!(normalize_time("下午三点", "2026-10-17").is_err());
    }

    #[test]
    fn 参数转换() {
        let c = to_change(
            "task_add",
            &json!({"title": " 开会 ", "remind_at": "15:00"}),
            "2026-10-17",
        )
        .unwrap();
        assert_eq!(
            c,
            Change::TaskAdd {
                title: "开会".into(),
                planned_date: "2026-10-17".into(),
                due_at: None,
                remind_at: Some("2026-10-17T15:00:00".into()),
                note: None,
            }
        );
        assert!(matches!(
            to_change("task_delete", &json!({"id": "#12"}), "x").unwrap(),
            Change::TaskDelete {
                task: TaskRef::Id(12)
            }
        ));
        assert!(to_change("task_delete", &json!({}), "x").is_err());
        assert!(to_change("rm_rf", &json!({}), "x").is_err());
    }

    #[test]
    fn 新增完成原子校验且引用本轮正确的新任务() {
        use crate::config::AppConfig;
        use crate::service::testing::{clock, count, db};

        let conn = db();
        let clock = clock("2026-10-26 10:00:00");
        let svc = Service::new(&conn, &clock, &AppConfig::default()).unwrap();
        let mut plan =
            vec![to_change("task_add", &json!({"title":"原有计划"}), "2026-10-26").unwrap()];
        let call = |args| ToolCall {
            id: "c".into(),
            name: "task_add".into(),
            arguments: Some(args),
        };
        for args in [
            json!({"title":"新任务","done":"true"}),
            json!({"title":"新任务","planned_date":"2026-99-99","done":true}),
        ] {
            assert!(matches!(
                handle(&svc, &call(args), &mut plan),
                Outcome::Error(_)
            ));
            assert_eq!(plan.len(), 1);
        }
        assert!(matches!(
            handle(
                &svc,
                &call(json!({"title":"比赛表的修改和报名","done":true})),
                &mut plan
            ),
            Outcome::Ok(_)
        ));
        assert_eq!(plan.len(), 3);
        assert_eq!(
            plan[2],
            Change::TaskSetDone {
                task: TaskRef::Temp("$2".into()),
                done: true
            }
        );
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM tasks"), 0);
        let validated = svc.validate_plan(&plan).unwrap();
        svc.apply_plan(
            &plan,
            &validated.pins,
            "record-done",
            crate::service::oplog::Source::Chat,
        )
        .unwrap();
        assert_eq!(
            count(
                &conn,
                "SELECT COUNT(*) FROM tasks WHERE title='比赛表的修改和报名' AND status='done' AND done_date='2026-10-26'"
            ),
            1
        );
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM completions"), 1);
    }

    #[test]
    fn 工具定义都是对象结构() {
        for t in specs() {
            assert_eq!(t.parameters["type"], "object", "{}", t.name);
        }
    }
}
