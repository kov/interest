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
use crate::{db, tax};
use anyhow::Result;
use colored::Colorize;
use tracing::info;

/// Route a parsed command to its handler
pub async fn dispatch_command(command: &crate::cli::Commands, json_output: bool) -> Result<()> {
    use crate::cli::Commands;

    match command {
        Commands::Import {
            file,
            dry_run,
            force_reimport,
        } => imports::dispatch_import(file, *dry_run, *force_reimport, json_output).await,
        Commands::ImportIrpf {
            file,
            year,
            dry_run,
        } => irpf::dispatch_irpf_import(file, *year, *dry_run).await,
        Commands::Portfolio { action } => portfolio::dispatch_portfolio(action, json_output).await,
        Commands::Performance { action } => dispatch_performance(action, json_output).await,
        Commands::CashFlow { action } => cashflow::dispatch_cashflow(action, json_output).await,
        Commands::Tax { action } => dispatch_tax(action, json_output).await,
        Commands::Income { action } => dispatch_income(action, json_output).await,
        Commands::Actions { action } => actions::dispatch_actions(action, json_output).await,
        Commands::Prices { action } => prices::dispatch_prices(action, json_output).await,
        Commands::Transactions { action } => {
            transactions::dispatch_transactions(action, json_output).await
        }
        Commands::Inspect { file, full, column } => {
            inspect::dispatch_inspect(file, *full, *column).await
        }
        Commands::ProcessTerms => terms::dispatch_process_terms().await,
        Commands::Inconsistencies { action } => {
            inconsistencies::dispatch_inconsistencies(action, json_output).await
        }
        Commands::Tickers { action } => tickers::dispatch_tickers(action, json_output).await,
        Commands::Assets { action } => assets::dispatch_assets(action, json_output).await,
        Commands::Interactive => {
            // This should never be reached since main.rs handles Interactive separately
            Err(anyhow::anyhow!(
                "Interactive mode should be handled by main.rs"
            ))
        }
    }
}

async fn dispatch_tax(action: &crate::cli::TaxCommands, json_output: bool) -> Result<()> {
    match action {
        crate::cli::TaxCommands::Report { year, export } => {
            dispatch_tax_report(*year, *export, json_output).await
        }
        crate::cli::TaxCommands::Summary { year } => dispatch_tax_summary(*year, json_output).await,
        crate::cli::TaxCommands::Calculate { month } => dispatch_tax_calculate(month).await,
    }
}

async fn dispatch_income(action: &crate::cli::IncomeCommands, json_output: bool) -> Result<()> {
    match action {
        crate::cli::IncomeCommands::Show { year } => dispatch_income_show(*year, json_output).await,
        crate::cli::IncomeCommands::Detail { year, asset } => {
            dispatch_income_detail(*year, asset.as_deref(), json_output).await
        }
        crate::cli::IncomeCommands::Summary { year } => {
            dispatch_income_summary(*year, json_output).await
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
                json_output,
            )
            .await
        }
    }
}

async fn dispatch_tax_report(year: i32, export_csv: bool, json_output: bool) -> Result<()> {
    use rust_decimal::Decimal;
    use serde::Serialize;
    use tabled::{
        settings::{object::Columns, Alignment, Modify, Style},
        Table, Tabled,
    };

    info!("Generating IRPF annual report for {}", year);

    // Initialize database
    db::init_database(None)?;
    let conn = db::open_db(None)?;

    // Generate report; suppress progress output in JSON mode
    let report = if json_output {
        tax::generate_annual_report_with_progress(&conn, year, |_ev| {})?
    } else {
        let mut printer = TaxProgressPrinter::new();
        tax::generate_annual_report_with_progress(&conn, year, |ev| printer.on_event(ev))?
    };

    let income_summary = build_income_summary(&conn, year)?;
    let has_income = income_summary
        .iter()
        .any(|entry| entry.dividends_net > Decimal::ZERO || entry.jcp_net > Decimal::ZERO);

    if json_output {
        // Emit concise JSON suitable for tests and scripting
        #[derive(Serialize)]
        struct MonthlySummaryJson {
            month: String,
            sales: rust_decimal::Decimal,
            profit: rust_decimal::Decimal,
            loss: rust_decimal::Decimal,
            tax_due: rust_decimal::Decimal,
        }

        let monthly: Vec<MonthlySummaryJson> = report
            .monthly_summaries
            .iter()
            .map(|m| MonthlySummaryJson {
                month: m.month_name.to_string(),
                sales: m.total_sales,
                profit: m.total_profit,
                loss: m.total_loss,
                tax_due: m.tax_due,
            })
            .collect();

        #[derive(Serialize)]
        struct IncomeSummaryJson {
            ticker: String,
            asset_type: String,
            cnpj: Option<String>,
            dividends_net: rust_decimal::Decimal,
            jcp_net: rust_decimal::Decimal,
            total_net: rust_decimal::Decimal,
        }

        let income: Vec<IncomeSummaryJson> = income_summary
            .iter()
            .map(|entry| IncomeSummaryJson {
                ticker: entry.ticker.clone(),
                asset_type: entry.asset_type.as_str().to_string(),
                cnpj: entry.cnpj.clone(),
                dividends_net: entry.dividends_net,
                jcp_net: entry.jcp_net,
                total_net: entry.dividends_net + entry.jcp_net,
            })
            .collect();

        let payload = serde_json::json!({
            "year": year,
            "annual_total_sales": report.annual_total_sales,
            "annual_total_profit": report.annual_total_profit,
            "annual_total_loss": report.annual_total_loss,
            "annual_total_tax": report.annual_total_tax,
            "monthly_summaries": monthly,
            "income_summary": income,
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    } else {
        println!(
            "\n{} Annual IRPF Tax Report - {}\n",
            "📊".cyan().bold(),
            year
        );
    }

    if report.monthly_summaries.is_empty() && !has_income {
        println!(
            "\n{} No transactions found for year {}\n",
            "ℹ".blue().bold(),
            year
        );
        return Ok(());
    }

    // Show prior-year carryforward losses if any
    if !report.previous_losses_carry_forward.is_empty() {
        println!("{} Carryover from previous years:", "📦".yellow().bold());
        for (category, amount) in &report.previous_losses_carry_forward {
            println!(
                "  {}: {}",
                category.display_name(),
                format_currency(*amount)
            );
        }
        println!();
    }

    if !report.monthly_summaries.is_empty() {
        // Monthly breakdown
        println!("{}", "Monthly Summary:".bold());
        for summary in &report.monthly_summaries {
            println!("\n  {}:", summary.month_name.bold());
            println!(
                "    Sales:  {}",
                format_currency(summary.total_sales).cyan()
            );
            println!(
                "    Profit: {}",
                format_currency(summary.total_profit).green()
            );
            println!("    Loss:   {}", format_currency(summary.total_loss).red());
            println!("    Tax:    {}", format_currency(summary.tax_due).yellow());
        }

        // Annual totals
        println!("\n{} Annual Totals:", "📈".cyan().bold());
        println!(
            "  Total Sales:  {}",
            format_currency(report.annual_total_sales).cyan()
        );
        println!(
            "  Total Profit: {}",
            format_currency(report.annual_total_profit).green()
        );
        println!(
            "  Total Loss:   {}",
            format_currency(report.annual_total_loss).red()
        );
        println!(
            "  {} {}\n",
            "Total Tax:".bold(),
            format_currency(report.annual_total_tax).yellow().bold()
        );

        // Losses to carry forward
        if !report.losses_to_carry_forward.is_empty() {
            println!("{} Losses to Carry Forward:", "📋".yellow().bold());
            for (category, loss) in &report.losses_to_carry_forward {
                println!(
                    "  {}: {}",
                    category.display_name(),
                    format_currency(*loss).yellow()
                );
            }
            println!();
        }
    }

    if has_income {
        #[derive(Tabled)]
        struct IncomeRow {
            #[tabled(rename = "Ticker")]
            ticker: String,
            #[tabled(rename = "CNPJ")]
            cnpj: String,
            #[tabled(rename = "Asset Type")]
            asset_type: String,
            #[tabled(rename = "Dividends (Net)")]
            dividends: String,
            #[tabled(rename = "JCP (Net)")]
            jcp: String,
            #[tabled(rename = "Total (Net)")]
            total: String,
        }

        let rows: Vec<IncomeRow> = income_summary
            .iter()
            .filter_map(|entry| {
                let total = entry.dividends_net + entry.jcp_net;
                if total <= Decimal::ZERO {
                    return None;
                }
                Some(IncomeRow {
                    ticker: entry.ticker.clone(),
                    cnpj: format_cnpj(entry.cnpj.as_deref()).unwrap_or_else(|| "-".to_string()),
                    asset_type: entry.asset_type.as_str().to_string(),
                    dividends: if entry.dividends_net > Decimal::ZERO {
                        format_currency(entry.dividends_net)
                    } else {
                        "-".to_string()
                    },
                    jcp: if entry.jcp_net > Decimal::ZERO {
                        format_currency(entry.jcp_net)
                    } else {
                        "-".to_string()
                    },
                    total: format_currency(total),
                })
            })
            .collect();

        if !rows.is_empty() {
            let total_dividends: Decimal = income_summary.iter().map(|e| e.dividends_net).sum();
            let total_jcp: Decimal = income_summary.iter().map(|e| e.jcp_net).sum();
            let total_all = total_dividends + total_jcp;
            let mut table_rows = rows;
            table_rows.push(IncomeRow {
                ticker: "TOTAL".to_string(),
                cnpj: "-".to_string(),
                asset_type: "TOTAL".to_string(),
                dividends: format_currency(total_dividends),
                jcp: format_currency(total_jcp),
                total: format_currency(total_all),
            });

            println!("{} Dividends & JCP Received:", "💵".cyan().bold());
            let mut table = Table::new(table_rows);
            let table = table
                .with(Style::rounded())
                .with(Modify::new(Columns::new(3..6)).with(Alignment::right()));
            println!("{table}");
            println!();
        }
    }

    if export_csv {
        let csv_content = tax::irpf::export_to_csv(&report);
        let csv_path = format!("irpf_report_{}.csv", year);
        std::fs::write(&csv_path, csv_content)?;

        println!("{} Report exported to: {}\n", "✓".green().bold(), csv_path);
    }

    Ok(())
}

#[derive(Clone)]
struct IncomeByType {
    ticker: String,
    asset_type: db::AssetType,
    cnpj: Option<String>,
    dividends_net: rust_decimal::Decimal,
    jcp_net: rust_decimal::Decimal,
}

fn build_income_summary(conn: &rusqlite::Connection, year: i32) -> Result<Vec<IncomeByType>> {
    use chrono::NaiveDate;
    use rust_decimal::Decimal;
    use std::collections::HashMap;

    let from_date = NaiveDate::from_ymd_opt(year, 1, 1).unwrap();
    let to_date = NaiveDate::from_ymd_opt(year, 12, 31).unwrap();
    let events = db::get_income_events_with_assets(conn, Some(from_date), Some(to_date), None)?;

    let tracked_types = [
        db::AssetType::Fii,
        db::AssetType::FiInfra,
        db::AssetType::Fiagro,
        db::AssetType::Stock,
        db::AssetType::Etf,
        db::AssetType::Bdr,
    ];

    let tracked_set: std::collections::HashSet<db::AssetType> =
        tracked_types.iter().copied().collect();
    let mut by_ticker: HashMap<String, IncomeByType> = HashMap::new();
    for (event, asset) in events {
        if !tracked_set.contains(&asset.asset_type) {
            continue;
        }
        let entry = by_ticker
            .entry(asset.ticker.clone())
            .or_insert(IncomeByType {
                ticker: asset.ticker.clone(),
                asset_type: asset.asset_type,
                cnpj: asset.cnpj.clone(),
                dividends_net: Decimal::ZERO,
                jcp_net: Decimal::ZERO,
            });
        if entry.cnpj.is_none() {
            entry.cnpj = asset.cnpj.clone();
        }
        let net_amount = event.total_amount - event.withholding_tax;
        match event.event_type {
            db::IncomeEventType::Dividend => entry.dividends_net += net_amount,
            db::IncomeEventType::Jcp => entry.jcp_net += net_amount,
            _ => {}
        }
    }

    let mut summary: Vec<IncomeByType> = by_ticker.into_values().collect();
    summary.sort_by(|a, b| a.ticker.cmp(&b.ticker));
    Ok(summary)
}

fn format_cnpj(value: Option<&str>) -> Option<String> {
    let raw = value?;
    let digits: String = raw.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() != 14 {
        return Some(raw.to_string());
    }
    Some(format!(
        "{}.{}.{}/{}-{}",
        &digits[0..2],
        &digits[2..5],
        &digits[5..8],
        &digits[8..12],
        &digits[12..14]
    ))
}

async fn dispatch_tax_summary(year: i32, _json_output: bool) -> Result<()> {
    use tabled::{
        settings::{object::Columns, Alignment, Modify, Style},
        Table, Tabled,
    };

    info!("Generating tax summary for {}", year);

    // Initialize database
    db::init_database(None)?;
    let conn = db::open_db(None)?;

    // Generate report with in-place spinner progress (terse)
    let mut printer = TaxProgressPrinter::new();
    let report = tax::generate_annual_report_with_progress(&conn, year, |ev| printer.on_event(ev))?;

    if report.monthly_summaries.is_empty() {
        println!(
            "\n{} No transactions found for year {}\n",
            "ℹ".blue().bold(),
            year
        );
        return Ok(());
    }

    println!("\n{} Tax Summary - {}\n", "📊".cyan().bold(), year);

    // Display monthly table
    #[derive(Tabled)]
    struct MonthRow {
        #[tabled(rename = "Month")]
        month: String,
        #[tabled(rename = "Sales")]
        sales: String,
        #[tabled(rename = "Profit")]
        profit: String,
        #[tabled(rename = "Loss")]
        loss: String,
        #[tabled(rename = "Tax Due")]
        tax: String,
    }

    let rows: Vec<MonthRow> = report
        .monthly_summaries
        .iter()
        .map(|s| MonthRow {
            month: s.month_name.to_string(),
            sales: format_currency(s.total_sales),
            profit: format_currency(s.total_profit),
            loss: format_currency(s.total_loss),
            tax: format_currency(s.tax_due),
        })
        .collect();

    let table = Table::new(rows)
        .with(Style::rounded())
        .with(Modify::new(Columns::new(1..)).with(Alignment::right()))
        .to_string();
    println!("{}", table);

    // Annual summary
    println!("\n{} Annual Total", "📈".cyan().bold());
    println!(
        "  Sales:  {}",
        format_currency(report.annual_total_sales).cyan()
    );
    println!(
        "  Profit: {}",
        format_currency(report.annual_total_profit).green()
    );
    println!(
        "  Loss:   {}",
        format_currency(report.annual_total_loss).red()
    );
    println!(
        "  {} {}\n",
        "Tax:".bold(),
        format_currency(report.annual_total_tax).yellow().bold()
    );

    Ok(())
}

/// Show income summary by asset, grouped by asset type
async fn dispatch_income_show(year: Option<i32>, json_output: bool) -> Result<()> {
    use chrono::Datelike;
    use rust_decimal::Decimal;
    use serde::Serialize;
    use std::collections::HashMap;
    use tabled::{
        settings::{object::Columns, Alignment, Modify, Style},
        Table, Tabled,
    };

    info!("Showing income summary by asset");

    // Initialize database
    db::init_database(None)?;
    let conn = db::open_db(None)?;

    // Determine date range
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

    // Query income events
    let events = db::get_income_events_with_assets(&conn, from_date, to_date, None)?;

    if events.is_empty() {
        println!(
            "\n{} No income events found for {}.\n",
            "ℹ".blue().bold(),
            year_val
        );
        return Ok(());
    }

    // Group by asset type and ticker
    struct AssetIncome {
        ticker: String,
        asset_type: db::AssetType,
        dividends: Decimal,
        jcp: Decimal,
        amortization: Decimal,
    }

    let mut by_ticker: HashMap<String, AssetIncome> = HashMap::new();

    for (event, asset) in &events {
        let entry = by_ticker
            .entry(asset.ticker.clone())
            .or_insert(AssetIncome {
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

    // Group by asset type
    let mut by_type: HashMap<db::AssetType, Vec<AssetIncome>> = HashMap::new();
    for (_, income) in by_ticker {
        by_type.entry(income.asset_type).or_default().push(income);
    }

    // Sort each group by total (descending)
    for assets in by_type.values_mut() {
        assets.sort_by(|a, b| {
            let total_a = a.dividends + a.jcp + a.amortization;
            let total_b = b.dividends + b.jcp + b.amortization;
            total_b.cmp(&total_a)
        });
    }

    if json_output {
        #[derive(Serialize)]
        struct JsonAssetIncome {
            ticker: String,
            asset_type: String,
            dividends: String,
            jcp: String,
            amortization: String,
            total: String,
        }

        let mut all_assets: Vec<JsonAssetIncome> = Vec::new();
        for (asset_type, assets) in &by_type {
            for a in assets {
                let total = a.dividends + a.jcp + a.amortization;
                all_assets.push(JsonAssetIncome {
                    ticker: a.ticker.clone(),
                    asset_type: asset_type.as_str().to_string(),
                    dividends: a.dividends.to_string(),
                    jcp: a.jcp.to_string(),
                    amortization: a.amortization.to_string(),
                    total: total.to_string(),
                });
            }
        }
        println!("{}", serde_json::to_string_pretty(&all_assets)?);
        return Ok(());
    }

    println!("\n{} Income Summary - {}\n", "💰".cyan().bold(), year_val);

    // Define display order for asset types
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

    let mut grand_total = Decimal::ZERO;

    for asset_type in &type_order {
        if let Some(assets) = by_type.get(asset_type) {
            if assets.is_empty() {
                continue;
            }

            #[derive(Tabled)]
            struct IncomeRow {
                #[tabled(rename = "Ticker")]
                ticker: String,
                #[tabled(rename = "Dividends")]
                dividends: String,
                #[tabled(rename = "JCP")]
                jcp: String,
                #[tabled(rename = "Amort")]
                amort: String,
                #[tabled(rename = "Total")]
                total: String,
            }

            let rows: Vec<IncomeRow> = assets
                .iter()
                .map(|a| {
                    let total = a.dividends + a.jcp + a.amortization;
                    IncomeRow {
                        ticker: a.ticker.clone(),
                        dividends: if a.dividends > Decimal::ZERO {
                            format_currency(a.dividends)
                        } else {
                            "-".to_string()
                        },
                        jcp: if a.jcp > Decimal::ZERO {
                            format_currency(a.jcp)
                        } else {
                            "-".to_string()
                        },
                        amort: if a.amortization > Decimal::ZERO {
                            format_currency(a.amortization)
                        } else {
                            "-".to_string()
                        },
                        total: format_currency(total),
                    }
                })
                .collect();

            let type_total: Decimal = assets
                .iter()
                .map(|a| a.dividends + a.jcp + a.amortization)
                .sum();
            grand_total += type_total;

            println!(
                "{} {} ({})",
                "▸".cyan(),
                asset_type.as_str().to_uppercase().bold(),
                format_currency(type_total).cyan()
            );

            let table = Table::new(&rows)
                .with(Style::rounded())
                .with(Modify::new(Columns::new(1..)).with(Alignment::right()))
                .to_string();
            println!("{}\n", table);
        }
    }

    println!(
        "{} {}\n",
        "Grand Total:".bold(),
        format_currency(grand_total).green().bold()
    );

    Ok(())
}

/// Show detailed income events
async fn dispatch_income_detail(
    year: Option<i32>,
    asset: Option<&str>,
    json_output: bool,
) -> Result<()> {
    use chrono::Datelike;
    use rust_decimal::Decimal;
    use serde::Serialize;
    use tabled::{
        settings::{object::Columns, Alignment, Modify, Style},
        Table, Tabled,
    };

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
        println!(
            "\n{} No income events found for {}{}.\n",
            "ℹ".blue().bold(),
            year_str,
            asset_str
        );
        return Ok(());
    }

    if json_output {
        #[derive(Serialize)]
        struct IncomeRow {
            date: String,
            ticker: String,
            asset_type: String,
            event_type: String,
            amount: String,
            notes: Option<String>,
        }

        let rows: Vec<IncomeRow> = events
            .iter()
            .map(|(event, asset)| IncomeRow {
                date: event.event_date.to_string(),
                ticker: asset.ticker.clone(),
                asset_type: asset.asset_type.as_str().to_string(),
                event_type: event.event_type.as_str().to_string(),
                amount: event.total_amount.to_string(),
                notes: event.notes.clone(),
            })
            .collect();

        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }

    // Display table
    let year_str = year
        .map(|y| y.to_string())
        .unwrap_or_else(|| today.year().to_string());
    let asset_str = asset.map(|a| format!(" - {}", a)).unwrap_or_default();
    println!(
        "\n{} Income Events - {}{}\n",
        "💰".cyan().bold(),
        year_str,
        asset_str
    );

    #[derive(Tabled)]
    struct IncomeTableRow {
        #[tabled(rename = "Date")]
        date: String,
        #[tabled(rename = "Ticker")]
        ticker: String,
        #[tabled(rename = "Type")]
        event_type: String,
        #[tabled(rename = "Amount")]
        amount: String,
        #[tabled(rename = "Notes")]
        notes: String,
    }

    let rows: Vec<IncomeTableRow> = events
        .iter()
        .map(|(event, asset)| IncomeTableRow {
            date: event.event_date.format("%Y-%m-%d").to_string(),
            ticker: asset.ticker.clone(),
            event_type: match event.event_type {
                db::IncomeEventType::Dividend => "Dividend",
                db::IncomeEventType::Jcp => "JCP",
                db::IncomeEventType::Amortization => "Amort",
            }
            .to_string(),
            amount: format_currency(event.total_amount),
            notes: event.notes.clone().unwrap_or_default(),
        })
        .collect();

    let table = Table::new(&rows)
        .with(Style::rounded())
        .with(Modify::new(Columns::new(3..4)).with(Alignment::right()))
        .to_string();
    println!("{}", table);

    // Summary
    let total: Decimal = events.iter().map(|(e, _)| e.total_amount).sum();
    let dividends: Decimal = events
        .iter()
        .filter(|(e, _)| matches!(e.event_type, db::IncomeEventType::Dividend))
        .map(|(e, _)| e.total_amount)
        .sum();
    let jcp: Decimal = events
        .iter()
        .filter(|(e, _)| matches!(e.event_type, db::IncomeEventType::Jcp))
        .map(|(e, _)| e.total_amount)
        .sum();
    let amort: Decimal = events
        .iter()
        .filter(|(e, _)| matches!(e.event_type, db::IncomeEventType::Amortization))
        .map(|(e, _)| e.total_amount)
        .sum();

    println!("\n{} Summary:", "📊".cyan().bold());
    if dividends > Decimal::ZERO {
        println!("  Dividends:    {}", format_currency(dividends).green());
    }
    if jcp > Decimal::ZERO {
        println!("  JCP:          {}", format_currency(jcp).green());
    }
    if amort > Decimal::ZERO {
        println!("  Amortization: {}", format_currency(amort).yellow());
    }
    println!(
        "  {} {}\n",
        "Total:".bold(),
        format_currency(total).green().bold()
    );

    Ok(())
}

/// Show income summary - monthly breakdown if year given, yearly totals otherwise
pub async fn dispatch_income_summary(year: Option<i32>, json_output: bool) -> Result<()> {
    use chrono::Datelike;
    use rust_decimal::Decimal;
    use serde::Serialize;
    use std::collections::BTreeMap;
    use tabled::{
        settings::{object::Columns, Alignment, Modify, Style},
        Table, Tabled,
    };

    // Initialize database
    db::init_database(None)?;
    let conn = db::open_db(None)?;

    match year {
        Some(y) => {
            // Monthly breakdown for specific year
            info!("Showing income summary with monthly breakdown for {}", y);

            let from_date = chrono::NaiveDate::from_ymd_opt(y, 1, 1).unwrap();
            let to_date = chrono::NaiveDate::from_ymd_opt(y, 12, 31).unwrap();

            let events =
                db::get_income_events_with_assets(&conn, Some(from_date), Some(to_date), None)?;

            if events.is_empty() {
                println!(
                    "\n{} No income events found for {}.\n",
                    "ℹ".blue().bold(),
                    y
                );
                return Ok(());
            }

            let month_names = [
                "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
            ];

            struct MonthlyTotals {
                dividends: Decimal,
                jcp: Decimal,
                amortization: Decimal,
            }

            let mut monthly: Vec<MonthlyTotals> = (0..12)
                .map(|_| MonthlyTotals {
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

            // Calculate totals by asset type
            let mut asset_type_totals: std::collections::HashMap<db::AssetType, Decimal> =
                std::collections::HashMap::new();
            for (event, asset) in &events {
                *asset_type_totals
                    .entry(asset.asset_type)
                    .or_insert(Decimal::ZERO) += event.total_amount;
            }
            let mut asset_type_vec: Vec<_> = asset_type_totals.iter().collect();
            asset_type_vec.sort_by(|a, b| b.1.cmp(a.1)); // Sort by amount descending

            if json_output {
                #[derive(Serialize)]
                struct JsonMonthlyRow {
                    month: String,
                    dividends: String,
                    jcp: String,
                    amortization: String,
                    total: String,
                }

                #[derive(Serialize)]
                struct JsonSummary {
                    year: i32,
                    monthly: Vec<JsonMonthlyRow>,
                    totals: JsonMonthlyRow,
                    months_with_income: usize,
                    avg_per_month: String,
                }

                let monthly_rows: Vec<JsonMonthlyRow> = monthly
                    .iter()
                    .enumerate()
                    .map(|(i, m)| {
                        let total = m.dividends + m.jcp + m.amortization;
                        JsonMonthlyRow {
                            month: month_names[i].to_string(),
                            dividends: m.dividends.to_string(),
                            jcp: m.jcp.to_string(),
                            amortization: m.amortization.to_string(),
                            total: total.to_string(),
                        }
                    })
                    .collect();

                let summary = JsonSummary {
                    year: y,
                    monthly: monthly_rows,
                    totals: JsonMonthlyRow {
                        month: "TOTAL".to_string(),
                        dividends: total_dividends.to_string(),
                        jcp: total_jcp.to_string(),
                        amortization: total_amortization.to_string(),
                        total: grand_total.to_string(),
                    },
                    months_with_income,
                    avg_per_month: avg_per_month.to_string(),
                };

                println!("{}", serde_json::to_string_pretty(&summary)?);
                return Ok(());
            }

            println!(
                "\n{} Income Summary - {} (Monthly Breakdown)\n",
                "💰".cyan().bold(),
                y
            );

            #[derive(Tabled)]
            struct MonthRow {
                #[tabled(rename = "Month")]
                month: String,
                #[tabled(rename = "Dividends")]
                dividends: String,
                #[tabled(rename = "JCP")]
                jcp: String,
                #[tabled(rename = "Amort")]
                amort: String,
                #[tabled(rename = "Total")]
                total: String,
            }

            let mut rows: Vec<MonthRow> = monthly
                .iter()
                .enumerate()
                .map(|(i, m)| {
                    let total = m.dividends + m.jcp + m.amortization;
                    MonthRow {
                        month: month_names[i].to_string(),
                        dividends: if m.dividends > Decimal::ZERO {
                            format_currency(m.dividends)
                        } else {
                            "-".to_string()
                        },
                        jcp: if m.jcp > Decimal::ZERO {
                            format_currency(m.jcp)
                        } else {
                            "-".to_string()
                        },
                        amort: if m.amortization > Decimal::ZERO {
                            format_currency(m.amortization)
                        } else {
                            "-".to_string()
                        },
                        total: if total > Decimal::ZERO {
                            format_currency(total)
                        } else {
                            "-".to_string()
                        },
                    }
                })
                .collect();

            rows.push(MonthRow {
                month: "─────".to_string(),
                dividends: "───────────".to_string(),
                jcp: "───────────".to_string(),
                amort: "───────────".to_string(),
                total: "───────────".to_string(),
            });
            rows.push(MonthRow {
                month: "TOTAL".to_string(),
                dividends: format_currency(total_dividends),
                jcp: format_currency(total_jcp),
                amort: format_currency(total_amortization),
                total: format_currency(grand_total),
            });

            let table = Table::new(&rows)
                .with(Style::rounded())
                .with(Modify::new(Columns::new(1..)).with(Alignment::right()))
                .to_string();
            println!("{}", table);

            println!("\n{} Subtotals by Type:", "📊".cyan().bold());
            if total_dividends > Decimal::ZERO {
                println!(
                    "  Dividends:    {}",
                    format_currency(total_dividends).green()
                );
            }
            if total_jcp > Decimal::ZERO {
                println!("  JCP:          {}", format_currency(total_jcp).green());
            }
            if total_amortization > Decimal::ZERO {
                println!(
                    "  Amortization: {}",
                    format_currency(total_amortization).yellow()
                );
            }
            println!(
                "  {} {}",
                "Total:".bold(),
                format_currency(grand_total).green().bold()
            );

            println!("\n{} Subtotals by Asset Type:", "📊".cyan().bold());
            for (asset_type, total) in asset_type_vec {
                println!(
                    "  {:12} {}",
                    format!("{:?}:", asset_type),
                    format_currency(*total).green()
                );
            }

            println!("\n{} Statistics:", "📈".cyan().bold());
            println!("  Months with income: {}", months_with_income);
            println!(
                "  Average per month:  {}",
                format_currency(avg_per_month).cyan()
            );
            println!();
        }
        None => {
            // Yearly summary across all years
            info!("Showing income summary with yearly totals");

            let events = db::get_income_events_with_assets(&conn, None, None, None)?;

            if events.is_empty() {
                println!("\n{} No income events found.\n", "ℹ".blue().bold());
                return Ok(());
            }

            struct YearlyTotals {
                dividends: Decimal,
                jcp: Decimal,
                amortization: Decimal,
            }

            let mut yearly: BTreeMap<i32, YearlyTotals> = BTreeMap::new();

            for (event, _asset) in &events {
                let year = event.event_date.year();
                let entry = yearly.entry(year).or_insert(YearlyTotals {
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

            // Calculate totals by asset type
            let mut asset_type_totals: std::collections::HashMap<db::AssetType, Decimal> =
                std::collections::HashMap::new();
            for (event, asset) in &events {
                *asset_type_totals
                    .entry(asset.asset_type)
                    .or_insert(Decimal::ZERO) += event.total_amount;
            }
            let mut asset_type_vec: Vec<_> = asset_type_totals.iter().collect();
            asset_type_vec.sort_by(|a, b| b.1.cmp(a.1)); // Sort by amount descending

            if json_output {
                #[derive(Serialize)]
                struct JsonYearlyRow {
                    year: i32,
                    dividends: String,
                    jcp: String,
                    amortization: String,
                    total: String,
                }

                #[derive(Serialize)]
                struct JsonYearlySummary {
                    yearly: Vec<JsonYearlyRow>,
                    totals: JsonYearlyRow,
                    years_with_income: usize,
                    avg_per_year: String,
                }

                let yearly_rows: Vec<JsonYearlyRow> = yearly
                    .iter()
                    .map(|(yr, y)| {
                        let total = y.dividends + y.jcp + y.amortization;
                        JsonYearlyRow {
                            year: *yr,
                            dividends: y.dividends.to_string(),
                            jcp: y.jcp.to_string(),
                            amortization: y.amortization.to_string(),
                            total: total.to_string(),
                        }
                    })
                    .collect();

                let summary = JsonYearlySummary {
                    yearly: yearly_rows,
                    totals: JsonYearlyRow {
                        year: 0,
                        dividends: total_dividends.to_string(),
                        jcp: total_jcp.to_string(),
                        amortization: total_amortization.to_string(),
                        total: grand_total.to_string(),
                    },
                    years_with_income,
                    avg_per_year: avg_per_year.to_string(),
                };

                println!("{}", serde_json::to_string_pretty(&summary)?);
                return Ok(());
            }

            println!(
                "\n{} Income Summary (Yearly Breakdown)\n",
                "💰".cyan().bold()
            );

            #[derive(Tabled)]
            struct YearRow {
                #[tabled(rename = "Year")]
                year: String,
                #[tabled(rename = "Dividends")]
                dividends: String,
                #[tabled(rename = "JCP")]
                jcp: String,
                #[tabled(rename = "Amort")]
                amort: String,
                #[tabled(rename = "Total")]
                total: String,
            }

            let mut rows: Vec<YearRow> = yearly
                .iter()
                .map(|(yr, y)| {
                    let total = y.dividends + y.jcp + y.amortization;
                    YearRow {
                        year: yr.to_string(),
                        dividends: if y.dividends > Decimal::ZERO {
                            format_currency(y.dividends)
                        } else {
                            "-".to_string()
                        },
                        jcp: if y.jcp > Decimal::ZERO {
                            format_currency(y.jcp)
                        } else {
                            "-".to_string()
                        },
                        amort: if y.amortization > Decimal::ZERO {
                            format_currency(y.amortization)
                        } else {
                            "-".to_string()
                        },
                        total: format_currency(total),
                    }
                })
                .collect();

            rows.push(YearRow {
                year: "─────".to_string(),
                dividends: "───────────".to_string(),
                jcp: "───────────".to_string(),
                amort: "───────────".to_string(),
                total: "───────────".to_string(),
            });
            rows.push(YearRow {
                year: "TOTAL".to_string(),
                dividends: format_currency(total_dividends),
                jcp: format_currency(total_jcp),
                amort: format_currency(total_amortization),
                total: format_currency(grand_total),
            });

            let table = Table::new(&rows)
                .with(Style::rounded())
                .with(Modify::new(Columns::new(1..)).with(Alignment::right()))
                .to_string();
            println!("{}", table);

            println!("\n{} Subtotals by Type:", "📊".cyan().bold());
            if total_dividends > Decimal::ZERO {
                println!(
                    "  Dividends:    {}",
                    format_currency(total_dividends).green()
                );
            }
            if total_jcp > Decimal::ZERO {
                println!("  JCP:          {}", format_currency(total_jcp).green());
            }
            if total_amortization > Decimal::ZERO {
                println!(
                    "  Amortization: {}",
                    format_currency(total_amortization).yellow()
                );
            }
            println!(
                "  {} {}",
                "Total:".bold(),
                format_currency(grand_total).green().bold()
            );

            println!("\n{} Subtotals by Asset Type:", "📊".cyan().bold());
            for (asset_type, total) in asset_type_vec {
                println!(
                    "  {:12} {}",
                    format!("{:?}:", asset_type),
                    format_currency(*total).green()
                );
            }

            println!("\n{} Statistics:", "📈".cyan().bold());
            println!("  Years with income:  {}", years_with_income);
            println!(
                "  Average per year:   {}",
                format_currency(avg_per_year).cyan()
            );
            println!();
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
    json_output: bool,
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

    if json_output {
        let payload = serde_json::json!({
            "id": event_id,
            "ticker": ticker,
            "event_date": event_date.to_string(),
            "total_amount": total_amount.to_string(),
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else {
        println!("Income event added: {} {}", ticker, event_date);
    }

    Ok(())
}

async fn dispatch_tax_calculate(month_str: &str) -> Result<()> {
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
        println!(
            "\n{} No sales found for {}/{}\n",
            "ℹ".blue().bold(),
            month,
            year
        );
        return Ok(());
    }

    println!(
        "\n{} Swing Trade Tax Calculation - {}/{}\n",
        "💰".cyan().bold(),
        month,
        year
    );

    // Display results by tax category
    for calc in &calculations {
        println!(
            "{} {}",
            "Tax Category:".bold(),
            calc.category.display_name()
        );
        println!(
            "  Total Sales:      {}",
            format_currency(calc.total_sales).cyan()
        );
        println!(
            "  Total Cost Basis: {}",
            format_currency(calc.total_cost_basis).cyan()
        );
        println!(
            "  Gross Profit:     {}",
            format_currency(calc.total_profit).green()
        );
        println!(
            "  Gross Loss:       {}",
            format_currency(calc.total_loss).red()
        );

        let net_str = if calc.net_profit >= rust_decimal::Decimal::ZERO {
            format_currency(calc.net_profit).green()
        } else {
            format_currency(calc.net_profit).red()
        };
        println!("  Net P&L:          {}", net_str);

        // Show loss offset if applied
        if calc.loss_offset_applied > rust_decimal::Decimal::ZERO {
            println!(
                "  Loss Offset:      {} (from previous months)",
                format_currency(calc.loss_offset_applied).cyan()
            );
            println!(
                "  After Loss Offset: {}",
                format_currency(calc.profit_after_loss_offset).green()
            );
        }

        if calc.exemption_applied > rust_decimal::Decimal::ZERO {
            println!(
                "  Exemption:        {} (sales under R$20.000)",
                format_currency(calc.exemption_applied).yellow().bold()
            );
        }

        if calc.taxable_amount > rust_decimal::Decimal::ZERO {
            println!(
                "  Taxable Amount:   {}",
                format_currency(calc.taxable_amount).yellow()
            );
            let tax_rate_pct = calc.tax_rate * rust_decimal::Decimal::from(100);
            println!(
                "  Tax Rate:         {}",
                format!("{:.0}%", tax_rate_pct).yellow()
            );
            println!(
                "  {} {}",
                "Tax Due:".bold(),
                format_currency(calc.tax_due).red().bold()
            );
        } else if calc.profit_after_loss_offset < rust_decimal::Decimal::ZERO {
            println!(
                "  {} Loss to carry forward",
                format_currency(calc.net_profit.abs()).yellow().bold()
            );
        } else {
            println!("  {} No tax due (exempt)", "Tax Due:".bold().green());
        }

        println!();
    }

    // Summary
    let total_tax: rust_decimal::Decimal = calculations.iter().map(|c| c.tax_due).sum();

    if total_tax > rust_decimal::Decimal::ZERO {
        println!(
            "{} Total Tax Due for {}/{}: {}\n",
            "📋".cyan().bold(),
            month,
            year,
            format_currency(total_tax).red().bold()
        );

        // Generate DARF payments
        let darf_payments = tax::generate_darf_payments(calculations, year, month)?;

        if !darf_payments.is_empty() {
            println!("{} DARF Payments:\n", "💳".cyan().bold());

            for payment in &darf_payments {
                println!(
                    "  {} Code {}: {}",
                    "DARF".yellow().bold(),
                    payment.darf_code,
                    payment.description
                );
                println!("    Amount:   {}", format_currency(payment.tax_due).red());
                println!(
                    "    Due Date: {}",
                    payment.due_date.format("%d/%m/%Y").to_string().yellow()
                );
                println!();
            }

            println!(
                "{} Payment due by {}\n",
                "⏰".yellow(),
                darf_payments[0].due_date.format("%d/%m/%Y")
            );
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
    fn new() -> Self {
        Self {
            printer: crate::ui::progress::ProgressPrinter::new(false),
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

// Tests removed - dispatcher now works with clap Commands
// Integration tests in tests/ directory provide coverage
