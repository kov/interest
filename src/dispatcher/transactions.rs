use anyhow::Result;

pub async fn dispatch_transactions(
    action: &crate::cli::TransactionCommands,
    json_output: bool,
) -> Result<()> {
    match action {
        crate::cli::TransactionCommands::Add {
            ticker,
            transaction_type,
            quantity,
            price,
            date,
            fees,
            day_trade,
            notes,
        } => {
            dispatch_transaction_add(
                ticker,
                transaction_type,
                quantity,
                price,
                date,
                fees,
                *day_trade,
                notes.as_deref(),
            )
            .await
        }
        crate::cli::TransactionCommands::List { ticker } => {
            dispatch_transactions_list(ticker.as_deref(), json_output).await
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn dispatch_transaction_add(
    ticker: &str,
    transaction_type: &str,
    quantity_str: &str,
    price_str: &str,
    date_str: &str,
    fees_str: &str,
    day_trade: bool,
    notes: Option<&str>,
) -> Result<()> {
    use anyhow::Context;
    use chrono::NaiveDate;
    use colored::Colorize;
    use rust_decimal::Decimal;
    use std::str::FromStr;

    tracing::info!("Adding manual transaction for {}", ticker);

    // Parse and validate inputs
    let quantity =
        Decimal::from_str(quantity_str).context("Invalid quantity. Must be a decimal number")?;

    let price = Decimal::from_str(price_str).context("Invalid price. Must be a decimal number")?;

    let fees = Decimal::from_str(fees_str).context("Invalid fees. Must be a decimal number")?;

    let trade_date = NaiveDate::parse_from_str(date_str, "%Y-%m-%d")
        .context("Invalid date format. Use YYYY-MM-DD")?;

    // Parse transaction type
    let tx_type = match transaction_type.to_uppercase().as_str() {
        "BUY" => crate::db::TransactionType::Buy,
        "SELL" => crate::db::TransactionType::Sell,
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
    crate::db::init_database(None)?;
    let conn = crate::db::open_db(None)?;

    // Detect asset type from ticker
    let asset_type = crate::db::AssetType::Unknown;

    // Upsert asset
    let asset_id = crate::db::upsert_asset(&conn, ticker, &asset_type, None)?;

    // Create transaction
    let transaction = crate::db::Transaction {
        id: None,
        asset_id,
        transaction_type: tx_type.clone(),
        trade_date,
        settlement_date: Some(trade_date), // Same as trade date for manual entries
        quantity,
        price_per_unit: price,
        total_cost,
        fees,
        is_day_trade: day_trade,
        quota_issuance_date: None,
        notes: notes.map(|s| s.to_string()),
        source: "MANUAL".to_string(),
        created_at: chrono::Utc::now(),
    };

    // Insert transaction
    let tx_id = crate::db::insert_transaction(&conn, &transaction)?;

    // Display confirmation
    println!("\n{} Transaction added successfully!", "✓".green().bold());
    println!("  Transaction ID: {}", tx_id);
    println!("  Ticker:         {}", ticker.cyan().bold());
    println!("  Type:           {}", tx_type.as_str().to_uppercase());
    println!("  Date:           {}", trade_date.format("%Y-%m-%d"));
    println!("  Quantity:       {}", quantity);
    println!(
        "  Price:          {}",
        crate::utils::format_currency(price).cyan()
    );
    println!(
        "  Fees:           {}",
        crate::utils::format_currency(fees).cyan()
    );
    println!(
        "  Total:          {}",
        crate::utils::format_currency(total_cost).cyan().bold()
    );
    if let Some(n) = notes {
        println!("  Notes:          {}", n);
    }

    println!();

    Ok(())
}

async fn dispatch_transactions_list(ticker: Option<&str>, json_output: bool) -> Result<()> {
    use serde::Serialize;

    crate::db::init_database(None)?;
    let conn = crate::db::open_db(None)?;

    #[derive(Serialize)]
    struct TransactionRow {
        id: Option<i64>,
        ticker: String,
        transaction_type: String,
        trade_date: String,
        settlement_date: Option<String>,
        quantity: String,
        price_per_unit: String,
        total_cost: String,
        fees: String,
        is_day_trade: bool,
        notes: Option<String>,
        source: String,
    }

    let mut rows = Vec::new();
    if let Some(ticker) = ticker {
        let asset = crate::db::get_asset_by_ticker(&conn, ticker)?
            .ok_or_else(|| anyhow::anyhow!("Ticker {} not found", ticker))?;

        let mut stmt = conn.prepare(
            "SELECT id, transaction_type, trade_date, settlement_date, quantity, price_per_unit,
                    total_cost, fees, is_day_trade, notes, source
             FROM transactions
             WHERE asset_id = ?1
             ORDER BY trade_date ASC, id ASC",
        )?;
        let mut iter = stmt.query([asset.id.expect("asset id")])?;
        while let Some(row) = iter.next()? {
            rows.push(TransactionRow {
                id: row.get(0)?,
                ticker: asset.ticker.clone(),
                transaction_type: row.get::<_, String>(1)?,
                trade_date: row.get::<_, String>(2)?,
                settlement_date: row.get::<_, Option<String>>(3)?,
                quantity: crate::db::get_decimal_value(row, 4)?.to_string(),
                price_per_unit: crate::db::get_decimal_value(row, 5)?.to_string(),
                total_cost: crate::db::get_decimal_value(row, 6)?.to_string(),
                fees: crate::db::get_decimal_value(row, 7)?.to_string(),
                is_day_trade: row.get(8)?,
                notes: row.get(9)?,
                source: row.get(10)?,
            });
        }
    } else {
        let mut stmt = conn.prepare(
            "SELECT t.id, a.ticker, t.transaction_type, t.trade_date, t.settlement_date,
                    t.quantity, t.price_per_unit, t.total_cost, t.fees, t.is_day_trade,
                    t.notes, t.source
             FROM transactions t
             JOIN assets a ON t.asset_id = a.id
             ORDER BY t.trade_date ASC, t.id ASC",
        )?;
        let mut iter = stmt.query([])?;
        while let Some(row) = iter.next()? {
            rows.push(TransactionRow {
                id: row.get(0)?,
                ticker: row.get::<_, String>(1)?,
                transaction_type: row.get::<_, String>(2)?,
                trade_date: row.get::<_, String>(3)?,
                settlement_date: row.get::<_, Option<String>>(4)?,
                quantity: crate::db::get_decimal_value(row, 5)?.to_string(),
                price_per_unit: crate::db::get_decimal_value(row, 6)?.to_string(),
                total_cost: crate::db::get_decimal_value(row, 7)?.to_string(),
                fees: crate::db::get_decimal_value(row, 8)?.to_string(),
                is_day_trade: row.get(9)?,
                notes: row.get(10)?,
                source: row.get(11)?,
            });
        }
    }

    if json_output {
        println!("{}", serde_json::to_string_pretty(&rows)?);
    } else {
        let mut out = String::new();
        for row in rows {
            out.push_str(&format!(
                "{} {} {} {} @ {} (fees {})\n",
                row.trade_date,
                row.ticker,
                row.transaction_type,
                row.quantity,
                row.price_per_unit,
                row.fees
            ));
        }
        if out.is_empty() {
            println!("No transactions found");
        } else {
            print!("{}", out);
        }
    }

    Ok(())
}
