use anyhow::{Context, Result};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde_json::Value;
use tempfile::TempDir;

mod cli_helpers;
use cli_helpers::{add_asset, add_income, add_transaction, base_cmd, run_cmd, tax_report_json};

fn decimal_from_value(value: &Value) -> Result<Decimal> {
    if let Some(s) = value.as_str() {
        return Decimal::from_str_exact(s).context("invalid decimal string");
    }
    if let Some(f) = value.as_f64() {
        return Decimal::try_from(f).context("invalid decimal number");
    }
    Err(anyhow::anyhow!("expected decimal value"))
}

fn add_rename(home: &TempDir, from: &str, to: &str, date: &str) -> Result<()> {
    run_cmd(
        home,
        &["actions", "rename", "add", from, to, date, "--notes", "test rename"],
    )?;
    Ok(())
}

fn add_split(home: &TempDir, ticker: &str, qty: &str, date: &str) -> Result<()> {
    run_cmd(
        home,
        &["actions", "split", "add", ticker, qty, date, "--notes", "test split"],
    )?;
    Ok(())
}

/// Check portfolio position via CLI table output
fn check_portfolio_position(
    home: &TempDir,
    date: &str,
    ticker: &str,
    expected_qty: &str,
    expected_cost: &str,
) -> Result<()> {
    let output = base_cmd(home)
        .arg("portfolio")
        .arg("show")
        .arg("--at")
        .arg(date)
        .output()?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "portfolio show failed at {}: {}",
        date,
        stderr
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let row = stdout
        .lines()
        .find(|l| l.contains(&format!("│ {}", ticker)))
        .context(format!("{} not found in portfolio at {}", ticker, date))?;
    let cols: Vec<String> = row
        .split('│')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    assert_eq!(cols[1], expected_qty, "Quantity mismatch for {} at {}", ticker, date);
    assert_eq!(cols[3], expected_cost, "Total cost mismatch for {} at {}", ticker, date);
    Ok(())
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

/// Sale after rename + post-rename split: primary bug scenario.
/// The bug was double-applying the split to the rename carryover.
#[test]
fn test_rename_sale_after_post_rename_split() -> Result<()> {
    let home = TempDir::new()?;

    add_asset(&home, "JSLG3", "STOCK")?;
    add_asset(&home, "SIMH3", "STOCK")?;

    add_transaction(&home, "JSLG3", "buy", "1700", "10", "2019-12-31", false)?;
    add_transaction(&home, "JSLG3", "sell", "300", "12", "2020-08-01", false)?;
    add_rename(&home, "JSLG3", "SIMH3", "2020-09-21")?;
    add_transaction(&home, "SIMH3", "buy", "2600", "8", "2021-08-01", false)?;
    add_split(&home, "SIMH3", "3000", "2021-08-12")?;
    add_transaction(&home, "SIMH3", "buy", "27500", "7", "2022-01-15", false)?;
    add_transaction(&home, "SIMH3", "sell", "1000", "9", "2023-03-15", false)?;

    // Portfolio check: 34500 - 1000 = 33500 shares remaining
    check_portfolio_position(&home, "2023-03-16", "SIMH3", "33500.00", "R$ 220.711,59")?;

    // Tax check: avg_cost = 227300/34500 ≈ 6.5884
    // cost_basis = avg_cost * 1000, profit = 9000 - cost_basis ≈ 2411.59
    let report = tax_report_json(&home, "2023")?;
    let mar = month_summary(&report, "Março")?;
    let sales = decimal_from_value(&mar["sales"])?;
    let profit = decimal_from_value(&mar["profit"])?;
    assert_eq!(sales, dec!(9000));
    assert!(
        profit > dec!(2411) && profit < dec!(2412),
        "Expected profit ~R$2411.59, got {}",
        profit
    );

    Ok(())
}

/// Sale after rename without corporate actions — baseline correctness.
#[test]
fn test_rename_sale_without_corporate_actions() -> Result<()> {
    let home = TempDir::new()?;

    add_asset(&home, "TKRA", "STOCK")?;
    add_asset(&home, "TKRB", "STOCK")?;

    add_transaction(&home, "TKRA", "buy", "1000", "20", "2020-01-15", false)?;
    add_rename(&home, "TKRA", "TKRB", "2020-06-01")?;
    add_transaction(&home, "TKRB", "sell", "500", "25", "2020-09-15", false)?;

    // Portfolio: 500 shares remain, cost R$10000
    check_portfolio_position(&home, "2020-09-16", "TKRB", "500.00", "R$ 10.000,00")?;

    // Tax: gain = 12500 - 10000 = 2500, but under 20k exemption → tax = 0
    let report = tax_report_json(&home, "2020")?;
    let sep = month_summary(&report, "Setembro")?;
    let sales = decimal_from_value(&sep["sales"])?;
    let profit = decimal_from_value(&sep["profit"])?;
    assert_eq!(sales, dec!(12500));
    assert_eq!(profit, dec!(2500));

    Ok(())
}

/// Source-asset split before rename: the carryover builder adjusts for
/// source-asset corporate actions between each transaction and the rename date.
/// Both portfolio and tax must agree.
#[test]
fn test_rename_source_asset_split_before_rename() -> Result<()> {
    let home = TempDir::new()?;

    add_asset(&home, "TKRC", "STOCK")?;
    add_asset(&home, "TKRD", "STOCK")?;

    add_transaction(&home, "TKRC", "buy", "1000", "40", "2020-01-15", false)?;
    add_split(&home, "TKRC", "1000", "2020-06-01")?;
    // After split: 2000 shares, R$40000, avg R$20
    add_rename(&home, "TKRC", "TKRD", "2020-09-01")?;
    add_transaction(&home, "TKRD", "sell", "500", "25", "2021-03-15", false)?;

    // Carryover: 2000 shares at avg R$20 (split applied to source)
    // Sale of 500: proceeds R$12500, cost_basis R$10000, gain R$2500
    // Remaining: 1500 shares, R$30000

    // Portfolio check
    check_portfolio_position(&home, "2021-03-16", "TKRD", "1500.00", "R$ 30.000,00")?;

    // Tax check
    let report = tax_report_json(&home, "2021")?;
    let mar = month_summary(&report, "Março")?;
    let sales = decimal_from_value(&mar["sales"])?;
    let profit = decimal_from_value(&mar["profit"])?;
    assert_eq!(sales, dec!(12500));
    assert_eq!(profit, dec!(2500));

    Ok(())
}

/// Source-asset amortization before rename: cost reduction must carry over.
/// Both portfolio and tax must agree.
#[test]
fn test_rename_source_asset_amortization_before_rename() -> Result<()> {
    let home = TempDir::new()?;

    add_asset(&home, "FIIOLD", "FII")?;
    add_asset(&home, "FIINEW", "FII")?;

    add_transaction(&home, "FIIOLD", "buy", "1000", "100", "2020-01-15", false)?;
    // Amortization reduces cost by R$5000 → cost becomes R$95000
    add_income(&home, "FIIOLD", "amortization", "5000", "2020-06-01")?;
    add_rename(&home, "FIIOLD", "FIINEW", "2020-09-01")?;
    add_transaction(&home, "FIINEW", "sell", "500", "110", "2021-03-15", false)?;

    // Carryover: 1000 shares, cost R$95000, avg R$95
    // Sale of 500: proceeds R$55000, cost_basis R$47500, gain R$7500
    // Remaining: 500 shares, R$47500

    // Portfolio check
    check_portfolio_position(&home, "2021-03-16", "FIINEW", "500.00", "R$ 47.500,00")?;

    // Tax check (FII swing trade: 20%, no exemption)
    let report = tax_report_json(&home, "2021")?;
    let mar = month_summary(&report, "Março")?;
    let sales = decimal_from_value(&mar["sales"])?;
    let profit = decimal_from_value(&mar["profit"])?;
    let tax_due = decimal_from_value(&mar["tax_due"])?;
    assert_eq!(sales, dec!(55000));
    assert_eq!(profit, dec!(7500));
    assert_eq!(tax_due, dec!(1500)); // 20% of 7500

    Ok(())
}
