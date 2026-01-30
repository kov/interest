/// Formatters for asset management commands
use crate::db;
use crate::formatters::OutputMode;
use crate::output::{ColumnDef, KeyValueRow, OutputBlock, OutputDocument, Row, Value, ValueKind};

//
// 1. ASSET LIST
//

/// Format assets list
pub fn format_assets_list(assets: &[db::Asset], mode: OutputMode) -> String {
    let document = build_assets_list_document(assets);
    crate::formatters::render_document(&document, mode)
}

/// Format asset show details
pub fn format_asset_show(asset: &db::Asset, tx_count: i64, mode: OutputMode) -> String {
    let document = build_asset_show_document(asset, tx_count);
    crate::formatters::render_document(&document, mode)
}

/// Format asset add confirmation
pub fn format_asset_add(asset_id: i64, asset: &db::Asset, mode: OutputMode) -> String {
    let document = build_asset_add_document(asset_id, asset);
    crate::formatters::render_document(&document, mode)
}

/// Format asset set-type confirmation
pub fn format_asset_set_type(ticker: &str, asset_type: &db::AssetType, mode: OutputMode) -> String {
    let document = build_asset_set_type_document(ticker, asset_type);
    crate::formatters::render_document(&document, mode)
}

/// Format asset set-name confirmation
pub fn format_asset_set_name(ticker: &str, name: &str, mode: OutputMode) -> String {
    let document = build_asset_set_name_document(ticker, name);
    crate::formatters::render_document(&document, mode)
}

/// Format asset rename confirmation
pub fn format_asset_rename(old_ticker: &str, new_ticker: &str, mode: OutputMode) -> String {
    let document = build_asset_rename_document(old_ticker, new_ticker);
    crate::formatters::render_document(&document, mode)
}

/// Format asset remove confirmation
pub fn format_asset_remove(ticker: &str, mode: OutputMode) -> String {
    let document = build_asset_remove_document(ticker);
    crate::formatters::render_document(&document, mode)
}

/// Format sync-maisretorno results
pub fn format_sync_maisretorno(
    sources: &[&crate::scraping::maisretorno::MaisRetornoListSource],
    stats: &crate::scraping::maisretorno::SyncStats,
    mode: OutputMode,
) -> String {
    let document = build_sync_maisretorno_document(sources, stats);
    crate::formatters::render_document(&document, mode)
}

// Internal implementations below

fn build_assets_list_document(assets: &[db::Asset]) -> OutputDocument {
    if assets.is_empty() {
        return OutputDocument {
            title: None,
            blocks: vec![OutputBlock::EmptyState {
                message: "No assets found".to_string(),
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
                Value::Text(asset.asset_type.as_str().to_string()),
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
                ColumnDef::new("asset_type", "Type", ValueKind::Text),
                ColumnDef::new("name", "Name", ValueKind::Text),
            ],
            rows,
            footer: None,
            options: Default::default(),
        }],
        meta: Default::default(),
    }
}

//
// 2. ASSET SHOW
//

fn build_asset_show_document(asset: &db::Asset, tx_count: i64) -> OutputDocument {
    OutputDocument {
        title: None,
        blocks: vec![
            OutputBlock::Header {
                level: 1,
                text: format!("Asset: {}", asset.ticker),
            },
            OutputBlock::KeyValue {
                title: None,
                rows: vec![
                    KeyValueRow {
                        label: "Type".to_string(),
                        value: Value::Text(asset.asset_type.as_str().to_string()),
                    },
                    KeyValueRow {
                        label: "Name".to_string(),
                        value: asset.name.clone().map(Value::Text).unwrap_or(Value::Null),
                    },
                    KeyValueRow {
                        label: "CNPJ".to_string(),
                        value: asset.cnpj.clone().map(Value::Text).unwrap_or(Value::Null),
                    },
                    KeyValueRow {
                        label: "Created".to_string(),
                        value: Value::DateTime(asset.created_at),
                    },
                    KeyValueRow {
                        label: "Updated".to_string(),
                        value: Value::DateTime(asset.updated_at),
                    },
                    KeyValueRow {
                        label: "Transactions".to_string(),
                        value: Value::Text(tx_count.to_string()),
                    },
                ],
            },
        ],
        meta: Default::default(),
    }
}

//
// 3. ASSET ADD
//

fn build_asset_add_document(asset_id: i64, asset: &db::Asset) -> OutputDocument {
    let mut rows = vec![
        KeyValueRow {
            label: "ID".to_string(),
            value: Value::Text(asset_id.to_string()),
        },
        KeyValueRow {
            label: "Ticker".to_string(),
            value: Value::Text(asset.ticker.clone()),
        },
        KeyValueRow {
            label: "Type".to_string(),
            value: Value::Text(asset.asset_type.as_str().to_string()),
        },
    ];

    if let Some(name) = &asset.name {
        rows.push(KeyValueRow {
            label: "Name".to_string(),
            value: Value::Text(name.clone()),
        });
    }

    OutputDocument {
        title: None,
        blocks: vec![
            OutputBlock::Header {
                level: 1,
                text: "Asset added successfully!".to_string(),
            },
            OutputBlock::KeyValue { title: None, rows },
        ],
        meta: Default::default(),
    }
}

//
// 4. ASSET SET TYPE
//

fn build_asset_set_type_document(ticker: &str, asset_type: &db::AssetType) -> OutputDocument {
    OutputDocument {
        title: None,
        blocks: vec![OutputBlock::KeyValue {
            title: Some("Asset type updated".to_string()),
            rows: vec![
                KeyValueRow {
                    label: "Ticker".to_string(),
                    value: Value::Text(ticker.to_uppercase()),
                },
                KeyValueRow {
                    label: "Type".to_string(),
                    value: Value::Text(asset_type.as_str().to_string()),
                },
            ],
        }],
        meta: Default::default(),
    }
}

//
// 5. ASSET SET NAME
//

fn build_asset_set_name_document(ticker: &str, name: &str) -> OutputDocument {
    OutputDocument {
        title: None,
        blocks: vec![OutputBlock::KeyValue {
            title: Some("Asset name updated".to_string()),
            rows: vec![
                KeyValueRow {
                    label: "Ticker".to_string(),
                    value: Value::Text(ticker.to_uppercase()),
                },
                KeyValueRow {
                    label: "Name".to_string(),
                    value: Value::Text(name.to_string()),
                },
            ],
        }],
        meta: Default::default(),
    }
}

//
// 6. ASSET RENAME
//

fn build_asset_rename_document(old_ticker: &str, new_ticker: &str) -> OutputDocument {
    OutputDocument {
        title: None,
        blocks: vec![OutputBlock::KeyValue {
            title: Some("Asset renamed".to_string()),
            rows: vec![
                KeyValueRow {
                    label: "Old Ticker".to_string(),
                    value: Value::Text(old_ticker.to_uppercase()),
                },
                KeyValueRow {
                    label: "New Ticker".to_string(),
                    value: Value::Text(new_ticker.to_uppercase()),
                },
            ],
        }],
        meta: Default::default(),
    }
}

//
// 7. ASSET REMOVE
//

fn build_asset_remove_document(ticker: &str) -> OutputDocument {
    OutputDocument {
        title: None,
        blocks: vec![OutputBlock::KeyValue {
            title: Some("Asset removed".to_string()),
            rows: vec![KeyValueRow {
                label: "Ticker".to_string(),
                value: Value::Text(ticker.to_uppercase()),
            }],
        }],
        meta: Default::default(),
    }
}

//
// 8. SYNC MAIS RETORNO
//

fn build_sync_maisretorno_document(
    sources: &[&crate::scraping::maisretorno::MaisRetornoListSource],
    stats: &crate::scraping::maisretorno::SyncStats,
) -> OutputDocument {
    let header = if stats.dry_run {
        "Mais Retorno sync (dry run)"
    } else {
        "Mais Retorno sync complete"
    };

    let source_rows = sources
        .iter()
        .map(|source| Row {
            cells: vec![
                Value::Text(source.asset_type.as_str().to_string()),
                Value::Text(source.url.to_string()),
            ],
        })
        .collect::<Vec<_>>();

    let mut stats_rows = vec![KeyValueRow {
        label: "Entries fetched".to_string(),
        value: Value::Text(stats.total_entries.to_string()),
    }];

    if stats.dry_run {
        stats_rows.push(KeyValueRow {
            label: "Registry writes".to_string(),
            value: Value::Text("skipped (dry run)".to_string()),
        });
        stats_rows.push(KeyValueRow {
            label: "Asset updates".to_string(),
            value: Value::Text("skipped (dry run)".to_string()),
        });
    } else {
        stats_rows.push(KeyValueRow {
            label: "Registry entries written".to_string(),
            value: Value::Text(stats.registry_written.to_string()),
        });
        stats_rows.push(KeyValueRow {
            label: "Assets updated".to_string(),
            value: Value::Text(stats.assets_updated.to_string()),
        });
        stats_rows.push(KeyValueRow {
            label: "Type updates".to_string(),
            value: Value::Text(stats.updated_type.to_string()),
        });
        stats_rows.push(KeyValueRow {
            label: "Name updates".to_string(),
            value: Value::Text(stats.updated_name.to_string()),
        });
        stats_rows.push(KeyValueRow {
            label: "CNPJ updates".to_string(),
            value: Value::Text(stats.updated_cnpj.to_string()),
        });
    }

    OutputDocument {
        title: None,
        blocks: vec![
            OutputBlock::Header {
                level: 1,
                text: header.to_string(),
            },
            OutputBlock::Table {
                title: Some("Sources".to_string()),
                columns: vec![
                    ColumnDef::new("asset_type", "Asset Type", ValueKind::Text),
                    ColumnDef::new("url", "URL", ValueKind::Text),
                ],
                rows: source_rows,
                footer: None,
                options: Default::default(),
            },
            OutputBlock::KeyValue {
                title: Some("Stats".to_string()),
                rows: stats_rows,
            },
        ],
        meta: Default::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    #[test]
    fn test_asset_show_json_datetime_as_iso_string() {
        let asset = db::Asset {
            id: Some(1),
            ticker: "PETR4".to_string(),
            asset_type: db::AssetType::Stock,
            name: Some("Petrobras".to_string()),
            cnpj: None,
            created_at: Utc.with_ymd_and_hms(2024, 1, 1, 10, 0, 0).unwrap(),
            updated_at: Utc.with_ymd_and_hms(2024, 6, 15, 14, 30, 0).unwrap(),
        };

        let json_str = format_asset_show(&asset, 10, OutputMode::Json);
        let value: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        let blocks = value["blocks"].as_array().expect("blocks array missing");
        let mut dates = Vec::new();

        for block in blocks {
            if block.get("type").and_then(|t| t.as_str()) == Some("key_value") {
                if let Some(rows) = block.get("rows").and_then(|r| r.as_array()) {
                    for row in rows {
                        if let Some(val) = row.get("value") {
                            if val.get("kind").and_then(|k| k.as_str()) == Some("datetime") {
                                if let Some(text) = val.get("value") {
                                    dates.push(text.as_str().unwrap().to_string());
                                }
                            }
                        }
                    }
                }
            }
        }

        assert!(dates.contains(&"2024-01-01T10:00:00Z".to_string()));
        assert!(dates.contains(&"2024-06-15T14:30:00Z".to_string()));
    }

    #[test]
    fn test_asset_add_json_structure() {
        let asset = db::Asset {
            id: Some(42),
            ticker: "VALE3".to_string(),
            asset_type: db::AssetType::Stock,
            name: Some("Vale".to_string()),
            cnpj: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let json_str = format_asset_add(42, &asset, OutputMode::Json);
        let value: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        let blocks = value["blocks"].as_array().expect("blocks array missing");
        let mut values = Vec::new();

        for block in blocks {
            if block.get("type").and_then(|t| t.as_str()) == Some("key_value") {
                if let Some(rows) = block.get("rows").and_then(|r| r.as_array()) {
                    for row in rows {
                        if let Some(val) = row.get("value") {
                            if let Some(text) = val.get("value").and_then(|v| v.as_str()) {
                                values.push(text.to_string());
                            }
                        }
                    }
                }
            }
        }

        assert!(values.contains(&"42".to_string()));
        assert!(values.contains(&"VALE3".to_string()));
        assert!(values.contains(&"STOCK".to_string()));
        assert!(values.contains(&"Vale".to_string()));
    }

    #[test]
    fn test_assets_list_json_valid() {
        let assets = vec![db::Asset {
            id: Some(1),
            ticker: "PETR4".to_string(),
            asset_type: db::AssetType::Stock,
            name: Some("Petrobras".to_string()),
            cnpj: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }];

        let json_str = format_assets_list(&assets, OutputMode::Json);
        let value: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        let blocks = value["blocks"].as_array().expect("blocks array missing");
        let mut row_count = 0;

        for block in blocks {
            if block.get("type").and_then(|t| t.as_str()) == Some("table") {
                if let Some(rows) = block.get("rows").and_then(|r| r.as_array()) {
                    row_count += rows.len();
                }
            }
        }

        assert_eq!(row_count, 1);
    }
}
