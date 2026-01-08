//! Performance command dispatcher implementation

use crate::{db, reports};
use anyhow::{anyhow, Result};
use chrono::NaiveDate;
use colored::Colorize;

/// Parse a period string (MTD, QTD, YTD, 1Y, ALL, YYYY, or from:to)
fn parse_period_string(period: &str) -> Result<reports::Period> {
    let upper = period.to_uppercase();
    match upper.as_str() {
        "MTD" => Ok(reports::Period::MTD),
        "QTD" => Ok(reports::Period::QTD),
        "YTD" => Ok(reports::Period::YTD),
        "1Y" | "ONEYEAR" => Ok(reports::Period::OneYear),
        "ALL" | "ALLTIME" => Ok(reports::Period::AllTime),
        _ => {
            // Try parsing as year shorthand: YYYY -> YYYY-01-01:YYYY-12-31
            if let Ok(year) = period.parse::<i32>() {
                if (1900..=2100).contains(&year) {
                    let from = NaiveDate::from_ymd_opt(year, 1, 1)
                        .ok_or_else(|| anyhow!("Invalid year: {}", year))?;
                    let to = NaiveDate::from_ymd_opt(year, 12, 31)
                        .ok_or_else(|| anyhow!("Invalid year: {}", year))?;
                    return Ok(reports::Period::Custom { from, to });
                }
            }

            // Try parsing as custom range: YYYY-MM-DD:YYYY-MM-DD
            if let Some((from_str, to_str)) = period.split_once(':') {
                let from = NaiveDate::parse_from_str(from_str, "%Y-%m-%d").map_err(|_| {
                    anyhow!("Invalid from date: {}. Use YYYY-MM-DD format.", from_str)
                })?;
                let to = NaiveDate::parse_from_str(to_str, "%Y-%m-%d")
                    .map_err(|_| anyhow!("Invalid to date: {}. Use YYYY-MM-DD format.", to_str))?;
                Ok(reports::Period::Custom { from, to })
            } else {
                Err(anyhow!(
                    "Invalid period '{}'. Use: MTD, QTD, YTD, 1Y, ALL, YYYY, or from:to (YYYY-MM-DD:YYYY-MM-DD)",
                    period
                ))
            }
        }
    }
}

pub async fn dispatch_performance_show(period_str: &str, json_output: bool) -> Result<()> {
    db::init_database(None)?;
    let mut conn = db::open_db(None)?;

    let period = parse_period_string(period_str)?;

    // Calculate performance
    let report = reports::calculate_performance(&mut conn, period)?;

    if json_output {
        let payload = serde_json::json!({
            "start_date": report.start_date,
            "end_date": report.end_date,
            "start_value": report.start_value,
            "end_value": report.end_value,
            "total_return": report.total_return,
            "total_return_pct": report.return_pct(),
            "realized_gains": report.realized_gains,
            "unrealized_gains": report.unrealized_gains,
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else {
        println!("\n{} Performance Report", "📈".cyan().bold());
        println!("  Period: {} → {}", report.start_date, report.end_date);
        println!();
        println!(
            "  Start Value:      {}",
            format!("R$ {:.2}", report.start_value).cyan()
        );
        println!(
            "  End Value:        {}",
            format!("R$ {:.2}", report.end_value).cyan()
        );
        println!();

        let return_color = if report.total_return >= rust_decimal::Decimal::ZERO {
            "green"
        } else {
            "red"
        };

        let return_str = format!("R$ {:.2}", report.total_return);
        let return_pct_str = format!("{:.2}%", report.return_pct());

        // Show whether this includes cash flows or is pure return
        let growth_label = if report.cash_flows.is_some() {
            "Portfolio Growth:"
        } else {
            "Total Return:    "
        };

        match return_color {
            "green" => {
                println!(
                    "  {} {} ({})",
                    growth_label,
                    return_str.green(),
                    return_pct_str.green()
                );
            }
            _ => {
                println!(
                    "  {} {} ({})",
                    growth_label,
                    return_str.red(),
                    return_pct_str.red()
                );
            }
        }

        // Show TWR explicitly when cash flows present
        if report.cash_flows.is_some() {
            let twr_color = if report.time_weighted_return >= rust_decimal::Decimal::ZERO {
                "green"
            } else {
                "red"
            };
            let twr_str = format!("{:.2}%", report.time_weighted_return);
            match twr_color {
                "green" => {
                    println!("  Investment Return: {}", twr_str.green());
                }
                _ => {
                    println!("  Investment Return: {}", twr_str.red());
                }
            }
        }

        println!(
            "  Realized Gains:   {}",
            format!("R$ {:.2}", report.realized_gains).yellow()
        );

        // Normalize -0.00 to 0.00 for display (handle Decimal precision quirks)
        let unrealized_display = if report.unrealized_gains.abs() < rust_decimal::Decimal::new(1, 2)
        {
            rust_decimal::Decimal::ZERO
        } else {
            report.unrealized_gains
        };
        println!(
            "  Unrealized Gains: {}",
            format!("R$ {:.2}", unrealized_display).blue()
        );

        // Show cash flow summary if available
        if let Some(ref cf) = report.cash_flows {
            println!();
            println!(
                "  {} Cash Flows ({} transactions)",
                "💰".cyan().bold(),
                cf.flow_count
            );
            println!(
                "    Contributions: {}",
                format!("R$ {:.2}", cf.total_contributions).green()
            );
            println!(
                "    Withdrawals:   {}",
                format!("R$ {:.2}", cf.total_withdrawals).red()
            );
            println!(
                "    Net Flow:      {}",
                format!("R$ {:.2}", cf.net_flow).cyan()
            );
        }

        // Show asset type breakdown
        if !report.asset_breakdown.is_empty() {
            println!();
            println!("  {} By Asset Type", "📊".cyan().bold());

            // Sort by start value (largest positions first)
            let mut breakdown_vec: Vec<_> = report.asset_breakdown.iter().collect();
            breakdown_vec.sort_by(|a, b| b.1.start_value.cmp(&a.1.start_value));

            for (asset_type, perf) in breakdown_vec {
                let return_display = if perf.return_pct >= rust_decimal::Decimal::ZERO {
                    format!("{:>7.2}%", perf.return_pct).green()
                } else {
                    format!("{:>7.2}%", perf.return_pct).red()
                };

                println!(
                    "    {:12} {} → {}  {}",
                    format!("{:?}", asset_type),
                    format!("R$ {:>10.2}", perf.start_value).dimmed(),
                    format!("R$ {:>10.2}", perf.end_value).cyan(),
                    return_display
                );
            }
        }

        println!();
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_period_mtd() {
        let period = parse_period_string("MTD").unwrap();
        assert!(matches!(period, reports::Period::MTD));
    }

    #[test]
    fn test_parse_period_ytd() {
        let period = parse_period_string("ytd").unwrap();
        assert!(matches!(period, reports::Period::YTD));
    }

    #[test]
    fn test_parse_period_custom() {
        let period = parse_period_string("2024-01-01:2024-12-31").unwrap();
        if let reports::Period::Custom { from, to } = period {
            assert_eq!(from, NaiveDate::from_ymd_opt(2024, 1, 1).unwrap());
            assert_eq!(to, NaiveDate::from_ymd_opt(2024, 12, 31).unwrap());
        } else {
            panic!("Expected Custom period");
        }
    }

    #[test]
    fn test_parse_period_invalid() {
        let result = parse_period_string("INVALID");
        assert!(result.is_err());
    }
}
