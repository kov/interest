use anyhow::{Context, Result};
use chrono::NaiveDate;
use rust_decimal::Decimal;
use std::str::FromStr;

use crate::{db, formatters, options, reports};

pub async fn dispatch_actions(
    action: &crate::cli::ActionCommands,
    options: options::OutputOptions,
) -> Result<()> {
    match action {
        crate::cli::ActionCommands::Rename { action } => dispatch_rename(action, options),
        crate::cli::ActionCommands::Split { action } => dispatch_split(action, options),
        crate::cli::ActionCommands::Bonus { action } => dispatch_bonus(action, options),
        crate::cli::ActionCommands::Spinoff { action } => {
            dispatch_exchange(action, options, db::AssetExchangeType::Spinoff)
        }
        crate::cli::ActionCommands::Merger { action } => {
            dispatch_exchange(action, options, db::AssetExchangeType::Merger)
        }
    }
}

fn dispatch_rename(
    action: &crate::cli::RenameCommands,
    options: options::OutputOptions,
) -> Result<()> {
    match action {
        crate::cli::RenameCommands::Add {
            from,
            to,
            date,
            notes,
        } => add_rename(from, to, date, notes.as_deref(), options),
        crate::cli::RenameCommands::List { ticker } => list_renames(ticker.as_deref(), options),
        crate::cli::RenameCommands::Remove { id } => remove_rename(*id, options),
    }
}

fn dispatch_split(
    action: &crate::cli::SplitCommands,
    options: options::OutputOptions,
) -> Result<()> {
    match action {
        crate::cli::SplitCommands::Add {
            ticker,
            quantity_adjustment,
            date,
            notes,
        } => add_split_or_bonus(
            ticker,
            quantity_adjustment,
            date,
            notes.as_deref(),
            options,
            db::CorporateActionType::Split,
        ),
        crate::cli::SplitCommands::List { ticker } => list_corporate_actions(
            ticker.as_deref(),
            options,
            &[
                db::CorporateActionType::Split,
                db::CorporateActionType::ReverseSplit,
            ],
        ),
        crate::cli::SplitCommands::Remove { id } => remove_corporate_action(
            *id,
            options,
            &[
                db::CorporateActionType::Split,
                db::CorporateActionType::ReverseSplit,
            ],
        ),
    }
}

fn dispatch_bonus(
    action: &crate::cli::BonusCommands,
    options: options::OutputOptions,
) -> Result<()> {
    match action {
        crate::cli::BonusCommands::Add {
            ticker,
            quantity_adjustment,
            date,
            notes,
        } => add_split_or_bonus(
            ticker,
            quantity_adjustment,
            date,
            notes.as_deref(),
            options,
            db::CorporateActionType::Bonus,
        ),
        crate::cli::BonusCommands::List { ticker } => list_corporate_actions(
            ticker.as_deref(),
            options,
            &[db::CorporateActionType::Bonus],
        ),
        crate::cli::BonusCommands::Remove { id } => {
            remove_corporate_action(*id, options, &[db::CorporateActionType::Bonus])
        }
    }
}

fn dispatch_exchange(
    action: &crate::cli::ExchangeCommands,
    options: options::OutputOptions,
    event_type: db::AssetExchangeType,
) -> Result<()> {
    match action {
        crate::cli::ExchangeCommands::Add {
            from,
            to,
            date,
            quantity,
            allocated_cost,
            cash,
            notes,
        } => add_exchange(
            from,
            to,
            date,
            quantity,
            allocated_cost,
            cash.as_deref(),
            notes.as_deref(),
            options,
            event_type,
        ),
        crate::cli::ExchangeCommands::List { ticker } => {
            list_exchanges(ticker.as_deref(), options, event_type)
        }
        crate::cli::ExchangeCommands::Remove { id } => remove_exchange(*id, options, event_type),
    }
}

fn open_conn() -> Result<rusqlite::Connection> {
    db::init_database(None)?;
    db::open_db(None)
}

fn add_rename(
    from: &str,
    to: &str,
    date_str: &str,
    notes: Option<&str>,
    options: options::OutputOptions,
) -> Result<()> {
    let effective_date = parse_date(date_str)?;
    let conn = open_conn()?;

    let asset_type = db::AssetType::Unknown;
    let from_id = db::upsert_asset(&conn, from, &asset_type, None)?;
    let to_id = db::upsert_asset(&conn, to, &asset_type, None)?;

    let rename = db::AssetRename {
        id: None,
        from_asset_id: from_id,
        to_asset_id: to_id,
        effective_date,
        notes: notes.map(|s| s.to_string()),
        created_at: chrono::Utc::now(),
    };

    let rename_id = db::insert_asset_rename(&conn, &rename)?;
    reports::invalidate_snapshots_after(&conn, effective_date)?;

    println!(
        "{}",
        formatters::actions::format_rename_add(
            rename_id,
            from,
            to,
            effective_date,
            notes,
            options
        )?
    );

    Ok(())
}

fn list_renames(ticker: Option<&str>, options: options::OutputOptions) -> Result<()> {
    let conn = open_conn()?;
    let rows = db::list_asset_renames_with_assets(&conn, ticker)?;

    println!(
        "{}",
        formatters::actions::format_renames_list(&rows, options)?
    );

    Ok(())
}

fn remove_rename(id: i64, options: options::OutputOptions) -> Result<()> {
    let conn = open_conn()?;
    let rename = db::get_asset_rename(&conn, id)?.context("Rename id not found")?;
    let effective_date = rename.effective_date;

    let deleted = db::delete_asset_rename(&conn, id)?;
    if deleted == 0 {
        anyhow::bail!("Rename id not found");
    }
    reports::invalidate_snapshots_after(&conn, effective_date)?;

    println!(
        "{}",
        formatters::actions::format_rename_remove(id, options)?
    );

    Ok(())
}

fn add_split_or_bonus(
    ticker: &str,
    quantity_str: &str,
    date_str: &str,
    notes: Option<&str>,
    options: options::OutputOptions,
    action_type: db::CorporateActionType,
) -> Result<()> {
    let ex_date = parse_date(date_str)?;
    let quantity_adjustment = parse_decimal(quantity_str)?;

    let final_type =
        if action_type == db::CorporateActionType::Split && quantity_adjustment < Decimal::ZERO {
            db::CorporateActionType::ReverseSplit
        } else {
            action_type
        };

    if final_type == db::CorporateActionType::ReverseSplit {
        // Keep negative adjustment for reverse split; do not flip sign.
    }

    let conn = open_conn()?;
    let asset_id = db::upsert_asset(&conn, ticker, &db::AssetType::Unknown, None)?;

    let action = db::CorporateAction {
        id: None,
        asset_id,
        action_type: final_type.clone(),
        event_date: ex_date,
        ex_date,
        quantity_adjustment,
        source: "MANUAL".to_string(),
        notes: notes.map(|s| s.to_string()),
        created_at: chrono::Utc::now(),
    };

    let action_id = db::insert_corporate_action(&conn, &action)?;
    reports::invalidate_snapshots_after(&conn, ex_date)?;

    // For bonus actions, automatically apply them (create synthetic transaction)
    // Splits use query-time adjustments and don't need this
    if final_type == db::CorporateActionType::Bonus {
        let asset = db::get_all_assets(&conn)?
            .into_iter()
            .find(|a| a.id == Some(asset_id))
            .context("Asset not found after insert")?;
        crate::corporate_actions::apply_corporate_action(&conn, &action, &asset)?;
    }

    println!(
        "{}",
        formatters::actions::format_corporate_action_add(
            action_id,
            ticker,
            &final_type,
            quantity_adjustment,
            ex_date,
            notes,
            options,
        )?
    );

    Ok(())
}

fn list_corporate_actions(
    ticker: Option<&str>,
    options: options::OutputOptions,
    types: &[db::CorporateActionType],
) -> Result<()> {
    let conn = open_conn()?;
    let results = db::list_corporate_actions(&conn, ticker)?;
    let filtered: Vec<_> = results
        .into_iter()
        .filter(|(action, _)| types.contains(&action.action_type))
        .collect();

    println!(
        "{}",
        formatters::actions::format_corporate_actions_list(&filtered, options)?
    );

    Ok(())
}

fn remove_corporate_action(
    id: i64,
    options: options::OutputOptions,
    allowed_types: &[db::CorporateActionType],
) -> Result<()> {
    let conn = open_conn()?;
    let (action, _asset) =
        db::get_corporate_action(&conn, id)?.context("Corporate action id not found")?;

    if !allowed_types.contains(&action.action_type) {
        anyhow::bail!("Corporate action id does not match requested type");
    }

    let deleted = db::delete_corporate_action(&conn, id)?;
    if deleted == 0 {
        anyhow::bail!("Corporate action id not found");
    }
    reports::invalidate_snapshots_after(&conn, action.ex_date)?;

    println!(
        "{}",
        formatters::actions::format_corporate_action_remove(id, options)?
    );

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn add_exchange(
    from: &str,
    to: &str,
    date_str: &str,
    quantity_str: &str,
    allocated_cost_str: &str,
    cash_str: Option<&str>,
    notes: Option<&str>,
    options: options::OutputOptions,
    event_type: db::AssetExchangeType,
) -> Result<()> {
    let effective_date = parse_date(date_str)?;
    let to_quantity = parse_decimal(quantity_str)?;
    let allocated_cost = parse_decimal(allocated_cost_str)?;
    let cash_amount = match cash_str {
        Some(value) => parse_decimal(value)?,
        None => Decimal::ZERO,
    };

    let conn = open_conn()?;
    let asset_type = db::AssetType::Unknown;
    let from_id = db::upsert_asset(&conn, from, &asset_type, None)?;
    let to_id = db::upsert_asset(&conn, to, &asset_type, None)?;

    let exchange = db::AssetExchange {
        id: None,
        event_type: event_type.clone(),
        from_asset_id: from_id,
        to_asset_id: to_id,
        effective_date,
        to_quantity,
        allocated_cost,
        cash_amount,
        source: "MANUAL".to_string(),
        notes: notes.map(|s| s.to_string()),
        created_at: chrono::Utc::now(),
    };

    let exchange_id = db::insert_asset_exchange(&conn, &exchange)?;
    reports::invalidate_snapshots_after(&conn, effective_date)?;

    println!(
        "{}",
        formatters::actions::format_exchange_add(
            exchange_id,
            &event_type,
            from,
            to,
            effective_date,
            to_quantity,
            allocated_cost,
            cash_amount,
            notes,
            options,
        )?
    );

    Ok(())
}

fn list_exchanges(
    ticker: Option<&str>,
    options: options::OutputOptions,
    event_type: db::AssetExchangeType,
) -> Result<()> {
    let conn = open_conn()?;
    let results = db::list_asset_exchanges_with_assets(&conn, ticker)?;
    let filtered: Vec<_> = results
        .into_iter()
        .filter(|(exchange, _, _)| exchange.event_type == event_type)
        .collect();

    println!(
        "{}",
        formatters::actions::format_exchanges_list(&filtered, options)?
    );

    Ok(())
}

fn remove_exchange(
    id: i64,
    options: options::OutputOptions,
    event_type: db::AssetExchangeType,
) -> Result<()> {
    let conn = open_conn()?;
    let exchange = db::get_asset_exchange(&conn, id)?.context("Exchange id not found")?;

    if exchange.event_type != event_type {
        anyhow::bail!("Exchange id does not match requested type");
    }

    let deleted = db::delete_asset_exchange(&conn, id)?;
    if deleted == 0 {
        anyhow::bail!("Exchange id not found");
    }
    reports::invalidate_snapshots_after(&conn, exchange.effective_date)?;

    println!(
        "{}",
        formatters::actions::format_exchange_remove(id, options)?
    );

    Ok(())
}

fn parse_date(date_str: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(date_str, "%Y-%m-%d").context("Invalid date format. Use YYYY-MM-DD")
}

fn parse_decimal(value: &str) -> Result<Decimal> {
    Decimal::from_str(value).context("Invalid decimal value")
}
