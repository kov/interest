use anyhow::Result;
use chrono::NaiveDate;
use rusqlite::Connection;
use rust_decimal::Decimal;
use std::collections::HashMap;
use std::str::FromStr;

use crate::db::{Asset, AssetType, CorporateActionType, Transaction, TransactionType};

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
        let mut transactions = get_asset_transactions(conn, asset_id)?;

        for (source_ticker, effective_date) in crate::db::rename_sources_for(&asset.ticker) {
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

        for tx in transactions {
            match tx.transaction_type {
                TransactionType::Buy => {
                    position.add_buy(tx.quantity, tx.total_cost);
                }
                TransactionType::Sell => {
                    position.remove_sell(tx.quantity, &asset.ticker)?;
                }
            }
        }

        // Skip assets with zero quantity
        if position.quantity <= Decimal::ZERO {
            continue;
        }

        // Get current price
        let latest_price = crate::db::get_latest_price(conn, asset_id)?;
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
        .query_map([asset_id], |row| {
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
        })?
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
        .query_map(rusqlite::params![asset_id, before_date], |row| {
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
        })?
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

    let mut quantity = position.quantity;
    let mut total_cost = position.total_cost;
    apply_actions_to_carryover(
        conn,
        target_asset_id,
        effective_date,
        &mut quantity,
        &mut total_cost,
    )?;
    if quantity <= Decimal::ZERO {
        return Ok(None);
    }

    if let Some(target_qty) =
        crate::db::rename_quantity_override(target_ticker, &source_asset.ticker)
    {
        if target_qty > Decimal::ZERO && quantity > Decimal::ZERO && target_qty != quantity {
            // FIXME: consider amortized cash when quantity changes via rename.
            quantity = target_qty;
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

fn apply_actions_to_carryover(
    conn: &Connection,
    asset_id: i64,
    effective_date: NaiveDate,
    quantity: &mut Decimal,
    total_cost: &mut Decimal,
) -> Result<()> {
    let mut stmt = conn.prepare(
        "SELECT action_type, ratio_from, ratio_to, ex_date
         FROM corporate_actions
         WHERE asset_id = ?1 AND applied = 1 AND ex_date >= ?2
         ORDER BY ex_date ASC",
    )?;

    let actions = stmt
        .query_map(rusqlite::params![asset_id, effective_date], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i32>(1)?,
                row.get::<_, i32>(2)?,
                row.get::<_, NaiveDate>(3)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    for (action_type_str, ratio_from, ratio_to, _ex_date) in actions {
        let action_type = action_type_str
            .parse::<CorporateActionType>()
            .unwrap_or(CorporateActionType::Split);
        let ratio_from = Decimal::from(ratio_from);
        let ratio_to = Decimal::from(ratio_to);

        match action_type {
            CorporateActionType::CapitalReturn => {
                let amount_per_share = ratio_from / Decimal::from(100);
                let reduction = amount_per_share * *quantity;
                *total_cost = (*total_cost - reduction).max(Decimal::ZERO);
            }
            _ => {
                *quantity = *quantity * ratio_to / ratio_from;
            }
        }
    }

    Ok(())
}

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
