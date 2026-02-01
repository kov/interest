use anyhow::Result;

use crate::{formatters, options};

pub async fn dispatch_transactions(
    action: &crate::cli::TransactionCommands,
    options: options::OutputOptions,
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
                options,
            )
            .await
        }
        crate::cli::TransactionCommands::List { ticker } => {
            dispatch_transactions_list(ticker.as_deref(), options).await
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
    options: options::OutputOptions,
) -> Result<()> {
    use anyhow::Context;
    use chrono::NaiveDate;
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
    print!(
        "{}",
        formatters::transactions::format_transaction_add_table(
            tx_id,
            ticker,
            tx_type.as_str(),
            trade_date,
            quantity,
            price,
            fees,
            total_cost,
            notes,
            options,
        )
    );

    Ok(())
}

async fn dispatch_transactions_list(
    ticker: Option<&str>,
    options: options::OutputOptions,
) -> Result<()> {
    crate::db::init_database(None)?;
    let conn = crate::db::open_db(None)?;

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
            rows.push(formatters::transactions::TransactionRow {
                id: row.get(0)?,
                ticker: asset.ticker.clone(),
                transaction_type: row.get::<_, String>(1)?,
                trade_date: row.get(2)?,
                settlement_date: row.get(3)?,
                quantity: crate::db::get_decimal_value(row, 4)?,
                price_per_unit: crate::db::get_decimal_value(row, 5)?,
                total_cost: crate::db::get_decimal_value(row, 6)?,
                fees: crate::db::get_decimal_value(row, 7)?,
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
            rows.push(formatters::transactions::TransactionRow {
                id: row.get(0)?,
                ticker: row.get::<_, String>(1)?,
                transaction_type: row.get::<_, String>(2)?,
                trade_date: row.get(3)?,
                settlement_date: row.get(4)?,
                quantity: crate::db::get_decimal_value(row, 5)?,
                price_per_unit: crate::db::get_decimal_value(row, 6)?,
                total_cost: crate::db::get_decimal_value(row, 7)?,
                fees: crate::db::get_decimal_value(row, 8)?,
                is_day_trade: row.get(9)?,
                notes: row.get(10)?,
                source: row.get(11)?,
            });
        }
    }

    println!(
        "{}",
        formatters::transactions::format_transactions_list(&rows, options)
    );

    Ok(())
}
