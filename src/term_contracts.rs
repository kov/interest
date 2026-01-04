//! Term Contract (Compra a Termo) Handling
//!
//! Term contracts in Brazil work as follows:
//! 1. Purchase: You buy TICKER3T (e.g., ANIM3T) - term contract
//! 2. Liquidation: Contract expires, you receive TICKER3 (e.g., ANIM3) shares
//! 3. Cost Basis: The cost from ANIM3T transfers to ANIM3
//!
//! This module handles matching liquidations to purchases and tracking the cost basis transfer.

use anyhow::Result;
use chrono::NaiveDate;
use rust_decimal::Decimal;
use rusqlite::Connection;
use std::str::FromStr;
use tracing::{info, warn};

use crate::db::models::{Transaction, TransactionType};

/// Helper to read Decimal from SQLite (handles both INTEGER, REAL and TEXT)
fn get_decimal_value(row: &rusqlite::Row, idx: usize) -> Result<Decimal, rusqlite::Error> {
    use rusqlite::types::ValueRef;

    match row.get_ref(idx)? {
        ValueRef::Text(bytes) => {
            let s = std::str::from_utf8(bytes)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            Decimal::from_str(s)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
        }
        ValueRef::Integer(i) => Ok(Decimal::from(i)),
        ValueRef::Real(f) => Decimal::try_from(f)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e))),
        _ => Err(rusqlite::Error::InvalidColumnType(
            idx,
            "decimal".to_string(),
            rusqlite::types::Type::Null,
        )),
    }
}

/// Check if a ticker is a term contract (ends with 'T')
pub fn is_term_contract(ticker: &str) -> bool {
    ticker.len() >= 5 && ticker.ends_with('T')
        && ticker.chars().rev().nth(1).map(|c| c.is_numeric()).unwrap_or(false)
}

/// Get the base ticker from a term contract ticker
/// Example: "ANIM3T" -> "ANIM3"
pub fn get_base_ticker(term_ticker: &str) -> String {
    if is_term_contract(term_ticker) {
        term_ticker[..term_ticker.len() - 1].to_string()
    } else {
        term_ticker.to_string()
    }
}

/// Get the term ticker from a base ticker
/// Example: "ANIM3" -> "ANIM3T"
pub fn get_term_ticker(base_ticker: &str) -> String {
    format!("{}T", base_ticker)
}

/// Match a term liquidation transaction to its original purchase(s)
///
/// Returns a list of (purchase_transaction, liquidated_quantity) pairs
pub fn match_liquidation_to_purchases(
    conn: &Connection,
    base_ticker: &str,
    liquidation_date: NaiveDate,
    liquidation_quantity: Decimal,
) -> Result<Vec<(Transaction, Decimal)>> {
    let term_ticker = get_term_ticker(base_ticker);

    // Get the asset ID for the term ticker
    let term_asset_id: Option<i64> = conn
        .query_row(
            "SELECT id FROM assets WHERE ticker = ?1",
            [&term_ticker],
            |row| row.get(0),
        )
        .ok();

    let term_asset_id = match term_asset_id {
        Some(id) => id,
        None => {
            warn!("No term contract asset found for {}", term_ticker);
            return Ok(Vec::new());
        }
    };

    // Get all purchases of the term contract before the liquidation date
    let mut stmt = conn.prepare(
        "SELECT id, asset_id, transaction_type, trade_date, settlement_date,
                quantity, price_per_unit, total_cost, fees, is_day_trade,
                quota_issuance_date, notes, source, created_at
         FROM transactions
         WHERE asset_id = ?1
           AND transaction_type = 'BUY'
           AND trade_date <= ?2
         ORDER BY trade_date ASC",
    )?;

    let transactions = stmt
        .query_map(rusqlite::params![term_asset_id, liquidation_date], |row| {
            Ok(Transaction {
                id: Some(row.get(0)?),
                asset_id: row.get(1)?,
                transaction_type: TransactionType::from_str(&row.get::<_, String>(2)?)
                    .ok_or_else(|| rusqlite::Error::InvalidQuery)?,
                trade_date: row.get(3)?,
                settlement_date: row.get(4)?,
                quantity: get_decimal_value(row, 5)?,
                price_per_unit: get_decimal_value(row, 6)?,
                total_cost: get_decimal_value(row, 7)?,
                fees: get_decimal_value(row, 8)?,
                is_day_trade: row.get(9)?,
                quota_issuance_date: row.get(10)?,
                notes: row.get(11)?,
                source: row.get(12)?,
                created_at: row.get(13)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    // Match using FIFO (First-In-First-Out)
    let mut matches = Vec::new();
    let mut remaining = liquidation_quantity;

    for purchase in transactions {
        if remaining <= Decimal::ZERO {
            break;
        }

        // Check how much of this purchase hasn't been liquidated yet
        // (This would require tracking previous liquidations, but for MVP we assume all is available)
        let available = purchase.quantity;
        let matched = available.min(remaining);

        matches.push((purchase, matched));
        remaining -= matched;
    }

    if remaining > Decimal::ZERO {
        warn!(
            "Liquidation of {} {} shares exceeds available term purchases ({} remaining unmatched)",
            base_ticker, liquidation_quantity, remaining
        );
    }

    Ok(matches)
}

/// Process all term contract liquidations and create corresponding transactions
///
/// This scans for liquidation transactions and creates linked transactions showing
/// the cost basis transfer from TICKERT to TICKER
pub fn process_term_liquidations(conn: &Connection) -> Result<usize> {
    info!("Processing term contract liquidations...");

    // Find all transactions that are term liquidations
    // (identified by notes containing "Term contract liquidation")
    let mut stmt = conn.prepare(
        "SELECT t.id, a.ticker, t.trade_date, t.quantity, t.price_per_unit, t.notes
         FROM transactions t
         JOIN assets a ON t.asset_id = a.id
         WHERE t.notes LIKE '%Term contract liquidation%'
           AND t.transaction_type = 'BUY'",
    )?;

    let liquidations: Vec<(i64, String, NaiveDate, Decimal, Decimal, Option<String>)> = stmt
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                get_decimal_value(row, 3)?,
                get_decimal_value(row, 4)?,
                row.get(5)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut processed = 0;

    for (_liquidation_id, base_ticker, liquidation_date, quantity, _price, _notes) in liquidations {
        // Match this liquidation to term purchases
        let matches = match_liquidation_to_purchases(conn, &base_ticker, liquidation_date, quantity)?;

        if matches.is_empty() {
            warn!(
                "No matching term purchases found for liquidation of {} on {}",
                base_ticker, liquidation_date
            );
            continue;
        }

        // Calculate weighted average cost from matched purchases
        let mut total_cost = Decimal::ZERO;
        let mut total_qty = Decimal::ZERO;

        for (purchase, matched_qty) in &matches {
            let cost = purchase.price_per_unit * matched_qty;
            total_cost += cost;
            total_qty += matched_qty;
        }

        let avg_cost = if total_qty > Decimal::ZERO {
            total_cost / total_qty
        } else {
            Decimal::ZERO
        };

        info!(
            "Matched liquidation of {} {} shares to {} term purchase(s), avg cost: R$ {:.2}",
            base_ticker,
            quantity,
            matches.len(),
            avg_cost
        );

        processed += 1;
    }

    info!("Processed {} term contract liquidations", processed);
    Ok(processed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_term_contract() {
        assert!(is_term_contract("ANIM3T"));
        assert!(is_term_contract("PETR4T"));
        assert!(is_term_contract("SHUL4T"));

        assert!(!is_term_contract("ANIM3"));
        assert!(!is_term_contract("PETR4"));
        assert!(!is_term_contract("TEST"));  // Doesn't end in digit+T
        assert!(!is_term_contract("T"));      // Too short
    }

    #[test]
    fn test_get_base_ticker() {
        assert_eq!(get_base_ticker("ANIM3T"), "ANIM3");
        assert_eq!(get_base_ticker("PETR4T"), "PETR4");
        assert_eq!(get_base_ticker("ANIM3"), "ANIM3");  // Already base ticker
    }

    #[test]
    fn test_get_term_ticker() {
        assert_eq!(get_term_ticker("ANIM3"), "ANIM3T");
        assert_eq!(get_term_ticker("PETR4"), "PETR4T");
    }
}
