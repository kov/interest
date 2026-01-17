use anyhow::{anyhow, Context, Result};
use chrono::{NaiveDate, NaiveDateTime};
use encoding_rs::ISO_8859_15;
use headless_chrome::Browser;
use reqwest::blocking::Client;
use rusqlite::Connection;
use rust_decimal::Decimal;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::db::{Asset, GovBondRate, PriceHistory};
use crate::tesouro;

const TESOURO_CSV_URL: &str = "https://www.tesourotransparente.gov.br/ckan/dataset/df56aa42-484a-4a59-8184-7676580c81e3/resource/796d2059-14e9-44e3-80c9-2d9e30b405c1/download/precotaxatesourodireto.csv";
#[allow(dead_code)]
const TESOURO_RESGATAR_URL: &str = "https://www.tesourodireto.com.br/o/rentabilidade/resgatar";
const CACHE_FILENAME: &str = "precotaxatesourodireto.csv";
const CACHE_MAX_AGE_HOURS: i64 = 24;

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ResgatarBond {
    #[serde(rename = "treasuryBondName")]
    name: String,
    #[serde(rename = "maturityDate")]
    maturity_date: String,
    #[serde(rename = "redemptionProfitabilityFeeIndexerName")]
    redemption_rate: String,
    #[serde(rename = "unitaryRedemptionValue")]
    redemption_value: f64,
}

pub fn get_tesouro_cache_dir() -> Result<PathBuf> {
    let cache_dir =
        dir_spec::cache_home().ok_or_else(|| anyhow!("Could not determine cache directory"))?;
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
    import_tesouro_csv_from_content(conn, assets, start_date, end_date, &content)
}

#[allow(dead_code)]
pub fn fetch_current_resgatar_prices(conn: &Connection, assets: &[Asset]) -> Result<usize> {
    if assets.is_empty() {
        return Ok(0);
    }

    let bytes = fetch_resgatar_bytes()?;
    let decoded = decode_resgatar_bytes(&bytes);
    let payload = extract_json_payload(&decoded)?;
    let records: Vec<ResgatarBond> =
        serde_json::from_str(&payload).context("Failed to parse Tesouro resgatar JSON")?;

    let mut map: HashMap<String, (Decimal, Option<Decimal>)> = HashMap::new();
    for bond in records {
        let ticker = tesouro::ticker_from_name(&bond.name).or_else(|| {
            parse_maturity_date(&bond.maturity_date)
                .ok()
                .and_then(|date| tesouro::ticker_from_type_and_maturity(&bond.name, date))
        });
        let ticker = match ticker {
            Some(t) => t,
            None => continue,
        };

        let price = Decimal::try_from(bond.redemption_value)
            .map_err(|err| anyhow!("Invalid redemption price for {}: {}", bond.name, err))?;
        let rate = tesouro::extract_rate_percent(&bond.redemption_rate);

        map.insert(ticker, (price, rate));
    }

    let today = chrono::Local::now().date_naive();
    let asset_map = build_asset_map(assets);
    let mut inserted = 0usize;

    for (ticker, asset_id) in asset_map {
        let Some((price_value, rate_value)) = map.get(&ticker) else {
            continue;
        };

        let price = PriceHistory {
            id: None,
            asset_id,
            price_date: today,
            close_price: *price_value,
            open_price: None,
            high_price: None,
            low_price: None,
            volume: None,
            source: "TESOURO_RESGATAR".to_string(),
            created_at: chrono::Utc::now(),
        };
        crate::db::insert_price_history(conn, &price)?;

        if let Some(rate_value) = rate_value {
            let rate = GovBondRate {
                id: None,
                asset_id,
                price_date: today,
                sell_rate: *rate_value,
                source: Some("TESOURO_RESGATAR".to_string()),
                created_at: chrono::Utc::now(),
            };
            crate::db::insert_gov_bond_rate(conn, &rate)?;
        }

        inserted += 1;
    }

    Ok(inserted)
}

#[allow(dead_code)]
fn fetch_resgatar_bytes() -> Result<Vec<u8>> {
    let client = Client::new();
    if let Ok(response) = client.get(TESOURO_RESGATAR_URL).send() {
        if response.status().is_success() {
            if let Ok(bytes) = response.bytes() {
                let raw = bytes.to_vec();
                if is_json_array(&raw) {
                    return Ok(raw);
                }
            }
        }
    }

    let browser = Browser::default().context("Failed to start headless Chrome")?;
    let tab = browser
        .new_tab()
        .context("Failed to open tab for Tesouro resgatar")?;

    tab.navigate_to(TESOURO_RESGATAR_URL)
        .context("Failed to navigate to Tesouro resgatar endpoint")?;
    tab.wait_for_element_with_custom_timeout("body", Duration::from_secs(10))
        .context("Timed out waiting for Tesouro resgatar response")?;

    let fetch_script = format!(
        "(async function() {{\n  const res = await fetch('{}');\n  const json = await res.json();\n  return JSON.stringify(json);\n}})();",
        TESOURO_RESGATAR_URL
    );

    for _ in 0..20 {
        if let Ok(result) = tab.evaluate("document.body.innerText", false) {
            if let Some(value) = result.value.as_ref().and_then(|v| v.as_str()) {
                eprintln!(
                    "tesouro resgatar innerText (snippet): {}",
                    &value.chars().take(2000).collect::<String>()
                );
                if value.trim_start().starts_with('[') {
                    return Ok(value.to_string().into_bytes());
                }
            }
        }

        if let Ok(result) = tab.evaluate(&fetch_script, true) {
            if let Some(value) = result.value.as_ref().and_then(|v| v.as_str()) {
                if value.trim_start().starts_with('[') {
                    return Ok(value.to_string().into_bytes());
                }
            }
        }

        if let Ok(pre) = tab.find_element("pre") {
            let text = pre
                .get_inner_text()
                .context("Failed to read resgatar JSON")?;
            if text.trim_start().starts_with('[') {
                return Ok(text.into_bytes());
            }
        }

        std::thread::sleep(Duration::from_millis(500));
    }

    Err(anyhow!(
        "Tesouro resgatar JSON not available after challenge"
    ))
}

#[allow(dead_code)]
fn decode_resgatar_bytes(bytes: &[u8]) -> String {
    let (decoded, _, _) = ISO_8859_15.decode(bytes);
    decoded.into_owned()
}

#[allow(dead_code)]
fn is_json_array(bytes: &[u8]) -> bool {
    bytes
        .iter()
        .copied()
        .find(|b| !b.is_ascii_whitespace())
        .map(|b| b == b'[')
        .unwrap_or(false)
}

#[allow(dead_code)]
fn extract_json_payload(decoded: &str) -> Result<String> {
    let trimmed = decoded.trim();
    if trimmed.starts_with('[') {
        return Ok(trimmed.to_string());
    }

    let start = trimmed
        .find('[')
        .ok_or_else(|| anyhow!("Missing JSON array"))?;
    let end = trimmed
        .rfind(']')
        .ok_or_else(|| anyhow!("Missing JSON array end"))?;
    if end <= start {
        return Err(anyhow!("Invalid JSON array bounds"));
    }

    Ok(trimmed[start..=end].to_string())
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

#[allow(dead_code)]
fn parse_maturity_date(value: &str) -> Result<NaiveDate> {
    let parsed = NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M")
        .context(format!("Invalid maturity date: {}", value))?;
    Ok(parsed.date())
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

    #[test]
    fn test_decode_resgatar_bytes_iso() {
        let input = vec![b'T', 0xed, b't', b'u'];
        let decoded = decode_resgatar_bytes(&input);
        assert!(decoded.contains('í'));
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
}
