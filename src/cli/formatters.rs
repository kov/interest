//! Output formatting module for CLI display
//!
//! This module handles all terminal output formatting, separating
//! the concerns of data calculation from presentation.

use crate::db::models::AssetType;
use crate::reports::PortfolioReport;
use crate::utils::format_currency;
use colored::Colorize;
use rust_decimal::Decimal;
use serde::Serialize;
use std::collections::BTreeMap;
use tabled::{
    settings::{object::Columns, Alignment, Style},
    Table, Tabled,
};

/// Format a portfolio report for JSON output
#[allow(dead_code)] // Planned for JSON output support
pub fn format_portfolio_json(report: &PortfolioReport) -> String {
    #[derive(Serialize)]
    struct JsonPosition {
        ticker: String,
        asset_type: String,
        quantity: String,
        average_cost: String,
        total_cost: String,
        current_price: Option<String>,
        current_value: Option<String>,
        unrealized_pl: Option<String>,
        unrealized_pl_pct: Option<String>,
    }

    #[derive(Serialize)]
    struct JsonPortfolio {
        positions: Vec<JsonPosition>,
        total_cost: String,
        total_value: String,
        total_pl: String,
        total_pl_pct: String,
    }

    let positions = report
        .positions
        .iter()
        .map(|p| JsonPosition {
            ticker: p.asset.ticker.clone(),
            asset_type: p.asset.asset_type.as_str().to_string(),
            quantity: p.quantity.to_string(),
            average_cost: p.average_cost.to_string(),
            total_cost: p.total_cost.to_string(),
            current_price: p.current_price.map(|pr: Decimal| pr.to_string()),
            current_value: p.current_value.map(|v: Decimal| v.to_string()),
            unrealized_pl: p.unrealized_pl.map(|pl: Decimal| pl.to_string()),
            unrealized_pl_pct: p.unrealized_pl_pct.map(|pl: Decimal| pl.to_string()),
        })
        .collect();

    let json_report = JsonPortfolio {
        positions,
        total_cost: report.total_cost.to_string(),
        total_value: report.total_value.to_string(),
        total_pl: report.total_pl.to_string(),
        total_pl_pct: report.total_pl_pct.to_string(),
    };

    serde_json::to_string_pretty(&json_report)
        .unwrap_or_else(|e| format!(r#"{{"error": "JSON serialization failed: {}"}}"#, e))
}

/// Format a portfolio report for terminal table output
pub fn format_portfolio_table(report: &PortfolioReport, asset_type_filter: Option<&str>) -> String {
    let mut output = String::new();

    // Display header
    if let Some(filter) = asset_type_filter {
        output.push_str(&format!(
            "\n{} Portfolio - {} only\n",
            "📊".cyan().bold(),
            filter.to_uppercase()
        ));
    } else {
        output.push_str(&format!("\n{} Complete Portfolio\n", "📊".cyan().bold()));
    }

    // Group positions by asset type
    let mut grouped: BTreeMap<AssetType, Vec<_>> = BTreeMap::new();
    for position in &report.positions {
        grouped
            .entry(position.asset.asset_type)
            .or_insert_with(Vec::new)
            .push(position);
    }

    // Sort positions within each group by ticker (ascending)
    for positions in grouped.values_mut() {
        positions.sort_by(|a, b| a.asset.ticker.cmp(&b.asset.ticker));
    }

    // Display positions table
    #[derive(Tabled)]
    struct PositionRow {
        #[tabled(rename = "Ticker")]
        ticker: String,
        #[tabled(rename = "Quantity")]
        quantity: String,
        #[tabled(rename = "Avg Cost")]
        avg_cost: String,
        #[tabled(rename = "Total Cost")]
        total_cost: String,
        #[tabled(rename = "Price")]
        price: String,
        #[tabled(rename = "Value")]
        value: String,
        #[tabled(rename = "P&L")]
        pl: String,
        #[tabled(rename = "Return %")]
        return_pct: String,
    }

    // Render each asset type group
    for (asset_type, positions) in &grouped {
        // Calculate subtotals for this asset type
        let mut subtotal_cost = Decimal::ZERO;
        let mut subtotal_value = Decimal::ZERO;

        for p in positions.iter() {
            subtotal_cost += p.total_cost;
            if let Some(v) = p.current_value {
                subtotal_value += v;
            }
        }

        let subtotal_pl = subtotal_value - subtotal_cost;
        let subtotal_pl_pct = if subtotal_cost > Decimal::ZERO {
            (subtotal_pl / subtotal_cost) * Decimal::from(100)
        } else {
            Decimal::ZERO
        };

        // Asset type header
        output.push_str(&format!(
            "\n## {} ({})\n",
            asset_type_name(asset_type).bold(),
            asset_type.as_str()
        ));

        if positions.is_empty() {
            output.push_str("No positions\n");
            continue;
        }

        let rows: Vec<PositionRow> = positions
            .iter()
            .map(|p| {
                let price_str = p
                    .current_price
                    .map(|pr: Decimal| format_currency(pr))
                    .unwrap_or_else(|| "N/A".to_string());

                let value_str = p
                    .current_value
                    .map(|v: Decimal| format_currency(v))
                    .unwrap_or_else(|| "N/A".to_string());

                let pl_str = p
                    .unrealized_pl
                    .map(|pl: Decimal| {
                        if pl >= Decimal::ZERO {
                            format_currency(pl).green().to_string()
                        } else {
                            format_currency(pl).red().to_string()
                        }
                    })
                    .unwrap_or_else(|| "N/A".to_string());

                let return_str = p
                    .unrealized_pl_pct
                    .map(|pct: Decimal| {
                        let colored = if pct >= Decimal::ZERO {
                            format!("{:.2}%", pct).green().to_string()
                        } else {
                            format!("{:.2}%", pct).red().to_string()
                        };
                        colored
                    })
                    .unwrap_or_else(|| "N/A".to_string());

                PositionRow {
                    ticker: p.asset.ticker.clone(),
                    quantity: format!("{:.2}", p.quantity),
                    avg_cost: format_currency(p.average_cost),
                    total_cost: format_currency(p.total_cost),
                    price: price_str,
                    value: value_str,
                    pl: pl_str,
                    return_pct: return_str,
                }
            })
            .collect();

        let mut table = Table::new(&rows);
        table.with(Style::modern());
        // Right-align all columns except Ticker (0)
        table.modify(Columns::new(1..), Alignment::right());

        output.push_str(&table.to_string());

        // Display subtotals for this asset type
        output.push_str(&format!("\n{} Subtotal", "─".repeat(40).bright_black()));
        output.push_str(&format!(
            "\n  Cost: {}  |  Value: {}  |  ",
            format_currency(subtotal_cost),
            format_currency(subtotal_value)
        ));

        let pl_colored = if subtotal_pl >= Decimal::ZERO {
            format!("P&L: {}", format_currency(subtotal_pl)).green()
        } else {
            format!("P&L: {}", format_currency(subtotal_pl)).red()
        };
        output.push_str(&pl_colored);

        let return_colored = if subtotal_pl_pct >= Decimal::ZERO {
            format!(" ({:.2}%)", subtotal_pl_pct).green()
        } else {
            format!(" ({:.2}%)", subtotal_pl_pct).red()
        };
        output.push_str(&return_colored);
        output.push('\n');
    }

    // Display overall summary
    output.push_str(&format!(
        "\n\n{} Portfolio Summary",
        "━".repeat(80).bright_black()
    ));
    output.push_str(&format!(
        "\n{:<20} {}",
        "Total Cost:".bold(),
        format_currency(report.total_cost)
    ));
    output.push_str(&format!(
        "\n{:<20} {}",
        "Total Value:".bold(),
        format_currency(report.total_value)
    ));

    let pl_colored = if report.total_pl >= Decimal::ZERO {
        format_currency(report.total_pl).green()
    } else {
        format_currency(report.total_pl).red()
    };
    output.push_str(&format!("\n{:<20} {}", "Total P&L:".bold(), pl_colored));

    let return_colored = if report.total_pl_pct >= Decimal::ZERO {
        format!("{:.2}%", report.total_pl_pct).green()
    } else {
        format!("{:.2}%", report.total_pl_pct).red()
    };
    output.push_str(&format!(
        "\n{:<20} {}\n",
        "Total Return:".bold(),
        return_colored
    ));

    output
}

/// Get friendly name for asset type
fn asset_type_name(asset_type: &AssetType) -> &'static str {
    match asset_type {
        AssetType::Stock => "Stocks",
        AssetType::Etf => "ETFs",
        AssetType::Fii => "Real Estate Funds",
        AssetType::Fiagro => "Agribusiness Funds",
        AssetType::FiInfra => "Infrastructure Funds",
        AssetType::Bond => "Corporate Bonds",
        AssetType::GovBond => "Government Bonds",
    }
}

/// Format empty portfolio message
pub fn format_empty_portfolio() -> String {
    format!(
        "{} No positions found\nImport transactions first using: {} import <file>\n",
        "ℹ".blue().bold(),
        "interest".bold()
    )
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

        let output = format_portfolio_table(&report, None);

        // Verify grouping by asset type
        assert!(output.contains("## Stocks (STOCK)"));
        assert!(output.contains("## Real Estate Funds (FII)"));

        // Verify both groups are present
        let stocks_idx = output.find("## Stocks").unwrap();
        let fii_idx = output.find("## Real Estate Funds").unwrap();
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

        let output = format_portfolio_table(&report, None);

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

        let output = format_portfolio_table(&report, None);

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
    fn test_portfolio_filter_shows_single_asset_type() {
        control::set_override(false); // Disable colors for testing

        let positions = [
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
            create_test_position(
                "BRCR11",
                AssetType::Fii,
                Decimal::from(20),
                Decimal::from(110),
            ),
        ];

        // Filter to only stocks
        let stock_positions: Vec<_> = positions
            .iter()
            .filter(|p| p.asset.asset_type == AssetType::Stock)
            .cloned()
            .collect();
        let total_cost: Decimal = stock_positions.iter().map(|p| p.total_cost).sum();
        let total_value: Decimal = stock_positions
            .iter()
            .map(|p| p.current_value.unwrap_or_default())
            .sum();

        let report = PortfolioReport {
            positions: stock_positions,
            total_cost,
            total_value,
            total_pl: total_value - total_cost,
            total_pl_pct: ((total_value - total_cost) / total_cost) * Decimal::from(100),
        };

        let output = format_portfolio_table(&report, Some("STOCK"));

        // Should only show Stocks group
        assert!(
            output.contains("## Stocks (STOCK)"),
            "Should contain Stocks group"
        );
        assert!(
            !output.contains("## Real Estate Funds"),
            "Should not contain FII group"
        );
        assert!(!output.contains("HFOF11"), "Should not contain FII ticker");
        assert!(output.contains("PETR4"), "Should contain stock ticker");
        assert!(
            output.contains("VALVE3") || output.contains("VALE3"),
            "Should contain stock ticker"
        );
    }

    #[test]
    fn test_portfolio_shows_overall_summary() {
        control::set_override(false); // Disable colors for testing

        let positions = vec![
            create_test_position(
                "PETR4",
                AssetType::Stock,
                Decimal::from(100),
                Decimal::from(20),
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

        let output = format_portfolio_table(&report, None);

        // Verify overall summary section
        assert!(
            output.contains("Portfolio Summary"),
            "Should contain overall summary section"
        );
        assert!(output.contains("Total Cost:"), "Should show total cost");
        assert!(output.contains("Total Value:"), "Should show total value");
        assert!(output.contains("Total P&L:"), "Should show total P&L");
        assert!(
            output.contains("Total Return:"),
            // No colors used in this test
            "Should show total return percentage"
        );
    }

    #[test]
    fn test_asset_type_name_mapping() {
        assert_eq!(asset_type_name(&AssetType::Stock), "Stocks");
        assert_eq!(asset_type_name(&AssetType::Etf), "ETFs");
        assert_eq!(asset_type_name(&AssetType::Fii), "Real Estate Funds");
        assert_eq!(asset_type_name(&AssetType::Fiagro), "Agribusiness Funds");
        assert_eq!(asset_type_name(&AssetType::FiInfra), "Infrastructure Funds");
        assert_eq!(asset_type_name(&AssetType::Bond), "Corporate Bonds");
        assert_eq!(asset_type_name(&AssetType::GovBond), "Government Bonds");
    }
}
