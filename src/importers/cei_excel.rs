use anyhow::{anyhow, Context, Result};
use calamine::{open_workbook, Reader, Xlsx, Data, DataType};
use chrono::NaiveDate;
use rust_decimal::Decimal;
use std::path::Path;
use std::str::FromStr;
use tracing::{debug, info, warn};

use crate::db::models::{Transaction, TransactionType};

/// Raw transaction data parsed from Excel
#[derive(Debug, Clone)]
pub struct RawTransaction {
    pub ticker: String,
    pub transaction_type: String,  // "C", "V", "COMPRA", "VENDA", etc.
    pub trade_date: NaiveDate,
    pub quantity: Decimal,
    pub price: Decimal,
    pub fees: Decimal,
    pub total: Decimal,
    pub market: Option<String>,  // "VISTA", "FRACIONÁRIO", etc.
}

impl RawTransaction {
    /// Normalize ticker for fractional market (e.g., AMBP3F -> AMBP3)
    pub fn normalized_ticker(&self) -> String {
        let is_fractional = self.market
            .as_deref()
            .map(|m| m.to_uppercase().contains("FRACION"))
            .unwrap_or(false);

        if is_fractional && self.ticker.ends_with('F') && self.ticker.len() > 1 {
            self.ticker[..self.ticker.len() - 1].to_string()
        } else {
            self.ticker.clone()
        }
    }

    /// Convert to Transaction model with asset type detection
    pub fn to_transaction(&self, asset_id: i64) -> Result<Transaction> {
        let transaction_type = TransactionType::from_str(&self.transaction_type)
            .ok_or_else(|| anyhow!("Invalid transaction type: {}", self.transaction_type))?;

        Ok(Transaction {
            id: None,
            asset_id,
            transaction_type,
            trade_date: self.trade_date,
            settlement_date: None,  // Can be calculated as trade_date + 2 business days
            quantity: self.quantity,
            price_per_unit: self.price,
            total_cost: self.total,
            fees: self.fees,
            is_day_trade: false,  // Will be detected later
            quota_issuance_date: None,  // For funds, can be filled in later
            notes: self.market.clone(),
            source: "CEI".to_string(),
            created_at: chrono::Utc::now(),
        })
    }
}

/// Column mapping for B3/CEI Excel exports
#[derive(Debug, Clone)]
struct ColumnMapping {
    date: Option<usize>,
    ticker: Option<usize>,
    transaction_type: Option<usize>,
    quantity: Option<usize>,
    price: Option<usize>,
    total: Option<usize>,
    fees: Option<usize>,
    market: Option<usize>,
}

impl ColumnMapping {
    /// Create column mapping by scanning header row
    fn from_header(header: &[Data]) -> Self {
        let mut mapping = ColumnMapping {
            date: None,
            ticker: None,
            transaction_type: None,
            quantity: None,
            price: None,
            total: None,
            fees: None,
            market: None,
        };

        for (idx, cell) in header.iter().enumerate() {
            let text = cell.to_string().to_lowercase();

            // Date columns
            if text.contains("data") && (text.contains("negó") || text.contains("nego")) {
                mapping.date = Some(idx);
            } else if mapping.date.is_none() && text.contains("data") {
                mapping.date = Some(idx);
            }

            // Ticker/Code columns
            if text.contains("código") || text.contains("codigo") || text.contains("ticker") {
                mapping.ticker = Some(idx);
            } else if mapping.ticker.is_none() && text.contains("produto") {
                mapping.ticker = Some(idx);
            }

            // Transaction type (Buy/Sell)
            if text.contains("c/v") || text.contains("tipo") && text.contains("moviment") {
                mapping.transaction_type = Some(idx);
            }

            // Quantity
            if text.contains("quantidade") || text.contains("qtd") {
                mapping.quantity = Some(idx);
            }

            // Price
            if text.contains("preço") || text.contains("preco") {
                if text.contains("unitário") || text.contains("unitario") {
                    mapping.price = Some(idx);
                } else if mapping.price.is_none() {
                    mapping.price = Some(idx);
                }
            }

            // Total value
            if text.contains("valor") && text.contains("total") {
                mapping.total = Some(idx);
            } else if mapping.total.is_none() && text == "valor" {
                mapping.total = Some(idx);
            }

            // Fees
            if text.contains("taxa") || text.contains("despesa") {
                mapping.fees = Some(idx);
            }

            // Market type
            if text.contains("mercado") {
                mapping.market = Some(idx);
            }
        }

        mapping
    }

    /// Check if all required columns are present
    fn is_valid(&self) -> bool {
        self.date.is_some()
            && self.ticker.is_some()
            && self.transaction_type.is_some()
            && self.quantity.is_some()
            && self.price.is_some()
    }
}

/// Parse B3/CEI Excel file and extract transactions
pub fn parse_cei_excel<P: AsRef<Path>>(file_path: P) -> Result<Vec<RawTransaction>> {
    let path = file_path.as_ref();
    info!("Parsing CEI Excel file: {:?}", path);

    let mut workbook: Xlsx<_> = open_workbook(path)
        .context("Failed to open Excel file")?;

    // Try to find the "Negociação de Ativos" sheet or similar
    let sheet_name = find_trading_sheet(&workbook)?;

    info!("Found trading sheet: {}", sheet_name);

    let range = workbook
        .worksheet_range(&sheet_name)
        .context("Failed to read worksheet")?;

    let mut transactions = Vec::new();
    let mut header_row_idx = None;
    let mut column_mapping: Option<ColumnMapping> = None;

    // Scan for header row (look for key column names)
    for (idx, row) in range.rows().enumerate() {
        // Check if this row looks like a header
        let row_text = row.iter()
            .map(|cell| cell.to_string().to_lowercase())
            .collect::<Vec<_>>()
            .join(" ");

        if row_text.contains("data") && (row_text.contains("ticker") || row_text.contains("código") || row_text.contains("produto")) {
            header_row_idx = Some(idx);
            let mapping = ColumnMapping::from_header(row);

            if mapping.is_valid() {
                debug!("Column mapping: {:?}", mapping);
                column_mapping = Some(mapping);
                break;
            } else {
                warn!("Found potential header row but missing required columns");
            }
        }
    }

    let header_idx = header_row_idx.ok_or_else(|| anyhow!("Could not find header row with required columns"))?;
    let mapping = column_mapping.ok_or_else(|| anyhow!("Could not create valid column mapping"))?;

    // Parse data rows (skip header and any rows before it)
    for (idx, row) in range.rows().enumerate() {
        if idx <= header_idx {
            continue;  // Skip header and rows before it
        }

        // Skip empty rows
        if row.iter().all(|cell| cell.is_empty()) {
            continue;
        }

        match parse_row(row, &mapping) {
            Ok(Some(transaction)) => {
                transactions.push(transaction);
            }
            Ok(None) => {
                // Skip row (e.g., subtotal, summary row)
                continue;
            }
            Err(e) => {
                warn!("Skipping row {}: {}", idx + 1, e);
                continue;
            }
        }
    }

    info!("Successfully parsed {} transactions", transactions.len());
    Ok(transactions)
}

/// Find the sheet containing trading data
fn find_trading_sheet(workbook: &Xlsx<std::io::BufReader<std::fs::File>>) -> Result<String> {
    let sheet_names = workbook.sheet_names();

    // Look for common sheet names
    let patterns = ["negociação", "negociacao", "ativos", "trading", "trades"];

    for pattern in &patterns {
        for name in &sheet_names {
            if name.to_lowercase().contains(pattern) {
                return Ok(name.clone());
            }
        }
    }

    // If no match, try the first sheet
    sheet_names
        .first()
        .cloned()
        .ok_or_else(|| anyhow!("No sheets found in workbook"))
}

/// Parse a single row into a RawTransaction
fn parse_row(row: &[Data], mapping: &ColumnMapping) -> Result<Option<RawTransaction>> {
    // Get ticker - if empty, skip row
    let ticker_cell = row.get(mapping.ticker.unwrap()).ok_or_else(|| anyhow!("Missing ticker column"))?;
    let ticker = ticker_cell.to_string().trim().to_uppercase();

    if ticker.is_empty() {
        return Ok(None);  // Skip empty rows
    }

    // Parse date
    let date_cell = row.get(mapping.date.unwrap()).ok_or_else(|| anyhow!("Missing date column"))?;
    let trade_date = parse_date(date_cell)?;

    // Parse transaction type
    let type_cell = row.get(mapping.transaction_type.unwrap()).ok_or_else(|| anyhow!("Missing transaction type"))?;
    let transaction_type = type_cell.to_string().trim().to_uppercase();

    // Parse quantity
    let qty_cell = row.get(mapping.quantity.unwrap()).ok_or_else(|| anyhow!("Missing quantity"))?;
    let quantity = parse_decimal(qty_cell)?;

    // Parse price
    let price_cell = row.get(mapping.price.unwrap()).ok_or_else(|| anyhow!("Missing price"))?;
    let price = parse_decimal(price_cell)?;

    // Parse total (or calculate if not present)
    let total = if let Some(total_idx) = mapping.total {
        if let Some(total_cell) = row.get(total_idx) {
            parse_decimal(total_cell).unwrap_or(quantity * price)
        } else {
            quantity * price
        }
    } else {
        quantity * price
    };

    // Parse fees (optional)
    let fees = if let Some(fees_idx) = mapping.fees {
        if let Some(fees_cell) = row.get(fees_idx) {
            parse_decimal(fees_cell).unwrap_or(Decimal::ZERO)
        } else {
            Decimal::ZERO
        }
    } else {
        Decimal::ZERO
    };

    // Parse market (optional)
    let market = if let Some(market_idx) = mapping.market {
        row.get(market_idx).map(|cell| cell.to_string())
    } else {
        None
    };

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

/// Parse date from various formats
fn parse_date(cell: &Data) -> Result<NaiveDate> {
    match cell {
        Data::DateTime(dt) => {
            // ExcelDateTime already has methods to convert
            // Get the underlying float value and convert to days
            let days_since_epoch = dt.as_f64().floor() as i64;
            let excel_epoch = NaiveDate::from_ymd_opt(1899, 12, 30)
                .ok_or_else(|| anyhow!("Invalid Excel epoch"))?;
            excel_epoch
                .checked_add_signed(chrono::Duration::days(days_since_epoch))
                .ok_or_else(|| anyhow!("Date overflow"))
        }
        _ => {
            // Try parsing as string
            let date_str = cell.to_string();

            // Try common Brazilian formats: DD/MM/YYYY, DD-MM-YYYY
            if let Ok(date) = NaiveDate::parse_from_str(&date_str, "%d/%m/%Y") {
                return Ok(date);
            }
            if let Ok(date) = NaiveDate::parse_from_str(&date_str, "%d-%m-%Y") {
                return Ok(date);
            }
            if let Ok(date) = NaiveDate::parse_from_str(&date_str, "%Y-%m-%d") {
                return Ok(date);
            }

            Err(anyhow!("Could not parse date: {}", date_str))
        }
    }
}

/// Parse decimal from cell (handles numbers, strings with Brazilian format)
fn parse_decimal(cell: &Data) -> Result<Decimal> {
    match cell {
        Data::Int(i) => Ok(Decimal::from(*i)),
        Data::Float(f) => Decimal::from_f64_retain(*f)
            .ok_or_else(|| anyhow!("Invalid decimal: {}", f)),
        _ => {
            // Try parsing as string
            let text = cell.to_string()
                .replace("R$", "")
                .replace(" ", "")
                .replace(".", "")  // Remove thousand separators
                .replace(",", ".");  // Replace decimal comma with dot

            Decimal::from_str(&text)
                .context("Failed to parse decimal")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalized_ticker_fractional() {
        let tx = RawTransaction {
            ticker: "AMBP3F".to_string(),
            transaction_type: "C".to_string(),
            trade_date: NaiveDate::from_ymd_opt(2024, 7, 22).unwrap(),
            quantity: Decimal::from(10),
            price: Decimal::from(1),
            fees: Decimal::ZERO,
            total: Decimal::from(10),
            market: Some("Mercado Fracionário".to_string()),
        };

        assert_eq!(tx.normalized_ticker(), "AMBP3");
    }

    #[test]
    fn test_normalized_ticker_non_fractional() {
        let tx = RawTransaction {
            ticker: "AMBP3F".to_string(),
            transaction_type: "C".to_string(),
            trade_date: NaiveDate::from_ymd_opt(2024, 7, 22).unwrap(),
            quantity: Decimal::from(10),
            price: Decimal::from(1),
            fees: Decimal::ZERO,
            total: Decimal::from(10),
            market: Some("Mercado à Vista".to_string()),
        };

        assert_eq!(tx.normalized_ticker(), "AMBP3F");
    }

    #[test]
    fn test_parse_decimal_brazilian_format() {
        // Brazilian format: 1.234,56 = 1234.56
        let result = parse_decimal(&Data::String("1.234,56".to_string())).unwrap();
        assert_eq!(result, Decimal::from_str("1234.56").unwrap());
    }

    #[test]
    fn test_parse_date_brazilian_format() {
        let result = parse_date(&Data::String("15/03/2025".to_string())).unwrap();
        assert_eq!(result, NaiveDate::from_ymd_opt(2025, 3, 15).unwrap());
    }
}
