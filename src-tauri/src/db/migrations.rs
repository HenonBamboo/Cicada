//! 版本化迁移：PRAGMA user_version 记录已执行到第几个迁移

use rusqlite::Connection;

use crate::error::{AppError, AppResult};

/// 按顺序执行；已发布的迁移不能修改，只能追加
const MIGRATIONS: &[&str] = &[
    include_str!("migrations/0001_init.sql"),
    include_str!("migrations/0002_inbox_retry.sql"),
    include_str!("migrations/0003_inbox_project_context.sql"),
];

pub const LATEST_VERSION: u32 = MIGRATIONS.len() as u32;

pub fn schema_version(conn: &Connection) -> AppResult<u32> {
    Ok(conn.pragma_query_value(None, "user_version", |r| r.get(0))?)
}

/// 执行尚未执行的迁移，每个迁移一个事务。
/// `before` 在确实需要迁移、且数据库已有结构时调用一次（用于迁移前备份）。
pub fn migrate(
    conn: &mut Connection,
    before: impl FnOnce(&Connection, u32) -> AppResult<()>,
) -> AppResult<u32> {
    let current = schema_version(conn)?;
    if current > LATEST_VERSION {
        return Err(AppError::Invalid(format!(
            "数据库版本 v{current} 比当前应用支持的 v{LATEST_VERSION} 新，请升级知了"
        )));
    }
    if current == LATEST_VERSION {
        return Ok(current);
    }
    if current > 0 {
        before(conn, current)?;
    }
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(current as usize) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", (i + 1) as u32)?;
        tx.commit()?;
    }
    Ok(LATEST_VERSION)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{list_tables, open_in_memory};
    use rusqlite::params;

    const TABLES: &[&str] = &[
        "completions",
        "greetings",
        "inbox",
        "meta",
        "nodes",
        "op_log",
        "pending_plans",
        "recur_rules",
        "reports",
        "stickies",
        "tasks",
    ];
    const NOW: &str = "2026-10-17T10:00:00";

    fn fresh() -> Connection {
        let mut conn = open_in_memory().unwrap();
        migrate(&mut conn, |_, _| panic!("新库不需要迁移前备份")).unwrap();
        conn
    }

    fn add_project(conn: &Connection) -> i64 {
        conn.execute(
            "INSERT INTO nodes (depth, title, project_status, created_at, updated_at)
             VALUES (1, '知了', 'active', ?1, ?1)",
            [NOW],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn add_node(conn: &Connection, parent: i64, depth: i64) -> rusqlite::Result<i64> {
        conn.execute(
            "INSERT INTO nodes (parent_id, depth, title, created_at, updated_at)
             VALUES (?1, ?2, '节点', ?3, ?3)",
            params![parent, depth, NOW],
        )?;
        Ok(conn.last_insert_rowid())
    }

    fn count(conn: &Connection, table: &str) -> i64 {
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn 新库建好全部表() {
        let conn = fresh();
        assert_eq!(schema_version(&conn).unwrap(), LATEST_VERSION);
        assert_eq!(list_tables(&conn).unwrap(), TABLES);
    }

    #[test]
    fn 已是最新时不再迁移() {
        let mut conn = fresh();
        let v = migrate(&mut conn, |_, _| panic!("已是最新，不应备份")).unwrap();
        assert_eq!(v, LATEST_VERSION);
    }

    #[test]
    fn 从旧版升级保留原始输入和已有项目() {
        let mut conn = open_in_memory().unwrap();
        conn.execute_batch(MIGRATIONS[0]).unwrap();
        conn.execute_batch(MIGRATIONS[1]).unwrap();
        conn.pragma_update(None, "user_version", 2).unwrap();
        let project = add_project(&conn);
        conn.execute("INSERT INTO inbox(id,source,mode,content,received_at,business_date,status,updated_at) VALUES('old','main','project',?1,?2,'2026-10-17','queued',?2)",
            params!["# 发布\n- 保留 **完整原文**", NOW]).unwrap();
        let mut backed_up = false;
        migrate(&mut conn, |_, version| {
            assert_eq!(version, 2);
            backed_up = true;
            Ok(())
        })
        .unwrap();
        assert!(backed_up);
        let (text, target): (String, Option<i64>) = conn
            .query_row(
                "SELECT content,target_project_id FROM inbox WHERE id='old'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(text, "# 发布\n- 保留 **完整原文**");
        assert_eq!(target, None);
        assert_eq!(
            conn.query_row("SELECT id FROM nodes", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            project
        );
    }

    #[test]
    fn 数据库版本过新时拒绝() {
        let mut conn = fresh();
        conn.pragma_update(None, "user_version", LATEST_VERSION + 1)
            .unwrap();
        assert!(migrate(&mut conn, |_, _| Ok(())).is_err());
    }

    #[test]
    fn 节点最多三层() {
        let conn = fresh();
        let p = add_project(&conn);
        let n2 = add_node(&conn, p, 2).unwrap();
        add_node(&conn, n2, 3).unwrap();
        assert!(add_node(&conn, n2, 4).is_err());
    }

    #[test]
    fn 只有项目带项目状态() {
        let conn = fresh();
        let p = add_project(&conn);
        let bad = conn.execute(
            "INSERT INTO nodes (parent_id, depth, title, project_status, created_at, updated_at)
             VALUES (?1, 2, 'x', 'active', ?2, ?2)",
            params![p, NOW],
        );
        assert!(bad.is_err());
    }

    #[test]
    fn 一个节点只能有一条未完成关联任务() {
        let conn = fresh();
        let p = add_project(&conn);
        let add = |status: &str, done_at: Option<&str>| {
            conn.execute(
                "INSERT INTO tasks (title, planned_date, status, node_id, created_at, updated_at, done_at)
                 VALUES ('写文档', '2026-10-17', ?1, ?2, ?3, ?3, ?4)",
                params![status, p, NOW, done_at],
            )
        };
        add("done", Some(NOW)).unwrap();
        add("open", None).unwrap();
        assert!(add("open", None).is_err());
    }

    #[test]
    fn 删除项目连同子节点并保留完成记录() {
        let conn = fresh();
        let p = add_project(&conn);
        let child = add_node(&conn, p, 2).unwrap();
        conn.execute(
            "INSERT INTO completions (business_date, kind, node_id, title, project_title, completed_at)
             VALUES ('2026-10-17', 'node', ?1, '节点', '知了', ?2)",
            params![child, NOW],
        )
        .unwrap();
        conn.execute("DELETE FROM nodes WHERE id = ?1", [p])
            .unwrap();
        assert_eq!(count(&conn, "nodes"), 0);
        assert_eq!(count(&conn, "completions"), 1);
    }

    #[test]
    fn 快照得到完整一致的副本() {
        let tmp = tempfile::tempdir().unwrap();
        let mut conn = crate::db::open(&tmp.path().join("cicada.db")).unwrap();
        migrate(&mut conn, |_, _| Ok(())).unwrap();
        add_project(&conn);
        let snap = tmp.path().join("snap.db");
        crate::db::snapshot_to(&conn, &snap).unwrap();
        let copy = crate::db::open(&snap).unwrap();
        assert_eq!(list_tables(&copy).unwrap(), TABLES);
        assert_eq!(schema_version(&copy).unwrap(), LATEST_VERSION);
        assert_eq!(count(&copy, "nodes"), 1);
    }
}
