//! Ofertas Públicas Excel file importer
//!
//! Parses B3 "Ofertas Públicas" reports which contain subscription/offer
//! allocations (e.g., tickers with L suffix).

use anyhow::{anyhow, Context, Result};
use calamine::{open_workbook, Data, DataType, Reader, Xlsx};
use chrono::NaiveDate;
use rust_decimal::Decimal;
use std::path::Path;
use std::str::FromStr;
use tracing::info;

use crate::db::models::{Transaction, TransactionType};

/// Parsed ofertas públicas entry
#[derive(Debug, Clone)]
pub struct OfertaPublicaEntry {
    pub date: NaiveDate,
    pub offer: String,
    pub ticker: String,
    pub raw_ticker: String,
    #[allow(dead_code)]
    pub institution: String,
    pub quantity: Decimal,
    pub unit_price: Decimal,
    pub operation_value: Decimal,
}

impl OfertaPublicaEntry {
    /// Normalize tickers like AMBP3L -> AMBP3
    fn normalize_ticker(ticker: &str) -> String {
        let trimmed = ticker.trim();
        if trimmed.len() > 1 && trimmed.ends_with('L') {
            return trimmed.trim_end_matches('L').to_string();
        }
        trimmed.to_string()
    }

    /// Convert to a Buy transaction
    pub fn to_transaction(&self, asset_id: i64) -> Result<Transaction> {
        Ok(Transaction {
            id: None,
            asset_id,
            transaction_type: TransactionType::Buy,
            trade_date: self.date,
            settlement_date: Some(self.date),
            quantity: self.quantity,
            price_per_unit: self.unit_price,
            total_cost: self.operation_value,
            fees: Decimal::ZERO,
            is_day_trade: false,
            quota_issuance_date: None,
            notes: Some(format!(
                "Oferta pública: {} (orig: {})",
                self.offer, self.raw_ticker
            )),
            source: "OFERTAS_PUBLICAS".to_string(),
            created_at: chrono::Utc::now(),
        })
    }
}

/// Parse ofertas públicas Excel file
pub fn parse_ofertas_publicas_excel<P: AsRef<Path>>(path: P) -> Result<Vec<OfertaPublicaEntry>> {
    info!("Parsing ofertas públicas Excel file: {:?}", path.as_ref());

    let mut workbook: Xlsx<_> =
        open_workbook(path).context("Failed to open ofertas públicas Excel file")?;

    let range = workbook
        .worksheet_range("Movimentação")
        .context("Failed to read Movimentação sheet")?;

    let mut rows = range.rows();
    let header: &[Data] = rows.next().ok_or_else(|| anyhow!("Missing header row"))?;

    let mut col_idx = std::collections::HashMap::new();
    for (idx, cell) in header.iter().enumerate() {
        if let Some(name) = cell.get_string() {
            col_idx.insert(name.trim().to_string(), idx);
        }
    }

    let col_date = *col_idx
        .get("Data de liquidação")
        .ok_or_else(|| anyhow!("Missing 'Data de liquidação' column"))?;
    let col_offer = *col_idx
        .get("Oferta")
        .ok_or_else(|| anyhow!("Missing 'Oferta' column"))?;
    let col_ticker = *col_idx
        .get("Código de Negociação")
        .ok_or_else(|| anyhow!("Missing 'Código de Negociação' column"))?;
    let col_institution = *col_idx
        .get("Instituição")
        .ok_or_else(|| anyhow!("Missing 'Instituição' column"))?;
    let col_quantity = *col_idx
        .get("Quantidade")
        .ok_or_else(|| anyhow!("Missing 'Quantidade' column"))?;
    let col_price = *col_idx
        .get("Preço")
        .ok_or_else(|| anyhow!("Missing 'Preço' column"))?;
    let col_value = *col_idx
        .get("Valor")
        .ok_or_else(|| anyhow!("Missing 'Valor' column"))?;

    let mut entries = Vec::new();

    for row in rows {
        let date_str = row
            .get(col_date)
            .and_then(|d| d.get_string())
            .unwrap_or("")
            .to_string();

        if date_str.trim().is_empty() {
            continue;
        }

        let date = parse_date(&date_str)?;
        let offer = row
            .get(col_offer)
            .and_then(|d| d.get_string())
            .unwrap_or("")
            .to_string();
        let raw_ticker = row
            .get(col_ticker)
            .and_then(|d| d.get_string())
            .unwrap_or("")
            .to_string();
        if raw_ticker.trim().is_empty() {
            continue;
        }
        let ticker = OfertaPublicaEntry::normalize_ticker(&raw_ticker);
        let institution = row
            .get(col_institution)
            .and_then(|d| d.get_string())
            .unwrap_or("")
            .to_string();
        let quantity = parse_decimal(
            row.get(col_quantity)
                .ok_or_else(|| anyhow!("Missing quantity"))?,
        )?;
        let unit_price =
            parse_decimal(row.get(col_price).ok_or_else(|| anyhow!("Missing price"))?)?;
        let operation_value =
            parse_decimal(row.get(col_value).ok_or_else(|| anyhow!("Missing value"))?)?;

        entries.push(OfertaPublicaEntry {
            date,
            offer,
            ticker,
            raw_ticker,
            institution,
            quantity,
            unit_price,
            operation_value,
        });
    }

    info!("Parsed {} ofertas públicas entries", entries.len());
    Ok(entries)
}

fn parse_date(date_str: &str) -> Result<NaiveDate> {
    let date = NaiveDate::parse_from_str(date_str.trim(), "%d/%m/%Y")
        .with_context(|| format!("Failed to parse date: {}", date_str))?;
    Ok(date)
}

fn parse_decimal(data: &Data) -> Result<Decimal> {
    match data {
        Data::Float(f) => {
            Decimal::from_str(&f.to_string()).context("Failed to parse float as decimal")
        }
        Data::Int(i) => Ok(Decimal::from(*i)),
        Data::String(s) => {
            let normalized = s.replace('.', "").replace(',', ".");
            Decimal::from_str(normalized.trim()).context("Failed to parse string as decimal")
        }
        _ => Err(anyhow!("Unsupported numeric cell type")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_ticker_suffix_l() {
        assert_eq!(OfertaPublicaEntry::normalize_ticker("AMBP3L"), "AMBP3");
        assert_eq!(OfertaPublicaEntry::normalize_ticker("AMBP3"), "AMBP3");
    }
}
