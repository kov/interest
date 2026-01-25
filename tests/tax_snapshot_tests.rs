use anyhow::{Context, Result};
use rust_decimal::Decimal;
use tempfile::TempDir;

mod cli_helpers;
use cli_helpers::{add_asset, add_transaction, tax_report_json};

fn decimal_from_value(value: &serde_json::Value) -> Result<Decimal> {
    if let Some(s) = value.as_str() {
        return Decimal::from_str_exact(s).context("invalid decimal string");
    }
    if let Some(f) = value.as_f64() {
        return Decimal::try_from(f).context("invalid decimal number");
    }
    Err(anyhow::anyhow!("expected decimal value"))
}

fn annual_totals(report: &serde_json::Value) -> Result<(Decimal, Decimal, Decimal)> {
    let sales = decimal_from_value(&report["annual_total_sales"])?;
    let profit = decimal_from_value(&report["annual_total_profit"])?;
    let tax = decimal_from_value(&report["annual_total_tax"])?;
    Ok((sales, profit, tax))
}

#[test]
fn test_tax_report_is_stable_on_repeat() -> Result<()> {
    let home = TempDir::new()?;

    add_asset(&home, "PETR4", "STOCK")?;
    add_transaction(&home, "PETR4", "buy", "100", "10", "2025-01-10", false)?;
    add_transaction(&home, "PETR4", "sell", "50", "12", "2025-01-20", false)?;

    let first = tax_report_json(&home, "2025")?;
    let second = tax_report_json(&home, "2025")?;

    assert_eq!(first, second, "report should be deterministic across runs");

    Ok(())
}

#[test]
fn test_tax_report_changes_after_new_trade() -> Result<()> {
    let home = TempDir::new()?;

    add_asset(&home, "VALE3", "STOCK")?;
    add_transaction(&home, "VALE3", "buy", "100", "80", "2025-01-05", false)?;
    add_transaction(&home, "VALE3", "sell", "50", "90", "2025-01-20", false)?;

    let before = tax_report_json(&home, "2025")?;
    let before_totals = annual_totals(&before)?;

    add_transaction(&home, "VALE3", "sell", "10", "95", "2025-02-01", false)?;

    let after = tax_report_json(&home, "2025")?;
    let after_totals = annual_totals(&after)?;

    assert_ne!(
        before_totals, after_totals,
        "totals should change after new trade"
    );

    Ok(())
}

#[test]
fn test_tax_report_order_independent() -> Result<()> {
    let home_a = TempDir::new()?;
    let home_b = TempDir::new()?;

    add_asset(&home_a, "AMER3", "STOCK")?;
    add_transaction(&home_a, "AMER3", "buy", "100", "10", "2025-03-01", false)?;
    add_transaction(&home_a, "AMER3", "sell", "50", "12", "2025-03-15", false)?;

    add_asset(&home_b, "AMER3", "STOCK")?;
    add_transaction(&home_b, "AMER3", "sell", "50", "12", "2025-03-15", false)?;
    add_transaction(&home_b, "AMER3", "buy", "100", "10", "2025-03-01", false)?;

    let report_a = tax_report_json(&home_a, "2025")?;
    let report_b = tax_report_json(&home_b, "2025")?;

    assert_eq!(annual_totals(&report_a)?, annual_totals(&report_b)?);

    Ok(())
}
