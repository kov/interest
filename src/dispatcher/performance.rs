//! Performance command dispatcher implementation

use crate::reports::scope::Scope;
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
    let (period_start, period_end) =
        crate::reports::performance::get_period_dates(period.clone(), Some(&conn))?;

    // Ensure prices are available for the required date range
    let assets = db::get_assets_with_transactions(&conn)?;
    if !assets.is_empty() {
        let earliest = db::get_earliest_transaction_date(&conn)?;
        if let Some(earliest_date) = earliest {
            let price_start = std::cmp::max(earliest_date, period_start);
            super::prices_ui::ensure_prices_with_ui(
                &mut conn,
                &assets,
                (price_start, period_end),
                &options,
            )
            .await?;
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
