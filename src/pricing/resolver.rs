//! Smart Price Resolver
//!
//! This module automatically resolves missing price data by:
//! 1. Detecting gaps in price_history table
//! 2. Choosing optimal strategy (B3 COTAHIST bulk vs Yahoo API)
//! 3. Downloading and importing data
//! 4. Gracefully handling failures with degraded service
//!
//! **Design Philosophy**: Make it work automatically - don't make users think about
//! price data management.

use anyhow::{anyhow, Result};
use chrono::{Datelike, Local, NaiveDate};
use rusqlite::{Connection, OptionalExtension};
use rust_decimal::Decimal;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::utils::format_currency;

use crate::db::models::{Asset, AssetType};
use crate::importers::b3_cotahist;
use crate::pricing::tesouro;

/// Maximum concurrent API requests to avoid rate limiting
const MAX_CONCURRENT_REQUESTS: usize = 5;

#[derive(Debug)]
struct PriceResolutionNeeds {
    needed_years: HashSet<i32>,
    need_current_prices_assets: Vec<Asset>,
}

fn determine_price_resolution_needs(
    conn: &Connection,
    assets: &[Asset],
    start_date: NaiveDate,
    end_date: NaiveDate,
    today: NaiveDate,
) -> Result<PriceResolutionNeeds> {
    let mut needed_years = HashSet::new();
    let mut need_current_prices_assets = Vec::new();

    for asset in assets {
        let asset_id = asset.id.expect("Asset from database must have id");
        let has_prices = if start_date == end_date && end_date < today {
            conn.query_row(
                "SELECT 1 FROM price_history WHERE asset_id = ?1 AND price_date <= ?2 LIMIT 1",
                rusqlite::params![asset_id, end_date],
                |_| Ok(()),
            )
            .optional()?
            .is_some()
        } else {
            crate::db::has_any_prices(conn, asset_id, start_date, end_date)?
        };

        if !has_prices {
            // Determine if we need historical bulk or current API fetch
            if end_date < today {
                // All historical - use COTAHIST
                for year in start_date.year()..=end_date.year() {
                    needed_years.insert(year);
                }
            } else if start_date == today {
                // Current-only - use API
                need_current_prices_assets.push(asset.clone());
            } else {
                // Mixed: historical + current
                for year in start_date.year()..=today.year() {
                    needed_years.insert(year);
                }
                need_current_prices_assets.push(asset.clone());
            }
        }
    }

    Ok(PriceResolutionNeeds {
        needed_years,
        need_current_prices_assets,
    })
}

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
use crate::ui::progress::ProgressEvent;

pub async fn ensure_prices_available_with_progress<F>(
    conn: &mut Connection,
    assets: &[Asset],
    date_range: (NaiveDate, NaiveDate),
    mut progress: F,
) -> Result<()>
where
    F: FnMut(&ProgressEvent),
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
    F: FnMut(&crate::ui::progress::ProgressEvent),
{
    let (start_date, end_date) = date_range;
    let today = Local::now().date_naive();

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

    let yesterday = today - chrono::Duration::days(1);
    let current_only = start_date == today && end_date == today;

    let gov_bond_assets: Vec<Asset> = assets
        .iter()
        .filter(|a| a.asset_type == AssetType::GovBond)
        .cloned()
        .collect();

    let priceable_assets: Vec<Asset> = assets
        .iter()
        .filter(|a| is_priceable_asset(a))
        .cloned()
        .collect();

    // Count priceable assets (exclude bonds)
    let priceable_asset_ids: Vec<i64> = priceable_assets.iter().filter_map(|a| a.id).collect();

    if priceable_asset_ids.is_empty() && gov_bond_assets.is_empty() {
        progress(&ProgressEvent::from_message("✓ No price updates needed"));
        tracing::debug!("No priceable assets in portfolio, skipping resolution");
        return Ok(());
    }

    if current_only {
        if !gov_bond_assets.is_empty() {
            progress(&ProgressEvent::from_message(
                "Importing Tesouro Direto recent prices...",
            ));
            let recent_start = today - chrono::Duration::days(365);
            let count =
                import_gov_bond_prices(gov_bond_assets.clone(), recent_start, today).await?;
            progress(&ProgressEvent::from_message(&format!(
                "✓ Imported {} Tesouro prices",
                count
            )));
        }

        if priceable_asset_ids.is_empty() {
            progress(&ProgressEvent::from_message("✓ Tesouro prices updated"));
            return Ok(());
        }

        // Fast path: check if we already have recent prices for all *priceable* assets
        // If we have prices from yesterday or today, skip the expensive COTAHIST parsing
        progress(&ProgressEvent::from_message(&format!(
            "Checking {} assets...",
            priceable_asset_ids.len()
        )));

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
            progress(&ProgressEvent::from_message("✓ All prices are up to date!"));
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
            let recent_query = format!(
                "SELECT DISTINCT asset_id FROM price_history
                 WHERE asset_id IN ({}) AND price_date >= ?",
                placeholders
            );
            let mut recent_ids: HashSet<i64> = HashSet::new();
            let mut stmt = conn.prepare(&recent_query)?;
            let rows = stmt.query_map(params_refs.as_slice(), |row| row.get::<_, i64>(0))?;
            for row in rows {
                recent_ids.insert(row?);
            }

            let need_update_assets: Vec<Asset> = assets
                .iter()
                .filter(|a| is_priceable_asset(a))
                .filter(|a| a.id.map(|id| !recent_ids.contains(&id)).unwrap_or(false))
                .cloned()
                .collect();

            if !need_update_assets.is_empty() {
                progress(&ProgressEvent::from_message(&format!(
                    "Fetching prices for {} assets...",
                    need_update_assets.len()
                )));
                tracing::info!(
                    "Fetching current prices for {} assets from Yahoo",
                    need_update_assets.len()
                );
                fetch_current_prices_with_progress(conn, &need_update_assets, progress).await?;
                progress(&ProgressEvent::from_message("✓ Price updates complete!"));
            }
            return Ok(());
        }
    }

    // Skip COTAHIST if we have ANY prices - just fetch current prices via API
    // This is faster than parsing millions of records for historical data
    let has_any_price_at_all: bool = conn
        .query_row("SELECT COUNT(*) FROM price_history LIMIT 1", [], |row| {
            Ok(row.get::<_, i64>(0)? > 0)
        })
        .unwrap_or(false);

    if has_any_price_at_all && current_only {
        tracing::debug!(
            "Skipping COTAHIST (have some prices), fetching current prices only via API"
        );

        if !priceable_assets.is_empty() {
            progress(&ProgressEvent::from_message(&format!(
                "Fetching prices for {} assets...",
                priceable_assets.len()
            )));
            tracing::info!(
                "Fetching current prices for {} assets from Yahoo",
                priceable_assets.len()
            );
            fetch_current_prices_with_progress(conn, &priceable_assets, progress).await?;
            progress(&ProgressEvent::from_message("✓ Price updates complete!"));
        }
        return Ok(());
    }

    // Determine which years need bulk download and which assets need current prices
    let needs =
        determine_price_resolution_needs(conn, &priceable_assets, start_date, end_date, today)?;
    let needed_years = needs.needed_years;
    let need_current_prices_assets = needs.need_current_prices_assets;

    // Fetch historical prices first (bulk COTAHIST)
    if !needed_years.is_empty() {
        tracing::info!("Fetching historical prices from B3 COTAHIST");
        let mut sorted_years: Vec<i32> = needed_years.into_iter().collect();
        sorted_years.sort();

        // Use a channel to communicate progress from parallel tasks to the main thread
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();

        // Use JoinSet for parallel processing of multiple years
        let mut join_set = JoinSet::new();

        for year in sorted_years {
            let tx = tx.clone();

            join_set.spawn_blocking(move || {
                // Create a callback that forwards progress events with display mode info
                let callback = |progress_event: &b3_cotahist::DownloadProgress| {
                    use b3_cotahist::{DisplayMode, DownloadStage};

                    let (msg, display_mode) = match progress_event.stage {
                        DownloadStage::Downloading => (
                            format!("⬇️  Downloading COTAHIST {}...", progress_event.year),
                            DisplayMode::Spinner,
                        ),
                        DownloadStage::Decompressing => (
                            format!("📦 Decompressing COTAHIST {}...", progress_event.year),
                            DisplayMode::Spinner,
                        ),
                        DownloadStage::Parsing => {
                            // Show parsing progress with percentage
                            let msg_text = if progress_event.display_mode == DisplayMode::Persist {
                                format!(
                                    "✓ Parsed {} prices from COTAHIST {}",
                                    progress_event.records_processed, progress_event.year
                                )
                            } else if let Some(total) = progress_event.total_records {
                                let pct = if total > 0 {
                                    progress_event.records_processed * 100 / total
                                } else {
                                    0
                                };
                                format!(
                                    "📝 Parsing COTAHIST {} ({}/{}  {}%)",
                                    progress_event.year,
                                    progress_event.records_processed,
                                    total,
                                    pct
                                )
                            } else {
                                format!("📝 Parsing COTAHIST {}...", progress_event.year)
                            };
                            (msg_text, progress_event.display_mode.clone())
                        }
                        DownloadStage::Complete => (
                            format!(
                                "✓ Imported {} prices for {}",
                                progress_event.records_processed, progress_event.year
                            ),
                            DisplayMode::Persist,
                        ),
                    };

                    // Send progress message through channel
                    let prefixed_msg = match display_mode {
                        DisplayMode::Persist => format!("__PERSIST__:{}", msg),
                        DisplayMode::Spinner => msg,
                    };
                    let _ = tx.send(prefixed_msg);
                };

                // Open a fresh connection for this task
                let mut conn_task = match crate::db::open_db(None) {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::warn!("Failed to open database for year {}: {}", year, e);
                        return;
                    }
                };

                // Import the year
                match b3_cotahist::import_cotahist_year(
                    &mut conn_task,
                    year,
                    false,
                    Some(&callback),
                ) {
                    Ok(count) => {
                        tracing::info!("Imported {} price records for {}", count, year);
                    }
                    Err(e) => {
                        // Show error in progress callback with high-level reason
                        let reason = if e.to_string().contains("Download failed") {
                            "download failed"
                        } else if e.to_string().contains("ZIP") {
                            "invalid file format"
                        } else if e.to_string().contains("not be available") {
                            "year not available"
                        } else {
                            "import failed"
                        };
                        let _ = tx.send(format!("__PERSIST__:❌ COTAHIST {}: {}", year, reason));
                        tracing::warn!("Failed to import COTAHIST for {}: {}", year, e);
                        // Continue - graceful degradation
                    }
                }
            });
        }

        // Drop the main tx so the receiver knows when all tasks are done
        drop(tx);

        // Create a task that collects progress messages while join_set completes
        let progress_future = async {
            while let Some(msg) = rx.recv().await {
                progress(&ProgressEvent::from_message(&msg));
            }
        };

        // Run both concurrently: collect progress messages and wait for tasks
        tokio::select! {
            _ = progress_future => {
                // Progress finished first, wait for remaining tasks
                while let Some(_result) = join_set.join_next().await {
                    // Tasks handle errors internally
                }
            }
            _ = async {
                while let Some(_result) = join_set.join_next().await {
                    // Tasks handle errors internally
                }
            } => {
                // All tasks finished before progress channel closed
            }
        }
    }

    if !gov_bond_assets.is_empty() && start_date < today {
        let historical_end = if end_date < today { end_date } else { today };
        if start_date <= historical_end {
            progress(&ProgressEvent::from_message(
                "Importing Tesouro Direto historical prices...",
            ));
            let count =
                import_gov_bond_prices(gov_bond_assets.clone(), start_date, historical_end).await?;
            progress(&ProgressEvent::from_message(&format!(
                "✓ Imported {} Tesouro historical prices",
                count
            )));
        }
    }

    // Filter out assets that we know don't have prices available from Yahoo
    // (bonds, government bonds - these need different pricing sources)
    let priceable_assets: Vec<Asset> = need_current_prices_assets
        .into_iter()
        .filter(is_priceable_asset)
        .collect();

    // Fetch current prices via API (requires async runtime)
    if !priceable_assets.is_empty() {
        progress(&ProgressEvent::from_message(&format!(
            "Fetching prices for {} assets...",
            priceable_assets.len()
        )));
        tracing::info!(
            "Fetching current prices for {} assets from Yahoo",
            priceable_assets.len()
        );
        fetch_current_prices_with_progress(conn, &priceable_assets, progress).await?;
        progress(&ProgressEvent::from_message("✓ Price updates complete!"));
    }

    Ok(())
}

async fn import_gov_bond_prices(
    assets: Vec<Asset>,
    start_date: NaiveDate,
    end_date: NaiveDate,
) -> Result<usize> {
    tokio::task::spawn_blocking(move || {
        let conn = crate::db::open_db(None)?;
        tesouro::import_tesouro_csv(&conn, &assets, start_date, end_date)
    })
    .await
    .map_err(|err| anyhow!("Failed to import Tesouro prices: {}", err))?
}

/// Check if an asset can be priced via Yahoo Finance APIs.
/// Bonds and government bonds need different pricing sources (not yet implemented).
/// FIXME: this is a hack that should be mostly fixed by parsing asset types properly.
pub(crate) fn is_priceable_asset(asset: &Asset) -> bool {
    match asset.asset_type {
        AssetType::Stock
        | AssetType::Bdr
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
        AssetType::Bond
        | AssetType::GovBond
        | AssetType::Fidc
        | AssetType::Fip
        | AssetType::Option
        | AssetType::TermContract
        | AssetType::Unknown => false,
    }
}

pub(crate) fn filter_priceable_assets(assets: &[Asset]) -> Vec<Asset> {
    assets
        .iter()
        .filter(|&asset| is_priceable_asset(asset))
        .cloned()
        .collect()
}

/// Fetch prices in parallel with semaphore-based rate limiting.
/// Progress callback is called as each price completes (in completion order, not spawn order).
async fn fetch_current_prices_with_progress<F>(
    conn: &Connection,
    assets: &[Asset],
    progress: &mut F,
) -> Result<()>
where
    F: FnMut(&crate::ui::progress::ProgressEvent),
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
                let msg = format!(
                    "{} → {} ({}/{})",
                    ticker,
                    format_currency(price),
                    completed,
                    total
                );
                progress(&ProgressEvent::from_message(&msg));
                tracing::debug!("Fetched price for {}: {}", ticker, price);
            }
            Err(e) => {
                let msg = format!("{} → failed ({}/{})", ticker, completed, total);
                progress(&ProgressEvent::from_message(&msg));
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
    use anyhow::Result;
    use chrono::{NaiveDate, Utc};
    use rust_decimal::Decimal;
    use tempfile::NamedTempFile;

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
        Asset {
            id: Some(1),
            ticker: ticker.to_string(),
            name: Some("Test Asset".to_string()),
            cnpj: None,
            asset_type,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn setup_db() -> Result<(NamedTempFile, Connection)> {
        let tmp = NamedTempFile::new()?;
        let path = tmp.path().to_path_buf();
        crate::db::init_database(Some(path.clone()))?;
        let conn = crate::db::open_db(Some(path))?;
        Ok((tmp, conn))
    }

    fn insert_asset(conn: &Connection, ticker: &str, asset_type: AssetType) -> Result<Asset> {
        conn.execute(
            "INSERT INTO assets (ticker, asset_type, name) VALUES (?1, ?2, NULL)",
            rusqlite::params![ticker.to_ascii_uppercase(), asset_type.as_str()],
        )?;
        Ok(crate::db::get_asset_by_ticker(conn, ticker)?.expect("asset should exist"))
    }

    fn insert_price(conn: &Connection, asset_id: i64, date: NaiveDate) -> Result<()> {
        let price = crate::db::PriceHistory {
            id: None,
            asset_id,
            price_date: date,
            close_price: Decimal::from(10),
            open_price: None,
            high_price: None,
            low_price: None,
            volume: None,
            source: "TEST".to_string(),
            created_at: Utc::now(),
        };
        crate::db::insert_price_history(conn, &price)?;
        Ok(())
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

    #[test]
    fn test_determine_needs_historical_only() -> Result<()> {
        let (_tmp, conn) = setup_db()?;
        let asset = insert_asset(&conn, "PETR4", AssetType::Stock)?;
        let today = Local::now().date_naive();
        let start = NaiveDate::from_ymd_opt(2024, 12, 31).unwrap();
        let end = start;

        let needs = determine_price_resolution_needs(
            &conn,
            std::slice::from_ref(&asset),
            start,
            end,
            today,
        )?;

        assert!(needs.needed_years.contains(&2024));
        assert!(needs.need_current_prices_assets.is_empty());
        Ok(())
    }

    #[test]
    fn test_determine_needs_mixed_range() -> Result<()> {
        let (_tmp, conn) = setup_db()?;
        let asset = insert_asset(&conn, "PETR4", AssetType::Stock)?;
        let today = NaiveDate::from_ymd_opt(2025, 3, 10).unwrap();
        let start = NaiveDate::from_ymd_opt(2024, 12, 31).unwrap();
        let end = today;

        let needs = determine_price_resolution_needs(
            &conn,
            std::slice::from_ref(&asset),
            start,
            end,
            today,
        )?;

        assert!(needs.needed_years.contains(&2024));
        assert!(needs.needed_years.contains(&2025));
        assert_eq!(needs.need_current_prices_assets.len(), 1);
        assert_eq!(needs.need_current_prices_assets[0].ticker, "PETR4");
        Ok(())
    }

    #[test]
    fn test_determine_needs_cache_hit_for_range() -> Result<()> {
        let (_tmp, conn) = setup_db()?;
        let asset = insert_asset(&conn, "PETR4", AssetType::Stock)?;
        let today = NaiveDate::from_ymd_opt(2025, 3, 10).unwrap();
        let start = NaiveDate::from_ymd_opt(2024, 12, 31).unwrap();
        let end = start;

        insert_price(&conn, asset.id.unwrap(), start)?;

        let needs = determine_price_resolution_needs(&conn, &[asset], start, end, today)?;

        assert!(needs.needed_years.is_empty());
        assert!(needs.need_current_prices_assets.is_empty());
        Ok(())
    }

    #[test]
    fn test_determine_needs_historical_date_cache_hit_on_or_before() -> Result<()> {
        let (_tmp, conn) = setup_db()?;
        let asset = insert_asset(&conn, "PETR4", AssetType::Stock)?;
        let today = NaiveDate::from_ymd_opt(2025, 3, 10).unwrap();
        let price_date = NaiveDate::from_ymd_opt(2025, 3, 6).unwrap();
        let as_of = NaiveDate::from_ymd_opt(2025, 3, 7).unwrap();

        insert_price(&conn, asset.id.unwrap(), price_date)?;

        let needs = determine_price_resolution_needs(&conn, &[asset], as_of, as_of, today)?;

        assert!(needs.needed_years.is_empty());
        assert!(needs.need_current_prices_assets.is_empty());
        Ok(())
    }
}
