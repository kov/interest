use anyhow::Result;
use chrono::{Datelike, Local, NaiveDate};
use rusqlite::Connection;
use rust_decimal::Decimal;
use std::collections::HashMap;

use crate::db::{self, AssetType, Transaction, TransactionType};
use crate::reports::aggregation::{aggregate_positions_by_asset_type, AssetTypeTotals};
use crate::reports::portfolio::{
    calculate_portfolio_at_date, get_valid_snapshot, save_portfolio_snapshot,
};
use crate::tax::cost_basis::AverageCostMatcher;

#[derive(Debug, Clone)]
pub struct PerformanceReport {
    #[allow(dead_code)] // Kept for future dashboard/charting features
    pub period: Period,
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    pub start_value: Decimal,
    pub end_value: Decimal,
    pub total_return: Decimal,         // Absolute return (end - start)
    pub time_weighted_return: Decimal, // Percentage return
    pub realized_gains: Decimal,       // Placeholder (0 until realized_gains populated)
    pub unrealized_gains: Decimal,     // From snapshot end unrealized sum
    pub asset_breakdown: HashMap<AssetType, AssetPerformance>,
    pub cash_flows: Option<CashFlowSummary>, // Cash flow summary if available
}

#[derive(Debug, Clone)]
pub struct AssetPerformance {
    #[allow(dead_code)] // Kept for future detailed performance breakdown
    pub asset_type: AssetType,
    pub cost_basis: Decimal,
    pub market_value: Decimal,
    pub unrealized_pl: Decimal,
    pub return_pct: Decimal,
    #[allow(dead_code)] // Kept for future detailed performance breakdown
    pub contribution_to_total: Decimal, // Percentage points
    pub realized_gains: Decimal,
}

#[derive(Debug, Clone)]
pub enum Period {
    Mtd,     // Month-to-date
    Qtd,     // Quarter-to-date
    Ytd,     // Year-to-date
    OneYear, // Last 365 days
    AllTime, // Since first transaction
    Custom { from: NaiveDate, to: NaiveDate },
}

#[derive(Debug, Clone)]
pub struct CashFlow {
    pub date: NaiveDate,
    pub flow_type: FlowType,
    pub amount: Decimal,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FlowType {
    Contribution, // Money added to portfolio (buys)
    Withdrawal,   // Money removed from portfolio (sells)
}

#[derive(Debug, Clone)]
pub struct CashFlowSummary {
    pub total_contributions: Decimal,
    pub total_withdrawals: Decimal,
    pub net_flow: Decimal,
    pub flow_count: usize,
}

pub fn get_period_dates(
    period: Period,
    conn: Option<&Connection>,
) -> Result<(NaiveDate, NaiveDate)> {
    let today = Local::now().date_naive();
    let (start, end) = match period {
        Period::Mtd => {
            let start = NaiveDate::from_ymd_opt(today.year(), today.month(), 1)
                .ok_or_else(|| anyhow::anyhow!("Invalid current month"))?;
            (start, today)
        }
        Period::Qtd => {
            let quarter_start_month = ((today.month() - 1) / 3) * 3 + 1;
            let start = NaiveDate::from_ymd_opt(today.year(), quarter_start_month, 1)
                .ok_or_else(|| anyhow::anyhow!("Invalid quarter start"))?;
            (start, today)
        }
        Period::Ytd => {
            let start = NaiveDate::from_ymd_opt(today.year(), 1, 1)
                .ok_or_else(|| anyhow::anyhow!("Invalid year start"))?;
            (start, today)
        }
        Period::OneYear => {
            let start = today
                .checked_sub_days(chrono::Days::new(365))
                .ok_or_else(|| anyhow::anyhow!("Failed to compute one-year start"))?;
            (start, today)
        }
        Period::AllTime => {
            // If a connection is provided, try to fetch earliest transaction date, else default to 20 years ago
            if let Some(c) = conn {
                let mut stmt = c.prepare("SELECT MIN(trade_date) FROM transactions")?;
                let min_date: Option<NaiveDate> = stmt.query_row([], |row| row.get(0)).ok();
                let start = min_date.unwrap_or_else(|| {
                    today
                        .checked_sub_days(chrono::Days::new(365 * 20))
                        .unwrap_or(today)
                });
                (start, today)
            } else {
                let start = today
                    .checked_sub_days(chrono::Days::new(365 * 20))
                    .unwrap_or(today);
                (start, today)
            }
        }
        Period::Custom { from, to } => {
            if from > to {
                anyhow::bail!("Custom period 'from' must be <= 'to'");
            }
            (from, to)
        }
    };

    Ok((start, end))
}

/// Ensure a valid snapshot exists for the given date; create it if missing/stale.
fn ensure_snapshot(conn: &mut Connection, date: NaiveDate) -> Result<()> {
    if get_valid_snapshot(conn, date)?.is_none() {
        // Compute snapshot using as-of portfolio and persist
        save_portfolio_snapshot(conn, date, None)?;
    }
    Ok(())
}

pub fn calculate_performance(conn: &mut Connection, period: Period) -> Result<PerformanceReport> {
    let (start_date, end_date) = get_period_dates(period.clone(), Some(conn))?;

    // Ensure snapshots exist
    ensure_snapshot(conn, start_date)?;
    ensure_snapshot(conn, end_date)?;

    // Load snapshots, with graceful fallback to on-the-fly portfolio calculation
    let start_snapshot = match get_valid_snapshot(conn, start_date)? {
        Some(s) => s,
        None => calculate_portfolio_at_date(conn, start_date, None)?,
    };
    let end_snapshot = match get_valid_snapshot(conn, end_date)? {
        Some(s) => s,
        None => calculate_portfolio_at_date(conn, end_date, None)?,
    };

    // Aggregate values
    let start_value = start_snapshot.total_value;
    let end_value = end_snapshot.total_value;
    let total_return = end_value - start_value; // Absolute return in currency

    // Extract cash flows in period
    let cash_flows = extract_cash_flows(conn, start_date, end_date)?;
    let cash_flow_summary = if !cash_flows.is_empty() {
        Some(summarize_cash_flows(&cash_flows))
    } else {
        None
    };

    // Calculate time-weighted return (TWR)
    // If we have cash flows, use proper TWR calculation
    // Otherwise fall back to simple percentage return
    let twr = if !cash_flows.is_empty() {
        // Load daily snapshots for sub-period calculations
        let snapshots = load_daily_snapshots(conn, start_date, end_date)?;
        calculate_time_weighted_return(start_value, end_value, &cash_flows, &snapshots)?
    } else {
        // Simple percentage return when no cash flows
        if start_value > Decimal::ZERO {
            (total_return / start_value) * Decimal::from(100)
        } else {
            Decimal::ZERO
        }
    };

    // Unrealized gains: calculate from positions (market_value - cost_basis)
    let unrealized_sum = end_snapshot
        .positions
        .iter()
        .map(|p| {
            p.unrealized_pl.unwrap_or_else(|| {
                // Fallback calculation if unrealized_pl not set
                let market_val = p.quantity * p.current_price.unwrap_or(p.average_cost);
                let cost_val = p.quantity * p.average_cost;
                market_val - cost_val
            })
        })
        .fold(Decimal::ZERO, |acc, x| acc + x);

    let realized_by_type = calculate_realized_gains_by_asset_type(conn, start_date, end_date)?;
    let realized_gains = realized_by_type
        .values()
        .fold(Decimal::ZERO, |acc, x| acc + *x);

    let totals_by_type = aggregate_positions_by_asset_type(&end_snapshot.positions);
    let breakdown = build_asset_breakdown(&totals_by_type, &realized_by_type)?;

    Ok(PerformanceReport {
        period,
        start_date,
        end_date,
        start_value,
        end_value,
        total_return,
        time_weighted_return: twr,
        realized_gains,
        unrealized_gains: unrealized_sum,
        asset_breakdown: breakdown,
        cash_flows: cash_flow_summary,
    })
}

fn build_asset_breakdown(
    totals_by_type: &HashMap<AssetType, AssetTypeTotals>,
    realized_by_type: &HashMap<AssetType, Decimal>,
) -> Result<HashMap<AssetType, AssetPerformance>> {
    let total_cost_basis = totals_by_type
        .values()
        .map(|totals| totals.cost_basis)
        .fold(Decimal::ZERO, |acc, x| acc + x);

    let mut asset_types: Vec<AssetType> = totals_by_type
        .keys()
        .chain(realized_by_type.keys())
        .copied()
        .collect();
    asset_types.sort();
    asset_types.dedup();

    let mut breakdown = HashMap::new();
    for asset_type in asset_types {
        let totals = totals_by_type.get(&asset_type).cloned().unwrap_or_default();
        let realized_gains = realized_by_type
            .get(&asset_type)
            .cloned()
            .unwrap_or(Decimal::ZERO);

        if totals.cost_basis <= Decimal::ZERO
            && totals.market_value <= Decimal::ZERO
            && realized_gains == Decimal::ZERO
        {
            continue;
        }

        let contribution = if total_cost_basis > Decimal::ZERO {
            (totals.cost_basis / total_cost_basis) * totals.return_pct
        } else {
            Decimal::ZERO
        };

        breakdown.insert(
            asset_type,
            AssetPerformance {
                asset_type,
                cost_basis: totals.cost_basis,
                market_value: totals.market_value,
                unrealized_pl: totals.unrealized_pl,
                return_pct: totals.return_pct,
                contribution_to_total: contribution,
                realized_gains,
            },
        );
    }
    Ok(breakdown)
}

fn calculate_realized_gains_by_asset_type(
    conn: &Connection,
    from_date: NaiveDate,
    to_date: NaiveDate,
) -> Result<HashMap<AssetType, Decimal>> {
    let assets = crate::db::get_all_assets(conn)?;
    let mut assets_by_id = HashMap::new();
    for asset in &assets {
        if let Some(id) = asset.id {
            assets_by_id.insert(id, asset.clone());
        }
    }

    let mut realized_by_type: HashMap<AssetType, Decimal> = HashMap::new();

    for asset in assets {
        if !crate::db::is_supported_portfolio_ticker(&asset.ticker)
            && !matches!(asset.asset_type, AssetType::Bond | AssetType::GovBond)
        {
            continue;
        }

        let asset_id = match asset.id {
            Some(id) => id,
            None => continue,
        };

        if crate::db::is_rename_source_asset(conn, asset_id, to_date)? {
            continue;
        }

        let mut transactions =
            crate::reports::portfolio::get_asset_transactions_until(conn, asset_id, to_date)?;

        let renames = crate::db::get_asset_renames_as_target_up_to(conn, asset_id, to_date)?;
        for rename in renames {
            if let Some(source_asset) = assets_by_id.get(&rename.from_asset_id) {
                if let Some(carryover) =
                    crate::reports::portfolio::build_rename_carryover_transaction(
                        conn,
                        source_asset,
                        asset_id,
                        rename.effective_date,
                    )?
                {
                    transactions.push(carryover);
                }
            }
        }

        let exchanges = crate::db::get_asset_exchanges_as_target_up_to(conn, asset_id, to_date)?;
        for exchange in exchanges {
            if exchange.to_quantity <= Decimal::ZERO {
                continue;
            }

            let source_ticker = assets_by_id
                .get(&exchange.from_asset_id)
                .map(|a| a.ticker.as_str())
                .unwrap_or("UNKNOWN");
            let notes = match exchange.event_type {
                crate::db::AssetExchangeType::Spinoff => {
                    format!("Spin-off from {}", source_ticker)
                }
                crate::db::AssetExchangeType::Merger => {
                    format!("Merger from {}", source_ticker)
                }
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

        let mut swing_matcher = AverageCostMatcher::new();
        let mut day_trade_matcher = AverageCostMatcher::new();

        let amortizations =
            crate::db::get_amortizations_for_asset(conn, asset_id, None, Some(to_date))?;
        let mut amort_idx: usize = 0;
        let exchanges_as_source =
            crate::db::get_asset_exchanges_as_source_up_to(conn, asset_id, to_date)?;
        let mut exchange_idx: usize = 0;
        let actions_up_to = crate::corporate_actions::get_actions_up_to(conn, asset_id, to_date)?;
        let mut action_idx: usize = 0;

        for tx in transactions {
            while amort_idx < amortizations.len()
                && amortizations[amort_idx].event_date <= tx.trade_date
            {
                swing_matcher.apply_amortization(amortizations[amort_idx].total_amount);
                amort_idx += 1;
            }

            while exchange_idx < exchanges_as_source.len()
                && exchanges_as_source[exchange_idx].effective_date <= tx.trade_date
            {
                apply_exchange_source_effect(
                    &mut swing_matcher,
                    &exchanges_as_source[exchange_idx],
                );
                exchange_idx += 1;
            }

            while action_idx < actions_up_to.len()
                && actions_up_to[action_idx].ex_date <= tx.trade_date
            {
                match actions_up_to[action_idx].action_type {
                    crate::db::CorporateActionType::Split
                    | crate::db::CorporateActionType::ReverseSplit => {
                        swing_matcher.apply_quantity_adjustment(
                            actions_up_to[action_idx].quantity_adjustment,
                        );
                    }
                    _ => {}
                }
                action_idx += 1;
            }

            match tx.transaction_type {
                TransactionType::Buy => {
                    if tx.is_day_trade {
                        day_trade_matcher.add_purchase(&tx, None, None);
                    } else {
                        swing_matcher.add_purchase(&tx, None, None);
                    }
                }
                TransactionType::Sell => {
                    let matcher = if tx.is_day_trade {
                        &mut day_trade_matcher
                    } else {
                        &mut swing_matcher
                    };

                    let sale = matcher.match_sale(&tx, None)?;
                    if tx.trade_date >= from_date && tx.trade_date <= to_date {
                        *realized_by_type
                            .entry(asset.asset_type)
                            .or_insert(Decimal::ZERO) += sale.profit_loss;
                    }
                }
            }
        }
    }

    Ok(realized_by_type)
}

fn apply_exchange_source_effect(
    matcher: &mut AverageCostMatcher,
    exchange: &crate::db::AssetExchange,
) {
    match exchange.event_type {
        crate::db::AssetExchangeType::Spinoff => {
            let reduction = exchange.allocated_cost + exchange.cash_amount;
            matcher.apply_amortization(reduction);
        }
        crate::db::AssetExchangeType::Merger => {
            matcher.clear_position();
        }
    }
}

/// Extract cash flows from transaction history within a date range
/// BUY transactions = CONTRIBUTION (money flowing into portfolio)
/// SELL transactions = WITHDRAWAL (money flowing out of portfolio)
pub fn extract_cash_flows(
    conn: &Connection,
    from_date: NaiveDate,
    to_date: NaiveDate,
) -> Result<Vec<CashFlow>> {
    let mut stmt = conn.prepare(
        "SELECT COALESCE(settlement_date, trade_date) as flow_date,
                transaction_type,
                quantity,
                price_per_unit,
                fees
         FROM transactions
         WHERE COALESCE(settlement_date, trade_date) >= ?1
           AND COALESCE(settlement_date, trade_date) <= ?2
           AND transaction_type IN ('BUY', 'SELL')
           AND (notes IS NULL OR notes NOT LIKE '%Term contract liquidation%')
         ORDER BY flow_date",
    )?;

    let flows = stmt
        .query_map([from_date, to_date], |row| {
            let date: NaiveDate = row.get(0)?;
            let tx_type: String = row.get(1)?;
            let quantity = db::get_decimal_value(row, 2)?;
            let price = db::get_decimal_value(row, 3)?;
            let fees = db::get_optional_decimal_value(row, 4)?.unwrap_or(Decimal::ZERO);

            let gross = quantity * price;
            let amount = match tx_type.as_str() {
                "BUY" => gross + fees,
                "SELL" => gross - fees,
                _ => gross,
            };

            let flow_type = match tx_type.as_str() {
                "BUY" => FlowType::Contribution,
                "SELL" => FlowType::Withdrawal,
                _ => return Ok(None),
            };

            Ok(Some(CashFlow {
                date,
                flow_type,
                amount,
            }))
        })?
        .filter_map(|r| r.ok().and_then(|opt| opt))
        .collect::<Vec<_>>();

    let mut flows = flows;

    let income_events =
        db::get_income_events_with_assets(conn, Some(from_date), Some(to_date), None)?;
    for (event, _asset) in income_events {
        let net_income = event.total_amount - event.withholding_tax;
        flows.push(CashFlow {
            date: event.event_date,
            flow_type: FlowType::Withdrawal,
            amount: net_income,
        });
    }

    Ok(flows)
}

/// Summarize cash flows for reporting
pub fn summarize_cash_flows(flows: &[CashFlow]) -> CashFlowSummary {
    let mut total_contributions = Decimal::ZERO;
    let mut total_withdrawals = Decimal::ZERO;

    for flow in flows {
        match flow.flow_type {
            FlowType::Contribution => total_contributions += flow.amount,
            FlowType::Withdrawal => total_withdrawals += flow.amount,
        }
    }

    CashFlowSummary {
        total_contributions,
        total_withdrawals,
        net_flow: total_contributions - total_withdrawals,
        flow_count: flows.len(),
    }
}

/// Load daily portfolio values from snapshots table
pub fn load_daily_snapshots(
    conn: &Connection,
    from_date: NaiveDate,
    to_date: NaiveDate,
) -> Result<HashMap<NaiveDate, Decimal>> {
    let mut stmt = conn.prepare(
        "SELECT snapshot_date, SUM(market_value) as total_value
         FROM position_snapshots
         WHERE snapshot_date >= ?1 AND snapshot_date <= ?2
         GROUP BY snapshot_date
         ORDER BY snapshot_date",
    )?;

    let mut snapshots = HashMap::new();
    let rows = stmt.query_map([from_date, to_date], |row| {
        let date: NaiveDate = row.get(0)?;
        let value_val: rusqlite::types::Value = row.get(1)?;
        Ok((date, value_val))
    })?;

    for row in rows {
        let (date, value_val) = row?;
        let value = match value_val {
            rusqlite::types::Value::Text(s) => s.parse::<Decimal>().unwrap_or(Decimal::ZERO),
            rusqlite::types::Value::Real(f) => Decimal::try_from(f).unwrap_or(Decimal::ZERO),
            rusqlite::types::Value::Integer(i) => Decimal::from(i),
            _ => Decimal::ZERO,
        };
        snapshots.insert(date, value);
    }

    Ok(snapshots)
}

/// Calculate time-weighted return (TWR) accounting for cash flows
///
/// TWR breaks the period into sub-periods at each cash flow date and chains
/// the returns together. This isolates investment performance from the effect
/// of contributions and withdrawals.
///
/// Algorithm:
/// 1. Sort cash flows by date
/// 2. For each sub-period between flows:
///    - Get portfolio value at start of sub-period
///    - Get portfolio value at end of sub-period
///    - Adjust end value by subtracting any flows on that day
///    - Calculate sub-period return: (adjusted_end / start) - 1
/// 3. Chain sub-period returns: TWR = (1 + r1) * (1 + r2) * ... - 1
///
/// Example from plan:
/// - Start: R$ 100k, Contribution day 10: R$ 50k, End: R$ 165k
/// - Period 1: (110k / 100k) - 1 = 10%
/// - Period 2: ((165k - 50k) / 110k) - 1 = 4.54%
/// - TWR: (1.10 * 1.0454) - 1 = 15.00%
pub fn calculate_time_weighted_return(
    start_value: Decimal,
    end_value: Decimal,
    cash_flows: &[CashFlow],
    snapshots: &HashMap<NaiveDate, Decimal>,
) -> Result<Decimal> {
    if start_value <= Decimal::ZERO {
        return Ok(Decimal::ZERO);
    }

    // If no cash flows, simple return
    if cash_flows.is_empty() {
        return Ok(((end_value - start_value) / start_value) * Decimal::from(100));
    }

    // Group flows by date and net them (contributions - withdrawals)
    let mut daily_flows: HashMap<NaiveDate, Decimal> = HashMap::new();
    for flow in cash_flows {
        let amount = match flow.flow_type {
            FlowType::Contribution => flow.amount,
            FlowType::Withdrawal => -flow.amount,
        };
        *daily_flows.entry(flow.date).or_insert(Decimal::ZERO) += amount;
    }

    // Get sorted flow dates
    let mut flow_dates: Vec<NaiveDate> = daily_flows.keys().copied().collect();
    flow_dates.sort();

    // Chain sub-period returns
    let mut cumulative_factor = Decimal::ONE;
    let mut prev_value = start_value;

    for date in flow_dates {
        // Get portfolio value at this date (before the flow)
        let value_before_flow = snapshots.get(&date).copied().unwrap_or(prev_value);

        // Calculate return for this sub-period
        if prev_value > Decimal::ZERO {
            let sub_return = (value_before_flow / prev_value) - Decimal::ONE;
            cumulative_factor *= Decimal::ONE + sub_return;
        }

        // Adjust for the flow to get starting value for next period
        let net_flow = daily_flows.get(&date).copied().unwrap_or(Decimal::ZERO);
        prev_value = value_before_flow + net_flow;
    }

    // Final sub-period: last flow date to end date
    if prev_value > Decimal::ZERO {
        let final_return = (end_value / prev_value) - Decimal::ONE;
        cumulative_factor *= Decimal::ONE + final_return;
    }

    // Convert to percentage
    let twr_pct = (cumulative_factor - Decimal::ONE) * Decimal::from(100);
    Ok(twr_pct)
}

#[allow(dead_code)] // Kept for Phase 6: Performance Tracking (see PERFORMANCE_TRACKING_PLAN.md)
pub fn backfill_daily_snapshots(
    conn: &mut Connection,
    from_date: NaiveDate,
    to_date: NaiveDate,
    progress_callback: impl Fn(usize, usize),
) -> Result<()> {
    if from_date > to_date {
        anyhow::bail!("from_date must be <= to_date");
    }

    let mut dates = Vec::new();
    let mut d = from_date;
    while d <= to_date {
        dates.push(d);
        d = d
            .succ_opt()
            .ok_or_else(|| anyhow::anyhow!("Failed to increment date"))?;
    }

    let total = dates.len();
    for (idx, date) in dates.into_iter().enumerate() {
        if get_valid_snapshot(conn, date)?.is_none() {
            // Compute using historical portfolio as-of date and persist
            let _ = calculate_portfolio_at_date(conn, date, None)?;
            save_portfolio_snapshot(conn, date, None)?;
        }
        progress_callback(idx + 1, total);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{self, AssetType};

    use rust_decimal::Decimal;

    #[test]
    fn test_get_period_dates_mtd() {
        let (start, end) = get_period_dates(Period::Mtd, None).unwrap();
        assert!(start <= end);
    }

    #[test]
    fn test_calculate_performance_basic() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!("../db/schema.sql"))
            .unwrap();

        let asset_id = db::upsert_asset(&conn, "TEST5", &AssetType::Stock, None).unwrap();

        // Insert a buy and prices around start/end
        let buy_tx = crate::db::Transaction {
            id: None,
            asset_id,
            transaction_type: crate::db::TransactionType::Buy,
            trade_date: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            settlement_date: None,
            quantity: Decimal::from(10),
            price_per_unit: Decimal::from(10),
            total_cost: Decimal::from(100),
            fees: Decimal::ZERO,
            is_day_trade: false,
            quota_issuance_date: None,
            notes: None,
            source: "TEST".to_string(),
            created_at: chrono::Utc::now(),
        };
        db::insert_transaction(&conn, &buy_tx).unwrap();

        let start_price = crate::db::PriceHistory {
            id: None,
            asset_id,
            price_date: NaiveDate::from_ymd_opt(2024, 2, 1).unwrap(),
            close_price: Decimal::from(12),
            open_price: None,
            high_price: None,
            low_price: None,
            volume: Some(1_000),
            source: "TEST".to_string(),
            created_at: chrono::Utc::now(),
        };
        let end_price = crate::db::PriceHistory {
            id: None,
            asset_id,
            price_date: NaiveDate::from_ymd_opt(2024, 3, 1).unwrap(),
            close_price: Decimal::from(15),
            open_price: None,
            high_price: None,
            low_price: None,
            volume: Some(1_000),
            source: "TEST".to_string(),
            created_at: chrono::Utc::now(),
        };
        db::insert_price_history(&conn, &start_price).unwrap();
        db::insert_price_history(&conn, &end_price).unwrap();

        let period = Period::Custom {
            from: NaiveDate::from_ymd_opt(2024, 2, 1).unwrap(),
            to: NaiveDate::from_ymd_opt(2024, 3, 1).unwrap(),
        };
        let report = calculate_performance(&mut conn, period).unwrap();
        assert_eq!(report.start_value, Decimal::from(120));
        assert_eq!(report.end_value, Decimal::from(150));
        assert!(report.total_return > Decimal::ZERO);
        assert!(report.unrealized_gains >= Decimal::ZERO);
    }

    #[test]
    fn test_asset_breakdown_includes_realized_only() {
        let totals_by_type = HashMap::new();
        let mut realized_by_type = HashMap::new();
        realized_by_type.insert(AssetType::Bdr, Decimal::from(250));

        let breakdown = build_asset_breakdown(&totals_by_type, &realized_by_type).unwrap();
        let perf = breakdown.get(&AssetType::Bdr).unwrap();
        assert_eq!(perf.realized_gains, Decimal::from(250));
        assert_eq!(perf.cost_basis, Decimal::ZERO);
        assert_eq!(perf.market_value, Decimal::ZERO);
        assert_eq!(perf.return_pct, Decimal::ZERO);
    }

    #[test]
    fn test_extract_cash_flows_ignores_term_liquidation() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!("../db/schema.sql"))
            .unwrap();

        let asset_id = crate::db::upsert_asset(&conn, "TESTX", &AssetType::Stock, None).unwrap();

        // Regular buy
        let buy_tx = crate::db::Transaction {
            id: None,
            asset_id,
            transaction_type: crate::db::TransactionType::Buy,
            trade_date: NaiveDate::from_ymd_opt(2023, 5, 2).unwrap(),
            settlement_date: None,
            quantity: Decimal::from(100),
            price_per_unit: Decimal::from(10),
            total_cost: Decimal::from(1000),
            fees: Decimal::ZERO,
            is_day_trade: false,
            quota_issuance_date: None,
            notes: Some("Imported trade".to_string()),
            source: "TEST".to_string(),
            created_at: chrono::Utc::now(),
        };
        crate::db::insert_transaction(&conn, &buy_tx).unwrap();

        // Term contract liquidation - should be ignored for cash flow aggregation
        let term_liq = crate::db::Transaction {
            id: None,
            asset_id,
            transaction_type: crate::db::TransactionType::Buy,
            trade_date: NaiveDate::from_ymd_opt(2023, 5, 2).unwrap(),
            settlement_date: None,
            quantity: Decimal::from(50),
            price_per_unit: Decimal::from(10),
            total_cost: Decimal::from(500),
            fees: Decimal::ZERO,
            is_day_trade: false,
            quota_issuance_date: None,
            notes: Some("Term contract liquidation (original ticker: TESTXT → TESTX)".to_string()),
            source: "TEST".to_string(),
            created_at: chrono::Utc::now(),
        };
        crate::db::insert_transaction(&conn, &term_liq).unwrap();

        let flows = extract_cash_flows(
            &conn,
            NaiveDate::from_ymd_opt(2023, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2023, 12, 31).unwrap(),
        )
        .unwrap();
        assert_eq!(flows.len(), 1); // only the regular buy counted

        let summary = summarize_cash_flows(&flows);
        assert_eq!(summary.total_contributions, Decimal::from(1000));
        assert_eq!(summary.total_withdrawals, Decimal::ZERO);
    }
}
