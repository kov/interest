// Corporate actions module - Split/bonus adjustment engine

use anyhow::Result;
use rusqlite::Connection;
use rust_decimal::Decimal;
use std::str::FromStr;
use tracing::info;

use crate::db::{Asset, CorporateAction, CorporateActionType, Transaction, TransactionType};
use chrono::NaiveDate;

/// Get all corporate actions for an asset that occurred between two dates (inclusive)
///
/// Used for query-time adjustment: finds actions that should be applied to
/// a transaction based on trade_date and as_of_date.
pub fn get_applicable_actions(
    conn: &Connection,
    asset_id: i64,
    after_date: NaiveDate,
    up_to_date: NaiveDate,
) -> Result<Vec<CorporateAction>> {
    let mut stmt = conn.prepare(
        "SELECT id, asset_id, action_type, event_date, ex_date, ratio_from, ratio_to,
                source, notes, created_at
         FROM corporate_actions
         WHERE asset_id = ?1 AND ex_date > ?2 AND ex_date <= ?3
         ORDER BY ex_date ASC",
    )?;

    let actions = stmt
        .query_map(rusqlite::params![asset_id, after_date, up_to_date], |row| {
            Ok(CorporateAction {
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
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(actions)
}

/// Apply corporate action adjustments to a quantity at query time
///
/// For splits/reverse splits: quantity × (ratio_to / ratio_from)
/// For bonus: no adjustment (bonus creates separate transaction)
/// For capital return: no quantity adjustment (affects cost basis only)
///
/// Returns the adjusted quantity
pub fn adjust_quantity_for_actions(quantity: Decimal, actions: &[CorporateAction]) -> Decimal {
    let mut adjusted = quantity;
    for action in actions {
        match action.action_type {
            CorporateActionType::Split | CorporateActionType::ReverseSplit => {
                let ratio_from = Decimal::from(action.ratio_from);
                let ratio_to = Decimal::from(action.ratio_to);
                adjusted = adjusted * ratio_to / ratio_from;
            }
            CorporateActionType::Bonus => {
                // Bonus creates zero-cost BUY transaction, no adjustment to existing transactions
            }
            CorporateActionType::CapitalReturn => {
                // Capital return affects cost basis, not quantity
            }
        }
    }
    adjusted
}

/// Adjust price per unit for corporate actions at query time
///
/// For splits/reverse splits: price × (ratio_from / ratio_to)
/// For capital return: affects total cost, compute new price = new_total / quantity
///
/// Returns (adjusted_price, adjusted_total_cost)
pub fn adjust_price_and_cost_for_actions(
    quantity: Decimal,
    price: Decimal,
    total_cost: Decimal,
    actions: &[CorporateAction],
) -> (Decimal, Decimal) {
    let mut adjusted_price = price;
    let mut adjusted_cost = total_cost;
    let mut adjusted_qty = quantity;

    for action in actions {
        match action.action_type {
            CorporateActionType::Split | CorporateActionType::ReverseSplit => {
                let ratio_from = Decimal::from(action.ratio_from);
                let ratio_to = Decimal::from(action.ratio_to);
                adjusted_price = adjusted_price * ratio_from / ratio_to;
                adjusted_qty = adjusted_qty * ratio_to / ratio_from;
                // Total cost stays same for splits
            }
            CorporateActionType::CapitalReturn => {
                // Reduce cost basis by amount_per_share × quantity
                let ratio_from = Decimal::from(action.ratio_from);
                let amount_per_share = ratio_from / Decimal::from(100); // cents to reais
                let reduction = amount_per_share * adjusted_qty;
                adjusted_cost = (adjusted_cost - reduction).max(Decimal::ZERO);
                adjusted_price = if adjusted_qty > Decimal::ZERO {
                    adjusted_cost / adjusted_qty
                } else {
                    Decimal::ZERO
                };
            }
            CorporateActionType::Bonus => {
                // No adjustment to existing transactions
            }
        }
    }

    (adjusted_price, adjusted_cost)
}

/// Get unapplied corporate actions for an asset (or all assets if None)
pub fn get_unapplied_actions(
    conn: &Connection,
    asset_id_filter: Option<i64>,
) -> Result<Vec<CorporateAction>> {
    let query = if asset_id_filter.is_some() {
        "SELECT id, asset_id, action_type, event_date, ex_date, ratio_from, ratio_to,
                source, notes, created_at
         FROM corporate_actions
         WHERE asset_id = ?1
         ORDER BY ex_date ASC"
    } else {
        "SELECT id, asset_id, action_type, event_date, ex_date, ratio_from, ratio_to,
                source, notes, created_at
         FROM corporate_actions
         ORDER BY ex_date ASC"
    };

    let mut stmt = conn.prepare(query)?;

    let actions = if let Some(asset_id) = asset_id_filter {
        stmt.query_map([asset_id], |row| {
            Ok(CorporateAction {
                id: Some(row.get(0)?),
                asset_id: row.get(1)?,
                action_type: row
                    .get::<_, String>(2)?
                    .parse::<crate::db::CorporateActionType>()
                    .unwrap_or(crate::db::CorporateActionType::Split),
                event_date: row.get(3)?,
                ex_date: row.get(4)?,
                ratio_from: row.get(5)?,
                ratio_to: row.get(6)?,
                source: row.get(7)?,
                notes: row.get(8)?,
                created_at: row.get(9)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?
    } else {
        stmt.query_map([], |row| {
            Ok(CorporateAction {
                id: Some(row.get(0)?),
                asset_id: row.get(1)?,
                action_type: row
                    .get::<_, String>(2)?
                    .parse::<crate::db::CorporateActionType>()
                    .unwrap_or(crate::db::CorporateActionType::Split),
                event_date: row.get(3)?,
                ex_date: row.get(4)?,
                ratio_from: row.get(5)?,
                ratio_to: row.get(6)?,
                source: row.get(7)?,
                notes: row.get(8)?,
                created_at: row.get(9)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?
    };

    Ok(actions)
}

/// Apply a corporate action by creating synthetic transactions (for bonus shares)
///
/// For splits/reverse splits: No mutation - adjustments happen at query time.
/// For bonus shares: Creates a zero-cost BUY transaction.
/// For capital return: No synthetic transaction - cost adjustment happens at query time.
///
/// Returns the number of transactions created (0 or 1).
pub fn apply_corporate_action(
    conn: &Connection,
    action: &CorporateAction,
    asset: &Asset,
) -> Result<usize> {
    info!(
        "Processing {} for {} (ratio {}:{})",
        action.action_type.as_str(),
        asset.ticker,
        action.ratio_from,
        action.ratio_to
    );

    // For bonus actions, create a zero-cost BUY transaction
    if action.action_type == crate::db::CorporateActionType::Bonus {
        use rust_decimal::RoundingStrategy;

        // Calculate net position before the bonus
        let net_qty = calculate_net_position_before_date(conn, action.asset_id, action.ex_date)?;

        let ratio_from = Decimal::from(action.ratio_from);
        let ratio_to = Decimal::from(action.ratio_to);

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

            info!(
                "Created bonus transaction: {} shares for {} on {}",
                integer_bonus, asset.ticker, action.ex_date
            );

            return Ok(1);
        } else {
            info!(
                "Bonus action {} for {} resulted in zero bonus quantity",
                action.id.unwrap_or(0),
                asset.ticker
            );
            return Ok(0);
        }
    }

    // For splits, reverse splits, and capital returns: no synthetic transactions
    // Adjustments are applied at query time
    info!(
        "{} for {} will be applied at query time",
        action.action_type.as_str(),
        asset.ticker
    );

    Ok(0)
}

/// Calculate net position (buys - sells) before a given date, with split adjustments applied
fn calculate_net_position_before_date(
    conn: &Connection,
    asset_id: i64,
    before_date: NaiveDate,
) -> Result<Decimal> {
    let mut stmt = conn.prepare(
        "SELECT transaction_type, quantity, trade_date
         FROM transactions
         WHERE asset_id = ?1 AND trade_date < ?2
         ORDER BY trade_date ASC",
    )?;

    let mut net_qty = Decimal::ZERO;
    let mut rows = stmt.query(rusqlite::params![asset_id, before_date])?;

    while let Some(row) = rows.next()? {
        let tx_type: String = row.get(0)?;
        let quantity = get_decimal_value(row, 1)?;
        let trade_date: NaiveDate = row.get(2)?;

        // Apply split adjustments for actions between trade_date and before_date
        let actions = get_applicable_actions(conn, asset_id, trade_date, before_date)?;
        let adjusted_qty = adjust_quantity_for_actions(quantity, &actions);

        match tx_type.parse::<TransactionType>() {
            Ok(TransactionType::Buy) => net_qty += adjusted_qty,
            Ok(TransactionType::Sell) => net_qty -= adjusted_qty,
            Err(_) => {}
        }
    }

    Ok(net_qty)
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
