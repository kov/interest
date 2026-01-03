use anyhow::{anyhow, Context, Result};
use calamine::{open_workbook, Reader, Xlsx, Data, DataType};
use chrono::NaiveDate;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use std::path::Path;
use std::str::FromStr;
use tracing::{debug, info, warn};

use crate::db::models::{Transaction, TransactionType};
use crate::db::{CorporateAction, CorporateActionType};

/// Parsed movimentacao entry
#[derive(Debug, Clone)]
pub struct MovimentacaoEntry {
    pub direction: String,          // Entrada/Saída (Credito/Debito)
    pub date: NaiveDate,
    pub movement_type: String,      // Compra, Venda, Liquidação Termo, Desdobro, etc.
    pub product: String,            // Full product name with ticker
    pub ticker: Option<String>,     // Extracted ticker
    pub institution: String,
    pub quantity: Option<Decimal>,
    pub unit_price: Option<Decimal>,
    pub operation_value: Option<Decimal>,
}

impl MovimentacaoEntry {
    /// Extract ticker from product name (e.g., "PETR4 - PETROBRAS" -> "PETR4")
    fn extract_ticker(product: &str) -> Option<String> {
        // Ticker is usually before the first space or dash
        let parts: Vec<&str> = product.split(&[' ', '-'][..]).collect();

        if let Some(first) = parts.first() {
            let potential_ticker = first.trim();
            // Brazilian tickers are typically 4-6 characters ending in digit
            if potential_ticker.len() >= 4
                && potential_ticker.len() <= 6
                && potential_ticker.chars().last().map(|c| c.is_numeric()).unwrap_or(false) {
                return Some(potential_ticker.to_uppercase());
            }
        }

        None
    }

    /// Parse a movimentacao entry from a row
    pub fn from_row(row: &[Data]) -> Result<Self> {
        // Column indices based on header:
        // [0] Entrada/Saída, [1] Data, [2] Movimentação, [3] Produto
        // [4] Instituição, [5] Quantidade, [6] Preço unitário, [7] Valor da Operação

        let direction = row.get(0)
            .and_then(|d| d.get_string())
            .ok_or_else(|| anyhow!("Missing direction (Entrada/Saída)"))?
            .to_string();

        let date_str = row.get(1)
            .ok_or_else(|| anyhow!("Missing date"))?
            .to_string();
        let date = parse_date(&date_str)?;

        let movement_type = row.get(2)
            .and_then(|d| d.get_string())
            .ok_or_else(|| anyhow!("Missing movement type"))?
            .to_string();

        let product = row.get(3)
            .and_then(|d| d.get_string())
            .ok_or_else(|| anyhow!("Missing product"))?
            .to_string();

        let ticker = Self::extract_ticker(&product);

        let institution = row.get(4)
            .and_then(|d| d.get_string())
            .unwrap_or("")
            .to_string();

        let quantity = row.get(5)
            .and_then(|d| parse_decimal(d).ok())
            .filter(|q| *q > Decimal::ZERO);

        let unit_price = row.get(6)
            .and_then(|d| parse_decimal(d).ok())
            .filter(|p| *p > Decimal::ZERO);

        let operation_value = row.get(7)
            .and_then(|d| parse_decimal(d).ok());

        Ok(MovimentacaoEntry {
            direction,
            date,
            movement_type,
            product,
            ticker,
            institution,
            quantity,
            unit_price,
            operation_value,
        })
    }

    /// Determine if this is a trade transaction (buy/sell/term)
    pub fn is_trade(&self) -> bool {
        matches!(self.movement_type.as_str(),
            "Compra" | "Venda" | "Liquidação Termo" |
            "COMPRA/VENDA" | "COMPRA / VENDA" | "COMPRA/VENDA DEFINITIVA/CESSAO"
        )
    }

    /// Determine if this is a corporate action
    pub fn is_corporate_action(&self) -> bool {
        matches!(self.movement_type.as_str(),
            "Desdobro" | "Bonificação em Ativos" | "Incorporação"
        )
    }

    /// Determine if this is an income event
    pub fn is_income_event(&self) -> bool {
        matches!(self.movement_type.as_str(),
            "Rendimento" | "Dividendo" | "Juros Sobre Capital Próprio" |
            "Amortização" | "Reembolso" | "AMORTIZAÇÃO" | "PAGAMENTO DE JUROS" |
            "Rendimento - Transferido" | "Dividendo - Transferido" |
            "Juros Sobre Capital Próprio - Transferido"
        )
    }

    /// Convert to Transaction (for trades)
    pub fn to_transaction(&self, asset_id: i64) -> Result<Transaction> {
        let transaction_type = match self.movement_type.as_str() {
            "Compra" | "COMPRA/VENDA" | "COMPRA / VENDA" | "COMPRA/VENDA DEFINITIVA/CESSAO" => {
                TransactionType::Buy
            }
            "Venda" => TransactionType::Sell,
            "Liquidação Termo" => {
                // Term contracts can be buy or sell - check direction
                if self.direction.contains("Debito") || self.direction.contains("Saída") {
                    TransactionType::Buy
                } else {
                    TransactionType::Sell
                }
            }
            _ => return Err(anyhow!("Not a trade movement type: {}", self.movement_type)),
        };

        let quantity = self.quantity
            .ok_or_else(|| anyhow!("Missing quantity for trade"))?;

        let price = self.unit_price
            .ok_or_else(|| anyhow!("Missing unit price for trade"))?;

        let total = self.operation_value
            .unwrap_or_else(|| quantity * price);

        Ok(Transaction {
            id: None,
            asset_id,
            transaction_type,
            trade_date: self.date,
            settlement_date: None,
            quantity,
            price_per_unit: price,
            total_cost: total.abs(),
            fees: Decimal::ZERO,  // Fees not separate in movimentacao file
            is_day_trade: false,
            quota_issuance_date: None,
            notes: Some(format!("Imported from movimentacao: {}", self.movement_type)),
            source: "MOVIMENTACAO".to_string(),
            created_at: chrono::Utc::now(),
        })
    }

    /// Convert to CorporateAction
    pub fn to_corporate_action(&self, asset_id: i64) -> Result<CorporateAction> {
        let (action_type, ratio_from, ratio_to) = match self.movement_type.as_str() {
            "Desdobro" => {
                // Stock split - need to extract ratio from quantity or notes
                // For now, mark as 1:1 and require manual update
                warn!("Desdobro found but ratio unknown, defaulting to 1:1");
                (CorporateActionType::Split, 1, 1)
            }
            "Bonificação em Ativos" => {
                // Bonus shares - extract percentage from quantity
                if let Some(qty) = self.quantity {
                    let bonus_pct = qty.to_i32().unwrap_or(10);
                    (CorporateActionType::Bonus, 100, 100 + bonus_pct)
                } else {
                    (CorporateActionType::Bonus, 100, 110)
                }
            }
            "Incorporação" => {
                // Merger - ratio unknown, use 1:1 as placeholder
                (CorporateActionType::Split, 1, 1)  // May need custom type
            }
            _ => return Err(anyhow!("Not a corporate action: {}", self.movement_type)),
        };

        Ok(CorporateAction {
            id: None,
            asset_id,
            action_type,
            event_date: self.date,
            ex_date: self.date,
            ratio_from,
            ratio_to,
            applied: false,
            source: "MOVIMENTACAO".to_string(),
            notes: Some(format!("{} - {}", self.movement_type, self.product)),
            created_at: chrono::Utc::now(),
        })
    }
}

/// Parse Excel file in Movimentação format
pub fn parse_movimentacao_excel<P: AsRef<Path>>(path: P) -> Result<Vec<MovimentacaoEntry>> {
    info!("Parsing movimentacao Excel file: {:?}", path.as_ref());

    let mut workbook: Xlsx<_> = open_workbook(path.as_ref())
        .context("Failed to open movimentacao Excel file")?;

    let sheet_name = "Movimentação";
    let range = workbook
        .worksheet_range(sheet_name)
        .context(format!("Sheet '{}' not found", sheet_name))?;

    let rows: Vec<_> = range.rows().collect();

    if rows.is_empty() {
        return Err(anyhow!("Empty sheet"));
    }

    // Skip header row
    let mut entries = Vec::new();
    let mut errors = 0;

    for (idx, row) in rows.iter().enumerate().skip(1) {
        match MovimentacaoEntry::from_row(row) {
            Ok(entry) => entries.push(entry),
            Err(e) => {
                debug!("Failed to parse row {}: {}", idx + 1, e);
                errors += 1;
            }
        }
    }

    if errors > 0 {
        warn!("Failed to parse {} rows out of {}", errors, rows.len() - 1);
    }

    info!("Parsed {} movimentacao entries from {} rows", entries.len(), rows.len() - 1);

    Ok(entries)
}

/// Parse date from Brazilian format (DD/MM/YYYY)
fn parse_date(date_str: &str) -> Result<NaiveDate> {
    // Try DD/MM/YYYY format
    if let Some(date) = NaiveDate::parse_from_str(date_str, "%d/%m/%Y").ok() {
        return Ok(date);
    }

    // Try YYYY-MM-DD format
    if let Some(date) = NaiveDate::parse_from_str(date_str, "%Y-%m-%d").ok() {
        return Ok(date);
    }

    Err(anyhow!("Invalid date format: {}", date_str))
}

/// Parse decimal from Data cell
fn parse_decimal(data: &Data) -> Result<Decimal> {
    match data {
        Data::Int(i) => Ok(Decimal::from(*i)),
        Data::Float(f) => Decimal::from_f64_retain(*f)
            .ok_or_else(|| anyhow!("Invalid decimal")),
        Data::String(s) => {
            let cleaned = s
                .replace("R$", "")
                .replace(".", "")
                .replace(",", ".")
                .trim()
                .to_string();

            if cleaned == "-" || cleaned.is_empty() {
                return Err(anyhow!("Empty value"));
            }

            Decimal::from_str(&cleaned)
                .context("Failed to parse decimal")
        }
        _ => Err(anyhow!("Unsupported data type")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_ticker() {
        assert_eq!(
            MovimentacaoEntry::extract_ticker("PETR4 - PETROBRAS"),
            Some("PETR4".to_string())
        );

        assert_eq!(
            MovimentacaoEntry::extract_ticker("MXRF11 - MAXI RENDA FII"),
            Some("MXRF11".to_string())
        );

        assert_eq!(
            MovimentacaoEntry::extract_ticker("LOGG3 - LOG COMMERCIAL PROPERTIES"),
            Some("LOGG3".to_string())
        );
    }
}
