// Database module - SQLite connection and models

pub mod models;

use anyhow::{Context, Result};
use chrono::Datelike;
use chrono::NaiveDate;
use rusqlite::{params, Connection, OptionalExtension};
use rust_decimal::Decimal;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::OnceLock;
use tracing::info;

use crate::term_contracts;
pub use models::{
    Asset, AssetType, CorporateAction, CorporateActionType, IncomeEvent, IncomeEventType,
    PriceHistory, Transaction, TransactionType,
};

/// Get the default database path (~/.interest/data.db)
pub fn get_default_db_path() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME environment variable not set")?;
    let interest_dir = PathBuf::from(home).join(".interest");

    // Create directory if it doesn't exist
    std::fs::create_dir_all(&interest_dir).context("Failed to create .interest directory")?;

    Ok(interest_dir.join("data.db"))
}

/// Open database connection
pub fn open_db(db_path: Option<PathBuf>) -> Result<Connection> {
    let path = db_path.unwrap_or(get_default_db_path()?);
    let conn = Connection::open(&path).context(format!("Failed to open database at {:?}", path))?;

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
    let existing: Option<i64> = stmt.query_row([ticker], |row| row.get(0)).optional()?;

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

/// Get position quantity for an asset before a given date
pub fn get_asset_position_before_date(
    conn: &Connection,
    asset_id: i64,
    before_date: chrono::NaiveDate,
) -> Result<Decimal> {
    let mut stmt = conn.prepare(
        "SELECT transaction_type, quantity
         FROM transactions
         WHERE asset_id = ?1 AND trade_date < ?2
         ORDER BY trade_date ASC, id ASC",
    )?;

    let mut rows = stmt.query(params![asset_id, before_date])?;
    let mut position = Decimal::ZERO;

    while let Some(row) = rows.next()? {
        let tx_type: String = row.get(0)?;
        let quantity = get_decimal_value(row, 1).context("Failed to parse transaction quantity")?;
        match tx_type.parse::<TransactionType>() {
            Ok(TransactionType::Buy) => position += quantity,
            Ok(TransactionType::Sell) => position -= quantity,
            Err(_) => {
                return Err(anyhow::anyhow!(
                    "Unknown transaction type '{}' while computing position",
                    tx_type
                ));
            }
        }
    }

    Ok(position)
}

/// Get last imported date for a source and entry type
pub fn get_last_import_date(
    conn: &Connection,
    source: &str,
    entry_type: &str,
) -> Result<Option<chrono::NaiveDate>> {
    let mut stmt = conn.prepare(
        "SELECT last_date FROM import_state
         WHERE source = ?1 AND entry_type = ?2",
    )?;

    let date: Option<chrono::NaiveDate> = stmt
        .query_row(params![source, entry_type], |row| row.get(0))
        .optional()?;

    if date.is_some() {
        return Ok(date);
    }

    // Fallback: derive last date from existing data if import_state is empty.
    match entry_type {
        "trades" => {
            let mut stmt = conn.prepare(
                "SELECT COALESCE(MAX(trade_date), '') FROM transactions WHERE source = ?1",
            )?;
            let max_date_str: String = stmt.query_row(params![source], |row| row.get(0))?;
            Ok((!max_date_str.is_empty())
                .then(|| chrono::NaiveDate::parse_from_str(&max_date_str, "%Y-%m-%d").ok())
                .flatten())
        }
        "corporate_actions" => {
            let mut stmt = conn.prepare(
                "SELECT COALESCE(MAX(event_date), '') FROM corporate_actions WHERE source = ?1",
            )?;
            let max_date_str: String = stmt.query_row(params![source], |row| row.get(0))?;
            Ok((!max_date_str.is_empty())
                .then(|| chrono::NaiveDate::parse_from_str(&max_date_str, "%Y-%m-%d").ok())
                .flatten())
        }
        _ => Ok(None),
    }
}

/// Update last imported date for a source and entry type
pub fn set_last_import_date(
    conn: &Connection,
    source: &str,
    entry_type: &str,
    last_date: chrono::NaiveDate,
) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO import_state (source, entry_type, last_date)
         VALUES (?1, ?2, ?3)",
        params![source, entry_type, last_date],
    )?;

    Ok(())
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

/// Filter tickers unsupported in portfolio/tax (e.g., options like ITSAA101).
pub fn is_supported_portfolio_ticker(ticker: &str) -> bool {
    ticker.len() <= 6
        && !term_contracts::is_term_contract(ticker)
        && !is_follow_on_option_ticker(ticker)
}

fn is_follow_on_option_ticker(ticker: &str) -> bool {
    matches!(ticker.to_uppercase().as_str(), "JURO15" | "CDII15")
}

/// Rename/merger mappings applied in portfolio/tax calculations.
pub struct RenameMapping {
    pub from: &'static str,
    pub to: &'static str,
    pub effective_date: NaiveDate,
    pub target_quantity: Option<rust_decimal::Decimal>,
}

pub fn rename_mappings() -> &'static [RenameMapping] {
    static MAPPINGS: OnceLock<Vec<RenameMapping>> = OnceLock::new();
    MAPPINGS.get_or_init(|| {
        vec![
            RenameMapping {
                from: "JSLG3",
                to: "SIMH3",
                effective_date: NaiveDate::from_ymd_opt(2020, 9, 21).unwrap(),
                target_quantity: None,
            },
            RenameMapping {
                from: "BAHI3",
                to: "BIED3",
                effective_date: NaiveDate::from_ymd_opt(2024, 11, 26).unwrap(),
                target_quantity: None,
            },
            RenameMapping {
                from: "ALZM11",
                to: "ALZC11",
                effective_date: NaiveDate::from_ymd_opt(2025, 2, 24).unwrap(),
                target_quantity: Some(rust_decimal::Decimal::from(762)),
            },
            RenameMapping {
                from: "RBRF11",
                to: "RBRX11",
                effective_date: NaiveDate::from_ymd_opt(2025, 12, 1).unwrap(),
                target_quantity: Some(rust_decimal::Decimal::from(7894)),
            },
        ]
    })
}

pub fn is_rename_source_ticker(ticker: &str) -> bool {
    rename_mappings()
        .iter()
        .any(|m| m.from.eq_ignore_ascii_case(ticker))
}

pub fn rename_sources_for(ticker: &str) -> Vec<(&'static str, NaiveDate)> {
    rename_mappings()
        .iter()
        .filter(|m| m.to.eq_ignore_ascii_case(ticker))
        .map(|m| (m.from, m.effective_date))
        .collect()
}

pub fn rename_quantity_override(ticker: &str, source: &str) -> Option<rust_decimal::Decimal> {
    rename_mappings()
        .iter()
        .find(|m| m.to.eq_ignore_ascii_case(ticker) && m.from.eq_ignore_ascii_case(source))
        .and_then(|m| m.target_quantity)
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

    let result = stmt
        .query_row([asset_id], |row| {
            Ok(PriceHistory {
                id: Some(row.get(0)?),
                asset_id: row.get(1)?,
                price_date: row.get(2)?,
                close_price: get_decimal_value(row, 3)?,
                open_price: get_optional_decimal_value(row, 4)?,
                high_price: get_optional_decimal_value(row, 5)?,
                low_price: get_optional_decimal_value(row, 6)?,
                volume: row.get(7)?,
                source: row.get(8)?,
                created_at: row.get(9)?,
            })
        })
        .optional()?;

    Ok(result)
}

/// Get the latest price on or before a given date
pub fn get_price_on_or_before(
    conn: &Connection,
    asset_id: i64,
    as_of_date: NaiveDate,
) -> Result<Option<PriceHistory>> {
    let mut stmt = conn.prepare(
        "SELECT id, asset_id, price_date, close_price, open_price, high_price, low_price, volume, source, created_at
         FROM price_history
         WHERE asset_id = ?1 AND price_date <= ?2
         ORDER BY price_date DESC
         LIMIT 1",
    )?;

    let result = stmt
        .query_row(rusqlite::params![asset_id, as_of_date], |row| {
            Ok(PriceHistory {
                id: Some(row.get(0)?),
                asset_id: row.get(1)?,
                price_date: row.get(2)?,
                close_price: get_decimal_value(row, 3)?,
                open_price: get_optional_decimal_value(row, 4)?,
                high_price: get_optional_decimal_value(row, 5)?,
                low_price: get_optional_decimal_value(row, 6)?,
                volume: row.get(7)?,
                source: row.get(8)?,
                created_at: row.get(9)?,
            })
        })
        .optional()?;

    Ok(result)
}

/// Helper to read Decimal from SQLite (handles both INTEGER, REAL and TEXT)
pub fn get_decimal_value(row: &rusqlite::Row, idx: usize) -> Result<Decimal, rusqlite::Error> {
    use rusqlite::types::ValueRef;

    match row.get_ref(idx)? {
        ValueRef::Text(bytes) => {
            let s = std::str::from_utf8(bytes)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            Decimal::from_str(s).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
        }
        ValueRef::Integer(i) => Ok(Decimal::from(i)),
        ValueRef::Real(f) => {
            Decimal::try_from(f).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
        }
        _ => Err(rusqlite::Error::InvalidColumnType(
            idx,
            "decimal".to_string(),
            rusqlite::types::Type::Null,
        )),
    }
}

/// Helper to read optional Decimal from SQLite
fn get_optional_decimal_value(
    row: &rusqlite::Row,
    idx: usize,
) -> Result<Option<Decimal>, rusqlite::Error> {
    use rusqlite::types::ValueRef;

    match row.get_ref(idx)? {
        ValueRef::Null => Ok(None),
        ValueRef::Text(bytes) => {
            let s = std::str::from_utf8(bytes)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            Decimal::from_str(s)
                .map(Some)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
        }
        ValueRef::Integer(i) => Ok(Some(Decimal::from(i))),
        ValueRef::Real(f) => Decimal::try_from(f)
            .map(Some)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e))),
        _ => Ok(None),
    }
}

/// Insert corporate action
pub fn insert_corporate_action(conn: &Connection, action: &CorporateAction) -> Result<i64> {
    conn.execute(
        "INSERT INTO corporate_actions (
            asset_id, action_type, event_date, ex_date, ratio_from, ratio_to, source, notes
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            action.asset_id,
            action.action_type.as_str(),
            action.event_date,
            action.ex_date,
            action.ratio_from,
            action.ratio_to,
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

    let count: i64 = stmt.query_row(params![asset_id, ex_date, action_type.as_str()], |row| {
        row.get(0)
    })?;

    Ok(count > 0)
}

/// List corporate actions with optional ticker filter
pub fn list_corporate_actions(
    conn: &Connection,
    ticker: Option<&str>,
) -> Result<Vec<(CorporateAction, Asset)>> {
    let query = if ticker.is_some() {
        "SELECT ca.id, ca.asset_id, ca.action_type, ca.event_date, ca.ex_date,
                ca.ratio_from, ca.ratio_to, ca.source, ca.notes, ca.created_at,
                a.id, a.ticker, a.asset_type, a.name, a.created_at, a.updated_at
         FROM corporate_actions ca
         JOIN assets a ON ca.asset_id = a.id
         WHERE a.ticker = ?1
         ORDER BY ca.ex_date DESC"
    } else {
        "SELECT ca.id, ca.asset_id, ca.action_type, ca.event_date, ca.ex_date,
                ca.ratio_from, ca.ratio_to, ca.source, ca.notes, ca.created_at,
                a.id, a.ticker, a.asset_type, a.name, a.created_at, a.updated_at
         FROM corporate_actions ca
         JOIN assets a ON ca.asset_id = a.id
         ORDER BY ca.ex_date DESC"
    };

    let mut stmt = conn.prepare(query)?;

    let results = if let Some(t) = ticker {
        stmt.query_map([t], |row| {
            Ok((
                CorporateAction {
                    id: Some(row.get(0)?),
                    asset_id: row.get(1)?,
                    action_type: row
                        .get::<_, String>(2)?
                        .parse::<CorporateActionType>()
                        .unwrap_or(CorporateActionType::Split),
                    event_date: row.get(3)?,
                    ex_date: row.get(4)?,
                    ratio_from: row.get(5)?,
                    ratio_to: row.get(6)?,
                    source: row.get(7)?,
                    notes: row.get(8)?,
                    created_at: row.get(9)?,
                },
                Asset {
                    id: Some(row.get(10)?),
                    ticker: row.get(11)?,
                    asset_type: row
                        .get::<_, String>(12)?
                        .parse::<AssetType>()
                        .unwrap_or(AssetType::Stock),
                    name: row.get(13)?,
                    created_at: row.get(14)?,
                    updated_at: row.get(15)?,
                },
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?
    } else {
        stmt.query_map([], |row| {
            Ok((
                CorporateAction {
                    id: Some(row.get(0)?),
                    asset_id: row.get(1)?,
                    action_type: row
                        .get::<_, String>(2)?
                        .parse::<CorporateActionType>()
                        .unwrap_or(CorporateActionType::Split),
                    event_date: row.get(3)?,
                    ex_date: row.get(4)?,
                    ratio_from: row.get(5)?,
                    ratio_to: row.get(6)?,
                    source: row.get(7)?,
                    notes: row.get(8)?,
                    created_at: row.get(9)?,
                },
                Asset {
                    id: Some(row.get(10)?),
                    ticker: row.get(11)?,
                    asset_type: row
                        .get::<_, String>(12)?
                        .parse::<AssetType>()
                        .unwrap_or(AssetType::Stock),
                    name: row.get(13)?,
                    created_at: row.get(14)?,
                    updated_at: row.get(15)?,
                },
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?
    };

    Ok(results)
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

/// Check if an income event already exists (for duplicate detection)
pub fn income_event_exists(
    conn: &Connection,
    asset_id: i64,
    event_date: NaiveDate,
    event_type: &IncomeEventType,
    total_amount: Decimal,
) -> Result<bool> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM income_events
         WHERE asset_id = ?1 AND event_date = ?2 AND event_type = ?3 AND total_amount = ?4",
        params![
            asset_id,
            event_date,
            event_type.as_str(),
            total_amount.to_string()
        ],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

/// Get income events with asset information (for display)
pub fn get_income_events_with_assets(
    conn: &Connection,
    from_date: Option<NaiveDate>,
    to_date: Option<NaiveDate>,
    asset_filter: Option<&str>,
) -> Result<Vec<(IncomeEvent, Asset)>> {
    let mut sql = String::from(
        "SELECT ie.id, ie.asset_id, ie.event_date, ie.ex_date, ie.event_type,
                ie.amount_per_quota, ie.total_amount, ie.withholding_tax,
                ie.is_quota_pre_2026, ie.source, ie.notes, ie.created_at,
                a.id, a.ticker, a.asset_type, a.name, a.created_at, a.updated_at
         FROM income_events ie
         JOIN assets a ON ie.asset_id = a.id
         WHERE 1=1",
    );

    let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    if let Some(f) = from_date {
        sql.push_str(" AND ie.event_date >= ?");
        params.push(Box::new(f));
    }
    if let Some(t) = to_date {
        sql.push_str(" AND ie.event_date <= ?");
        params.push(Box::new(t));
    }
    if let Some(ticker) = asset_filter {
        sql.push_str(" AND a.ticker = ?");
        params.push(Box::new(ticker.to_uppercase()));
    }

    sql.push_str(" ORDER BY ie.event_date DESC, a.ticker ASC");

    let mut stmt = conn.prepare(&sql)?;
    let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();

    let results = stmt
        .query_map(param_refs.as_slice(), |row| {
            let event = IncomeEvent {
                id: Some(row.get(0)?),
                asset_id: row.get(1)?,
                event_date: row.get(2)?,
                ex_date: row.get(3)?,
                event_type: row
                    .get::<_, String>(4)?
                    .parse::<IncomeEventType>()
                    .unwrap_or(IncomeEventType::Dividend),
                amount_per_quota: get_decimal_value(row, 5)?,
                total_amount: get_decimal_value(row, 6)?,
                withholding_tax: get_decimal_value(row, 7)?,
                is_quota_pre_2026: row.get(8)?,
                source: row.get(9)?,
                notes: row.get(10)?,
                created_at: row.get(11)?,
            };
            let asset = Asset {
                id: Some(row.get(12)?),
                ticker: row.get(13)?,
                asset_type: row
                    .get::<_, String>(14)?
                    .parse::<AssetType>()
                    .unwrap_or(AssetType::Stock),
                name: row.get(15)?,
                created_at: row.get(16)?,
                updated_at: row.get(17)?,
            };
            Ok((event, asset))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(results)
}

/// Get all assets (for batch price updates)
pub fn get_all_assets(conn: &Connection) -> Result<Vec<Asset>> {
    let mut stmt = conn.prepare(
        "SELECT id, ticker, asset_type, name, created_at, updated_at FROM assets ORDER BY ticker",
    )?;

    let assets = stmt
        .query_map([], |row| {
            Ok(Asset {
                id: Some(row.get(0)?),
                ticker: row.get(1)?,
                asset_type: row
                    .get::<_, String>(2)?
                    .parse::<AssetType>()
                    .unwrap_or(AssetType::Stock),
                name: row.get(3)?,
                created_at: row.get(4)?,
                updated_at: row.get(5)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(assets)
}

/// Get dates where prices are missing for an asset within a date range
#[allow(dead_code)]
pub fn get_missing_price_dates(
    conn: &Connection,
    asset_id: i64,
    from_date: NaiveDate,
    to_date: NaiveDate,
) -> Result<Vec<NaiveDate>> {
    use chrono::Weekday;
    use std::collections::HashSet;

    // Get all trading dates in the range (approximate: weekdays only)
    let mut trading_dates = HashSet::new();
    let mut current = from_date;
    while current <= to_date {
        // Check if it's a weekday (Monday=0 to Friday=4)
        let weekday_num = match current.weekday() {
            Weekday::Mon => 0,
            Weekday::Tue => 1,
            Weekday::Wed => 2,
            Weekday::Thu => 3,
            Weekday::Fri => 4,
            _ => 5, // Weekend
        };
        if weekday_num < 5 {
            trading_dates.insert(current);
        }
        current = current.succ_opt().unwrap_or(current);
    }

    // Get dates where we have prices
    let mut stmt = conn.prepare(
        "SELECT DISTINCT price_date FROM price_history WHERE asset_id = ?1 AND price_date >= ?2 AND price_date <= ?3",
    )?;

    let existing_dates: HashSet<NaiveDate> = stmt
        .query_map(rusqlite::params![asset_id, from_date, to_date], |row| {
            row.get(0)
        })?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .collect();

    // Return dates that exist in trading_dates but not in existing_dates
    let missing: Vec<NaiveDate> = trading_dates
        .difference(&existing_dates)
        .copied()
        .collect::<Vec<_>>();

    let mut sorted_missing = missing;
    sorted_missing.sort();

    Ok(sorted_missing)
}

/// Determine which years need COTAHIST data for a date range
#[allow(dead_code)]
pub fn get_required_years(from_date: NaiveDate, to_date: NaiveDate) -> Vec<i32> {
    use chrono::Datelike;

    let mut years = Vec::new();
    let mut current_year = from_date.year();
    let end_year = to_date.year();

    while current_year <= end_year {
        years.push(current_year);
        current_year += 1;
    }

    years
}

/// Check if any prices exist for an asset within a date range
#[allow(dead_code)]
pub fn has_any_prices(
    conn: &Connection,
    asset_id: i64,
    from_date: NaiveDate,
    to_date: NaiveDate,
) -> Result<bool> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM price_history WHERE asset_id = ?1 AND price_date >= ?2 AND price_date <= ?3",
        rusqlite::params![asset_id, from_date, to_date],
        |row| row.get(0),
    )?;

    Ok(count > 0)
}

/// Get the earliest transaction date in the portfolio
#[allow(dead_code)]
pub fn get_earliest_transaction_date(conn: &Connection) -> Result<Option<NaiveDate>> {
    let mut stmt = conn.prepare("SELECT MIN(trade_date) FROM transactions")?;

    // MIN() returns a single row with NULL when table is empty; map NULL to None
    let result: Option<Option<NaiveDate>> = stmt.query_row([], |row| row.get(0)).optional()?;

    Ok(result.flatten())
}

/// Get only assets that have transactions (owned or previously owned)
#[allow(dead_code)]
pub fn get_assets_with_transactions(conn: &Connection) -> Result<Vec<Asset>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT a.id, a.ticker, a.name, a.asset_type, a.created_at, a.updated_at
         FROM assets a 
         INNER JOIN transactions t ON a.id = t.asset_id
         ORDER BY a.ticker",
    )?;

    let assets = stmt.query_map([], |row| {
        Ok(Asset {
            id: Some(row.get(0)?),
            ticker: row.get(1)?,
            name: row.get(2)?,
            asset_type: row.get::<_, String>(3)?.parse().unwrap(),
            created_at: row.get(4)?,
            updated_at: row.get(5)?,
        })
    })?;

    let mut result = Vec::new();
    for asset in assets {
        result.push(asset?);
    }
    Ok(result)
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
