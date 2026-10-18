use super::{Ctx, tools::Outcome};
use crate::llm::ToolCall;
use crate::scheduler::reports;

pub fn handle(ctx: &Ctx<'_>, call: &ToolCall) -> Outcome {
    let result = (|| -> Result<String, String> {
        let args = call
            .arguments
            .as_ref()
            .and_then(|a| a.as_object())
            .ok_or("参数需要 JSON 对象")?;
        if args
            .keys()
            .any(|k| !matches!(k.as_str(), "id" | "kind" | "period"))
        {
            return Err("报告参数只支持 id、kind、period".into());
        }
        let kind = args
            .get("kind")
            .map(|v| v.as_str().ok_or("kind 需要字符串"))
            .transpose()?;
        if kind.is_some_and(|v| !matches!(v, "weekly" | "monthly" | "yearly")) {
            return Err("报告类型不正确".into());
        }
        let period = args
            .get("period")
            .map(|v| v.as_str().ok_or("period 需要字符串"))
            .transpose()?;
        let id = args
            .get("id")
            .map(|v| v.as_i64().filter(|n| *n > 0).ok_or("id 需要有效整数"))
            .transpose()?;
        let conn = ctx.db.lock().map_err(|_| "数据库暂不可用")?;
        let entries = reports::list(&conn).map_err(|e| e.to_string())?;
        let entry = entries.iter().find(|r| {
            id.is_none_or(|v| r.id == v)
                && kind.is_none_or(|v| r.kind == v)
                && period.is_none_or(|v| r.period == v)
        });
        let Some(entry) = entry else {
            return Ok(
                "还没有找到对应报告；已配置模型后，后台会生成到期的周报、月报和年报。".into(),
            );
        };
        let root = ctx
            .prompts_dir
            .parent()
            .ok_or("数据目录不存在")?
            .join("reports");
        let doc = reports::read_dir(&conn, &root, entry.id).map_err(|e| e.to_string())?;
        serde_json::to_string(&doc).map_err(|e| e.to_string())
    })();
    match result {
        Ok(text) => Outcome::Ok(text),
        Err(e) => Outcome::Error(e),
    }
}
