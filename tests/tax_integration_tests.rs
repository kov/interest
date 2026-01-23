use anyhow::{Context, Result};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde_json::Value;
use tempfile::TempDir;

mod cli_helpers;
use cli_helpers::{add_asset, add_transaction, tax_report_json};

fn decimal_from_value(value: &Value) -> Result<Decimal> {
    if let Some(s) = value.as_str() {
        return Decimal::from_str_exact(s).context("invalid decimal string");
    }
    if let Some(f) = value.as_f64() {
        return Decimal::try_from(f).context("invalid decimal number");
    }
    Err(anyhow::anyhow!("expected decimal value"))
}

fn month_summary<'a>(report: &'a Value, month_name: &str) -> Result<&'a Value> {
    let summaries = report
        .get("monthly_summaries")
        .and_then(|v| v.as_array())
        .context("monthly_summaries missing")?;

    summaries
        .iter()
        .find(|s| s.get("month").and_then(|v| v.as_str()) == Some(month_name))
        .context("month summary not found")
}

#[test]
fn test_stock_swing_trade_under_exemption() -> Result<()> {
    let home = TempDir::new()?;

    add_asset(&home, "PETR4", "STOCK")?;
    add_transaction(&home, "PETR4", "buy", "100", "25", "2025-01-05", false)?;
    add_transaction(&home, "PETR4", "sell", "50", "30", "2025-01-20", false)?;

    let report = tax_report_json(&home, "2025")?;
    let jan = month_summary(&report, "Janeiro")?;

    let sales = decimal_from_value(&jan["sales"])?;
    let profit = decimal_from_value(&jan["profit"])?;
    let tax_due = decimal_from_value(&jan["tax_due"])?;

    assert_eq!(sales, dec!(1500.00));
    assert_eq!(profit, dec!(250.00));
    assert_eq!(tax_due, dec!(0));

    Ok(())
}

#[test]
fn test_stock_swing_trade_over_exemption() -> Result<()> {
    let home = TempDir::new()?;

    add_asset(&home, "VALE3", "STOCK")?;
    add_transaction(&home, "VALE3", "buy", "1000", "50", "2025-02-01", false)?;
    add_transaction(&home, "VALE3", "sell", "500", "60", "2025-02-15", false)?;

    let report = tax_report_json(&home, "2025")?;
    let feb = month_summary(&report, "Fevereiro")?;

    let sales = decimal_from_value(&feb["sales"])?;
    let profit = decimal_from_value(&feb["profit"])?;
    let tax_due = decimal_from_value(&feb["tax_due"])?;

    assert_eq!(sales, dec!(30000.00));
    assert_eq!(profit, dec!(5000.00));
    assert_eq!(tax_due, dec!(750.00));

    Ok(())
}

#[test]
fn test_stock_day_trade_always_taxable() -> Result<()> {
    let home = TempDir::new()?;

    add_asset(&home, "MGLU3", "STOCK")?;
    add_transaction(&home, "MGLU3", "buy", "100", "10", "2025-03-10", true)?;
    add_transaction(&home, "MGLU3", "sell", "100", "12", "2025-03-10", true)?;

    let report = tax_report_json(&home, "2025")?;
    let mar = month_summary(&report, "Março")?;

    let sales = decimal_from_value(&mar["sales"])?;
    let profit = decimal_from_value(&mar["profit"])?;
    let tax_due = decimal_from_value(&mar["tax_due"])?;

    assert_eq!(sales, dec!(1200.00));
    assert_eq!(profit, dec!(200.00));
    assert_eq!(tax_due, dec!(40.00));

    Ok(())
}

#[test]
fn test_fii_always_taxable_20_percent() -> Result<()> {
    let home = TempDir::new()?;

    add_asset(&home, "MXRF11", "FII")?;
    add_transaction(&home, "MXRF11", "buy", "100", "10", "2025-04-01", false)?;
    add_transaction(&home, "MXRF11", "sell", "50", "12", "2025-04-15", false)?;

    let report = tax_report_json(&home, "2025")?;
    let apr = month_summary(&report, "Abril")?;

    let sales = decimal_from_value(&apr["sales"])?;
    let profit = decimal_from_value(&apr["profit"])?;
    let tax_due = decimal_from_value(&apr["tax_due"])?;

    assert_eq!(sales, dec!(600.00));
    assert_eq!(profit, dec!(100.00));
    assert_eq!(tax_due, dec!(20.00));

    Ok(())
}
