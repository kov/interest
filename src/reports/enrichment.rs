use anyhow::Result;
use chrono::NaiveDate;
use rusqlite::Connection;
use rust_decimal::Decimal;
use std::collections::HashMap;

use crate::db::{
    Asset, AssetExchange, AssetExchangeType, CorporateAction, CorporateActionType, IncomeEvent,
    Transaction, TransactionType,
};
use crate::tax::cost_basis::{apply_exchange_source_effect, CostTracker};

/// All data needed to replay an asset's transaction history with interleaved
/// corporate events (amortizations, exchange effects, splits).
pub struct EnrichedTransactions {
    pub transactions: Vec<Transaction>,
    pub amortizations: Vec<IncomeEvent>,
    pub exchanges_as_source: Vec<AssetExchange>,
    pub corporate_actions: Vec<CorporateAction>,
}

/// Fetch transactions (including rename carryovers and exchange synthetic buys),
/// plus the side-channel events needed for cost basis replay.
pub fn build_enriched_transactions(
    conn: &Connection,
    asset_id: i64,
    as_of: NaiveDate,
    assets_by_id: &HashMap<i64, Asset>,
) -> Result<EnrichedTransactions> {
    let mut transactions = crate::db::get_asset_transactions_until(conn, asset_id, as_of)?;

    // Rename carryovers
    let renames = crate::db::get_asset_renames_as_target_up_to(conn, asset_id, as_of)?;
    for rename in renames {
        if let Some(source_asset) = assets_by_id.get(&rename.from_asset_id) {
            if let Some(carryover) = crate::reports::portfolio::build_rename_carryover_transaction(
                conn,
                source_asset,
                asset_id,
                rename.effective_date,
            )? {
                transactions.push(carryover);
            }
        }
    }

    // Exchange synthetic buys (spinoffs / mergers as target)
    let exchanges = crate::db::get_asset_exchanges_as_target_up_to(conn, asset_id, as_of)?;
    for exchange in exchanges {
        if exchange.to_quantity <= Decimal::ZERO {
            continue;
        }

        let source_ticker = assets_by_id
            .get(&exchange.from_asset_id)
            .map(|a| a.ticker.as_str())
            .unwrap_or("UNKNOWN");
        let notes = match exchange.event_type {
            AssetExchangeType::Spinoff => format!("Spin-off from {}", source_ticker),
            AssetExchangeType::Merger => format!("Merger from {}", source_ticker),
        };

        let price_per_unit = if exchange.to_quantity > Decimal::ZERO {
            exchange.allocated_cost / exchange.to_quantity
        } else {
            Decimal::ZERO
        };

        transactions.push(Transaction {
            id: None,
            asset_id,
            transaction_type: TransactionType::Buy,
            trade_date: exchange.effective_date,
            settlement_date: Some(exchange.effective_date),
            quantity: exchange.to_quantity,
            price_per_unit,
            total_cost: exchange.allocated_cost,
            fees: Decimal::ZERO,
            is_day_trade: false,
            quota_issuance_date: None,
            notes: Some(notes),
            source: "EXCHANGE".to_string(),
            created_at: chrono::Utc::now(),
        });
    }

    transactions.sort_by(|a, b| (a.trade_date, a.id).cmp(&(b.trade_date, b.id)));

    let amortizations = crate::db::get_amortizations_for_asset(conn, asset_id, None, Some(as_of))?;
    let exchanges_as_source =
        crate::db::get_asset_exchanges_as_source_up_to(conn, asset_id, as_of)?;
    let corporate_actions = crate::corporate_actions::get_actions_up_to(conn, asset_id, as_of)?;

    Ok(EnrichedTransactions {
        transactions,
        amortizations,
        exchanges_as_source,
        corporate_actions,
    })
}

impl EnrichedTransactions {
    /// Replay transactions with interleaved amortizations, exchange effects,
    /// and split/reverse-split adjustments applied to `tracker` before each
    /// transaction. After all transactions, remaining events up to `as_of`
    /// are applied (needed for portfolio position tracking).
    ///
    /// The `handler` closure receives each transaction and a mutable reference
    /// to the tracker so it can perform buy/sell logic.
    pub fn replay<T, F>(&self, tracker: &mut T, as_of: NaiveDate, mut handler: F) -> Result<()>
    where
        T: CostTracker,
        F: FnMut(&Transaction, &mut T) -> Result<()>,
    {
        let mut amort_idx = 0usize;
        let mut exchange_idx = 0usize;
        let mut action_idx = 0usize;

        for tx in &self.transactions {
            // Apply amortizations effective up to this transaction
            while amort_idx < self.amortizations.len()
                && self.amortizations[amort_idx].event_date <= tx.trade_date
            {
                tracker.apply_amortization(self.amortizations[amort_idx].total_amount);
                amort_idx += 1;
            }

            // Apply exchange-as-source effects (spinoff cost reduction, merger clear)
            while exchange_idx < self.exchanges_as_source.len()
                && self.exchanges_as_source[exchange_idx].effective_date <= tx.trade_date
            {
                apply_exchange_source_effect(tracker, &self.exchanges_as_source[exchange_idx]);
                exchange_idx += 1;
            }

            // Apply split / reverse-split quantity adjustments
            while action_idx < self.corporate_actions.len()
                && self.corporate_actions[action_idx].ex_date <= tx.trade_date
            {
                match self.corporate_actions[action_idx].action_type {
                    CorporateActionType::Split | CorporateActionType::ReverseSplit => {
                        tracker.apply_quantity_adjustment(
                            self.corporate_actions[action_idx].quantity_adjustment,
                        );
                    }
                    _ => {}
                }
                action_idx += 1;
            }

            handler(tx, tracker)?;
        }

        // Apply remaining events after the last transaction but before as_of
        while amort_idx < self.amortizations.len()
            && self.amortizations[amort_idx].event_date <= as_of
        {
            tracker.apply_amortization(self.amortizations[amort_idx].total_amount);
            amort_idx += 1;
        }

        while exchange_idx < self.exchanges_as_source.len()
            && self.exchanges_as_source[exchange_idx].effective_date <= as_of
        {
            apply_exchange_source_effect(tracker, &self.exchanges_as_source[exchange_idx]);
            exchange_idx += 1;
        }

        while action_idx < self.corporate_actions.len()
            && self.corporate_actions[action_idx].ex_date <= as_of
        {
            match self.corporate_actions[action_idx].action_type {
                CorporateActionType::Split | CorporateActionType::ReverseSplit => {
                    tracker.apply_quantity_adjustment(
                        self.corporate_actions[action_idx].quantity_adjustment,
                    );
                }
                _ => {}
            }
            action_idx += 1;
        }

        Ok(())
    }
}
