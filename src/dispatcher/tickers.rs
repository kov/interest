use anyhow::Result;
use std::io::{stdin, stdout, Write};

use clap::ValueEnum;

use crate::{
    db::{self, AssetType},
    formatters, options,
};

pub async fn dispatch_tickers(
    action: &crate::cli::TickersCommands,
    options: options::OutputOptions,
) -> Result<()> {
    match action {
        crate::cli::TickersCommands::Refresh { force } => {
            let force = *force;
            let path = crate::tickers::refresh_b3_tickers(force)?;
            let path_str = path.display().to_string();
            println!(
                "{}",
                formatters::tickers::format_refresh(&path_str, options)?
            );
            Ok(())
        }
        crate::cli::TickersCommands::Status => {
            db::init_database(None)?;
            let conn = db::open_db(None)?;
            let cache_dir = crate::tickers::get_tickers_cache_dir()?;
            let csv_path = cache_dir.join("tickers.csv");
            let meta = crate::tickers::read_cache_meta(Some(&cache_dir))?;
            let unknown_assets = db::list_assets_by_type(&conn, AssetType::Unknown)?;

            let cache_exists = csv_path.exists();
            let fetched_at = meta.as_ref().map(|m| m.fetched_at.to_rfc3339());
            let source_url = meta.as_ref().map(|m| m.source_url.clone());
            let unknown_count = unknown_assets.len();

            println!(
                "{}",
                formatters::tickers::format_status(
                    &csv_path,
                    cache_exists,
                    fetched_at.as_deref(),
                    source_url.as_deref(),
                    unknown_count,
                    options,
                )?
            );
            Ok(())
        }
        crate::cli::TickersCommands::ListUnknown => {
            db::init_database(None)?;
            let conn = db::open_db(None)?;
            let unknown_assets = db::list_assets_by_type(&conn, AssetType::Unknown)?;

            println!(
                "{}",
                formatters::tickers::format_unknown_list(&unknown_assets, options)?
            );
            Ok(())
        }
        crate::cli::TickersCommands::Resolve { ticker, asset_type } => {
            db::init_database(None)?;
            let conn = db::open_db(None)?;

            if options.is_json() && ticker.is_none() {
                anyhow::bail!("tickers resolve without a ticker is not supported in JSON mode");
            }

            if let Some(ticker) = ticker {
                let at = asset_type.ok_or_else(|| {
                    anyhow::anyhow!("tickers resolve requires --type when a ticker is provided")
                })?;
                db::update_asset_type(&conn, ticker, &at)?;
                println!(
                    "{}",
                    formatters::tickers::format_resolve(ticker, at.as_str(), options)?
                );
                return Ok(());
            }

            let unknown_assets = db::list_assets_by_type(&conn, AssetType::Unknown)?;
            if unknown_assets.is_empty() {
                println!("No unknown assets to resolve.");
                return Ok(());
            }

            println!(
                "Found {} unknown asset{}. Going through them one by one.\n\
                 (Enter 's' to skip, 'q' to quit)\n",
                unknown_assets.len(),
                if unknown_assets.len() == 1 { "" } else { "s" }
            );

            let total = unknown_assets.len();
            let mut resolved = 0;

            for (idx, asset) in unknown_assets.iter().enumerate() {
                if total > 1 {
                    println!(
                        "━━━ [{}/{}] ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━",
                        idx + 1,
                        total
                    );
                }

                let selection = prompt_asset_type(&asset.ticker)?;
                match selection {
                    PromptSelection::Skip => {
                        println!("Skipped.\n");
                    }
                    PromptSelection::Quit => {
                        println!("Stopping resolution.");
                        break;
                    }
                    PromptSelection::Selected(asset_type) => {
                        db::update_asset_type(&conn, &asset.ticker, &asset_type)?;
                        println!("Updated {} to {}\n", asset.ticker, asset_type.as_str());
                        resolved += 1;
                    }
                }
            }

            if total > 1 {
                println!("Done. Resolved {}/{} unknown assets.", resolved, total);
            }
            Ok(())
        }
    }
}

enum PromptSelection {
    Skip,
    Quit,
    Selected(AssetType),
}

fn prompt_asset_type(ticker: &str) -> Result<PromptSelection> {
    let type_names: Vec<&str> = AssetType::value_variants()
        .iter()
        .map(|v| v.as_str())
        .collect();

    let mut input = String::new();
    loop {
        print!("Type for {} [{}]: ", ticker, type_names.join("/"));
        stdout().flush()?;
        input.clear();
        if stdin().read_line(&mut input)? == 0 {
            return Ok(PromptSelection::Quit);
        }
        let trimmed = input.trim();
        if trimmed.eq_ignore_ascii_case("s") || trimmed.is_empty() {
            return Ok(PromptSelection::Skip);
        }
        if trimmed.eq_ignore_ascii_case("q") {
            return Ok(PromptSelection::Quit);
        }
        match trimmed.parse::<AssetType>() {
            Ok(asset_type) => return Ok(PromptSelection::Selected(asset_type)),
            Err(_) => {
                println!("Invalid type. Use one of: {}", type_names.join(", "));
            }
        }
    }
}
