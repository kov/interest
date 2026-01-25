//! Integration tests for income event functionality (CLI-driven)

use anyhow::{Context, Result};
use rust_decimal::Decimal;
use serde_json::Value;
use tempfile::TempDir;

mod cli_helpers;
use cli_helpers::{add_asset, add_income, import_json, income_detail_json};

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

    let first = import_json(&home, "tests/data/07_capital_return.xlsx")?;
    assert!(first
        .get("success")
        .and_then(|v| v.as_bool())
        .unwrap_or(false));
    let data_first = first.get("data").context("missing data")?;
    assert_eq!(data_first["imported_income"].as_u64().unwrap_or(0), 1);

    let second = import_json(&home, "tests/data/07_capital_return.xlsx")?;
    let data_second = second.get("data").context("missing data")?;
    assert_eq!(data_second["imported_income"].as_u64().unwrap_or(0), 0);
    assert!(data_second["skipped_income_old"].as_u64().unwrap_or(0) >= 1);

    Ok(())
}
