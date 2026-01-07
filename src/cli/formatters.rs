//! Output formatting module for CLI display
//!
//! This module handles all terminal output formatting, separating
//! the concerns of data calculation from presentation.

use crate::reports::PortfolioReport;
use colored::Colorize;
use rust_decimal::Decimal;
use serde::Serialize;
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
            "\n{} Portfolio - {} only\n\n",
            "📊".cyan().bold(),
            filter.to_uppercase()
        ));
    } else {
        output.push_str(&format!("\n{} Complete Portfolio\n\n", "📊".cyan().bold()));
    }

    // Display positions table
    #[derive(Tabled)]
    struct PositionRow {
        #[tabled(rename = "Ticker")]
        ticker: String,
        #[tabled(rename = "Type")]
        asset_type: String,
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

    let rows: Vec<PositionRow> = report
        .positions
        .iter()
        .map(|p| {
            let price_str = p
                .current_price
                .map(|pr: Decimal| format!("R$ {:.2}", pr))
                .unwrap_or_else(|| "N/A".to_string());

            let value_str = p
                .current_value
                .map(|v: Decimal| format!("R$ {:.2}", v))
                .unwrap_or_else(|| "N/A".to_string());

            let pl_str = p
                .unrealized_pl
                .map(|pl: Decimal| {
                    let colored = if pl >= Decimal::ZERO {
                        format!("R$ {:.2}", pl).green().to_string()
                    } else {
                        format!("R$ {:.2}", pl).red().to_string()
                    };
                    colored
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
                asset_type: p.asset.asset_type.as_str().to_string(),
                quantity: format!("{:.2}", p.quantity),
                avg_cost: format!("R$ {:.2}", p.average_cost),
                total_cost: format!("R$ {:.2}", p.total_cost),
                price: price_str,
                value: value_str,
                pl: pl_str,
                return_pct: return_str,
            }
        })
        .collect();

    let mut table = Table::new(&rows);
    table.with(Style::modern());
    // Right-align all columns except Ticker (0) and Type (1)
    table.modify(Columns::new(2..), Alignment::right());

    output.push_str(&table.to_string());

    // Display summary
    output.push_str(&format!("\n\n{} Summary", "━".repeat(80).bright_black()));
    output.push_str(&format!(
        "\n{:<20} R$ {:.2}",
        "Total Cost:".bold(),
        report.total_cost
    ));
    output.push_str(&format!(
        "\n{:<20} R$ {:.2}",
        "Total Value:".bold(),
        report.total_value
    ));

    let pl_colored = if report.total_pl >= Decimal::ZERO {
        format!("R$ {:.2}", report.total_pl).green()
    } else {
        format!("R$ {:.2}", report.total_pl).red()
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

    #[test]
    fn test_empty_portfolio_message() {
        let msg = format_empty_portfolio();
        assert!(msg.contains("No positions found"));
        assert!(msg.contains("import"));
    }
}
