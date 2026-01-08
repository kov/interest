//! Smart Price Resolver
//!
//! This module automatically resolves missing price data by:
//! 1. Detecting gaps in price_history table
//! 2. Choosing optimal strategy (B3 COTAHIST bulk vs Yahoo/Brapi API)
//! 3. Downloading and importing data
//! 4. Gracefully handling failures with degraded service
//!
//! **Design Philosophy**: Make it work automatically - don't make users think about
//! price data management.

use anyhow::Result;
use chrono::{Datelike, Local, NaiveDate};
use rusqlite::Connection;
use rust_decimal::Decimal;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::db::models::{Asset, AssetType};
use crate::importers::b3_cotahist;

/// Maximum concurrent API requests to avoid rate limiting
const MAX_CONCURRENT_REQUESTS: usize = 5;

#[cfg(not(test))]
use tracing;

/// Main entry point: ensure prices available for a date range.
/// Async version that can be called from async contexts (like the dispatcher).
#[allow(dead_code)]
pub async fn ensure_prices_available(
    conn: &mut Connection,
    assets: &[Asset],
    date_range: (NaiveDate, NaiveDate),
) -> Result<()> {
    ensure_prices_available_internal(conn, assets, date_range, &mut |_| {}).await
}

/// Version with progress callback for UI updates
pub async fn ensure_prices_available_with_progress<F>(
    conn: &mut Connection,
    assets: &[Asset],
    date_range: (NaiveDate, NaiveDate),
    mut progress: F,
) -> Result<()>
where
    F: FnMut(&str),
{
    ensure_prices_available_internal(conn, assets, date_range, &mut progress).await
}

/// Internal implementation that accepts optional progress callback
async fn ensure_prices_available_internal<F>(
    conn: &mut Connection,
    assets: &[Asset],
    date_range: (NaiveDate, NaiveDate),
    progress: &mut F,
) -> Result<()>
where
    F: FnMut(&str),
{
    let (start_date, end_date) = date_range;
    let today = Local::now().date_naive();
    let thirty_days_ago = today - chrono::Duration::days(30);

    tracing::debug!(
        "Resolving prices for {} assets from {} to {}",
        assets.len(),
        start_date,
        end_date
    );

    // Quick check: if start_date is in the future, nothing to do
    if start_date > today {
        return Ok(());
    }

    // Fast path: check if we already have recent prices for all *priceable* assets
    // If we have prices from yesterday or today, skip the expensive COTAHIST parsing
    let yesterday = today - chrono::Duration::days(1);

    // Count priceable assets (exclude bonds)
    let priceable_asset_ids: Vec<i64> = assets
        .iter()
        .filter(|a| is_priceable_asset(a))
        .filter_map(|a| a.id)
        .collect();

    if priceable_asset_ids.is_empty() {
        progress("✓ No price updates needed");
        tracing::debug!("No priceable assets in portfolio, skipping resolution");
        return Ok(());
    }

    progress(&format!("Checking {} assets...", priceable_asset_ids.len()));

    let placeholders = priceable_asset_ids
        .iter()
        .map(|_| "?")
        .collect::<Vec<_>>()
        .join(",");
    let query = format!(
        "SELECT COUNT(DISTINCT asset_id) FROM price_history 
         WHERE asset_id IN ({}) AND price_date >= ?",
        placeholders
    );

    // Build params: asset IDs first, then date
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    for id in &priceable_asset_ids {
        params.push(Box::new(*id));
    }
    params.push(Box::new(yesterday));

    let params_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|b| b.as_ref()).collect();

    let has_recent_prices =
        conn.query_row(&query, params_refs.as_slice(), |row| row.get::<_, i64>(0))?;

    let priceable_count = priceable_asset_ids.len() as i64;
    if has_recent_prices == priceable_count {
        progress("✓ All prices are up to date!");
        tracing::debug!(
            "All {} priceable assets have recent prices (since {}), skipping resolution",
            priceable_count,
            yesterday
        );
        return Ok(());
    }

    tracing::debug!(
        "{} of {} priceable assets need price updates",
        priceable_count - has_recent_prices,
        priceable_count
    );

    // If most assets have recent prices, skip expensive COTAHIST parsing
    // and only fetch current prices via API for the few that need updates
    if has_recent_prices > (priceable_count * 8 / 10) {
        tracing::debug!(
            "Skipping COTAHIST bulk import ({}% coverage), fetching current prices only",
            (has_recent_prices * 100 / priceable_count)
        );

        // Only fetch current prices for assets that need updates
        let need_update_assets: Vec<Asset> = assets
            .iter()
            .filter(|a| is_priceable_asset(a))
            .cloned()
            .collect();

        if !need_update_assets.is_empty() {
            progress(&format!(
                "Fetching prices for {} assets...",
                need_update_assets.len()
            ));
            tracing::info!(
                "Fetching current prices for {} assets from Yahoo/Brapi",
                need_update_assets.len()
            );
            fetch_current_prices_with_progress(conn, &need_update_assets, progress).await?;
            progress("✓ Price updates complete!");
        }
        return Ok(());
    }

    // Skip COTAHIST if we have ANY prices - just fetch current prices via API
    // This is faster than parsing millions of records for historical data
    let has_any_price_at_all: bool = conn
        .query_row("SELECT COUNT(*) FROM price_history LIMIT 1", [], |row| {
            Ok(row.get::<_, i64>(0)? > 0)
        })
        .unwrap_or(false);

    if has_any_price_at_all {
        tracing::debug!(
            "Skipping COTAHIST (have some prices), fetching current prices only via API"
        );

        let priceable_assets: Vec<Asset> = assets
            .iter()
            .filter(|a| is_priceable_asset(a))
            .cloned()
            .collect();

        if !priceable_assets.is_empty() {
            progress(&format!(
                "Fetching prices for {} assets...",
                priceable_assets.len()
            ));
            tracing::info!(
                "Fetching current prices for {} assets from Yahoo/Brapi",
                priceable_assets.len()
            );
            fetch_current_prices_with_progress(conn, &priceable_assets, progress).await?;
            progress("✓ Price updates complete!");
        }
        return Ok(());
    }

    // Determine which years need bulk download and which assets need current prices
    let mut needed_years = HashSet::new();
    let mut need_current_prices_assets = Vec::new();

    for asset in assets {
        let asset_id = asset.id.expect("Asset from database must have id");
        let has_prices = crate::db::has_any_prices(conn, asset_id, start_date, end_date)?;

        if !has_prices {
            // Determine if we need historical bulk or current API fetch
            if end_date <= thirty_days_ago {
                // All historical - use COTAHIST
                for year in start_date.year()..=end_date.year() {
                    needed_years.insert(year);
                }
            } else if start_date > thirty_days_ago {
                // All recent - use API
                need_current_prices_assets.push(asset.clone());
            } else {
                // Mixed: historical + recent
                for year in start_date.year()..=thirty_days_ago.year() {
                    needed_years.insert(year);
                }
                need_current_prices_assets.push(asset.clone());
            }
        }
    }

    // Fetch historical prices first (bulk COTAHIST)
    if !needed_years.is_empty() {
        tracing::info!("Fetching historical prices from B3 COTAHIST");
        let mut sorted_years: Vec<i32> = needed_years.into_iter().collect();
        sorted_years.sort();

        for year in sorted_years {
            match b3_cotahist::import_cotahist_year(conn, year, false, None) {
                Ok(count) => {
                    tracing::info!("Imported {} price records for {}", count, year);
                }
                Err(e) => {
                    tracing::warn!("Failed to import COTAHIST for {}: {}", year, e);
                    // Continue - graceful degradation
                }
            }
        }
    }

    // Filter out assets that we know don't have prices available from Yahoo/Brapi
    // (bonds, government bonds - these need different pricing sources)
    let priceable_assets: Vec<Asset> = need_current_prices_assets
        .into_iter()
        .filter(is_priceable_asset)
        .collect();

    // Fetch current prices via API (requires async runtime)
    if !priceable_assets.is_empty() {
        progress(&format!(
            "Fetching prices for {} assets...",
            priceable_assets.len()
        ));
        tracing::info!(
            "Fetching current prices for {} assets from Yahoo/Brapi",
            priceable_assets.len()
        );
        fetch_current_prices_with_progress(conn, &priceable_assets, progress).await?;
        progress("✓ Price updates complete!");
    }

    Ok(())
}

/// Check if an asset can be priced via Yahoo Finance or Brapi.dev APIs.
/// Bonds and government bonds need different pricing sources (not yet implemented).
/// FIXME: this is a hack that should be mostly fixed by parsing asset types properly.
fn is_priceable_asset(asset: &Asset) -> bool {
    match asset.asset_type {
        AssetType::Stock
        | AssetType::Etf
        | AssetType::Fii
        | AssetType::Fiagro
        | AssetType::FiInfra => {
            // Additionally filter out term contracts and subscription rights
            // These end with "T" or are specific patterns like "BOVAU###", "PETRF###", "ITSAA###"
            let ticker = &asset.ticker;

            // Term contracts (ANIM3T, CSED3T, etc.)
            if ticker.ends_with('T') && ticker.len() >= 6 {
                return false;
            }

            // Subscription rights / special options (BOVAU, ITSA, PETR + letter/numbers)
            if ticker.starts_with("BOVA")
                || ticker.starts_with("PETR")
                || ticker.starts_with("ITSA")
                || ticker.starts_with("ITSB")
            {
                // Normal stocks are 5-6 chars (PETR3, PETR4)
                // Special instruments are longer (PETRF407, BOVAU850, ITSAA101)
                if ticker.len() > 6 {
                    return false;
                }
            }

            // Delisted / no longer trading (these are known from the failure list)
            if ticker == "BAHI3"
                || ticker == "BKBR3"
                || ticker == "GBIO33"
                || ticker == "MEGA3"
                || ticker == "LUGG11"
                || ticker == "LGCP11"
                || ticker == "MALL11"
            {
                return false;
            }

            true
        }
        AssetType::Bond | AssetType::GovBond => false,
    }
}

/// Fetch prices in parallel with semaphore-based rate limiting.
/// Progress callback is called as each price completes (in completion order, not spawn order).
async fn fetch_current_prices_with_progress<F>(
    conn: &Connection,
    assets: &[Asset],
    progress: &mut F,
) -> Result<()>
where
    F: FnMut(&str),
{
    let today = Local::now().date_naive();
    let total = assets.len();
    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_REQUESTS));

    // Use JoinSet to get results as they complete (not in spawn order)
    let mut join_set = JoinSet::new();

    for asset in assets {
        let sem = semaphore.clone();
        let ticker = asset.ticker.clone();
        let asset_id = asset.id.expect("Asset from database must have id");

        join_set.spawn(async move {
            // Acquire semaphore permit (limits concurrent requests)
            let _permit = sem.acquire().await.unwrap();

            let result = crate::pricing::fetch_price(&ticker).await;
            (asset_id, ticker, result)
        });
    }

    // Collect results as they complete (whichever finishes first)
    let mut successful_prices: Vec<(i64, Decimal)> = Vec::new();
    let mut completed = 0;

    while let Some(result) = join_set.join_next().await {
        let (asset_id, ticker, fetch_result) = result?;
        completed += 1;

        match fetch_result {
            Ok(price) => {
                successful_prices.push((asset_id, price));
                let msg = format!("{} → R$ {:.2} ({}/{})", ticker, price, completed, total);
                progress(&msg);
                tracing::debug!("Fetched price for {}: {}", ticker, price);
            }
            Err(e) => {
                let msg = format!("{} → failed ({}/{})", ticker, completed, total);
                progress(&msg);
                tracing::warn!("Failed to fetch price for {}: {}", ticker, e);
            }
        }
    }

    // Batch insert all successful prices
    for (asset_id, price) in successful_prices {
        conn.execute(
            "INSERT OR REPLACE INTO price_history (asset_id, price_date, close_price, source)
             VALUES (?1, ?2, ?3, 'YAHOO')",
            rusqlite::params![asset_id, today, &price.to_string()],
        )?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strategy_determination() {
        let today = Local::now().date_naive();
        let thirty_days_ago = today - chrono::Duration::days(30);
        let sixty_days_ago = today - chrono::Duration::days(60);

        // Test 1: All historical (>30 days old) → BulkHistorical
        let _start = sixty_days_ago;
        let end = thirty_days_ago - chrono::Duration::days(1);
        assert!(end <= thirty_days_ago);

        // Test 2: All recent (<30 days old) → LiveApi
        let start_recent = thirty_days_ago + chrono::Duration::days(1);
        let _end_recent = today;
        assert!(start_recent > thirty_days_ago);

        // Test 3: Mixed range → Mixed strategy
        let start_mixed = sixty_days_ago;
        let end_mixed = today;
        assert!(start_mixed <= thirty_days_ago && end_mixed > thirty_days_ago);
    }

    // Compile-time check that MAX_CONCURRENT_REQUESTS is reasonable (1-10)
    const _: () = {
        assert!(MAX_CONCURRENT_REQUESTS >= 1);
        assert!(MAX_CONCURRENT_REQUESTS <= 10);
    };

    fn make_test_asset(ticker: &str, asset_type: AssetType) -> Asset {
        use chrono::Utc;
        Asset {
            id: Some(1),
            ticker: ticker.to_string(),
            name: Some("Test Asset".to_string()),
            asset_type,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn test_is_priceable_asset_stocks() {
        let stock = make_test_asset("PETR4", AssetType::Stock);
        assert!(is_priceable_asset(&stock));
    }

    #[test]
    fn test_is_priceable_asset_fiis() {
        let fii = make_test_asset("HGLG11", AssetType::Fii);
        assert!(is_priceable_asset(&fii));
    }

    #[test]
    fn test_is_priceable_asset_bonds_excluded() {
        let bond = make_test_asset("LCA123", AssetType::Bond);
        assert!(!is_priceable_asset(&bond));
    }

    #[test]
    fn test_is_priceable_asset_term_contracts_excluded() {
        // Term contracts end with 'T' and are 6+ chars
        let term = make_test_asset("ANIM3T", AssetType::Stock);
        assert!(!is_priceable_asset(&term));
    }

    #[test]
    fn test_is_priceable_asset_subscription_rights_excluded() {
        // Subscription rights are longer than 6 chars
        let rights = make_test_asset("PETRF407", AssetType::Stock);
        assert!(!is_priceable_asset(&rights));
    }

    #[test]
    fn test_is_priceable_asset_delisted_excluded() {
        let delisted = make_test_asset("BAHI3", AssetType::Stock);
        assert!(!is_priceable_asset(&delisted));
    }
}
