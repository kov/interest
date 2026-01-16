//! Performance command dispatcher implementation

use crate::ui::progress::ProgressPrinter;
use crate::utils::{format_currency, format_currency_aligned};
use crate::{db, reports};
use anyhow::{anyhow, Result};
use chrono::NaiveDate;
use colored::Colorize;
use tracing;

/// Parse a period string (MTD, QTD, YTD, 1Y, ALL, YYYY, or from:to)
fn parse_period_string(period: &str) -> Result<reports::Period> {
    let upper = period.to_uppercase();
    match upper.as_str() {
        "MTD" => Ok(reports::Period::Mtd),
        "QTD" => Ok(reports::Period::Qtd),
        "YTD" => Ok(reports::Period::Ytd),
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

    // Get blocked assets (those with open blocking inconsistencies)
    let blocked_assets = db::get_blocked_assets(&conn)?;
    let blocked_tickers: Vec<&str> = blocked_assets.iter().map(|(_, t)| t.as_str()).collect();

    if !blocked_tickers.is_empty() {
        anyhow::bail!(
            "Refusing to show performance due to open blocking inconsistencies.\nAssets: {}\nResolve with `inconsistencies resolve`.",
            blocked_tickers.join(", ")
        );
    }

    let period = parse_period_string(period_str)?;
    // Determine period boundaries (used for price range limiting)
    let (period_start, period_end) =
        crate::reports::performance::get_period_dates(period.clone(), Some(&conn))?;
    // Allow disabling live price fetching via env var (mirrors portfolio command)
    let skip_price_fetch = std::env::var("INTEREST_SKIP_PRICE_FETCH")
        .map(|v| v != "0")
        .unwrap_or(false);

    // Ensure prices are available for the required date range
    // Filter out blocked assets
    let assets = db::get_assets_with_transactions(&conn)?;
    let priceable_assets = crate::pricing::resolver::filter_priceable_assets(&assets);
    if !assets.is_empty() {
        // Get the date range for prices
        let earliest = db::get_earliest_transaction_date(&conn)?;
        if let Some(earliest_date) = earliest {
            // Limit price resolution to the end of the requested period
            let today = period_end;
            let price_start = std::cmp::max(earliest_date, period_start);

            if !json_output && !skip_price_fetch {
                let total = priceable_assets.len();
                let printer = ProgressPrinter::new(json_output);
                let mut completed = 0usize;

                // Show initial spinner
                printer.update(&format!("Fetching prices 0/{}...", total));

                crate::pricing::resolver::ensure_prices_available_with_progress(
                    &mut conn,
                    &assets,
                    (price_start, today),
                    |event| {
                        let (raw_text, should_persist) = match event {
                            crate::ui::progress::ProgressEvent::Line { text, persist } => {
                                (text.clone(), *persist)
                            }
                        };
                        let msg = raw_text.as_str();

                        // Check if this is a ticker result (contains "→")
                        if msg.contains("→") {
                            // Parse completion count from message like "TICKER → R$ XX.XX (N/M)"
                            if let Some(paren_start) = msg.rfind('(') {
                                if let Some(slash) = msg[paren_start..].find('/') {
                                    if let Ok(n) =
                                        msg[paren_start + 1..paren_start + slash].parse::<usize>()
                                    {
                                        completed = n;
                                    }
                                }
                            }

                            // Print ticker result, re-draw spinner message
                            printer.handle_event(crate::ui::progress::ProgressEvent::Line {
                                text: msg.to_string(),
                                persist: true,
                            });
                            printer.update(&format!("Fetching prices {}/{}...", completed, total));
                        } else if msg.starts_with("✓") {
                            printer.handle_event(crate::ui::progress::ProgressEvent::Line {
                                text: msg.to_string(),
                                persist: true,
                            });
                        } else {
                            printer.handle_event(crate::ui::progress::ProgressEvent::Line {
                                text: raw_text,
                                persist: should_persist,
                            });
                        }
                    },
                )
                .await
                .or_else(|e: anyhow::Error| {
                    tracing::warn!("Price resolution failed: {}", e);
                    // Continue anyway - performance calculation will use available prices
                    Ok::<(), anyhow::Error>(())
                })?;
            } else if !skip_price_fetch {
                // JSON mode: no spinner, just fetch silently
                crate::pricing::resolver::ensure_prices_available(
                    &mut conn,
                    &assets,
                    (price_start, today),
                )
                .await
                .or_else(|e: anyhow::Error| {
                    tracing::warn!("Price resolution failed: {}", e);
                    // Continue anyway - performance calculation will use available prices
                    Ok::<(), anyhow::Error>(())
                })?;
            }
        }
    }

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
            format_currency(report.start_value).cyan()
        );
        println!(
            "  End Value:        {}",
            format_currency(report.end_value).cyan()
        );
        println!();

        let return_color = if report.total_return >= rust_decimal::Decimal::ZERO {
            "green"
        } else {
            "red"
        };

        let return_str = format_currency(report.total_return);
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
            format_currency(report.realized_gains).yellow()
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
            format_currency(unrealized_display).blue()
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
                format_currency(cf.total_contributions).green()
            );
            println!(
                "    Withdrawals:   {}",
                format_currency(cf.total_withdrawals).red()
            );
            println!("    Net Flow:      {}", format_currency(cf.net_flow).cyan());
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
                    format_currency_aligned(perf.start_value, 16).dimmed(),
                    format_currency_aligned(perf.end_value, 16).cyan(),
                    return_display
                );
            }
        }

        println!();
    }

    Ok(())
}

pub async fn dispatch_performance(
    action: crate::commands::PerformanceAction,
    json_output: bool,
) -> Result<()> {
    match action {
        crate::commands::PerformanceAction::Show { period } => {
            dispatch_performance_show(&period, json_output).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_period_mtd() {
        let period = parse_period_string("MTD").unwrap();
        assert!(matches!(period, reports::Period::Mtd));
    }

    #[test]
    fn test_parse_period_ytd() {
        let period = parse_period_string("ytd").unwrap();
        assert!(matches!(period, reports::Period::Ytd));
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
