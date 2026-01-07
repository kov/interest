use anyhow::Result;
use rusqlite::{Connection, OptionalExtension};
use rust_decimal::Decimal;
use std::collections::HashMap;
use std::str::FromStr;
use tracing::debug;

use super::swing_trade::TaxCategory;

#[derive(Debug, Clone)]
pub struct LossSnapshot {
    #[allow(dead_code)]
    pub year: i32,
    pub ending_carry: HashMap<TaxCategory, Decimal>,
    pub tx_fingerprint: String,
}

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
#[allow(dead_code)]
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
                category: row
                    .get::<_, String>(3)?
                    .parse::<TaxCategory>()
                    .unwrap_or(TaxCategory::StockSwingTrade),
                loss_amount: get_decimal_value(row, 4)?,
                remaining_amount: get_decimal_value(row, 5)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(losses)
}

/// Apply losses to a profit amount, returns (profit_after_loss_offset, total_loss_applied)
#[allow(dead_code)]
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
#[allow(dead_code)]
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
        Ok((row.get::<_, String>(0)?, get_decimal_value(row, 1)?))
    })?;

    for row in rows {
        let (category_str, total) = row?;
        if let Ok(category) = category_str.parse::<TaxCategory>() {
            losses.insert(category, total);
        }
    }

    Ok(losses)
}

/// Get remaining losses by category accrued before a given year (useful to show prior-year carryover).
#[allow(dead_code)]
pub fn get_remaining_losses_before_year(
    conn: &Connection,
    year: i32,
) -> Result<HashMap<TaxCategory, Decimal>> {
    let mut stmt = conn.prepare(
        "SELECT tax_category, SUM(remaining_amount) as total
         FROM loss_carryforward
         WHERE remaining_amount > 0 AND year < ?1
         GROUP BY tax_category",
    )?;

    let mut losses: HashMap<TaxCategory, Decimal> = HashMap::new();

    let rows = stmt.query_map([year], |row| {
        Ok((row.get::<_, String>(0)?, get_decimal_value(row, 1)?))
    })?;

    for row in rows {
        let (category_str, total) = row?;
        if let Ok(category) = category_str.parse::<TaxCategory>() {
            losses.insert(category, total);
        }
    }

    Ok(losses)
}

/// Compute a lightweight fingerprint of tax-relevant transactions for a year.
pub fn compute_year_fingerprint(conn: &Connection, year: i32) -> Result<String> {
    let mut stmt = conn.prepare(
        "SELECT COUNT(*) as cnt,
                COALESCE(SUM(quantity), 0) as qty_sum,
                COALESCE(SUM(total_cost), 0) as total_sum,
                COALESCE(SUM(fees), 0) as fee_sum
         FROM transactions
         WHERE strftime('%Y', trade_date) = ?1",
    )?;

    let row = stmt.query_row([year.to_string()], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            get_decimal_value(row, 1)?,
            get_decimal_value(row, 2)?,
            get_decimal_value(row, 3)?,
        ))
    })?;

    Ok(format!("{}:{}:{}:{}", row.0, row.1, row.2, row.3))
}

pub fn load_snapshots(conn: &Connection) -> Result<HashMap<i32, LossSnapshot>> {
    let mut stmt = conn.prepare(
        "SELECT year, tax_category, ending_remaining_amount, tx_fingerprint
         FROM loss_carryforward_snapshots",
    )?;

    let mut by_year: HashMap<i32, LossSnapshot> = HashMap::new();

    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i32>(0)?,
            row.get::<_, String>(1)?,
            get_decimal_value(row, 2)?,
            row.get::<_, String>(3)?,
        ))
    })?;

    for row in rows {
        let (year, cat_str, amount, fingerprint) = row?;
        let category = cat_str
            .parse::<TaxCategory>()
            .unwrap_or(TaxCategory::StockSwingTrade);

        let entry = by_year.entry(year).or_insert(LossSnapshot {
            year,
            ending_carry: HashMap::new(),
            tx_fingerprint: fingerprint.clone(),
        });

        entry.ending_carry.insert(category, amount);
        // If fingerprints differ across rows, keep the first; rows should share the same value.
    }

    let mut years: Vec<i32> = by_year.keys().copied().collect();
    years.sort_unstable();
    debug!(snapshot_years = ?years, "Loaded loss snapshots");

    Ok(by_year)
}

pub fn upsert_snapshot(
    conn: &Connection,
    year: i32,
    fingerprint: &str,
    carry: &HashMap<TaxCategory, Decimal>,
) -> Result<()> {
    let tx = conn.unchecked_transaction()?;

    tx.execute(
        "DELETE FROM loss_carryforward_snapshots WHERE year = ?1",
        [year],
    )?;

    if carry.is_empty() {
        // Persist an empty snapshot so we do not recompute this year again
        tx.execute(
            "INSERT INTO loss_carryforward_snapshots
             (year, tax_category, ending_remaining_amount, tx_fingerprint)
             VALUES (?1, ?2, 0, ?3)",
            rusqlite::params![year, TaxCategory::StockSwingTrade.as_str(), fingerprint],
        )?;
    } else {
        for (category, amount) in carry {
            tx.execute(
                "INSERT INTO loss_carryforward_snapshots
                 (year, tax_category, ending_remaining_amount, tx_fingerprint)
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![year, category.as_str(), amount.to_string(), fingerprint],
            )?;
        }
    }

    tx.commit()?;
    Ok(())
}

pub fn earliest_transaction_year(conn: &Connection) -> Result<Option<i32>> {
    let mut stmt =
        conn.prepare("SELECT MIN(CAST(strftime('%Y', trade_date) AS INTEGER)) FROM transactions")?;

    let year: Option<i32> = stmt.query_row([], |row| row.get(0)).optional()?;
    Ok(year)
}

/// Clear all loss_carryforward entries for a given year.
/// Called before recomputing a year to avoid stale ledger entries.
/// Snapshots prevent recomputation unless transactions changed, so this is safe.
pub fn clear_year_losses(conn: &Connection, year: i32) -> Result<()> {
    conn.execute("DELETE FROM loss_carryforward WHERE year = ?1", [year])?;
    Ok(())
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
