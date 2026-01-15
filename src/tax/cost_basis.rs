use anyhow::{anyhow, Result};
use chrono::NaiveDate;
use rust_decimal::Decimal;

use crate::db::models::AssetType;
use crate::db::{Transaction, TransactionType};

/// Cost basis result for a sale
#[derive(Debug, Clone)]
pub struct SaleCostBasis {
    #[allow(dead_code)]
    pub sale_date: NaiveDate,
    #[allow(dead_code)]
    pub quantity: Decimal,
    #[allow(dead_code)]
    pub sale_price: Decimal,
    pub sale_total: Decimal,
    pub cost_basis: Decimal,
    pub profit_loss: Decimal,
    #[allow(dead_code)]
    pub matched_lots: Vec<MatchedLot>,
    pub asset_type: AssetType,
}

/// A matched lot from average cost calculation
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct MatchedLot {
    pub purchase_date: NaiveDate,
    pub quantity: Decimal,
    pub cost: Decimal,
}

/// Average-cost matcher for calculating cost basis of sales
pub struct AverageCostMatcher {
    total_quantity: Decimal,
    total_cost: Decimal,
}

impl AverageCostMatcher {
    pub fn new() -> Self {
        Self {
            total_quantity: Decimal::ZERO,
            total_cost: Decimal::ZERO,
        }
    }

    /// Apply a quantity-only adjustment (e.g., split/reverse-split, bonus)
    /// This changes the total quantity while leaving total cost unchanged.
    /// Positive values increase quantity (lowering average price),
    /// negative values decrease quantity (raising average price).
    pub fn apply_quantity_adjustment(&mut self, adjustment: Decimal) {
        self.total_quantity += adjustment;
    }

    /// Add a purchase transaction with optional adjusted values
    /// If adjusted_quantity and adjusted_cost are None, uses tx values
    pub fn add_purchase(
        &mut self,
        tx: &Transaction,
        adjusted_quantity: Option<Decimal>,
        adjusted_cost: Option<Decimal>,
    ) {
        if tx.transaction_type != TransactionType::Buy {
            return;
        }

        let quantity = adjusted_quantity.unwrap_or(tx.quantity);
        let cost = adjusted_cost.unwrap_or(tx.total_cost);

        self.total_quantity += quantity;
        self.total_cost += cost;
    }

    /// Apply an amortization (capital return) to the running position.
    /// Quantity stays the same; total_cost is reduced by the returned capital.
    pub fn apply_amortization(&mut self, amount: Decimal) {
        if amount <= Decimal::ZERO {
            return;
        }

        self.total_cost -= amount;
        if self.total_cost < Decimal::ZERO {
            self.total_cost = Decimal::ZERO;
        }
    }

    /// Clear the current position without generating a sale (e.g., mergers/exchanges).
    pub fn clear_position(&mut self) {
        self.total_quantity = Decimal::ZERO;
        self.total_cost = Decimal::ZERO;
    }

    /// Match a sale using average cost up to that point, with optional adjusted quantity
    pub fn match_sale(
        &mut self,
        tx: &Transaction,
        adjusted_quantity: Option<Decimal>,
    ) -> Result<SaleCostBasis> {
        if tx.transaction_type != TransactionType::Sell {
            return Err(anyhow!("Transaction is not a sale"));
        }

        let quantity = adjusted_quantity.unwrap_or(tx.quantity);

        if quantity > self.total_quantity {
            return Err(anyhow!(
                "Insufficient purchase history for sale on {}. Selling {} units but only {} available.\n\
                \nThis usually means:\n\
                1. Shares came from sources not in the import (term contracts, transfers, etc.)\n\
                2. Incomplete transaction history in the CEI export\n\
                3. Short selling (not yet supported)\n\
                \nTo fix: Manually add the missing purchase transactions to the database or \n\
                adjust the import file to include all historical purchases.",
                tx.trade_date,
                quantity,
                self.total_quantity
            ));
        }

        let avg_cost = if self.total_quantity > Decimal::ZERO {
            self.total_cost / self.total_quantity
        } else {
            Decimal::ZERO
        };

        let cost_basis = avg_cost * quantity;
        self.total_quantity -= quantity;
        self.total_cost -= cost_basis;

        let sale_total = tx.total_cost.abs();
        let profit_loss = sale_total - cost_basis - tx.fees;

        Ok(SaleCostBasis {
            sale_date: tx.trade_date,
            quantity,
            sale_price: tx.price_per_unit,
            sale_total,
            cost_basis,
            profit_loss,
            matched_lots: vec![MatchedLot {
                purchase_date: tx.trade_date,
                quantity,
                cost: cost_basis,
            }],
            asset_type: AssetType::Stock,
        })
    }

    #[allow(dead_code)]
    pub fn remaining_quantity(&self) -> Decimal {
        self.total_quantity
    }

    #[allow(dead_code)]
    pub fn average_cost(&self) -> Decimal {
        if self.total_quantity > Decimal::ZERO {
            self.total_cost / self.total_quantity
        } else {
            Decimal::ZERO
        }
    }
}

impl Default for AverageCostMatcher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use rust_decimal_macros::dec;

    fn make_buy(date: NaiveDate, qty: i32, price: i32) -> Transaction {
        Transaction {
            id: None,
            asset_id: 1,
            transaction_type: TransactionType::Buy,
            trade_date: date,
            settlement_date: Some(date),
            quantity: Decimal::from(qty),
            price_per_unit: Decimal::from(price),
            total_cost: Decimal::from(qty * price),
            fees: Decimal::ZERO,
            is_day_trade: false,
            quota_issuance_date: None,
            notes: None,
            source: "TEST".to_string(),
            created_at: chrono::Utc::now(),
        }
    }

    fn make_sell(date: NaiveDate, qty: i32, price: i32) -> Transaction {
        Transaction {
            id: None,
            asset_id: 1,
            transaction_type: TransactionType::Sell,
            trade_date: date,
            settlement_date: Some(date),
            quantity: Decimal::from(qty),
            price_per_unit: Decimal::from(price),
            total_cost: Decimal::from(qty * price),
            fees: Decimal::ZERO,
            is_day_trade: false,
            quota_issuance_date: None,
            notes: None,
            source: "TEST".to_string(),
            created_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn test_avg_cost_simple() {
        let mut matcher = AverageCostMatcher::new();
        let buy = make_buy(NaiveDate::from_ymd_opt(2025, 1, 10).unwrap(), 100, 10);
        matcher.add_purchase(&buy, None, None);

        let sell = make_sell(NaiveDate::from_ymd_opt(2025, 2, 10).unwrap(), 50, 12);
        let result = matcher.match_sale(&sell, None).unwrap();

        assert_eq!(result.cost_basis, dec!(500));
        assert_eq!(matcher.remaining_quantity(), dec!(50));
        assert_eq!(matcher.average_cost(), dec!(10));
    }

    #[test]
    fn test_avg_cost_multiple_buys() {
        let mut matcher = AverageCostMatcher::new();
        let buy1 = make_buy(NaiveDate::from_ymd_opt(2025, 1, 10).unwrap(), 100, 10);
        let buy2 = make_buy(NaiveDate::from_ymd_opt(2025, 2, 10).unwrap(), 50, 20);
        matcher.add_purchase(&buy1, None, None);
        matcher.add_purchase(&buy2, None, None);

        let sell = make_sell(NaiveDate::from_ymd_opt(2025, 3, 10).unwrap(), 60, 15);
        let result = matcher.match_sale(&sell, None).unwrap();

        let expected_avg = (buy1.total_cost + buy2.total_cost) / (buy1.quantity + buy2.quantity);
        let expected_cost_basis = expected_avg * sell.quantity;

        assert_eq!(result.cost_basis, expected_cost_basis);
        assert_eq!(matcher.remaining_quantity(), dec!(90));
        assert_eq!(matcher.average_cost(), expected_avg);
    }

    #[test]
    fn test_avg_cost_with_fees() {
        let mut matcher = AverageCostMatcher::new();
        let buy = make_buy(NaiveDate::from_ymd_opt(2025, 1, 10).unwrap(), 100, 10);
        matcher.add_purchase(&buy, None, None);

        let mut sell = make_sell(NaiveDate::from_ymd_opt(2025, 2, 10).unwrap(), 40, 12);
        sell.fees = dec!(5);

        let result = matcher.match_sale(&sell, None).unwrap();
        assert_eq!(result.cost_basis, dec!(400));
        assert_eq!(result.profit_loss, dec!(75));
    }

    #[test]
    fn test_avg_cost_oversell() {
        let mut matcher = AverageCostMatcher::new();
        let buy = make_buy(NaiveDate::from_ymd_opt(2025, 1, 10).unwrap(), 10, 10);
        matcher.add_purchase(&buy, None, None);

        let sell = make_sell(NaiveDate::from_ymd_opt(2025, 2, 10).unwrap(), 20, 12);
        let result = matcher.match_sale(&sell, None);
        assert!(result.is_err());
    }
}
