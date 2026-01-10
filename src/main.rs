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
mod ui;
mod utils;

use anyhow::Result;
use clap::Parser;
use cli::{
    ActionCommands, Cli, Commands, IncomeCommands, InconsistenciesCommands, PerformanceCommands,
    PortfolioCommands, PriceCommands, TaxCommands, TransactionCommands,
};
use rust_decimal::Decimal;
use serde::Serialize;
use std::io::IsTerminal;
use std::io::Write as _;
use tax::swing_trade::TaxCategory;
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
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));

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

    match command {
        Commands::Import { file, dry_run } => handle_import(&file, dry_run, cli.json).await,

        Commands::ImportIrpf {
            file,
            year,
            dry_run,
        } => handle_irpf_import(&file, year, dry_run).await,

        Commands::Portfolio { action } => match action {
            PortfolioCommands::Show { asset_type, at } => {
                let as_of_date = match at.as_ref() {
                    Some(d) => Some(
                        commands::parse_flexible_date(d).map_err(|e| anyhow::anyhow!("{}", e))?,
                    ),
                    None => None,
                };
                dispatcher::dispatch_command(
                    commands::Command::PortfolioShow {
                        filter: asset_type.clone(),
                        as_of_date,
                    },
                    cli.json,
                )
                .await
            }
        },

        Commands::Prices { action } => match action {
            PriceCommands::Update => handle_price_update().await,
            PriceCommands::History { ticker, from, to } => {
                handle_price_history(&ticker, &from, &to).await
            }
        },

        Commands::Tax { action } => match action {
            TaxCommands::Calculate { month } => handle_tax_calculate(&month).await,
            TaxCommands::Report { year, export } => handle_tax_report(year, export).await,
            TaxCommands::Summary { year } => handle_tax_summary(year).await,
        },

        Commands::Performance { action } => match action {
            PerformanceCommands::Show { period } => {
                handle_performance_show(&period, cli.json).await
            }
        },

        Commands::Income { action } => match action {
            IncomeCommands::Show { year } => handle_income_show(year, cli.json).await,
            IncomeCommands::Detail { year, asset } => {
                handle_income_detail(year, asset.as_deref(), cli.json).await
            }
            IncomeCommands::Summary { year } => handle_income_summary(year, cli.json).await,
        },

        Commands::Actions { action } => match action {
            ActionCommands::Add {
                ticker,
                action_type,
                ratio,
                date,
                notes,
            } => handle_action_add(&ticker, &action_type, &ratio, &date, notes.as_deref()).await,
            ActionCommands::Scrape {
                ticker,
                url,
                name,
                save,
            } => handle_action_scrape(&ticker, url.as_deref(), name.as_deref(), save).await,
            ActionCommands::Update => handle_actions_update().await,
            ActionCommands::List { ticker } => {
                handle_actions_list(ticker.as_deref(), cli.json).await
            }
            ActionCommands::Apply { ticker } => {
                handle_action_apply(ticker.as_deref(), cli.json).await
            }
            ActionCommands::Delete { id } => handle_action_delete(id).await,
            ActionCommands::Edit {
                id,
                action_type,
                ratio,
                date,
                notes,
            } => {
                handle_action_edit(
                    id,
                    action_type.as_deref(),
                    ratio.as_deref(),
                    date.as_deref(),
                    notes.as_deref(),
                )
                .await
            }
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

        Commands::Inspect { file, full, column } => handle_inspect(&file, full, column).await,

        Commands::Interactive => interest::ui::launch_tui().await,

        Commands::ProcessTerms => handle_process_terms().await,
    }
}

/// Handle import command with automatic format detection
async fn handle_import(file_path: &str, dry_run: bool, json_output: bool) -> Result<()> {
    use colored::Colorize;
    use rusqlite::OptionalExtension;
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

            // Display preview
            #[derive(Tabled)]
            struct TransactionPreview {
                #[tabled(rename = "Date")]
                date: String,
                #[tabled(rename = "Ticker")]
                ticker: String,
                #[tabled(rename = "Type")]
                tx_type: String,
                #[tabled(rename = "Quantity")]
                quantity: String,
                #[tabled(rename = "Price")]
                price: String,
                #[tabled(rename = "Total")]
                total: String,
            }

            let preview: Vec<TransactionPreview> = raw_transactions
                .iter()
                .take(10)
                .map(|tx| TransactionPreview {
                    date: tx.trade_date.format("%d/%m/%Y").to_string(),
                    ticker: tx.ticker.clone(),
                    tx_type: tx.transaction_type.clone(),
                    quantity: tx.quantity.to_string(),
                    price: format_currency(tx.price),
                    total: format_currency(tx.total),
                })
                .collect();

            let table = Table::new(preview)
                .with(Style::rounded())
                .with(Modify::new(Columns::new(3..)).with(Alignment::right()))
                .to_string();
            println!("{}", table);

            if raw_transactions.len() > 10 {
                println!(
                    "\n... and {} more transactions",
                    raw_transactions.len() - 10
                );
            }

            if dry_run {
                println!("\n{} Dry run - no changes saved", "ℹ".blue().bold());
                return Ok(());
            }

            // Initialize database if needed
            db::init_database(None)?;

            // Open connection
            let conn = db::open_db(None)?;

            // Import transactions
            let mut imported = 0;
            let mut skipped_old = 0;
            let mut errors = 0;
            let mut max_imported_date: Option<chrono::NaiveDate> = None;
            let mut earliest_imported_date: Option<chrono::NaiveDate> = None;

            let last_import_date = db::get_last_import_date(&conn, "CEI", "trades")?;

            let asset_exists = |ticker: &str| -> Result<bool> {
                let exists: Option<i64> = conn
                    .query_row("SELECT id FROM assets WHERE ticker = ?1", [ticker], |row| {
                        row.get(0)
                    })
                    .optional()?;
                Ok(exists.is_some())
            };

            for raw_tx in &raw_transactions {
                if let Some(last_date) = last_import_date {
                    if raw_tx.trade_date <= last_date {
                        skipped_old += 1;
                        continue;
                    }
                }

                // Detect asset type from ticker
                let (normalized_ticker, notes_override) =
                    importers::cei_excel::resolve_option_exercise_ticker(raw_tx, asset_exists)?;
                let asset_type = db::AssetType::detect_from_ticker(&normalized_ticker)
                    .unwrap_or(db::AssetType::Stock);

                // Upsert asset
                let asset_id = match db::upsert_asset(&conn, &normalized_ticker, &asset_type, None)
                {
                    Ok(id) => id,
                    Err(e) => {
                        eprintln!("Error upserting asset {}: {}", normalized_ticker, e);
                        errors += 1;
                        continue;
                    }
                };

                // Convert to Transaction model
                let mut transaction = match raw_tx.to_transaction(asset_id) {
                    Ok(tx) => tx,
                    Err(e) => {
                        eprintln!("Error converting transaction for {}: {}", raw_tx.ticker, e);
                        errors += 1;
                        continue;
                    }
                };
                if let Some(notes) = notes_override {
                    transaction.notes = Some(notes);
                }

                // Insert transaction
                match db::insert_transaction(&conn, &transaction) {
                    Ok(_) => {
                        imported += 1;
                        max_imported_date = Some(match max_imported_date {
                            Some(current) if current >= transaction.trade_date => current,
                            _ => transaction.trade_date,
                        });
                        earliest_imported_date = Some(match earliest_imported_date {
                            Some(current) if current <= transaction.trade_date => current,
                            _ => transaction.trade_date,
                        });
                    }
                    Err(e) => {
                        eprintln!("Error inserting transaction: {}", e);
                        errors += 1;
                    }
                }
            }

            if let Some(last_date) = max_imported_date {
                db::set_last_import_date(&conn, "CEI", "trades", last_date)?;
            }

            if imported > 0 {
                if let Some(date) = earliest_imported_date {
                    reports::invalidate_snapshots_after(&conn, date)?;
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

                #[derive(Tabled)]
                struct TradePreview {
                    #[tabled(rename = "Date")]
                    date: String,
                    #[tabled(rename = "Type")]
                    movement_type: String,
                    #[tabled(rename = "Ticker")]
                    ticker: String,
                    #[tabled(rename = "Qty")]
                    quantity: String,
                    #[tabled(rename = "Price")]
                    price: String,
                }

                let preview: Vec<TradePreview> = trades
                    .iter()
                    .take(5)
                    .map(|e| TradePreview {
                        date: e.date.format("%d/%m/%Y").to_string(),
                        movement_type: e.movement_type.clone(),
                        ticker: e.ticker.clone().unwrap_or_else(|| "?".to_string()),
                        quantity: e
                            .quantity
                            .map(|q| q.to_string())
                            .unwrap_or_else(|| "-".to_string()),
                        price: e
                            .unit_price
                            .map(format_currency)
                            .unwrap_or_else(|| "-".to_string()),
                    })
                    .collect();

                let table = Table::new(preview)
                    .with(Style::rounded())
                    .with(Modify::new(Columns::new(3..)).with(Alignment::right()))
                    .to_string();
                println!("{}\n", table);
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
                let asset_type = db::AssetType::detect_from_ticker(&entry.ticker)
                    .unwrap_or(db::AssetType::Stock);

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
            let asset_type =
                db::AssetType::detect_from_ticker(&position.ticker).unwrap_or(db::AssetType::Stock);

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
                    "  {} Replaced {} existing IRPF position(s) for {}",
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
                        "  {} Added opening position: {} {} @ {}",
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
                    source: "YAHOO/BRAPI".to_string(),
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

/// Handle corporate actions update
async fn handle_actions_update() -> Result<()> {
    use colored::Colorize;

    info!("Updating corporate actions");

    // Initialize database
    db::init_database(None)?;
    let conn = db::open_db(None)?;

    // Get all assets
    let assets = db::get_all_assets(&conn)?;

    if assets.is_empty() {
        println!("{} No assets found in database", "ℹ".blue().bold());
        return Ok(());
    }

    println!(
        "\n{} Fetching corporate actions for {} assets\n",
        "→".cyan().bold(),
        assets.len()
    );

    let mut total_actions = 0;
    let mut total_events = 0;

    for asset in &assets {
        print!("  {} {}... ", asset.ticker, "→".cyan());

        match pricing::brapi::fetch_quote(&asset.ticker, true).await {
            Ok((_price, actions_opt, events_opt)) => {
                let mut count = 0;

                // Store corporate actions
                if let Some(actions) = actions_opt {
                    for brapi_action in actions {
                        // Parse ratio from factor string (e.g., "1:2", "10%")
                        let (ratio_from, ratio_to) = parse_factor(&brapi_action.factor);

                        let action = db::CorporateAction {
                            id: None,
                            asset_id: asset.id.unwrap(),
                            action_type: brapi_action.action_type,
                            event_date: brapi_action.approved_date,
                            ex_date: brapi_action.ex_date,
                            ratio_from,
                            ratio_to,
                            source: "BRAPI".to_string(),
                            notes: brapi_action.remarks,
                            created_at: chrono::Utc::now(),
                        };

                        db::insert_corporate_action(&conn, &action)?;
                        count += 1;
                    }
                    total_actions += count;
                }

                // Store income events
                if let Some(events) = events_opt {
                    for brapi_event in events {
                        let event_type = brapi_event
                            .event_type
                            .parse::<db::IncomeEventType>()
                            .unwrap_or(db::IncomeEventType::Dividend);

                        let event = db::IncomeEvent {
                            id: None,
                            asset_id: asset.id.unwrap(),
                            event_date: brapi_event.payment_date,
                            ex_date: brapi_event.ex_date,
                            event_type,
                            amount_per_quota: brapi_event.amount,
                            total_amount: brapi_event.amount, // Will be calculated based on holdings
                            withholding_tax: rust_decimal::Decimal::ZERO,
                            is_quota_pre_2026: None,
                            source: "BRAPI".to_string(),
                            notes: brapi_event.remarks,
                            created_at: chrono::Utc::now(),
                        };

                        db::insert_income_event(&conn, &event)?;
                        total_events += 1;
                    }
                }

                if count > 0 {
                    println!("{} {} actions", "✓".green(), count);
                } else {
                    println!("{}", "✓".green());
                }
            }
            Err(e) => {
                println!("{} {}", "✗".red(), e);
            }
        }
    }

    println!(
        "\n{} Corporate actions update complete!",
        "✓".green().bold()
    );
    println!("  Actions: {}", total_actions.to_string().green());
    println!("  Events: {}", total_events.to_string().green());

    Ok(())
}

/// Handle listing corporate actions
async fn handle_actions_list(ticker: Option<&str>, json_output: bool) -> Result<()> {
    use colored::Colorize;
    use tabled::{
        settings::{object::Columns, Alignment, Modify, Style},
        Table, Tabled,
    };

    db::init_database(None)?;
    let conn = db::open_db(None)?;

    let results = db::list_corporate_actions(&conn, ticker)?;

    if results.is_empty() {
        if json_output {
            #[derive(Serialize)]
            struct Empty {
                actions: Vec<()>,
            }
            println!("{}", json_success(&Empty { actions: vec![] }));
        } else {
            println!("{} No corporate actions found", "ℹ".blue().bold());
            if let Some(t) = ticker {
                println!("  For ticker: {}", t);
            }
        }
        return Ok(());
    }

    if json_output {
        #[derive(Serialize)]
        struct JsonAction {
            id: i64,
            ticker: String,
            action_type: String,
            ratio: String,
            ex_date: String,
            source: String,
            notes: Option<String>,
        }

        #[derive(Serialize)]
        struct JsonActionList {
            actions: Vec<JsonAction>,
        }

        let actions = results
            .iter()
            .map(|(action, asset)| JsonAction {
                id: action.id.unwrap(),
                ticker: asset.ticker.clone(),
                action_type: action.action_type.as_str().to_string(),
                ratio: format!("{}:{}", action.ratio_from, action.ratio_to),
                ex_date: action.ex_date.format("%Y-%m-%d").to_string(),
                source: action.source.clone(),
                notes: action.notes.clone(),
            })
            .collect();

        println!("{}", json_success(&JsonActionList { actions }));
        return Ok(());
    }

    // Formatted output
    println!("\n{} Corporate Actions\n", "🏢".cyan().bold());

    #[derive(Tabled)]
    struct ActionRow {
        #[tabled(rename = "ID")]
        id: String,
        #[tabled(rename = "Ticker")]
        ticker: String,
        #[tabled(rename = "Type")]
        action_type: String,
        #[tabled(rename = "Ratio")]
        ratio: String,
        #[tabled(rename = "Ex-Date")]
        ex_date: String,
        #[tabled(rename = "Source")]
        source: String,
    }

    let rows: Vec<ActionRow> = results
        .iter()
        .map(|(action, asset)| ActionRow {
            id: action.id.unwrap().to_string(),
            ticker: asset.ticker.clone(),
            action_type: action.action_type.as_str().to_string(),
            ratio: format!("{}:{}", action.ratio_from, action.ratio_to),
            ex_date: action.ex_date.format("%d/%m/%Y").to_string(),
            source: action.source.clone(),
        })
        .collect();

    let table = Table::new(rows)
        .with(Style::rounded())
        .with(Modify::new(Columns::single(0)).with(Alignment::right()))
        .to_string();
    println!("{}", table);

    println!("\n{} {} total actions", "ℹ".blue().bold(), results.len());

    Ok(())
}

/// Handle manual corporate action add command
async fn handle_action_add(
    ticker: &str,
    action_type_str: &str,
    ratio_str: &str,
    date_str: &str,
    notes: Option<&str>,
) -> Result<()> {
    use anyhow::Context;
    use chrono::NaiveDate;
    use colored::Colorize;

    info!("Adding manual corporate action for {}", ticker);

    // Parse action type
    let action_type = match action_type_str.to_uppercase().as_str() {
        "SPLIT" => db::CorporateActionType::Split,
        "REVERSE-SPLIT" => db::CorporateActionType::ReverseSplit,
        "BONUS" => db::CorporateActionType::Bonus,
        _ => {
            return Err(anyhow::anyhow!(
                "Action type must be 'split', 'reverse-split', or 'bonus'"
            ))
        }
    };

    // Parse ratio (from:to format, e.g., "1:2" or "10:1")
    let ratio_parts: Vec<&str> = ratio_str.split(':').collect();
    if ratio_parts.len() != 2 {
        return Err(anyhow::anyhow!(
            "Ratio must be in format 'from:to' (e.g., '1:2', '10:1')"
        ));
    }

    let ratio_from: i32 = ratio_parts[0]
        .trim()
        .parse()
        .context("Invalid ratio 'from' value. Must be an integer")?;
    let ratio_to: i32 = ratio_parts[1]
        .trim()
        .parse()
        .context("Invalid ratio 'to' value. Must be an integer")?;

    if ratio_from <= 0 || ratio_to <= 0 {
        return Err(anyhow::anyhow!("Ratio values must be positive integers"));
    }

    // Parse ex-date
    let ex_date = NaiveDate::parse_from_str(date_str, "%Y-%m-%d")
        .context("Invalid date format. Use YYYY-MM-DD")?;

    // Initialize database
    db::init_database(None)?;
    let conn = db::open_db(None)?;

    // Get or create asset
    let asset_type = db::AssetType::detect_from_ticker(ticker).unwrap_or(db::AssetType::Stock);
    let asset_id = db::upsert_asset(&conn, ticker, &asset_type, None)?;

    // Create corporate action
    let action = db::CorporateAction {
        id: None,
        asset_id,
        action_type: action_type.clone(),
        event_date: ex_date, // Same as ex_date for manual entries
        ex_date,
        ratio_from,
        ratio_to,
        source: "MANUAL".to_string(),
        notes: notes.map(|s| s.to_string()),
        created_at: chrono::Utc::now(),
    };

    // Insert corporate action
    let action_id = db::insert_corporate_action(&conn, &action)?;

    // Display confirmation
    println!(
        "\n{} Corporate action added successfully!",
        "✓".green().bold()
    );
    println!("  Action ID:      {}", action_id);
    println!("  Ticker:         {}", ticker.cyan().bold());
    println!("  Type:           {}", action_type.as_str());
    println!(
        "  Ratio:          {}:{} ({})",
        ratio_from,
        ratio_to,
        match action_type {
            db::CorporateActionType::Split =>
                format!("each share becomes {}", ratio_to as f64 / ratio_from as f64),
            db::CorporateActionType::ReverseSplit =>
                format!("{} shares become 1", ratio_from as f64 / ratio_to as f64),
            db::CorporateActionType::Bonus => format!(
                "{}% bonus",
                ((ratio_to as f64 / ratio_from as f64) - 1.0) * 100.0
            ),
            db::CorporateActionType::CapitalReturn => format!(
                "{} per share",
                format_currency(Decimal::from(ratio_from) / Decimal::from(100))
            ),
        }
    );
    println!("  Ex-Date:        {}", ex_date.format("%Y-%m-%d"));
    if let Some(n) = notes {
        println!("  Notes:          {}", n);
    }
    println!(
        "\n{} Run this command to apply the action:",
        "→".blue().bold()
    );
    println!("  interest actions apply {}", ticker);
    println!();

    Ok(())
}

/// Handle scrape corporate actions from investing.com
async fn handle_action_scrape(
    ticker: &str,
    url: Option<&str>,
    name: Option<&str>,
    save: bool,
) -> Result<()> {
    use anyhow::Context;
    use colored::Colorize;

    info!(
        "Scraping corporate actions for {} from investing.com",
        ticker
    );

    // Initialize database to get asset info
    db::init_database(None)?;
    let conn = db::open_db(None)?;

    // Get or create asset
    let asset_type = db::AssetType::detect_from_ticker(ticker).unwrap_or(db::AssetType::Stock);
    let asset_id = db::upsert_asset(&conn, ticker, &asset_type, None)?;

    // Determine URL
    let scrape_url = if let Some(u) = url {
        u.to_string()
    } else {
        // Try to build URL automatically
        let company_name = if let Some(n) = name {
            // User provided name - save it to database for future use
            conn.execute(
                "UPDATE assets SET name = ?1 WHERE id = ?2",
                rusqlite::params![n, asset_id],
            )?;
            n.to_string()
        } else {
            // Try to get asset name from database
            let assets = db::get_all_assets(&conn)?;
            let asset = assets
                .iter()
                .find(|a| a.ticker.eq_ignore_ascii_case(ticker));

            let db_name = asset.and_then(|a| a.name.clone());

            if let Some(name) = db_name {
                name
            } else {
                // Fetch company name from Yahoo Finance
                println!(
                    "{} Fetching company name from Yahoo Finance...",
                    "🔍".cyan().bold()
                );

                match pricing::yahoo::fetch_company_name(ticker).await {
                    Ok(fetched_name) => {
                        println!("  Found: {}", fetched_name.green());

                        // Save to database for future use
                        if let Some(asset) = asset {
                            if let Some(asset_id) = asset.id {
                                let _ = conn.execute(
                                    "UPDATE assets SET name = ?1 WHERE id = ?2",
                                    rusqlite::params![&fetched_name, asset_id],
                                );
                            }
                        }

                        fetched_name
                    }
                    Err(e) => {
                        println!(
                            "{} Could not fetch company name from Yahoo Finance: {}",
                            "⚠".yellow().bold(),
                            e
                        );
                        println!("  Please provide either:");
                        println!("    1. URL with --url flag");
                        println!("    2. Company name with --name flag");
                        println!("\n{} Example:", "→".blue().bold());
                        println!(
                            "  interest actions scrape {} --name \"Advanced Micro Devices Inc\"",
                            ticker
                        );
                        return Err(anyhow::anyhow!("Could not determine company name"));
                    }
                }
            }
        };

        let auto_url = crate::scraping::InvestingScraper::build_splits_url(ticker, &company_name);
        println!("{} Auto-built URL from company name:", "🔗".cyan().bold());
        println!("  {}", auto_url.dimmed());
        println!("  If this URL is incorrect, provide the correct one with --url\n");
        auto_url
    };

    println!(
        "\n{} Launching headless browser to scrape: {}",
        "🌐".cyan().bold(),
        scrape_url
    );
    println!("  This may take 10-30 seconds to bypass Cloudflare...\n");

    // Create scraper and fetch data
    let scraper = crate::scraping::InvestingScraper::new()
        .context("Failed to create scraper. Ensure Chrome/Chromium is installed.")?;

    let mut actions = scraper
        .scrape_corporate_actions(&scrape_url, &scrape_url)
        .context("Failed to scrape corporate actions")?;

    if actions.is_empty() {
        println!(
            "{} No corporate actions found on the page",
            "ℹ".yellow().bold()
        );
        return Ok(());
    }

    // Set asset_id for all actions
    for action in &mut actions {
        action.asset_id = asset_id;
    }

    // Display scraped actions
    println!(
        "{} Found {} corporate action(s):\n",
        "✓".green().bold(),
        actions.len()
    );

    for action in &actions {
        println!(
            "  {} {} {}:{} on {}",
            match action.action_type {
                db::CorporateActionType::Split => "📈",
                db::CorporateActionType::ReverseSplit => "📉",
                db::CorporateActionType::Bonus => "🎁",
                db::CorporateActionType::CapitalReturn => "💰",
            },
            action.action_type.as_str().cyan(),
            action.ratio_from,
            action.ratio_to,
            action.ex_date.format("%Y-%m-%d")
        );
    }

    if save {
        println!("\n{} Saving to database...", "💾".cyan().bold());

        let mut saved_count = 0;

        for action in actions {
            db::insert_corporate_action(&conn, &action)?;
            saved_count += 1;
        }

        println!("\n{} Saved {} action(s)", "✓".green().bold(), saved_count);

        if saved_count > 0 {
            println!(
                "\n{} Run this command to apply the actions:",
                "→".blue().bold()
            );
            println!("  interest actions apply {}", ticker);
        }
    } else {
        println!(
            "\n{} Actions not saved. Use --save flag to save to database",
            "ℹ".blue().bold()
        );
    }
    println!();

    Ok(())
}

/// Handle apply corporate actions command
async fn handle_action_apply(ticker_filter: Option<&str>, json_output: bool) -> Result<()> {
    use colored::Colorize;

    info!("Applying corporate actions");

    // Initialize database
    db::init_database(None)?;
    let conn = db::open_db(None)?;

    // Get unapplied corporate actions
    let actions = if let Some(ticker) = ticker_filter {
        // Get asset ID for the ticker
        let assets = db::get_all_assets(&conn)?;
        let asset = assets
            .iter()
            .find(|a| a.ticker.eq_ignore_ascii_case(ticker))
            .ok_or_else(|| anyhow::anyhow!("Ticker {} not found in database", ticker))?;

        crate::corporate_actions::get_unapplied_actions(&conn, Some(asset.id.unwrap()))?
    } else {
        crate::corporate_actions::get_unapplied_actions(&conn, None)?
    };

    if actions.is_empty() {
        if json_output {
            #[derive(Serialize)]
            struct EmptyApply {
                applied_actions: Vec<()>,
            }
            println!(
                "{}",
                json_success(&EmptyApply {
                    applied_actions: vec![]
                })
            );
        } else {
            println!("{} No unapplied corporate actions found", "ℹ".blue().bold());
            if let Some(t) = ticker_filter {
                println!("  Filter: {}", t);
            }
        }
        return Ok(());
    }

    #[derive(Serialize)]
    struct AppliedAction {
        action_id: i64,
        ticker: String,
        action_type: String,
        ex_date: String,
        adjusted_transactions: usize,
    }

    let mut applied = Vec::new();

    if !json_output {
        println!(
            "\n{} Found {} unapplied corporate action(s)\n",
            "📋".cyan().bold(),
            actions.len()
        );
    }

    // Apply each action
    for action in actions {
        let asset = db::get_all_assets(&conn)?
            .into_iter()
            .find(|a| a.id == Some(action.asset_id))
            .ok_or_else(|| {
                anyhow::anyhow!("Asset not found for action {}", action.id.unwrap_or(0))
            })?;

        if !json_output {
            println!(
                "  {} Applying {} for {} (ex-date: {})",
                "→".blue(),
                action.action_type.as_str().cyan(),
                asset.ticker.cyan().bold(),
                action.ex_date.format("%Y-%m-%d")
            );
        }

        // Apply the action
        let adjusted_count =
            crate::corporate_actions::apply_corporate_action(&conn, &action, &asset)?;

        applied.push(AppliedAction {
            action_id: action.id.unwrap(),
            ticker: asset.ticker.clone(),
            action_type: action.action_type.as_str().to_string(),
            ex_date: action.ex_date.format("%Y-%m-%d").to_string(),
            adjusted_transactions: adjusted_count,
        });

        if !json_output {
            println!(
                "    {} Adjusted {} transaction(s)",
                "✓".green(),
                adjusted_count
            );
        }
    }

    if json_output {
        #[derive(Serialize)]
        struct ApplyResult {
            applied_actions: Vec<AppliedAction>,
        }
        println!(
            "{}",
            json_success(&ApplyResult {
                applied_actions: applied
            })
        );
    } else {
        println!(
            "\n{} All corporate actions applied successfully!",
            "✓".green().bold()
        );
        println!();
    }

    Ok(())
}

/// Handle action delete command
async fn handle_action_delete(action_id: i64) -> Result<()> {
    use anyhow::Context;
    use colored::Colorize;

    info!("Deleting corporate action {}", action_id);

    // Initialize database
    db::init_database(None)?;
    let conn = db::open_db(None)?;

    // Get the action details before deleting
    let action = conn.query_row(
        "SELECT id, asset_id, action_type, event_date, ex_date, ratio_from, ratio_to, source, notes, created_at
         FROM corporate_actions WHERE id = ?1",
        rusqlite::params![action_id],
        |row| {
            Ok(db::CorporateAction {
                id: Some(row.get(0)?),
                asset_id: row.get(1)?,
                action_type: row
                    .get::<_, String>(2)?
                    .parse::<db::CorporateActionType>()
                    .map_err(|_| rusqlite::Error::InvalidQuery)?,
                event_date: row.get(3)?,
                ex_date: row.get(4)?,
                ratio_from: row.get(5)?,
                ratio_to: row.get(6)?,
                source: row.get(7)?,
                notes: row.get(8)?,
                created_at: row.get(9)?,
            })
        },
    ).context(format!("Corporate action with ID {} not found", action_id))?;

    // Get asset details
    let asset = db::get_all_assets(&conn)?
        .into_iter()
        .find(|a| a.id == Some(action.asset_id))
        .ok_or_else(|| anyhow::anyhow!("Asset not found for action"))?;

    // Display action details
    println!("\n{} Deleting corporate action:\n", "🗑".red().bold());
    println!("  ID:       {}", action_id);
    println!("  Ticker:   {}", asset.ticker.cyan().bold());
    println!("  Type:     {}", action.action_type.as_str());
    println!("  Ratio:    {}:{}", action.ratio_from, action.ratio_to);
    println!("  Ex-date:  {}", action.ex_date.format("%Y-%m-%d"));
    println!("  Source:   {}", action.source);
    if let Some(ref notes) = action.notes {
        println!("  Notes:    {}", notes);
    }

    // Delete from database
    conn.execute(
        "DELETE FROM corporate_actions WHERE id = ?1",
        rusqlite::params![action_id],
    )?;

    println!(
        "\n{} Corporate action deleted successfully!\n",
        "✓".green().bold()
    );

    Ok(())
}

/// Handle action edit command
async fn handle_action_edit(
    action_id: i64,
    action_type: Option<&str>,
    ratio: Option<&str>,
    date: Option<&str>,
    notes: Option<&str>,
) -> Result<()> {
    use anyhow::Context;
    use chrono::NaiveDate;
    use colored::Colorize;

    info!("Editing corporate action {}", action_id);

    // Validate that at least one field is provided
    if action_type.is_none() && ratio.is_none() && date.is_none() && notes.is_none() {
        println!(
            "\n{} No changes specified. Use --action-type, --ratio, --date, or --notes",
            "ℹ".yellow().bold()
        );
        return Ok(());
    }

    // Initialize database
    db::init_database(None)?;
    let conn = db::open_db(None)?;

    // Get the action details
    let mut action = conn.query_row(
        "SELECT id, asset_id, action_type, event_date, ex_date, ratio_from, ratio_to, source, notes, created_at
         FROM corporate_actions WHERE id = ?1",
        rusqlite::params![action_id],
        |row| {
            Ok(db::CorporateAction {
                id: Some(row.get(0)?),
                asset_id: row.get(1)?,
                action_type: row
                    .get::<_, String>(2)?
                    .parse::<db::CorporateActionType>()
                    .map_err(|_| rusqlite::Error::InvalidQuery)?,
                event_date: row.get(3)?,
                ex_date: row.get(4)?,
                ratio_from: row.get(5)?,
                ratio_to: row.get(6)?,
                source: row.get(7)?,
                notes: row.get(8)?,
                created_at: row.get(9)?,
            })
        },
    ).context(format!("Corporate action with ID {} not found", action_id))?;

    // Check if action exists (removed applied check)

    // Get asset details
    let asset = db::get_all_assets(&conn)?
        .into_iter()
        .find(|a| a.id == Some(action.asset_id))
        .ok_or_else(|| anyhow::anyhow!("Asset not found for action"))?;

    println!(
        "\n{} Editing corporate action for {}\n",
        "✏".cyan().bold(),
        asset.ticker.cyan().bold()
    );

    // Apply updates
    let mut updates = Vec::new();

    if let Some(new_type) = action_type {
        let new_action_type = new_type
            .parse::<db::CorporateActionType>()
            .map_err(|_| anyhow::anyhow!("Invalid action type: {}", new_type))?;
        println!(
            "  Type:     {} → {}",
            action.action_type.as_str().dimmed(),
            new_action_type.as_str().green()
        );
        action.action_type = new_action_type;
        updates.push("action_type");
    }

    if let Some(ratio_str) = ratio {
        let parts: Vec<&str> = ratio_str.split(':').collect();
        if parts.len() != 2 {
            return Err(anyhow::anyhow!(
                "Invalid ratio format. Use 'from:to' (e.g., '1:8')"
            ));
        }
        let new_from: i32 = parts[0]
            .trim()
            .parse()
            .context("Invalid ratio 'from' value")?;
        let new_to: i32 = parts[1]
            .trim()
            .parse()
            .context("Invalid ratio 'to' value")?;

        println!(
            "  Ratio:    {}:{} → {}:{}",
            action.ratio_from.to_string().dimmed(),
            action.ratio_to.to_string().dimmed(),
            new_from.to_string().green(),
            new_to.to_string().green()
        );
        action.ratio_from = new_from;
        action.ratio_to = new_to;
        updates.push("ratio_from");
        updates.push("ratio_to");
    }

    if let Some(date_str) = date {
        let new_date = NaiveDate::parse_from_str(date_str, "%Y-%m-%d")
            .context("Invalid date format. Use YYYY-MM-DD")?;

        println!(
            "  Ex-date:  {} → {}",
            action.ex_date.format("%Y-%m-%d").to_string().dimmed(),
            new_date.format("%Y-%m-%d").to_string().green()
        );
        action.ex_date = new_date;
        action.event_date = new_date;
        updates.push("ex_date");
        updates.push("event_date");
    }

    if let Some(new_notes) = notes {
        let old_notes = action.notes.as_deref().unwrap_or("(none)");
        println!("  Notes:    {} → {}", old_notes.dimmed(), new_notes.green());
        action.notes = Some(new_notes.to_string());
        updates.push("notes");
    }

    // Update database
    conn.execute(
        "UPDATE corporate_actions
         SET action_type = ?1, ratio_from = ?2, ratio_to = ?3, ex_date = ?4, event_date = ?5, notes = ?6
         WHERE id = ?7",
        rusqlite::params![
            action.action_type.as_str(),
            action.ratio_from,
            action.ratio_to,
            action.ex_date,
            action.event_date,
            action.notes,
            action_id,
        ],
    )?;

    println!(
        "\n{} Corporate action updated successfully!\n",
        "✓".green().bold()
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

/// Handle IRPF annual report generation
async fn handle_tax_report(year: i32, export_csv: bool) -> Result<()> {
    use colored::Colorize;

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
            println!(
                "  {}: {}",
                category.display_name(),
                format_currency(*amount)
            );
        }
        println!();
    }

    // Helper function to check if a category is a stock category
    fn is_stock_category(category: &TaxCategory) -> bool {
        matches!(
            category,
            TaxCategory::StockSwingTrade | TaxCategory::StockDayTrade
        )
    }

    fn is_fii_category(category: &TaxCategory) -> bool {
        matches!(
            category,
            TaxCategory::FiiSwingTrade
                | TaxCategory::FiiDayTrade
                | TaxCategory::FiagroSwingTrade
                | TaxCategory::FiagroDayTrade
        )
    }

    use tabled::{
        settings::{object::Columns, Alignment, Modify, Style},
        Table, Tabled,
    };

    // Helper struct for table display with colored profit/loss and offset
    #[derive(Tabled, Clone)]
    struct TaxTableRow {
        #[tabled(rename = "Month")]
        month: String,
        #[tabled(rename = "Category")]
        category: String,
        #[tabled(rename = "Sales")]
        sales: String,
        #[tabled(rename = "Profit/Loss")]
        profit_loss: String,
        #[tabled(rename = "Exempt")]
        exempt: String,
        #[tabled(rename = "Offset")]
        offset: String,
        #[tabled(rename = "Tax")]
        tax: String,
    }

    // Helper to color profit/loss values
    fn color_profit_loss(value: Decimal, formatted: &str) -> String {
        if value > Decimal::ZERO {
            formatted.green().to_string()
        } else if value < Decimal::ZERO {
            formatted.red().to_string()
        } else {
            "—".to_string()
        }
    }

    // Helper to color offset values
    fn color_offset(value: Decimal, formatted: &str) -> String {
        if value > Decimal::ZERO {
            formatted.cyan().to_string()
        } else {
            "—".to_string()
        }
    }

    // STOCKS SECTION
    println!("{}", "📈 Stocks (Ações)".bold());

    let mut stock_total_sales = Decimal::ZERO;
    let mut stock_total_profit = Decimal::ZERO;
    let mut stock_total_loss = Decimal::ZERO;
    let mut stock_total_offset = Decimal::ZERO;
    let mut stock_total_tax = Decimal::ZERO;

    let mut stock_rows: Vec<TaxTableRow> = Vec::new();
    let mut last_month: Option<&str> = None;

    for summary in &report.monthly_summaries {
        let month_stock_categories: Vec<_> = summary
            .by_category
            .iter()
            .filter(|(cat, _)| is_stock_category(cat))
            .collect();

        if !month_stock_categories.is_empty() {
            // Add a separator before a new month (except first)
            if last_month.is_some() && !stock_rows.is_empty() {
                stock_rows.push(TaxTableRow {
                    month: String::new(),
                    category: String::new(),
                    sales: String::new(),
                    profit_loss: String::new(),
                    exempt: String::new(),
                    offset: String::new(),
                    tax: String::new(),
                });
            }
            last_month = Some(summary.month_name);

            let mut is_first_in_month = true;

            for (category, cat_summary) in month_stock_categories {
                let month_str = if is_first_in_month {
                    summary.month_name.to_string()
                } else {
                    String::new()
                };
                is_first_in_month = false;

                stock_total_sales += cat_summary.sales;
                stock_total_profit += if cat_summary.profit_loss > Decimal::ZERO {
                    cat_summary.profit_loss
                } else {
                    Decimal::ZERO
                };
                stock_total_loss += if cat_summary.profit_loss < Decimal::ZERO {
                    cat_summary.profit_loss.abs()
                } else {
                    Decimal::ZERO
                };
                stock_total_offset += cat_summary.loss_offset_applied;
                stock_total_tax += cat_summary.tax_due;

                let sales_str = format_currency(cat_summary.sales);
                let profit_raw = format_currency(cat_summary.profit_loss);
                let profit_str = color_profit_loss(cat_summary.profit_loss, &profit_raw);

                // Exempt: ✓ or ✗
                let exempt_str = if cat_summary.exemption_applied > Decimal::ZERO {
                    "✓".green().to_string()
                } else {
                    "✗".red().to_string()
                };

                // Offset: show only applied loss offset
                let offset_value = cat_summary.loss_offset_applied;

                let offset_raw = if offset_value == Decimal::ZERO {
                    "—".to_string()
                } else {
                    format_currency(offset_value)
                };
                let offset_str = color_offset(offset_value, &offset_raw);

                let tax_str = if cat_summary.tax_due > Decimal::ZERO {
                    format_currency(cat_summary.tax_due)
                } else {
                    "—".to_string()
                };

                stock_rows.push(TaxTableRow {
                    month: month_str,
                    category: category.display_name().to_string(),
                    sales: sales_str,
                    profit_loss: profit_str,
                    exempt: exempt_str,
                    offset: offset_str,
                    tax: tax_str,
                });
            }
        }
    }

    if !stock_rows.is_empty() {
        // Add total row
        stock_rows.push(TaxTableRow {
            month: "TOTAL".to_string(),
            category: String::new(),
            sales: format_currency(stock_total_sales),
            profit_loss: format_currency(stock_total_profit),
            exempt: String::new(),
            offset: if stock_total_offset > Decimal::ZERO {
                format_currency(stock_total_offset).cyan().to_string()
            } else {
                "—".to_string()
            },
            tax: format_currency(stock_total_tax),
        });

        let table = Table::new(&stock_rows)
            .with(Style::rounded())
            .with(Modify::new(Columns::new(2..)).with(Alignment::right()))
            .to_string();
        println!("{}\n", table);
    }

    // FII/FIAGRO SECTION
    println!("{}", "💰 FIIs and FIAGROs".bold());

    let mut fii_total_sales = Decimal::ZERO;
    let mut fii_total_profit = Decimal::ZERO;
    let mut fii_total_loss = Decimal::ZERO;
    let mut fii_total_offset = Decimal::ZERO;
    let mut fii_total_tax = Decimal::ZERO;

    let mut fii_rows: Vec<TaxTableRow> = Vec::new();
    let mut last_month_fii: Option<&str> = None;

    for summary in &report.monthly_summaries {
        let month_fii_categories: Vec<_> = summary
            .by_category
            .iter()
            .filter(|(cat, _)| is_fii_category(cat))
            .collect();

        if !month_fii_categories.is_empty() {
            // Add a separator before a new month (except first)
            if last_month_fii.is_some() && !fii_rows.is_empty() {
                fii_rows.push(TaxTableRow {
                    month: String::new(),
                    category: String::new(),
                    sales: String::new(),
                    profit_loss: String::new(),
                    exempt: String::new(),
                    offset: String::new(),
                    tax: String::new(),
                });
            }
            last_month_fii = Some(summary.month_name);

            let mut is_first_in_month = true;

            for (category, cat_summary) in month_fii_categories {
                let month_str = if is_first_in_month {
                    summary.month_name.to_string()
                } else {
                    String::new()
                };
                is_first_in_month = false;

                fii_total_sales += cat_summary.sales;
                fii_total_profit += if cat_summary.profit_loss > Decimal::ZERO {
                    cat_summary.profit_loss
                } else {
                    Decimal::ZERO
                };
                fii_total_loss += if cat_summary.profit_loss < Decimal::ZERO {
                    cat_summary.profit_loss.abs()
                } else {
                    Decimal::ZERO
                };
                fii_total_offset += cat_summary.loss_offset_applied;
                fii_total_tax += cat_summary.tax_due;

                let sales_str = format_currency(cat_summary.sales);
                let profit_raw = format_currency(cat_summary.profit_loss);
                let profit_str = color_profit_loss(cat_summary.profit_loss, &profit_raw);

                // Exempt: ✓ or ✗
                let exempt_str = if cat_summary.exemption_applied > Decimal::ZERO {
                    "✓".green().to_string()
                } else {
                    "✗".red().to_string()
                };

                // Offset: show only applied loss offset
                let offset_value = cat_summary.loss_offset_applied;

                let offset_raw = if offset_value == Decimal::ZERO {
                    "—".to_string()
                } else {
                    format_currency(offset_value)
                };
                let offset_str = color_offset(offset_value, &offset_raw);

                let tax_str = if cat_summary.tax_due > Decimal::ZERO {
                    format_currency(cat_summary.tax_due)
                } else {
                    "—".to_string()
                };

                fii_rows.push(TaxTableRow {
                    month: month_str,
                    category: category.display_name().to_string(),
                    sales: sales_str,
                    profit_loss: profit_str,
                    exempt: exempt_str,
                    offset: offset_str,
                    tax: tax_str,
                });
            }
        }
    }

    if !fii_rows.is_empty() {
        // Add total row
        fii_rows.push(TaxTableRow {
            month: "TOTAL".to_string(),
            category: String::new(),
            sales: format_currency(fii_total_sales),
            profit_loss: format_currency(fii_total_profit),
            exempt: String::new(),
            offset: if fii_total_offset > Decimal::ZERO {
                format_currency(fii_total_offset).cyan().to_string()
            } else {
                "—".to_string()
            },
            tax: format_currency(fii_total_tax),
        });

        let table = Table::new(&fii_rows)
            .with(Style::rounded())
            .with(Modify::new(Columns::new(2..)).with(Alignment::right()))
            .to_string();
        println!("{}\n", table);
    }

    // Annual totals
    println!("\n{} Annual Summary:", "📋".cyan().bold());
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
    let total_loss_offset: Decimal = report
        .monthly_summaries
        .iter()
        .map(|s| s.total_loss_offset_applied)
        .sum();
    if total_loss_offset > Decimal::ZERO {
        println!(
            "  Total Loss Offset: {}",
            format_currency(total_loss_offset).yellow()
        );
    }
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

/// Handle tax summary display
async fn handle_tax_summary(year: i32) -> Result<()> {
    use colored::Colorize;
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

struct TaxProgressPrinter {
    spinner: interest::ui::crossterm_engine::Spinner,
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
            spinner: interest::ui::crossterm_engine::Spinner::new(),
            in_place,
            in_progress: false,
            from_year: None,
            target_year: None,
            total_years: 0,
            completed_years: 0,
        }
    }

    fn render_line(&mut self, text: &str) {
        use std::io::{stdout, Write};
        if self.in_place {
            print!("\r\x1b[2K{} {}", self.spinner.tick(), text);
            let _ = stdout().flush();
        } else {
            println!("{} {}", self.spinner.tick(), text);
        }
    }

    fn finish_line(&mut self) {
        use std::io::{stdout, Write};
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
                        let _ = std::io::stdout().flush();
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

/// Handle performance show command
async fn handle_performance_show(period: &str, json_output: bool) -> Result<()> {
    use interest::dispatcher::performance;
    performance::dispatch_performance_show(period, json_output).await
}

/// Handle income show command (summary by asset)
async fn handle_income_show(year: Option<i32>, json_output: bool) -> Result<()> {
    dispatcher::dispatch_command(crate::commands::Command::IncomeShow { year }, json_output).await
}

/// Handle income detail command (detailed events)
async fn handle_income_detail(
    year: Option<i32>,
    asset: Option<&str>,
    json_output: bool,
) -> Result<()> {
    dispatcher::dispatch_command(
        crate::commands::Command::IncomeDetail {
            year,
            asset: asset.map(|s| s.to_string()),
        },
        json_output,
    )
    .await
}

async fn handle_income_summary(year: Option<i32>, json_output: bool) -> Result<()> {
    dispatcher::dispatch_command(
        crate::commands::Command::IncomeSummary { year },
        json_output,
    )
    .await
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
    let asset_type = db::AssetType::detect_from_ticker(ticker).unwrap_or(db::AssetType::Stock);

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

/// Parse factor string into ratio (from, to)
/// Examples: "1:2" -> (1, 2), "10%" -> (100, 110), "2:1" -> (2, 1)
fn parse_factor(factor: &str) -> (i32, i32) {
    if let Some((from, to)) = factor.split_once(':') {
        let from_val = from.trim().parse::<i32>().unwrap_or(1);
        let to_val = to.trim().parse::<i32>().unwrap_or(1);
        (from_val, to_val)
    } else if factor.contains('%') {
        // Percentage bonus: "10%" means 100:110
        let pct = factor.replace('%', "").trim().parse::<f64>().unwrap_or(0.0);
        let to_val = (100.0 + pct) as i32;
        (100, to_val)
    } else {
        (1, 1)
    }
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
