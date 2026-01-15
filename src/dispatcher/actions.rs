use anyhow::{Context, Result};
use chrono::NaiveDate;
use colored::Colorize;
use rust_decimal::Decimal;
use std::str::FromStr;
use tabled::{Table, Tabled};

use crate::commands::{ActionsAction, BonusAction, ExchangeAction, RenameAction, SplitAction};
use crate::{db, reports};

pub async fn dispatch_actions(action: ActionsAction, json_output: bool) -> Result<()> {
    match action {
        ActionsAction::Rename { action } => dispatch_rename(action, json_output),
        ActionsAction::Split { action } => dispatch_split(action, json_output),
        ActionsAction::Bonus { action } => dispatch_bonus(action, json_output),
        ActionsAction::Spinoff { action } => {
            dispatch_exchange(action, json_output, db::AssetExchangeType::Spinoff)
        }
        ActionsAction::Merger { action } => {
            dispatch_exchange(action, json_output, db::AssetExchangeType::Merger)
        }
        ActionsAction::Apply { ticker } => dispatch_apply(ticker.as_deref(), json_output).await,
    }
}

fn dispatch_rename(action: RenameAction, json_output: bool) -> Result<()> {
    match action {
        RenameAction::Add {
            from,
            to,
            date,
            notes,
        } => add_rename(&from, &to, &date, notes.as_deref(), json_output),
        RenameAction::List { ticker } => list_renames(ticker.as_deref(), json_output),
        RenameAction::Remove { id } => remove_rename(id, json_output),
    }
}

fn dispatch_split(action: SplitAction, json_output: bool) -> Result<()> {
    match action {
        SplitAction::Add {
            ticker,
            quantity_adjustment,
            date,
            notes,
        } => add_split_or_bonus(
            &ticker,
            &quantity_adjustment,
            &date,
            notes.as_deref(),
            json_output,
            db::CorporateActionType::Split,
        ),
        SplitAction::List { ticker } => list_corporate_actions(
            ticker.as_deref(),
            json_output,
            &[
                db::CorporateActionType::Split,
                db::CorporateActionType::ReverseSplit,
            ],
        ),
        SplitAction::Remove { id } => remove_corporate_action(
            id,
            json_output,
            &[
                db::CorporateActionType::Split,
                db::CorporateActionType::ReverseSplit,
            ],
        ),
    }
}

fn dispatch_bonus(action: BonusAction, json_output: bool) -> Result<()> {
    match action {
        BonusAction::Add {
            ticker,
            quantity_adjustment,
            date,
            notes,
        } => add_split_or_bonus(
            &ticker,
            &quantity_adjustment,
            &date,
            notes.as_deref(),
            json_output,
            db::CorporateActionType::Bonus,
        ),
        BonusAction::List { ticker } => list_corporate_actions(
            ticker.as_deref(),
            json_output,
            &[db::CorporateActionType::Bonus],
        ),
        BonusAction::Remove { id } => {
            remove_corporate_action(id, json_output, &[db::CorporateActionType::Bonus])
        }
    }
}

fn dispatch_exchange(
    action: ExchangeAction,
    json_output: bool,
    event_type: db::AssetExchangeType,
) -> Result<()> {
    match action {
        ExchangeAction::Add {
            from,
            to,
            date,
            quantity,
            allocated_cost,
            cash,
            notes,
        } => add_exchange(
            &from,
            &to,
            &date,
            &quantity,
            &allocated_cost,
            cash.as_deref(),
            notes.as_deref(),
            json_output,
            event_type,
        ),
        ExchangeAction::List { ticker } => {
            list_exchanges(ticker.as_deref(), json_output, event_type)
        }
        ExchangeAction::Remove { id } => remove_exchange(id, json_output, event_type),
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
    json_output: bool,
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

    if json_output {
        let payload = serde_json::json!({
            "id": rename_id,
            "from": from,
            "to": to,
            "effective_date": effective_date.to_string(),
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }

    println!("\n{} Asset rename added successfully!", "✓".green().bold());
    println!("  Rename ID:      {}", rename_id);
    println!("  From:           {}", from.cyan().bold());
    println!("  To:             {}", to.cyan().bold());
    println!("  Effective Date: {}", effective_date.format("%Y-%m-%d"));
    if let Some(n) = notes {
        println!("  Notes:          {}", n);
    }
    println!();

    Ok(())
}

fn list_renames(ticker: Option<&str>, json_output: bool) -> Result<()> {
    let conn = open_conn()?;
    let rows = db::list_asset_renames_with_assets(&conn, ticker)?;

    if json_output {
        let payload: Vec<_> = rows
            .iter()
            .map(|(rename, from, to)| {
                serde_json::json!({
                    "id": rename.id,
                    "from": from.ticker,
                    "to": to.ticker,
                    "effective_date": rename.effective_date.to_string(),
                    "notes": rename.notes,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }

    if rows.is_empty() {
        println!("{} No renames found", "ℹ".blue().bold());
        return Ok(());
    }

    #[derive(Tabled)]
    struct RenameRow {
        #[tabled(rename = "ID")]
        id: String,
        #[tabled(rename = "From")]
        from: String,
        #[tabled(rename = "To")]
        to: String,
        #[tabled(rename = "Date")]
        date: String,
    }

    let table_rows: Vec<_> = rows
        .into_iter()
        .map(|(rename, from, to)| RenameRow {
            id: rename.id.unwrap_or(0).to_string(),
            from: from.ticker,
            to: to.ticker,
            date: rename.effective_date.format("%Y-%m-%d").to_string(),
        })
        .collect();

    let table = Table::new(table_rows).to_string();
    println!("{}", table);

    Ok(())
}

fn remove_rename(id: i64, json_output: bool) -> Result<()> {
    let conn = open_conn()?;
    let rename = db::get_asset_rename(&conn, id)?.context("Rename id not found")?;
    let effective_date = rename.effective_date;

    let deleted = db::delete_asset_rename(&conn, id)?;
    if deleted == 0 {
        anyhow::bail!("Rename id not found");
    }
    reports::invalidate_snapshots_after(&conn, effective_date)?;

    if json_output {
        let payload = serde_json::json!({ "deleted": id });
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }

    println!("{} Removed rename {}", "✓".green().bold(), id);
    Ok(())
}

fn add_split_or_bonus(
    ticker: &str,
    quantity_str: &str,
    date_str: &str,
    notes: Option<&str>,
    json_output: bool,
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

    if json_output {
        let payload = serde_json::json!({
            "id": action_id,
            "ticker": ticker,
            "type": final_type.as_str(),
            "quantity_adjustment": quantity_adjustment.to_string(),
            "ex_date": ex_date.to_string(),
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }

    println!(
        "\n{} Corporate action added successfully!",
        "✓".green().bold()
    );
    println!("  Action ID:      {}", action_id);
    println!("  Ticker:         {}", ticker.cyan().bold());
    println!("  Type:           {}", final_type.as_str());
    println!("  Adjustment:     {} shares", quantity_adjustment);
    println!("  Ex-Date:        {}", ex_date.format("%Y-%m-%d"));
    if let Some(n) = notes {
        println!("  Notes:          {}", n);
    }
    println!();

    Ok(())
}

fn list_corporate_actions(
    ticker: Option<&str>,
    json_output: bool,
    types: &[db::CorporateActionType],
) -> Result<()> {
    let conn = open_conn()?;
    let results = db::list_corporate_actions(&conn, ticker)?;
    let filtered: Vec<_> = results
        .into_iter()
        .filter(|(action, _)| types.contains(&action.action_type))
        .collect();

    if json_output {
        let payload: Vec<_> = filtered
            .iter()
            .map(|(action, asset)| {
                serde_json::json!({
                    "id": action.id,
                    "ticker": asset.ticker,
                    "type": action.action_type.as_str(),
                    "quantity_adjustment": action.quantity_adjustment.to_string(),
                    "ex_date": action.ex_date.to_string(),
                    "source": action.source,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }

    if filtered.is_empty() {
        println!("{} No corporate actions found", "ℹ".blue().bold());
        return Ok(());
    }

    #[derive(Tabled)]
    struct ActionRow {
        #[tabled(rename = "ID")]
        id: String,
        #[tabled(rename = "Ticker")]
        ticker: String,
        #[tabled(rename = "Type")]
        action_type: String,
        #[tabled(rename = "Adj Qty")]
        quantity_adjustment: String,
        #[tabled(rename = "Ex-Date")]
        ex_date: String,
        #[tabled(rename = "Source")]
        source: String,
    }

    let rows: Vec<_> = filtered
        .into_iter()
        .map(|(action, asset)| ActionRow {
            id: action.id.unwrap_or(0).to_string(),
            ticker: asset.ticker,
            action_type: action.action_type.as_str().to_string(),
            quantity_adjustment: action.quantity_adjustment.to_string(),
            ex_date: action.ex_date.format("%Y-%m-%d").to_string(),
            source: action.source,
        })
        .collect();

    let table = Table::new(rows).to_string();
    println!("{}", table);

    Ok(())
}

fn remove_corporate_action(
    id: i64,
    json_output: bool,
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

    if json_output {
        let payload = serde_json::json!({ "deleted": id });
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }

    println!("{} Removed corporate action {}", "✓".green().bold(), id);
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
    json_output: bool,
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

    if json_output {
        let payload = serde_json::json!({
            "id": exchange_id,
            "type": event_type.as_str(),
            "from": from,
            "to": to,
            "effective_date": effective_date.to_string(),
            "quantity": to_quantity.to_string(),
            "allocated_cost": allocated_cost.to_string(),
            "cash_amount": cash_amount.to_string(),
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }

    let label = if event_type == db::AssetExchangeType::Spinoff {
        "Spin-off"
    } else {
        "Merger"
    };
    println!("\n{} {} added successfully!", "✓".green().bold(), label);
    println!("  Exchange ID:    {}", exchange_id);
    println!("  From:           {}", from.cyan().bold());
    println!("  To:             {}", to.cyan().bold());
    println!("  Effective Date: {}", effective_date.format("%Y-%m-%d"));
    println!("  Quantity:       {}", to_quantity);
    println!("  Allocated Cost: {}", allocated_cost);
    if cash_amount > Decimal::ZERO {
        println!("  Cash Amount:    {}", cash_amount);
    }
    if let Some(n) = notes {
        println!("  Notes:          {}", n);
    }
    println!();

    Ok(())
}

fn list_exchanges(
    ticker: Option<&str>,
    json_output: bool,
    event_type: db::AssetExchangeType,
) -> Result<()> {
    let conn = open_conn()?;
    let results = db::list_asset_exchanges_with_assets(&conn, ticker)?;
    let filtered: Vec<_> = results
        .into_iter()
        .filter(|(exchange, _, _)| exchange.event_type == event_type)
        .collect();

    if json_output {
        let payload: Vec<_> = filtered
            .iter()
            .map(|(exchange, from, to)| {
                serde_json::json!({
                    "id": exchange.id,
                    "type": exchange.event_type.as_str(),
                    "from": from.ticker,
                    "to": to.ticker,
                    "effective_date": exchange.effective_date.to_string(),
                    "quantity": exchange.to_quantity.to_string(),
                    "allocated_cost": exchange.allocated_cost.to_string(),
                    "cash_amount": exchange.cash_amount.to_string(),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }

    if filtered.is_empty() {
        println!("{} No exchanges found", "ℹ".blue().bold());
        return Ok(());
    }

    #[derive(Tabled)]
    struct ExchangeRow {
        #[tabled(rename = "ID")]
        id: String,
        #[tabled(rename = "From")]
        from: String,
        #[tabled(rename = "To")]
        to: String,
        #[tabled(rename = "Date")]
        date: String,
        #[tabled(rename = "Qty")]
        quantity: String,
        #[tabled(rename = "Alloc Cost")]
        allocated_cost: String,
        #[tabled(rename = "Cash")]
        cash: String,
    }

    let rows: Vec<_> = filtered
        .into_iter()
        .map(|(exchange, from, to)| ExchangeRow {
            id: exchange.id.unwrap_or(0).to_string(),
            from: from.ticker,
            to: to.ticker,
            date: exchange.effective_date.format("%Y-%m-%d").to_string(),
            quantity: exchange.to_quantity.to_string(),
            allocated_cost: exchange.allocated_cost.to_string(),
            cash: exchange.cash_amount.to_string(),
        })
        .collect();

    let table = Table::new(rows).to_string();
    println!("{}", table);

    Ok(())
}

fn remove_exchange(id: i64, json_output: bool, event_type: db::AssetExchangeType) -> Result<()> {
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

    if json_output {
        let payload = serde_json::json!({ "deleted": id });
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }

    println!("{} Removed exchange {}", "✓".green().bold(), id);
    Ok(())
}

async fn dispatch_apply(ticker: Option<&str>, json_output: bool) -> Result<()> {
    use crate::corporate_actions;

    let conn = open_conn()?;
    let actions = if let Some(ticker) = ticker {
        let assets = db::get_all_assets(&conn)?;
        let asset = assets
            .into_iter()
            .find(|a| a.ticker.eq_ignore_ascii_case(ticker))
            .context("Ticker not found in database")?;
        corporate_actions::get_unapplied_actions(&conn, Some(asset.id.unwrap()))?
    } else {
        corporate_actions::get_unapplied_actions(&conn, None)?
    };

    if actions.is_empty() {
        if json_output {
            let payload = serde_json::json!({ "applied": [] });
            println!("{}", serde_json::to_string_pretty(&payload)?);
            return Ok(());
        }
        println!("{} No unapplied corporate actions found", "ℹ".blue().bold());
        return Ok(());
    }

    let mut applied = Vec::new();
    for action in actions {
        let asset = db::get_all_assets(&conn)?
            .into_iter()
            .find(|a| a.id == Some(action.asset_id))
            .context("Asset not found")?;
        let adjusted_count = corporate_actions::apply_corporate_action(&conn, &action, &asset)?;
        applied.push((action, asset, adjusted_count));
    }

    if json_output {
        let payload: Vec<_> = applied
            .iter()
            .map(|(action, asset, adjusted)| {
                serde_json::json!({
                    "id": action.id,
                    "ticker": asset.ticker,
                    "type": action.action_type.as_str(),
                    "adjusted_transactions": adjusted,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }

    println!(
        "\n{} Applied {} corporate action(s)",
        "✓".green().bold(),
        applied.len()
    );
    for (action, asset, adjusted) in applied {
        println!(
            "  • {} {} ({} tx)",
            asset.ticker,
            action.action_type.as_str(),
            adjusted
        );
    }
    println!();

    Ok(())
}

fn parse_date(date_str: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(date_str, "%Y-%m-%d").context("Invalid date format. Use YYYY-MM-DD")
}

fn parse_decimal(value: &str) -> Result<Decimal> {
    Decimal::from_str(value).context("Invalid decimal value")
}
