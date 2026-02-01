use anyhow::{Context, Result};
use std::io::{stdin, stdout, Write};

use crate::{db, formatters, options, reports, scraping};

pub async fn dispatch_assets(
    action: &crate::cli::AssetsCommands,
    options: options::OutputOptions,
) -> Result<()> {
    match action {
        crate::cli::AssetsCommands::List { asset_type } => {
            list_assets(asset_type.as_deref(), options)
        }
        crate::cli::AssetsCommands::Show { ticker } => show_asset(ticker, options),
        crate::cli::AssetsCommands::Add {
            ticker,
            asset_type,
            name,
        } => add_asset(ticker, asset_type.as_deref(), name.as_deref(), options),
        crate::cli::AssetsCommands::SetType { ticker, asset_type } => {
            set_asset_type(ticker, asset_type, options)
        }
        crate::cli::AssetsCommands::SetName { ticker, name } => {
            set_asset_name(ticker, name, options)
        }
        crate::cli::AssetsCommands::Rename {
            old_ticker,
            new_ticker,
        } => rename_asset(old_ticker, new_ticker, options),
        crate::cli::AssetsCommands::Remove { ticker } => remove_asset(ticker, options),
        crate::cli::AssetsCommands::SyncMaisRetorno {
            asset_type,
            dry_run,
        } => sync_maisretorno(asset_type.as_deref(), *dry_run, options).await,
    }
}

fn open_conn() -> Result<rusqlite::Connection> {
    db::init_database(None)?;
    db::open_db(None)
}

fn list_assets(asset_type: Option<&str>, options: options::OutputOptions) -> Result<()> {
    let conn = open_conn()?;
    let assets = if let Some(type_str) = asset_type {
        let parsed = parse_asset_type(type_str)?;
        db::list_assets_by_type(&conn, parsed)?
    } else {
        db::get_all_assets(&conn)?
    };

    println!(
        "{}",
        formatters::assets::format_assets_list(&assets, options)
    );

    Ok(())
}

fn show_asset(ticker: &str, options: options::OutputOptions) -> Result<()> {
    let conn = open_conn()?;
    let asset = db::get_asset_by_ticker(&conn, ticker)?.context("Ticker not found in assets")?;
    let tx_count = db::count_transactions_for_asset(&conn, &asset.ticker)?;

    println!(
        "{}",
        formatters::assets::format_asset_show(&asset, tx_count, options)
    );

    Ok(())
}

fn add_asset(
    ticker: &str,
    asset_type: Option<&str>,
    name: Option<&str>,
    options: options::OutputOptions,
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

    println!(
        "{}",
        formatters::assets::format_asset_add(asset_id, &asset, options)
    );

    Ok(())
}

fn set_asset_type(ticker: &str, asset_type: &str, options: options::OutputOptions) -> Result<()> {
    let conn = open_conn()?;
    let parsed = parse_asset_type(asset_type)?;
    db::update_asset_type(&conn, ticker, &parsed)?;

    println!(
        "{}",
        formatters::assets::format_asset_set_type(ticker, &parsed, options)
    );

    Ok(())
}

fn set_asset_name(ticker: &str, name: &str, options: options::OutputOptions) -> Result<()> {
    let conn = open_conn()?;
    db::update_asset_name(&conn, ticker, name)?;

    println!(
        "{}",
        formatters::assets::format_asset_set_name(ticker, name, options)
    );

    Ok(())
}

fn rename_asset(old_ticker: &str, new_ticker: &str, options: options::OutputOptions) -> Result<()> {
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

    println!(
        "{}",
        formatters::assets::format_asset_rename(old_ticker, new_ticker, options)
    );

    Ok(())
}

fn remove_asset(ticker: &str, options: options::OutputOptions) -> Result<()> {
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

    println!(
        "{}",
        formatters::assets::format_asset_remove(&asset.ticker, options)
    );

    Ok(())
}

async fn sync_maisretorno(
    asset_type: Option<&str>,
    dry_run: bool,
    options: options::OutputOptions,
) -> Result<()> {
    let conn = open_conn()?;
    let parsed_type = asset_type.map(parse_asset_type).transpose()?;
    let sources = scraping::maisretorno::select_sources(parsed_type);
    if sources.is_empty() {
        anyhow::bail!("No Mais Retorno sources available for this asset type");
    }

    let printer = crate::ui::progress::ProgressPrinter::new(options);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<crate::ui::progress::ProgressEvent>();
    let progress_handle = tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            printer.handle_event(&event);
        }
    });

    let stats = scraping::maisretorno::sync_registry(&conn, &sources, dry_run, Some(tx)).await?;
    let _ = progress_handle.await;
    if !options.is_json() {
        crate::ui::progress::clear_progress_line();
    }

    println!(
        "{}",
        formatters::assets::format_sync_maisretorno(&sources, &stats, options)
    );

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
