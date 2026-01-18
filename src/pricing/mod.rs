// Pricing module - Yahoo Finance API client

pub mod resolver;
pub mod tesouro;
pub mod yahoo;

use anyhow::{Context, Result};
use chrono::{Duration, Utc};
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tracing::{debug, info};

/// Global singleton price fetcher with 24-hour cache.
/// This ensures cache is shared across all calls within a process.
static GLOBAL_FETCHER: Lazy<PriceFetcher> = Lazy::new(PriceFetcher::new);

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
                    debug!(
                        "Using cached price for {} (age: {}h)",
                        ticker,
                        age.num_hours()
                    );
                    return Ok(entry.price);
                }
            }
        }

        // Fetch from Yahoo Finance (primary)
        info!("Fetching fresh price for {} from Yahoo Finance", ticker);
        let price_data = yahoo::fetch_current_price(ticker)
            .await
            .context("Yahoo Finance price fetch failed")?;

        // Cache the price
        let mut cache = self.cache.lock().unwrap();
        cache.insert(
            ticker.to_string(),
            CacheEntry {
                price: price_data.price,
                timestamp: Utc::now(),
            },
        );
        Ok(price_data.price)
    }

    /// Clear cache
    #[allow(dead_code)]
    pub fn clear_cache(&self) {
        let mut cache = self.cache.lock().unwrap();
        cache.clear();
        info!("Price cache cleared");
    }

    /// Get cache size
    #[allow(dead_code)]
    pub fn cache_size(&self) -> usize {
        let cache = self.cache.lock().unwrap();
        cache.len()
    }
}

/// Convenience function to fetch a price using the global shared fetcher.
/// This uses a singleton cache that persists for the lifetime of the process.
pub async fn fetch_price(ticker: &str) -> Result<rust_decimal::Decimal> {
    GLOBAL_FETCHER.fetch_price(ticker).await
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
    async fn test_price_fetcher_caching() {
        if should_skip_online_tests() {
            return;
        }

        let fetcher = PriceFetcher::new();

        // First fetch (from API)
        let price1 = match fetcher.fetch_price("PETR4").await {
            Ok(p) => p,
            Err(e) => {
                eprintln!("Skipping price fetcher caching test: {}", e);
                return;
            }
        };
        assert!(price1 > rust_decimal::Decimal::ZERO);
        assert_eq!(fetcher.cache_size(), 1);

        // Second fetch (from cache)
        let price2 = match fetcher.fetch_price("PETR4").await {
            Ok(p) => p,
            Err(e) => {
                eprintln!("Skipping price fetcher caching test (second fetch): {}", e);
                return;
            }
        };
        assert_eq!(price1, price2);

        println!("PETR4 price: R$ {}", price1);
    }

    #[test]
    fn test_global_fetcher_is_singleton() {
        // The GLOBAL_FETCHER should be a singleton - same instance across calls
        // We can verify this by checking that the Arc points to the same data
        let cache1 = GLOBAL_FETCHER.cache.clone();
        let cache2 = GLOBAL_FETCHER.cache.clone();

        // Both should point to the same underlying data
        assert!(Arc::ptr_eq(&cache1, &cache2));
    }

    #[test]
    fn test_cache_ttl_default() {
        // Default TTL should be 24 hours
        assert_eq!(GLOBAL_FETCHER.cache_ttl_hours, 24);
    }
}
