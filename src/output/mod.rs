//! Output document model and renderers.
//!
//! Formatters build OutputDocument instances; renderers turn them into
//! terminal tables, JSON, or other formats.

use chrono::{DateTime, NaiveDate, SecondsFormat, Utc};
use rust_decimal::Decimal;

pub mod json;
pub mod terminal;

#[derive(Debug, Clone, Default)]
pub struct OutputDocument {
    pub title: Option<String>,
    pub blocks: Vec<OutputBlock>,
    pub meta: OutputMeta,
}

#[derive(Debug, Clone, Default)]
pub struct OutputMeta {
    pub generated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub enum OutputBlock {
    Header {
        level: u8,
        text: String,
    },
    EmptyState {
        message: String,
        hint: Option<String>,
    },
    KeyValue {
        title: Option<String>,
        rows: Vec<KeyValueRow>,
    },
    Table {
        title: Option<String>,
        columns: Vec<ColumnDef>,
        rows: Vec<Row>,
        footer: Option<Row>,
        options: TableOptions,
    },
    Section {
        title: Option<String>,
        blocks: Vec<OutputBlock>,
    },
}

#[derive(Debug, Clone)]
pub struct KeyValueRow {
    pub label: String,
    pub value: Value,
}

#[derive(Debug, Clone)]
pub struct ColumnDef {
    pub key: String,
    pub label: String,
    pub kind: ValueKind,
    pub align: Option<AlignmentHint>,
}

impl ColumnDef {
    pub fn new(key: &str, label: &str, kind: ValueKind) -> Self {
        Self {
            key: key.to_string(),
            label: label.to_string(),
            kind,
            align: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Row {
    pub cells: Vec<Value>,
}

#[derive(Debug, Clone, Copy)]
pub enum AlignmentHint {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy)]
pub enum TableStyle {
    Modern,
    Rounded,
}

#[derive(Debug, Clone)]
pub struct TableOptions {
    pub style: TableStyle,
}

impl Default for TableOptions {
    fn default() -> Self {
        Self {
            style: TableStyle::Modern,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueKind {
    Text,
    Quantity,
    Currency,
    CurrencyDelta,
    Percent,
    Date,
    DateTime,
    Null,
}

impl ValueKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ValueKind::Text => "text",
            ValueKind::Quantity => "quantity",
            ValueKind::Currency => "currency",
            ValueKind::CurrencyDelta => "currency_delta",
            ValueKind::Percent => "percent",
            ValueKind::Date => "date",
            ValueKind::DateTime => "datetime",
            ValueKind::Null => "null",
        }
    }

    pub fn is_numeric(&self) -> bool {
        matches!(
            self,
            ValueKind::Quantity
                | ValueKind::Currency
                | ValueKind::CurrencyDelta
                | ValueKind::Percent
        )
    }
}

#[derive(Debug, Clone)]
pub enum Value {
    Text(String),
    Quantity(Decimal),
    Currency(Decimal),
    CurrencyDelta(Decimal),
    Percent(Decimal),
    Date(NaiveDate),
    DateTime(DateTime<Utc>),
    Null,
}

impl Value {
    pub fn kind(&self) -> ValueKind {
        match self {
            Value::Text(_) => ValueKind::Text,
            Value::Quantity(_) => ValueKind::Quantity,
            Value::Currency(_) => ValueKind::Currency,
            Value::CurrencyDelta(_) => ValueKind::CurrencyDelta,
            Value::Percent(_) => ValueKind::Percent,
            Value::Date(_) => ValueKind::Date,
            Value::DateTime(_) => ValueKind::DateTime,
            Value::Null => ValueKind::Null,
        }
    }

    pub fn to_raw_json_value(&self) -> serde_json::Value {
        match self {
            Value::Text(value) => serde_json::Value::String(value.clone()),
            Value::Quantity(value) => serde_json::Value::String(value.to_string()),
            Value::Currency(value) => serde_json::Value::String(value.to_string()),
            Value::CurrencyDelta(value) => serde_json::Value::String(value.to_string()),
            Value::Percent(value) => serde_json::Value::String(value.to_string()),
            Value::Date(value) => serde_json::Value::String(value.format("%Y-%m-%d").to_string()),
            Value::DateTime(value) => {
                serde_json::Value::String(value.to_rfc3339_opts(SecondsFormat::Secs, true))
            }
            Value::Null => serde_json::Value::Null,
        }
    }
}
