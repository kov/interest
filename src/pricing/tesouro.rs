use anyhow::{anyhow, Context, Result};
use chrono::NaiveDate;
use reqwest::blocking::Client;
use rusqlite::Connection;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::db::{Asset, GovBondRate, PriceHistory};
use crate::tesouro;

const TESOURO_CSV_URL: &str = "https://www.tesourotransparente.gov.br/ckan/dataset/df56aa42-484a-4a59-8184-7676580c81e3/resource/796d2059-14e9-44e3-80c9-2d9e30b405c1/download/precotaxatesourodireto.csv";
const CACHE_FILENAME: &str = "precotaxatesourodireto.csv";
const CACHE_MAX_AGE_HOURS: i64 = 24;

pub fn get_tesouro_cache_dir() -> Result<PathBuf> {
    let cache_dir = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(dir_spec::cache_home)
        .ok_or_else(|| anyhow!("Could not determine cache directory"))?;
    Ok(cache_dir.join("interest").join("tesouro"))
}

pub fn refresh_tesouro_csv(force: bool) -> Result<PathBuf> {
    let cache_dir = get_tesouro_cache_dir()?;
    fs::create_dir_all(&cache_dir).context("Failed to create Tesouro cache directory")?;

    let csv_path = cache_dir.join(CACHE_FILENAME);
    if !force && csv_path.exists() && !cache_is_stale(&csv_path)? {
        return Ok(csv_path);
    }

    let client = Client::new();
    let response = client
        .get(TESOURO_CSV_URL)
        .send()
        .context("Failed to download Tesouro CSV")?
        .error_for_status()
        .context("Tesouro CSV returned error status")?;

    let bytes = response
        .bytes()
        .context("Failed to read Tesouro CSV bytes")?;
    let tmp_path = cache_dir.join(format!("{}.tmp", CACHE_FILENAME));
    fs::write(&tmp_path, &bytes).context("Failed to write Tesouro CSV cache")?;
    fs::rename(&tmp_path, &csv_path).context("Failed to finalize Tesouro CSV cache file")?;

    Ok(csv_path)
}

pub fn import_tesouro_csv(
    conn: &Connection,
    assets: &[Asset],
    start_date: NaiveDate,
    end_date: NaiveDate,
) -> Result<usize> {
    if assets.is_empty() {
        return Ok(0);
    }

    let csv_path = refresh_tesouro_csv(false)?;
    let content = fs::read_to_string(&csv_path).context("Failed to read Tesouro CSV file")?;
    let count = import_tesouro_csv_from_content(conn, assets, start_date, end_date, &content)?;

    // Mark that we've imported this version of the file (stores mtime)
    if let Err(e) = mark_tesouro_imported() {
        tracing::warn!("Failed to mark Tesouro CSV as imported: {}", e);
    }

    Ok(count)
}

fn find_header(headers: &csv::StringRecord, name: &str) -> Result<usize> {
    headers
        .iter()
        .position(|h| h.trim().eq_ignore_ascii_case(name))
        .ok_or_else(|| anyhow!("Missing Tesouro CSV column: {}", name))
}

fn parse_date_br(value: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(value, "%d/%m/%Y")
        .context(format!("Invalid date in Tesouro CSV: {}", value))
}

fn build_asset_map(assets: &[Asset]) -> HashMap<String, i64> {
    assets
        .iter()
        .filter_map(|asset| asset.id.map(|id| (asset.ticker.clone(), id)))
        .collect()
}

fn cache_is_stale(csv_path: &Path) -> Result<bool> {
    let metadata = fs::metadata(csv_path).context("Failed to read Tesouro CSV metadata")?;
    let modified = metadata
        .modified()
        .context("Failed to read Tesouro CSV mtime")?;
    let age = SystemTime::now()
        .duration_since(modified)
        .unwrap_or(Duration::from_secs(0));
    Ok(age.as_secs() > (CACHE_MAX_AGE_HOURS as u64) * 3600)
}

/// Get the modification time of the cached Tesouro CSV file as Unix timestamp
fn get_cache_file_mtime() -> Result<Option<i64>> {
    let cache_dir = get_tesouro_cache_dir()?;
    let csv_path = cache_dir.join(CACHE_FILENAME);

    if !csv_path.exists() {
        return Ok(None);
    }

    let metadata = fs::metadata(&csv_path).context("Failed to read Tesouro CSV metadata")?;
    let mtime = metadata
        .modified()
        .context("Failed to get file modification time")?
        .duration_since(std::time::UNIX_EPOCH)
        .context("Invalid file modification time")?
        .as_secs() as i64;

    Ok(Some(mtime))
}

/// Check if the Tesouro CSV has been imported (based on mtime)
///
/// Pass a connection to use a specific database (for tests), or None for the default database
pub fn has_tesouro_been_imported_with_conn(conn_opt: Option<&Connection>) -> Result<bool> {
    let current_mtime = match get_cache_file_mtime()? {
        Some(mt) => mt,
        None => return Ok(false), // No cache file
    };

    let key = "tesouro_csv_imported_mtime";

    let stored_mtime = match conn_opt {
        Some(conn) => crate::db::get_metadata(conn, key)?,
        None => {
            let conn = crate::db::open_db(None)?;
            crate::db::get_metadata(&conn, key)?
        }
    };

    match stored_mtime {
        Some(stored) => Ok(stored == current_mtime.to_string()),
        None => Ok(false), // Never imported
    }
}

/// Check if the Tesouro CSV has been imported (uses default database)
pub fn has_tesouro_been_imported() -> Result<bool> {
    has_tesouro_been_imported_with_conn(None)
}

/// Mark that the Tesouro CSV has been imported (stores file's current mtime)
pub fn mark_tesouro_imported() -> Result<()> {
    let conn = crate::db::open_db(None)?;
    let mtime =
        get_cache_file_mtime()?.ok_or_else(|| anyhow!("Tesouro CSV cache file not found"))?;

    let key = "tesouro_csv_imported_mtime";
    crate::db::set_metadata(&conn, key, &mtime.to_string())
}

fn import_tesouro_csv_from_content(
    conn: &Connection,
    assets: &[Asset],
    start_date: NaiveDate,
    end_date: NaiveDate,
    content: &str,
) -> Result<usize> {
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(b';')
        .from_reader(content.as_bytes());
    let headers = reader.headers()?.clone();

    let tipo_idx = find_header(&headers, "Tipo Titulo")?;
    let venc_idx = find_header(&headers, "Data Vencimento")?;
    let base_idx = find_header(&headers, "Data Base")?;
    let taxa_venda_idx = find_header(&headers, "Taxa Venda Manha")?;
    let pu_venda_idx = find_header(&headers, "PU Venda Manha")?;

    let asset_map = build_asset_map(assets);
    let mut inserted = 0usize;

    for result in reader.records() {
        let record = result?;
        let tipo = record.get(tipo_idx).unwrap_or("").trim();
        let venc = record.get(venc_idx).unwrap_or("").trim();
        let base = record.get(base_idx).unwrap_or("").trim();
        let taxa_venda = record.get(taxa_venda_idx).unwrap_or("").trim();
        let pu_venda = record.get(pu_venda_idx).unwrap_or("").trim();

        if tipo.is_empty() || venc.is_empty() || base.is_empty() || pu_venda.is_empty() {
            continue;
        }

        let maturity = parse_date_br(venc)?;
        let base_date = parse_date_br(base)?;
        if base_date < start_date || base_date > end_date {
            continue;
        }

        let ticker = match tesouro::ticker_from_type_and_maturity(tipo, maturity) {
            Some(t) => t,
            None => continue,
        };

        let asset_id = match asset_map.get(&ticker) {
            Some(id) => *id,
            None => continue,
        };

        let close_price = match tesouro::parse_decimal_br(pu_venda) {
            Ok(value) => value,
            Err(_) => continue,
        };

        let price = PriceHistory {
            id: None,
            asset_id,
            price_date: base_date,
            close_price,
            open_price: None,
            high_price: None,
            low_price: None,
            volume: None,
            source: "TESOURO_CSV".to_string(),
            created_at: chrono::Utc::now(),
        };
        crate::db::insert_price_history(conn, &price)?;

        if !taxa_venda.is_empty() {
            if let Ok(rate_value) = tesouro::parse_decimal_br(taxa_venda) {
                let rate = GovBondRate {
                    id: None,
                    asset_id,
                    price_date: base_date,
                    sell_rate: rate_value,
                    source: Some("TESOURO_CSV".to_string()),
                    created_at: chrono::Utc::now(),
                };
                crate::db::insert_gov_bond_rate(conn, &rate)?;
            }
        }

        inserted += 1;
    }

    Ok(inserted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Datelike;
    use rusqlite::Connection;
    use tempfile::TempDir;

    fn with_temp_cache_dir<T>(f: impl FnOnce(&Path) -> T) -> T {
        static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = ENV_LOCK.lock().unwrap();
        let temp_dir = TempDir::new().unwrap();
        let old = std::env::var_os("XDG_CACHE_HOME");
        std::env::set_var("XDG_CACHE_HOME", temp_dir.path());
        let result = f(temp_dir.path());
        match old {
            Some(value) => std::env::set_var("XDG_CACHE_HOME", value),
            None => std::env::remove_var("XDG_CACHE_HOME"),
        }
        result
    }

    #[test]
    fn test_parse_date_br() {
        let date = parse_date_br("17/09/2007").unwrap();
        assert_eq!(date.year(), 2007);
    }

    #[test]
    fn test_import_tesouro_csv_from_content() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!("../db/schema.sql"))
            .unwrap();

        conn.execute(
            "INSERT INTO assets (ticker, asset_type) VALUES (?1, ?2)",
            rusqlite::params!["TESOURO_IPCA_JUROS_2045", "GOV_BOND"],
        )
        .unwrap();
        let asset_id = conn.last_insert_rowid();

        let asset = Asset {
            id: Some(asset_id),
            ticker: "TESOURO_IPCA_JUROS_2045".to_string(),
            asset_type: crate::db::AssetType::GovBond,
            name: None,
            cnpj: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        let csv = "Tipo Titulo;Data Vencimento;Data Base;Taxa Compra Manha;Taxa Venda Manha;PU Compra Manha;PU Venda Manha;PU Base Manha\n\
Tesouro IPCA+ com Juros Semestrais;15/05/2045;17/09/2007;6,37;6,47;1617,98;1595,98;1595,39\n";

        let count = import_tesouro_csv_from_content(
            &conn,
            &[asset],
            NaiveDate::from_ymd_opt(2007, 9, 17).unwrap(),
            NaiveDate::from_ymd_opt(2007, 9, 17).unwrap(),
            csv,
        )
        .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_import_tesouro_csv_from_fixture() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!("../db/schema.sql"))
            .unwrap();

        conn.execute(
            "INSERT INTO assets (ticker, asset_type) VALUES (?1, ?2)",
            rusqlite::params!["TESOURO_IPCA_JUROS_2035", "GOV_BOND"],
        )
        .unwrap();
        let asset_id = conn.last_insert_rowid();

        let asset = Asset {
            id: Some(asset_id),
            ticker: "TESOURO_IPCA_JUROS_2035".to_string(),
            asset_type: crate::db::AssetType::GovBond,
            name: None,
            cnpj: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        let csv = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/tesouro_precotaxatesourodireto.csv"
        ));
        let base_date = NaiveDate::from_ymd_opt(2012, 12, 5).unwrap();
        let count =
            import_tesouro_csv_from_content(&conn, &[asset], base_date, base_date, csv).unwrap();
        assert!(count >= 1);
    }

    #[test]
    #[ignore]
    fn test_refresh_tesouro_csv_online() {
        with_temp_cache_dir(|_| {
            let path = refresh_tesouro_csv(true).unwrap();
            let content = fs::read_to_string(path).unwrap();
            assert!(content.contains("Tipo Titulo"));
        });
    }
}
