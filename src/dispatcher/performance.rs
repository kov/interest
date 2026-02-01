//! Performance command dispatcher implementation

use crate::ui::progress::{ProgressEvent, ProgressPrinter};
use crate::{db, formatters, options, reports};
use anyhow::{anyhow, Result};
use chrono::NaiveDate;
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

pub async fn dispatch_performance_show(
    period_str: &str,
    options: options::OutputOptions,
) -> Result<()> {
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

            if !options.is_json() && !skip_price_fetch {
                let total = priceable_assets.len();
                let printer = ProgressPrinter::new(options);

                // Show initial spinner
                printer.handle_event(&ProgressEvent::Spinner {
                    message: format!("Fetching prices 0/{}...", total),
                });

                crate::pricing::resolver::ensure_prices_available_with_progress(
                    &mut conn,
                    &assets,
                    (price_start, today),
                    options,
                    |event| {
                        // For ticker results, also update the spinner with current count
                        match event {
                            ProgressEvent::TickerResult {
                                ticker,
                                price,
                                current,
                                total,
                            } => {
                                let masked = if options.is_private() {
                                    ProgressEvent::TickerResult {
                                        ticker: ticker.clone(),
                                        price: price
                                            .as_ref()
                                            .map(|_| "R$ ***".to_string())
                                            .map_err(|err| err.clone()),
                                        current: *current,
                                        total: *total,
                                    }
                                } else {
                                    ProgressEvent::TickerResult {
                                        ticker: ticker.clone(),
                                        price: price.clone(),
                                        current: *current,
                                        total: *total,
                                    }
                                };
                                printer.handle_event(&masked);
                                printer.handle_event(&ProgressEvent::Spinner {
                                    message: format!("Fetching prices {}/{}...", current, total),
                                });
                            }
                            _ => printer.handle_event(event),
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
                    options,
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

    // Print performance output (JSON or table)
    formatters::performance::print(&report, options);

    Ok(())
}

pub async fn dispatch_performance(
    action: &crate::cli::PerformanceCommands,
    options: options::OutputOptions,
) -> Result<()> {
    match action {
        crate::cli::PerformanceCommands::Show { period } => {
            dispatch_performance_show(period, options).await
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
