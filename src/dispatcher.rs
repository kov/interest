//! Command dispatcher that routes clap Commands to the appropriate handlers.
//!
//! This module provides a unified interface for command routing, with clap
//! as the single source of truth for command definitions.

pub mod performance;
use performance::dispatch_performance;
mod actions;
mod assets;
mod cashflow;
pub mod imports;
pub mod imports_helpers;
mod inconsistencies;
mod inspect;
mod irpf;
mod portfolio;
mod prices;
mod terms;
mod tickers;
mod transactions;
use crate::utils::format_currency;
use crate::{db, formatters, options, tax};
use anyhow::Result;
use colored::Colorize;
use std::io::IsTerminal;
use tracing::info;

/// Route a parsed command to its handler
pub async fn dispatch_command(
    command: &crate::cli::Commands,
    options: options::OutputOptions,
) -> Result<()> {
    use crate::cli::Commands;

    match command {
        Commands::Import {
            file,
            dry_run,
            force_reimport,
        } => imports::dispatch_import(file, *dry_run, *force_reimport, options).await,
        Commands::ImportIrpf {
            file,
            year,
            dry_run,
        } => irpf::dispatch_irpf_import(file, *year, *dry_run, options).await,
        Commands::Portfolio { action } => portfolio::dispatch_portfolio(action, options).await,
        Commands::Performance { action } => dispatch_performance(action, options).await,
        Commands::CashFlow { action } => cashflow::dispatch_cashflow(action, options).await,
        Commands::Tax { action } => dispatch_tax(action, options).await,
        Commands::Income { action } => dispatch_income(action, options).await,
        Commands::Actions { action } => actions::dispatch_actions(action, options).await,
        Commands::Prices { action } => prices::dispatch_prices(action, options).await,
        Commands::Transactions { action } => {
            transactions::dispatch_transactions(action, options).await
        }
        Commands::Inspect { file, full, column } => {
            inspect::dispatch_inspect(file, *full, *column).await
        }
        Commands::ProcessTerms => terms::dispatch_process_terms().await,
        Commands::Inconsistencies { action } => {
            inconsistencies::dispatch_inconsistencies(action, options).await
        }
        Commands::Tickers { action } => tickers::dispatch_tickers(action, options).await,
        Commands::Assets { action } => assets::dispatch_assets(action, options).await,
        Commands::Interactive => {
            // This should never be reached since main.rs handles Interactive separately
            Err(anyhow::anyhow!(
                "Interactive mode should be handled by main.rs"
            ))
        }
        Commands::Chat => {
            // This should never be reached since main.rs handles Chat separately
            Err(anyhow::anyhow!("Chat mode should be handled by main.rs"))
        }
        Commands::Completions { shell, no_install } => {
            dispatch_completions(*shell, *no_install, options)
        }
        Commands::Complete { args } => dispatch_dynamic_complete(args),
        Commands::Privacy { .. } => Err(anyhow::anyhow!(
            "Privacy mode is only supported in interactive mode. Use --privacy for CLI commands."
        )),
    }
}

async fn dispatch_tax(
    action: &crate::cli::TaxCommands,
    options: options::OutputOptions,
) -> Result<()> {
    match action {
        crate::cli::TaxCommands::Report { year, export } => {
            dispatch_tax_report(*year, *export, options).await
        }
        crate::cli::TaxCommands::Summary { year } => dispatch_tax_summary(*year, options).await,
        crate::cli::TaxCommands::Calculate { month } => {
            dispatch_tax_calculate(month, options).await
        }
    }
}

async fn dispatch_income(
    action: &crate::cli::IncomeCommands,
    options: options::OutputOptions,
) -> Result<()> {
    match action {
        crate::cli::IncomeCommands::Show { year } => dispatch_income_show(*year, options).await,
        crate::cli::IncomeCommands::Detail { year, asset } => {
            dispatch_income_detail(*year, asset.as_deref(), options).await
        }
        crate::cli::IncomeCommands::Summary { year } => {
            dispatch_income_summary(*year, options).await
        }
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
    }
}

async fn dispatch_tax_report(
    year: i32,
    export_csv: bool,
    options: options::OutputOptions,
) -> Result<()> {
    info!("Generating IRPF annual report for {}", year);

    // Initialize database
    db::init_database(None)?;
    let conn = db::open_db(None)?;

    // Generate report; suppress progress output in JSON mode
    let report = if options.is_json() {
        tax::generate_annual_report_with_progress(&conn, year, |_ev| {})?
    } else {
        let mut printer = TaxProgressPrinter::new(&options);
        tax::generate_annual_report_with_progress(&conn, year, |ev| printer.on_event(ev))?
    };

    let income_summary = formatters::tax::build_income_summary(&conn, year)?;

    let output =
        formatters::tax::format_tax_report(&report, &income_summary, year, options.clone())?;
    options.writer().writeln(&output)?;

    if export_csv {
        let csv_content = tax::irpf::export_to_csv(&report);
        let csv_path = format!("irpf_report_{}.csv", year);
        std::fs::write(&csv_path, csv_content)?;

        options.writer().writeln(&format!(
            "{} Report exported to: {}\n",
            "✓".green().bold(),
            csv_path
        ))?;
    }

    Ok(())
}

async fn dispatch_tax_summary(year: i32, options: options::OutputOptions) -> Result<()> {
    info!("Generating tax summary for {}", year);

    // Initialize database
    db::init_database(None)?;
    let conn = db::open_db(None)?;

    // Generate report with in-place spinner progress (terse)
    // Suppress progress output in JSON mode
    let report = if options.is_json() {
        tax::generate_annual_report_with_progress(&conn, year, |_ev| {})?
    } else {
        let mut printer = TaxProgressPrinter::new(&options);
        tax::generate_annual_report_with_progress(&conn, year, |ev| printer.on_event(ev))?
    };

    let output = formatters::tax::format_tax_summary(&report, year, options.clone())?;
    options.writer().writeln(&output)?;

    Ok(())
}

/// Show income summary by asset, grouped by asset type
async fn dispatch_income_show(year: Option<i32>, options: options::OutputOptions) -> Result<()> {
    use chrono::Datelike;
    use rust_decimal::Decimal;
    use std::collections::HashMap;

    info!("Showing income summary by asset");

    db::init_database(None)?;
    let conn = db::open_db(None)?;

    let today = chrono::Local::now().date_naive();
    let (from_date, to_date, year_val) = match year {
        Some(y) => {
            let from = chrono::NaiveDate::from_ymd_opt(y, 1, 1).unwrap();
            let to = chrono::NaiveDate::from_ymd_opt(y, 12, 31).unwrap();
            (Some(from), Some(to), y)
        }
        None => {
            let y = today.year();
            let from = chrono::NaiveDate::from_ymd_opt(y, 1, 1).unwrap();
            (Some(from), Some(today), y)
        }
    };

    let events = db::get_income_events_with_assets(&conn, from_date, to_date, None)?;
    if events.is_empty() {
        options.writer().writeln(&format!(
            "\n{} No income events found for {}.\n",
            "ℹ".blue().bold(),
            year_val
        ))?;
        return Ok(());
    }

    let mut by_ticker: HashMap<String, formatters::income::AssetIncome> = HashMap::new();
    for (event, asset) in &events {
        let entry =
            by_ticker
                .entry(asset.ticker.clone())
                .or_insert(formatters::income::AssetIncome {
                    ticker: asset.ticker.clone(),
                    asset_type: asset.asset_type,
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

    let type_order = [
        db::AssetType::Stock,
        db::AssetType::Bdr,
        db::AssetType::Fii,
        db::AssetType::Fiagro,
        db::AssetType::FiInfra,
        db::AssetType::Etf,
        db::AssetType::Fidc,
        db::AssetType::Fip,
        db::AssetType::Bond,
        db::AssetType::GovBond,
        db::AssetType::Option,
        db::AssetType::TermContract,
        db::AssetType::Unknown,
    ];

    let mut ordered: Vec<(db::AssetType, Vec<formatters::income::AssetIncome>)> = Vec::new();
    let mut all_assets: Vec<formatters::income::AssetIncome> = Vec::new();
    for asset_type in &type_order {
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

    // Initialize database
    db::init_database(None)?;
    let conn = db::open_db(None)?;

    // Determine date range
    let today = chrono::Local::now().date_naive();
    let (from_date, to_date) = match year {
        Some(y) => {
            let from = chrono::NaiveDate::from_ymd_opt(y, 1, 1).unwrap();
            let to = chrono::NaiveDate::from_ymd_opt(y, 12, 31).unwrap();
            (Some(from), Some(to))
        }
        None => {
            // Default to current year
            let y = today.year();
            let from = chrono::NaiveDate::from_ymd_opt(y, 1, 1).unwrap();
            (Some(from), Some(today))
        }
    };

    // Query income events
    let events = db::get_income_events_with_assets(&conn, from_date, to_date, asset)?;

    if events.is_empty() {
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

/// Show income summary - monthly breakdown if year given, yearly totals otherwise
pub async fn dispatch_income_summary(
    year: Option<i32>,
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
                options.writer().writeln(&format!(
                    "\n{} No income events found for {}.\n",
                    "ℹ".blue().bold(),
                    y
                ))?;
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

            let mut asset_type_totals: std::collections::HashMap<db::AssetType, Decimal> =
                std::collections::HashMap::new();
            for (event, asset) in &events {
                *asset_type_totals
                    .entry(asset.asset_type)
                    .or_insert(Decimal::ZERO) += event.total_amount;
            }
            let mut asset_type_vec: Vec<_> = asset_type_totals.iter().collect();
            asset_type_vec.sort_by(|a, b| b.1.cmp(a.1));
            let totals_by_type = asset_type_vec
                .iter()
                .map(|(t, total)| (**t, **total))
                .collect::<Vec<_>>();

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

            let output = formatters::income::format_income_summary_monthly(
                y,
                &monthly,
                &totals_by_type,
                stats,
                totals,
                options.clone(),
            )?;
            options.writer().writeln(&output)?;
        }
        None => {
            info!("Showing income summary with yearly totals");

            let events = db::get_income_events_with_assets(&conn, None, None, None)?;

            if events.is_empty() {
                options.writer().writeln(&format!(
                    "\n{} No income events found.\n",
                    "ℹ".blue().bold()
                ))?;
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

            let mut asset_type_totals: std::collections::HashMap<db::AssetType, Decimal> =
                std::collections::HashMap::new();
            for (event, asset) in &events {
                *asset_type_totals
                    .entry(asset.asset_type)
                    .or_insert(Decimal::ZERO) += event.total_amount;
            }
            let mut asset_type_vec: Vec<_> = asset_type_totals.iter().collect();
            asset_type_vec.sort_by(|a, b| b.1.cmp(a.1));
            let totals_by_type = asset_type_vec
                .iter()
                .map(|(t, total)| (**t, **total))
                .collect::<Vec<_>>();

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
            let output = formatters::income::format_income_summary_yearly(
                &yearly_rows,
                &totals_by_type,
                stats,
                totals,
                options.clone(),
            )?;
            options.writer().writeln(&output)?;
        }
    }

    Ok(())
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

async fn dispatch_tax_calculate(month_str: &str, options: options::OutputOptions) -> Result<()> {
    use anyhow::Context;
    use colored::Colorize;

    tracing::info!("Calculating swing trade tax for {}", month_str);

    // Parse month string (MM/YYYY)
    let parts: Vec<&str> = month_str.split('/').collect();
    if parts.len() != 2 {
        return Err(anyhow::anyhow!(
            "Invalid month format. Use MM/YYYY (e.g., 01/2025)"
        ));
    }

    let month: u32 = parts[0].parse().context("Invalid month number")?;
    let year: i32 = parts[1].parse().context("Invalid year")?;

    if !(1..=12).contains(&month) {
        return Err(anyhow::anyhow!("Month must be between 01 and 12"));
    }

    // Initialize database
    db::init_database(None)?;
    let conn = db::open_db(None)?;

    // Calculate monthly tax; carryforward map stays empty for one-off calculation
    let mut carryforward = std::collections::HashMap::new();
    let calculations = tax::calculate_monthly_tax(&conn, year, month, &mut carryforward)?;

    if calculations.is_empty() {
        options.writer().writeln(&format!(
            "\n{} No sales found for {}/{}\n",
            "ℹ".blue().bold(),
            month,
            year
        ))?;
        return Ok(());
    }

    options.writer().writeln(&format!(
        "\n{} Swing Trade Tax Calculation - {}/{}\n",
        "💰".cyan().bold(),
        month,
        year
    ))?;

    // Display results by tax category
    for calc in &calculations {
        options.writer().writeln(&format!(
            "{} {}",
            "Tax Category:".bold(),
            calc.category.display_name()
        ))?;
        options.writer().writeln(&format!(
            "  Total Sales:      {}",
            format_currency(calc.total_sales, &options).cyan()
        ))?;
        options.writer().writeln(&format!(
            "  Total Cost Basis: {}",
            format_currency(calc.total_cost_basis, &options).cyan()
        ))?;
        options.writer().writeln(&format!(
            "  Gross Profit:     {}",
            format_currency(calc.total_profit, &options).green()
        ))?;
        options.writer().writeln(&format!(
            "  Gross Loss:       {}",
            format_currency(calc.total_loss, &options).red()
        ))?;

        let net_str = if calc.net_profit >= rust_decimal::Decimal::ZERO {
            format_currency(calc.net_profit, &options).green()
        } else {
            format_currency(calc.net_profit, &options).red()
        };
        options
            .writer()
            .writeln(&format!("  Net P&L:          {}", net_str))?;

        // Show loss offset if applied
        if calc.loss_offset_applied > rust_decimal::Decimal::ZERO {
            options.writer().writeln(&format!(
                "  Loss Offset:      {} (from previous months)",
                format_currency(calc.loss_offset_applied, &options).cyan()
            ))?;
            options.writer().writeln(&format!(
                "  After Loss Offset: {}",
                format_currency(calc.profit_after_loss_offset, &options).green()
            ))?;
        }

        if calc.exemption_applied > rust_decimal::Decimal::ZERO {
            options.writer().writeln(&format!(
                "  Exemption:        {} (sales under R$20.000)",
                format_currency(calc.exemption_applied, &options)
                    .yellow()
                    .bold()
            ))?;
        }

        if calc.taxable_amount > rust_decimal::Decimal::ZERO {
            options.writer().writeln(&format!(
                "  Taxable Amount:   {}",
                format_currency(calc.taxable_amount, &options).yellow()
            ))?;
            let tax_rate_pct = calc.tax_rate * rust_decimal::Decimal::from(100);
            options.writer().writeln(&format!(
                "  Tax Rate:         {}",
                format!("{:.0}%", tax_rate_pct).yellow()
            ))?;
            options.writer().writeln(&format!(
                "  {} {}",
                "Tax Due:".bold(),
                format_currency(calc.tax_due, &options).red().bold()
            ))?;
        } else if calc.profit_after_loss_offset < rust_decimal::Decimal::ZERO {
            options.writer().writeln(&format!(
                "  {} Loss to carry forward",
                format_currency(calc.net_profit.abs(), &options)
                    .yellow()
                    .bold()
            ))?;
        } else {
            options.writer().writeln(&format!(
                "  {} No tax due (exempt)",
                "Tax Due:".bold().green()
            ))?;
        }

        options.writer().writeln("")?;
    }

    // Summary
    let total_tax: rust_decimal::Decimal = calculations.iter().map(|c| c.tax_due).sum();

    if total_tax > rust_decimal::Decimal::ZERO {
        options.writer().writeln(&format!(
            "{} Total Tax Due for {}/{}: {}\n",
            "📋".cyan().bold(),
            month,
            year,
            format_currency(total_tax, &options).red().bold()
        ))?;

        // Generate DARF payments
        let darf_payments = tax::generate_darf_payments(calculations, year, month)?;

        if !darf_payments.is_empty() {
            options
                .writer()
                .writeln(&format!("{} DARF Payments:\n", "💳".cyan().bold()))?;

            for payment in &darf_payments {
                options.writer().writeln(&format!(
                    "  {} Code {}: {}",
                    "DARF".yellow().bold(),
                    payment.darf_code,
                    payment.description
                ))?;
                options.writer().writeln(&format!(
                    "    Amount:   {}",
                    format_currency(payment.tax_due, &options).red()
                ))?;
                options.writer().writeln(&format!(
                    "    Due Date: {}",
                    payment.due_date.format("%d/%m/%Y").to_string().yellow()
                ))?;
                options.writer().writeln("")?;
            }

            options.writer().writeln(&format!(
                "{} Payment due by {}\n",
                "⏰".yellow(),
                darf_payments[0].due_date.format("%d/%m/%Y")
            ))?;
        }
    }

    Ok(())
}

// Snapshot commands are intentionally internal-only; no public dispatcher.

struct TaxProgressPrinter {
    printer: crate::ui::progress::ProgressPrinter,
    in_progress: bool,
    from_year: Option<i32>,
    target_year: Option<i32>,
    total_years: usize,
    completed_years: usize,
}

impl TaxProgressPrinter {
    fn new(options: &options::OutputOptions) -> Self {
        Self {
            printer: crate::ui::progress::ProgressPrinter::new(options),
            in_progress: false,
            from_year: None,
            target_year: None,
            total_years: 0,
            completed_years: 0,
        }
    }

    fn on_event(&mut self, event: tax::ReportProgress) {
        match event {
            tax::ReportProgress::Start { target_year, .. } => {
                self.target_year = Some(target_year);
            }
            tax::ReportProgress::RecomputeStart { from_year } => {
                self.from_year = Some(from_year);
                self.in_progress = true;
                self.completed_years = 0;
                self.total_years = self
                    .target_year
                    .map(|t| (t - from_year + 1).max(1) as usize)
                    .unwrap_or(1);
                self.printer
                    .handle_event(&crate::ui::progress::ProgressEvent::Recomputing {
                        what: format!("snapshots (starting {})", from_year),
                        progress: Some(crate::ui::progress::ProgressData {
                            current: self.completed_years,
                            total: Some(self.total_years),
                        }),
                    });
            }
            tax::ReportProgress::RecomputedYear { year } => {
                if self.in_progress {
                    self.completed_years = (self.completed_years + 1).min(self.total_years);
                    let from = self.from_year.unwrap_or(year);
                    if Some(year) == self.target_year {
                        self.printer
                            .handle_event(&crate::ui::progress::ProgressEvent::Success {
                                message: format!("Snapshots updated {}→{}", from, year),
                            });
                        self.in_progress = false;
                    } else {
                        self.printer.handle_event(
                            &crate::ui::progress::ProgressEvent::Recomputing {
                                what: format!("snapshots (year {})", year),
                                progress: Some(crate::ui::progress::ProgressData {
                                    current: self.completed_years,
                                    total: Some(self.total_years),
                                }),
                            },
                        );
                    }
                }
            }
            tax::ReportProgress::TargetCacheHit { year } => {
                self.printer
                    .handle_event(&crate::ui::progress::ProgressEvent::Success {
                        message: format!("Cache hit for {}; using cached carry", year),
                    });
            }
            _ => {}
        }
    }
}

/// Generate shell completion scripts
fn dispatch_completions(
    shell: Option<crate::cli::Shell>,
    no_install: bool,
    options: options::OutputOptions,
) -> Result<()> {
    use clap::CommandFactory;
    use clap_complete::{generate, shells};

    // Reject --json flag for completions since the output is a shell script
    if options.output_mode == options::OutputMode::Json {
        return Err(anyhow::anyhow!(
            "JSON output is not supported for shell completions. \
             The completions command generates shell scripts that must be written to stdout directly."
        ));
    }

    // Auto-detect shell if not specified
    let shell = match shell {
        Some(s) => s,
        None => detect_shell()?,
    };

    let mut cmd = crate::cli::Cli::command();
    let bin_name = cmd
        .get_bin_name()
        .unwrap_or_else(|| cmd.get_name())
        .to_string();

    // Determine if we should use interactive installation
    let is_tty = std::io::stdout().is_terminal();
    let should_install = is_tty && !no_install;

    if should_install {
        // Interactive installation mode
        install_completion_interactively(shell, &mut cmd, &bin_name)
    } else {
        // Print to stdout (for piping or when --no-install is specified)
        match shell {
            crate::cli::Shell::Bash => {
                // Generate base Bash completions
                let mut buffer = Vec::new();
                generate(shells::Bash, &mut cmd, &bin_name, &mut buffer);

                // Add dynamic completion support
                let completion_script = String::from_utf8(buffer)?;
                let enhanced_script = add_bash_dynamic_completions(&completion_script, &bin_name);
                print!("{}", enhanced_script);
            }
            crate::cli::Shell::Fish => {
                // Generate base Fish completions
                let mut buffer = Vec::new();
                generate(shells::Fish, &mut cmd, &bin_name, &mut buffer);

                // Add dynamic completion support
                let completion_script = String::from_utf8(buffer)?;
                let enhanced_script = add_fish_dynamic_completions(&completion_script, &bin_name);
                print!("{}", enhanced_script);
            }
            crate::cli::Shell::Zsh => {
                // Generate base Zsh completions
                let mut buffer = Vec::new();
                generate(shells::Zsh, &mut cmd, &bin_name, &mut buffer);

                // Add dynamic completion support
                let completion_script = String::from_utf8(buffer)?;
                let enhanced_script = add_zsh_dynamic_completions(&completion_script, &bin_name);
                print!("{}", enhanced_script);
            }
        }
        Ok(())
    }
}

/// Detect the current shell from the environment
fn detect_shell() -> Result<crate::cli::Shell> {
    use std::env;

    let shell = env::var("SHELL")
        .map_err(|_| anyhow::anyhow!("Could not detect shell from $SHELL environment variable. Please specify the shell explicitly."))?;

    // Extract the shell name from the path (e.g., /bin/bash -> bash)
    let shell_name = shell.rsplit('/').next().unwrap_or(&shell);

    match shell_name {
        "bash" => Ok(crate::cli::Shell::Bash),
        "fish" => Ok(crate::cli::Shell::Fish),
        "zsh" => Ok(crate::cli::Shell::Zsh),
        _ => Err(anyhow::anyhow!(
            "Unsupported shell '{}'. Supported shells: bash, fish, zsh. Please specify the shell explicitly.",
            shell_name
        )),
    }
}

/// Install completion script interactively
fn install_completion_interactively(
    shell: crate::cli::Shell,
    cmd: &mut clap::Command,
    bin_name: &str,
) -> Result<()> {
    use clap_complete::{generate, shells};
    use std::fs;
    use std::io::{self, Write};

    // Determine the installation path based on shell
    let install_path = get_completion_install_path(shell)?;

    // Ask user for confirmation
    eprint!(
        "Install {} completion to {}? [Y/n] ",
        shell_name(shell),
        install_path.display()
    );
    io::stderr().flush()?;

    let mut response = String::new();
    io::stdin().read_line(&mut response)?;
    let response = response.trim().to_lowercase();

    if !response.is_empty() && response != "y" && response != "yes" {
        eprintln!("Installation cancelled.");
        return Ok(());
    }

    // Create parent directory if it doesn't exist
    if let Some(parent) = install_path.parent() {
        fs::create_dir_all(parent)?;
    }

    // Generate completion script to a buffer
    let mut buffer = Vec::new();
    match shell {
        crate::cli::Shell::Bash => {
            // Generate base Bash completions
            generate(shells::Bash, cmd, bin_name, &mut buffer);

            // Add dynamic completion support
            let completion_script = String::from_utf8(buffer)?;
            let enhanced_script = add_bash_dynamic_completions(&completion_script, bin_name);
            buffer = enhanced_script.into_bytes();
        }
        crate::cli::Shell::Fish => {
            // Generate base Fish completions
            generate(shells::Fish, cmd, bin_name, &mut buffer);

            // Add dynamic completion support
            let completion_script = String::from_utf8(buffer)?;
            let enhanced_script = add_fish_dynamic_completions(&completion_script, bin_name);
            buffer = enhanced_script.into_bytes();
        }
        crate::cli::Shell::Zsh => {
            // Generate base Zsh completions
            generate(shells::Zsh, cmd, bin_name, &mut buffer);

            // Add dynamic completion support
            let completion_script = String::from_utf8(buffer)?;
            let enhanced_script = add_zsh_dynamic_completions(&completion_script, bin_name);
            buffer = enhanced_script.into_bytes();
        }
    }

    // Write to file
    fs::write(&install_path, buffer)?;

    eprintln!("✓ Completion installed to {}", install_path.display());
    eprintln!("\nTo activate completions:");
    match shell {
        crate::cli::Shell::Bash => {
            eprintln!("  source {}", install_path.display());
            eprintln!("Or restart your shell.");
        }
        crate::cli::Shell::Fish => {
            eprintln!("  Restart your shell or run: source ~/.config/fish/config.fish");
        }
        crate::cli::Shell::Zsh => {
            eprintln!("  Restart your shell or run: source ~/.zshrc");
        }
    }

    Ok(())
}

/// Add dynamic completion support to Fish completion script
fn add_fish_dynamic_completions(script: &str, bin_name: &str) -> String {
    let mut result = script.to_string();

    // Add dynamic completions for specific options
    let dynamic_completions = format!(
        r#"
# Dynamic completions for --asset-type
complete -c {bin} -n "__fish_interest_using_subcommand portfolio; and __fish_seen_subcommand_from show" -l asset-type -a "({bin} complete (commandline -opc))" -d 'Asset type'

# Dynamic completions for --at
complete -c {bin} -n "__fish_interest_using_subcommand portfolio; and __fish_seen_subcommand_from show" -l at -a "({bin} complete (commandline -opc))" -d 'Date or year'
"#,
        bin = bin_name
    );

    result.push_str(&dynamic_completions);
    result
}

/// Add dynamic completion support to Bash completion script
fn add_bash_dynamic_completions(script: &str, bin_name: &str) -> String {
    let mut result = script.to_string();

    // Add helper function for dynamic completions
    let dynamic_completions = format!(
        r#"

# Dynamic completion helper function
__{bin}_dynamic_complete() {{
    local cur prev_word cmd_line
    cur="${{COMP_WORDS[$COMP_CWORD]}}"
    
    # Get the full command line building from COMP_WORDS
    cmd_line=("${{COMP_WORDS[@]}}")
    
    # Call interest complete with the command line
    COMPREPLY=($(compgen -W "$({bin} complete "${{cmd_line[@]}}")" -- "$cur"))
}}

# Dynamic completions for portfolio show --asset-type
__{bin}_asset_type_complete() {{
    local cur="${{COMP_WORDS[$COMP_CWORD]}}"
    if [[ "${{#COMP_WORDS[@]}}" -gt 2 ]] && [[ "${{COMP_WORDS[1]}}" == "portfolio" ]] && [[ "${{COMP_WORDS[2]}}" == "show" ]]; then
        COMPREPLY=($(compgen -W "$({bin} complete "${{COMP_WORDS[@]}}")" -- "$cur"))
    fi
}}

# Dynamic completions for portfolio show --at
__{bin}_at_complete() {{
    local cur="${{COMP_WORDS[$COMP_CWORD]}}"
    if [[ "${{#COMP_WORDS[@]}}" -gt 2 ]] && [[ "${{COMP_WORDS[1]}}" == "portfolio" ]] && [[ "${{COMP_WORDS[2]}}" == "show" ]]; then
        COMPREPLY=($(compgen -W "$({bin} complete "${{COMP_WORDS[@]}}")" -- "$cur"))
    fi
}}
"#,
        bin = bin_name
    );

    result.push_str(&dynamic_completions);
    result
}

/// Add dynamic completion support to Zsh completion script
fn add_zsh_dynamic_completions(script: &str, bin_name: &str) -> String {
    let mut result = script.to_string();

    // Add helper functions for dynamic completions
    let dynamic_completions = format!(
        r#"

# Dynamic completion helper function for zsh
(( $+functions[__{bin}_complete] )) || __{bin}_complete() {{
    local -a values
    values=($({bin} complete "${{words[@]}}"))
    compadd -a values
}}

# Dynamic completions for asset types
(( $+functions[__{bin}_asset_types] )) || __{bin}_asset_types() {{
    __{bin}_complete
}}

# Dynamic completions for years
(( $+functions[__{bin}_years] )) || __{bin}_years() {{
    __{bin}_complete
}}
"#,
        bin = bin_name
    );

    result.push_str(&dynamic_completions);
    result
}

/// Get the standard installation path for completion scripts
fn get_completion_install_path(shell: crate::cli::Shell) -> Result<std::path::PathBuf> {
    use std::env;
    use std::path::PathBuf;

    match shell {
        crate::cli::Shell::Bash => {
            // Try XDG_DATA_HOME first, then fall back to ~/.local/share
            let data_home = env::var("XDG_DATA_HOME")
                .ok()
                .map(PathBuf::from)
                .or_else(|| {
                    env::var("HOME")
                        .ok()
                        .map(|h| PathBuf::from(h).join(".local/share"))
                });

            if let Some(base) = data_home {
                Ok(base.join("bash-completion/completions/interest"))
            } else {
                Err(anyhow::anyhow!(
                    "Could not determine home directory for bash completion installation"
                ))
            }
        }
        crate::cli::Shell::Fish => {
            // Respect XDG_CONFIG_HOME if set, otherwise use ~/.config
            let config_home = env::var("XDG_CONFIG_HOME")
                .ok()
                .filter(|s| !s.is_empty())
                .or_else(|| env::var("HOME").ok().map(|h| format!("{}/.config", h)))
                .ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;
            Ok(PathBuf::from(config_home).join("fish/completions/interest.fish"))
        }
        crate::cli::Shell::Zsh => {
            // Use ~/.zsh/completions as recommended in README
            let home = env::var("HOME")
                .map_err(|_| anyhow::anyhow!("Could not determine home directory"))?;
            Ok(PathBuf::from(home).join(".zsh/completions/_interest"))
        }
    }
}

/// Get human-readable shell name
fn shell_name(shell: crate::cli::Shell) -> &'static str {
    match shell {
        crate::cli::Shell::Bash => "bash",
        crate::cli::Shell::Fish => "fish",
        crate::cli::Shell::Zsh => "zsh",
    }
}

/// Handle dynamic completion requests
fn dispatch_dynamic_complete(args: &[String]) -> Result<()> {
    // Parse the command line to understand what we're completing
    // args contains the partial command line being completed

    if args.is_empty() {
        return Ok(());
    }

    // Try to find what option we're completing for
    let completing_for = args.iter().rev().find(|arg| arg.starts_with("--"));

    match completing_for.map(|s| s.as_str()) {
        Some("--asset-type") | Some("-a") => {
            // Provide asset type completions
            println!("STOCK");
            println!("FII");
            println!("FIAGRO");
            println!("FI_INFRA");
        }
        Some("--at") => {
            // Provide year completions from database
            if let Ok(years) = get_available_years() {
                for year in years {
                    println!("{}", year);
                }
            }
        }
        Some("--ticker") | Some("ticker") if args.contains(&"assets".to_string()) => {
            // Provide ticker completions from database
            if let Ok(tickers) = get_available_tickers() {
                for ticker in tickers {
                    println!("{}", ticker);
                }
            }
        }
        _ => {
            // No specific completions for this context
        }
    }

    Ok(())
}

/// Get available years from transactions in the database
fn get_available_years() -> Result<Vec<i32>> {
    let conn = db::open_db_read_only(None)?;

    let mut stmt = conn.prepare(
        "SELECT DISTINCT strftime('%Y', trade_date) as year 
         FROM transactions 
         WHERE trade_date IS NOT NULL 
         ORDER BY year DESC",
    )?;

    let years = stmt
        .query_map([], |row| {
            let year_str: String = row.get(0)?;
            year_str
                .parse::<i32>()
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
        })?
        .collect::<Result<Vec<i32>, _>>()?;

    Ok(years)
}

/// Get available tickers from the database
fn get_available_tickers() -> Result<Vec<String>> {
    let conn = db::open_db_read_only(None)?;

    let mut stmt = conn.prepare("SELECT DISTINCT ticker FROM assets ORDER BY ticker")?;

    let tickers = stmt
        .query_map([], |row| row.get(0))?
        .collect::<Result<Vec<String>, _>>()?;

    Ok(tickers)
}

// Tests removed - dispatcher now works with clap Commands
// Integration tests in tests/ directory provide coverage
