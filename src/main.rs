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
                if let Some(ref atype) = asset_type {
                    println!("Showing portfolio for asset type: {}", atype);
                } else {
                    println!("Showing full portfolio");
                }
                // TODO: Implement portfolio show
                Ok(())
            }
            PortfolioCommands::Performance { period } => {
                println!("Showing performance for period: {}", period);
                // TODO: Implement performance metrics
                Ok(())
            }
        },

        Commands::Prices { action } => match action {
            PriceCommands::Update => {
                println!("Updating all prices...");
                // TODO: Implement price updates
                Ok(())
            }
            PriceCommands::History { ticker, from, to } => {
                println!("Fetching historical prices for {} from {} to {}", ticker, from, to);
                // TODO: Implement historical price fetching
                Ok(())
            }
        },

        Commands::Tax { action } => match action {
            TaxCommands::Calculate { month } => {
                println!("Calculating tax for month: {}", month);
                // TODO: Implement tax calculation
                Ok(())
            }
            TaxCommands::Report { year } => {
                println!("Generating IRPF report for year: {}", year);
                // TODO: Implement IRPF report
                Ok(())
            }
            TaxCommands::Summary { year } => {
                println!("Showing tax summary for year: {}", year);
                // TODO: Implement tax summary
                Ok(())
            }
        },

        Commands::Actions { action } => match action {
            ActionCommands::Update => {
                println!("Updating corporate actions...");
                // TODO: Implement corporate actions update
                Ok(())
            }
            ActionCommands::List { ticker } => {
                if let Some(ref t) = ticker {
                    println!("Listing corporate actions for: {}", t);
                } else {
                    println!("Listing all corporate actions");
                }
                // TODO: Implement corporate actions list
                Ok(())
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
