use anyhow::Result;
use colored::Colorize;
use std::collections::HashSet;

pub async fn dispatch_prices(
    action: crate::commands::PricesAction,
    _json_output: bool,
) -> Result<()> {
    use crate::commands::PricesAction;
    use crate::db;
    use crate::importers::b3_cotahist;

    match action {
        PricesAction::ImportB3 { year, no_cache } => {
            tracing::info!("Importing B3 COTAHIST for year {}", year);

            // Initialize database
            db::init_database(None)?;
            let mut conn = db::open_db(None)?;

            println!("📥 Importing B3 COTAHIST for year {}...", year);

            // Create progress callback
            let callback = |progress: &b3_cotahist::DownloadProgress| {
                use b3_cotahist::{DisplayMode, DownloadStage};

                let stage_msg = match progress.stage {
                    DownloadStage::Downloading => {
                        format!("📥 Downloading COTAHIST {} ZIP", progress.year)
                    }
                    DownloadStage::Decompressing => {
                        format!("📦 Decompressing COTAHIST {}", progress.year)
                    }
                    DownloadStage::Parsing => {
                        if let Some(total) = progress.total_records {
                            if progress.records_processed.is_multiple_of(50000)
                                || progress.records_processed == total
                            {
                                let pct = (progress.records_processed as f64 / total as f64 * 100.0)
                                    as usize;
                                format!(
                                    "📝 Parsing COTAHIST {} ({}/{}  {}%)",
                                    progress.year, progress.records_processed, total, pct
                                )
                            } else {
                                return;
                            }
                        } else {
                            format!(
                                "📝 Parsing COTAHIST {} ({})",
                                progress.year, progress.records_processed
                            )
                        }
                    }
                    _ => "".to_string(),
                };

                match progress.display_mode {
                    // Persist updates as newline
                    DisplayMode::Persist => println!("__PERSIST__:{}", stage_msg),
                    // Show spinner-style in-place updates
                    DisplayMode::Spinner => print!("\r{}", stage_msg),
                }
            };

            let cb_ref: &dyn Fn(&b3_cotahist::DownloadProgress) = &callback;

            // Run importer (download, parse, import)
            match (|| -> Result<()> {
                let zip_path = b3_cotahist::download_cotahist_year(year, no_cache, Some(cb_ref))?;
                let records = b3_cotahist::parse_cotahist_file(&zip_path, Some(cb_ref))?;
                let imported =
                    b3_cotahist::import_records_to_db(&mut conn, &records, Some(cb_ref), year)?;
                let assets: HashSet<String> = records.into_iter().map(|r| r.ticker).collect();
                println!(
                    "\n{} Imported COTAHIST {}: {} assets, {} prices",
                    "✓".green(),
                    year,
                    assets.len(),
                    imported
                );
                Ok(())
            })() {
                Ok(_) => {}
                Err(e) => println!("\n{} Import failed for {}: {}", "❌".red(), year, e),
            }

            Ok(())
        }
        PricesAction::ImportB3File { path } => {
            tracing::info!("Importing B3 COTAHIST from file {}", path);
            // Use provided helper to import directly into the database
            db::init_database(None)?;
            let mut conn = db::open_db(None)?;
            let imported =
                crate::importers::b3_cotahist::import_cotahist_from_file(&mut conn, path)?;
            println!(
                "{} Imported COTAHIST file: {} prices",
                "✓".green(),
                imported
            );
            Ok(())
        }
        PricesAction::ClearCache { year } => {
            tracing::info!("Clearing COTAHIST cache {:?}", year);
            crate::importers::b3_cotahist::clear_cache(year)?;
            Ok(())
        }
    }
}
