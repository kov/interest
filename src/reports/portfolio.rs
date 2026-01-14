use anyhow::Result;
use blake3::Hasher;
use chrono::NaiveDate;
use rusqlite::Connection;
use rust_decimal::Decimal;
use std::collections::HashMap;
use std::str::FromStr;

use crate::db::{Asset, AssetType, Transaction, TransactionType};

/// Summary of a single position
#[derive(Debug, Clone)]
pub struct PositionSummary {
    pub asset: Asset,
    pub quantity: Decimal,
    pub average_cost: Decimal,
    pub total_cost: Decimal,
    pub current_price: Option<Decimal>,
    pub current_value: Option<Decimal>,
    pub unrealized_pl: Option<Decimal>,
    pub unrealized_pl_pct: Option<Decimal>,
}

/// Complete portfolio report
#[derive(Debug)]
pub struct PortfolioReport {
    pub positions: Vec<PositionSummary>,
    pub total_cost: Decimal,
    pub total_value: Decimal,
    pub total_pl: Decimal,
    pub total_pl_pct: Decimal,
}

/// Average-cost position tracker for a single asset
#[derive(Debug)]
struct AvgCostPosition {
    #[allow(dead_code)]
    asset_id: i64,
    quantity: Decimal,
    total_cost: Decimal,
}

impl AvgCostPosition {
    fn new(asset_id: i64) -> Self {
        Self {
            asset_id,
            quantity: Decimal::ZERO,
            total_cost: Decimal::ZERO,
        }
    }

    fn add_buy(&mut self, quantity: Decimal, cost: Decimal) {
        self.quantity += quantity;
        self.total_cost += cost;
    }

    fn remove_sell(&mut self, quantity: Decimal, ticker: &str) -> Result<Decimal> {
        if quantity > self.quantity {
            anyhow::bail!(
                "{}: Insufficient purchase history: Selling {} units but only {} available.\n\
                \nThis usually means:\n\
                1. Shares came from sources not in the import (term contracts, transfers, etc.)\n\
                2. Incomplete transaction history in the CEI export\n\
                3. Short selling (not yet supported)\n\
                \nTo fix: Manually add the missing purchase transactions using:\n\
                interest transactions add {} buy <quantity> <price> <date>",
                ticker,
                quantity,
                self.quantity,
                ticker
            );
        }

        // Calculate proportional cost basis for the sold units
        let avg_cost = if self.quantity > Decimal::ZERO {
            self.total_cost / self.quantity
        } else {
            Decimal::ZERO
        };

        let cost_basis = avg_cost * quantity;

        self.quantity -= quantity;
        self.total_cost -= cost_basis;

        Ok(cost_basis)
    }

    fn apply_amortization(&mut self, amount: Decimal) {
        if amount <= Decimal::ZERO {
            return;
        }

        self.total_cost -= amount;
        if self.total_cost < Decimal::ZERO {
            self.total_cost = Decimal::ZERO;
        }
    }

    fn average_cost(&self) -> Decimal {
        if self.quantity > Decimal::ZERO {
            self.total_cost / self.quantity
        } else {
            Decimal::ZERO
        }
    }
}

/// Calculate current portfolio positions using average cost
pub fn calculate_portfolio(
    conn: &Connection,
    asset_type_filter: Option<&AssetType>,
) -> Result<PortfolioReport> {
    calculate_portfolio_with_cutoff(conn, asset_type_filter, None)
}

/// Calculate portfolio positions as of a specific date (inclusive)
pub fn calculate_portfolio_at_date(
    conn: &Connection,
    as_of_date: NaiveDate,
    asset_type_filter: Option<&AssetType>,
) -> Result<PortfolioReport> {
    calculate_portfolio_with_cutoff(conn, asset_type_filter, Some(as_of_date))
}

fn calculate_portfolio_with_cutoff(
    conn: &Connection,
    asset_type_filter: Option<&AssetType>,
    as_of_date: Option<NaiveDate>,
) -> Result<PortfolioReport> {
    // Get all assets
    let assets = crate::db::get_all_assets(conn)?;

    let mut assets_by_ticker = HashMap::new();
    for asset in &assets {
        assets_by_ticker.insert(asset.ticker.clone(), asset.clone());
    }

    // Filter by asset type if requested
    let filtered_assets: Vec<_> = if let Some(filter) = asset_type_filter {
        assets
            .into_iter()
            .filter(|a| &a.asset_type == filter)
            .filter(|a| crate::db::is_supported_portfolio_ticker(&a.ticker))
            .filter(|a| !crate::db::is_rename_source_ticker(&a.ticker))
            .collect()
    } else {
        assets
            .into_iter()
            .filter(|a| crate::db::is_supported_portfolio_ticker(&a.ticker))
            .filter(|a| !crate::db::is_rename_source_ticker(&a.ticker))
            .collect()
    };

    // Calculate positions for each asset
    let mut positions = Vec::new();
    let mut total_cost = Decimal::ZERO;
    let mut total_value = Decimal::ZERO;

    for asset in filtered_assets {
        let asset_id = asset.id.unwrap();

        // Get all transactions for this asset, ordered by date
        let mut transactions = match as_of_date {
            Some(cutoff) => get_asset_transactions_until(conn, asset_id, cutoff)?,
            None => get_asset_transactions(conn, asset_id)?,
        };

        for (source_ticker, effective_date) in crate::db::rename_sources_for(&asset.ticker) {
            if let Some(limit) = as_of_date {
                if limit < effective_date {
                    continue;
                }
            }

            if let Some(source_asset) = assets_by_ticker.get(source_ticker) {
                if let Some(carryover) = build_rename_carryover_transaction(
                    conn,
                    source_asset,
                    asset_id,
                    &asset.ticker,
                    effective_date,
                )? {
                    transactions.push(carryover);
                }
            }
        }

        transactions.sort_by(|a, b| (a.trade_date, a.id).cmp(&(b.trade_date, b.id)));

        // Calculate average-cost position
        let mut position = AvgCostPosition::new(asset_id);

        // Apply fixed split adjustments forward-only, at the time they occur
        let as_of = as_of_date.unwrap_or_else(|| chrono::Local::now().date_naive());
        let amortizations =
            crate::db::get_amortizations_for_asset(conn, asset_id, None, Some(as_of))?;
        let mut amort_idx = 0usize;
        let actions = crate::corporate_actions::get_actions_up_to(conn, asset_id, as_of)?;

        let mut action_idx = 0usize;
        for tx in transactions {
            while amort_idx < amortizations.len()
                && amortizations[amort_idx].event_date <= tx.trade_date
            {
                position.apply_amortization(amortizations[amort_idx].total_amount);
                amort_idx += 1;
            }

            // Apply any corporate actions effective up to this transaction's date
            crate::corporate_actions::apply_forward_qty_adjustments(
                &mut position.quantity,
                &actions,
                &mut action_idx,
                tx.trade_date,
            );

            // Build raw position for this transaction
            match tx.transaction_type {
                TransactionType::Buy => {
                    position.add_buy(tx.quantity, tx.total_cost);
                }
                TransactionType::Sell => {
                    position.remove_sell(tx.quantity, &asset.ticker)?;
                }
            }
        }

        while amort_idx < amortizations.len() && amortizations[amort_idx].event_date <= as_of {
            position.apply_amortization(amortizations[amort_idx].total_amount);
            amort_idx += 1;
        }

        // Apply any remaining actions after the last transaction but before as_of
        crate::corporate_actions::apply_forward_qty_adjustments(
            &mut position.quantity,
            &actions,
            &mut action_idx,
            as_of,
        );

        // Skip assets with zero quantity
        if position.quantity <= Decimal::ZERO {
            continue;
        }

        // Get current price
        let latest_price = if let Some(cutoff) = as_of_date {
            crate::db::get_price_on_or_before(conn, asset_id, cutoff)?
        } else {
            crate::db::get_latest_price(conn, asset_id)?
        };
        let current_price = latest_price.as_ref().map(|p| p.close_price);

        // Calculate current value and P&L
        let (current_value, unrealized_pl, unrealized_pl_pct) = if let Some(price) = current_price {
            let value = price * position.quantity;
            let pl = value - position.total_cost;
            let pl_pct = if position.total_cost > Decimal::ZERO {
                (pl / position.total_cost) * Decimal::from(100)
            } else {
                Decimal::ZERO
            };
            (Some(value), Some(pl), Some(pl_pct))
        } else {
            (None, None, None)
        };

        total_cost += position.total_cost;
        if let Some(value) = current_value {
            total_value += value;
        }

        positions.push(PositionSummary {
            asset,
            quantity: position.quantity,
            average_cost: position.average_cost(),
            total_cost: position.total_cost,
            current_price,
            current_value,
            unrealized_pl,
            unrealized_pl_pct,
        });
    }

    // Sort positions by total value (descending)
    positions.sort_by(|a, b| {
        let a_val = a.current_value.unwrap_or(a.total_cost);
        let b_val = b.current_value.unwrap_or(b.total_cost);
        b_val.cmp(&a_val)
    });

    let total_pl = total_value - total_cost;
    let total_pl_pct = if total_cost > Decimal::ZERO {
        (total_pl / total_cost) * Decimal::from(100)
    } else {
        Decimal::ZERO
    };

    Ok(PortfolioReport {
        positions,
        total_cost,
        total_value,
        total_pl,
        total_pl_pct,
    })
}

fn map_transaction(row: &rusqlite::Row) -> Result<Transaction, rusqlite::Error> {
    Ok(Transaction {
        id: Some(row.get(0)?),
        asset_id: row.get(1)?,
        transaction_type: row
            .get::<_, String>(2)?
            .parse::<TransactionType>()
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
}

/// Get all transactions for an asset, ordered by trade date
fn get_asset_transactions(conn: &Connection, asset_id: i64) -> Result<Vec<Transaction>> {
    let mut stmt = conn.prepare(
        "SELECT id, asset_id, transaction_type, trade_date, settlement_date,
                quantity, price_per_unit, total_cost, fees, is_day_trade,
                quota_issuance_date, notes, source, created_at
         FROM transactions
         WHERE asset_id = ?1
         ORDER BY trade_date ASC, id ASC",
    )?;

    let transactions = stmt
        .query_map([asset_id], map_transaction)?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(transactions)
}

/// Get all transactions for an asset before a cutoff date (exclusive).
fn get_asset_transactions_before(
    conn: &Connection,
    asset_id: i64,
    before_date: NaiveDate,
) -> Result<Vec<Transaction>> {
    let mut stmt = conn.prepare(
        "SELECT id, asset_id, transaction_type, trade_date, settlement_date,
                quantity, price_per_unit, total_cost, fees, is_day_trade,
                quota_issuance_date, notes, source, created_at
         FROM transactions
         WHERE asset_id = ?1 AND trade_date < ?2
         ORDER BY trade_date ASC, id ASC",
    )?;

    let transactions = stmt
        .query_map(rusqlite::params![asset_id, before_date], map_transaction)?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(transactions)
}

/// Get all transactions for an asset up to and including a cutoff date.
fn get_asset_transactions_until(
    conn: &Connection,
    asset_id: i64,
    cutoff_date: NaiveDate,
) -> Result<Vec<Transaction>> {
    let mut stmt = conn.prepare(
        "SELECT id, asset_id, transaction_type, trade_date, settlement_date,
                quantity, price_per_unit, total_cost, fees, is_day_trade,
                quota_issuance_date, notes, source, created_at
         FROM transactions
         WHERE asset_id = ?1 AND trade_date <= ?2
         ORDER BY trade_date ASC, id ASC",
    )?;

    let transactions = stmt
        .query_map(rusqlite::params![asset_id, cutoff_date], map_transaction)?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(transactions)
}

fn build_rename_carryover_transaction(
    conn: &Connection,
    source_asset: &Asset,
    target_asset_id: i64,
    target_ticker: &str,
    effective_date: NaiveDate,
) -> Result<Option<Transaction>> {
    let source_id = match source_asset.id {
        Some(id) => id,
        None => return Ok(None),
    };

    let transactions = get_asset_transactions_before(conn, source_id, effective_date)?;
    let mut position = AvgCostPosition::new(source_id);

    for tx in transactions {
        if tx.is_day_trade {
            continue;
        }

        match tx.transaction_type {
            TransactionType::Buy => {
                position.add_buy(tx.quantity, tx.total_cost);
            }
            TransactionType::Sell => {
                position.remove_sell(tx.quantity, &source_asset.ticker)?;
            }
        }
    }

    if position.quantity <= Decimal::ZERO {
        return Ok(None);
    }

    let quantity = position.quantity;
    let total_cost = position.total_cost;
    // NOTE: Do NOT apply corporate actions here - they will be applied naturally
    // in the main transaction loop via apply_forward_qty_adjustments based on
    // the carryover transaction's trade_date

    if let Some(target_qty) =
        crate::db::rename_quantity_override(target_ticker, &source_asset.ticker)
    {
        if target_qty > Decimal::ZERO && quantity > Decimal::ZERO && target_qty != quantity {
            // FIXME: consider amortized cash when quantity changes via rename.
            return Ok(Some(Transaction {
                id: None,
                asset_id: target_asset_id,
                transaction_type: TransactionType::Buy,
                trade_date: effective_date,
                settlement_date: Some(effective_date),
                quantity: target_qty,
                price_per_unit: if target_qty > Decimal::ZERO {
                    total_cost / target_qty
                } else {
                    Decimal::ZERO
                },
                total_cost,
                fees: Decimal::ZERO,
                is_day_trade: false,
                quota_issuance_date: None,
                notes: Some(format!(
                    "Rename from {} (quantity override)",
                    source_asset.ticker
                )),
                source: "RENAME".to_string(),
                created_at: chrono::Utc::now(),
            }));
        }
    }

    let price_per_unit = if quantity > Decimal::ZERO {
        total_cost / quantity
    } else {
        Decimal::ZERO
    };

    Ok(Some(Transaction {
        id: None,
        asset_id: target_asset_id,
        transaction_type: TransactionType::Buy,
        trade_date: effective_date,
        settlement_date: Some(effective_date),
        quantity,
        price_per_unit,
        total_cost,
        fees: Decimal::ZERO,
        is_day_trade: false,
        quota_issuance_date: None,
        notes: Some(format!("Rename from {}", source_asset.ticker)),
        source: "RENAME".to_string(),
        created_at: chrono::Utc::now(),
    }))
}

// NOTE: apply_actions_to_carryover removed - carryover transaction is created at
// the rename effective_date, and corporate actions are applied naturally by
// the main transaction loop's apply_forward_qty_adjustments based on chronological
// trade_date ordering. Applying actions here caused double-adjustment bugs.

/// Helper to read Decimal from SQLite (handles both INTEGER and TEXT)
fn get_decimal_value(row: &rusqlite::Row, idx: usize) -> Result<Decimal, rusqlite::Error> {
    // Try to get as String first (for TEXT storage)
    if let Ok(s) = row.get::<_, String>(idx) {
        return Decimal::from_str(&s)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)));
    }

    // Fall back to i64 (for INTEGER storage due to SQLite type affinity)
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
        "quantity".to_string(),
        rusqlite::types::Type::Null,
    ))
}

/// Calculate asset allocation breakdown
pub fn calculate_allocation(report: &PortfolioReport) -> HashMap<AssetType, (Decimal, Decimal)> {
    let mut allocation: HashMap<AssetType, (Decimal, Decimal)> = HashMap::new();

    for position in &report.positions {
        let value = position.current_value.unwrap_or(position.total_cost);
        let entry = allocation
            .entry(position.asset.asset_type)
            .or_insert((Decimal::ZERO, Decimal::ZERO));
        entry.0 += value;
    }

    // Calculate percentages
    if report.total_value > Decimal::ZERO {
        for (_asset_type, (value, pct)) in allocation.iter_mut() {
            *pct = (*value / report.total_value) * Decimal::from(100);
        }
    }

    allocation
}

/// Compute a fingerprint for all transactions up to and including a date.
/// Includes corporate actions to detect when adjustments change.
pub fn compute_snapshot_fingerprint(conn: &Connection, as_of_date: NaiveDate) -> Result<String> {
    let mut hasher = Hasher::new();

    // Hash transactions
    let mut stmt = conn.prepare(
        "SELECT id, asset_id, transaction_type, trade_date, quantity, price_per_unit, total_cost
         FROM transactions
         WHERE trade_date <= ?1
         ORDER BY trade_date ASC, id ASC",
    )?;

    let mut rows = stmt.query([as_of_date])?;

    while let Some(row) = rows.next()? {
        let id: i64 = row.get(0)?;
        let asset_id: i64 = row.get(1)?;
        let tx_type: String = row.get(2)?;
        let trade_date: NaiveDate = row.get(3)?;
        let quantity = get_decimal_value(row, 4)?;
        let price_per_unit = get_decimal_value(row, 5)?;
        let total_cost = get_decimal_value(row, 6)?;

        let line = format!(
            "{}|{}|{}|{}|{}|{}|{}\n",
            id, asset_id, tx_type, trade_date, quantity, price_per_unit, total_cost
        );
        hasher.update(line.as_bytes());
    }

    // Hash corporate actions that apply up to this date
    let mut ca_stmt = conn.prepare(
        "SELECT id, asset_id, action_type, ex_date, quantity_adjustment
         FROM corporate_actions
         WHERE ex_date <= ?1
         ORDER BY ex_date ASC, id ASC",
    )?;

    let mut ca_rows = ca_stmt.query([as_of_date])?;

    while let Some(row) = ca_rows.next()? {
        let id: i64 = row.get(0)?;
        let asset_id: i64 = row.get(1)?;
        let action_type: String = row.get(2)?;
        let ex_date: NaiveDate = row.get(3)?;
        let quantity_adjustment = crate::db::get_decimal_value(row, 4)?;

        let line = format!(
            "CA|{}|{}|{}|{}|{}\n",
            id, asset_id, action_type, ex_date, quantity_adjustment
        );
        hasher.update(line.as_bytes());
    }

    Ok(hasher.finalize().to_hex().to_string())
}

/// Save a portfolio snapshot for a specific date, replacing any existing rows for that date.
pub fn save_portfolio_snapshot(
    conn: &mut Connection,
    date: NaiveDate,
    label: Option<String>,
) -> Result<()> {
    let report = calculate_portfolio_at_date(conn, date, None)?;
    let fingerprint = compute_snapshot_fingerprint(conn, date)?;

    let tx = conn.transaction()?;
    tx.execute(
        "DELETE FROM position_snapshots WHERE snapshot_date = ?1",
        [date],
    )?;

    for position in report.positions {
        let asset_id = position
            .asset
            .id
            .ok_or_else(|| anyhow::anyhow!("Asset missing id for snapshot"))?;

        let market_price = position.current_price.unwrap_or(position.average_cost);
        let market_value = position
            .current_value
            .unwrap_or_else(|| market_price * position.quantity);
        let unrealized_pl = position
            .unrealized_pl
            .unwrap_or_else(|| market_value - position.total_cost);

        tx.execute(
            "INSERT INTO position_snapshots (
                snapshot_date, asset_id, quantity, average_cost, market_price,
                market_value, unrealized_pl, tx_fingerprint, label
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                date,
                asset_id,
                position.quantity.to_string(),
                position.average_cost.to_string(),
                market_price.to_string(),
                market_value.to_string(),
                unrealized_pl.to_string(),
                &fingerprint,
                label.clone(),
            ],
        )?;
    }

    tx.commit()?;
    Ok(())
}

/// Load a snapshot if the stored fingerprint matches the current transaction state.
pub fn get_valid_snapshot(conn: &Connection, date: NaiveDate) -> Result<Option<PortfolioReport>> {
    let mut stmt = conn.prepare(
        "SELECT ps.asset_id, ps.quantity, ps.average_cost, ps.market_price, ps.market_value,
                ps.unrealized_pl, ps.tx_fingerprint, a.ticker, a.asset_type, a.name,
                a.created_at, a.updated_at
         FROM position_snapshots ps
         JOIN assets a ON ps.asset_id = a.id
         WHERE ps.snapshot_date = ?1
         ORDER BY ps.market_value DESC",
    )?;

    let rows = stmt
        .query_map([date], |row| {
            let asset_type: AssetType =
                row.get::<_, String>(8)?.parse().unwrap_or(AssetType::Unknown);

            Ok((
                Asset {
                    id: Some(row.get(0)?),
                    ticker: row.get(7)?,
                    asset_type,
                    name: row.get(9)?,
                    created_at: row.get(10)?,
                    updated_at: row.get(11)?,
                },
                get_decimal_value(row, 1)?,
                get_decimal_value(row, 2)?,
                get_decimal_value(row, 3)?,
                get_decimal_value(row, 4)?,
                get_decimal_value(row, 5)?,
                row.get::<_, String>(6)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    if rows.is_empty() {
        return Ok(None);
    }

    let stored_fingerprint = rows[0].6.clone();
    let current_fingerprint = compute_snapshot_fingerprint(conn, date)?;
    if stored_fingerprint != current_fingerprint {
        return Ok(None);
    }

    let mut positions = Vec::new();
    let mut total_cost = Decimal::ZERO;
    let mut total_value = Decimal::ZERO;

    for (asset, quantity, average_cost, market_price, market_value, unrealized_pl, _) in rows {
        let position_cost = average_cost * quantity;
        let unrealized_pl_pct = if position_cost > Decimal::ZERO {
            (unrealized_pl / position_cost) * Decimal::from(100)
        } else {
            Decimal::ZERO
        };

        total_cost += position_cost;
        total_value += market_value;

        positions.push(PositionSummary {
            asset,
            quantity,
            average_cost,
            total_cost: position_cost,
            current_price: Some(market_price),
            current_value: Some(market_value),
            unrealized_pl: Some(unrealized_pl),
            unrealized_pl_pct: Some(unrealized_pl_pct),
        });
    }

    let total_pl = total_value - total_cost;
    let total_pl_pct = if total_cost > Decimal::ZERO {
        (total_pl / total_cost) * Decimal::from(100)
    } else {
        Decimal::ZERO
    };

    Ok(Some(PortfolioReport {
        positions,
        total_cost,
        total_value,
        total_pl,
        total_pl_pct,
    }))
}

/// Delete snapshots on or after a given date to force recomputation.
pub fn invalidate_snapshots_after(
    conn: &Connection,
    earliest_changed_date: NaiveDate,
) -> Result<()> {
    conn.execute(
        "DELETE FROM position_snapshots WHERE snapshot_date >= ?1",
        [earliest_changed_date],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{self, AssetType, PriceHistory, Transaction, TransactionType};
    use chrono::{NaiveDate, Utc};
    use rusqlite::Connection;

    #[test]
    fn test_avg_position_buy_and_sell() {
        let mut position = AvgCostPosition::new(1);

        // Buy 100 @ R$10 = R$1000
        position.add_buy(Decimal::from(100), Decimal::from(1000));
        assert_eq!(position.quantity, Decimal::from(100));
        assert_eq!(position.total_cost, Decimal::from(1000));
        assert_eq!(position.average_cost(), Decimal::from(10));

        // Buy 50 @ R$15 = R$750
        position.add_buy(Decimal::from(50), Decimal::from(750));
        assert_eq!(position.quantity, Decimal::from(150));
        assert_eq!(position.total_cost, Decimal::from(1750));

        // Average cost should be 1750 / 150 = 11.67 (rounded)
        let avg = position.average_cost();
        assert!(avg > Decimal::from(11) && avg < Decimal::from(12));

        // Sell 75 units
        let cost_basis = position.remove_sell(Decimal::from(75), "TEST").unwrap();
        assert_eq!(position.quantity, Decimal::from(75));

        // Cost basis for sold units should be 75 * avg_cost
        // With 1750/150 = 11.666... * 75 = 875
        assert_eq!(cost_basis, Decimal::from(875));
    }

    #[test]
    fn test_avg_position_oversell() {
        let mut position = AvgCostPosition::new(1);
        position.add_buy(Decimal::from(100), Decimal::from(1000));

        // Try to sell more than we have
        let result = position.remove_sell(Decimal::from(150), "TEST");
        assert!(result.is_err());
    }

    #[test]
    fn test_snapshot_fingerprint_stable() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!("../db/schema.sql"))
            .unwrap();

        let asset_id = db::upsert_asset(&conn, "TEST3", &AssetType::Stock, None).unwrap();

        let base_tx = Transaction {
            id: None,
            asset_id,
            transaction_type: TransactionType::Buy,
            trade_date: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            settlement_date: None,
            quantity: Decimal::from(10),
            price_per_unit: Decimal::from(100),
            total_cost: Decimal::from(1000),
            fees: Decimal::ZERO,
            is_day_trade: false,
            quota_issuance_date: None,
            notes: None,
            source: "TEST".to_string(),
            created_at: Utc::now(),
        };

        db::insert_transaction(&conn, &base_tx).unwrap();

        let fp1 = compute_snapshot_fingerprint(&conn, base_tx.trade_date).unwrap();
        let fp2 = compute_snapshot_fingerprint(&conn, base_tx.trade_date).unwrap();
        assert_eq!(fp1, fp2);

        let later_tx = Transaction {
            trade_date: NaiveDate::from_ymd_opt(2024, 2, 1).unwrap(),
            ..base_tx
        };
        db::insert_transaction(&conn, &later_tx).unwrap();

        let fp_unchanged = compute_snapshot_fingerprint(&conn, base_tx.trade_date).unwrap();
        assert_eq!(fp1, fp_unchanged);

        let fp_changed = compute_snapshot_fingerprint(&conn, later_tx.trade_date).unwrap();
        assert_ne!(fp1, fp_changed);
    }

    #[test]
    fn test_snapshot_save_and_load_roundtrip() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!("../db/schema.sql"))
            .unwrap();

        let asset_id = db::upsert_asset(&conn, "TEST4", &AssetType::Stock, None).unwrap();

        let tx = Transaction {
            id: None,
            asset_id,
            transaction_type: TransactionType::Buy,
            trade_date: NaiveDate::from_ymd_opt(2024, 1, 5).unwrap(),
            settlement_date: None,
            quantity: Decimal::from(5),
            price_per_unit: Decimal::from(10),
            total_cost: Decimal::from(50),
            fees: Decimal::ZERO,
            is_day_trade: false,
            quota_issuance_date: None,
            notes: None,
            source: "TEST".to_string(),
            created_at: Utc::now(),
        };

        db::insert_transaction(&conn, &tx).unwrap();

        let price = PriceHistory {
            id: None,
            asset_id,
            price_date: NaiveDate::from_ymd_opt(2024, 1, 6).unwrap(),
            close_price: Decimal::from(12),
            open_price: None,
            high_price: None,
            low_price: None,
            volume: Some(1_000),
            source: "TEST".to_string(),
            created_at: Utc::now(),
        };

        db::insert_price_history(&conn, &price).unwrap();

        let snapshot_date = NaiveDate::from_ymd_opt(2024, 1, 6).unwrap();
        save_portfolio_snapshot(&mut conn, snapshot_date, Some("label".to_string())).unwrap();

        let loaded = get_valid_snapshot(&conn, snapshot_date).unwrap();
        assert!(loaded.is_some());

        let report = loaded.unwrap();
        assert_eq!(report.positions.len(), 1);
        let position = &report.positions[0];
        assert_eq!(position.quantity, Decimal::from(5));
        assert_eq!(position.average_cost, Decimal::from(10));
        assert_eq!(position.current_price, Some(Decimal::from(12)));
        assert_eq!(position.current_value, Some(Decimal::from(60)));
        assert_eq!(position.unrealized_pl, Some(Decimal::from(10)));
    }
}
