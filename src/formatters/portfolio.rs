//! Portfolio report formatting (JSON and terminal table)

use crate::db::models::AssetType;
use crate::formatters::OutputMode;
use crate::output::{
    ColumnDef, KeyValueRow, OutputBlock, OutputDocument, Row, TableOptions, Value, ValueKind,
};
use crate::reports::aggregation::aggregate_positions_by_asset_type;
use crate::reports::PortfolioReport;
use std::collections::BTreeMap;

/// Format portfolio report (testable, no I/O)
///
/// This is the main formatting entry point that dispatches to the appropriate
/// formatter based on output mode.
///
/// # Arguments
///
/// * `report` - The portfolio report to format
/// * `asset_type_filter` - Optional asset type filter
/// * `mode` - Output format (Table or Json)
///
/// # Example
///
/// ```ignore
/// let mode = OutputMode::from_json_flag(json_output);
/// let output = formatters::portfolio::format(&report, Some(AssetType::Stock), mode);
/// println!("{}", output);
/// ```
pub fn format(
    report: &PortfolioReport,
    asset_type_filter: Option<AssetType>,
    mode: OutputMode,
) -> String {
    let document = build_portfolio_document(report, asset_type_filter);
    crate::formatters::render_document(&document, mode)
}

/// Print portfolio report to stdout
///
/// Wrapper around `format()` that handles I/O.
///
/// # Example
///
/// ```ignore
/// let mode = OutputMode::from_json_flag(json_output);
/// formatters::portfolio::print(&report, Some(AssetType::Fii), mode);
/// ```
pub fn print(report: &PortfolioReport, asset_type_filter: Option<AssetType>, mode: OutputMode) {
    println!("{}", format(report, asset_type_filter, mode));
}

fn build_portfolio_document(
    report: &PortfolioReport,
    asset_type_filter: Option<AssetType>,
) -> OutputDocument {
    let header_text = match asset_type_filter {
        Some(filter) => format!("Portfolio - {} only", filter.as_str().to_uppercase()),
        None => "Complete Portfolio".to_string(),
    };

    let mut document = OutputDocument {
        title: None,
        blocks: vec![OutputBlock::Header {
            level: 1,
            text: header_text,
        }],
        meta: Default::default(),
    };

    if report.positions.is_empty() {
        document.blocks.push(OutputBlock::EmptyState {
            message: "No positions found".to_string(),
            hint: Some("Import transactions first using: interest import <file>".to_string()),
        });
        return document;
    }

    let mut grouped: BTreeMap<AssetType, Vec<_>> = BTreeMap::new();
    for position in &report.positions {
        grouped
            .entry(position.asset.asset_type)
            .or_insert_with(Vec::new)
            .push(position);
    }

    for positions in grouped.values_mut() {
        positions.sort_by(|a, b| a.asset.ticker.cmp(&b.asset.ticker));
    }

    let totals_by_type = aggregate_positions_by_asset_type(&report.positions);

    for (asset_type, positions) in &grouped {
        let totals = totals_by_type.get(asset_type).cloned().unwrap_or_default();

        let rows = positions
            .iter()
            .map(|position| Row {
                cells: vec![
                    Value::Text(position.asset.ticker.clone()),
                    Value::Quantity(position.quantity),
                    Value::Currency(position.average_cost),
                    Value::Currency(position.total_cost),
                    position
                        .current_price
                        .map(Value::Currency)
                        .unwrap_or(Value::Null),
                    position
                        .current_value
                        .map(Value::Currency)
                        .unwrap_or(Value::Null),
                    position
                        .unrealized_pl
                        .map(Value::CurrencyDelta)
                        .unwrap_or(Value::Null),
                    position
                        .unrealized_pl_pct
                        .map(Value::Percent)
                        .unwrap_or(Value::Null),
                ],
            })
            .collect::<Vec<_>>();

        let columns = vec![
            ColumnDef::new("ticker", "Ticker", ValueKind::Text),
            ColumnDef::new("quantity", "Quantity", ValueKind::Quantity),
            ColumnDef::new("avg_cost", "Avg Cost", ValueKind::Currency),
            ColumnDef::new("total_cost", "Total Cost", ValueKind::Currency),
            ColumnDef::new("price", "Price", ValueKind::Currency),
            ColumnDef::new("value", "Value", ValueKind::Currency),
            ColumnDef::new("pl", "P&L", ValueKind::CurrencyDelta),
            ColumnDef::new("return_pct", "Return", ValueKind::Percent),
        ];

        let section_blocks = vec![
            OutputBlock::Table {
                title: None,
                columns,
                rows,
                footer: None,
                options: TableOptions::default(),
            },
            OutputBlock::KeyValue {
                title: Some("Subtotal".to_string()),
                rows: vec![
                    KeyValueRow {
                        label: "Cost".to_string(),
                        value: Value::Currency(totals.cost_basis),
                    },
                    KeyValueRow {
                        label: "Value".to_string(),
                        value: Value::Currency(totals.market_value),
                    },
                    KeyValueRow {
                        label: "P&L".to_string(),
                        value: Value::CurrencyDelta(totals.unrealized_pl),
                    },
                    KeyValueRow {
                        label: "Return".to_string(),
                        value: Value::Percent(totals.return_pct),
                    },
                ],
            },
        ];

        document.blocks.push(OutputBlock::Section {
            title: Some(format!(
                "{} ({})",
                asset_type_name(asset_type),
                asset_type.as_str()
            )),
            blocks: section_blocks,
        });
    }

    document.blocks.push(OutputBlock::KeyValue {
        title: Some("Portfolio Summary".to_string()),
        rows: vec![
            KeyValueRow {
                label: "Total Cost".to_string(),
                value: Value::Currency(report.total_cost),
            },
            KeyValueRow {
                label: "Total Value".to_string(),
                value: Value::Currency(report.total_value),
            },
            KeyValueRow {
                label: "Total P&L".to_string(),
                value: Value::CurrencyDelta(report.total_pl),
            },
            KeyValueRow {
                label: "Total Return".to_string(),
                value: Value::Percent(report.total_pl_pct),
            },
        ],
    });

    document
}

/// Get friendly name for asset type
fn asset_type_name(asset_type: &AssetType) -> &'static str {
    match asset_type {
        AssetType::Stock => "Stocks",
        AssetType::Bdr => "BDRs",
        AssetType::Etf => "ETFs",
        AssetType::Fii => "Real Estate Funds",
        AssetType::Fiagro => "Agribusiness Funds",
        AssetType::FiInfra => "Infrastructure Funds",
        AssetType::Fidc => "FIDCs",
        AssetType::Fip => "FIPs",
        AssetType::Bond => "Corporate Bonds",
        AssetType::GovBond => "Government Bonds",
        AssetType::Option => "Options",
        AssetType::TermContract => "Term Contracts",
        AssetType::Unknown => "Unknown",
    }
}

/// Format empty portfolio message
pub fn format_empty_portfolio() -> String {
    let document = OutputDocument {
        title: None,
        blocks: vec![OutputBlock::EmptyState {
            message: "No positions found".to_string(),
            hint: Some("Import transactions first using: interest import <file>".to_string()),
        }],
        meta: Default::default(),
    };
    crate::formatters::render_document(&document, OutputMode::Table)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Asset;
    use crate::reports::portfolio::PositionSummary;
    use crate::reports::PortfolioReport;
    use chrono::Utc;
    use colored::control;
    use rust_decimal::Decimal;

    #[test]
    fn test_empty_portfolio_message() {
        control::set_override(false); // Disable colors for testing
        let msg = format_empty_portfolio();
        assert!(msg.contains("No positions found"));
        assert!(msg.contains("import"));
    }

    fn create_test_position(
        ticker: &str,
        asset_type: AssetType,
        quantity: Decimal,
        average_cost: Decimal,
    ) -> PositionSummary {
        let total_cost = quantity * average_cost;
        let current_price = average_cost + Decimal::from(10);
        let current_value = quantity * current_price;
        let unrealized_pl = current_value - total_cost;
        let unrealized_pl_pct = if total_cost > Decimal::ZERO {
            Some((unrealized_pl / total_cost) * Decimal::from(100))
        } else {
            None
        };

        PositionSummary {
            asset: Asset {
                id: None,
                ticker: ticker.to_string(),
                asset_type,
                name: Some(format!("{} Company", ticker)),
                cnpj: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
            quantity,
            average_cost,
            total_cost,
            current_price: Some(current_price),
            current_value: Some(current_value),
            unrealized_pl: Some(unrealized_pl),
            unrealized_pl_pct,
        }
    }

    #[test]
    fn test_json_decimals_as_strings() {
        let positions = vec![create_test_position(
            "PETR4",
            AssetType::Stock,
            Decimal::from(100),
            Decimal::from(20),
        )];

        let total_cost: Decimal = positions.iter().map(|p| p.total_cost).sum();
        let total_value: Decimal = positions
            .iter()
            .map(|p| p.current_value.unwrap_or_default())
            .sum();

        let report = PortfolioReport {
            positions,
            total_cost,
            total_value,
            total_pl: total_value - total_cost,
            total_pl_pct: ((total_value - total_cost) / total_cost) * Decimal::from(100),
        };

        let json_str = format(&report, None, OutputMode::Json);
        let json: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        let blocks = json["blocks"].as_array().expect("blocks array missing");
        let mut has_quantity = false;
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
                                        let cell_kind = cell.get("kind").and_then(|k| k.as_str());
                                        if cell_kind == Some(kind) {
                                            values.push(cell.clone());
                                        }
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

        for cell in collect_cells(blocks, "quantity") {
            if let Some(value) = cell.get("value") {
                assert!(value.is_string());
                has_quantity = true;
                break;
            }
        }

        for cell in collect_cells(blocks, "currency") {
            if let Some(value) = cell.get("value") {
                assert!(value.is_string());
                has_currency = true;
                break;
            }
        }

        assert!(has_quantity, "expected quantity cell in JSON");
        assert!(has_currency, "expected currency cell in JSON");
    }

    #[test]
    fn test_portfolio_groups_by_asset_type() {
        control::set_override(false); // Disable colors for testing

        let positions = vec![
            create_test_position(
                "PETR4",
                AssetType::Stock,
                Decimal::from(100),
                Decimal::from(20),
            ),
            create_test_position(
                "VALE3",
                AssetType::Stock,
                Decimal::from(50),
                Decimal::from(15),
            ),
            create_test_position(
                "BBAS3",
                AssetType::Stock,
                Decimal::from(75),
                Decimal::from(25),
            ),
            create_test_position(
                "HFOF11",
                AssetType::Fii,
                Decimal::from(10),
                Decimal::from(100),
            ),
            create_test_position(
                "BRCR11",
                AssetType::Fii,
                Decimal::from(20),
                Decimal::from(110),
            ),
        ];

        let total_cost: Decimal = positions.iter().map(|p| p.total_cost).sum();
        let total_value: Decimal = positions
            .iter()
            .map(|p| p.current_value.unwrap_or_default())
            .sum();
        let report = PortfolioReport {
            positions,
            total_cost,
            total_value,
            total_pl: total_value - total_cost,
            total_pl_pct: ((total_value - total_cost) / total_cost) * Decimal::from(100),
        };

        let output = format(&report, None, OutputMode::Table);

        // Verify grouping by asset type
        assert!(output.contains("Stocks (STOCK)"));
        assert!(output.contains("Real Estate Funds (FII)"));

        // Verify both groups are present
        let stocks_idx = output.find("Stocks (STOCK)").unwrap();
        let fii_idx = output.find("Real Estate Funds (FII)").unwrap();
        assert!(stocks_idx < fii_idx, "Stocks should appear before FIIs");
    }

    #[test]
    fn test_portfolio_sorts_by_ticker_within_group() {
        control::set_override(false); // Disable colors for testing

        let positions = vec![
            create_test_position(
                "VALE3",
                AssetType::Stock,
                Decimal::from(50),
                Decimal::from(15),
            ),
            create_test_position(
                "PETR4",
                AssetType::Stock,
                Decimal::from(100),
                Decimal::from(20),
            ),
            create_test_position(
                "BBAS3",
                AssetType::Stock,
                Decimal::from(75),
                Decimal::from(25),
            ),
        ];

        let total_cost: Decimal = positions.iter().map(|p| p.total_cost).sum();
        let total_value: Decimal = positions
            .iter()
            .map(|p| p.current_value.unwrap_or_default())
            .sum();
        let report = PortfolioReport {
            positions,
            total_cost,
            total_value,
            total_pl: total_value - total_cost,
            total_pl_pct: ((total_value - total_cost) / total_cost) * Decimal::from(100),
        };

        let output = format(&report, None, OutputMode::Table);

        // Find positions in output - they should be in alphabetical order
        let bbas_idx = output.find("BBAS3").unwrap();
        let petr_idx = output.find("PETR4").unwrap();
        let vale_idx = output.find("VALE3").unwrap();

        assert!(bbas_idx < petr_idx, "BBAS3 should appear before PETR4");
        assert!(petr_idx < vale_idx, "PETR4 should appear before VALE3");
    }

    #[test]
    fn test_portfolio_shows_subtotals_per_asset_type() {
        control::set_override(false); // Disable colors for testing

        let positions = vec![
            create_test_position(
                "PETR4",
                AssetType::Stock,
                Decimal::from(100),
                Decimal::from(20),
            ),
            create_test_position(
                "VALE3",
                AssetType::Stock,
                Decimal::from(50),
                Decimal::from(15),
            ),
            create_test_position(
                "HFOF11",
                AssetType::Fii,
                Decimal::from(10),
                Decimal::from(100),
            ),
        ];

        let total_cost: Decimal = positions.iter().map(|p| p.total_cost).sum();
        let total_value: Decimal = positions
            .iter()
            .map(|p| p.current_value.unwrap_or_default())
            .sum();
        let report = PortfolioReport {
            positions,
            total_cost,
            total_value,
            total_pl: total_value - total_cost,
            total_pl_pct: ((total_value - total_cost) / total_cost) * Decimal::from(100),
        };

        let output = format(&report, None, OutputMode::Table);

        // Verify subtotals are shown
        assert!(
            output.contains("Subtotal"),
            "Should contain subtotal sections"
        );
        // Each asset type group should have a subtotal
        let subtotal_count = output.matches("Subtotal").count();
        assert_eq!(
            subtotal_count, 2,
            "Should have 2 subtotals (one per asset type group)"
        );
    }

    #[test]
    fn test_asset_type_name_mapping() {
        assert_eq!(asset_type_name(&AssetType::Stock), "Stocks");
        assert_eq!(asset_type_name(&AssetType::Bdr), "BDRs");
        assert_eq!(asset_type_name(&AssetType::Etf), "ETFs");
        assert_eq!(asset_type_name(&AssetType::Fii), "Real Estate Funds");
        assert_eq!(asset_type_name(&AssetType::Fiagro), "Agribusiness Funds");
        assert_eq!(asset_type_name(&AssetType::FiInfra), "Infrastructure Funds");
        assert_eq!(asset_type_name(&AssetType::Fidc), "FIDCs");
        assert_eq!(asset_type_name(&AssetType::Fip), "FIPs");
        assert_eq!(asset_type_name(&AssetType::Bond), "Corporate Bonds");
        assert_eq!(asset_type_name(&AssetType::GovBond), "Government Bonds");
        assert_eq!(asset_type_name(&AssetType::Option), "Options");
        assert_eq!(asset_type_name(&AssetType::TermContract), "Term Contracts");
        assert_eq!(asset_type_name(&AssetType::Unknown), "Unknown");
    }
}
