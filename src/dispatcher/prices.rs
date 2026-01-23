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
            let printer = crate::ui::progress::ProgressPrinter::new(json_output);
            printer.handle_event(&crate::ui::progress::ProgressEvent::Spinner {
                message: format!("Importing B3 COTAHIST for year {}...", year),
            });

            let (tx, mut rx) =
                tokio::sync::mpsc::unbounded_channel::<crate::ui::progress::ProgressEvent>();

            let mut handle = tokio::task::spawn_blocking(move || -> Result<(usize, usize)> {
                let mut conn = db::open_db(None)?;

                let callback = |progress: &b3_cotahist::DownloadProgress| {
                    use b3_cotahist::DownloadStage;

                    let event = match progress.stage {
                        DownloadStage::Downloading => {
                            crate::ui::progress::ProgressEvent::Downloading {
                                resource: format!("COTAHIST {} ZIP", progress.year),
                            }
                        }
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
                    let _ = tx.send(event);
                };

                let cb_ref: &dyn Fn(&b3_cotahist::DownloadProgress) = &callback;

                let zip_path = b3_cotahist::download_cotahist_year(year, no_cache, Some(cb_ref))?;
                let records = b3_cotahist::parse_cotahist_file(&zip_path, Some(cb_ref))?;
                let imported =
                    b3_cotahist::import_records_to_db(&mut conn, &records, Some(cb_ref), year)?;
                let assets: HashSet<String> = records.into_iter().map(|r| r.ticker).collect();

                Ok((assets.len(), imported))
            });

            let mut import_result: Option<Result<(usize, usize)>> = None;

            loop {
                tokio::select! {
                    Some(event) = rx.recv() => {
                        printer.handle_event(&event);
                    }
                    result = &mut handle => {
                        import_result = Some(result.map_err(|e| anyhow::anyhow!(e.to_string()))?);
                        break;
                    }
                    else => break,
                }
            }

            match import_result.transpose()? {
                Some((asset_count, imported)) => {
                    printer.handle_event(&crate::ui::progress::ProgressEvent::Success {
                        message: format!(
                            "Imported COTAHIST {}: {} assets, {} prices",
                            year, asset_count, imported
                        ),
                    });
                }
                None => {
                    printer.handle_event(&crate::ui::progress::ProgressEvent::Error {
                        message: format!("Import failed for {}: task cancelled", year),
                    });
                }
            }

            Ok(())
        }
        PricesAction::ImportB3File { path } => {
            tracing::info!("Importing B3 COTAHIST from file {}", path);
            db::init_database(None)?;
            let imported = tokio::task::spawn_blocking(move || -> Result<usize> {
                let mut conn = db::open_db(None)?;
                crate::importers::b3_cotahist::import_cotahist_from_file(&mut conn, path)
            })
            .await
            .map_err(|e| anyhow::anyhow!(e.to_string()))??;

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
