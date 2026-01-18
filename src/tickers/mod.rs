use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Local, Utc};
use encoding_rs::ISO_8859_15;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;
use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};

use crate::db::AssetType;
pub(crate) mod ambima;

const B3_REQUEST_BASE_URL: &str = "https://arquivos.b3.com.br/api/download/requestname?fileName=InstrumentsConsolidatedFile&date=";
const B3_API_BASE_URL: &str = "https://arquivos.b3.com.br/api";
const CACHE_FILENAME: &str = "tickers.csv";
const META_FILENAME: &str = "tickers.meta.json";
const CACHE_MAX_AGE_HOURS: i64 = 24;

#[derive(Debug, Clone)]
pub struct TickerRecord {
    pub ticker: String,
    pub security_category: String,
    pub cfi_code: Option<String>,
    pub corporate_name: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct TickersMeta {
    fetched_at: DateTime<Utc>,
    source_url: String,
}

#[derive(Debug, Clone)]
pub struct TickersMetaInfo {
    pub fetched_at: DateTime<Utc>,
    pub source_url: String,
}

#[derive(Debug, Deserialize)]
struct B3DownloadResponse {
    #[serde(rename = "redirectUrl")]
    redirect_url: Option<String>,
    token: Option<String>,
}

#[derive(Debug)]
struct CachedMap {
    map: Arc<HashMap<String, TickerRecord>>,
    mtime: SystemTime,
}

static TICKERS_CACHE: OnceLock<Mutex<Option<CachedMap>>> = OnceLock::new();

pub fn get_tickers_cache_dir() -> Result<PathBuf> {
    let cache_dir = dir_spec::cache_home()
        .ok_or_else(|| anyhow::anyhow!("Could not determine cache directory"))?;

    Ok(cache_dir.join("interest").join("tickers"))
}

pub fn refresh_b3_tickers(force: bool) -> Result<PathBuf> {
    let cache_dir = get_tickers_cache_dir()?;
    fs::create_dir_all(&cache_dir).context("Failed to create tickers cache directory")?;

    let csv_path = cache_dir.join(CACHE_FILENAME);
    if !force && csv_path.exists() && !cache_is_stale(&cache_dir)? {
        return Ok(csv_path);
    }

    let today = Local::now().date_naive();
    let (bytes, download_url) = download_b3_tickers(today)?;

    let tmp_path = cache_dir.join(format!("{}.tmp", CACHE_FILENAME));
    fs::write(&tmp_path, &bytes).context("Failed to write B3 tickers file")?;
    fs::rename(&tmp_path, &csv_path).context("Failed to finalize B3 tickers cache file")?;

    let meta = TickersMeta {
        fetched_at: Utc::now(),
        source_url: download_url,
    };
    let meta_path = cache_dir.join(META_FILENAME);
    fs::write(&meta_path, serde_json::to_vec_pretty(&meta)?)
        .context("Failed to write tickers metadata")?;

    {
        let mut guard = cache_guard();
        *guard = None;
    }

    Ok(csv_path)
}

pub fn read_cache_meta(cache_dir: Option<&Path>) -> Result<Option<TickersMetaInfo>> {
    let cache_dir = match cache_dir {
        Some(path) => path.to_path_buf(),
        None => get_tickers_cache_dir()?,
    };
    let meta_path = cache_dir.join(META_FILENAME);
    if !meta_path.exists() {
        return Ok(None);
    }
    let meta_bytes = fs::read(&meta_path).context("Failed to read tickers metadata")?;
    let meta: TickersMeta =
        serde_json::from_slice(&meta_bytes).context("Failed to parse tickers metadata")?;
    Ok(Some(TickersMetaInfo {
        fetched_at: meta.fetched_at,
        source_url: meta.source_url,
    }))
}

pub fn load_b3_tickers_map(cache_dir: Option<&Path>) -> Result<HashMap<String, TickerRecord>> {
    let cache_dir = match cache_dir {
        Some(path) => path.to_path_buf(),
        None => get_tickers_cache_dir()?,
    };
    let csv_path = cache_dir.join(CACHE_FILENAME);
    let bytes = fs::read(&csv_path).context("Failed to read cached tickers CSV")?;

    let (decoded, _, _) = ISO_8859_15.decode(&bytes);
    let content = decoded.into_owned();
    let (cleaned, delimiter) = normalize_csv_content(&content)?;

    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .from_reader(cleaned.as_bytes());
    let headers = reader.headers()?.clone();

    let mut map: HashMap<String, TickerRecord> = HashMap::new();
    for result in reader.records() {
        let record = result?;
        let ticker = get_field(&record, &headers, "TckrSymb");
        if ticker.is_empty() {
            continue;
        }
        let ticker = ticker.to_ascii_uppercase();
        if map.contains_key(&ticker) {
            tracing::error!("Duplicate ticker in B3 list: {}", ticker);
            continue;
        }

        let security_category = get_field(&record, &headers, "SctyCtgyNm")
            .to_ascii_uppercase()
            .trim()
            .to_string();
        let cfi_code = get_field(&record, &headers, "CFICd");
        let corporate_name = get_field(&record, &headers, "CrpnNm");

        map.insert(
            ticker.clone(),
            TickerRecord {
                ticker,
                security_category,
                cfi_code: if cfi_code.is_empty() {
                    None
                } else {
                    Some(cfi_code.to_ascii_uppercase())
                },
                corporate_name: if corporate_name.is_empty() {
                    None
                } else {
                    Some(corporate_name.to_string())
                },
            },
        );
    }

    Ok(map)
}

pub fn resolve_asset_type_with_name(ticker: &str, name: Option<&str>) -> Result<Option<AssetType>> {
    let normalized = ticker.trim().to_ascii_uppercase();
    if normalized.starts_with("TESOURO_") {
        return Ok(Some(AssetType::GovBond));
    }

    if crate::term_contracts::is_term_contract(&normalized) {
        return Ok(Some(AssetType::TermContract));
    }

    let lookup_ticker = normalized.clone();

    let cache_dir = get_tickers_cache_dir()?;
    let csv_path = cache_dir.join(CACHE_FILENAME);
    if !csv_path.exists() {
        if let Err(err) = refresh_b3_tickers(true) {
            tracing::warn!("Failed to download B3 tickers list: {}", err);
            return Ok(None);
        }
    }

    let map = get_cached_map(&cache_dir)?;
    if let Some(record) = map.get(&lookup_ticker) {
        return Ok(map_record_to_asset_type(record));
    }
    if let Some(record) = find_record_by_name(&map, &lookup_ticker, name) {
        return Ok(map_record_to_asset_type(record));
    }
    if let Some(record) = find_record_by_prefix(&map, &lookup_ticker) {
        return Ok(map_record_to_asset_type(record));
    }

    if let Some(asset_type) = registry_asset_type_lookup(&normalized)? {
        return Ok(Some(asset_type));
    }

    if cache_is_stale(&cache_dir)? {
        if let Err(err) = refresh_b3_tickers(true) {
            tracing::warn!("Failed to refresh B3 tickers list: {}", err);
        } else {
            let refreshed_map = get_cached_map(&cache_dir)?;
            if let Some(record) = refreshed_map.get(&lookup_ticker) {
                return Ok(map_record_to_asset_type(record));
            }
            if let Some(record) = find_record_by_name(&refreshed_map, &lookup_ticker, name) {
                return Ok(map_record_to_asset_type(record));
            }
            if let Some(record) = find_record_by_prefix(&refreshed_map, &lookup_ticker) {
                return Ok(map_record_to_asset_type(record));
            }
        }
    }

    if let Some(asset_type) = ambima_debenture_lookup(&normalized)? {
        return Ok(Some(asset_type));
    }

    Ok(None)
}

fn registry_asset_type_lookup(ticker: &str) -> Result<Option<AssetType>> {
    let conn = match crate::db::open_db(None) {
        Ok(conn) => conn,
        Err(err) => {
            tracing::warn!("Registry lookup failed to open DB: {}", err);
            return Ok(None);
        }
    };

    if let Some(entry) = crate::db::get_asset_registry_by_ticker(&conn, "MAIS_RETORNO", ticker)? {
        return Ok(Some(entry.asset_type));
    }

    if should_refresh_registry(&conn)? {
        if let Err(err) = refresh_registry_and_wait() {
            tracing::warn!("Mais Retorno registry refresh failed: {}", err);
        } else if let Some(entry) =
            crate::db::get_asset_registry_by_ticker(&conn, "MAIS_RETORNO", ticker)?
        {
            return Ok(Some(entry.asset_type));
        }
    }

    Ok(None)
}

fn should_refresh_registry(conn: &rusqlite::Connection) -> Result<bool> {
    use chrono::{DateTime, Duration, Utc};

    let last = crate::db::get_metadata(conn, "registry_maisretorno_refreshed_at")?;
    let Some(last) = last else {
        return Ok(true);
    };
    let Ok(parsed) = DateTime::parse_from_rfc3339(&last) else {
        return Ok(true);
    };
    let last = parsed.with_timezone(&Utc);
    Ok(Utc::now().signed_duration_since(last) > Duration::days(1))
}

fn refresh_registry_and_wait() -> Result<()> {
    let handle = std::thread::spawn(refresh_registry_blocking);
    match handle.join() {
        Ok(result) => result,
        Err(_) => Err(anyhow::anyhow!("Mais Retorno refresh thread panicked")),
    }
}

fn refresh_registry_blocking() -> Result<()> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let conn = crate::db::open_db(None)?;
        let sources = crate::scraping::maisretorno::select_sources(None);
        let printer = crate::ui::progress::ProgressPrinter::new(false);
        let (tx, mut rx) =
            tokio::sync::mpsc::unbounded_channel::<crate::ui::progress::ProgressEvent>();
        let progress_handle = tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                printer.handle_event(&event);
            }
        });

        let _stats =
            crate::scraping::maisretorno::sync_registry(&conn, &sources, false, Some(tx)).await?;
        let _ = progress_handle.await;
        crate::ui::progress::clear_progress_line();
        Ok(())
    })
}

pub fn ambima_debenture_lookup(ticker: &str) -> Result<Option<AssetType>> {
    if std::env::var("INTEREST_SKIP_AMBIMA").ok().as_deref() == Some("1") {
        return Ok(None);
    }
    match crate::tickers::ambima::is_debenture(ticker) {
        Ok(true) => Ok(Some(AssetType::Bond)),
        Ok(false) => Ok(None),
        Err(err) => {
            tracing::warn!("Ambima lookup failed for {}: {}", ticker, err);
            Ok(None)
        }
    }
}

fn map_record_to_asset_type(record: &TickerRecord) -> Option<AssetType> {
    match record.security_category.as_str() {
        "SHARES" => Some(AssetType::Stock),
        "BDR" => Some(AssetType::Bdr),
        "OPTION ON EQUITIES" => Some(AssetType::Option),
        "FUNDS" => classify_fund_by_name(record),
        "ETF EQUITIES" => Some(AssetType::Etf),
        "ETF FOREIGN INDEX" => classify_foreign_etf(record).or(Some(AssetType::Etf)),
        "DEBENTURES" | "BONDS" | "CORPORATE BONDS" => Some(AssetType::Bond),
        "GOVERNMENT" | "TITULOS PUBLICOS" => Some(AssetType::GovBond),
        _ => None,
    }
}

fn classify_fund_by_name(record: &TickerRecord) -> Option<AssetType> {
    let name = record.corporate_name.as_deref().unwrap_or("");
    let normalized = normalize_name(name);

    if normalized.is_empty() {
        return classify_by_cfi_hint(record.cfi_code.as_deref());
    }

    if contains_any(&normalized, &FOF_KEYWORDS) {
        return Some(AssetType::Fii);
    }

    if contains_any(&normalized, &FIAGRO_KEYWORDS) {
        return Some(AssetType::Fiagro);
    }

    if contains_any(&normalized, &FI_INFRA_KEYWORDS) {
        return Some(AssetType::FiInfra);
    }

    if contains_any(&normalized, &FIP_KEYWORDS) {
        return Some(AssetType::Fip);
    }

    if contains_any(&normalized, &FII_KEYWORDS) {
        return Some(AssetType::Fii);
    }

    if contains_any(&normalized, &FIDC_STRONG_KEYWORDS)
        || (contains_any(&normalized, &FIDC_WEAK_KEYWORDS)
            && !contains_any(&normalized, &FII_KEYWORDS))
    {
        return Some(AssetType::Fidc);
    }

    classify_by_cfi_hint(record.cfi_code.as_deref())
}

fn find_record_by_name<'a>(
    map: &'a HashMap<String, TickerRecord>,
    ticker: &str,
    name: Option<&str>,
) -> Option<&'a TickerRecord> {
    if !is_subscription_like_ticker(ticker) {
        return None;
    }
    let name = name.unwrap_or("");
    let normalized = normalize_name(name);
    if normalized.is_empty() {
        return None;
    }
    map.values().find(|record| {
        let record_name = record.corporate_name.as_deref().unwrap_or("");
        if record_name.is_empty() {
            return false;
        }
        let record_normalized = normalize_name(record_name);
        if record_normalized != normalized {
            return false;
        }
        let record_prefix: String = record.ticker.chars().take(4).collect();
        let target_prefix: String = ticker.chars().take(4).collect();
        record_prefix.eq_ignore_ascii_case(&target_prefix)
    })
}

fn find_record_by_prefix<'a>(
    map: &'a HashMap<String, TickerRecord>,
    ticker: &str,
) -> Option<&'a TickerRecord> {
    if !is_subscription_like_ticker(ticker) {
        return None;
    }
    let prefix: String = ticker.chars().take(4).collect();
    map.values()
        .find(|record| record.ticker.starts_with(&prefix))
}

fn is_subscription_like_ticker(ticker: &str) -> bool {
    let upper = ticker.trim().to_ascii_uppercase();
    if upper.starts_with("CDB")
        || upper.starts_with("CRI_")
        || upper.starts_with("CRA_")
        || upper.starts_with("TESOURO_")
    {
        return false;
    }
    if upper.len() < 4 {
        return false;
    }
    if upper.ends_with('9') {
        return true;
    }
    upper.ends_with("12") || upper.ends_with("13") || upper.ends_with("14") || upper.ends_with("15")
}

fn classify_by_cfi_hint(cfi_code: Option<&str>) -> Option<AssetType> {
    match cfi_code {
        Some(code) if code.starts_with("CF") => Some(AssetType::Fii),
        Some(code) if code.starts_with("CE") => Some(AssetType::Etf),
        _ => None,
    }
}

fn classify_foreign_etf(record: &TickerRecord) -> Option<AssetType> {
    let name = record.corporate_name.as_deref().unwrap_or("");
    let normalized = normalize_name(name);
    if contains_any(&normalized, &FIXED_INCOME_ETF_KEYWORDS) {
        return Some(AssetType::Etf);
    }
    Some(AssetType::Etf)
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|kw| haystack.contains(kw))
}

pub(crate) fn normalize_name(input: &str) -> String {
    let upper = input.to_uppercase();
    let mut out = String::with_capacity(upper.len());
    for ch in upper.nfkd() {
        if is_combining_mark(ch) {
            continue;
        }
        if ch.is_ascii_alphanumeric() || ch == ' ' {
            out.push(ch);
        } else if ch == '-' {
            out.push(' ');
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

const FIAGRO_KEYWORDS: [&str; 4] = ["FIAGRO", "AGRO", "AGRONEGOCIOS", "AGROINDUSTRIA"];
const FI_INFRA_KEYWORDS: [&str; 6] = [
    "INFRA",
    "INFRAESTRUTURA",
    "INV INFRA",
    "INV EM INFR",
    "CRI INFRA",
    "DEBENTURES INFRA",
];
const FII_KEYWORDS: [&str; 9] = [
    "IMOB",
    "IMOBILIARIO",
    "IMOBILIARIA",
    "FII",
    "RENDA IMOB",
    "LOGISTICA",
    "SHOPPING",
    "LAJES",
    "CORPORATIVO",
];
const FIDC_STRONG_KEYWORDS: [&str; 5] = ["FIDC", "CREDITO ESTRUTURADO", "CRI", "CRA", "RECEBIVEIS"];
const FIDC_WEAK_KEYWORDS: [&str; 1] = ["CREDITO"];
const FIP_KEYWORDS: [&str; 3] = ["FIP", "PARTICIPACOES", "PRIVATE EQUITY"];
const FOF_KEYWORDS: [&str; 5] = [
    "FUNDO DE FUNDOS",
    "FUNDOS DE FUNDOS",
    "FOF",
    "MULTIESTRATEGIA",
    "MULTI ESTRATEGIA",
];
const FIXED_INCOME_ETF_KEYWORDS: [&str; 9] = [
    "BOND",
    "FIXED INCOME",
    "IMAB",
    "IDKA",
    "IMB",
    "US BOND",
    "GLOBAL BOND",
    "TREASURY",
    "INCOME",
];

fn cache_is_stale(cache_dir: &Path) -> Result<bool> {
    let meta_path = cache_dir.join(META_FILENAME);
    if !meta_path.exists() {
        return Ok(true);
    }
    let meta_bytes = fs::read(&meta_path).context("Failed to read tickers metadata")?;
    let meta: TickersMeta =
        serde_json::from_slice(&meta_bytes).context("Failed to parse tickers metadata")?;
    Ok(Utc::now() - meta.fetched_at > Duration::hours(CACHE_MAX_AGE_HOURS))
}

fn build_download_url(response: &B3DownloadResponse) -> Result<String> {
    if let Some(token) = response.token.as_deref() {
        return Ok(format!(
            "https://arquivos.b3.com.br/api/download/?token={}",
            token
        ));
    }
    if let Some(redirect_url) = response.redirect_url.as_deref() {
        if redirect_url.starts_with("http") {
            return Ok(redirect_url.to_string());
        }
        if let Some(path) = redirect_url.strip_prefix("~") {
            return Ok(format!("{}{}", B3_API_BASE_URL, path));
        }
        return Ok(format!("{}{}", B3_API_BASE_URL, redirect_url));
    }
    Err(anyhow::anyhow!(
        "B3 download response missing redirectUrl and token"
    ))
}

fn download_b3_tickers(date: chrono::NaiveDate) -> Result<(Vec<u8>, String)> {
    std::thread::spawn(move || {
        let request_url = format!(
            "{}{}&recaptchaToken=",
            B3_REQUEST_BASE_URL,
            date.format("%Y-%m-%d")
        );
        let client = reqwest::blocking::Client::new();
        let response: B3DownloadResponse = client
            .get(&request_url)
            .send()
            .context("Failed to request B3 tickers download")?
            .error_for_status()
            .context("B3 tickers request returned an error status")?
            .json()
            .context("Failed to parse B3 tickers request response")?;

        let download_url = build_download_url(&response)?;

        let bytes = client
            .get(&download_url)
            .send()
            .context("Failed to download B3 tickers file")?
            .error_for_status()
            .context("B3 tickers download returned an error status")?
            .bytes()
            .context("Failed to read B3 tickers response")?
            .to_vec();

        Ok((bytes, download_url))
    })
    .join()
    .map_err(|_| anyhow::anyhow!("Tickers download thread panicked"))?
}

fn cache_guard() -> std::sync::MutexGuard<'static, Option<CachedMap>> {
    TICKERS_CACHE
        .get_or_init(|| Mutex::new(None))
        .lock()
        .expect("tickers cache mutex poisoned")
}

fn get_cached_map(cache_dir: &Path) -> Result<Arc<HashMap<String, TickerRecord>>> {
    let csv_path = cache_dir.join(CACHE_FILENAME);
    let metadata = fs::metadata(&csv_path).context("Failed to stat tickers cache file")?;
    let mtime = metadata
        .modified()
        .context("Failed to read tickers cache modified time")?;

    let mut guard = cache_guard();
    if let Some(cached) = guard.as_ref() {
        if cached.mtime == mtime {
            return Ok(cached.map.clone());
        }
    }

    let map = load_b3_tickers_map(Some(cache_dir))?;
    let arc_map = Arc::new(map);
    *guard = Some(CachedMap {
        map: arc_map.clone(),
        mtime,
    });
    Ok(arc_map)
}

fn detect_delimiter(content: &str) -> u8 {
    let line = content.lines().next().unwrap_or("");
    let semicolons = line.matches(';').count();
    let commas = line.matches(',').count();
    if semicolons > commas {
        b';'
    } else {
        b','
    }
}

fn get_field<'a>(
    record: &'a csv::StringRecord,
    headers: &'a csv::StringRecord,
    name: &str,
) -> &'a str {
    if let Some(idx) = headers.iter().position(|h| h == name) {
        record.get(idx).unwrap_or("")
    } else {
        ""
    }
}

fn normalize_csv_content(content: &str) -> Result<(String, u8)> {
    let mut header_index = None;
    for (idx, line) in content.lines().enumerate() {
        if line.contains("TckrSymb") && line.contains("SctyCtgyNm") {
            header_index = Some(idx);
            break;
        }
    }

    let header_index = header_index
        .ok_or_else(|| anyhow::anyhow!("B3 tickers CSV header not found in downloaded content"))?;

    let cleaned = content
        .lines()
        .skip(header_index)
        .collect::<Vec<_>>()
        .join("\n");
    let delimiter = detect_delimiter(&cleaned);
    Ok((cleaned, delimiter))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record_with(
        ticker: &str,
        security_category: &str,
        corporate_name: &str,
        cfi_code: Option<&str>,
    ) -> TickerRecord {
        TickerRecord {
            ticker: ticker.to_string(),
            security_category: security_category.to_string(),
            cfi_code: cfi_code.map(|v| v.to_string()),
            corporate_name: Some(corporate_name.to_string()),
        }
    }

    #[test]
    fn normalize_name_strips_punctuation() {
        assert_eq!(normalize_name("INV. EM INFR."), "INV EM INFR");
        assert_eq!(normalize_name("FII  BTG   PACTUAL"), "FII BTG PACTUAL");
    }

    #[test]
    fn classify_funds_by_name_keywords() {
        let fi_infra = record_with(
            "JURO11",
            "FUNDS",
            "SPARTA INFRA FIC FI INFRA RENDA FIXA CP",
            None,
        );
        assert_eq!(
            map_record_to_asset_type(&fi_infra),
            Some(AssetType::FiInfra)
        );

        let fiagro = record_with("CRAA11", "FUNDS", "ASSET BANK AGRONEGOCIOS FIAGRO", None);
        assert_eq!(map_record_to_asset_type(&fiagro), Some(AssetType::Fiagro));

        let fidc = record_with("ABCD11", "FUNDS", "XPTO FIDC RECEBIVEIS", None);
        assert_eq!(map_record_to_asset_type(&fidc), Some(AssetType::Fidc));

        let fip = record_with("FIPX11", "FUNDS", "ALPHA FIP PRIVATE EQUITY", None);
        assert_eq!(map_record_to_asset_type(&fip), Some(AssetType::Fip));

        let fii_credito_imob = record_with(
            "ALZC11",
            "FUNDS",
            "ALIANZA CREDITO IMOB FUND DE INVEST IMOB RESP LIM",
            None,
        );
        assert_eq!(
            map_record_to_asset_type(&fii_credito_imob),
            Some(AssetType::Fii)
        );

        let fii = record_with("BRCR11", "FUNDS", "BTG CORPORATE OFFICE FUND FII", None);
        assert_eq!(map_record_to_asset_type(&fii), Some(AssetType::Fii));
    }

    #[test]
    fn subscription_name_match_requires_exact_name_and_prefix() {
        let record = record_with(
            "BRCR11",
            "FUNDS",
            "FII BTG PACTUAL CORPORATE OFFICE FUND",
            None,
        );
        let mut map = HashMap::new();
        map.insert(record.ticker.clone(), record);

        let matched = find_record_by_name(
            &map,
            "BRCR13",
            Some("FII BTG PACTUAL CORPORATE OFFICE FUND"),
        );
        assert!(matched.is_some());

        let mismatched = find_record_by_name(&map, "BRCR13", Some("BTG CORPORATE OFFICE FUND"));
        assert!(mismatched.is_none());
    }

    #[test]
    fn subscription_prefix_fallback_matches_base() {
        let record = record_with("BRCR11", "FUNDS", "BTG CORPORATE OFFICE FUND FII", None);
        let mut map = HashMap::new();
        map.insert(record.ticker.clone(), record);

        let matched = find_record_by_prefix(&map, "BRCR13");
        assert!(matched.is_some());

        let not_subscription = find_record_by_prefix(&map, "BRCR11");
        assert!(not_subscription.is_none());
    }
}
