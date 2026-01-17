use anyhow::{Context, Result};
use colored::Colorize;
use std::io::{stdin, stdout, Write};
use tabled::{Table, Tabled};

use crate::commands::AssetsAction;
use crate::{db, reports, scraping};

pub async fn dispatch_assets(action: AssetsAction, json_output: bool) -> Result<()> {
    match action {
        AssetsAction::List { asset_type } => list_assets(asset_type.as_deref(), json_output),
        AssetsAction::Show { ticker } => show_asset(&ticker, json_output),
        AssetsAction::Add {
            ticker,
            asset_type,
            name,
        } => add_asset(&ticker, asset_type.as_deref(), name.as_deref(), json_output),
        AssetsAction::SetType { ticker, asset_type } => {
            set_asset_type(&ticker, &asset_type, json_output)
        }
        AssetsAction::SetName { ticker, name } => set_asset_name(&ticker, &name, json_output),
        AssetsAction::Rename {
            old_ticker,
            new_ticker,
        } => rename_asset(&old_ticker, &new_ticker, json_output),
        AssetsAction::Remove { ticker } => remove_asset(&ticker, json_output),
        AssetsAction::SyncMaisRetorno {
            asset_type,
            dry_run,
        } => sync_maisretorno(asset_type.as_deref(), dry_run, json_output).await,
    }
}

fn open_conn() -> Result<rusqlite::Connection> {
    db::init_database(None)?;
    db::open_db(None)
}

fn list_assets(asset_type: Option<&str>, json_output: bool) -> Result<()> {
    let conn = open_conn()?;
    let assets = if let Some(type_str) = asset_type {
        let parsed = parse_asset_type(type_str)?;
        db::list_assets_by_type(&conn, parsed)?
    } else {
        db::get_all_assets(&conn)?
    };

    if json_output {
        println!("{}", serde_json::to_string_pretty(&assets)?);
        return Ok(());
    }

    if assets.is_empty() {
        println!("{} No assets found.", "ℹ".blue().bold());
        return Ok(());
    }

    #[derive(Tabled)]
    struct AssetRow {
        #[tabled(rename = "Ticker")]
        ticker: String,
        #[tabled(rename = "Type")]
        asset_type: String,
        #[tabled(rename = "Name")]
        name: String,
    }

    let rows: Vec<_> = assets
        .into_iter()
        .map(|asset| AssetRow {
            ticker: asset.ticker,
            asset_type: asset.asset_type.as_str().to_string(),
            name: asset.name.unwrap_or_else(|| "-".to_string()),
        })
        .collect();

    let table = Table::new(rows).to_string();
    println!("{}", table);
    Ok(())
}

fn show_asset(ticker: &str, json_output: bool) -> Result<()> {
    let conn = open_conn()?;
    let asset = db::get_asset_by_ticker(&conn, ticker)?.context("Ticker not found in assets")?;
    let tx_count = db::count_transactions_for_asset(&conn, &asset.ticker)?;

    if json_output {
        let payload = serde_json::json!({
            "ticker": asset.ticker,
            "asset_type": asset.asset_type.as_str(),
            "name": asset.name,
            "cnpj": asset.cnpj,
            "created_at": asset.created_at.to_rfc3339(),
            "updated_at": asset.updated_at.to_rfc3339(),
            "transactions": tx_count,
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }

    println!("Asset: {}", asset.ticker);
    println!("  Type: {}", asset.asset_type.as_str());
    println!("  Name: {}", asset.name.unwrap_or_else(|| "-".to_string()));
    println!("  CNPJ: {}", asset.cnpj.unwrap_or_else(|| "-".to_string()));
    println!("  Created: {}", asset.created_at.to_rfc3339());
    println!("  Updated: {}", asset.updated_at.to_rfc3339());
    println!("  Transactions: {}", tx_count);
    Ok(())
}

fn add_asset(
    ticker: &str,
    asset_type: Option<&str>,
    name: Option<&str>,
    json_output: bool,
) -> Result<()> {
    let conn = open_conn()?;
    if db::asset_exists(&conn, ticker)? {
        anyhow::bail!("Ticker {} already exists in assets", ticker);
    }

    let asset_type = asset_type.map(parse_asset_type).transpose()?;
    let asset_id = if let Some(asset_type) = asset_type {
        db::insert_asset(&conn, ticker, &asset_type, name)?
    } else {
        db::upsert_asset(&conn, ticker, &db::AssetType::Unknown, name)?
    };
    let asset = db::get_asset_by_ticker(&conn, ticker)?.context("Asset not found after insert")?;

    if json_output {
        let payload = serde_json::json!({
            "id": asset_id,
            "ticker": asset.ticker,
            "asset_type": asset.asset_type.as_str(),
            "name": asset.name,
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }

    println!("\n{} Asset added successfully!", "✓".green().bold());
    println!("  ID:     {}", asset_id);
    println!("  Ticker: {}", asset.ticker.cyan().bold());
    println!("  Type:   {}", asset.asset_type.as_str());
    if let Some(name) = asset.name {
        println!("  Name:   {}", name);
    }
    println!();
    Ok(())
}

fn set_asset_type(ticker: &str, asset_type: &str, json_output: bool) -> Result<()> {
    let conn = open_conn()?;
    let parsed = parse_asset_type(asset_type)?;
    db::update_asset_type(&conn, ticker, &parsed)?;

    if json_output {
        let payload = serde_json::json!({
            "ticker": ticker.to_uppercase(),
            "asset_type": parsed.as_str(),
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }

    println!("Updated {} to {}", ticker, parsed.as_str());
    Ok(())
}

fn set_asset_name(ticker: &str, name: &str, json_output: bool) -> Result<()> {
    let conn = open_conn()?;
    db::update_asset_name(&conn, ticker, name)?;

    if json_output {
        let payload = serde_json::json!({
            "ticker": ticker.to_uppercase(),
            "name": name,
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }

    println!("Updated {} name to {}", ticker, name);
    Ok(())
}

fn rename_asset(old_ticker: &str, new_ticker: &str, json_output: bool) -> Result<()> {
    println!(
        "Are you sure you want to rename {} to {}?",
        old_ticker, new_ticker
    );
    println!("This is a rare, correction-only change. Type 'yes' to confirm:");
    if !prompt_exact(&["yes"])? {
        println!("Aborted.");
        return Ok(());
    }

    let conn = open_conn()?;
    db::update_asset_ticker(&conn, old_ticker, new_ticker)?;

    if json_output {
        let payload = serde_json::json!({
            "old_ticker": old_ticker.to_uppercase(),
            "new_ticker": new_ticker.to_uppercase(),
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }

    println!(
        "{} Renamed {} to {}",
        "✓".green().bold(),
        old_ticker,
        new_ticker
    );
    Ok(())
}

fn remove_asset(ticker: &str, json_output: bool) -> Result<()> {
    let conn = open_conn()?;
    let asset = db::get_asset_by_ticker(&conn, ticker)?.context("Ticker not found in assets")?;
    let tx_count = db::count_transactions_for_asset(&conn, &asset.ticker)?;

    println!(
        "WARNING: This will permanently delete asset {} and ALL {} related transactions.",
        asset.ticker, tx_count
    );
    println!("Type 'yes' or 'DELETE' to confirm:");
    if !prompt_exact(&["yes", "DELETE"])? {
        println!("Aborted.");
        return Ok(());
    }

    let earliest_trade_date = db::get_earliest_transaction_date_for_asset(&conn, &asset.ticker)?;
    let deleted = db::delete_asset(&conn, &asset.ticker)?;
    if deleted == 0 {
        anyhow::bail!("Ticker {} not found in assets", asset.ticker);
    }
    if let Some(date) = earliest_trade_date {
        reports::invalidate_snapshots_after(&conn, date)?;
    }

    if json_output {
        let payload = serde_json::json!({
            "deleted": asset.ticker,
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }

    println!("{} Removed asset {}", "✓".green().bold(), asset.ticker);
    Ok(())
}

async fn sync_maisretorno(
    asset_type: Option<&str>,
    dry_run: bool,
    json_output: bool,
) -> Result<()> {
    let conn = open_conn()?;
    let parsed_type = asset_type.map(parse_asset_type).transpose()?;
    let sources = scraping::maisretorno::select_sources(parsed_type);
    if sources.is_empty() {
        anyhow::bail!("No Mais Retorno sources available for this asset type");
    }

    let printer = crate::ui::progress::ProgressPrinter::new(json_output);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let progress_handle = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            printer.handle_event(crate::ui::progress::ProgressEvent::from_message(&msg));
        }
    });

    let stats = scraping::maisretorno::sync_registry(&conn, &sources, dry_run, Some(tx)).await?;
    let _ = progress_handle.await;
    if !json_output {
        crate::ui::progress::clear_progress_line();
    }

    if json_output {
        let payload = serde_json::json!({
            "sources": sources.iter().map(|s| {
                serde_json::json!({
                    "asset_type": s.asset_type.as_str(),
                    "url": s.url,
                })
            }).collect::<Vec<_>>(),
            "entries": stats.total_entries,
            "registry_written": stats.registry_written,
            "assets_updated": stats.assets_updated,
            "updated_type": stats.updated_type,
            "updated_name": stats.updated_name,
            "updated_cnpj": stats.updated_cnpj,
            "dry_run": stats.dry_run,
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }

    println!(
        "{} Mais Retorno sync complete.",
        if dry_run {
            "ℹ".blue().bold()
        } else {
            "✓".green().bold()
        }
    );
    println!("  Entries fetched: {}", stats.total_entries);
    if dry_run {
        println!("  Registry writes skipped (dry run).");
    } else {
        println!("  Registry entries written: {}", stats.registry_written);
    }
    if dry_run {
        println!("  Asset updates skipped (dry run).");
    } else {
        println!("  Assets updated: {}", stats.assets_updated);
        println!("    Type updates: {}", stats.updated_type);
        println!("    Name updates: {}", stats.updated_name);
        println!("    CNPJ updates: {}", stats.updated_cnpj);
    }

    Ok(())
}

fn prompt_exact(allowed: &[&str]) -> Result<bool> {
    let mut input = String::new();
    stdout().flush()?;
    if stdin().read_line(&mut input)? == 0 {
        return Ok(false);
    }
    let trimmed = input.trim();
    Ok(allowed.contains(&trimmed))
}

fn parse_asset_type(input: &str) -> Result<db::AssetType> {
    input
        .parse::<db::AssetType>()
        .map_err(|_| anyhow::anyhow!("Unknown asset type: {}", input))
}
