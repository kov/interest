// Pricing module - Yahoo Finance and Brapi.dev API clients

pub mod yahoo;
pub mod brapi;

use anyhow::{Context, Result};
use chrono::{Duration, NaiveDate, Utc};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tracing::{debug, info};

pub use yahoo::{PriceData, HistoricalPrice};
pub use brapi::{BrapiPriceData, BrapiCorporateAction, BrapiIncomeEvent};

/// Price cache entry
#[derive(Debug, Clone)]
struct CacheEntry {
    price: rust_decimal::Decimal,
    timestamp: chrono::DateTime<chrono::Utc>,
}

/// Price fetcher with caching (24hr TTL)
pub struct PriceFetcher {
    cache: Arc<Mutex<HashMap<String, CacheEntry>>>,
    cache_ttl_hours: i64,
}

impl Default for PriceFetcher {
    fn default() -> Self {
        Self::new()
    }
}

impl PriceFetcher {
    pub fn new() -> Self {
        Self {
            cache: Arc::new(Mutex::new(HashMap::new())),
            cache_ttl_hours: 24,
        }
    }

    /// Fetch current price with caching
    pub async fn fetch_price(&self, ticker: &str) -> Result<rust_decimal::Decimal> {
        // Check cache first
        {
            let cache = self.cache.lock().unwrap();
            if let Some(entry) = cache.get(ticker) {
                let age = Utc::now().signed_duration_since(entry.timestamp);
                if age < Duration::hours(self.cache_ttl_hours) {
                    debug!("Using cached price for {} (age: {}h)", ticker, age.num_hours());
                    return Ok(entry.price);
                }
            }
        }

        // Fetch from Yahoo Finance (primary)
        info!("Fetching fresh price for {} from Yahoo Finance", ticker);
        let price_data = yahoo::fetch_current_price(ticker).await;

        match price_data {
            Ok(data) => {
                // Cache the price
                let mut cache = self.cache.lock().unwrap();
                cache.insert(
                    ticker.to_string(),
                    CacheEntry {
                        price: data.price,
                        timestamp: Utc::now(),
                    },
                );
                Ok(data.price)
            }
            Err(e) => {
                // Fallback to Brapi.dev
                info!("Yahoo Finance failed, trying Brapi.dev: {}", e);
                let (brapi_data, _, _) = brapi::fetch_quote(ticker, false).await
                    .context("Both Yahoo Finance and Brapi.dev failed")?;

                // Cache the price
                let mut cache = self.cache.lock().unwrap();
                cache.insert(
                    ticker.to_string(),
                    CacheEntry {
                        price: brapi_data.price,
                        timestamp: Utc::now(),
                    },
                );
                Ok(brapi_data.price)
            }
        }
    }

    /// Clear cache
    pub fn clear_cache(&self) {
        let mut cache = self.cache.lock().unwrap();
        cache.clear();
        info!("Price cache cleared");
    }

    /// Get cache size
    pub fn cache_size(&self) -> usize {
        let cache = self.cache.lock().unwrap();
        cache.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore] // Requires network access
    async fn test_price_fetcher_caching() {
        let fetcher = PriceFetcher::new();

        // First fetch (from API)
        let price1 = fetcher.fetch_price("PETR4").await.unwrap();
        assert!(price1 > rust_decimal::Decimal::ZERO);
        assert_eq!(fetcher.cache_size(), 1);

        // Second fetch (from cache)
        let price2 = fetcher.fetch_price("PETR4").await.unwrap();
        assert_eq!(price1, price2);

        println!("PETR4 price: R$ {}", price1);
    }
}
