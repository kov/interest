//! Cashflow report formatting (JSON and terminal table)

use crate::db::AssetType;
use crate::formatters::OutputOptions;
use crate::output::{
    ColumnDef, KeyValueRow, OutputBlock, OutputDocument, Row, TableOptions, Value, ValueKind,
};
use crate::reports::cashflow::{CashFlowEntry, CashFlowReport, CashFlowStats, TrendDirection};
use anyhow::Result;
use chrono::Datelike;
use rust_decimal::Decimal;
use std::collections::HashMap;

// Type alias for complex monthly breakdown map
type MonthlyBreakdown = HashMap<(i32, u32), HashMap<AssetType, (Decimal, Decimal, Decimal)>>;

// =============================================================================
// Cash Flow Show Command (year-by-year breakdown)
// =============================================================================

/// Format cash flow report (standard year-by-year breakdown)
pub fn format_cashflow_show(report: &CashFlowReport, options: OutputOptions) -> Result<String> {
    let document = build_cashflow_show_document(report);
    crate::formatters::render_document(&document, options)
}

/// Format cash flow with monthly breakdown (single-year periods only, table mode)
/// Note: JSON mode uses standard format, as monthly breakdown is table-specific
pub fn format_cashflow_show_monthly(
    report: &CashFlowReport,
    entries: &[CashFlowEntry],
    options: OutputOptions,
) -> Result<String> {
    let document = build_cashflow_monthly_document(report, entries);
    crate::formatters::render_document(&document, options)
}

/// Format cash flow stats
pub fn format_cashflow_stats(
    stats: &CashFlowStats,
    from_date: &str,
    to_date: &str,
    options: OutputOptions,
) -> Result<String> {
    let document = build_cashflow_stats_document(stats, from_date, to_date);
    crate::formatters::render_document(&document, options)
}

// Internal implementations below

fn build_cashflow_show_document(report: &CashFlowReport) -> OutputDocument {
    let type_order = asset_type_order();

    let mut rows = Vec::new();
    for year in &report.years {
        for asset_type in &type_order {
            if let Some(values) = year.by_asset_type.get(asset_type) {
                if values.net_flow == Decimal::ZERO {
                    continue;
                }
                rows.push(Row {
                    cells: vec![
                        Value::Text(year.year.to_string()),
                        Value::Text(asset_type.as_str().to_string()),
                        Value::Currency(values.money_in),
                        Value::Currency(values.money_out_sells),
                        Value::Currency(values.money_out_income),
                        Value::CurrencyDelta(values.net_flow),
                    ],
                });
            }
        }
    }

    let breakdown_rows = build_cashflow_breakdown_rows(report, &type_order);

    OutputDocument {
        title: None,
        blocks: vec![
            OutputBlock::Header {
                level: 1,
                text: "Cash Flow Summary".to_string(),
            },
            OutputBlock::KeyValue {
                title: Some("Period".to_string()),
                rows: vec![
                    KeyValueRow {
                        label: "From".to_string(),
                        value: Value::Date(report.from_date),
                    },
                    KeyValueRow {
                        label: "To".to_string(),
                        value: Value::Date(report.to_date),
                    },
                ],
            },
            OutputBlock::Table {
                title: Some("Yearly Breakdown".to_string()),
                columns: vec![
                    ColumnDef::new("year", "Year", ValueKind::Text),
                    ColumnDef::new("asset_type", "Asset Type", ValueKind::Text),
                    ColumnDef::new("money_in", "In", ValueKind::Currency),
                    ColumnDef::new("out_sells", "Out (Sells)", ValueKind::Currency),
                    ColumnDef::new("out_income", "Out (Income)", ValueKind::Currency),
                    ColumnDef::new("net_flow", "Net Flow", ValueKind::CurrencyDelta),
                ],
                rows,
                footer: None,
                options: TableOptions::default(),
            },
            OutputBlock::KeyValue {
                title: Some("Totals".to_string()),
                rows: vec![
                    KeyValueRow {
                        label: "Total In".to_string(),
                        value: Value::Currency(report.total_in),
                    },
                    KeyValueRow {
                        label: "Total Out".to_string(),
                        value: Value::Currency(report.total_out),
                    },
                    KeyValueRow {
                        label: "Net Flow".to_string(),
                        value: Value::CurrencyDelta(report.net_flow),
                    },
                ],
            },
            OutputBlock::Table {
                title: Some("Net Flow Breakdown".to_string()),
                columns: vec![
                    ColumnDef::new("asset_type", "Asset Type", ValueKind::Text),
                    ColumnDef::new("net_flow", "Net Flow", ValueKind::CurrencyDelta),
                ],
                rows: breakdown_rows,
                footer: None,
                options: TableOptions::default(),
            },
        ],
        meta: Default::default(),
    }
}

fn build_cashflow_monthly_document(
    report: &CashFlowReport,
    entries: &[CashFlowEntry],
) -> OutputDocument {
    let type_order = asset_type_order();

    let mut by_month: MonthlyBreakdown = HashMap::new();
    for entry in entries {
        let key = (entry.date.year(), entry.date.month());
        let month_map = by_month.entry(key).or_default();
        let totals = month_map.entry(entry.asset_type).or_insert((
            Decimal::ZERO,
            Decimal::ZERO,
            Decimal::ZERO,
        ));
        totals.0 += entry.money_in;
        totals.1 += entry.money_out_sells;
        totals.2 += entry.money_out_income;
    }

    let mut months: Vec<_> = by_month.into_iter().collect();
    months.sort_by_key(|(key, _)| *key);

    let mut rows = Vec::new();
    for ((year, month), asset_map) in months {
        let label = format!("{} {}", month_name_pt(month), year);
        for asset_type in &type_order {
            if let Some((money_in, money_out_sells, money_out_income)) = asset_map.get(asset_type) {
                let net = *money_in - *money_out_sells - *money_out_income;
                if net == Decimal::ZERO {
                    continue;
                }
                rows.push(Row {
                    cells: vec![
                        Value::Text(label.clone()),
                        Value::Text(asset_type.as_str().to_string()),
                        Value::Currency(*money_in),
                        Value::Currency(*money_out_sells),
                        Value::Currency(*money_out_income),
                        Value::CurrencyDelta(net),
                    ],
                });
            }
        }
    }

    let breakdown_rows = build_cashflow_breakdown_rows(report, &type_order);

    OutputDocument {
        title: None,
        blocks: vec![
            OutputBlock::Header {
                level: 1,
                text: "Cash Flow Summary (Monthly)".to_string(),
            },
            OutputBlock::KeyValue {
                title: Some("Period".to_string()),
                rows: vec![
                    KeyValueRow {
                        label: "From".to_string(),
                        value: Value::Date(report.from_date),
                    },
                    KeyValueRow {
                        label: "To".to_string(),
                        value: Value::Date(report.to_date),
                    },
                ],
            },
            OutputBlock::Table {
                title: Some("Monthly Breakdown".to_string()),
                columns: vec![
                    ColumnDef::new("month", "Month", ValueKind::Text),
                    ColumnDef::new("asset_type", "Asset Type", ValueKind::Text),
                    ColumnDef::new("money_in", "In", ValueKind::Currency),
                    ColumnDef::new("out_sells", "Out (Sells)", ValueKind::Currency),
                    ColumnDef::new("out_income", "Out (Income)", ValueKind::Currency),
                    ColumnDef::new("net_flow", "Net Flow", ValueKind::CurrencyDelta),
                ],
                rows,
                footer: None,
                options: TableOptions::default(),
            },
            OutputBlock::KeyValue {
                title: Some("Totals".to_string()),
                rows: vec![
                    KeyValueRow {
                        label: "Total In".to_string(),
                        value: Value::Currency(report.total_in),
                    },
                    KeyValueRow {
                        label: "Total Out".to_string(),
                        value: Value::Currency(report.total_out),
                    },
                    KeyValueRow {
                        label: "Net Flow".to_string(),
                        value: Value::CurrencyDelta(report.net_flow),
                    },
                ],
            },
            OutputBlock::Table {
                title: Some("Net Flow Breakdown".to_string()),
                columns: vec![
                    ColumnDef::new("asset_type", "Asset Type", ValueKind::Text),
                    ColumnDef::new("net_flow", "Net Flow", ValueKind::CurrencyDelta),
                ],
                rows: breakdown_rows,
                footer: None,
                options: TableOptions::default(),
            },
        ],
        meta: Default::default(),
    }
}

/// Get Portuguese month name
fn month_name_pt(month: u32) -> &'static str {
    match month {
        1 => "Janeiro",
        2 => "Fevereiro",
        3 => "Março",
        4 => "Abril",
        5 => "Maio",
        6 => "Junho",
        7 => "Julho",
        8 => "Agosto",
        9 => "Setembro",
        10 => "Outubro",
        11 => "Novembro",
        12 => "Dezembro",
        _ => "Mês inválido",
    }
}

fn asset_type_order() -> Vec<AssetType> {
    vec![
        AssetType::Stock,
        AssetType::Bdr,
        AssetType::Fii,
        AssetType::Fiagro,
        AssetType::FiInfra,
        AssetType::Etf,
        AssetType::Fidc,
        AssetType::Fip,
        AssetType::Bond,
        AssetType::GovBond,
    ]
}

fn build_cashflow_breakdown_rows(report: &CashFlowReport, type_order: &[AssetType]) -> Vec<Row> {
    let mut overall_by_type: HashMap<AssetType, Decimal> = HashMap::new();
    for year in &report.years {
        for (asset_type, values) in &year.by_asset_type {
            *overall_by_type.entry(*asset_type).or_insert(Decimal::ZERO) += values.net_flow;
        }
    }

    let mut rows = Vec::new();
    for asset_type in type_order {
        if let Some(net) = overall_by_type.get(asset_type) {
            if *net == Decimal::ZERO {
                continue;
            }
            rows.push(Row {
                cells: vec![
                    Value::Text(asset_type.as_str().to_string()),
                    Value::CurrencyDelta(*net),
                ],
            });
        }
    }
    rows
}

// =============================================================================
// Cash Flow Stats Command (statistical analysis)
// =============================================================================

fn build_cashflow_stats_document(
    stats: &CashFlowStats,
    from_date: &str,
    to_date: &str,
) -> OutputDocument {
    let avg_growth_value = stats
        .trend
        .avg_yearly_growth_rate
        .map(Value::Percent)
        .unwrap_or(Value::Null);

    let mut blocks = vec![
        OutputBlock::Header {
            level: 1,
            text: "Cash Flow Statistics".to_string(),
        },
        OutputBlock::KeyValue {
            title: Some("Period".to_string()),
            rows: vec![
                KeyValueRow {
                    label: "From".to_string(),
                    value: Value::Text(from_date.to_string()),
                },
                KeyValueRow {
                    label: "To".to_string(),
                    value: Value::Text(to_date.to_string()),
                },
            ],
        },
    ];

    blocks.push(OutputBlock::KeyValue {
        title: Some("Averages".to_string()),
        rows: vec![
            KeyValueRow {
                label: "Avg Monthly In".to_string(),
                value: Value::Currency(stats.avg_monthly_in),
            },
            KeyValueRow {
                label: "Avg Monthly Out".to_string(),
                value: Value::Currency(stats.avg_monthly_out),
            },
            KeyValueRow {
                label: "Avg Monthly Net".to_string(),
                value: Value::CurrencyDelta(stats.avg_monthly_net),
            },
            KeyValueRow {
                label: "Avg Yearly In".to_string(),
                value: Value::Currency(stats.avg_yearly_in),
            },
            KeyValueRow {
                label: "Avg Yearly Out".to_string(),
                value: Value::Currency(stats.avg_yearly_out),
            },
            KeyValueRow {
                label: "Avg Yearly Net".to_string(),
                value: Value::CurrencyDelta(stats.avg_yearly_net),
            },
        ],
    });

    blocks.push(OutputBlock::KeyValue {
        title: Some("Coverage".to_string()),
        rows: vec![
            KeyValueRow {
                label: "Months with Data".to_string(),
                value: Value::Text(stats.months_with_data.to_string()),
            },
            KeyValueRow {
                label: "Years with Data".to_string(),
                value: Value::Text(stats.years_with_data.to_string()),
            },
            KeyValueRow {
                label: "Avg YoY Growth".to_string(),
                value: avg_growth_value,
            },
        ],
    });

    if !stats.trend.yearly_changes.is_empty() {
        let rows = stats
            .trend
            .yearly_changes
            .iter()
            .map(|change| Row {
                cells: vec![
                    Value::Text(change.year.to_string()),
                    Value::CurrencyDelta(change.curr_year_net),
                    change
                        .growth_rate_pct
                        .map(Value::Percent)
                        .unwrap_or(Value::Null),
                ],
            })
            .collect::<Vec<_>>();

        blocks.push(OutputBlock::Table {
            title: Some("Year-over-Year Changes".to_string()),
            columns: vec![
                ColumnDef::new("year", "Year", ValueKind::Text),
                ColumnDef::new("net_flow", "Net Flow", ValueKind::CurrencyDelta),
                ColumnDef::new("growth", "vs Prior Year", ValueKind::Percent),
            ],
            rows,
            footer: None,
            options: TableOptions::default(),
        });
    }

    let trend_text = match stats.trend.direction {
        TrendDirection::Increasing => "Increasing contributions over time",
        TrendDirection::Decreasing => "Decreasing contributions over time",
        TrendDirection::Stable => "Stable contributions over time",
    };

    blocks.push(OutputBlock::KeyValue {
        title: Some("Trend".to_string()),
        rows: vec![KeyValueRow {
            label: "Direction".to_string(),
            value: Value::Text(trend_text.to_string()),
        }],
    });

    OutputDocument {
        title: None,
        blocks,
        meta: Default::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reports::cashflow::{
        AssetTypeCashFlow, CashFlowTrend, YearlyCashFlow, YearlyChange,
    };
    use chrono::NaiveDate;
    use rust_decimal_macros::dec;

    fn create_test_report() -> CashFlowReport {
        let mut by_asset_type = HashMap::new();
        by_asset_type.insert(
            AssetType::Stock,
            AssetTypeCashFlow {
                money_in: dec!(5000),
                money_out_sells: dec!(1000),
                money_out_income: dec!(500),
                net_flow: dec!(3500),
            },
        );

        CashFlowReport {
            from_date: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            to_date: NaiveDate::from_ymd_opt(2024, 12, 31).unwrap(),
            years: vec![YearlyCashFlow {
                year: 2024,
                by_asset_type,
            }],
            total_in: dec!(5000),
            total_out: dec!(1500),
            net_flow: dec!(3500),
        }
    }

    fn create_test_stats() -> CashFlowStats {
        CashFlowStats {
            avg_monthly_in: dec!(416.67),
            avg_monthly_out: dec!(125.00),
            avg_monthly_net: dec!(291.67),
            avg_yearly_in: dec!(5000),
            avg_yearly_out: dec!(1500),
            avg_yearly_net: dec!(3500),
            months_with_data: 12,
            years_with_data: 1,
            trend: CashFlowTrend {
                direction: TrendDirection::Increasing,
                avg_yearly_growth_rate: Some(dec!(10.5)),
                yearly_changes: vec![YearlyChange {
                    year: 2024,
                    curr_year_net: dec!(3500),
                    growth_rate_pct: Some(dec!(16.67)),
                }],
            },
        }
    }

    #[test]
    fn test_cashflow_show_json_decimals_as_strings() {
        let report = create_test_report();
        let json_str =
            format_cashflow_show(&report, OutputOptions::from_flags(true, false)).unwrap();
        let json: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        let blocks = json["blocks"].as_array().expect("blocks array missing");
        let mut has_currency = false;
        fn collect_cells(blocks: &[serde_json::Value], kind: &str) -> Vec<serde_json::Value> {
            let mut values = Vec::new();
            for block in blocks {
                match block.get("type").and_then(|t| t.as_str()) {
                    Some("table") => {
                        if let Some(rows) = block.get("rows").and_then(|r| r.as_array()) {
                            for row in rows {
                                if let Some(cells) = row.get("cells").and_then(|c| c.as_array()) {
                                    for cell in cells {
                                        if cell.get("kind").and_then(|k| k.as_str()) == Some(kind) {
                                            values.push(cell.clone());
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Some("key_value") => {
                        if let Some(rows) = block.get("rows").and_then(|r| r.as_array()) {
                            for row in rows {
                                if let Some(value) = row.get("value") {
                                    if value.get("kind").and_then(|k| k.as_str()) == Some(kind) {
                                        values.push(value.clone());
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            values
        }

        for cell in collect_cells(blocks, "currency") {
            if let Some(value) = cell.get("value") {
                assert!(value.is_string());
                has_currency = true;
                break;
            }
        }

        assert!(has_currency, "expected currency cell in JSON");
    }

    #[test]
    fn test_cashflow_stats_json_decimals_as_strings() {
        let stats = create_test_stats();
        let json_str = format_cashflow_stats(
            &stats,
            "2024-01-01",
            "2024-12-31",
            OutputOptions::from_flags(true, false),
        )
        .unwrap();
        let json: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        let blocks = json["blocks"].as_array().expect("blocks array missing");
        let mut has_currency = false;
        let mut has_percent = false;

        for block in blocks {
            match block.get("type").and_then(|t| t.as_str()) {
                Some("key_value") => {
                    if let Some(rows) = block.get("rows").and_then(|r| r.as_array()) {
                        for row in rows {
                            if let Some(value) = row.get("value") {
                                if let Some(kind) = value.get("kind").and_then(|k| k.as_str()) {
                                    if kind == "currency" || kind == "currency_delta" {
                                        if let Some(raw) = value.get("value") {
                                            assert!(raw.is_string());
                                            has_currency = true;
                                        }
                                    }
                                    if kind == "percent" {
                                        if let Some(raw) = value.get("value") {
                                            assert!(raw.is_string());
                                            has_percent = true;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                Some("table") => {
                    if let Some(rows) = block.get("rows").and_then(|r| r.as_array()) {
                        for row in rows {
                            if let Some(cells) = row.get("cells").and_then(|c| c.as_array()) {
                                for cell in cells {
                                    if cell.get("kind").and_then(|k| k.as_str()) == Some("percent")
                                    {
                                        if let Some(raw) = cell.get("value") {
                                            assert!(raw.is_string());
                                            has_percent = true;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        assert!(has_currency, "expected currency cell in JSON");
        assert!(has_percent, "expected percent cell in JSON");
    }

    #[test]
    fn test_cashflow_show_table_has_key_sections() {
        let report = create_test_report();
        let output =
            format_cashflow_show(&report, OutputOptions::from_flags(false, false)).unwrap();

        assert!(output.contains("Cash Flow Summary"));
        assert!(output.contains("Yearly Breakdown"));
        assert!(output.contains("Totals"));
        assert!(output.contains("Net Flow Breakdown"));
    }

    #[test]
    fn test_cashflow_stats_table_has_key_sections() {
        let stats = create_test_stats();
        let output = format_cashflow_stats(
            &stats,
            "2024-01-01",
            "2024-12-31",
            OutputOptions::from_flags(false, false),
        )
        .unwrap();

        assert!(output.contains("Cash Flow Statistics"));
        assert!(output.contains("Averages"));
        assert!(output.contains("Year-over-Year Changes"));
        assert!(output.contains("Trend"));
    }
}
