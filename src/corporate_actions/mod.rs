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
        "SELECT id, asset_id, action_type, event_date, ex_date, quantity_adjustment,
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
                quantity_adjustment: get_decimal_value(row, 5)?,
                source: row.get(6)?,
                notes: row.get(7)?,
                created_at: row.get(8)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(actions)
}

/// Get all corporate actions for an asset with `ex_date <= up_to_date` (inclusive), sorted by `ex_date ASC`.
pub fn get_actions_up_to(
    conn: &Connection,
    asset_id: i64,
    up_to_date: chrono::NaiveDate,
) -> Result<Vec<CorporateAction>> {
    let mut stmt = conn.prepare(
        "SELECT id, asset_id, action_type, event_date, ex_date, quantity_adjustment,
                source, notes, created_at
         FROM corporate_actions
         WHERE asset_id = ?1 AND ex_date <= ?2
         ORDER BY ex_date ASC",
    )?;

    let actions = stmt
        .query_map(rusqlite::params![asset_id, up_to_date.to_string()], |row| {
            Ok(CorporateAction {
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
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(actions)
}

/// Apply forward-only quantity adjustments for splits/reverse splits up to `cutoff_date`.
///
/// Advances `action_idx` as actions are applied. Ignores bonus and capital return.
pub fn apply_forward_qty_adjustments(
    quantity: &mut Decimal,
    actions: &[CorporateAction],
    action_idx: &mut usize,
    cutoff_date: chrono::NaiveDate,
) {
    while *action_idx < actions.len() {
        let action = &actions[*action_idx];
        if action.ex_date <= cutoff_date {
            match action.action_type {
                CorporateActionType::Split | CorporateActionType::ReverseSplit => {
                    *quantity += action.quantity_adjustment;
                }
                _ => {}
            }
            *action_idx += 1;
        } else {
            break;
        }
    }
}

/// Apply corporate action adjustments to a quantity at query time
///
/// Adds/subtracts the adjustment quantity (sign convention: positive = add, negative = subtract)
/// For bonus: no adjustment (bonus creates separate transaction)
/// For capital return: no quantity adjustment (affects cost basis only)
///
/// Returns the adjusted quantity
pub fn adjust_quantity_for_actions(quantity: Decimal, actions: &[CorporateAction]) -> Decimal {
    let mut adjusted = quantity;
    for action in actions {
        match action.action_type {
            CorporateActionType::Split | CorporateActionType::ReverseSplit => {
                // Apply the quantity adjustment directly (sign convention: positive = add, negative = subtract)
                adjusted += action.quantity_adjustment;
            }
            CorporateActionType::Bonus => {
                // Bonus creates a separate zero-cost transaction, doesn't adjust existing transactions
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
/// For quantity-based adjustments: recalculate price from adjusted total cost
/// For capital return: affects total cost, compute new price = new_total / quantity
///
/// Returns (adjusted_price, adjusted_total_cost)
pub fn adjust_price_and_cost_for_actions(
    quantity: Decimal,
    _price: Decimal,
    total_cost: Decimal,
    actions: &[CorporateAction],
) -> (Decimal, Decimal) {
    let mut adjusted_qty = quantity;
    let adjusted_cost = total_cost;

    for action in actions {
        match action.action_type {
            CorporateActionType::Split
            | CorporateActionType::ReverseSplit
            | CorporateActionType::Bonus => {
                // Apply quantity adjustment
                adjusted_qty += action.quantity_adjustment;
                // Total cost stays same for splits/bonuses; price per unit adjusts automatically
            }
            CorporateActionType::CapitalReturn => {
                // Capital return handling should be revised for new approach
            }
        }
    }

    // Recompute price per unit based on adjusted quantity
    let adjusted_price = if adjusted_qty > Decimal::ZERO {
        adjusted_cost / adjusted_qty
    } else {
        Decimal::ZERO
    };

    (adjusted_price, adjusted_cost)
}

/// Get unapplied corporate actions for an asset (or all assets if None)
pub fn get_unapplied_actions(
    conn: &Connection,
    asset_id_filter: Option<i64>,
) -> Result<Vec<CorporateAction>> {
    let query = if asset_id_filter.is_some() {
        "SELECT id, asset_id, action_type, event_date, ex_date, quantity_adjustment,
                source, notes, created_at
         FROM corporate_actions
         WHERE asset_id = ?1
         ORDER BY ex_date ASC"
    } else {
        "SELECT id, asset_id, action_type, event_date, ex_date, quantity_adjustment,
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
                quantity_adjustment: get_decimal_value(row, 5)?,
                source: row.get(6)?,
                notes: row.get(7)?,
                created_at: row.get(8)?,
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
                quantity_adjustment: get_decimal_value(row, 5)?,
                source: row.get(6)?,
                notes: row.get(7)?,
                created_at: row.get(8)?,
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
        "Processing {} for {} (adjustment: {} shares)",
        action.action_type.as_str(),
        asset.ticker,
        action.quantity_adjustment
    );

    // For bonus actions, create a zero-cost BUY transaction
    if action.action_type == crate::db::CorporateActionType::Bonus {
        // For bonus with quantity adjustment, the quantity_adjustment represents
        // the shares to add to each shareholder's position
        let bonus_qty = action.quantity_adjustment;

        if bonus_qty > Decimal::ZERO {
            let notes = format!(
                "Bonus shares from {} ({} shares)",
                action.action_type.as_str(),
                bonus_qty
            );

            let bonus_tx = Transaction {
                id: None,
                asset_id: action.asset_id,
                transaction_type: TransactionType::Buy,
                trade_date: action.ex_date,
                settlement_date: Some(action.ex_date),
                quantity: bonus_qty,
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
                bonus_qty, asset.ticker, action.ex_date
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
    fn test_stock_split_adjustment() {
        // Stock split: add 100 shares to existing 100 = 200 total
        let old_qty = Decimal::from(100);
        let quantity_adjustment = Decimal::from(100); // Add 100 shares
        let old_total_cost = Decimal::from(5000); // 100 * 50

        let new_qty = old_qty + quantity_adjustment;
        let new_price = old_total_cost / new_qty;

        assert_eq!(new_qty, Decimal::from(200));
        assert_eq!(new_price, Decimal::from(25)); // Cost per share halves
        assert_eq!(old_qty * (old_total_cost / old_qty), new_qty * new_price); // Total cost unchanged
    }

    #[test]
    fn test_reverse_split_adjustment() {
        // Reverse split: subtract 90 shares from 100 = 10 total
        let old_qty = Decimal::from(100);
        let quantity_adjustment = Decimal::from(-90); // Remove 90 shares
        let old_total_cost = Decimal::from(5000); // 100 * 50

        let new_qty = old_qty + quantity_adjustment;
        let new_price = old_total_cost / new_qty;

        assert_eq!(new_qty, Decimal::from(10));
        assert_eq!(new_price, Decimal::from(500)); // Cost per share increases
        assert_eq!(old_qty * (old_total_cost / old_qty), new_qty * new_price); // Total cost unchanged
    }

    #[test]
    fn test_bonus_shares_adjustment() {
        // Bonus: add 10 shares per 100 existing = 110 total
        let old_qty = Decimal::from(100);
        let quantity_adjustment = Decimal::from(10); // Add 10 bonus shares
        let old_total_cost = Decimal::from(1000); // 100 * 10

        let new_qty = old_qty + quantity_adjustment;
        let new_price = old_total_cost / new_qty;

        assert_eq!(new_qty, Decimal::from(110));
        // 1000 / 110 ≈ 9.09
        assert!(new_price < Decimal::from(10) && new_price > Decimal::from(9));
    }

    #[test]
    fn test_quantity_adjustment_sign_convention() {
        // Verify sign convention: positive = add, negative = subtract
        let quantity = Decimal::from(100);

        // Positive adjustment (add shares)
        let add_adjustment = Decimal::from(50);
        let result = quantity + add_adjustment;
        assert_eq!(result, Decimal::from(150));

        // Negative adjustment (subtract shares)
        let subtract_adjustment = Decimal::from(-30);
        let result = quantity + subtract_adjustment;
        assert_eq!(result, Decimal::from(70));
    }

    #[test]
    fn test_price_recalculation_after_adjustment() {
        // Verify that price is recalculated correctly after quantity adjustment
        let quantity = Decimal::from(100);
        let total_cost = Decimal::from(10000);
        let original_price = total_cost / quantity; // 100

        // Add 100 shares
        let quantity_adjustment = Decimal::from(100);
        let new_qty = quantity + quantity_adjustment; // 200
        let new_price = total_cost / new_qty; // 50

        assert_eq!(new_price, Decimal::from(50));
        // Total cost unchanged
        assert_eq!(quantity * original_price, new_qty * new_price);
    }
}
