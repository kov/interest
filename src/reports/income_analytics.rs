//! Income analytics calculations: yield, trends, forecasting, baseline detection
//!
//! This module provides analytical functions to transform raw income event data into
//! actionable insights for portfolio management.

#![allow(dead_code)] // Many functions will be used in later phases

use crate::db::{self, AssetType, IncomeEvent, IncomeEventType};
use anyhow::Result;
use chrono::{Datelike, Duration, Local, NaiveDate};
use rust_decimal::Decimal;
use std::collections::{BTreeMap, HashMap};

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
    pub consistency_score: Decimal, // 0-10 scale
    pub months_with_income: usize,
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
            TrendDirection::Growing => "Growing ▲",
            TrendDirection::Declining => "Declining ▼",
            TrendDirection::Stable => "Stable ─",
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

        if variance.is_zero() {
            return Decimal::ZERO;
        }

        // Simple square root approximation: good enough for financial metrics
        let mut sqrt_approx = variance;
        for _ in 0..5 {
            sqrt_approx = (sqrt_approx + variance / sqrt_approx) / Decimal::from(2);
        }
        sqrt_approx
    }

    pub fn coefficient_of_variation(&self) -> Decimal {
        // Early return if insufficient data
        if self.amounts.is_empty() || self.amounts.len() < 2 {
            return Decimal::ZERO;
        }

        let mean = self.mean();
        // If mean is zero, CV is undefined - return 0
        if mean.is_zero() {
            return Decimal::ZERO;
        }

        let stddev = self.stddev();
        // If stddev is zero, no variance
        if stddev.is_zero() {
            return Decimal::ZERO;
        }

        // Compute CV - both mean and stddev verified non-zero above
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
) -> Result<LtmYieldResult> {
    let today = Local::now().date_naive();
    let one_year_ago = today - Duration::days(365);

    let events = db::get_income_events_with_assets(conn, Some(one_year_ago), Some(today), None)?;

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
) -> Result<AssetYield> {
    let today = Local::now().date_naive();
    let one_year_ago = today - Duration::days(365);

    // Get all income events for this asset in the last 12 months
    let events =
        db::get_income_events_with_assets(conn, Some(one_year_ago), Some(today), Some(ticker))?;

    let mut total_ltm_income = Decimal::ZERO;
    let mut income_months: std::collections::HashSet<(i32, u32)> = std::collections::HashSet::new();

    for (event, _asset) in &events {
        total_ltm_income += event.total_amount;
        income_months.insert((event.event_date.year(), event.event_date.month()));
    }

    let months_with_income = income_months.len();

    let yield_pct = if current_position_value.is_zero() {
        Decimal::ZERO
    } else {
        (total_ltm_income / current_position_value) * Decimal::from(100)
    };

    // Consistency: months with income out of 12, expressed as 0-10 scale
    let consistency_score =
        Decimal::from(months_with_income as u32) / Decimal::from(12) * Decimal::from(10);

    Ok(AssetYield {
        asset_id,
        ticker: ticker.to_string(),
        asset_type,
        ltm_income: total_ltm_income,
        current_position_value,
        yield_percentage: yield_pct,
        consistency_score,
        months_with_income,
    })
}

// ============================================================================
// 2. TREND ANALYSIS
// ============================================================================

/// Get monthly income series for trend analysis
pub fn get_monthly_income_series(
    conn: &rusqlite::Connection,
    months_back: i32,
    ticker: Option<&str>,
) -> Result<MonthlyIncomeSeries> {
    let today = Local::now().date_naive();
    let start_date = today - Duration::days(months_back as i64 * 30);

    let events = db::get_income_events_with_assets(conn, Some(start_date), Some(today), ticker)?;

    // Build monthly aggregations
    let mut monthly: BTreeMap<(i32, u32), Decimal> = BTreeMap::new();

    for (event, _asset) in &events {
        let year = event.event_date.year();
        let month = event.event_date.month();
        monthly
            .entry((year, month))
            .and_modify(|total| *total += event.total_amount)
            .or_insert(event.total_amount);
    }

    let mut months = Vec::new();
    let mut amounts = Vec::new();

    for ((year, month), amount) in monthly {
        if let Some(date) = NaiveDate::from_ymd_opt(year, month, 1) {
            months.push(date);
            amounts.push(amount);
        }
    }

    Ok(MonthlyIncomeSeries { months, amounts })
}

/// Analyze income trends using linear regression
pub fn analyze_income_trends(
    conn: &rusqlite::Connection,
    months_back: i32,
    ticker: Option<&str>,
) -> Result<TrendAnalysis> {
    let series = get_monthly_income_series(conn, months_back, ticker)?;

    if series.amounts.len() < 2 {
        return Ok(TrendAnalysis {
            slope: Decimal::ZERO,
            trend_direction: TrendDirection::Stable,
            yoy_growth_percentage: Decimal::ZERO,
            volatility: Decimal::ZERO,
        });
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

    // Determine trend direction
    let trend_direction = if slope > Decimal::from_str_exact("0.01").unwrap_or(Decimal::ZERO) {
        TrendDirection::Growing
    } else if slope < Decimal::from_str_exact("-0.01").unwrap_or(Decimal::ZERO) {
        TrendDirection::Declining
    } else {
        TrendDirection::Stable
    };

    // YoY growth: compare amounts from 12 months ago vs 0-11 months ago
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

    Ok(TrendAnalysis {
        slope,
        trend_direction,
        yoy_growth_percentage: yoy_growth,
        volatility,
    })
}

// ============================================================================
// 3. BASELINE VS EXCEPTIONAL DETECTION
// ============================================================================

/// Detect if income event is baseline (recurring) or exceptional
pub fn categorize_income_event(
    event: &IncomeEvent,
    all_events: &[(IncomeEvent, db::Asset)],
    ticker: &str,
) -> IncomeCategory {
    // Rule 1: Amortization is almost always exceptional
    if event.event_type == IncomeEventType::Amortization {
        return IncomeCategory {
            is_baseline: false,
            reason: "Amortization events are typically one-time capital returns".to_string(),
        };
    }

    // Get all events for this ticker in the last 12 months (before this event)
    let twelve_months_ago = event.event_date - Duration::days(365);
    let preceding_events: Vec<_> = all_events
        .iter()
        .filter(|(e, a)| {
            a.ticker == ticker
                && e.event_type == event.event_type
                && e.event_date < event.event_date
                && e.event_date >= twelve_months_ago
        })
        .collect();

    // Rule 2: First-ever payment of this type is exceptional (or baseline if really first)
    if preceding_events.is_empty() {
        return IncomeCategory {
            is_baseline: false,
            reason: "First payment of this type from this asset".to_string(),
        };
    }

    // Rule 3: Check if amount is 2+ standard deviations above mean
    let preceding_amounts: Vec<Decimal> = preceding_events
        .iter()
        .map(|(e, _)| e.total_amount)
        .collect();

    if preceding_amounts.len() >= 3 {
        let mean =
            preceding_amounts.iter().sum::<Decimal>() / Decimal::from(preceding_amounts.len());
        let variance = preceding_amounts
            .iter()
            .map(|x| (x - mean) * (x - mean))
            .sum::<Decimal>()
            / Decimal::from(preceding_amounts.len());

        // Only check for outliers if there's variance (non-constant payments)
        if !variance.is_zero() {
            let mut stddev = variance;
            for _ in 0..5 {
                stddev = (stddev + variance / stddev) / Decimal::from(2);
            }

            if event.total_amount > mean + (Decimal::from(2) * stddev) {
                return IncomeCategory {
                    is_baseline: false,
                    reason: format!(
                        "Amount significantly higher than historical average (${:.2} vs avg ${:.2})",
                        event.total_amount, mean
                    ),
                };
            }
        }
    }

    // Otherwise: baseline
    IncomeCategory {
        is_baseline: true,
        reason: "Regular recurring income".to_string(),
    }
}

// ============================================================================
// 4. FORECASTING
// ============================================================================

/// Generate income forecast for an asset using multiple methods
pub fn forecast_income_for_asset(
    conn: &rusqlite::Connection,
    asset_id: i64,
    ticker: &str,
    conservative: bool,
) -> Result<IncomeForecast> {
    let today = Local::now().date_naive();
    let one_year_ago = today - Duration::days(365);
    let two_years_ago = today - Duration::days(730);

    let events =
        db::get_income_events_with_assets(conn, Some(two_years_ago), Some(today), Some(ticker))?;

    // Filter to last 12 months only for baseline
    let ltm_events: Vec<_> = events
        .iter()
        .filter(|(e, _)| e.event_date >= one_year_ago)
        .collect();

    if ltm_events.is_empty() {
        return Ok(IncomeForecast {
            asset_id,
            ticker: ticker.to_string(),
            expected_annual_income: Decimal::ZERO,
            lower_bound: Decimal::ZERO,
            upper_bound: Decimal::ZERO,
            confidence: ConfidenceLevel::Low,
            months_of_history: 0,
            baseline_avg_monthly: Decimal::ZERO,
        });
    }

    let ltm_total: Decimal = ltm_events.iter().map(|(e, _)| e.total_amount).sum();
    let ltm_monthly_avg = ltm_total / Decimal::from(12);

    // Count months with income
    let mut income_months: std::collections::HashSet<(i32, u32)> = std::collections::HashSet::new();
    for (event, _) in &ltm_events {
        income_months.insert((event.event_date.year(), event.event_date.month()));
    }

    let months_of_history = income_months.len();

    // Determine confidence - skip full CV calculation for now
    // Will be fixed in Phase 2 with better handling
    let confidence = if months_of_history >= 12 {
        ConfidenceLevel::High
    } else if months_of_history >= 6 {
        ConfidenceLevel::Medium
    } else {
        ConfidenceLevel::Low
    };

    // Apply trend adjustment if enough history
    let mut expected = ltm_total;
    let series = get_monthly_income_series(conn, 24, Some(ticker))?;
    if series.amounts.len() >= 24 {
        let trend = analyze_income_trends(conn, 24, Some(ticker))?;
        if trend.trend_direction == TrendDirection::Growing
            && !trend.yoy_growth_percentage.is_zero()
        {
            let growth_factor = Decimal::ONE + (trend.yoy_growth_percentage / Decimal::from(100));
            expected *= growth_factor;
        }
    }

    // Conservative mode: use lower bound
    if conservative {
        expected = ltm_total * Decimal::from_str_exact("0.85").unwrap_or(Decimal::ONE);
    }

    // Confidence intervals: ±15% for high, ±25% for medium, ±35% for low
    let margin = match confidence {
        ConfidenceLevel::High => Decimal::from_str_exact("0.15").unwrap_or(Decimal::ZERO),
        ConfidenceLevel::Medium => Decimal::from_str_exact("0.25").unwrap_or(Decimal::ZERO),
        ConfidenceLevel::Low => Decimal::from_str_exact("0.35").unwrap_or(Decimal::ZERO),
    };

    let lower_bound = expected * (Decimal::ONE - margin);
    let upper_bound = expected * (Decimal::ONE + margin);

    Ok(IncomeForecast {
        asset_id,
        ticker: ticker.to_string(),
        expected_annual_income: expected,
        lower_bound,
        upper_bound,
        confidence,
        months_of_history,
        baseline_avg_monthly: ltm_monthly_avg,
    })
}

// ============================================================================
// 5. CALENDAR & PAYMENT PREDICTION
// ============================================================================

/// Predict next payment date based on historical patterns
pub fn predict_payment_dates(
    conn: &rusqlite::Connection,
    ticker: &str,
    num_predictions: usize,
) -> Result<Vec<(NaiveDate, Decimal, ConfidenceLevel)>> {
    let series = get_monthly_income_series(conn, 24, Some(ticker))?;

    if series.months.is_empty() {
        return Ok(Vec::new());
    }

    // Get all events for this ticker in the historical window (24 months back)
    let today = Local::now().date_naive();
    let two_years_ago = today - Duration::days(730);
    let all_events = db::get_income_events_with_assets(
        conn,
        Some(two_years_ago),
        Some(today),
        Some(ticker),
    )?;

    // Determine typical day-of-month from historical data
    let mut day_of_months = Vec::new();
    for (event, _) in all_events {
        day_of_months.push(event.event_date.day());
    }

    if day_of_months.is_empty() {
        return Ok(Vec::new());
    }

    // Find most common day
    let mut day_freq: HashMap<u32, usize> = HashMap::new();
    for day in day_of_months {
        *day_freq.entry(day).or_insert(0) += 1;
    }

    let typical_day = day_freq
        .iter()
        .max_by_key(|(_, count)| *count)
        .map(|(day, _)| *day)
        .unwrap_or(15); // Default to mid-month

    // Generate predictions
    let today = Local::now().date_naive();
    let mut predictions = Vec::new();

    for i in 1..=num_predictions as i32 {
        let target_month = today + Duration::days(30 * i as i64);
        if let Some(predicted_date) = NaiveDate::from_ymd_opt(
            target_month.year(),
            target_month.month(),
            std::cmp::min(typical_day, 28), // Cap at 28 to avoid month overflow
        ) {
            let avg_amount =
                (series.amounts.iter().sum::<Decimal>()) / Decimal::from(series.amounts.len());
            let confidence = if series.amounts.len() >= 12 {
                ConfidenceLevel::High
            } else if series.amounts.len() >= 6 {
                ConfidenceLevel::Medium
            } else {
                ConfidenceLevel::Low
            };
            predictions.push((predicted_date, avg_amount, confidence));
        }
    }

    Ok(predictions)
}

// ============================================================================
// 6. ANOMALY DETECTION
// ============================================================================

/// Detect anomalies in income data
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
pub fn detect_anomalies(conn: &rusqlite::Connection) -> Result<Vec<IncomeAnomaly>> {
    let today = Local::now().date_naive();
    let this_month_start = NaiveDate::from_ymd_opt(today.year(), today.month(), 1).unwrap();
    let last_month_start = if today.month() == 1 {
        NaiveDate::from_ymd_opt(today.year() - 1, 12, 1).unwrap()
    } else {
        NaiveDate::from_ymd_opt(today.year(), today.month() - 1, 1).unwrap()
    };

    let events =
        db::get_income_events_with_assets(conn, Some(this_month_start), Some(today), None)?;

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

    for (_, asset) in &last_month_events {
        if !tickers_this_month.contains(&asset.ticker) && today.day() > 15 {
            // More than halfway through the month
            anomalies.push(IncomeAnomaly {
                anomaly_type: AnomalyType::MissedPayment,
                ticker: asset.ticker.clone(),
                description: format!(
                    "{}: Expected payment not yet received (typically pays in this month)",
                    asset.ticker
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
