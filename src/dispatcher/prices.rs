use anyhow::Result;
use colored::Colorize;
use std::collections::HashSet;

pub async fn dispatch_prices(
    action: crate::commands::PricesAction,
    json_output: bool,
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

            let printer = crate::ui::progress::ProgressPrinter::new(json_output);
            printer.handle_event(&crate::ui::progress::ProgressEvent::Spinner {
                message: format!("Importing B3 COTAHIST for year {}...", year),
            });

            // Create progress callback
            let callback = |progress: &b3_cotahist::DownloadProgress| {
                use b3_cotahist::DownloadStage;

                let event = match progress.stage {
                    DownloadStage::Downloading => crate::ui::progress::ProgressEvent::Downloading {
                        resource: format!("COTAHIST {} ZIP", progress.year),
                    },
                    DownloadStage::Decompressing => {
                        crate::ui::progress::ProgressEvent::Decompressing {
                            file: format!("COTAHIST {}", progress.year),
                        }
                    }
                    DownloadStage::Parsing => crate::ui::progress::ProgressEvent::Parsing {
                        file: format!("COTAHIST {}", progress.year),
                        progress: progress.total_records.map(|total| {
                            crate::ui::progress::ProgressData {
                                current: progress.records_processed,
                                total: Some(total),
                            }
                        }),
                    },
                    DownloadStage::Complete => crate::ui::progress::ProgressEvent::Success {
                        message: format!(
                            "Imported {} prices for {}",
                            progress.records_processed, progress.year
                        ),
                    },
                };
                printer.handle_event(&event);
            };

            let cb_ref: &dyn Fn(&b3_cotahist::DownloadProgress) = &callback;

            // Run importer (download, parse, import)
            match (|| -> Result<()> {
                let zip_path = b3_cotahist::download_cotahist_year(year, no_cache, Some(cb_ref))?;
                let records = b3_cotahist::parse_cotahist_file(&zip_path, Some(cb_ref))?;
                let imported =
                    b3_cotahist::import_records_to_db(&mut conn, &records, Some(cb_ref), year)?;
                let assets: HashSet<String> = records.into_iter().map(|r| r.ticker).collect();
                printer.handle_event(&crate::ui::progress::ProgressEvent::Success {
                    message: format!(
                        "Imported COTAHIST {}: {} assets, {} prices",
                        year,
                        assets.len(),
                        imported
                    ),
                });
                Ok(())
            })() {
                Ok(_) => {}
                Err(e) => {
                    printer.handle_event(&crate::ui::progress::ProgressEvent::Error {
                        message: format!("Import failed for {}: {}", year, e),
                    });
                }
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
