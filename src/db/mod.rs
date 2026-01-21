// Database module - SQLite connection and models

pub mod models;

use anyhow::{Context, Result};
use chrono::Datelike;
use chrono::NaiveDate;
use rusqlite::{params, Connection, OptionalExtension};
use rust_decimal::Decimal;
use std::path::PathBuf;
use std::str::FromStr;
use tracing::info;

use crate::term_contracts;
pub use models::{
    Asset, AssetExchange, AssetExchangeType, AssetRegistryEntry, AssetRename, AssetType,
    CorporateAction, CorporateActionType, GovBondRate, IncomeEvent, IncomeEventType, Inconsistency,
    InconsistencySeverity, InconsistencyStatus, InconsistencyType, PriceHistory, Transaction,
    TransactionType,
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

    // Apply schema updates (idempotent) to ensure new tables exist.
    let schema_sql = include_str!("schema.sql");
    conn.execute_batch(schema_sql)
        .context("Failed to apply database schema")?;

    Ok(conn)
}

/// Read a metadata value by key.
pub fn get_metadata(conn: &Connection, key: &str) -> Result<Option<String>> {
    let mut stmt = conn.prepare("SELECT value FROM metadata WHERE key = ?1")?;
    let value = stmt.query_row(params![key], |row| row.get(0)).optional()?;
    Ok(value)
}

/// Insert or update a metadata key.
pub fn set_metadata(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO metadata (key, value, updated_at) VALUES (?1, ?2, CURRENT_TIMESTAMP)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = CURRENT_TIMESTAMP",
        params![key, value],
    )?;
    Ok(())
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
    _asset_type: &AssetType,
    name: Option<&str>,
) -> Result<i64> {
    // Try to find existing asset
    let mut stmt = conn.prepare("SELECT id FROM assets WHERE ticker = ?1")?;
    let existing: Option<i64> = stmt.query_row([ticker], |row| row.get(0)).optional()?;

    if let Some(id) = existing {
        return Ok(id);
    }

    let resolved_type = match crate::tickers::resolve_asset_type_with_name(ticker, name) {
        Ok(Some(asset_type)) => asset_type,
        Ok(None) => AssetType::Unknown,
        Err(err) => {
            tracing::warn!("Failed to resolve asset type for {}: {}", ticker, err);
            AssetType::Unknown
        }
    };

    let registry = get_asset_registry_by_ticker(conn, "MAIS_RETORNO", ticker)?;
    let (final_type, final_name, final_cnpj) = if let Some(entry) = registry {
        let asset_type = if resolved_type == AssetType::Unknown {
            entry.asset_type
        } else {
            resolved_type
        };
        let name = name.map(|s| s.to_string()).or_else(|| entry.name.clone());
        let cnpj = entry.cnpj.clone();
        (asset_type, name, cnpj)
    } else {
        (resolved_type, name.map(|s| s.to_string()), None)
    };

    // Insert new asset
    conn.execute(
        "INSERT INTO assets (ticker, asset_type, name, cnpj) VALUES (?1, ?2, ?3, ?4)",
        params![
            ticker.to_uppercase(),
            final_type.as_str(),
            final_name,
            final_cnpj
        ],
    )?;

    Ok(conn.last_insert_rowid())
}

/// Check whether a ticker exists in `assets`
pub fn asset_exists(conn: &Connection, ticker: &str) -> Result<bool> {
    let mut stmt = conn.prepare("SELECT id FROM assets WHERE ticker = ?1")?;
    let existing: Option<i64> = stmt
        .query_row([ticker.to_uppercase()], |row| row.get(0))
        .optional()?;
    Ok(existing.is_some())
}

/// Get asset by ticker
pub fn get_asset_by_ticker(conn: &Connection, ticker: &str) -> Result<Option<Asset>> {
    let mut stmt = conn.prepare(
        "SELECT id, ticker, asset_type, name, cnpj, created_at, updated_at
         FROM assets WHERE ticker = ?1",
    )?;
    let asset = stmt
        .query_row([ticker.to_uppercase()], |row| {
            Ok(Asset {
                id: row.get(0)?,
                ticker: row.get(1)?,
                asset_type: row
                    .get::<_, String>(2)?
                    .parse::<AssetType>()
                    .unwrap_or(AssetType::Unknown),
                name: row.get(3)?,
                cnpj: row.get(4)?,
                created_at: row.get(5)?,
                updated_at: row.get(6)?,
            })
        })
        .optional()?;
    Ok(asset)
}

/// Insert asset with an explicit type (no auto-detect)
pub fn insert_asset(
    conn: &Connection,
    ticker: &str,
    asset_type: &AssetType,
    name: Option<&str>,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO assets (ticker, asset_type, name, cnpj) VALUES (?1, ?2, ?3, ?4)",
        params![
            ticker.to_uppercase(),
            asset_type.as_str(),
            name,
            Option::<String>::None
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Update asset name for a ticker
pub fn update_asset_name(conn: &Connection, ticker: &str, name: &str) -> Result<()> {
    let count = conn.execute(
        "UPDATE assets SET name = ?1, updated_at = CURRENT_TIMESTAMP WHERE ticker = ?2",
        params![name, ticker.to_uppercase()],
    )?;
    if count == 0 {
        return Err(anyhow::anyhow!("Ticker {} not found in assets", ticker));
    }
    Ok(())
}

/// Update asset CNPJ for a ticker
pub fn update_asset_cnpj(conn: &Connection, ticker: &str, cnpj: &str) -> Result<()> {
    let count = conn.execute(
        "UPDATE assets SET cnpj = ?1, updated_at = CURRENT_TIMESTAMP WHERE ticker = ?2",
        params![cnpj, ticker.to_uppercase()],
    )?;
    if count == 0 {
        return Err(anyhow::anyhow!("Ticker {} not found in assets", ticker));
    }
    Ok(())
}

/// Insert or update an external asset registry entry.
pub fn upsert_asset_registry(conn: &Connection, entry: &AssetRegistryEntry) -> Result<()> {
    conn.execute(
        "INSERT INTO asset_registry (
            source, ticker, asset_type, name, cnpj, actuation_segment, actuation_sector,
            issue, situation, indexer, security_type, codigo, data_emissao, data_vencimento,
            source_url, raw_json
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
        ON CONFLICT(source, ticker) DO UPDATE SET
            asset_type = excluded.asset_type,
            name = excluded.name,
            cnpj = excluded.cnpj,
            actuation_segment = excluded.actuation_segment,
            actuation_sector = excluded.actuation_sector,
            issue = excluded.issue,
            situation = excluded.situation,
            indexer = excluded.indexer,
            security_type = excluded.security_type,
            codigo = excluded.codigo,
            data_emissao = excluded.data_emissao,
            data_vencimento = excluded.data_vencimento,
            source_url = excluded.source_url,
            raw_json = excluded.raw_json,
            updated_at = CURRENT_TIMESTAMP",
        params![
            entry.source,
            entry.ticker.to_uppercase(),
            entry.asset_type.as_str(),
            entry.name,
            entry.cnpj,
            entry.actuation_segment,
            entry.actuation_sector,
            entry.issue,
            entry.situation,
            entry.indexer,
            entry.security_type,
            entry.codigo,
            entry.data_emissao,
            entry.data_vencimento,
            entry.source_url,
            entry.raw_json,
        ],
    )?;
    Ok(())
}

/// Lookup an asset registry entry by source and ticker.
pub fn get_asset_registry_by_ticker(
    conn: &Connection,
    source: &str,
    ticker: &str,
) -> Result<Option<AssetRegistryEntry>> {
    let mut stmt = conn.prepare(
        "SELECT source, ticker, asset_type, name, cnpj, actuation_segment, actuation_sector,
                issue, situation, indexer, security_type, codigo, data_emissao, data_vencimento,
                source_url, raw_json, updated_at
         FROM asset_registry
         WHERE source = ?1 AND ticker = ?2",
    )?;

    let entry = stmt
        .query_row(params![source, ticker.to_uppercase()], |row| {
            Ok(AssetRegistryEntry {
                source: row.get(0)?,
                ticker: row.get(1)?,
                asset_type: row
                    .get::<_, String>(2)?
                    .parse::<AssetType>()
                    .unwrap_or(AssetType::Unknown),
                name: row.get(3)?,
                cnpj: row.get(4)?,
                actuation_segment: row.get(5)?,
                actuation_sector: row.get(6)?,
                issue: row.get(7)?,
                situation: row.get(8)?,
                indexer: row.get(9)?,
                security_type: row.get(10)?,
                codigo: row.get(11)?,
                data_emissao: row.get(12)?,
                data_vencimento: row.get(13)?,
                source_url: row.get(14)?,
                raw_json: row.get(15)?,
                updated_at: row.get(16)?,
            })
        })
        .optional()?;

    Ok(entry)
}

/// Rename an asset ticker (correction-only, no historical tracking)
pub fn update_asset_ticker(conn: &Connection, old_ticker: &str, new_ticker: &str) -> Result<()> {
    let new_upper = new_ticker.to_uppercase();
    let existing: Option<i64> = conn
        .query_row(
            "SELECT id FROM assets WHERE ticker = ?1",
            [new_upper.clone()],
            |row| row.get(0),
        )
        .optional()?;
    if existing.is_some() {
        return Err(anyhow::anyhow!("Ticker {} already exists", new_upper));
    }

    let count = conn.execute(
        "UPDATE assets SET ticker = ?1, updated_at = CURRENT_TIMESTAMP WHERE ticker = ?2",
        params![new_upper, old_ticker.to_uppercase()],
    )?;
    if count == 0 {
        return Err(anyhow::anyhow!("Ticker {} not found in assets", old_ticker));
    }
    Ok(())
}

/// Delete asset by ticker (cascades to related tables)
pub fn delete_asset(conn: &Connection, ticker: &str) -> Result<usize> {
    let count = conn.execute(
        "DELETE FROM assets WHERE ticker = ?1",
        params![ticker.to_uppercase()],
    )?;
    Ok(count)
}

/// Count transactions for an asset ticker
pub fn count_transactions_for_asset(conn: &Connection, ticker: &str) -> Result<i64> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM transactions t
         JOIN assets a ON t.asset_id = a.id
         WHERE a.ticker = ?1",
        [ticker.to_uppercase()],
        |row| row.get(0),
    )?;
    Ok(count)
}

/// Get earliest transaction date for an asset ticker
pub fn get_earliest_transaction_date_for_asset(
    conn: &Connection,
    ticker: &str,
) -> Result<Option<NaiveDate>> {
    let date: Option<NaiveDate> = conn
        .query_row(
            "SELECT MIN(t.trade_date) FROM transactions t
             JOIN assets a ON t.asset_id = a.id
             WHERE a.ticker = ?1",
            [ticker.to_uppercase()],
            |row| row.get(0),
        )
        .optional()?;
    Ok(date)
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

/// Insert inconsistency record
pub fn insert_inconsistency(conn: &Connection, issue: &Inconsistency) -> Result<i64> {
    conn.execute(
        "INSERT INTO inconsistencies (
            issue_type, status, severity, asset_id, transaction_id, ticker, trade_date,
            quantity, source, source_ref, missing_fields_json, context_json,
            resolution_action, resolution_json, resolved_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        params![
            issue.issue_type.as_str(),
            issue.status.as_str(),
            issue.severity.as_str(),
            issue.asset_id,
            issue.transaction_id,
            issue.ticker,
            issue.trade_date,
            issue.quantity.as_ref().map(|q| q.to_string()),
            issue.source,
            issue.source_ref,
            issue.missing_fields_json,
            issue.context_json,
            issue.resolution_action,
            issue.resolution_json,
            issue.resolved_at,
        ],
    )?;

    Ok(conn.last_insert_rowid())
}

/// Fetch a single inconsistency by id
pub fn get_inconsistency(conn: &Connection, id: i64) -> Result<Option<Inconsistency>> {
    let mut stmt = conn.prepare(
        "SELECT id, issue_type, status, severity, asset_id, transaction_id, ticker, trade_date,
                quantity, source, source_ref, missing_fields_json, context_json,
                resolution_action, resolution_json, created_at, resolved_at
         FROM inconsistencies
         WHERE id = ?1",
    )?;

    let result = stmt
        .query_row(params![id], |row| {
            Ok(Inconsistency {
                id: Some(row.get(0)?),
                issue_type: row
                    .get::<_, String>(1)?
                    .parse::<InconsistencyType>()
                    .unwrap_or(InconsistencyType::MissingCostBasis),
                status: row
                    .get::<_, String>(2)?
                    .parse::<InconsistencyStatus>()
                    .unwrap_or(InconsistencyStatus::Open),
                severity: row
                    .get::<_, String>(3)?
                    .parse::<InconsistencySeverity>()
                    .unwrap_or(InconsistencySeverity::Warn),
                asset_id: row.get(4)?,
                transaction_id: row.get(5)?,
                ticker: row.get(6)?,
                trade_date: row.get(7)?,
                quantity: get_optional_decimal_value(row, 8)?,
                source: row.get(9)?,
                source_ref: row.get(10)?,
                missing_fields_json: row.get(11)?,
                context_json: row.get(12)?,
                resolution_action: row.get(13)?,
                resolution_json: row.get(14)?,
                created_at: row.get(15)?,
                resolved_at: row.get(16)?,
            })
        })
        .optional()?;

    Ok(result)
}

/// List inconsistencies with optional filters
pub fn list_inconsistencies(
    conn: &Connection,
    status: Option<InconsistencyStatus>,
    issue_type: Option<InconsistencyType>,
    ticker: Option<&str>,
) -> Result<Vec<Inconsistency>> {
    let mut query = String::from(
        "SELECT id, issue_type, status, severity, asset_id, transaction_id, ticker, trade_date,
                quantity, source, source_ref, missing_fields_json, context_json,
                resolution_action, resolution_json, created_at, resolved_at
         FROM inconsistencies
         WHERE 1=1",
    );

    let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    if let Some(status) = status {
        query.push_str(" AND status = ?");
        params.push(Box::new(status.as_str()));
    }

    if let Some(issue_type) = issue_type {
        query.push_str(" AND issue_type = ?");
        params.push(Box::new(issue_type.as_str()));
    }

    if let Some(ticker) = ticker {
        query.push_str(" AND ticker = ?");
        params.push(Box::new(ticker));
    }

    query.push_str(" ORDER BY id ASC");

    let params_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();
    let mut stmt = conn.prepare(&query)?;

    let rows = stmt.query_map(params_refs.as_slice(), |row| {
        Ok(Inconsistency {
            id: Some(row.get(0)?),
            issue_type: row
                .get::<_, String>(1)?
                .parse::<InconsistencyType>()
                .unwrap_or(InconsistencyType::MissingCostBasis),
            status: row
                .get::<_, String>(2)?
                .parse::<InconsistencyStatus>()
                .unwrap_or(InconsistencyStatus::Open),
            severity: row
                .get::<_, String>(3)?
                .parse::<InconsistencySeverity>()
                .unwrap_or(InconsistencySeverity::Warn),
            asset_id: row.get(4)?,
            transaction_id: row.get(5)?,
            ticker: row.get(6)?,
            trade_date: row.get(7)?,
            quantity: get_optional_decimal_value(row, 8)?,
            source: row.get(9)?,
            source_ref: row.get(10)?,
            missing_fields_json: row.get(11)?,
            context_json: row.get(12)?,
            resolution_action: row.get(13)?,
            resolution_json: row.get(14)?,
            created_at: row.get(15)?,
            resolved_at: row.get(16)?,
        })
    })?;

    let results = rows.collect::<Result<Vec<_>, _>>()?;
    Ok(results)
}

/// Mark inconsistency resolved (caller performs any data changes first)
pub fn resolve_inconsistency(
    conn: &Connection,
    id: i64,
    resolution_action: Option<&str>,
    resolution_json: Option<&str>,
) -> Result<()> {
    conn.execute(
        "UPDATE inconsistencies
         SET status = 'RESOLVED',
             resolution_action = ?1,
             resolution_json = ?2,
             resolved_at = CURRENT_TIMESTAMP
         WHERE id = ?3",
        params![resolution_action, resolution_json, id],
    )?;
    Ok(())
}

/// Mark inconsistency ignored with optional reason
pub fn ignore_inconsistency(conn: &Connection, id: i64, reason: Option<&str>) -> Result<()> {
    conn.execute(
        "UPDATE inconsistencies
         SET status = 'IGNORED',
             resolution_action = 'IGNORE',
             resolution_json = ?1,
             resolved_at = CURRENT_TIMESTAMP
         WHERE id = ?2",
        params![reason, id],
    )?;
    Ok(())
}

/// Get asset IDs and tickers that have open blocking inconsistencies
pub fn get_blocked_assets(conn: &Connection) -> Result<Vec<(i64, String)>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT i.asset_id, COALESCE(a.ticker, i.ticker)
         FROM inconsistencies i
         LEFT JOIN assets a ON i.asset_id = a.id
         WHERE i.status = 'OPEN' AND i.severity = 'BLOCKING'
         AND (i.asset_id IS NOT NULL OR i.ticker IS NOT NULL)",
    )?;

    let mut rows = stmt.query([])?;
    let mut result = Vec::new();

    while let Some(row) = rows.next()? {
        let asset_id: Option<i64> = row.get(0)?;
        let ticker: Option<String> = row.get(1)?;

        if let (Some(id), Some(t)) = (asset_id, ticker) {
            result.push((id, t));
        }
    }

    Ok(result)
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

/// Insert government bond rate history
pub fn insert_gov_bond_rate(conn: &Connection, rate: &GovBondRate) -> Result<i64> {
    conn.execute(
        "INSERT OR REPLACE INTO gov_bond_rates (
            asset_id, price_date, sell_rate, source
        ) VALUES (?1, ?2, ?3, ?4)",
        params![
            rate.asset_id,
            rate.price_date,
            rate.sell_rate.to_string(),
            rate.source.as_deref(),
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

/// Insert an asset rename (symbol-only change).
pub fn insert_asset_rename(conn: &Connection, rename: &AssetRename) -> Result<i64> {
    conn.execute(
        "INSERT INTO asset_renames (from_asset_id, to_asset_id, effective_date, notes)
         VALUES (?1, ?2, ?3, ?4)",
        params![
            rename.from_asset_id,
            rename.to_asset_id,
            rename.effective_date,
            rename.notes
        ],
    )?;

    Ok(conn.last_insert_rowid())
}

/// Get an asset rename by id.
pub fn get_asset_rename(conn: &Connection, id: i64) -> Result<Option<AssetRename>> {
    let mut stmt = conn.prepare(
        "SELECT id, from_asset_id, to_asset_id, effective_date, notes, created_at
         FROM asset_renames
         WHERE id = ?1",
    )?;

    let result = stmt
        .query_row(params![id], |row| {
            Ok(AssetRename {
                id: Some(row.get(0)?),
                from_asset_id: row.get(1)?,
                to_asset_id: row.get(2)?,
                effective_date: row.get(3)?,
                notes: row.get(4)?,
                created_at: row.get(5)?,
            })
        })
        .optional()?;

    Ok(result)
}

/// List renames with asset tickers for display, optionally filtered by ticker.
pub fn list_asset_renames_with_assets(
    conn: &Connection,
    ticker: Option<&str>,
) -> Result<Vec<(AssetRename, Asset, Asset)>> {
    let base_sql =
        "SELECT r.id, r.from_asset_id, r.to_asset_id, r.effective_date, r.notes, r.created_at,
                    af.id, af.ticker, af.asset_type, af.name, af.cnpj, af.created_at, af.updated_at,
                    at.id, at.ticker, at.asset_type, at.name, at.cnpj, at.created_at, at.updated_at
             FROM asset_renames r
             JOIN assets af ON r.from_asset_id = af.id
             JOIN assets at ON r.to_asset_id = at.id";

    let map_row = |row: &rusqlite::Row| {
        let rename = AssetRename {
            id: Some(row.get(0)?),
            from_asset_id: row.get(1)?,
            to_asset_id: row.get(2)?,
            effective_date: row.get(3)?,
            notes: row.get(4)?,
            created_at: row.get(5)?,
        };
        let from_asset = Asset {
            id: Some(row.get(6)?),
            ticker: row.get(7)?,
            asset_type: row
                .get::<_, String>(8)?
                .parse::<AssetType>()
                .unwrap_or(AssetType::Unknown),
            name: row.get(9)?,
            cnpj: row.get(10)?,
            created_at: row.get(11)?,
            updated_at: row.get(12)?,
        };
        let to_asset = Asset {
            id: Some(row.get(13)?),
            ticker: row.get(14)?,
            asset_type: row
                .get::<_, String>(15)?
                .parse::<AssetType>()
                .unwrap_or(AssetType::Unknown),
            name: row.get(16)?,
            cnpj: row.get(17)?,
            created_at: row.get(18)?,
            updated_at: row.get(19)?,
        };
        Ok((rename, from_asset, to_asset))
    };

    let rows = if let Some(t) = ticker {
        let sql = format!(
            "{} WHERE af.ticker = ?1 OR at.ticker = ?1 ORDER BY r.effective_date ASC",
            base_sql
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map(rusqlite::params![t], map_row)?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    } else {
        let sql = format!("{} ORDER BY r.effective_date ASC", base_sql);
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map(rusqlite::params![], map_row)?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };

    Ok(rows)
}

/// Delete an asset rename by id.
pub fn delete_asset_rename(conn: &Connection, id: i64) -> Result<usize> {
    let count = conn.execute("DELETE FROM asset_renames WHERE id = ?1", params![id])?;
    Ok(count)
}

/// Get renames where this asset is the target, effective up to a date.
pub fn get_asset_renames_as_target_up_to(
    conn: &Connection,
    asset_id: i64,
    as_of: NaiveDate,
) -> Result<Vec<AssetRename>> {
    let mut stmt = conn.prepare(
        "SELECT id, from_asset_id, to_asset_id, effective_date, notes, created_at
         FROM asset_renames
         WHERE to_asset_id = ?1 AND effective_date <= ?2
         ORDER BY effective_date ASC",
    )?;

    let results = stmt
        .query_map(params![asset_id, as_of], |row| {
            Ok(AssetRename {
                id: Some(row.get(0)?),
                from_asset_id: row.get(1)?,
                to_asset_id: row.get(2)?,
                effective_date: row.get(3)?,
                notes: row.get(4)?,
                created_at: row.get(5)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(results)
}

/// Check if an asset is a rename source effective on or before the provided date.
pub fn is_rename_source_asset(conn: &Connection, asset_id: i64, as_of: NaiveDate) -> Result<bool> {
    let exists = conn
        .query_row(
            "SELECT 1 FROM asset_renames WHERE from_asset_id = ?1 AND effective_date <= ?2 LIMIT 1",
            params![asset_id, as_of],
            |_| Ok(()),
        )
        .optional()?
        .is_some();

    Ok(exists)
}

/// Insert an asset exchange (spin-off or merger).
pub fn insert_asset_exchange(conn: &Connection, exchange: &AssetExchange) -> Result<i64> {
    conn.execute(
        "INSERT INTO asset_exchanges (
            event_type, from_asset_id, to_asset_id, effective_date,
            to_quantity, allocated_cost, cash_amount, source, notes
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            exchange.event_type.as_str(),
            exchange.from_asset_id,
            exchange.to_asset_id,
            exchange.effective_date,
            exchange.to_quantity.to_string(),
            exchange.allocated_cost.to_string(),
            exchange.cash_amount.to_string(),
            exchange.source,
            exchange.notes
        ],
    )?;

    Ok(conn.last_insert_rowid())
}

/// Get an asset exchange by id.
pub fn get_asset_exchange(conn: &Connection, id: i64) -> Result<Option<AssetExchange>> {
    let mut stmt = conn.prepare(
        "SELECT id, event_type, from_asset_id, to_asset_id, effective_date,
                to_quantity, allocated_cost, cash_amount, source, notes, created_at
         FROM asset_exchanges
         WHERE id = ?1",
    )?;

    let result = stmt
        .query_row(params![id], |row| {
            Ok(AssetExchange {
                id: Some(row.get(0)?),
                event_type: row
                    .get::<_, String>(1)?
                    .parse::<AssetExchangeType>()
                    .unwrap_or(AssetExchangeType::Spinoff),
                from_asset_id: row.get(2)?,
                to_asset_id: row.get(3)?,
                effective_date: row.get(4)?,
                to_quantity: get_decimal_value(row, 5)?,
                allocated_cost: get_decimal_value(row, 6)?,
                cash_amount: get_decimal_value(row, 7)?,
                source: row.get(8)?,
                notes: row.get(9)?,
                created_at: row.get(10)?,
            })
        })
        .optional()?;

    Ok(result)
}

/// List exchanges with asset tickers for display, optionally filtered by ticker.
pub fn list_asset_exchanges_with_assets(
    conn: &Connection,
    ticker: Option<&str>,
) -> Result<Vec<(AssetExchange, Asset, Asset)>> {
    let base_sql = "SELECT e.id, e.event_type, e.from_asset_id, e.to_asset_id, e.effective_date,
                    e.to_quantity, e.allocated_cost, e.cash_amount, e.source, e.notes, e.created_at,
                    af.id, af.ticker, af.asset_type, af.name, af.cnpj, af.created_at, af.updated_at,
                    at.id, at.ticker, at.asset_type, at.name, at.cnpj, at.created_at, at.updated_at
             FROM asset_exchanges e
             JOIN assets af ON e.from_asset_id = af.id
             JOIN assets at ON e.to_asset_id = at.id";

    let map_row = |row: &rusqlite::Row| {
        let exchange = AssetExchange {
            id: Some(row.get(0)?),
            event_type: row
                .get::<_, String>(1)?
                .parse::<AssetExchangeType>()
                .unwrap_or(AssetExchangeType::Spinoff),
            from_asset_id: row.get(2)?,
            to_asset_id: row.get(3)?,
            effective_date: row.get(4)?,
            to_quantity: get_decimal_value(row, 5)?,
            allocated_cost: get_decimal_value(row, 6)?,
            cash_amount: get_decimal_value(row, 7)?,
            source: row.get(8)?,
            notes: row.get(9)?,
            created_at: row.get(10)?,
        };
        let from_asset = Asset {
            id: Some(row.get(11)?),
            ticker: row.get(12)?,
            asset_type: row
                .get::<_, String>(13)?
                .parse::<AssetType>()
                .unwrap_or(AssetType::Unknown),
            name: row.get(14)?,
            cnpj: row.get(15)?,
            created_at: row.get(16)?,
            updated_at: row.get(17)?,
        };
        let to_asset = Asset {
            id: Some(row.get(18)?),
            ticker: row.get(19)?,
            asset_type: row
                .get::<_, String>(20)?
                .parse::<AssetType>()
                .unwrap_or(AssetType::Unknown),
            name: row.get(21)?,
            cnpj: row.get(22)?,
            created_at: row.get(23)?,
            updated_at: row.get(24)?,
        };
        Ok((exchange, from_asset, to_asset))
    };

    let rows = if let Some(t) = ticker {
        let sql = format!(
            "{} WHERE af.ticker = ?1 OR at.ticker = ?1 ORDER BY e.effective_date ASC",
            base_sql
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map(rusqlite::params![t], map_row)?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    } else {
        let sql = format!("{} ORDER BY e.effective_date ASC", base_sql);
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map(rusqlite::params![], map_row)?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };

    Ok(rows)
}

/// Delete an asset exchange by id.
pub fn delete_asset_exchange(conn: &Connection, id: i64) -> Result<usize> {
    let count = conn.execute("DELETE FROM asset_exchanges WHERE id = ?1", params![id])?;
    Ok(count)
}

/// Get exchanges where this asset is the source, effective up to a date.
pub fn get_asset_exchanges_as_source_up_to(
    conn: &Connection,
    asset_id: i64,
    as_of: NaiveDate,
) -> Result<Vec<AssetExchange>> {
    let mut stmt = conn.prepare(
        "SELECT id, event_type, from_asset_id, to_asset_id, effective_date,
                to_quantity, allocated_cost, cash_amount, source, notes, created_at
         FROM asset_exchanges
         WHERE from_asset_id = ?1 AND effective_date <= ?2
         ORDER BY effective_date ASC",
    )?;

    let results = stmt
        .query_map(params![asset_id, as_of], |row| {
            Ok(AssetExchange {
                id: Some(row.get(0)?),
                event_type: row
                    .get::<_, String>(1)?
                    .parse::<AssetExchangeType>()
                    .unwrap_or(AssetExchangeType::Spinoff),
                from_asset_id: row.get(2)?,
                to_asset_id: row.get(3)?,
                effective_date: row.get(4)?,
                to_quantity: get_decimal_value(row, 5)?,
                allocated_cost: get_decimal_value(row, 6)?,
                cash_amount: get_decimal_value(row, 7)?,
                source: row.get(8)?,
                notes: row.get(9)?,
                created_at: row.get(10)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(results)
}

/// Get exchanges where this asset is the target, effective up to a date.
pub fn get_asset_exchanges_as_target_up_to(
    conn: &Connection,
    asset_id: i64,
    as_of: NaiveDate,
) -> Result<Vec<AssetExchange>> {
    let mut stmt = conn.prepare(
        "SELECT id, event_type, from_asset_id, to_asset_id, effective_date,
                to_quantity, allocated_cost, cash_amount, source, notes, created_at
         FROM asset_exchanges
         WHERE to_asset_id = ?1 AND effective_date <= ?2
         ORDER BY effective_date ASC",
    )?;

    let results = stmt
        .query_map(params![asset_id, as_of], |row| {
            Ok(AssetExchange {
                id: Some(row.get(0)?),
                event_type: row
                    .get::<_, String>(1)?
                    .parse::<AssetExchangeType>()
                    .unwrap_or(AssetExchangeType::Spinoff),
                from_asset_id: row.get(2)?,
                to_asset_id: row.get(3)?,
                effective_date: row.get(4)?,
                to_quantity: get_decimal_value(row, 5)?,
                allocated_cost: get_decimal_value(row, 6)?,
                cash_amount: get_decimal_value(row, 7)?,
                source: row.get(8)?,
                notes: row.get(9)?,
                created_at: row.get(10)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(results)
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
pub(crate) fn get_optional_decimal_value(
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
            asset_id, action_type, event_date, ex_date, quantity_adjustment, source, notes
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            action.asset_id,
            action.action_type.as_str(),
            action.event_date,
            action.ex_date,
            action.quantity_adjustment.to_string(),
            action.source,
            action.notes,
        ],
    )?;

    Ok(conn.last_insert_rowid())
}

/// List corporate actions with optional ticker filter
pub fn list_corporate_actions(
    conn: &Connection,
    ticker: Option<&str>,
) -> Result<Vec<(CorporateAction, Asset)>> {
    let query = if ticker.is_some() {
        "SELECT ca.id, ca.asset_id, ca.action_type, ca.event_date, ca.ex_date,
                ca.quantity_adjustment, ca.source, ca.notes, ca.created_at,
                a.id, a.ticker, a.asset_type, a.name, a.cnpj, a.created_at, a.updated_at
         FROM corporate_actions ca
         JOIN assets a ON ca.asset_id = a.id
         WHERE a.ticker = ?1
         ORDER BY ca.ex_date ASC"
    } else {
        "SELECT ca.id, ca.asset_id, ca.action_type, ca.event_date, ca.ex_date,
                ca.quantity_adjustment, ca.source, ca.notes, ca.created_at,
                a.id, a.ticker, a.asset_type, a.name, a.cnpj, a.created_at, a.updated_at
         FROM corporate_actions ca
         JOIN assets a ON ca.asset_id = a.id
         ORDER BY ca.ex_date ASC"
    };

    let mut stmt = conn.prepare(query)?;

    let parse_row = |row: &rusqlite::Row| {
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
                quantity_adjustment: {
                    use rusqlite::types::ValueRef;
                    match row.get_ref(5)? {
                        ValueRef::Text(bytes) => {
                            let s = std::str::from_utf8(bytes).unwrap_or("0");
                            Decimal::from_str(s).unwrap_or(Decimal::ZERO)
                        }
                        ValueRef::Integer(i) => Decimal::from(i),
                        ValueRef::Real(f) => Decimal::try_from(f).unwrap_or(Decimal::ZERO),
                        _ => Decimal::ZERO,
                    }
                },
                source: row.get(6)?,
                notes: row.get(7)?,
                created_at: row.get(8)?,
            },
            Asset {
                id: Some(row.get(9)?),
                ticker: row.get(10)?,
                asset_type: row
                    .get::<_, String>(11)?
                    .parse::<AssetType>()
                    .unwrap_or(AssetType::Unknown),
                name: row.get(12)?,
                cnpj: row.get(13)?,
                created_at: row.get(14)?,
                updated_at: row.get(15)?,
            },
        ))
    };

    let results = if let Some(t) = ticker {
        stmt.query_map([t], parse_row)?
            .collect::<Result<Vec<_>, _>>()?
    } else {
        stmt.query_map([], parse_row)?
            .collect::<Result<Vec<_>, _>>()?
    };

    Ok(results)
}

/// Get a corporate action by id with asset info.
pub fn get_corporate_action(
    conn: &Connection,
    id: i64,
) -> Result<Option<(CorporateAction, Asset)>> {
    let mut stmt = conn.prepare(
        "SELECT ca.id, ca.asset_id, ca.action_type, ca.event_date, ca.ex_date,
                ca.quantity_adjustment, ca.source, ca.notes, ca.created_at,
                a.id, a.ticker, a.asset_type, a.name, a.cnpj, a.created_at, a.updated_at
         FROM corporate_actions ca
         JOIN assets a ON ca.asset_id = a.id
         WHERE ca.id = ?1",
    )?;

    let result = stmt
        .query_row(params![id], |row| {
            let action = CorporateAction {
                id: Some(row.get(0)?),
                asset_id: row.get(1)?,
                action_type: row
                    .get::<_, String>(2)?
                    .parse::<CorporateActionType>()
                    .unwrap_or(CorporateActionType::Split),
                event_date: row.get(3)?,
                ex_date: row.get(4)?,
                quantity_adjustment: get_decimal_value(row, 5)?,
                source: row.get(6)?,
                notes: row.get(7)?,
                created_at: row.get(8)?,
            };
            let asset = Asset {
                id: Some(row.get(9)?),
                ticker: row.get(10)?,
                asset_type: row
                    .get::<_, String>(11)?
                    .parse::<AssetType>()
                    .unwrap_or(AssetType::Unknown),
                name: row.get(12)?,
                cnpj: row.get(13)?,
                created_at: row.get(14)?,
                updated_at: row.get(15)?,
            };
            Ok((action, asset))
        })
        .optional()?;

    Ok(result)
}

/// Delete a corporate action by id.
pub fn delete_corporate_action(conn: &Connection, id: i64) -> Result<usize> {
    let count = conn.execute("DELETE FROM corporate_actions WHERE id = ?1", params![id])?;
    Ok(count)
}

/// Delete all transactions from a specific source with trade_date >= the given date
/// Used for force-reimport functionality
pub fn delete_transactions_from_source_after_date(
    conn: &Connection,
    source: &str,
    from_date: NaiveDate,
) -> Result<usize> {
    let count = conn.execute(
        "DELETE FROM transactions WHERE source = ?1 AND trade_date >= ?2",
        params![source, from_date],
    )?;
    Ok(count)
}

/// Delete all corporate actions from a specific source with ex_date >= the given date
/// Used for force-reimport functionality
pub fn delete_corporate_actions_from_source_after_date(
    conn: &Connection,
    source: &str,
    from_date: NaiveDate,
) -> Result<usize> {
    let count = conn.execute(
        "DELETE FROM corporate_actions WHERE source = ?1 AND ex_date >= ?2",
        params![source, from_date],
    )?;
    Ok(count)
}

/// Delete all income events from a specific source with event_date >= the given date
/// Used for force-reimport functionality
pub fn delete_income_events_from_source_after_date(
    conn: &Connection,
    source: &str,
    from_date: NaiveDate,
) -> Result<usize> {
    let count = conn.execute(
        "DELETE FROM income_events WHERE source = ?1 AND event_date >= ?2",
        params![source, from_date],
    )?;
    Ok(count)
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
                a.id, a.ticker, a.asset_type, a.name, a.cnpj, a.created_at, a.updated_at
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

    sql.push_str(" ORDER BY ie.event_date ASC, a.ticker ASC");

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
                withholding_tax: get_optional_decimal_value(row, 7)?.unwrap_or(Decimal::ZERO),
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
                    .unwrap_or(AssetType::Unknown),
                name: row.get(15)?,
                cnpj: row.get(16)?,
                created_at: row.get(17)?,
                updated_at: row.get(18)?,
            };
            Ok((event, asset))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(results)
}

/// Get amortization (capital return) events for a specific asset, ordered ASC by event_date.
pub fn get_amortizations_for_asset(
    conn: &Connection,
    asset_id: i64,
    from_date: Option<NaiveDate>,
    to_date: Option<NaiveDate>,
) -> Result<Vec<IncomeEvent>> {
    let mut sql = String::from(
        "SELECT id, asset_id, event_date, ex_date, event_type, amount_per_quota, total_amount, \
                withholding_tax, is_quota_pre_2026, source, notes, created_at\n         FROM income_events\n         WHERE asset_id = ? AND event_type = 'AMORTIZATION'",
    );

    let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(asset_id)];

    if let Some(from) = from_date {
        sql.push_str(" AND event_date >= ?");
        params.push(Box::new(from));
    }
    if let Some(to) = to_date {
        sql.push_str(" AND event_date <= ?");
        params.push(Box::new(to));
    }

    sql.push_str(" ORDER BY event_date ASC");

    let mut stmt = conn.prepare(&sql)?;
    let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();

    let events = stmt
        .query_map(param_refs.as_slice(), |row| {
            Ok(IncomeEvent {
                id: Some(row.get(0)?),
                asset_id: row.get(1)?,
                event_date: row.get(2)?,
                ex_date: row.get(3)?,
                event_type: row
                    .get::<_, String>(4)?
                    .parse::<IncomeEventType>()
                    .unwrap_or(IncomeEventType::Amortization),
                amount_per_quota: get_decimal_value(row, 5)?,
                total_amount: get_decimal_value(row, 6)?,
                withholding_tax: get_optional_decimal_value(row, 7)?.unwrap_or(Decimal::ZERO),
                is_quota_pre_2026: row.get(8)?,
                source: row.get(9)?,
                notes: row.get(10)?,
                created_at: row.get(11)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(events)
}

/// Get all assets (for batch price updates)
pub fn get_all_assets(conn: &Connection) -> Result<Vec<Asset>> {
    let mut stmt = conn.prepare(
        "SELECT id, ticker, asset_type, name, cnpj, created_at, updated_at FROM assets ORDER BY ticker",
    )?;

    let assets = stmt
        .query_map([], |row| {
            Ok(Asset {
                id: Some(row.get(0)?),
                ticker: row.get(1)?,
                asset_type: row
                    .get::<_, String>(2)?
                    .parse::<AssetType>()
                    .unwrap_or(AssetType::Unknown),
                name: row.get(3)?,
                cnpj: row.get(4)?,
                created_at: row.get(5)?,
                updated_at: row.get(6)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(assets)
}

/// Get assets with a specific asset type
pub fn list_assets_by_type(conn: &Connection, asset_type: AssetType) -> Result<Vec<Asset>> {
    let mut stmt = conn.prepare(
        "SELECT id, ticker, asset_type, name, cnpj, created_at, updated_at FROM assets WHERE asset_type = ? ORDER BY ticker",
    )?;

    let assets = stmt
        .query_map([asset_type.as_str()], |row| {
            Ok(Asset {
                id: Some(row.get(0)?),
                ticker: row.get(1)?,
                asset_type: row
                    .get::<_, String>(2)?
                    .parse::<AssetType>()
                    .unwrap_or(AssetType::Unknown),
                name: row.get(3)?,
                cnpj: row.get(4)?,
                created_at: row.get(5)?,
                updated_at: row.get(6)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(assets)
}

/// Update asset type for a ticker
pub fn update_asset_type(conn: &Connection, ticker: &str, asset_type: &AssetType) -> Result<()> {
    let updated = conn.execute(
        "UPDATE assets SET asset_type = ?1, updated_at = CURRENT_TIMESTAMP WHERE ticker = ?2",
        params![asset_type.as_str(), ticker.to_uppercase()],
    )?;

    if updated == 0 {
        return Err(anyhow::anyhow!("Ticker {} not found in assets", ticker));
    }

    Ok(())
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
        "SELECT DISTINCT a.id, a.ticker, a.name, a.cnpj, a.asset_type, a.created_at, a.updated_at
         FROM assets a 
         INNER JOIN transactions t ON a.id = t.asset_id
         ORDER BY a.ticker",
    )?;

    let assets = stmt.query_map([], |row| {
        Ok(Asset {
            id: Some(row.get(0)?),
            ticker: row.get(1)?,
            name: row.get(2)?,
            cnpj: row.get(3)?,
            asset_type: row.get::<_, String>(4)?.parse().unwrap(),
            created_at: row.get(5)?,
            updated_at: row.get(6)?,
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

    #[test]
    fn test_asset_exists() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let db_path = tmp.path().join("test.db");
        init_database(Some(db_path.clone()))?;
        let conn = Connection::open(&db_path)?;

        // Initially absent
        assert!(!asset_exists(&conn, "NOSUCH")?);

        // Create asset
        let id = upsert_asset(&conn, "EXIST1", &AssetType::Stock, None)?;
        assert!(id > 0);

        // Now exists
        assert!(asset_exists(&conn, "EXIST1")?);

        Ok(())
    }
}
