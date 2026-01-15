//! Command dispatcher that routes both clap Commands and custom Command enums
//! to the appropriate handlers.
//!
//! This module provides a unified interface for command routing, making it easy
//! to switch between different command sources (CLI args vs interactive input).

pub mod performance;
use performance::dispatch_performance;

use crate::commands::Command;
mod actions;
mod imports;
pub mod imports_helpers;
mod inconsistencies;
mod portfolio;
mod prices;
mod tickers;
use crate::utils::format_currency;
use crate::{db, tax};
use anyhow::Result;
use colored::Colorize;
use tracing::info;

/// Route a parsed command to its handler
pub async fn dispatch_command(command: Command, json_output: bool) -> Result<()> {
    match command {
        Command::Import { action } => dispatch_import(action, json_output).await,
        Command::Portfolio { action } => portfolio::dispatch_portfolio(action, json_output).await,
        Command::Performance { action } => dispatch_performance(action, json_output).await,
        Command::Tax { action } => dispatch_tax(action, json_output).await,
        Command::Income { action } => dispatch_income(action, json_output).await,
        Command::Actions { action } => actions::dispatch_actions(action, json_output).await,
        Command::Prices { action } => prices::dispatch_prices(action, json_output).await,
        Command::Inconsistencies { action } => {
            inconsistencies::dispatch_inconsistencies(action, json_output).await
        }
        Command::Tickers { action } => tickers::dispatch_tickers(action, json_output).await,
        Command::Help => {
            println!("Help: interest <command> [options]");
            println!("\nAvailable commands:");
            println!("  import <file>              - Import transactions");
            println!(
                "  portfolio show [--at DATE] - Show portfolio (DATE: YYYY-MM-DD|YYYY-MM|YYYY)"
            );
            println!(
                "  performance show <P>       - Show performance (P: MTD|QTD|YTD|1Y|ALL|from:to)"
            );
            println!("  tax report <year>          - Generate tax report");
            println!("  tax summary <year>         - Show tax summary");
            println!("  income show [year]         - Show income summary by asset");
            println!("  income detail [year]       - Show detailed income events");
            println!(
                "  income summary [year]      - Show yearly totals (or monthly if year given)"
            );
            println!("  actions rename <...>       - Manage asset renames");
            println!("  actions split <...>        - Manage splits/reverse splits");
            println!("  actions bonus <...>        - Manage bonus actions");
            println!("  actions spinoff <...>      - Manage spin-off exchanges");
            println!("  actions merger <...>       - Manage merger exchanges");
            println!("  prices import-b3 <year>    - Import B3 COTAHIST data for year");
            println!("  prices import-b3-file <p>  - Import COTAHIST from local ZIP file");
            println!("  prices clear-cache [year]  - Clear B3 COTAHIST cache");
            println!("  inconsistencies list       - List inconsistencies");
            println!("  inconsistencies show <id>  - Show inconsistency details");
            println!("  inconsistencies resolve    - Resolve inconsistency");
            println!("  inconsistencies ignore     - Ignore inconsistency");
            println!("  tickers refresh            - Refresh B3 tickers cache");
            println!("  tickers status             - Show tickers cache status");
            println!("  tickers list-unknown        - List unknown assets");
            println!("  tickers resolve             - Resolve unknown assets");
            println!("  help                       - Show this help");
            println!("  exit                       - Exit application");
            Ok(())
        }
        Command::Exit => {
            std::process::exit(0);
        }
    }
}

async fn dispatch_tax(action: crate::commands::TaxAction, json_output: bool) -> Result<()> {
    match action {
        crate::commands::TaxAction::Report { year, export_csv } => {
            dispatch_tax_report(year, export_csv, json_output).await
        }
        crate::commands::TaxAction::Summary { year } => {
            dispatch_tax_summary(year, json_output).await
        }
    }
}

async fn dispatch_income(action: crate::commands::IncomeAction, json_output: bool) -> Result<()> {
    match action {
        crate::commands::IncomeAction::Show { year } => {
            dispatch_income_show(year, json_output).await
        }
        crate::commands::IncomeAction::Detail { year, asset } => {
            dispatch_income_detail(year, asset.as_deref(), json_output).await
        }
        crate::commands::IncomeAction::Summary { year } => {
            dispatch_income_summary(year, json_output).await
        }
    }
}

async fn dispatch_import(action: crate::commands::ImportAction, json_output: bool) -> Result<()> {
    imports::dispatch_import(action, json_output).await
}

async fn dispatch_tax_report(year: i32, export_csv: bool, json_output: bool) -> Result<()> {
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

    if report.monthly_summaries.is_empty() {
        println!(
            "\n{} No transactions found for year {}\n",
            "ℹ".blue().bold(),
            year
        );
        return Ok(());
    }

    if json_output {
        // Emit concise JSON suitable for tests and scripting
        #[derive(serde::Serialize)]
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

        let payload = serde_json::json!({
            "year": year,
            "annual_total_sales": report.annual_total_sales,
            "annual_total_profit": report.annual_total_profit,
            "annual_total_loss": report.annual_total_loss,
            "annual_total_tax": report.annual_total_tax,
            "monthly_summaries": monthly,
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

    if export_csv {
        let csv_content = tax::irpf::export_to_csv(&report);
        let csv_path = format!("irpf_report_{}.csv", year);
        std::fs::write(&csv_path, csv_content)?;

        println!("{} Report exported to: {}\n", "✓".green().bold(), csv_path);
    }

    Ok(())
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
                self.printer.update(&format!(
                    "↻ Recomputing snapshots {}/{} (starting {})",
                    self.completed_years, self.total_years, from_year
                ));
            }
            tax::ReportProgress::RecomputedYear { year } => {
                if self.in_progress {
                    self.completed_years = (self.completed_years + 1).min(self.total_years);
                    let from = self.from_year.unwrap_or(year);
                    if Some(year) == self.target_year {
                        self.printer.finish(true, &format!("Snapshots updated {}→{}", from, year));
                        self.in_progress = false;
                    } else {
                        self.printer.update(&format!(
                            "↻ Recomputing snapshots {}/{} (year {})",
                            self.completed_years, self.total_years, year
                        ));
                    }
                }
            }
            tax::ReportProgress::TargetCacheHit { year } => {
                self.printer
                    .persist(&format!("✓ Cache hit for {}; using cached carry", year));
            }
            _ => {}
        }
    }
}

/// Parse a progress message to extract the completion count.
/// Messages like "TICKER → R$ XX.XX (N/M)" return Some(N).
/// Returns None if the message doesn't match the expected format.
fn parse_progress_count(msg: &str) -> Option<usize> {
    if !msg.contains("→") {
        return None;
    }
    let paren_start = msg.rfind('(')?;
    let slash_offset = msg[paren_start..].find('/')?;
    msg[paren_start + 1..paren_start + slash_offset]
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_dispatch_help_command() {
        let result = dispatch_command(Command::Help, false).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_dispatch_exit_command() {
        // We can't really test exit, but we can check it would be called
        // In reality, this would exit the process
    }

    #[tokio::test]
    async fn test_dispatch_import() {
        let result = dispatch_command(
            Command::Import {
                action: crate::commands::ImportAction::File {
                    path: "test.xlsx".to_string(),
                    dry_run: false,
                },
            },
            false,
        )
        .await;
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_progress_count_valid() {
        assert_eq!(parse_progress_count("PETR4 → R$ 35.50 (1/35)"), Some(1));
        assert_eq!(parse_progress_count("HGLG11 → R$ 156.99 (15/35)"), Some(15));
        assert_eq!(
            parse_progress_count("VALE3 → R$ 58.20 (100/100)"),
            Some(100)
        );
    }

    #[test]
    fn test_parse_progress_count_failed() {
        assert_eq!(parse_progress_count("PETR4 → failed (5/35)"), Some(5));
    }

    #[test]
    fn test_parse_progress_count_no_arrow() {
        assert_eq!(parse_progress_count("Checking 35 assets..."), None);
        assert_eq!(parse_progress_count("✓ All prices are up to date!"), None);
    }

    #[test]
    fn test_parse_progress_count_no_parens() {
        assert_eq!(parse_progress_count("PETR4 → R$ 35.50"), None);
    }
}
