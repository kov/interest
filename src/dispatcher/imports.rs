use anyhow::Result;
use colored::Colorize;
use rusqlite::OptionalExtension;
use tabled::Tabled;
use tabled::{
    settings::{object::Columns, Alignment, Modify, Style},
    Table,
};

use crate::{db, reports};

pub async fn dispatch_import(
    action: crate::commands::ImportAction,
    json_output: bool,
) -> Result<()> {
    use crate::importers::{self, ImportResult};

    match action {
        crate::commands::ImportAction::File { path, dry_run } => {
            tracing::info!("Importing from: {}", path);

            let import_result = match importers::import_file_auto(&path) {
                Ok(r) => r,
                Err(e) => {
                    return Err(anyhow::anyhow!(
                        "Error reading import file {}: {}",
                        path,
                        e
                    ));
                }
            };

            match import_result {
                ImportResult::Cei(raw_transactions) => {
                    if !json_output {
                        println!(
                            "\n{} Found {} transactions\n",
                            "✓".green().bold(),
                            raw_transactions.len()
                        );
                    }

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
                            price: crate::utils::format_currency(tx.price),
                            total: crate::utils::format_currency(tx.total),
                        })
                        .collect();

                    if !preview.is_empty() && !json_output {
                        let table = Table::new(preview)
                            .with(Style::rounded())
                            .with(Modify::new(Columns::new(3..)).with(Alignment::right()))
                            .to_string();
                        println!("{}", table);
                    }

                    if dry_run {
                        if !json_output {
                            println!("\n{} Dry run - no changes saved", "ℹ".blue().bold());
                        }
                        return Ok(());
                    }

                    db::init_database(None)?;
                    let conn = db::open_db(None)?;

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

                        let (normalized_ticker, notes_override) =
                            importers::cei_excel::resolve_option_exercise_ticker(
                                raw_tx,
                                asset_exists,
                            )?;
                        let asset_type = db::AssetType::detect_from_ticker(&normalized_ticker)
                            .unwrap_or(db::AssetType::Stock);

                        // Upsert asset
                        let asset_id =
                            match db::upsert_asset(&conn, &normalized_ticker, &asset_type, None) {
                                Ok(id) => id,
                                Err(e) => {
                                    eprintln!("Error upserting asset {}: {}", normalized_ticker, e);
                                    errors += 1;
                                    continue;
                                }
                            };

                        let mut transaction = match raw_tx.to_transaction(asset_id) {
                            Ok(tx) => tx,
                            Err(e) => {
                                eprintln!(
                                    "Error converting transaction for {}: {}",
                                    raw_tx.ticker, e
                                );
                                errors += 1;
                                continue;
                            }
                        };
                        if let Some(notes) = notes_override {
                            transaction.notes = Some(notes);
                        }

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

                    if !json_output {
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
                    }

                    Ok(())
                }

                ImportResult::Movimentacao(entries) => {
                    if !json_output {
                        println!(
                            "\n{} Found {} movimentacao entries\n",
                            "✓".green().bold(),
                            entries.len()
                        );
                    }

                    let trades: Vec<_> = entries.iter().filter(|e| e.is_trade()).collect();
                    let mut corporate_actions: Vec<_> =
                        entries.iter().filter(|e| e.is_corporate_action()).collect();
                    corporate_actions.sort_by_key(|e| e.date);
                    let income_events: Vec<_> =
                        entries.iter().filter(|e| e.is_income_event()).collect();
                    let other: Vec<_> = entries
                        .iter()
                        .filter(|e| {
                            !e.is_trade() && !e.is_corporate_action() && !e.is_income_event()
                        })
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
                                    .map(crate::utils::format_currency)
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
                                .map(crate::utils::format_currency)
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
                        if !json_output {
                            println!("\n{} Dry run - no changes saved", "ℹ".blue().bold());
                            println!("\n{} What would be imported:", "📝".cyan().bold());
                            println!("  • {} trade transactions", trades.len());
                            println!("  • {} corporate actions", corporate_actions.len());
                            println!(
                                "  • {} income events (not yet implemented)",
                                income_events.len()
                            );
                        }
                        return Ok(());
                    }

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
                        println!(
                            "{}",
                            serde_json::to_string_pretty(
                                &serde_json::json!({"success": true, "data": stats})
                            )?
                        );
                        return Ok(());
                    }

                    if !json_output {
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
                    }

                    Ok(())
                }

                ImportResult::OfertasPublicas(entries) => {
                    if !json_output {
                        println!(
                            "\n{} Found {} ofertas públicas entries\n",
                            "✓".green().bold(),
                            entries.len()
                        );
                    }

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
                            price: crate::utils::format_currency(e.unit_price),
                            offer: e.offer.clone(),
                        })
                        .collect();

                    if !preview.is_empty() && !json_output {
                        let table = Table::new(preview)
                            .with(Style::rounded())
                            .with(Modify::new(Columns::new(2..4)).with(Alignment::right()))
                            .to_string();
                        println!("{}", table);
                    }

                    if dry_run {
                        if !json_output {
                            println!("\n{} Dry run - no changes saved", "ℹ".blue().bold());
                            println!("\n{} What would be imported:", "📝".cyan().bold());
                            println!("  • {} offer allocation transactions", entries.len());
                        }
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

                        let asset_id =
                            match db::upsert_asset(&conn, &entry.ticker, &asset_type, None) {
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
                                eprintln!("Error inserting offer allocation transaction: {}", e);
                                errors += 1;
                            }
                        }
                    }

                    if let Some(d) = max_date {
                        db::set_last_import_date(&conn, "OFERTAS_PUBLICAS", "allocations", d)?;
                    }

                    if !json_output {
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
                    }

                    Ok(())
                }
            }
        }
    }
}
