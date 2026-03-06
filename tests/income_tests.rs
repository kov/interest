//! Integration tests for income event functionality (CLI-driven)

use anyhow::{Context, Result};
use chrono::Datelike;
use rust_decimal::Decimal;
use serde_json::Value;
use tempfile::TempDir;

mod cli_helpers;
use cli_helpers::{add_asset, add_income, import_stats_json, income_detail_json};

fn decimal_from_value(value: &Value) -> Result<Decimal> {
    if let Some(s) = value.as_str() {
        return Decimal::from_str_exact(s).context("invalid decimal string");
    }
    if let Some(f) = value.as_f64() {
        return Decimal::try_from(f).context("invalid decimal number");
    }
    Err(anyhow::anyhow!("expected decimal value"))
}

#[test]
fn test_income_add_and_detail() -> Result<()> {
    let home = TempDir::new()?;

    add_asset(&home, "XPLG11", "FII")?;
    add_income(&home, "XPLG11", "DIVIDEND", "850", "2024-06-15")?;

    let events = income_detail_json(&home, "2024", "XPLG11")?;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["ticker"], "XPLG11");
    assert_eq!(events[0]["event_type"], "DIVIDEND");
    let amount = decimal_from_value(&events[0]["amount"])?;
    assert_eq!(amount, Decimal::from_str_exact("850")?);

    Ok(())
}

#[test]
fn test_income_detail_orders_by_date() -> Result<()> {
    let home = TempDir::new()?;

    add_asset(&home, "MXRF11", "FII")?;
    add_income(&home, "MXRF11", "DIVIDEND", "10", "2024-02-15")?;
    add_income(&home, "MXRF11", "DIVIDEND", "5", "2024-01-10")?;

    let events = income_detail_json(&home, "2024", "MXRF11")?;
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["date"], "2024-01-10");
    assert_eq!(events[1]["date"], "2024-02-15");

    Ok(())
}

#[test]
fn test_income_detail_year_filter() -> Result<()> {
    let home = TempDir::new()?;

    add_asset(&home, "XPLG11", "FII")?;
    add_income(&home, "XPLG11", "DIVIDEND", "5", "2023-12-31")?;
    add_income(&home, "XPLG11", "DIVIDEND", "10", "2024-01-10")?;

    let events = income_detail_json(&home, "2024", "XPLG11")?;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["date"], "2024-01-10");

    Ok(())
}

#[test]
fn test_income_import_duplicate_detection() -> Result<()> {
    let home = TempDir::new()?;

    let data_first = import_stats_json(&home, "tests/data/07_capital_return.xlsx")?;
    assert_eq!(data_first["imported_income"].as_u64().unwrap_or(0), 1);

    let data_second = import_stats_json(&home, "tests/data/07_capital_return.xlsx")?;
    assert_eq!(data_second["imported_income"].as_u64().unwrap_or(0), 0);
    assert!(data_second["skipped_income_old"].as_u64().unwrap_or(0) >= 1);

    Ok(())
}

#[test]
fn test_income_yield_command() -> Result<()> {
    let home = TempDir::new()?;

    // Setup: Add asset and transactions to have a position
    add_asset(&home, "XPLG11", "FII")?;
    cli_helpers::run_cmd(
        &home,
        &[
            "transactions",
            "add",
            "XPLG11",
            "buy",
            "100",
            "120.00",
            "2023-01-15",
        ],
    )?;

    // Add income events for LTM
    add_income(&home, "XPLG11", "DIVIDEND", "850", "2024-01-15")?;
    add_income(&home, "XPLG11", "DIVIDEND", "850", "2024-06-15")?;

    // Run yield command with JSON output
    let output = cli_helpers::run_cmd_json(&home, &["--json", "income", "yield"])?;

    // Verify structure
    assert!(output.get("blocks").is_some());
    let blocks = output["blocks"].as_array().unwrap();
    assert!(!blocks.is_empty());

    // Should have a header
    assert_eq!(blocks[0]["type"], "header");
    assert!(blocks[0]["text"].as_str().unwrap().contains("Yield"));

    Ok(())
}

#[test]
fn test_income_yield_positional_ticker() -> Result<()> {
    let home = TempDir::new()?;

    add_asset(&home, "XPLG11", "FII")?;
    cli_helpers::run_cmd(
        &home,
        &[
            "transactions",
            "add",
            "XPLG11",
            "buy",
            "100",
            "120.00",
            "2023-01-15",
        ],
    )?;
    add_income(&home, "XPLG11", "DIVIDEND", "850", "2024-06-15")?;

    // Positional ticker: `income yield XPLG11`
    let output = cli_helpers::run_cmd_json(&home, &["--json", "income", "yield", "XPLG11"])?;

    assert!(output.get("blocks").is_some());
    let blocks = output["blocks"].as_array().unwrap();
    assert!(blocks.iter().any(|b| b["type"] == "header"));

    Ok(())
}

#[test]
fn test_income_trends_command() -> Result<()> {
    let home = TempDir::new()?;

    // Setup: Add asset with income history
    add_asset(&home, "MXRF11", "FII")?;
    add_income(&home, "MXRF11", "DIVIDEND", "100", "2023-01-15")?;
    add_income(&home, "MXRF11", "DIVIDEND", "110", "2023-07-15")?;
    add_income(&home, "MXRF11", "DIVIDEND", "120", "2024-01-15")?;

    // Run trends command
    let output = cli_helpers::run_cmd_json(&home, &["--json", "income", "trends"])?;

    // Verify structure
    assert!(output.get("blocks").is_some());
    let blocks = output["blocks"].as_array().unwrap();

    // Should have header and key-value section
    assert!(blocks.iter().any(|b| b["type"] == "header"));
    assert!(blocks.iter().any(|b| b["type"] == "key_value"));

    Ok(())
}

#[test]
fn test_income_trends_positional_ticker() -> Result<()> {
    let home = TempDir::new()?;

    add_asset(&home, "XPLG11", "FII")?;
    add_income(&home, "XPLG11", "DIVIDEND", "100", "2024-01-15")?;
    add_income(&home, "XPLG11", "DIVIDEND", "100", "2024-06-15")?;

    // Positional ticker: `income trends XPLG11`
    let output = cli_helpers::run_cmd_json(&home, &["--json", "income", "trends", "XPLG11"])?;

    assert!(output.get("blocks").is_some());
    let blocks = output["blocks"].as_array().unwrap();
    assert!(blocks.iter().any(|b| b["type"] == "header"));

    Ok(())
}

#[test]
fn test_income_trends_gap_filling() -> Result<()> {
    let home = TempDir::new()?;

    // Asset that paid in Jan 2024 and Jul 2024, with a 5-month gap in between
    add_asset(&home, "GAPPY11", "FII")?;
    add_income(&home, "GAPPY11", "DIVIDEND", "100", "2024-01-15")?;
    add_income(&home, "GAPPY11", "DIVIDEND", "200", "2024-07-15")?;

    // Run trends for this ticker — the monthly series table should include zero months
    let output = cli_helpers::run_cmd_json(&home, &["--json", "income", "trends", "GAPPY11"])?;

    let blocks = output["blocks"].as_array().unwrap();
    let table = blocks
        .iter()
        .find(|b| b["type"] == "table")
        .expect("Should have a monthly series table");

    let rows = table["rows"].as_array().unwrap();

    // Count zero-amount months: cells are { "kind": "currency", "value": "0" }
    let zero_months = rows
        .iter()
        .filter(|row| {
            let cells = row["cells"].as_array().unwrap();
            if let Some(cell) = cells.get(1) {
                cell["value"].as_str() == Some("0")
            } else {
                false
            }
        })
        .count();

    // Between Jan and Jul there are 5 gap months (Feb-Jun), so there should be
    // at least 5 zero-amount rows in the series
    assert!(
        zero_months >= 5,
        "Expected at least 5 zero-income months in the gap, got {}",
        zero_months
    );

    // Also verify the trend is not "Growing" — with a 5-month drought, it shouldn't be
    let kv = blocks
        .iter()
        .find(|b| b["type"] == "key_value")
        .expect("Should have trend summary");
    let direction_row = kv["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["label"] == "Direction")
        .expect("Should have Direction row");
    // KeyValue values serialize as { "kind": "text", "value": "..." }
    let direction = direction_row["value"]["value"].as_str().unwrap();
    assert_ne!(
        direction, "Growing",
        "Trend should not be Growing when there's a 5-month payment drought"
    );

    Ok(())
}

#[test]
fn test_income_forecast_command() -> Result<()> {
    let home = TempDir::new()?;

    // Setup: Add multiple assets with positions and income history
    add_asset(&home, "XPLG11", "FII")?;
    add_asset(&home, "HGLG11", "FII")?;

    // Need positions for forecast to work
    cli_helpers::run_cmd(
        &home,
        &[
            "transactions",
            "add",
            "XPLG11",
            "buy",
            "100",
            "100.00",
            "2022-01-15",
        ],
    )?;
    cli_helpers::run_cmd(
        &home,
        &[
            "transactions",
            "add",
            "HGLG11",
            "buy",
            "50",
            "150.00",
            "2022-01-15",
        ],
    )?;

    let today = chrono::Local::now().date_naive();
    let current_year = today.year();
    let previous_year = current_year - 1;

    // Add previous year and current year data
    for month in 1..=12 {
        let date_prev = format!("{}-{:02}-15", previous_year, month);
        let date_curr = format!("{}-{:02}-15", current_year, month);
        add_income(&home, "XPLG11", "DIVIDEND", "850", &date_prev)?;
        add_income(&home, "HGLG11", "DIVIDEND", "500", &date_prev)?;
        add_income(&home, "XPLG11", "DIVIDEND", "850", &date_curr)?;
        add_income(&home, "HGLG11", "DIVIDEND", "500", &date_curr)?;
    }

    // Run forecast command for next year
    let next_year = current_year + 1;
    let output = cli_helpers::run_cmd_json(
        &home,
        &["--json", "income", "forecast", &next_year.to_string()],
    )?;

    // Verify structure
    assert!(output.get("blocks").is_some());
    let blocks = output["blocks"].as_array().unwrap();

    // Should have header and table with forecasts
    assert!(blocks.iter().any(|b| b["type"] == "header"));
    assert!(blocks.iter().any(|b| b["type"] == "table"));

    Ok(())
}

#[test]
fn test_income_calendar_command() -> Result<()> {
    let home = TempDir::new()?;

    let today = chrono::Local::now().date_naive();
    let current_year = today.year();
    let current_month = today.month();

    add_asset(&home, "HGLG11", "FII")?;

    // Add income events for the last 3 months
    for month_offset in 0..3 {
        let months_back = 2 - month_offset;
        let (year, month) = if current_month > months_back {
            (current_year, current_month - months_back)
        } else {
            (current_year - 1, 12 + current_month - months_back)
        };
        let date = format!("{}-{:02}-15", year, month);
        add_income(&home, "HGLG11", "DIVIDEND", "100", &date)?;
    }

    // Run calendar command - may return predictions or empty result depending on
    // whether 3 data points are enough for the prediction algorithm
    let output = cli_helpers::run_cmd_json(&home, &["--json", "income", "calendar"])?;

    // Verify it returns valid JSON with blocks array
    assert!(output.get("blocks").is_some());

    Ok(())
}

#[test]
fn test_income_alerts_command() -> Result<()> {
    let home = TempDir::new()?;

    let today = chrono::Local::now().date_naive();
    let current_year = today.year();
    let date = format!("{}-01-15", current_year);

    add_asset(&home, "KNSC11", "FII")?;
    add_income(&home, "KNSC11", "DIVIDEND", "100", &date)?;

    // Run alerts command with JSON (should succeed even with no anomalies)
    let output = cli_helpers::run_cmd_json(&home, &["--json", "income", "alerts"])?;

    assert!(output.get("blocks").is_some());
    let blocks = output["blocks"].as_array().unwrap();
    assert!(blocks.iter().any(|b| b["type"] == "header"));

    Ok(())
}

#[test]
fn test_income_summary_with_categorize() -> Result<()> {
    let home = TempDir::new()?;

    add_asset(&home, "VGHF11", "FII")?;
    add_income(&home, "VGHF11", "DIVIDEND", "100", "2024-01-15")?;
    add_income(&home, "VGHF11", "DIVIDEND", "100", "2024-02-15")?;
    add_income(&home, "VGHF11", "AMORTIZATION", "500", "2024-03-15")?;

    let output = cli_helpers::run_cmd_json(
        &home,
        &["--json", "income", "summary", "2024", "--categorize"],
    )?;

    assert!(output.get("blocks").is_some());
    let blocks = output["blocks"].as_array().unwrap();

    // Should have "Income Categorization" key-value block
    let has_categorization = blocks.iter().any(|b| {
        b["type"] == "key_value"
            && b.get("title").and_then(|t| t.as_str()) == Some("Income Categorization")
    });
    assert!(
        has_categorization,
        "Should have Income Categorization section"
    );

    Ok(())
}

#[test]
fn test_income_summary_with_tax_aware() -> Result<()> {
    let home = TempDir::new()?;

    add_asset(&home, "BRCR11", "FII")?;
    add_income(&home, "BRCR11", "DIVIDEND", "100", "2024-01-15")?;

    let output = cli_helpers::run_cmd_json(
        &home,
        &[
            "--json",
            "income",
            "summary",
            "2024",
            "--categorize",
            "--tax-aware",
        ],
    )?;

    assert!(output.get("blocks").is_some());
    let blocks = output["blocks"].as_array().unwrap();

    // Should have Income Categorization with tax impact row
    let cat_block = blocks.iter().find(|b| {
        b["type"] == "key_value"
            && b.get("title").and_then(|t| t.as_str()) == Some("Income Categorization")
    });
    assert!(cat_block.is_some(), "Should have Income Categorization");

    // With tax-aware, should have "Est. Tax Impact" row
    let rows = cat_block.unwrap()["rows"].as_array().unwrap();
    let has_tax = rows
        .iter()
        .any(|r| r["label"].as_str() == Some("Est. JCP Withholding (15%)"));
    assert!(has_tax, "Should have tax impact row when --tax-aware");

    Ok(())
}

#[test]
fn test_income_export_csv() -> Result<()> {
    let home = TempDir::new()?;

    add_asset(&home, "ALZC11", "FII")?;
    add_income(&home, "ALZC11", "DIVIDEND", "100", "2024-01-15")?;
    add_income(&home, "ALZC11", "DIVIDEND", "105", "2024-06-15")?;

    let output_path = home.path().join("test_export.csv");
    cli_helpers::run_cmd(
        &home,
        &[
            "income",
            "export",
            "2024",
            "--format",
            "csv",
            "-o",
            output_path.to_str().unwrap(),
        ],
    )?;

    assert!(output_path.exists(), "CSV file should be created");

    let content = std::fs::read_to_string(&output_path)?;
    assert!(
        content.contains("Date,Ticker,Asset Type"),
        "Should have CSV header"
    );
    assert!(content.contains("ALZC11"), "Should contain asset ticker");
    assert!(content.contains("DIVIDEND"), "Should contain event type");

    Ok(())
}

#[test]
fn test_income_export_xlsx() -> Result<()> {
    let home = TempDir::new()?;

    add_asset(&home, "JURO11", "FII")?;
    add_income(&home, "JURO11", "DIVIDEND", "150", "2024-01-15")?;

    let output_path = home.path().join("test_export.xlsx");
    cli_helpers::run_cmd(
        &home,
        &[
            "income",
            "export",
            "2024",
            "--format",
            "xlsx",
            "-o",
            output_path.to_str().unwrap(),
        ],
    )?;

    assert!(output_path.exists(), "Excel file should be created");

    let file_content = std::fs::read(&output_path)?;
    assert!(file_content.len() > 100, "Excel file should have content");
    // XLSX files are ZIP archives starting with PK
    assert_eq!(
        &file_content[0..2],
        b"PK",
        "Excel file should be a ZIP archive"
    );

    Ok(())
}
