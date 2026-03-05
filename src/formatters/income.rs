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
    pub cost_basis: Option<Decimal>,
}

pub fn format_income_show_json(assets: &[AssetIncome], options: OutputOptions) -> Result<String> {
    let has_yield = assets.iter().any(|a| a.cost_basis.is_some());

    let rows = assets
        .iter()
        .map(|asset| {
            let total = asset.dividends + asset.jcp + asset.amortization;
            let mut cells = vec![
                Value::Text(asset.ticker.clone()),
                Value::Text(asset.asset_type.as_str().to_string()),
                Value::Currency(asset.dividends),
                Value::Currency(asset.jcp),
                Value::Currency(asset.amortization),
                Value::Currency(total),
            ];
            if has_yield {
                cells.push(yield_value(total, asset.cost_basis));
            }
            Row { cells }
        })
        .collect::<Vec<_>>();

    let mut columns = vec![
        ColumnDef::new("ticker", "Ticker", ValueKind::Text),
        ColumnDef::new("asset_type", "Asset Type", ValueKind::Text),
        ColumnDef::new("dividends", "Dividends", ValueKind::Currency),
        ColumnDef::new("jcp", "JCP", ValueKind::Currency),
        ColumnDef::new("amortization", "Amort", ValueKind::Currency),
        ColumnDef::new("total", "Total", ValueKind::Currency),
    ];
    if has_yield {
        columns.push(ColumnDef::new("yield_pct", "Yield%", ValueKind::Percent));
    }

    let document = OutputDocument {
        title: None,
        blocks: vec![OutputBlock::Table {
            title: Some("Income Summary".to_string()),
            columns,
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
    let has_yield = assets_by_type
        .iter()
        .flat_map(|(_, assets)| assets)
        .any(|a| a.cost_basis.is_some());

    let mut blocks = Vec::new();
    blocks.push(OutputBlock::Header {
        level: 1,
        text: format!("Income Summary - {}", year),
    });

    let mut grand_total = Decimal::ZERO;
    let mut grand_cost_basis = Decimal::ZERO;
    for (asset_type, assets) in assets_by_type {
        if assets.is_empty() {
            continue;
        }

        let rows = assets
            .iter()
            .map(|asset| {
                let total = asset.dividends + asset.jcp + asset.amortization;
                let mut cells = vec![
                    Value::Text(asset.ticker.clone()),
                    Value::Currency(asset.dividends),
                    Value::Currency(asset.jcp),
                    Value::Currency(asset.amortization),
                    Value::Currency(total),
                ];
                if has_yield {
                    cells.push(yield_value(total, asset.cost_basis));
                }
                Row { cells }
            })
            .collect::<Vec<_>>();

        let type_total: Decimal = assets
            .iter()
            .map(|a| a.dividends + a.jcp + a.amortization)
            .sum();
        let type_cost: Decimal = assets.iter().filter_map(|a| a.cost_basis).sum();
        grand_total += type_total;
        grand_cost_basis += type_cost;

        let mut columns = vec![
            ColumnDef::new("ticker", "Ticker", ValueKind::Text),
            ColumnDef::new("dividends", "Dividends", ValueKind::Currency),
            ColumnDef::new("jcp", "JCP", ValueKind::Currency),
            ColumnDef::new("amortization", "Amort", ValueKind::Currency),
            ColumnDef::new("total", "Total", ValueKind::Currency),
        ];
        if has_yield {
            columns.push(ColumnDef::new("yield_pct", "Yield%", ValueKind::Percent));
        }

        blocks.push(OutputBlock::Section {
            title: Some(format!(
                "{} ({})",
                asset_type.as_str().to_uppercase(),
                crate::utils::format_currency(type_total, &options)
            )),
            blocks: vec![OutputBlock::Table {
                title: None,
                columns,
                rows,
                footer: None,
                options: TableOptions {
                    style: TableStyle::Rounded,
                },
            }],
        });
    }

    let mut total_rows = vec![KeyValueRow {
        label: "Total".to_string(),
        value: Value::Currency(grand_total),
    }];
    if has_yield && grand_cost_basis > Decimal::ZERO {
        let yield_pct = (grand_total / grand_cost_basis) * Decimal::from(100);
        total_rows.push(KeyValueRow {
            label: "Yield on Cost".to_string(),
            value: Value::Percent(yield_pct),
        });
    }

    blocks.push(OutputBlock::KeyValue {
        title: Some("Grand Total".to_string()),
        rows: total_rows,
    });

    let document = OutputDocument {
        title: None,
        blocks,
        meta: Default::default(),
    };

    crate::formatters::render_document(&document, options)
}

fn yield_value(income: Decimal, cost_basis: Option<Decimal>) -> Value {
    match cost_basis {
        Some(cost) if cost > Decimal::ZERO => Value::Percent((income / cost) * Decimal::from(100)),
        _ => Value::Null,
    }
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

//
// 5. INCOME SUMMARY WITH CATEGORIES (--categorize / --tax-aware)
//

#[allow(clippy::too_many_arguments)]
pub fn format_income_summary_monthly_with_categories(
    year: i32,
    monthly: &[IncomeTotals],
    totals_by_type: &[(db::AssetType, Decimal)],
    stats: IncomeSummaryStats,
    totals: IncomeTotals,
    baseline_total: Decimal,
    exceptional_total: Decimal,
    jcp_total: Decimal,
    tax_aware: bool,
    options: OutputOptions,
) -> Result<String> {
    let mut document = build_income_summary_document(
        format!("Income Summary - {} (Monthly Breakdown)", year),
        "Month",
        monthly,
        totals_by_type,
        stats,
        totals,
    );
    append_categorization_block(
        &mut document,
        baseline_total,
        exceptional_total,
        jcp_total,
        tax_aware,
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
    jcp_total: Decimal,
    tax_aware: bool,
    options: OutputOptions,
) -> Result<String> {
    let mut document = build_income_summary_document(
        "Income Summary (Yearly Breakdown)".to_string(),
        "Year",
        yearly,
        totals_by_type,
        stats,
        totals,
    );
    append_categorization_block(
        &mut document,
        baseline_total,
        exceptional_total,
        jcp_total,
        tax_aware,
    );
    crate::formatters::render_document(&document, options)
}

fn append_categorization_block(
    document: &mut OutputDocument,
    baseline_total: Decimal,
    exceptional_total: Decimal,
    jcp_total: Decimal,
    tax_aware: bool,
) {
    let grand_total = baseline_total + exceptional_total;
    let mut cat_rows = vec![
        KeyValueRow {
            label: "Baseline Income".to_string(),
            value: Value::Currency(baseline_total),
        },
        KeyValueRow {
            label: "Exceptional Income".to_string(),
            value: Value::Currency(exceptional_total),
        },
        KeyValueRow {
            label: "Baseline %".to_string(),
            value: if grand_total > Decimal::ZERO {
                Value::Percent((baseline_total / grand_total) * Decimal::from(100))
            } else {
                Value::Percent(Decimal::ZERO)
            },
        },
    ];
    if tax_aware {
        let jcp_tax_rate = Decimal::new(15, 2); // 0.15
        let estimated_tax = jcp_total * jcp_tax_rate;
        cat_rows.push(KeyValueRow {
            label: "Est. JCP Withholding (15%)".to_string(),
            value: Value::Currency(estimated_tax),
        });
    }
    document.blocks.push(OutputBlock::KeyValue {
        title: Some("Income Categorization".to_string()),
        rows: cat_rows,
    });
}

//
// 6. ANALYTICS FORMATTERS
//

pub fn format_yield_report(
    ltm_yield: &crate::reports::income_analytics::LtmYieldResult,
    asset_yields: &[crate::reports::income_analytics::AssetYield],
    options: OutputOptions,
) -> Result<String> {
    let mut blocks = vec![
        OutputBlock::Header {
            level: 1,
            text: "Income Yield Analysis".to_string(),
        },
        OutputBlock::KeyValue {
            title: Some("Portfolio LTM Yield".to_string()),
            rows: vec![
                KeyValueRow {
                    label: "LTM Income".to_string(),
                    value: Value::Currency(ltm_yield.total_ltm_income),
                },
                KeyValueRow {
                    label: "Portfolio Value".to_string(),
                    value: Value::Currency(ltm_yield.portfolio_value),
                },
                KeyValueRow {
                    label: "Yield %".to_string(),
                    value: Value::Percent(ltm_yield.yield_percentage),
                },
            ],
        },
    ];

    if !asset_yields.is_empty() {
        let rows = asset_yields
            .iter()
            .map(|ay| Row {
                cells: vec![
                    Value::Text(ay.ticker.clone()),
                    Value::Text(ay.asset_type.as_str().to_string()),
                    Value::Currency(ay.ltm_income),
                    Value::Currency(ay.current_position_value),
                    Value::Percent(ay.yield_percentage),
                ],
            })
            .collect();

        blocks.push(OutputBlock::Table {
            title: Some("Per-Asset Yields".to_string()),
            columns: vec![
                ColumnDef::new("ticker", "Ticker", ValueKind::Text),
                ColumnDef::new("asset_type", "Type", ValueKind::Text),
                ColumnDef::new("ltm_income", "LTM Income", ValueKind::Currency),
                ColumnDef::new("value", "Value", ValueKind::Currency),
                ColumnDef::new("yield_pct", "Yield%", ValueKind::Percent),
            ],
            rows,
            footer: None,
            options: TableOptions {
                style: TableStyle::Rounded,
            },
        });
    }

    let document = OutputDocument {
        title: None,
        blocks,
        meta: Default::default(),
    };
    crate::formatters::render_document(&document, options)
}

pub fn format_trends_report(
    series: &crate::reports::income_analytics::MonthlyIncomeSeries,
    trend: &crate::reports::income_analytics::TrendAnalysis,
    months: i32,
    options: OutputOptions,
) -> Result<String> {
    let mut blocks = vec![
        OutputBlock::Header {
            level: 1,
            text: format!("Income Trends ({} months)", months),
        },
        OutputBlock::KeyValue {
            title: Some("Trend Summary".to_string()),
            rows: vec![
                KeyValueRow {
                    label: "Direction".to_string(),
                    value: Value::Text(trend.trend_direction.as_str().to_string()),
                },
                KeyValueRow {
                    label: "YoY Growth".to_string(),
                    value: Value::Percent(trend.yoy_growth_percentage),
                },
                KeyValueRow {
                    label: "Volatility".to_string(),
                    value: Value::Percent(trend.volatility),
                },
                KeyValueRow {
                    label: "Monthly Average".to_string(),
                    value: Value::Currency(series.mean()),
                },
            ],
        },
    ];

    if !series.months.is_empty() {
        let rows = series
            .months
            .iter()
            .zip(series.amounts.iter())
            .map(|(month, amount)| Row {
                cells: vec![
                    Value::Text(month.format("%Y-%m").to_string()),
                    Value::Currency(*amount),
                ],
            })
            .collect();

        blocks.push(OutputBlock::Table {
            title: Some("Monthly Series".to_string()),
            columns: vec![
                ColumnDef::new("month", "Month", ValueKind::Text),
                ColumnDef::new("income", "Income", ValueKind::Currency),
            ],
            rows,
            footer: None,
            options: TableOptions {
                style: TableStyle::Rounded,
            },
        });
    }

    let document = OutputDocument {
        title: None,
        blocks,
        meta: Default::default(),
    };
    crate::formatters::render_document(&document, options)
}

pub fn format_forecast_report(
    forecasts: &[crate::reports::income_analytics::IncomeForecast],
    forecast_year: i32,
    conservative: bool,
    options: OutputOptions,
) -> Result<String> {
    let total_expected: Decimal = forecasts.iter().map(|f| f.expected_annual_income).sum();
    let total_lower: Decimal = forecasts.iter().map(|f| f.lower_bound).sum();
    let total_upper: Decimal = forecasts.iter().map(|f| f.upper_bound).sum();

    let rows = forecasts
        .iter()
        .map(|f| Row {
            cells: vec![
                Value::Text(f.ticker.clone()),
                Value::Currency(f.expected_annual_income),
                Value::Currency(f.lower_bound),
                Value::Currency(f.upper_bound),
                Value::Text(f.confidence.as_str().to_string()),
                Value::Text(format!("{} mo", f.months_of_history)),
            ],
        })
        .collect();

    let mode = if conservative {
        "Conservative"
    } else {
        "Standard"
    };

    let document = OutputDocument {
        title: None,
        blocks: vec![
            OutputBlock::Header {
                level: 1,
                text: format!("Income Forecast - {} ({})", forecast_year, mode),
            },
            OutputBlock::Table {
                title: Some("Per-Asset Forecasts".to_string()),
                columns: vec![
                    ColumnDef::new("ticker", "Ticker", ValueKind::Text),
                    ColumnDef::new("expected", "Expected", ValueKind::Currency),
                    ColumnDef::new("lower", "Lower", ValueKind::Currency),
                    ColumnDef::new("upper", "Upper", ValueKind::Currency),
                    ColumnDef::new("confidence", "Confidence", ValueKind::Text),
                    ColumnDef::new("history", "History", ValueKind::Text),
                ],
                rows,
                footer: None,
                options: TableOptions {
                    style: TableStyle::Rounded,
                },
            },
            OutputBlock::KeyValue {
                title: Some("Portfolio Forecast".to_string()),
                rows: vec![
                    KeyValueRow {
                        label: "Expected Total".to_string(),
                        value: Value::Currency(total_expected),
                    },
                    KeyValueRow {
                        label: "Range".to_string(),
                        value: Value::Text(format!("{} - {}", total_lower, total_upper)),
                    },
                ],
            },
        ],
        meta: Default::default(),
    };
    crate::formatters::render_document(&document, options)
}

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
                Value::Date(*date),
                Value::Text(ticker.clone()),
                Value::Currency(*amount),
                Value::Text(confidence.as_str().to_string()),
            ],
        })
        .collect();

    let document = OutputDocument {
        title: None,
        blocks: vec![
            OutputBlock::Header {
                level: 1,
                text: "Predicted Payment Calendar".to_string(),
            },
            OutputBlock::Table {
                title: None,
                columns: vec![
                    ColumnDef::new("date", "Expected Date", ValueKind::Date),
                    ColumnDef::new("ticker", "Ticker", ValueKind::Text),
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

pub fn format_alerts_report(
    anomalies: &[crate::reports::income_analytics::IncomeAnomaly],
    options: OutputOptions,
) -> Result<String> {
    if anomalies.is_empty() {
        let document = OutputDocument {
            title: None,
            blocks: vec![
                OutputBlock::Header {
                    level: 1,
                    text: "Income Alerts".to_string(),
                },
                OutputBlock::EmptyState {
                    message: "No income anomalies detected.".to_string(),
                    hint: None,
                },
            ],
            meta: Default::default(),
        };
        return crate::formatters::render_document(&document, options);
    }

    let rows = anomalies
        .iter()
        .map(|a| {
            let type_str = match a.anomaly_type {
                crate::reports::income_analytics::AnomalyType::MissedPayment => "Missed Payment",
                crate::reports::income_analytics::AnomalyType::UnusualAmount => "Unusual Amount",
                crate::reports::income_analytics::AnomalyType::IncomeDrop => "Income Drop",
            };
            Row {
                cells: vec![
                    Value::Text(type_str.to_string()),
                    Value::Text(a.ticker.clone()),
                    Value::Text(a.description.clone()),
                ],
            }
        })
        .collect();

    let document = OutputDocument {
        title: None,
        blocks: vec![
            OutputBlock::Header {
                level: 1,
                text: "Income Alerts".to_string(),
            },
            OutputBlock::Table {
                title: None,
                columns: vec![
                    ColumnDef::new("type", "Alert Type", ValueKind::Text),
                    ColumnDef::new("ticker", "Ticker", ValueKind::Text),
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
            cost_basis: None,
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
