/// Formatters for income commands
use anyhow::Result;
use rust_decimal::Decimal;

use crate::db;
use crate::formatters::OutputOptions;
use crate::output::{
    ColumnDef, KeyValueRow, OutputBlock, OutputDocument, Row, TableOptions, TableStyle, Value,
    ValueKind,
};

//
// 1. INCOME SHOW (grouped by asset type and ticker)
//
#[derive(Debug, Clone)]
pub struct AssetIncome {
    pub ticker: String,
    pub asset_type: db::AssetType,
    pub dividends: Decimal,
    pub jcp: Decimal,
    pub amortization: Decimal,
}

pub fn format_income_show_json(assets: &[AssetIncome], options: OutputOptions) -> Result<String> {
    let rows = assets
        .iter()
        .map(|asset| {
            let total = asset.dividends + asset.jcp + asset.amortization;
            Row {
                cells: vec![
                    Value::Text(asset.ticker.clone()),
                    Value::Text(asset.asset_type.as_str().to_string()),
                    Value::Currency(asset.dividends),
                    Value::Currency(asset.jcp),
                    Value::Currency(asset.amortization),
                    Value::Currency(total),
                ],
            }
        })
        .collect::<Vec<_>>();

    let document = OutputDocument {
        title: None,
        blocks: vec![OutputBlock::Table {
            title: Some("Income Summary".to_string()),
            columns: vec![
                ColumnDef::new("ticker", "Ticker", ValueKind::Text),
                ColumnDef::new("asset_type", "Asset Type", ValueKind::Text),
                ColumnDef::new("dividends", "Dividends", ValueKind::Currency),
                ColumnDef::new("jcp", "JCP", ValueKind::Currency),
                ColumnDef::new("amortization", "Amort", ValueKind::Currency),
                ColumnDef::new("total", "Total", ValueKind::Currency),
            ],
            rows,
            footer: None,
            options: TableOptions {
                style: TableStyle::Rounded,
            },
        }],
        meta: Default::default(),
    };

    crate::formatters::render_document(&document, options)
}

pub fn format_income_show_table(
    assets_by_type: &[(db::AssetType, Vec<AssetIncome>)],
    year: i32,
    options: OutputOptions,
) -> Result<String> {
    let mut blocks = Vec::new();
    blocks.push(OutputBlock::Header {
        level: 1,
        text: format!("Income Summary - {}", year),
    });

    let mut grand_total = Decimal::ZERO;
    for (asset_type, assets) in assets_by_type {
        if assets.is_empty() {
            continue;
        }

        let rows = assets
            .iter()
            .map(|asset| {
                let total = asset.dividends + asset.jcp + asset.amortization;
                Row {
                    cells: vec![
                        Value::Text(asset.ticker.clone()),
                        Value::Currency(asset.dividends),
                        Value::Currency(asset.jcp),
                        Value::Currency(asset.amortization),
                        Value::Currency(total),
                    ],
                }
            })
            .collect::<Vec<_>>();

        let type_total: Decimal = assets
            .iter()
            .map(|a| a.dividends + a.jcp + a.amortization)
            .sum();
        grand_total += type_total;

        blocks.push(OutputBlock::Section {
            title: Some(format!(
                "{} ({})",
                asset_type.as_str().to_uppercase(),
                crate::utils::format_currency(type_total, &options)
            )),
            blocks: vec![OutputBlock::Table {
                title: None,
                columns: vec![
                    ColumnDef::new("ticker", "Ticker", ValueKind::Text),
                    ColumnDef::new("dividends", "Dividends", ValueKind::Currency),
                    ColumnDef::new("jcp", "JCP", ValueKind::Currency),
                    ColumnDef::new("amortization", "Amort", ValueKind::Currency),
                    ColumnDef::new("total", "Total", ValueKind::Currency),
                ],
                rows,
                footer: None,
                options: TableOptions {
                    style: TableStyle::Rounded,
                },
            }],
        });
    }

    blocks.push(OutputBlock::KeyValue {
        title: Some("Grand Total".to_string()),
        rows: vec![KeyValueRow {
            label: "Total".to_string(),
            value: Value::Currency(grand_total),
        }],
    });

    let document = OutputDocument {
        title: None,
        blocks,
        meta: Default::default(),
    };

    crate::formatters::render_document(&document, options)
}

//
// 2. INCOME DETAIL (individual events)
//
pub fn format_income_detail_json(
    events: &[(db::IncomeEvent, db::Asset)],
    options: OutputOptions,
) -> Result<String> {
    let rows = events
        .iter()
        .map(|(event, asset)| {
            let net = event.total_amount - event.withholding_tax;
            Row {
                cells: vec![
                    Value::Date(event.event_date),
                    Value::Text(asset.ticker.clone()),
                    Value::Text(asset.asset_type.as_str().to_string()),
                    Value::Text(event.event_type.as_str().to_string()),
                    Value::Currency(event.total_amount),
                    Value::Currency(event.withholding_tax),
                    Value::Currency(net),
                    event
                        .notes
                        .as_ref()
                        .map(|n| Value::Text(n.clone()))
                        .unwrap_or(Value::Null),
                ],
            }
        })
        .collect::<Vec<_>>();

    let document = OutputDocument {
        title: None,
        blocks: vec![OutputBlock::Table {
            title: Some("Income Detail".to_string()),
            columns: vec![
                ColumnDef::new("date", "Date", ValueKind::Date),
                ColumnDef::new("ticker", "Ticker", ValueKind::Text),
                ColumnDef::new("asset_type", "Asset Type", ValueKind::Text),
                ColumnDef::new("type", "Type", ValueKind::Text),
                ColumnDef::new("amount", "Amount", ValueKind::Currency),
                ColumnDef::new("tax", "Tax", ValueKind::Currency),
                ColumnDef::new("net", "Net", ValueKind::Currency),
                ColumnDef::new("notes", "Notes", ValueKind::Text),
            ],
            rows,
            footer: None,
            options: TableOptions {
                style: TableStyle::Rounded,
            },
        }],
        meta: Default::default(),
    };

    crate::formatters::render_document(&document, options)
}

pub fn format_income_detail_table(
    events: &[(db::IncomeEvent, db::Asset)],
    year: i32,
    options: OutputOptions,
) -> Result<String> {
    let rows = events
        .iter()
        .map(|(event, asset)| {
            let net = event.total_amount - event.withholding_tax;
            Row {
                cells: vec![
                    Value::Date(event.event_date),
                    Value::Text(asset.ticker.clone()),
                    Value::Text(asset.asset_type.as_str().to_string()),
                    Value::Text(event.event_type.as_str().to_string()),
                    Value::Currency(event.total_amount),
                    Value::Currency(event.withholding_tax),
                    Value::Currency(net),
                    event
                        .notes
                        .as_ref()
                        .map(|n| Value::Text(n.clone()))
                        .unwrap_or(Value::Null),
                ],
            }
        })
        .collect::<Vec<_>>();

    let total_amount: Decimal = events.iter().map(|(e, _)| e.total_amount).sum();
    let total_tax: Decimal = events.iter().map(|(e, _)| e.withholding_tax).sum();
    let total_net = total_amount - total_tax;

    let document = OutputDocument {
        title: None,
        blocks: vec![
            OutputBlock::Header {
                level: 1,
                text: format!("Income Detail - {}", year),
            },
            OutputBlock::Table {
                title: None,
                columns: vec![
                    ColumnDef::new("date", "Date", ValueKind::Date),
                    ColumnDef::new("ticker", "Ticker", ValueKind::Text),
                    ColumnDef::new("asset_type", "Asset Type", ValueKind::Text),
                    ColumnDef::new("type", "Type", ValueKind::Text),
                    ColumnDef::new("amount", "Amount", ValueKind::Currency),
                    ColumnDef::new("tax", "Tax", ValueKind::Currency),
                    ColumnDef::new("net", "Net", ValueKind::Currency),
                    ColumnDef::new("notes", "Notes", ValueKind::Text),
                ],
                rows,
                footer: None,
                options: TableOptions {
                    style: TableStyle::Rounded,
                },
            },
            OutputBlock::KeyValue {
                title: Some("Summary".to_string()),
                rows: vec![
                    KeyValueRow {
                        label: "Total Amount".to_string(),
                        value: Value::Currency(total_amount),
                    },
                    KeyValueRow {
                        label: "Tax Withheld".to_string(),
                        value: Value::Currency(total_tax),
                    },
                    KeyValueRow {
                        label: "Net Amount".to_string(),
                        value: Value::Currency(total_net),
                    },
                ],
            },
        ],
        meta: Default::default(),
    };

    crate::formatters::render_document(&document, options)
}

//
// 4. INCOME SUMMARY (monthly/yearly breakdown)
//

#[derive(Debug, Clone)]
pub struct IncomeTotals {
    pub label: String,
    pub dividends: Decimal,
    pub jcp: Decimal,
    pub amortization: Decimal,
}

#[derive(Debug, Clone)]
pub struct IncomeSummaryStats {
    pub periods_with_income: usize,
    pub avg_per_period: Decimal,
}

pub fn format_income_summary_monthly(
    year: i32,
    monthly: &[IncomeTotals],
    totals_by_type: &[(db::AssetType, Decimal)],
    stats: IncomeSummaryStats,
    totals: IncomeTotals,
    options: OutputOptions,
) -> Result<String> {
    let document = build_income_summary_document(
        format!("Income Summary - {} (Monthly Breakdown)", year),
        "Month",
        monthly,
        totals_by_type,
        stats,
        totals,
    );
    crate::formatters::render_document(&document, options)
}

pub fn format_income_summary_yearly(
    yearly: &[IncomeTotals],
    totals_by_type: &[(db::AssetType, Decimal)],
    stats: IncomeSummaryStats,
    totals: IncomeTotals,
    options: OutputOptions,
) -> Result<String> {
    let document = build_income_summary_document(
        "Income Summary (Yearly Breakdown)".to_string(),
        "Year",
        yearly,
        totals_by_type,
        stats,
        totals,
    );
    crate::formatters::render_document(&document, options)
}

fn build_income_summary_document(
    title: String,
    period_label: &str,
    periods: &[IncomeTotals],
    totals_by_type: &[(db::AssetType, Decimal)],
    stats: IncomeSummaryStats,
    totals: IncomeTotals,
) -> OutputDocument {
    let rows = periods
        .iter()
        .map(|entry| {
            let total = entry.dividends + entry.jcp + entry.amortization;
            Row {
                cells: vec![
                    Value::Text(entry.label.clone()),
                    Value::Currency(entry.dividends),
                    Value::Currency(entry.jcp),
                    Value::Currency(entry.amortization),
                    Value::Currency(total),
                ],
            }
        })
        .collect::<Vec<_>>();

    let total_row = Row {
        cells: vec![
            Value::Text(totals.label.clone()),
            Value::Currency(totals.dividends),
            Value::Currency(totals.jcp),
            Value::Currency(totals.amortization),
            Value::Currency(totals.dividends + totals.jcp + totals.amortization),
        ],
    };

    let by_type_rows = totals_by_type
        .iter()
        .map(|(asset_type, total)| KeyValueRow {
            label: asset_type.as_str().to_uppercase(),
            value: Value::Currency(*total),
        })
        .collect::<Vec<_>>();

    OutputDocument {
        title: None,
        blocks: vec![
            OutputBlock::Header {
                level: 1,
                text: title,
            },
            OutputBlock::Table {
                title: None,
                columns: vec![
                    ColumnDef::new("period", period_label, ValueKind::Text),
                    ColumnDef::new("dividends", "Dividends", ValueKind::Currency),
                    ColumnDef::new("jcp", "JCP", ValueKind::Currency),
                    ColumnDef::new("amortization", "Amortization", ValueKind::Currency),
                    ColumnDef::new("total", "Total", ValueKind::Currency),
                ],
                rows,
                footer: Some(total_row),
                options: TableOptions {
                    style: TableStyle::Rounded,
                },
            },
            OutputBlock::KeyValue {
                title: Some("Statistics".to_string()),
                rows: vec![
                    KeyValueRow {
                        label: format!("{} with income", period_label),
                        value: Value::Text(stats.periods_with_income.to_string()),
                    },
                    KeyValueRow {
                        label: "Average per period".to_string(),
                        value: Value::Currency(stats.avg_per_period),
                    },
                ],
            },
            OutputBlock::KeyValue {
                title: Some("Subtotals by Asset Type".to_string()),
                rows: by_type_rows,
            },
        ],
        meta: Default::default(),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn format_income_summary_monthly_with_categories(
    year: i32,
    monthly: &[IncomeTotals],
    totals_by_type: &[(db::AssetType, Decimal)],
    stats: IncomeSummaryStats,
    totals: IncomeTotals,
    baseline_total: Decimal,
    exceptional_total: Decimal,
    tax_aware: bool,
    options: OutputOptions,
) -> Result<String> {
    let document = build_income_summary_document_with_categories(
        format!(
            "Income Summary - {} (Monthly Breakdown{})",
            year,
            if tax_aware { ", Net After Tax" } else { "" }
        ),
        "Month",
        monthly,
        totals_by_type,
        stats,
        totals,
        baseline_total,
        exceptional_total,
    );
    crate::formatters::render_document(&document, options)
}

#[allow(clippy::too_many_arguments)]
pub fn format_income_summary_yearly_with_categories(
    yearly: &[IncomeTotals],
    totals_by_type: &[(db::AssetType, Decimal)],
    stats: IncomeSummaryStats,
    totals: IncomeTotals,
    baseline_total: Decimal,
    exceptional_total: Decimal,
    tax_aware: bool,
    options: OutputOptions,
) -> Result<String> {
    let document = build_income_summary_document_with_categories(
        format!(
            "Income Summary (Yearly Breakdown{})",
            if tax_aware { ", Net After Tax" } else { "" }
        ),
        "Year",
        yearly,
        totals_by_type,
        stats,
        totals,
        baseline_total,
        exceptional_total,
    );
    crate::formatters::render_document(&document, options)
}

#[allow(clippy::too_many_arguments)]
fn build_income_summary_document_with_categories(
    title: String,
    period_label: &str,
    periods: &[IncomeTotals],
    totals_by_type: &[(db::AssetType, Decimal)],
    stats: IncomeSummaryStats,
    totals: IncomeTotals,
    baseline_total: Decimal,
    exceptional_total: Decimal,
) -> OutputDocument {
    let grand_total = baseline_total + exceptional_total;
    let baseline_pct = if !grand_total.is_zero() {
        (baseline_total / grand_total * Decimal::from(100)).round_dp(1)
    } else {
        Decimal::ZERO
    };
    let exceptional_pct = if !grand_total.is_zero() {
        (exceptional_total / grand_total * Decimal::from(100)).round_dp(1)
    } else {
        Decimal::ZERO
    };

    let rows = periods
        .iter()
        .map(|entry| {
            let total = entry.dividends + entry.jcp + entry.amortization;
            Row {
                cells: vec![
                    Value::Text(entry.label.clone()),
                    Value::Currency(entry.dividends),
                    Value::Currency(entry.jcp),
                    Value::Currency(entry.amortization),
                    Value::Currency(total),
                ],
            }
        })
        .collect::<Vec<_>>();

    let total_row = Row {
        cells: vec![
            Value::Text(totals.label.clone()),
            Value::Currency(totals.dividends),
            Value::Currency(totals.jcp),
            Value::Currency(totals.amortization),
            Value::Currency(totals.dividends + totals.jcp + totals.amortization),
        ],
    };

    let by_type_rows = totals_by_type
        .iter()
        .map(|(asset_type, total)| KeyValueRow {
            label: asset_type.as_str().to_uppercase(),
            value: Value::Currency(*total),
        })
        .collect::<Vec<_>>();

    OutputDocument {
        title: None,
        blocks: vec![
            OutputBlock::Header {
                level: 1,
                text: title,
            },
            OutputBlock::KeyValue {
                title: Some("Income Categorization".to_string()),
                rows: vec![
                    KeyValueRow {
                        label: "Baseline Income (Recurring)".to_string(),
                        value: Value::Currency(baseline_total),
                    },
                    KeyValueRow {
                        label: "Baseline Income (%)".to_string(),
                        value: Value::Percent(baseline_pct),
                    },
                    KeyValueRow {
                        label: "Exceptional Income".to_string(),
                        value: Value::Currency(exceptional_total),
                    },
                    KeyValueRow {
                        label: "Exceptional Income (%)".to_string(),
                        value: Value::Percent(exceptional_pct),
                    },
                    KeyValueRow {
                        label: "Monthly Baseline Avg".to_string(),
                        value: Value::Currency(if stats.periods_with_income > 0 {
                            baseline_total / Decimal::from(stats.periods_with_income)
                        } else {
                            Decimal::ZERO
                        }),
                    },
                ],
            },
            OutputBlock::Table {
                title: None,
                columns: vec![
                    ColumnDef::new("period", period_label, ValueKind::Text),
                    ColumnDef::new("dividends", "Dividends", ValueKind::Currency),
                    ColumnDef::new("jcp", "JCP", ValueKind::Currency),
                    ColumnDef::new("amortization", "Amortization", ValueKind::Currency),
                    ColumnDef::new("total", "Total", ValueKind::Currency),
                ],
                rows,
                footer: Some(total_row),
                options: TableOptions {
                    style: TableStyle::Rounded,
                },
            },
            OutputBlock::KeyValue {
                title: Some("Statistics".to_string()),
                rows: vec![
                    KeyValueRow {
                        label: format!("{} with income", period_label),
                        value: Value::Text(stats.periods_with_income.to_string()),
                    },
                    KeyValueRow {
                        label: "Average per period".to_string(),
                        value: Value::Currency(stats.avg_per_period),
                    },
                ],
            },
            OutputBlock::KeyValue {
                title: Some("Subtotals by Asset Type".to_string()),
                rows: by_type_rows,
            },
        ],
        meta: Default::default(),
    }
}

//
// 3. INCOME ADD CONFIRMATION
//

/// Format income add confirmation
pub fn format_income_add(
    event_id: i64,
    ticker: &str,
    event_date: chrono::NaiveDate,
    total_amount: Decimal,
    options: OutputOptions,
) -> Result<String> {
    let document = build_income_add_document(event_id, ticker, event_date, total_amount);
    crate::formatters::render_document(&document, options)
}

// Internal implementations below

fn build_income_add_document(
    event_id: i64,
    ticker: &str,
    event_date: chrono::NaiveDate,
    total_amount: Decimal,
) -> OutputDocument {
    OutputDocument {
        title: None,
        blocks: vec![
            OutputBlock::Header {
                level: 1,
                text: "Income event added".to_string(),
            },
            OutputBlock::KeyValue {
                title: None,
                rows: vec![
                    KeyValueRow {
                        label: "ID".to_string(),
                        value: Value::Text(event_id.to_string()),
                    },
                    KeyValueRow {
                        label: "Ticker".to_string(),
                        value: Value::Text(ticker.to_string()),
                    },
                    KeyValueRow {
                        label: "Date".to_string(),
                        value: Value::Date(event_date),
                    },
                    KeyValueRow {
                        label: "Total Amount".to_string(),
                        value: Value::Currency(total_amount),
                    },
                ],
            },
        ],
        meta: Default::default(),
    }
}

// ============================================================================
// NEW: Income Analytics Formatters
// ============================================================================

/// Format yield report
pub fn format_yield_report(
    ltm_yield: &crate::reports::income_analytics::LtmYieldResult,
    options: OutputOptions,
) -> Result<String> {
    let document = OutputDocument {
        title: None,
        blocks: vec![
            OutputBlock::Header {
                level: 1,
                text: "Income Yield Analysis (Last 12 Months)".to_string(),
            },
            OutputBlock::KeyValue {
                title: Some("Portfolio Overview".to_string()),
                rows: vec![
                    KeyValueRow {
                        label: "Total LTM Income".to_string(),
                        value: Value::Currency(ltm_yield.total_ltm_income),
                    },
                    KeyValueRow {
                        label: "Current Portfolio Value".to_string(),
                        value: Value::Currency(ltm_yield.portfolio_value),
                    },
                    KeyValueRow {
                        label: "Portfolio Yield".to_string(),
                        value: Value::Text(format!("{:.2}%", ltm_yield.yield_percentage)),
                    },
                ],
            },
        ],
        meta: Default::default(),
    };

    crate::formatters::render_document(&document, options)
}

/// Format trends report with ASCII chart and analysis
pub fn format_trends_report(
    series: &crate::reports::income_analytics::MonthlyIncomeSeries,
    trend: &crate::reports::income_analytics::TrendAnalysis,
    _months: i32,
    options: OutputOptions,
) -> Result<String> {
    let mean = series.mean();
    let volatility_pct = trend.volatility * Decimal::from(100);

    let document = OutputDocument {
        title: None,
        blocks: vec![
            OutputBlock::Header {
                level: 1,
                text: "Income Trends Analysis".to_string(),
            },
            OutputBlock::KeyValue {
                title: Some("Summary".to_string()),
                rows: vec![
                    KeyValueRow {
                        label: "Trend Direction".to_string(),
                        value: Value::Text(trend.trend_direction.as_str().to_string()),
                    },
                    KeyValueRow {
                        label: "YoY Growth".to_string(),
                        value: Value::Percent(trend.yoy_growth_percentage),
                    },
                    KeyValueRow {
                        label: "Volatility (CV)".to_string(),
                        value: Value::Percent(volatility_pct),
                    },
                    KeyValueRow {
                        label: "Average Monthly Income".to_string(),
                        value: Value::Currency(mean),
                    },
                ],
            },
        ],
        meta: Default::default(),
    };

    crate::formatters::render_document(&document, options)
}

/// Format forecast report
pub fn format_forecast_report(
    forecasts: &[crate::reports::income_analytics::IncomeForecast],
    year: i32,
    conservative: bool,
    options: OutputOptions,
) -> Result<String> {
    let total_expected: Decimal = forecasts.iter().map(|f| f.expected_annual_income).sum();

    let rows = forecasts
        .iter()
        .map(|forecast| Row {
            cells: vec![
                Value::Text(forecast.ticker.clone()),
                Value::Currency(forecast.expected_annual_income),
                Value::Currency(forecast.lower_bound),
                Value::Currency(forecast.upper_bound),
                Value::Text(forecast.confidence.as_str().to_string()),
                Value::Text(forecast.months_of_history.to_string()),
            ],
        })
        .collect::<Vec<_>>();

    let mode_str = if conservative { " (Conservative)" } else { "" };

    let document = OutputDocument {
        title: None,
        blocks: vec![
            OutputBlock::Header {
                level: 1,
                text: format!("Income Forecast {}{}", year, mode_str),
            },
            OutputBlock::KeyValue {
                title: Some("Overall Forecast".to_string()),
                rows: vec![KeyValueRow {
                    label: "Expected Annual Income".to_string(),
                    value: Value::Currency(total_expected),
                }],
            },
            OutputBlock::Table {
                title: Some("By Asset".to_string()),
                columns: vec![
                    ColumnDef::new("ticker", "Ticker", ValueKind::Text),
                    ColumnDef::new("forecast", "Forecast", ValueKind::Currency),
                    ColumnDef::new("lower", "Lower Bound", ValueKind::Currency),
                    ColumnDef::new("upper", "Upper Bound", ValueKind::Currency),
                    ColumnDef::new("confidence", "Confidence", ValueKind::Text),
                    ColumnDef::new("history", "Months", ValueKind::Text),
                ],
                rows,
                footer: None,
                options: TableOptions {
                    style: TableStyle::Rounded,
                },
            },
        ],
        meta: Default::default(),
    };

    crate::formatters::render_document(&document, options)
}

/// Format calendar report with predicted payment dates
pub fn format_calendar_report(
    predictions: &[(
        String,
        chrono::NaiveDate,
        Decimal,
        crate::reports::income_analytics::ConfidenceLevel,
    )],
    options: OutputOptions,
) -> Result<String> {
    let rows = predictions
        .iter()
        .map(|(ticker, date, amount, confidence)| Row {
            cells: vec![
                Value::Text(ticker.clone()),
                Value::Date(*date),
                Value::Currency(*amount),
                Value::Text(confidence.as_str().to_string()),
            ],
        })
        .collect::<Vec<_>>();

    let document = OutputDocument {
        title: None,
        blocks: vec![
            OutputBlock::Header {
                level: 1,
                text: "Predicted Payment Calendar (Next 3 Months)".to_string(),
            },
            OutputBlock::Table {
                title: None,
                columns: vec![
                    ColumnDef::new("ticker", "Ticker", ValueKind::Text),
                    ColumnDef::new("date", "Predicted Date", ValueKind::Date),
                    ColumnDef::new("amount", "Est. Amount", ValueKind::Currency),
                    ColumnDef::new("confidence", "Confidence", ValueKind::Text),
                ],
                rows,
                footer: None,
                options: TableOptions {
                    style: TableStyle::Rounded,
                },
            },
        ],
        meta: Default::default(),
    };

    crate::formatters::render_document(&document, options)
}

/// Format alerts report for anomalies
pub fn format_alerts_report(
    anomalies: &[crate::reports::income_analytics::IncomeAnomaly],
    options: OutputOptions,
) -> Result<String> {
    let rows = anomalies
        .iter()
        .map(|anomaly| {
            let icon = match anomaly.anomaly_type {
                crate::reports::income_analytics::AnomalyType::MissedPayment => "⚠",
                crate::reports::income_analytics::AnomalyType::UnusualAmount => "!",
                crate::reports::income_analytics::AnomalyType::IncomeDrop => "↓",
            };
            Row {
                cells: vec![
                    Value::Text(format!("{} {}", icon, anomaly.ticker)),
                    Value::Text(anomaly.description.clone()),
                ],
            }
        })
        .collect::<Vec<_>>();

    let document = OutputDocument {
        title: None,
        blocks: vec![
            OutputBlock::Header {
                level: 1,
                text: "Income Alerts & Anomalies".to_string(),
            },
            OutputBlock::Table {
                title: None,
                columns: vec![
                    ColumnDef::new("asset", "Asset", ValueKind::Text),
                    ColumnDef::new("description", "Description", ValueKind::Text),
                ],
                rows,
                footer: None,
                options: TableOptions {
                    style: TableStyle::Rounded,
                },
            },
        ],
        meta: Default::default(),
    };

    crate::formatters::render_document(&document, options)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;
    use std::str::FromStr;

    #[test]
    fn test_income_add_json_decimals_as_strings() {
        let json_str = format_income_add(
            123,
            "PETR4",
            chrono::NaiveDate::from_ymd_opt(2024, 1, 15).unwrap(),
            Decimal::from_str("100.50").unwrap(),
            OutputOptions::from_flags(true, false),
        )
        .unwrap();
        let value: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        let blocks = value["blocks"].as_array().expect("blocks array missing");
        let mut values = Vec::new();
        for block in blocks {
            if block.get("type").and_then(|t| t.as_str()) == Some("key_value") {
                if let Some(rows) = block.get("rows").and_then(|r| r.as_array()) {
                    for row in rows {
                        if let Some(val) = row.get("value") {
                            if let Some(raw) = val.get("value") {
                                if let Some(text) = raw.as_str() {
                                    values.push(text.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }

        assert!(values.contains(&"123".to_string()));
        assert!(values.contains(&"PETR4".to_string()));
        assert!(values.contains(&"2024-01-15".to_string()));
        assert!(values.contains(&"100.50".to_string()));
    }

    #[test]
    fn test_income_show_json_decimals_as_strings() {
        let assets = vec![AssetIncome {
            ticker: "PETR4".to_string(),
            asset_type: db::AssetType::Stock,
            dividends: Decimal::from_str("50.25").unwrap(),
            jcp: Decimal::from_str("25.50").unwrap(),
            amortization: Decimal::ZERO,
        }];

        let json_str =
            format_income_show_json(&assets, OutputOptions::from_flags(true, false)).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        let blocks = value["blocks"].as_array().expect("blocks array missing");
        let mut has_currency = false;
        for block in blocks {
            if block.get("type").and_then(|t| t.as_str()) == Some("table") {
                if let Some(rows) = block.get("rows").and_then(|r| r.as_array()) {
                    for row in rows {
                        if let Some(cells) = row.get("cells").and_then(|c| c.as_array()) {
                            for cell in cells {
                                if cell.get("kind").and_then(|k| k.as_str()) == Some("currency") {
                                    if let Some(raw) = cell.get("value") {
                                        assert!(raw.is_string());
                                        has_currency = true;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        assert!(has_currency, "expected currency fields in JSON");
    }
}
