use anyhow::Result;
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

/// FIFO position tracker for a single asset
#[derive(Debug)]
struct FifoPosition {
    #[allow(dead_code)]
    asset_id: i64,
    quantity: Decimal,
    total_cost: Decimal,
}

impl FifoPosition {
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

/// Calculate current portfolio positions using FIFO
pub fn calculate_portfolio(
    conn: &Connection,
    asset_type_filter: Option<&AssetType>,
) -> Result<PortfolioReport> {
    // Get all assets
    let assets = crate::db::get_all_assets(conn)?;

    // Filter by asset type if requested
    let filtered_assets: Vec<_> = if let Some(filter) = asset_type_filter {
        assets.into_iter().filter(|a| &a.asset_type == filter).collect()
    } else {
        assets
    };

    // Calculate positions for each asset
    let mut positions = Vec::new();
    let mut total_cost = Decimal::ZERO;
    let mut total_value = Decimal::ZERO;

    for asset in filtered_assets {
        let asset_id = asset.id.unwrap();

        // Get all transactions for this asset, ordered by date
        let transactions = get_asset_transactions(conn, asset_id)?;

        // Calculate FIFO position
        let mut position = FifoPosition::new(asset_id);

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
         ORDER BY trade_date ASC, id ASC"
    )?;

    let transactions = stmt
        .query_map([asset_id], |row| {
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

    Ok(transactions)
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
        rusqlite::types::Type::Null
    ))
}

/// Calculate asset allocation breakdown
pub fn calculate_allocation(report: &PortfolioReport) -> HashMap<AssetType, (Decimal, Decimal)> {
    let mut allocation: HashMap<AssetType, (Decimal, Decimal)> = HashMap::new();

    for position in &report.positions {
        let value = position.current_value.unwrap_or(position.total_cost);
        let entry = allocation.entry(position.asset.asset_type.clone()).or_insert((Decimal::ZERO, Decimal::ZERO));
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
    use chrono::Utc;

    #[test]
    fn test_fifo_position_buy_and_sell() {
        let mut position = FifoPosition::new(1);

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
        let cost_basis = position.remove_sell(Decimal::from(75)).unwrap();
        assert_eq!(position.quantity, Decimal::from(75));

        // Cost basis for sold units should be 75 * avg_cost
        assert!(cost_basis > Decimal::from(875) && cost_basis < Decimal::from(876));
    }

    #[test]
    fn test_fifo_position_oversell() {
        let mut position = FifoPosition::new(1);
        position.add_buy(Decimal::from(100), Decimal::from(1000));

        // Try to sell more than we have
        let result = position.remove_sell(Decimal::from(150));
        assert!(result.is_err());
    }
}
