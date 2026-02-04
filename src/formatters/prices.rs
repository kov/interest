use crate::formatters::OutputOptions;
use crate::output::{
    ColumnDef, OutputBlock, OutputDocument, Row, TableOptions, TableStyle, Value, ValueKind,
};
use crate::pricing::yahoo::HistoricalPrice;
/// Formatters for price commands
use anyhow::Result;

//
// PRICE DATA TABLE
//

pub fn format_prices_table(prices: &[HistoricalPrice], options: OutputOptions) -> Result<String> {
    let document = build_prices_document(prices);
    crate::formatters::render_document(&document, options)
}

fn build_prices_document(prices: &[HistoricalPrice]) -> OutputDocument {
    if prices.is_empty() {
        return OutputDocument {
            title: None,
            blocks: vec![OutputBlock::EmptyState {
                message: "No price data found".to_string(),
                hint: None,
            }],
            meta: Default::default(),
        };
    }

    let rows = prices
        .iter()
        .map(|p| Row {
            cells: vec![
                Value::Date(p.date),
                p.open.map(Value::Currency).unwrap_or(Value::Null),
                p.high.map(Value::Currency).unwrap_or(Value::Null),
                p.low.map(Value::Currency).unwrap_or(Value::Null),
                Value::Currency(p.close),
                p.volume
                    .map(|v| Value::Text(v.to_string()))
                    .unwrap_or(Value::Null),
            ],
        })
        .collect::<Vec<_>>();

    OutputDocument {
        title: None,
        blocks: vec![OutputBlock::Table {
            title: None,
            columns: vec![
                ColumnDef::new("date", "Date", ValueKind::Date),
                ColumnDef::new("open", "Open", ValueKind::Currency),
                ColumnDef::new("high", "High", ValueKind::Currency),
                ColumnDef::new("low", "Low", ValueKind::Currency),
                ColumnDef::new("close", "Close", ValueKind::Currency),
                ColumnDef::new("volume", "Volume", ValueKind::Text),
            ],
            rows,
            footer: None,
            options: TableOptions {
                style: TableStyle::Rounded,
            },
        }],
        meta: Default::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prices_table_empty() {
        let prices: Vec<HistoricalPrice> = vec![];
        let output = format_prices_table(&prices, OutputOptions::from_flags(false, false)).unwrap();
        assert!(output.contains("No price data found"));
        assert!(output.contains("ℹ"));
    }
}
