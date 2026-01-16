use anyhow::Result;
use chrono::{Datelike, NaiveDate};
use rust_decimal::Decimal;
use std::collections::HashMap;

use crate::db::{self, AssetType, TransactionType};

#[derive(Debug, Clone)]
pub struct CashFlowReport {
    pub from_date: NaiveDate,
    pub to_date: NaiveDate,
    pub years: Vec<YearlyCashFlow>,
    pub total_in: Decimal,
    pub total_out: Decimal,
    pub net_flow: Decimal,
}

#[derive(Debug, Clone)]
pub struct YearlyCashFlow {
    pub year: i32,
    pub money_in: Decimal,
    pub money_out: Decimal,
    pub net_flow: Decimal,
    pub by_asset_type: HashMap<AssetType, AssetTypeCashFlow>,
}

#[derive(Debug, Clone)]
pub struct AssetTypeCashFlow {
    #[allow(dead_code)] // Kept for future detailed breakdown reporting
    pub asset_type: AssetType,
    pub money_in: Decimal,
    pub money_out_sells: Decimal,
    pub money_out_income: Decimal,
    pub net_flow: Decimal,
}

#[derive(Debug, Clone)]
pub struct CashFlowStats {
    pub avg_monthly_in: Decimal,
    pub avg_monthly_out: Decimal,
    pub avg_monthly_net: Decimal,
    pub avg_yearly_in: Decimal,
    pub avg_yearly_out: Decimal,
    pub avg_yearly_net: Decimal,
    pub months_with_data: usize,
    pub years_with_data: usize,
    pub trend: CashFlowTrend,
}

#[derive(Debug, Clone)]
pub struct CashFlowTrend {
    pub direction: TrendDirection,
    pub avg_yearly_growth_rate: Option<Decimal>,
    pub yearly_changes: Vec<YearlyChange>,
}

#[derive(Debug, Clone)]
pub enum TrendDirection {
    Increasing,
    Decreasing,
    Stable,
}

#[derive(Debug, Clone)]
pub struct YearlyChange {
    pub year: i32,
    #[allow(dead_code)] // Kept for future detailed reporting
    pub prev_year_net: Decimal,
    pub curr_year_net: Decimal,
    pub growth_rate_pct: Option<Decimal>,
}

#[derive(Debug, Clone)]
pub struct CashFlowEntry {
    pub date: NaiveDate,
    pub asset_type: AssetType,
    pub money_in: Decimal,
    pub money_out_sells: Decimal,
    pub money_out_income: Decimal,
}

pub fn calculate_cash_flow_report(
    conn: &rusqlite::Connection,
    from_date: NaiveDate,
    to_date: NaiveDate,
) -> Result<CashFlowReport> {
    let entries = cash_flow_entries(conn, from_date, to_date)?;

    let mut years_map: HashMap<i32, HashMap<AssetType, AssetTypeCashFlow>> = HashMap::new();
    let mut total_in = Decimal::ZERO;
    let mut total_out = Decimal::ZERO;

    for entry in &entries {
        let year = entry.date.year();
        let asset_map = years_map.entry(year).or_default();
        let bucket = asset_map
            .entry(entry.asset_type)
            .or_insert(AssetTypeCashFlow {
                asset_type: entry.asset_type,
                money_in: Decimal::ZERO,
                money_out_sells: Decimal::ZERO,
                money_out_income: Decimal::ZERO,
                net_flow: Decimal::ZERO,
            });

        bucket.money_in += entry.money_in;
        bucket.money_out_sells += entry.money_out_sells;
        bucket.money_out_income += entry.money_out_income;
        bucket.net_flow = bucket.money_in - bucket.money_out_sells - bucket.money_out_income;

        total_in += entry.money_in;
        total_out += entry.money_out_sells + entry.money_out_income;
    }

    let mut years: Vec<YearlyCashFlow> = years_map
        .into_iter()
        .map(|(year, by_asset_type)| {
            let money_in = by_asset_type
                .values()
                .fold(Decimal::ZERO, |acc, a| acc + a.money_in);
            let money_out = by_asset_type.values().fold(Decimal::ZERO, |acc, a| {
                acc + a.money_out_sells + a.money_out_income
            });
            let net_flow = money_in - money_out;
            YearlyCashFlow {
                year,
                money_in,
                money_out,
                net_flow,
                by_asset_type,
            }
        })
        .collect();

    years.sort_by_key(|y| y.year);

    Ok(CashFlowReport {
        from_date,
        to_date,
        years,
        total_in,
        total_out,
        net_flow: total_in - total_out,
    })
}

pub fn calculate_cash_flow_stats(
    conn: &rusqlite::Connection,
    from_date: NaiveDate,
    to_date: NaiveDate,
) -> Result<CashFlowStats> {
    let entries = cash_flow_entries(conn, from_date, to_date)?;

    let mut monthly: HashMap<(i32, u32), (Decimal, Decimal, Decimal)> = HashMap::new();
    let mut yearly: HashMap<i32, (Decimal, Decimal, Decimal)> = HashMap::new();

    for entry in &entries {
        let month_key = (entry.date.year(), entry.date.month());
        let month =
            monthly
                .entry(month_key)
                .or_insert((Decimal::ZERO, Decimal::ZERO, Decimal::ZERO));
        month.0 += entry.money_in;
        month.1 += entry.money_out_sells;
        month.2 += entry.money_out_income;

        let year_key = entry.date.year();
        let year = yearly
            .entry(year_key)
            .or_insert((Decimal::ZERO, Decimal::ZERO, Decimal::ZERO));
        year.0 += entry.money_in;
        year.1 += entry.money_out_sells;
        year.2 += entry.money_out_income;
    }

    let months_with_data = monthly.len();
    let years_with_data = yearly.len();

    let total_monthly_in = monthly.values().fold(Decimal::ZERO, |acc, v| acc + v.0);
    let total_monthly_out_sells = monthly.values().fold(Decimal::ZERO, |acc, v| acc + v.1);
    let total_monthly_out_income = monthly.values().fold(Decimal::ZERO, |acc, v| acc + v.2);
    let total_monthly_out = total_monthly_out_sells + total_monthly_out_income;

    let total_yearly_in = yearly.values().fold(Decimal::ZERO, |acc, v| acc + v.0);
    let total_yearly_out_sells = yearly.values().fold(Decimal::ZERO, |acc, v| acc + v.1);
    let total_yearly_out_income = yearly.values().fold(Decimal::ZERO, |acc, v| acc + v.2);
    let total_yearly_out = total_yearly_out_sells + total_yearly_out_income;

    let avg_monthly_in = if months_with_data > 0 {
        total_monthly_in / Decimal::from(months_with_data as i64)
    } else {
        Decimal::ZERO
    };
    let avg_monthly_out = if months_with_data > 0 {
        total_monthly_out / Decimal::from(months_with_data as i64)
    } else {
        Decimal::ZERO
    };
    let avg_monthly_net = avg_monthly_in - avg_monthly_out;
    let avg_yearly_in = if years_with_data > 0 {
        total_yearly_in / Decimal::from(years_with_data as i64)
    } else {
        Decimal::ZERO
    };
    let avg_yearly_out = if years_with_data > 0 {
        total_yearly_out / Decimal::from(years_with_data as i64)
    } else {
        Decimal::ZERO
    };
    let avg_yearly_net = avg_yearly_in - avg_yearly_out;

    let mut yearly_pairs: Vec<(i32, Decimal)> = yearly
        .iter()
        .map(|(year, (money_in, money_out_sells, money_out_income))| {
            (*year, *money_in - *money_out_sells - *money_out_income)
        })
        .collect();
    yearly_pairs.sort_by_key(|(year, _)| *year);

    let mut yearly_changes = Vec::new();
    let mut growth_rates = Vec::new();

    if let Some((first_year, first_net)) = yearly_pairs.first().copied() {
        yearly_changes.push(YearlyChange {
            year: first_year,
            prev_year_net: Decimal::ZERO,
            curr_year_net: first_net,
            growth_rate_pct: None,
        });
    }

    for window in yearly_pairs.windows(2) {
        let (_prev_year, prev_net) = window[0];
        let (curr_year, curr_net) = window[1];
        let growth = if prev_net > Decimal::ZERO {
            Some(((curr_net - prev_net) / prev_net) * Decimal::from(100))
        } else {
            None
        };
        if let Some(rate) = growth {
            growth_rates.push(rate);
        }
        yearly_changes.push(YearlyChange {
            year: curr_year,
            prev_year_net: prev_net,
            curr_year_net: curr_net,
            growth_rate_pct: growth,
        });
    }

    let avg_yearly_growth_rate = if !growth_rates.is_empty() {
        Some(
            growth_rates.iter().fold(Decimal::ZERO, |acc, v| acc + *v)
                / Decimal::from(growth_rates.len() as i64),
        )
    } else {
        None
    };

    let direction = match avg_yearly_growth_rate {
        Some(rate) if rate > Decimal::ZERO => TrendDirection::Increasing,
        Some(rate) if rate < Decimal::ZERO => TrendDirection::Decreasing,
        _ => TrendDirection::Stable,
    };

    Ok(CashFlowStats {
        avg_monthly_in,
        avg_monthly_out,
        avg_monthly_net,
        avg_yearly_in,
        avg_yearly_out,
        avg_yearly_net,
        months_with_data,
        years_with_data,
        trend: CashFlowTrend {
            direction,
            avg_yearly_growth_rate,
            yearly_changes,
        },
    })
}

pub fn cash_flow_entries(
    conn: &rusqlite::Connection,
    from_date: NaiveDate,
    to_date: NaiveDate,
) -> Result<Vec<CashFlowEntry>> {
    let mut entries = Vec::new();

    let mut stmt = conn.prepare(
        "SELECT COALESCE(t.settlement_date, t.trade_date) as flow_date,
                t.transaction_type,
                t.quantity,
                t.price_per_unit,
                t.fees,
                a.asset_type
         FROM transactions t
         JOIN assets a ON t.asset_id = a.id
         WHERE COALESCE(t.settlement_date, t.trade_date) >= ?1
           AND COALESCE(t.settlement_date, t.trade_date) <= ?2
           AND t.transaction_type IN ('BUY', 'SELL')
           AND (t.notes IS NULL OR t.notes NOT LIKE '%Term contract liquidation%')
         ORDER BY flow_date",
    )?;

    let tx_rows = stmt.query_map([from_date, to_date], |row| {
        let date: NaiveDate = row.get(0)?;
        let tx_type_str: String = row.get(1)?;
        let quantity = db::get_decimal_value(row, 2)?;
        let price = db::get_decimal_value(row, 3)?;
        let fees = db::get_optional_decimal_value(row, 4)?.unwrap_or(Decimal::ZERO);
        let asset_type_str: String = row.get(5)?;

        let tx_type = tx_type_str
            .parse::<TransactionType>()
            .unwrap_or(TransactionType::Buy);
        let asset_type = asset_type_str
            .parse::<AssetType>()
            .unwrap_or(AssetType::Unknown);

        let gross = quantity * price;
        let net_amount = match tx_type {
            TransactionType::Buy => gross + fees,
            TransactionType::Sell => gross - fees,
        };

        let (money_in, money_out_sells) = match tx_type {
            TransactionType::Buy => (net_amount, Decimal::ZERO),
            TransactionType::Sell => (Decimal::ZERO, net_amount),
        };

        Ok(CashFlowEntry {
            date,
            asset_type,
            money_in,
            money_out_sells,
            money_out_income: Decimal::ZERO,
        })
    })?;

    for row in tx_rows {
        entries.push(row?);
    }

    let income_events =
        db::get_income_events_with_assets(conn, Some(from_date), Some(to_date), None)?;
    for (event, asset) in income_events {
        let withholding = event.withholding_tax;
        let net_income = event.total_amount - withholding;

        entries.push(CashFlowEntry {
            date: event.event_date,
            asset_type: asset.asset_type,
            money_in: Decimal::ZERO,
            money_out_sells: Decimal::ZERO,
            money_out_income: net_income,
        });
    }

    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{self, IncomeEventType, Transaction, TransactionType};
    use chrono::NaiveDate;
    use rusqlite::Connection;
    use rust_decimal::Decimal;

    #[test]
    fn test_cash_flow_entries_netting_and_settlement() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!("../db/schema.sql"))
            .unwrap();

        let asset_id = db::insert_asset(&conn, "TEST1", &AssetType::Stock, None).unwrap();

        let buy_tx = Transaction {
            id: None,
            asset_id,
            transaction_type: TransactionType::Buy,
            trade_date: NaiveDate::from_ymd_opt(2024, 1, 10).unwrap(),
            settlement_date: Some(NaiveDate::from_ymd_opt(2024, 1, 12).unwrap()),
            quantity: Decimal::from(10),
            price_per_unit: Decimal::from(10),
            total_cost: Decimal::from(100),
            fees: Decimal::from(2),
            is_day_trade: false,
            quota_issuance_date: None,
            notes: None,
            source: "TEST".to_string(),
            created_at: chrono::Utc::now(),
        };
        db::insert_transaction(&conn, &buy_tx).unwrap();

        let sell_tx = Transaction {
            id: None,
            asset_id,
            transaction_type: TransactionType::Sell,
            trade_date: NaiveDate::from_ymd_opt(2024, 2, 5).unwrap(),
            settlement_date: Some(NaiveDate::from_ymd_opt(2024, 2, 7).unwrap()),
            quantity: Decimal::from(5),
            price_per_unit: Decimal::from(12),
            total_cost: Decimal::from(60),
            fees: Decimal::from(1),
            is_day_trade: false,
            quota_issuance_date: None,
            notes: None,
            source: "TEST".to_string(),
            created_at: chrono::Utc::now(),
        };
        db::insert_transaction(&conn, &sell_tx).unwrap();

        let income_event = db::IncomeEvent {
            id: None,
            asset_id,
            event_date: NaiveDate::from_ymd_opt(2024, 2, 15).unwrap(),
            ex_date: None,
            event_type: IncomeEventType::Dividend,
            amount_per_quota: Decimal::ZERO,
            total_amount: Decimal::from(20),
            withholding_tax: Decimal::from(3),
            is_quota_pre_2026: None,
            source: "TEST".to_string(),
            notes: None,
            created_at: chrono::Utc::now(),
        };
        db::insert_income_event(&conn, &income_event).unwrap();

        let flows = cash_flow_entries(
            &conn,
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 12, 31).unwrap(),
        )
        .unwrap();

        assert_eq!(flows.len(), 3);

        let buy_flow = flows.iter().find(|f| f.money_in > Decimal::ZERO).unwrap();
        assert_eq!(buy_flow.date, NaiveDate::from_ymd_opt(2024, 1, 12).unwrap());
        assert_eq!(buy_flow.money_in, Decimal::from(102));
        assert_eq!(buy_flow.money_out_sells, Decimal::ZERO);
        assert_eq!(buy_flow.money_out_income, Decimal::ZERO);

        let sell_flow = flows
            .iter()
            .find(|f| f.money_out_sells > Decimal::ZERO)
            .unwrap();
        assert_eq!(sell_flow.date, NaiveDate::from_ymd_opt(2024, 2, 7).unwrap());
        assert_eq!(sell_flow.money_out_sells, Decimal::from(59));

        let income_flow = flows
            .iter()
            .find(|f| f.money_out_income > Decimal::ZERO)
            .unwrap();
        assert_eq!(
            income_flow.date,
            NaiveDate::from_ymd_opt(2024, 2, 15).unwrap()
        );
        assert_eq!(income_flow.money_out_income, Decimal::from(17));
    }

    #[test]
    fn test_calculate_cash_flow_report_totals() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!("../db/schema.sql"))
            .unwrap();

        let asset_id = db::insert_asset(&conn, "TEST2", &AssetType::Fii, None).unwrap();

        let buy_tx = Transaction {
            id: None,
            asset_id,
            transaction_type: TransactionType::Buy,
            trade_date: NaiveDate::from_ymd_opt(2024, 6, 1).unwrap(),
            settlement_date: None,
            quantity: Decimal::from(2),
            price_per_unit: Decimal::from(50),
            total_cost: Decimal::from(100),
            fees: Decimal::from(1),
            is_day_trade: false,
            quota_issuance_date: None,
            notes: None,
            source: "TEST".to_string(),
            created_at: chrono::Utc::now(),
        };
        db::insert_transaction(&conn, &buy_tx).unwrap();

        let income_event = db::IncomeEvent {
            id: None,
            asset_id,
            event_date: NaiveDate::from_ymd_opt(2024, 6, 15).unwrap(),
            ex_date: None,
            event_type: IncomeEventType::Jcp,
            amount_per_quota: Decimal::ZERO,
            total_amount: Decimal::from(10),
            withholding_tax: Decimal::from(2),
            is_quota_pre_2026: None,
            source: "TEST".to_string(),
            notes: None,
            created_at: chrono::Utc::now(),
        };
        db::insert_income_event(&conn, &income_event).unwrap();

        let report = calculate_cash_flow_report(
            &conn,
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 12, 31).unwrap(),
        )
        .unwrap();

        assert_eq!(report.total_in, Decimal::from(101));
        assert_eq!(report.total_out, Decimal::from(8));
        assert_eq!(report.net_flow, Decimal::from(93));
        assert_eq!(report.years.len(), 1);

        let year = &report.years[0];
        let asset = year.by_asset_type.get(&AssetType::Fii).unwrap_or_else(|| {
            panic!(
                "Expected FII entry, got keys: {:?}",
                year.by_asset_type.keys()
            )
        });
        assert_eq!(asset.money_in, Decimal::from(101));
        assert_eq!(asset.money_out_income, Decimal::from(8));
        assert_eq!(asset.net_flow, Decimal::from(93));
    }
}
