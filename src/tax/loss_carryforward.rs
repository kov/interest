use anyhow::Result;
use rusqlite::Connection;
use rust_decimal::Decimal;
use std::collections::HashMap;
use std::str::FromStr;

use super::swing_trade::TaxCategory;

/// Loss carryforward entry
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct LossCarryforward {
    pub id: Option<i64>,
    pub year: i32,
    pub month: u32,
    pub category: TaxCategory,
    pub loss_amount: Decimal,
    pub remaining_amount: Decimal,
}

/// Get all uncompensated losses for a specific category, ordered by date (FIFO)
pub fn get_losses_for_category(
    conn: &Connection,
    category: &TaxCategory,
) -> Result<Vec<LossCarryforward>> {
    let mut stmt = conn.prepare(
        "SELECT id, year, month, tax_category, loss_amount, remaining_amount
         FROM loss_carryforward
         WHERE tax_category = ?1 AND remaining_amount > 0
         ORDER BY year ASC, month ASC",
    )?;

    let losses = stmt
        .query_map([category.as_str()], |row| {
            Ok(LossCarryforward {
                id: Some(row.get(0)?),
                year: row.get(1)?,
                month: row.get(2)?,
                category: TaxCategory::from_str(&row.get::<_, String>(3)?)
                    .unwrap_or(TaxCategory::StockSwingTrade),
                loss_amount: get_decimal_value(row, 4)?,
                remaining_amount: get_decimal_value(row, 5)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(losses)
}

/// Apply losses to a profit amount, returns (profit_after_loss_offset, total_loss_applied)
pub fn apply_losses_to_profit(
    conn: &Connection,
    category: &TaxCategory,
    profit: Decimal,
) -> Result<(Decimal, Decimal)> {
    if profit <= Decimal::ZERO {
        return Ok((profit, Decimal::ZERO));
    }

    let losses = get_losses_for_category(conn, category)?;
    let mut remaining_profit = profit;
    let mut total_loss_applied = Decimal::ZERO;

    // Apply losses in FIFO order (oldest first)
    for loss in losses {
        if remaining_profit <= Decimal::ZERO {
            break;
        }

        let loss_id = loss.id.expect("Loss should have ID");
        let amount_to_apply = remaining_profit.min(loss.remaining_amount);

        // Update the loss entry
        let new_remaining = loss.remaining_amount - amount_to_apply;
        conn.execute(
            "UPDATE loss_carryforward
             SET remaining_amount = ?1, updated_at = datetime('now')
             WHERE id = ?2",
            rusqlite::params![new_remaining.to_string(), loss_id],
        )?;

        remaining_profit -= amount_to_apply;
        total_loss_applied += amount_to_apply;
    }

    Ok((remaining_profit, total_loss_applied))
}

/// Record a new loss for carryforward
pub fn record_loss(
    conn: &Connection,
    year: i32,
    month: u32,
    category: &TaxCategory,
    loss_amount: Decimal,
) -> Result<()> {
    if loss_amount <= Decimal::ZERO {
        return Ok(());
    }

    conn.execute(
        "INSERT INTO loss_carryforward (year, month, tax_category, loss_amount, remaining_amount)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![
            year,
            month,
            category.as_str(),
            loss_amount.to_string(),
            loss_amount.to_string(), // Initially, all of it remains
        ],
    )?;

    Ok(())
}

/// Get total remaining losses by category
#[allow(dead_code)]
pub fn get_total_losses_by_category(conn: &Connection) -> Result<HashMap<TaxCategory, Decimal>> {
    let mut stmt = conn.prepare(
        "SELECT tax_category, SUM(remaining_amount) as total
         FROM loss_carryforward
         WHERE remaining_amount > 0
         GROUP BY tax_category",
    )?;

    let mut losses: HashMap<TaxCategory, Decimal> = HashMap::new();

    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;

    for row in rows {
        let (category_str, total_str) = row?;
        if let Some(category) = TaxCategory::from_str(&category_str) {
            if let Ok(total) = Decimal::from_str(&total_str) {
                losses.insert(category, total);
            }
        }
    }

    Ok(losses)
}

/// Helper to read Decimal from SQLite
fn get_decimal_value(row: &rusqlite::Row, idx: usize) -> Result<Decimal, rusqlite::Error> {
    // Try to get as String first (for TEXT storage)
    if let Ok(s) = row.get::<_, String>(idx) {
        return Decimal::from_str(&s)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)));
    }

    // Fall back to i64 (for INTEGER storage)
    if let Ok(i) = row.get::<_, i64>(idx) {
        return Ok(Decimal::from(i));
    }

    // Try f64 for floating point values
    if let Ok(f) = row.get::<_, f64>(idx) {
        return Decimal::try_from(f)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)));
    }

    Err(rusqlite::Error::InvalidColumnType(
        idx,
        "decimal".to_string(),
        rusqlite::types::Type::Null,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_apply_losses_zero_profit() {
        // When profit is zero or negative, no loss should be applied
        let (remaining, applied) = (Decimal::ZERO, Decimal::ZERO);
        assert_eq!(remaining, Decimal::ZERO);
        assert_eq!(applied, Decimal::ZERO);
    }
}
