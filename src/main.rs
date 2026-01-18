mod cli;
mod commands;
mod corporate_actions;
mod db;
mod dispatcher;
mod importers;
mod pricing;
mod reports;
mod scraping;
mod tax;
mod term_contracts;
mod tesouro;
mod tickers;
mod ui;
mod utils;

use anyhow::Result;
use clap::Parser;
use cli::{
    runner, Cli, Commands, InconsistenciesCommands, PriceCommands, TaxCommands, TransactionCommands,
};
use rust_decimal::Decimal;
use serde::Serialize;
use std::io::IsTerminal;
use tracing::info;
use tracing_subscriber::EnvFilter;
use utils::format_currency;

// JSON response utilities
#[derive(Serialize)]
struct JsonResponse<T> {
    success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

fn json_success<T: Serialize>(data: T) -> String {
    serde_json::to_string_pretty(&JsonResponse {
        success: true,
        data: Some(data),
        error: None,
    })
    .unwrap_or_else(|e| {
        format!(
            r#"{{"success": false, "error": "JSON serialization error: {}"}}"#,
            e
        )
    })
}

// Note: json_error removed as unused; add back when standardized error JSON is needed

#[tokio::main]
async fn main() -> Result<()> {
    // Parse CLI first to configure logging and color
    let cli = Cli::parse();

    // Determine color usage: disable when requested or when stdout is not a TTY (piped)
    let stdout_is_tty = std::io::stdout().is_terminal();
    let disable_color = cli.no_color || !stdout_is_tty || cli.json;

    // Initialize logging - always write to stderr to keep stdout clean
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("warn"))
        .add_directive("headless_chrome=error".parse().unwrap());

    tracing_subscriber::fmt()
        .with_ansi(!disable_color)
        .with_writer(std::io::stderr)
        .with_env_filter(env_filter)
        .init();

    // Disable colored crate globally when needed
    if disable_color {
        colored::control::set_override(false);
    }

    // Default to interactive mode if no command given
    let command = match cli.command {
        Some(cmd) => cmd,
        None => {
            return interest::ui::launch_tui().await;
        }
    };

    match runner::to_internal_command(&command) {
        Ok(Some(internal)) => dispatcher::dispatch_command(internal, cli.json).await,
        Ok(None) => {
            // Fallback: handle commands that require special treatment or were not converted
            match command {
                Commands::Import { file, dry_run } => handle_import(&file, dry_run, cli.json).await,

                Commands::ImportIrpf {
                    file,
                    year,
                    dry_run,
                } => handle_irpf_import(&file, year, dry_run).await,

                Commands::Prices { action } => match action {
                    PriceCommands::Update => handle_price_update().await,
                    PriceCommands::History { ticker, from, to } => {
                        handle_price_history(&ticker, &from, &to).await
                    }
                },

                Commands::Transactions { action } => match action {
                    TransactionCommands::Add {
                        ticker,
                        transaction_type,
                        quantity,
                        price,
                        date,
                        fees,
                        notes,
                    } => {
                        handle_transaction_add(
                            &ticker,
                            &transaction_type,
                            &quantity,
                            &price,
                            &date,
                            &fees,
                            notes.as_deref(),
                        )
                        .await
                    }
                },

                Commands::Inspect { file, full, column } => {
                    handle_inspect(&file, full, column).await
                }

                Commands::Interactive => interest::ui::launch_tui().await,

                Commands::ProcessTerms => handle_process_terms().await,

                Commands::Tax { action } => match action {
                    TaxCommands::Calculate { month } => handle_tax_calculate(&month).await,
                    _ => Err(anyhow::anyhow!("Unimplemented tax subcommand")),
                },

                Commands::Inconsistencies { action } => match action {
                    InconsistenciesCommands::List {
                        open,
                        all,
                        status,
                        issue_type,
                        asset,
                    } => {
                        let status = if let Some(status) = status {
                            Some(status)
                        } else if all {
                            Some("ALL".to_string())
                        } else if open || !all {
                            Some("OPEN".to_string())
                        } else {
                            None
                        };

                        dispatcher::dispatch_command(
                            commands::Command::Inconsistencies {
                                action: commands::InconsistenciesAction::List {
                                    status,
                                    issue_type,
                                    asset,
                                },
                            },
                            cli.json,
                        )
                        .await
                    }

                    InconsistenciesCommands::Show { id } => {
                        dispatcher::dispatch_command(
                            commands::Command::Inconsistencies {
                                action: commands::InconsistenciesAction::Show { id },
                            },
                            cli.json,
                        )
                        .await
                    }

                    InconsistenciesCommands::Resolve {
                        id,
                        set,
                        json_payload,
                    } => {
                        let mut pairs = Vec::new();
                        for item in set {
                            if let Some((k, v)) = item.split_once('=') {
                                pairs.push((k.to_string(), v.to_string()));
                            }
                        }

                        dispatcher::dispatch_command(
                            commands::Command::Inconsistencies {
                                action: commands::InconsistenciesAction::Resolve {
                                    id,
                                    set: pairs,
                                    json: json_payload,
                                },
                            },
                            cli.json,
                        )
                        .await
                    }

                    InconsistenciesCommands::Ignore { id, reason } => {
                        dispatcher::dispatch_command(
                            commands::Command::Inconsistencies {
                                action: commands::InconsistenciesAction::Ignore { id, reason },
                            },
                            cli.json,
                        )
                        .await
                    }
                },

                _ => Err(anyhow::anyhow!("Unimplemented command")),
            }
        }
        Err(e) => Err(anyhow::anyhow!("{}", e)),
    }
}

/// Handle import command with automatic format detection
async fn handle_import(file_path: &str, dry_run: bool, json_output: bool) -> Result<()> {
    use colored::Colorize;
    use tabled::{
        settings::{object::Columns, Alignment, Modify, Style},
        Table, Tabled,
    };

    info!("Importing from: {}", file_path);

    // Auto-detect file type and parse
    let import_result = importers::import_file_auto(file_path)?;

    match import_result {
        importers::ImportResult::Cei(raw_transactions) => {
            // Handle CEI format
            info!("Detected CEI format");
            println!(
                "\n{} Found {} transactions\n",
                "✓".green().bold(),
                raw_transactions.len()
            );

            if !json_output {
                if let Some(table) =
                    crate::dispatcher::imports_helpers::preview_cei_table(&raw_transactions)
                {
                    println!("{}", table);
                    if raw_transactions.len() > 10 {
                        println!(
                            "\n... and {} more transactions",
                            raw_transactions.len() - 10
                        );
                    }
                }
            }

            if dry_run {
                println!("\n{} Dry run - no changes saved", "ℹ".blue().bold());
                return Ok(());
            }

            // Initialize database and open connection
            db::init_database(None)?;
            let conn = db::open_db(None)?;

            let stats = crate::dispatcher::imports_helpers::import_cei(&conn, &raw_transactions)?;

            if stats.imported > 0 {
                if let Some(date) = stats.earliest {
                    if !json_output {
                        println!(
                            "  {} Snapshots on/after {} invalidated",
                            "⚠".yellow().bold(),
                            date
                        );
                    }
                }
            }

            println!("\n{} Import complete!", "✓".green().bold());
            println!("  Imported: {}", stats.imported.to_string().green());
            if stats.skipped_old > 0 {
                println!(
                    "  Skipped (before last import date): {}",
                    stats.skipped_old.to_string().yellow()
                );
            }
            if stats.errors > 0 {
                println!("  Errors: {}", stats.errors.to_string().red());
            }

            Ok(())
        }

        importers::ImportResult::Movimentacao(entries) => {
            // Handle Movimentacao format
            info!("Detected Movimentacao format");

            if !json_output {
                println!(
                    "\n{} Found {} movimentacao entries\n",
                    "✓".green().bold(),
                    entries.len()
                );
            }

            // Categorize entries
            let trades: Vec<_> = entries.iter().filter(|e| e.is_trade()).collect();
            let mut corporate_actions: Vec<_> =
                entries.iter().filter(|e| e.is_corporate_action()).collect();
            corporate_actions.sort_by_key(|e| e.date);
            let income_events: Vec<_> = entries.iter().filter(|e| e.is_income_event()).collect();
            let other: Vec<_> = entries
                .iter()
                .filter(|e| !e.is_trade() && !e.is_corporate_action() && !e.is_income_event())
                .collect();

            if !json_output {
                println!("{} Summary:", "📊".cyan().bold());
                println!(
                    "  {} Trades (buy/sell/term)",
                    trades.len().to_string().green()
                );
                println!(
                    "  {} Corporate actions (splits, bonuses, mergers)",
                    corporate_actions.len().to_string().yellow()
                );
                println!(
                    "  {} Income events (dividends, yields, amortization)",
                    income_events.len().to_string().cyan()
                );
                println!("  {} Other movements", other.len().to_string().dimmed());
                println!();
            }

            // Show preview of trades
            if !json_output && !trades.is_empty() {
                println!("{} Sample trades:", "💰".cyan().bold());
                let cloned_trades: Vec<_> = trades.iter().map(|e| (*e).clone()).collect();
                if let Some(table) =
                    crate::dispatcher::imports_helpers::preview_movimentacao_trades(&cloned_trades)
                {
                    println!("{}\n", table);
                }
            }

            // Show preview of corporate actions
            if !json_output && !corporate_actions.is_empty() {
                println!("{} Corporate actions:", "🏢".cyan().bold());

                for action in corporate_actions.iter().take(5) {
                    println!(
                        "  {} {} - {}",
                        action.date.format("%d/%m/%Y").to_string().dimmed(),
                        action.movement_type.yellow(),
                        action.ticker.as_ref().unwrap_or(&action.product)
                    );
                }
                println!();
            }

            // Show preview of income events
            if !json_output && !income_events.is_empty() {
                println!("{} Income events:", "💵".cyan().bold());

                for event in income_events.iter().take(5) {
                    let value = event
                        .operation_value
                        .map(format_currency)
                        .unwrap_or_else(|| "-".to_string());

                    println!(
                        "  {} {} - {} {}",
                        event.date.format("%d/%m/%Y").to_string().dimmed(),
                        event.movement_type.cyan(),
                        event.ticker.as_ref().unwrap_or(&event.product),
                        value.green()
                    );
                }
                println!();
            }

            if dry_run {
                println!("\n{} Dry run - no changes saved", "ℹ".blue().bold());
                println!("\n{} What would be imported:", "📝".cyan().bold());
                println!("  • {} trade transactions", trades.len());
                println!("  • {} corporate actions", corporate_actions.len());
                println!(
                    "  • {} income events (not yet implemented)",
                    income_events.len()
                );
                return Ok(());
            }

            // Initialize database
            db::init_database(None)?;
            let conn = db::open_db(None)?;

            if !json_output {
                println!(
                    "{} Importing trades, corporate actions, and income events...",
                    "⏳".cyan().bold()
                );
            }
            let stats = importers::import_movimentacao_entries(&conn, entries, true)?;

            if json_output {
                println!("{}", json_success(stats));
                return Ok(());
            }

            println!("\n{} Import complete!", "✓".green().bold());
            println!("  {} Trades:", "💰".cyan());
            println!(
                "    Imported: {}",
                stats.imported_trades.to_string().green()
            );
            if stats.skipped_trades_old > 0 {
                println!(
                    "    Skipped (before last import date): {}",
                    stats.skipped_trades_old.to_string().yellow()
                );
            }
            if stats.skipped_trades > 0 {
                println!("    Skipped: {}", stats.skipped_trades.to_string().yellow());
            }
            println!("  {} Corporate actions:", "🏢".cyan());
            println!(
                "    Imported: {}",
                stats.imported_actions.to_string().green()
            );
            if stats.skipped_actions_old > 0 {
                println!(
                    "    Skipped (before last import date): {}",
                    stats.skipped_actions_old.to_string().yellow()
                );
            }
            if stats.skipped_actions > 0 {
                println!(
                    "    Skipped: {}",
                    stats.skipped_actions.to_string().yellow()
                );
            }
            if stats.errors > 0 {
                println!(
                    "  {} Errors: {}",
                    "❌".red(),
                    stats.errors.to_string().red()
                );
            }
            println!("  {} Income events:", "💵".cyan());
            println!(
                "    Imported: {}",
                stats.imported_income.to_string().green()
            );
            if stats.skipped_income_old > 0 {
                println!(
                    "    Skipped (before last import date): {}",
                    stats.skipped_income_old.to_string().yellow()
                );
            }
            if stats.skipped_income > 0 {
                println!(
                    "    Skipped (duplicates): {}",
                    stats.skipped_income.to_string().yellow()
                );
            }

            Ok(())
        }

        importers::ImportResult::OfertasPublicas(entries) => {
            info!("Detected Ofertas Públicas format");
            println!(
                "\n{} Found {} ofertas públicas entries\n",
                "✓".green().bold(),
                entries.len()
            );

            #[derive(Tabled)]
            struct OfertaPreview {
                #[tabled(rename = "Date")]
                date: String,
                #[tabled(rename = "Ticker")]
                ticker: String,
                #[tabled(rename = "Qty")]
                quantity: String,
                #[tabled(rename = "Price")]
                price: String,
                #[tabled(rename = "Offer")]
                offer: String,
            }

            let preview: Vec<OfertaPreview> = entries
                .iter()
                .take(5)
                .map(|e| OfertaPreview {
                    date: e.date.format("%d/%m/%Y").to_string(),
                    ticker: e.ticker.clone(),
                    quantity: e.quantity.to_string(),
                    price: format_currency(e.unit_price),
                    offer: e.offer.clone(),
                })
                .collect();

            let table = Table::new(preview)
                .with(Style::rounded())
                .with(Modify::new(Columns::new(2..4)).with(Alignment::right()))
                .to_string();
            println!("{}\n", table);

            if dry_run {
                println!("\n{} Dry run - no changes saved", "ℹ".blue().bold());
                println!("\n{} What would be imported:", "📝".cyan().bold());
                println!("  • {} offer allocation transactions", entries.len());
                return Ok(());
            }

            db::init_database(None)?;
            let conn = db::open_db(None)?;

            let mut imported = 0;
            let mut skipped_old = 0;
            let mut errors = 0;
            let mut max_date: Option<chrono::NaiveDate> = None;

            let last_import_date =
                db::get_last_import_date(&conn, "OFERTAS_PUBLICAS", "allocations")?;

            println!("{} Importing offer allocations...", "⏳".cyan().bold());

            for entry in entries {
                let asset_type = db::AssetType::Unknown;

                let asset_id = match db::upsert_asset(&conn, &entry.ticker, &asset_type, None) {
                    Ok(id) => id,
                    Err(e) => {
                        eprintln!("Error upserting asset {}: {}", entry.ticker, e);
                        errors += 1;
                        continue;
                    }
                };

                if let Some(last_date) = last_import_date {
                    if entry.date <= last_date {
                        skipped_old += 1;
                        continue;
                    }
                }

                let transaction = match entry.to_transaction(asset_id) {
                    Ok(tx) => tx,
                    Err(e) => {
                        eprintln!("Error converting offer to transaction: {}", e);
                        errors += 1;
                        continue;
                    }
                };

                match db::insert_transaction(&conn, &transaction) {
                    Ok(_) => {
                        imported += 1;
                        max_date = Some(match max_date {
                            Some(current) if current >= transaction.trade_date => current,
                            _ => transaction.trade_date,
                        });
                    }
                    Err(e) => {
                        eprintln!("Error inserting offer transaction: {}", e);
                        errors += 1;
                    }
                }
            }

            if let Some(last_date) = max_date {
                db::set_last_import_date(&conn, "OFERTAS_PUBLICAS", "allocations", last_date)?;
            }

            println!("\n{} Import complete!", "✓".green().bold());
            println!("  Imported: {}", imported.to_string().green());
            if skipped_old > 0 {
                println!(
                    "  Skipped (before last import date): {}",
                    skipped_old.to_string().yellow()
                );
            }
            if errors > 0 {
                println!("  Errors: {}", errors.to_string().red());
            }

            Ok(())
        }
    }
}

/// Handle IRPF PDF import command
async fn handle_irpf_import(file_path: &str, year: i32, dry_run: bool) -> Result<()> {
    use colored::Colorize;
    use tabled::{
        settings::{object::Columns, Alignment, Modify, Style},
        Table, Tabled,
    };

    info!(
        "Importing IRPF positions from: {} for year {}",
        file_path, year
    );

    // Parse IRPF PDF for positions and loss carryforward
    let positions = importers::irpf_pdf::parse_irpf_pdf(file_path, year)?;
    let losses = importers::irpf_pdf::parse_irpf_pdf_losses(file_path, year).unwrap_or_else(|e| {
        info!("Could not parse loss carryforward: {}", e);
        importers::irpf_pdf::IrpfLossCarryforward {
            year,
            stock_swing_loss: Decimal::ZERO,
            stock_day_loss: Decimal::ZERO,
            fii_fiagro_loss: Decimal::ZERO,
        }
    });

    // Check if there's anything to import (positions or losses)
    let has_losses = losses.stock_swing_loss > Decimal::ZERO
        || losses.stock_day_loss > Decimal::ZERO
        || losses.fii_fiagro_loss > Decimal::ZERO;

    if positions.is_empty() && !has_losses {
        println!(
            "\n{} No positions or loss carryforward found for year {}",
            "ℹ".yellow().bold(),
            year
        );
        println!("Check that the PDF contains 'DECLARAÇÃO DE BENS E DIREITOS' section with Code 31 entries.");
        return Ok(());
    }

    // Display what was found
    if !positions.is_empty() {
        println!(
            "\n{} Found {} opening position(s) from IRPF {}\n",
            "✓".green().bold(),
            positions.len(),
            year
        );

        // Display preview
        #[derive(Tabled)]
        struct PositionPreview {
            #[tabled(rename = "Ticker")]
            ticker: String,
            #[tabled(rename = "Quantity")]
            quantity: String,
            #[tabled(rename = "Total Cost")]
            total_cost: String,
            #[tabled(rename = "Avg Cost")]
            avg_cost: String,
            #[tabled(rename = "Date")]
            date: String,
        }

        let preview: Vec<PositionPreview> = positions
            .iter()
            .map(|pos| PositionPreview {
                ticker: pos.ticker.clone(),
                quantity: pos.quantity.to_string(),
                total_cost: format_currency(pos.total_cost),
                avg_cost: format_currency(pos.average_cost),
                date: format!("31/12/{}", pos.year),
            })
            .collect();

        let table = Table::new(preview)
            .with(Style::rounded())
            .with(Modify::new(Columns::new(1..4)).with(Alignment::right()))
            .to_string();
        println!("{}", table);
    }

    if has_losses {
        println!(
            "\n{} Found loss carryforward for year {}\n",
            "✓".green().bold(),
            year
        );
        if losses.stock_swing_loss > Decimal::ZERO {
            println!(
                "  • Stock Swing Trade: {}",
                format_currency(losses.stock_swing_loss)
            );
        }
        if losses.stock_day_loss > Decimal::ZERO {
            println!(
                "  • Stock Day Trade: {}",
                format_currency(losses.stock_day_loss)
            );
        }
        if losses.fii_fiagro_loss > Decimal::ZERO {
            println!(
                "  • FII/FIAGRO: {}",
                format_currency(losses.fii_fiagro_loss)
            );
        }
    }

    if dry_run {
        println!("\n{} Dry run - no changes saved", "ℹ".blue().bold());
        println!("\n{} What would be imported:", "📝".cyan().bold());
        if !positions.is_empty() {
            println!(
                "  • {} opening BUY transactions dated {}-12-31",
                positions.len(),
                year
            );
            println!("  • Previous IRPF opening positions for these tickers would be deleted");
        }
        if has_losses {
            println!("  • Loss carryforward snapshot would be created:");
            if losses.stock_swing_loss > Decimal::ZERO {
                println!(
                    "    - Stock Swing Trade: {}",
                    format_currency(losses.stock_swing_loss)
                );
            }
            if losses.stock_day_loss > Decimal::ZERO {
                println!(
                    "    - Stock Day Trade: {}",
                    format_currency(losses.stock_day_loss)
                );
            }
            if losses.fii_fiagro_loss > Decimal::ZERO {
                println!(
                    "    - FII/FIAGRO: {}",
                    format_currency(losses.fii_fiagro_loss)
                );
            }
        }
        return Ok(());
    }

    // Initialize database
    db::init_database(None)?;
    let conn = db::open_db(None)?;

    // Import positions
    let mut imported = 0;
    let mut replaced = 0;

    if !positions.is_empty() {
        println!("\n{} Importing opening positions...\n", "⏳".cyan().bold());

        for position in positions {
            // Detect asset type from ticker
            let asset_type = db::AssetType::Unknown;

            // Upsert asset
            let asset_id = match db::upsert_asset(&conn, &position.ticker, &asset_type, None) {
                Ok(id) => id,
                Err(e) => {
                    eprintln!(
                        "{} Error upserting asset {}: {}",
                        "✗".red(),
                        position.ticker,
                        e
                    );
                    continue;
                }
            };

            // Check if there are existing IRPF opening positions for this ticker
            let existing_count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM transactions WHERE asset_id = ?1 AND source = 'IRPF_PDF'",
                rusqlite::params![asset_id],
                |row| row.get(0),
            )?;

            // Delete existing IRPF opening positions for this ticker
            if existing_count > 0 {
                conn.execute(
                    "DELETE FROM transactions WHERE asset_id = ?1 AND source = 'IRPF_PDF'",
                    rusqlite::params![asset_id],
                )?;
                replaced += 1;
                println!(
                    "{} Replaced {} existing IRPF position(s) for {}",
                    "↻".yellow(),
                    existing_count,
                    position.ticker.cyan()
                );
            }

            // Convert to opening transaction
            let transaction = match position.to_opening_transaction(asset_id) {
                Ok(tx) => tx,
                Err(e) => {
                    eprintln!(
                        "{} Error converting position for {}: {}",
                        "✗".red(),
                        position.ticker,
                        e
                    );
                    continue;
                }
            };

            // Insert opening transaction
            match db::insert_transaction(&conn, &transaction) {
                Ok(_) => {
                    println!(
                        "{} Added opening position: {} {} @ {}",
                        "✓".green(),
                        position.quantity,
                        position.ticker.cyan(),
                        format_currency(position.average_cost)
                    );
                    imported += 1;
                }
                Err(e) => {
                    eprintln!(
                        "{} Error inserting transaction for {}: {}",
                        "✗".red(),
                        position.ticker,
                        e
                    );
                }
            }
        }

        println!("\n{} Import complete!", "✓".green().bold());
        println!("  Imported: {}", imported.to_string().green());
        if replaced > 0 {
            println!(
                "  Replaced: {} (previous IRPF positions)",
                replaced.to_string().yellow()
            );
        }

        // Set import cutoff to prevent older CEI/Movimentação imports
        let year_end = chrono::NaiveDate::from_ymd_opt(year, 12, 31)
            .ok_or_else(|| anyhow::anyhow!("Invalid year: {}", year))?;

        db::set_last_import_date(&conn, "CEI", "trades", year_end)?;
        db::set_last_import_date(&conn, "MOVIMENTACAO", "trades", year_end)?;
        db::set_last_import_date(&conn, "MOVIMENTACAO", "corporate_actions", year_end)?;

        println!(
            "\n{} Set import cutoff to {} for CEI and Movimentação",
            "ℹ".blue().bold(),
            year_end.format("%Y-%m-%d")
        );
        println!(
            "  This prevents importing older data that conflicts with these IRPF opening positions"
        );
    }

    // Import loss carryforward if any losses exist
    if has_losses {
        println!(
            "\n{} Importing loss carryforward snapshot for year {}",
            "⏳".cyan().bold(),
            year
        );

        // Create a snapshot with the extracted losses
        // Compute a fingerprint from the year's transactions so the snapshot matches cache lookups
        let fingerprint = match tax::loss_carryforward::compute_year_fingerprint(&conn, year) {
            Ok(fp) => fp,
            Err(e) => {
                eprintln!(
                    "  {} Warning: Could not compute year fingerprint: {}; using 'irpf_import'",
                    "⚠".yellow(),
                    e
                );
                "irpf_import".to_string()
            }
        };
        let mut loss_carry = std::collections::HashMap::new();

        if losses.stock_swing_loss > Decimal::ZERO {
            loss_carry.insert(
                tax::swing_trade::TaxCategory::StockSwingTrade,
                losses.stock_swing_loss,
            );
        }
        if losses.stock_day_loss > Decimal::ZERO {
            loss_carry.insert(
                tax::swing_trade::TaxCategory::StockDayTrade,
                losses.stock_day_loss,
            );
        }
        if losses.fii_fiagro_loss > Decimal::ZERO {
            // FII/FIAGRO losses are combined in the PDF, so split proportionally or use FII category
            // For now, assign to FII swing trade (most common)
            loss_carry.insert(
                tax::swing_trade::TaxCategory::FiiSwingTrade,
                losses.fii_fiagro_loss,
            );
        }

        match tax::loss_carryforward::upsert_snapshot(&conn, year, &fingerprint, &loss_carry) {
            Ok(_) => {
                println!("  {} Loss carryforward snapshot imported", "✓".green());
                for (category, amount) in &loss_carry {
                    println!(
                        "    • {}: {}",
                        category.display_name(),
                        format_currency(*amount)
                    );
                }
            }
            Err(e) => {
                eprintln!(
                    "  {} Warning: Could not import loss carryforward: {}",
                    "⚠".yellow(),
                    e
                );
            }
        }
    }

    println!(
        "\n{} These opening positions will be used for cost basis calculations",
        "ℹ".blue().bold()
    );
    println!(
        "  Run 'interest tax calculate <month>' to see tax calculations with these cost bases\n"
    );

    Ok(())
}

/// Handle price update command
async fn handle_price_update() -> Result<()> {
    use colored::Colorize;
    use pricing::PriceFetcher;

    info!("Updating all asset prices");

    // Initialize database
    db::init_database(None)?;
    let conn = db::open_db(None)?;

    // Get all assets
    let assets = db::get_all_assets(&conn)?;

    if assets.is_empty() {
        println!("{} No assets found in database", "ℹ".blue().bold());
        println!("Import transactions first using: interest import <file>");
        return Ok(());
    }

    println!(
        "\n{} Updating prices for {} assets\n",
        "→".cyan().bold(),
        assets.len()
    );

    let fetcher = PriceFetcher::new();
    let mut updated = 0;
    let mut errors = 0;

    for asset in &assets {
        print!("  {} {}... ", asset.ticker, "→".cyan());

        match fetcher.fetch_price(&asset.ticker).await {
            Ok(price) => {
                // Store price in database
                let price_history = db::PriceHistory {
                    id: None,
                    asset_id: asset.id.unwrap(),
                    price_date: chrono::Utc::now().date_naive(),
                    close_price: price,
                    open_price: None,
                    high_price: None,
                    low_price: None,
                    volume: None,
                    source: "YAHOO".to_string(),
                    created_at: chrono::Utc::now(),
                };

                match db::insert_price_history(&conn, &price_history) {
                    Ok(_) => {
                        println!("{} {}", "✓".green(), format_currency(price));
                        updated += 1;
                    }
                    Err(e) => {
                        println!("{} {}", "✗".red(), e);
                        errors += 1;
                    }
                }
            }
            Err(e) => {
                println!("{} {}", "✗".red(), e);
                errors += 1;
            }
        }
    }

    println!("\n{} Price update complete!", "✓".green().bold());
    println!("  Updated: {}", updated.to_string().green());
    if errors > 0 {
        println!("  Errors: {}", errors.to_string().red());
    }

    Ok(())
}

/// Handle historical price fetching
async fn handle_price_history(ticker: &str, from: &str, to: &str) -> Result<()> {
    use anyhow::Context;
    use chrono::NaiveDate;
    use colored::Colorize;
    use tabled::{
        settings::{object::Columns, Alignment, Modify, Style},
        Table, Tabled,
    };

    info!(
        "Fetching historical prices for {} from {} to {}",
        ticker, from, to
    );

    let from_date = NaiveDate::parse_from_str(from, "%Y-%m-%d")
        .context("Invalid from date. Use YYYY-MM-DD format")?;
    let to_date = NaiveDate::parse_from_str(to, "%Y-%m-%d")
        .context("Invalid to date. Use YYYY-MM-DD format")?;

    println!(
        "\n{} Fetching historical prices for {}",
        "→".cyan().bold(),
        ticker
    );

    let prices = pricing::yahoo::fetch_historical_prices(ticker, from_date, to_date).await?;

    if prices.is_empty() {
        println!("{} No price data found", "ℹ".blue().bold());
        return Ok(());
    }

    // Display prices in table
    #[derive(Tabled)]
    struct PriceRow {
        #[tabled(rename = "Date")]
        date: String,
        #[tabled(rename = "Open")]
        open: String,
        #[tabled(rename = "High")]
        high: String,
        #[tabled(rename = "Low")]
        low: String,
        #[tabled(rename = "Close")]
        close: String,
        #[tabled(rename = "Volume")]
        volume: String,
    }

    let rows: Vec<PriceRow> = prices
        .iter()
        .map(|p| PriceRow {
            date: p.date.format("%Y-%m-%d").to_string(),
            open: p
                .open
                .as_ref()
                .map(|o| format_currency(*o))
                .unwrap_or_else(|| "-".to_string()),
            high: p
                .high
                .as_ref()
                .map(|h| format_currency(*h))
                .unwrap_or_else(|| "-".to_string()),
            low: p
                .low
                .as_ref()
                .map(|l| format_currency(*l))
                .unwrap_or_else(|| "-".to_string()),
            close: format_currency(p.close),
            volume: p
                .volume
                .map(|v| v.to_string())
                .unwrap_or_else(|| "-".to_string()),
        })
        .collect();

    let table = Table::new(rows)
        .with(Style::rounded())
        .with(Modify::new(Columns::new(1..)).with(Alignment::right()))
        .to_string();
    println!("\n{}", table);
    println!(
        "\n{} Total: {} price points",
        "✓".green().bold(),
        prices.len()
    );

    Ok(())
}

/// Handle tax calculation for a specific month
async fn handle_tax_calculate(month_str: &str) -> Result<()> {
    use anyhow::Context;
    use colored::Colorize;

    info!("Calculating swing trade tax for {}", month_str);

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

/// Handle manual transaction add command
async fn handle_transaction_add(
    ticker: &str,
    transaction_type: &str,
    quantity_str: &str,
    price_str: &str,
    date_str: &str,
    fees_str: &str,
    notes: Option<&str>,
) -> Result<()> {
    use anyhow::Context;
    use chrono::NaiveDate;
    use colored::Colorize;
    use rust_decimal::Decimal;
    use std::str::FromStr;

    info!("Adding manual transaction for {}", ticker);

    // Parse and validate inputs
    let quantity =
        Decimal::from_str(quantity_str).context("Invalid quantity. Must be a decimal number")?;

    let price = Decimal::from_str(price_str).context("Invalid price. Must be a decimal number")?;

    let fees = Decimal::from_str(fees_str).context("Invalid fees. Must be a decimal number")?;

    let trade_date = NaiveDate::parse_from_str(date_str, "%Y-%m-%d")
        .context("Invalid date format. Use YYYY-MM-DD")?;

    // Parse transaction type
    let tx_type = match transaction_type.to_uppercase().as_str() {
        "BUY" => db::TransactionType::Buy,
        "SELL" => db::TransactionType::Sell,
        _ => return Err(anyhow::anyhow!("Transaction type must be 'buy' or 'sell'")),
    };

    // Validate inputs
    if quantity <= Decimal::ZERO {
        return Err(anyhow::anyhow!("Quantity must be greater than zero"));
    }

    if price <= Decimal::ZERO {
        return Err(anyhow::anyhow!("Price must be greater than zero"));
    }

    if fees < Decimal::ZERO {
        return Err(anyhow::anyhow!("Fees cannot be negative"));
    }

    // Calculate total cost
    let total_cost = (quantity * price) + fees;

    // Initialize database
    db::init_database(None)?;
    let conn = db::open_db(None)?;

    // Detect asset type from ticker
    let asset_type = db::AssetType::Unknown;

    // Upsert asset
    let asset_id = db::upsert_asset(&conn, ticker, &asset_type, None)?;

    // Create transaction
    let transaction = db::Transaction {
        id: None,
        asset_id,
        transaction_type: tx_type.clone(),
        trade_date,
        settlement_date: Some(trade_date), // Same as trade date for manual entries
        quantity,
        price_per_unit: price,
        total_cost,
        fees,
        is_day_trade: false,
        quota_issuance_date: None,
        notes: notes.map(|s| s.to_string()),
        source: "MANUAL".to_string(),
        created_at: chrono::Utc::now(),
    };

    // Insert transaction
    let tx_id = db::insert_transaction(&conn, &transaction)?;

    // Display confirmation
    println!("\n{} Transaction added successfully!", "✓".green().bold());
    println!("  Transaction ID: {}", tx_id);
    println!("  Ticker:         {}", ticker.cyan().bold());
    println!("  Type:           {}", tx_type.as_str().to_uppercase());
    println!("  Date:           {}", trade_date.format("%Y-%m-%d"));
    println!("  Quantity:       {}", quantity);
    println!("  Price:          {}", format_currency(price).cyan());
    println!("  Fees:           {}", format_currency(fees).cyan());
    println!(
        "  Total:          {}",
        format_currency(total_cost).cyan().bold()
    );
    if let Some(n) = notes {
        println!("  Notes:          {}", n);
    }

    println!();

    Ok(())
}

/// Handle term contract processing command
async fn handle_process_terms() -> Result<()> {
    use colored::Colorize;

    println!(
        "{} Processing term contract liquidations...\n",
        "🔄".cyan().bold()
    );

    // Initialize database
    db::init_database(None)?;
    let conn = db::open_db(None)?;

    // Process term liquidations
    let processed = term_contracts::process_term_liquidations(&conn)?;

    if processed == 0 {
        println!("{} No term contract liquidations found", "ℹ".blue().bold());
        println!("\nTerm contracts are identified by transactions with notes containing");
        println!("'Term contract liquidation' and show the TICKERT → TICKER transition.");
    } else {
        println!(
            "\n{} Successfully processed {} term contract liquidation(s)!",
            "✓".green().bold(),
            processed
        );
        println!("\nCost basis from TICKERT purchases has been matched to TICKER liquidations.");
    }

    Ok(())
}

/// Handle inspect command - show Excel file structure
async fn handle_inspect(file_path: &str, full: bool, column: Option<usize>) -> Result<()> {
    use anyhow::Context;
    use calamine::{open_workbook, Data, Reader, Xlsx};
    use colored::Colorize;
    use std::collections::HashMap;

    println!(
        "{} Inspecting file: {}\n",
        "📊".cyan().bold(),
        file_path.green()
    );

    let mut workbook: Xlsx<_> = open_workbook(file_path).context("Failed to open Excel file")?;

    let sheet_names = workbook.sheet_names().to_vec();
    println!(
        "{} Found {} sheet(s):",
        "📄".cyan().bold(),
        sheet_names.len()
    );
    for name in &sheet_names {
        println!("  • {}", name.yellow());
    }
    println!();

    // Inspect each sheet
    for sheet_name in sheet_names {
        println!("{}", "=".repeat(80).dimmed());
        println!(
            "{} Sheet: {}",
            "📋".cyan().bold(),
            sheet_name.yellow().bold()
        );
        println!("{}", "=".repeat(80).dimmed());

        match workbook.worksheet_range(&sheet_name) {
            Ok(range) => {
                let rows: Vec<&[Data]> = range.rows().collect();

                if rows.is_empty() {
                    println!("  {}", "Empty sheet".dimmed());
                    continue;
                }

                println!(
                    "  {} rows, {} columns\n",
                    rows.len(),
                    rows.first().map(|r: &&[Data]| r.len()).unwrap_or(0)
                );

                // Show first row (usually headers)
                if let Some(header) = rows.first() {
                    println!("{} Header row:", "📌".cyan().bold());
                    for (i, cell) in header.iter().enumerate() {
                        let cell_str: String = cell.to_string();
                        if !cell_str.trim().is_empty() {
                            println!("  [{}] {}", i, cell_str.green());
                        }
                    }
                    println!();
                }

                // Show a few data rows if requested
                if full {
                    let data_rows = rows.iter().skip(1).take(10);
                    println!("{} Sample data rows:", "📝".cyan().bold());

                    for (row_idx, row) in data_rows.enumerate() {
                        println!("  Row {}:", row_idx + 2);
                        for (col_idx, cell) in row.iter().enumerate() {
                            let cell_str: String = cell.to_string();
                            if !cell_str.trim().is_empty() {
                                println!("    [{}] {}", col_idx, cell_str);
                            }
                        }
                        println!();
                    }
                } else {
                    // Just show how many data rows
                    if rows.len() > 1 {
                        println!(
                            "  {} data rows (use --full to see sample data)\n",
                            (rows.len() - 1).to_string().yellow()
                        );
                    }
                }

                // Analyze column unique values if requested
                if let Some(col_idx) = column {
                    println!("{} Analyzing column [{}]:", "🔍".cyan().bold(), col_idx);

                    let mut value_counts: HashMap<String, usize> = HashMap::new();

                    for row in rows.iter().skip(1) {
                        // Skip header
                        if let Some(cell) = row.get(col_idx) {
                            let cell_str: String = cell.to_string();
                            if !cell_str.trim().is_empty() && cell_str != "-" {
                                *value_counts.entry(cell_str).or_insert(0) += 1;
                            }
                        }
                    }

                    // Sort by count descending
                    let mut sorted_values: Vec<_> = value_counts.into_iter().collect();
                    sorted_values.sort_by(|a, b| b.1.cmp(&a.1));

                    println!(
                        "  Found {} unique values:\n",
                        sorted_values.len().to_string().yellow()
                    );

                    for (value, count) in sorted_values {
                        println!(
                            "    {} → {} occurrences",
                            value.green(),
                            count.to_string().dimmed()
                        );
                    }
                    println!();
                }
            }
            Err(e) => {
                println!("  {} Failed to read sheet: {}", "❌".red(), e);
            }
        }

        println!();
    }

    Ok(())
}
