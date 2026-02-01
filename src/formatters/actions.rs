/// Formatters for corporate actions (renames, splits, bonuses, exchanges)
use crate::db;
use crate::formatters::OutputOptions;
use crate::output::{ColumnDef, KeyValueRow, OutputBlock, OutputDocument, Row, Value, ValueKind};

//
// 1. RENAME FORMATTERS
//

/// Format rename add confirmation
pub fn format_rename_add(
    id: i64,
    from: &str,
    to: &str,
    effective_date: chrono::NaiveDate,
    notes: Option<&str>,
    options: OutputOptions,
) -> String {
    let document = build_rename_add_document(id, from, to, effective_date, notes);
    crate::formatters::render_document(&document, options)
}

/// Format renames list
pub fn format_renames_list(
    rows: &[(db::AssetRename, db::Asset, db::Asset)],
    options: OutputOptions,
) -> String {
    let document = build_renames_list_document(rows);
    crate::formatters::render_document(&document, options)
}

/// Format rename remove confirmation
pub fn format_rename_remove(id: i64, options: OutputOptions) -> String {
    let document = build_rename_remove_document(id);
    crate::formatters::render_document(&document, options)
}

// Internal implementations below

fn build_rename_add_document(
    id: i64,
    from: &str,
    to: &str,
    effective_date: chrono::NaiveDate,
    notes: Option<&str>,
) -> OutputDocument {
    let mut rows = vec![
        KeyValueRow {
            label: "Rename ID".to_string(),
            value: Value::Text(id.to_string()),
        },
        KeyValueRow {
            label: "From".to_string(),
            value: Value::Text(from.to_string()),
        },
        KeyValueRow {
            label: "To".to_string(),
            value: Value::Text(to.to_string()),
        },
        KeyValueRow {
            label: "Effective Date".to_string(),
            value: Value::Date(effective_date),
        },
    ];

    if let Some(n) = notes {
        rows.push(KeyValueRow {
            label: "Notes".to_string(),
            value: Value::Text(n.to_string()),
        });
    }

    OutputDocument {
        title: None,
        blocks: vec![
            OutputBlock::Header {
                level: 1,
                text: "Asset rename added successfully!".to_string(),
            },
            OutputBlock::KeyValue { title: None, rows },
        ],
        meta: Default::default(),
    }
}

fn build_renames_list_document(rows: &[(db::AssetRename, db::Asset, db::Asset)]) -> OutputDocument {
    if rows.is_empty() {
        return OutputDocument {
            title: None,
            blocks: vec![OutputBlock::EmptyState {
                message: "No renames found".to_string(),
                hint: None,
            }],
            meta: Default::default(),
        };
    }

    let table_rows = rows
        .iter()
        .map(|(rename, from, to)| Row {
            cells: vec![
                Value::Text(rename.id.unwrap_or(0).to_string()),
                Value::Text(from.ticker.clone()),
                Value::Text(to.ticker.clone()),
                Value::Date(rename.effective_date),
            ],
        })
        .collect::<Vec<_>>();

    OutputDocument {
        title: None,
        blocks: vec![OutputBlock::Table {
            title: None,
            columns: vec![
                ColumnDef::new("id", "ID", ValueKind::Text),
                ColumnDef::new("from", "From", ValueKind::Text),
                ColumnDef::new("to", "To", ValueKind::Text),
                ColumnDef::new("effective_date", "Date", ValueKind::Date),
            ],
            rows: table_rows,
            footer: None,
            options: Default::default(),
        }],
        meta: Default::default(),
    }
}

fn build_rename_remove_document(id: i64) -> OutputDocument {
    OutputDocument {
        title: None,
        blocks: vec![OutputBlock::KeyValue {
            title: Some("Rename removed".to_string()),
            rows: vec![KeyValueRow {
                label: "ID".to_string(),
                value: Value::Text(id.to_string()),
            }],
        }],
        meta: Default::default(),
    }
}

//
// 2. CORPORATE ACTION FORMATTERS (Splits, Bonuses)
//

/// Format corporate action add confirmation
pub fn format_corporate_action_add(
    id: i64,
    ticker: &str,
    action_type: &db::CorporateActionType,
    quantity_adjustment: rust_decimal::Decimal,
    ex_date: chrono::NaiveDate,
    notes: Option<&str>,
    options: OutputOptions,
) -> String {
    let document = build_corporate_action_add_document(
        id,
        ticker,
        action_type,
        quantity_adjustment,
        ex_date,
        notes,
    );
    crate::formatters::render_document(&document, options)
}

/// Format corporate actions list
pub fn format_corporate_actions_list(
    filtered: &[(db::CorporateAction, db::Asset)],
    options: OutputOptions,
) -> String {
    let document = build_corporate_actions_list_document(filtered);
    crate::formatters::render_document(&document, options)
}

/// Format corporate action remove confirmation
pub fn format_corporate_action_remove(id: i64, options: OutputOptions) -> String {
    let document = build_corporate_action_remove_document(id);
    crate::formatters::render_document(&document, options)
}

// Internal implementations below

fn build_corporate_action_add_document(
    id: i64,
    ticker: &str,
    action_type: &db::CorporateActionType,
    quantity_adjustment: rust_decimal::Decimal,
    ex_date: chrono::NaiveDate,
    notes: Option<&str>,
) -> OutputDocument {
    let mut rows = vec![
        KeyValueRow {
            label: "Action ID".to_string(),
            value: Value::Text(id.to_string()),
        },
        KeyValueRow {
            label: "Ticker".to_string(),
            value: Value::Text(ticker.to_string()),
        },
        KeyValueRow {
            label: "Type".to_string(),
            value: Value::Text(action_type.as_str().to_string()),
        },
        KeyValueRow {
            label: "Adjustment".to_string(),
            value: Value::Quantity(quantity_adjustment),
        },
        KeyValueRow {
            label: "Ex-Date".to_string(),
            value: Value::Date(ex_date),
        },
    ];

    if let Some(n) = notes {
        rows.push(KeyValueRow {
            label: "Notes".to_string(),
            value: Value::Text(n.to_string()),
        });
    }

    OutputDocument {
        title: None,
        blocks: vec![
            OutputBlock::Header {
                level: 1,
                text: "Corporate action added successfully!".to_string(),
            },
            OutputBlock::KeyValue { title: None, rows },
        ],
        meta: Default::default(),
    }
}

fn build_corporate_actions_list_document(
    filtered: &[(db::CorporateAction, db::Asset)],
) -> OutputDocument {
    if filtered.is_empty() {
        return OutputDocument {
            title: None,
            blocks: vec![OutputBlock::EmptyState {
                message: "No corporate actions found".to_string(),
                hint: None,
            }],
            meta: Default::default(),
        };
    }

    let rows = filtered
        .iter()
        .map(|(action, asset)| Row {
            cells: vec![
                Value::Text(action.id.unwrap_or(0).to_string()),
                Value::Text(asset.ticker.clone()),
                Value::Text(action.action_type.as_str().to_string()),
                Value::Quantity(action.quantity_adjustment),
                Value::Date(action.ex_date),
                Value::Text(action.source.clone()),
            ],
        })
        .collect::<Vec<_>>();

    OutputDocument {
        title: None,
        blocks: vec![OutputBlock::Table {
            title: None,
            columns: vec![
                ColumnDef::new("id", "ID", ValueKind::Text),
                ColumnDef::new("ticker", "Ticker", ValueKind::Text),
                ColumnDef::new("type", "Type", ValueKind::Text),
                ColumnDef::new("quantity_adjustment", "Adj Qty", ValueKind::Quantity),
                ColumnDef::new("ex_date", "Ex-Date", ValueKind::Date),
                ColumnDef::new("source", "Source", ValueKind::Text),
            ],
            rows,
            footer: None,
            options: Default::default(),
        }],
        meta: Default::default(),
    }
}

fn build_corporate_action_remove_document(id: i64) -> OutputDocument {
    OutputDocument {
        title: None,
        blocks: vec![OutputBlock::KeyValue {
            title: Some("Corporate action removed".to_string()),
            rows: vec![KeyValueRow {
                label: "ID".to_string(),
                value: Value::Text(id.to_string()),
            }],
        }],
        meta: Default::default(),
    }
}

//
// 3. EXCHANGE FORMATTERS (Spinoffs, Mergers)
//

/// Format exchange add confirmation
#[allow(clippy::too_many_arguments)]
pub fn format_exchange_add(
    id: i64,
    event_type: &db::AssetExchangeType,
    from: &str,
    to: &str,
    effective_date: chrono::NaiveDate,
    to_quantity: rust_decimal::Decimal,
    allocated_cost: rust_decimal::Decimal,
    cash_amount: rust_decimal::Decimal,
    notes: Option<&str>,
    options: OutputOptions,
) -> String {
    let document = build_exchange_add_document(
        id,
        event_type,
        from,
        to,
        effective_date,
        to_quantity,
        allocated_cost,
        cash_amount,
        notes,
    );
    crate::formatters::render_document(&document, options)
}

/// Format exchanges list
pub fn format_exchanges_list(
    rows: &[(db::AssetExchange, db::Asset, db::Asset)],
    options: OutputOptions,
) -> String {
    let document = build_exchanges_list_document(rows);
    crate::formatters::render_document(&document, options)
}

/// Format exchange remove confirmation
pub fn format_exchange_remove(id: i64, options: OutputOptions) -> String {
    let document = build_exchange_remove_document(id);
    crate::formatters::render_document(&document, options)
}

// Internal implementations below

#[allow(clippy::too_many_arguments)]
fn build_exchange_add_document(
    id: i64,
    event_type: &db::AssetExchangeType,
    from: &str,
    to: &str,
    effective_date: chrono::NaiveDate,
    to_quantity: rust_decimal::Decimal,
    allocated_cost: rust_decimal::Decimal,
    cash_amount: rust_decimal::Decimal,
    notes: Option<&str>,
) -> OutputDocument {
    let label = if *event_type == db::AssetExchangeType::Spinoff {
        "Spin-off"
    } else {
        "Merger"
    };

    let mut rows = vec![
        KeyValueRow {
            label: "Exchange ID".to_string(),
            value: Value::Text(id.to_string()),
        },
        KeyValueRow {
            label: "From".to_string(),
            value: Value::Text(from.to_string()),
        },
        KeyValueRow {
            label: "To".to_string(),
            value: Value::Text(to.to_string()),
        },
        KeyValueRow {
            label: "Effective Date".to_string(),
            value: Value::Date(effective_date),
        },
        KeyValueRow {
            label: "Quantity".to_string(),
            value: Value::Quantity(to_quantity),
        },
        KeyValueRow {
            label: "Allocated Cost".to_string(),
            value: Value::Currency(allocated_cost),
        },
    ];

    if cash_amount > rust_decimal::Decimal::ZERO {
        rows.push(KeyValueRow {
            label: "Cash Amount".to_string(),
            value: Value::Currency(cash_amount),
        });
    }

    if let Some(n) = notes {
        rows.push(KeyValueRow {
            label: "Notes".to_string(),
            value: Value::Text(n.to_string()),
        });
    }

    OutputDocument {
        title: None,
        blocks: vec![
            OutputBlock::Header {
                level: 1,
                text: format!("{} added successfully!", label),
            },
            OutputBlock::KeyValue { title: None, rows },
        ],
        meta: Default::default(),
    }
}

fn build_exchanges_list_document(
    filtered: &[(db::AssetExchange, db::Asset, db::Asset)],
) -> OutputDocument {
    if filtered.is_empty() {
        return OutputDocument {
            title: None,
            blocks: vec![OutputBlock::EmptyState {
                message: "No exchanges found".to_string(),
                hint: None,
            }],
            meta: Default::default(),
        };
    }

    let rows = filtered
        .iter()
        .map(|(exchange, from, to)| Row {
            cells: vec![
                Value::Text(exchange.id.unwrap_or(0).to_string()),
                Value::Text(from.ticker.clone()),
                Value::Text(to.ticker.clone()),
                Value::Date(exchange.effective_date),
                Value::Quantity(exchange.to_quantity),
                Value::Currency(exchange.allocated_cost),
                Value::Currency(exchange.cash_amount),
            ],
        })
        .collect::<Vec<_>>();

    OutputDocument {
        title: None,
        blocks: vec![OutputBlock::Table {
            title: None,
            columns: vec![
                ColumnDef::new("id", "ID", ValueKind::Text),
                ColumnDef::new("from", "From", ValueKind::Text),
                ColumnDef::new("to", "To", ValueKind::Text),
                ColumnDef::new("effective_date", "Date", ValueKind::Date),
                ColumnDef::new("quantity", "Qty", ValueKind::Quantity),
                ColumnDef::new("allocated_cost", "Alloc Cost", ValueKind::Currency),
                ColumnDef::new("cash", "Cash", ValueKind::Currency),
            ],
            rows,
            footer: None,
            options: Default::default(),
        }],
        meta: Default::default(),
    }
}

fn build_exchange_remove_document(id: i64) -> OutputDocument {
    OutputDocument {
        title: None,
        blocks: vec![OutputBlock::KeyValue {
            title: Some("Exchange removed".to_string()),
            rows: vec![KeyValueRow {
                label: "ID".to_string(),
                value: Value::Text(id.to_string()),
            }],
        }],
        meta: Default::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use rust_decimal::Decimal;
    use std::str::FromStr;

    #[test]
    fn test_rename_add_json_format() {
        let json_str = format_rename_add(
            42,
            "PETR3",
            "PETR4",
            NaiveDate::from_ymd_opt(2024, 1, 15).unwrap(),
            None,
            OutputOptions::from_flags(true, false),
        );
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
        assert!(values.contains(&"PETR3".to_string()));
        assert!(values.contains(&"PETR4".to_string()));
        assert!(values.contains(&"2024-01-15".to_string()));
    }

    #[test]
    fn test_corporate_action_json_decimals_as_strings() {
        let json_str = format_corporate_action_add(
            1,
            "PETR4",
            &db::CorporateActionType::Split,
            Decimal::from_str("100").unwrap(),
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            None,
            OutputOptions::from_flags(true, false),
        );
        let value: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        let blocks = value["blocks"].as_array().expect("blocks array missing");
        let mut found_quantity = false;

        for block in blocks {
            if block.get("type").and_then(|t| t.as_str()) == Some("key_value") {
                if let Some(rows) = block.get("rows").and_then(|r| r.as_array()) {
                    for row in rows {
                        if let Some(val) = row.get("value") {
                            if val.get("kind").and_then(|k| k.as_str()) == Some("quantity") {
                                if let Some(raw) = val.get("value") {
                                    assert!(raw.is_string());
                                    assert_eq!(raw, "100");
                                    found_quantity = true;
                                }
                            }
                        }
                    }
                }
            }
        }

        assert!(found_quantity, "expected quantity value");
    }

    #[test]
    fn test_exchange_json_decimals_as_strings() {
        let json_str = format_exchange_add(
            1,
            &db::AssetExchangeType::Spinoff,
            "PETR4",
            "RECV3",
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            Decimal::from_str("50").unwrap(),
            Decimal::from_str("1234.56").unwrap(),
            Decimal::from_str("10.00").unwrap(),
            None,
            OutputOptions::from_flags(true, false),
        );
        let value: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        let blocks = value["blocks"].as_array().expect("blocks array missing");
        let mut has_quantity = false;
        let mut has_currency = false;

        for block in blocks {
            if block.get("type").and_then(|t| t.as_str()) == Some("key_value") {
                if let Some(rows) = block.get("rows").and_then(|r| r.as_array()) {
                    for row in rows {
                        if let Some(val) = row.get("value") {
                            let kind = val.get("kind").and_then(|k| k.as_str());
                            if let Some(raw) = val.get("value") {
                                if kind == Some("quantity") {
                                    assert_eq!(raw, "50");
                                    has_quantity = true;
                                }
                                if kind == Some("currency") {
                                    assert!(raw.is_string());
                                    has_currency = true;
                                }
                            }
                        }
                    }
                }
            }
        }

        assert!(has_quantity, "expected quantity value");
        assert!(has_currency, "expected currency values");
    }
}
