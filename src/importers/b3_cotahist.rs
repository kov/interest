//! B3 COTAHIST Historical Price Data Importer
//!
//! This module handles downloading and parsing historical price data from B3's
//! COTAHIST (Cotações Históricas) dataset.
//!
//! Key features:
//! - Automatic discovery of download URLs via web scraping
//! - Persistent caching to avoid re-downloading (uses ~/.cache/interest/cotahist/)
//! - Fixed-width format parsing according to B3 specification
//! - Graceful error handling with fallback strategies

use anyhow::{anyhow, Context, Result};
use chrono::NaiveDate;
use reqwest::blocking::Client;
use rusqlite::Connection;
use rust_decimal::Decimal;
use std::io::Read;
use std::path::{Path, PathBuf};
use zip::ZipArchive;

// Direct download URL pattern discovered from B3's SeriesHistoricasI.js
const B3_COTAHIST_BASE_URL: &str = "https://bvmf.bmfbovespa.com.br/InstDados/SerHist";

/// Represents a single COTAHIST price record
#[derive(Debug, Clone)]
pub struct CotahistRecord {
    pub date: NaiveDate,
    pub ticker: String,
    pub open_price: Decimal,
    pub high_price: Decimal,
    pub low_price: Decimal,
    pub close_price: Decimal,
    pub volume: i64,
}

/// Progress information for COTAHIST downloads
#[derive(Debug, Clone)]
pub struct DownloadProgress {
    pub stage: DownloadStage,
    pub year: i32,
    pub records_processed: usize,
    pub total_records: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DownloadStage {
    Downloading,
    Decompressing,
    Parsing,
    Complete,
}

/// Get the platform-specific cache directory for COTAHIST files
pub fn get_cotahist_cache_dir() -> Result<PathBuf> {
    let cache_dir =
        dir_spec::cache_home().ok_or_else(|| anyhow!("Could not determine cache directory"))?;

    Ok(cache_dir.join("interest").join("cotahist"))
}

/// Clear cached COTAHIST files
pub fn clear_cache(year: Option<i32>) -> Result<()> {
    let cache_dir = get_cotahist_cache_dir()?;

    if !cache_dir.exists() {
        return Ok(()); // Nothing to clean
    }

    match year {
        Some(y) => {
            // Delete specific year
            let file = cache_dir.join(format!("COTAHIST_A{}.ZIP", y));
            if file.exists() {
                std::fs::remove_file(&file).context("Failed to delete cache file")?;
                tracing::info!("Deleted cache for COTAHIST {}", y);
            }
        }
        None => {
            // Delete entire cache directory
            std::fs::remove_dir_all(&cache_dir).context("Failed to remove cache directory")?;
            tracing::info!("Cleared all COTAHIST cache");
        }
    }

    Ok(())
}

/// Construct the direct download URL for a COTAHIST file
///
/// Uses the direct URL pattern from B3's SeriesHistoricasI.js:
/// /InstDados/SerHist/COTAHIST_A{YEAR}.ZIP
///
/// This bypasses the captcha dialog required for web interface navigation.
fn get_cotahist_url(year: i32) -> String {
    format!("{}/COTAHIST_A{}.ZIP", B3_COTAHIST_BASE_URL, year)
}

/// Get the modification time of a cached COTAHIST file as Unix timestamp
fn get_cache_file_mtime(year: i32) -> Result<Option<i64>> {
    let cache_dir = get_cotahist_cache_dir()?;
    let zip_path = cache_dir.join(format!("COTAHIST_A{}.ZIP", year));

    if !zip_path.exists() {
        return Ok(None);
    }

    let metadata = std::fs::metadata(&zip_path).context("Failed to read cache file metadata")?;
    let mtime = metadata
        .modified()
        .context("Failed to get file modification time")?
        .duration_since(std::time::UNIX_EPOCH)
        .context("Invalid file modification time")?
        .as_secs() as i64;

    Ok(Some(mtime))
}

/// Check if we should check for COTAHIST updates (>1 day since last check)
fn should_check_for_updates(year: i32) -> Result<bool> {
    let conn = crate::db::open_db(None)?;
    let key = format!("cotahist_last_checked_{}", year);
    let last_checked = crate::db::get_metadata(&conn, &key)?;

    match last_checked {
        None => Ok(true), // Never checked, should check
        Some(timestamp_str) => {
            let last_checked_ts: i64 = timestamp_str
                .parse()
                .context("Invalid timestamp in metadata")?;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .context("System time error")?
                .as_secs() as i64;
            let one_day = 86400; // seconds in a day

            Ok(now - last_checked_ts > one_day)
        }
    }
}

/// Mark that we checked for COTAHIST updates (stores current timestamp)
fn mark_last_checked(year: i32) -> Result<()> {
    let conn = crate::db::open_db(None)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .context("System time error")?
        .as_secs();
    let key = format!("cotahist_last_checked_{}", year);
    crate::db::set_metadata(&conn, &key, &now.to_string())
}

/// Check if a COTAHIST file has been imported (based on mtime)
///
/// Pass a connection to use a specific database (for tests), or None for the default database
pub fn has_cotahist_been_imported_with_conn(
    year: i32,
    conn_opt: Option<&Connection>,
) -> Result<bool> {
    let current_mtime = match get_cache_file_mtime(year)? {
        Some(mt) => mt,
        None => return Ok(false), // No cache file
    };

    let key = format!("cotahist_imported_{}_mtime", year);

    let stored_mtime = match conn_opt {
        Some(conn) => crate::db::get_metadata(conn, &key)?,
        None => {
            let conn = crate::db::open_db(None)?;
            crate::db::get_metadata(&conn, &key)?
        }
    };

    match stored_mtime {
        Some(stored) => Ok(stored == current_mtime.to_string()),
        None => Ok(false), // Never imported
    }
}

/// Mark that a COTAHIST file has been imported (stores file's current mtime)
pub fn mark_cotahist_imported(year: i32) -> Result<()> {
    let conn = crate::db::open_db(None)?;
    let mtime = get_cache_file_mtime(year)?
        .ok_or_else(|| anyhow!("Cache file not found for year {}", year))?;

    let key = format!("cotahist_imported_{}_mtime", year);
    crate::db::set_metadata(&conn, &key, &mtime.to_string())
}

/// Download COTAHIST file for a year and cache it (synchronous)
///
/// Implements conditional downloads using If-Modified-Since:
/// - If cache exists and < 1 day since last check: use cache
/// - If cache exists and > 1 day since last check: do conditional GET
///   - If 304 Not Modified: use cache, mtime unchanged
///   - If 200 OK: download new file, mtime updates automatically
/// - If no cache or force_redownload: do full download
pub fn download_cotahist_year(
    year: i32,
    force_redownload: bool,
    progress_callback: Option<&dyn Fn(&DownloadProgress)>,
) -> Result<PathBuf> {
    let cache_dir = get_cotahist_cache_dir()?;
    let zip_path = cache_dir.join(format!("COTAHIST_A{}.ZIP", year));

    // Check cache first
    if !force_redownload && zip_path.exists() {
        // Check if we should do a conditional GET to see if file was updated
        let should_check = should_check_for_updates(year).unwrap_or(true);

        if should_check {
            tracing::debug!(
                "Checking for COTAHIST {} updates (>1 day since last check)",
                year
            );

            // Get current file mtime for If-Modified-Since header
            let metadata =
                std::fs::metadata(&zip_path).context("Failed to read cache file metadata")?;
            let mtime = metadata
                .modified()
                .context("Failed to get file modification time")?;

            let client = Client::builder()
                .timeout(std::time::Duration::from_secs(300)) // 5 minute timeout
                .build()
                .context("Failed to create HTTP client")?;

            let download_url = get_cotahist_url(year);

            // Attempt conditional download with If-Modified-Since
            let response = client
                .get(&download_url)
                .header("If-Modified-Since", httpdate::fmt_http_date(mtime))
                .send();

            // Mark that we checked (regardless of outcome)
            if let Err(e) = mark_last_checked(year) {
                tracing::warn!("Failed to mark last checked for year {}: {}", year, e);
            }

            match response {
                Ok(resp) if resp.status() == reqwest::StatusCode::NOT_MODIFIED => {
                    // File unchanged on server, use cache
                    // mtime stays the same → won't trigger re-import
                    tracing::debug!("COTAHIST {} not modified on server, using cache", year);

                    if let Some(callback) = progress_callback {
                        callback(&DownloadProgress {
                            stage: DownloadStage::Complete,
                            year,
                            records_processed: 0,
                            total_records: None,
                        });
                    }

                    return Ok(zip_path);
                }
                Ok(resp) if resp.status().is_success() => {
                    // New data available, update cache
                    // This will naturally update the file's mtime → triggers re-import
                    tracing::info!("COTAHIST {} has updates on server, refreshing cache", year);

                    if let Some(ref callback) = progress_callback {
                        callback(&DownloadProgress {
                            stage: DownloadStage::Downloading,
                            year,
                            records_processed: 0,
                            total_records: None,
                        });
                    }

                    let bytes = resp.bytes().context("Failed to read download response")?;

                    tracing::debug!("Downloaded {} bytes for year {}", bytes.len(), year);

                    // Save to cache (this updates mtime automatically)
                    std::fs::write(&zip_path, bytes)
                        .context("Failed to write COTAHIST to cache")?;

                    tracing::debug!("Updated cached COTAHIST to: {:?}", zip_path);

                    if let Some(callback) = progress_callback {
                        callback(&DownloadProgress {
                            stage: DownloadStage::Complete,
                            year,
                            records_processed: 0,
                            total_records: None,
                        });
                    }

                    return Ok(zip_path);
                }
                Ok(resp) => {
                    // Other status code (e.g., 404, 403), fall back to cache
                    tracing::warn!(
                        "Failed to check for updates (status {}), using cached COTAHIST {}",
                        resp.status(),
                        year
                    );
                }
                Err(e) => {
                    // Network error, fall back to cache
                    tracing::warn!(
                        "Network error checking for updates ({}), using cached COTAHIST {}",
                        e,
                        year
                    );
                }
            }
        } else {
            // Checked recently (< 1 day ago), use cache without HTTP request
            tracing::debug!("Using cached COTAHIST {} (checked recently)", year);
        }

        // Use cache
        if let Some(callback) = progress_callback {
            callback(&DownloadProgress {
                stage: DownloadStage::Complete,
                year,
                records_processed: 0,
                total_records: None,
            });
        }

        return Ok(zip_path);
    }

    // No cache or force_redownload - do full download
    // Create cache directory if needed
    std::fs::create_dir_all(&cache_dir).context("Failed to create cache directory")?;

    // Construct direct download URL
    let download_url = get_cotahist_url(year);
    tracing::info!("Downloading COTAHIST {} (no cache)", year);

    // Download file
    if let Some(ref callback) = progress_callback {
        callback(&DownloadProgress {
            stage: DownloadStage::Downloading,
            year,
            records_processed: 0,
            total_records: None,
        });
    }

    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(300)) // 5 minute timeout
        .build()
        .context("Failed to create HTTP client")?;

    let response = client
        .get(&download_url)
        .send()
        .context("Failed to download COTAHIST file")?;

    if !response.status().is_success() {
        return Err(anyhow!(
            "Download failed with status: {}. Year {} may not be available.",
            response.status(),
            year
        ));
    }

    let bytes = response
        .bytes()
        .context("Failed to read download response")?;

    tracing::debug!("Downloaded {} bytes for year {}", bytes.len(), year);

    // Save to cache
    std::fs::write(&zip_path, bytes).context("Failed to write COTAHIST to cache")?;

    tracing::debug!("Cached COTAHIST to: {:?}", zip_path);

    // Mark that we checked
    if let Err(e) = mark_last_checked(year) {
        tracing::warn!("Failed to mark last checked for year {}: {}", year, e);
    }

    if let Some(callback) = progress_callback {
        callback(&DownloadProgress {
            stage: DownloadStage::Complete,
            year,
            records_processed: 0,
            total_records: None,
        });
    }

    Ok(zip_path)
}

/// Parse a COTAHIST record line according to B3 specification
///
/// COTAHIST format is fixed-width, 245 bytes per line:
/// - Positions 01-02 (0..2): Record type (00=header, 01=data, 99=trailer)
/// - Positions 03-10 (2..10): Date (YYYYMMDD)
/// - Positions 13-24 (12..24): Ticker (CODNEG)
/// - Positions 57-69 (56..69): Opening price (PREABE) - 11 digits + 2 decimals
/// - Positions 70-82 (69..82): Max price (PREMAX)
/// - Positions 83-95 (82..95): Min price (PREMIN)
/// - Positions 109-121 (108..121): Close price (PREULT)
/// - Positions 171-188 (170..188): Total volume (VOLTOT)
///
/// All prices are stored as integers with 2 implied decimal places.
fn parse_cotahist_line(line: &str) -> Result<Option<CotahistRecord>> {
    // Check minimum length
    if line.len() < 245 {
        return Ok(None); // Skip incomplete lines
    }

    // Parse record type
    let record_type = &line[0..2];

    match record_type {
        "00" | "99" => return Ok(None), // Header/trailer - skip
        "01" => {}                      // Data record - process
        _ => return Ok(None),           // Unknown type - skip
    }

    // Parse date (YYYYMMDD)
    let date_str = &line[2..10];
    let date = NaiveDate::parse_from_str(date_str, "%Y%m%d")
        .with_context(|| format!("Invalid date: {}", date_str))?;

    // Parse ticker (12 chars, right-padded with spaces)
    let ticker = line[12..24].trim().to_string();

    // Skip if ticker is empty
    if ticker.is_empty() {
        return Ok(None);
    }

    // Helper function to parse price fields (13 chars, 2 implied decimals)
    let parse_price = |start: usize| -> Result<Decimal> {
        let end = start + 13;
        let price_str = &line[start..end];
        let price_int: i64 = price_str
            .parse()
            .with_context(|| format!("Invalid price at position {}: {}", start, price_str))?;

        Ok(Decimal::new(price_int, 2))
    };

    // Parse prices (all prices are 13 chars with 2 implied decimals)
    // PREABE - Open price (positions 57-69 in 1-based = 56-68 in 0-based)
    let open_price = parse_price(56)?;
    // PREMAX - High price (positions 70-82)
    let high_price = parse_price(69)?;
    // PREMIN - Low price (positions 83-95)
    let low_price = parse_price(82)?;
    // PREULT - Close price (positions 109-121)
    let close_price = parse_price(108)?;

    // Parse volume (VOLTOT - positions 171-188 in 1-based = 170-187 in 0-based)
    let volume_str = &line[170..188];
    let volume: i64 = volume_str
        .parse()
        .with_context(|| format!("Invalid volume: {}", volume_str))?;

    Ok(Some(CotahistRecord {
        date,
        ticker,
        open_price,
        high_price,
        low_price,
        close_price,
        volume,
    }))
}

/// Parse COTAHIST file from disk
pub fn parse_cotahist_file<P: AsRef<Path>>(
    zip_path: P,
    progress_callback: Option<&dyn Fn(&DownloadProgress)>,
) -> Result<Vec<CotahistRecord>> {
    let zip_path = zip_path.as_ref();

    // Extract year from filename
    let filename = zip_path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow!("Invalid file path"))?;

    let year: i32 = filename
        .chars()
        .filter(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .context("Could not extract year from filename")?;

    if let Some(ref callback) = progress_callback {
        callback(&DownloadProgress {
            stage: DownloadStage::Decompressing,
            year,
            records_processed: 0,
            total_records: None,
        });
    }

    // Open ZIP file
    let file = std::fs::File::open(zip_path).context("Failed to open ZIP file")?;

    let mut archive = ZipArchive::new(file).context("Failed to read ZIP archive")?;

    // Find the COTAHIST text file inside (usually the only file)
    let mut cotahist_file = archive.by_index(0).context("ZIP archive is empty")?;

    // Read entire file to string
    let mut contents = String::new();
    cotahist_file
        .read_to_string(&mut contents)
        .context("Failed to read COTAHIST file")?;

    if let Some(ref callback) = progress_callback {
        callback(&DownloadProgress {
            stage: DownloadStage::Parsing,
            year,
            records_processed: 0,
            total_records: Some(contents.lines().count()),
        });
    }

    // Parse all lines
    let mut records = Vec::new();
    let total_lines = contents.lines().count();

    for (idx, line) in contents.lines().enumerate() {
        if let Some(record) = parse_cotahist_line(line)? {
            records.push(record);
        }

        // Report progress every 10000 lines
        if let Some(ref callback) = progress_callback {
            if idx % 10000 == 0 || idx == total_lines - 1 {
                callback(&DownloadProgress {
                    stage: DownloadStage::Parsing,
                    year,
                    records_processed: idx + 1,
                    total_records: Some(total_lines),
                });
            }
        }
    }

    if let Some(callback) = progress_callback {
        callback(&DownloadProgress {
            stage: DownloadStage::Complete,
            year,
            records_processed: records.len(),
            total_records: Some(total_lines),
        });
    }

    tracing::info!("Parsed {} records from COTAHIST {}", records.len(), year);

    // Heuristic sanity check: official yearly files are large (hundreds of thousands of lines).
    // If we see very few records, surface a warning so users can catch a bad download early.
    if records.len() < 1000 {
        tracing::warn!(
            "COTAHIST {} parsed only {} records; file may be incorrect or truncated",
            year,
            records.len()
        );
    }

    Ok(records)
}

/// Import COTAHIST records into database
/// Import COTAHIST records to database (optimized with batching)
/// Only imports records for tickers that exist in the assets table
pub fn import_records_to_db(
    conn: &mut Connection,
    records: &[CotahistRecord],
    progress_callback: Option<&dyn Fn(&DownloadProgress)>,
    year: i32,
) -> Result<usize> {
    use std::collections::{HashMap, HashSet};

    // Pre-fetch all existing assets to avoid repeated lookups
    let mut asset_map: HashMap<String, i64> = HashMap::new();
    {
        let mut stmt = conn.prepare("SELECT id, ticker FROM assets")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(1)?, row.get::<_, i64>(0)?))
        })?;
        for row in rows {
            let (ticker, id) = row?;
            asset_map.insert(ticker, id);
        }
    }

    if asset_map.is_empty() {
        tracing::debug!("No assets in database - skipping COTAHIST import");
        return Ok(0);
    }

    tracing::info!(
        "Filtering COTAHIST records for {} portfolio assets",
        asset_map.len()
    );

    // Filter records to only those matching portfolio assets
    let relevant_records: Vec<_> = records
        .iter()
        .filter(|r| asset_map.contains_key(&r.ticker))
        .collect();

    tracing::info!(
        "Filtered {} relevant records from {} total",
        relevant_records.len(),
        records.len()
    );

    if relevant_records.is_empty() {
        return Ok(0);
    }

    // Get existing price dates to avoid duplicates (chunked queries to avoid SQL variable limit)
    let mut existing_prices: HashSet<(i64, NaiveDate)> = HashSet::new();
    {
        let asset_ids: Vec<i64> = asset_map.values().copied().collect();

        // SQLite has a limit on number of parameters (32766), so chunk queries
        const CHUNK_SIZE: usize = 500;

        for chunk in asset_ids.chunks(CHUNK_SIZE) {
            let placeholders = chunk.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            let query = format!(
                "SELECT asset_id, price_date FROM price_history WHERE asset_id IN ({})",
                placeholders
            );

            let mut stmt = conn.prepare(&query)?;
            let params: Vec<&dyn rusqlite::ToSql> =
                chunk.iter().map(|id| id as &dyn rusqlite::ToSql).collect();
            let rows = stmt.query_map(params.as_slice(), |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?;

            for row in rows {
                let (asset_id, date_str) = row?;
                if let Ok(date) = chrono::NaiveDate::parse_from_str(&date_str, "%Y-%m-%d") {
                    existing_prices.insert((asset_id, date));
                }
            }
        }
    }

    tracing::debug!("Found {} existing price records", existing_prices.len());

    // Batch insert with single transaction (much faster)
    let tx = conn.transaction()?;
    let mut inserted = 0;

    {
        let mut stmt = tx.prepare(
            "INSERT OR IGNORE INTO price_history (asset_id, price_date, close_price, open_price, high_price, low_price, volume, source)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'B3_COTAHIST')"
        )?;

        for record in relevant_records.iter() {
            let asset_id = match asset_map.get(&record.ticker) {
                Some(id) => *id,
                None => continue, // Should never happen due to filter above
            };

            // Skip if exists
            if existing_prices.contains(&(asset_id, record.date)) {
                continue;
            }

            stmt.execute(rusqlite::params![
                asset_id,
                record.date.format("%Y-%m-%d").to_string(),
                record.close_price.to_string(),
                record.open_price.to_string(),
                record.high_price.to_string(),
                record.low_price.to_string(),
                record.volume,
            ])?;

            inserted += 1;
        }
    }

    tx.commit()?;
    tracing::info!("Imported {} new price records", inserted);

    // Report import completion with fun emoji
    if let Some(callback) = progress_callback {
        callback(&DownloadProgress {
            stage: DownloadStage::Complete,
            year,
            records_processed: inserted,
            total_records: Some(records.len()),
        });
    }

    Ok(inserted)
}

/// Download and import COTAHIST for a specific year (main entry point)
pub fn import_cotahist_year(
    conn: &mut Connection,
    year: i32,
    force_redownload: bool,
    progress_callback: Option<&dyn Fn(&DownloadProgress)>,
) -> Result<usize> {
    // Download (or use cache)
    let zip_path = download_cotahist_year(year, force_redownload, progress_callback)?;

    // Parse records
    let records = parse_cotahist_file(&zip_path, progress_callback)?;

    // Import to database
    let inserted = import_records_to_db(conn, &records, progress_callback, year)?;

    tracing::info!("Imported {} new price records for year {}", inserted, year);

    // Mark that we've imported this version of the file (stores mtime)
    if let Err(e) = mark_cotahist_imported(year) {
        tracing::warn!("Failed to mark COTAHIST {} as imported: {}", year, e);
    }

    Ok(inserted)
}

/// Import COTAHIST from a user-provided ZIP file (manual download flow)
pub fn import_cotahist_from_file<P: AsRef<Path>>(
    conn: &mut Connection,
    zip_path: P,
) -> Result<usize> {
    // Parse records (no progress callback for manual flow)
    let records = parse_cotahist_file(zip_path, None)?;

    // Import to database (no progress callback for manual flow)
    let inserted = import_records_to_db(conn, &records, None, 0)?;

    Ok(inserted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_cotahist_line_valid() {
        // Sample line from B3's official demo file (DemoCotacoesHistoricas12022003.txt)
        // VALE3 stock from 2003-02-12
        let line = "012003021202VALE3       010VALE R DOCE ON           R$  000000001050100000000105010000000010250000000001036800000000103210000000010321000000001043800142000000000000069500000000000720641400000000000000009999123100000010000000000000BRVALEACNOR0159";

        let record = parse_cotahist_line(line).unwrap();
        assert!(record.is_some());

        let record = record.unwrap();
        assert_eq!(record.ticker, "VALE3");
        assert_eq!(record.date, NaiveDate::from_ymd_opt(2003, 2, 12).unwrap());
        // Open: 105.01
        assert_eq!(record.open_price, Decimal::new(10501, 2));
        // High: 105.01
        assert_eq!(record.high_price, Decimal::new(10501, 2));
        // Low: 102.50
        assert_eq!(record.low_price, Decimal::new(10250, 2));
        // Close: 103.21
        assert_eq!(record.close_price, Decimal::new(10321, 2));
    }

    #[test]
    fn test_parse_cotahist_line_header() {
        let line = "00COTAHIST.2023BOVESPA 20231231                                                                                                                                                                                                                      ";

        let record = parse_cotahist_line(line).unwrap();
        assert!(record.is_none()); // Header should be skipped
    }

    #[test]
    fn test_get_cotahist_url() {
        assert_eq!(
            get_cotahist_url(2025),
            "https://bvmf.bmfbovespa.com.br/InstDados/SerHist/COTAHIST_A2025.ZIP"
        );

        assert_eq!(
            get_cotahist_url(2023),
            "https://bvmf.bmfbovespa.com.br/InstDados/SerHist/COTAHIST_A2023.ZIP"
        );
    }

    #[test]
    fn test_cache_dir_creation() {
        let cache_dir = get_cotahist_cache_dir().unwrap();
        assert!(cache_dir.to_str().unwrap().contains("interest"));
        assert!(cache_dir.to_str().unwrap().contains("cotahist"));
    }
}
