//! 问候语：按业务日、时段和重要变化缓存。失败短冷却，界面读取不发网络请求。
use super::ai::{generate, poisoned};
use crate::config::AppConfig;
use crate::error::AppResult;
use crate::prompts::PromptId;
use crate::service::Service;
use crate::state::AppState;
use chrono::{Days, NaiveDate, NaiveDateTime, Timelike};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const RETRY_SECONDS: i64 = 300;
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GreetingView {
    pub text: String,
    pub nudge: Option<String>,
    pub generated: bool,
    pub task_placeholder: String,
    pub project_placeholder: String,
    pub task_empty: String,
    pub project_empty: String,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UiCopyView {
    pub greeting: String,
    pub nudge: Option<String>,
    pub task_placeholder: String,
    pub project_placeholder: String,
    pub task_empty: String,
    pub project_empty: String,
    pub generated: bool,
}
impl From<&GreetingView> for UiCopyView {
    fn from(view: &GreetingView) -> Self {
        Self {
            greeting: view.text.clone(),
            nudge: view.nudge.clone(),
            task_placeholder: view.task_placeholder.clone(),
            project_placeholder: view.project_placeholder.clone(),
            task_empty: view.task_empty.clone(),
            project_empty: view.project_empty.clone(),
            generated: view.generated,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GeneratedCopy {
    greeting: String,
    task_placeholder: String,
    project_placeholder: String,
    task_empty: String,
    project_empty: String,
}
fn parse_copy(text: &str, fallback: &GreetingView) -> Option<GreetingView> {
    let raw = text
        .trim()
        .strip_prefix("```json")
        .and_then(|s| s.strip_suffix("```"))
        .unwrap_or(text)
        .trim();
    let copy: GeneratedCopy = serde_json::from_str(raw).ok()?;
    let fields = [
        (&copy.greeting, 60),
        (&copy.task_placeholder, 80),
        (&copy.project_placeholder, 80),
        (&copy.task_empty, 60),
        (&copy.project_empty, 60),
    ];
    if fields.iter().any(|(s, max)| {
        s.trim().is_empty()
            || s.chars().count() > *max
            || s.contains(['\n', '\r'])
            || s.contains("http://")
            || s.contains("https://")
    }) {
        return None;
    }
    // 问候只作状态回顾；催办使用基于当前未完成任务生成的固定副标题。
    // 不让模型将已完成的里程碑误写成“别忘了/记得完成”的提醒。
    if [
        "别忘",
        "不要忘",
        "记得",
        "记着",
        "记住要",
        "别漏",
        "不要漏",
        "抓紧",
        "赶紧",
    ]
    .iter()
    .any(|phrase| copy.greeting.contains(phrase))
    {
        return None;
    }
    Some(GreetingView {
        text: copy.greeting.trim().into(),
        nudge: fallback.nudge.clone(),
        generated: true,
        task_placeholder: copy.task_placeholder.trim().into(),
        project_placeholder: copy.project_placeholder.trim().into(),
        task_empty: copy.task_empty.trim().into(),
        project_empty: copy.project_empty.trim().into(),
    })
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
fn project_milestones(conn: &Connection) -> AppResult<Vec<Value>> {
    let mut stmt = conn.prepare(
        "WITH RECURSIVE tree(project_id,id,status,done_at) AS (
           SELECT id,id,status,done_at FROM nodes WHERE parent_id IS NULL AND project_status!='archived'
           UNION ALL
           SELECT t.project_id,n.id,n.status,n.done_at FROM nodes n JOIN tree t ON n.parent_id=t.id
         )
         SELECT p.title,p.status,MAX(t.done_at) AS latest FROM tree t JOIN nodes p ON p.id=t.project_id
         WHERE t.status='done' GROUP BY p.id ORDER BY latest DESC,p.id DESC LIMIT 5",
    )?;
    Ok(stmt
        .query_map([], |r| {
            let title: String = r.get(0)?;
            let status: String = r.get(1)?;
            let progress = if status == "done" {
                "已完成"
            } else {
                "阶段性完成"
            };
            Ok(json!({"project":title,"progress":progress,"summary":format!("{title}{progress}")}))
        })?
        .collect::<Result<Vec<_>, _>>()?)
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
        task_placeholder: "说点什么，比如：明天下午三点提醒我交周报".into(),
        project_placeholder: "比如：给「知了」加一个阶段：打包发布".into(),
        task_empty: "今天还没有任务，说一句就能记下。".into(),
        project_empty: "还没有项目，说一句就能开始。".into(),
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
    let milestones = project_milestones(conn)?;
    let mut stmt = conn.prepare("SELECT title FROM nodes WHERE depth=1 AND project_status='active' ORDER BY id DESC LIMIT 5")?;
    let projects = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    let input = json!({"businessDate":today.to_string(),"timeSlot":time_slot,"remaining":left,"streakDays":streak(conn,today)?,"upcomingDue":due,"completedMilestones":milestones,"milestonePolicy":"completedMilestones 是已发生的完成进展，只可回顾肯定，不是待办，不可提醒再做。里程碑只使用项目名称和 progress 表达，不引用具体子任务。项目未勾选完成时只能称为阶段性完成，即使全部子节点已经完成。greeting 不使用别忘了、记得、抓紧等催办表达，未完成任务的提醒另有确定性副标题。","activeProjects":projects,"longDelayedTask":stale.map(|task|json!({"title":task.title,"days":task.delay_days}))}).to_string();
    let mut digest = Sha256::new();
    digest.update(input.as_bytes());
    digest.update(crate::prompts::load(&state.paths.prompts(), PromptId::Greeting).as_bytes());
    digest.update(serde_json::to_vec(&config.model.chat)?);
    let fingerprint = format!("ui-v2:{:x}", digest.finalize());
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
    Ok(if fresh {
        serde_json::from_str::<GreetingView>(&text)
            .ok()
            .map(|mut view| {
                view.nudge = ctx.fallback.nudge.clone();
                view.generated = generated;
                view
            })
    } else {
        None
    })
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
    let view = response
        .ok()
        .and_then(|text| parse_copy(&text, &ctx.fallback))
        .unwrap_or_else(|| ctx.fallback.clone());
    let generated = view.generated;
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
        conn.execute("INSERT INTO greetings(business_date,slot,reason,text,created_at) VALUES(?1,?2,?3,?4,?5)",params![ctx.date,ctx.slot,format!("{}:{}",if generated {"model"}else{"fallback"},ctx.fingerprint),serde_json::to_string(&view)?,now])?;
    }
    Ok(view)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn 里程碑按项目勾选状态汇总且不暴露子任务() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE nodes(id INTEGER PRIMARY KEY,parent_id INTEGER,title TEXT,status TEXT,project_status TEXT,done_at TEXT);
          INSERT INTO nodes VALUES(1,NULL,'机械光瓦项目','open','active',NULL),(2,1,'替换螺丝','done',NULL,'2026-10-20T09:00:00'),(3,2,'检查尺寸','done',NULL,'2026-10-20T10:00:00');").unwrap();
        let milestones = project_milestones(&conn).unwrap();
        assert_eq!(
            milestones,
            vec![
                json!({"project":"机械光瓦项目","progress":"阶段性完成","summary":"机械光瓦项目阶段性完成"})
            ]
        );
        conn.execute("UPDATE nodes SET status='done',project_status='done',done_at='2026-10-20T11:00:00' WHERE id=1", []).unwrap();
        assert_eq!(project_milestones(&conn).unwrap()[0]["progress"], "已完成");
    }
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

    #[test]
    fn 文案包必须完整且单行_不替换固定延期提示() {
        let fallback = GreetingView {
            text: "上午好".into(),
            nudge: Some("延期提示".into()),
            generated: false,
            task_placeholder: "任务示例".into(),
            project_placeholder: "项目示例".into(),
            task_empty: "没有任务".into(),
            project_empty: "没有项目".into(),
        };
        let valid = r#"{"greeting":"上午好，慢慢来","taskPlaceholder":"比如：明天交周报","projectPlaceholder":"比如：新建读书项目","taskEmpty":"今天还没有任务","projectEmpty":"说一句开始一个项目"}"#;
        let view = parse_copy(valid, &fallback).unwrap();
        assert!(view.generated);
        assert_eq!(view.nudge, fallback.nudge);
        assert!(parse_copy("普通问候文字", &fallback).is_none());
        assert!(
            parse_copy(
                &valid.replace("今天还没有任务", "第一行\\n第二行"),
                &fallback
            )
            .is_none()
        );
        assert!(parse_copy(&valid.replace("说一句开始一个项目", ""), &fallback).is_none());
        assert!(
            parse_copy(
                &valid.replace("上午好，慢慢来", "别忘了机械光瓦项目阶段性完成的里程碑"),
                &fallback
            )
            .is_none()
        );
    }
}
