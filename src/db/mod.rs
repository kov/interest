// Database module - SQLite connection and models

pub mod models;

use anyhow::{Context, Result};
use rusqlite::{Connection, params, OptionalExtension};
use std::path::PathBuf;
use std::str::FromStr;
use tracing::info;

pub use models::{
    Asset, AssetType, Transaction, TransactionType,
    CorporateAction, CorporateActionType, PriceHistory, IncomeEvent, IncomeEventType,
};

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

/// Insert price history
pub fn insert_price_history(conn: &Connection, price: &PriceHistory) -> Result<i64> {
    conn.execute(
        "INSERT OR REPLACE INTO price_history (
            asset_id, price_date, close_price, open_price, high_price, low_price, volume, source
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            price.asset_id,
            price.price_date,
            price.close_price.to_string(),
            price.open_price.as_ref().map(|d| d.to_string()),
            price.high_price.as_ref().map(|d| d.to_string()),
            price.low_price.as_ref().map(|d| d.to_string()),
            price.volume,
            price.source,
        ],
    )?;

    Ok(conn.last_insert_rowid())
}

/// Get latest price for an asset
pub fn get_latest_price(conn: &Connection, asset_id: i64) -> Result<Option<PriceHistory>> {
    let mut stmt = conn.prepare(
        "SELECT id, asset_id, price_date, close_price, open_price, high_price, low_price, volume, source, created_at
         FROM price_history
         WHERE asset_id = ?1
         ORDER BY price_date DESC
         LIMIT 1"
    )?;

    let result = stmt.query_row([asset_id], |row| {
        Ok(PriceHistory {
            id: Some(row.get(0)?),
            asset_id: row.get(1)?,
            price_date: row.get(2)?,
            close_price: rust_decimal::Decimal::from_str(&row.get::<_, String>(3)?)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?,
            open_price: row.get::<_, Option<String>>(4)?
                .and_then(|s| rust_decimal::Decimal::from_str(&s).ok()),
            high_price: row.get::<_, Option<String>>(5)?
                .and_then(|s| rust_decimal::Decimal::from_str(&s).ok()),
            low_price: row.get::<_, Option<String>>(6)?
                .and_then(|s| rust_decimal::Decimal::from_str(&s).ok()),
            volume: row.get(7)?,
            source: row.get(8)?,
            created_at: row.get(9)?,
        })
    }).optional()?;

    Ok(result)
}

/// Insert corporate action
pub fn insert_corporate_action(conn: &Connection, action: &CorporateAction) -> Result<i64> {
    conn.execute(
        "INSERT INTO corporate_actions (
            asset_id, action_type, event_date, ex_date, ratio_from, ratio_to, applied, source, notes
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            action.asset_id,
            action.action_type.as_str(),
            action.event_date,
            action.ex_date,
            action.ratio_from,
            action.ratio_to,
            action.applied,
            action.source,
            action.notes,
        ],
    )?;

    Ok(conn.last_insert_rowid())
}

/// Check if corporate action already exists
pub fn corporate_action_exists(
    conn: &Connection,
    asset_id: i64,
    ex_date: &chrono::NaiveDate,
    action_type: &CorporateActionType,
) -> Result<bool> {
    let mut stmt = conn.prepare(
        "SELECT COUNT(*) FROM corporate_actions
         WHERE asset_id = ?1 AND ex_date = ?2 AND action_type = ?3",
    )?;

    let count: i64 = stmt.query_row(
        params![asset_id, ex_date, action_type.as_str()],
        |row| row.get(0),
    )?;

    Ok(count > 0)
}

/// Insert income event
pub fn insert_income_event(conn: &Connection, event: &IncomeEvent) -> Result<i64> {
    conn.execute(
        "INSERT INTO income_events (
            asset_id, event_date, ex_date, event_type, amount_per_quota, total_amount,
            withholding_tax, is_quota_pre_2026, source, notes
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            event.asset_id,
            event.event_date,
            event.ex_date,
            event.event_type.as_str(),
            event.amount_per_quota.to_string(),
            event.total_amount.to_string(),
            event.withholding_tax.to_string(),
            event.is_quota_pre_2026,
            event.source,
            event.notes,
        ],
    )?;

    Ok(conn.last_insert_rowid())
}

/// Get all assets (for batch price updates)
pub fn get_all_assets(conn: &Connection) -> Result<Vec<Asset>> {
    let mut stmt = conn.prepare(
        "SELECT id, ticker, asset_type, name, created_at, updated_at FROM assets ORDER BY ticker"
    )?;

    let assets = stmt
        .query_map([], |row| {
            Ok(Asset {
                id: Some(row.get(0)?),
                ticker: row.get(1)?,
                asset_type: AssetType::from_str(&row.get::<_, String>(2)?)
                    .unwrap_or(AssetType::Stock),
                name: row.get(3)?,
                created_at: row.get(4)?,
                updated_at: row.get(5)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(assets)
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
