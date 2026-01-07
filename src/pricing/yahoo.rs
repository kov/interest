use anyhow::{anyhow, Context, Result};
use chrono::NaiveDate;
use reqwest::Client;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

/// Yahoo Finance quote response
#[derive(Debug, Deserialize)]
struct YahooQuoteResponse {
    chart: ChartData,
}

#[derive(Debug, Deserialize)]
struct ChartData {
    result: Option<Vec<ChartResult>>,
    error: Option<YahooError>,
}

#[derive(Debug, Deserialize)]
struct ChartResult {
    meta: Meta,
    timestamp: Option<Vec<i64>>,
    indicators: Indicators,
}

#[derive(Debug, Deserialize)]
struct Meta {
    #[serde(rename = "regularMarketPrice")]
    regular_market_price: Option<f64>,
    currency: Option<String>,
    #[allow(dead_code)]
    symbol: String,
}

#[derive(Debug, Deserialize)]
struct Indicators {
    quote: Vec<Quote>,
}

#[derive(Debug, Deserialize)]
struct Quote {
    open: Option<Vec<Option<f64>>>,
    high: Option<Vec<Option<f64>>>,
    low: Option<Vec<Option<f64>>>,
    close: Option<Vec<Option<f64>>>,
    volume: Option<Vec<Option<i64>>>,
}

#[derive(Debug, Deserialize)]
struct YahooError {
    code: String,
    description: String,
}

/// Fetched price data
#[derive(Debug, Clone, Serialize)]
pub struct PriceData {
    pub ticker: String,
    pub price: Decimal,
    pub currency: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Historical price point
#[derive(Debug, Clone, Serialize)]
pub struct HistoricalPrice {
    pub date: NaiveDate,
    pub open: Option<Decimal>,
    pub high: Option<Decimal>,
    pub low: Option<Decimal>,
    pub close: Decimal,
    pub volume: Option<i64>,
}

/// Fetch current price from Yahoo Finance
pub async fn fetch_current_price(ticker: &str) -> Result<PriceData> {
    let symbol = format!("{}.SA", ticker);
    info!("Fetching current price for {} from Yahoo Finance", symbol);

    let client = Client::builder()
        .user_agent("Mozilla/5.0 (compatible; InterestBot/1.0)")
        .build()?;

    let url = format!(
        "https://query1.finance.yahoo.com/v8/finance/chart/{}",
        symbol
    );

    let response = client
        .get(&url)
        .send()
        .await
        .context("Failed to send request to Yahoo Finance")?;

    if !response.status().is_success() {
        return Err(anyhow!(
            "Yahoo Finance returned error status: {}",
            response.status()
        ));
    }

    let data: YahooQuoteResponse = response
        .json()
        .await
        .context("Failed to parse Yahoo Finance response")?;

    if let Some(error) = data.chart.error {
        return Err(anyhow!(
            "Yahoo Finance API error: {} - {}",
            error.code,
            error.description
        ));
    }

    let result = data
        .chart
        .result
        .and_then(|r| r.into_iter().next())
        .ok_or_else(|| anyhow!("No data returned from Yahoo Finance"))?;

    let price = result
        .meta
        .regular_market_price
        .ok_or_else(|| anyhow!("No price data available"))?;

    let currency = result.meta.currency.unwrap_or_else(|| "BRL".to_string());

    Ok(PriceData {
        ticker: ticker.to_string(),
        price: Decimal::from_f64_retain(price).ok_or_else(|| anyhow!("Invalid price value"))?,
        currency,
        timestamp: chrono::Utc::now(),
    })
}

/// Fetch historical prices from Yahoo Finance
///
/// # Arguments
/// * `ticker` - Ticker symbol (without .SA suffix)
/// * `from` - Start date
/// * `to` - End date
pub async fn fetch_historical_prices(
    ticker: &str,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<HistoricalPrice>> {
    let symbol = format!("{}.SA", ticker);
    info!(
        "Fetching historical prices for {} from {} to {}",
        symbol, from, to
    );

    let client = Client::builder()
        .user_agent("Mozilla/5.0 (compatible; InterestBot/1.0)")
        .build()?;

    // Convert dates to Unix timestamps
    let from_timestamp = from
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| anyhow!("Invalid from date"))?
        .and_utc()
        .timestamp();

    let to_timestamp = to
        .and_hms_opt(23, 59, 59)
        .ok_or_else(|| anyhow!("Invalid to date"))?
        .and_utc()
        .timestamp();

    let url = format!(
        "https://query1.finance.yahoo.com/v8/finance/chart/{}?period1={}&period2={}&interval=1d",
        symbol, from_timestamp, to_timestamp
    );

    let response = client
        .get(&url)
        .send()
        .await
        .context("Failed to send request to Yahoo Finance")?;

    if !response.status().is_success() {
        return Err(anyhow!(
            "Yahoo Finance returned error status: {}",
            response.status()
        ));
    }

    let data: YahooQuoteResponse = response
        .json()
        .await
        .context("Failed to parse Yahoo Finance response")?;

    if let Some(error) = data.chart.error {
        return Err(anyhow!(
            "Yahoo Finance API error: {} - {}",
            error.code,
            error.description
        ));
    }

    let result = data
        .chart
        .result
        .and_then(|r| r.into_iter().next())
        .ok_or_else(|| anyhow!("No data returned from Yahoo Finance"))?;

    let timestamps = result
        .timestamp
        .ok_or_else(|| anyhow!("No timestamp data"))?;

    let quote = result
        .indicators
        .quote
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("No quote data"))?;

    let opens = quote.open.unwrap_or_default();
    let highs = quote.high.unwrap_or_default();
    let lows = quote.low.unwrap_or_default();
    let closes = quote.close.ok_or_else(|| anyhow!("No close prices"))?;
    let volumes = quote.volume.unwrap_or_default();

    let mut prices = Vec::new();

    for (i, &timestamp) in timestamps.iter().enumerate() {
        let date = chrono::DateTime::from_timestamp(timestamp, 0)
            .ok_or_else(|| anyhow!("Invalid timestamp"))?
            .date_naive();

        let close = closes
            .get(i)
            .and_then(|&v| v)
            .ok_or_else(|| anyhow!("Missing close price for date {}", date))?;

        prices.push(HistoricalPrice {
            date,
            open: opens
                .get(i)
                .and_then(|&v| v)
                .and_then(Decimal::from_f64_retain),
            high: highs
                .get(i)
                .and_then(|&v| v)
                .and_then(Decimal::from_f64_retain),
            low: lows
                .get(i)
                .and_then(|&v| v)
                .and_then(Decimal::from_f64_retain),
            close: Decimal::from_f64_retain(close).ok_or_else(|| anyhow!("Invalid close price"))?,
            volume: volumes.get(i).and_then(|&v| v),
        });
    }

    debug!("Fetched {} historical prices", prices.len());
    Ok(prices)
}

/// Fetch company name from Yahoo Finance by scraping the quote page
pub async fn fetch_company_name(ticker: &str) -> Result<String> {
    let symbol = format!("{}.SA", ticker);
    info!("Fetching company name for {} from Yahoo Finance", symbol);

    let client = Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .build()?;

    let url = format!("https://finance.yahoo.com/quote/{}", symbol);

    let response = client
        .get(&url)
        .send()
        .await
        .context("Failed to send request to Yahoo Finance")?;

    if !response.status().is_success() {
        return Err(anyhow!(
            "Yahoo Finance returned error status: {}",
            response.status()
        ));
    }

    let html = response
        .text()
        .await
        .context("Failed to get HTML from Yahoo Finance")?;

    // Try to extract company name from <h1> tag
    // The page typically has: <h1>COMPANY NAME (TICKER)</h1>
    if let Some(start) = html.find("<h1") {
        if let Some(content_start) = html[start..].find('>') {
            let abs_start = start + content_start + 1;
            if let Some(end) = html[abs_start..].find("</h1>") {
                let h1_content = &html[abs_start..abs_start + end];

                // Extract just the company name (before the ticker in parentheses)
                let name = if let Some(paren_pos) = h1_content.find(" (") {
                    h1_content[..paren_pos].trim()
                } else {
                    h1_content.trim()
                };

                if !name.is_empty() {
                    info!("Found company name: {}", name);
                    return Ok(name.to_string());
                }
            }
        }
    }

    Err(anyhow!(
        "Could not extract company name from Yahoo Finance page"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore] // Requires network access
    async fn test_fetch_current_price() {
        let result = fetch_current_price("PETR4").await;
        assert!(result.is_ok());

        if let Ok(price_data) = result {
            assert_eq!(price_data.ticker, "PETR4");
            assert!(price_data.price > Decimal::ZERO);
            println!("PETR4 price: R$ {}", price_data.price);
        }
    }

    #[tokio::test]
    #[ignore] // Requires network access
    async fn test_fetch_historical_prices() {
        let from = NaiveDate::from_ymd_opt(2025, 1, 1).unwrap();
        let to = NaiveDate::from_ymd_opt(2025, 1, 10).unwrap();

        let result = fetch_historical_prices("PETR4", from, to).await;
        assert!(result.is_ok());

        if let Ok(prices) = result {
            assert!(!prices.is_empty());
            println!("Fetched {} historical prices", prices.len());
        }
    }
}
