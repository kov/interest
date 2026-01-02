// Corporate actions module - Split/bonus adjustment engine

use anyhow::Result;
use rusqlite::Connection;
use rust_decimal::Decimal;
use std::str::FromStr;
use tracing::info;

use crate::db::{Asset, CorporateAction, Transaction, TransactionType};

/// Get unapplied corporate actions for an asset (or all assets if None)
pub fn get_unapplied_actions(
    conn: &Connection,
    asset_id_filter: Option<i64>,
) -> Result<Vec<CorporateAction>> {
    let query = if asset_id_filter.is_some() {
        "SELECT id, asset_id, action_type, event_date, ex_date, ratio_from, ratio_to,
                applied, source, notes, created_at
         FROM corporate_actions
         WHERE applied = 0 AND asset_id = ?1
         ORDER BY ex_date ASC"
    } else {
        "SELECT id, asset_id, action_type, event_date, ex_date, ratio_from, ratio_to,
                applied, source, notes, created_at
         FROM corporate_actions
         WHERE applied = 0
         ORDER BY ex_date ASC"
    };

    let mut stmt = conn.prepare(query)?;

    let actions = if let Some(asset_id) = asset_id_filter {
        stmt.query_map([asset_id], |row| {
            Ok(CorporateAction {
                id: Some(row.get(0)?),
                asset_id: row.get(1)?,
                action_type: crate::db::CorporateActionType::from_str(&row.get::<_, String>(2)?)
                    .unwrap_or(crate::db::CorporateActionType::Split),
                event_date: row.get(3)?,
                ex_date: row.get(4)?,
                ratio_from: row.get(5)?,
                ratio_to: row.get(6)?,
                applied: row.get(7)?,
                source: row.get(8)?,
                notes: row.get(9)?,
                created_at: row.get(10)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?
    } else {
        stmt.query_map([], |row| {
            Ok(CorporateAction {
                id: Some(row.get(0)?),
                asset_id: row.get(1)?,
                action_type: crate::db::CorporateActionType::from_str(&row.get::<_, String>(2)?)
                    .unwrap_or(crate::db::CorporateActionType::Split),
                event_date: row.get(3)?,
                ex_date: row.get(4)?,
                ratio_from: row.get(5)?,
                ratio_to: row.get(6)?,
                applied: row.get(7)?,
                source: row.get(8)?,
                notes: row.get(9)?,
                created_at: row.get(10)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?
    };

    Ok(actions)
}

/// Apply a corporate action to all transactions before the ex-date
///
/// This adjusts quantities and prices while keeping total cost unchanged.
/// For splits:    new_qty = old_qty × (ratio_to / ratio_from)
///                new_price = old_price × (ratio_from / ratio_to)
///
/// Returns the number of transactions adjusted.
pub fn apply_corporate_action(
    conn: &Connection,
    action: &CorporateAction,
    asset: &Asset,
) -> Result<usize> {
    info!(
        "Applying {} for {} (ratio {}:{})",
        action.action_type.as_str(),
        asset.ticker,
        action.ratio_from,
        action.ratio_to
    );

    // Get all transactions for this asset before the ex-date
    let mut stmt = conn.prepare(
        "SELECT id, asset_id, transaction_type, trade_date, settlement_date,
                quantity, price_per_unit, total_cost, fees, is_day_trade,
                quota_issuance_date, notes, source, created_at
         FROM transactions
         WHERE asset_id = ?1 AND trade_date < ?2
         ORDER BY trade_date ASC"
    )?;

    let transactions = stmt
        .query_map(rusqlite::params![action.asset_id, action.ex_date], |row| {
            Ok(Transaction {
                id: Some(row.get(0)?),
                asset_id: row.get(1)?,
                transaction_type: TransactionType::from_str(&row.get::<_, String>(2)?)
                    .unwrap_or(TransactionType::Buy),
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

    if transactions.is_empty() {
        info!("No transactions found before ex-date {}", action.ex_date);
        // Mark as applied anyway
        mark_action_as_applied(conn, action.id.unwrap())?;
        return Ok(0);
    }

    let ratio_from = Decimal::from(action.ratio_from);
    let ratio_to = Decimal::from(action.ratio_to);

    // Adjust each transaction
    let mut adjusted_count = 0;
    for tx in transactions {
        let new_quantity = tx.quantity * ratio_to / ratio_from;
        let new_price = tx.price_per_unit * ratio_from / ratio_to;

        // Verify total cost remains unchanged (within rounding tolerance)
        let old_total = tx.quantity * tx.price_per_unit;
        let new_total = new_quantity * new_price;
        let diff = (new_total - old_total).abs();
        let tolerance = Decimal::from_str("0.01").unwrap(); // 1 cent tolerance

        if diff > tolerance {
            tracing::warn!(
                "Total cost changed for transaction {}: {} -> {} (diff: {})",
                tx.id.unwrap_or(0),
                old_total,
                new_total,
                diff
            );
        }

        // Update transaction in database
        conn.execute(
            "UPDATE transactions
             SET quantity = ?1, price_per_unit = ?2
             WHERE id = ?3",
            rusqlite::params![
                new_quantity.to_string(),
                new_price.to_string(),
                tx.id.unwrap()
            ],
        )?;

        adjusted_count += 1;
    }

    // Mark the corporate action as applied
    mark_action_as_applied(conn, action.id.unwrap())?;

    info!(
        "Successfully adjusted {} transactions for {} {}",
        adjusted_count,
        asset.ticker,
        action.action_type.as_str()
    );

    Ok(adjusted_count)
}

/// Mark a corporate action as applied
fn mark_action_as_applied(conn: &Connection, action_id: i64) -> Result<()> {
    conn.execute(
        "UPDATE corporate_actions SET applied = 1 WHERE id = ?1",
        [action_id],
    )?;
    Ok(())
}

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
            rusqlite::types::Type::Null
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_calculation() {
        // 1:2 split - each share becomes 2
        let old_qty = Decimal::from(100);
        let old_price = Decimal::from(50);
        let ratio_from = Decimal::from(1);
        let ratio_to = Decimal::from(2);

        let new_qty = old_qty * ratio_to / ratio_from;
        let new_price = old_price * ratio_from / ratio_to;

        assert_eq!(new_qty, Decimal::from(200));
        assert_eq!(new_price, Decimal::from(25));
        assert_eq!(old_qty * old_price, new_qty * new_price); // Total cost unchanged
    }

    #[test]
    fn test_reverse_split_calculation() {
        // 10:1 reverse split - 10 shares become 1
        let old_qty = Decimal::from(100);
        let old_price = Decimal::from(50);
        let ratio_from = Decimal::from(10);
        let ratio_to = Decimal::from(1);

        let new_qty = old_qty * ratio_to / ratio_from;
        let new_price = old_price * ratio_from / ratio_to;

        assert_eq!(new_qty, Decimal::from(10));
        assert_eq!(new_price, Decimal::from(500));
        assert_eq!(old_qty * old_price, new_qty * new_price); // Total cost unchanged
    }
}
