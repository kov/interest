//! JSON renderer for OutputDocument.

use crate::output::{
    AlignmentHint, ColumnDef, KeyValueRow, OutputBlock, OutputDocument, Row, TableOptions,
    TableStyle, Value, ValueKind,
};
use serde::Serialize;

pub struct JsonRenderer;

impl JsonRenderer {
    pub fn render(doc: &OutputDocument) -> String {
        let json_doc = OutputDocumentJson::from_document(doc);
        serde_json::to_string_pretty(&json_doc)
            .unwrap_or_else(|e| format!(r#"{{"error": "JSON serialization failed: {}"}}"#, e))
    }
}

#[derive(Serialize)]
struct OutputDocumentJson {
    title: Option<String>,
    blocks: Vec<OutputBlockJson>,
    meta: OutputMetaJson,
}

#[derive(Serialize)]
struct OutputMetaJson {
    #[serde(skip_serializing_if = "Option::is_none")]
    generated_at: Option<String>,
}

#[derive(Serialize)]
#[serde(tag = "type")]
enum OutputBlockJson {
    #[serde(rename = "header")]
    Header { level: u8, text: String },
    #[serde(rename = "empty_state")]
    EmptyState {
        message: String,
        hint: Option<String>,
    },
    #[serde(rename = "key_value")]
    KeyValue {
        title: Option<String>,
        rows: Vec<KeyValueRowJson>,
    },
    #[serde(rename = "table")]
    Table {
        title: Option<String>,
        columns: Vec<ColumnDefJson>,
        rows: Vec<RowJson>,
        footer: Option<RowJson>,
        options: TableOptionsJson,
    },
    #[serde(rename = "section")]
    Section {
        title: Option<String>,
        blocks: Vec<OutputBlockJson>,
    },
}

#[derive(Serialize)]
struct KeyValueRowJson {
    label: String,
    value: ValueJson,
}

#[derive(Serialize)]
struct ColumnDefJson {
    key: String,
    label: String,
    kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    align: Option<String>,
}

#[derive(Serialize)]
struct RowJson {
    cells: Vec<ValueJson>,
}

#[derive(Serialize)]
struct TableOptionsJson {
    style: String,
}

#[derive(Serialize)]
struct ValueJson {
    kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<serde_json::Value>,
}

impl OutputDocumentJson {
    fn from_document(doc: &OutputDocument) -> Self {
        Self {
            title: doc.title.clone(),
            blocks: doc.blocks.iter().map(OutputBlockJson::from_block).collect(),
            meta: OutputMetaJson {
                generated_at: doc
                    .meta
                    .generated_at
                    .map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
            },
        }
    }
}

impl OutputBlockJson {
    fn from_block(block: &OutputBlock) -> Self {
        match block {
            OutputBlock::Header { level, text } => OutputBlockJson::Header {
                level: *level,
                text: text.clone(),
            },
            OutputBlock::EmptyState { message, hint } => OutputBlockJson::EmptyState {
                message: message.clone(),
                hint: hint.clone(),
            },
            OutputBlock::KeyValue { title, rows } => OutputBlockJson::KeyValue {
                title: title.clone(),
                rows: rows.iter().map(KeyValueRowJson::from_row).collect(),
            },
            OutputBlock::Table {
                title,
                columns,
                rows,
                footer,
                options,
            } => OutputBlockJson::Table {
                title: title.clone(),
                columns: columns.iter().map(ColumnDefJson::from_column).collect(),
                rows: rows.iter().map(RowJson::from_row).collect(),
                footer: footer.as_ref().map(RowJson::from_row),
                options: TableOptionsJson::from_options(options),
            },
            OutputBlock::Section { title, blocks } => OutputBlockJson::Section {
                title: title.clone(),
                blocks: blocks.iter().map(OutputBlockJson::from_block).collect(),
            },
        }
    }
}

impl KeyValueRowJson {
    fn from_row(row: &KeyValueRow) -> Self {
        Self {
            label: row.label.clone(),
            value: ValueJson::from_value(&row.value),
        }
    }
}

impl ColumnDefJson {
    fn from_column(column: &ColumnDef) -> Self {
        Self {
            key: column.key.clone(),
            label: column.label.clone(),
            kind: column.kind.as_str().to_string(),
            align: column.align.map(AlignmentHintJson::from_align),
        }
    }
}

struct AlignmentHintJson;

impl AlignmentHintJson {
    fn from_align(align: AlignmentHint) -> String {
        match align {
            AlignmentHint::Left => "left",
            AlignmentHint::Right => "right",
        }
        .to_string()
    }
}

impl RowJson {
    fn from_row(row: &Row) -> Self {
        Self {
            cells: row.cells.iter().map(ValueJson::from_value).collect(),
        }
    }
}

impl TableOptionsJson {
    fn from_options(options: &TableOptions) -> Self {
        Self {
            style: TableStyleJson::from_style(options.style),
        }
    }
}

struct TableStyleJson;

impl TableStyleJson {
    fn from_style(style: TableStyle) -> String {
        match style {
            TableStyle::Modern => "modern",
            TableStyle::Rounded => "rounded",
        }
        .to_string()
    }
}

impl ValueJson {
    fn from_value(value: &Value) -> Self {
        let kind = value.kind().as_str().to_string();
        let raw = value.to_raw_json_value();
        let value = match value.kind() {
            ValueKind::Null => None,
            _ => Some(raw),
        };
        Self { kind, value }
    }
}
