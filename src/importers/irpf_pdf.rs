// IRPF PDF Parser - Extract stock positions from Brazilian tax declarations
//
// Parses the "DECLARAÇÃO DE BENS E DIREITOS" section (Code 31 - stocks)
// and creates opening position transactions with cost basis.

use anyhow::{anyhow, Context, Result};
use chrono::NaiveDate;
use pdf_extract::extract_text;
use regex::Regex;
use rust_decimal::Decimal;
use std::path::Path;
use std::str::FromStr;
use tracing::{info, warn};

use crate::db::models::{Transaction, TransactionType};

/// Represents a stock position extracted from IRPF PDF
#[derive(Debug, Clone)]
pub struct IrpfPosition {
    pub ticker: String,
    pub year: i32,             // IRPF year (e.g., 2018)
    pub quantity: Decimal,     // Shares held at year-end
    pub total_cost: Decimal,   // Cost basis from IRPF
    pub average_cost: Decimal, // Calculated: total_cost / quantity
    #[allow(dead_code)]
    pub cnpj: Option<String>,
}

/// Represents loss carryforward amounts extracted from IRPF PDF
#[derive(Debug, Clone)]
pub struct IrpfLossCarryforward {
    #[allow(dead_code)]
    pub year: i32, // IRPF year (e.g., 2024)
    pub stock_swing_loss: Decimal, // Ações (Operações Comuns) losses
    pub stock_day_loss: Decimal,   // Ações (Day Trade) losses
    pub fii_fiagro_loss: Decimal,  // FII/FIAGRO losses (combined)
}

impl IrpfPosition {
    /// Convert IRPF position to a virtual opening BUY transaction
    pub fn to_opening_transaction(&self, asset_id: i64) -> Result<Transaction> {
        let trade_date = NaiveDate::from_ymd_opt(self.year, 12, 31)
            .ok_or_else(|| anyhow!("Invalid year: {}", self.year))?;

        Ok(Transaction {
            id: None,
            asset_id,
            trade_date,
            settlement_date: Some(trade_date),
            transaction_type: TransactionType::Buy,
            quantity: self.quantity,
            price_per_unit: self.average_cost,
            total_cost: self.total_cost,
            fees: Decimal::ZERO,
            is_day_trade: false,
            quota_issuance_date: None,
            notes: Some(format!("Opening position from IRPF {} year-end", self.year)),
            source: "IRPF_PDF".to_string(),
            created_at: chrono::Utc::now(),
        })
    }
}

/// Parse IRPF PDF and extract stock positions for a specific year
pub fn parse_irpf_pdf<P: AsRef<Path>>(path: P, year: i32) -> Result<Vec<IrpfPosition>> {
    let path = path.as_ref();
    info!("Parsing IRPF PDF: {:?} for year {}", path, year);

    // Extract text from PDF
    let text = extract_text(path).context("Failed to extract text from PDF")?;

    // Find the "DECLARAÇÃO DE BENS E DIREITOS" section
    if !text.contains("DECLARAÇÃO DE BENS E DIREITOS")
        && !text.contains("DECLARACAO DE BENS E DIREITOS")
    {
        return Err(anyhow!(
            "PDF does not contain 'DECLARAÇÃO DE BENS E DIREITOS' section. \
             This may not be an IRPF PDF."
        ));
    }

    info!("Found 'DECLARAÇÃO DE BENS E DIREITOS' section");

    // Parse positions from the text
    parse_positions_from_text(&text, year)
}

/// Parse IRPF PDF and extract loss carryforward amounts for a specific year
pub fn parse_irpf_pdf_losses<P: AsRef<Path>>(path: P, year: i32) -> Result<IrpfLossCarryforward> {
    let path = path.as_ref();
    info!("Parsing IRPF PDF losses: {:?} for year {}", path, year);

    // Extract text from PDF
    let text = extract_text(path).context("Failed to extract text from PDF")?;

    // Find loss carryforward data from two sections:
    // 1. "RENDA VARIÁVEL - OPERAÇÕES COMUNS/DAYTRADE - TITULAR" for stocks
    // 2. "FUNDOS DE INVESTIMENTO IMOBILIÁRIO OU NAS CADEIAS PRODUTIVAS AGROINDUSTRIAIS - TITULAR" for FII/FIAGRO

    let stock_swing_loss = extract_stock_losses(&text, "OPERAÇÕES COMUNS");
    let stock_day_loss = extract_stock_losses(&text, "OPERAÇÕES DAY-TRADE");
    let fii_fiagro_loss = extract_fii_fiagro_losses(&text);

    Ok(IrpfLossCarryforward {
        year,
        stock_swing_loss: stock_swing_loss.unwrap_or(Decimal::ZERO),
        stock_day_loss: stock_day_loss.unwrap_or(Decimal::ZERO),
        fii_fiagro_loss: fii_fiagro_loss.unwrap_or(Decimal::ZERO),
    })
}

/// Extract stock losses from the "RENDA VARIÁVEL - OPERAÇÕES COMUNS/DAYTRADE" section
fn extract_stock_losses(text: &str, operation_type: &str) -> Option<Decimal> {
    // Find the section for this operation type
    let section_start = text.find("RENDA VARIÁVEL - OPERAÇÕES COMUNS")?;

    // Get text from section start to a reasonable endpoint
    let search_end = (section_start + 50000).min(text.len());
    let search_text = &text[section_start..search_end];

    // Look for "GANHOS LÍQUIDOS OU PERDAS - DEZ" (December section in stock losses)
    // Then find "Prejuízo a compensar" with its value
    let dezembro_marker = "GANHOS LÍQUIDOS OU PERDAS - DEZ";
    let dezembro_start = search_text.find(dezembro_marker)?;
    let dezembro_section = &search_text[dezembro_start..];

    // Find the next section marker or end
    let next_section = dezembro_section
        .find("RENDA VARIÁVEL - OPERAÇÕES COMUNS/DAYTRADE - DEPENDENTES")
        .unwrap_or(dezembro_section.len());
    let target_text = &dezembro_section[..next_section];

    // Extract "Prejuízo a compensar" line which has format:
    // "Prejuízo a compensar                                                                          5,704.00                             0.00"
    // We want the first number (for OPERAÇÕES COMUNS) or second number (for OPERAÇÕES DAY-TRADE)
    let loss_regex = Regex::new(r"Prejuízo a compensar\s+([\d,\.]+)\s+([\d,\.]+)").ok()?;

    let captures = loss_regex.captures(target_text)?;

    // For OPERAÇÕES COMUNS, use first value; for DAY-TRADE, use second
    let loss_str = if operation_type.contains("COMUNS") {
        captures.get(1)?.as_str()
    } else {
        captures.get(2)?.as_str()
    };

    parse_brazilian_decimal(loss_str).ok()
}

/// Extract FII/FIAGRO losses from the "FUNDOS DE INVESTIMENTO IMOBILIÁRIO..." section
fn extract_fii_fiagro_losses(text: &str) -> Option<Decimal> {
    // Find the FII/FIAGRO section
    let section_marker = "FUNDOS DE INVESTIMENTO IMOBILIÁRIO";
    let section_start = text.find(section_marker)?;

    // Get text from section start
    let search_end = (section_start + 50000).min(text.len());
    let search_text = &text[section_start..search_end];

    // Find December section ("MÊS" header followed by months and then "Dezembro" column)
    let dezembro_marker = "Dezembro";
    let dezembro_pos = search_text.find(dezembro_marker)?;

    // Go back to find the "PREJUÍZO A COMPENSAR" line in the December column
    let search_back_start = dezembro_pos.saturating_sub(2000);
    let search_forward = (dezembro_pos + 2000).min(search_text.len());
    let relevant_section = &search_text[search_back_start..search_forward];

    // Find "PREJUÍZO A COMPENSAR" followed by numbers across months
    // The December value is the last number in that row
    let loss_regex =
        Regex::new(r"PREJUÍZO A COMPENSAR\s+(?:[\d,\.]+\s+)*?([\d,\.]+)\s*(?:\n|$)").ok()?;

    let captures = loss_regex.captures(relevant_section)?;
    let loss_str = captures.get(1)?.as_str();

    parse_brazilian_decimal(loss_str).ok()
}

/// Parse positions from extracted PDF text
fn parse_positions_from_text(text: &str, year: i32) -> Result<Vec<IrpfPosition>> {
    let mut positions = Vec::new();

    // Split text into sections by "CÓDIGO:" markers
    // Each Code 31 (stocks/BDRs) or Code 73 (FIIs) entry is a separate section
    let sections: Vec<&str> = text.split("CÓDIGO").collect();

    // Compile once, reuse across sections
    let line_regex = Regex::new(r"(?:^|\n)(31|73)\s+([^\n]+)")?;

    for section in sections {
        // Check if this section contains any Code 31/73 entries (stocks/FIIs)
        // The section format is: "DISCRIMINAÇÃO SITUAÇÃO EM\n\n31/12/...\n\n31 TICKER..."
        // A section may contain multiple "31 " lines
        if !section.contains("\n31 ")
            && !section.starts_with("31 ")
            && !section.contains("\n73 ")
            && !section.starts_with("73 ")
        {
            continue;
        }

        // Find all lines starting with "31 " or "73 " in this section
        // Use the precompiled regex to find all occurrences
        for captures in line_regex.captures_iter(section) {
            let line = captures.get(2).map(|m| m.as_str()).unwrap_or("");

            match parse_code_31_line(line, year) {
                Ok(Some(position)) => {
                    info!(
                        "Extracted position: {} {} shares @ R${:.2} = R${:.2}",
                        position.ticker,
                        position.quantity,
                        position.average_cost,
                        position.total_cost
                    );
                    positions.push(position);
                }
                Ok(None) => {
                    // Entry didn't match target year, skip silently
                }
                Err(e) => {
                    warn!(
                        "Failed to parse Code 31 line '{}': {}",
                        if line.len() > 50 { &line[..50] } else { line },
                        e
                    );
                    // Continue processing other entries
                }
            }
        }
    }

    info!(
        "Extracted {} positions from IRPF for year {}",
        positions.len(),
        year
    );

    if positions.is_empty() {
        warn!(
            "No positions found for year {}. Check that the PDF is for the correct year.",
            year
        );
    }

    Ok(positions)
}

/// Parse a single Code 31/73 line (stock/FII position)
/// Takes the content of a line starting with "31 " or "73 " (without the prefix)
fn parse_code_31_line(discrim: &str, target_year: i32) -> Result<Option<IrpfPosition>> {
    // Format: "TICKER quantity (year) quantity (year) value1 value2"
    // Example: "CYRE3 600 (2017) 131 (2016) 7,611.60 0.00"

    // Extract ticker using pattern: TICKER quantity (year)
    // Examples: "ITSA4 1926 (2018)", "ANIM3 500 (2017) 1300 (2018)", "A1MD34 100 (2018)"
    // Supports: regular stocks (4 letters + digits), BDRs (1+ letters + digits), units (ending in 11)
    let ticker_regex = Regex::new(r"^([A-Z]\d?[A-Z]{0,3}\d{1,2}[A-Z]?)")?;
    let ticker = ticker_regex
        .captures(discrim)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
        .ok_or_else(|| anyhow!("Could not extract ticker from: {}", discrim))?;

    // Extract quantity for the target year
    // Pattern: "number (year)"
    let qty_regex = Regex::new(&format!(r"(\d+(?:\.\d+)?)\s*\({}\)", target_year))?;
    let quantity_str = match qty_regex.captures(discrim) {
        Some(cap) => cap.get(1).map(|m| m.as_str()).unwrap(),
        None => {
            // This entry doesn't have data for the target year
            return Ok(None);
        }
    };
    let quantity = Decimal::from_str(quantity_str).context("Failed to parse quantity")?;

    // Extract cost basis for the target year
    // The discriminação line ends with two values: "value1 value2"
    // These correspond to consecutive years in chronological order
    // Example: "ITSA4 1926 (2018), 800 (2017) 7,674.47 20,245.73"
    //          7,674.47 is for 2017, 20,245.73 is for 2018
    // The years in the line are in descending order, but values are in ascending order

    // First, find all years mentioned in the line to determine the latest year
    let year_regex = Regex::new(r"\((\d{4})\)")?;
    let years: Vec<i32> = year_regex
        .captures_iter(discrim)
        .filter_map(|cap| cap.get(1))
        .filter_map(|m| m.as_str().parse::<i32>().ok())
        .collect();

    let latest_year = years
        .iter()
        .max()
        .copied()
        .ok_or_else(|| anyhow!("No years found in discriminação"))?;

    // Extract the last two decimal values from the discriminação line
    let values_regex = Regex::new(r"([0-9.,]+)\s+([0-9.,]+)\s*$")?;
    let captures = values_regex
        .captures(discrim)
        .ok_or_else(|| anyhow!("Could not find cost values in discriminação"))?;

    // Determine which value to use based on target year
    // value1 is for (latest_year - 1), value2 is for latest_year
    let cost_str = if target_year == latest_year {
        captures.get(2) // Second value for latest year
    } else if target_year == latest_year - 1 {
        captures.get(1) // First value for previous year
    } else {
        return Ok(None); // Target year not covered by these values
    }
    .map(|m| m.as_str())
    .ok_or_else(|| anyhow!("Could not extract cost for year {}", target_year))?;

    // Parse cost (Brazilian format: 1.234,56 → 1234.56)
    let total_cost = parse_brazilian_decimal(cost_str)?;

    // Calculate average cost
    let average_cost = if quantity > Decimal::ZERO {
        total_cost / quantity
    } else {
        return Err(anyhow!("Invalid quantity: {}", quantity));
    };

    // CNPJ is not in the line itself, so we don't extract it
    let cnpj = None;

    Ok(Some(IrpfPosition {
        ticker,
        year: target_year,
        quantity,
        total_cost,
        average_cost,
        cnpj,
    }))
}

/// Parse decimal format - handles both Brazilian (1.234,56) and international (1,234.56) formats
fn parse_brazilian_decimal(s: &str) -> Result<Decimal> {
    // Determine format based on which separator appears last
    let last_comma = s.rfind(',');
    let last_dot = s.rfind('.');

    let normalized = match (last_comma, last_dot) {
        (Some(comma_pos), Some(dot_pos)) => {
            if comma_pos > dot_pos {
                // Brazilian format: "1.234,56" - dot is thousands, comma is decimal
                s.replace('.', "").replace(',', ".")
            } else {
                // International format: "1,234.56" - comma is thousands, dot is decimal
                s.replace(',', "")
            }
        }
        (Some(_), None) => {
            // Only comma: assume Brazilian decimal "1234,56"
            s.replace(',', ".")
        }
        (None, Some(_)) => {
            // Only dot: assume international decimal "1234.56"
            s.to_string()
        }
        (None, None) => {
            // No separators: "1234"
            s.to_string()
        }
    };

    Decimal::from_str(&normalized).context(format!("Failed to parse decimal: {}", s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_brazilian_decimal() {
        assert_eq!(
            parse_brazilian_decimal("1.234,56").unwrap(),
            Decimal::new(123456, 2)
        );
        assert_eq!(
            parse_brazilian_decimal("20,245.73").unwrap(),
            Decimal::new(2024573, 2)
        );
        assert_eq!(
            parse_brazilian_decimal("100").unwrap(),
            Decimal::new(100, 0)
        );
    }

    #[test]
    fn test_ticker_extraction() {
        let discrim = "ITSA4 1926 (2018), 800 (2017)";
        let regex = Regex::new(r"^([A-Z]{4}\d{1,2}[A-Z]?)").unwrap();
        let ticker = regex.captures(discrim).unwrap().get(1).unwrap().as_str();
        assert_eq!(ticker, "ITSA4");
    }

    #[test]
    fn test_quantity_extraction() {
        let discrim = "ITSA4 1926 (2018), 800 (2017)";
        let regex = Regex::new(r"(\d+)\s*\(2018\)").unwrap();
        let qty = regex.captures(discrim).unwrap().get(1).unwrap().as_str();
        assert_eq!(qty, "1926");
    }

    #[test]
    fn test_parse_code_31_line_simple() {
        // Simple case: single year with values
        let line = "ITSA4 1926 (2018) 7,674.47 20,245.73";
        let result = parse_code_31_line(line, 2018).unwrap().unwrap();

        assert_eq!(result.ticker, "ITSA4");
        assert_eq!(result.quantity, Decimal::new(1926, 0));
        assert_eq!(result.total_cost, Decimal::new(2024573, 2)); // 20245.73
        assert_eq!(
            result.average_cost,
            Decimal::new(2024573, 2) / Decimal::new(1926, 0)
        );
    }

    #[test]
    fn test_parse_code_31_line_multiple_years() {
        // Multiple years: should extract only the target year
        let line = "ITSA4 1926 (2018), 800 (2017), 706 (2016) 7,674.47 20,245.73";
        let result = parse_code_31_line(line, 2018).unwrap().unwrap();

        assert_eq!(result.ticker, "ITSA4");
        assert_eq!(result.quantity, Decimal::new(1926, 0)); // 2018 quantity
        assert_eq!(result.total_cost, Decimal::new(2024573, 2));
    }

    #[test]
    fn test_parse_code_31_line_different_year() {
        // Request 2017 data from line with multiple years
        let line = "ITSA4 1926 (2018), 800 (2017), 706 (2016) 7,674.47 20,245.73";
        let result = parse_code_31_line(line, 2017).unwrap().unwrap();

        assert_eq!(result.ticker, "ITSA4");
        assert_eq!(result.quantity, Decimal::new(800, 0)); // 2017 quantity
        assert_eq!(result.total_cost, Decimal::new(767447, 2)); // First value is for 2017
    }

    #[test]
    fn test_parse_code_31_line_wrong_year() {
        // Request year not present in data
        let line = "ITSA4 1926 (2018), 800 (2017) 7,674.47 20,245.73";
        let result = parse_code_31_line(line, 2019).unwrap();

        assert!(result.is_none()); // Should return None for missing year
    }

    #[test]
    fn test_parse_code_31_line_unit() {
        // Unit (SAPR11) - uses "11" suffix but is actually a stock unit, not a FII
        let line = "SAPR11 300 (2018) 15,000.00 15,861.86";
        let result = parse_code_31_line(line, 2018).unwrap().unwrap();

        assert_eq!(result.ticker, "SAPR11");
        assert_eq!(result.quantity, Decimal::new(300, 0));
    }

    #[test]
    fn test_parse_code_73_line_fii() {
        // FII entry (Code 73 in IRPF) uses same line structure
        let line = "BRCR11 113 (2019) 0.00 10,934.90";
        let result = parse_code_31_line(line, 2019).unwrap().unwrap();

        assert_eq!(result.ticker, "BRCR11");
        assert_eq!(result.quantity, Decimal::new(113, 0));
        assert_eq!(result.total_cost, Decimal::new(1093490, 2));
    }

    #[test]
    fn test_parse_code_31_line_zero_value() {
        // Position sold (zero value at year end)
        let line = "CYRE3 600 (2017) 131 (2016) 7,611.60 0.00";
        let result = parse_code_31_line(line, 2018);

        // Should return None since 2018 not in the line
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn test_parse_code_31_line_bdr() {
        // BDR ticker (ends with F or different pattern)
        let line = "A1MD34 100 (2018) 0.00 5,000.00";
        let result = parse_code_31_line(line, 2018).unwrap().unwrap();

        assert_eq!(result.ticker, "A1MD34");
        assert_eq!(result.quantity, Decimal::new(100, 0));
        assert_eq!(result.total_cost, Decimal::new(500000, 2));
    }

    #[test]
    fn test_decimal_formats() {
        // Test various decimal formats
        assert_eq!(
            parse_brazilian_decimal("20,245.73").unwrap(),
            Decimal::new(2024573, 2)
        );
        assert_eq!(
            parse_brazilian_decimal("20.245,73").unwrap(),
            Decimal::new(2024573, 2)
        );
        assert_eq!(
            parse_brazilian_decimal("1234.56").unwrap(),
            Decimal::new(123456, 2)
        );
        assert_eq!(
            parse_brazilian_decimal("1234,56").unwrap(),
            Decimal::new(123456, 2)
        );
        assert_eq!(
            parse_brazilian_decimal("1,234,567.89").unwrap(),
            Decimal::new(123456789, 2)
        );
    }
}
