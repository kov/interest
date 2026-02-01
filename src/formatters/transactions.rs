/// Formatters for transaction commands
use rust_decimal::Decimal;

use crate::formatters::OutputMode;
use crate::output::{ColumnDef, KeyValueRow, OutputBlock, OutputDocument, Row, Value, ValueKind};

//
// 1. TRANSACTION ADD CONFIRMATION
//

#[allow(clippy::too_many_arguments)]
pub fn format_transaction_add_table(
    tx_id: i64,
    ticker: &str,
    tx_type: &str,
    trade_date: chrono::NaiveDate,
    quantity: Decimal,
    price: Decimal,
    fees: Decimal,
    total_cost: Decimal,
    notes: Option<&str>,
) -> String {
    let mut rows = vec![
        KeyValueRow {
            label: "Transaction ID".to_string(),
            value: Value::Text(tx_id.to_string()),
        },
        KeyValueRow {
            label: "Ticker".to_string(),
            value: Value::Text(ticker.to_string()),
        },
        KeyValueRow {
            label: "Type".to_string(),
            value: Value::Text(tx_type.to_uppercase()),
        },
        KeyValueRow {
            label: "Date".to_string(),
            value: Value::Date(trade_date),
        },
        KeyValueRow {
            label: "Quantity".to_string(),
            value: Value::Quantity(quantity),
        },
        KeyValueRow {
            label: "Price".to_string(),
            value: Value::Currency(price),
        },
        KeyValueRow {
            label: "Fees".to_string(),
            value: Value::Currency(fees),
        },
        KeyValueRow {
            label: "Total".to_string(),
            value: Value::Currency(total_cost),
        },
    ];

    if let Some(n) = notes {
        rows.push(KeyValueRow {
            label: "Notes".to_string(),
            value: Value::Text(n.to_string()),
        });
    }

    let document = OutputDocument {
        title: None,
        blocks: vec![
            OutputBlock::Header {
                level: 1,
                text: "Transaction added successfully!".to_string(),
            },
            OutputBlock::KeyValue { title: None, rows },
        ],
        meta: Default::default(),
    };

    crate::formatters::render_document(&document, OutputMode::Table)
}

//
// 2. TRANSACTION LIST FORMATTERS
//

pub struct TransactionRow {
    pub id: Option<i64>,
    pub ticker: String,
    pub transaction_type: String,
    pub trade_date: chrono::NaiveDate,
    pub settlement_date: Option<chrono::NaiveDate>,
    pub quantity: Decimal,
    pub price_per_unit: Decimal,
    pub total_cost: Decimal,
    pub fees: Decimal,
    pub is_day_trade: bool,
    pub notes: Option<String>,
    pub source: String,
}

/// Format transactions list
pub fn format_transactions_list(rows: &[TransactionRow], mode: OutputMode) -> String {
    let document = build_transactions_list_document(rows);
    crate::formatters::render_document(&document, mode)
}

// Internal implementations below

fn build_transactions_list_document(rows: &[TransactionRow]) -> OutputDocument {
    if rows.is_empty() {
        return OutputDocument {
            title: None,
            blocks: vec![OutputBlock::EmptyState {
                message: "No transactions found".to_string(),
                hint: None,
            }],
            meta: Default::default(),
        };
    }

    let table_rows = rows
        .iter()
        .map(|row| Row {
            cells: vec![
                Value::Text(row.id.unwrap_or(0).to_string()),
                Value::Text(row.ticker.clone()),
                Value::Text(row.transaction_type.clone()),
                Value::Date(row.trade_date),
                row.settlement_date.map(Value::Date).unwrap_or(Value::Null),
                Value::Quantity(row.quantity),
                Value::Currency(row.price_per_unit),
                Value::Currency(row.total_cost),
                Value::Currency(row.fees),
                Value::Text(row.is_day_trade.to_string()),
                row.notes.clone().map(Value::Text).unwrap_or(Value::Null),
                Value::Text(row.source.clone()),
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
                ColumnDef::new("trade_date", "Trade Date", ValueKind::Date),
                ColumnDef::new("settlement_date", "Settlement Date", ValueKind::Date),
                ColumnDef::new("quantity", "Quantity", ValueKind::Quantity),
                ColumnDef::new("price_per_unit", "Price", ValueKind::Currency),
                ColumnDef::new("total_cost", "Total Cost", ValueKind::Currency),
                ColumnDef::new("fees", "Fees", ValueKind::Currency),
                ColumnDef::new("day_trade", "Day Trade", ValueKind::Text),
                ColumnDef::new("notes", "Notes", ValueKind::Text),
                ColumnDef::new("source", "Source", ValueKind::Text),
            ],
            rows: table_rows,
            footer: None,
            options: Default::default(),
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
    fn test_transaction_add_table_format() {
        let output = format_transaction_add_table(
            42,
            "PETR4",
            "buy",
            NaiveDate::from_ymd_opt(2024, 1, 15).unwrap(),
            Decimal::from_str("100").unwrap(),
            Decimal::from_str("25.50").unwrap(),
            Decimal::from_str("5.00").unwrap(),
            Decimal::from_str("2555.00").unwrap(),
            Some("Test transaction"),
        );

        assert!(output.contains("Transaction ID") && output.contains("42"));
        assert!(output.contains("Ticker") && output.contains("PETR4"));
        assert!(output.contains("Type") && output.contains("BUY"));
        assert!(output.contains("Notes") && output.contains("Test transaction"));
    }

    #[test]
    fn test_transactions_list_json_decimals_as_strings() {
        let rows = vec![TransactionRow {
            id: Some(1),
            ticker: "PETR4".to_string(),
            transaction_type: "BUY".to_string(),
            trade_date: NaiveDate::from_ymd_opt(2024, 1, 15).unwrap(),
            settlement_date: Some(NaiveDate::from_ymd_opt(2024, 1, 17).unwrap()),
            quantity: Decimal::from_str("100").unwrap(),
            price_per_unit: Decimal::from_str("25.50").unwrap(),
            total_cost: Decimal::from_str("2555.00").unwrap(),
            fees: Decimal::from_str("5.00").unwrap(),
            is_day_trade: false,
            notes: None,
            source: "CEI".to_string(),
        }];

        let json_str = format_transactions_list(&rows, OutputMode::Json);
        let value: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        let blocks = value["blocks"].as_array().expect("blocks array missing");
        let mut has_currency = false;
        let mut has_quantity = false;

        for block in blocks {
            if block.get("type").and_then(|t| t.as_str()) == Some("table") {
                if let Some(rows) = block.get("rows").and_then(|r| r.as_array()) {
                    for row in rows {
                        if let Some(cells) = row.get("cells").and_then(|c| c.as_array()) {
                            for cell in cells {
                                match cell.get("kind").and_then(|k| k.as_str()) {
                                    Some("currency") => {
                                        if let Some(raw) = cell.get("value") {
                                            assert!(raw.is_string());
                                            has_currency = true;
                                        }
                                    }
                                    Some("quantity") => {
                                        if let Some(raw) = cell.get("value") {
                                            assert!(raw.is_string());
                                            has_quantity = true;
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            }
        }

        assert!(has_currency, "expected currency fields in JSON");
        assert!(has_quantity, "expected quantity fields in JSON");
    }

    #[test]
    fn test_transactions_list_table_empty() {
        let rows: Vec<TransactionRow> = vec![];
        let output = format_transactions_list(&rows, OutputMode::Table);
        assert!(output.contains("No transactions found"));
    }
}
