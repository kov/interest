use anyhow::{anyhow, Context, Result};
use chrono::NaiveDate;
use reqwest::Client;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::db::models::CorporateActionType;

/// Brapi.dev API response for quote
#[derive(Debug, Deserialize)]
struct BrapiQuoteResponse {
    results: Vec<BrapiQuote>,
}

#[derive(Debug, Deserialize)]
struct BrapiQuote {
    #[allow(dead_code)]
    symbol: String,
    #[serde(rename = "regularMarketPrice")]
    regular_market_price: Option<f64>,
    #[serde(rename = "regularMarketOpen")]
    regular_market_open: Option<f64>,
    #[serde(rename = "regularMarketDayHigh")]
    regular_market_day_high: Option<f64>,
    #[serde(rename = "regularMarketDayLow")]
    regular_market_day_low: Option<f64>,
    #[serde(rename = "regularMarketVolume")]
    regular_market_volume: Option<i64>,
    currency: Option<String>,
    #[serde(rename = "dividendsData")]
    dividends_data: Option<DividendsData>,
}

#[derive(Debug, Deserialize)]
struct DividendsData {
    #[serde(rename = "cashDividends")]
    cash_dividends: Option<Vec<CashDividend>>,
    #[serde(rename = "stockDividends")]
    stock_dividends: Option<Vec<StockDividend>>,
}

#[derive(Debug, Deserialize)]
struct CashDividend {
    #[serde(rename = "assetIssued")]
    #[allow(dead_code)]
    asset_issued: String,
    #[serde(rename = "paymentDate")]
    payment_date: String,
    #[serde(rename = "rate")]
    rate: Option<f64>,
    #[serde(rename = "relatedTo")]
    #[allow(dead_code)]
    related_to: Option<String>,
    #[serde(rename = "approvedOn")]
    #[allow(dead_code)]
    approved_on: Option<String>,
    #[serde(rename = "isinCode")]
    #[allow(dead_code)]
    isin_code: Option<String>,
    #[serde(rename = "label")]
    label: Option<String>,
    #[serde(rename = "lastDatePrior")]
    last_date_prior: Option<String>,
    #[serde(rename = "remarks")]
    remarks: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StockDividend {
    #[serde(rename = "assetIssued")]
    #[allow(dead_code)]
    asset_issued: String,
    #[serde(rename = "factor")]
    factor: Option<String>,
    #[serde(rename = "approvedOn")]
    approved_on: String,
    #[serde(rename = "isinCode")]
    #[allow(dead_code)]
    isin_code: Option<String>,
    #[serde(rename = "label")]
    label: Option<String>,
    #[serde(rename = "lastDatePrior")]
    last_date_prior: String,
    #[serde(rename = "remarks")]
    remarks: Option<String>,
}

/// Price data from Brapi
#[derive(Debug, Clone, Serialize)]
pub struct BrapiPriceData {
    pub ticker: String,
    pub price: Decimal,
    pub open: Option<Decimal>,
    pub high: Option<Decimal>,
    pub low: Option<Decimal>,
    pub volume: Option<i64>,
    pub currency: String,
}

/// Corporate action from Brapi
#[derive(Debug, Clone, Serialize)]
pub struct BrapiCorporateAction {
    pub ticker: String,
    pub action_type: CorporateActionType,
    pub approved_date: NaiveDate,
    pub ex_date: NaiveDate,
    pub factor: String, // e.g., "1:2", "10%", etc.
    pub remarks: Option<String>,
}

/// Dividend/Income event from Brapi
#[derive(Debug, Clone, Serialize)]
pub struct BrapiIncomeEvent {
    pub ticker: String,
    pub event_type: String, // "DIVIDEND", "JCP", "RENDIMENTO"
    pub payment_date: NaiveDate,
    pub ex_date: Option<NaiveDate>,
    pub amount: Decimal,
    pub remarks: Option<String>,
}

/// Fetch quote with optional dividend/corporate action data
pub async fn fetch_quote(
    ticker: &str,
    include_dividends: bool,
) -> Result<(
    BrapiPriceData,
    Option<Vec<BrapiCorporateAction>>,
    Option<Vec<BrapiIncomeEvent>>,
)> {
    info!("Fetching quote for {} from Brapi.dev", ticker);

    let client = Client::builder()
        .user_agent("Mozilla/5.0 (compatible; InterestBot/1.0)")
        .build()?;

    let mut url = format!("https://brapi.dev/api/quote/{}", ticker);

    if include_dividends {
        url.push_str("?dividends=true");
    }

    let response = client
        .get(&url)
        .send()
        .await
        .context("Failed to send request to Brapi.dev")?;

    if !response.status().is_success() {
        return Err(anyhow!(
            "Brapi.dev returned error status: {}",
            response.status()
        ));
    }

    let data: BrapiQuoteResponse = response
        .json()
        .await
        .context("Failed to parse Brapi.dev response")?;

    let quote = data
        .results
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("No quote data returned"))?;

    let price_data = BrapiPriceData {
        ticker: ticker.to_string(),
        price: quote
            .regular_market_price
            .and_then(Decimal::from_f64_retain)
            .ok_or_else(|| anyhow!("No price available"))?,
        open: quote.regular_market_open.and_then(Decimal::from_f64_retain),
        high: quote
            .regular_market_day_high
            .and_then(Decimal::from_f64_retain),
        low: quote
            .regular_market_day_low
            .and_then(Decimal::from_f64_retain),
        volume: quote.regular_market_volume,
        currency: quote.currency.unwrap_or_else(|| "BRL".to_string()),
    };

    // Parse corporate actions and dividends if available
    let mut corporate_actions = Vec::new();
    let mut income_events = Vec::new();

    if let Some(dividends_data) = quote.dividends_data {
        // Parse stock dividends (bonificação, desdobramento, grupamento)
        if let Some(stock_divs) = dividends_data.stock_dividends {
            for sd in stock_divs {
                if let Some(action) = parse_stock_dividend(&sd, ticker) {
                    corporate_actions.push(action);
                }
            }
        }

        // Parse cash dividends
        if let Some(cash_divs) = dividends_data.cash_dividends {
            for cd in cash_divs {
                if let Some(event) = parse_cash_dividend(&cd, ticker) {
                    income_events.push(event);
                }
            }
        }
    }

    let actions = if corporate_actions.is_empty() {
        None
    } else {
        Some(corporate_actions)
    };

    let events = if income_events.is_empty() {
        None
    } else {
        Some(income_events)
    };

    Ok((price_data, actions, events))
}

/// Parse stock dividend (bonificação, desdobramento, grupamento)
fn parse_stock_dividend(sd: &StockDividend, ticker: &str) -> Option<BrapiCorporateAction> {
    let label = sd.label.as_deref().unwrap_or("").to_uppercase();
    let remarks = sd.remarks.as_deref().unwrap_or("").to_uppercase();

    // Determine action type from label/remarks
    let action_type = if label.contains("DESDOBRAMENTO") || remarks.contains("DESDOBRAMENTO") {
        CorporateActionType::Split
    } else if label.contains("GRUPAMENTO") || remarks.contains("GRUPAMENTO") {
        CorporateActionType::ReverseSplit
    } else if label.contains("BONIFICAÇÃO")
        || label.contains("BONIFICACAO")
        || remarks.contains("BONIFICAÇÃO")
        || remarks.contains("BONIFICACAO")
    {
        CorporateActionType::Bonus
    } else {
        // Unknown type, skip
        warn!("Unknown stock dividend type for {}: {}", ticker, label);
        return None;
    };

    let approved_date = parse_brapi_date(&sd.approved_on).ok()?;
    let ex_date = parse_brapi_date(&sd.last_date_prior).ok()?;

    let factor = sd.factor.clone().unwrap_or_else(|| "1:1".to_string());

    Some(BrapiCorporateAction {
        ticker: ticker.to_string(),
        action_type,
        approved_date,
        ex_date,
        factor,
        remarks: sd.remarks.clone(),
    })
}

/// Parse cash dividend
fn parse_cash_dividend(cd: &CashDividend, ticker: &str) -> Option<BrapiIncomeEvent> {
    let payment_date = parse_brapi_date(&cd.payment_date).ok()?;
    let ex_date = cd
        .last_date_prior
        .as_ref()
        .and_then(|d| parse_brapi_date(d).ok());

    let amount = cd.rate.and_then(Decimal::from_f64_retain)?;

    // Determine event type from label
    let label = cd.label.as_deref().unwrap_or("").to_uppercase();
    let event_type = if label.contains("JCP") || label.contains("JUROS") {
        "JCP".to_string()
    } else if label.contains("RENDIMENTO") {
        "RENDIMENTO".to_string()
    } else {
        "DIVIDEND".to_string()
    };

    Some(BrapiIncomeEvent {
        ticker: ticker.to_string(),
        event_type,
        payment_date,
        ex_date,
        amount,
        remarks: cd.remarks.clone(),
    })
}

/// Parse Brapi date format (YYYY-MM-DD)
fn parse_brapi_date(date_str: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(date_str, "%Y-%m-%d")
        .or_else(|_| NaiveDate::parse_from_str(date_str, "%Y-%m-%dT%H:%M:%S"))
        .context(format!("Failed to parse date: {}", date_str))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn should_skip_online_tests() -> bool {
        std::env::var("INTEREST_SKIP_ONLINE_TESTS")
            .map(|v| v != "0")
            .unwrap_or(false)
    }

    #[tokio::test]
    async fn test_fetch_quote() {
        if should_skip_online_tests() {
            return;
        }

        let result = fetch_quote("PETR4", false).await;
        if let Err(e) = &result {
            eprintln!("Skipping Brapi quote test: {}", e);
            return;
        }
        let (price_data, _, _) = result.unwrap();

        assert_eq!(price_data.ticker, "PETR4");
        assert!(price_data.price > Decimal::ZERO);
        println!("PETR4 price from Brapi: R$ {}", price_data.price);
    }

    #[tokio::test]
    async fn test_fetch_quote_with_dividends() {
        if should_skip_online_tests() {
            return;
        }

        let result = fetch_quote("MXRF11", true).await;
        if let Err(e) = &result {
            eprintln!("Skipping Brapi quote+dividends test: {}", e);
            return;
        }
        let (price_data, actions, events) = result.unwrap();

        println!("MXRF11 price: R$ {}", price_data.price);
        if let Some(acts) = actions {
            println!("Corporate actions: {}", acts.len());
        }
        if let Some(evts) = events {
            println!("Income events: {}", evts.len());
        }
    }

    #[test]
    fn test_parse_brapi_date() {
        let result = parse_brapi_date("2025-01-15");
        assert!(result.is_ok());
        assert_eq!(
            result.unwrap(),
            NaiveDate::from_ymd_opt(2025, 1, 15).unwrap()
        );
    }
}
