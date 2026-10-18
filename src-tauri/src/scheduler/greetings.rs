//! 问候语：按业务日、时段和重要变化缓存。失败短冷却，界面读取不发网络请求。
use super::ai::{generate, poisoned};
use crate::config::AppConfig;
use crate::error::AppResult;
use crate::prompts::PromptId;
use crate::service::Service;
use crate::state::AppState;
use chrono::{Days, NaiveDate, NaiveDateTime, Timelike};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const RETRY_SECONDS: i64 = 300;
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GreetingView {
    pub text: String,
    pub nudge: Option<String>,
    pub generated: bool,
}
struct Context {
    date: String,
    slot: &'static str,
    fingerprint: String,
    input: String,
    fallback: GreetingView,
}
fn slot(hour: u32) -> &'static str {
    match hour {
        0..=4 | 23 => "夜深了",
        5..=8 => "早上好",
        9..=11 => "上午好",
        12..=13 => "中午好",
        14..=17 => "下午好",
        _ => "晚上好",
    }
}
fn streak(conn: &Connection, today: NaiveDate) -> AppResult<usize> {
    let mut stmt = conn.prepare("SELECT DISTINCT business_date FROM completions WHERE business_date<=?1 ORDER BY business_date DESC")?;
    let dates = stmt
        .query_map([today.to_string()], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    let mut expected = today;
    let mut count = 0;
    for date in dates {
        if count == 0 && date != expected.to_string() {
            expected = today.checked_sub_days(Days::new(1)).unwrap_or(today);
        }
        if date != expected.to_string() {
            break;
        }
        count += 1;
        let Some(previous) = expected.pred_opt() else {
            break;
        };
        expected = previous;
    }
    Ok(count)
}
fn context(conn: &Connection, state: &AppState, config: &AppConfig) -> AppResult<Context> {
    let svc = Service::new(conn, state.clock.as_ref(), config)?;
    let today = svc.today();
    let board = svc.task_board()?;
    let left = board.today.len() + board.legacy.len();
    let stale = board
        .legacy
        .iter()
        .filter(|task| task.delay_days > 7)
        .max_by_key(|task| task.delay_days);
    let nudge = stale.map(|task| {
        format!(
            "「{}」已经拖了 {} 天，还要做吗？",
            task.title, task.delay_days
        )
    });
    let time_slot = slot(state.clock.now().hour());
    let fallback = GreetingView {
        text: if left == 0 {
            format!("{time_slot}，今天的事都办完了")
        } else {
            format!("{time_slot}，今天还剩 {left} 件事")
        },
        nudge,
        generated: false,
    };
    let now = state
        .clock
        .now()
        .format(crate::service::DATETIME_FMT)
        .to_string();
    let tomorrow_end = today
        .checked_add_days(Days::new(2))
        .unwrap_or(today)
        .to_string();
    let mut stmt = conn.prepare("SELECT title,due_at FROM tasks WHERE status='open' AND due_at>=?1 AND due_at<?2 UNION ALL SELECT title,due_at FROM nodes WHERE status='open' AND due_at>=?1 AND due_at<?2 ORDER BY due_at LIMIT 5")?;
    let due = stmt
        .query_map(params![now, tomorrow_end], |r| {
            Ok(json!({"title":r.get::<_,String>(0)?,"dueAt":r.get::<_,String>(1)?}))
        })?
        .collect::<Result<Vec<Value>, _>>()?;
    let mut stmt =
        conn.prepare("SELECT id,title FROM nodes WHERE status='done' AND depth<=2 ORDER BY id")?;
    let milestones = stmt
        .query_map([], |r| {
            Ok(json!({"id":r.get::<_,i64>(0)?,"title":r.get::<_,String>(1)?}))
        })?
        .collect::<Result<Vec<Value>, _>>()?;
    let input = json!({"businessDate":today.to_string(),"timeSlot":time_slot,"remaining":left,"streakDays":streak(conn,today)?,"upcomingDue":due,"milestones":milestones,"longDelayedTask":stale.map(|task|json!({"id":task.id,"title":task.title,"days":task.delay_days}))}).to_string();
    let fingerprint = format!("{:x}", Sha256::digest(input.as_bytes()));
    Ok(Context {
        date: today.to_string(),
        slot: time_slot,
        fingerprint,
        input,
        fallback,
    })
}
fn cached(conn: &Connection, ctx: &Context, now: NaiveDateTime) -> AppResult<Option<GreetingView>> {
    let record: Option<(String,String,String)> = conn.query_row(
        "SELECT text,reason,created_at FROM greetings WHERE business_date=?1 AND slot=?2 AND (reason=?3 OR reason=?4) ORDER BY id DESC LIMIT 1",
        params![ctx.date,ctx.slot,format!("model:{}",ctx.fingerprint),format!("fallback:{}",ctx.fingerprint)],
        |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
    let Some((text, reason, created)) = record else {
        return Ok(None);
    };
    let generated = reason.starts_with("model:");
    let fresh = generated
        || NaiveDateTime::parse_from_str(&created, crate::service::DATETIME_FMT)
            .is_ok_and(|at| (now - at).num_seconds() < RETRY_SECONDS);
    Ok(fresh.then(|| GreetingView {
        text,
        nudge: ctx.fallback.nudge.clone(),
        generated,
    }))
}
/// 前端命令调用：同步读取当前指纹对应的缓存，不发模型请求。调用方持maintenance读锁。
pub fn current(state: &AppState) -> AppResult<GreetingView> {
    let config = state.config.lock().map_err(poisoned)?.clone();
    let conn = state.db.lock().map_err(poisoned)?;
    let ctx = context(&conn, state, &config)?;
    Ok(cached(&conn, &ctx, state.clock.now())?.unwrap_or(ctx.fallback))
}
/// 后台串行worker调用。网络等待不持有db/config Mutex。
pub async fn get(state: &AppState) -> AppResult<GreetingView> {
    let _maintenance = state.maintenance.read().await;
    let config = state.config.lock().map_err(poisoned)?.clone();
    let ctx = {
        let conn = state.db.lock().map_err(poisoned)?;
        let ctx = context(&conn, state, &config)?;
        if let Some(view) = cached(&conn, &ctx, state.clock.now())? {
            return Ok(view);
        }
        ctx
    };
    // 指纹变化时也遵守失败冷却，避免离线期间每次数据变化都请求。
    let cooling = {
        let conn = state.db.lock().map_err(poisoned)?;
        let latest:Option<String>=conn.query_row("SELECT created_at FROM greetings WHERE reason LIKE 'fallback:%' ORDER BY id DESC LIMIT 1",[],|r|r.get(0)).optional()?;
        latest
            .and_then(|s| NaiveDateTime::parse_from_str(&s, crate::service::DATETIME_FMT).ok())
            .is_some_and(|at| (state.clock.now() - at).num_seconds() < RETRY_SECONDS)
    };
    if cooling {
        return Ok(ctx.fallback);
    }
    let response = generate(state, &config, PromptId::Greeting, ctx.input.clone()).await;
    let generated = response.is_ok();
    let text = response
        .ok()
        .filter(|s| s.lines().count() <= 2 && s.chars().count() <= 120);
    let generated = generated && text.is_some();
    let view = GreetingView {
        text: text.unwrap_or_else(|| ctx.fallback.text.clone()),
        nudge: ctx.fallback.nudge.clone(),
        generated,
    };
    let now = state
        .clock
        .now()
        .format(crate::service::DATETIME_FMT)
        .to_string();
    {
        let conn = state.db.lock().map_err(poisoned)?;
        // 等待模型期间有任务变化时不把旧快照结果交给界面，下一次tick重新生成。
        let latest = context(&conn, state, &config)?;
        if latest.fingerprint != ctx.fingerprint {
            return Ok(latest.fallback);
        }
        conn.execute("INSERT INTO greetings(business_date,slot,reason,text,created_at) VALUES(?1,?2,?3,?4,?5)",params![ctx.date,ctx.slot,format!("{}:{}",if generated {"model"}else{"fallback"},ctx.fingerprint),view.text,now])?;
    }
    Ok(view)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn 六个时段边界() {
        assert_eq!(slot(4), "夜深了");
        assert_eq!(slot(5), "早上好");
        assert_eq!(slot(9), "上午好");
        assert_eq!(slot(12), "中午好");
        assert_eq!(slot(14), "下午好");
        assert_eq!(slot(18), "晚上好");
        assert_eq!(slot(23), "夜深了");
    }
}
