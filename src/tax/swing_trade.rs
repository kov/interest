use anyhow::Result;
use chrono::NaiveDate;
use rusqlite::Connection;
use rust_decimal::Decimal;
use std::collections::HashMap;
use std::str::FromStr;

use crate::db::{Asset, AssetType, Transaction, TransactionType};
use super::cost_basis::{FifoMatcher, SaleCostBasis};
use super::loss_carryforward;

/// Tax category for operations
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TaxCategory {
    StockSwingTrade,   // 15%, R$20k exemption
    StockDayTrade,     // 20%, no exemption
    FiiSwingTrade,     // 20%, no exemption
    FiiDayTrade,       // 20%, no exemption
    FiagroSwingTrade,  // 20%, no exemption
    FiagroDayTrade,    // 20%, no exemption
    FiInfra,           // Exempt
}

impl TaxCategory {
    pub fn from_asset_and_trade_type(asset_type: &AssetType, is_day_trade: bool) -> Self {
        match (asset_type, is_day_trade) {
            (AssetType::Stock, false) => TaxCategory::StockSwingTrade,
            (AssetType::Stock, true) => TaxCategory::StockDayTrade,
            (AssetType::Fii, false) => TaxCategory::FiiSwingTrade,
            (AssetType::Fii, true) => TaxCategory::FiiDayTrade,
            (AssetType::Fiagro, false) => TaxCategory::FiagroSwingTrade,
            (AssetType::Fiagro, true) => TaxCategory::FiagroDayTrade,
            (AssetType::FiInfra, _) => TaxCategory::FiInfra,
            _ => TaxCategory::StockSwingTrade, // Default for bonds, etc.
        }
    }

    pub fn tax_rate(&self) -> Decimal {
        match self {
            TaxCategory::StockSwingTrade => Decimal::from_str("0.15").unwrap(), // 15%
            TaxCategory::StockDayTrade => Decimal::from_str("0.20").unwrap(),   // 20%
            TaxCategory::FiiSwingTrade => Decimal::from_str("0.20").unwrap(),   // 20%
            TaxCategory::FiiDayTrade => Decimal::from_str("0.20").unwrap(),     // 20%
            TaxCategory::FiagroSwingTrade => Decimal::from_str("0.20").unwrap(), // 20%
            TaxCategory::FiagroDayTrade => Decimal::from_str("0.20").unwrap(),   // 20%
            TaxCategory::FiInfra => Decimal::ZERO, // Exempt
        }
    }

    pub fn monthly_exemption_threshold(&self) -> Decimal {
        match self {
            TaxCategory::StockSwingTrade => Decimal::from(20000), // R$20,000
            _ => Decimal::ZERO, // No exemption for others
        }
    }

    pub fn is_exempt(&self) -> bool {
        matches!(self, TaxCategory::FiInfra)
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            TaxCategory::StockSwingTrade => "Ações (Swing Trade)",
            TaxCategory::StockDayTrade => "Ações (Day Trade)",
            TaxCategory::FiiSwingTrade => "FII (Swing Trade)",
            TaxCategory::FiiDayTrade => "FII (Day Trade)",
            TaxCategory::FiagroSwingTrade => "FIAGRO (Swing Trade)",
            TaxCategory::FiagroDayTrade => "FIAGRO (Day Trade)",
            TaxCategory::FiInfra => "FI-Infra (Isento)",
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            TaxCategory::StockSwingTrade => "STOCK_SWING",
            TaxCategory::StockDayTrade => "STOCK_DAY",
            TaxCategory::FiiSwingTrade => "FII_SWING",
            TaxCategory::FiiDayTrade => "FII_DAY",
            TaxCategory::FiagroSwingTrade => "FIAGRO_SWING",
            TaxCategory::FiagroDayTrade => "FIAGRO_DAY",
            TaxCategory::FiInfra => "FI_INFRA",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "STOCK_SWING" => Some(TaxCategory::StockSwingTrade),
            "STOCK_DAY" => Some(TaxCategory::StockDayTrade),
            "FII_SWING" => Some(TaxCategory::FiiSwingTrade),
            "FII_DAY" => Some(TaxCategory::FiiDayTrade),
            "FIAGRO_SWING" => Some(TaxCategory::FiagroSwingTrade),
            "FIAGRO_DAY" => Some(TaxCategory::FiagroDayTrade),
            "FI_INFRA" => Some(TaxCategory::FiInfra),
            _ => None,
        }
    }

    /// Returns DARF code for this tax category (Brazilian federal tax payment form)
    /// All capital gains use code 6015, but are reported separately
    pub fn darf_code(&self) -> Option<&'static str> {
        match self {
            TaxCategory::FiInfra => None, // Exempt, no DARF needed
            _ => Some("6015"), // Capital gains code
        }
    }

    /// Returns a description for DARF payment purposes
    pub fn darf_description(&self) -> &'static str {
        match self {
            TaxCategory::StockSwingTrade => "Renda Variável - Operações Comuns",
            TaxCategory::StockDayTrade => "Renda Variável - Day Trade",
            TaxCategory::FiiSwingTrade => "FII - Operações Comuns",
            TaxCategory::FiiDayTrade => "FII - Day Trade",
            TaxCategory::FiagroSwingTrade => "FIAGRO - Operações Comuns",
            TaxCategory::FiagroDayTrade => "FIAGRO - Day Trade",
            TaxCategory::FiInfra => "FI-Infra - Isento",
        }
    }
}

/// Monthly tax calculation result
#[derive(Debug, Clone)]
pub struct MonthlyTaxCalculation {
    #[allow(dead_code)]
    pub year: i32,
    #[allow(dead_code)]
    pub month: u32,
    pub category: TaxCategory,
    pub total_sales: Decimal,
    pub total_cost_basis: Decimal,
    pub total_profit: Decimal,
    pub total_loss: Decimal,
    pub net_profit: Decimal,
    pub loss_offset_applied: Decimal,  // Amount of previous losses applied
    pub profit_after_loss_offset: Decimal,  // Net profit after applying previous losses
    pub exemption_applied: Decimal,
    pub taxable_amount: Decimal,
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

    // Group sales by tax category
    let mut sales_by_category: HashMap<TaxCategory, Vec<SaleCostBasis>> = HashMap::new();

    let month_start = NaiveDate::from_ymd_opt(year, month, 1).unwrap();
    let month_end = if month == 12 {
        NaiveDate::from_ymd_opt(year + 1, 1, 1).unwrap().pred_opt().unwrap()
    } else {
        NaiveDate::from_ymd_opt(year, month + 1, 1).unwrap().pred_opt().unwrap()
    };

    // Process each asset ONCE
    for asset in assets {
        // Skip FI-Infra entirely
        if asset.asset_type == AssetType::FiInfra {
            continue;
        }

        let asset_id = asset.id.unwrap();

        // Get all transactions for this asset up to end of month
        let transactions = get_transactions_up_to_month(conn, asset_id, year, month)?;

        // Calculate cost basis for sales in this month using FIFO
        // ONE matcher per asset, shared between swing and day trades
        let mut matcher = FifoMatcher::new();

        for tx in transactions {
            match tx.transaction_type {
                TransactionType::Buy => {
                    matcher.add_purchase(&tx);
                }
                TransactionType::Sell => {
                    // Only process sales in the target month
                    if tx.trade_date >= month_start && tx.trade_date <= month_end {
                        // Determine category based on asset type and day trade flag
                        let category = TaxCategory::from_asset_and_trade_type(
                            &asset.asset_type,
                            tx.is_day_trade
                        );

                        let sale = matcher.match_sale(&tx)?;
                        sales_by_category.entry(category)
                            .or_insert_with(Vec::new)
                            .push(sale);
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

    // Now calculate tax for each category
    let mut results = Vec::new();

    for (category, sales) in sales_by_category {
        if sales.is_empty() {
            continue;
        }

        let mut total_sales = Decimal::ZERO;
        let mut total_cost_basis = Decimal::ZERO;
        let mut total_profit = Decimal::ZERO;
        let mut total_loss = Decimal::ZERO;

        for sale in &sales {
            total_sales += sale.sale_total;
            total_cost_basis += sale.cost_basis;

            if sale.profit_loss > Decimal::ZERO {
                total_profit += sale.profit_loss;
            } else {
                total_loss += sale.profit_loss.abs();
            }
        }

        // Calculate net profit/loss
        let net_profit = total_profit - total_loss;

        // Apply loss carryforward if there's a profit
        let (profit_after_loss_offset, loss_offset_applied) = if net_profit > Decimal::ZERO {
            loss_carryforward::apply_losses_to_profit(conn, &category, net_profit)?
        } else {
            (net_profit, Decimal::ZERO)
        };

        // Record new loss if there is one
        if profit_after_loss_offset < Decimal::ZERO {
            loss_carryforward::record_loss(
                conn,
                year,
                month,
                &category,
                profit_after_loss_offset.abs(),
            )?;
        }

        // Apply exemption threshold (only to positive profit)
        let exemption_threshold = category.monthly_exemption_threshold();
        let (exemption_applied, taxable_amount) = if profit_after_loss_offset <= Decimal::ZERO {
            // Loss or break-even - no tax
            (Decimal::ZERO, Decimal::ZERO)
        } else if exemption_threshold > Decimal::ZERO && total_sales <= exemption_threshold {
            // Full exemption (stocks under R$20k)
            (profit_after_loss_offset, Decimal::ZERO)
        } else {
            // No exemption, full tax
            (Decimal::ZERO, profit_after_loss_offset)
        };

        // Calculate tax
        let tax_rate = category.tax_rate();
        let tax_due = taxable_amount * tax_rate;

        results.push(MonthlyTaxCalculation {
            year,
            month,
            category,
            total_sales,
            total_cost_basis,
            total_profit,
            total_loss,
            net_profit,
            loss_offset_applied,
            profit_after_loss_offset,
            exemption_applied,
            taxable_amount,
            tax_rate,
            tax_due,
            sales,
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
    fn test_tax_category_rates() {
        assert_eq!(
            TaxCategory::StockSwingTrade.tax_rate(),
            Decimal::from_str("0.15").unwrap()
        );
        assert_eq!(
            TaxCategory::StockDayTrade.tax_rate(),
            Decimal::from_str("0.20").unwrap()
        );
        assert_eq!(
            TaxCategory::FiiSwingTrade.tax_rate(),
            Decimal::from_str("0.20").unwrap()
        );
        assert_eq!(
            TaxCategory::FiInfra.tax_rate(),
            Decimal::ZERO
        );
    }

    #[test]
    fn test_tax_category_exemptions() {
        assert_eq!(
            TaxCategory::StockSwingTrade.monthly_exemption_threshold(),
            Decimal::from(20000)
        );
        assert_eq!(
            TaxCategory::StockDayTrade.monthly_exemption_threshold(),
            Decimal::ZERO
        );
        assert_eq!(
            TaxCategory::FiiSwingTrade.monthly_exemption_threshold(),
            Decimal::ZERO
        );
        assert_eq!(
            TaxCategory::FiInfra.monthly_exemption_threshold(),
            Decimal::ZERO
        );
    }

    #[test]
    fn test_fi_infra_exempt() {
        assert!(TaxCategory::FiInfra.is_exempt());
        assert!(!TaxCategory::StockSwingTrade.is_exempt());
        assert!(!TaxCategory::FiiSwingTrade.is_exempt());
    }

    #[test]
    fn test_exemption_logic() {
        // Sales under R$20,000 - full exemption for stock swing trades
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
    fn test_tax_calculation_stock_swing() {
        let taxable = Decimal::from(10000);
        let tax_rate = TaxCategory::StockSwingTrade.tax_rate();
        let tax_due = taxable * tax_rate;

        assert_eq!(tax_due, Decimal::from(1500)); // 15%
    }

    #[test]
    fn test_tax_calculation_day_trade() {
        let taxable = Decimal::from(10000);
        let tax_rate = TaxCategory::StockDayTrade.tax_rate();
        let tax_due = taxable * tax_rate;

        assert_eq!(tax_due, Decimal::from(2000)); // 20%
    }

    #[test]
    fn test_tax_calculation_fii() {
        let taxable = Decimal::from(10000);
        let tax_rate = TaxCategory::FiiSwingTrade.tax_rate();
        let tax_due = taxable * tax_rate;

        assert_eq!(tax_due, Decimal::from(2000)); // 20%
    }
}
