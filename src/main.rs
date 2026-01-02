mod cli;
mod db;
mod importers;
mod pricing;
mod corporate_actions;
mod tax;
mod reports;
mod utils;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Commands, PortfolioCommands, PriceCommands, TaxCommands, ActionCommands};
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Import { file, dry_run } => {
            handle_import(&file, dry_run).await
        }

        Commands::Portfolio { action } => match action {
            PortfolioCommands::Show { asset_type } => {
                handle_portfolio_show(asset_type.as_deref()).await
            }
            PortfolioCommands::Performance { period } => {
                println!("Showing performance for period: {}", period);
                // TODO: Implement performance metrics
                Ok(())
            }
        },

        Commands::Prices { action } => match action {
            PriceCommands::Update => {
                handle_price_update().await
            }
            PriceCommands::History { ticker, from, to } => {
                handle_price_history(&ticker, &from, &to).await
            }
        },

        Commands::Tax { action } => match action {
            TaxCommands::Calculate { month } => {
                handle_tax_calculate(&month).await
            }
            TaxCommands::Report { year } => {
                handle_tax_report(year).await
            }
            TaxCommands::Summary { year } => {
                handle_tax_summary(year).await
            }
        },

        Commands::Actions { action } => match action {
            ActionCommands::Update => {
                handle_actions_update().await
            }
            ActionCommands::List { ticker } => {
                handle_actions_list(ticker.as_deref()).await
            }
        },
    }
}

/// Handle import command
async fn handle_import(file_path: &str, dry_run: bool) -> Result<()> {
    use colored::Colorize;
    use tabled::{Table, Tabled, settings::Style};

    info!("Importing transactions from: {}", file_path);

    // Parse the file
    let raw_transactions = importers::import_file(file_path)?;

    println!("\n{} Found {} transactions\n", "✓".green().bold(), raw_transactions.len());

    // Display preview
    #[derive(Tabled)]
    struct TransactionPreview {
        #[tabled(rename = "Date")]
        date: String,
        #[tabled(rename = "Ticker")]
        ticker: String,
        #[tabled(rename = "Type")]
        tx_type: String,
        #[tabled(rename = "Quantity")]
        quantity: String,
        #[tabled(rename = "Price")]
        price: String,
        #[tabled(rename = "Total")]
        total: String,
    }

    let preview: Vec<TransactionPreview> = raw_transactions
        .iter()
        .take(10)
        .map(|tx| TransactionPreview {
            date: tx.trade_date.format("%d/%m/%Y").to_string(),
            ticker: tx.ticker.clone(),
            tx_type: tx.transaction_type.clone(),
            quantity: tx.quantity.to_string(),
            price: format!("R$ {}", tx.price),
            total: format!("R$ {}", tx.total),
        })
        .collect();

    let table = Table::new(preview).with(Style::rounded()).to_string();
    println!("{}", table);

    if raw_transactions.len() > 10 {
        println!("\n... and {} more transactions", raw_transactions.len() - 10);
    }

    if dry_run {
        println!("\n{} Dry run - no changes saved", "ℹ".blue().bold());
        return Ok(());
    }

    // Initialize database if needed
    db::init_database(None)?;

    // Open connection
    let conn = db::open_db(None)?;

    // Import transactions
    let mut imported = 0;
    let mut skipped = 0;
    let mut errors = 0;

    for raw_tx in &raw_transactions {
        // Detect asset type from ticker
        let asset_type = db::AssetType::detect_from_ticker(&raw_tx.ticker)
            .unwrap_or(db::AssetType::Stock);

        // Upsert asset
        let asset_id = match db::upsert_asset(&conn, &raw_tx.ticker, &asset_type, None) {
            Ok(id) => id,
            Err(e) => {
                eprintln!("Error upserting asset {}: {}", raw_tx.ticker, e);
                errors += 1;
                continue;
            }
        };

        // Convert to Transaction model
        let transaction = match raw_tx.to_transaction(asset_id) {
            Ok(tx) => tx,
            Err(e) => {
                eprintln!("Error converting transaction for {}: {}", raw_tx.ticker, e);
                errors += 1;
                continue;
            }
        };

        // Check for duplicates
        if db::transaction_exists(
            &conn,
            asset_id,
            &transaction.trade_date,
            &transaction.transaction_type,
            &transaction.quantity,
        )? {
            skipped += 1;
            continue;
        }

        // Insert transaction
        match db::insert_transaction(&conn, &transaction) {
            Ok(_) => imported += 1,
            Err(e) => {
                eprintln!("Error inserting transaction: {}", e);
                errors += 1;
            }
        }
    }

    println!("\n{} Import complete!", "✓".green().bold());
    println!("  Imported: {}", imported.to_string().green());
    if skipped > 0 {
        println!("  Skipped (duplicates): {}", skipped.to_string().yellow());
    }
    if errors > 0 {
        println!("  Errors: {}", errors.to_string().red());
    }

    Ok(())
}

/// Handle price update command
async fn handle_price_update() -> Result<()> {
    use colored::Colorize;
    use pricing::PriceFetcher;

    info!("Updating all asset prices");

    // Initialize database
    db::init_database(None)?;
    let conn = db::open_db(None)?;

    // Get all assets
    let assets = db::get_all_assets(&conn)?;

    if assets.is_empty() {
        println!("{} No assets found in database", "ℹ".blue().bold());
        println!("Import transactions first using: interest import <file>");
        return Ok(());
    }

    println!("\n{} Updating prices for {} assets\n", "→".cyan().bold(), assets.len());

    let fetcher = PriceFetcher::new();
    let mut updated = 0;
    let mut errors = 0;

    for asset in &assets {
        print!("  {} {}... ", asset.ticker, "→".cyan());

        match fetcher.fetch_price(&asset.ticker).await {
            Ok(price) => {
                // Store price in database
                let price_history = db::PriceHistory {
                    id: None,
                    asset_id: asset.id.unwrap(),
                    price_date: chrono::Utc::now().date_naive(),
                    close_price: price,
                    open_price: None,
                    high_price: None,
                    low_price: None,
                    volume: None,
                    source: "YAHOO/BRAPI".to_string(),
                    created_at: chrono::Utc::now(),
                };

                match db::insert_price_history(&conn, &price_history) {
                    Ok(_) => {
                        println!("{} R$ {}", "✓".green(), price);
                        updated += 1;
                    }
                    Err(e) => {
                        println!("{} {}", "✗".red(), e);
                        errors += 1;
                    }
                }
            }
            Err(e) => {
                println!("{} {}", "✗".red(), e);
                errors += 1;
            }
        }
    }

    println!("\n{} Price update complete!", "✓".green().bold());
    println!("  Updated: {}", updated.to_string().green());
    if errors > 0 {
        println!("  Errors: {}", errors.to_string().red());
    }

    Ok(())
}

/// Handle historical price fetching
async fn handle_price_history(ticker: &str, from: &str, to: &str) -> Result<()> {
    use colored::Colorize;
    use chrono::NaiveDate;
    use tabled::{Table, Tabled, settings::Style};
    use anyhow::Context;

    info!("Fetching historical prices for {} from {} to {}", ticker, from, to);

    let from_date = NaiveDate::parse_from_str(from, "%Y-%m-%d")
        .context("Invalid from date. Use YYYY-MM-DD format")?;
    let to_date = NaiveDate::parse_from_str(to, "%Y-%m-%d")
        .context("Invalid to date. Use YYYY-MM-DD format")?;

    println!("\n{} Fetching historical prices for {}", "→".cyan().bold(), ticker);

    let prices = pricing::yahoo::fetch_historical_prices(ticker, from_date, to_date).await?;

    if prices.is_empty() {
        println!("{} No price data found", "ℹ".blue().bold());
        return Ok(());
    }

    // Display prices in table
    #[derive(Tabled)]
    struct PriceRow {
        #[tabled(rename = "Date")]
        date: String,
        #[tabled(rename = "Open")]
        open: String,
        #[tabled(rename = "High")]
        high: String,
        #[tabled(rename = "Low")]
        low: String,
        #[tabled(rename = "Close")]
        close: String,
        #[tabled(rename = "Volume")]
        volume: String,
    }

    let rows: Vec<PriceRow> = prices
        .iter()
        .map(|p| PriceRow {
            date: p.date.format("%Y-%m-%d").to_string(),
            open: p.open.as_ref().map(|o| format!("R$ {:.2}", o)).unwrap_or_else(|| "-".to_string()),
            high: p.high.as_ref().map(|h| format!("R$ {:.2}", h)).unwrap_or_else(|| "-".to_string()),
            low: p.low.as_ref().map(|l| format!("R$ {:.2}", l)).unwrap_or_else(|| "-".to_string()),
            close: format!("R$ {:.2}", p.close),
            volume: p.volume.map(|v| v.to_string()).unwrap_or_else(|| "-".to_string()),
        })
        .collect();

    let table = Table::new(rows).with(Style::rounded()).to_string();
    println!("\n{}", table);
    println!("\n{} Total: {} price points", "✓".green().bold(), prices.len());

    Ok(())
}

/// Handle corporate actions update
async fn handle_actions_update() -> Result<()> {
    use colored::Colorize;

    info!("Updating corporate actions");

    // Initialize database
    db::init_database(None)?;
    let conn = db::open_db(None)?;

    // Get all assets
    let assets = db::get_all_assets(&conn)?;

    if assets.is_empty() {
        println!("{} No assets found in database", "ℹ".blue().bold());
        return Ok(());
    }

    println!("\n{} Fetching corporate actions for {} assets\n", "→".cyan().bold(), assets.len());

    let mut total_actions = 0;
    let mut total_events = 0;

    for asset in &assets {
        print!("  {} {}... ", asset.ticker, "→".cyan());

        match pricing::brapi::fetch_quote(&asset.ticker, true).await {
            Ok((_price, actions_opt, events_opt)) => {
                let mut count = 0;

                // Store corporate actions
                if let Some(actions) = actions_opt {
                    for brapi_action in actions {
                        // Parse ratio from factor string (e.g., "1:2", "10%")
                        let (ratio_from, ratio_to) = parse_factor(&brapi_action.factor);

                        let action = db::CorporateAction {
                            id: None,
                            asset_id: asset.id.unwrap(),
                            action_type: brapi_action.action_type,
                            event_date: brapi_action.approved_date,
                            ex_date: brapi_action.ex_date,
                            ratio_from,
                            ratio_to,
                            applied: false,
                            source: "BRAPI".to_string(),
                            notes: brapi_action.remarks,
                            created_at: chrono::Utc::now(),
                        };

                        // Check for duplicates
                        if !db::corporate_action_exists(&conn, asset.id.unwrap(), &action.ex_date, &action.action_type)? {
                            db::insert_corporate_action(&conn, &action)?;
                            count += 1;
                        }
                    }
                    total_actions += count;
                }

                // Store income events
                if let Some(events) = events_opt {
                    for brapi_event in events {
                        let event_type = db::IncomeEventType::from_str(&brapi_event.event_type)
                            .unwrap_or(db::IncomeEventType::Dividend);

                        let event = db::IncomeEvent {
                            id: None,
                            asset_id: asset.id.unwrap(),
                            event_date: brapi_event.payment_date,
                            ex_date: brapi_event.ex_date,
                            event_type,
                            amount_per_quota: brapi_event.amount,
                            total_amount: brapi_event.amount,  // Will be calculated based on holdings
                            withholding_tax: rust_decimal::Decimal::ZERO,
                            is_quota_pre_2026: None,
                            source: "BRAPI".to_string(),
                            notes: brapi_event.remarks,
                            created_at: chrono::Utc::now(),
                        };

                        db::insert_income_event(&conn, &event)?;
                        total_events += 1;
                    }
                }

                if count > 0 {
                    println!("{} {} actions", "✓".green(), count);
                } else {
                    println!("{}", "✓".green());
                }
            }
            Err(e) => {
                println!("{} {}", "✗".red(), e);
            }
        }
    }

    println!("\n{} Corporate actions update complete!", "✓".green().bold());
    println!("  Actions: {}", total_actions.to_string().green());
    println!("  Events: {}", total_events.to_string().green());

    Ok(())
}

/// Handle listing corporate actions
async fn handle_actions_list(ticker: Option<&str>) -> Result<()> {
    use colored::Colorize;

    println!("{} Listing corporate actions is not yet implemented", "ℹ".blue().bold());
    if let Some(t) = ticker {
        println!("  Filter: {}", t);
    }

    Ok(())
}

/// Handle portfolio show command
async fn handle_portfolio_show(asset_type: Option<&str>) -> Result<()> {
    use colored::Colorize;
    use tabled::{Table, Tabled, settings::Style};
    use anyhow::Context;
    use std::str::FromStr;

    info!("Generating portfolio report");

    // Initialize database
    db::init_database(None)?;
    let conn = db::open_db(None)?;

    // Parse asset type filter if provided
    let asset_type_filter = if let Some(type_str) = asset_type {
        Some(db::AssetType::from_str(type_str)
            .context(format!("Invalid asset type: {}", type_str))?)
    } else {
        None
    };

    // Calculate portfolio
    let report = reports::calculate_portfolio(&conn, asset_type_filter.as_ref())?;

    if report.positions.is_empty() {
        println!("{} No positions found", "ℹ".blue().bold());
        println!("Import transactions first using: interest import <file>");
        return Ok(());
    }

    // Display header
    if let Some(ref filter) = asset_type_filter {
        let filter_name: &db::AssetType = filter;
        println!("\n{} Portfolio - {} only\n", "📊".cyan().bold(), filter_name.as_str().to_uppercase());
    } else {
        println!("\n{} Complete Portfolio\n", "📊".cyan().bold());
    }

    // Display positions table
    #[derive(Tabled)]
    struct PositionRow {
        #[tabled(rename = "Ticker")]
        ticker: String,
        #[tabled(rename = "Type")]
        asset_type: String,
        #[tabled(rename = "Quantity")]
        quantity: String,
        #[tabled(rename = "Avg Cost")]
        avg_cost: String,
        #[tabled(rename = "Total Cost")]
        total_cost: String,
        #[tabled(rename = "Price")]
        price: String,
        #[tabled(rename = "Value")]
        value: String,
        #[tabled(rename = "P&L")]
        pl: String,
        #[tabled(rename = "P&L %")]
        pl_pct: String,
    }

    let rows: Vec<PositionRow> = report
        .positions
        .iter()
        .map(|p| {
            let pl_str = if let Some(pl) = p.unrealized_pl {
                if pl >= rust_decimal::Decimal::ZERO {
                    format!("R$ {:.2}", pl).green().to_string()
                } else {
                    format!("R$ {:.2}", pl).red().to_string()
                }
            } else {
                "-".to_string()
            };

            let pl_pct_str = if let Some(pl_pct) = p.unrealized_pl_pct {
                if pl_pct >= rust_decimal::Decimal::ZERO {
                    format!("{:.2}%", pl_pct).green().to_string()
                } else {
                    format!("{:.2}%", pl_pct).red().to_string()
                }
            } else {
                "-".to_string()
            };

            PositionRow {
                ticker: p.asset.ticker.clone(),
                asset_type: p.asset.asset_type.as_str().to_string(),
                quantity: format!("{:.2}", p.quantity),
                avg_cost: format!("R$ {:.2}", p.average_cost),
                total_cost: format!("R$ {:.2}", p.total_cost),
                price: p.current_price
                    .map(|pr| format!("R$ {:.2}", pr))
                    .unwrap_or_else(|| "-".to_string()),
                value: p.current_value
                    .map(|v| format!("R$ {:.2}", v))
                    .unwrap_or_else(|| "-".to_string()),
                pl: pl_str,
                pl_pct: pl_pct_str,
            }
        })
        .collect();

    let table = Table::new(rows).with(Style::rounded()).to_string();
    println!("{}", table);

    // Display summary
    println!("\n{} Summary", "📈".cyan().bold());
    println!("  Total Cost:  {}", format!("R$ {:.2}", report.total_cost).cyan());
    println!("  Total Value: {}", format!("R$ {:.2}", report.total_value).cyan());

    if report.total_pl >= rust_decimal::Decimal::ZERO {
        println!("  Total P&L:   {} ({})",
            format!("R$ {:.2}", report.total_pl).green().bold(),
            format!("{:.2}%", report.total_pl_pct).green().bold()
        );
    } else {
        println!("  Total P&L:   {} ({})",
            format!("R$ {:.2}", report.total_pl).red().bold(),
            format!("{:.2}%", report.total_pl_pct).red().bold()
        );
    }

    // Display asset allocation if showing full portfolio
    if asset_type_filter.is_none() {
        let allocation = reports::calculate_allocation(&report);

        if allocation.len() > 1 {
            println!("\n{} Asset Allocation", "🎯".cyan().bold());

            let mut alloc_vec: Vec<_> = allocation.iter().collect();
            alloc_vec.sort_by(|a, b| b.1.0.cmp(&a.1.0));

            for (asset_type, (value, pct)) in alloc_vec {
                let type_ref: &db::AssetType = asset_type;
                println!("  {}: {} ({:.2}%)",
                    type_ref.as_str().to_uppercase(),
                    format!("R$ {:.2}", value).cyan(),
                    pct
                );
            }
        }
    }

    println!();
    Ok(())
}

/// Handle tax calculation for a specific month
async fn handle_tax_calculate(month_str: &str) -> Result<()> {
    use colored::Colorize;
    use tabled::{Table, Tabled, settings::Style};
    use anyhow::Context;

    info!("Calculating swing trade tax for {}", month_str);

    // Parse month string (MM/YYYY)
    let parts: Vec<&str> = month_str.split('/').collect();
    if parts.len() != 2 {
        return Err(anyhow::anyhow!("Invalid month format. Use MM/YYYY (e.g., 01/2025)"));
    }

    let month: u32 = parts[0].parse().context("Invalid month number")?;
    let year: i32 = parts[1].parse().context("Invalid year")?;

    if month < 1 || month > 12 {
        return Err(anyhow::anyhow!("Month must be between 01 and 12"));
    }

    // Initialize database
    db::init_database(None)?;
    let conn = db::open_db(None)?;

    // Calculate monthly tax
    let calculations = tax::calculate_monthly_tax(&conn, year, month)?;

    if calculations.is_empty() {
        println!("\n{} No sales found for {}/{}\n", "ℹ".blue().bold(), month, year);
        return Ok(());
    }

    println!("\n{} Swing Trade Tax Calculation - {}/{}\n", "💰".cyan().bold(), month, year);

    // Display results by asset type
    for calc in &calculations {
        println!("{} {}", "Asset Type:".bold(), calc.asset_type.as_str().to_uppercase());
        println!("  Total Sales:      {}", format!("R$ {:.2}", calc.total_sales).cyan());
        println!("  Total Cost Basis: {}", format!("R$ {:.2}", calc.total_cost_basis).cyan());
        println!("  Gross Profit:     {}", format!("R$ {:.2}", calc.total_profit).green());
        println!("  Gross Loss:       {}", format!("R$ {:.2}", calc.total_loss).red());

        let net_str = if calc.net_profit >= rust_decimal::Decimal::ZERO {
            format!("R$ {:.2}", calc.net_profit).green()
        } else {
            format!("R$ {:.2}", calc.net_profit).red()
        };
        println!("  Net P&L:          {}", net_str);

        if calc.exemption_applied > rust_decimal::Decimal::ZERO {
            println!("  Exemption:        {} (sales under R$20,000)",
                format!("R$ {:.2}", calc.exemption_applied).yellow().bold()
            );
        }

        if calc.taxable_amount > rust_decimal::Decimal::ZERO {
            println!("  Taxable Amount:   {}", format!("R$ {:.2}", calc.taxable_amount).yellow());
            println!("  Tax Rate:         {}", "15%".yellow());
            println!("  {} {}",
                "Tax Due:".bold(),
                format!("R$ {:.2}", calc.tax_due).red().bold()
            );
        } else if calc.net_profit < rust_decimal::Decimal::ZERO {
            println!("  {} Loss to carry forward",
                format!("R$ {:.2}", calc.net_profit.abs()).yellow().bold()
            );
        } else {
            println!("  {} No tax due (exempt)", "Tax Due:".bold().green());
        }

        println!();
    }

    // Summary
    let total_tax: rust_decimal::Decimal = calculations.iter()
        .map(|c| c.tax_due)
        .sum();

    if total_tax > rust_decimal::Decimal::ZERO {
        println!("{} Total Tax Due for {}/{}: {}\n",
            "📋".cyan().bold(),
            month,
            year,
            format!("R$ {:.2}", total_tax).red().bold()
        );
        println!("{} Payment due by last business day of {}/{}\n",
            "⏰".yellow(),
            if month == 12 { 1 } else { month + 1 },
            if month == 12 { year + 1 } else { year }
        );
    }

    Ok(())
}

/// Handle IRPF annual report generation
async fn handle_tax_report(year: i32) -> Result<()> {
    use colored::Colorize;

    info!("Generating IRPF annual report for {}", year);

    // Initialize database
    db::init_database(None)?;
    let conn = db::open_db(None)?;

    // Generate report
    let report = tax::generate_annual_report(&conn, year)?;

    if report.monthly_summaries.is_empty() {
        println!("\n{} No transactions found for year {}\n", "ℹ".blue().bold(), year);
        return Ok(());
    }

    println!("\n{} Annual IRPF Tax Report - {}\n", "📊".cyan().bold(), year);

    // Monthly breakdown
    println!("{}", "Monthly Summary:".bold());
    for summary in &report.monthly_summaries {
        println!("\n  {} ({}):", summary.month_name.bold(), summary.month);
        println!("    Sales:  {}", format!("R$ {:.2}", summary.total_sales).cyan());
        println!("    Profit: {}", format!("R$ {:.2}", summary.total_profit).green());
        println!("    Loss:   {}", format!("R$ {:.2}", summary.total_loss).red());
        println!("    Tax:    {}", format!("R$ {:.2}", summary.tax_due).yellow());
    }

    // Annual totals
    println!("\n{} Annual Totals:", "📈".cyan().bold());
    println!("  Total Sales:  {}", format!("R$ {:.2}", report.annual_total_sales).cyan());
    println!("  Total Profit: {}", format!("R$ {:.2}", report.annual_total_profit).green());
    println!("  Total Loss:   {}", format!("R$ {:.2}", report.annual_total_loss).red());
    println!("  {} {}\n",
        "Total Tax:".bold(),
        format!("R$ {:.2}", report.annual_total_tax).yellow().bold()
    );

    // Losses to carry forward
    if !report.losses_to_carry_forward.is_empty() {
        println!("{} Losses to Carry Forward:", "📋".yellow().bold());
        for (asset_type, loss) in &report.losses_to_carry_forward {
            println!("  {}: {}",
                asset_type.as_str().to_uppercase(),
                format!("R$ {:.2}", loss).yellow()
            );
        }
        println!();
    }

    // Export to CSV
    let csv_content = tax::irpf::export_to_csv(&report);
    let csv_path = format!("irpf_report_{}.csv", year);
    std::fs::write(&csv_path, csv_content)?;

    println!("{} Report exported to: {}\n", "✓".green().bold(), csv_path);

    Ok(())
}

/// Handle tax summary display
async fn handle_tax_summary(year: i32) -> Result<()> {
    use colored::Colorize;
    use tabled::{Table, Tabled, settings::Style};

    info!("Generating tax summary for {}", year);

    // Initialize database
    db::init_database(None)?;
    let conn = db::open_db(None)?;

    // Generate report
    let report = tax::generate_annual_report(&conn, year)?;

    if report.monthly_summaries.is_empty() {
        println!("\n{} No transactions found for year {}\n", "ℹ".blue().bold(), year);
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
            sales: format!("R$ {:.2}", s.total_sales),
            profit: format!("R$ {:.2}", s.total_profit),
            loss: format!("R$ {:.2}", s.total_loss),
            tax: format!("R$ {:.2}", s.tax_due),
        })
        .collect();

    let table = Table::new(rows).with(Style::rounded()).to_string();
    println!("{}", table);

    // Annual summary
    println!("\n{} Annual Total", "📈".cyan().bold());
    println!("  Sales:  {}", format!("R$ {:.2}", report.annual_total_sales).cyan());
    println!("  Profit: {}", format!("R$ {:.2}", report.annual_total_profit).green());
    println!("  Loss:   {}", format!("R$ {:.2}", report.annual_total_loss).red());
    println!("  {} {}\n",
        "Tax:".bold(),
        format!("R$ {:.2}", report.annual_total_tax).yellow().bold()
    );

    Ok(())
}

/// Parse factor string into ratio (from, to)
/// Examples: "1:2" -> (1, 2), "10%" -> (100, 110), "2:1" -> (2, 1)
fn parse_factor(factor: &str) -> (i32, i32) {
    if let Some((from, to)) = factor.split_once(':') {
        let from_val = from.trim().parse::<i32>().unwrap_or(1);
        let to_val = to.trim().parse::<i32>().unwrap_or(1);
        (from_val, to_val)
    } else if factor.contains('%') {
        // Percentage bonus: "10%" means 100:110
        let pct = factor.replace('%', "").trim().parse::<f64>().unwrap_or(0.0);
        let to_val = (100.0 + pct) as i32;
        (100, to_val)
    } else {
        (1, 1)
    }
}
