use anyhow::{anyhow, Result};
use chrono::{Datelike, NaiveDate};
use rust_decimal::Decimal;
use std::str::FromStr;

const TESOURO_PREFIX: &str = "TESOURO";

pub fn ticker_from_name(name: &str) -> Option<String> {
    let trimmed = name.trim();
    if !trimmed.to_ascii_lowercase().starts_with("tesouro ") {
        return None;
    }

    let year = trimmed.split_whitespace().last()?;
    let year = normalize_year_token(year)?;
    let bond_type = normalize_bond_type_from_text(trimmed)?;
    let has_juros = has_juros_semestrais(trimmed);

    Some(build_ticker(&bond_type, has_juros, year))
}

pub fn ticker_from_type_and_maturity(tipo: &str, maturity: NaiveDate) -> Option<String> {
    let trimmed = tipo.trim();
    let bond_type = normalize_bond_type_from_text(trimmed)?;
    let has_juros = has_juros_semestrais(trimmed);
    let year = if bond_type == "RENDA" {
        let adjusted_year = maturity.year() - 19;
        adjusted_year.to_string()
    } else {
        maturity.year().to_string()
    };

    Some(build_ticker(&bond_type, has_juros, &year))
}

pub fn parse_decimal_br(input: &str) -> Result<Decimal> {
    let cleaned = input
        .trim()
        .replace('.', "")
        .replace(',', ".")
        .replace('%', "");
    if cleaned.is_empty() {
        return Err(anyhow!("Empty decimal input"));
    }
    Decimal::from_str(&cleaned).map_err(|err| anyhow!("Invalid decimal '{}': {}", input, err))
}

#[allow(dead_code)]
pub fn extract_rate_percent(input: &str) -> Option<Decimal> {
    let mut last_number = String::new();
    let mut current = String::new();
    for ch in input.chars() {
        if ch.is_ascii_digit() || ch == ',' || ch == '.' {
            current.push(ch);
        } else if !current.is_empty() {
            last_number = current.clone();
            current.clear();
        }
    }
    if !current.is_empty() {
        last_number = current;
    }

    if last_number.is_empty() {
        return None;
    }

    parse_decimal_br(&last_number).ok()
}

fn normalize_bond_type_from_text(raw: &str) -> Option<String> {
    let normalized = raw.to_ascii_lowercase();
    if normalized.contains("ipca") {
        return Some("IPCA".to_string());
    }
    if normalized.contains("igpm") {
        return Some("IGPM".to_string());
    }
    if normalized.contains("selic") {
        return Some("SELIC".to_string());
    }
    if normalized.contains("prefixado") {
        return Some("PREFIXADO".to_string());
    }
    if normalized.contains("renda+") || normalized.contains("renda") {
        return Some("RENDA".to_string());
    }
    if normalized.contains("educa+") || normalized.contains("educa") {
        return Some("EDUCA".to_string());
    }

    raw.split_whitespace()
        .find(|part| !part.eq_ignore_ascii_case("tesouro"))
        .map(|part| part.replace('+', "").to_ascii_uppercase())
}

fn has_juros_semestrais(input: &str) -> bool {
    input.to_ascii_lowercase().contains("juros semestrais")
}

fn build_ticker(bond_type: &str, has_juros: bool, year: &str) -> String {
    if has_juros {
        format!("{}_{}_JUROS_{}", TESOURO_PREFIX, bond_type, year).to_ascii_uppercase()
    } else {
        format!("{}_{}_{}", TESOURO_PREFIX, bond_type, year).to_ascii_uppercase()
    }
}

fn normalize_year_token(token: &str) -> Option<&str> {
    let year_part = token
        .rsplit('/')
        .next()
        .unwrap_or(token)
        .trim()
        .trim_matches(|c: char| !c.is_ascii_digit());
    if year_part.is_empty() {
        None
    } else {
        Some(year_part)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    #[test]
    fn test_ticker_from_name_with_juros() {
        let ticker = ticker_from_name("Tesouro IPCA+ com Juros Semestrais 2035");
        assert_eq!(ticker, Some("TESOURO_IPCA_JUROS_2035".to_string()));
    }

    #[test]
    fn test_ticker_from_name_without_juros() {
        let ticker = ticker_from_name("Tesouro IPCA+ 2035");
        assert_eq!(ticker, Some("TESOURO_IPCA_2035".to_string()));
    }

    #[test]
    fn test_ticker_from_name_month_year() {
        let ticker = ticker_from_name("Tesouro Prefixado 01/2005");
        assert_eq!(ticker, Some("TESOURO_PREFIXADO_2005".to_string()));
    }

    #[test]
    fn test_ticker_from_type_and_maturity() {
        let maturity = NaiveDate::from_ymd_opt(2040, 8, 15).unwrap();
        let ticker = ticker_from_type_and_maturity("Tesouro IPCA+", maturity);
        assert_eq!(ticker, Some("TESOURO_IPCA_2040".to_string()));
    }

    #[test]
    fn test_ticker_from_name_renda() {
        let ticker = ticker_from_name("Tesouro Renda+ Aposentadoria Extra 2030");
        assert_eq!(ticker, Some("TESOURO_RENDA_2030".to_string()));
    }

    #[test]
    fn test_ticker_from_name_educa() {
        let ticker = ticker_from_name("Tesouro Educa+ 2029");
        assert_eq!(ticker, Some("TESOURO_EDUCA_2029".to_string()));
    }

    #[test]
    fn test_ticker_from_name_selic() {
        let ticker = ticker_from_name("Tesouro Selic 2026");
        assert_eq!(ticker, Some("TESOURO_SELIC_2026".to_string()));
    }

    #[test]
    fn test_ticker_from_type_igpm() {
        let maturity = NaiveDate::from_ymd_opt(2031, 1, 1).unwrap();
        let ticker = ticker_from_type_and_maturity("Tesouro IGPM+ com Juros Semestrais", maturity);
        assert_eq!(ticker, Some("TESOURO_IGPM_JUROS_2031".to_string()));
    }

    #[test]
    fn test_ticker_from_type_renda_maturity_offset() {
        let maturity = NaiveDate::from_ymd_opt(2079, 12, 15).unwrap();
        let ticker = ticker_from_type_and_maturity("Tesouro Renda+ Aposentadoria Extra", maturity);
        assert_eq!(ticker, Some("TESOURO_RENDA_2060".to_string()));
    }

    #[test]
    fn test_parse_decimal_br() {
        let value = parse_decimal_br("1.234,56").unwrap();
        assert_eq!(value, Decimal::from_str("1234.56").unwrap());
    }

    #[test]
    fn test_extract_rate_percent() {
        let value = extract_rate_percent("SELIC + 0,0321%").unwrap();
        assert_eq!(value, Decimal::from_str("0.0321").unwrap());
    }
}
