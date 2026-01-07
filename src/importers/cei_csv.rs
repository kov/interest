use anyhow::{anyhow, Context, Result};
use chrono::NaiveDate;
use csv::ReaderBuilder;
use rust_decimal::Decimal;
use std::path::Path;
use std::str::FromStr;
use tracing::{debug, info, warn};

use super::cei_excel::RawTransaction;

/// Parse B3/CEI CSV file and extract transactions
pub fn parse_cei_csv<P: AsRef<Path>>(file_path: P) -> Result<Vec<RawTransaction>> {
    let path = file_path.as_ref();
    info!("Parsing CEI CSV file: {:?}", path);

    let mut reader = ReaderBuilder::new()
        .delimiter(b';') // Brazilian CSV often uses semicolon
        .flexible(true) // Allow variable number of columns
        .from_path(path)
        .context("Failed to open CSV file")?;

    let headers = reader
        .headers()
        .context("Failed to read CSV headers")?
        .clone();

    debug!("CSV headers: {:?}", headers);

    // Find column indices
    let column_mapping = find_columns(&headers)?;
    debug!("Column mapping: {:?}", column_mapping);

    let mut transactions = Vec::new();

    for (idx, result) in reader.records().enumerate() {
        let record = result.context("Failed to read CSV record")?;

        match parse_csv_row(&record, &column_mapping, idx + 2) {
            Ok(Some(transaction)) => {
                transactions.push(transaction);
            }
            Ok(None) => {
                // Skip row
                continue;
            }
            Err(e) => {
                warn!("Skipping row {}: {}", idx + 2, e);
                continue;
            }
        }
    }

    info!(
        "Successfully parsed {} transactions from CSV",
        transactions.len()
    );
    Ok(transactions)
}

#[derive(Debug)]
struct CsvColumnMapping {
    date: usize,
    ticker: usize,
    transaction_type: usize,
    quantity: usize,
    price: usize,
    total: Option<usize>,
    fees: Option<usize>,
    market: Option<usize>,
}

fn find_columns(headers: &csv::StringRecord) -> Result<CsvColumnMapping> {
    let mut date_idx = None;
    let mut ticker_idx = None;
    let mut type_idx = None;
    let mut quantity_idx = None;
    let mut price_idx = None;
    let mut total_idx = None;
    let mut fees_idx = None;
    let mut market_idx = None;

    for (idx, header) in headers.iter().enumerate() {
        let text = header.to_lowercase();

        // Date
        if text.contains("data")
            && (text.contains("negó") || text.contains("nego") || date_idx.is_none())
        {
            date_idx = Some(idx);
        }

        // Ticker
        if text.contains("código")
            || text.contains("codigo")
            || text.contains("ticker")
            || text.contains("produto")
        {
            ticker_idx = Some(idx);
        }

        // Transaction type
        if text.contains("c/v") || (text.contains("tipo") && text.contains("moviment")) {
            type_idx = Some(idx);
        }

        // Quantity
        if text.contains("quantidade") || text.contains("qtd") || text.contains("qtde") {
            quantity_idx = Some(idx);
        }

        // Price
        if text.contains("preço") || text.contains("preco") {
            price_idx = Some(idx);
        }

        // Total
        if text.contains("valor") && text.contains("total") {
            total_idx = Some(idx);
        }

        // Fees
        if text.contains("taxa") || text.contains("despesa") {
            fees_idx = Some(idx);
        }

        // Market
        if text.contains("mercado") {
            market_idx = Some(idx);
        }
    }

    Ok(CsvColumnMapping {
        date: date_idx.ok_or_else(|| anyhow!("Date column not found"))?,
        ticker: ticker_idx.ok_or_else(|| anyhow!("Ticker column not found"))?,
        transaction_type: type_idx.ok_or_else(|| anyhow!("Transaction type column not found"))?,
        quantity: quantity_idx.ok_or_else(|| anyhow!("Quantity column not found"))?,
        price: price_idx.ok_or_else(|| anyhow!("Price column not found"))?,
        total: total_idx,
        fees: fees_idx,
        market: market_idx,
    })
}

fn parse_csv_row(
    record: &csv::StringRecord,
    mapping: &CsvColumnMapping,
    row_num: usize,
) -> Result<Option<RawTransaction>> {
    // Get ticker - skip if empty
    let ticker = record
        .get(mapping.ticker)
        .ok_or_else(|| anyhow!("Missing ticker at row {}", row_num))?
        .trim()
        .to_uppercase();

    if ticker.is_empty() {
        return Ok(None);
    }

    // Parse date
    let date_str = record
        .get(mapping.date)
        .ok_or_else(|| anyhow!("Missing date at row {}", row_num))?;
    let trade_date = parse_csv_date(date_str)?;

    // Transaction type
    let transaction_type = record
        .get(mapping.transaction_type)
        .ok_or_else(|| anyhow!("Missing transaction type at row {}", row_num))?
        .trim()
        .to_uppercase();

    // Quantity
    let quantity_str = record
        .get(mapping.quantity)
        .ok_or_else(|| anyhow!("Missing quantity at row {}", row_num))?;
    let quantity = parse_csv_decimal(quantity_str)?;

    // Price
    let price_str = record
        .get(mapping.price)
        .ok_or_else(|| anyhow!("Missing price at row {}", row_num))?;
    let price = parse_csv_decimal(price_str)?;

    // Total (optional, calculate if not present)
    let total = if let Some(total_idx) = mapping.total {
        if let Some(total_str) = record.get(total_idx) {
            parse_csv_decimal(total_str).unwrap_or(quantity * price)
        } else {
            quantity * price
        }
    } else {
        quantity * price
    };

    // Fees (optional)
    let fees = if let Some(fees_idx) = mapping.fees {
        record
            .get(fees_idx)
            .and_then(|s| parse_csv_decimal(s).ok())
            .unwrap_or(Decimal::ZERO)
    } else {
        Decimal::ZERO
    };

    // Market (optional)
    let market = mapping
        .market
        .and_then(|idx| record.get(idx))
        .map(|s| s.to_string());

    Ok(Some(RawTransaction {
        ticker,
        transaction_type,
        trade_date,
        quantity,
        price,
        fees,
        total,
        market,
    }))
}

fn parse_csv_date(date_str: &str) -> Result<NaiveDate> {
    // Try common Brazilian formats
    if let Ok(date) = NaiveDate::parse_from_str(date_str, "%d/%m/%Y") {
        return Ok(date);
    }
    if let Ok(date) = NaiveDate::parse_from_str(date_str, "%d-%m-%Y") {
        return Ok(date);
    }
    if let Ok(date) = NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
        return Ok(date);
    }
    if let Ok(date) = NaiveDate::parse_from_str(date_str, "%d/%m/%y") {
        return Ok(date);
    }

    Err(anyhow!("Could not parse date: {}", date_str))
}

fn parse_csv_decimal(text: &str) -> Result<Decimal> {
    let cleaned = text
        .replace("R$", "")
        .replace(" ", "")
        .replace(".", "") // Remove thousand separators
        .replace(",", "."); // Replace decimal comma with dot

    Decimal::from_str(&cleaned).context("Failed to parse decimal")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_csv_decimal() {
        assert_eq!(
            parse_csv_decimal("1.234,56").unwrap(),
            Decimal::from_str("1234.56").unwrap()
        );
        assert_eq!(
            parse_csv_decimal("R$ 10,50").unwrap(),
            Decimal::from_str("10.50").unwrap()
        );
    }

    #[test]
    fn test_parse_csv_date() {
        assert_eq!(
            parse_csv_date("15/03/2025").unwrap(),
            NaiveDate::from_ymd_opt(2025, 3, 15).unwrap()
        );
    }
}
