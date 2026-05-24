#![allow(dead_code)]

use anyhow::{bail, Context, Result};
use assert_cmd::cargo;
use rust_decimal::Decimal;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::str::FromStr;
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

pub fn import_stats_json(home: &TempDir, file_path: &str) -> Result<Value> {
    let doc = import_json(home, file_path)?;
    Ok(import_stats_from_doc(&doc))
}

pub fn import_stats_from_doc(doc: &Value) -> Value {
    let mut map = serde_json::Map::new();
    let trade = find_key_value_block(doc, "Trades");
    let actions = find_key_value_block(doc, "Corporate actions");
    let income = find_key_value_block(doc, "Income events");

    insert_kv_number(&mut map, "imported_trades", trade, "Imported");
    insert_kv_number(&mut map, "skipped_trades", trade, "Skipped");
    insert_kv_number(
        &mut map,
        "skipped_trades_old",
        trade,
        "Skipped (before last import date)",
    );

    insert_kv_number(&mut map, "imported_actions", actions, "Imported");
    insert_kv_number(&mut map, "skipped_actions", actions, "Skipped");
    insert_kv_number(
        &mut map,
        "skipped_actions_old",
        actions,
        "Skipped (before last import date)",
    );
    insert_kv_number(&mut map, "auto_applied_actions", actions, "Auto-applied");

    insert_kv_number(&mut map, "imported_income", income, "Imported");
    insert_kv_number(&mut map, "skipped_income", income, "Skipped");
    insert_kv_number(
        &mut map,
        "skipped_income_old",
        income,
        "Skipped (before last import date)",
    );

    Value::Object(map)
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
    let rows = extract_table_rows(&value, None, Some("trade_date"))?;
    Ok(rows
        .into_iter()
        .map(|row| remap_transactions_row(&row))
        .collect())
}

pub fn portfolio_json(home: &TempDir) -> Result<Value> {
    let doc = run_cmd_json(home, &["--json", "portfolio", "show"])?;
    let rows = extract_table_rows(&doc, None, Some("avg_cost"))?;
    let positions: Vec<Value> = rows.into_iter().collect();
    Ok(serde_json::json!({ "positions": positions }))
}

pub fn income_detail_json(home: &TempDir, year: &str, ticker: &str) -> Result<Vec<Value>> {
    let value = run_cmd_json(
        home,
        &["--json", "income", "detail", year, "--asset", ticker],
    )?;
    let rows = extract_table_rows(&value, Some("Income Detail"), None)?;
    Ok(rows
        .into_iter()
        .map(|row| remap_income_detail_row(&row))
        .collect())
}

pub fn tax_report_json(home: &TempDir, year: &str) -> Result<Value> {
    let doc = run_cmd_json(home, &["--json", "tax", "report", year])?;
    let monthly = aggregate_monthly_summary_rows(&doc)?;
    let annual = find_key_value_block(&doc, "Annual Totals (All Categories)");

    let report = serde_json::json!({
        "monthly_summaries": monthly,
        "annual_total_sales": kv_value(annual, "Total Sales"),
        "annual_total_profit": kv_value(annual, "Total Profit"),
        "annual_total_loss": kv_value(annual, "Total Loss"),
        "annual_total_tax": kv_value(annual, "Total Tax"),
    });
    Ok(report)
}

const PT_BR_MONTH_NAMES: [&str; 12] = [
    "Janeiro",
    "Fevereiro",
    "Março",
    "Abril",
    "Maio",
    "Junho",
    "Julho",
    "Agosto",
    "Setembro",
    "Outubro",
    "Novembro",
    "Dezembro",
];

/// Walk every "Monthly Summary — ..." per-category table and sum the per-month
/// values back into a single per-month view shaped like the pre-split helper.
pub fn aggregate_monthly_summary_rows(doc: &Value) -> Result<Vec<Value>> {
    let blocks = doc
        .get("blocks")
        .and_then(|b| b.as_array())
        .context("blocks missing")?;
    let mut tables = Vec::new();
    collect_table_blocks(blocks, &mut tables);

    let month_to_index: std::collections::HashMap<&str, usize> = PT_BR_MONTH_NAMES
        .iter()
        .enumerate()
        .map(|(i, m)| (*m, i))
        .collect();

    // [sales, profit, loss, tax_due] per month index
    let mut aggregates: BTreeMap<usize, [Decimal; 4]> = BTreeMap::new();

    for table in tables {
        let title = table.get("title").and_then(|t| t.as_str()).unwrap_or("");
        if !title.starts_with("Monthly Summary") {
            continue;
        }
        let columns: Vec<&str> = table
            .get("columns")
            .and_then(|c| c.as_array())
            .map(|cs| {
                cs.iter()
                    .filter_map(|c| c.get("key").and_then(|v| v.as_str()))
                    .collect()
            })
            .unwrap_or_default();
        let rows = table
            .get("rows")
            .and_then(|r| r.as_array())
            .cloned()
            .unwrap_or_default();

        for row in &rows {
            let cells = row
                .get("cells")
                .and_then(|c| c.as_array())
                .cloned()
                .unwrap_or_default();
            let cell_value = |key: &str| -> Option<Value> {
                let idx = columns.iter().position(|k| *k == key)?;
                cells.get(idx).and_then(|c| c.get("value")).cloned()
            };
            let month_str = cell_value("month")
                .as_ref()
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_default();
            let Some(&idx) = month_to_index.get(month_str.as_str()) else {
                continue;
            };
            let entry = aggregates.entry(idx).or_insert([Decimal::ZERO; 4]);
            for (i, key) in ["sales", "profit", "loss", "tax_due"].iter().enumerate() {
                let Some(v) = cell_value(key) else { continue };
                let Some(s) = v.as_str() else { continue };
                if let Ok(d) = Decimal::from_str(s) {
                    entry[i] += d;
                }
            }
        }
    }

    Ok(aggregates
        .into_iter()
        .map(|(idx, [sales, profit, loss, tax_due])| {
            serde_json::json!({
                "month": PT_BR_MONTH_NAMES[idx],
                "sales": sales.to_string(),
                "profit": profit.to_string(),
                "loss": loss.to_string(),
                "tax_due": tax_due.to_string(),
            })
        })
        .collect())
}

pub fn list_split_actions_json(home: &TempDir, ticker: Option<&str>) -> Result<Vec<Value>> {
    let mut args = vec!["--json", "actions", "split", "list"];
    if let Some(ticker) = ticker {
        args.push(ticker);
    }
    let doc = run_cmd_json(home, &args)?;
    extract_table_rows(&doc, None, Some("ex_date"))
}

pub fn extract_table_rows(
    doc: &Value,
    title: Option<&str>,
    column_key: Option<&str>,
) -> Result<Vec<Value>> {
    let blocks = doc
        .get("blocks")
        .and_then(|b| b.as_array())
        .context("blocks missing")?;
    let mut tables = Vec::new();
    collect_table_blocks(blocks, &mut tables);

    let table = if let Some(title) = title {
        tables.into_iter().find(|block| {
            block
                .get("title")
                .and_then(|t| t.as_str())
                .map(|t| t == title)
                .unwrap_or(false)
        })
    } else if let Some(key) = column_key {
        tables.into_iter().find(|block| {
            block
                .get("columns")
                .and_then(|c| c.as_array())
                .map(|cols| {
                    cols.iter()
                        .any(|col| col.get("key").and_then(|v| v.as_str()) == Some(key))
                })
                .unwrap_or(false)
        })
    } else {
        tables.into_iter().next()
    };

    let table = table.context("table block not found")?;
    let columns = table
        .get("columns")
        .and_then(|c| c.as_array())
        .context("columns missing")?;
    let keys: Vec<&str> = columns
        .iter()
        .filter_map(|col| col.get("key").and_then(|v| v.as_str()))
        .collect();

    let mut rows_out = Vec::new();
    if let Some(rows) = table.get("rows").and_then(|r| r.as_array()) {
        for row in rows {
            let Some(cells) = row.get("cells").and_then(|c| c.as_array()) else {
                continue;
            };
            let mut map = serde_json::Map::new();
            for (idx, key) in keys.iter().enumerate() {
                let cell = cells.get(idx);
                let value = cell
                    .and_then(|c| c.get("value"))
                    .cloned()
                    .unwrap_or(Value::Null);
                map.insert((*key).to_string(), value);
            }
            rows_out.push(Value::Object(map));
        }
    }
    Ok(rows_out)
}

fn collect_table_blocks<'a>(blocks: &'a [Value], tables: &mut Vec<&'a Value>) {
    for block in blocks {
        match block.get("type").and_then(|t| t.as_str()) {
            Some("table") => tables.push(block),
            Some("section") => {
                if let Some(nested) = block.get("blocks").and_then(|b| b.as_array()) {
                    collect_table_blocks(nested, tables);
                }
            }
            _ => {}
        }
    }
}

pub fn find_key_value_block<'a>(doc: &'a Value, title: &str) -> Option<&'a Value> {
    let blocks = doc.get("blocks")?.as_array()?;
    find_key_value_block_in_blocks(blocks, title)
}

fn find_key_value_block_in_blocks<'a>(blocks: &'a [Value], title: &str) -> Option<&'a Value> {
    for block in blocks {
        match block.get("type").and_then(|t| t.as_str()) {
            Some("key_value") => {
                if block
                    .get("title")
                    .and_then(|t| t.as_str())
                    .map(|t| t == title)
                    .unwrap_or(false)
                {
                    return Some(block);
                }
            }
            Some("section") => {
                if let Some(nested) = block.get("blocks").and_then(|b| b.as_array()) {
                    if let Some(found) = find_key_value_block_in_blocks(nested, title) {
                        return Some(found);
                    }
                }
            }
            _ => {}
        }
    }
    None
}

pub fn kv_value(block: Option<&Value>, label: &str) -> Value {
    if let Some(block) = block {
        if let Some(rows) = block.get("rows").and_then(|r| r.as_array()) {
            for row in rows {
                if row.get("label").and_then(|l| l.as_str()) == Some(label) {
                    return row
                        .get("value")
                        .and_then(|v| v.get("value"))
                        .cloned()
                        .unwrap_or(Value::Null);
                }
            }
        }
    }
    Value::Null
}

fn insert_kv_number(
    map: &mut serde_json::Map<String, Value>,
    key: &str,
    block: Option<&Value>,
    label: &str,
) {
    let value = kv_value(block, label);
    let number = if let Some(s) = value.as_str() {
        s.parse::<u64>().ok()
    } else {
        value.as_u64()
    };
    map.insert(key.to_string(), Value::Number(number.unwrap_or(0).into()));
}

fn remap_transactions_row(row: &Value) -> Value {
    let mut map = serde_json::Map::new();
    map.insert(
        "trade_date".to_string(),
        row.get("trade_date").cloned().unwrap_or(Value::Null),
    );
    map.insert(
        "transaction_type".to_string(),
        row.get("type").cloned().unwrap_or(Value::Null),
    );
    map.insert(
        "quantity".to_string(),
        row.get("quantity").cloned().unwrap_or(Value::Null),
    );
    map.insert(
        "price_per_unit".to_string(),
        row.get("price_per_unit").cloned().unwrap_or(Value::Null),
    );
    map.insert(
        "total_cost".to_string(),
        row.get("total_cost").cloned().unwrap_or(Value::Null),
    );
    map.insert(
        "notes".to_string(),
        row.get("notes").cloned().unwrap_or(Value::Null),
    );
    let day_trade = row
        .get("day_trade")
        .and_then(|v| v.as_str())
        .and_then(|v| v.parse::<bool>().ok())
        .unwrap_or(false);
    map.insert("is_day_trade".to_string(), Value::Bool(day_trade));
    Value::Object(map)
}

fn remap_income_detail_row(row: &Value) -> Value {
    let mut map = serde_json::Map::new();
    map.insert(
        "date".to_string(),
        row.get("date").cloned().unwrap_or(Value::Null),
    );
    map.insert(
        "ticker".to_string(),
        row.get("ticker").cloned().unwrap_or(Value::Null),
    );
    map.insert(
        "event_type".to_string(),
        row.get("type").cloned().unwrap_or(Value::Null),
    );
    map.insert(
        "asset_type".to_string(),
        row.get("asset_type").cloned().unwrap_or(Value::Null),
    );
    map.insert(
        "amount".to_string(),
        row.get("amount").cloned().unwrap_or(Value::Null),
    );
    map.insert(
        "withholding_tax".to_string(),
        row.get("tax").cloned().unwrap_or(Value::Null),
    );
    map.insert(
        "net_amount".to_string(),
        row.get("net").cloned().unwrap_or(Value::Null),
    );
    map.insert(
        "notes".to_string(),
        row.get("notes").cloned().unwrap_or(Value::Null),
    );
    Value::Object(map)
}
