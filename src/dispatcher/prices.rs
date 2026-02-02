use anyhow::Result;
use colored::Colorize;
use std::collections::HashSet;

use crate::formatters;
use crate::options;

pub async fn dispatch_prices(
    action: &crate::cli::PriceCommands,
    options: options::OutputOptions,
) -> Result<()> {
    use crate::db;
    use crate::importers::b3_cotahist;

    match action {
        crate::cli::PriceCommands::Update => dispatch_price_update(options).await,
        crate::cli::PriceCommands::ImportB3 { year, no_cache } => {
            let year = *year;
            let no_cache = *no_cache;
            tracing::info!("Importing B3 COTAHIST for year {}", year);

            // Initialize database
            db::init_database(None)?;
            let printer = crate::ui::progress::ProgressPrinter::new(&options);
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
        crate::cli::PriceCommands::ImportB3File { path } => {
            tracing::info!("Importing B3 COTAHIST from file {}", path);
            db::init_database(None)?;
            let path = path.clone();
            let imported = tokio::task::spawn_blocking(move || -> Result<usize> {
                let mut conn = db::open_db(None)?;
                crate::importers::b3_cotahist::import_cotahist_from_file(&mut conn, &path)
            })
            .await
            .map_err(|e| anyhow::anyhow!(e.to_string()))??;

            options.writer().writeln(&format!(
                "{} Imported COTAHIST file: {} prices",
                "✓".green(),
                imported
            ))?;
            Ok(())
        }
        crate::cli::PriceCommands::ClearCache { year } => {
            tracing::info!("Clearing COTAHIST cache {:?}", year);
            crate::importers::b3_cotahist::clear_cache(*year)?;
            Ok(())
        }
        crate::cli::PriceCommands::History { ticker, from, to } => {
            dispatch_price_history(ticker, from, to, options).await
        }
    }
}

async fn dispatch_price_update(options: options::OutputOptions) -> Result<()> {
    use crate::pricing::PriceFetcher;
    use colored::Colorize;

    tracing::info!("Updating all asset prices");

    // Initialize database
    crate::db::init_database(None)?;
    let conn = crate::db::open_db(None)?;

    // Get all assets
    let assets = crate::db::get_all_assets(&conn)?;

    if assets.is_empty() {
        options.writer().writeln(&format!(
            "{} No assets found in database",
            "ℹ".blue().bold()
        ))?;
        options
            .writer()
            .writeln("Import transactions first using: interest import <file>")?;
        return Ok(());
    }

    options.writer().writeln(&format!(
        "\n{} Updating prices for {} assets\n",
        "→".cyan().bold(),
        assets.len()
    ))?;

    let fetcher = PriceFetcher::new();
    let mut updated = 0;
    let mut errors = 0;

    for asset in &assets {
        options
            .writer()
            .write(&format!("  {} {}... ", asset.ticker, "→".cyan()))?;

        match fetcher.fetch_price(&asset.ticker).await {
            Ok(price) => {
                // Store price in database
                let price_history = crate::db::PriceHistory {
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

                match crate::db::insert_price_history(&conn, &price_history) {
                    Ok(_) => {
                        options.writer().writeln(&format!(
                            "{} {}",
                            "✓".green(),
                            crate::utils::format_currency(price, &options)
                        ))?;
                        updated += 1;
                    }
                    Err(e) => {
                        options.writer().writeln(&format!("{} {}", "✗".red(), e))?;
                        errors += 1;
                    }
                }
            }
            Err(e) => {
                options.writer().writeln(&format!("{} {}", "✗".red(), e))?;
                errors += 1;
            }
        }
    }

    options
        .writer()
        .writeln(&format!("\n{} Price update complete!", "✓".green().bold()))?;
    options
        .writer()
        .writeln(&format!("  Updated: {}", updated.to_string().green()))?;
    if errors > 0 {
        options
            .writer()
            .writeln(&format!("  Errors: {}", errors.to_string().red()))?;
    }

    Ok(())
}

async fn dispatch_price_history(
    ticker: &str,
    from: &str,
    to: &str,
    options: options::OutputOptions,
) -> Result<()> {
    use anyhow::Context;
    use chrono::NaiveDate;
    use colored::Colorize;

    tracing::info!(
        "Fetching historical prices for {} from {} to {}",
        ticker,
        from,
        to
    );

    let from_date = NaiveDate::parse_from_str(from, "%Y-%m-%d")
        .context("Invalid from date. Use YYYY-MM-DD format")?;
    let to_date = NaiveDate::parse_from_str(to, "%Y-%m-%d")
        .context("Invalid to date. Use YYYY-MM-DD format")?;

    options.writer().writeln(&format!(
        "\n{} Fetching historical prices for {}",
        "→".cyan().bold(),
        ticker
    ))?;

    let prices = crate::pricing::yahoo::fetch_historical_prices(ticker, from_date, to_date).await?;

    let table = formatters::prices::format_prices_table(&prices, options.clone())?;
    options.writer().writeln(&format!("\n{}", table))?;

    if !prices.is_empty() {
        options.writer().writeln(&format!(
            "\n{} Total: {} price points",
            "✓".green().bold(),
            prices.len()
        ))?;
    }

    Ok(())
}
