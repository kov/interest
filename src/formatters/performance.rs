//! Performance report formatting (JSON and terminal table)

use crate::formatters::OutputOptions;
use crate::output::{
    ColumnDef, KeyValueRow, OutputBlock, OutputDocument, Row, TableOptions, Value, ValueKind,
};
use crate::reports::performance::PerformanceReport;
use anyhow::Result;
use rust_decimal::Decimal;

/// Format performance report (testable, no I/O)
///
/// This is the main formatting entry point that dispatches to the appropriate
/// formatter based on output mode.
///
/// # Arguments
///
/// * `report` - The performance report to format
/// * `mode` - Output format (Table or Json)
///
/// # Example
///
/// ```ignore
/// let mode = OutputMode::from_json_flag(json_output);
/// let output = formatters::performance::format(&report, mode)?;
/// println!("{}", output);
/// ```
pub fn format(report: &PerformanceReport, options: OutputOptions) -> Result<String> {
    let document = build_performance_document(report);
    crate::formatters::render_document(&document, options)
}

fn build_performance_document(report: &PerformanceReport) -> OutputDocument {
    let growth_label = if report.cash_flows.is_some() {
        "Portfolio Growth"
    } else {
        "Total Return"
    };

    let unrealized_display = if report.unrealized_gains.abs() < Decimal::new(1, 2) {
        Decimal::ZERO
    } else {
        report.unrealized_gains
    };

    let mut blocks = Vec::new();
    blocks.push(OutputBlock::Header {
        level: 1,
        text: "Performance Report".to_string(),
    });

    blocks.push(OutputBlock::KeyValue {
        title: Some("Period".to_string()),
        rows: vec![
            KeyValueRow {
                label: "Start".to_string(),
                value: Value::Date(report.start_date),
            },
            KeyValueRow {
                label: "End".to_string(),
                value: Value::Date(report.end_date),
            },
        ],
    });

    let mut summary_rows = vec![
        KeyValueRow {
            label: "Start Value".to_string(),
            value: Value::Currency(report.start_value),
        },
        KeyValueRow {
            label: "End Value".to_string(),
            value: Value::Currency(report.end_value),
        },
        KeyValueRow {
            label: growth_label.to_string(),
            value: Value::CurrencyDelta(report.total_return),
        },
        KeyValueRow {
            label: "Return".to_string(),
            value: Value::Percent(total_return_pct(
                report.end_value,
                report.unrealized_gains,
                report.realized_gains,
            )),
        },
        KeyValueRow {
            label: "Time-weighted".to_string(),
            value: Value::Percent(report.time_weighted_return),
        },
    ];

    summary_rows.push(KeyValueRow {
        label: "Realized Gains".to_string(),
        value: Value::CurrencyDelta(report.realized_gains),
    });
    summary_rows.push(KeyValueRow {
        label: "Unrealized Gains".to_string(),
        value: Value::CurrencyDelta(unrealized_display),
    });

    blocks.push(OutputBlock::KeyValue {
        title: Some("Summary".to_string()),
        rows: summary_rows,
    });

    if let Some(ref cf) = report.cash_flows {
        blocks.push(OutputBlock::KeyValue {
            title: Some(format!("Cash Flows ({} transactions)", cf.flow_count)),
            rows: vec![
                KeyValueRow {
                    label: "Contributions".to_string(),
                    value: Value::CurrencyDelta(cf.total_contributions),
                },
                KeyValueRow {
                    label: "Withdrawals".to_string(),
                    value: Value::CurrencyDelta(cf.total_withdrawals),
                },
                KeyValueRow {
                    label: "Net Flow".to_string(),
                    value: Value::CurrencyDelta(cf.net_flow),
                },
            ],
        });
    }

    if !report.asset_breakdown.is_empty() {
        let mut breakdown_vec: Vec<_> = report.asset_breakdown.iter().collect();
        breakdown_vec.sort_by(|a, b| b.1.cost_basis.cmp(&a.1.cost_basis));

        let rows = breakdown_vec
            .iter()
            .map(|(asset_type, perf)| Row {
                cells: vec![
                    Value::Text(asset_type.as_str().to_string()),
                    Value::Currency(perf.cost_basis),
                    Value::Currency(perf.market_value),
                    Value::CurrencyDelta(perf.unrealized_pl),
                    Value::Percent(perf.return_pct),
                    Value::Percent(perf.contribution_to_total),
                    Value::CurrencyDelta(perf.realized_gains),
                ],
            })
            .collect();

        blocks.push(OutputBlock::Table {
            title: Some("By Asset Type".to_string()),
            columns: vec![
                ColumnDef::new("asset_type", "Asset Type", ValueKind::Text),
                ColumnDef::new("cost_basis", "Cost", ValueKind::Currency),
                ColumnDef::new("market_value", "Value", ValueKind::Currency),
                ColumnDef::new("unrealized_pl", "P&L", ValueKind::CurrencyDelta),
                ColumnDef::new("return", "Return", ValueKind::Percent),
                ColumnDef::new("contribution", "Contribution", ValueKind::Percent),
                ColumnDef::new("realized_gains", "Realized Gains", ValueKind::CurrencyDelta),
            ],
            rows,
            footer: None,
            options: TableOptions::default(),
        });
    }

    OutputDocument {
        title: None,
        blocks,
        meta: Default::default(),
    }
}

fn total_return_pct(
    end_value: Decimal,
    unrealized_gains: Decimal,
    realized_gains: Decimal,
) -> Decimal {
    if end_value > Decimal::ZERO {
        ((unrealized_gains + realized_gains) / (end_value - unrealized_gains)) * Decimal::from(100)
    } else {
        Decimal::ZERO
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::models::AssetType;
    use crate::reports::performance::{AssetPerformance, CashFlowSummary, PerformanceReport};
    use crate::reports::Period;
    use chrono::NaiveDate;
    use rust_decimal_macros::dec;
    use std::collections::HashMap;

    fn create_test_report() -> PerformanceReport {
        let mut asset_breakdown = HashMap::new();
        asset_breakdown.insert(
            AssetType::Stock,
            AssetPerformance {
                asset_type: AssetType::Stock,
                cost_basis: dec!(10000),
                market_value: dec!(11000),
                unrealized_pl: dec!(1000),
                return_pct: dec!(10.0),
                contribution_to_total: dec!(5.0),
                realized_gains: dec!(200),
            },
        );

        PerformanceReport {
            period: Period::Ytd,
            start_date: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            end_date: NaiveDate::from_ymd_opt(2024, 12, 31).unwrap(),
            start_value: dec!(10000),
            end_value: dec!(11000),
            total_return: dec!(1000),
            time_weighted_return: dec!(10.0),
            realized_gains: dec!(200),
            unrealized_gains: dec!(800),
            asset_breakdown,
            cash_flows: Some(CashFlowSummary {
                total_contributions: dec!(5000),
                total_withdrawals: dec!(1000),
                net_flow: dec!(4000),
                flow_count: 10,
            }),
        }
    }

    #[test]
    fn test_json_decimals_as_strings() {
        let report = create_test_report();
        let json_str = format(&report, OutputOptions::from_flags(true, false)).unwrap();
        let json: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        let blocks = json["blocks"].as_array().expect("blocks array missing");
        let mut has_currency = false;
        let mut has_percent = false;

        fn collect_cells(blocks: &[serde_json::Value], kind: &str) -> Vec<serde_json::Value> {
            let mut values = Vec::new();
            for block in blocks {
                match block.get("type").and_then(|t| t.as_str()) {
                    Some("table") => {
                        if let Some(rows) = block.get("rows").and_then(|r| r.as_array()) {
                            for row in rows {
                                if let Some(cells) = row.get("cells").and_then(|c| c.as_array()) {
                                    for cell in cells {
                                        let cell_kind = cell.get("kind").and_then(|k| k.as_str());
                                        if cell_kind == Some(kind) {
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
                                    let cell_kind = value.get("kind").and_then(|k| k.as_str());
                                    if cell_kind == Some(kind) {
                                        values.push(value.clone());
                                    }
                                }
                            }
                        }
                    }
                    Some("section") => {
                        if let Some(nested) = block.get("blocks").and_then(|b| b.as_array()) {
                            values.extend(collect_cells(nested, kind));
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

        for cell in collect_cells(blocks, "percent") {
            if let Some(value) = cell.get("value") {
                assert!(value.is_string());
                has_percent = true;
                break;
            }
        }

        assert!(has_currency, "expected currency cell in JSON");
        assert!(has_percent, "expected percent cell in JSON");
    }

    #[test]
    fn test_json_dates_as_strings() {
        let report = create_test_report();
        let json_str = format(&report, OutputOptions::from_flags(true, false)).unwrap();
        let json: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        let blocks = json["blocks"].as_array().expect("blocks array missing");
        let mut dates = Vec::new();

        for block in blocks {
            if block.get("type").and_then(|t| t.as_str()) == Some("key_value")
                && block
                    .get("title")
                    .and_then(|t| t.as_str())
                    .unwrap_or_default()
                    == "Period"
            {
                if let Some(rows) = block.get("rows").and_then(|r| r.as_array()) {
                    for row in rows {
                        if let Some(value) = row.get("value") {
                            if value.get("kind").and_then(|k| k.as_str()) == Some("date") {
                                if let Some(date_value) = value.get("value") {
                                    dates.push(date_value.as_str().unwrap().to_string());
                                }
                            }
                        }
                    }
                }
            }
        }

        assert!(dates.contains(&"2024-01-01".to_string()));
        assert!(dates.contains(&"2024-12-31".to_string()));
    }

    #[test]
    fn test_table_has_key_sections() {
        let report = create_test_report();
        let output = format(&report, OutputOptions::from_flags(false, false)).unwrap();

        assert!(output.contains("Performance Report"));
        assert!(output.contains("Start Value"));
        assert!(output.contains("End Value"));
        assert!(output.contains("Cash Flows"));
        assert!(output.contains("By Asset Type"));
    }
}
