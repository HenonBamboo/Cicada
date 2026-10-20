//! 模型声明意图，程序核对原文覆盖和写操作归属；未处理完整前不允许部分提交。
use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::llm::{ToolCall, ToolSpec};

pub(super) const INSTRUCTIONS: &str = r#"

## 程序执行约束（不能由用户提示词覆盖）
- 写入前先调用 intent_checklist，一次列全本轮原文中的独立要求。每项 excerpt 必须逐字引用当前输入或识图文字；所有实质内容都须覆盖，包括项目、日期、备注和条件。上下文说明也列为 context，不能把多件独立事项塞进同一项或只列第一件。Markdown 的一个标题加其从属说明可以是一项。
- kind 为 write/query/context/unsupported。每个写工具必须关联一个 write 意图的 intent_id；一个意图可关联多个步骤。context 仅用于说明归属，不能承载独立待办。全部 write 意图处理完才能结束。
- 查询完成后用 intent_resolve(result=answered) 记录 query 意图。查证当前状态已符合要求时，可用 result=unchanged 记录无需重复写入的 write 意图；必须给出证据说明。跨模式不能处理的要求用 ask_clarify 说明如何切换，不要静默跳过。
- task_query/project_query/recur_query 返回的当前对象才可用于写入。历史内容和历史编号仅供指代理解，不能直接当作写入依据。项目查询中 nodes 包含上下文，matchedIds 才是直接命中项。
- 每个 Markdown 列表项、标题层次、日期、提醒、备注和条件都要保留。title 使用纯文本；详细说明、清单、链接放入 note 并保留 Markdown。连续动作中的对象和目的不能随意拆开，例如“联系崔所长进行调度对接”是一个节点。
- 有遗漏或匹配歧义时先补齐工具调用或 ask_clarify。全部写入会统一提交；不要在工具成功前声称已经执行。回复使用名称和项目路径，不向用户报内部 id 或 $ 引用。
"#;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(super) struct IntentChecklist {
    items: Vec<IntentItem>,
    /// 与计划中的每一步一一对应，值为意图 id。
    operation_intents: Vec<String>,
    #[serde(default)]
    write_attempted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct IntentItem {
    id: String,
    excerpt: String,
    kind: String,
    #[serde(default)]
    handled: bool,
    #[serde(default)]
    resolution: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(super) struct ReadScope {
    tasks: HashSet<i64>,
    nodes: HashSet<i64>,
    rules: HashSet<i64>,
    any_query: bool,
}

pub(super) fn is_write(name: &str) -> bool {
    matches!(
        name,
        "task_add"
            | "task_update"
            | "task_delete"
            | "task_set_done"
            | "project_add"
            | "project_set_status"
            | "node_add"
            | "node_update"
            | "node_move"
            | "node_delete"
            | "node_set_done"
            | "node_schedule_today"
            | "recur_add"
            | "recur_update"
            | "recur_delete"
            | "undo_last"
    )
}

pub(super) fn specs(mut specs: Vec<ToolSpec>) -> Vec<ToolSpec> {
    for spec in &mut specs {
        if is_write(&spec.name) {
            spec.parameters["properties"]["intent_id"] =
                json!({"type":"string","description":"intent_checklist 中对应 write 意图的 id"});
            spec.parameters["required"]
                .as_array_mut()
                .unwrap()
                .push(json!("intent_id"));
        }
    }
    specs.push(super::tools::spec("intent_checklist", "写入前一次列全当前输入的全部意图和原文片段，程序验证覆盖后才可写入。", json!({
        "items":{"type":"array","items":{"type":"object","properties":{
            "id":{"type":"string"},"excerpt":{"type":"string","description":"当前输入中的连续原文片段，逐字保留"},
            "kind":{"type":"string","enum":["write","query","context","unsupported"]}
        },"required":["id","excerpt","kind"],"additionalProperties":false}}
    }), &["items"]));
    specs.push(super::tools::spec("intent_resolve", "记录查询已经回答，或查证请求状态已经符合要求而无需重复修改。不能跳过待办。", json!({
        "intent_id":{"type":"string"},"result":{"type":"string","enum":["answered","unchanged"]},
        "evidence":{"type":"string","description":"当前查询结果中支持此结论的名称和状态"}
    }), &["intent_id","result","evidence"]));
    specs
}

fn significant(c: char) -> bool {
    !c.is_whitespace() && !"#*-_~`>+[]().,;:!?，。；：！？、（）【】「」\"'".contains(c)
}

impl IntentChecklist {
    pub(super) fn declare(
        &mut self,
        args: Option<&Value>,
        source: &str,
        plan_len: usize,
    ) -> Result<String, String> {
        let mut items: Vec<IntentItem> = serde_json::from_value(
            args.and_then(|v| v.get("items"))
                .cloned()
                .unwrap_or(Value::Null),
        )
        .map_err(|_| "items 需要包含 id、excerpt、kind 的数组".to_string())?;
        if items.is_empty() || items.len() > 80 {
            return Err("请列出 1 到 80 项独立意图".into());
        }
        let mut ids = HashSet::new();
        for item in &mut items {
            if item.id.trim().is_empty()
                || item.id.chars().count() > 40
                || !ids.insert(item.id.clone())
            {
                return Err("每项意图必须有唯一且简短的 id".into());
            }
            if !matches!(
                item.kind.as_str(),
                "write" | "query" | "context" | "unsupported"
            ) {
                return Err("意图类型须为 write/query/context/unsupported".into());
            }
            if item.excerpt.trim().is_empty() || !source.contains(&item.excerpt) {
                return Err(format!(
                    "意图 {} 的 excerpt 必须逐字引用本轮输入，不能改写或引用历史",
                    item.id
                ));
            }
            // 模型不能在声明阶段自行声称已经处理。
            item.handled = matches!(item.kind.as_str(), "context" | "unsupported");
            item.resolution = None;
            if let Some(previous) = self.items.iter().find(|previous| {
                previous.id == item.id
                    && previous.excerpt == item.excerpt
                    && previous.kind == item.kind
            }) {
                item.handled = previous.handled;
                item.resolution = previous.resolution.clone();
            }
        }
        for owner in &self.operation_intents {
            let old = self.items.iter().find(|item| item.id == *owner).unwrap();
            if !items.iter().any(|item| {
                item.id == old.id && item.excerpt == old.excerpt && item.kind == old.kind
            }) {
                return Err("已有操作关联的意图不能删除或改写；补充回答只能追加新意图，必要时通过修改工具调整本轮新建项。".into());
            }
        }
        let missing = missing_source(&items, source);
        if !missing.is_empty() {
            return Err(format!(
                "意图清单遗漏原文内容：{missing}。请一次重新列全，包括日期、备注和条件。"
            ));
        }
        self.items = items;
        if plan_len == 0 {
            self.operation_intents.clear();
        }
        Ok("意图清单已登记。按项查询和处理，未处理完不会提交任何改动。".into())
    }

    pub(super) fn prepare(
        &mut self,
        call: &ToolCall,
        reads: &ReadScope,
        source: &str,
    ) -> Result<(ToolCall, String), String> {
        self.write_attempted = true;
        let missing = missing_source(&self.items, source);
        if !missing.is_empty() {
            return Err(format!(
                "当前原文或追问回答尚未完整列入清单：{missing}。请补充 intent_checklist，再处理整轮要求。"
            ));
        }
        let args = call
            .arguments
            .as_ref()
            .and_then(Value::as_object)
            .ok_or("工具参数需要 JSON 对象")?;
        let id = args
            .get("intent_id")
            .and_then(Value::as_str)
            .ok_or("写入前先 intent_checklist 列全意图，每个写工具必须带 intent_id")?;
        if !self.items.iter().any(|i| i.id == id && i.kind == "write") {
            return Err("intent_id 不是已登记的 write 意图；请先列全意图清单".into());
        }
        reads.validate(call)?;
        let mut stripped = call.clone();
        stripped
            .arguments
            .as_mut()
            .unwrap()
            .as_object_mut()
            .unwrap()
            .remove("intent_id");
        Ok((stripped, id.to_string()))
    }

    pub(super) fn record(&mut self, id: &str, before: usize, after: usize) {
        if after > before {
            self.operation_intents
                .extend((before..after).map(|_| id.to_string()));
            if let Some(item) = self.items.iter_mut().find(|i| i.id == id) {
                item.handled = true;
                item.resolution = None;
            }
        }
    }

    pub(super) fn resolve(
        &mut self,
        args: Option<&Value>,
        reads: &ReadScope,
    ) -> Result<String, String> {
        let args = args.ok_or("缺少参数")?;
        let id = args
            .get("intent_id")
            .and_then(Value::as_str)
            .ok_or("缺少 intent_id")?;
        let result = args
            .get("result")
            .and_then(Value::as_str)
            .ok_or("缺少 result")?;
        let evidence = args
            .get("evidence")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .ok_or("需要当前查询结果作为 evidence")?;
        if !reads.any_query {
            return Err("请先查询当前数据库，再说明查询结论或无需修改的原因".into());
        }
        let item = self
            .items
            .iter_mut()
            .find(|i| i.id == id)
            .ok_or("意图不存在")?;
        if !matches!(
            (item.kind.as_str(), result),
            ("query", "answered") | ("write", "unchanged")
        ) {
            return Err(
                "query 只能标记 answered；write 只有经查证无需变更才能标记 unchanged".into(),
            );
        }
        item.handled = true;
        item.resolution = Some(evidence.to_string());
        Ok("已记录该项结论。".into())
    }

    pub(super) fn incomplete(&self, plan_len: usize, source: &str) -> Option<String> {
        if self.items.is_empty() {
            return self.write_attempted.then(|| "写工具未关联完整意图清单，没有执行任何改动。请先 intent_checklist，或 ask_clarify 向用户补充说明。".into());
        }
        let missing_source = missing_source(&self.items, source);
        if !missing_source.is_empty() {
            return Some(format!(
                "原文或追问回答遗漏内容：{missing_source}。请补充完整意图清单，不能忽略后提交。"
            ));
        }
        let missing: Vec<_> = self
            .items
            .iter()
            .filter(|i| !i.handled)
            .map(|i| format!("{}：「{}」", i.id, i.excerpt))
            .collect();
        if !missing.is_empty() {
            return Some(format!(
                "仍有未处理意图：{}。不得提交部分结果；请继续工具调用，或 ask_clarify。",
                missing.join("；")
            ));
        }
        if self.operation_intents.len() != plan_len {
            return Some("计划中的操作与意图对应关系不完整，不能提交。".into());
        }
        // unsupported 只可在无写入的解释中结束，不能静默遗漏后提交剩余内容。
        if plan_len > 0 && self.items.iter().any(|i| i.kind == "unsupported") {
            return Some("本轮含有尚不能处理的独立要求，请用 ask_clarify 说明后再决定整轮操作，不提交部分结果。".into());
        }
        None
    }

    /// 混合查询与修改时，保留已经通过当前查询给出的只读结论。
    pub(super) fn query_answers(&self) -> String {
        self.items
            .iter()
            .filter(|item| item.kind == "query" && item.handled)
            .filter_map(|item| item.resolution.as_deref())
            .map(|answer| format!("- {answer}"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn missing_source(items: &[IntentItem], source: &str) -> String {
    let mut covered = vec![false; source.len()];
    for item in items {
        for (start, fragment) in source.match_indices(&item.excerpt) {
            covered[start..start + fragment.len()].fill(true);
        }
    }
    source
        .char_indices()
        .filter(|(i, c)| significant(*c) && !covered[*i])
        .map(|(_, c)| c)
        .take(100)
        .collect()
}

impl ReadScope {
    pub(super) fn observe(&mut self, name: &str, content: &str) {
        if !matches!(
            name,
            "task_query" | "project_query" | "recur_query" | "summarize" | "report_get"
        ) {
            return;
        }
        self.any_query = true;
        let value: Value = serde_json::from_str(content).unwrap_or(Value::Null);
        let rows = if name == "project_query" {
            value.get("nodes")
        } else {
            Some(&value)
        };
        let target = match name {
            "task_query" => &mut self.tasks,
            "project_query" => &mut self.nodes,
            "recur_query" => &mut self.rules,
            _ => return,
        };
        if let Some(rows) = rows.and_then(Value::as_array) {
            target.extend(
                rows.iter()
                    .filter_map(|row| row.get("id").and_then(Value::as_i64)),
            );
        }
    }

    fn validate(&self, call: &ToolCall) -> Result<(), String> {
        let args = call.arguments.as_ref().unwrap();
        let (scope, keys, query): (&HashSet<i64>, &[&str], &str) = match call.name.as_str() {
            "task_update" | "task_delete" | "task_set_done" => (&self.tasks, &["id"], "task_query"),
            "node_add" => (&self.nodes, &["parent_id"], "project_query"),
            "node_move" => (&self.nodes, &["id", "parent_id"], "project_query"),
            "node_update"
            | "node_delete"
            | "node_set_done"
            | "project_set_status"
            | "node_schedule_today" => (&self.nodes, &["id"], "project_query"),
            "recur_update" | "recur_delete" => (&self.rules, &["id"], "recur_query"),
            _ => return Ok(()),
        };
        for key in keys {
            if let Some(value) = args.get(*key) {
                if value.as_str().is_some_and(|v| v.starts_with('$')) {
                    continue;
                }
                let id = value.as_i64().or_else(|| {
                    value
                        .as_str()
                        .and_then(|s| s.trim().trim_start_matches('#').parse().ok())
                });
                if !id.is_some_and(|id| scope.contains(&id)) {
                    return Err(format!(
                        "{key} 尚未由本轮 {query} 查证，请重新查询当前数据，不能使用历史编号或猜测编号。"
                    ));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 原文漏项和未处理意图阻止整轮提交() {
        let mut check = IntentChecklist::default();
        let bad = json!({"items":[{"id":"a","excerpt":"买菜","kind":"write"}]});
        assert!(check.declare(Some(&bad), "- 买菜\n- 交周报", 0).is_err());
        let all = json!({"items":[{"id":"a","excerpt":"买菜","kind":"write"},{"id":"b","excerpt":"交周报","kind":"write"}]});
        check.declare(Some(&all), "- 买菜\n- 交周报", 0).unwrap();
        check.record("a", 0, 1);
        assert!(
            check
                .incomplete(1, "- 买菜\n- 交周报")
                .unwrap()
                .contains("交周报")
        );
        check.record("b", 1, 2);
        assert!(check.incomplete(2, "- 买菜\n- 交周报").is_none());
        assert!(
            check
                .incomplete(2, "- 买菜\n- 交周报\n再提醒我开会")
                .unwrap()
                .contains("再提醒我开会")
        );
    }

    #[test]
    fn 历史编号不能替代当前查询() {
        let call = ToolCall {
            id: "c".into(),
            name: "node_move".into(),
            arguments: Some(json!({"id":"12","parent_id":"20"})),
        };
        let mut reads = ReadScope::default();
        assert!(reads.validate(&call).is_err());
        reads.observe("project_query", r#"{"nodes":[{"id":12},{"id":20}]}"#);
        assert!(reads.validate(&call).is_ok());
    }
}
