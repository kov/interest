/// Formatters for import commands (CEI, Movimentação, Ofertas Públicas)
use crate::output::{
    ColumnDef, KeyValueRow, OutputBlock, OutputDocument, Row, TableOptions, TableStyle, Value,
    ValueKind,
};

use crate::formatters::OutputMode;
use crate::importers::ImportStats;

//
// 1. IMPORT STATS FORMATTERS
//

/// Format import stats
pub fn format_import_stats(stats: &ImportStats, mode: OutputMode) -> String {
    let document = build_import_stats_document(stats);
    crate::formatters::render_document(&document, mode)
}

// Internal implementations below

fn build_import_stats_document(stats: &ImportStats) -> OutputDocument {
    let mut trade_rows = vec![KeyValueRow {
        label: "Imported".to_string(),
        value: Value::Text(stats.imported_trades.to_string()),
    }];
    if stats.skipped_trades_old > 0 {
        trade_rows.push(KeyValueRow {
            label: "Skipped (before last import date)".to_string(),
            value: Value::Text(stats.skipped_trades_old.to_string()),
        });
    }
    if stats.skipped_trades > 0 {
        trade_rows.push(KeyValueRow {
            label: "Skipped".to_string(),
            value: Value::Text(stats.skipped_trades.to_string()),
        });
    }

    let mut action_rows = vec![KeyValueRow {
        label: "Imported".to_string(),
        value: Value::Text(stats.imported_actions.to_string()),
    }];
    if stats.skipped_actions_old > 0 {
        action_rows.push(KeyValueRow {
            label: "Skipped (before last import date)".to_string(),
            value: Value::Text(stats.skipped_actions_old.to_string()),
        });
    }
    if stats.skipped_actions > 0 {
        action_rows.push(KeyValueRow {
            label: "Skipped".to_string(),
            value: Value::Text(stats.skipped_actions.to_string()),
        });
    }
    if stats.auto_applied_actions > 0 {
        action_rows.push(KeyValueRow {
            label: "Auto-applied".to_string(),
            value: Value::Text(stats.auto_applied_actions.to_string()),
        });
    }

    let mut income_rows = vec![KeyValueRow {
        label: "Imported".to_string(),
        value: Value::Text(stats.imported_income.to_string()),
    }];
    if stats.skipped_income_old > 0 {
        income_rows.push(KeyValueRow {
            label: "Skipped (before last import date)".to_string(),
            value: Value::Text(stats.skipped_income_old.to_string()),
        });
    }
    if stats.skipped_income > 0 {
        income_rows.push(KeyValueRow {
            label: "Skipped".to_string(),
            value: Value::Text(stats.skipped_income.to_string()),
        });
    }

    OutputDocument {
        title: None,
        blocks: vec![
            OutputBlock::Header {
                level: 1,
                text: "Import complete!".to_string(),
            },
            OutputBlock::KeyValue {
                title: Some("Trades".to_string()),
                rows: trade_rows,
            },
            OutputBlock::KeyValue {
                title: Some("Corporate actions".to_string()),
                rows: action_rows,
            },
            OutputBlock::KeyValue {
                title: Some("Income events".to_string()),
                rows: income_rows,
            },
        ],
        meta: Default::default(),
    }
}

//
// 2. CEI PREVIEW TABLE
//

pub fn format_cei_preview_table(txs: &[crate::importers::RawTransaction]) -> Option<String> {
    let rows = txs
        .iter()
        .take(10)
        .map(|tx| Row {
            cells: vec![
                Value::Text(tx.trade_date.format("%d/%m/%Y").to_string()),
                Value::Text(tx.ticker.clone()),
                Value::Text(tx.transaction_type.as_str().to_string()),
                Value::Quantity(tx.quantity),
                Value::Currency(tx.price),
                Value::Currency(tx.total),
            ],
        })
        .collect::<Vec<_>>();

    if rows.is_empty() {
        None
    } else {
        let document = OutputDocument {
            title: None,
            blocks: vec![OutputBlock::Table {
                title: None,
                columns: vec![
                    ColumnDef::new("date", "Date", ValueKind::Text),
                    ColumnDef::new("ticker", "Ticker", ValueKind::Text),
                    ColumnDef::new("type", "Type", ValueKind::Text),
                    ColumnDef::new("quantity", "Quantity", ValueKind::Quantity),
                    ColumnDef::new("price", "Price", ValueKind::Currency),
                    ColumnDef::new("total", "Total", ValueKind::Currency),
                ],
                rows,
                footer: None,
                options: TableOptions {
                    style: TableStyle::Rounded,
                },
            }],
            meta: Default::default(),
        };
        Some(crate::formatters::render_document(
            &document,
            OutputMode::Table,
        ))
    }
}

//
// 3. MOVIMENTAÇÃO PREVIEW TABLE
//

pub fn format_movimentacao_preview_table(
    trades: &[crate::importers::MovimentacaoEntry],
) -> Option<String> {
    let rows = trades
        .iter()
        .take(5)
        .map(|e| Row {
            cells: vec![
                Value::Text(e.date.format("%d/%m/%Y").to_string()),
                Value::Text(e.movement_type.clone()),
                Value::Text(e.ticker.clone().unwrap_or_else(|| "?".to_string())),
                e.quantity.map(Value::Quantity).unwrap_or(Value::Null),
                e.unit_price.map(Value::Currency).unwrap_or(Value::Null),
            ],
        })
        .collect::<Vec<_>>();

    if rows.is_empty() {
        None
    } else {
        let document = OutputDocument {
            title: None,
            blocks: vec![OutputBlock::Table {
                title: None,
                columns: vec![
                    ColumnDef::new("date", "Date", ValueKind::Text),
                    ColumnDef::new("type", "Type", ValueKind::Text),
                    ColumnDef::new("ticker", "Ticker", ValueKind::Text),
                    ColumnDef::new("quantity", "Qty", ValueKind::Quantity),
                    ColumnDef::new("price", "Price", ValueKind::Currency),
                ],
                rows,
                footer: None,
                options: TableOptions {
                    style: TableStyle::Rounded,
                },
            }],
            meta: Default::default(),
        };
        Some(crate::formatters::render_document(
            &document,
            OutputMode::Table,
        ))
    }
}

//
// 4. OFERTAS PÚBLICAS PREVIEW TABLE
//

pub fn format_ofertas_preview_table(
    entries: &[crate::importers::OfertaPublicaEntry],
) -> Option<String> {
    let rows = entries
        .iter()
        .take(5)
        .map(|e| Row {
            cells: vec![
                Value::Text(e.date.format("%d/%m/%Y").to_string()),
                Value::Text(e.ticker.clone()),
                Value::Quantity(e.quantity),
                Value::Currency(e.unit_price),
                Value::Text(e.offer.clone()),
            ],
        })
        .collect::<Vec<_>>();

    if rows.is_empty() {
        None
    } else {
        let document = OutputDocument {
            title: None,
            blocks: vec![OutputBlock::Table {
                title: None,
                columns: vec![
                    ColumnDef::new("date", "Date", ValueKind::Text),
                    ColumnDef::new("ticker", "Ticker", ValueKind::Text),
                    ColumnDef::new("quantity", "Qty", ValueKind::Quantity),
                    ColumnDef::new("price", "Price", ValueKind::Currency),
                    ColumnDef::new("offer", "Offer", ValueKind::Text),
                ],
                rows,
                footer: None,
                options: TableOptions {
                    style: TableStyle::Rounded,
                },
            }],
            meta: Default::default(),
        };
        Some(crate::formatters::render_document(
            &document,
            OutputMode::Table,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_import_stats_json_structure() {
        let stats = ImportStats {
            imported: 5,
            skipped_old: 0,
            errors: 0,
            earliest: None,
            latest: None,
            imported_trades: 3,
            skipped_trades: 1,
            skipped_trades_old: 0,
            imported_actions: 2,
            skipped_actions: 0,
            skipped_actions_old: 0,
            auto_applied_actions: 1,
            imported_income: 0,
            skipped_income: 0,
            skipped_income_old: 0,
        };

        let json_str = format_import_stats(&stats, OutputMode::Json);
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

        assert!(values.contains(&"3".to_string()));
        assert!(values.contains(&"2".to_string()));
    }

    #[test]
    fn test_cei_preview_empty() {
        let txs: Vec<crate::importers::RawTransaction> = vec![];
        assert!(format_cei_preview_table(&txs).is_none());
    }

    #[test]
    fn test_movimentacao_preview_empty() {
        let trades: Vec<crate::importers::MovimentacaoEntry> = vec![];
        assert!(format_movimentacao_preview_table(&trades).is_none());
    }
}
