//! Income analytics calculations: yield, trends, forecasting, baseline detection
//!
//! This module provides analytical functions to transform raw income event data into
//! actionable insights for portfolio management.

#![allow(dead_code)]

use crate::db::{self, AssetType, IncomeEvent, IncomeEventType};
use anyhow::Result;
use chrono::{Datelike, Months, NaiveDate};
use rust_decimal::Decimal;
use std::collections::{BTreeMap, HashMap};

/// Newton-Raphson square root approximation for Decimal.
fn decimal_sqrt(value: Decimal) -> Decimal {
    if value.is_zero() {
        return Decimal::ZERO;
    }
    let mut approx = value;
    for _ in 0..5 {
        approx = (approx + value / approx) / Decimal::from(2);
    }
    approx
}

/// Result of last twelve months (LTM) yield calculation
#[derive(Debug, Clone)]
pub struct LtmYieldResult {
    pub total_ltm_income: Decimal,
    pub portfolio_value: Decimal,
    pub yield_percentage: Decimal,
}

/// Per-asset yield with consistency metrics
#[derive(Debug, Clone)]
pub struct AssetYield {
    pub asset_id: i64,
    pub ticker: String,
    pub asset_type: AssetType,
    pub ltm_income: Decimal,
    pub current_position_value: Decimal,
    pub yield_percentage: Decimal,
}

/// Trend analysis result
#[derive(Debug, Clone)]
pub struct TrendAnalysis {
    pub slope: Decimal,
    pub trend_direction: TrendDirection,
    pub yoy_growth_percentage: Decimal,
    pub volatility: Decimal,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TrendDirection {
    Growing,
    Declining,
    Stable,
}

impl TrendDirection {
    pub fn as_str(&self) -> &'static str {
        match self {
            TrendDirection::Growing => "Growing",
            TrendDirection::Declining => "Declining",
            TrendDirection::Stable => "Stable",
        }
    }
}

/// Monthly income series for analysis
#[derive(Debug, Clone)]
pub struct MonthlyIncomeSeries {
    pub months: Vec<NaiveDate>,
    pub amounts: Vec<Decimal>,
}

impl MonthlyIncomeSeries {
    pub fn mean(&self) -> Decimal {
        if self.amounts.is_empty() {
            return Decimal::ZERO;
        }
        self.amounts.iter().sum::<Decimal>() / Decimal::from(self.amounts.len())
    }

    pub fn stddev(&self) -> Decimal {
        let mean = self.mean();
        if self.amounts.len() < 2 {
            return Decimal::ZERO;
        }

        let variance = self
            .amounts
            .iter()
            .map(|x| (x - mean) * (x - mean))
            .sum::<Decimal>()
            / Decimal::from(self.amounts.len() - 1);

        decimal_sqrt(variance)
    }

    pub fn coefficient_of_variation(&self) -> Decimal {
        if self.amounts.is_empty() || self.amounts.len() < 2 {
            return Decimal::ZERO;
        }

        let mean = self.mean();
        if mean.is_zero() {
            return Decimal::ZERO;
        }

        let stddev = self.stddev();
        if stddev.is_zero() {
            return Decimal::ZERO;
        }

        stddev / mean
    }
}

/// Confidence level for forecasts
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ConfidenceLevel {
    Low,
    Medium,
    High,
}

impl ConfidenceLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            ConfidenceLevel::Low => "Low",
            ConfidenceLevel::Medium => "Medium",
            ConfidenceLevel::High => "High",
        }
    }
}

/// Income forecast result
#[derive(Debug, Clone)]
pub struct IncomeForecast {
    pub asset_id: i64,
    pub ticker: String,
    pub expected_annual_income: Decimal,
    pub lower_bound: Decimal,
    pub upper_bound: Decimal,
    pub confidence: ConfidenceLevel,
    pub months_of_history: usize,
    pub baseline_avg_monthly: Decimal,
}

/// Categorization of income as baseline or exceptional
#[derive(Debug, Clone)]
pub struct IncomeCategory {
    pub is_baseline: bool,
    pub reason: String,
}

// ============================================================================
// 1. YIELD CALCULATIONS
// ============================================================================

/// Calculate LTM yield for entire portfolio
pub fn calculate_ltm_yield(
    conn: &rusqlite::Connection,
    current_portfolio_value: Decimal,
    as_of: NaiveDate,
) -> Result<LtmYieldResult> {
    let one_year_ago = as_of - Months::new(12);

    let events = db::get_income_events_with_assets(conn, Some(one_year_ago), Some(as_of), None)?;

    let mut total_income = Decimal::ZERO;
    for (event, _asset) in &events {
        total_income += event.total_amount;
    }

    let yield_percentage = if current_portfolio_value.is_zero() {
        Decimal::ZERO
    } else {
        (total_income / current_portfolio_value) * Decimal::from(100)
    };

    Ok(LtmYieldResult {
        total_ltm_income: total_income,
        portfolio_value: current_portfolio_value,
        yield_percentage,
    })
}

/// Calculate yield for a specific asset
pub fn calculate_asset_yield(
    conn: &rusqlite::Connection,
    asset_id: i64,
    ticker: &str,
    asset_type: AssetType,
    current_position_value: Decimal,
    as_of: NaiveDate,
) -> Result<AssetYield> {
    let one_year_ago = as_of - Months::new(12);

    let events =
        db::get_income_events_with_assets(conn, Some(one_year_ago), Some(as_of), Some(ticker))?;

    let mut total_ltm_income = Decimal::ZERO;
    for (event, _asset) in &events {
        total_ltm_income += event.total_amount;
    }

    let yield_pct = if current_position_value.is_zero() {
        Decimal::ZERO
    } else {
        (total_ltm_income / current_position_value) * Decimal::from(100)
    };

    Ok(AssetYield {
        asset_id,
        ticker: ticker.to_string(),
        asset_type,
        ltm_income: total_ltm_income,
        current_position_value,
        yield_percentage: yield_pct,
    })
}

// ============================================================================
// 2. TREND ANALYSIS
// ============================================================================

/// Build a monthly income series from pre-fetched events.
///
/// Returns a contiguous series from `start_date` to `as_of`, filling months
/// with no income events as zero. This ensures gaps (payment droughts) are
/// visible in trend analysis and volatility calculations.
pub fn build_monthly_series_from_events(
    events: &[(IncomeEvent, db::Asset)],
    start_date: NaiveDate,
    as_of: NaiveDate,
) -> MonthlyIncomeSeries {
    let mut monthly: BTreeMap<(i32, u32), Decimal> = BTreeMap::new();
    for (event, _asset) in events {
        let year = event.event_date.year();
        let month = event.event_date.month();
        monthly
            .entry((year, month))
            .and_modify(|total| *total += event.total_amount)
            .or_insert(event.total_amount);
    }

    let mut months = Vec::new();
    let mut amounts = Vec::new();

    let mut cursor = NaiveDate::from_ymd_opt(start_date.year(), start_date.month(), 1).unwrap();
    let end = NaiveDate::from_ymd_opt(as_of.year(), as_of.month(), 1).unwrap();

    while cursor <= end {
        let key = (cursor.year(), cursor.month());
        let amount = monthly.get(&key).copied().unwrap_or(Decimal::ZERO);
        months.push(cursor);
        amounts.push(amount);

        cursor = if cursor.month() == 12 {
            NaiveDate::from_ymd_opt(cursor.year() + 1, 1, 1).unwrap()
        } else {
            NaiveDate::from_ymd_opt(cursor.year(), cursor.month() + 1, 1).unwrap()
        };
    }

    MonthlyIncomeSeries { months, amounts }
}

/// Convenience: fetch events from DB and build monthly series.
pub fn get_monthly_income_series(
    conn: &rusqlite::Connection,
    months_back: i32,
    ticker: Option<&str>,
    as_of: NaiveDate,
) -> Result<MonthlyIncomeSeries> {
    let start_date = as_of - Months::new(months_back as u32);
    let events = db::get_income_events_with_assets(conn, Some(start_date), Some(as_of), ticker)?;
    Ok(build_monthly_series_from_events(&events, start_date, as_of))
}

/// Analyze income trends using linear regression.
///
/// Accepts a pre-computed series to avoid redundant DB queries when the
/// caller already has the data (e.g. `dispatch_income_trends`).
pub fn analyze_income_trends_from_series(series: &MonthlyIncomeSeries) -> TrendAnalysis {
    if series.amounts.len() < 2 {
        return TrendAnalysis {
            slope: Decimal::ZERO,
            trend_direction: TrendDirection::Stable,
            yoy_growth_percentage: Decimal::ZERO,
            volatility: Decimal::ZERO,
        };
    }

    // Linear regression: y = a + bx
    let x_mean = (Decimal::from(series.amounts.len()) - Decimal::ONE) / Decimal::from(2);
    let y_mean = series.mean();

    let mut numerator = Decimal::ZERO;
    let mut denominator = Decimal::ZERO;

    for (i, y) in series.amounts.iter().enumerate() {
        let x = Decimal::from(i as i32);
        numerator += (x - x_mean) * (y - y_mean);
        denominator += (x - x_mean) * (x - x_mean);
    }

    let slope = if denominator.is_zero() {
        Decimal::ZERO
    } else {
        numerator / denominator
    };

    let trend_direction = if slope > Decimal::from_str_exact("0.01").unwrap_or(Decimal::ZERO) {
        TrendDirection::Growing
    } else if slope < Decimal::from_str_exact("-0.01").unwrap_or(Decimal::ZERO) {
        TrendDirection::Declining
    } else {
        TrendDirection::Stable
    };

    // YoY growth: compare first half vs second half amounts
    let yoy_growth = if series.amounts.len() >= 24 {
        let first_half: Decimal = series.amounts[..series.amounts.len() / 2].iter().sum();
        let second_half: Decimal = series.amounts[series.amounts.len() / 2..].iter().sum();

        if first_half.is_zero() {
            Decimal::ZERO
        } else {
            ((second_half - first_half) / first_half) * Decimal::from(100)
        }
    } else {
        Decimal::ZERO
    };

    let volatility = series.coefficient_of_variation();

    TrendAnalysis {
        slope,
        trend_direction,
        yoy_growth_percentage: yoy_growth,
        volatility,
    }
}

/// Convenience wrapper: fetch series from DB and analyze trends.
pub fn analyze_income_trends(
    conn: &rusqlite::Connection,
    months_back: i32,
    ticker: Option<&str>,
    as_of: NaiveDate,
) -> Result<TrendAnalysis> {
    let series = get_monthly_income_series(conn, months_back, ticker, as_of)?;
    Ok(analyze_income_trends_from_series(&series))
}

// ============================================================================
// 3. BASELINE VS EXCEPTIONAL DETECTION
// ============================================================================

/// Detect if income event is baseline (recurring) or exceptional.
///
/// Accepts pre-grouped history (events of the same ticker and event_type) to
/// avoid scanning the full events slice per call. Prefer this over
/// `categorize_income_event` when processing many events.
pub fn categorize_income_event_with_history(
    event: &IncomeEvent,
    same_type_history: &[&IncomeEvent],
) -> IncomeCategory {
    // Rule 1: Amortization is almost always exceptional
    if event.event_type == IncomeEventType::Amortization {
        return IncomeCategory {
            is_baseline: false,
            reason: "Amortization events are typically one-time capital returns".to_string(),
        };
    }

    // Get preceding events in the last 12 months
    let twelve_months_ago = event.event_date - Months::new(12);
    let preceding_amounts: Vec<Decimal> = same_type_history
        .iter()
        .filter(|e| e.event_date < event.event_date && e.event_date >= twelve_months_ago)
        .map(|e| e.total_amount)
        .collect();

    // Rule 2: First-ever payment of this type is exceptional
    if preceding_amounts.is_empty() {
        return IncomeCategory {
            is_baseline: false,
            reason: "First payment of this type from this asset".to_string(),
        };
    }

    // Rule 3: Check if amount is 2+ standard deviations above mean
    if preceding_amounts.len() >= 3 {
        let mean =
            preceding_amounts.iter().sum::<Decimal>() / Decimal::from(preceding_amounts.len());
        let variance = preceding_amounts
            .iter()
            .map(|x| (x - mean) * (x - mean))
            .sum::<Decimal>()
            / Decimal::from(preceding_amounts.len());

        if !variance.is_zero() {
            let stddev = decimal_sqrt(variance);
            if event.total_amount > mean + (Decimal::from(2) * stddev) {
                return IncomeCategory {
                    is_baseline: false,
                    reason: format!(
                        "Amount significantly higher than historical average ({:.2} vs avg {:.2})",
                        event.total_amount, mean
                    ),
                };
            }
        }
    }

    IncomeCategory {
        is_baseline: true,
        reason: "Regular recurring income".to_string(),
    }
}

/// Convenience wrapper: categorize using the full events slice (scans for matching ticker/type).
pub fn categorize_income_event(
    event: &IncomeEvent,
    all_events: &[(IncomeEvent, db::Asset)],
    ticker: &str,
) -> IncomeCategory {
    let history: Vec<&IncomeEvent> = all_events
        .iter()
        .filter(|(_, a)| a.ticker == ticker)
        .filter(|(e, _)| e.event_type == event.event_type)
        .map(|(e, _)| e)
        .collect();
    categorize_income_event_with_history(event, &history)
}

// ============================================================================
// 4. FORECASTING
// ============================================================================

/// Generate income forecast from pre-fetched events (covers at least 2 years).
pub fn forecast_income_from_events(
    events: &[(IncomeEvent, db::Asset)],
    asset_id: i64,
    ticker: &str,
    conservative: bool,
    as_of: NaiveDate,
) -> IncomeForecast {
    let one_year_ago = as_of - Months::new(12);

    let ltm_events: Vec<_> = events
        .iter()
        .filter(|(e, _)| e.event_date >= one_year_ago)
        .collect();

    if ltm_events.is_empty() {
        return IncomeForecast {
            asset_id,
            ticker: ticker.to_string(),
            expected_annual_income: Decimal::ZERO,
            lower_bound: Decimal::ZERO,
            upper_bound: Decimal::ZERO,
            confidence: ConfidenceLevel::Low,
            months_of_history: 0,
            baseline_avg_monthly: Decimal::ZERO,
        };
    }

    let ltm_total: Decimal = ltm_events.iter().map(|(e, _)| e.total_amount).sum();
    let ltm_monthly_avg = ltm_total / Decimal::from(12);

    let mut income_months: std::collections::HashSet<(i32, u32)> = std::collections::HashSet::new();
    for (event, _) in &ltm_events {
        income_months.insert((event.event_date.year(), event.event_date.month()));
    }

    let months_of_history = income_months.len();

    let confidence = if months_of_history >= 12 {
        ConfidenceLevel::High
    } else if months_of_history >= 6 {
        ConfidenceLevel::Medium
    } else {
        ConfidenceLevel::Low
    };

    // Apply trend adjustment if enough history
    let mut expected = ltm_total;
    let series_start = as_of - Months::new(24);
    let series = build_monthly_series_from_events(events, series_start, as_of);
    if series.amounts.len() >= 24 {
        let trend = analyze_income_trends_from_series(&series);
        if trend.trend_direction == TrendDirection::Growing
            && !trend.yoy_growth_percentage.is_zero()
        {
            let growth_factor = Decimal::ONE + (trend.yoy_growth_percentage / Decimal::from(100));
            expected *= growth_factor;
        }
    }

    if conservative {
        expected *= Decimal::from_str_exact("0.85").unwrap_or(Decimal::ONE);
    }

    let margin = match confidence {
        ConfidenceLevel::High => Decimal::from_str_exact("0.15").unwrap_or(Decimal::ZERO),
        ConfidenceLevel::Medium => Decimal::from_str_exact("0.25").unwrap_or(Decimal::ZERO),
        ConfidenceLevel::Low => Decimal::from_str_exact("0.35").unwrap_or(Decimal::ZERO),
    };

    let lower_bound = expected * (Decimal::ONE - margin);
    let upper_bound = expected * (Decimal::ONE + margin);

    IncomeForecast {
        asset_id,
        ticker: ticker.to_string(),
        expected_annual_income: expected,
        lower_bound,
        upper_bound,
        confidence,
        months_of_history,
        baseline_avg_monthly: ltm_monthly_avg,
    }
}

/// Convenience wrapper: fetch events from DB and forecast.
pub fn forecast_income_for_asset(
    conn: &rusqlite::Connection,
    asset_id: i64,
    ticker: &str,
    conservative: bool,
    as_of: NaiveDate,
) -> Result<IncomeForecast> {
    let two_years_ago = as_of - Months::new(24);
    let events =
        db::get_income_events_with_assets(conn, Some(two_years_ago), Some(as_of), Some(ticker))?;
    Ok(forecast_income_from_events(
        &events,
        asset_id,
        ticker,
        conservative,
        as_of,
    ))
}

// ============================================================================
// 5. CALENDAR & PAYMENT PREDICTION
// ============================================================================

/// Predict payment dates from pre-fetched events (covers at least 2 years).
pub fn predict_payment_dates_from_events(
    events: &[(IncomeEvent, db::Asset)],
    num_predictions: usize,
    as_of: NaiveDate,
) -> Vec<(NaiveDate, Decimal, ConfidenceLevel)> {
    if events.is_empty() {
        return Vec::new();
    }

    let series_start = as_of - Months::new(24);
    let series = build_monthly_series_from_events(events, series_start, as_of);

    if series.months.is_empty() {
        return Vec::new();
    }

    // Find most common day of month from raw events
    let mut day_freq: HashMap<u32, usize> = HashMap::new();
    for (event, _) in events {
        *day_freq.entry(event.event_date.day()).or_insert(0) += 1;
    }

    let typical_day = day_freq
        .iter()
        .max_by_key(|(_, count)| *count)
        .map(|(day, _)| *day)
        .unwrap_or(15);

    let avg_amount = series.mean();
    let confidence = if series.amounts.len() >= 12 {
        ConfidenceLevel::High
    } else if series.amounts.len() >= 6 {
        ConfidenceLevel::Medium
    } else {
        ConfidenceLevel::Low
    };

    let mut predictions = Vec::new();

    let mut year = as_of.year();
    let mut month = as_of.month();
    for _ in 0..num_predictions {
        month += 1;
        if month > 12 {
            month = 1;
            year += 1;
        }
        if let Some(predicted_date) =
            NaiveDate::from_ymd_opt(year, month, std::cmp::min(typical_day, 28))
        {
            predictions.push((predicted_date, avg_amount, confidence));
        }
    }

    predictions
}

/// Convenience wrapper: fetch events from DB and predict.
pub fn predict_payment_dates(
    conn: &rusqlite::Connection,
    ticker: &str,
    num_predictions: usize,
    as_of: NaiveDate,
) -> Result<Vec<(NaiveDate, Decimal, ConfidenceLevel)>> {
    let two_years_ago = as_of - Months::new(24);
    let events =
        db::get_income_events_with_assets(conn, Some(two_years_ago), Some(as_of), Some(ticker))?;
    Ok(predict_payment_dates_from_events(
        &events,
        num_predictions,
        as_of,
    ))
}

// ============================================================================
// 6. ANOMALY DETECTION
// ============================================================================

#[derive(Debug, Clone)]
pub struct IncomeAnomaly {
    pub anomaly_type: AnomalyType,
    pub ticker: String,
    pub description: String,
}

#[derive(Debug, Clone)]
pub enum AnomalyType {
    MissedPayment,
    UnusualAmount,
    IncomeDrop,
}

/// Detect anomalies for the current month
pub fn detect_anomalies(
    conn: &rusqlite::Connection,
    as_of: NaiveDate,
) -> Result<Vec<IncomeAnomaly>> {
    let this_month_start = NaiveDate::from_ymd_opt(as_of.year(), as_of.month(), 1).unwrap();
    let last_month_start = if as_of.month() == 1 {
        NaiveDate::from_ymd_opt(as_of.year() - 1, 12, 1).unwrap()
    } else {
        NaiveDate::from_ymd_opt(as_of.year(), as_of.month() - 1, 1).unwrap()
    };

    let events =
        db::get_income_events_with_assets(conn, Some(this_month_start), Some(as_of), None)?;

    let mut anomalies = Vec::new();
    let mut tickers_this_month: std::collections::HashSet<String> =
        std::collections::HashSet::new();

    for (_, asset) in &events {
        tickers_this_month.insert(asset.ticker.clone());
    }

    // Check for missed payments: assets that paid last month but not this month (yet)
    let last_month_events = db::get_income_events_with_assets(
        conn,
        Some(last_month_start),
        Some(this_month_start),
        None,
    )?;

    let last_month_tickers: std::collections::HashSet<&str> = last_month_events
        .iter()
        .map(|(_, asset)| asset.ticker.as_str())
        .collect();

    for ticker in &last_month_tickers {
        if !tickers_this_month.contains(*ticker) && as_of.day() > 15 {
            anomalies.push(IncomeAnomaly {
                anomaly_type: AnomalyType::MissedPayment,
                ticker: ticker.to_string(),
                description: format!(
                    "{}: Expected payment not yet received (typically pays in this month)",
                    ticker
                ),
            });
        }
    }

    Ok(anomalies)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trend_direction_stable() {
        let series = MonthlyIncomeSeries {
            months: vec![
                NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
                NaiveDate::from_ymd_opt(2025, 2, 1).unwrap(),
                NaiveDate::from_ymd_opt(2025, 3, 1).unwrap(),
            ],
            amounts: vec![
                Decimal::from_str_exact("1000").unwrap(),
                Decimal::from_str_exact("1000").unwrap(),
                Decimal::from_str_exact("1000").unwrap(),
            ],
        };
        assert_eq!(series.mean(), Decimal::from_str_exact("1000").unwrap());
    }

    #[test]
    fn test_confidence_levels() {
        assert_eq!(ConfidenceLevel::Low.as_str(), "Low");
        assert_eq!(ConfidenceLevel::Medium.as_str(), "Medium");
        assert_eq!(ConfidenceLevel::High.as_str(), "High");
    }

    #[test]
    fn test_mean_and_stddev_zero_for_constant_series() {
        let series = MonthlyIncomeSeries {
            months: vec![
                NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
                NaiveDate::from_ymd_opt(2025, 2, 1).unwrap(),
            ],
            amounts: vec![
                Decimal::from_str_exact("500").unwrap(),
                Decimal::from_str_exact("500").unwrap(),
            ],
        };

        assert_eq!(series.mean(), Decimal::from_str_exact("500").unwrap());
        assert_eq!(series.stddev(), Decimal::ZERO);
        assert_eq!(series.coefficient_of_variation(), Decimal::ZERO);
    }

    #[test]
    fn test_coefficient_of_variation_handles_zero_mean() {
        let series = MonthlyIncomeSeries {
            months: vec![
                NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
                NaiveDate::from_ymd_opt(2025, 2, 1).unwrap(),
            ],
            amounts: vec![Decimal::ZERO, Decimal::ZERO],
        };

        assert_eq!(series.mean(), Decimal::ZERO);
        assert_eq!(series.coefficient_of_variation(), Decimal::ZERO);
    }

    #[test]
    fn test_stddev_positive_for_variable_series() {
        let series = MonthlyIncomeSeries {
            months: vec![
                NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
                NaiveDate::from_ymd_opt(2025, 2, 1).unwrap(),
                NaiveDate::from_ymd_opt(2025, 3, 1).unwrap(),
            ],
            amounts: vec![
                Decimal::from_str_exact("100").unwrap(),
                Decimal::from_str_exact("200").unwrap(),
                Decimal::from_str_exact("300").unwrap(),
            ],
        };

        assert!(series.stddev() > Decimal::ZERO);
        assert!(series.coefficient_of_variation() > Decimal::ZERO);
    }
}
