//! SQLite 连接与迁移

#[cfg(debug_assertions)]
pub mod dev_seed;
mod migrations;

use std::path::Path;

use rusqlite::Connection;

use crate::error::AppResult;

pub use migrations::{LATEST_VERSION, migrate, schema_version};

/// 打开数据库并设置连接参数
pub fn open(path: &Path) -> AppResult<Connection> {
    let conn = Connection::open(path)?;
    configure(&conn)?;
    Ok(conn)
}

#[cfg(test)]
pub fn open_in_memory() -> AppResult<Connection> {
    let conn = Connection::open_in_memory()?;
    configure(&conn)?;
    Ok(conn)
}

fn configure(conn: &Connection) -> AppResult<()> {
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    Ok(())
}

/// 当前数据库中的业务表（不含 SQLite 内部表）
pub fn list_tables(conn: &Connection) -> AppResult<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )?;
    let names = stmt
        .query_map([], |r| r.get(0))?
        .collect::<Result<Vec<String>, _>>()?;
    Ok(names)
}

/// 用 VACUUM INTO 生成一致的数据库副本（WAL 模式下直接复制文件可能漏掉未落盘的数据）
pub fn snapshot_to(conn: &Connection, dest: &Path) -> AppResult<()> {
    conn.execute("VACUUM INTO ?1", [dest.to_string_lossy()])?;
    Ok(())
}
