//! Command dispatcher that routes both clap Commands and custom Command enums
//! to the appropriate handlers.
//!
//! This module provides a unified interface for command routing, making it easy
//! to switch between different command sources (CLI args vs interactive input).

pub mod performance;
use performance::dispatch_performance_show;

use crate::commands::{Command, InconsistenciesAction};
use crate::ui::crossterm_engine::Spinner;
use crate::utils::format_currency;
use crate::{cli, db, reports, tax};
use anyhow::Result;
use colored::Colorize;
use rust_decimal::Decimal;
use serde_json::{json, Map, Value};
use std::io::{stdin, stdout, BufRead, Write};
use std::str::FromStr;
use tracing::info;

/// Route a parsed command to its handler
pub async fn dispatch_command(command: Command, json_output: bool) -> Result<()> {
    match command {
        Command::Import { path, dry_run } => {
            // TODO: Wire up import handler
            eprintln!("Import command: {} (dry_run: {})", path, dry_run);
            Ok(())
        }
        Command::PortfolioShow { filter, as_of_date } => {
            dispatch_portfolio_show(filter.as_deref(), as_of_date.as_deref(), json_output).await
        }
        Command::PerformanceShow { period } => {
            dispatch_performance_show(&period, json_output).await
        }
        Command::TaxReport { year, export_csv } => {
            dispatch_tax_report(year, export_csv, json_output).await
        }
        Command::TaxSummary { year } => dispatch_tax_summary(year, json_output).await,
        Command::IncomeShow { year } => dispatch_income_show(year, json_output).await,
        Command::IncomeDetail { year, asset } => {
            dispatch_income_detail(year, asset.as_deref(), json_output).await
        }
        Command::IncomeSummary { year } => dispatch_income_summary(year, json_output).await,
        Command::Prices { action } => dispatch_prices(action, json_output).await,
        Command::Inconsistencies { action } => dispatch_inconsistencies(action, json_output).await,
        Command::Help => {
            println!("Help: interest <command> [options]");
            println!("\nAvailable commands:");
            println!("  import <file>              - Import transactions");
            println!(
                "  portfolio show [--at DATE] - Show portfolio (DATE: YYYY-MM-DD|YYYY-MM|YYYY)"
            );
            println!(
                "  performance show <P>       - Show performance (P: MTD|QTD|YTD|1Y|ALL|from:to)"
            );
            println!("  tax report <year>          - Generate tax report");
            println!("  tax summary <year>         - Show tax summary");
            println!("  income show [year]         - Show income summary by asset");
            println!("  income detail [year]       - Show detailed income events");
            println!(
                "  income summary [year]      - Show yearly totals (or monthly if year given)"
            );
            println!("  prices import-b3 <year>    - Import B3 COTAHIST data for year");
            println!("  prices import-b3-file <p>  - Import COTAHIST from local ZIP file");
            println!("  prices clear-cache [year]  - Clear B3 COTAHIST cache");
            println!("  inconsistencies list       - List inconsistencies");
            println!("  inconsistencies show <id>  - Show inconsistency details");
            println!("  inconsistencies resolve    - Resolve inconsistency");
            println!("  inconsistencies ignore     - Ignore inconsistency");
            println!("  help                       - Show this help");
            println!("  exit                       - Exit application");
            Ok(())
        }
        Command::Exit => {
            std::process::exit(0);
        }
    }
}

async fn dispatch_inconsistencies(action: InconsistenciesAction, json_output: bool) -> Result<()> {
    db::init_database(None)?;
    let conn = db::open_db(None)?;

    match action {
        InconsistenciesAction::List {
            status,
            issue_type,
            asset,
        } => {
            let status = match status.as_deref() {
                Some("ALL") => None,
                Some(value) => Some(parse_inconsistency_status(value)?),
                None => Some(db::InconsistencyStatus::Open),
            };
            let issue_type = issue_type
                .as_deref()
                .map(parse_inconsistency_type)
                .transpose()?;

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
        InconsistenciesAction::Show { id } => {
            let issue = db::get_inconsistency(&conn, id)?
                .ok_or_else(|| anyhow::anyhow!("Inconsistency {} not found", id))?;

            if json_output {
                println!("{}", serde_json::to_string_pretty(&issue)?);
                return Ok(());
            }

            println!("ID: {}", issue.id.unwrap_or(id));
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
        InconsistenciesAction::Resolve { id, set, json } => {
            // If no ID provided, iterate through all open inconsistencies
            let issues_to_resolve: Vec<db::Inconsistency> = if let Some(id) = id {
                let issue = db::get_inconsistency(&conn, id)?
                    .ok_or_else(|| anyhow::anyhow!("Inconsistency {} not found", id))?;
                vec![issue]
            } else {
                // Get all open inconsistencies
                let issues = db::list_inconsistencies(
                    &conn,
                    Some(db::InconsistencyStatus::Open),
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
                        db::InconsistencyType::MissingCostBasis => prompt_missing_cost_basis(issue),
                        db::InconsistencyType::MissingPurchaseHistory => {
                            prompt_missing_purchase_history(issue)
                        }
                        db::InconsistencyType::InvalidTicker
                        | db::InconsistencyType::InvalidDate => {
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
                    build_resolution_map(&set, json.as_deref())?
                };

                apply_inconsistency_resolution(&conn, issue, &resolution)?;
                resolved_count += 1;

                if json_output {
                    println!("{}", json!({"resolved": issue_id}));
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
        InconsistenciesAction::Ignore { id, reason } => {
            db::ignore_inconsistency(&conn, id, reason.as_deref())?;
            if json_output {
                println!("{}", json!({"ignored": id}));
            } else {
                println!("Ignored inconsistency {}", id);
            }
            Ok(())
        }
    }
}

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
                let asset_type =
                    db::AssetType::detect_from_ticker(ticker).unwrap_or(db::AssetType::Stock);
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
                let asset_type =
                    db::AssetType::detect_from_ticker(ticker).unwrap_or(db::AssetType::Stock);
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

async fn dispatch_portfolio_show(
    asset_type: Option<&str>,
    as_of_date: Option<&str>,
    json_output: bool,
) -> Result<()> {
    info!("Generating portfolio report");

    // Initialize database
    db::init_database(None)?;
    let mut conn = db::open_db(None)?;

    // Get blocked assets (those with open blocking inconsistencies)
    let blocked_assets = db::get_blocked_assets(&conn)?;
    if !blocked_assets.is_empty() {
        let blocked_tickers: Vec<&str> = blocked_assets.iter().map(|(_, t)| t.as_str()).collect();
        anyhow::bail!(
            "Refusing to show portfolio due to open blocking inconsistencies.\nAssets: {}\nResolve with `inconsistencies resolve`.",
            blocked_tickers.join(", ")
        );
    }

    // Parse date if provided (already validated by parse_flexible_date in commands.rs)
    let historical_date = if let Some(date_str) = as_of_date {
        let date = chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d")
            .map_err(|e| anyhow::anyhow!("Invalid date '{}': {}", date_str, e))?;
        let today = chrono::Local::now().date_naive();
        if date > today {
            return Err(anyhow::anyhow!(
                "Date cannot be in the future (today is {})",
                today
            ));
        }
        Some(date)
    } else {
        None
    };

    // Skip live price fetch for historical views (use cached prices from price_history)
    let skip_price_fetch = historical_date.is_some()
        || std::env::var("INTEREST_SKIP_PRICE_FETCH")
            .map(|v| v != "0")
            .unwrap_or(false);

    // Parse asset type filter if provided
    let asset_type_filter = if let Some(type_str) = asset_type {
        Some(
            type_str
                .parse::<db::AssetType>()
                .map_err(|_| anyhow::anyhow!("Invalid asset type: {}", type_str))?,
        )
    } else {
        None
    };

    // Get earliest transaction date to determine price range needed
    let earliest_date = db::get_earliest_transaction_date(&conn)?;
    if earliest_date.is_none() {
        // No transactions - nothing to show
        if !json_output {
            println!("{}", cli::formatters::format_empty_portfolio());
        }
        return Ok(());
    }

    let earliest_date = earliest_date.unwrap();
    let today = chrono::Local::now().date_naive();

    // Calculate portfolio positions first (fast, no network calls)
    let mut report = if let Some(date) = historical_date {
        reports::calculate_portfolio_at_date(&conn, date, asset_type_filter.as_ref())?
    } else {
        reports::calculate_portfolio(&conn, asset_type_filter.as_ref())?
    };

    if report.positions.is_empty() {
        if !json_output {
            println!("{}", cli::formatters::format_empty_portfolio());
        }
        return Ok(());
    }

    // Now fetch prices ONLY for assets that have current positions
    if !skip_price_fetch {
        if !json_output {
            let assets_with_positions: Vec<_> =
                report.positions.iter().map(|p| p.asset.clone()).collect();

            if !assets_with_positions.is_empty() {
                let total = assets_with_positions.len();
                let spinner = Spinner::new();
                let mut completed = 0usize;

                // Show initial spinner
                print!("{} Fetching prices 0/{}...", spinner.tick(), total);
                stdout().flush().ok();

                crate::pricing::resolver::ensure_prices_available_with_progress(
                    &mut conn,
                    &assets_with_positions,
                    (earliest_date, today),
                    |msg| {
                        // Extract display mode from message prefix
                        let (msg_content, should_persist) = if msg.starts_with("__PERSIST__:") {
                            (msg.strip_prefix("__PERSIST__:").unwrap_or(msg), true)
                        } else {
                            (msg, false)
                        };

                        // Check if this is a ticker result (contains "→")
                        if let Some(count) = parse_progress_count(msg_content) {
                            completed = count;
                            // Clear spinner line, print ticker result, re-draw spinner
                            print!("\r\x1B[2K"); // Clear current line
                            println!("  {} {}", "↳".dimmed(), msg_content); // Print ticker with newline
                            print!(
                                "{} Fetching prices {}/{}...",
                                spinner.tick(),
                                completed,
                                total
                            );
                            stdout().flush().ok();
                        } else if should_persist {
                            // Message should be persisted to terminal with newline
                            print!("\r\x1B[2K"); // Clear current line

                            // Format messages: completion (✓) in green, errors (❌) in red
                            if msg_content.starts_with("✓") {
                                println!("  {} {}", "↳".dimmed(), msg_content.green());
                            } else if msg_content.starts_with("❌") {
                                println!("  {} {}", "↳".dimmed(), msg_content.red());
                            } else {
                                println!("  {} {}", "↳".dimmed(), msg_content);
                            }

                            // Re-draw spinner on next line
                            print!(
                                "{} Fetching prices {}/{}...",
                                spinner.tick(),
                                completed,
                                total
                            );
                            stdout().flush().ok();
                        } else {
                            // Spinner-only update (download, decompress, parsing intermediate)
                            print!("\r\x1B[2K{} {}", spinner.tick(), msg_content);
                            stdout().flush().ok();
                        }
                    },
                )
                .await
                .or_else(|e: anyhow::Error| {
                    print!("\r\x1B[2K"); // Clear spinner on error
                    stdout().flush().ok();
                    tracing::warn!("Price resolution failed: {}", e);
                    Ok::<(), anyhow::Error>(())
                })?;

                // Recalculate portfolio with updated prices
                report = if let Some(date) = historical_date {
                    reports::calculate_portfolio_at_date(&conn, date, asset_type_filter.as_ref())?
                } else {
                    reports::calculate_portfolio(&conn, asset_type_filter.as_ref())?
                };
            }
        } else {
            // JSON mode: no spinner, just fetch silently
            let assets_with_positions: Vec<_> =
                report.positions.iter().map(|p| p.asset.clone()).collect();

            if !assets_with_positions.is_empty() {
                crate::pricing::resolver::ensure_prices_available(
                    &mut conn,
                    &assets_with_positions,
                    (earliest_date, today),
                )
                .await
                .or_else(|e: anyhow::Error| {
                    tracing::warn!("Price resolution failed: {}", e);
                    Ok::<(), anyhow::Error>(())
                })?;

                report = if let Some(date) = historical_date {
                    reports::calculate_portfolio_at_date(&conn, date, asset_type_filter.as_ref())?
                } else {
                    reports::calculate_portfolio(&conn, asset_type_filter.as_ref())?
                };
            }
        }
    }

    if report.positions.is_empty() {
        if !json_output {
            if let Some(date) = historical_date {
                println!(
                    "{} No positions held as of {}\n",
                    "ℹ".blue().bold(),
                    date.format("%Y-%m-%d")
                );
            } else {
                println!("{}", cli::formatters::format_empty_portfolio());
            }
        }
        return Ok(());
    }

    if json_output {
        println!("{}", cli::formatters::format_portfolio_json(&report));
        return Ok(());
    }

    // Show historical date header if applicable
    if let Some(date) = historical_date {
        println!(
            "\n{} Portfolio as of {}\n",
            "📅".cyan().bold(),
            date.format("%Y-%m-%d")
        );
    }

    // Use formatter for table output
    let filter_str = asset_type.map(|f| f.to_uppercase());
    println!(
        "{}",
        cli::formatters::format_portfolio_table(&report, filter_str.as_deref())
    );

    // Display asset allocation if showing full portfolio
    if asset_type_filter.is_none() {
        let allocation = reports::calculate_allocation(&report);

        if allocation.len() > 1 {
            println!("\n{} Asset Allocation", "🎯".cyan().bold());

            let mut alloc_vec: Vec<_> = allocation.iter().collect();
            alloc_vec.sort_by(|a, b| b.1 .0.cmp(&a.1 .0));

            for (asset_type, (value, pct)) in alloc_vec {
                let type_ref: &db::AssetType = asset_type;
                println!(
                    "  {}: {} ({:.2}%)",
                    type_ref.as_str().to_uppercase(),
                    format_currency(*value).cyan(),
                    pct
                );
            }
        }
    }

    println!();
    Ok(())
}

async fn dispatch_tax_report(year: i32, export_csv: bool, json_output: bool) -> Result<()> {
    info!("Generating IRPF annual report for {}", year);

    // Initialize database
    db::init_database(None)?;
    let conn = db::open_db(None)?;

    // Generate report; suppress progress output in JSON mode
    let report = if json_output {
        tax::generate_annual_report_with_progress(&conn, year, |_ev| {})?
    } else {
        let mut printer = TaxProgressPrinter::new(true);
        tax::generate_annual_report_with_progress(&conn, year, |ev| printer.on_event(ev))?
    };

    if report.monthly_summaries.is_empty() {
        println!(
            "\n{} No transactions found for year {}\n",
            "ℹ".blue().bold(),
            year
        );
        return Ok(());
    }

    if json_output {
        // Emit concise JSON suitable for tests and scripting
        #[derive(serde::Serialize)]
        struct MonthlySummaryJson {
            month: String,
            sales: rust_decimal::Decimal,
            profit: rust_decimal::Decimal,
            loss: rust_decimal::Decimal,
            tax_due: rust_decimal::Decimal,
        }

        let monthly: Vec<MonthlySummaryJson> = report
            .monthly_summaries
            .iter()
            .map(|m| MonthlySummaryJson {
                month: m.month_name.to_string(),
                sales: m.total_sales,
                profit: m.total_profit,
                loss: m.total_loss,
                tax_due: m.tax_due,
            })
            .collect();

        let payload = serde_json::json!({
            "year": year,
            "annual_total_sales": report.annual_total_sales,
            "annual_total_profit": report.annual_total_profit,
            "annual_total_loss": report.annual_total_loss,
            "annual_total_tax": report.annual_total_tax,
            "monthly_summaries": monthly,
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    } else {
        println!(
            "\n{} Annual IRPF Tax Report - {}\n",
            "📊".cyan().bold(),
            year
        );
    }

    // Show prior-year carryforward losses if any
    if !report.previous_losses_carry_forward.is_empty() {
        println!("{} Carryover from previous years:", "📦".yellow().bold());
        for (category, amount) in &report.previous_losses_carry_forward {
            println!(
                "  {}: {}",
                category.display_name(),
                format_currency(*amount)
            );
        }
        println!();
    }

    // Monthly breakdown
    println!("{}", "Monthly Summary:".bold());
    for summary in &report.monthly_summaries {
        println!("\n  {}:", summary.month_name.bold());
        println!(
            "    Sales:  {}",
            format_currency(summary.total_sales).cyan()
        );
        println!(
            "    Profit: {}",
            format_currency(summary.total_profit).green()
        );
        println!("    Loss:   {}", format_currency(summary.total_loss).red());
        println!("    Tax:    {}", format_currency(summary.tax_due).yellow());
    }

    // Annual totals
    println!("\n{} Annual Totals:", "📈".cyan().bold());
    println!(
        "  Total Sales:  {}",
        format_currency(report.annual_total_sales).cyan()
    );
    println!(
        "  Total Profit: {}",
        format_currency(report.annual_total_profit).green()
    );
    println!(
        "  Total Loss:   {}",
        format_currency(report.annual_total_loss).red()
    );
    println!(
        "  {} {}\n",
        "Total Tax:".bold(),
        format_currency(report.annual_total_tax).yellow().bold()
    );

    // Losses to carry forward
    if !report.losses_to_carry_forward.is_empty() {
        println!("{} Losses to Carry Forward:", "📋".yellow().bold());
        for (category, loss) in &report.losses_to_carry_forward {
            println!(
                "  {}: {}",
                category.display_name(),
                format_currency(*loss).yellow()
            );
        }
        println!();
    }

    if export_csv {
        let csv_content = tax::irpf::export_to_csv(&report);
        let csv_path = format!("irpf_report_{}.csv", year);
        std::fs::write(&csv_path, csv_content)?;

        println!("{} Report exported to: {}\n", "✓".green().bold(), csv_path);
    }

    Ok(())
}

async fn dispatch_tax_summary(year: i32, _json_output: bool) -> Result<()> {
    use tabled::{
        settings::{object::Columns, Alignment, Modify, Style},
        Table, Tabled,
    };

    info!("Generating tax summary for {}", year);

    // Initialize database
    db::init_database(None)?;
    let conn = db::open_db(None)?;

    // Generate report with in-place spinner progress (terse)
    let mut printer = TaxProgressPrinter::new(true);
    let report = tax::generate_annual_report_with_progress(&conn, year, |ev| printer.on_event(ev))?;

    if report.monthly_summaries.is_empty() {
        println!(
            "\n{} No transactions found for year {}\n",
            "ℹ".blue().bold(),
            year
        );
        return Ok(());
    }

    println!("\n{} Tax Summary - {}\n", "📊".cyan().bold(), year);

    // Display monthly table
    #[derive(Tabled)]
    struct MonthRow {
        #[tabled(rename = "Month")]
        month: String,
        #[tabled(rename = "Sales")]
        sales: String,
        #[tabled(rename = "Profit")]
        profit: String,
        #[tabled(rename = "Loss")]
        loss: String,
        #[tabled(rename = "Tax Due")]
        tax: String,
    }

    let rows: Vec<MonthRow> = report
        .monthly_summaries
        .iter()
        .map(|s| MonthRow {
            month: s.month_name.to_string(),
            sales: format_currency(s.total_sales),
            profit: format_currency(s.total_profit),
            loss: format_currency(s.total_loss),
            tax: format_currency(s.tax_due),
        })
        .collect();

    let table = Table::new(rows)
        .with(Style::rounded())
        .with(Modify::new(Columns::new(1..)).with(Alignment::right()))
        .to_string();
    println!("{}", table);

    // Annual summary
    println!("\n{} Annual Total", "📈".cyan().bold());
    println!(
        "  Sales:  {}",
        format_currency(report.annual_total_sales).cyan()
    );
    println!(
        "  Profit: {}",
        format_currency(report.annual_total_profit).green()
    );
    println!(
        "  Loss:   {}",
        format_currency(report.annual_total_loss).red()
    );
    println!(
        "  {} {}\n",
        "Tax:".bold(),
        format_currency(report.annual_total_tax).yellow().bold()
    );

    Ok(())
}

/// Show income summary by asset, grouped by asset type
async fn dispatch_income_show(year: Option<i32>, json_output: bool) -> Result<()> {
    use chrono::Datelike;
    use rust_decimal::Decimal;
    use serde::Serialize;
    use std::collections::HashMap;
    use tabled::{
        settings::{object::Columns, Alignment, Modify, Style},
        Table, Tabled,
    };

    info!("Showing income summary by asset");

    // Initialize database
    db::init_database(None)?;
    let conn = db::open_db(None)?;

    // Determine date range
    let today = chrono::Local::now().date_naive();
    let (from_date, to_date, year_val) = match year {
        Some(y) => {
            let from = chrono::NaiveDate::from_ymd_opt(y, 1, 1).unwrap();
            let to = chrono::NaiveDate::from_ymd_opt(y, 12, 31).unwrap();
            (Some(from), Some(to), y)
        }
        None => {
            let y = today.year();
            let from = chrono::NaiveDate::from_ymd_opt(y, 1, 1).unwrap();
            (Some(from), Some(today), y)
        }
    };

    // Query income events
    let events = db::get_income_events_with_assets(&conn, from_date, to_date, None)?;

    if events.is_empty() {
        println!(
            "\n{} No income events found for {}.\n",
            "ℹ".blue().bold(),
            year_val
        );
        return Ok(());
    }

    // Group by asset type and ticker
    struct AssetIncome {
        ticker: String,
        asset_type: db::AssetType,
        dividends: Decimal,
        jcp: Decimal,
        amortization: Decimal,
    }

    let mut by_ticker: HashMap<String, AssetIncome> = HashMap::new();

    for (event, asset) in &events {
        let entry = by_ticker
            .entry(asset.ticker.clone())
            .or_insert(AssetIncome {
                ticker: asset.ticker.clone(),
                asset_type: asset.asset_type,
                dividends: Decimal::ZERO,
                jcp: Decimal::ZERO,
                amortization: Decimal::ZERO,
            });

        match event.event_type {
            db::IncomeEventType::Dividend => entry.dividends += event.total_amount,
            db::IncomeEventType::Jcp => entry.jcp += event.total_amount,
            db::IncomeEventType::Amortization => entry.amortization += event.total_amount,
        }
    }

    // Group by asset type
    let mut by_type: HashMap<db::AssetType, Vec<AssetIncome>> = HashMap::new();
    for (_, income) in by_ticker {
        by_type.entry(income.asset_type).or_default().push(income);
    }

    // Sort each group by total (descending)
    for assets in by_type.values_mut() {
        assets.sort_by(|a, b| {
            let total_a = a.dividends + a.jcp + a.amortization;
            let total_b = b.dividends + b.jcp + b.amortization;
            total_b.cmp(&total_a)
        });
    }

    if json_output {
        #[derive(Serialize)]
        struct JsonAssetIncome {
            ticker: String,
            asset_type: String,
            dividends: String,
            jcp: String,
            amortization: String,
            total: String,
        }

        let mut all_assets: Vec<JsonAssetIncome> = Vec::new();
        for (asset_type, assets) in &by_type {
            for a in assets {
                let total = a.dividends + a.jcp + a.amortization;
                all_assets.push(JsonAssetIncome {
                    ticker: a.ticker.clone(),
                    asset_type: asset_type.as_str().to_string(),
                    dividends: a.dividends.to_string(),
                    jcp: a.jcp.to_string(),
                    amortization: a.amortization.to_string(),
                    total: total.to_string(),
                });
            }
        }
        println!("{}", serde_json::to_string_pretty(&all_assets)?);
        return Ok(());
    }

    println!("\n{} Income Summary - {}\n", "💰".cyan().bold(), year_val);

    // Define display order for asset types
    let type_order = [
        db::AssetType::Stock,
        db::AssetType::Fii,
        db::AssetType::Fiagro,
        db::AssetType::FiInfra,
        db::AssetType::Etf,
        db::AssetType::Bond,
        db::AssetType::GovBond,
    ];

    let mut grand_total = Decimal::ZERO;

    for asset_type in &type_order {
        if let Some(assets) = by_type.get(asset_type) {
            if assets.is_empty() {
                continue;
            }

            #[derive(Tabled)]
            struct IncomeRow {
                #[tabled(rename = "Ticker")]
                ticker: String,
                #[tabled(rename = "Dividends")]
                dividends: String,
                #[tabled(rename = "JCP")]
                jcp: String,
                #[tabled(rename = "Amort")]
                amort: String,
                #[tabled(rename = "Total")]
                total: String,
            }

            let rows: Vec<IncomeRow> = assets
                .iter()
                .map(|a| {
                    let total = a.dividends + a.jcp + a.amortization;
                    IncomeRow {
                        ticker: a.ticker.clone(),
                        dividends: if a.dividends > Decimal::ZERO {
                            format_currency(a.dividends)
                        } else {
                            "-".to_string()
                        },
                        jcp: if a.jcp > Decimal::ZERO {
                            format_currency(a.jcp)
                        } else {
                            "-".to_string()
                        },
                        amort: if a.amortization > Decimal::ZERO {
                            format_currency(a.amortization)
                        } else {
                            "-".to_string()
                        },
                        total: format_currency(total),
                    }
                })
                .collect();

            let type_total: Decimal = assets
                .iter()
                .map(|a| a.dividends + a.jcp + a.amortization)
                .sum();
            grand_total += type_total;

            println!(
                "{} {} ({})",
                "▸".cyan(),
                asset_type.as_str().to_uppercase().bold(),
                format_currency(type_total).cyan()
            );

            let table = Table::new(&rows)
                .with(Style::rounded())
                .with(Modify::new(Columns::new(1..)).with(Alignment::right()))
                .to_string();
            println!("{}\n", table);
        }
    }

    println!(
        "{} {}\n",
        "Grand Total:".bold(),
        format_currency(grand_total).green().bold()
    );

    Ok(())
}

/// Show detailed income events
async fn dispatch_income_detail(
    year: Option<i32>,
    asset: Option<&str>,
    json_output: bool,
) -> Result<()> {
    use chrono::Datelike;
    use rust_decimal::Decimal;
    use serde::Serialize;
    use tabled::{
        settings::{object::Columns, Alignment, Modify, Style},
        Table, Tabled,
    };

    info!("Showing income events detail");

    // Initialize database
    db::init_database(None)?;
    let conn = db::open_db(None)?;

    // Determine date range
    let today = chrono::Local::now().date_naive();
    let (from_date, to_date) = match year {
        Some(y) => {
            let from = chrono::NaiveDate::from_ymd_opt(y, 1, 1).unwrap();
            let to = chrono::NaiveDate::from_ymd_opt(y, 12, 31).unwrap();
            (Some(from), Some(to))
        }
        None => {
            // Default to current year
            let y = today.year();
            let from = chrono::NaiveDate::from_ymd_opt(y, 1, 1).unwrap();
            (Some(from), Some(today))
        }
    };

    // Query income events
    let events = db::get_income_events_with_assets(&conn, from_date, to_date, asset)?;

    if events.is_empty() {
        let year_str = year
            .map(|y| y.to_string())
            .unwrap_or_else(|| today.year().to_string());
        let asset_str = asset.map(|a| format!(" for {}", a)).unwrap_or_default();
        println!(
            "\n{} No income events found for {}{}.\n",
            "ℹ".blue().bold(),
            year_str,
            asset_str
        );
        return Ok(());
    }

    if json_output {
        #[derive(Serialize)]
        struct IncomeRow {
            date: String,
            ticker: String,
            asset_type: String,
            event_type: String,
            amount: String,
            notes: Option<String>,
        }

        let rows: Vec<IncomeRow> = events
            .iter()
            .map(|(event, asset)| IncomeRow {
                date: event.event_date.to_string(),
                ticker: asset.ticker.clone(),
                asset_type: asset.asset_type.as_str().to_string(),
                event_type: event.event_type.as_str().to_string(),
                amount: event.total_amount.to_string(),
                notes: event.notes.clone(),
            })
            .collect();

        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }

    // Display table
    let year_str = year
        .map(|y| y.to_string())
        .unwrap_or_else(|| today.year().to_string());
    let asset_str = asset.map(|a| format!(" - {}", a)).unwrap_or_default();
    println!(
        "\n{} Income Events - {}{}\n",
        "💰".cyan().bold(),
        year_str,
        asset_str
    );

    #[derive(Tabled)]
    struct IncomeTableRow {
        #[tabled(rename = "Date")]
        date: String,
        #[tabled(rename = "Ticker")]
        ticker: String,
        #[tabled(rename = "Type")]
        event_type: String,
        #[tabled(rename = "Amount")]
        amount: String,
        #[tabled(rename = "Notes")]
        notes: String,
    }

    let rows: Vec<IncomeTableRow> = events
        .iter()
        .map(|(event, asset)| IncomeTableRow {
            date: event.event_date.format("%Y-%m-%d").to_string(),
            ticker: asset.ticker.clone(),
            event_type: match event.event_type {
                db::IncomeEventType::Dividend => "Dividend",
                db::IncomeEventType::Jcp => "JCP",
                db::IncomeEventType::Amortization => "Amort",
            }
            .to_string(),
            amount: format_currency(event.total_amount),
            notes: event.notes.clone().unwrap_or_default(),
        })
        .collect();

    let table = Table::new(&rows)
        .with(Style::rounded())
        .with(Modify::new(Columns::new(3..4)).with(Alignment::right()))
        .to_string();
    println!("{}", table);

    // Summary
    let total: Decimal = events.iter().map(|(e, _)| e.total_amount).sum();
    let dividends: Decimal = events
        .iter()
        .filter(|(e, _)| matches!(e.event_type, db::IncomeEventType::Dividend))
        .map(|(e, _)| e.total_amount)
        .sum();
    let jcp: Decimal = events
        .iter()
        .filter(|(e, _)| matches!(e.event_type, db::IncomeEventType::Jcp))
        .map(|(e, _)| e.total_amount)
        .sum();
    let amort: Decimal = events
        .iter()
        .filter(|(e, _)| matches!(e.event_type, db::IncomeEventType::Amortization))
        .map(|(e, _)| e.total_amount)
        .sum();

    println!("\n{} Summary:", "📊".cyan().bold());
    if dividends > Decimal::ZERO {
        println!("  Dividends:    {}", format_currency(dividends).green());
    }
    if jcp > Decimal::ZERO {
        println!("  JCP:          {}", format_currency(jcp).green());
    }
    if amort > Decimal::ZERO {
        println!("  Amortization: {}", format_currency(amort).yellow());
    }
    println!(
        "  {} {}\n",
        "Total:".bold(),
        format_currency(total).green().bold()
    );

    Ok(())
}

/// Show income summary - monthly breakdown if year given, yearly totals otherwise
pub async fn dispatch_income_summary(year: Option<i32>, json_output: bool) -> Result<()> {
    use chrono::Datelike;
    use rust_decimal::Decimal;
    use serde::Serialize;
    use std::collections::BTreeMap;
    use tabled::{
        settings::{object::Columns, Alignment, Modify, Style},
        Table, Tabled,
    };

    // Initialize database
    db::init_database(None)?;
    let conn = db::open_db(None)?;

    match year {
        Some(y) => {
            // Monthly breakdown for specific year
            info!("Showing income summary with monthly breakdown for {}", y);

            let from_date = chrono::NaiveDate::from_ymd_opt(y, 1, 1).unwrap();
            let to_date = chrono::NaiveDate::from_ymd_opt(y, 12, 31).unwrap();

            let events =
                db::get_income_events_with_assets(&conn, Some(from_date), Some(to_date), None)?;

            if events.is_empty() {
                println!(
                    "\n{} No income events found for {}.\n",
                    "ℹ".blue().bold(),
                    y
                );
                return Ok(());
            }

            let month_names = [
                "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
            ];

            struct MonthlyTotals {
                dividends: Decimal,
                jcp: Decimal,
                amortization: Decimal,
            }

            let mut monthly: Vec<MonthlyTotals> = (0..12)
                .map(|_| MonthlyTotals {
                    dividends: Decimal::ZERO,
                    jcp: Decimal::ZERO,
                    amortization: Decimal::ZERO,
                })
                .collect();

            for (event, _asset) in &events {
                let month_idx = (event.event_date.month() - 1) as usize;
                match event.event_type {
                    db::IncomeEventType::Dividend => {
                        monthly[month_idx].dividends += event.total_amount
                    }
                    db::IncomeEventType::Jcp => monthly[month_idx].jcp += event.total_amount,
                    db::IncomeEventType::Amortization => {
                        monthly[month_idx].amortization += event.total_amount
                    }
                }
            }

            let total_dividends: Decimal = monthly.iter().map(|m| m.dividends).sum();
            let total_jcp: Decimal = monthly.iter().map(|m| m.jcp).sum();
            let total_amortization: Decimal = monthly.iter().map(|m| m.amortization).sum();
            let grand_total = total_dividends + total_jcp + total_amortization;

            let months_with_income = monthly
                .iter()
                .filter(|m| m.dividends + m.jcp + m.amortization > Decimal::ZERO)
                .count();
            let avg_per_month = if months_with_income > 0 {
                grand_total / Decimal::from(months_with_income)
            } else {
                Decimal::ZERO
            };

            // Calculate totals by asset type
            let mut asset_type_totals: std::collections::HashMap<db::AssetType, Decimal> =
                std::collections::HashMap::new();
            for (event, asset) in &events {
                *asset_type_totals
                    .entry(asset.asset_type)
                    .or_insert(Decimal::ZERO) += event.total_amount;
            }
            let mut asset_type_vec: Vec<_> = asset_type_totals.iter().collect();
            asset_type_vec.sort_by(|a, b| b.1.cmp(a.1)); // Sort by amount descending

            if json_output {
                #[derive(Serialize)]
                struct JsonMonthlyRow {
                    month: String,
                    dividends: String,
                    jcp: String,
                    amortization: String,
                    total: String,
                }

                #[derive(Serialize)]
                struct JsonSummary {
                    year: i32,
                    monthly: Vec<JsonMonthlyRow>,
                    totals: JsonMonthlyRow,
                    months_with_income: usize,
                    avg_per_month: String,
                }

                let monthly_rows: Vec<JsonMonthlyRow> = monthly
                    .iter()
                    .enumerate()
                    .map(|(i, m)| {
                        let total = m.dividends + m.jcp + m.amortization;
                        JsonMonthlyRow {
                            month: month_names[i].to_string(),
                            dividends: m.dividends.to_string(),
                            jcp: m.jcp.to_string(),
                            amortization: m.amortization.to_string(),
                            total: total.to_string(),
                        }
                    })
                    .collect();

                let summary = JsonSummary {
                    year: y,
                    monthly: monthly_rows,
                    totals: JsonMonthlyRow {
                        month: "TOTAL".to_string(),
                        dividends: total_dividends.to_string(),
                        jcp: total_jcp.to_string(),
                        amortization: total_amortization.to_string(),
                        total: grand_total.to_string(),
                    },
                    months_with_income,
                    avg_per_month: avg_per_month.to_string(),
                };

                println!("{}", serde_json::to_string_pretty(&summary)?);
                return Ok(());
            }

            println!(
                "\n{} Income Summary - {} (Monthly Breakdown)\n",
                "💰".cyan().bold(),
                y
            );

            #[derive(Tabled)]
            struct MonthRow {
                #[tabled(rename = "Month")]
                month: String,
                #[tabled(rename = "Dividends")]
                dividends: String,
                #[tabled(rename = "JCP")]
                jcp: String,
                #[tabled(rename = "Amort")]
                amort: String,
                #[tabled(rename = "Total")]
                total: String,
            }

            let mut rows: Vec<MonthRow> = monthly
                .iter()
                .enumerate()
                .map(|(i, m)| {
                    let total = m.dividends + m.jcp + m.amortization;
                    MonthRow {
                        month: month_names[i].to_string(),
                        dividends: if m.dividends > Decimal::ZERO {
                            format_currency(m.dividends)
                        } else {
                            "-".to_string()
                        },
                        jcp: if m.jcp > Decimal::ZERO {
                            format_currency(m.jcp)
                        } else {
                            "-".to_string()
                        },
                        amort: if m.amortization > Decimal::ZERO {
                            format_currency(m.amortization)
                        } else {
                            "-".to_string()
                        },
                        total: if total > Decimal::ZERO {
                            format_currency(total)
                        } else {
                            "-".to_string()
                        },
                    }
                })
                .collect();

            rows.push(MonthRow {
                month: "─────".to_string(),
                dividends: "───────────".to_string(),
                jcp: "───────────".to_string(),
                amort: "───────────".to_string(),
                total: "───────────".to_string(),
            });
            rows.push(MonthRow {
                month: "TOTAL".to_string(),
                dividends: format_currency(total_dividends),
                jcp: format_currency(total_jcp),
                amort: format_currency(total_amortization),
                total: format_currency(grand_total),
            });

            let table = Table::new(&rows)
                .with(Style::rounded())
                .with(Modify::new(Columns::new(1..)).with(Alignment::right()))
                .to_string();
            println!("{}", table);

            println!("\n{} Subtotals by Type:", "📊".cyan().bold());
            if total_dividends > Decimal::ZERO {
                println!(
                    "  Dividends:    {}",
                    format_currency(total_dividends).green()
                );
            }
            if total_jcp > Decimal::ZERO {
                println!("  JCP:          {}", format_currency(total_jcp).green());
            }
            if total_amortization > Decimal::ZERO {
                println!(
                    "  Amortization: {}",
                    format_currency(total_amortization).yellow()
                );
            }
            println!(
                "  {} {}",
                "Total:".bold(),
                format_currency(grand_total).green().bold()
            );

            println!("\n{} Subtotals by Asset Type:", "📊".cyan().bold());
            for (asset_type, total) in asset_type_vec {
                println!(
                    "  {:12} {}",
                    format!("{:?}:", asset_type),
                    format_currency(*total).green()
                );
            }

            println!("\n{} Statistics:", "📈".cyan().bold());
            println!("  Months with income: {}", months_with_income);
            println!(
                "  Average per month:  {}",
                format_currency(avg_per_month).cyan()
            );
            println!();
        }
        None => {
            // Yearly summary across all years
            info!("Showing income summary with yearly totals");

            let events = db::get_income_events_with_assets(&conn, None, None, None)?;

            if events.is_empty() {
                println!("\n{} No income events found.\n", "ℹ".blue().bold());
                return Ok(());
            }

            struct YearlyTotals {
                dividends: Decimal,
                jcp: Decimal,
                amortization: Decimal,
            }

            let mut yearly: BTreeMap<i32, YearlyTotals> = BTreeMap::new();

            for (event, _asset) in &events {
                let year = event.event_date.year();
                let entry = yearly.entry(year).or_insert(YearlyTotals {
                    dividends: Decimal::ZERO,
                    jcp: Decimal::ZERO,
                    amortization: Decimal::ZERO,
                });
                match event.event_type {
                    db::IncomeEventType::Dividend => entry.dividends += event.total_amount,
                    db::IncomeEventType::Jcp => entry.jcp += event.total_amount,
                    db::IncomeEventType::Amortization => entry.amortization += event.total_amount,
                }
            }

            let total_dividends: Decimal = yearly.values().map(|y| y.dividends).sum();
            let total_jcp: Decimal = yearly.values().map(|y| y.jcp).sum();
            let total_amortization: Decimal = yearly.values().map(|y| y.amortization).sum();
            let grand_total = total_dividends + total_jcp + total_amortization;

            let years_with_income = yearly.len();
            let avg_per_year = if years_with_income > 0 {
                grand_total / Decimal::from(years_with_income)
            } else {
                Decimal::ZERO
            };

            // Calculate totals by asset type
            let mut asset_type_totals: std::collections::HashMap<db::AssetType, Decimal> =
                std::collections::HashMap::new();
            for (event, asset) in &events {
                *asset_type_totals
                    .entry(asset.asset_type)
                    .or_insert(Decimal::ZERO) += event.total_amount;
            }
            let mut asset_type_vec: Vec<_> = asset_type_totals.iter().collect();
            asset_type_vec.sort_by(|a, b| b.1.cmp(a.1)); // Sort by amount descending

            if json_output {
                #[derive(Serialize)]
                struct JsonYearlyRow {
                    year: i32,
                    dividends: String,
                    jcp: String,
                    amortization: String,
                    total: String,
                }

                #[derive(Serialize)]
                struct JsonYearlySummary {
                    yearly: Vec<JsonYearlyRow>,
                    totals: JsonYearlyRow,
                    years_with_income: usize,
                    avg_per_year: String,
                }

                let yearly_rows: Vec<JsonYearlyRow> = yearly
                    .iter()
                    .map(|(yr, y)| {
                        let total = y.dividends + y.jcp + y.amortization;
                        JsonYearlyRow {
                            year: *yr,
                            dividends: y.dividends.to_string(),
                            jcp: y.jcp.to_string(),
                            amortization: y.amortization.to_string(),
                            total: total.to_string(),
                        }
                    })
                    .collect();

                let summary = JsonYearlySummary {
                    yearly: yearly_rows,
                    totals: JsonYearlyRow {
                        year: 0,
                        dividends: total_dividends.to_string(),
                        jcp: total_jcp.to_string(),
                        amortization: total_amortization.to_string(),
                        total: grand_total.to_string(),
                    },
                    years_with_income,
                    avg_per_year: avg_per_year.to_string(),
                };

                println!("{}", serde_json::to_string_pretty(&summary)?);
                return Ok(());
            }

            println!(
                "\n{} Income Summary (Yearly Breakdown)\n",
                "💰".cyan().bold()
            );

            #[derive(Tabled)]
            struct YearRow {
                #[tabled(rename = "Year")]
                year: String,
                #[tabled(rename = "Dividends")]
                dividends: String,
                #[tabled(rename = "JCP")]
                jcp: String,
                #[tabled(rename = "Amort")]
                amort: String,
                #[tabled(rename = "Total")]
                total: String,
            }

            let mut rows: Vec<YearRow> = yearly
                .iter()
                .map(|(yr, y)| {
                    let total = y.dividends + y.jcp + y.amortization;
                    YearRow {
                        year: yr.to_string(),
                        dividends: if y.dividends > Decimal::ZERO {
                            format_currency(y.dividends)
                        } else {
                            "-".to_string()
                        },
                        jcp: if y.jcp > Decimal::ZERO {
                            format_currency(y.jcp)
                        } else {
                            "-".to_string()
                        },
                        amort: if y.amortization > Decimal::ZERO {
                            format_currency(y.amortization)
                        } else {
                            "-".to_string()
                        },
                        total: format_currency(total),
                    }
                })
                .collect();

            rows.push(YearRow {
                year: "─────".to_string(),
                dividends: "───────────".to_string(),
                jcp: "───────────".to_string(),
                amort: "───────────".to_string(),
                total: "───────────".to_string(),
            });
            rows.push(YearRow {
                year: "TOTAL".to_string(),
                dividends: format_currency(total_dividends),
                jcp: format_currency(total_jcp),
                amort: format_currency(total_amortization),
                total: format_currency(grand_total),
            });

            let table = Table::new(&rows)
                .with(Style::rounded())
                .with(Modify::new(Columns::new(1..)).with(Alignment::right()))
                .to_string();
            println!("{}", table);

            println!("\n{} Subtotals by Type:", "📊".cyan().bold());
            if total_dividends > Decimal::ZERO {
                println!(
                    "  Dividends:    {}",
                    format_currency(total_dividends).green()
                );
            }
            if total_jcp > Decimal::ZERO {
                println!("  JCP:          {}", format_currency(total_jcp).green());
            }
            if total_amortization > Decimal::ZERO {
                println!(
                    "  Amortization: {}",
                    format_currency(total_amortization).yellow()
                );
            }
            println!(
                "  {} {}",
                "Total:".bold(),
                format_currency(grand_total).green().bold()
            );

            println!("\n{} Subtotals by Asset Type:", "📊".cyan().bold());
            for (asset_type, total) in asset_type_vec {
                println!(
                    "  {:12} {}",
                    format!("{:?}:", asset_type),
                    format_currency(*total).green()
                );
            }

            println!("\n{} Statistics:", "📈".cyan().bold());
            println!("  Years with income:  {}", years_with_income);
            println!(
                "  Average per year:   {}",
                format_currency(avg_per_year).cyan()
            );
            println!();
        }
    }

    Ok(())
}

// Snapshot commands are intentionally internal-only; no public dispatcher.

struct TaxProgressPrinter {
    spinner: Spinner,
    in_place: bool,
    in_progress: bool,
    from_year: Option<i32>,
    target_year: Option<i32>,
    total_years: usize,
    completed_years: usize,
}

impl TaxProgressPrinter {
    fn new(in_place: bool) -> Self {
        Self {
            spinner: Spinner::new(),
            in_place,
            in_progress: false,
            from_year: None,
            target_year: None,
            total_years: 0,
            completed_years: 0,
        }
    }

    fn render_line(&mut self, text: &str) {
        if self.in_place {
            print!("\r\x1b[2K{} {}", self.spinner.tick(), text);
            let _ = stdout().flush();
        } else {
            println!("{} {}", self.spinner.tick(), text);
        }
    }

    fn finish_line(&mut self) {
        if self.in_place {
            println!();
            let _ = stdout().flush();
        }
    }

    fn on_event(&mut self, event: tax::ReportProgress) {
        match event {
            tax::ReportProgress::Start { target_year, .. } => {
                self.target_year = Some(target_year);
            }
            tax::ReportProgress::RecomputeStart { from_year } => {
                self.from_year = Some(from_year);
                self.in_progress = true;
                self.completed_years = 0;
                self.total_years = self
                    .target_year
                    .map(|t| (t - from_year + 1).max(1) as usize)
                    .unwrap_or(1);
                self.render_line(&format!(
                    "↻ Recomputing snapshots {}/{} (starting {})",
                    self.completed_years, self.total_years, from_year
                ));
            }
            tax::ReportProgress::RecomputedYear { year } => {
                if self.in_progress {
                    self.completed_years = (self.completed_years + 1).min(self.total_years);
                    let from = self.from_year.unwrap_or(year);
                    if Some(year) == self.target_year {
                        // Finalize with a clean success line
                        if self.in_place {
                            print!("\r\x1b[2K");
                        }
                        println!("✓ Snapshots updated {}→{}", from, year);
                        let _ = stdout().flush();
                        self.in_progress = false;
                    } else {
                        self.render_line(&format!(
                            "↻ Recomputing snapshots {}/{} (year {})",
                            self.completed_years, self.total_years, year
                        ));
                    }
                }
            }
            tax::ReportProgress::TargetCacheHit { year } => {
                self.render_line(&format!("✓ Cache hit for {}; using cached carry", year));
                self.finish_line();
            }
            _ => {}
        }
    }
}

async fn dispatch_prices(action: crate::commands::PricesAction, _json_output: bool) -> Result<()> {
    use crate::commands::PricesAction;
    use crate::importers::b3_cotahist;

    match action {
        PricesAction::ImportB3 { year, no_cache } => {
            info!("Importing B3 COTAHIST for year {}", year);

            // Initialize database
            db::init_database(None)?;
            let mut conn = db::open_db(None)?;

            println!("📥 Importing B3 COTAHIST for year {}...", year);

            // Create progress callback
            let callback = |progress: &b3_cotahist::DownloadProgress| {
                use b3_cotahist::{DisplayMode, DownloadStage};

                let stage_msg = match progress.stage {
                    DownloadStage::Downloading => {
                        format!("📥 Downloading COTAHIST {} ZIP", progress.year)
                    }
                    DownloadStage::Decompressing => {
                        format!("📦 Decompressing COTAHIST {}", progress.year)
                    }
                    DownloadStage::Parsing => {
                        if let Some(total) = progress.total_records {
                            if progress.records_processed.is_multiple_of(50000)
                                || progress.records_processed == total
                            {
                                let pct = (progress.records_processed as f64 / total as f64 * 100.0)
                                    as usize;
                                format!(
                                    "📝 Parsing COTAHIST {} ({}/{}  {}%)",
                                    progress.year, progress.records_processed, total, pct
                                )
                            } else {
                                return; // Don't print intermediate parsing progress
                            }
                        } else {
                            format!("📝 Parsing COTAHIST {}", progress.year)
                        }
                    }
                    DownloadStage::Complete => {
                        return; // Don't print on complete (will be printed after import)
                    }
                };

                // Respect display_mode: spinner-only updates don't print
                if progress.display_mode == DisplayMode::Persist {
                    println!("{}", stage_msg);
                }
            };

            // Import the year
            match b3_cotahist::import_cotahist_year(&mut conn, year, no_cache, Some(&callback)) {
                Ok(count) => {
                    if count > 0 {
                        println!(
                            "{} Imported {} new price records for year {}",
                            "✓".green(),
                            count,
                            year
                        );
                    } else {
                        println!(
                            "{} All COTAHIST {} prices already in database (cache hit)",
                            "✓".green(),
                            year
                        );
                    }
                }
                Err(e) => {
                    eprintln!("{} Failed to import COTAHIST {}: {}", "✗".red(), year, e);
                    return Err(e);
                }
            }

            Ok(())
        }
        PricesAction::ImportB3File { path } => {
            info!("Importing COTAHIST from local file: {}", path);

            db::init_database(None)?;
            let mut conn = db::open_db(None)?;

            match b3_cotahist::import_cotahist_from_file(&mut conn, &path) {
                Ok(count) => {
                    if count > 0 {
                        println!(
                            "{} Imported {} new price records from {}",
                            "✓".green(),
                            count,
                            path
                        );
                    } else {
                        println!(
                            "{} No new prices inserted from {} (possible duplicates)",
                            "✓".green(),
                            path
                        );
                    }
                }
                Err(e) => {
                    eprintln!(
                        "{} Failed to import COTAHIST file {}: {}",
                        "✗".red(),
                        path,
                        e
                    );
                    return Err(e);
                }
            }

            Ok(())
        }
        PricesAction::ClearCache { year } => {
            match year {
                Some(y) => {
                    b3_cotahist::clear_cache(Some(y))?;
                    println!("{} Cleared COTAHIST cache for year {}", "✓".green(), y);
                }
                None => {
                    b3_cotahist::clear_cache(None)?;
                    println!("{} Cleared all COTAHIST cache", "✓".green());
                }
            }
            Ok(())
        }
    }
}

/// Parse a progress message to extract the completion count.
/// Messages like "TICKER → R$ XX.XX (N/M)" return Some(N).
/// Returns None if the message doesn't match the expected format.
fn parse_progress_count(msg: &str) -> Option<usize> {
    if !msg.contains("→") {
        return None;
    }
    let paren_start = msg.rfind('(')?;
    let slash_offset = msg[paren_start..].find('/')?;
    msg[paren_start + 1..paren_start + slash_offset]
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_dispatch_help_command() {
        let result = dispatch_command(Command::Help, false).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_dispatch_exit_command() {
        // We can't really test exit, but we can check it would be called
        // In reality, this would exit the process
    }

    #[tokio::test]
    async fn test_dispatch_import() {
        let result = dispatch_command(
            Command::Import {
                path: "test.xlsx".to_string(),
                dry_run: false,
            },
            false,
        )
        .await;
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_progress_count_valid() {
        assert_eq!(parse_progress_count("PETR4 → R$ 35.50 (1/35)"), Some(1));
        assert_eq!(parse_progress_count("HGLG11 → R$ 156.99 (15/35)"), Some(15));
        assert_eq!(
            parse_progress_count("VALE3 → R$ 58.20 (100/100)"),
            Some(100)
        );
    }

    #[test]
    fn test_parse_progress_count_failed() {
        assert_eq!(parse_progress_count("PETR4 → failed (5/35)"), Some(5));
    }

    #[test]
    fn test_parse_progress_count_no_arrow() {
        assert_eq!(parse_progress_count("Checking 35 assets..."), None);
        assert_eq!(parse_progress_count("✓ All prices are up to date!"), None);
    }

    #[test]
    fn test_parse_progress_count_no_parens() {
        assert_eq!(parse_progress_count("PETR4 → R$ 35.50"), None);
    }
}
