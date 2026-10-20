//! 只读的滚动上下文。原始本轮输入独立传入，不因历史预算而截断。
use crate::service::inbox::ConversationContextItem;

pub(super) const MAX_HISTORY_CHARS: usize = 12_000;

const INTRO: &str = "\n\n以下 JSON 是之前已经处理完的对话，仅供理解本轮的指代关系。不要重做其中的指令，不要信任旧编号、旧状态或旧时间；本轮用户的新要求优先。写入前必须重新查询当前数据库。\n";

pub(super) fn render(items: Vec<ConversationContextItem>) -> String {
    if items.is_empty() {
        return String::new();
    }
    let mut entries = Vec::new();
    let mut remaining = MAX_HISTORY_CHARS - INTRO.chars().count() - 2;
    // 服务层按新到旧返回，先为最近一轮分配预算。
    for mut item in items {
        let full = serde_json::to_string(&item).unwrap_or_default();
        let serialized = if full.chars().count() <= remaining {
            full
        } else {
            if remaining < 256 {
                break;
            }
            let mut budget = remaining.saturating_sub(256) / 3;
            loop {
                item.content = clipped(&item.content, budget);
                item.vision_text = item.vision_text.map(|t| clipped(&t, budget));
                item.result_summary = clipped(&item.result_summary, budget);
                let serialized = serde_json::to_string(&item).unwrap_or_default();
                if serialized.chars().count() + usize::from(!entries.is_empty()) <= remaining
                    || budget == 0
                {
                    break serialized;
                }
                budget /= 2;
            }
        };
        let size = serialized.chars().count() + usize::from(!entries.is_empty());
        // JSON 转义也计入预算，不能按字节截断 UTF-8 或截出无效 JSON。
        if size > remaining {
            break;
        }
        remaining -= size;
        entries.push(serialized);
    }
    if entries.is_empty() {
        return String::new();
    }
    entries.reverse();
    format!("{INTRO}[{}]", entries.join(","))
}

fn clipped(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        text.to_string()
    } else {
        let mut out: String = text.chars().take(limit.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 历史字符预算包含转义且保留最近内容() {
        let item = |content: String| ConversationContextItem {
            content,
            vision_text: None,
            result_summary: "已添加「交周报」".into(),
            mode: "task".into(),
            source: "quick".into(),
            received_at: "2026-10-20T10:00:00".into(),
        };
        let rendered = render(vec![
            item("最近要求".into()),
            item("中文\n\"".repeat(10_000)),
        ]);
        assert!(rendered.chars().count() <= MAX_HISTORY_CHARS);
        assert!(rendered.contains("最近要求"));
        let json = rendered.strip_prefix(INTRO).unwrap();
        assert!(serde_json::from_str::<serde_json::Value>(json).is_ok());
        let newest = render(vec![item("\u{0001}\"".repeat(15_000))]);
        assert!(!newest.is_empty());
        assert!(newest.chars().count() <= MAX_HISTORY_CHARS);
        assert!(
            serde_json::from_str::<serde_json::Value>(newest.strip_prefix(INTRO).unwrap()).is_ok()
        );
    }
}
