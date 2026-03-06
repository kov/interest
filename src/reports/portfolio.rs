use anyhow::Result;
use blake3::Hasher;
use chrono::NaiveDate;
use rusqlite::Connection;
use rust_decimal::Decimal;
use std::collections::HashMap;
use crate::db::{Asset, AssetType, Transaction, TransactionType};
use crate::reports::aggregation::normalize_positions_with_prices;

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
    pub ltm_income: Decimal,
    pub ltm_yield_pct: Option<Decimal>,
    pub ltm_yield_on_cost_pct: Option<Decimal>,
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

    fn clear(&mut self) {
        self.quantity = Decimal::ZERO;
        self.total_cost = Decimal::ZERO;
    }

    fn average_cost(&self) -> Decimal {
        if self.quantity > Decimal::ZERO {
            self.total_cost / self.quantity
        } else {
            Decimal::ZERO
        }
    }
}

impl crate::tax::cost_basis::CostTracker for AvgCostPosition {
    fn apply_amortization(&mut self, amount: Decimal) {
        self.apply_amortization(amount);
    }

    fn clear_position(&mut self) {
        self.clear();
    }

    fn apply_quantity_adjustment(&mut self, adjustment: Decimal) {
        self.quantity += adjustment;
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

    let mut assets_by_id = HashMap::new();
    for asset in &assets {
        if let Some(id) = asset.id {
            assets_by_id.insert(id, asset.clone());
        }
    }

    let as_of = as_of_date.unwrap_or_else(|| chrono::Local::now().date_naive());

    // Filter by asset type if requested
    let mut filtered_assets = Vec::new();
    for asset in assets {
        if let Some(filter) = asset_type_filter {
            if &asset.asset_type != filter {
                continue;
            }
        }

        if !crate::db::is_supported_portfolio_ticker(&asset.ticker)
            && !matches!(asset.asset_type, AssetType::Bond | AssetType::GovBond)
        {
            continue;
        }

        let asset_id = match asset.id {
            Some(id) => id,
            None => continue,
        };

        if crate::db::is_rename_source_asset(conn, asset_id, as_of)? {
            continue;
        }

        filtered_assets.push(asset);
    }

    // Calculate positions for each asset
    let mut positions = Vec::new();
    for asset in filtered_assets {
        let asset_id = asset.id.unwrap();

        // Build enriched transaction list and replay with interleaved events
        let enriched =
            crate::reports::enrichment::build_enriched_transactions(
                conn, asset_id, as_of, &assets_by_id,
            )?;
        let mut position = AvgCostPosition::new(asset_id);
        let ticker = asset.ticker.clone();
        enriched.replay(&mut position, as_of, |tx, pos| {
            match tx.transaction_type {
                TransactionType::Buy => {
                    pos.add_buy(tx.quantity, tx.total_cost);
                }
                TransactionType::Sell => {
                    pos.remove_sell(tx.quantity, &ticker)?;
                }
            }
            Ok(())
        })?;

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

        positions.push(PositionSummary {
            asset,
            quantity: position.quantity,
            average_cost: position.average_cost(),
            total_cost: position.total_cost,
            current_price,
            current_value,
            unrealized_pl,
            unrealized_pl_pct,
            ltm_income: Decimal::ZERO,
            ltm_yield_pct: None,
            ltm_yield_on_cost_pct: None,
        });
    }

    // Sort positions by total value (descending)
    positions.sort_by(|a, b| {
        let a_val = a.current_value.unwrap_or(a.total_cost);
        let b_val = b.current_value.unwrap_or(b.total_cost);
        b_val.cmp(&a_val)
    });

    let (mut positions, total_cost, total_value) =
        normalize_positions_with_prices(conn, as_of, positions)?;
    apply_ltm_income(conn, as_of, &mut positions);
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

/// Enrich positions with LTM income data
fn apply_ltm_income(conn: &Connection, as_of: NaiveDate, positions: &mut [PositionSummary]) {
    let one_year_ago = as_of - chrono::Duration::days(365);
    let totals = match crate::db::get_income_totals_by_asset(conn, one_year_ago, as_of) {
        Ok(t) => t,
        Err(_) => return,
    };

    for position in positions.iter_mut() {
        if let Some(asset_id) = position.asset.id {
            if let Some(&income) = totals.get(&asset_id) {
                position.ltm_income = income;
                if let Some(value) = position.current_value {
                    if value > Decimal::ZERO {
                        position.ltm_yield_pct = Some((income / value) * Decimal::from(100));
                    }
                }
                if position.total_cost > Decimal::ZERO {
                    position.ltm_yield_on_cost_pct =
                        Some((income / position.total_cost) * Decimal::from(100));
                }
            }
        }
    }
}

fn get_asset_transactions_before(
    conn: &Connection,
    asset_id: i64,
    before_date: NaiveDate,
) -> Result<Vec<Transaction>> {
    crate::db::get_asset_transactions_before(conn, asset_id, before_date)
}

/// Build a synthetic buy transaction representing the carryover position
/// from a renamed source asset. Replays source-asset transactions with
/// interleaved amortizations and per-transaction corporate action
/// adjustments (source-asset splits between trade date and rename date).
///
/// Target-asset corporate actions are NOT applied here — they are handled
/// by the caller's main transaction loop at the correct chronological point.
pub(crate) fn build_rename_carryover_transaction(
    conn: &Connection,
    source_asset: &Asset,
    target_asset_id: i64,
    effective_date: NaiveDate,
) -> Result<Option<Transaction>> {
    use crate::tax::cost_basis::AverageCostMatcher;

    let source_id = match source_asset.id {
        Some(id) => id,
        None => return Ok(None),
    };

    let transactions = get_asset_transactions_before(conn, source_id, effective_date)?;
    let mut matcher = AverageCostMatcher::new();

    // Interleave amortizations with transactions
    let amortizations =
        crate::db::get_amortizations_for_asset(conn, source_id, None, Some(effective_date))?;
    let mut amort_idx: usize = 0;

    for tx in transactions {
        if tx.is_day_trade {
            continue;
        }

        // Apply amortizations that occurred on or before this transaction date
        while amort_idx < amortizations.len()
            && amortizations[amort_idx].event_date <= tx.trade_date
        {
            matcher.apply_amortization(amortizations[amort_idx].total_amount);
            amort_idx += 1;
        }

        // Adjust this transaction for source-asset corporate actions that
        // occurred between its trade date and the rename effective date
        let actions = crate::corporate_actions::get_applicable_actions(
            conn,
            source_id,
            tx.trade_date,
            effective_date,
        )?;

        let mut adjusted_quantity = tx.quantity;
        let adjusted_cost = tx.total_cost;
        for action in &actions {
            match action.action_type {
                crate::db::CorporateActionType::Split
                | crate::db::CorporateActionType::ReverseSplit => {
                    adjusted_quantity += action.quantity_adjustment;
                }
                _ => {}
            }
        }

        match tx.transaction_type {
            TransactionType::Buy => {
                matcher.add_purchase(&tx, Some(adjusted_quantity), Some(adjusted_cost));
            }
            TransactionType::Sell => {
                let _ = matcher.match_sale(&tx, Some(adjusted_quantity))?;
            }
        }
    }

    // Apply any remaining amortizations up to the effective date
    while amort_idx < amortizations.len()
        && amortizations[amort_idx].event_date <= effective_date
    {
        matcher.apply_amortization(amortizations[amort_idx].total_amount);
        amort_idx += 1;
    }

    let quantity = matcher.remaining_quantity();
    if quantity <= Decimal::ZERO {
        return Ok(None);
    }

    let total_cost = matcher.average_cost() * quantity;
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
        let quantity = crate::db::get_decimal_value(row, 4)?;
        let price_per_unit = crate::db::get_decimal_value(row, 5)?;
        let total_cost = crate::db::get_decimal_value(row, 6)?;

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
                ps.unrealized_pl, ps.tx_fingerprint, a.ticker, a.asset_type, a.name, a.cnpj,
                a.created_at, a.updated_at
         FROM position_snapshots ps
         JOIN assets a ON ps.asset_id = a.id
         WHERE ps.snapshot_date = ?1
         ORDER BY ps.market_value DESC",
    )?;

    let rows = stmt
        .query_map([date], |row| {
            let asset_type: AssetType = row
                .get::<_, String>(8)?
                .parse()
                .unwrap_or(AssetType::Unknown);

            Ok((
                Asset {
                    id: Some(row.get(0)?),
                    ticker: row.get(7)?,
                    asset_type,
                    name: row.get(9)?,
                    cnpj: row.get(10)?,
                    created_at: row.get(11)?,
                    updated_at: row.get(12)?,
                },
                crate::db::get_decimal_value(row, 1)?,
                crate::db::get_decimal_value(row, 2)?,
                crate::db::get_decimal_value(row, 3)?,
                crate::db::get_decimal_value(row, 4)?,
                crate::db::get_decimal_value(row, 5)?,
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
            ltm_income: Decimal::ZERO,
            ltm_yield_pct: None,
            ltm_yield_on_cost_pct: None,
        });
    }

    apply_ltm_income(conn, date, &mut positions);

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
