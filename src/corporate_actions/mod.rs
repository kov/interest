// Corporate actions module - Split/bonus adjustment engine

use anyhow::Result;
use rusqlite::Connection;
use rust_decimal::Decimal;
use std::str::FromStr;
use tracing::info;

use crate::db::{Asset, CorporateAction, CorporateActionType, Transaction, TransactionType};

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
/// This function is idempotent - safe to call multiple times. It only adjusts
/// transactions that haven't been adjusted by this action yet.
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

    let action_id = action
        .id
        .ok_or_else(|| anyhow::anyhow!("Corporate action must have an ID"))?;

    // Get all transactions for this asset before the ex-date that haven't been adjusted yet
    let mut stmt = conn.prepare(
        "SELECT t.id, t.asset_id, t.transaction_type, t.trade_date, t.settlement_date,
                t.quantity, t.price_per_unit, t.total_cost, t.fees, t.is_day_trade,
                t.quota_issuance_date, t.notes, t.source, t.created_at
         FROM transactions t
         WHERE t.asset_id = ?1 AND t.trade_date < ?2
           AND NOT EXISTS (
               SELECT 1 FROM corporate_action_adjustments caa
               WHERE caa.action_id = ?3 AND caa.transaction_id = t.id
           )
         ORDER BY t.trade_date ASC",
    )?;

    let transactions = stmt
        .query_map(
            rusqlite::params![action.asset_id, action.ex_date, action_id],
            |row| {
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
            },
        )?
        .collect::<Result<Vec<_>, _>>()?;

    if transactions.is_empty() {
        info!(
            "No unadjusted transactions found before ex-date {}",
            action.ex_date
        );
        // Mark as applied anyway
        mark_action_as_applied(conn, action_id)?;
        return Ok(0);
    }

    let ratio_from = Decimal::from(action.ratio_from);
    let ratio_to = Decimal::from(action.ratio_to);

    if action.action_type == crate::db::CorporateActionType::Bonus {
        use rust_decimal::RoundingStrategy;

        let net_qty: Decimal = transactions
            .iter()
            .map(|tx| match tx.transaction_type {
                TransactionType::Buy => tx.quantity,
                TransactionType::Sell => -tx.quantity,
            })
            .sum();
        let new_total_qty = net_qty * ratio_to / ratio_from;
        let bonus_qty = new_total_qty - net_qty;
        let integer_bonus = bonus_qty.round_dp_with_strategy(0, RoundingStrategy::ToZero);
        let fractional_bonus = bonus_qty - integer_bonus;

        if integer_bonus > Decimal::ZERO {
            let mut notes = format!(
                "Bonus shares from {} (ratio {}:{})",
                action.action_type.as_str(),
                action.ratio_from,
                action.ratio_to
            );
            if fractional_bonus > Decimal::ZERO {
                notes = format!("{}; fractional remainder: {}", notes, fractional_bonus);
            }

            let bonus_tx = Transaction {
                id: None,
                asset_id: action.asset_id,
                transaction_type: TransactionType::Buy,
                trade_date: action.ex_date,
                settlement_date: Some(action.ex_date),
                quantity: integer_bonus,
                price_per_unit: Decimal::ZERO,
                total_cost: Decimal::ZERO,
                fees: Decimal::ZERO,
                is_day_trade: false,
                quota_issuance_date: None,
                notes: Some(notes),
                source: "CORPORATE_ACTION".to_string(),
                created_at: chrono::Utc::now(),
            };
            crate::db::insert_transaction(conn, &bonus_tx)?;
        } else {
            info!(
                "Bonus action {} for {} resulted in zero bonus quantity",
                action.id.unwrap_or(0),
                asset.ticker
            );
        }

        for tx in &transactions {
            let tx_id = tx
                .id
                .ok_or_else(|| anyhow::anyhow!("Transaction must have an ID"))?;
            conn.execute(
                "INSERT INTO corporate_action_adjustments
                 (action_id, transaction_id, old_quantity, new_quantity, old_price, new_price)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    action_id,
                    tx_id,
                    tx.quantity.to_string(),
                    tx.quantity.to_string(),
                    tx.price_per_unit.to_string(),
                    tx.price_per_unit.to_string()
                ],
            )?;
        }

        mark_action_as_applied(conn, action_id)?;
        return Ok(transactions.len());
    }

    // Adjust each transaction
    let mut adjusted_count = 0;
    for tx in transactions {
        let tx_id = tx
            .id
            .ok_or_else(|| anyhow::anyhow!("Transaction must have an ID"))?;

        let old_quantity = tx.quantity;
        let old_price = tx.price_per_unit;
        let old_total = tx.total_cost;

        let (new_quantity, new_price, new_total) = match action.action_type {
            CorporateActionType::CapitalReturn => {
                // Capital return: reduce cost basis by amount_per_share * quantity
                // ratio_from stores the amount in cents (e.g., 100 for R$1.00)
                let amount_per_share = ratio_from / Decimal::from(100); // Convert cents to reais
                let reduction = amount_per_share * old_quantity;
                let new_total_cost = (old_total - reduction).max(Decimal::ZERO); // Don't go negative
                let new_price_per_unit = if old_quantity > Decimal::ZERO {
                    new_total_cost / old_quantity
                } else {
                    Decimal::ZERO
                };
                (old_quantity, new_price_per_unit, new_total_cost)
            }
            _ => {
                // Split/Reverse split/Bonus: adjust quantity and price, keep total unchanged
                let new_qty = old_quantity * ratio_to / ratio_from;
                let new_pr = old_price * ratio_from / ratio_to;
                (new_qty, new_pr, old_total)
            }
        };

        // Verify adjustments (for splits, total cost should remain unchanged)
        if action.action_type != CorporateActionType::CapitalReturn {
            let expected_total = old_quantity * old_price;
            let actual_total = new_quantity * new_price;
            let diff = (actual_total - expected_total).abs();
            let tolerance = Decimal::from_str("0.01").unwrap(); // 1 cent tolerance

            if diff > tolerance {
                tracing::warn!(
                    "Total cost changed for transaction {}: {} -> {} (diff: {})",
                    tx_id,
                    expected_total,
                    actual_total,
                    diff
                );
            }
        }

        // Update transaction in database
        conn.execute(
            "UPDATE transactions
             SET quantity = ?1, price_per_unit = ?2, total_cost = ?3
             WHERE id = ?4",
            rusqlite::params![
                new_quantity.to_string(),
                new_price.to_string(),
                new_total.to_string(),
                tx_id
            ],
        )?;

        // Record the adjustment in the junction table
        conn.execute(
            "INSERT INTO corporate_action_adjustments
             (action_id, transaction_id, old_quantity, new_quantity, old_price, new_price)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                action_id,
                tx_id,
                old_quantity.to_string(),
                new_quantity.to_string(),
                old_price.to_string(),
                new_price.to_string()
            ],
        )?;

        adjusted_count += 1;
    }

    // Mark the corporate action as applied
    mark_action_as_applied(conn, action_id)?;

    info!(
        "Successfully adjusted {} transactions for {} {}",
        adjusted_count,
        asset.ticker,
        action.action_type.as_str()
    );

    Ok(adjusted_count)
}

/// Apply relevant corporate actions to a single transaction
///
/// This is called when adding manual historical transactions. It finds all
/// corporate actions with ex-date after the transaction's trade date and
/// applies them in chronological order.
///
/// Returns the number of actions applied.
pub fn apply_actions_to_transaction(conn: &Connection, transaction_id: i64) -> Result<usize> {
    // Get the transaction details
    let mut stmt = conn.prepare("SELECT asset_id, trade_date FROM transactions WHERE id = ?1")?;

    let (asset_id, trade_date): (i64, chrono::NaiveDate) =
        stmt.query_row([transaction_id], |row| Ok((row.get(0)?, row.get(1)?)))?;

    // Find all applied corporate actions for this asset with ex-date after trade_date
    let mut actions_stmt = conn.prepare(
        "SELECT id, asset_id, action_type, event_date, ex_date, ratio_from, ratio_to,
                applied, source, notes, created_at
         FROM corporate_actions
         WHERE asset_id = ?1 AND ex_date > ?2 AND applied = 1
         ORDER BY ex_date ASC",
    )?;

    let actions = actions_stmt
        .query_map(rusqlite::params![asset_id, trade_date], |row| {
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
        .collect::<Result<Vec<_>, _>>()?;

    if actions.is_empty() {
        return Ok(0);
    }

    // Get the current transaction state
    let mut tx_stmt =
        conn.prepare("SELECT quantity, price_per_unit FROM transactions WHERE id = ?1")?;

    let (mut quantity, mut price): (Decimal, Decimal) = tx_stmt
        .query_row([transaction_id], |row| {
            Ok((get_decimal_value(row, 0)?, get_decimal_value(row, 1)?))
        })?;

    let mut applied_count = 0;

    // Apply each action in chronological order
    for action in actions {
        let action_id = action.id.unwrap();

        // Check if this action has already been applied to this transaction
        let already_adjusted: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM corporate_action_adjustments
             WHERE action_id = ?1 AND transaction_id = ?2)",
            rusqlite::params![action_id, transaction_id],
            |row| row.get(0),
        )?;

        if already_adjusted {
            continue;
        }

        let old_quantity = quantity;
        let old_price = price;

        let ratio_from = Decimal::from(action.ratio_from);
        let ratio_to = Decimal::from(action.ratio_to);

        if action.action_type == crate::db::CorporateActionType::Bonus {
            let new_quantity = quantity * ratio_to / ratio_from;
            let bonus_qty = new_quantity - quantity;

            if bonus_qty > Decimal::ZERO {
                let bonus_tx = Transaction {
                    id: None,
                    asset_id,
                    transaction_type: TransactionType::Buy,
                    trade_date: action.ex_date,
                    settlement_date: Some(action.ex_date),
                    quantity: bonus_qty,
                    price_per_unit: Decimal::ZERO,
                    total_cost: Decimal::ZERO,
                    fees: Decimal::ZERO,
                    is_day_trade: false,
                    quota_issuance_date: None,
                    notes: Some(format!(
                        "Bonus shares from {} (ratio {}:{})",
                        action.action_type.as_str(),
                        action.ratio_from,
                        action.ratio_to
                    )),
                    source: "CORPORATE_ACTION".to_string(),
                    created_at: chrono::Utc::now(),
                };
                let bonus_id = crate::db::insert_transaction(conn, &bonus_tx)?;
                apply_actions_to_transaction(conn, bonus_id)?;
            }

            conn.execute(
                "INSERT INTO corporate_action_adjustments
                 (action_id, transaction_id, old_quantity, new_quantity, old_price, new_price)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    action_id,
                    transaction_id,
                    old_quantity.to_string(),
                    old_quantity.to_string(),
                    old_price.to_string(),
                    old_price.to_string()
                ],
            )?;

            applied_count += 1;
        } else {
            quantity = quantity * ratio_to / ratio_from;
            price = price * ratio_from / ratio_to;

            // Record the adjustment
            conn.execute(
                "INSERT INTO corporate_action_adjustments
                 (action_id, transaction_id, old_quantity, new_quantity, old_price, new_price)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    action_id,
                    transaction_id,
                    old_quantity.to_string(),
                    quantity.to_string(),
                    old_price.to_string(),
                    price.to_string()
                ],
            )?;

            applied_count += 1;
        }

        info!(
            "Auto-applied {} (ratio {}:{}) to transaction {}",
            action.action_type.as_str(),
            action.ratio_from,
            action.ratio_to,
            transaction_id
        );
    }

    // Update the transaction with the final adjusted values
    if applied_count > 0 {
        conn.execute(
            "UPDATE transactions SET quantity = ?1, price_per_unit = ?2 WHERE id = ?3",
            rusqlite::params![quantity.to_string(), price.to_string(), transaction_id],
        )?;

        info!(
            "Auto-applied {} corporate action(s) to transaction {}",
            applied_count, transaction_id
        );
    }

    Ok(applied_count)
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

    #[test]
    fn test_capital_return_calculation() {
        // Capital return of R$1.00 per share
        let old_qty = Decimal::from(100);
        let old_price = Decimal::from(10);
        let old_total = old_qty * old_price; // 1000
        let ratio_from = Decimal::from(100); // 1.00 in cents
        let _ratio_to = Decimal::from(100); // Ignored for capital return

        let amount_per_share = ratio_from / Decimal::from(100);
        let reduction = amount_per_share * old_qty; // 1.00 * 100 = 100
        let new_total = (old_total - reduction).max(Decimal::ZERO); // 1000 - 100 = 900
        let new_price = new_total / old_qty; // 900 / 100 = 9

        assert_eq!(new_total, Decimal::from(900));
        assert_eq!(new_price, Decimal::from(9));
        assert_eq!(old_qty, old_qty); // Quantity unchanged
    }

    #[test]
    fn test_capital_return_exceeds_cost() {
        // Capital return exceeds cost - should floor at zero
        let old_qty = Decimal::from(100);
        let old_price = Decimal::from(5);
        let old_total = old_qty * old_price; // 500
        let ratio_from = Decimal::from(1000); // 10.00 in cents

        let amount_per_share = ratio_from / Decimal::from(100);
        let reduction = amount_per_share * old_qty; // 10.00 * 100 = 1000
        let new_total = (old_total - reduction).max(Decimal::ZERO); // Should be 0

        assert_eq!(new_total, Decimal::ZERO);
    }

    #[test]
    fn test_fractional_split() {
        // 3:10 split (e.g., bonus shares)
        let old_qty = Decimal::from(100);
        let old_price = Decimal::from(30);
        let ratio_from = Decimal::from(3);
        let ratio_to = Decimal::from(10);

        let new_qty = old_qty * ratio_to / ratio_from;
        let new_price = old_price * ratio_from / ratio_to;

        // 100 * 10 / 3 = 333.333...
        assert!(new_qty > Decimal::from(333) && new_qty < Decimal::from(334));
        // 30 * 3 / 10 = 9
        assert_eq!(new_price, Decimal::from(9));
    }

    #[test]
    fn test_zero_quantity_handling() {
        // Edge case: zero quantity (shouldn't happen in practice)
        let old_qty = Decimal::ZERO;
        let old_price = Decimal::from(50);
        let ratio_from = Decimal::from(1);
        let ratio_to = Decimal::from(2);

        let new_qty = old_qty * ratio_to / ratio_from;
        let new_price = old_price * ratio_from / ratio_to;

        assert_eq!(new_qty, Decimal::ZERO);
        assert_eq!(new_price, Decimal::from(25));
    }
}
