use crate::utils::format_currency;
use crate::{db, reports};
use anyhow::Result;
use rust_decimal::Decimal;
use serde_json::{Map, Value};
use std::io::{stdin, stdout, BufRead, Write};
use std::str::FromStr;

pub async fn dispatch_inconsistencies(
    action: &crate::cli::InconsistenciesCommands,
    json_output: bool,
) -> Result<()> {
    db::init_database(None)?;
    let conn = db::open_db(None)?;

    match action {
        crate::cli::InconsistenciesCommands::List {
            open,
            all,
            status,
            issue_type,
            asset,
        } => {
            // Convert open/all flags to status
            let status = if *all {
                None
            } else if *open || status.is_none() {
                Some("OPEN")
            } else {
                status.as_deref()
            };
            let status = match status {
                Some("ALL") => None,
                Some(value) => Some(parse_inconsistency_status(value)?),
                None => Some(db::InconsistencyStatus::Open),
            };
            let issue_type = if let Some(s) = issue_type.as_deref() {
                Some(parse_inconsistency_type(s)?)
            } else {
                None
            };
            let issues = db::list_inconsistencies(&conn, status, issue_type, asset.as_deref())?;

            if json_output {
                println!("{}", serde_json::to_string_pretty(&issues)?);
                return Ok(());
            }

            if issues.is_empty() {
                println!("No inconsistencies found.");
                return Ok(());
            }

            for issue in issues {
                let qty = issue
                    .quantity
                    .as_ref()
                    .map(|q| q.to_string())
                    .unwrap_or_else(|| "-".to_string());
                let date = issue
                    .trade_date
                    .map(|d| d.to_string())
                    .unwrap_or_else(|| "-".to_string());
                let ticker = issue.ticker.clone().unwrap_or_else(|| "-".to_string());
                println!(
                    "#{:<5} {:<9} {:<24} {:<8} {:<10} qty={}",
                    issue.id.unwrap_or(0),
                    issue.status.as_str(),
                    issue.issue_type.as_str(),
                    ticker,
                    date,
                    qty
                );
            }
            Ok(())
        }
        crate::cli::InconsistenciesCommands::Show { id } => {
            let issue = db::get_inconsistency(&conn, *id)?
                .ok_or_else(|| anyhow::anyhow!("Inconsistency {} not found", id))?;

            if json_output {
                println!("{}", serde_json::to_string_pretty(&issue)?);
                return Ok(());
            }

            println!("ID: {}", issue.id.unwrap_or(*id));
            println!("Type: {}", issue.issue_type.as_str());
            println!("Status: {}", issue.status.as_str());
            println!("Severity: {}", issue.severity.as_str());
            println!(
                "Ticker: {}",
                issue.ticker.clone().unwrap_or_else(|| "-".to_string())
            );
            println!(
                "Trade date: {}",
                issue
                    .trade_date
                    .map(|d| d.to_string())
                    .unwrap_or_else(|| "-".to_string())
            );
            println!(
                "Quantity: {}",
                issue
                    .quantity
                    .map(|q| q.to_string())
                    .unwrap_or_else(|| "-".to_string())
            );
            println!(
                "Source: {}",
                issue.source.clone().unwrap_or_else(|| "-".to_string())
            );
            if let Some(source_ref) = issue.source_ref.clone() {
                println!("Source ref: {}", source_ref);
            }
            if let Some(missing) = issue.missing_fields_json.clone() {
                println!("Missing fields: {}", missing);
            }
            if let Some(context) = issue.context_json.clone() {
                println!("Context: {}", context);
            }
            if let Some(resolution) = issue.resolution_json.clone() {
                println!("Resolution: {}", resolution);
            }
            Ok(())
        }
        // The Resolve and Ignore variants rely on interactive helpers in the parent dispatcher module.
        crate::cli::InconsistenciesCommands::Resolve {
            id,
            set,
            json_payload,
        } => {
            let json = json_payload;
            // If no ID provided, iterate through all open inconsistencies
            let issues_to_resolve: Vec<crate::db::Inconsistency> = if let Some(id) = id {
                let issue = crate::db::get_inconsistency(&conn, *id)?
                    .ok_or_else(|| anyhow::anyhow!("Inconsistency {} not found", *id))?;
                vec![issue]
            } else {
                // Get all open inconsistencies
                let issues = crate::db::list_inconsistencies(
                    &conn,
                    Some(crate::db::InconsistencyStatus::Open),
                    None,
                    None,
                )?;
                if issues.is_empty() {
                    println!("No open inconsistencies to resolve.");
                    return Ok(());
                }
                println!(
                    "Found {} open inconsistenc{}. Going through them one by one.\n\
                     (Enter 's' to skip, 'q' to quit)\n",
                    issues.len(),
                    if issues.len() == 1 { "y" } else { "ies" }
                );
                issues
            };

            let mut resolved_count = 0;
            let total = issues_to_resolve.len();

            for (idx, issue) in issues_to_resolve.iter().enumerate() {
                let issue_id = issue.id.unwrap_or(0);

                // For batch mode (no ID), show progress
                if id.is_none() && total > 1 {
                    println!(
                        "━━━ [{}/{}] ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━",
                        idx + 1,
                        total
                    );
                }

                // Use inline values if provided, otherwise prompt interactively
                let resolution = if set.is_empty() && json.is_none() {
                    // Interactive mode: prompt based on issue type
                    let result = match &issue.issue_type {
                        crate::db::InconsistencyType::MissingCostBasis => {
                            prompt_missing_cost_basis(issue)
                        }
                        crate::db::InconsistencyType::MissingPurchaseHistory => {
                            prompt_missing_purchase_history(issue)
                        }
                        crate::db::InconsistencyType::InvalidTicker
                        | crate::db::InconsistencyType::InvalidDate => {
                            println!(
                                "Skipping #{} - interactive resolution for {} not implemented yet.",
                                issue_id,
                                issue.issue_type.as_str()
                            );
                            continue;
                        }
                    };

                    match result {
                        Ok(res) => res,
                        Err(e) => {
                            let msg = e.to_string();
                            // Check if user wants to skip or quit
                            if msg.contains("skip") || msg.contains("cancelled") {
                                println!("Skipped.\n");
                                continue;
                            } else if msg.contains("quit") {
                                println!("Stopping resolution.");
                                break;
                            } else {
                                // Re-raise other errors
                                return Err(e);
                            }
                        }
                    }
                } else {
                    // Parse Vec<String> with key=value format into Vec<(String, String)>
                    let parsed_set: Result<Vec<(String, String)>> = set
                        .iter()
                        .map(|s| {
                            let parts: Vec<&str> = s.splitn(2, '=').collect();
                            if parts.len() == 2 {
                                Ok((parts[0].to_string(), parts[1].to_string()))
                            } else {
                                Err(anyhow::anyhow!(
                                    "Invalid --set format: '{}'. Use key=value",
                                    s
                                ))
                            }
                        })
                        .collect();
                    build_resolution_map(&parsed_set?, json.as_deref())?
                };

                apply_inconsistency_resolution(&conn, issue, &resolution)?;
                resolved_count += 1;

                if json_output {
                    println!("{}", serde_json::json!({"resolved": issue_id}));
                } else {
                    println!("Resolved inconsistency #{}\n", issue_id);
                }
            }

            if id.is_none() && total > 1 && !json_output {
                println!(
                    "Done. Resolved {}/{} inconsistencies.",
                    resolved_count, total
                );
            }

            Ok(())
        }
        crate::cli::InconsistenciesCommands::Ignore { id, reason } => {
            crate::db::ignore_inconsistency(&conn, *id, reason.as_deref())?;
            if json_output {
                println!("{}", serde_json::json!({"ignored": id}));
            } else {
                println!("Ignored inconsistency {}", id);
            }
            Ok(())
        }
    }
}

// Helper parsing functions
fn parse_inconsistency_status(input: &str) -> Result<db::InconsistencyStatus> {
    input
        .parse::<db::InconsistencyStatus>()
        .map_err(|_| anyhow::anyhow!("Invalid inconsistency status: {}", input))
}

fn parse_inconsistency_type(input: &str) -> Result<db::InconsistencyType> {
    input
        .parse::<db::InconsistencyType>()
        .map_err(|_| anyhow::anyhow!("Invalid inconsistency type: {}", input))
}

fn build_resolution_map(
    set: &[(String, String)],
    json_payload: Option<&str>,
) -> Result<Map<String, Value>> {
    let mut map = Map::new();

    if let Some(payload) = json_payload {
        let value: Value = serde_json::from_str(payload)?;
        let obj = value
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("resolution JSON must be an object"))?;
        for (k, v) in obj.iter() {
            map.insert(k.clone(), v.clone());
        }
    }

    for (k, v) in set {
        map.insert(k.clone(), Value::String(v.clone()));
    }

    Ok(map)
}

fn get_decimal_field(map: &Map<String, Value>, key: &str) -> Result<Option<Decimal>> {
    match map.get(key) {
        None => Ok(None),
        Some(Value::String(s)) => Decimal::from_str(s)
            .map(Some)
            .map_err(|e| anyhow::anyhow!(e)),
        Some(Value::Number(n)) => Decimal::from_str(&n.to_string())
            .map(Some)
            .map_err(|e| anyhow::anyhow!(e)),
        _ => Err(anyhow::anyhow!("Invalid decimal value for {}", key)),
    }
}

fn get_string_field(map: &Map<String, Value>, key: &str) -> Option<String> {
    map.get(key).and_then(|v| v.as_str()).map(|s| s.to_string())
}

// ============ Interactive prompt helpers ============

fn prompt_line(msg: &str) -> Result<String> {
    print!("{}", msg);
    stdout().flush()?;
    let mut input = String::new();
    stdin().lock().read_line(&mut input)?;
    Ok(input.trim().to_string())
}

/// Check if input is a skip/quit command and return appropriate error
fn check_skip_quit(input: &str) -> Result<()> {
    let lower = input.to_lowercase();
    if lower == "s" || lower == "skip" {
        return Err(anyhow::anyhow!("skip"));
    }
    if lower == "q" || lower == "quit" {
        return Err(anyhow::anyhow!("quit"));
    }
    Ok(())
}

fn prompt_decimal(msg: &str, default: Option<Decimal>) -> Result<Option<Decimal>> {
    let prompt = match default {
        Some(d) => format!("{} [{}]: ", msg, d),
        None => format!("{}: ", msg),
    };
    let input = prompt_line(&prompt)?;
    check_skip_quit(&input)?;
    if input.is_empty() {
        Ok(default)
    } else {
        let val =
            Decimal::from_str(&input).map_err(|_| anyhow::anyhow!("Invalid number: {}", input))?;
        Ok(Some(val))
    }
}

fn prompt_date(msg: &str, default: Option<chrono::NaiveDate>) -> Result<Option<chrono::NaiveDate>> {
    let prompt = match default {
        Some(d) => format!("{} [{}]: ", msg, d),
        None => format!("{} (YYYY-MM-DD): ", msg),
    };
    let input = prompt_line(&prompt)?;
    check_skip_quit(&input)?;
    if input.is_empty() {
        Ok(default)
    } else {
        let date = chrono::NaiveDate::parse_from_str(&input, "%Y-%m-%d")
            .map_err(|_| anyhow::anyhow!("Invalid date format, use YYYY-MM-DD: {}", input))?;
        Ok(Some(date))
    }
}

fn prompt_confirm(msg: &str) -> Result<bool> {
    let input = prompt_line(&format!("{} [Y/n]: ", msg))?;
    check_skip_quit(&input)?;
    Ok(input.is_empty() || input.eq_ignore_ascii_case("y") || input.eq_ignore_ascii_case("yes"))
}

fn prompt_missing_cost_basis(issue: &db::Inconsistency) -> Result<Map<String, Value>> {
    println!(
        "\nResolving inconsistency #{}: MissingCostBasis",
        issue.id.unwrap_or(0)
    );
    println!("  Ticker: {}", issue.ticker.as_deref().unwrap_or("-"));
    println!(
        "  Date: {}",
        issue
            .trade_date
            .map(|d| d.to_string())
            .unwrap_or_else(|| "-".to_string())
    );
    println!(
        "  Quantity: {}",
        issue
            .quantity
            .map(|q| q.to_string())
            .unwrap_or_else(|| "-".to_string())
    );
    println!();

    let price = prompt_decimal("Enter price per unit (required)", None)?
        .ok_or_else(|| anyhow::anyhow!("price_per_unit is required"))?;

    let quantity = prompt_decimal("Enter quantity", issue.quantity)?
        .ok_or_else(|| anyhow::anyhow!("quantity is required"))?;

    let fees = prompt_decimal("Enter fees", Some(Decimal::ZERO))?.unwrap_or(Decimal::ZERO);

    let trade_date = prompt_date("Enter trade date", issue.trade_date)?
        .ok_or_else(|| anyhow::anyhow!("trade_date is required"))?;

    let total_cost = price * quantity + fees;
    println!();
    println!("Creating BUY transaction:");
    println!(
        "  {} x {} @ {} + {} fees = {}",
        issue.ticker.as_deref().unwrap_or("?"),
        quantity,
        format_currency(price),
        format_currency(fees),
        format_currency(total_cost)
    );

    if !prompt_confirm("Confirm?")? {
        return Err(anyhow::anyhow!("Resolution cancelled"));
    }

    let mut map = Map::new();
    map.insert(
        "price_per_unit".to_string(),
        Value::String(price.to_string()),
    );
    map.insert("quantity".to_string(), Value::String(quantity.to_string()));
    map.insert("fees".to_string(), Value::String(fees.to_string()));
    map.insert(
        "trade_date".to_string(),
        Value::String(trade_date.to_string()),
    );
    Ok(map)
}

fn prompt_missing_purchase_history(issue: &db::Inconsistency) -> Result<Map<String, Value>> {
    println!(
        "\nResolving inconsistency #{}: MissingPurchaseHistory",
        issue.id.unwrap_or(0)
    );
    println!("  Ticker: {}", issue.ticker.as_deref().unwrap_or("-"));
    println!(
        "  Date: {}",
        issue
            .trade_date
            .map(|d| d.to_string())
            .unwrap_or_else(|| "-".to_string())
    );
    println!(
        "  Quantity: {}",
        issue
            .quantity
            .map(|q| q.to_string())
            .unwrap_or_else(|| "-".to_string())
    );
    println!();
    println!("You need to add the missing purchase(s) for this asset.");
    println!();

    let price = prompt_decimal("Enter price per unit (required)", None)?
        .ok_or_else(|| anyhow::anyhow!("price_per_unit is required"))?;

    let quantity = prompt_decimal("Enter quantity", issue.quantity)?
        .ok_or_else(|| anyhow::anyhow!("quantity is required"))?;

    let fees = prompt_decimal("Enter fees", Some(Decimal::ZERO))?.unwrap_or(Decimal::ZERO);

    let trade_date = prompt_date("Enter trade date", issue.trade_date)?
        .ok_or_else(|| anyhow::anyhow!("trade_date is required"))?;

    let total_cost = price * quantity + fees;
    println!();
    println!("Creating BUY transaction:");
    println!(
        "  {} x {} @ {} + {} fees = {}",
        issue.ticker.as_deref().unwrap_or("?"),
        quantity,
        format_currency(price),
        format_currency(fees),
        format_currency(total_cost)
    );

    if !prompt_confirm("Confirm?")? {
        return Err(anyhow::anyhow!("Resolution cancelled"));
    }

    let mut map = Map::new();
    map.insert(
        "price_per_unit".to_string(),
        Value::String(price.to_string()),
    );
    map.insert("quantity".to_string(), Value::String(quantity.to_string()));
    map.insert("fees".to_string(), Value::String(fees.to_string()));
    map.insert(
        "trade_date".to_string(),
        Value::String(trade_date.to_string()),
    );
    Ok(map)
}

fn apply_inconsistency_resolution(
    conn: &rusqlite::Connection,
    issue: &db::Inconsistency,
    resolution: &Map<String, Value>,
) -> Result<()> {
    match &issue.issue_type {
        db::InconsistencyType::MissingCostBasis => {
            let price = get_decimal_field(resolution, "price_per_unit")?
                .or_else(|| get_decimal_field(resolution, "price").ok().flatten())
                .ok_or_else(|| anyhow::anyhow!("price_per_unit is required"))?;

            let quantity = get_decimal_field(resolution, "quantity")?
                .or(issue.quantity)
                .ok_or_else(|| anyhow::anyhow!("quantity is required"))?;

            let fees = get_decimal_field(resolution, "fees")?.unwrap_or(Decimal::ZERO);
            let total_cost = get_decimal_field(resolution, "total_cost")?
                .unwrap_or_else(|| price * quantity + fees);

            let trade_date = if let Some(date_str) = get_string_field(resolution, "trade_date") {
                chrono::NaiveDate::parse_from_str(&date_str, "%Y-%m-%d")
                    .map_err(|e| anyhow::anyhow!("Invalid trade_date: {}", e))?
            } else if let Some(date) = issue.trade_date {
                date
            } else {
                return Err(anyhow::anyhow!("trade_date is required"));
            };

            let asset_id = if let Some(asset_id) = issue.asset_id {
                asset_id
            } else if let Some(ticker) = issue.ticker.as_ref() {
                let asset_type = db::AssetType::Unknown;
                db::upsert_asset(conn, ticker, &asset_type, None)?
            } else {
                return Err(anyhow::anyhow!("asset is required"));
            };

            let notes = format!(
                "Resolved inconsistency {} (missing cost basis)",
                issue.id.unwrap_or(0)
            );

            let tx = db::Transaction {
                id: None,
                asset_id,
                transaction_type: db::TransactionType::Buy,
                trade_date,
                settlement_date: Some(trade_date),
                quantity,
                price_per_unit: price,
                total_cost,
                fees,
                is_day_trade: false,
                quota_issuance_date: None,
                notes: Some(notes),
                source: "INCONSISTENCY".to_string(),
                created_at: chrono::Utc::now(),
            };
            db::insert_transaction(conn, &tx)?;
            reports::invalidate_snapshots_after(conn, trade_date)?;
            db::resolve_inconsistency(
                conn,
                issue.id.unwrap_or(0),
                Some("ADD_TX"),
                Some(&Value::Object(resolution.clone()).to_string()),
            )?;
            Ok(())
        }
        db::InconsistencyType::MissingPurchaseHistory => {
            // Same logic as MissingCostBasis - creates a BUY transaction
            let price = get_decimal_field(resolution, "price_per_unit")?
                .or_else(|| get_decimal_field(resolution, "price").ok().flatten())
                .ok_or_else(|| anyhow::anyhow!("price_per_unit is required"))?;

            let quantity = get_decimal_field(resolution, "quantity")?
                .or(issue.quantity)
                .ok_or_else(|| anyhow::anyhow!("quantity is required"))?;

            let fees = get_decimal_field(resolution, "fees")?.unwrap_or(Decimal::ZERO);
            let total_cost = get_decimal_field(resolution, "total_cost")?
                .unwrap_or_else(|| price * quantity + fees);

            let trade_date = if let Some(date_str) = get_string_field(resolution, "trade_date") {
                chrono::NaiveDate::parse_from_str(&date_str, "%Y-%m-%d")
                    .map_err(|e| anyhow::anyhow!("Invalid trade_date: {}", e))?
            } else if let Some(date) = issue.trade_date {
                date
            } else {
                return Err(anyhow::anyhow!("trade_date is required"));
            };

            let asset_id = if let Some(asset_id) = issue.asset_id {
                asset_id
            } else if let Some(ticker) = issue.ticker.as_ref() {
                let asset_type = db::AssetType::Unknown;
                db::upsert_asset(conn, ticker, &asset_type, None)?
            } else {
                return Err(anyhow::anyhow!("asset is required"));
            };

            let notes = format!(
                "Resolved inconsistency {} (missing purchase history)",
                issue.id.unwrap_or(0)
            );

            let tx = db::Transaction {
                id: None,
                asset_id,
                transaction_type: db::TransactionType::Buy,
                trade_date,
                settlement_date: Some(trade_date),
                quantity,
                price_per_unit: price,
                total_cost,
                fees,
                is_day_trade: false,
                quota_issuance_date: None,
                notes: Some(notes),
                source: "INCONSISTENCY".to_string(),
                created_at: chrono::Utc::now(),
            };
            db::insert_transaction(conn, &tx)?;
            reports::invalidate_snapshots_after(conn, trade_date)?;
            db::resolve_inconsistency(
                conn,
                issue.id.unwrap_or(0),
                Some("ADD_TX"),
                Some(&Value::Object(resolution.clone()).to_string()),
            )?;
            Ok(())
        }
        db::InconsistencyType::InvalidTicker | db::InconsistencyType::InvalidDate => Err(
            anyhow::anyhow!("Resolution for this inconsistency type is not implemented yet"),
        ),
    }
}
