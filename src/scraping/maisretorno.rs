use anyhow::{Context, Result};
use regex::Regex;
use reqwest::Client;
use serde_json::Value;
use std::time::Duration;
use tokio::sync::mpsc;

use crate::db::{AssetRegistryEntry, AssetType};
use crate::tesouro;

const BASE_URL: &str = "https://maisretorno.com";
const SOURCE_NAME: &str = "MAIS_RETORNO";

#[derive(Debug, Clone, Copy)]
pub struct MaisRetornoListSource {
    pub asset_type: AssetType,
    pub url: &'static str,
}

#[derive(Debug, Clone)]
pub struct SourceFetchStats {
    pub asset_type: AssetType,
    pub label: &'static str,
    pub pages: usize,
    pub entries: usize,
}

#[derive(Debug, Clone)]
pub struct SyncStats {
    pub total_entries: usize,
    pub registry_written: usize,
    pub assets_updated: usize,
    pub updated_type: usize,
    pub updated_name: usize,
    pub updated_cnpj: usize,
    pub dry_run: bool,
}

pub const LIST_SOURCES: &[MaisRetornoListSource] = &[
    MaisRetornoListSource {
        asset_type: AssetType::Stock,
        url: "https://maisretorno.com/lista-acoes",
    },
    MaisRetornoListSource {
        asset_type: AssetType::Bdr,
        url: "https://maisretorno.com/lista-bdr",
    },
    MaisRetornoListSource {
        asset_type: AssetType::Etf,
        url: "https://maisretorno.com/lista-etf",
    },
    MaisRetornoListSource {
        asset_type: AssetType::Fii,
        url: "https://maisretorno.com/lista-fii",
    },
    MaisRetornoListSource {
        asset_type: AssetType::Fiagro,
        url: "https://maisretorno.com/lista-fiagro",
    },
    MaisRetornoListSource {
        asset_type: AssetType::FiInfra,
        url: "https://maisretorno.com/lista-fi-infra",
    },
    MaisRetornoListSource {
        asset_type: AssetType::Fip,
        url: "https://maisretorno.com/lista-fip",
    },
    MaisRetornoListSource {
        asset_type: AssetType::Bond,
        url: "https://maisretorno.com/lista-debentures",
    },
    MaisRetornoListSource {
        asset_type: AssetType::GovBond,
        url: "https://maisretorno.com/lista-titulos-publicos",
    },
    MaisRetornoListSource {
        asset_type: AssetType::GovBond,
        url: "https://maisretorno.com/lista-tesouro-direto",
    },
];

pub fn select_sources(asset_type: Option<AssetType>) -> Vec<&'static MaisRetornoListSource> {
    match asset_type {
        None => LIST_SOURCES.iter().collect(),
        Some(target) => LIST_SOURCES
            .iter()
            .filter(|s| s.asset_type == target)
            .collect(),
    }
}

pub async fn fetch_registry_entries(
    client: &Client,
    sources: &[&MaisRetornoListSource],
    progress_tx: Option<mpsc::UnboundedSender<crate::ui::progress::ProgressEvent>>,
) -> Result<(Vec<AssetRegistryEntry>, Vec<SourceFetchStats>)> {
    let mut entries = Vec::new();
    let mut per_source = Vec::new();
    for source in sources {
        let source_label = source_label(source.url);
        send_progress(
            &progress_tx,
            crate::ui::progress::ProgressEvent::Spinner {
                message: format!(
                    "Fetching {} {} page 1...",
                    source.asset_type.as_str(),
                    source_label
                ),
            },
        );
        let page = 1;
        let url = build_page_url(source.url, page);
        let html = fetch_html(client, &url).await?;
        let (page_entries, pagination) = parse_list_page(&html, source.asset_type, &url)?;
        let total_pages = pagination.pages_quantity.unwrap_or(1);
        let mut entries_count = page_entries.len();

        entries.extend(page_entries);

        if total_pages > 1 {
            let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(6));
            let mut join_set = tokio::task::JoinSet::new();

            for page in 2..=total_pages {
                let permit = semaphore.clone().acquire_owned().await?;
                let client = client.clone();
                let tx = progress_tx.clone();
                let source_url = source.url;
                let asset_type = source.asset_type;
                join_set.spawn(async move {
                    let _permit = permit;
                    send_progress(
                        &tx,
                        crate::ui::progress::ProgressEvent::Spinner {
                            message: format!(
                                "Fetching {} {} page {}/{}",
                                asset_type.as_str(),
                                source_label,
                                page,
                                total_pages
                            ),
                        },
                    );
                    let url = build_page_url(source_url, page);
                    let html = fetch_html(&client, &url).await?;
                    let (page_entries, _pagination) = parse_list_page(&html, asset_type, &url)?;
                    Ok::<_, anyhow::Error>(page_entries)
                });
            }

            while let Some(result) = join_set.join_next().await {
                match result {
                    Ok(Ok(page_entries)) => {
                        entries_count += page_entries.len();
                        entries.extend(page_entries);
                    }
                    Ok(Err(err)) => {
                        send_progress(
                            &progress_tx,
                            crate::ui::progress::ProgressEvent::Error {
                                message: format!(
                                    "{} {} page error: {}",
                                    source.asset_type.as_str(),
                                    source_label,
                                    err
                                ),
                            },
                        );
                    }
                    Err(err) => {
                        send_progress(
                            &progress_tx,
                            crate::ui::progress::ProgressEvent::Error {
                                message: format!(
                                    "{} {} page task failed: {}",
                                    source.asset_type.as_str(),
                                    source_label,
                                    err
                                ),
                            },
                        );
                    }
                }
            }
        }

        let source_stats = SourceFetchStats {
            asset_type: source.asset_type,
            label: source_label,
            pages: total_pages,
            entries: entries_count,
        };
        per_source.push(source_stats.clone());
        send_progress(
            &progress_tx,
            crate::ui::progress::ProgressEvent::Success {
                message: format!(
                    "Fetched {} {} data - {} page{}, {} entries.",
                    source_stats.asset_type.as_str(),
                    source_stats.label,
                    source_stats.pages,
                    if source_stats.pages == 1 { "" } else { "s" },
                    source_stats.entries
                ),
            },
        );

        tokio::time::sleep(Duration::from_millis(150)).await;
    }

    Ok((entries, per_source))
}

async fn fetch_html(client: &Client, url: &str) -> Result<String> {
    const MAX_RETRIES: usize = 3;
    const BASE_DELAY_MS: u64 = 200;

    let mut attempt = 0;
    loop {
        attempt += 1;
        let resp = client
            .get(url)
            .header("User-Agent", "interest/0.1 (asset sync)")
            .send()
            .await;

        match resp {
            Ok(resp) => {
                let status = resp.status();
                let body = resp
                    .text()
                    .await
                    .with_context(|| format!("failed reading response for {}", url))?;
                if status.is_success() {
                    return Ok(body);
                }
                if attempt >= MAX_RETRIES {
                    anyhow::bail!("Mais Retorno request failed: {} ({})", url, status);
                }
            }
            Err(err) => {
                if attempt >= MAX_RETRIES {
                    return Err(err).with_context(|| format!("request failed for {}", url));
                }
            }
        }

        let delay = BASE_DELAY_MS * attempt as u64;
        tokio::time::sleep(Duration::from_millis(delay)).await;
    }
}

fn build_page_url(base: &str, page: usize) -> String {
    if page <= 1 {
        base.to_string()
    } else {
        format!("{}/page/{}", base, page)
    }
}

#[derive(Debug)]
struct PaginationInfo {
    pages_quantity: Option<usize>,
}

fn parse_list_page(
    html: &str,
    asset_type: AssetType,
    source_url: &str,
) -> Result<(Vec<AssetRegistryEntry>, PaginationInfo)> {
    let data = extract_next_data(html)?;
    let page_props = data
        .get("props")
        .and_then(|v| v.get("pageProps"))
        .context("missing pageProps")?;

    let list = page_props
        .get("list")
        .and_then(|v| v.as_array())
        .context("missing list array")?;

    let pages_quantity = page_props
        .get("pagination")
        .and_then(|v| v.get("pages_quantity"))
        .and_then(|v| v.as_u64())
        .map(|v| v as usize);

    let mut entries = Vec::new();
    for item in list {
        if let Some(entry) = parse_list_item(item, asset_type, source_url)? {
            entries.push(entry);
        }
    }

    Ok((entries, PaginationInfo { pages_quantity }))
}

fn extract_next_data(html: &str) -> Result<Value> {
    let re = Regex::new(r#"(?s)__NEXT_DATA__\" type=\"application/json\">(.*?)</script>"#)
        .context("invalid regex")?;
    let caps = re
        .captures(html)
        .context("unable to locate __NEXT_DATA__ payload")?;
    let raw = caps
        .get(1)
        .context("missing __NEXT_DATA__ capture")?
        .as_str();
    let data: Value = serde_json::from_str(raw).context("failed to parse __NEXT_DATA__")?;
    Ok(data)
}

fn parse_list_item(
    item: &Value,
    asset_type: AssetType,
    list_url: &str,
) -> Result<Option<AssetRegistryEntry>> {
    let main = match item.get("mainInfo") {
        Some(v) => v,
        None => return Ok(None),
    };
    let raw_name = main.get("name").and_then(|v| v.as_str()).unwrap_or("");
    if raw_name.trim().is_empty() {
        return Ok(None);
    }

    let additional = main
        .get("additionalText")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let link = main.get("link").and_then(|v| v.as_str());
    let detail_url = link.map(|l| format!("{}/{}", BASE_URL, l.trim_start_matches('/')));

    let mut actuation_segment = None;
    let mut actuation_sector = None;
    let mut issue = None;
    let mut situation = None;
    let mut indexer = None;
    let mut security_type = None;
    let mut codigo = None;
    let mut data_emissao = None;
    let mut data_vencimento = None;
    let mut cnpj = None;

    if let Some(data_items) = item.get("data").and_then(|v| v.as_array()) {
        for data_item in data_items {
            let slug = data_item
                .get("slug")
                .and_then(|v| v.as_str())
                .map(|s| s.trim())
                .unwrap_or("");
            let value = data_item
                .get("value")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());

            match slug {
                "cnpj" => {
                    cnpj = value.and_then(|v| normalize_cnpj(&v));
                }
                "actuation_segment" => {
                    actuation_segment = value;
                }
                "actuation_sector" => {
                    actuation_sector = value;
                }
                "issue" => {
                    issue = value;
                }
                "situation" => {
                    situation = value;
                }
                "indexer" => {
                    indexer = value;
                }
                "type" => {
                    security_type = value;
                }
                "codigo" => {
                    codigo = value;
                }
                "data_emissao" => {
                    data_emissao = value;
                }
                "data_vencimento" => {
                    data_vencimento = value;
                }
                _ => {}
            }
        }
    }

    let mut ticker = raw_name.trim().to_uppercase();
    let mut display_name = additional.clone();

    if list_url.contains("lista-debentures") {
        let (bond_ticker, bond_name) = parse_bond_name(raw_name);
        ticker = bond_ticker;
        display_name = bond_name.or(additional);
    }

    if list_url.contains("lista-tesouro-direto") {
        if let Some(synthetic) = tesouro::ticker_from_name(raw_name) {
            ticker = synthetic;
        }
        if display_name.is_none() {
            display_name = Some(raw_name.trim().to_string());
        }
    }

    let raw_json = serde_json::to_string(item).ok();

    Ok(Some(AssetRegistryEntry {
        source: SOURCE_NAME.to_string(),
        ticker,
        asset_type,
        name: display_name,
        cnpj,
        actuation_segment,
        actuation_sector,
        issue,
        situation,
        indexer,
        security_type,
        codigo,
        data_emissao,
        data_vencimento,
        source_url: detail_url.or_else(|| Some(list_url.to_string())),
        raw_json,
        updated_at: None,
    }))
}

fn normalize_cnpj(value: &str) -> Option<String> {
    let digits: String = value.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        None
    } else {
        Some(digits)
    }
}

fn parse_bond_name(raw: &str) -> (String, Option<String>) {
    let parts: Vec<&str> = raw.split(" - ").map(|s| s.trim()).collect();
    if parts.len() >= 2 {
        let ticker = parts[0].to_uppercase();
        let name = parts[1..].join(" - ");
        let name = name.trim();
        let name = if name.is_empty() {
            None
        } else {
            Some(name.to_string())
        };
        (ticker, name)
    } else {
        (raw.trim().to_uppercase(), None)
    }
}

fn send_progress(
    tx: &Option<mpsc::UnboundedSender<crate::ui::progress::ProgressEvent>>,
    event: crate::ui::progress::ProgressEvent,
) {
    if let Some(tx) = tx {
        let _ = tx.send(event);
    }
}

fn source_label(url: &str) -> &'static str {
    if let Some(label) = url.rsplit('/').next() {
        match label {
            "lista-titulos-publicos" => "(titulos-publicos)",
            "lista-tesouro-direto" => "(tesouro-direto)",
            "lista-debentures" => "(debentures)",
            "lista-fi-infra" => "(fi-infra)",
            "lista-fiagro" => "(fiagro)",
            "lista-fii" => "(fii)",
            "lista-etf" => "(etf)",
            "lista-bdr" => "(bdr)",
            "lista-acoes" => "(acoes)",
            "lista-fip" => "(fip)",
            _ => "(lista)",
        }
    } else {
        "(lista)"
    }
}

pub async fn sync_registry(
    conn: &rusqlite::Connection,
    sources: &[&MaisRetornoListSource],
    dry_run: bool,
    progress_tx: Option<mpsc::UnboundedSender<crate::ui::progress::ProgressEvent>>,
) -> Result<SyncStats> {
    send_progress(
        &progress_tx,
        crate::ui::progress::ProgressEvent::Spinner {
            message: "Refreshing asset data from MaisRetorno...".to_string(),
        },
    );
    let client = Client::new();
    let (entries, _per_source) =
        fetch_registry_entries(&client, sources, progress_tx.clone()).await?;

    let mut registry_written = 0;
    let mut assets_updated = 0;
    let mut updated_type = 0;
    let mut updated_name = 0;
    let mut updated_cnpj = 0;

    if !dry_run {
        for entry in &entries {
            crate::db::upsert_asset_registry(conn, entry)?;
            registry_written += 1;
        }
        crate::db::set_metadata(
            conn,
            "registry_maisretorno_refreshed_at",
            &chrono::Utc::now().to_rfc3339(),
        )?;
    }

    if !dry_run {
        for entry in &entries {
            let asset = crate::db::get_asset_by_ticker(conn, &entry.ticker)?;
            let Some(asset) = asset else {
                continue;
            };
            let mut touched = false;
            if asset.asset_type == AssetType::Unknown && entry.asset_type != AssetType::Unknown {
                crate::db::update_asset_type(conn, &asset.ticker, &entry.asset_type)?;
                updated_type += 1;
                touched = true;
            }
            if asset.name.is_none() {
                if let Some(name) = entry.name.as_deref() {
                    crate::db::update_asset_name(conn, &asset.ticker, name)?;
                    updated_name += 1;
                    touched = true;
                }
            }
            if asset.cnpj.is_none() {
                if let Some(cnpj) = entry.cnpj.as_deref() {
                    crate::db::update_asset_cnpj(conn, &asset.ticker, cnpj)?;
                    updated_cnpj += 1;
                    touched = true;
                }
            }
            if touched {
                assets_updated += 1;
            }
        }
    }

    Ok(SyncStats {
        total_entries: entries.len(),
        registry_written,
        assets_updated,
        updated_type,
        updated_name,
        updated_cnpj,
        dry_run,
    })
}
