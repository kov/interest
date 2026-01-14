use anyhow::Result;
use std::io::{stdin, stdout, Write};

use crate::commands::TickersAction;
use crate::db::{self, AssetType};

const KNOWN_TYPES: &[&str] = &[
    "STOCK", "BDR", "ETF", "FII", "FIAGRO", "FI_INFRA", "FIDC", "FIP", "BOND", "GOV_BOND",
    "OPTION", "UNKNOWN",
];

pub async fn dispatch_tickers(action: TickersAction, json_output: bool) -> Result<()> {
    match action {
        TickersAction::Refresh { force } => {
            let path = crate::tickers::refresh_b3_tickers(force)?;
            if json_output {
                println!(
                    "{}",
                    serde_json::json!({
                        "refreshed": true,
                        "path": path,
                    })
                );
            } else {
                println!("Updated tickers cache: {}", path.display());
            }
            Ok(())
        }
        TickersAction::Status => {
            db::init_database(None)?;
            let conn = db::open_db(None)?;
            let cache_dir = crate::tickers::get_tickers_cache_dir()?;
            let csv_path = cache_dir.join("tickers.csv");
            let meta = crate::tickers::read_cache_meta(Some(&cache_dir))?;
            let unknown_assets = db::list_assets_by_type(&conn, AssetType::Unknown)?;

            if json_output {
                let payload = serde_json::json!({
                    "cache_path": csv_path,
                    "cache_exists": csv_path.exists(),
                    "fetched_at": meta.as_ref().map(|m| m.fetched_at.to_rfc3339()),
                    "source_url": meta.as_ref().map(|m| m.source_url.clone()),
                    "unknown_count": unknown_assets.len(),
                });
                println!("{}", serde_json::to_string_pretty(&payload)?);
                return Ok(());
            }

            println!("Cache path: {}", csv_path.display());
            if let Some(meta) = meta {
                let age = chrono::Utc::now().signed_duration_since(meta.fetched_at);
                println!(
                    "Last fetch: {} ({} hours ago)",
                    meta.fetched_at.to_rfc3339(),
                    age.num_hours()
                );
                println!("Source URL: {}", meta.source_url);
            } else {
                println!("Last fetch: not available");
            }
            println!("Unknown assets: {}", unknown_assets.len());
            Ok(())
        }
        TickersAction::ListUnknown => {
            db::init_database(None)?;
            let conn = db::open_db(None)?;
            let unknown_assets = db::list_assets_by_type(&conn, AssetType::Unknown)?;

            if json_output {
                println!("{}", serde_json::to_string_pretty(&unknown_assets)?);
                return Ok(());
            }

            if unknown_assets.is_empty() {
                println!("No unknown assets found.");
                return Ok(());
            }

            for asset in unknown_assets {
                let name = asset.name.unwrap_or_else(|| "-".to_string());
                println!("{} {}", asset.ticker, name);
            }
            Ok(())
        }
        TickersAction::Resolve { ticker, asset_type } => {
            db::init_database(None)?;
            let conn = db::open_db(None)?;

            if json_output && ticker.is_none() {
                anyhow::bail!("tickers resolve without a ticker is not supported in JSON mode");
            }

            if let Some(ticker) = ticker {
                let asset_type = asset_type.ok_or_else(|| {
                    anyhow::anyhow!("tickers resolve requires --type when a ticker is provided")
                })?;
                let parsed = parse_asset_type(&asset_type)?;
                db::update_asset_type(&conn, &ticker, &parsed)?;
                if json_output {
                    println!(
                        "{}",
                        serde_json::json!({
                            "ticker": ticker,
                            "asset_type": parsed.as_str(),
                        })
                    );
                } else {
                    println!("Updated {} to {}", ticker, parsed.as_str());
                }
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
    let mut input = String::new();
    loop {
        print!("Type for {} [{}]: ", ticker, KNOWN_TYPES.join("/"));
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
        match parse_asset_type(trimmed) {
            Ok(asset_type) => return Ok(PromptSelection::Selected(asset_type)),
            Err(_) => {
                println!("Invalid type. Use one of: {}", KNOWN_TYPES.join(", "));
            }
        }
    }
}

fn parse_asset_type(input: &str) -> Result<AssetType> {
    input
        .parse::<AssetType>()
        .map_err(|_| anyhow::anyhow!("Unknown asset type: {}", input))
}
