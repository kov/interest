#![allow(dead_code)]

use anyhow::{bail, Result};
use assert_cmd::cargo;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

pub fn setup_test_tickers_cache(cache_root: &Path) {
    let tickers_dir = cache_root.join("interest").join("tickers");
    std::fs::create_dir_all(&tickers_dir).expect("failed to create tickers cache dir");
    std::fs::copy(
        "tests/fixtures/b3_cache/tickers.csv",
        tickers_dir.join("tickers.csv"),
    )
    .expect("failed to copy tickers.csv fixture");
    std::fs::copy(
        "tests/fixtures/b3_cache/tickers.meta.json",
        tickers_dir.join("tickers.meta.json"),
    )
    .expect("failed to copy tickers.meta.json fixture");
}

pub fn cache_root_for_home(home: &TempDir) -> PathBuf {
    if cfg!(target_os = "macos") {
        home.path().join("Library").join("Caches")
    } else {
        home.path().join(".cache")
    }
}

pub fn base_cmd(home: &TempDir) -> Command {
    let mut cmd = Command::new(cargo::cargo_bin!("interest"));
    cmd.env("HOME", home.path());

    let cache_dir = cache_root_for_home(home);
    setup_test_tickers_cache(&cache_dir);
    cmd.env("XDG_CACHE_HOME", &cache_dir);

    cmd.env("INTEREST_SKIP_PRICE_FETCH", "1");
    cmd.env("INTEREST_OFFLINE", "1");
    cmd.arg("--no-color");
    cmd
}

pub fn run_cmd(home: &TempDir, args: &[&str]) -> Result<Output> {
    let mut cmd = base_cmd(home);
    cmd.args(args);
    let output = cmd.output()?;
    if !output.status.success() {
        bail!(
            "command failed: {:?}\nstdout: {}\nstderr: {}",
            args,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(output)
}

pub fn run_cmd_json(home: &TempDir, args: &[&str]) -> Result<Value> {
    let output = run_cmd(home, args)?;
    let stdout = String::from_utf8(output.stdout)?;
    Ok(serde_json::from_str(&stdout)?)
}

pub fn import_json(home: &TempDir, file_path: &str) -> Result<Value> {
    run_cmd_json(home, &["--json", "import", file_path])
}

pub fn add_asset(home: &TempDir, ticker: &str, asset_type: &str) -> Result<()> {
    run_cmd(home, &["assets", "add", ticker, "--type", asset_type])?;
    Ok(())
}

pub fn add_transaction(
    home: &TempDir,
    ticker: &str,
    tx_type: &str,
    quantity: &str,
    price: &str,
    date: &str,
    day_trade: bool,
) -> Result<()> {
    let mut args = vec![
        "transactions",
        "add",
        ticker,
        tx_type,
        quantity,
        price,
        date,
    ];
    if day_trade {
        args.push("--day-trade");
    }
    run_cmd(home, &args)?;
    Ok(())
}

pub fn add_income(
    home: &TempDir,
    ticker: &str,
    event_type: &str,
    total_amount: &str,
    date: &str,
) -> Result<()> {
    run_cmd(
        home,
        &["income", "add", ticker, event_type, total_amount, date],
    )?;
    Ok(())
}

pub fn list_transactions_json(home: &TempDir, ticker: &str) -> Result<Vec<Value>> {
    let value = run_cmd_json(
        home,
        &["--json", "transactions", "list", "--ticker", ticker],
    )?;
    Ok(value.as_array().cloned().unwrap_or_default())
}

pub fn portfolio_json(home: &TempDir) -> Result<Value> {
    run_cmd_json(home, &["--json", "portfolio", "show"])
}

pub fn income_detail_json(home: &TempDir, year: &str, ticker: &str) -> Result<Vec<Value>> {
    let value = run_cmd_json(
        home,
        &["--json", "income", "detail", year, "--asset", ticker],
    )?;
    Ok(value.as_array().cloned().unwrap_or_default())
}

pub fn tax_report_json(home: &TempDir, year: &str) -> Result<Value> {
    run_cmd_json(home, &["--json", "tax", "report", year])
}
