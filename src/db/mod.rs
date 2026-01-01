// Database module - SQLite connection and models

pub mod models;

use anyhow::{Context, Result};
use rusqlite::{Connection, params, OptionalExtension};
use std::path::PathBuf;
use tracing::info;

pub use models::{Asset, AssetType, Transaction, TransactionType};

/// Get the default database path (~/.interest/data.db)
pub fn get_default_db_path() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME environment variable not set")?;
    let interest_dir = PathBuf::from(home).join(".interest");

    // Create directory if it doesn't exist
    std::fs::create_dir_all(&interest_dir)
        .context("Failed to create .interest directory")?;

    Ok(interest_dir.join("data.db"))
}

/// Open database connection
pub fn open_db(db_path: Option<PathBuf>) -> Result<Connection> {
    let path = db_path.unwrap_or(get_default_db_path()?);
    let conn = Connection::open(&path)
        .context(format!("Failed to open database at {:?}", path))?;

    // Enable foreign keys
    conn.execute("PRAGMA foreign_keys = ON", [])
        .context("Failed to enable foreign keys")?;

    Ok(conn)
}

/// Initialize the database with schema
///
/// This function creates the database file and runs the schema SQL
/// to set up all tables and indexes.
pub fn init_database(db_path: Option<PathBuf>) -> Result<()> {
    let path = db_path.unwrap_or(get_default_db_path()?);

    info!("Initializing database at: {:?}", path);

    let conn = open_db(Some(path))?;

    // Read schema SQL
    let schema_sql = include_str!("schema.sql");

    // Execute schema
    conn.execute_batch(schema_sql)
        .context("Failed to execute schema")?;

    info!("Database initialized successfully");
    Ok(())
}

/// Insert or get asset, returns asset_id
pub fn upsert_asset(
    conn: &Connection,
    ticker: &str,
    asset_type: &AssetType,
    name: Option<&str>,
) -> Result<i64> {
    // Try to find existing asset
    let mut stmt = conn.prepare("SELECT id FROM assets WHERE ticker = ?1")?;
    let existing: Option<i64> = stmt
        .query_row([ticker], |row| row.get(0))
        .optional()?;

    if let Some(id) = existing {
        return Ok(id);
    }

    // Insert new asset
    conn.execute(
        "INSERT INTO assets (ticker, asset_type, name) VALUES (?1, ?2, ?3)",
        params![ticker, asset_type.as_str(), name],
    )?;

    Ok(conn.last_insert_rowid())
}

/// Insert transaction
pub fn insert_transaction(conn: &Connection, tx: &Transaction) -> Result<i64> {
    conn.execute(
        "INSERT INTO transactions (
            asset_id, transaction_type, trade_date, settlement_date,
            quantity, price_per_unit, total_cost, fees,
            is_day_trade, quota_issuance_date, notes, source
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            tx.asset_id,
            tx.transaction_type.as_str(),
            tx.trade_date,
            tx.settlement_date,
            tx.quantity.to_string(),
            tx.price_per_unit.to_string(),
            tx.total_cost.to_string(),
            tx.fees.to_string(),
            tx.is_day_trade,
            tx.quota_issuance_date,
            tx.notes,
            tx.source,
        ],
    )?;

    Ok(conn.last_insert_rowid())
}

/// Check if a transaction already exists (duplicate detection)
pub fn transaction_exists(
    conn: &Connection,
    asset_id: i64,
    trade_date: &chrono::NaiveDate,
    transaction_type: &TransactionType,
    quantity: &rust_decimal::Decimal,
) -> Result<bool> {
    let mut stmt = conn.prepare(
        "SELECT COUNT(*) FROM transactions
         WHERE asset_id = ?1 AND trade_date = ?2
           AND transaction_type = ?3 AND quantity = ?4",
    )?;

    let count: i64 = stmt.query_row(
        params![
            asset_id,
            trade_date,
            transaction_type.as_str(),
            quantity.to_string()
        ],
        |row| row.get(0),
    )?;

    Ok(count > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_default_db_path() {
        let path = get_default_db_path().unwrap();
        assert!(path.to_string_lossy().contains(".interest"));
        assert!(path.to_string_lossy().ends_with("data.db"));
    }

    #[test]
    fn test_init_database() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db_path = temp_dir.path().join("test.db");

        init_database(Some(db_path.clone())).unwrap();

        // Verify database exists and has tables
        let conn = Connection::open(&db_path).unwrap();
        let table_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table'",
                [],
                |row| row.get(0),
            )
            .unwrap();

        assert!(table_count > 0);
    }
}
