//! Command dispatcher that routes both clap Commands and custom Command enums
//! to the appropriate handlers.
//!
//! This module provides a unified interface for command routing, making it easy
//! to switch between different command sources (CLI args vs interactive input).

pub mod performance;
use performance::dispatch_performance_show;

use crate::commands::Command;
use crate::ui::crossterm_engine::Spinner;
use crate::{cli, db, reports, tax};
use anyhow::Result;
use colored::Colorize;
use std::io::{stdout, Write};
use tracing::info;

/// Route a parsed command to its handler
pub async fn dispatch_command(command: Command, json_output: bool) -> Result<()> {
    match command {
        Command::Import { path, dry_run } => {
            // TODO: Wire up import handler
            eprintln!("Import command: {} (dry_run: {})", path, dry_run);
            Ok(())
        }
        Command::PortfolioShow { filter } => {
            dispatch_portfolio_show(filter.as_deref(), json_output).await
        }
        Command::PerformanceShow { period } => {
            dispatch_performance_show(&period, json_output).await
        }
        Command::TaxReport { year, export_csv } => {
            dispatch_tax_report(year, export_csv, json_output).await
        }
        Command::TaxSummary { year } => dispatch_tax_summary(year, json_output).await,
        Command::Help => {
            println!("Help: interest <command> [options]");
            println!("\nAvailable commands:");
            println!("  import <file>        - Import transactions");
            println!("  portfolio show       - Show portfolio");
            println!("  performance show <P> - Show performance (P: MTD|QTD|YTD|1Y|ALL|from:to)");
            println!("  tax report <year>    - Generate tax report");
            println!("  tax summary <year>   - Show tax summary");
            println!("  help                 - Show this help");
            println!("  exit                 - Exit application");
            Ok(())
        }
        Command::Exit => {
            std::process::exit(0);
        }
    }
}

async fn dispatch_portfolio_show(asset_type: Option<&str>, json_output: bool) -> Result<()> {
    info!("Generating portfolio report");

    // Initialize database
    db::init_database(None)?;
    let conn = db::open_db(None)?;

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

    // Calculate portfolio
    let report = reports::calculate_portfolio(&conn, asset_type_filter.as_ref())?;

    if report.positions.is_empty() {
        if !json_output {
            println!("{}", cli::formatters::format_empty_portfolio());
        }
        return Ok(());
    }

    if json_output {
        println!("{}", cli::formatters::format_portfolio_json(&report));
        return Ok(());
    }

    // Use formatter for table output
    let filter_str = asset_type.map(|f| f.to_uppercase());
    println!(
        "{}",
        cli::formatters::format_portfolio_table(&report, filter_str.as_deref())
    );

    // Display asset allocation if showing full portfolio
    if asset_type_filter.is_none() {
        let allocation = reports::calculate_allocation(&report);

        if allocation.len() > 1 {
            println!("\n{} Asset Allocation", "🎯".cyan().bold());

            let mut alloc_vec: Vec<_> = allocation.iter().collect();
            alloc_vec.sort_by(|a, b| b.1 .0.cmp(&a.1 .0));

            for (asset_type, (value, pct)) in alloc_vec {
                let type_ref: &db::AssetType = asset_type;
                println!(
                    "  {}: {} ({:.2}%)",
                    type_ref.as_str().to_uppercase(),
                    format!("R$ {:.2}", value).cyan(),
                    pct
                );
            }
        }
    }

    println!();
    Ok(())
}

async fn dispatch_tax_report(year: i32, export_csv: bool, _json_output: bool) -> Result<()> {
    info!("Generating IRPF annual report for {}", year);

    // Initialize database
    db::init_database(None)?;
    let conn = db::open_db(None)?;

    // Generate report with in-place spinner progress
    let mut printer = TaxProgressPrinter::new(true);
    let report = tax::generate_annual_report_with_progress(&conn, year, |ev| printer.on_event(ev))?;

    if report.monthly_summaries.is_empty() {
        println!(
            "\n{} No transactions found for year {}\n",
            "ℹ".blue().bold(),
            year
        );
        return Ok(());
    }

    println!(
        "\n{} Annual IRPF Tax Report - {}\n",
        "📊".cyan().bold(),
        year
    );

    // Show prior-year carryforward losses if any
    if !report.previous_losses_carry_forward.is_empty() {
        println!("{} Carryover from previous years:", "📦".yellow().bold());
        for (category, amount) in &report.previous_losses_carry_forward {
            println!("  {}: R$ {:.2}", category.display_name(), amount);
        }
        println!();
    }

    // Monthly breakdown
    println!("{}", "Monthly Summary:".bold());
    for summary in &report.monthly_summaries {
        println!("\n  {}:", summary.month_name.bold());
        println!(
            "    Sales:  {}",
            format!("R$ {:.2}", summary.total_sales).cyan()
        );
        println!(
            "    Profit: {}",
            format!("R$ {:.2}", summary.total_profit).green()
        );
        println!(
            "    Loss:   {}",
            format!("R$ {:.2}", summary.total_loss).red()
        );
        println!(
            "    Tax:    {}",
            format!("R$ {:.2}", summary.tax_due).yellow()
        );
    }

    // Annual totals
    println!("\n{} Annual Totals:", "📈".cyan().bold());
    println!(
        "  Total Sales:  {}",
        format!("R$ {:.2}", report.annual_total_sales).cyan()
    );
    println!(
        "  Total Profit: {}",
        format!("R$ {:.2}", report.annual_total_profit).green()
    );
    println!(
        "  Total Loss:   {}",
        format!("R$ {:.2}", report.annual_total_loss).red()
    );
    println!(
        "  {} {}\n",
        "Total Tax:".bold(),
        format!("R$ {:.2}", report.annual_total_tax).yellow().bold()
    );

    // Losses to carry forward
    if !report.losses_to_carry_forward.is_empty() {
        println!("{} Losses to Carry Forward:", "📋".yellow().bold());
        for (category, loss) in &report.losses_to_carry_forward {
            println!(
                "  {}: {}",
                category.display_name(),
                format!("R$ {:.2}", loss).yellow()
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
    let mut printer = TaxProgressPrinter::new(true);
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
            sales: format!("R$ {:.2}", s.total_sales),
            profit: format!("R$ {:.2}", s.total_profit),
            loss: format!("R$ {:.2}", s.total_loss),
            tax: format!("R$ {:.2}", s.tax_due),
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
        format!("R$ {:.2}", report.annual_total_sales).cyan()
    );
    println!(
        "  Profit: {}",
        format!("R$ {:.2}", report.annual_total_profit).green()
    );
    println!(
        "  Loss:   {}",
        format!("R$ {:.2}", report.annual_total_loss).red()
    );
    println!(
        "  {} {}\n",
        "Tax:".bold(),
        format!("R$ {:.2}", report.annual_total_tax).yellow().bold()
    );

    Ok(())
}

// Snapshot commands are intentionally internal-only; no public dispatcher.

struct TaxProgressPrinter {
    spinner: Spinner,
    in_place: bool,
    in_progress: bool,
    from_year: Option<i32>,
    target_year: Option<i32>,
    total_years: usize,
    completed_years: usize,
}

impl TaxProgressPrinter {
    fn new(in_place: bool) -> Self {
        Self {
            spinner: Spinner::new(),
            in_place,
            in_progress: false,
            from_year: None,
            target_year: None,
            total_years: 0,
            completed_years: 0,
        }
    }

    fn render_line(&mut self, text: &str) {
        if self.in_place {
            print!("\r\x1b[2K{} {}", self.spinner.tick(), text);
            let _ = stdout().flush();
        } else {
            println!("{} {}", self.spinner.tick(), text);
        }
    }

    fn finish_line(&mut self) {
        if self.in_place {
            println!();
            let _ = stdout().flush();
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
                self.render_line(&format!(
                    "↻ Recomputing snapshots {}/{} (starting {})",
                    self.completed_years, self.total_years, from_year
                ));
            }
            tax::ReportProgress::RecomputedYear { year } => {
                if self.in_progress {
                    self.completed_years = (self.completed_years + 1).min(self.total_years);
                    let from = self.from_year.unwrap_or(year);
                    if Some(year) == self.target_year {
                        // Finalize with a clean success line
                        if self.in_place {
                            print!("\r\x1b[2K");
                        }
                        println!("✓ Snapshots updated {}→{}", from, year);
                        let _ = stdout().flush();
                        self.in_progress = false;
                    } else {
                        self.render_line(&format!(
                            "↻ Recomputing snapshots {}/{} (year {})",
                            self.completed_years, self.total_years, year
                        ));
                    }
                }
            }
            tax::ReportProgress::TargetCacheHit { year } => {
                self.render_line(&format!("✓ Cache hit for {}; using cached carry", year));
                self.finish_line();
            }
            _ => {}
        }
    }
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
    async fn test_dispatch_portfolio_show() {
        let result = dispatch_command(Command::PortfolioShow { filter: None }, false).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_dispatch_import() {
        let result = dispatch_command(
            Command::Import {
                path: "test.xlsx".to_string(),
                dry_run: false,
            },
            false,
        )
        .await;
        assert!(result.is_ok());
    }
}
