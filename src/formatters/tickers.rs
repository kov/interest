/// Formatters for ticker/B3 cache commands
use crate::formatters::OutputMode;
use crate::output::{ColumnDef, KeyValueRow, OutputBlock, OutputDocument, Row, Value, ValueKind};

//
// 1. REFRESH CONFIRMATION
//

/// Format refresh confirmation
pub fn format_refresh(path: &str, mode: OutputMode) -> String {
    let document = build_refresh_document(path);
    crate::formatters::render_document(&document, mode)
}

/// Format status output
pub fn format_status(
    cache_path: &std::path::Path,
    cache_exists: bool,
    fetched_at: Option<&str>,
    source_url: Option<&str>,
    unknown_count: usize,
    mode: OutputMode,
) -> String {
    let document = build_status_document(
        cache_path,
        cache_exists,
        fetched_at,
        source_url,
        unknown_count,
    );
    crate::formatters::render_document(&document, mode)
}

/// Format resolve confirmation
pub fn format_resolve(ticker: &str, asset_type: &str, mode: OutputMode) -> String {
    let document = build_resolve_document(ticker, asset_type);
    crate::formatters::render_document(&document, mode)
}

/// Format unknown assets list
pub fn format_unknown_list(assets: &[crate::db::Asset], mode: OutputMode) -> String {
    let document = build_unknown_list_document(assets);
    crate::formatters::render_document(&document, mode)
}

// Internal implementations below

fn build_refresh_document(path: &str) -> OutputDocument {
    OutputDocument {
        title: None,
        blocks: vec![
            OutputBlock::Header {
                level: 1,
                text: "Tickers refreshed successfully!".to_string(),
            },
            OutputBlock::KeyValue {
                title: None,
                rows: vec![KeyValueRow {
                    label: "Cached at".to_string(),
                    value: Value::Text(path.to_string()),
                }],
            },
        ],
        meta: Default::default(),
    }
}

//
// 2. STATUS OUTPUT
//

fn build_status_document(
    cache_path: &std::path::Path,
    cache_exists: bool,
    fetched_at: Option<&str>,
    source_url: Option<&str>,
    unknown_count: usize,
) -> OutputDocument {
    let mut rows = vec![
        KeyValueRow {
            label: "Path".to_string(),
            value: Value::Text(cache_path.display().to_string()),
        },
        KeyValueRow {
            label: "Exists".to_string(),
            value: Value::Text(cache_exists.to_string()),
        },
    ];
    if let Some(fetched) = fetched_at {
        rows.push(KeyValueRow {
            label: "Fetched at".to_string(),
            value: Value::Text(fetched.to_string()),
        });
    }
    if let Some(url) = source_url {
        rows.push(KeyValueRow {
            label: "Source".to_string(),
            value: Value::Text(url.to_string()),
        });
    }
    rows.push(KeyValueRow {
        label: "Unknown assets".to_string(),
        value: Value::Text(unknown_count.to_string()),
    });

    OutputDocument {
        title: None,
        blocks: vec![
            OutputBlock::Header {
                level: 1,
                text: "B3 Tickers Cache Status".to_string(),
            },
            OutputBlock::KeyValue { title: None, rows },
        ],
        meta: Default::default(),
    }
}

//
// 3. RESOLVE CONFIRMATION
//

fn build_resolve_document(ticker: &str, asset_type: &str) -> OutputDocument {
    OutputDocument {
        title: None,
        blocks: vec![OutputBlock::KeyValue {
            title: Some("Ticker resolved".to_string()),
            rows: vec![
                KeyValueRow {
                    label: "Ticker".to_string(),
                    value: Value::Text(ticker.to_string()),
                },
                KeyValueRow {
                    label: "Asset Type".to_string(),
                    value: Value::Text(asset_type.to_string()),
                },
            ],
        }],
        meta: Default::default(),
    }
}

fn build_unknown_list_document(assets: &[crate::db::Asset]) -> OutputDocument {
    if assets.is_empty() {
        return OutputDocument {
            title: None,
            blocks: vec![OutputBlock::EmptyState {
                message: "No unknown assets found".to_string(),
                hint: None,
            }],
            meta: Default::default(),
        };
    }

    let rows = assets
        .iter()
        .map(|asset| Row {
            cells: vec![
                Value::Text(asset.ticker.clone()),
                asset.name.clone().map(Value::Text).unwrap_or(Value::Null),
            ],
        })
        .collect::<Vec<_>>();

    OutputDocument {
        title: None,
        blocks: vec![OutputBlock::Table {
            title: None,
            columns: vec![
                ColumnDef::new("ticker", "Ticker", ValueKind::Text),
                ColumnDef::new("name", "Name", ValueKind::Text),
            ],
            rows,
            footer: None,
            options: Default::default(),
        }],
        meta: Default::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_refresh_json_structure() {
        let json_str = format_refresh("/path/to/cache", OutputMode::Json);
        let value: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        let blocks = value["blocks"].as_array().expect("blocks array missing");
        let mut values = Vec::new();
        for block in blocks {
            if block.get("type").and_then(|t| t.as_str()) == Some("key_value") {
                if let Some(rows) = block.get("rows").and_then(|r| r.as_array()) {
                    for row in rows {
                        if let Some(val) = row.get("value").and_then(|v| v.get("value")) {
                            if let Some(text) = val.as_str() {
                                values.push(text.to_string());
                            }
                        }
                    }
                }
            }
        }
        assert!(values.contains(&"/path/to/cache".to_string()));
    }

    #[test]
    fn test_resolve_json_structure() {
        let json_str = format_resolve("PETR4", "STOCK", OutputMode::Json);
        let value: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        let blocks = value["blocks"].as_array().expect("blocks array missing");
        let mut values = Vec::new();
        for block in blocks {
            if block.get("type").and_then(|t| t.as_str()) == Some("key_value") {
                if let Some(rows) = block.get("rows").and_then(|r| r.as_array()) {
                    for row in rows {
                        if let Some(val) = row.get("value").and_then(|v| v.get("value")) {
                            if let Some(text) = val.as_str() {
                                values.push(text.to_string());
                            }
                        }
                    }
                }
            }
        }
        assert!(values.contains(&"PETR4".to_string()));
        assert!(values.contains(&"STOCK".to_string()));
    }

    #[test]
    fn test_status_table_format() {
        let path = std::path::Path::new("/cache/tickers.csv");
        let output = format_status(path, true, Some("2024-01-01"), None, 5, OutputMode::Table);
        assert!(output.contains("B3 Tickers Cache Status"));
        assert!(output.contains("Unknown assets"));
    }
}
