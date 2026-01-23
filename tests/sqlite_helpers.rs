use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::PathBuf;
use tempfile::TempDir;

pub fn db_path(home: &TempDir) -> PathBuf {
    home.path().join(".interest").join("data.db")
}

pub fn open_conn(home: &TempDir) -> Result<Connection> {
    let path = db_path(home);
    Connection::open(path).context("failed to open test database")
}

pub fn list_import_state(conn: &Connection) -> Result<Vec<(String, String, String)>> {
    let mut stmt = conn.prepare(
        "SELECT source, entry_type, last_date FROM import_state ORDER BY source, entry_type",
    )?;
    let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
    let mut items = Vec::new();
    for row in rows {
        items.push(row?);
    }
    Ok(items)
}

pub fn list_metadata_keys(conn: &Connection, prefix: &str) -> Result<Vec<String>> {
    let like_pattern = format!("{}%", prefix);
    let mut stmt = conn.prepare("SELECT key FROM metadata WHERE key LIKE ?1 ORDER BY key")?;
    let rows = stmt.query_map([like_pattern], |row| row.get(0))?;
    let mut keys = Vec::new();
    for row in rows {
        keys.push(row?);
    }
    Ok(keys)
}

pub fn count_snapshots_on_or_after(conn: &Connection, date: &str) -> Result<i64> {
    let mut stmt =
        conn.prepare("SELECT COUNT(*) FROM position_snapshots WHERE snapshot_date >= ?1")?;
    let count = stmt.query_row([date], |row| row.get(0))?;
    Ok(count)
}
