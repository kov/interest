//! Movimentacao Excel file importer
//!
//! Parses B3 "Movimentação" files which contain comprehensive account movement history:
//! - Trade transactions (buy/sell)
//! - **Term contracts** (compra a termo) and their liquidations
//! - Corporate actions (splits, bonuses, mergers)
//! - Income events (dividends, yields, amortization)
//! - Stock lending, subscription rights, and more
//!
//! ## Important: Term Contract Handling
//!
//! Term contracts have special ticker behavior:
//! - **Purchase**: Ticker has 'T' suffix (e.g., ANIM3T)
//! - **Liquidation**: When term expires, 'T' is dropped (e.g., ANIM3T → ANIM3)
//! - The liquidation entry in movimentacao shows the BASE ticker (ANIM3)
//! - Cost basis from ANIM3T purchase should transfer to ANIM3 holding
//!
//! This is tracked via transaction notes and should be handled during position calculations.

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
    /// Extract ticker from product name
    ///
    /// Handles multiple formats:
    /// - Standard: "PETR4 - PETROBRAS" -> "PETR4"
    /// - Debentures: "DEB - ELET23 - COMPANY" -> "ELET23"
    /// - CDB: "CDB - CDB92576XY3 - ITAU" -> "CDB92576XY3"
    /// - Tesouro Direto: "Tesouro IPCA+ 2035" -> "TESOURO_IPCA_2035"
    fn extract_ticker(product: &str) -> Option<String> {
        // Handle CDBs: "CDB - CODE" or "CDB - CODE - BANK"
        if product.starts_with("CDB ") {
            let parts: Vec<&str> = product.split(" - ").collect();
            if parts.len() >= 2 {
                // The CDB code is the identifier (e.g., "CDB92576XY3")
                return Some(parts[1].trim().to_uppercase());
            }
        }

        // Handle debentures: "DEB - TICKER - COMPANY"
        if product.starts_with("DEB ") {
            let parts: Vec<&str> = product.split(" - ").collect();
            if parts.len() >= 2 {
                let ticker = parts[1].trim();
                if ticker.len() >= 4 && ticker.len() <= 6
                    && ticker.chars().last().map(|c| c.is_numeric()).unwrap_or(false) {
                    return Some(ticker.to_uppercase());
                }
            }
        }

        // Handle Tesouro Direto (government bonds) - create synthetic ticker from name
        if product.starts_with("Tesouro ") {
            // Extract bond type and year
            // e.g., "Tesouro IPCA+ com Juros Semestrais 2035" -> "TESOURO_IPCA_2035"
            let parts: Vec<&str> = product.split_whitespace().collect();
            if parts.len() >= 2 {
                let bond_type = parts.get(1)?.replace('+', "").replace("com", "");
                let year = parts.last()?;
                let synthetic_ticker = format!("TESOURO_{}_{}", bond_type, year);
                return Some(synthetic_ticker.to_uppercase());
            }
        }

        // Handle CRI (Real Estate Receivables Certificate): "CRI - CODE - COMPANY"
        if product.starts_with("CRI ") {
            let parts: Vec<&str> = product.split(" - ").collect();
            if parts.len() >= 2 {
                return Some(format!("CRI_{}", parts[1].trim()));
            }
        }

        // Handle Options: "Opção de Compra - PETRF407 - PETR"
        if product.starts_with("Opção ") {
            let parts: Vec<&str> = product.split(" - ").collect();
            if parts.len() >= 2 {
                return Some(parts[1].trim().to_uppercase());
            }
        }

        // Handle term contracts with English names: "COMMON STOCK - ANIM3T - ANIM"
        if product.contains("STOCK - ") {
            let parts: Vec<&str> = product.split(" - ").collect();
            if parts.len() >= 2 {
                let ticker = parts[1].trim();
                if ticker.len() >= 4 {
                    return Some(ticker.to_uppercase());
                }
            }
        }

        // Handle term contracts in Portuguese: "Termo de Ação ANIM3 - ANIM3T - ANIM"
        if product.starts_with("Termo de ") {
            let parts: Vec<&str> = product.split(" - ").collect();
            if parts.len() >= 2 {
                let ticker = parts[1].trim();
                if ticker.len() >= 4 {
                    return Some(ticker.to_uppercase());
                }
            }
        }

        // Standard format: "TICKER - COMPANY NAME"
        let parts: Vec<&str> = product.split(&[' ', '-'][..]).collect();

        if let Some(first) = parts.first() {
            let potential_ticker = first.trim();
            // Brazilian tickers are typically 4-6 characters, but ETFs can be longer (up to 9)
            // They must end in a digit
            if potential_ticker.len() >= 4
                && potential_ticker.len() <= 9
                && potential_ticker.chars().last().map(|c| c.is_numeric()).unwrap_or(false) {
                return Some(potential_ticker.to_uppercase());
            }
        }

        debug!("Could not extract ticker from product: '{}'", product);
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

    /// Check if this is a term contract liquidation
    pub fn is_term_liquidation(&self) -> bool {
        self.movement_type == "Liquidação Termo"
    }

    /// Get the term contract ticker (adds 'T' suffix to base ticker)
    /// Used for matching liquidations to their original term purchases
    pub fn get_term_ticker(&self) -> Option<String> {
        if let Some(base_ticker) = &self.ticker {
            Some(format!("{}T", base_ticker))
        } else {
            None
        }
    }

    /// Determine if this is a corporate action
    pub fn is_corporate_action(&self) -> bool {
        matches!(self.movement_type.as_str(),
            "Desdobro" | "Bonificação em Ativos" | "Incorporação" | "Atualização"
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
        let (transaction_type, notes) = match self.movement_type.as_str() {
            "Compra" | "COMPRA/VENDA" | "COMPRA / VENDA" | "COMPRA/VENDA DEFINITIVA/CESSAO" => {
                (TransactionType::Buy, format!("Imported from movimentacao: {}", self.movement_type))
            }
            "Venda" => {
                (TransactionType::Sell, format!("Imported from movimentacao: {}", self.movement_type))
            }
            "Liquidação Termo" => {
                // Term contract liquidation
                // Note: The ticker in the movimentacao file will be the BASE ticker (e.g., ANIM3)
                // but the original purchase would have been with the T suffix (e.g., ANIM3T)
                // When the term expires, the T is dropped and you receive the base asset

                let note = if let Some(ticker) = &self.ticker {
                    format!("Term contract liquidation (original ticker: {}T → {})", ticker, ticker)
                } else {
                    "Term contract liquidation".to_string()
                };

                (TransactionType::Buy, note)
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
            notes: Some(notes),
            source: "MOVIMENTACAO".to_string(),
            created_at: chrono::Utc::now(),
        })
    }

    /// Convert to CorporateAction
    pub fn to_corporate_action(&self, asset_id: i64) -> Result<CorporateAction> {
        let (action_type, ratio_from, ratio_to) = match self.movement_type.as_str() {
            "Desdobro" => {
                // Stock split - need to extract ratio from quantity or notes
                // For now, mark as 1:1; import flow may infer from holdings
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
            "Atualização" => {
                // Position update / subscription credit - infer ratio from holdings later
                (CorporateActionType::Bonus, 1, 1)
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
        // Standard stock tickers
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

        // Term contracts (with T suffix) - need special format
        assert_eq!(
            MovimentacaoEntry::extract_ticker("COMMON STOCK - PETR4T - PETROBRAS"),
            Some("PETR4T".to_string())
        );

        assert_eq!(
            MovimentacaoEntry::extract_ticker("Termo de Ação VALE3 - VALE3T - VALE"),
            Some("VALE3T".to_string())
        );

        // Various formats
        assert_eq!(
            MovimentacaoEntry::extract_ticker("ITSA4-ITAUSA"),
            Some("ITSA4".to_string())
        );

        assert_eq!(
            MovimentacaoEntry::extract_ticker("BBDC3  -  BRADESCO"),
            Some("BBDC3".to_string())
        );

        // FII with different patterns
        assert_eq!(
            MovimentacaoEntry::extract_ticker("HGLG11 - CSHG LOGISTICA FII"),
            Some("HGLG11".to_string())
        );

        // FIAGRO
        assert_eq!(
            MovimentacaoEntry::extract_ticker("TEST32 - TEST FIAGRO"),
            Some("TEST32".to_string())
        );

        // No separator
        assert_eq!(
            MovimentacaoEntry::extract_ticker("MGLU3"),
            Some("MGLU3".to_string())
        );

        // Ticker only (no description)
        assert_eq!(
            MovimentacaoEntry::extract_ticker("WEGE3 "),
            Some("WEGE3".to_string())
        );

        // Multiple spaces around separator (but not leading spaces)
        assert_eq!(
            MovimentacaoEntry::extract_ticker("VALE3   -   VALE"),
            Some("VALE3".to_string())
        );

        // Empty or invalid inputs
        assert_eq!(
            MovimentacaoEntry::extract_ticker(""),
            None
        );

        assert_eq!(
            MovimentacaoEntry::extract_ticker(" - NO TICKER"),
            None
        );

        assert_eq!(
            MovimentacaoEntry::extract_ticker("INVALID"),
            None  // Doesn't end in a digit (required for standard tickers)
        );
    }
}
