use anyhow::Result;
use chrono::NaiveDate;
use rusqlite::Connection;
use rust_decimal::Decimal;
use std::collections::HashMap;
use std::str::FromStr;

use super::cost_basis::{AverageCostMatcher, SaleCostBasis};
use crate::db::{Asset, AssetType, CorporateActionType, Transaction, TransactionType};

/// Tax category for operations
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TaxCategory {
    StockSwingTrade,  // 15%, R$20k exemption
    StockDayTrade,    // 20%, no exemption
    FiiSwingTrade,    // 20%, no exemption
    FiiDayTrade,      // 20%, no exemption
    FiagroSwingTrade, // 20%, no exemption
    FiagroDayTrade,   // 20%, no exemption
    FiInfra,          // Exempt
}

impl TaxCategory {
    pub fn from_asset_and_trade_type(asset_type: &AssetType, is_day_trade: bool) -> Self {
        match (asset_type, is_day_trade) {
            (AssetType::Stock, false) => TaxCategory::StockSwingTrade,
            (AssetType::Stock, true) => TaxCategory::StockDayTrade,
            (AssetType::Etf, false) => TaxCategory::StockSwingTrade,
            (AssetType::Etf, true) => TaxCategory::StockDayTrade,
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
            TaxCategory::FiagroDayTrade => Decimal::from_str("0.20").unwrap(),  // 20%
            TaxCategory::FiInfra => Decimal::ZERO,                              // Exempt
        }
    }

    pub fn monthly_exemption_threshold(&self) -> Decimal {
        match self {
            TaxCategory::StockSwingTrade => Decimal::from(20000), // R$20,000
            _ => Decimal::ZERO,                                   // No exemption for others
        }
    }

    #[allow(dead_code)]
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

    /// Returns DARF code for this tax category (Brazilian federal tax payment form)
    /// All capital gains use code 6015, but are reported separately
    pub fn darf_code(&self) -> Option<&'static str> {
        match self {
            TaxCategory::FiInfra => None, // Exempt, no DARF needed
            _ => Some("6015"),            // Capital gains code
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

impl FromStr for TaxCategory {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim() {
            "STOCK_SWING" => Ok(TaxCategory::StockSwingTrade),
            "STOCK_DAY" => Ok(TaxCategory::StockDayTrade),
            "FII_SWING" => Ok(TaxCategory::FiiSwingTrade),
            "FII_DAY" => Ok(TaxCategory::FiiDayTrade),
            "FIAGRO_SWING" => Ok(TaxCategory::FiagroSwingTrade),
            "FIAGRO_DAY" => Ok(TaxCategory::FiagroDayTrade),
            "FI_INFRA" => Ok(TaxCategory::FiInfra),
            _ => Err(()),
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
    pub loss_offset_applied: Decimal, // Amount of previous losses applied
    pub profit_after_loss_offset: Decimal, // Net profit after applying previous losses
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
    carryforward: &mut HashMap<TaxCategory, Decimal>,
) -> Result<Vec<MonthlyTaxCalculation>> {
    // Get all assets
    let assets = crate::db::get_all_assets(conn)?;

    // Group sales by tax category
    let mut sales_by_category: HashMap<TaxCategory, Vec<SaleCostBasis>> = HashMap::new();

    let month_start = NaiveDate::from_ymd_opt(year, month, 1).unwrap();
    let month_end = if month == 12 {
        NaiveDate::from_ymd_opt(year + 1, 1, 1)
            .unwrap()
            .pred_opt()
            .unwrap()
    } else {
        NaiveDate::from_ymd_opt(year, month + 1, 1)
            .unwrap()
            .pred_opt()
            .unwrap()
    };

    let mut assets_by_ticker = HashMap::new();
    for asset in &assets {
        assets_by_ticker.insert(asset.ticker.clone(), asset.clone());
    }

    // Process each asset ONCE
    for asset in assets {
        if !crate::db::is_supported_portfolio_ticker(&asset.ticker) {
            continue;
        }

        if crate::db::is_rename_source_ticker(&asset.ticker) {
            continue;
        }
        if !crate::db::is_supported_portfolio_ticker(&asset.ticker) {
            continue;
        }

        // Skip FI-Infra entirely
        if asset.asset_type == AssetType::FiInfra {
            continue;
        }

        let asset_id = asset.id.unwrap();

        // Get all transactions for this asset up to end of month
        let mut transactions = get_transactions_up_to_month(conn, asset_id, year, month)?;

        for (source_ticker, effective_date) in crate::db::rename_sources_for(&asset.ticker) {
            if effective_date > month_end {
                continue;
            }

            if let Some(source_asset) = assets_by_ticker.get(source_ticker) {
                if let Some(carryover) = build_rename_carryover_transaction(
                    conn,
                    source_asset,
                    asset_id,
                    effective_date,
                )? {
                    transactions.push(carryover);
                }
            }
        }

        transactions.sort_by(|a, b| (a.trade_date, a.id).cmp(&(b.trade_date, b.id)));

        // Calculate cost basis for sales in this month using average cost
        // Separate matchers for swing and day trade flows
        let mut swing_matcher = AverageCostMatcher::new();
        let mut day_trade_matcher = AverageCostMatcher::new();

        for tx in transactions {
            // Apply corporate action adjustments at query time
            let actions = crate::corporate_actions::get_applicable_actions(
                conn,
                asset_id,
                tx.trade_date,
                month_end,
            )?;

            let adjusted_quantity =
                crate::corporate_actions::adjust_quantity_for_actions(tx.quantity, &actions);

            let (_adjusted_price, adjusted_cost) =
                crate::corporate_actions::adjust_price_and_cost_for_actions(
                    tx.quantity,
                    tx.price_per_unit,
                    tx.total_cost,
                    &actions,
                );

            match tx.transaction_type {
                TransactionType::Buy => {
                    if tx.is_day_trade {
                        day_trade_matcher.add_purchase(
                            &tx,
                            Some(adjusted_quantity),
                            Some(adjusted_cost),
                        );
                    } else {
                        swing_matcher.add_purchase(
                            &tx,
                            Some(adjusted_quantity),
                            Some(adjusted_cost),
                        );
                    }
                }
                TransactionType::Sell => {
                    // Only process sales in the target month
                    if tx.trade_date >= month_start && tx.trade_date <= month_end {
                        // Determine category based on asset type and day trade flag
                        let category = TaxCategory::from_asset_and_trade_type(
                            &asset.asset_type,
                            tx.is_day_trade,
                        );

                        let mut sale = if tx.is_day_trade {
                            day_trade_matcher.match_sale(&tx, Some(adjusted_quantity))?
                        } else {
                            swing_matcher.match_sale(&tx, Some(adjusted_quantity))?
                        };
                        sale.asset_type = asset.asset_type;
                        sales_by_category.entry(category).or_default().push(sale);
                    } else if tx.trade_date > month_end {
                        // We've passed the target month, no need to process further
                        break;
                    } else {
                        // Sale before target month, still need to process to maintain average cost
                        if tx.is_day_trade {
                            let _ = day_trade_matcher.match_sale(&tx, Some(adjusted_quantity))?;
                        } else {
                            let _ = swing_matcher.match_sale(&tx, Some(adjusted_quantity))?;
                        }
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

        // Determine exemptable portion (only stock swing trades under R$20k sales)
        let exemption_threshold = category.monthly_exemption_threshold();
        let stock_sales_total: Decimal = sales
            .iter()
            .filter(|sale| sale.asset_type == AssetType::Stock)
            .map(|sale| sale.sale_total)
            .sum();
        let stock_profit_total: Decimal = sales
            .iter()
            .filter(|sale| sale.asset_type == AssetType::Stock)
            .map(|sale| sale.profit_loss)
            .sum();
        let exemptable_profit = if category == TaxCategory::StockSwingTrade
            && net_profit > Decimal::ZERO
            && stock_sales_total <= exemption_threshold
            && stock_profit_total > Decimal::ZERO
        {
            stock_profit_total.min(net_profit)
        } else {
            Decimal::ZERO
        };
        let profit_after_exemption = net_profit - exemptable_profit;

        // Apply loss carryforward only to the taxable portion (after exemption)
        let starting_carry = carryforward
            .get(&category)
            .cloned()
            .unwrap_or(Decimal::ZERO);
        let loss_offset_applied = if profit_after_exemption > Decimal::ZERO {
            profit_after_exemption.min(starting_carry)
        } else {
            Decimal::ZERO
        };
        let profit_after_loss_offset = profit_after_exemption - loss_offset_applied;

        let mut new_carry = starting_carry - loss_offset_applied;
        if profit_after_loss_offset < Decimal::ZERO {
            new_carry += profit_after_loss_offset.abs();
        }
        if new_carry.is_zero() {
            carryforward.remove(&category);
        } else {
            carryforward.insert(category.clone(), new_carry);
        }

        // Taxable amount excludes exempt stock profit; carry is untouched by exempt gains
        let (exemption_applied, taxable_amount) = if profit_after_loss_offset <= Decimal::ZERO {
            (exemptable_profit, Decimal::ZERO)
        } else {
            (exemptable_profit, profit_after_loss_offset)
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

fn get_transactions_before(
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
    effective_date: NaiveDate,
) -> Result<Option<Transaction>> {
    let source_id = match source_asset.id {
        Some(id) => id,
        None => return Ok(None),
    };

    let transactions = get_transactions_before(conn, source_id, effective_date)?;
    let mut matcher = AverageCostMatcher::new();

    for tx in transactions {
        if tx.is_day_trade {
            continue;
        }

        // Apply corporate action adjustments for rename carryover
        let actions = crate::corporate_actions::get_applicable_actions(
            conn,
            source_id,
            tx.trade_date,
            effective_date,
        )?;

        let adjusted_quantity =
            crate::corporate_actions::adjust_quantity_for_actions(tx.quantity, &actions);

        let (_adjusted_price, adjusted_cost) =
            crate::corporate_actions::adjust_price_and_cost_for_actions(
                tx.quantity,
                tx.price_per_unit,
                tx.total_cost,
                &actions,
            );

        match tx.transaction_type {
            TransactionType::Buy => {
                matcher.add_purchase(&tx, Some(adjusted_quantity), Some(adjusted_cost))
            }
            TransactionType::Sell => {
                let _ = matcher.match_sale(&tx, Some(adjusted_quantity))?;
            }
        }
    }

    let mut quantity = matcher.remaining_quantity();
    if quantity <= Decimal::ZERO {
        return Ok(None);
    }

    let mut total_cost = matcher.average_cost() * quantity;
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
         WHERE asset_id = ?1 AND ex_date >= ?2
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

/// Get all transactions for an asset up to the end of specified month
fn get_transactions_up_to_month(
    conn: &Connection,
    asset_id: i64,
    year: i32,
    month: u32,
) -> Result<Vec<Transaction>> {
    let end_date = if month == 12 {
        NaiveDate::from_ymd_opt(year + 1, 1, 1)
            .unwrap()
            .pred_opt()
            .unwrap()
    } else {
        NaiveDate::from_ymd_opt(year, month + 1, 1)
            .unwrap()
            .pred_opt()
            .unwrap()
    };

    let mut stmt = conn.prepare(
        "SELECT id, asset_id, transaction_type, trade_date, settlement_date,
                quantity, price_per_unit, total_cost, fees, is_day_trade,
                quota_issuance_date, notes, source, created_at
         FROM transactions
         WHERE asset_id = ?1 AND trade_date <= ?2
         ORDER BY trade_date ASC, id ASC",
    )?;

    let transactions = stmt
        .query_map([asset_id.to_string(), end_date.to_string()], |row| {
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
        assert_eq!(TaxCategory::FiInfra.tax_rate(), Decimal::ZERO);
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
