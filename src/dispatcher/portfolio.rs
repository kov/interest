use anyhow::Result;
use colored::Colorize;

use crate::reports::portfolio::calculate_allocation;
use crate::ui::progress::ProgressPrinter;
use crate::utils::format_currency;
use crate::{cli, db, reports};

pub async fn dispatch_portfolio_show(
    asset_type: Option<&str>,
    as_of_date: Option<&str>,
    json_output: bool,
) -> Result<()> {
    tracing::info!("Generating portfolio report");

    // Note: kept as a focused function to do the actual "show" work.
    // A thin dispatcher wrapper `dispatch_portfolio` will route portfolio actions
    // to this function; this keeps the public handler API stable and testable.

    // Initialize database
    db::init_database(None)?;
    let mut conn = db::open_db(None)?;

    // Get blocked assets (those with open blocking inconsistencies)
    let blocked_assets = db::get_blocked_assets(&conn)?;
    if !blocked_assets.is_empty() {
        let blocked_tickers: Vec<&str> = blocked_assets.iter().map(|(_, t)| t.as_str()).collect();
        anyhow::bail!(
            "Refusing to show portfolio due to open blocking inconsistencies.\nAssets: {}\nResolve with `inconsistencies resolve`.",
            blocked_tickers.join(", ")
        );
    }

    // Parse date if provided (already validated by parse_flexible_date in commands.rs)
    let historical_date = if let Some(date_str) = as_of_date {
        let date = chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d")
            .map_err(|e| anyhow::anyhow!("Invalid date '{}': {}", date_str, e))?;
        let today = chrono::Local::now().date_naive();
        if date > today {
            return Err(anyhow::anyhow!(
                "Date cannot be in the future (today is {})",
                today
            ));
        }
        Some(date)
    } else {
        None
    };

    // Allow disabling live price fetching via env var
    let skip_price_fetch = std::env::var("INTEREST_SKIP_PRICE_FETCH")
        .map(|v| v != "0")
        .unwrap_or(false);

    // Parse asset type filter if provided
    let asset_type_filter = if let Some(type_str) = asset_type {
        Some(
            type_str
                .parse::<db::AssetType>()
                .map_err(|_| anyhow::anyhow!("Invalid asset type: {}", type_str))?,
        )
    } else {
        None
    };

    // Get earliest transaction date to determine price range needed
    let earliest_date = db::get_earliest_transaction_date(&conn)?;
    if earliest_date.is_none() {
        // No transactions - nothing to show
        if !json_output {
            println!("{}", cli::formatters::format_empty_portfolio());
        }
        return Ok(());
    }

    let today = chrono::Local::now().date_naive();

    // Calculate portfolio positions first (fast, no network calls)
    // Make mutable so we can re-run after fetching current prices to include
    // up-to-date market values in the printed report.
    let mut report = if let Some(date) = historical_date {
        reports::calculate_portfolio_at_date(&conn, date, asset_type_filter.as_ref())?
    } else {
        reports::calculate_portfolio(&conn, asset_type_filter.as_ref())?
    };

    if report.positions.is_empty() {
        if !json_output {
            println!("{}", cli::formatters::format_empty_portfolio());
        }
        return Ok(());
    }

    // Now fetch prices ONLY for assets that have current positions
    if !skip_price_fetch {
        if !json_output {
            let assets_with_positions: Vec<_> =
                report.positions.iter().map(|p| p.asset.clone()).collect();
            let priceable_assets =
                crate::pricing::resolver::filter_priceable_assets(&assets_with_positions);

            if !priceable_assets.is_empty() {
                let total = priceable_assets.len();
                let printer = ProgressPrinter::new(json_output);
                let mut completed = 0usize;
                let price_range = if let Some(date) = historical_date {
                    (date, date)
                } else {
                    (today, today)
                };

                // Show initial spinner
                printer.update(&format!("Fetching prices 0/{}...", total));

                crate::pricing::resolver::ensure_prices_available_with_progress(
                    &mut conn,
                    &priceable_assets,
                    price_range,
                    |event| {
                        // Map typed event to legacy display values
                        let (raw_text, should_persist) = match event {
                            crate::ui::progress::ProgressEvent::Line { text, persist } => {
                                (text.clone(), *persist)
                            }
                        };
                        let msg_content = raw_text.as_str();

                        // Check if this is a ticker result (contains "→")
                        if let Some(count) = crate::dispatcher::parse_progress_count(msg_content) {
                            completed = count;
                            // Print ticker result, re-draw spinner message
                            printer.handle_event(crate::ui::progress::ProgressEvent::Line {
                                text: msg_content.to_string(),
                                persist: true,
                            });
                            printer.update(&format!("Fetching prices {}/{}...", completed, total));
                        } else if should_persist {
                            printer.handle_event(crate::ui::progress::ProgressEvent::Line {
                                text: msg_content.to_string(),
                                persist: true,
                            });

                            printer.update(&format!("Fetching prices {}/{}...", completed, total));
                        } else {
                            // Be permissive for non-persistent progress messages: show any
                            // transient activity so the user sees progress instead of a
                            // stagnant spinner.
                            if !msg_content.trim().is_empty() {
                                printer.handle_event(crate::ui::progress::ProgressEvent::Line {
                                    text: raw_text,
                                    persist: should_persist,
                                });
                            }
                        }
                    },
                )
                .await
                .or_else(|e: anyhow::Error| {
                    tracing::warn!("Price resolution failed: {}", e);
                    // Continue anyway - use available prices
                    Ok::<(), anyhow::Error>(())
                })?;

                printer.finish(true, "Price resolution complete");

                // Recompute the portfolio now that current prices have been fetched
                // so displayed values (Price, Value, P&L) reflect the latest data.
                report = if let Some(date) = historical_date {
                    // Historical view - prices shouldn't have changed, but keep behavior consistent
                    reports::calculate_portfolio_at_date(&conn, date, asset_type_filter.as_ref())?
                } else {
                    reports::calculate_portfolio(&conn, asset_type_filter.as_ref())?
                };
            }
        } else {
            // JSON mode: no spinner, just fetch silently
            let assets_with_positions: Vec<_> =
                report.positions.iter().map(|p| p.asset.clone()).collect();
            let priceable_assets =
                crate::pricing::resolver::filter_priceable_assets(&assets_with_positions);

            if !priceable_assets.is_empty() {
                let price_range = if let Some(date) = historical_date {
                    (date, date)
                } else {
                    (today, today)
                };

                crate::pricing::resolver::ensure_prices_available(
                    &mut conn,
                    &priceable_assets,
                    price_range,
                )
                .await
                .or_else(|e: anyhow::Error| {
                    tracing::warn!("Price resolution failed: {}", e);
                    // Continue anyway - use available prices
                    Ok::<(), anyhow::Error>(())
                })?;
            }

            // Recompute for JSON mode as well so JSON output contains updated prices
            report = if let Some(date) = historical_date {
                reports::calculate_portfolio_at_date(&conn, date, asset_type_filter.as_ref())?
            } else {
                reports::calculate_portfolio(&conn, asset_type_filter.as_ref())?
            };
        }
    }

    if json_output {
        println!("{}", cli::formatters::format_portfolio_json(&report));
    } else {
        println!(
            "{}",
            cli::formatters::format_portfolio_table(&report, asset_type)
        );

        // Display asset allocation if showing full portfolio
        if asset_type_filter.is_none() {
            let allocation = calculate_allocation(&report);

            if allocation.len() > 1 {
                println!("\n{} Asset Allocation", "🎯".cyan().bold());

                let mut alloc_vec: Vec<_> = allocation.iter().collect();
                alloc_vec.sort_by(|a, b| b.1 .0.cmp(&a.1 .0));

                for (asset_type, (value, pct)) in alloc_vec {
                    let type_ref: &db::AssetType = asset_type;
                    println!(
                        "  {}: {} ({:.2}%)",
                        type_ref.as_str().to_uppercase(),
                        format_currency(*value).cyan(),
                        pct
                    );
                }
            }
        }
    }

    Ok(())
}

// Top-level dispatcher for portfolio sub-commands
pub async fn dispatch_portfolio(
    action: crate::commands::PortfolioAction,
    json_output: bool,
) -> Result<()> {
    match action {
        crate::commands::PortfolioAction::Show { filter, as_of_date } => {
            dispatch_portfolio_show(filter.as_deref(), as_of_date.as_deref(), json_output).await
        }
    }
}
