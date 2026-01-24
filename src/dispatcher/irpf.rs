use anyhow::Result;
use rust_decimal::Decimal;

pub async fn dispatch_irpf_import(file_path: &str, year: i32, dry_run: bool) -> Result<()> {
    use colored::Colorize;
    use tabled::{
        settings::{object::Columns, Alignment, Modify, Style},
        Table, Tabled,
    };
    use tracing::info;

    info!(
        "Importing IRPF positions from: {} for year {}",
        file_path, year
    );

    // Parse IRPF PDF for positions and loss carryforward
    let positions = crate::importers::irpf_pdf::parse_irpf_pdf(file_path, year)?;
    let losses =
        crate::importers::irpf_pdf::parse_irpf_pdf_losses(file_path, year).unwrap_or_else(|e| {
            info!("Could not parse loss carryforward: {}", e);
            crate::importers::irpf_pdf::IrpfLossCarryforward {
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
                total_cost: crate::utils::format_currency(pos.total_cost),
                avg_cost: crate::utils::format_currency(pos.average_cost),
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
                crate::utils::format_currency(losses.stock_swing_loss)
            );
        }
        if losses.stock_day_loss > Decimal::ZERO {
            println!(
                "  • Stock Day Trade: {}",
                crate::utils::format_currency(losses.stock_day_loss)
            );
        }
        if losses.fii_fiagro_loss > Decimal::ZERO {
            println!(
                "  • FII/FIAGRO: {}",
                crate::utils::format_currency(losses.fii_fiagro_loss)
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
                    crate::utils::format_currency(losses.stock_swing_loss)
                );
            }
            if losses.stock_day_loss > Decimal::ZERO {
                println!(
                    "    - Stock Day Trade: {}",
                    crate::utils::format_currency(losses.stock_day_loss)
                );
            }
            if losses.fii_fiagro_loss > Decimal::ZERO {
                println!(
                    "    - FII/FIAGRO: {}",
                    crate::utils::format_currency(losses.fii_fiagro_loss)
                );
            }
        }
        return Ok(());
    }

    // Initialize database
    crate::db::init_database(None)?;
    let conn = crate::db::open_db(None)?;

    // Import positions
    let mut imported = 0;
    let mut replaced = 0;

    if !positions.is_empty() {
        println!("\n{} Importing opening positions...\n", "⏳".cyan().bold());

        for position in positions {
            // Detect asset type from ticker
            let asset_type = crate::db::AssetType::Unknown;

            // Upsert asset
            let asset_id = match crate::db::upsert_asset(&conn, &position.ticker, &asset_type, None)
            {
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
            match crate::db::insert_transaction(&conn, &transaction) {
                Ok(_) => {
                    println!(
                        "{} Added opening position: {} {} @ {}",
                        "✓".green(),
                        position.quantity,
                        position.ticker.cyan(),
                        crate::utils::format_currency(position.average_cost)
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

        crate::db::set_last_import_date(&conn, "CEI", "trades", year_end)?;
        crate::db::set_last_import_date(&conn, "MOVIMENTACAO", "trades", year_end)?;
        crate::db::set_last_import_date(&conn, "MOVIMENTACAO", "corporate_actions", year_end)?;

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
        let fingerprint = match crate::tax::loss_carryforward::compute_year_fingerprint(&conn, year)
        {
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
                crate::tax::swing_trade::TaxCategory::StockSwingTrade,
                losses.stock_swing_loss,
            );
        }
        if losses.stock_day_loss > Decimal::ZERO {
            loss_carry.insert(
                crate::tax::swing_trade::TaxCategory::StockDayTrade,
                losses.stock_day_loss,
            );
        }
        if losses.fii_fiagro_loss > Decimal::ZERO {
            // FII/FIAGRO losses are combined in the PDF, so split proportionally or use FII category
            // For now, assign to FII swing trade (most common)
            loss_carry.insert(
                crate::tax::swing_trade::TaxCategory::FiiSwingTrade,
                losses.fii_fiagro_loss,
            );
        }

        match crate::tax::loss_carryforward::upsert_snapshot(&conn, year, &fingerprint, &loss_carry)
        {
            Ok(_) => {
                println!("  {} Loss carryforward snapshot imported", "✓".green());
                for (category, amount) in &loss_carry {
                    println!(
                        "    • {}: {}",
                        category.display_name(),
                        crate::utils::format_currency(*amount)
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
