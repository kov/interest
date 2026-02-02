use crate::db;
use crate::formatters::OutputOptions;
use crate::output::{ColumnDef, Row, ValueKind};
use crate::output::{KeyValueRow, OutputBlock, OutputDocument, Value};
/// Formatters for inconsistency commands
use anyhow::Result;

//
// 1. RESOLVE CONFIRMATION
//

/// Format resolve confirmation
pub fn format_resolve(issue_id: i64, options: OutputOptions) -> Result<String> {
    let document = build_resolve_document(issue_id);
    crate::formatters::render_document(&document, options)
}

/// Format ignore confirmation
/// TODO: Hook up when inconsistencies ignore command is added
#[allow(dead_code)]
pub fn format_ignore(issue_id: i64, options: OutputOptions) -> Result<String> {
    let document = build_ignore_document(issue_id);
    crate::formatters::render_document(&document, options)
}

/// Format inconsistencies list
pub fn format_inconsistencies_list(
    issues: &[db::Inconsistency],
    options: OutputOptions,
) -> Result<String> {
    let document = build_inconsistencies_list_document(issues);
    crate::formatters::render_document(&document, options)
}

// Internal implementations below

fn build_resolve_document(issue_id: i64) -> OutputDocument {
    OutputDocument {
        title: None,
        blocks: vec![OutputBlock::KeyValue {
            title: Some("Inconsistency resolved".to_string()),
            rows: vec![KeyValueRow {
                label: "ID".to_string(),
                value: Value::Text(issue_id.to_string()),
            }],
        }],
        meta: Default::default(),
    }
}

//
// 2. IGNORE CONFIRMATION
//

fn build_ignore_document(issue_id: i64) -> OutputDocument {
    OutputDocument {
        title: None,
        blocks: vec![OutputBlock::KeyValue {
            title: Some("Inconsistency ignored".to_string()),
            rows: vec![KeyValueRow {
                label: "ID".to_string(),
                value: Value::Text(issue_id.to_string()),
            }],
        }],
        meta: Default::default(),
    }
}

fn build_inconsistencies_list_document(issues: &[db::Inconsistency]) -> OutputDocument {
    if issues.is_empty() {
        return OutputDocument {
            title: None,
            blocks: vec![OutputBlock::EmptyState {
                message: "No inconsistencies found".to_string(),
                hint: None,
            }],
            meta: Default::default(),
        };
    }

    let rows = issues
        .iter()
        .map(|issue| Row {
            cells: vec![
                Value::Text(issue.id.unwrap_or(0).to_string()),
                Value::Text(issue.status.as_str().to_string()),
                Value::Text(issue.issue_type.as_str().to_string()),
                issue.ticker.clone().map(Value::Text).unwrap_or(Value::Null),
                issue.trade_date.map(Value::Date).unwrap_or(Value::Null),
                issue.quantity.map(Value::Quantity).unwrap_or(Value::Null),
            ],
        })
        .collect::<Vec<_>>();

    OutputDocument {
        title: None,
        blocks: vec![OutputBlock::Table {
            title: None,
            columns: vec![
                ColumnDef::new("id", "ID", ValueKind::Text),
                ColumnDef::new("status", "Status", ValueKind::Text),
                ColumnDef::new("type", "Type", ValueKind::Text),
                ColumnDef::new("ticker", "Ticker", ValueKind::Text),
                ColumnDef::new("trade_date", "Trade Date", ValueKind::Date),
                ColumnDef::new("quantity", "Quantity", ValueKind::Quantity),
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
    fn test_resolve_json_structure() {
        let json_str = format_resolve(42, OutputOptions::from_flags(true, false)).unwrap();
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
        assert!(values.contains(&"42".to_string()));
    }

    #[test]
    fn test_ignore_json_structure() {
        let json_str = format_ignore(99, OutputOptions::from_flags(true, false)).unwrap();
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
        assert!(values.contains(&"99".to_string()));
    }

    #[test]
    fn test_resolve_table_format() {
        let output = format_resolve(42, OutputOptions::from_flags(false, false)).unwrap();
        assert!(output.contains("Inconsistency resolved"));
    }
}
