//! Income command dispatcher

use crate::{db, formatters, options, output::OutputDocument};
use anyhow::Result;
use colored::Colorize;
use tracing::info;

pub async fn dispatch_income(
    action: &crate::cli::IncomeCommands,
    options: options::OutputOptions,
) -> Result<()> {
    match action {
        crate::cli::IncomeCommands::Show { period } => {
            let (from_date, to_date, year_val) = resolve_income_period(period.as_deref())?;
            dispatch_income_show_impl(from_date, to_date, year_val, None, options).await
        }
        crate::cli::IncomeCommands::Type { asset_type, period } => {
            let (from_date, to_date, year_val) = resolve_income_period(period.as_deref())?;
            dispatch_income_show_impl(from_date, to_date, year_val, Some(*asset_type), options).await
        }
        crate::cli::IncomeCommands::Asset { ticker, period } => {
            let (from_date, to_date, _year_val) = resolve_income_period(period.as_deref())?;
            dispatch_income_detail_impl(
                Some(from_date),
                Some(to_date),
                Some(ticker.as_str()),
                options,
            )
            .await
        }
        crate::cli::IncomeCommands::Detail { year, asset } => {
            dispatch_income_detail(*year, asset.as_deref(), options).await
        }
        crate::cli::IncomeCommands::Summary {
            year,
            categorize,
            tax_aware,
        } => dispatch_income_summary(*year, *categorize, *tax_aware, options).await,
        crate::cli::IncomeCommands::Add {
            ticker,
            event_type,
            total_amount,
            date,
            ex_date,
            withholding,
            amount_per_quota,
            notes,
        } => {
            dispatch_income_add(
                ticker,
                event_type,
                total_amount,
                date,
                ex_date.as_deref(),
                withholding,
                amount_per_quota,
                notes.as_deref(),
                options,
            )
            .await
        }
        crate::cli::IncomeCommands::Yield {
            ticker, asset_type, ..
        } => dispatch_income_yield(ticker.as_deref(), *asset_type, options).await,
        crate::cli::IncomeCommands::Trends { ticker, months } => {
            dispatch_income_trends(ticker.as_deref(), *months, options).await
        }
        crate::cli::IncomeCommands::Forecast { year, conservative } => {
            dispatch_income_forecast(*year, *conservative, options).await
        }
        crate::cli::IncomeCommands::Calendar { month } => {
            dispatch_income_calendar(month.as_deref(), options).await
        }
        crate::cli::IncomeCommands::Alerts => dispatch_income_alerts(options).await,
        crate::cli::IncomeCommands::Export {
            year,
            format,
            output,
        } => dispatch_income_export(*year, format, output.as_deref(), options).await,
    }
}

/// Resolve income period string to date range and display year.
fn resolve_income_period(
    period: Option<&str>,
) -> Result<(chrono::NaiveDate, chrono::NaiveDate, i32)> {
    use chrono::Datelike;

    let period_str = period.unwrap_or("YTD");
    let parsed = crate::reports::parse_period(period_str)?;
    let (from_date, to_date) = crate::reports::performance::get_period_dates(parsed, None)?;
    let year_val = to_date.year();
    Ok((from_date, to_date, year_val))
}

/// Show income summary by asset, grouped by asset type
async fn dispatch_income_show_impl(
    from_date: chrono::NaiveDate,
    to_date: chrono::NaiveDate,
    year_val: i32,
    asset_type_filter: Option<db::AssetType>,
    options: options::OutputOptions,
) -> Result<()> {
    use rust_decimal::Decimal;
    use std::collections::HashMap;

    info!("Showing income summary by asset");

    db::init_database(None)?;
    let conn = db::open_db(None)?;

    let events = db::get_income_events_with_assets(&conn, Some(from_date), Some(to_date), None)?;
    if events.is_empty() {
        if options.is_json() {
            let output = formatters::render_document(&OutputDocument::default(), options.clone())?;
            options.writer().writeln(&output)?;
        } else {
            options.writer().writeln(&format!(
                "\n{} No income events found for {}.\n",
                "ℹ".blue().bold(),
                year_val
            ))?;
        }
        return Ok(());
    }

    let filtered_events: Vec<_> = if let Some(ref at_filter) = asset_type_filter {
        events
            .iter()
            .filter(|(_, asset)| asset.asset_type == *at_filter)
            .collect()
    } else {
        events.iter().collect()
    };

    if filtered_events.is_empty() {
        if options.is_json() {
            let output = formatters::render_document(&OutputDocument::default(), options.clone())?;
            options.writer().writeln(&output)?;
        } else {
            options.writer().writeln(&format!(
                "\n{} No income events found for {}.\n",
                "ℹ".blue().bold(),
                year_val
            ))?;
        }
        return Ok(());
    }

    // Look up cost basis from current portfolio for yield% calculation
    let cost_by_ticker: HashMap<String, Decimal> = {
        let report = crate::reports::calculate_portfolio(&conn, None)?;
        report
            .positions
            .iter()
            .map(|p| (p.asset.ticker.clone(), p.total_cost))
            .collect()
    };

    let mut by_ticker: HashMap<String, formatters::income::AssetIncome> = HashMap::new();
    for (event, asset) in &filtered_events {
        let entry =
            by_ticker
                .entry(asset.ticker.clone())
                .or_insert(formatters::income::AssetIncome {
                    ticker: asset.ticker.clone(),
                    asset_type: asset.asset_type,
                    dividends: Decimal::ZERO,
                    jcp: Decimal::ZERO,
                    amortization: Decimal::ZERO,
                    cost_basis: cost_by_ticker.get(&asset.ticker).copied(),
                });

        match event.event_type {
            db::IncomeEventType::Dividend => entry.dividends += event.total_amount,
            db::IncomeEventType::Jcp => entry.jcp += event.total_amount,
            db::IncomeEventType::Amortization => entry.amortization += event.total_amount,
        }
    }

    let mut by_type: HashMap<db::AssetType, Vec<formatters::income::AssetIncome>> = HashMap::new();
    for (_, income) in by_ticker {
        by_type.entry(income.asset_type).or_default().push(income);
    }

    for assets in by_type.values_mut() {
        assets.sort_by(|a, b| {
            let total_a = a.dividends + a.jcp + a.amortization;
            let total_b = b.dividends + b.jcp + b.amortization;
            total_b.cmp(&total_a)
        });
    }

    let mut ordered: Vec<(db::AssetType, Vec<formatters::income::AssetIncome>)> = Vec::new();
    let mut all_assets: Vec<formatters::income::AssetIncome> = Vec::new();
    for asset_type in db::AssetType::display_order() {
        if let Some(assets) = by_type.get(asset_type) {
            if assets.is_empty() {
                continue;
            }
            ordered.push((*asset_type, assets.clone()));
            all_assets.extend(assets.iter().cloned());
        }
    }

    let output = if options.is_json() {
        formatters::income::format_income_show_json(&all_assets, options.clone())?
    } else {
        formatters::income::format_income_show_table(&ordered, year_val, options.clone())?
    };
    options.writer().writeln(&output)?;

    Ok(())
}

/// Show detailed income events
async fn dispatch_income_detail(
    year: Option<i32>,
    asset: Option<&str>,
    options: options::OutputOptions,
) -> Result<()> {
    use chrono::Datelike;

    info!("Showing income events detail");

    db::init_database(None)?;
    let conn = db::open_db(None)?;

    let today = chrono::Local::now().date_naive();
    let (from_date, to_date) = match year {
        Some(y) => {
            let from = chrono::NaiveDate::from_ymd_opt(y, 1, 1).unwrap();
            let to = chrono::NaiveDate::from_ymd_opt(y, 12, 31).unwrap();
            (Some(from), Some(to))
        }
        None => {
            let y = today.year();
            let from = chrono::NaiveDate::from_ymd_opt(y, 1, 1).unwrap();
            (Some(from), Some(today))
        }
    };

    let events = db::get_income_events_with_assets(&conn, from_date, to_date, asset)?;

    if events.is_empty() {
        if options.is_json() {
            let output = formatters::render_document(&OutputDocument::default(), options.clone())?;
            options.writer().writeln(&output)?;
        } else {
            let year_str = year
                .map(|y| y.to_string())
                .unwrap_or_else(|| today.year().to_string());
            let asset_str = asset.map(|a| format!(" for {}", a)).unwrap_or_default();
            options.writer().writeln(&format!(
                "\n{} No income events found for {}{}.\n",
                "ℹ".blue().bold(),
                year_str,
                asset_str
            ))?;
        }
        return Ok(());
    }

    let year_val = year.unwrap_or_else(|| today.year());
    let output = if options.is_json() {
        formatters::income::format_income_detail_json(&events, options.clone())?
    } else {
        formatters::income::format_income_detail_table(&events, year_val, options.clone())?
    };
    options.writer().writeln(&output)?;

    Ok(())
}

/// Show detailed income events with explicit date range (used by `income asset`)
async fn dispatch_income_detail_impl(
    from_date: Option<chrono::NaiveDate>,
    to_date: Option<chrono::NaiveDate>,
    asset: Option<&str>,
    options: options::OutputOptions,
) -> Result<()> {
    use chrono::Datelike;

    info!("Showing income events detail");

    db::init_database(None)?;
    let conn = db::open_db(None)?;

    let events = db::get_income_events_with_assets(&conn, from_date, to_date, asset)?;

    if events.is_empty() {
        if options.is_json() {
            let output = formatters::render_document(&OutputDocument::default(), options.clone())?;
            options.writer().writeln(&output)?;
        } else {
            let asset_str = asset.map(|a| format!(" for {}", a)).unwrap_or_default();
            options.writer().writeln(&format!(
                "\n{} No income events found{}.\n",
                "ℹ".blue().bold(),
                asset_str
            ))?;
        }
        return Ok(());
    }

    let year_val = to_date
        .map(|d| d.year())
        .unwrap_or_else(|| chrono::Local::now().date_naive().year());
    let output = if options.is_json() {
        formatters::income::format_income_detail_json(&events, options.clone())?
    } else {
        formatters::income::format_income_detail_table(&events, year_val, options.clone())?
    };
    options.writer().writeln(&output)?;

    Ok(())
}

/// Show income summary - monthly breakdown if year given, yearly totals otherwise
pub async fn dispatch_income_summary(
    year: Option<i32>,
    categorize: bool,
    tax_aware: bool,
    options: options::OutputOptions,
) -> Result<()> {
    use chrono::Datelike;
    use rust_decimal::Decimal;
    use std::collections::BTreeMap;

    db::init_database(None)?;
    let conn = db::open_db(None)?;

    match year {
        Some(y) => {
            info!("Showing income summary with monthly breakdown for {}", y);

            let from_date = chrono::NaiveDate::from_ymd_opt(y, 1, 1).unwrap();
            let to_date = chrono::NaiveDate::from_ymd_opt(y, 12, 31).unwrap();

            let events =
                db::get_income_events_with_assets(&conn, Some(from_date), Some(to_date), None)?;

            if events.is_empty() {
                if options.is_json() {
                    let output =
                        formatters::render_document(&OutputDocument::default(), options.clone())?;
                    options.writer().writeln(&output)?;
                } else {
                    options.writer().writeln(&format!(
                        "\n{} No income events found for {}.\n",
                        "ℹ".blue().bold(),
                        y
                    ))?;
                }
                return Ok(());
            }

            let month_names = [
                "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
            ];

            let mut monthly: Vec<formatters::income::IncomeTotals> = (0..12)
                .map(|idx| formatters::income::IncomeTotals {
                    label: month_names[idx].to_string(),
                    dividends: Decimal::ZERO,
                    jcp: Decimal::ZERO,
                    amortization: Decimal::ZERO,
                })
                .collect();

            for (event, _asset) in &events {
                let month_idx = (event.event_date.month() - 1) as usize;
                match event.event_type {
                    db::IncomeEventType::Dividend => {
                        monthly[month_idx].dividends += event.total_amount
                    }
                    db::IncomeEventType::Jcp => monthly[month_idx].jcp += event.total_amount,
                    db::IncomeEventType::Amortization => {
                        monthly[month_idx].amortization += event.total_amount
                    }
                }
            }

            let total_dividends: Decimal = monthly.iter().map(|m| m.dividends).sum();
            let total_jcp: Decimal = monthly.iter().map(|m| m.jcp).sum();
            let total_amortization: Decimal = monthly.iter().map(|m| m.amortization).sum();
            let grand_total = total_dividends + total_jcp + total_amortization;

            let months_with_income = monthly
                .iter()
                .filter(|m| m.dividends + m.jcp + m.amortization > Decimal::ZERO)
                .count();
            let avg_per_month = if months_with_income > 0 {
                grand_total / Decimal::from(months_with_income)
            } else {
                Decimal::ZERO
            };

            let totals_by_type = aggregate_by_asset_type(&events);

            let totals = formatters::income::IncomeTotals {
                label: "TOTAL".to_string(),
                dividends: total_dividends,
                jcp: total_jcp,
                amortization: total_amortization,
            };
            let stats = formatters::income::IncomeSummaryStats {
                periods_with_income: months_with_income,
                avg_per_period: avg_per_month,
            };

            let output = if categorize {
                let (baseline_total, exceptional_total) = compute_categorized_totals(&events);
                formatters::income::format_income_summary_monthly_with_categories(
                    y,
                    &monthly,
                    &totals_by_type,
                    stats,
                    totals,
                    baseline_total,
                    exceptional_total,
                    total_jcp,
                    tax_aware,
                    options.clone(),
                )?
            } else {
                formatters::income::format_income_summary_monthly(
                    y,
                    &monthly,
                    &totals_by_type,
                    stats,
                    totals,
                    options.clone(),
                )?
            };
            options.writer().writeln(&output)?;
        }
        None => {
            info!("Showing income summary with yearly totals");

            let events = db::get_income_events_with_assets(&conn, None, None, None)?;

            if events.is_empty() {
                if options.is_json() {
                    let output =
                        formatters::render_document(&OutputDocument::default(), options.clone())?;
                    options.writer().writeln(&output)?;
                } else {
                    options.writer().writeln(&format!(
                        "\n{} No income events found.\n",
                        "ℹ".blue().bold()
                    ))?;
                }
                return Ok(());
            }

            let mut yearly: BTreeMap<i32, formatters::income::IncomeTotals> = BTreeMap::new();
            for (event, _asset) in &events {
                let year = event.event_date.year();
                let entry = yearly
                    .entry(year)
                    .or_insert(formatters::income::IncomeTotals {
                        label: year.to_string(),
                        dividends: Decimal::ZERO,
                        jcp: Decimal::ZERO,
                        amortization: Decimal::ZERO,
                    });
                match event.event_type {
                    db::IncomeEventType::Dividend => entry.dividends += event.total_amount,
                    db::IncomeEventType::Jcp => entry.jcp += event.total_amount,
                    db::IncomeEventType::Amortization => entry.amortization += event.total_amount,
                }
            }

            let total_dividends: Decimal = yearly.values().map(|y| y.dividends).sum();
            let total_jcp: Decimal = yearly.values().map(|y| y.jcp).sum();
            let total_amortization: Decimal = yearly.values().map(|y| y.amortization).sum();
            let grand_total = total_dividends + total_jcp + total_amortization;

            let years_with_income = yearly.len();
            let avg_per_year = if years_with_income > 0 {
                grand_total / Decimal::from(years_with_income)
            } else {
                Decimal::ZERO
            };

            let totals_by_type = aggregate_by_asset_type(&events);

            let totals = formatters::income::IncomeTotals {
                label: "TOTAL".to_string(),
                dividends: total_dividends,
                jcp: total_jcp,
                amortization: total_amortization,
            };
            let stats = formatters::income::IncomeSummaryStats {
                periods_with_income: years_with_income,
                avg_per_period: avg_per_year,
            };

            let yearly_rows: Vec<formatters::income::IncomeTotals> = yearly.into_values().collect();

            let output = if categorize {
                let (baseline_total, exceptional_total) = compute_categorized_totals(&events);
                formatters::income::format_income_summary_yearly_with_categories(
                    &yearly_rows,
                    &totals_by_type,
                    stats,
                    totals,
                    baseline_total,
                    exceptional_total,
                    total_jcp,
                    tax_aware,
                    options.clone(),
                )?
            } else {
                formatters::income::format_income_summary_yearly(
                    &yearly_rows,
                    &totals_by_type,
                    stats,
                    totals,
                    options.clone(),
                )?
            };
            options.writer().writeln(&output)?;
        }
    }

    Ok(())
}

/// Aggregate income events by asset type, sorted by total descending.
fn aggregate_by_asset_type(
    events: &[(db::IncomeEvent, db::Asset)],
) -> Vec<(db::AssetType, rust_decimal::Decimal)> {
    let mut totals: std::collections::HashMap<db::AssetType, rust_decimal::Decimal> =
        std::collections::HashMap::new();
    for (event, asset) in events {
        *totals
            .entry(asset.asset_type)
            .or_insert(rust_decimal::Decimal::ZERO) += event.total_amount;
    }
    let mut sorted: Vec<_> = totals.into_iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1));
    sorted
}

/// Compute baseline vs exceptional totals using the categorization engine.
/// Pre-groups events by (ticker, event_type) to avoid O(N²) scanning.
fn compute_categorized_totals(
    events: &[(db::IncomeEvent, db::Asset)],
) -> (rust_decimal::Decimal, rust_decimal::Decimal) {
    use crate::reports::income_analytics;
    use rust_decimal::Decimal;

    // Pre-group events by (ticker, event_type) for O(1) lookups
    let mut grouped: std::collections::HashMap<(&str, db::IncomeEventType), Vec<&db::IncomeEvent>> =
        std::collections::HashMap::new();
    for (event, asset) in events {
        grouped
            .entry((&asset.ticker, event.event_type))
            .or_default()
            .push(event);
    }

    let mut baseline_total = Decimal::ZERO;
    let mut exceptional_total = Decimal::ZERO;

    for (event, asset) in events {
        let history = grouped
            .get(&(asset.ticker.as_str(), event.event_type))
            .map(|v| v.as_slice())
            .unwrap_or(&[]);
        let category = income_analytics::categorize_income_event_with_history(event, history);
        if category.is_baseline {
            baseline_total += event.total_amount;
        } else {
            exceptional_total += event.total_amount;
        }
    }

    (baseline_total, exceptional_total)
}

#[allow(clippy::too_many_arguments)]
async fn dispatch_income_add(
    ticker: &str,
    event_type: &str,
    total_amount_str: &str,
    date_str: &str,
    ex_date_str: Option<&str>,
    withholding_str: &str,
    amount_per_quota_str: &str,
    notes: Option<&str>,
    options: options::OutputOptions,
) -> Result<()> {
    use anyhow::Context;
    use chrono::NaiveDate;
    use rust_decimal::Decimal;
    use std::str::FromStr;

    let total_amount = Decimal::from_str(total_amount_str)
        .context("Invalid total amount. Must be a decimal number")?;
    let withholding = Decimal::from_str(withholding_str)
        .context("Invalid withholding amount. Must be a decimal number")?;
    let amount_per_quota = Decimal::from_str(amount_per_quota_str)
        .context("Invalid amount per quota. Must be a decimal number")?;
    let event_date = NaiveDate::parse_from_str(date_str, "%Y-%m-%d")
        .context("Invalid date format. Use YYYY-MM-DD")?;
    let ex_date = match ex_date_str {
        Some(value) => Some(
            NaiveDate::parse_from_str(value, "%Y-%m-%d")
                .context("Invalid ex-date format. Use YYYY-MM-DD")?,
        ),
        None => None,
    };

    let event_type = db::IncomeEventType::from_str(event_type)
        .map_err(|_| anyhow::anyhow!("Invalid event type: {}", event_type))?;

    db::init_database(None)?;
    let conn = db::open_db(None)?;
    let asset_type = db::AssetType::Unknown;
    let asset_id = db::upsert_asset(&conn, ticker, &asset_type, None)?;

    let event = db::IncomeEvent {
        id: None,
        asset_id,
        event_date,
        ex_date,
        event_type,
        amount_per_quota,
        total_amount,
        withholding_tax: withholding,
        is_quota_pre_2026: None,
        source: "MANUAL".to_string(),
        notes: notes.map(|s| s.to_string()),
        created_at: chrono::Utc::now(),
    };

    let event_id = db::insert_income_event(&conn, &event)?;

    let output = formatters::income::format_income_add(
        event_id,
        ticker,
        event_date,
        total_amount,
        options.clone(),
    )?;
    options.writer().writeln(&output)?;

    Ok(())
}

// ============================================================================
// Analytics commands
// ============================================================================

async fn dispatch_income_yield(
    ticker: Option<&str>,
    asset_type: Option<db::AssetType>,
    options: options::OutputOptions,
) -> Result<()> {
    use crate::reports::income_analytics;
    use rust_decimal::Decimal;

    db::init_database(None)?;
    let conn = db::open_db(None)?;

    // calculate_portfolio already populates ltm_income/yield via apply_ltm_income
    let report = crate::reports::calculate_portfolio(&conn, None)?;

    if report.positions.is_empty() {
        if options.is_json() {
            let output = formatters::render_document(&OutputDocument::default(), options.clone())?;
            options.writer().writeln(&output)?;
        } else {
            options.writer().writeln(&format!(
                "\n{} No portfolio positions found.\n",
                "ℹ".blue().bold()
            ))?;
        }
        return Ok(());
    }

    // Build portfolio-wide LTM yield from already-enriched positions (no extra DB query)
    let total_ltm_income: Decimal = report.positions.iter().map(|p| p.ltm_income).sum();
    let yield_percentage = if report.total_value.is_zero() {
        Decimal::ZERO
    } else {
        (total_ltm_income / report.total_value) * Decimal::from(100)
    };
    let ltm_yield = income_analytics::LtmYieldResult {
        total_ltm_income,
        portfolio_value: report.total_value,
        yield_percentage,
    };

    // Build per-asset yields from already-enriched positions (no extra DB queries)
    let mut asset_yields = Vec::new();
    for position in &report.positions {
        if let Some(filter_ticker) = ticker {
            if position.asset.ticker != filter_ticker.to_uppercase() {
                continue;
            }
        }
        if let Some(at) = asset_type {
            if position.asset.asset_type != at {
                continue;
            }
        }

        if position.ltm_income > Decimal::ZERO {
            let value = position.current_value.unwrap_or(Decimal::ZERO);

            asset_yields.push(income_analytics::AssetYield {
                asset_id: position.asset.id.unwrap_or(0),
                ticker: position.asset.ticker.clone(),
                asset_type: position.asset.asset_type,
                ltm_income: position.ltm_income,
                current_position_value: value,
                yield_percentage: position.ltm_yield_pct.unwrap_or(Decimal::ZERO),
            });
        }
    }

    let output =
        formatters::income::format_yield_report(&ltm_yield, &asset_yields, options.clone())?;
    options.writer().writeln(&output)?;

    Ok(())
}

async fn dispatch_income_trends(
    ticker: Option<&str>,
    months: i32,
    options: options::OutputOptions,
) -> Result<()> {
    use crate::reports::income_analytics;

    db::init_database(None)?;
    let conn = db::open_db(None)?;
    let today = chrono::Local::now().date_naive();

    let series = income_analytics::get_monthly_income_series(&conn, months, ticker, today)?;
    let trend = income_analytics::analyze_income_trends_from_series(&series);

    if series.amounts.is_empty() {
        if options.is_json() {
            let output = formatters::render_document(&OutputDocument::default(), options.clone())?;
            options.writer().writeln(&output)?;
        } else {
            options.writer().writeln(&format!(
                "\n{} No income data found for trend analysis.\n",
                "ℹ".blue().bold()
            ))?;
        }
        return Ok(());
    }

    let output =
        formatters::income::format_trends_report(&series, &trend, months, options.clone())?;
    options.writer().writeln(&output)?;

    Ok(())
}

async fn dispatch_income_forecast(
    year: Option<i32>,
    conservative: bool,
    options: options::OutputOptions,
) -> Result<()> {
    use crate::reports::income_analytics;
    use std::collections::HashMap;

    db::init_database(None)?;
    let conn = db::open_db(None)?;
    let today = chrono::Local::now().date_naive();

    let forecast_year = year.unwrap_or_else(|| chrono::Datelike::year(&today) + 1);

    let report = crate::reports::calculate_portfolio(&conn, None)?;

    if report.positions.is_empty() {
        if options.is_json() {
            let output = formatters::render_document(&OutputDocument::default(), options.clone())?;
            options.writer().writeln(&output)?;
        } else {
            options.writer().writeln(&format!(
                "\n{} No portfolio positions found.\n",
                "ℹ".blue().bold()
            ))?;
        }
        return Ok(());
    }

    // Single batch query for all income events in the 2-year window
    let two_years_ago = today - chrono::Duration::days(730);
    let all_events =
        db::get_income_events_with_assets(&conn, Some(two_years_ago), Some(today), None)?;

    // Partition events by ticker
    let mut events_by_ticker: HashMap<String, Vec<(db::IncomeEvent, db::Asset)>> = HashMap::new();
    for (event, asset) in all_events {
        events_by_ticker
            .entry(asset.ticker.clone())
            .or_default()
            .push((event, asset));
    }

    let mut forecasts = Vec::new();
    for position in &report.positions {
        let asset_id = position.asset.id.unwrap_or(0);
        let events = events_by_ticker
            .get(&position.asset.ticker)
            .map(|v| v.as_slice())
            .unwrap_or(&[]);

        let forecast = income_analytics::forecast_income_from_events(
            events,
            asset_id,
            &position.asset.ticker,
            conservative,
            today,
        );
        if forecast.expected_annual_income > rust_decimal::Decimal::ZERO {
            forecasts.push(forecast);
        }
    }

    if forecasts.is_empty() {
        if options.is_json() {
            let output = formatters::render_document(&OutputDocument::default(), options.clone())?;
            options.writer().writeln(&output)?;
        } else {
            options.writer().writeln(&format!(
                "\n{} No income history to generate forecasts.\n",
                "ℹ".blue().bold()
            ))?;
        }
        return Ok(());
    }

    let output = formatters::income::format_forecast_report(
        &forecasts,
        forecast_year,
        conservative,
        options.clone(),
    )?;
    options.writer().writeln(&output)?;

    Ok(())
}

async fn dispatch_income_calendar(
    month_filter: Option<&str>,
    options: options::OutputOptions,
) -> Result<()> {
    use crate::reports::income_analytics;
    use std::collections::HashMap;

    db::init_database(None)?;
    let conn = db::open_db(None)?;
    let today = chrono::Local::now().date_naive();

    // Parse month filter (MM/YYYY) if provided
    let filter = if let Some(m) = month_filter {
        let parts: Vec<&str> = m.split('/').collect();
        if parts.len() != 2 {
            anyhow::bail!("Invalid month format. Use MM/YYYY (e.g. 03/2026)");
        }
        let month: u32 = parts[0]
            .parse()
            .map_err(|_| anyhow::anyhow!("Invalid month number"))?;
        let year: i32 = parts[1]
            .parse()
            .map_err(|_| anyhow::anyhow!("Invalid year"))?;
        if !(1..=12).contains(&month) {
            anyhow::bail!("Month must be between 1 and 12");
        }
        Some((year, month))
    } else {
        None
    };

    let report = crate::reports::calculate_portfolio(&conn, None)?;

    // Single batch query for all income events in the 2-year window
    let two_years_ago = today - chrono::Duration::days(730);
    let all_events =
        db::get_income_events_with_assets(&conn, Some(two_years_ago), Some(today), None)?;

    // Partition events by ticker
    let mut events_by_ticker: HashMap<String, Vec<(db::IncomeEvent, db::Asset)>> = HashMap::new();
    for (event, asset) in all_events {
        events_by_ticker
            .entry(asset.ticker.clone())
            .or_default()
            .push((event, asset));
    }

    // Predict enough months to cover the filter if specified
    let num_predictions = if let Some((year, month)) = filter {
        use chrono::Datelike;
        let target = chrono::NaiveDate::from_ymd_opt(year, month, 28).unwrap_or(today);
        let months_ahead =
            (target.year() - today.year()) * 12 + target.month() as i32 - today.month() as i32;
        std::cmp::max(months_ahead as usize + 1, 3)
    } else {
        3
    };

    let mut all_predictions = Vec::new();
    for position in &report.positions {
        let events = events_by_ticker
            .get(&position.asset.ticker)
            .map(|v| v.as_slice())
            .unwrap_or(&[]);

        let predictions =
            income_analytics::predict_payment_dates_from_events(events, num_predictions, today);
        for (date, amount, confidence) in predictions {
            all_predictions.push((position.asset.ticker.clone(), date, amount, confidence));
        }
    }

    // Apply month filter
    if let Some((year, month)) = filter {
        all_predictions.retain(|(_, date, _, _)| {
            use chrono::Datelike;
            date.year() == year && date.month() == month
        });
    }

    all_predictions.sort_by_key(|(_, date, _, _)| *date);

    if all_predictions.is_empty() {
        if options.is_json() {
            let output = formatters::render_document(&OutputDocument::default(), options.clone())?;
            options.writer().writeln(&output)?;
        } else {
            options.writer().writeln(&format!(
                "\n{} No payment predictions available.\n",
                "ℹ".blue().bold()
            ))?;
        }
        return Ok(());
    }

    let output = formatters::income::format_calendar_report(&all_predictions, options.clone())?;
    options.writer().writeln(&output)?;

    Ok(())
}

async fn dispatch_income_alerts(options: options::OutputOptions) -> Result<()> {
    use crate::reports::income_analytics;

    db::init_database(None)?;
    let conn = db::open_db(None)?;
    let today = chrono::Local::now().date_naive();

    let anomalies = income_analytics::detect_anomalies(&conn, today)?;

    let output = formatters::income::format_alerts_report(&anomalies, options.clone())?;
    options.writer().writeln(&output)?;

    Ok(())
}

async fn dispatch_income_export(
    year: i32,
    format: &str,
    output_path: Option<&str>,
    options: options::OutputOptions,
) -> Result<()> {
    use rust_decimal::Decimal;

    db::init_database(None)?;
    let conn = db::open_db(None)?;

    let from_date = chrono::NaiveDate::from_ymd_opt(year, 1, 1).unwrap();
    let to_date = chrono::NaiveDate::from_ymd_opt(year, 12, 31).unwrap();

    let events = db::get_income_events_with_assets(&conn, Some(from_date), Some(to_date), None)?;

    if events.is_empty() {
        if options.is_json() {
            let output = formatters::render_document(&OutputDocument::default(), options.clone())?;
            options.writer().writeln(&output)?;
        } else {
            options.writer().writeln(&format!(
                "\n{} No income events found for {}.\n",
                "ℹ".blue().bold(),
                year
            ))?;
        }
        return Ok(());
    }

    let file_ext = format.to_lowercase();
    let file_path = output_path
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("income_{}.{}", year, file_ext));

    match file_ext.as_str() {
        "csv" => {
            let mut wtr = csv::Writer::from_path(&file_path)?;
            wtr.write_record([
                "Date",
                "Ticker",
                "Asset Type",
                "Type",
                "Amount",
                "Tax",
                "Net",
                "Notes",
            ])?;
            for (event, asset) in &events {
                let net = event.total_amount - event.withholding_tax;
                wtr.write_record([
                    event.event_date.to_string(),
                    asset.ticker.clone(),
                    asset.asset_type.as_str().to_string(),
                    event.event_type.as_str().to_string(),
                    event.total_amount.to_string(),
                    event.withholding_tax.to_string(),
                    net.to_string(),
                    event.notes.clone().unwrap_or_default(),
                ])?;
            }
            wtr.flush()?;
        }
        "xlsx" => {
            use rust_xlsxwriter::{Format, Workbook};

            let mut workbook = Workbook::new();
            let worksheet = workbook.add_worksheet();
            worksheet.set_name("Income Events")?;

            let header_format = Format::new().set_bold();
            let headers = [
                "Date",
                "Ticker",
                "Asset Type",
                "Type",
                "Amount",
                "Tax",
                "Net",
                "Notes",
            ];
            for (col, header) in headers.iter().enumerate() {
                worksheet.write_string_with_format(0, col as u16, *header, &header_format)?;
            }

            for (row_idx, (event, asset)) in events.iter().enumerate() {
                let row = (row_idx + 1) as u32;
                let net = event.total_amount - event.withholding_tax;
                worksheet.write_string(row, 0, event.event_date.to_string())?;
                worksheet.write_string(row, 1, &asset.ticker)?;
                worksheet.write_string(row, 2, asset.asset_type.as_str())?;
                worksheet.write_string(row, 3, event.event_type.as_str())?;
                worksheet.write_number(row, 4, decimal_to_f64(event.total_amount))?;
                worksheet.write_number(row, 5, decimal_to_f64(event.withholding_tax))?;
                worksheet.write_number(row, 6, decimal_to_f64(net))?;
                worksheet.write_string(row, 7, event.notes.as_deref().unwrap_or(""))?;
            }

            // Summary sheet
            let summary = workbook.add_worksheet();
            summary.set_name("Summary")?;

            let mut type_totals: std::collections::HashMap<db::AssetType, Decimal> =
                std::collections::HashMap::new();
            for (event, asset) in &events {
                *type_totals.entry(asset.asset_type).or_insert(Decimal::ZERO) += event.total_amount;
            }

            summary.write_string_with_format(0, 0, "Asset Type", &header_format)?;
            summary.write_string_with_format(0, 1, "Total", &header_format)?;

            let mut sorted_types: Vec<_> = type_totals.iter().collect();
            sorted_types.sort_by(|a, b| b.1.cmp(a.1));

            for (row_idx, (asset_type, total)) in sorted_types.iter().enumerate() {
                let row = (row_idx + 1) as u32;
                summary.write_string(row, 0, asset_type.as_str())?;
                summary.write_number(row, 1, decimal_to_f64(**total))?;
            }

            workbook.save(&file_path)?;
        }
        _ => {
            return Err(anyhow::anyhow!(
                "Unsupported export format: {}. Use csv or xlsx.",
                format
            ));
        }
    }

    if options.is_json() {
        let doc = OutputDocument {
            title: None,
            blocks: vec![crate::output::OutputBlock::KeyValue {
                title: Some("Export Complete".to_string()),
                rows: vec![
                    crate::output::KeyValueRow {
                        label: "File".to_string(),
                        value: crate::output::Value::Text(file_path.clone()),
                    },
                    crate::output::KeyValueRow {
                        label: "Events".to_string(),
                        value: crate::output::Value::Text(events.len().to_string()),
                    },
                ],
            }],
            meta: Default::default(),
        };
        let output = formatters::render_document(&doc, options.clone())?;
        options.writer().writeln(&output)?;
    } else {
        options.writer().writeln(&format!(
            "\n{} Exported {} income events to {}\n",
            "✓".green().bold(),
            events.len(),
            file_path
        ))?;
    }

    Ok(())
}

fn decimal_to_f64(d: rust_decimal::Decimal) -> f64 {
    use rust_decimal::prelude::ToPrimitive;
    d.to_f64().unwrap_or(0.0)
}
