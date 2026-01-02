use anyhow::Result;
use chrono::NaiveDate;
use rusqlite::Connection;
use rust_decimal::Decimal;
use std::collections::HashMap;
use std::str::FromStr;

use crate::db::{Asset, AssetType, Transaction, TransactionType};
use super::cost_basis::{FifoMatcher, SaleCostBasis};

/// Monthly tax calculation result
#[derive(Debug, Clone)]
pub struct MonthlyTaxCalculation {
    #[allow(dead_code)]
    pub year: i32,
    #[allow(dead_code)]
    pub month: u32,
    pub asset_type: AssetType,
    pub total_sales: Decimal,
    pub total_cost_basis: Decimal,
    pub total_profit: Decimal,
    pub total_loss: Decimal,
    pub net_profit: Decimal,
    pub exemption_applied: Decimal,
    pub taxable_amount: Decimal,
    #[allow(dead_code)]
    pub tax_rate: Decimal,
    pub tax_due: Decimal,
    #[allow(dead_code)]
    pub sales: Vec<SaleCostBasis>,
}

/// Calculate monthly swing trade tax for a specific month
pub fn calculate_monthly_tax(
    conn: &Connection,
    year: i32,
    month: u32,
) -> Result<Vec<MonthlyTaxCalculation>> {
    // Get all assets
    let assets = crate::db::get_all_assets(conn)?;

    // Group assets by type
    let mut assets_by_type: HashMap<AssetType, Vec<Asset>> = HashMap::new();
    for asset in assets {
        assets_by_type.entry(asset.asset_type.clone())
            .or_insert_with(Vec::new)
            .push(asset);
    }

    let mut results = Vec::new();

    // Process each asset type separately
    for (asset_type, assets_list) in assets_by_type {
        let mut total_sales = Decimal::ZERO;
        let mut total_cost_basis = Decimal::ZERO;
        let mut total_profit = Decimal::ZERO;
        let mut total_loss = Decimal::ZERO;
        let mut all_sales = Vec::new();

        // Process each asset of this type
        for asset in assets_list {
            let asset_id = asset.id.unwrap();

            // Get all transactions for this asset up to end of month
            let transactions = get_transactions_up_to_month(conn, asset_id, year, month)?;

            // Calculate cost basis for sales in this month using FIFO
            let mut matcher = FifoMatcher::new();
            let month_start = NaiveDate::from_ymd_opt(year, month, 1).unwrap();
            let month_end = if month == 12 {
                NaiveDate::from_ymd_opt(year + 1, 1, 1).unwrap().pred_opt().unwrap()
            } else {
                NaiveDate::from_ymd_opt(year, month + 1, 1).unwrap().pred_opt().unwrap()
            };

            for tx in transactions {
                match tx.transaction_type {
                    TransactionType::Buy => {
                        matcher.add_purchase(&tx);
                    }
                    TransactionType::Sell => {
                        // Only process sales in the target month
                        if tx.trade_date >= month_start && tx.trade_date <= month_end {
                            let sale = matcher.match_sale(&tx)?;

                            total_sales += sale.sale_total;
                            total_cost_basis += sale.cost_basis;

                            if sale.profit_loss > Decimal::ZERO {
                                total_profit += sale.profit_loss;
                            } else {
                                total_loss += sale.profit_loss.abs();
                            }

                            all_sales.push(sale);
                        } else if tx.trade_date > month_end {
                            // We've passed the target month, no need to process further
                            break;
                        } else {
                            // Sale before target month, still need to process for FIFO
                            let _ = matcher.match_sale(&tx)?;
                        }
                    }
                }
            }
        }

        // Skip if no sales in this month
        if all_sales.is_empty() {
            continue;
        }

        // Calculate net profit/loss
        let net_profit = total_profit - total_loss;

        // Apply exemption (R$20,000 for stocks only)
        let exemption_limit = match asset_type {
            AssetType::Stock => Decimal::from(20000),
            _ => Decimal::ZERO, // FII, FIAGRO, etc. have no exemption
        };

        let (exemption_applied, taxable_amount) = if total_sales <= exemption_limit {
            // Full exemption
            (net_profit.max(Decimal::ZERO), Decimal::ZERO)
        } else if net_profit > Decimal::ZERO {
            // Partial or no exemption
            (Decimal::ZERO, net_profit)
        } else {
            // Loss - no exemption or tax
            (Decimal::ZERO, Decimal::ZERO)
        };

        // Calculate tax (15% on taxable amount)
        let tax_rate = Decimal::from_str("0.15").unwrap(); // 15%
        let tax_due = taxable_amount * tax_rate;

        results.push(MonthlyTaxCalculation {
            year,
            month,
            asset_type,
            total_sales,
            total_cost_basis,
            total_profit,
            total_loss,
            net_profit,
            exemption_applied,
            taxable_amount,
            tax_rate,
            tax_due,
            sales: all_sales,
        });
    }

    Ok(results)
}

/// Get all transactions for an asset up to the end of specified month
fn get_transactions_up_to_month(
    conn: &Connection,
    asset_id: i64,
    year: i32,
    month: u32,
) -> Result<Vec<Transaction>> {
    let end_date = if month == 12 {
        NaiveDate::from_ymd_opt(year + 1, 1, 1).unwrap().pred_opt().unwrap()
    } else {
        NaiveDate::from_ymd_opt(year, month + 1, 1).unwrap().pred_opt().unwrap()
    };

    let mut stmt = conn.prepare(
        "SELECT id, asset_id, transaction_type, trade_date, settlement_date,
                quantity, price_per_unit, total_cost, fees, is_day_trade,
                quota_issuance_date, notes, source, created_at
         FROM transactions
         WHERE asset_id = ?1 AND trade_date <= ?2
         ORDER BY trade_date ASC, id ASC"
    )?;

    let transactions = stmt
        .query_map([asset_id.to_string(), end_date.to_string()], |row| {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exemption_logic() {
        // Sales under R$20,000 - full exemption for stocks
        let sales = Decimal::from(15000);
        let profit = Decimal::from(2000);
        let exemption_limit = Decimal::from(20000);

        let (exemption, taxable) = if sales <= exemption_limit {
            (profit, Decimal::ZERO)
        } else {
            (Decimal::ZERO, profit)
        };

        assert_eq!(exemption, Decimal::from(2000));
        assert_eq!(taxable, Decimal::ZERO);

        // Sales over R$20,000 - no exemption
        let sales2 = Decimal::from(25000);
        let profit2 = Decimal::from(3000);

        let (exemption2, taxable2) = if sales2 <= exemption_limit {
            (profit2, Decimal::ZERO)
        } else {
            (Decimal::ZERO, profit2)
        };

        assert_eq!(exemption2, Decimal::ZERO);
        assert_eq!(taxable2, Decimal::from(3000));
    }

    #[test]
    fn test_tax_calculation() {
        let taxable = Decimal::from(10000);
        let tax_rate = Decimal::from_str("0.15").unwrap();
        let tax_due = taxable * tax_rate;

        assert_eq!(tax_due, Decimal::from(1500));
    }
}
