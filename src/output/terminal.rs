//! Terminal renderer for OutputDocument (tables + colored text).

use crate::output::{
    AlignmentHint, ColumnDef, OutputBlock, OutputDocument, Row, TableOptions, TableStyle, Value,
    ValueKind,
};
use crate::utils::format_decimal_br;
use colored::Colorize;
use rust_decimal::Decimal;
use tabled::{
    builder::Builder,
    settings::{object::Columns, Alignment, Style},
};

pub struct TerminalRenderer;

impl TerminalRenderer {
    pub fn render(doc: &OutputDocument) -> String {
        let mut output = String::new();

        if let Some(title) = &doc.title {
            output.push_str(&format!("{}\n", title.bold().cyan()));
        }

        for block in &doc.blocks {
            let rendered = render_block(block, 0);
            if rendered.is_empty() {
                continue;
            }
            if !output.is_empty() && !output.ends_with('\n') {
                output.push('\n');
            }
            if !output.is_empty() {
                output.push('\n');
            }
            output.push_str(&rendered);
        }

        output
    }
}

fn render_block(block: &OutputBlock, indent: usize) -> String {
    match block {
        OutputBlock::Header { level, text } => render_header(*level, text, indent),
        OutputBlock::EmptyState { message, hint } => render_empty_state(message, hint, indent),
        OutputBlock::KeyValue { title, rows } => render_key_value(title, rows, indent),
        OutputBlock::Table {
            title,
            columns,
            rows,
            footer,
            options,
        } => render_table(title, columns, rows, footer.as_ref(), options, indent),
        OutputBlock::Section { title, blocks } => render_section(title, blocks, indent),
    }
}

fn render_header(level: u8, text: &str, indent: usize) -> String {
    let formatted = if level <= 1 {
        text.bold().cyan().to_string()
    } else {
        text.bold().to_string()
    };
    indent_lines(&formatted, indent)
}

fn render_empty_state(message: &str, hint: &Option<String>, indent: usize) -> String {
    let mut text = format!("{} {}", "ℹ".blue().bold(), message);
    if let Some(hint) = hint {
        text.push('\n');
        text.push_str(hint);
    }
    indent_lines(&text, indent)
}

fn render_key_value(
    title: &Option<String>,
    rows: &[crate::output::KeyValueRow],
    indent: usize,
) -> String {
    let mut lines = Vec::new();

    if let Some(title) = title {
        lines.push(title.bold().to_string());
    }

    let max_label_width = rows.iter().map(|row| row.label.len()).max().unwrap_or(0);
    let max_numeric_width = rows
        .iter()
        .filter_map(|row| match row.value {
            Value::Currency(amount) | Value::CurrencyDelta(amount) => {
                Some(visible_len(&format_decimal_br(amount)))
            }
            _ => None,
        })
        .max()
        .unwrap_or(0);
    let rendered_values: Vec<KeyValueRender> = rows
        .iter()
        .map(|row| KeyValueRender::from_value(&row.value, max_numeric_width))
        .collect();
    let max_total_width = rendered_values
        .iter()
        .map(|value| value.total_width)
        .max()
        .unwrap_or(0);

    for (row, render) in rows.iter().zip(rendered_values.iter()) {
        let padding = max_total_width.saturating_sub(render.total_width);
        lines.push(format!(
            "  {:width$}: {}{}{}",
            row.label,
            render.prefix,
            " ".repeat(padding),
            render.value,
            width = max_label_width
        ));
    }

    indent_lines(&lines.join("\n"), indent)
}

fn render_table(
    title: &Option<String>,
    columns: &[ColumnDef],
    rows: &[Row],
    footer: Option<&Row>,
    options: &TableOptions,
    indent: usize,
) -> String {
    if rows.is_empty() {
        return String::new();
    }

    let currency_widths = currency_numeric_widths(columns, rows, footer);
    let mut builder = Builder::default();
    builder.push_record(columns.iter().map(|col| col.label.clone()));

    for row in rows {
        let cells = columns
            .iter()
            .enumerate()
            .map(|(idx, _col)| {
                let cell = row.cells.get(idx).unwrap_or(&Value::Null);
                format_table_cell(cell, currency_widths[idx])
            })
            .collect::<Vec<_>>();
        builder.push_record(cells);
    }

    if let Some(footer) = footer {
        let cells = columns
            .iter()
            .enumerate()
            .map(|(idx, _col)| {
                let cell = footer.cells.get(idx).unwrap_or(&Value::Null);
                format_table_cell(cell, currency_widths[idx])
            })
            .collect::<Vec<_>>();
        builder.push_record(cells);
    }

    let mut table = builder.build();
    match options.style {
        TableStyle::Modern => {
            table.with(Style::modern());
        }
        TableStyle::Rounded => {
            table.with(Style::rounded());
        }
    }

    for (idx, column) in columns.iter().enumerate() {
        let align = column
            .align
            .or_else(|| match column.kind {
                ValueKind::Currency | ValueKind::CurrencyDelta => Some(AlignmentHint::Right),
                _ if column.kind.is_numeric() => Some(AlignmentHint::Right),
                _ => None,
            })
            .unwrap_or(AlignmentHint::Left);

        let alignment = match align {
            AlignmentHint::Left => Alignment::left(),
            AlignmentHint::Right => Alignment::right(),
        };

        table.modify(Columns::new(idx..idx + 1), alignment);
    }

    let mut output = String::new();
    if let Some(title) = title {
        output.push_str(&title.bold().to_string());
        output.push('\n');
    }
    output.push_str(&table.to_string());

    indent_lines(&output, indent)
}

fn render_section(title: &Option<String>, blocks: &[OutputBlock], indent: usize) -> String {
    let mut output = String::new();

    if let Some(title) = title {
        output.push_str(&indent_lines(&title.bold().to_string(), indent));
    }

    for block in blocks {
        let rendered = render_block(block, indent);
        if rendered.is_empty() {
            continue;
        }
        if !output.is_empty() {
            output.push('\n');
        }
        output.push_str(&rendered);
    }

    output
}

fn format_value(value: &Value) -> String {
    match value {
        Value::Text(text) => text.clone(),
        Value::Quantity(qty) => format!("{:.2}", qty),
        Value::Currency(amount) => crate::utils::format_currency(*amount),
        Value::CurrencyDelta(amount) => format_signed_currency(*amount),
        Value::Percent(pct) => format_signed_percent(*pct),
        Value::Date(date) => date.format("%Y-%m-%d").to_string(),
        Value::DateTime(dt) => dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        Value::Null => "N/A".to_string(),
    }
}

fn format_table_cell(value: &Value, currency_width: usize) -> String {
    match value {
        Value::Currency(amount) => format_currency_cell(*amount, currency_width, false),
        Value::CurrencyDelta(amount) => format_currency_cell(*amount, currency_width, true),
        _ => format_value(value),
    }
}

struct KeyValueRender {
    prefix: String,
    value: String,
    total_width: usize,
}

impl KeyValueRender {
    fn from_value(value: &Value, numeric_width: usize) -> Self {
        match value {
            Value::Currency(amount) => {
                let numeric = format_decimal_br(*amount);
                let padded = format!("{:>width$}", numeric, width = numeric_width);
                let total_width = visible_len("R$ ") + numeric_width;
                Self {
                    prefix: "R$ ".to_string(),
                    value: padded,
                    total_width,
                }
            }
            Value::CurrencyDelta(amount) => {
                let numeric = format_decimal_br(*amount);
                let padded = format!("{:>width$}", numeric, width = numeric_width);
                let colored = if *amount >= Decimal::ZERO {
                    padded.green().to_string()
                } else {
                    padded.red().to_string()
                };
                let total_width = visible_len("R$ ") + numeric_width;
                Self {
                    prefix: "R$ ".to_string(),
                    value: colored,
                    total_width,
                }
            }
            Value::Percent(percent) => {
                let numeric = format!("{:.2}%", percent);
                let colored = if *percent >= Decimal::ZERO {
                    numeric.green().to_string()
                } else {
                    numeric.red().to_string()
                };
                let total_width = visible_len(&numeric);
                Self {
                    prefix: String::new(),
                    value: colored,
                    total_width,
                }
            }
            Value::Quantity(quantity) => {
                let numeric = format!("{:.2}", quantity);
                let total_width = visible_len(&numeric);
                Self {
                    prefix: String::new(),
                    value: numeric,
                    total_width,
                }
            }
            Value::Date(date) => {
                let text = date.format("%Y-%m-%d").to_string();
                let total_width = visible_len(&text);
                Self {
                    prefix: String::new(),
                    value: text,
                    total_width,
                }
            }
            Value::DateTime(dt) => {
                let text = dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
                let total_width = visible_len(&text);
                Self {
                    prefix: String::new(),
                    value: text,
                    total_width,
                }
            }
            Value::Text(text) => {
                let total_width = visible_len(text);
                Self {
                    prefix: String::new(),
                    value: text.clone(),
                    total_width,
                }
            }
            Value::Null => {
                let text = "N/A".to_string();
                let total_width = visible_len(&text);
                Self {
                    prefix: String::new(),
                    value: text,
                    total_width,
                }
            }
        }
    }
}

fn format_signed_currency(value: Decimal) -> String {
    let formatted = crate::utils::format_currency(value);
    if value >= Decimal::ZERO {
        formatted.green().to_string()
    } else {
        formatted.red().to_string()
    }
}

fn format_currency_cell(value: Decimal, numeric_width: usize, colored: bool) -> String {
    let numeric = format_decimal_br(value);
    let padded = format!("{:>width$}", numeric, width = numeric_width);
    let display = if colored {
        if value >= Decimal::ZERO {
            padded.green().to_string()
        } else {
            padded.red().to_string()
        }
    } else {
        padded
    };
    format!("R$ {}", display)
}

fn currency_numeric_widths(
    columns: &[ColumnDef],
    rows: &[Row],
    footer: Option<&Row>,
) -> Vec<usize> {
    let mut widths = vec![0usize; columns.len()];
    let prefix_len = visible_len("R$ ");
    for (idx, col) in columns.iter().enumerate() {
        if matches!(col.kind, ValueKind::Currency | ValueKind::CurrencyDelta) {
            let header_width = visible_len(&col.label);
            widths[idx] = widths[idx].max(header_width.saturating_sub(prefix_len));
        }
    }
    let mut scan_row = |row: &Row| {
        for (idx, col) in columns.iter().enumerate() {
            if !matches!(col.kind, ValueKind::Currency | ValueKind::CurrencyDelta) {
                continue;
            }
            let Some(cell) = row.cells.get(idx) else {
                continue;
            };
            match cell {
                Value::Currency(amount) | Value::CurrencyDelta(amount) => {
                    let numeric = format_decimal_br(*amount);
                    widths[idx] = widths[idx].max(visible_len(&numeric));
                }
                _ => {}
            }
        }
    };

    for row in rows {
        scan_row(row);
    }
    if let Some(footer) = footer {
        scan_row(footer);
    }
    widths
}

fn format_signed_percent(value: Decimal) -> String {
    let formatted = format!("{:.2}%", value);
    if value >= Decimal::ZERO {
        formatted.green().to_string()
    } else {
        formatted.red().to_string()
    }
}

fn indent_lines(text: &str, indent: usize) -> String {
    if indent == 0 {
        return text.to_string();
    }
    let prefix = " ".repeat(indent);
    text.lines()
        .map(|line| format!("{}{}", prefix, line))
        .collect::<Vec<_>>()
        .join("\n")
}

fn visible_len(text: &str) -> usize {
    let mut count = 0;
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for next in chars.by_ref() {
                if next == 'm' {
                    break;
                }
            }
            continue;
        }
        count += 1;
    }
    count
}
