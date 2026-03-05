//! Performance command dispatcher implementation

use crate::reports::scope::Scope;
use crate::ui::progress::{ProgressEvent, ProgressPrinter};
use crate::{db, formatters, options, reports};
use anyhow::Result;

async fn dispatch_performance_scoped(
    period_str: &str,
    scope: Scope,
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

    let period = reports::parse_period(period_str)?;
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
                let printer = ProgressPrinter::new(&options);

                // Show initial spinner
                printer.handle_event(&ProgressEvent::Spinner {
                    message: format!("Fetching prices 0/{}...", total),
                });

                crate::pricing::resolver::ensure_prices_available_with_progress(
                    &mut conn,
                    &assets,
                    (price_start, today),
                    options.clone(),
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
                    options.clone(),
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

    // Scope the summary metrics to match the requested scope
    let report = scope_performance_report(report, &scope);

    // Print performance output (JSON or table)
    let output = formatters::performance::format(&report, &scope, options.clone())?;
    options.writer().writeln(&output)?;

    Ok(())
}

/// Recompute summary-level metrics to reflect only the requested scope.
///
/// TWR is omitted (set to zero) for scoped views because proper time-weighted
/// return requires scoped cash flows and sub-period snapshots.
fn scope_performance_report(
    mut report: crate::reports::performance::PerformanceReport,
    scope: &Scope,
) -> crate::reports::performance::PerformanceReport {
    use rust_decimal::Decimal;

    match scope {
        Scope::Portfolio => report,
        Scope::AssetType(at) => {
            // Derive summary from the asset-type breakdown entry
            if let Some(perf) = report.asset_breakdown.get(at) {
                report.end_value = perf.market_value;
                report.unrealized_gains = perf.unrealized_pl;
                report.realized_gains = perf.realized_gains;
                report.total_return = perf.unrealized_pl + perf.realized_gains;
                report.start_value = perf.market_value - report.total_return;
            } else {
                report.start_value = Decimal::ZERO;
                report.end_value = Decimal::ZERO;
                report.total_return = Decimal::ZERO;
                report.unrealized_gains = Decimal::ZERO;
                report.realized_gains = Decimal::ZERO;
            }
            // TWR not meaningful without scoped cash flows
            report.time_weighted_return = Decimal::ZERO;
            report.cash_flows = None;
            report
        }
        Scope::SingleAsset(ticker) => {
            let upper = ticker.to_uppercase();
            let matching: Vec<_> = report
                .ticker_breakdown
                .iter()
                .filter(|t| t.ticker == upper)
                .collect();

            if let Some(tp) = matching.first() {
                report.end_value = tp.market_value;
                report.unrealized_gains = tp.unrealized_pl;
                // Find realized gains from the asset's type breakdown
                report.realized_gains = report
                    .asset_breakdown
                    .get(&tp.asset_type)
                    .map(|_| Decimal::ZERO) // Per-ticker realized gains not tracked yet
                    .unwrap_or(Decimal::ZERO);
                report.total_return = tp.unrealized_pl + report.realized_gains;
                report.start_value = tp.market_value - report.total_return;
            } else {
                report.start_value = Decimal::ZERO;
                report.end_value = Decimal::ZERO;
                report.total_return = Decimal::ZERO;
                report.unrealized_gains = Decimal::ZERO;
                report.realized_gains = Decimal::ZERO;
            }
            report.time_weighted_return = Decimal::ZERO;
            report.cash_flows = None;
            report
        }
    }
}

pub async fn dispatch_performance(
    action: &crate::cli::PerformanceCommands,
    options: options::OutputOptions,
) -> Result<()> {
    match action {
        crate::cli::PerformanceCommands::Show { period } => {
            let period_str = period.as_deref().unwrap_or("YTD");
            dispatch_performance_scoped(period_str, Scope::Portfolio, options).await
        }
        crate::cli::PerformanceCommands::Type { asset_type, period } => {
            let at = asset_type
                .parse::<db::AssetType>()
                .map_err(|_| anyhow::anyhow!("Invalid asset type: {}", asset_type))?;
            let period_str = period.as_deref().unwrap_or("YTD");
            dispatch_performance_scoped(period_str, Scope::AssetType(at), options).await
        }
        crate::cli::PerformanceCommands::Asset { ticker, period } => {
            let period_str = period.as_deref().unwrap_or("YTD");
            dispatch_performance_scoped(period_str, Scope::SingleAsset(ticker.clone()), options)
                .await
        }
    }
}
