use crate::{db, reports};
use anyhow::Result;
use colored::Colorize;

pub async fn dispatch_import(
    file: &str,
    dry_run: bool,
    force_reimport: bool,
    json_output: bool,
) -> Result<()> {
    use crate::importers::{self, ImportResult};

    let path = file;
    tracing::info!("Importing from: {}", path);

    let import_result = match importers::import_file_auto(path) {
        Ok(r) => r,
        Err(e) => {
            return Err(anyhow::anyhow!("Error reading import file {}: {}", path, e));
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

            if !json_output {
                if let Some(table) =
                    crate::dispatcher::imports_helpers::preview_cei_table(&raw_transactions)
                {
                    println!("{}", table);
                }
            }

            if dry_run {
                if !json_output {
                    println!("\n{} Dry run - no changes saved", "ℹ".blue().bold());
                }
                return Ok(());
            }

            db::init_database(None)?;
            let conn = db::open_db(None)?;

            let stats = crate::dispatcher::imports_helpers::import_cei(&conn, &raw_transactions)?;

            if !json_output {
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

            // Handle force-reimport: delete existing data from same source
            if force_reimport {
                // Find earliest date across all entry types
                let earliest_trade = trades.iter().map(|e| e.date).min();
                let earliest_action = corporate_actions.iter().map(|e| e.date).min();
                let earliest_income = income_events.iter().map(|e| e.date).min();

                let earliest_date = [earliest_trade, earliest_action, earliest_income]
                    .iter()
                    .filter_map(|d| *d)
                    .min();

                if let Some(from_date) = earliest_date {
                    let source = "MOVIMENTACAO";

                    if !json_output {
                        println!(
                            "\n{} Force reimport: deleting {} data from {} onwards...",
                            "⚠".yellow().bold(),
                            source,
                            from_date.format("%Y-%m-%d").to_string().yellow()
                        );
                    }

                    let deleted_txs =
                        db::delete_transactions_from_source_after_date(&conn, source, from_date)?;
                    let deleted_actions = db::delete_corporate_actions_from_source_after_date(
                        &conn, source, from_date,
                    )?;
                    let deleted_income =
                        db::delete_income_events_from_source_after_date(&conn, source, from_date)?;

                    // Reset import state tracking for this source
                    conn.execute(
                        "DELETE FROM import_state WHERE source = ?1",
                        rusqlite::params![source],
                    )?;

                    if !json_output {
                        println!(
                            "  {} Deleted: {} transactions, {} corporate actions, {} income events",
                            "✓".green(),
                            deleted_txs.to_string().red(),
                            deleted_actions.to_string().red(),
                            deleted_income.to_string().red()
                        );
                    }
                }
            }

            if !json_output {
                println!(
                    "{} Importing trades, corporate actions, and income events...",
                    "⏳".cyan().bold()
                );
            }
            // Always track state - when force_reimport deleted metadata, get_last_import_date returns None
            // This allows importing old dates, then properly updates cutoff dates for future imports
            let stats = importers::import_movimentacao_entries(&conn, entries, true)?;
            if let Some(date) = stats.earliest {
                reports::invalidate_snapshots_after(&conn, date)?;
            }

            if json_output {
                // Use the unified ImportStats returned by the importer
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "success": true,
                        "data": stats
                    }))?
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

            if !json_output {
                if let Some(table) =
                    crate::dispatcher::imports_helpers::preview_ofertas_table(&entries)
                {
                    println!("{}", table);
                }
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

            if !json_output {
                println!("{} Importing offer allocations...", "⏳".cyan().bold());
            }

            let stats = crate::dispatcher::imports_helpers::import_ofertas(&conn, &entries)?;

            if !json_output {
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
            }

            Ok(())
        }
    }
}
