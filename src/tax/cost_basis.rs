use anyhow::{anyhow, Result};
use chrono::NaiveDate;
use rust_decimal::Decimal;
use std::collections::VecDeque;

use crate::db::{Transaction, TransactionType};

/// A purchase lot in FIFO queue
#[derive(Debug, Clone)]
struct PurchaseLot {
    date: NaiveDate,
    quantity: Decimal,
    #[allow(dead_code)]
    price_per_unit: Decimal,
    total_cost: Decimal,
    remaining: Decimal,
}

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
}

/// A matched lot from FIFO
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct MatchedLot {
    pub purchase_date: NaiveDate,
    pub quantity: Decimal,
    pub cost: Decimal,
}

/// FIFO matcher for calculating cost basis of sales
pub struct FifoMatcher {
    purchase_queue: VecDeque<PurchaseLot>,
}

impl FifoMatcher {
    pub fn new() -> Self {
        Self {
            purchase_queue: VecDeque::new(),
        }
    }

    /// Add a purchase transaction
    pub fn add_purchase(&mut self, tx: &Transaction) {
        if tx.transaction_type != TransactionType::Buy {
            return;
        }

        let lot = PurchaseLot {
            date: tx.trade_date,
            quantity: tx.quantity,
            price_per_unit: tx.price_per_unit,
            total_cost: tx.total_cost,
            remaining: tx.quantity,
        };

        self.purchase_queue.push_back(lot);
    }

    /// Match a sale against purchases using FIFO
    pub fn match_sale(&mut self, tx: &Transaction) -> Result<SaleCostBasis> {
        if tx.transaction_type != TransactionType::Sell {
            return Err(anyhow!("Transaction is not a sale"));
        }

        let mut remaining_to_sell = tx.quantity;
        let mut total_cost_basis = Decimal::ZERO;
        let mut matched_lots = Vec::new();

        // Match against purchase lots in FIFO order
        while remaining_to_sell > Decimal::ZERO {
            let lot = self.purchase_queue.front_mut()
                .ok_or_else(|| anyhow!(
                    "Insufficient purchase history for sale on {}. Selling {} units but no purchases available.\n\
                    \nThis usually means:\n\
                    1. Shares came from sources not in the import (term contracts, transfers, etc.)\n\
                    2. Incomplete transaction history in the CEI export\n\
                    3. Short selling (not yet supported)\n\
                    \nTo fix: Manually add the missing purchase transactions to the database or \n\
                    adjust the import file to include all historical purchases.",
                    tx.trade_date,
                    remaining_to_sell
                ))?;

            if lot.remaining <= Decimal::ZERO {
                // This lot is exhausted, remove it
                self.purchase_queue.pop_front();
                continue;
            }

            // Calculate how much to take from this lot
            let qty_from_lot = remaining_to_sell.min(lot.remaining);

            // Calculate proportional cost from this lot
            let cost_per_unit = lot.total_cost / lot.quantity;
            let cost_from_lot = cost_per_unit * qty_from_lot;

            // Record the match
            matched_lots.push(MatchedLot {
                purchase_date: lot.date,
                quantity: qty_from_lot,
                cost: cost_from_lot,
            });

            total_cost_basis += cost_from_lot;
            lot.remaining -= qty_from_lot;
            remaining_to_sell -= qty_from_lot;

            // If lot is now empty, remove it
            if lot.remaining <= Decimal::ZERO {
                self.purchase_queue.pop_front();
            }
        }

        let sale_total = tx.total_cost.abs(); // Sales have negative total_cost in some systems
        let profit_loss = sale_total - total_cost_basis - tx.fees;

        Ok(SaleCostBasis {
            sale_date: tx.trade_date,
            quantity: tx.quantity,
            sale_price: tx.price_per_unit,
            sale_total,
            cost_basis: total_cost_basis,
            profit_loss,
            matched_lots,
        })
    }

    /// Get remaining quantity in purchase queue
    #[allow(dead_code)]
    pub fn remaining_quantity(&self) -> Decimal {
        self.purchase_queue.iter().map(|lot| lot.remaining).sum()
    }
}

impl Default for FifoMatcher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn make_buy(date: NaiveDate, qty: i32, price: i32) -> Transaction {
        let qty_dec = Decimal::from(qty);
        let price_dec = Decimal::from(price);
        Transaction {
            id: None,
            asset_id: 1,
            transaction_type: TransactionType::Buy,
            trade_date: date,
            settlement_date: date,
            quantity: qty_dec,
            price_per_unit: price_dec,
            total_cost: qty_dec * price_dec,
            fees: Decimal::ZERO,
            is_day_trade: false,
            quota_issuance_date: None,
            notes: None,
            source: "TEST".to_string(),
            created_at: Utc::now(),
        }
    }

    fn make_sell(date: NaiveDate, qty: i32, price: i32) -> Transaction {
        let qty_dec = Decimal::from(qty);
        let price_dec = Decimal::from(price);
        Transaction {
            id: None,
            asset_id: 1,
            transaction_type: TransactionType::Sell,
            trade_date: date,
            settlement_date: date,
            quantity: qty_dec,
            price_per_unit: price_dec,
            total_cost: qty_dec * price_dec,
            fees: Decimal::ZERO,
            is_day_trade: false,
            quota_issuance_date: None,
            notes: None,
            source: "TEST".to_string(),
            created_at: Utc::now(),
        }
    }

    #[test]
    fn test_fifo_simple() {
        let mut matcher = FifoMatcher::new();

        // Buy 100 @ R$10 = R$1000
        let buy1 = make_buy(
            NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
            100,
            10
        );
        matcher.add_purchase(&buy1);

        // Sell 50 @ R$15 = R$750
        let sell1 = make_sell(
            NaiveDate::from_ymd_opt(2025, 2, 1).unwrap(),
            50,
            15
        );
        let result = matcher.match_sale(&sell1).unwrap();

        assert_eq!(result.quantity, Decimal::from(50));
        assert_eq!(result.cost_basis, Decimal::from(500)); // 50 * 10
        assert_eq!(result.sale_total, Decimal::from(750)); // 50 * 15
        assert_eq!(result.profit_loss, Decimal::from(250)); // 750 - 500
        assert_eq!(matcher.remaining_quantity(), Decimal::from(50));
    }

    #[test]
    fn test_fifo_multiple_lots() {
        let mut matcher = FifoMatcher::new();

        // Buy 100 @ R$10 = R$1000
        let buy1 = make_buy(
            NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
            100,
            10
        );
        matcher.add_purchase(&buy1);

        // Buy 100 @ R$15 = R$1500
        let buy2 = make_buy(
            NaiveDate::from_ymd_opt(2025, 1, 15).unwrap(),
            100,
            15
        );
        matcher.add_purchase(&buy2);

        // Sell 150 @ R$20 = R$3000
        // Should take all 100 from first lot (cost R$1000)
        // and 50 from second lot (cost 50 * 15 = R$750)
        // Total cost basis: R$1750
        let sell1 = make_sell(
            NaiveDate::from_ymd_opt(2025, 2, 1).unwrap(),
            150,
            20
        );
        let result = matcher.match_sale(&sell1).unwrap();

        assert_eq!(result.quantity, Decimal::from(150));
        assert_eq!(result.cost_basis, Decimal::from(1750));
        assert_eq!(result.sale_total, Decimal::from(3000));
        assert_eq!(result.profit_loss, Decimal::from(1250)); // 3000 - 1750
        assert_eq!(result.matched_lots.len(), 2);
        assert_eq!(matcher.remaining_quantity(), Decimal::from(50));
    }

    #[test]
    fn test_fifo_oversell() {
        let mut matcher = FifoMatcher::new();

        // Buy 100 @ R$10
        let buy1 = make_buy(
            NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
            100,
            10
        );
        matcher.add_purchase(&buy1);

        // Try to sell 150 (more than available)
        let sell1 = make_sell(
            NaiveDate::from_ymd_opt(2025, 2, 1).unwrap(),
            150,
            15
        );
        let result = matcher.match_sale(&sell1);

        assert!(result.is_err());
    }
}
