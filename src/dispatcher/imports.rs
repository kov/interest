use crate::{db, formatters, options, reports};
use anyhow::Result;
use colored::Colorize;

pub async fn dispatch_import(
    file: &str,
    dry_run: bool,
    force_reimport: bool,
    options: options::OutputOptions,
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
            if !options.is_json() {
                println!(
                    "\n{} Found {} transactions\n",
                    "✓".green().bold(),
                    raw_transactions.len()
                );
            }

            if !options.is_json() {
                if let Some(table) =
                    formatters::imports::format_cei_preview_table(&raw_transactions, options)
                {
                    println!("{}", table);
                }
            }

            if dry_run {
                if !options.is_json() {
                    println!("\n{} Dry run - no changes saved", "ℹ".blue().bold());
                }
                return Ok(());
            }

            db::init_database(None)?;
            let conn = db::open_db(None)?;

            let stats = crate::dispatcher::imports_helpers::import_cei(&conn, &raw_transactions)?;

            if !options.is_json() {
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
            if !options.is_json() {
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

            if !options.is_json() {
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
            if !options.is_json() && !trades.is_empty() {
                println!("{} Sample trades:", "💰".cyan().bold());
                let cloned_trades: Vec<_> = trades.iter().map(|e| (*e).clone()).collect();
                if let Some(table) =
                    formatters::imports::format_movimentacao_preview_table(&cloned_trades, options)
                {
                    println!("{}\n", table);
                }
            }

            // Show preview of corporate actions
            if !options.is_json() && !corporate_actions.is_empty() {
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
            if !options.is_json() && !income_events.is_empty() {
                println!("{} Income events:", "💵".cyan().bold());

                for event in income_events.iter().take(5) {
                    let value = event
                        .operation_value
                        .map(|amount| crate::utils::format_currency(amount, options))
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
                if !options.is_json() {
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

                    if !options.is_json() {
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

                    if !options.is_json() {
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

            if !options.is_json() {
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

            println!(
                "{}",
                formatters::imports::format_import_stats(&stats, options)
            );

            Ok(())
        }

        ImportResult::OfertasPublicas(entries) => {
            if !options.is_json() {
                println!(
                    "\n{} Found {} ofertas públicas entries\n",
                    "✓".green().bold(),
                    entries.len()
                );
            }

            if !options.is_json() {
                if let Some(table) =
                    formatters::imports::format_ofertas_preview_table(&entries, options)
                {
                    println!("{}", table);
                }
            }

            if dry_run {
                if !options.is_json() {
                    println!("\n{} Dry run - no changes saved", "ℹ".blue().bold());
                    println!("\n{} What would be imported:", "📝".cyan().bold());
                    println!("  • {} offer allocation transactions", entries.len());
                }
                return Ok(());
            }

            db::init_database(None)?;
            let conn = db::open_db(None)?;

            if !options.is_json() {
                println!("{} Importing offer allocations...", "⏳".cyan().bold());
            }

            let stats = crate::dispatcher::imports_helpers::import_ofertas(&conn, &entries)?;

            if !options.is_json() {
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
