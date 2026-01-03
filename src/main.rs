mod cli;
mod db;
mod importers;
mod pricing;
mod corporate_actions;
mod tax;
mod reports;
mod utils;
mod scraping;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Commands, PortfolioCommands, PriceCommands, TaxCommands, ActionCommands, TransactionCommands};
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
            ActionCommands::Add {
                ticker,
                action_type,
                ratio,
                date,
                notes,
            } => {
                handle_action_add(
                    &ticker,
                    &action_type,
                    &ratio,
                    &date,
                    notes.as_deref(),
                ).await
            }
            ActionCommands::Scrape { ticker, url, name, save } => {
                handle_action_scrape(
                    &ticker,
                    url.as_deref(),
                    name.as_deref(),
                    save,
                ).await
            }
            ActionCommands::Update => {
                handle_actions_update().await
            }
            ActionCommands::List { ticker } => {
                handle_actions_list(ticker.as_deref()).await
            }
            ActionCommands::Apply { ticker } => {
                handle_action_apply(ticker.as_deref()).await
            }
            ActionCommands::Delete { id } => {
                handle_action_delete(id).await
            }
            ActionCommands::Edit {
                id,
                action_type,
                ratio,
                date,
                notes,
            } => {
                handle_action_edit(
                    id,
                    action_type.as_deref(),
                    ratio.as_deref(),
                    date.as_deref(),
                    notes.as_deref(),
                ).await
            }
        },

        Commands::Transactions { action } => match action {
            TransactionCommands::Add {
                ticker,
                transaction_type,
                quantity,
                price,
                date,
                fees,
                notes,
            } => {
                handle_transaction_add(
                    &ticker,
                    &transaction_type,
                    &quantity,
                    &price,
                    &date,
                    &fees,
                    notes.as_deref(),
                ).await
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

/// Handle manual corporate action add command
async fn handle_action_add(
    ticker: &str,
    action_type_str: &str,
    ratio_str: &str,
    date_str: &str,
    notes: Option<&str>,
) -> Result<()> {
    use colored::Colorize;
    use chrono::NaiveDate;
    use anyhow::Context;

    info!("Adding manual corporate action for {}", ticker);

    // Parse action type
    let action_type = match action_type_str.to_uppercase().as_str() {
        "SPLIT" => db::CorporateActionType::Split,
        "REVERSE-SPLIT" => db::CorporateActionType::ReverseSplit,
        "BONUS" => db::CorporateActionType::Bonus,
        _ => return Err(anyhow::anyhow!("Action type must be 'split', 'reverse-split', or 'bonus'")),
    };

    // Parse ratio (from:to format, e.g., "1:2" or "10:1")
    let ratio_parts: Vec<&str> = ratio_str.split(':').collect();
    if ratio_parts.len() != 2 {
        return Err(anyhow::anyhow!("Ratio must be in format 'from:to' (e.g., '1:2', '10:1')"));
    }

    let ratio_from: i32 = ratio_parts[0].trim().parse()
        .context("Invalid ratio 'from' value. Must be an integer")?;
    let ratio_to: i32 = ratio_parts[1].trim().parse()
        .context("Invalid ratio 'to' value. Must be an integer")?;

    if ratio_from <= 0 || ratio_to <= 0 {
        return Err(anyhow::anyhow!("Ratio values must be positive integers"));
    }

    // Parse ex-date
    let ex_date = NaiveDate::parse_from_str(date_str, "%Y-%m-%d")
        .context("Invalid date format. Use YYYY-MM-DD")?;

    // Initialize database
    db::init_database(None)?;
    let conn = db::open_db(None)?;

    // Get or create asset
    let asset_type = db::AssetType::detect_from_ticker(ticker)
        .unwrap_or(db::AssetType::Stock);
    let asset_id = db::upsert_asset(&conn, ticker, &asset_type, None)?;

    // Check if this corporate action already exists
    if db::corporate_action_exists(&conn, asset_id, &ex_date, &action_type)? {
        println!("{} Corporate action already exists for {} on {}",
            "⚠".yellow().bold(), ticker.cyan().bold(), ex_date);
        return Ok(());
    }

    // Create corporate action
    let action = db::CorporateAction {
        id: None,
        asset_id,
        action_type: action_type.clone(),
        event_date: ex_date, // Same as ex_date for manual entries
        ex_date,
        ratio_from,
        ratio_to,
        applied: false,
        source: "MANUAL".to_string(),
        notes: notes.map(|s| s.to_string()),
        created_at: chrono::Utc::now(),
    };

    // Insert corporate action
    let action_id = db::insert_corporate_action(&conn, &action)?;

    // Display confirmation
    println!("\n{} Corporate action added successfully!", "✓".green().bold());
    println!("  Action ID:      {}", action_id);
    println!("  Ticker:         {}", ticker.cyan().bold());
    println!("  Type:           {}", action_type.as_str());
    println!("  Ratio:          {}:{} ({})", ratio_from, ratio_to,
        match action_type {
            db::CorporateActionType::Split => format!("each share becomes {}", ratio_to as f64 / ratio_from as f64),
            db::CorporateActionType::ReverseSplit => format!("{} shares become 1", ratio_from as f64 / ratio_to as f64),
            db::CorporateActionType::Bonus => format!("{}% bonus", ((ratio_to as f64 / ratio_from as f64) - 1.0) * 100.0),
        }
    );
    println!("  Ex-Date:        {}", ex_date.format("%Y-%m-%d"));
    println!("  Applied:        {}", "No (use 'interest actions apply' to apply)".yellow());
    if let Some(n) = notes {
        println!("  Notes:          {}", n);
    }
    println!("\n{} Run this command to apply the action:", "→".blue().bold());
    println!("  interest actions apply {}", ticker);
    println!();

    Ok(())
}

/// Handle scrape corporate actions from investing.com
async fn handle_action_scrape(
    ticker: &str,
    url: Option<&str>,
    name: Option<&str>,
    save: bool,
) -> Result<()> {
    use anyhow::Context;
    use colored::Colorize;

    info!("Scraping corporate actions for {} from investing.com", ticker);

    // Initialize database to get asset info
    db::init_database(None)?;
    let conn = db::open_db(None)?;

    // Get or create asset
    let asset_type = db::AssetType::detect_from_ticker(ticker)
        .unwrap_or(db::AssetType::Stock);
    let asset_id = db::upsert_asset(&conn, ticker, &asset_type, None)?;

    // Determine URL
    let scrape_url = if let Some(u) = url {
        u.to_string()
    } else {
        // Try to build URL automatically
        let company_name = if let Some(n) = name {
            // User provided name - save it to database for future use
            conn.execute(
                "UPDATE assets SET name = ?1 WHERE id = ?2",
                rusqlite::params![n, asset_id],
            )?;
            n.to_string()
        } else {
            // Try to get asset name from database
            let assets = db::get_all_assets(&conn)?;
            let asset = assets.iter()
                .find(|a| a.ticker.eq_ignore_ascii_case(ticker));

            let db_name = asset.and_then(|a| a.name.clone());

            if let Some(name) = db_name {
                name
            } else {
                // Fetch company name from Yahoo Finance
                println!("{} Fetching company name from Yahoo Finance...", "🔍".cyan().bold());

                match pricing::yahoo::fetch_company_name(ticker).await {
                    Ok(fetched_name) => {
                        println!("  Found: {}", fetched_name.green());

                        // Save to database for future use
                        if let Some(asset) = asset {
                            if let Some(asset_id) = asset.id {
                                let _ = conn.execute(
                                    "UPDATE assets SET name = ?1 WHERE id = ?2",
                                    rusqlite::params![&fetched_name, asset_id],
                                );
                            }
                        }

                        fetched_name
                    }
                    Err(e) => {
                        println!("{} Could not fetch company name from Yahoo Finance: {}",
                            "⚠".yellow().bold(), e);
                        println!("  Please provide either:");
                        println!("    1. URL with --url flag");
                        println!("    2. Company name with --name flag");
                        println!("\n{} Example:", "→".blue().bold());
                        println!("  interest actions scrape {} --name \"Advanced Micro Devices Inc\"", ticker);
                        return Err(anyhow::anyhow!("Could not determine company name"));
                    }
                }
            }
        };

        let auto_url = crate::scraping::InvestingScraper::build_splits_url(ticker, &company_name);
        println!("{} Auto-built URL from company name:", "🔗".cyan().bold());
        println!("  {}", auto_url.dimmed());
        println!("  If this URL is incorrect, provide the correct one with --url\n");
        auto_url
    };

    println!("\n{} Launching headless browser to scrape: {}", "🌐".cyan().bold(), scrape_url);
    println!("  This may take 10-30 seconds to bypass Cloudflare...\n");

    // Create scraper and fetch data
    let scraper = crate::scraping::InvestingScraper::new()
        .context("Failed to create scraper. Ensure Chrome/Chromium is installed.")?;

    let mut actions = scraper.scrape_corporate_actions(&scrape_url, &scrape_url)
        .context("Failed to scrape corporate actions")?;

    if actions.is_empty() {
        println!("{} No corporate actions found on the page", "ℹ".yellow().bold());
        return Ok(());
    }

    // Set asset_id for all actions
    for action in &mut actions {
        action.asset_id = asset_id;
    }

    // Display scraped actions
    println!("{} Found {} corporate action(s):\n", "✓".green().bold(), actions.len());

    for action in &actions {
        println!("  {} {} {}:{} on {}",
            match action.action_type {
                db::CorporateActionType::Split => "📈",
                db::CorporateActionType::ReverseSplit => "📉",
                db::CorporateActionType::Bonus => "🎁",
            },
            action.action_type.as_str().cyan(),
            action.ratio_from,
            action.ratio_to,
            action.ex_date.format("%Y-%m-%d")
        );
    }

    if save {
        println!("\n{} Saving to database...", "💾".cyan().bold());

        let mut saved_count = 0;
        let mut skipped_count = 0;

        for action in actions {
            // Check if already exists
            if db::corporate_action_exists(&conn, asset_id, &action.ex_date, &action.action_type)? {
                skipped_count += 1;
                continue;
            }

            db::insert_corporate_action(&conn, &action)?;
            saved_count += 1;
        }

        println!("\n{} Saved {} action(s), skipped {} duplicate(s)",
            "✓".green().bold(), saved_count, skipped_count);

        if saved_count > 0 {
            println!("\n{} Run this command to apply the actions:", "→".blue().bold());
            println!("  interest actions apply {}", ticker);
        }
    } else {
        println!("\n{} Actions not saved. Use --save flag to save to database", "ℹ".blue().bold());
    }
    println!();

    Ok(())
}

/// Handle apply corporate actions command
async fn handle_action_apply(ticker_filter: Option<&str>) -> Result<()> {
    use colored::Colorize;

    info!("Applying corporate actions");

    // Initialize database
    db::init_database(None)?;
    let conn = db::open_db(None)?;

    // Get unapplied corporate actions
    let actions = if let Some(ticker) = ticker_filter {
        // Get asset ID for the ticker
        let assets = db::get_all_assets(&conn)?;
        let asset = assets.iter()
            .find(|a| a.ticker.eq_ignore_ascii_case(ticker))
            .ok_or_else(|| anyhow::anyhow!("Ticker {} not found in database", ticker))?;

        crate::corporate_actions::get_unapplied_actions(&conn, Some(asset.id.unwrap()))?
    } else {
        crate::corporate_actions::get_unapplied_actions(&conn, None)?
    };

    if actions.is_empty() {
        println!("{} No unapplied corporate actions found", "ℹ".blue().bold());
        if let Some(t) = ticker_filter {
            println!("  Filter: {}", t);
        }
        return Ok(());
    }

    println!("\n{} Found {} unapplied corporate action(s)\n",
        "📋".cyan().bold(), actions.len());

    // Apply each action
    for action in actions {
        let asset = db::get_all_assets(&conn)?
            .into_iter()
            .find(|a| a.id == Some(action.asset_id))
            .ok_or_else(|| anyhow::anyhow!("Asset not found for action {}", action.id.unwrap_or(0)))?;

        println!("  {} Applying {} for {} (ex-date: {})",
            "→".blue(),
            action.action_type.as_str().cyan(),
            asset.ticker.cyan().bold(),
            action.ex_date.format("%Y-%m-%d")
        );

        // Apply the action
        let adjusted_count = crate::corporate_actions::apply_corporate_action(&conn, &action, &asset)?;

        println!("    {} Adjusted {} transaction(s)", "✓".green(), adjusted_count);
    }

    println!("\n{} All corporate actions applied successfully!", "✓".green().bold());
    println!();

    Ok(())
}

/// Handle action delete command
async fn handle_action_delete(action_id: i64) -> Result<()> {
    use anyhow::Context;
    use colored::Colorize;

    info!("Deleting corporate action {}", action_id);

    // Initialize database
    db::init_database(None)?;
    let conn = db::open_db(None)?;

    // Get the action details before deleting
    let action = conn.query_row(
        "SELECT id, asset_id, action_type, event_date, ex_date, ratio_from, ratio_to, applied, source, notes, created_at
         FROM corporate_actions WHERE id = ?1",
        rusqlite::params![action_id],
        |row| {
            Ok(db::CorporateAction {
                id: Some(row.get(0)?),
                asset_id: row.get(1)?,
                action_type: db::CorporateActionType::from_str(&row.get::<_, String>(2)?)
                    .ok_or_else(|| rusqlite::Error::InvalidQuery)?,
                event_date: row.get(3)?,
                ex_date: row.get(4)?,
                ratio_from: row.get(5)?,
                ratio_to: row.get(6)?,
                applied: row.get(7)?,
                source: row.get(8)?,
                notes: row.get(9)?,
                created_at: row.get(10)?,
            })
        },
    ).context(format!("Corporate action with ID {} not found", action_id))?;

    // Get asset details
    let asset = db::get_all_assets(&conn)?
        .into_iter()
        .find(|a| a.id == Some(action.asset_id))
        .ok_or_else(|| anyhow::anyhow!("Asset not found for action"))?;

    // Check if already applied
    if action.applied {
        println!("\n{} Cannot delete applied corporate action!", "⚠".yellow().bold());
        println!("  This action has already been applied to transactions.");
        println!("  You would need to manually revert the adjustments first.");
        return Err(anyhow::anyhow!("Cannot delete applied action"));
    }

    // Display action details
    println!("\n{} Deleting corporate action:\n", "🗑".red().bold());
    println!("  ID:       {}", action_id);
    println!("  Ticker:   {}", asset.ticker.cyan().bold());
    println!("  Type:     {}", action.action_type.as_str());
    println!("  Ratio:    {}:{}", action.ratio_from, action.ratio_to);
    println!("  Ex-date:  {}", action.ex_date.format("%Y-%m-%d"));
    println!("  Source:   {}", action.source);
    if let Some(ref notes) = action.notes {
        println!("  Notes:    {}", notes);
    }

    // Delete from database
    conn.execute("DELETE FROM corporate_actions WHERE id = ?1", rusqlite::params![action_id])?;

    println!("\n{} Corporate action deleted successfully!\n", "✓".green().bold());

    Ok(())
}

/// Handle action edit command
async fn handle_action_edit(
    action_id: i64,
    action_type: Option<&str>,
    ratio: Option<&str>,
    date: Option<&str>,
    notes: Option<&str>,
) -> Result<()> {
    use anyhow::Context;
    use colored::Colorize;
    use chrono::NaiveDate;

    info!("Editing corporate action {}", action_id);

    // Validate that at least one field is provided
    if action_type.is_none() && ratio.is_none() && date.is_none() && notes.is_none() {
        println!("\n{} No changes specified. Use --action-type, --ratio, --date, or --notes", "ℹ".yellow().bold());
        return Ok(());
    }

    // Initialize database
    db::init_database(None)?;
    let conn = db::open_db(None)?;

    // Get the action details
    let mut action = conn.query_row(
        "SELECT id, asset_id, action_type, event_date, ex_date, ratio_from, ratio_to, applied, source, notes, created_at
         FROM corporate_actions WHERE id = ?1",
        rusqlite::params![action_id],
        |row| {
            Ok(db::CorporateAction {
                id: Some(row.get(0)?),
                asset_id: row.get(1)?,
                action_type: db::CorporateActionType::from_str(&row.get::<_, String>(2)?)
                    .ok_or_else(|| rusqlite::Error::InvalidQuery)?,
                event_date: row.get(3)?,
                ex_date: row.get(4)?,
                ratio_from: row.get(5)?,
                ratio_to: row.get(6)?,
                applied: row.get(7)?,
                source: row.get(8)?,
                notes: row.get(9)?,
                created_at: row.get(10)?,
            })
        },
    ).context(format!("Corporate action with ID {} not found", action_id))?;

    // Check if already applied
    if action.applied {
        println!("\n{} Cannot edit applied corporate action!", "⚠".yellow().bold());
        println!("  This action has already been applied to transactions.");
        println!("  Delete and re-add it if you need to make changes.");
        return Err(anyhow::anyhow!("Cannot edit applied action"));
    }

    // Get asset details
    let asset = db::get_all_assets(&conn)?
        .into_iter()
        .find(|a| a.id == Some(action.asset_id))
        .ok_or_else(|| anyhow::anyhow!("Asset not found for action"))?;

    println!("\n{} Editing corporate action for {}\n", "✏".cyan().bold(), asset.ticker.cyan().bold());

    // Apply updates
    let mut updates = Vec::new();

    if let Some(new_type) = action_type {
        let new_action_type = db::CorporateActionType::from_str(new_type)
            .context(format!("Invalid action type: {}", new_type))?;
        println!("  Type:     {} → {}",
            action.action_type.as_str().dimmed(),
            new_action_type.as_str().green()
        );
        action.action_type = new_action_type;
        updates.push("action_type");
    }

    if let Some(ratio_str) = ratio {
        let parts: Vec<&str> = ratio_str.split(':').collect();
        if parts.len() != 2 {
            return Err(anyhow::anyhow!("Invalid ratio format. Use 'from:to' (e.g., '1:8')"));
        }
        let new_from: i32 = parts[0].trim().parse()
            .context("Invalid ratio 'from' value")?;
        let new_to: i32 = parts[1].trim().parse()
            .context("Invalid ratio 'to' value")?;

        println!("  Ratio:    {}:{} → {}:{}",
            action.ratio_from.to_string().dimmed(),
            action.ratio_to.to_string().dimmed(),
            new_from.to_string().green(),
            new_to.to_string().green()
        );
        action.ratio_from = new_from;
        action.ratio_to = new_to;
        updates.push("ratio_from");
        updates.push("ratio_to");
    }

    if let Some(date_str) = date {
        let new_date = NaiveDate::parse_from_str(date_str, "%Y-%m-%d")
            .context("Invalid date format. Use YYYY-MM-DD")?;

        println!("  Ex-date:  {} → {}",
            action.ex_date.format("%Y-%m-%d").to_string().dimmed(),
            new_date.format("%Y-%m-%d").to_string().green()
        );
        action.ex_date = new_date;
        action.event_date = new_date;
        updates.push("ex_date");
        updates.push("event_date");
    }

    if let Some(new_notes) = notes {
        let old_notes = action.notes.as_deref().unwrap_or("(none)");
        println!("  Notes:    {} → {}",
            old_notes.dimmed(),
            new_notes.green()
        );
        action.notes = Some(new_notes.to_string());
        updates.push("notes");
    }

    // Update database
    conn.execute(
        "UPDATE corporate_actions
         SET action_type = ?1, ratio_from = ?2, ratio_to = ?3, ex_date = ?4, event_date = ?5, notes = ?6
         WHERE id = ?7",
        rusqlite::params![
            action.action_type.as_str(),
            action.ratio_from,
            action.ratio_to,
            action.ex_date,
            action.event_date,
            action.notes,
            action_id,
        ],
    )?;

    println!("\n{} Corporate action updated successfully!\n", "✓".green().bold());

    Ok(())
}

/// Handle portfolio show command
async fn handle_portfolio_show(asset_type: Option<&str>) -> Result<()> {
    use colored::Colorize;
    use tabled::{Table, Tabled, settings::Style};
    use anyhow::Context;

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

/// Handle manual transaction add command
async fn handle_transaction_add(
    ticker: &str,
    transaction_type: &str,
    quantity_str: &str,
    price_str: &str,
    date_str: &str,
    fees_str: &str,
    notes: Option<&str>,
) -> Result<()> {
    use colored::Colorize;
    use rust_decimal::Decimal;
    use std::str::FromStr;
    use chrono::NaiveDate;
    use anyhow::Context;

    info!("Adding manual transaction for {}", ticker);

    // Parse and validate inputs
    let quantity = Decimal::from_str(quantity_str)
        .context("Invalid quantity. Must be a decimal number")?;

    let price = Decimal::from_str(price_str)
        .context("Invalid price. Must be a decimal number")?;

    let fees = Decimal::from_str(fees_str)
        .context("Invalid fees. Must be a decimal number")?;

    let trade_date = NaiveDate::parse_from_str(date_str, "%Y-%m-%d")
        .context("Invalid date format. Use YYYY-MM-DD")?;

    // Parse transaction type
    let tx_type = match transaction_type.to_uppercase().as_str() {
        "BUY" => db::TransactionType::Buy,
        "SELL" => db::TransactionType::Sell,
        _ => return Err(anyhow::anyhow!("Transaction type must be 'buy' or 'sell'")),
    };

    // Validate inputs
    if quantity <= Decimal::ZERO {
        return Err(anyhow::anyhow!("Quantity must be greater than zero"));
    }

    if price <= Decimal::ZERO {
        return Err(anyhow::anyhow!("Price must be greater than zero"));
    }

    if fees < Decimal::ZERO {
        return Err(anyhow::anyhow!("Fees cannot be negative"));
    }

    // Calculate total cost
    let total_cost = (quantity * price) + fees;

    // Initialize database
    db::init_database(None)?;
    let conn = db::open_db(None)?;

    // Detect asset type from ticker
    let asset_type = db::AssetType::detect_from_ticker(ticker)
        .unwrap_or(db::AssetType::Stock);

    // Upsert asset
    let asset_id = db::upsert_asset(&conn, ticker, &asset_type, None)?;

    // Create transaction
    let transaction = db::Transaction {
        id: None,
        asset_id,
        transaction_type: tx_type.clone(),
        trade_date,
        settlement_date: Some(trade_date), // Same as trade date for manual entries
        quantity,
        price_per_unit: price,
        total_cost,
        fees,
        is_day_trade: false,
        quota_issuance_date: None,
        notes: notes.map(|s| s.to_string()),
        source: "MANUAL".to_string(),
        created_at: chrono::Utc::now(),
    };

    // Insert transaction
    let tx_id = db::insert_transaction(&conn, &transaction)?;

    // Auto-apply any relevant corporate actions to this historical transaction
    let actions_applied = crate::corporate_actions::apply_actions_to_transaction(&conn, tx_id)?;

    // Display confirmation
    println!("\n{} Transaction added successfully!", "✓".green().bold());
    println!("  Transaction ID: {}", tx_id);
    println!("  Ticker:         {}", ticker.cyan().bold());
    println!("  Type:           {}", tx_type.as_str().to_uppercase());
    println!("  Date:           {}", trade_date.format("%Y-%m-%d"));
    println!("  Quantity:       {}", quantity);
    println!("  Price:          {}", format!("R$ {:.2}", price).cyan());
    println!("  Fees:           {}", format!("R$ {:.2}", fees).cyan());
    println!("  Total:          {}", format!("R$ {:.2}", total_cost).cyan().bold());
    if let Some(n) = notes {
        println!("  Notes:          {}", n);
    }

    if actions_applied > 0 {
        println!("\n{} Auto-applied {} corporate action(s) to this transaction",
            "ℹ".blue().bold(), actions_applied);
        println!("  The quantity and price have been adjusted automatically.");
        println!("  Run 'interest portfolio show' to see the adjusted values.");
    }
    println!();

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
