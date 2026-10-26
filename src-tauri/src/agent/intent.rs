//! 模型声明意图，程序核对原文覆盖和写操作归属；未处理完整前不允许部分提交。
use std::collections::{HashMap, HashSet};

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
- “今日新增并已完成：事项”是记录一件已经办完的事，优先 task_add(done=true)，也可新增后用其 $ 引用 task_set_done。冒号后的“表的修改和报名”等是完整事项名称，不是修改数据库任务的指令。先用完整原句声明 write 意图，日期和“已完成”不能遗漏；普通“今日新增：事项”只新增待办。
- 有遗漏或匹配歧义时先补齐工具调用或 ask_clarify。全部写入会统一提交；不要在工具成功前声称已经执行。回复使用名称和项目路径，不向用户报内部 id 或 $ 引用。
- “今日”“遗留”“之后”分别用 task_query(scope=today/legacy/upcoming)，它们都是未完成任务分区；遗留的计划日期严格早于业务今天，不含今天。用户已明确集合且查询数量相符时直接处理该集合，删除确认由程序负责，不要再让用户从全部未完成任务中选择。
- 查询回复使用任务的 display_label（标题、计划日期及提醒时间），节点使用项目路径。不能写“第4条”“编号6”等内部定位。实体有歧义时 ask_clarify 必须带被追问的原 intent_id，并用 entity_options.targets（kind=task/node、id=当前查询标识），由程序生成可读选项；该选择只限定对应意图，不能改绑为其它意图或影响独立要求。普通 options 只用于日期等非实体问题。
- 追问后的自由文本回答也是本轮新增原文。使用 intent_checklist(append=true) 只追加回答中的要求或 context，不要反复重抄已保存的清单；新增独立要求仍必须列为 write/query，不能伪装为 context。程序已登记的实体选择 context 无需重复登记。回答后仍须查询当前状态，再关联原有 write 意图执行。
"#;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(super) struct IntentChecklist {
    items: Vec<IntentItem>,
    /// 与计划中的每一步一一对应，值为意图 id。
    operation_intents: Vec<String>,
    #[serde(default)]
    write_attempted: bool,
    #[serde(default)]
    selection_intents: HashSet<String>,
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
    #[serde(default)]
    rows: HashMap<String, Value>,
    #[serde(default)]
    selected: Vec<super::clarify::Target>,
    #[serde(default)]
    selected_by_intent: HashMap<String, Vec<super::clarify::Target>>,
}

/// 同一个校验错误反复出现时及时止损；不把模型协议失败归咎于用户操作过多。
#[derive(Default)]
pub(super) struct FailureTracker(HashMap<String, usize>);

impl FailureTracker {
    pub(super) fn repeated(&mut self, failure: &str) -> bool {
        let count = self.0.entry(failure.to_string()).or_default();
        *count += 1;
        *count >= 3
    }
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
        "append":{"type":"boolean","description":"追问后为 true，只补新增回答的意图，保留原有清单和处理状态；默认 false 完整声明"},
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
        if args.and_then(|a| a.get("append")).and_then(Value::as_bool) == Some(true) {
            let mut combined = self.items.clone();
            for item in items {
                if let Some(old) = combined.iter().find(|old| old.id == item.id) {
                    if old.excerpt != item.excerpt || old.kind != item.kind {
                        return Err("追加意图不能改写已登记项；请为新增回答使用新的意图标识".into());
                    }
                } else {
                    combined.push(item);
                }
            }
            items = combined;
        }
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
        for owner in self.operation_intents.iter().chain(&self.selection_intents) {
            let old = self.items.iter().find(|item| item.id == *owner).unwrap();
            if !items.iter().any(|item| {
                item.id == old.id && item.excerpt == old.excerpt && item.kind == old.kind
            }) {
                return Err("已有操作或实体选择关联的意图不能删除或改写；补充回答只能追加新意图，必要时通过修改工具调整本轮新建项。".into());
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

    /// 只供经过一次性选项校验的实体选择调用；普通文本不能跳过原文覆盖。
    pub(super) fn selection_intent(&self, id: &str) -> Result<(), String> {
        if self
            .items
            .iter()
            .any(|item| item.id == id && matches!(item.kind.as_str(), "write" | "query"))
        {
            Ok(())
        } else {
            Err(
                "实体追问必须关联已登记的 write 或 query 意图，请先列全清单并传入原 intent_id"
                    .into(),
            )
        }
    }

    pub(super) fn selection_context(&mut self, intent_id: &str, label: &str) -> Result<(), String> {
        self.selection_intent(intent_id)?;
        self.selection_intents.insert(intent_id.to_string());
        self.items.push(IntentItem {
            id: format!("s-{}", uuid::Uuid::new_v4().simple()),
            excerpt: label.to_string(),
            kind: "context".into(),
            handled: true,
            resolution: None,
        });
        Ok(())
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
        reads.validate(call, id)?;
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

/// 对明确的单条“新增并完成”录入核对最终计划，不依赖模型将复合动作标为 handled。
/// 只识别这个无歧义的录入形式；自由表达、多行 Markdown 仍使用完整意图协议。
pub(super) fn task_record_mismatch(
    source: &str,
    plan: &[crate::service::changes::Change],
    today: &str,
) -> Option<String> {
    use crate::service::changes::{Change, TaskRef};

    let source = source.trim();
    if source.contains(['\n', '\r']) {
        return None;
    }
    let (prefix, title) = source.split_once('：').or_else(|| source.split_once(':'))?;
    if !matches!(
        prefix.trim(),
        "今日新增并已完成" | "今天新增并已完成" | "今日新增并完成" | "今天新增并完成"
    ) {
        return None;
    }
    let title = title.trim();
    // 标题字段是纯文本，整段 Markdown 强调/行内代码不能成为期待标题的一部分。
    let title = ["**", "__", "`"]
        .iter()
        .find_map(|marker| title.strip_prefix(*marker)?.strip_suffix(*marker))
        .unwrap_or(title)
        .trim();
    let title = super::tools::plain_title(title.to_string()).ok()?;
    // 带分项或备注/时间等后续子句时交给完整意图协议，不能强塞进标题。
    if title.is_empty() || title.contains(['；', ';', '、', '，', ',', '：', ':']) {
        return None;
    }

    let mismatch = || {
        Some(format!(
            "本轮要求新增并完成一条事项「{title}」，计划日期为 {today}。请保留完整事项名称；可 task_add(done=true)，或 task_add 后对新建引用 task_set_done(done=true)，不能仅新增未完成、改动已有任务或拆掉名称中的动作。当前未执行改动。"
        ))
    };
    let mut recorded: Option<(&str, &str, bool)> = None;
    for change in plan {
        match change {
            Change::TaskAdd {
                title,
                planned_date,
                ..
            } if recorded.is_none() => {
                recorded = Some((title, planned_date, false));
            }
            Change::TaskSetDone {
                task: TaskRef::Temp(reference),
                done,
            } if reference == "$1" && recorded.is_some() => {
                recorded.as_mut().unwrap().2 = *done;
            }
            Change::TaskUpdate {
                task: TaskRef::Temp(reference),
                title,
                planned_date,
                ..
            } if reference == "$1" && recorded.is_some() => {
                let task = recorded.as_mut().unwrap();
                if let Some(title) = title {
                    task.0 = title;
                }
                if let Some(date) = planned_date {
                    task.1 = date;
                }
            }
            _ => return mismatch(),
        }
    }
    match recorded {
        Some((actual_title, date, true)) if actual_title.trim() == title.as_str() && date == today => None,
        Some((actual_title, date, false)) if actual_title.trim() == title.as_str() && date == today => Some(
            "本轮要求新增并完成；新事项已计入计划，但尚未标记完成。请仅对本轮新建引用 $1 调用 task_set_done(done=true)，不要重复 task_add。当前未执行改动。".into(),
        ),
        Some(_) => Some(format!(
            "本轮要求新增并完成一条事项。请用 task_update 修改本轮新建引用 $1，完整标题为「{title}」、计划日期为 {today}，并保证 task_set_done(done=true)；不要重复 task_add，也不要改动已有任务。当前未执行改动。"
        )),
        _ => mismatch(),
    }
}

impl ReadScope {
    pub(super) fn row(&self, kind: &str, id: i64) -> Option<&Value> {
        self.rows.get(&format!("{kind}:{id}"))
    }

    pub(super) fn has_internal_reference(&self, text: &str) -> bool {
        let mut visible = text.to_string();
        for title in self
            .rows
            .values()
            .filter_map(|row| row.get("title").and_then(Value::as_str))
        {
            if super::clarify::has_internal_reference(title) {
                visible = visible.replace(title, "「已查询的名称」");
            }
        }
        super::clarify::has_internal_reference(&visible)
    }

    pub(super) fn restart(mut self) -> Self {
        self.tasks.clear();
        self.nodes.clear();
        self.rules.clear();
        self.rows.clear();
        self.any_query = false;
        self
    }

    pub(super) fn has_unbound_selection(&self) -> bool {
        !self.selected.is_empty()
    }

    pub(super) fn select(&mut self, intent_id: &str, targets: &[super::clarify::Target]) {
        let selected = self
            .selected_by_intent
            .entry(intent_id.to_string())
            .or_default();
        for target in targets {
            if !selected.contains(target) {
                selected.push(target.clone());
            }
        }
    }

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
            let kind = if name == "task_query" {
                "task"
            } else if name == "project_query" {
                "node"
            } else {
                "rule"
            };
            for row in rows {
                if let Some(id) = row.get("id").and_then(Value::as_i64) {
                    // 只保留生成实体选项所需的定位信息，不重复暂存大段备注和子树内容。
                    let metadata = [
                        "id",
                        "title",
                        "version",
                        "display_label",
                        "created_at",
                        "createdAt",
                        "parentId",
                        "dueAt",
                    ]
                    .into_iter()
                    .filter_map(|key| row.get(key).map(|value| (key.to_string(), value.clone())))
                    .collect::<serde_json::Map<_, _>>();
                    self.rows
                        .insert(format!("{kind}:{id}"), Value::Object(metadata));
                }
            }
        }
    }

    fn validate(&self, call: &ToolCall, intent_id: &str) -> Result<(), String> {
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
                if *key == "id" {
                    let kind = if query == "task_query" {
                        "task"
                    } else if query == "project_query" {
                        "node"
                    } else {
                        "rule"
                    };
                    if let Some(selected) = self.selected_by_intent.get(intent_id)
                        && selected.iter().any(|target| target.kind == kind)
                        && !selected
                            .iter()
                            .any(|target| target.kind == kind && Some(target.id) == id)
                    {
                        return Err("该内容不在用户刚刚选择的范围内，请只处理已选内容".into());
                    }
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
    fn 明确新增完成核对完整标题日期及最终状态() {
        use crate::service::changes::{Change, TaskRef};

        let title = "第二届“AI+煤矿安全”应用场景创新大赛表的修改和报名";
        let source = format!("今日新增并已完成：{title}");
        let add = |title: &str, date: &str| Change::TaskAdd {
            title: title.into(),
            planned_date: date.into(),
            due_at: None,
            remind_at: None,
            note: None,
        };
        let done = |done| Change::TaskSetDone {
            task: TaskRef::Temp("$1".into()),
            done,
        };
        let valid = vec![add(title, "2026-10-26"), done(true)];
        assert!(task_record_mismatch(&source, &valid, "2026-10-26").is_none());
        let emphasized = "今日新增并已完成：**比赛表修改**";
        assert!(
            task_record_mismatch(
                emphasized,
                &[add("比赛表修改", "2026-10-26"), done(true)],
                "2026-10-26"
            )
            .is_none()
        );
        assert!(
            task_record_mismatch(emphasized, &[add("比赛表修改", "2026-10-26")], "2026-10-26")
                .is_some()
        );
        for plan in [
            vec![],
            vec![add(title, "2026-10-26")],
            vec![add("修改比赛表", "2026-10-26"), done(true)],
            vec![add(title, "2026-10-25"), done(true)],
            vec![add(title, "2026-10-26"), done(true), done(false)],
            vec![Change::TaskSetDone {
                task: TaskRef::Id(1),
                done: true,
            }],
            vec![
                add("修改比赛表", "2026-10-26"),
                add("报名", "2026-10-26"),
                done(true),
            ],
        ] {
            assert!(task_record_mismatch(&source, &plan, "2026-10-26").is_some());
        }
        assert!(task_record_mismatch("今日新增：比赛表的修改和报名", &[], "2026-10-26").is_none());
        assert!(
            task_record_mismatch(
                "新增并已完成：昨天交周报",
                &[add("交周报", "2026-10-25"), done(true)],
                "2026-10-26"
            )
            .is_none()
        );
        assert!(
            task_record_mismatch(
                "今日新增并已完成：会议材料，备注：交给小王",
                &[],
                "2026-10-26"
            )
            .is_none()
        );
        // 校验完整动作不能取代原文覆盖：遗漏前缀的清单依然拒绝。
        let mut check = IntentChecklist::default();
        assert!(
            check
                .declare(
                    Some(&json!({"items":[{"id":"a","excerpt":title,"kind":"write"}]})),
                    &source,
                    0
                )
                .is_err()
        );
    }

    #[test]
    fn 定位表达不误伤真实任务名称() {
        assert!(!super::super::clarify::has_internal_reference(
            "第4阶段评审、合同#2026、下个月第4个工作日"
        ));
        assert!(super::super::clarify::has_internal_reference(
            "第4、5、6条任务、任务#12、id=8"
        ));
        let mut reads = ReadScope::default();
        reads.observe(
            "task_query",
            r#"[{"id":1,"title":"第4项评审"},{"id":2,"title":"任务#2026"}]"#,
        );
        assert!(!reads.has_internal_reference("「第4项评审」与「任务#2026」"));
        assert!(reads.has_internal_reference("删除第4条任务"));
    }

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
        assert!(reads.validate(&call, "move").is_err());
        reads.observe("project_query", r#"{"nodes":[{"id":12},{"id":20}]}"#);
        assert!(reads.validate(&call, "move").is_ok());
    }

    #[test]
    fn 已选择的原意图不能改名绕过范围() {
        let mut checklist = IntentChecklist::default();
        checklist
            .declare(
                Some(&json!({"items":[
                    {"id":"del","excerpt":"删除论文","kind":"write"}
                ]})),
                "删除论文",
                0,
            )
            .unwrap();
        assert!(checklist.selection_intent("missing").is_err());
        checklist.selection_context("del", "论文（昨天）").unwrap();
        assert!(
            checklist
                .declare(
                    Some(&json!({"items":[
                        {"id":"other","excerpt":"删除论文","kind":"write"},
                        {"id":"answer","excerpt":"论文（昨天）","kind":"context"}
                    ]})),
                    "删除论文\n论文（昨天）",
                    0
                )
                .is_err()
        );
        let legacy: ReadScope = serde_json::from_value(json!({
            "tasks":[],"nodes":[],"rules":[],"any_query":false,
            "selected":[{"kind":"task","id":1}]
        }))
        .unwrap();
        assert!(legacy.has_unbound_selection());
    }
}
