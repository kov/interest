use anyhow::Result;

use crate::reports::period::parse_date_flexible;
use crate::reports::scope::Scope;
use crate::{db, formatters, reports};
use formatters::portfolio::format_empty_portfolio;

/// Filter a portfolio report to a single ticker and recalculate totals.
fn apply_single_asset_filter(report: &mut reports::PortfolioReport, ticker: &str) {
    let ticker_upper = ticker.to_uppercase();
    report.positions.retain(|p| p.asset.ticker == ticker_upper);
    report.total_cost = report.positions.iter().map(|p| p.total_cost).sum();
    report.total_value = report
        .positions
        .iter()
        .map(|p| p.current_value.unwrap_or_default())
        .sum();
    report.total_pl = report.total_value - report.total_cost;
    report.total_pl_pct = if report.total_cost > rust_decimal::Decimal::ZERO {
        (report.total_pl / report.total_cost) * rust_decimal::Decimal::from(100)
    } else {
        rust_decimal::Decimal::ZERO
    };
}

/// Resolve a period string to an as-of date for portfolio commands.
/// Portfolio interprets periods as "as of the end date".
fn resolve_as_of_date(period: Option<&str>) -> Result<Option<chrono::NaiveDate>> {
    let period_str = match period {
        Some(p) => p,
        None => return Ok(None), // default: today (current portfolio)
    };

    let parsed = reports::parse_period(period_str)?;
    let today = chrono::Local::now().date_naive();
    let (_, end_date) = crate::reports::performance::get_period_dates(parsed, None, today)?;
    if end_date > today {
        anyhow::bail!(
            "Period end date is in the future ({}). Use a past date or omit for current portfolio.",
            end_date
        );
    }
    if end_date == today {
        Ok(None) // current portfolio
    } else {
        Ok(Some(end_date))
    }
}

/// Resolve scope and as-of date from a CLI subcommand, then run the portfolio show logic.
pub async fn dispatch_portfolio(
    action: &crate::cli::PortfolioCommands,
    options: crate::options::OutputOptions,
) -> Result<()> {
    match action {
        crate::cli::PortfolioCommands::Show {
            period,
            asset_type,
            at,
        } => {
            // Handle legacy --at flag: takes priority over period positional
            let as_of_date = if let Some(at_str) = at {
                let date = parse_date_flexible(at_str)?;
                Some(date)
            } else {
                resolve_as_of_date(period.as_deref())?
            };

            // Handle legacy --asset-type flag
            let scope = if let Some(at) = asset_type {
                Scope::AssetType(*at)
            } else {
                Scope::Portfolio
            };

            dispatch_portfolio_show(scope, as_of_date, options).await
        }
        crate::cli::PortfolioCommands::Type { asset_type, period } => {
            let as_of_date = resolve_as_of_date(period.as_deref())?;
            dispatch_portfolio_show(Scope::AssetType(*asset_type), as_of_date, options).await
        }
        crate::cli::PortfolioCommands::Asset { ticker, period } => {
            let as_of_date = resolve_as_of_date(period.as_deref())?;
            dispatch_portfolio_show(Scope::SingleAsset(ticker.clone()), as_of_date, options).await
        }
    }
}

async fn dispatch_portfolio_show(
    scope: Scope,
    as_of_date: Option<chrono::NaiveDate>,
    options: crate::options::OutputOptions,
) -> Result<()> {
    tracing::info!("Generating portfolio report");

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

    // Validate date is not in the future
    let historical_date = if let Some(date) = as_of_date {
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

    // Derive asset type filter from scope
    let asset_type_filter = match &scope {
        Scope::AssetType(at) => Some(*at),
        _ => None,
    };

    // Get earliest transaction date to determine price range needed
    let earliest_date = db::get_earliest_transaction_date(&conn)?;
    if earliest_date.is_none() {
        if !options.is_json() {
            options.writer().writeln(&format_empty_portfolio())?;
        }
        return Ok(());
    }

    let today = chrono::Local::now().date_naive();

    // Calculate portfolio positions first (fast, no network calls)
    let mut report = if let Some(date) = historical_date {
        reports::calculate_portfolio_at_date(&conn, date, asset_type_filter.as_ref())?
    } else {
        reports::calculate_portfolio(&conn, asset_type_filter.as_ref())?
    };

    // For single-asset scope, filter positions to just that ticker
    if let Scope::SingleAsset(ref ticker) = scope {
        apply_single_asset_filter(&mut report, ticker);
        if report.positions.is_empty() {
            if !options.is_json() {
                options.writer().writeln(&format!(
                    "\nNo position found for {}.\n",
                    ticker.to_uppercase()
                ))?;
            }
            return Ok(());
        }
    }

    if report.positions.is_empty() {
        if !options.is_json() {
            options.writer().writeln(&format_empty_portfolio())?;
        }
        return Ok(());
    }

    // Now fetch prices ONLY for assets that have current positions
    {
        let assets_with_positions: Vec<_> =
            report.positions.iter().map(|p| p.asset.clone()).collect();
        let price_range = if let Some(date) = historical_date {
            (date, date)
        } else {
            (today, today)
        };

        super::prices_ui::ensure_prices_with_ui(
            &mut conn,
            &assets_with_positions,
            price_range,
            &options,
        )
        .await?;

        // Recalculate with updated prices
        report = if let Some(date) = historical_date {
            reports::calculate_portfolio_at_date(&conn, date, asset_type_filter.as_ref())?
        } else {
            reports::calculate_portfolio(&conn, asset_type_filter.as_ref())?
        };

        if let Scope::SingleAsset(ref ticker) = scope {
            apply_single_asset_filter(&mut report, ticker);
        }
    }

    // Compute income YTD enrichment data (bounded to snapshot date)
    let enrichment = compute_income_enrichment(&conn, &report, historical_date);

    // Print portfolio output (JSON or table)
    let output = formatters::portfolio::format_enriched(
        &report,
        asset_type_filter,
        enrichment.as_ref(),
        options.clone(),
    )?;
    options.writer().writeln(&output)?;

    Ok(())
}

/// Compute income YTD enrichment data for portfolio display.
/// When `as_of_date` is provided (historical view), income is bounded to that date's year.
fn compute_income_enrichment(
    conn: &rusqlite::Connection,
    report: &reports::PortfolioReport,
    as_of_date: Option<chrono::NaiveDate>,
) -> Option<formatters::portfolio::PortfolioEnrichment> {
    use chrono::Datelike;
    use rust_decimal::Decimal;
    use std::collections::HashMap;

    let end_date = as_of_date.unwrap_or_else(|| chrono::Local::now().date_naive());
    let year_start = chrono::NaiveDate::from_ymd_opt(end_date.year(), 1, 1)?;

    let events =
        db::get_income_events_with_assets(conn, Some(year_start), Some(end_date), None).ok()?;
    if events.is_empty() {
        return None;
    }

    // Only include tickers that are in the current portfolio
    let portfolio_tickers: std::collections::HashSet<&str> = report
        .positions
        .iter()
        .map(|p| p.asset.ticker.as_str())
        .collect();

    let mut income_by_ticker: HashMap<String, Decimal> = HashMap::new();
    let mut income_by_type: HashMap<db::AssetType, Decimal> = HashMap::new();
    let mut total_income = Decimal::ZERO;

    for (event, asset) in &events {
        if !portfolio_tickers.contains(asset.ticker.as_str()) {
            continue;
        }
        *income_by_ticker
            .entry(asset.ticker.clone())
            .or_insert(Decimal::ZERO) += event.total_amount;
        *income_by_type
            .entry(asset.asset_type)
            .or_insert(Decimal::ZERO) += event.total_amount;
        total_income += event.total_amount;
    }

    if total_income == Decimal::ZERO {
        return None;
    }

    Some(formatters::portfolio::PortfolioEnrichment {
        income_by_ticker,
        income_by_type,
        total_income,
    })
}
