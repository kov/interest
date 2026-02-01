/// Formatters for tax commands
use rust_decimal::Decimal;

use crate::db;
use crate::formatters::OutputMode;
use crate::output::{
    ColumnDef, KeyValueRow, OutputBlock, OutputDocument, Row, TableOptions, TableStyle, Value,
    ValueKind,
};
use crate::tax;

//
// HELPER TYPES AND FUNCTIONS
//

#[derive(Clone)]
pub struct IncomeByType {
    pub ticker: String,
    pub asset_type: db::AssetType,
    pub cnpj: Option<String>,
    pub dividends_net: Decimal,
    pub jcp_net: Decimal,
}

/// Build income summary by ticker for a given year
pub fn build_income_summary(
    conn: &rusqlite::Connection,
    year: i32,
) -> anyhow::Result<Vec<IncomeByType>> {
    use chrono::NaiveDate;
    use std::collections::HashMap;

    let from_date = NaiveDate::from_ymd_opt(year, 1, 1).unwrap();
    let to_date = NaiveDate::from_ymd_opt(year, 12, 31).unwrap();
    let events = db::get_income_events_with_assets(conn, Some(from_date), Some(to_date), None)?;

    let tracked_types = [
        db::AssetType::Fii,
        db::AssetType::FiInfra,
        db::AssetType::Fiagro,
        db::AssetType::Stock,
        db::AssetType::Etf,
        db::AssetType::Bdr,
    ];

    let tracked_set: std::collections::HashSet<db::AssetType> =
        tracked_types.iter().copied().collect();
    let mut by_ticker: HashMap<String, IncomeByType> = HashMap::new();
    for (event, asset) in events {
        if !tracked_set.contains(&asset.asset_type) {
            continue;
        }
        let entry = by_ticker
            .entry(asset.ticker.clone())
            .or_insert(IncomeByType {
                ticker: asset.ticker.clone(),
                asset_type: asset.asset_type,
                cnpj: asset.cnpj.clone(),
                dividends_net: Decimal::ZERO,
                jcp_net: Decimal::ZERO,
            });
        if entry.cnpj.is_none() {
            entry.cnpj = asset.cnpj.clone();
        }
        let net_amount = event.total_amount - event.withholding_tax;
        match event.event_type {
            db::IncomeEventType::Dividend => entry.dividends_net += net_amount,
            db::IncomeEventType::Jcp => entry.jcp_net += net_amount,
            _ => {}
        }
    }

    let mut summary: Vec<IncomeByType> = by_ticker.into_values().collect();
    summary.sort_by(|a, b| a.ticker.cmp(&b.ticker));
    Ok(summary)
}

/// Format CNPJ with standard Brazilian formatting
pub fn format_cnpj(value: Option<&str>) -> Option<String> {
    let raw = value?;
    let digits: String = raw.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() != 14 {
        return Some(raw.to_string());
    }
    Some(format!(
        "{}.{}.{}/{}-{}",
        &digits[0..2],
        &digits[2..5],
        &digits[5..8],
        &digits[8..12],
        &digits[12..14]
    ))
}

//
// TAX REPORT (Annual IRPF Report)
//

/// Format tax report
pub fn format_tax_report(
    report: &tax::irpf::AnnualTaxReport,
    income_summary: &[IncomeByType],
    year: i32,
    mode: OutputMode,
) -> String {
    let document = build_tax_report_document(report, income_summary, year);
    crate::formatters::render_document(&document, mode)
}

fn build_tax_report_document(
    report: &tax::irpf::AnnualTaxReport,
    income_summary: &[IncomeByType],
    year: i32,
) -> OutputDocument {
    let has_income = income_summary
        .iter()
        .any(|entry| entry.dividends_net > Decimal::ZERO || entry.jcp_net > Decimal::ZERO);

    if report.monthly_summaries.is_empty() && !has_income {
        return OutputDocument {
            title: None,
            blocks: vec![
                OutputBlock::Header {
                    level: 1,
                    text: format!("Annual IRPF Tax Report - {}", year),
                },
                OutputBlock::EmptyState {
                    message: format!("No transactions found for year {}", year),
                    hint: None,
                },
            ],
            meta: Default::default(),
        };
    }

    let mut blocks = vec![OutputBlock::Header {
        level: 1,
        text: format!("Annual IRPF Tax Report - {}", year),
    }];

    if !report.previous_losses_carry_forward.is_empty() {
        let rows = report
            .previous_losses_carry_forward
            .iter()
            .map(|(category, amount)| KeyValueRow {
                label: category.display_name().to_string(),
                value: Value::CurrencyDelta(*amount),
            })
            .collect::<Vec<_>>();
        blocks.push(OutputBlock::KeyValue {
            title: Some("Carryover from previous years".to_string()),
            rows,
        });
    }

    if !report.monthly_summaries.is_empty() {
        let rows = report
            .monthly_summaries
            .iter()
            .map(|summary| Row {
                cells: vec![
                    Value::Text(summary.month_name.to_string()),
                    Value::Currency(summary.total_sales),
                    Value::CurrencyDelta(summary.total_profit),
                    Value::CurrencyDelta(summary.total_loss),
                    Value::CurrencyDelta(summary.tax_due),
                ],
            })
            .collect::<Vec<_>>();
        blocks.push(OutputBlock::Table {
            title: Some("Monthly Summary".to_string()),
            columns: vec![
                ColumnDef::new("month", "Month", ValueKind::Text),
                ColumnDef::new("sales", "Sales", ValueKind::Currency),
                ColumnDef::new("profit", "Profit", ValueKind::CurrencyDelta),
                ColumnDef::new("loss", "Loss", ValueKind::CurrencyDelta),
                ColumnDef::new("tax_due", "Tax Due", ValueKind::CurrencyDelta),
            ],
            rows,
            footer: None,
            options: TableOptions {
                style: TableStyle::Rounded,
            },
        });

        blocks.push(OutputBlock::KeyValue {
            title: Some("Annual Totals".to_string()),
            rows: vec![
                KeyValueRow {
                    label: "Total Sales".to_string(),
                    value: Value::Currency(report.annual_total_sales),
                },
                KeyValueRow {
                    label: "Total Profit".to_string(),
                    value: Value::CurrencyDelta(report.annual_total_profit),
                },
                KeyValueRow {
                    label: "Total Loss".to_string(),
                    value: Value::CurrencyDelta(report.annual_total_loss),
                },
                KeyValueRow {
                    label: "Total Tax".to_string(),
                    value: Value::CurrencyDelta(report.annual_total_tax),
                },
            ],
        });

        if !report.losses_to_carry_forward.is_empty() {
            let rows = report
                .losses_to_carry_forward
                .iter()
                .map(|(category, loss)| KeyValueRow {
                    label: category.display_name().to_string(),
                    value: Value::CurrencyDelta(*loss),
                })
                .collect::<Vec<_>>();
            blocks.push(OutputBlock::KeyValue {
                title: Some("Losses to Carry Forward".to_string()),
                rows,
            });
        }
    }

    if has_income {
        let rows = income_summary
            .iter()
            .filter_map(|entry| {
                let total = entry.dividends_net + entry.jcp_net;
                if total <= Decimal::ZERO {
                    return None;
                }
                Some(Row {
                    cells: vec![
                        Value::Text(entry.ticker.clone()),
                        Value::Text(
                            format_cnpj(entry.cnpj.as_deref()).unwrap_or_else(|| "-".to_string()),
                        ),
                        Value::Text(entry.asset_type.as_str().to_string()),
                        Value::Currency(entry.dividends_net),
                        Value::Currency(entry.jcp_net),
                        Value::Currency(total),
                    ],
                })
            })
            .collect::<Vec<_>>();

        if !rows.is_empty() {
            let total_dividends: Decimal = income_summary.iter().map(|e| e.dividends_net).sum();
            let total_jcp: Decimal = income_summary.iter().map(|e| e.jcp_net).sum();
            let total_all = total_dividends + total_jcp;
            let footer = Row {
                cells: vec![
                    Value::Text("TOTAL".to_string()),
                    Value::Text("-".to_string()),
                    Value::Text("TOTAL".to_string()),
                    Value::Currency(total_dividends),
                    Value::Currency(total_jcp),
                    Value::Currency(total_all),
                ],
            };

            blocks.push(OutputBlock::Table {
                title: Some("Dividends & JCP Received".to_string()),
                columns: vec![
                    ColumnDef::new("ticker", "Ticker", ValueKind::Text),
                    ColumnDef::new("cnpj", "CNPJ", ValueKind::Text),
                    ColumnDef::new("asset_type", "Asset Type", ValueKind::Text),
                    ColumnDef::new("dividends", "Dividends (Net)", ValueKind::Currency),
                    ColumnDef::new("jcp", "JCP (Net)", ValueKind::Currency),
                    ColumnDef::new("total", "Total (Net)", ValueKind::Currency),
                ],
                rows,
                footer: Some(footer),
                options: TableOptions {
                    style: TableStyle::Rounded,
                },
            });
        }
    }

    OutputDocument {
        title: None,
        blocks,
        meta: Default::default(),
    }
}

//
// TAX SUMMARY
//

/// Format tax summary
pub fn format_tax_summary(
    report: &tax::irpf::AnnualTaxReport,
    year: i32,
    mode: OutputMode,
) -> String {
    let document = build_tax_summary_document(report, year);
    crate::formatters::render_document(&document, mode)
}

fn build_tax_summary_document(report: &tax::irpf::AnnualTaxReport, year: i32) -> OutputDocument {
    if report.monthly_summaries.is_empty() {
        return OutputDocument {
            title: None,
            blocks: vec![
                OutputBlock::Header {
                    level: 1,
                    text: format!("Tax Summary - {}", year),
                },
                OutputBlock::EmptyState {
                    message: format!("No transactions found for year {}", year),
                    hint: None,
                },
            ],
            meta: Default::default(),
        };
    }

    let rows = report
        .monthly_summaries
        .iter()
        .map(|summary| Row {
            cells: vec![
                Value::Text(summary.month_name.to_string()),
                Value::Currency(summary.total_sales),
                Value::CurrencyDelta(summary.total_profit),
                Value::CurrencyDelta(summary.total_loss),
                Value::CurrencyDelta(summary.tax_due),
            ],
        })
        .collect::<Vec<_>>();

    OutputDocument {
        title: None,
        blocks: vec![
            OutputBlock::Header {
                level: 1,
                text: format!("Tax Summary - {}", year),
            },
            OutputBlock::Table {
                title: None,
                columns: vec![
                    ColumnDef::new("month", "Month", ValueKind::Text),
                    ColumnDef::new("sales", "Sales", ValueKind::Currency),
                    ColumnDef::new("profit", "Profit", ValueKind::CurrencyDelta),
                    ColumnDef::new("loss", "Loss", ValueKind::CurrencyDelta),
                    ColumnDef::new("tax_due", "Tax Due", ValueKind::CurrencyDelta),
                ],
                rows,
                footer: None,
                options: TableOptions {
                    style: TableStyle::Rounded,
                },
            },
            OutputBlock::KeyValue {
                title: Some("Annual Total".to_string()),
                rows: vec![
                    KeyValueRow {
                        label: "Sales".to_string(),
                        value: Value::Currency(report.annual_total_sales),
                    },
                    KeyValueRow {
                        label: "Profit".to_string(),
                        value: Value::CurrencyDelta(report.annual_total_profit),
                    },
                    KeyValueRow {
                        label: "Loss".to_string(),
                        value: Value::CurrencyDelta(report.annual_total_loss),
                    },
                    KeyValueRow {
                        label: "Tax".to_string(),
                        value: Value::CurrencyDelta(report.annual_total_tax),
                    },
                ],
            },
        ],
        meta: Default::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_format_cnpj() {
        assert_eq!(
            format_cnpj(Some("12345678000190")),
            Some("12.345.678/0001-90".to_string())
        );
        assert_eq!(format_cnpj(Some("invalid")), Some("invalid".to_string()));
        assert_eq!(format_cnpj(None), None);
    }

    #[test]
    fn test_tax_report_json_empty() {
        use std::collections::HashMap;

        let report = tax::irpf::AnnualTaxReport {
            year: 2024,
            monthly_summaries: vec![],
            annual_total_sales: dec!(0),
            annual_total_profit: dec!(0),
            annual_total_loss: dec!(0),
            annual_total_tax: dec!(0),
            previous_losses_carry_forward: HashMap::new(),
            losses_to_carry_forward: HashMap::new(),
        };

        let json_str = format_tax_report(&report, &[], 2024, OutputMode::Json);
        let value: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        let blocks = value["blocks"].as_array().expect("blocks array missing");
        let mut has_empty_state = false;
        for block in blocks {
            if block.get("type").and_then(|t| t.as_str()) == Some("empty_state") {
                has_empty_state = true;
            }
        }
        assert!(has_empty_state, "expected empty state for empty report");
    }
}
