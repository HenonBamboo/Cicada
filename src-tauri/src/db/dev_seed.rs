//! 开发版演示数据（仅 debug 构建，只写 Cicada-dev）。
//! 数据库为空时写入一份贴近原型的数据，方便对照 docs/原型/主界面.html 检查界面。
//! 这些数据不经过业务服务、不写 op_log，正式版不包含本模块。

use chrono::{Datelike, Days, NaiveDate};
use rusqlite::{Connection, params};

use crate::error::AppResult;

const DATE: &str = "%Y-%m-%d";

fn d(today: NaiveDate, offset: i64) -> String {
    let date = if offset >= 0 {
        today.checked_add_days(Days::new(offset as u64))
    } else {
        today.checked_sub_days(Days::new((-offset) as u64))
    };
    date.unwrap_or(today).format(DATE).to_string()
}

/// 数据库里还没有任务和项目时写入演示数据，返回是否写入
pub fn seed_if_empty(conn: &Connection, today: NaiveDate, now: &str) -> AppResult<bool> {
    let used: i64 = conn.query_row(
        "SELECT (SELECT COUNT(*) FROM tasks) + (SELECT COUNT(*) FROM nodes) + (SELECT COUNT(*) FROM completions)",
        [],
        |r| r.get(0),
    )?;
    if used > 0 {
        return Ok(false);
    }
    let tx = conn.unchecked_transaction()?;

    // 项目
    let node = |parent: Option<i64>,
                depth: i64,
                title: &str,
                done: bool,
                ps: Option<&str>,
                order: i64|
     -> AppResult<i64> {
        tx.execute(
            "INSERT INTO nodes (parent_id, depth, title, status, project_status, sort_order, created_at, updated_at, done_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, ?8)",
            params![parent, depth, title, if done { "done" } else { "open" }, ps, order, now, done.then_some(now)],
        )?;
        Ok(tx.last_insert_rowid())
    };
    let p1 = node(None, 1, "知了", false, Some("active"), 0)?;
    let s0 = node(Some(p1), 2, "阶段 0 · 界面原型", false, None, 0)?;
    node(Some(s0), 3, "主界面", true, None, 0)?;
    node(Some(s0), 3, "便利贴", true, None, 1)?;
    let quick = node(Some(s0), 3, "快速记录", false, None, 2)?;
    node(Some(s0), 3, "设置页", false, None, 3)?;
    let s1 = node(Some(p1), 2, "阶段 1 · 项目骨架", false, None, 1)?;
    node(Some(s1), 3, "Tauri 模板", false, None, 0)?;
    node(Some(s1), 3, "主题系统", false, None, 1)?;
    node(Some(s1), 3, "数据库迁移", false, None, 2)?;
    node(Some(p1), 2, "阶段 2 · 主界面", false, None, 2)?;

    let p2 = node(None, 1, "读书计划 2026", false, Some("active"), 1)?;
    let b1 = node(Some(p2), 2, "《置身事内》", false, None, 0)?;
    node(Some(b1), 3, "第 1–3 章", true, None, 0)?;
    node(Some(b1), 3, "第 4 章", false, None, 1)?;
    node(Some(b1), 3, "读书笔记", true, None, 2)?;
    let b2 = node(Some(p2), 2, "《原则》", true, None, 1)?;
    node(Some(b2), 3, "通读", true, None, 0)?;
    node(Some(b2), 3, "摘录", true, None, 1)?;
    node(Some(p2), 2, "《人类简史》", false, None, 2)?;

    let p3 = node(None, 1, "搬家", true, Some("archived"), 2)?;
    node(Some(p3), 2, "找房", true, None, 0)?;
    node(Some(p3), 2, "打包", true, None, 1)?;

    // 重复规则
    tx.execute(
        "INSERT INTO recur_rules (title, kind, start_date, created_at, updated_at) VALUES ('跑步 30 分钟', 'daily', ?1, ?2, ?2)",
        params![d(today, -60), now],
    )?;
    let daily = tx.last_insert_rowid();

    // 任务
    let to_monday = (7 - today.weekday().num_days_from_monday() as i64) % 7;
    let to_monday = if to_monday == 0 { 7 } else { to_monday };
    let task = |title: &str,
                planned: String,
                remind: Option<String>,
                due: Option<String>,
                node_id: Option<i64>,
                recur: Option<i64>|
     -> AppResult<i64> {
        tx.execute(
            "INSERT INTO tasks (title, planned_date, remind_at, due_at, node_id, recur_rule_id, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
            params![title, planned, remind, due, node_id, recur, now],
        )?;
        Ok(tx.last_insert_rowid())
    };
    let t = d(today, 0);
    task("整理本周会议纪要", t.clone(), None, None, None, None)?;
    task(
        "给「知了」做快速记录原型",
        t.clone(),
        None,
        None,
        Some(quick),
        None,
    )?;
    task(
        "下午 3 点产品评审会议",
        t.clone(),
        Some(format!("{t}T15:00:00")),
        None,
        None,
        None,
    )?;
    task(
        "准备周一例会的会议材料",
        t.clone(),
        None,
        Some(format!("{}T09:00:00", d(today, to_monday))),
        None,
        None,
    )?;
    task("回复王老师的邮件", d(today, -3), None, None, None, None)?;
    task("预约体检", d(today, -9), None, None, None, None)?;
    let run = task("跑步 30 分钟", t.clone(), None, None, None, Some(daily))?;
    let cat = task("买猫粮", t.clone(), None, None, None, None)?;
    task(
        "交季度个税申报",
        d(today, 3),
        Some(format!("{}T09:00:00", d(today, 3))),
        None,
        None,
        None,
    )?;
    task("妈妈生日，订蛋糕", d(today, 7), None, None, None, None)?;
    for (id, title, at) in [
        (run, "跑步 30 分钟", "07:10:00"),
        (cat, "买猫粮", "12:30:00"),
    ] {
        tx.execute(
            "UPDATE tasks SET status = 'done', done_at = ?2, done_date = ?3 WHERE id = ?1",
            params![id, format!("{t}T{at}"), t],
        )?;
        tx.execute(
            "INSERT INTO completions (business_date, kind, task_id, title, completed_at) VALUES (?1, 'task', ?2, ?3, ?4)",
            params![t, id, title, format!("{t}T{at}")],
        )?;
    }

    // 近一年的完成历史（确定性伪随机），让热力图有内容
    const POOL: [&str; 9] = [
        "整理会议纪要",
        "跑步 30 分钟",
        "回复客户邮件",
        "读《置身事内》",
        "买菜",
        "修改简历",
        "交水电费",
        "写周报",
        "整理照片",
    ];
    let mut seed: u64 = 20261017;
    let mut rand = move || {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((seed >> 33) as f64) / ((1u64 << 31) as f64)
    };
    for back in 1..=364i64 {
        let day = today
            .checked_sub_days(Days::new(back as u64))
            .unwrap_or(today);
        let weekend = day.weekday().num_days_from_monday() >= 5;
        let ramp = 0.45 + 0.55 * (1.0 - back as f64 / 364.0);
        if rand() < 0.22 {
            continue;
        }
        let n = ((if weekend { 1.4 } else { 3.0 }) * ramp * (0.3 + rand() * 1.5)).round() as usize;
        let date = day.format(DATE).to_string();
        for i in 0..n {
            let kind = if rand() < 0.7 { "task" } else { "node" };
            let title = POOL[(back as usize + i) % POOL.len()];
            tx.execute(
                "INSERT INTO completions (business_date, kind, task_id, node_id, title, project_title, completed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    date,
                    kind,
                    (kind == "task").then_some(0),
                    (kind == "node").then_some(0),
                    title,
                    (kind == "node").then_some("知了"),
                    format!("{date}T20:00:00"),
                ],
            )?;
        }
    }
    tx.execute(
        "INSERT OR REPLACE INTO meta (key, value) VALUES ('dev_seeded', ?1)",
        [now],
    )?;
    tx.commit()?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use crate::service::Service;
    use crate::service::testing::*;

    #[test]
    fn 空库写入演示数据且只写一次() {
        let conn = db();
        let clock = clock("2026-10-17 10:00:00");
        let svc = Service::new(&conn, &clock, &AppConfig::default()).unwrap();
        assert!(seed_if_empty(&conn, svc.today(), "2026-10-17T10:00:00").unwrap());
        assert!(!seed_if_empty(&conn, svc.today(), "2026-10-17T10:00:00").unwrap());

        let b = svc.board().unwrap();
        assert_eq!(b.tasks.today.len(), 4);
        assert_eq!(b.tasks.legacy.len(), 2);
        assert_eq!(b.tasks.done.len(), 2);
        assert_eq!(b.tasks.upcoming.len(), 2);
        assert_eq!(b.projects.len(), 3);
        assert!(svc.heatmap().unwrap().len() > 200);
    }
}
