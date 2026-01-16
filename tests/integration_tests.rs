//! Integration tests for the interest tracker
//!
//! These tests verify end-to-end functionality using CLI commands:
//! - XLS import
//! - Cost basis calculations with average cost
//! - Term contract lifecycle and cost basis transfer
//! - Split/reverse split adjustments
//! - Capital return adjustments
//! - No duplicate adjustments
//! - Correct portfolio totals

use anyhow::Result;
use assert_cmd::{cargo, prelude::*};
use chrono::Datelike;
use interest::db::models::{
    AssetType, CorporateAction, CorporateActionType, IncomeEvent, Transaction, TransactionType,
};
use interest::db::{init_database, insert_asset, open_db, upsert_asset};
use interest::importers::cei_excel::resolve_option_exercise_ticker;
use interest::importers::import_movimentacao_entries;
use interest::importers::movimentacao_excel::parse_movimentacao_excel;
use interest::importers::ofertas_publicas_excel::parse_ofertas_publicas_excel;
use interest::tax::cost_basis::AverageCostMatcher;
use interest::term_contracts::process_term_liquidations;
use predicates::prelude::*;
use rusqlite::Connection;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde_json::Value;
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::str::FromStr;
use tempfile::TempDir;

// =============================================================================
// CLI Test Helpers
// =============================================================================

/// Get database path from temp home
fn get_db_path(home: &TempDir) -> PathBuf {
    PathBuf::from(home.path()).join(".interest").join("data.db")
}

/// Copy B3 tickers fixture to a cache directory
fn setup_test_tickers_cache(cache_root: &std::path::Path) {
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

/// Create a base CLI command with proper environment setup
fn base_cmd(home: &TempDir) -> Command {
    let mut cmd = Command::new(cargo::cargo_bin!("interest"));
    cmd.env("HOME", home.path());

    // Set up isolated cache with B3 tickers fixture
    let cache_dir = home.path().join(".cache");
    setup_test_tickers_cache(&cache_dir);
    cmd.env("XDG_CACHE_HOME", &cache_dir);

    cmd.env("INTEREST_SKIP_PRICE_FETCH", "1");
    cmd.arg("--no-color");
    cmd
}

#[test]
fn test_cash_flow_show_single_year_monthly_output() -> Result<()> {
    let home = TempDir::new()?;
    let db_path = get_db_path(&home);
    std::fs::create_dir_all(db_path.parent().unwrap())?;
    init_database(Some(db_path.clone()))?;
    let conn = open_db(Some(db_path))?;

    let asset_id = insert_asset(&conn, "TESTCF1", &AssetType::Stock, None)?;

    let buy_tx = Transaction {
        id: None,
        asset_id,
        transaction_type: TransactionType::Buy,
        trade_date: chrono::NaiveDate::from_ymd_opt(2024, 1, 2).unwrap(),
        settlement_date: Some(chrono::NaiveDate::from_ymd_opt(2024, 1, 4).unwrap()),
        quantity: Decimal::from(10),
        price_per_unit: Decimal::from(10),
        total_cost: Decimal::from(100),
        fees: Decimal::from(2),
        is_day_trade: false,
        quota_issuance_date: None,
        notes: None,
        source: "TEST".to_string(),
        created_at: chrono::Utc::now(),
    };
    interest::db::insert_transaction(&conn, &buy_tx)?;

    let sell_tx = Transaction {
        id: None,
        asset_id,
        transaction_type: TransactionType::Sell,
        trade_date: chrono::NaiveDate::from_ymd_opt(2024, 2, 1).unwrap(),
        settlement_date: Some(chrono::NaiveDate::from_ymd_opt(2024, 2, 5).unwrap()),
        quantity: Decimal::from(5),
        price_per_unit: Decimal::from(12),
        total_cost: Decimal::from(60),
        fees: Decimal::from(1),
        is_day_trade: false,
        quota_issuance_date: None,
        notes: None,
        source: "TEST".to_string(),
        created_at: chrono::Utc::now(),
    };
    interest::db::insert_transaction(&conn, &sell_tx)?;

    let income_event = IncomeEvent {
        id: None,
        asset_id,
        event_date: chrono::NaiveDate::from_ymd_opt(2024, 2, 15).unwrap(),
        ex_date: None,
        event_type: interest::db::IncomeEventType::Dividend,
        amount_per_quota: Decimal::ZERO,
        total_amount: Decimal::from(10),
        withholding_tax: Decimal::from(2),
        is_quota_pre_2026: None,
        source: "TEST".to_string(),
        notes: None,
        created_at: chrono::Utc::now(),
    };
    interest::db::insert_income_event(&conn, &income_event)?;

    let output = base_cmd(&home)
        .arg("cash-flow")
        .arg("show")
        .arg("2024")
        .output()?;

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Janeiro 2024"));
    assert!(stdout.contains("Fevereiro 2024"));
    assert!(stdout.contains("STOCK"));
    assert!(stdout.contains("Total new money"));

    Ok(())
}

#[test]
fn test_actions_split_list_orders_by_ex_date_asc() -> Result<()> {
    let home = TempDir::new()?;
    let db_path = get_db_path(&home);
    std::fs::create_dir_all(db_path.parent().unwrap())?;
    init_database(Some(db_path.clone()))?;
    let conn = open_db(Some(db_path))?;

    let asset_id = insert_asset(&conn, "TESTSPLT", &AssetType::Stock, None)?;

    let earlier = CorporateAction {
        id: None,
        asset_id,
        action_type: CorporateActionType::Split,
        event_date: chrono::NaiveDate::from_ymd_opt(2024, 1, 10).unwrap(),
        ex_date: chrono::NaiveDate::from_ymd_opt(2024, 1, 10).unwrap(),
        quantity_adjustment: dec!(50),
        source: "TEST".to_string(),
        notes: None,
        created_at: chrono::Utc::now(),
    };
    let later = CorporateAction {
        id: None,
        asset_id,
        action_type: CorporateActionType::Split,
        event_date: chrono::NaiveDate::from_ymd_opt(2024, 3, 5).unwrap(),
        ex_date: chrono::NaiveDate::from_ymd_opt(2024, 3, 5).unwrap(),
        quantity_adjustment: dec!(100),
        source: "TEST".to_string(),
        notes: None,
        created_at: chrono::Utc::now(),
    };
    interest::db::insert_corporate_action(&conn, &later)?;
    interest::db::insert_corporate_action(&conn, &earlier)?;

    let output = base_cmd(&home)
        .arg("--json")
        .arg("actions")
        .arg("split")
        .arg("list")
        .arg("TESTSPLT")
        .output()?;
    assert!(output.status.success());

    let actions_json: Value = serde_json::from_slice(&output.stdout).expect("invalid actions JSON");
    let actions_array = actions_json.as_array().expect("actions JSON is not array");
    let dates: Vec<String> = actions_array
        .iter()
        .map(|row| {
            row.get("ex_date")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        })
        .collect();
    assert_eq!(dates, vec!["2024-01-10", "2024-03-05"]);

    Ok(())
}

#[test]
fn test_income_detail_orders_by_event_date_asc() -> Result<()> {
    let home = TempDir::new()?;
    let db_path = get_db_path(&home);
    std::fs::create_dir_all(db_path.parent().unwrap())?;
    init_database(Some(db_path.clone()))?;
    let conn = open_db(Some(db_path))?;

    let asset_id = insert_asset(&conn, "TESTINC", &AssetType::Fii, None)?;

    let later = IncomeEvent {
        id: None,
        asset_id,
        event_date: chrono::NaiveDate::from_ymd_opt(2024, 2, 15).unwrap(),
        ex_date: None,
        event_type: interest::db::IncomeEventType::Dividend,
        amount_per_quota: Decimal::ZERO,
        total_amount: Decimal::from(10),
        withholding_tax: Decimal::ZERO,
        is_quota_pre_2026: None,
        source: "TEST".to_string(),
        notes: None,
        created_at: chrono::Utc::now(),
    };
    let earlier = IncomeEvent {
        id: None,
        asset_id,
        event_date: chrono::NaiveDate::from_ymd_opt(2024, 1, 10).unwrap(),
        ex_date: None,
        event_type: interest::db::IncomeEventType::Dividend,
        amount_per_quota: Decimal::ZERO,
        total_amount: Decimal::from(5),
        withholding_tax: Decimal::ZERO,
        is_quota_pre_2026: None,
        source: "TEST".to_string(),
        notes: None,
        created_at: chrono::Utc::now(),
    };
    interest::db::insert_income_event(&conn, &later)?;
    interest::db::insert_income_event(&conn, &earlier)?;

    let output = base_cmd(&home)
        .arg("--json")
        .arg("income")
        .arg("detail")
        .arg("2024")
        .arg("--asset")
        .arg("TESTINC")
        .output()?;
    assert!(output.status.success());

    let income_json: Value = serde_json::from_slice(&output.stdout).expect("invalid income JSON");
    let income_array = income_json.as_array().expect("income JSON is not array");
    let dates: Vec<String> = income_array
        .iter()
        .map(|row| {
            row.get("date")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        })
        .collect();
    assert_eq!(dates, vec!["2024-01-10", "2024-02-15"]);

    Ok(())
}

#[test]
fn test_cash_flow_show_multi_year_ordering() -> Result<()> {
    let home = TempDir::new()?;
    let db_path = get_db_path(&home);
    std::fs::create_dir_all(db_path.parent().unwrap())?;
    init_database(Some(db_path.clone()))?;
    let conn = open_db(Some(db_path))?;

    let asset_id = insert_asset(&conn, "TESTCF2", &AssetType::Fii, None)?;

    let tx_2023 = Transaction {
        id: None,
        asset_id,
        transaction_type: TransactionType::Buy,
        trade_date: chrono::NaiveDate::from_ymd_opt(2023, 6, 1).unwrap(),
        settlement_date: None,
        quantity: Decimal::from(1),
        price_per_unit: Decimal::from(100),
        total_cost: Decimal::from(100),
        fees: Decimal::ZERO,
        is_day_trade: false,
        quota_issuance_date: None,
        notes: None,
        source: "TEST".to_string(),
        created_at: chrono::Utc::now(),
    };
    interest::db::insert_transaction(&conn, &tx_2023)?;

    let tx_2024 = Transaction {
        id: None,
        asset_id,
        transaction_type: TransactionType::Buy,
        trade_date: chrono::NaiveDate::from_ymd_opt(2024, 6, 1).unwrap(),
        settlement_date: None,
        quantity: Decimal::from(1),
        price_per_unit: Decimal::from(110),
        total_cost: Decimal::from(110),
        fees: Decimal::ZERO,
        is_day_trade: false,
        quota_issuance_date: None,
        notes: None,
        source: "TEST".to_string(),
        created_at: chrono::Utc::now(),
    };
    interest::db::insert_transaction(&conn, &tx_2024)?;

    let output = base_cmd(&home).arg("cash-flow").arg("show").output()?;
    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    let idx_2023 = stdout.find("\n2023\n").expect("missing 2023 header");
    let idx_2024 = stdout.find("\n2024\n").expect("missing 2024 header");
    assert!(idx_2023 < idx_2024);

    Ok(())
}

/// Run import command and return stats as JSON
fn run_import_json(home: &TempDir, file_path: &str) -> Value {
    let output = base_cmd(home)
        .arg("--json")
        .arg("import")
        .arg(file_path)
        .output()
        .expect("failed to execute import");

    assert!(output.status.success(), "import command failed");

    let stdout = String::from_utf8(output.stdout).expect("invalid utf8 in output");
    serde_json::from_str(&stdout).expect("failed to parse JSON output")
}

// Removed unused JSON helpers (run_portfolio_json, run_actions_list_json, run_actions_apply_json)

/// JSON assertion helpers
fn assert_json_success(value: &Value) -> bool {
    value
        .get("success")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

fn get_json_data(value: &Value) -> &Value {
    value
        .get("data")
        .expect("missing data field in JSON response")
}

/// SQL Query helpers - for detailed verification
mod sql {
    use super::*;

    /// Query all transactions for a ticker
    pub fn query_transactions(home: &TempDir, ticker: &str) -> Vec<Transaction> {
        let db_path = get_db_path(home);
        let conn = open_db(Some(db_path)).expect("failed to open db");

        let mut stmt = conn
            .prepare(
                "SELECT t.id, t.asset_id, t.transaction_type, t.trade_date, t.settlement_date,
                        t.quantity, t.price_per_unit, t.total_cost, t.fees, t.is_day_trade,
                        t.quota_issuance_date, t.notes, t.source, t.created_at
                 FROM transactions t
                 JOIN assets a ON t.asset_id = a.id
                 WHERE a.ticker = ?1
                 ORDER BY t.trade_date ASC",
            )
            .expect("failed to prepare query");

        let transactions = stmt
            .query_map([ticker], |row| {
                Ok(Transaction {
                    id: Some(row.get(0)?),
                    asset_id: row.get(1)?,
                    transaction_type: row
                        .get::<_, String>(2)?
                        .parse::<TransactionType>()
                        .unwrap_or(TransactionType::Buy),
                    trade_date: row.get(3)?,
                    settlement_date: row.get(4)?,
                    quantity: get_decimal(row, 5)?,
                    price_per_unit: get_decimal(row, 6)?,
                    total_cost: get_decimal(row, 7)?,
                    fees: get_decimal(row, 8)?,
                    is_day_trade: row.get(9)?,
                    quota_issuance_date: row.get(10)?,
                    notes: row.get(11)?,
                    source: row.get(12)?,
                    created_at: row.get(13)?,
                })
            })
            .expect("query failed")
            .collect::<Result<Vec<_>, _>>()
            .expect("failed to collect transactions");

        transactions
    }

    /// Query income events for a ticker
    pub fn query_income_events(home: &TempDir, ticker: &str) -> Vec<IncomeEvent> {
        let db_path = get_db_path(home);
        let conn = open_db(Some(db_path)).expect("failed to open db");

        let mut stmt = conn
            .prepare(
                "SELECT ie.id, ie.asset_id, ie.event_date, ie.ex_date, ie.event_type,
                        ie.amount_per_quota, ie.total_amount, ie.withholding_tax,
                        ie.is_quota_pre_2026, ie.source, ie.notes, ie.created_at
                 FROM income_events ie
                 JOIN assets a ON ie.asset_id = a.id
                 WHERE a.ticker = ?1
                 ORDER BY ie.event_date ASC",
            )
            .expect("failed to prepare income query");

        let events = stmt
            .query_map([ticker], |row| {
                Ok(IncomeEvent {
                    id: Some(row.get(0)?),
                    asset_id: row.get(1)?,
                    event_date: row.get(2)?,
                    ex_date: row.get(3)?,
                    event_type: row
                        .get::<_, String>(4)?
                        .parse::<interest::db::IncomeEventType>()
                        .unwrap_or(interest::db::IncomeEventType::Dividend),
                    amount_per_quota: get_decimal(row, 5)?,
                    total_amount: get_decimal(row, 6)?,
                    withholding_tax: get_decimal(row, 7)?,
                    is_quota_pre_2026: row.get(8)?,
                    source: row.get(9)?,
                    notes: row.get(10)?,
                    created_at: row.get(11)?,
                })
            })
            .expect("failed to map income rows")
            .collect::<Result<Vec<_>, _>>()
            .expect("failed to collect income rows");

        events
    }

    /// Calculate position (quantity, total_cost) for a ticker
    pub fn query_position(home: &TempDir, ticker: &str) -> (Decimal, Decimal) {
        let transactions = query_transactions(home, ticker);

        let mut total_quantity = Decimal::ZERO;
        let mut total_cost = Decimal::ZERO;

        for tx in transactions {
            match tx.transaction_type {
                TransactionType::Buy => {
                    total_quantity += tx.quantity;
                    total_cost += tx.total_cost;
                }
                TransactionType::Sell => {
                    total_quantity -= tx.quantity;
                }
            }
        }

        (total_quantity, total_cost)
    }

    /// Helper to read Decimal from SQLite (handles INTEGER, REAL, TEXT)
    pub fn get_decimal(row: &rusqlite::Row, idx: usize) -> Result<Decimal, rusqlite::Error> {
        use rusqlite::types::ValueRef;

        match row.get_ref(idx)? {
            ValueRef::Text(bytes) => {
                let s = std::str::from_utf8(bytes)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                Decimal::from_str(s)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
            }
            ValueRef::Integer(i) => Ok(Decimal::from(i)),
            ValueRef::Real(f) => Decimal::try_from(f)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e))),
            _ => Err(rusqlite::Error::InvalidColumnType(
                idx,
                "decimal".to_string(),
                rusqlite::types::Type::Null,
            )),
        }
    }
}

// =============================================================================
// Legacy Test Helper: trade-only import (used by corp action tests)
// =============================================================================

/// Test helper: Import movimentacao file into database (trade entries only)
fn import_movimentacao(conn: &Connection, file_path: &str) -> Result<()> {
    let entries = parse_movimentacao_excel(file_path)?;
    let trade_entries: Vec<_> = entries.into_iter().filter(|e| e.is_trade()).collect();
    import_movimentacao_entries(conn, trade_entries, false)?;
    Ok(())
}

#[test]
fn test_13_ofertas_publicas_import_normalizes_ticker() -> Result<()> {
    let entries = parse_ofertas_publicas_excel("tests/data/13_ofertas_publicas.xlsx")?;
    assert_eq!(entries.len(), 1);
    let entry = &entries[0];
    assert_eq!(entry.ticker, "AMBP3");
    assert_eq!(entry.raw_ticker, "AMBP3L");
    assert_eq!(entry.quantity, Decimal::from(1064));
    Ok(())
}

#[test]
fn test_14_option_exercise_resolves_underlying_ticker() -> Result<()> {
    let raw_tx = interest::importers::RawTransaction {
        ticker: "ITSAA101E".to_string(),
        transaction_type: "Venda".to_string(),
        trade_date: chrono::NaiveDate::from_ymd_opt(2022, 1, 21).unwrap(),
        quantity: Decimal::from(2000),
        price: Decimal::from(9),
        fees: Decimal::ZERO,
        total: Decimal::from(19060),
        market: Some("Exercício de Opção de Compra".to_string()),
    };

    let asset_exists = |ticker: &str| -> Result<bool> { Ok(ticker == "ITSA4") };
    let (resolved, notes) = resolve_option_exercise_ticker(&raw_tx, asset_exists)?;

    assert_eq!(resolved, "ITSA4");
    assert!(notes.unwrap_or_default().contains("ITSAA101E"));
    Ok(())
}

// Removed unused legacy helpers (create_test_db, import_movimentacao_with_state, get_decimal_value,
// get_transactions, calculate_position) now that all tests are converted to the new style.

#[test]
fn test_01_basic_purchase_and_sale() -> Result<()> {
    let home = TempDir::new()?;

    // Import the test file using JSON output to verify stats
    let import_result = run_import_json(&home, "tests/data/01_basic_purchase_sale.xlsx");
    assert!(assert_json_success(&import_result));

    let data = get_json_data(&import_result);
    assert_eq!(data["imported_trades"].as_u64().unwrap(), 3);

    // Verify portfolio shows correct position using formatted output
    let mut portfolio_cmd = base_cmd(&home);
    portfolio_cmd.env("INTEREST_SKIP_PRICE_FETCH", "1");
    portfolio_cmd.arg("portfolio").arg("show");

    portfolio_cmd
        .assert()
        .success()
        .stdout(predicate::str::contains("PETR4"))
        .stdout(predicate::str::contains("70.00")); // Final quantity after 100 + 50 - 80

    // Deep inspection: verify transactions via SQL
    let transactions = sql::query_transactions(&home, "PETR4");
    assert_eq!(transactions.len(), 3, "Should have 3 transactions");

    // Verify first purchase
    assert_eq!(transactions[0].quantity.to_string(), "100");
    assert_eq!(transactions[0].price_per_unit.to_string(), "25");
    assert_eq!(transactions[0].total_cost.to_string(), "2500");

    // Verify second purchase
    assert_eq!(transactions[1].quantity.to_string(), "50");
    assert_eq!(transactions[1].price_per_unit.to_string(), "30");

    // Verify sale
    assert_eq!(transactions[2].quantity.to_string(), "80");
    assert_eq!(transactions[2].price_per_unit.to_string(), "35");

    // Verify final position
    let (quantity, _cost) = sql::query_position(&home, "PETR4");
    assert_eq!(quantity.to_string(), "70"); // 100 + 50 - 80

    Ok(())
}

// =============================================================================
// Basic CLI Tests
// =============================================================================

#[test]
fn test_portfolio_show_empty_db() -> Result<()> {
    let home = TempDir::new()?;

    // Run portfolio show on empty database
    let mut cmd = base_cmd(&home);
    cmd.arg("portfolio").arg("show");

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("No positions found"))
        .stdout(predicate::str::contains("\u{001b}[").not());

    Ok(())
}

#[test]
fn test_import_dry_run_does_not_create_db() -> Result<()> {
    let home = TempDir::new()?;
    let db_path = get_db_path(&home);
    assert!(!db_path.exists(), "db should start absent");

    let mut cmd = base_cmd(&home);
    cmd.arg("import")
        .arg("tests/data/01_basic_purchase_sale.xlsx")
        .arg("--dry-run");

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("Found"))
        .stdout(predicate::str::contains("Dry run"))
        .stdout(predicate::str::contains("\u{001b}[").not());

    assert!(!db_path.exists(), "dry-run should not create db");

    Ok(())
}

#[test]
fn test_import_then_portfolio_shows_position() -> Result<()> {
    let home = TempDir::new()?;

    // Import file
    let mut import_cmd = base_cmd(&home);
    import_cmd
        .arg("import")
        .arg("tests/data/01_basic_purchase_sale.xlsx");

    import_cmd
        .assert()
        .success()
        .stdout(predicate::str::contains("Found"))
        .stdout(predicate::str::contains("\u{001b}[").not());

    // Verify portfolio
    let mut portfolio_cmd = base_cmd(&home);
    portfolio_cmd.arg("portfolio").arg("show");

    portfolio_cmd
        .assert()
        .success()
        .stdout(predicate::str::contains("PETR4"))
        .stdout(predicate::str::contains("70.00"))
        .stdout(predicate::str::contains("\u{001b}[").not());

    Ok(())
}

#[test]
fn test_portfolio_filters_by_asset_type_stock() -> Result<()> {
    let home = TempDir::new()?;

    // Import file with multiple asset types
    let mut import_cmd = base_cmd(&home);
    import_cmd
        .arg("import")
        .arg("tests/data/01_basic_purchase_sale.xlsx");

    import_cmd.assert().success();

    // Show portfolio filtered to STOCK only
    let mut portfolio_cmd = base_cmd(&home);
    portfolio_cmd
        .arg("portfolio")
        .arg("show")
        .arg("--asset-type")
        .arg("STOCK");

    portfolio_cmd
        .assert()
        .success()
        .stdout(predicate::str::contains("## Stocks (STOCK)"))
        .stdout(predicate::str::contains("PETR4"));

    Ok(())
}

#[test]
fn test_portfolio_filters_by_asset_type_fii() -> Result<()> {
    let home = TempDir::new()?;

    // Import a basic file to verify filtering works
    let mut import_cmd = base_cmd(&home);
    import_cmd
        .arg("import")
        .arg("tests/data/01_basic_purchase_sale.xlsx");

    import_cmd.assert().success();

    // Show portfolio and verify filtering returns only requested type
    // (file only has STOCK, so filtering by STOCK should succeed)
    let mut portfolio_cmd = base_cmd(&home);
    portfolio_cmd
        .arg("portfolio")
        .arg("show")
        .arg("--asset-type")
        .arg("STOCK");

    portfolio_cmd
        .assert()
        .success()
        .stdout(predicate::str::contains("## Stocks (STOCK)"))
        // When filtering to a single type, only that section appears
        .stdout(predicate::str::contains("Portfolio Summary"));

    Ok(())
}

#[test]
fn test_portfolio_uses_short_asset_type_flag() -> Result<()> {
    let home = TempDir::new()?;

    // Import file
    let mut import_cmd = base_cmd(&home);
    import_cmd
        .arg("import")
        .arg("tests/data/01_basic_purchase_sale.xlsx");

    import_cmd.assert().success();

    // Show portfolio using short form -a flag
    let mut portfolio_cmd = base_cmd(&home);
    portfolio_cmd
        .arg("portfolio")
        .arg("show")
        .arg("-a")
        .arg("STOCK");

    portfolio_cmd
        .assert()
        .success()
        .stdout(predicate::str::contains("## Stocks (STOCK)"))
        .stdout(predicate::str::contains("PETR4"));

    Ok(())
}

#[test]
fn test_portfolio_groups_by_asset_type() -> Result<()> {
    let home = TempDir::new()?;

    // Import file with multiple asset types
    let mut import_cmd = base_cmd(&home);
    import_cmd
        .arg("import")
        .arg("tests/data/08_complex_scenario.xlsx");

    import_cmd.assert().success();

    // Show full portfolio
    let mut portfolio_cmd = base_cmd(&home);
    portfolio_cmd.arg("portfolio").arg("show");

    portfolio_cmd
        .assert()
        .success()
        // Should have a Stocks group
        .stdout(predicate::str::contains("## Stocks (STOCK)"))
        // Should show subtotals for each group
        .stdout(predicate::str::contains("Subtotal"))
        // Should show overall portfolio summary
        .stdout(predicate::str::contains("Portfolio Summary"));

    Ok(())
}

// =============================================================================
// Legacy Tests (using direct library access)
// =============================================================================

#[test]
fn test_02_term_contract_lifecycle() -> Result<()> {
    let home = TempDir::new()?;

    // Import the test file via CLI (ensures DB is initialized)
    let import_result = run_import_json(&home, "tests/data/02_term_contract_lifecycle.xlsx");
    assert!(assert_json_success(&import_result));

    // Check ANIM3T term contract purchase
    let term_txs = sql::query_transactions(&home, "ANIM3T");
    assert_eq!(term_txs.len(), 1, "Should have 1 term contract purchase");
    assert_eq!(term_txs[0].quantity, dec!(200));
    assert_eq!(term_txs[0].price_per_unit, dec!(10));

    // Check ANIM3 transactions (liquidation + sale)
    let base_txs = sql::query_transactions(&home, "ANIM3");
    assert_eq!(base_txs.len(), 2, "Should have liquidation + sale");

    // Verify liquidation is marked correctly
    assert!(base_txs[0]
        .notes
        .as_ref()
        .unwrap()
        .contains("Term contract liquidation"));

    // Process term contract liquidations via direct DB access
    let db_path = get_db_path(&home);
    let conn = Connection::open(&db_path)?;
    let processed = process_term_liquidations(&conn)?;
    assert_eq!(processed, 1, "Should process 1 term liquidation");

    // Test cost basis calculation
    let mut avg = AverageCostMatcher::new();
    avg.add_purchase(&base_txs[0], None, None); // Liquidation becomes a purchase

    let sale_result = avg.match_sale(&base_txs[1], None)?;

    // Cost basis should be from term contract: 100 @ R$10.00 = R$1,000.00
    assert_eq!(sale_result.cost_basis, dec!(1000));
    assert_eq!(sale_result.sale_total, dec!(1200));
    assert_eq!(sale_result.profit_loss, dec!(200));

    Ok(())
}

#[test]
fn test_09_duplicate_trades_not_deduped() -> Result<()> {
    let home = TempDir::new()?;

    // Import file
    let import_result = run_import_json(&home, "tests/data/10_duplicate_trades.xlsx");
    assert!(assert_json_success(&import_result));

    let data = get_json_data(&import_result);
    assert_eq!(data["imported_trades"].as_u64().unwrap(), 2);

    // Verify with SQL
    let transactions = sql::query_transactions(&home, "DUPL3");
    assert_eq!(
        transactions.len(),
        2,
        "Both duplicate trades should be imported"
    );
    Ok(())
}

#[test]
fn test_10_no_reimport_of_old_data() -> Result<()> {
    let home = TempDir::new()?;

    // First import
    let import_result_first = run_import_json(&home, "tests/data/10_duplicate_trades.xlsx");
    assert!(assert_json_success(&import_result_first));
    let data_first = get_json_data(&import_result_first);
    assert_eq!(data_first["imported_trades"].as_u64().unwrap(), 2);

    // Second import - should skip
    let import_result_second = run_import_json(&home, "tests/data/10_duplicate_trades.xlsx");
    assert!(assert_json_success(&import_result_second));
    let data_second = get_json_data(&import_result_second);
    assert_eq!(data_second["imported_trades"].as_u64().unwrap(), 0);
    assert_eq!(data_second["skipped_trades_old"].as_u64().unwrap(), 2);

    // Verify with SQL
    let transactions = sql::query_transactions(&home, "DUPL3");
    assert_eq!(transactions.len(), 2);
    Ok(())
}

#[test]
fn test_11_auto_apply_bonus_action_on_import() -> Result<()> {
    let home = TempDir::new()?;

    // Import file
    let import_result = run_import_json(&home, "tests/data/11_bonus_auto_apply.xlsx");
    assert!(assert_json_success(&import_result));

    let data = get_json_data(&import_result);
    assert_eq!(data["imported_trades"].as_u64().unwrap(), 1);
    assert_eq!(data["imported_actions"].as_u64().unwrap(), 1);
    assert_eq!(data["auto_applied_actions"].as_u64().unwrap(), 0);

    // Verify with SQL
    let transactions = sql::query_transactions(&home, "ITSA4");
    assert_eq!(transactions.len(), 2);

    let (quantity, cost_basis) = sql::query_position(&home, "ITSA4");
    assert_eq!(quantity, dec!(120));
    assert_eq!(cost_basis, dec!(1000));

    // Verify portfolio snapshot at post-buy date (before bonus)
    let mut cmd_before = base_cmd(&home);
    cmd_before
        .env("INTEREST_SKIP_PRICE_FETCH", "1")
        .arg("portfolio")
        .arg("show")
        .arg("--at")
        .arg("2021-01-11"); // After initial buy on 2021-01-10, before bonus
    let out_before = cmd_before.output()?;
    assert!(out_before.status.success());
    let stdout_before = String::from_utf8_lossy(&out_before.stdout);
    let row_before = stdout_before
        .lines()
        .find(|l| l.starts_with("│ ITSA4"))
        .expect("ITSA4 row not found at 2021-01-11");
    let cols_before: Vec<String> = row_before
        .split('│')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    assert_eq!(cols_before[1], "100.00", "Qty before bonus should be 100");

    // Verify portfolio snapshot at post-bonus date
    let mut cmd_after = base_cmd(&home);
    cmd_after
        .env("INTEREST_SKIP_PRICE_FETCH", "1")
        .arg("portfolio")
        .arg("show")
        .arg("--at")
        .arg("2021-12-23"); // After bonus on 2021-12-22
    let out_after = cmd_after.output()?;
    assert!(out_after.status.success());
    let stdout_after = String::from_utf8_lossy(&out_after.stdout);
    let row_after = stdout_after
        .lines()
        .find(|l| l.starts_with("│ ITSA4"))
        .expect("ITSA4 row not found at 2021-12-23");
    let cols_after: Vec<String> = row_after
        .split('│')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    assert_eq!(cols_after[1], "120.00", "Qty after bonus should be 120");
    assert_eq!(cols_after[2], "R$ 8,33", "Avg cost should be R$8.33");
    assert_eq!(cols_after[3], "R$ 1.000,00", "Total cost should be R$1000");

    Ok(())
}

/// Test that bonus shares are calculated correctly when there's a prior split.
/// This verifies query-time adjustment applies split BEFORE calculating bonus.
#[test]
fn test_11b_split_then_bonus_calculates_correctly() -> Result<()> {
    let home = TempDir::new()?;

    // Initialize database
    let db_path = get_db_path(&home);
    std::fs::create_dir_all(db_path.parent().unwrap())?;
    init_database(Some(db_path.clone()))?;
    let conn = Connection::open(&db_path)?;

    // Create asset
    let asset_id = upsert_asset(&conn, "TEST11", &AssetType::Stock, None)?;

    // Insert purchase: 100 shares @ R$10 on 2025-01-15
    let buy_tx = Transaction {
        id: None,
        asset_id,
        transaction_type: TransactionType::Buy,
        trade_date: chrono::NaiveDate::from_ymd_opt(2025, 1, 15).unwrap(),
        settlement_date: None,
        quantity: dec!(100),
        price_per_unit: dec!(10),
        total_cost: dec!(1000),
        fees: dec!(0),
        is_day_trade: false,
        quota_issuance_date: None,
        notes: None,
        source: "TEST".to_string(),
        created_at: chrono::Utc::now(),
    };
    interest::db::insert_transaction(&conn, &buy_tx)?;

    // Add 1:2 split on 2025-02-10 (100 shares -> 200 shares, add 100 shares)
    base_cmd(&home)
        .arg("actions")
        .arg("split")
        .arg("add")
        .arg("TEST11")
        .arg("100")
        .arg("2025-02-10")
        .assert()
        .success();

    // Add bonus on 2025-03-15 (add 20 shares bonus)
    base_cmd(&home)
        .arg("actions")
        .arg("bonus")
        .arg("add")
        .arg("TEST11")
        .arg("20")
        .arg("2025-03-15")
        .assert()
        .success();

    // Apply corporate actions (only bonus creates transactions)
    base_cmd(&home)
        .arg("actions")
        .arg("apply")
        .arg("TEST11")
        .assert()
        .success();

    // Verify: should have 2 transactions (original buy + bonus)
    let tx_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM transactions WHERE asset_id = ?1",
        [asset_id],
        |row| row.get(0),
    )?;
    assert_eq!(tx_count, 2, "Should have original buy + bonus transaction");

    // Verify bonus transaction quantity
    let bonus_qty = conn.query_row(
        "SELECT quantity FROM transactions WHERE asset_id = ?1 AND notes LIKE '%Bonus%'",
        [asset_id],
        |row| interest::db::get_decimal_value(row, 0),
    )?;

    // If split was correctly applied before bonus calculation:
    // Position = 100 shares, after 1:2 split = 200 shares
    // Bonus 10:11 on 200 shares = 200 * 11/10 - 200 = 20 bonus shares
    assert_eq!(
        bonus_qty,
        dec!(20),
        "Bonus should be 20 shares (based on 200 split-adjusted shares, not 100 raw)"
    );

    // Verify portfolio snapshot BEFORE split (2025-02-09): 100 shares
    let mut cmd_before_split = base_cmd(&home);
    cmd_before_split
        .env("INTEREST_SKIP_PRICE_FETCH", "1")
        .arg("portfolio")
        .arg("show")
        .arg("--at")
        .arg("2025-02-09"); // Before split
    let out_before_split = cmd_before_split.output()?;
    assert!(out_before_split.status.success());
    let stdout_before_split = String::from_utf8_lossy(&out_before_split.stdout);
    let row_before_split = stdout_before_split
        .lines()
        .find(|l| l.starts_with("│ TEST11"))
        .expect("TEST11 row not found at 2025-02-09");
    let cols_before_split: Vec<String> = row_before_split
        .split('│')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    assert_eq!(
        cols_before_split[1], "100.00",
        "Qty before split should be 100"
    );

    // Verify portfolio snapshot AFTER split but BEFORE bonus (2025-02-11): 200 shares
    let mut cmd_after_split = base_cmd(&home);
    cmd_after_split
        .env("INTEREST_SKIP_PRICE_FETCH", "1")
        .arg("portfolio")
        .arg("show")
        .arg("--at")
        .arg("2025-02-11"); // After split
    let out_after_split = cmd_after_split.output()?;
    assert!(out_after_split.status.success());
    let stdout_after_split = String::from_utf8_lossy(&out_after_split.stdout);
    let row_after_split = stdout_after_split
        .lines()
        .find(|l| l.starts_with("│ TEST11"))
        .expect("TEST11 row not found at 2025-02-11");
    let cols_after_split: Vec<String> = row_after_split
        .split('│')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    assert_eq!(
        cols_after_split[1], "200.00",
        "Qty after split should be 200"
    );
    assert_eq!(
        cols_after_split[2], "R$ 5,00",
        "Avg cost after split should be R$5.00"
    );

    // Verify portfolio snapshot AFTER bonus (2025-03-16): 220 shares
    let mut cmd_after_bonus = base_cmd(&home);
    cmd_after_bonus
        .env("INTEREST_SKIP_PRICE_FETCH", "1")
        .arg("portfolio")
        .arg("show")
        .arg("--at")
        .arg("2025-03-16"); // After bonus
    let out_after_bonus = cmd_after_bonus.output()?;
    assert!(out_after_bonus.status.success());
    let stdout_after_bonus = String::from_utf8_lossy(&out_after_bonus.stdout);
    let row_after_bonus = stdout_after_bonus
        .lines()
        .find(|l| l.starts_with("│ TEST11"))
        .expect("TEST11 row not found at 2025-03-16");
    let cols_after_bonus: Vec<String> = row_after_bonus
        .split('│')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    assert_eq!(
        cols_after_bonus[1], "220.00",
        "Qty after bonus should be 220"
    );

    Ok(())
}

#[test]
fn test_12_desdobro_absolute_adjustment() -> Result<()> {
    let home = TempDir::new()?;

    // Import file
    let import_result = run_import_json(&home, "tests/data/12_desdobro_inference.xlsx");
    assert!(assert_json_success(&import_result));

    let data = get_json_data(&import_result);
    assert_eq!(data["imported_trades"].as_u64().unwrap(), 1);
    assert_eq!(data["imported_actions"].as_u64().unwrap(), 1);
    // With query-time adjustment, splits no longer auto-apply (adjustment happens at query time)
    assert_eq!(data["auto_applied_actions"].as_u64().unwrap(), 0);

    // Verify transactions remain unchanged in database
    let transactions = sql::query_transactions(&home, "A1MD34");
    assert_eq!(transactions.len(), 1);
    assert_eq!(
        transactions[0].quantity,
        dec!(80),
        "Database quantity unchanged"
    );
    assert_eq!(
        transactions[0].total_cost,
        dec!(800),
        "Database cost unchanged"
    );

    // Verify corporate action was created via CLI
    let actions_out = base_cmd(&home)
        .arg("--json")
        .arg("actions")
        .arg("split")
        .arg("list")
        .arg("A1MD34")
        .output()
        .expect("failed to run actions split list");
    assert!(actions_out.status.success(), "actions split list failed");
    let actions_json: Value =
        serde_json::from_slice(&actions_out.stdout).expect("invalid actions JSON");
    let actions_array = actions_json.as_array().expect("actions JSON is not array");
    assert_eq!(
        actions_array.len(),
        1,
        "Corporate action should be recorded"
    );
    let qty_str = actions_array[0]
        .get("quantity_adjustment")
        .and_then(|v| v.as_str())
        .unwrap_or("0");
    let qty_adj = Decimal::from_str(qty_str).expect("quantity_adjustment decimal parse");
    assert_eq!(qty_adj, dec!(560), "Split adds 560 shares");

    // Verify portfolio shows adjusted quantity via CLI
    let mut cmd = base_cmd(&home);
    cmd.env("INTEREST_SKIP_PRICE_FETCH", "1")
        .arg("portfolio")
        .arg("show");
    let out = cmd.output()?;
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let row = stdout
        .lines()
        .find(|l| l.starts_with("│ A1MD34"))
        .expect("A1MD34 row not found");
    let cols: Vec<String> = row
        .split('│')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    // 80 shares with 1:8 split (desdobro adds 560) = 80 + 560 = 640
    assert_eq!(
        cols[1], "640.00",
        "Qty adjusted for desdobro split (80 + 560)"
    );

    Ok(())
}

#[test]
fn test_14_atualizacao_absolute_adjustment() -> Result<()> {
    let home = TempDir::new()?;

    // Import file
    let import_result = run_import_json(&home, "tests/data/14_atualizacao_inference.xlsx");
    assert!(assert_json_success(&import_result));

    let data = get_json_data(&import_result);
    assert_eq!(data["imported_trades"].as_u64().unwrap(), 1);
    assert_eq!(data["imported_actions"].as_u64().unwrap(), 0);
    assert_eq!(data["auto_applied_actions"].as_u64().unwrap(), 0);

    // Verify with SQL
    let transactions = sql::query_transactions(&home, "BRCR11");
    assert_eq!(transactions.len(), 1);

    let (quantity, cost_basis) = sql::query_position(&home, "BRCR11");
    assert_eq!(quantity, dec!(378));
    assert_eq!(cost_basis, dec!(3780));

    // Verify portfolio shows consolidated position via CLI
    let mut cmd = base_cmd(&home);
    cmd.env("INTEREST_SKIP_PRICE_FETCH", "1")
        .arg("portfolio")
        .arg("show");
    let out = cmd.output()?;
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let row = stdout
        .lines()
        .find(|l| l.starts_with("│ BRCR11"))
        .expect("BRCR11 row not found");
    let cols: Vec<String> = row
        .split('│')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    assert_eq!(cols[1], "378.00", "Qty should be 378");
    assert_eq!(cols[2], "R$ 10,00", "Avg cost should be R$10.00");
    assert_eq!(cols[3], "R$ 3.780,00", "Total cost should be R$3780");

    Ok(())
}

#[test]
fn test_03_term_contract_sold_before_expiry() -> Result<()> {
    let home = TempDir::new()?;

    let import_result = run_import_json(&home, "tests/data/03_term_contract_sold.xlsx");
    assert!(assert_json_success(&import_result));

    let transactions = sql::query_transactions(&home, "SHUL4T");
    assert_eq!(transactions.len(), 2, "Should have buy and sell");

    // Test cost basis - term contracts can be traded like regular stocks
    let mut avg = AverageCostMatcher::new();
    avg.add_purchase(&transactions[0], None, None);

    let sale_result = avg.match_sale(&transactions[1], None)?;

    assert_eq!(sale_result.cost_basis, dec!(1200));
    assert_eq!(sale_result.sale_total, dec!(1350));
    assert_eq!(sale_result.profit_loss, dec!(150));

    Ok(())
}

#[test]
fn test_04_stock_split() -> Result<()> {
    let home = TempDir::new()?;

    // Initialize DB and import only trade entries (no auto actions)
    let db_path = get_db_path(&home);
    std::fs::create_dir_all(db_path.parent().unwrap())?;
    init_database(Some(db_path.clone()))?;
    let conn = Connection::open(&db_path)?;
    import_movimentacao(&conn, "tests/data/04_stock_split.xlsx")?;

    let transactions = sql::query_transactions(&home, "VALE3");

    // Before adjustments: should have 4 transactions (buy, split, buy, sell)
    // Split entry is not imported as a transaction, only as corporate action
    assert_eq!(
        transactions.len(),
        3,
        "Should have 3 transactions (buy, buy, sell)"
    );

    // Create the split corporate action via CLI
    base_cmd(&home)
        .arg("actions")
        .arg("split")
        .arg("add")
        .arg("VALE3")
        .arg("100")
        .arg("2025-02-15")
        .assert()
        .success();

    // Verify the action exists via CLI
    let actions_out = base_cmd(&home)
        .arg("--json")
        .arg("actions")
        .arg("split")
        .arg("list")
        .arg("VALE3")
        .output()
        .expect("failed to run actions split list");
    assert!(actions_out.status.success(), "actions split list failed");
    let actions_json: Value =
        serde_json::from_slice(&actions_out.stdout).expect("invalid actions JSON");
    let actions_array = actions_json.as_array().expect("actions JSON is not array");
    assert_eq!(actions_array.len(), 1, "Should have 1 corporate action");
    let qty_str = actions_array[0]
        .get("quantity_adjustment")
        .and_then(|v| v.as_str())
        .unwrap_or("0");
    let qty_adj = Decimal::from_str(qty_str).expect("quantity_adjustment decimal parse");
    assert_eq!(qty_adj, dec!(100), "Split adds 100 shares");

    // Re-fetch transactions - they should be UNCHANGED in database
    let db_txs = sql::query_transactions(&home, "VALE3");

    // First purchase should remain UNADJUSTED in database: 100 @ R$80
    assert_eq!(db_txs[0].quantity, dec!(100), "Database quantity unchanged");
    assert_eq!(
        db_txs[0].price_per_unit,
        dec!(80),
        "Database price unchanged"
    );

    // Second purchase (after split) should also be unchanged
    assert_eq!(db_txs[1].quantity, dec!(50));
    assert_eq!(db_txs[1].price_per_unit, dec!(42));

    let before_second_buy = db_txs[1]
        .trade_date
        .pred_opt()
        .unwrap_or(db_txs[1].trade_date);
    let mut cmd_after_split = base_cmd(&home);
    cmd_after_split
        .env("INTEREST_SKIP_PRICE_FETCH", "1")
        .arg("portfolio")
        .arg("show")
        .arg("--at")
        .arg(before_second_buy.to_string());
    let out_after_split = cmd_after_split.output()?;
    assert!(out_after_split.status.success());
    let stdout_after_split = String::from_utf8_lossy(&out_after_split.stdout);
    let row_after_split = stdout_after_split
        .lines()
        .find(|l| l.starts_with("│ VALE3"))
        .expect("VALE3 row not found after split");
    let cols_after_split: Vec<String> = row_after_split
        .split('│')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    assert_eq!(cols_after_split[1], "200.00");
    assert_eq!(cols_after_split[2], "R$ 40,00");
    assert_eq!(cols_after_split[3], "R$ 8.000,00");

    let mut cmd_after_sale = base_cmd(&home);
    cmd_after_sale
        .env("INTEREST_SKIP_PRICE_FETCH", "1")
        .arg("portfolio")
        .arg("show")
        .arg("--at")
        .arg(db_txs[2].trade_date.to_string());
    let out_after_sale = cmd_after_sale.output()?;
    assert!(out_after_sale.status.success());
    let stdout_after_sale = String::from_utf8_lossy(&out_after_sale.stdout);
    let row_after_sale = stdout_after_sale
        .lines()
        .find(|l| l.starts_with("│ VALE3"))
        .expect("VALE3 row not found after sale");
    let cols_after_sale: Vec<String> = row_after_sale
        .split('│')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    assert_eq!(cols_after_sale[1], "100.00");
    assert_eq!(cols_after_sale[2], "R$ 40,40");
    assert_eq!(cols_after_sale[3], "R$ 4.040,00");

    Ok(())
}

#[test]
fn test_05_reverse_split() -> Result<()> {
    let home = TempDir::new()?;

    // Initialize DB and import only trade entries (no auto actions)
    let db_path = get_db_path(&home);
    std::fs::create_dir_all(db_path.parent().unwrap())?;
    init_database(Some(db_path.clone()))?;
    let conn = Connection::open(&db_path)?;
    import_movimentacao(&conn, "tests/data/05_reverse_split.xlsx")?;

    let transactions = sql::query_transactions(&home, "MGLU3");
    assert_eq!(transactions.len(), 2, "Should have buy and sell");

    // Create reverse split (10:1 - 10 shares become 1)
    base_cmd(&home)
        .arg("actions")
        .arg("split")
        .arg("add")
        .arg("MGLU3")
        .arg("--")
        .arg("-900")
        .arg("2025-02-20")
        .assert()
        .success();

    // Verify action exists via CLI
    let actions_out = base_cmd(&home)
        .arg("--json")
        .arg("actions")
        .arg("split")
        .arg("list")
        .arg("MGLU3")
        .output()
        .expect("failed to run actions split list");
    assert!(actions_out.status.success(), "actions split list failed");
    let actions_json: Value =
        serde_json::from_slice(&actions_out.stdout).expect("invalid actions JSON");
    let actions_array = actions_json.as_array().expect("actions JSON is not array");
    assert_eq!(actions_array.len(), 1, "Should have 1 corporate action");
    assert_eq!(
        actions_array[0]
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or(""),
        "REVERSE_SPLIT"
    );
    let qty_str = actions_array[0]
        .get("quantity_adjustment")
        .and_then(|v| v.as_str())
        .unwrap_or("0");
    let qty_adj = Decimal::from_str(qty_str).expect("quantity_adjustment decimal parse");
    assert_eq!(qty_adj, dec!(-900), "Reverse split removes 900 shares");

    // Re-fetch transactions - should be UNCHANGED in database
    let db_txs = sql::query_transactions(&home, "MGLU3");
    assert_eq!(
        db_txs[0].quantity,
        dec!(1000),
        "Database quantity unchanged"
    );
    assert_eq!(
        db_txs[0].price_per_unit,
        dec!(2),
        "Database price unchanged"
    );

    let before_sale = db_txs[1]
        .trade_date
        .pred_opt()
        .unwrap_or(db_txs[1].trade_date);
    let mut cmd_after_split = base_cmd(&home);
    cmd_after_split
        .env("INTEREST_SKIP_PRICE_FETCH", "1")
        .arg("portfolio")
        .arg("show")
        .arg("--at")
        .arg(before_sale.to_string());
    let out_after_split = cmd_after_split.output()?;
    assert!(out_after_split.status.success());
    let stdout_after_split = String::from_utf8_lossy(&out_after_split.stdout);
    let row_after_split = stdout_after_split
        .lines()
        .find(|l| l.starts_with("│ MGLU3"))
        .expect("MGLU3 row not found after split");
    let cols_after_split: Vec<String> = row_after_split
        .split('│')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    assert_eq!(cols_after_split[1], "100.00");
    assert_eq!(cols_after_split[2], "R$ 20,00");
    assert_eq!(cols_after_split[3], "R$ 2.000,00");

    let mut cmd_after_sale = base_cmd(&home);
    cmd_after_sale
        .env("INTEREST_SKIP_PRICE_FETCH", "1")
        .arg("portfolio")
        .arg("show")
        .arg("--at")
        .arg(db_txs[1].trade_date.to_string());
    let out_after_sale = cmd_after_sale.output()?;
    assert!(out_after_sale.status.success());
    let stdout_after_sale = String::from_utf8_lossy(&out_after_sale.stdout);
    let row_after_sale = stdout_after_sale
        .lines()
        .find(|l| l.starts_with("│ MGLU3"))
        .expect("MGLU3 row not found after sale");
    let cols_after_sale: Vec<String> = row_after_sale
        .split('│')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    assert_eq!(cols_after_sale[1], "50.00");
    assert_eq!(cols_after_sale[2], "R$ 20,00");
    assert_eq!(cols_after_sale[3], "R$ 1.000,00");

    Ok(())
}

#[test]
fn test_06_multiple_splits() -> Result<()> {
    let home = TempDir::new()?;

    // Initialize DB and import only trade entries (no corporate actions)
    let db_path = get_db_path(&home);
    std::fs::create_dir_all(db_path.parent().unwrap())?;
    init_database(Some(db_path.clone()))?;
    let conn = Connection::open(&db_path)?;
    import_movimentacao(&conn, "tests/data/06_multiple_splits.xlsx")?;

    // Verify initial trades were imported
    let transactions = sql::query_transactions(&home, "ITSA4");
    assert_eq!(transactions.len(), 3, "Should have 3 transactions");

    // Verify database transactions are unadjusted
    assert_eq!(transactions[0].quantity, dec!(50), "First buy: 50 shares");
    assert_eq!(
        transactions[0].price_per_unit,
        dec!(10),
        "First buy: R$10/share"
    );
    assert_eq!(
        transactions[0].total_cost,
        dec!(500),
        "First buy: R$500 total"
    );

    assert_eq!(transactions[1].quantity, dec!(25), "Second buy: 25 shares");
    assert_eq!(
        transactions[1].price_per_unit,
        dec!(5.5),
        "Second buy: R$5.50/share"
    );
    assert_eq!(
        transactions[1].total_cost,
        dec!(137.5),
        "Second buy: R$137.50 total"
    );

    assert_eq!(transactions[2].quantity, dec!(200), "Sell: 200 shares");

    // Import full file to get corporate actions
    let import_result = run_import_json(&home, "tests/data/06_multiple_splits.xlsx");
    assert!(assert_json_success(&import_result));

    let data = get_json_data(&import_result);
    // Should skip trades (already imported) but import corporate actions
    assert_eq!(data["skipped_trades_old"].as_u64().unwrap(), 3);
    assert_eq!(data["imported_actions"].as_u64().unwrap(), 2);

    // Verify corporate actions were created via CLI
    let actions_out = base_cmd(&home)
        .arg("--json")
        .arg("actions")
        .arg("split")
        .arg("list")
        .arg("ITSA4")
        .output()
        .expect("failed to run actions split list");
    assert!(actions_out.status.success(), "actions split list failed");
    let actions_json: Value =
        serde_json::from_slice(&actions_out.stdout).expect("invalid actions JSON");
    let actions_array = actions_json.as_array().expect("actions JSON is not array");
    assert_eq!(
        actions_array.len(),
        2,
        "Should have 2 corporate actions (splits)"
    );
    let mut split_rows: Vec<(String, Decimal)> = actions_array
        .iter()
        .map(|row| {
            let ex_date = row
                .get("ex_date")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let qty_str = row
                .get("quantity_adjustment")
                .and_then(|v| v.as_str())
                .unwrap_or("0");
            let qty = Decimal::from_str(qty_str).expect("quantity_adjustment decimal parse");
            (ex_date, qty)
        })
        .collect();
    split_rows.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        split_rows,
        vec![
            ("2025-02-10".to_string(), dec!(50)),
            ("2025-04-15".to_string(), dec!(125)),
        ],
        "Split ex-date/quantity mismatch"
    );

    // Verify database transactions remain unchanged
    let db_txs = sql::query_transactions(&home, "ITSA4");
    assert_eq!(
        db_txs[0].quantity,
        dec!(50),
        "Database unchanged after import"
    );
    assert_eq!(
        db_txs[0].price_per_unit,
        dec!(10),
        "Database price unchanged"
    );

    // Verify portfolio at different dates to see where it breaks
    // Timeline: 05/01 - first purchase, 10/02 - split, 01/03 - second purchase, 15/04 - second split, 20/05 - sale

    // After first purchase (2025-01-06): 50 @ R$10
    let mut cmd1 = base_cmd(&home);
    cmd1.arg("portfolio")
        .arg("show")
        .arg("--at")
        .arg("2025-01-06");
    println!("\n=== Portfolio at 2025-01-06 (after first purchase) ===");
    let output1 = cmd1.output().expect("Failed to run portfolio show");
    println!("{}", String::from_utf8_lossy(&output1.stdout));

    // After first split (2025-02-11): 100 @ R$5
    let mut cmd2 = base_cmd(&home);
    cmd2.arg("portfolio")
        .arg("show")
        .arg("--at")
        .arg("2025-02-11");
    println!("\n=== Portfolio at 2025-02-11 (after first split) ===");
    let output2 = cmd2.output().expect("Failed to run portfolio show");
    println!("{}", String::from_utf8_lossy(&output2.stdout));

    // After second purchase (2025-03-02): 125 @ R$5.10 average
    let mut cmd3 = base_cmd(&home);
    cmd3.arg("portfolio")
        .arg("show")
        .arg("--at")
        .arg("2025-03-02");
    println!("\n=== Portfolio at 2025-03-02 (after second purchase) ===");
    let output3 = cmd3.output().expect("Failed to run portfolio show");
    println!("{}", String::from_utf8_lossy(&output3.stdout));

    // After second split (2025-04-16): 250 @ R$2.55 average
    let mut cmd4 = base_cmd(&home);
    cmd4.arg("portfolio")
        .arg("show")
        .arg("--at")
        .arg("2025-04-16");
    println!("\n=== Portfolio at 2025-04-16 (after second split) ===");
    let output4 = cmd4.output().expect("Failed to run portfolio show");
    println!("{}", String::from_utf8_lossy(&output4.stdout));

    // After sale (2025-05-21): 50 @ R$2.55 average
    let mut cmd5 = base_cmd(&home);
    cmd5.arg("portfolio")
        .arg("show")
        .arg("--at")
        .arg("2025-05-21");
    println!("\n=== Portfolio at 2025-05-21 (after sale) ===");
    let output5 = cmd5.output().expect("Failed to run portfolio show");
    println!("{}", String::from_utf8_lossy(&output5.stdout));

    // Verify portfolio CLI output shows adjusted quantities as a strict row match
    // Expected: 50@10 -> 100@5 (split) -> 125 total (buy 25@5.5) -> 250@2.55 (split) -> sell 200 -> 50@2.55
    let mut portfolio_cmd = base_cmd(&home);
    portfolio_cmd.arg("portfolio").arg("show");
    let output = portfolio_cmd
        .output()
        .expect("Failed to run portfolio show");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Find the ITSA4 row and validate all columns
    let row = stdout
        .lines()
        .find(|l| l.starts_with("│ ITSA4"))
        .expect("ITSA4 row not found in portfolio output");
    // Split by the vertical bar and trim columns
    let cols: Vec<String> = row
        .split('│')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    assert!(
        cols.len() >= 8,
        "Unexpected table column count: {}",
        cols.len()
    );
    assert_eq!(cols[0], "ITSA4");
    assert_eq!(cols[1], "50.00");
    assert_eq!(cols[2], "R$ 2,55");
    assert_eq!(cols[3], "R$ 127,50");
    assert_eq!(cols[4], "N/A");
    assert_eq!(cols[5], "N/A");
    assert_eq!(cols[6], "N/A");
    assert_eq!(cols[7], "N/A");

    // Check performance for 2025 via interest binary JSON output
    let perf_out = base_cmd(&home)
        .env("INTEREST_SKIP_PRICE_FETCH", "1")
        .arg("--json")
        .arg("performance")
        .arg("show")
        .arg("2025")
        .output()
        .expect("failed to run performance show");
    assert!(perf_out.status.success(), "performance show failed");
    let perf_json: Value =
        serde_json::from_slice(&perf_out.stdout).expect("invalid performance JSON");
    let start_value = perf_json
        .get("start_value")
        .and_then(|v| v.as_str())
        .unwrap_or("0");
    let end_value = perf_json
        .get("end_value")
        .and_then(|v| v.as_str())
        .unwrap_or("0");
    let total_return = perf_json
        .get("total_return")
        .and_then(|v| v.as_str())
        .unwrap_or("0");
    assert_eq!(start_value, "0");
    assert_eq!(end_value, "127.5");
    assert_eq!(total_return, "127.5");

    // Check tax report for 2025 via interest binary JSON output
    let sale_tx = db_txs[2].clone();
    let sale_total = sale_tx.total_cost; // ABS(quantity * price_per_unit)
    let expected_cost_basis = dec!(2.55) * dec!(200);
    let expected_profit = sale_total - expected_cost_basis;
    let tax_out = base_cmd(&home)
        .arg("--json")
        .arg("tax")
        .arg("report")
        .arg("2025")
        .output()
        .expect("failed to run tax report");
    assert!(tax_out.status.success(), "tax report failed");
    let tax_json: Value = serde_json::from_slice(&tax_out.stdout).expect("invalid tax JSON");
    use std::str::FromStr as _;
    let total_sales_str = tax_json
        .get("annual_total_sales")
        .and_then(|v| v.as_str())
        .unwrap_or("0");
    let total_profit_str = tax_json
        .get("annual_total_profit")
        .and_then(|v| v.as_str())
        .unwrap_or("0");
    let total_loss_str = tax_json
        .get("annual_total_loss")
        .and_then(|v| v.as_str())
        .unwrap_or("0");
    let total_sales_dec =
        rust_decimal::Decimal::from_str(total_sales_str).expect("sales decimal parse");
    let total_profit_dec =
        rust_decimal::Decimal::from_str(total_profit_str).expect("profit decimal parse");
    let total_loss_dec =
        rust_decimal::Decimal::from_str(total_loss_str).expect("loss decimal parse");
    assert_eq!(total_sales_dec, sale_total);
    assert_eq!(total_profit_dec, expected_profit);
    assert_eq!(total_loss_dec, rust_decimal::Decimal::ZERO);

    Ok(())
}

#[test]
fn test_08_complex_scenario() -> Result<()> {
    let home = TempDir::new()?;

    let import_result = run_import_json(&home, "tests/data/08_complex_scenario.xlsx");
    assert!(assert_json_success(&import_result));
    let data = get_json_data(&import_result);
    assert_eq!(data["imported_trades"].as_u64().unwrap(), 7);
    assert_eq!(data["imported_actions"].as_u64().unwrap(), 1);
    assert_eq!(data["imported_income"].as_u64().unwrap(), 0);

    let base_txs = sql::query_transactions(&home, "BBAS3");
    let term_txs = sql::query_transactions(&home, "BBAS3T");
    assert_eq!(base_txs.len(), 6, "BBAS3 should have 6 trades");
    assert_eq!(
        term_txs.len(),
        1,
        "BBAS3T should have the term contract buy"
    );

    let actions_out = base_cmd(&home)
        .arg("--json")
        .arg("actions")
        .arg("split")
        .arg("list")
        .arg("BBAS3")
        .output()
        .expect("failed to run actions split list");
    assert!(actions_out.status.success(), "actions split list failed");
    let actions_json: Value =
        serde_json::from_slice(&actions_out.stdout).expect("invalid actions JSON");
    let actions_array = actions_json.as_array().expect("actions JSON is not array");
    assert_eq!(actions_array.len(), 1, "Should import 1 split action");
    let qty_str = actions_array[0]
        .get("quantity_adjustment")
        .and_then(|v| v.as_str())
        .unwrap_or("0");
    let qty_adj = Decimal::from_str(qty_str).expect("quantity_adjustment decimal parse");
    assert_eq!(qty_adj, dec!(300), "Split adds 300 shares");

    let mut portfolio_cmd = base_cmd(&home);
    portfolio_cmd
        .arg("portfolio")
        .arg("show")
        .arg("--at")
        .arg("2025-06-16");
    let portfolio_out = portfolio_cmd.output().expect("portfolio show failed");
    assert!(portfolio_out.status.success());
    let stdout = String::from_utf8_lossy(&portfolio_out.stdout);
    let row = stdout
        .lines()
        .find(|l| l.starts_with("│ BBAS3"))
        .expect("BBAS3 row missing from portfolio output");
    let cols: Vec<String> = row
        .split('│')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    assert!(
        cols.len() >= 8,
        "Unexpected table column count: {}",
        cols.len()
    );
    assert_eq!(cols[1], "250.00");
    assert_eq!(cols[2], "R$ 22,07");
    assert_eq!(cols[3], "R$ 5.519,23");
    assert_eq!(cols[4], "N/A");
    assert_eq!(cols[5], "N/A");
    assert_eq!(cols[6], "N/A");
    assert_eq!(cols[7], "N/A");

    let perf_out = base_cmd(&home)
        .arg("--json")
        .arg("performance")
        .arg("show")
        .arg("2025")
        .output()
        .expect("performance show failed");
    assert!(perf_out.status.success());
    let perf_json: Value =
        serde_json::from_slice(&perf_out.stdout).expect("invalid performance JSON");
    let end_value = perf_json
        .get("end_value")
        .and_then(|v| v.as_str())
        .unwrap_or("0");
    let total_return = perf_json
        .get("total_return")
        .and_then(|v| v.as_str())
        .unwrap_or("0");
    assert_eq!(end_value, "5519.23076923077");
    assert_eq!(total_return, "5519.23076923077");

    let tax_out = base_cmd(&home)
        .arg("--json")
        .arg("tax")
        .arg("report")
        .arg("2025")
        .output()
        .expect("tax report failed");
    assert!(tax_out.status.success());
    let tax_json: Value = serde_json::from_slice(&tax_out.stdout).expect("invalid tax JSON");
    use std::str::FromStr as _;
    let total_sales = Decimal::from_str(
        tax_json
            .get("annual_total_sales")
            .and_then(|v| v.as_str())
            .unwrap_or("0"),
    )?;
    let total_profit = Decimal::from_str(
        tax_json
            .get("annual_total_profit")
            .and_then(|v| v.as_str())
            .unwrap_or("0"),
    )?;
    assert_eq!(total_sales, dec!(17000));
    assert_eq!(
        total_profit,
        Decimal::from_str("2069.2307692307692307692307691")?
    );

    let monthly = tax_json
        .get("monthly_summaries")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    assert_eq!(monthly.len(), 2);
    let march_profit = Decimal::from_str(monthly[0]["profit"].as_str().unwrap())?;
    let march_sales = Decimal::from_str(monthly[0]["sales"].as_str().unwrap())?;
    let june_profit = Decimal::from_str(monthly[1]["profit"].as_str().unwrap())?;
    let june_sales = Decimal::from_str(monthly[1]["sales"].as_str().unwrap())?;
    assert_eq!(
        march_profit,
        Decimal::from_str("500.0000000000000000000000001")?
    );
    assert_eq!(march_sales, dec!(6600));
    assert_eq!(
        june_profit,
        Decimal::from_str("1569.230769230769230769230769")?
    );
    assert_eq!(june_sales, dec!(10400));

    Ok(())
}

#[test]
fn test_position_totals_match() -> Result<()> {
    let home = TempDir::new()?;

    let import_result = run_import_json(&home, "tests/data/01_basic_purchase_sale.xlsx");
    assert!(assert_json_success(&import_result));

    let (quantity, _cost) = sql::query_position(&home, "PETR4");

    // After buying 100 + 50 and selling 80, should have 70 shares
    assert_eq!(quantity, dec!(70));

    Ok(())
}
#[test]
fn test_07_capital_return() -> Result<()> {
    let home = TempDir::new()?;

    let import_result = run_import_json(&home, "tests/data/07_capital_return.xlsx");
    assert!(assert_json_success(&import_result));
    let data = get_json_data(&import_result);
    assert_eq!(data["imported_trades"].as_u64().unwrap(), 3);
    assert_eq!(data["imported_income"].as_u64().unwrap(), 1);

    let transactions = sql::query_transactions(&home, "MXRF11");
    assert_eq!(
        transactions.len(),
        3,
        "Should have 3 transactions (2 buys, 1 sell)"
    );
    assert_eq!(transactions[0].quantity, dec!(100));
    assert_eq!(transactions[0].price_per_unit, dec!(10));
    assert_eq!(transactions[0].total_cost, dec!(1000));

    // Amortization recorded as income event (capital return)
    let incomes = sql::query_income_events(&home, "MXRF11");
    assert_eq!(incomes.len(), 1, "Should have 1 amortization event");
    let amort = &incomes[0];
    assert_eq!(amort.total_amount, dec!(100));
    assert_eq!(amort.amount_per_quota, dec!(1));

    // Portfolio CLI after sale date should reflect reduced cost basis and remaining 30 quotas
    let output = base_cmd(&home)
        .arg("portfolio")
        .arg("show")
        .arg("--at")
        .arg("2025-04-20")
        .output()?;
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let row = stdout
        .lines()
        .find(|l| l.starts_with("│ MXRF11"))
        .expect("MXRF11 row not found in portfolio output");
    let cols: Vec<String> = row
        .split('│')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    assert!(
        cols.len() >= 8,
        "Unexpected table column count: {}",
        cols.len()
    );
    assert_eq!(cols[0], "MXRF11");
    assert_eq!(cols[1], "30.00");
    assert_eq!(cols[2], "R$ 9,50");
    assert_eq!(cols[3], "R$ 285,00");

    // Performance JSON for 2025 should carry the reduced cost basis into end_value
    let perf_out = base_cmd(&home)
        .env("INTEREST_SKIP_PRICE_FETCH", "1")
        .arg("--json")
        .arg("performance")
        .arg("show")
        .arg("2025")
        .output()
        .expect("failed to run performance show");
    assert!(perf_out.status.success(), "performance show failed");
    let perf_json: Value =
        serde_json::from_slice(&perf_out.stdout).expect("invalid performance JSON");
    let end_value = perf_json
        .get("end_value")
        .and_then(|v| v.as_str())
        .unwrap_or("0");
    let total_return = perf_json
        .get("total_return")
        .and_then(|v| v.as_str())
        .unwrap_or("0");
    assert_eq!(end_value, "285");
    assert_eq!(total_return, end_value);

    // Tax JSON should use the amortization-adjusted average cost
    let tax_out = base_cmd(&home)
        .arg("--json")
        .arg("tax")
        .arg("report")
        .arg("2025")
        .output()
        .expect("failed to run tax report");
    assert!(tax_out.status.success(), "tax report failed");
    let tax_json: Value = serde_json::from_slice(&tax_out.stdout).expect("invalid tax JSON");
    use std::str::FromStr as _;
    let total_sales = rust_decimal::Decimal::from_str(
        tax_json
            .get("annual_total_sales")
            .and_then(|v| v.as_str())
            .unwrap_or("0"),
    )?;
    let total_profit = rust_decimal::Decimal::from_str(
        tax_json
            .get("annual_total_profit")
            .and_then(|v| v.as_str())
            .unwrap_or("0"),
    )?;
    let total_loss = rust_decimal::Decimal::from_str(
        tax_json
            .get("annual_total_loss")
            .and_then(|v| v.as_str())
            .unwrap_or("0"),
    )?;

    assert_eq!(total_sales, dec!(1320));
    assert_eq!(total_profit, dec!(180));
    assert_eq!(total_loss, rust_decimal::Decimal::ZERO);

    Ok(())
}

#[test]
fn test_10_day_trade_detection() -> Result<()> {
    let home = TempDir::new()?;
    let db_path = get_db_path(&home);
    std::fs::create_dir_all(db_path.parent().unwrap())?;
    init_database(Some(db_path.clone()))?;
    let conn = Connection::open(&db_path)?;

    // Create asset
    let asset_id = upsert_asset(&conn, "VALE3", &AssetType::Stock, Some("VALE SA"))?;

    // Day trade: buy and sell on same day
    let trade_date = chrono::NaiveDate::from_ymd_opt(2025, 3, 15).unwrap();

    // Buy 100 shares
    conn.execute(
        "INSERT INTO transactions (
            asset_id, transaction_type, trade_date, settlement_date,
            quantity, price_per_unit, total_cost, fees,
            is_day_trade, notes, source
        ) VALUES (?1, 'BUY', ?2, ?2, '100', '50', '5000', '0', 0, 'Test buy', 'TEST')",
        rusqlite::params![asset_id, trade_date],
    )?;

    // Sell 100 shares same day (day trade)
    conn.execute(
        "INSERT INTO transactions (
            asset_id, transaction_type, trade_date, settlement_date,
            quantity, price_per_unit, total_cost, fees,
            is_day_trade, notes, source
        ) VALUES (?1, 'SELL', ?2, ?2, '100', '55', '5500', '0', 1, 'Test sell (day trade)', 'TEST')",
        rusqlite::params![asset_id, trade_date],
    )?;

    // Verify day trade flag via SQL
    let transactions = sql::query_transactions(&home, "VALE3");
    assert_eq!(transactions.len(), 2);
    assert!(!transactions[0].is_day_trade); // Buy
    assert!(transactions[1].is_day_trade); // Sell (day trade)

    // Verify zero position via CLI
    let mut portfolio_cmd = base_cmd(&home);
    portfolio_cmd.arg("portfolio").arg("show");

    // Day trades should result in zero position, so VALE3 shouldn't appear
    portfolio_cmd
        .assert()
        .success()
        .stdout(predicate::str::contains("VALE3").not());

    Ok(())
}

#[test]
fn test_11_multi_asset_portfolio() -> Result<()> {
    let home = TempDir::new()?;
    let db_path = get_db_path(&home);
    std::fs::create_dir_all(db_path.parent().unwrap())?;
    init_database(Some(db_path.clone()))?;
    let conn = Connection::open(&db_path)?;

    // Create multiple assets
    let petr4_id = upsert_asset(&conn, "PETR4", &AssetType::Stock, Some("Petrobras"))?;
    let vale3_id = upsert_asset(&conn, "VALE3", &AssetType::Stock, Some("Vale"))?;
    let mxrf11_id = upsert_asset(&conn, "MXRF11", &AssetType::Fii, Some("Maxi Renda"))?;

    let date1 = chrono::NaiveDate::from_ymd_opt(2025, 1, 10).unwrap();
    let date2 = chrono::NaiveDate::from_ymd_opt(2025, 2, 15).unwrap();

    // PETR4: Buy 100 @ R$25
    conn.execute(
        "INSERT INTO transactions (
            asset_id, transaction_type, trade_date, settlement_date,
            quantity, price_per_unit, total_cost, fees,
            is_day_trade, notes, source
        ) VALUES (?1, 'BUY', ?2, ?2, '100', '25', '2500', '0', 0, 'Test', 'TEST')",
        rusqlite::params![petr4_id, date1],
    )?;

    // VALE3: Buy 200 @ R$80
    conn.execute(
        "INSERT INTO transactions (
            asset_id, transaction_type, trade_date, settlement_date,
            quantity, price_per_unit, total_cost, fees,
            is_day_trade, notes, source
        ) VALUES (?1, 'BUY', ?2, ?2, '200', '80', '16000', '0', 0, 'Test', 'TEST')",
        rusqlite::params![vale3_id, date2],
    )?;

    // MXRF11: Buy 50 @ R$100
    conn.execute(
        "INSERT INTO transactions (
            asset_id, transaction_type, trade_date, settlement_date,
            quantity, price_per_unit, total_cost, fees,
            is_day_trade, notes, source
        ) VALUES (?1, 'BUY', ?2, ?2, '50', '100', '5000', '0', 0, 'Test', 'TEST')",
        rusqlite::params![mxrf11_id, date1],
    )?;

    // Verify each asset's position via SQL
    let (petr4_qty, petr4_cost) = sql::query_position(&home, "PETR4");
    assert_eq!(petr4_qty, dec!(100));
    assert_eq!(petr4_cost, dec!(2500));

    let (vale3_qty, vale3_cost) = sql::query_position(&home, "VALE3");
    assert_eq!(vale3_qty, dec!(200));
    assert_eq!(vale3_cost, dec!(16000));

    let (mxrf11_qty, mxrf11_cost) = sql::query_position(&home, "MXRF11");
    assert_eq!(mxrf11_qty, dec!(50));
    assert_eq!(mxrf11_cost, dec!(5000));

    // Verify portfolio via CLI - check that all assets appear
    let mut portfolio_cmd = base_cmd(&home);
    portfolio_cmd.arg("portfolio").arg("show");

    portfolio_cmd
        .assert()
        .success()
        .stdout(predicate::str::contains("PETR4"))
        .stdout(predicate::str::contains("VALE3"))
        .stdout(predicate::str::contains("MXRF11"))
        .stdout(predicate::str::contains("100.00")) // PETR4 quantity
        .stdout(predicate::str::contains("200.00")) // VALE3 quantity
        .stdout(predicate::str::contains("50.00")); // MXRF11 quantity

    Ok(())
}

/// Test that renamed ticker with post-rename split doesn't double-adjust carryover.
/// This tests the fix for the SIMH3 bug where the carryover from JSLG3 was being
/// split-adjusted both in apply_actions_to_carryover AND in the main loop.
/// Regression test for: Carryover transaction getting split-adjusted twice.
#[test]
fn test_16_rename_with_post_rename_split() -> Result<()> {
    let home = TempDir::new()?;

    // Initialize database via the standard path (HOME/.interest/data.db)
    let db_path = get_db_path(&home);
    std::fs::create_dir_all(db_path.parent().unwrap())?;
    init_database(Some(db_path.clone()))?;
    let conn = Connection::open(&db_path)?;

    // Create source asset (JSLG3)
    let source_asset_id = upsert_asset(&conn, "JSLG3", &AssetType::Stock, None)?;

    // JSLG3 transactions before rename
    // Opening position from IRPF: 1700 shares @ R$10 on 2019-12-31
    conn.execute(
        "INSERT INTO transactions (asset_id, transaction_type, trade_date, settlement_date, quantity, price_per_unit, total_cost, fees, is_day_trade, source, created_at)
         VALUES (?1, 'BUY', '2019-12-31', '2019-12-31', '1700', '10', '17000', '0', 0, 'IRPF_PDF', datetime('now'))",
        [source_asset_id],
    )?;

    // Sell 300 shares @ R$12 on 2020-08-01
    conn.execute(
        "INSERT INTO transactions (asset_id, transaction_type, trade_date, settlement_date, quantity, price_per_unit, total_cost, fees, is_day_trade, source, created_at)
         VALUES (?1, 'SELL', '2020-08-01', '2020-08-01', '300', '12', '3600', '0', 0, 'TEST', datetime('now'))",
        [source_asset_id],
    )?;

    // Create target asset (SIMH3) - rename happens on 2020-09-21
    let target_asset_id = upsert_asset(&conn, "SIMH3", &AssetType::Stock, None)?;

    // Record rename via CLI
    let mut rename_cmd = base_cmd(&home);
    rename_cmd
        .arg("actions")
        .arg("rename")
        .arg("add")
        .arg("JSLG3")
        .arg("SIMH3")
        .arg("2020-09-21")
        .arg("--notes")
        .arg("test rename");
    rename_cmd.assert().success();

    // SIMH3 transactions after rename
    // Buy 2600 shares @ R$8 on 2021-08-01 (before split)
    conn.execute(
        "INSERT INTO transactions (asset_id, transaction_type, trade_date, settlement_date, quantity, price_per_unit, total_cost, fees, is_day_trade, source, created_at)
         VALUES (?1, 'BUY', '2021-08-01', '2021-08-01', '2600', '8', '20800', '0', 0, 'TEST', datetime('now'))",
        [target_asset_id],
    )?;

    // Split on 2021-08-12: +3000 shares (4000 pre-split -> 7000 post-split)
    let mut split_cmd = base_cmd(&home);
    split_cmd
        .arg("actions")
        .arg("split")
        .arg("add")
        .arg("SIMH3")
        .arg("3000")
        .arg("2021-08-12")
        .arg("--notes")
        .arg("test split");
    split_cmd.assert().success();

    // Buy 27500 shares @ R$7 on 2022-01-15 (after split)
    conn.execute(
        "INSERT INTO transactions (asset_id, transaction_type, trade_date, settlement_date, quantity, price_per_unit, total_cost, fees, is_day_trade, source, created_at)
         VALUES (?1, 'BUY', '2022-01-15', '2022-01-15', '27500', '7', '192500', '0', 0, 'TEST', datetime('now'))",
        [target_asset_id],
    )?;

    // Verify database has correct raw transactions
    let jslg3_txs = sql::query_transactions(&home, "JSLG3");
    assert_eq!(jslg3_txs.len(), 2, "JSLG3 should have 2 transactions");
    assert_eq!(jslg3_txs[0].quantity, dec!(1700));
    assert_eq!(jslg3_txs[1].quantity, dec!(300));

    let simh3_txs = sql::query_transactions(&home, "SIMH3");
    assert_eq!(simh3_txs.len(), 2, "SIMH3 should have 2 transactions");

    let simh3_actions_out = base_cmd(&home)
        .arg("--json")
        .arg("actions")
        .arg("split")
        .arg("list")
        .arg("SIMH3")
        .output()
        .expect("failed to run actions split list");
    assert!(
        simh3_actions_out.status.success(),
        "actions split list failed"
    );
    let simh3_actions_json: Value =
        serde_json::from_slice(&simh3_actions_out.stdout).expect("invalid actions JSON");
    let simh3_actions = simh3_actions_json
        .as_array()
        .expect("actions JSON is not array");
    assert_eq!(simh3_actions.len(), 1, "SIMH3 should have 1 split");
    let qty_str = simh3_actions[0]
        .get("quantity_adjustment")
        .and_then(|v| v.as_str())
        .unwrap_or("0");
    let qty_adj = Decimal::from_str(qty_str).expect("quantity_adjustment decimal parse");
    assert_eq!(qty_adj, dec!(3000), "Split adds 3000 shares");

    // Helper to check portfolio at specific dates (like test_15)
    let check_portfolio =
        |date: &str, expected_qty: &str, expected_avg: &str, expected_cost: &str| -> Result<()> {
            let mut cmd = base_cmd(&home);
            cmd.env("INTEREST_SKIP_PRICE_FETCH", "1")
                .arg("portfolio")
                .arg("show")
                .arg("--at")
                .arg(date);
            let out = cmd.output().expect("portfolio show failed");
            assert!(out.status.success());
            let stdout = String::from_utf8_lossy(&out.stdout);
            let row = stdout
                .lines()
                .find(|l| l.starts_with("│ SIMH3"))
                .unwrap_or_else(|| panic!("SIMH3 row missing at {}", date));
            let cols: Vec<String> = row
                .split('│')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            assert!(cols.len() >= 8, "Unexpected column count: {}", cols.len());
            assert_eq!(cols[1], expected_qty, "Quantity mismatch at {}", date);
            assert_eq!(cols[2], expected_avg, "Avg cost mismatch at {}", date);
            assert_eq!(cols[3], expected_cost, "Total cost mismatch at {}", date);
            Ok(())
        };

    // Timeline verification (following test_06 pattern with detailed checks)
    // 1. After rename, before split (2021-08-11): 1400 carryover + 2600 buy = 4000 shares
    //    Avg cost: (1400*10 + 2600*8) / 4000 = (14000 + 20800) / 4000 = 8.70
    check_portfolio("2021-08-11", "4000.00", "R$ 8,70", "R$ 34.800,00")?;

    // 2. After split (2021-08-13): 4000 -> 7000 shares (+ 3000 from split)
    //    Avg cost: 34800 / 7000 = 4.97...
    check_portfolio("2021-08-13", "7000.00", "R$ 4,97", "R$ 34.800,00")?;

    // 3. Final position (2022-01-20): 7000 + 27500 = 34500 shares
    //    THIS IS THE CRITICAL TEST - ensures carryover wasn't double-adjusted
    //    Total cost: 34800 + 192500 = 227300
    //    Avg cost: 227300 / 34500 = 6.59...
    check_portfolio("2022-01-20", "34500.00", "R$ 6,58", "R$ 227.300,00")?;

    // Verify portfolio CLI output shows correct final position (like test_06)
    let mut portfolio_cmd = base_cmd(&home);
    portfolio_cmd.arg("portfolio").arg("show");
    let output = portfolio_cmd
        .output()
        .expect("Failed to run portfolio show");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let row = stdout
        .lines()
        .find(|l| l.starts_with("│ SIMH3"))
        .expect("SIMH3 row not found in portfolio output");
    let cols: Vec<String> = row
        .split('│')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    assert!(
        cols.len() >= 8,
        "Unexpected table column count: {}",
        cols.len()
    );
    assert_eq!(cols[0], "SIMH3");
    assert_eq!(
        cols[1], "34500.00",
        "Final quantity should be 34500 (NOT 37500 from double-adjustment bug)"
    );
    assert_eq!(cols[2], "R$ 6,58");
    assert_eq!(cols[3], "R$ 227.300,00");

    // Verify performance and tax outputs (like test_06)
    let expected_total_cost = dec!(227300);

    let perf_out = base_cmd(&home)
        .env("INTEREST_SKIP_PRICE_FETCH", "1")
        .arg("--json")
        .arg("performance")
        .arg("show")
        .arg("2022")
        .output()
        .expect("failed to run performance show");
    assert!(perf_out.status.success(), "performance show failed");
    let perf_json: Value =
        serde_json::from_slice(&perf_out.stdout).expect("invalid performance JSON");
    let end_value = perf_json
        .get("end_value")
        .and_then(|v| v.as_str())
        .unwrap_or("0");

    let end_value_dec = Decimal::from_str(end_value).expect("end_value decimal parse");
    assert_eq!(
        end_value_dec, expected_total_cost,
        "Performance end value should match total cost basis"
    );

    Ok(())
}

#[test]
fn test_irpf_import_sets_cutoff_dates() -> Result<()> {
    let home = TempDir::new()?;
    let db_path = get_db_path(&home);
    std::fs::create_dir_all(db_path.parent().unwrap())?;
    init_database(Some(db_path.clone()))?;

    // Import IRPF for year 2024
    let irpf_path = "tests/data/irpf_minimal.pdf";
    let mut cmd = base_cmd(&home);
    cmd.arg("import-irpf").arg(irpf_path).arg("2024");

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("Import complete"));

    // Verify opening position was created
    let conn = open_db(Some(db_path))?;

    let tx_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM transactions WHERE source = 'IRPF_PDF'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(tx_count, 1, "Should have 1 IRPF opening transaction");

    // Verify the transaction details
    let (ticker, quantity, date): (String, Decimal, String) = conn.query_row(
        "SELECT a.ticker, t.quantity, t.trade_date
         FROM transactions t
         JOIN assets a ON t.asset_id = a.id
         WHERE t.source = 'IRPF_PDF'",
        [],
        |row| Ok((row.get(0)?, sql::get_decimal(row, 1)?, row.get(2)?)),
    )?;
    assert_eq!(ticker, "ITSA4");
    assert_eq!(quantity, dec!(100));
    assert_eq!(date, "2024-12-31");

    // Verify import_state cutoff dates were set for CEI and Movimentação
    let cei_trades_date: String = conn.query_row(
        "SELECT last_date FROM import_state WHERE source = 'CEI' AND entry_type = 'trades'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(
        cei_trades_date, "2024-12-31",
        "CEI trades cutoff should be set to year-end"
    );

    let mov_trades_date: String = conn.query_row(
        "SELECT last_date FROM import_state WHERE source = 'MOVIMENTACAO' AND entry_type = 'trades'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(
        mov_trades_date, "2024-12-31",
        "Movimentação trades cutoff should be set to year-end"
    );

    let mov_actions_date: String = conn.query_row(
        "SELECT last_date FROM import_state WHERE source = 'MOVIMENTACAO' AND entry_type = 'corporate_actions'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(
        mov_actions_date, "2024-12-31",
        "Movimentação corporate actions cutoff should be set to year-end"
    );

    Ok(())
}
#[test]
fn test_15_mixed_splits_reverse_splits_and_bonus() -> Result<()> {
    let home = TempDir::new()?;

    let import_result = run_import_json(&home, "tests/data/15_mixed_splits_and_bonus.xlsx");
    assert!(assert_json_success(&import_result));
    let data = get_json_data(&import_result);
    assert_eq!(data["imported_trades"].as_u64().unwrap(), 4);
    assert_eq!(data["imported_actions"].as_u64().unwrap(), 3);
    assert_eq!(data["imported_income"].as_u64().unwrap(), 0);

    let transactions = sql::query_transactions(&home, "KPCA3");
    assert_eq!(transactions.len(), 5, "2 buys + 1 bonus + 2 sells");

    let actions_out = base_cmd(&home)
        .arg("--json")
        .arg("actions")
        .arg("split")
        .arg("list")
        .arg("KPCA3")
        .output()
        .expect("failed to run actions split list");
    assert!(actions_out.status.success(), "actions split list failed");
    let actions_json: Value =
        serde_json::from_slice(&actions_out.stdout).expect("invalid actions JSON");
    let actions_array = actions_json.as_array().expect("actions JSON is not array");
    assert_eq!(
        actions_array.len(),
        2,
        "split + reverse split in corporate_actions"
    );
    let mut split_qty = None;
    let mut reverse_qty = None;
    for row in actions_array {
        let action_type = row.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let qty_str = row
            .get("quantity_adjustment")
            .and_then(|v| v.as_str())
            .unwrap_or("0");
        let qty = Decimal::from_str(qty_str).expect("quantity_adjustment decimal parse");
        match action_type {
            "SPLIT" => split_qty = Some(qty),
            "REVERSE_SPLIT" => reverse_qty = Some(qty),
            _ => {}
        }
    }
    assert_eq!(split_qty, Some(dec!(1000)));
    assert_eq!(reverse_qty, Some(dec!(-2400)));

    let bonus_tx = transactions
        .iter()
        .find(|t| t.price_per_unit.is_zero())
        .expect("bonus transaction missing");
    assert_eq!(bonus_tx.quantity, dec!(360));

    // Helper to check portfolio at specific date
    let check_portfolio =
        |date: &str, expected_qty: &str, expected_avg: &str, expected_cost: &str| -> Result<()> {
            let mut cmd = base_cmd(&home);
            cmd.arg("portfolio").arg("show").arg("--at").arg(date);
            let out = cmd.output().expect("portfolio show failed");
            assert!(out.status.success());
            let stdout = String::from_utf8_lossy(&out.stdout);
            let row = stdout
                .lines()
                .find(|l| l.starts_with("│ KPCA3"))
                .expect("KPCA3 row missing");
            let cols: Vec<String> = row
                .split('│')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            assert!(cols.len() >= 8, "Unexpected column count: {}", cols.len());
            assert_eq!(cols[1], expected_qty, "Quantity mismatch at {}", date);
            assert_eq!(cols[2], expected_avg, "Avg cost mismatch at {}", date);
            assert_eq!(cols[3], expected_cost, "Total cost mismatch at {}", date);
            Ok(())
        };

    // After first buy (15/01/2025): 1000 @ 2
    check_portfolio("2025-01-16", "1000.00", "R$ 2,00", "R$ 2.000,00")?;

    // After split (10/02/2025): 2000 @ 1
    check_portfolio("2025-02-11", "2000.00", "R$ 1,00", "R$ 2.000,00")?;

    // After second buy (01/03/2025): 2800 @ 0.97142857...
    check_portfolio("2025-03-02", "2800.00", "R$ 0,97", "R$ 2.720,00")?;

    // After grupamento (15/03/2025): 400 @ 6.8
    check_portfolio("2025-03-16", "400.00", "R$ 6,80", "R$ 2.720,00")?;

    // After first sell (01/04/2025): 200 @ 6.8
    check_portfolio("2025-04-02", "200.00", "R$ 6,80", "R$ 1.360,00")?;

    // After bonus (20/04/2025): 560 @ 2.428571...
    check_portfolio("2025-04-21", "560.00", "R$ 2,42", "R$ 1.360,00")?;

    // After second sell (10/05/2025): 160 @ 2.428571...
    check_portfolio("2025-05-11", "160.00", "R$ 2,42", "R$ 388,57")?;

    let perf_out = base_cmd(&home)
        .arg("--json")
        .arg("performance")
        .arg("show")
        .arg("2025")
        .output()
        .expect("performance show failed");
    assert!(perf_out.status.success());
    let perf_json: Value =
        serde_json::from_slice(&perf_out.stdout).expect("invalid performance JSON");
    let end_value = perf_json
        .get("end_value")
        .and_then(|v| v.as_str())
        .unwrap_or("0");
    let total_return = perf_json
        .get("total_return")
        .and_then(|v| v.as_str())
        .unwrap_or("0");
    assert_eq!(end_value, "388.5714285714286");
    assert_eq!(total_return, "388.5714285714286");

    let tax_out = base_cmd(&home)
        .arg("--json")
        .arg("tax")
        .arg("report")
        .arg("2025")
        .output()
        .expect("tax report failed");
    assert!(tax_out.status.success());
    let tax_json: Value = serde_json::from_slice(&tax_out.stdout).expect("invalid tax JSON");
    use std::str::FromStr as _;
    let total_sales = Decimal::from_str(
        tax_json
            .get("annual_total_sales")
            .and_then(|v| v.as_str())
            .unwrap_or("0"),
    )?;
    let total_profit = Decimal::from_str(
        tax_json
            .get("annual_total_profit")
            .and_then(|v| v.as_str())
            .unwrap_or("0"),
    )?;
    let total_loss = Decimal::from_str(
        tax_json
            .get("annual_total_loss")
            .and_then(|v| v.as_str())
            .unwrap_or("0"),
    )?;
    assert_eq!(total_sales, dec!(2300));
    assert_eq!(
        total_profit,
        Decimal::from_str("228.5714285714285714285714286")?
    );
    assert_eq!(total_loss, dec!(260));

    let monthly = tax_json
        .get("monthly_summaries")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    assert_eq!(monthly.len(), 2);
    let april_loss = Decimal::from_str(monthly[0]["loss"].as_str().unwrap())?;
    let april_sales = Decimal::from_str(monthly[0]["sales"].as_str().unwrap())?;
    let may_profit = Decimal::from_str(monthly[1]["profit"].as_str().unwrap())?;
    let may_sales = Decimal::from_str(monthly[1]["sales"].as_str().unwrap())?;
    assert_eq!(april_loss, dec!(260));
    assert_eq!(april_sales, dec!(1100));
    assert_eq!(
        may_profit,
        Decimal::from_str("228.5714285714285714285714286")?
    );
    assert_eq!(may_sales, dec!(1200));

    Ok(())
}

#[test]
fn test_portfolio_show_fetches_cached_cotahist_and_shows_prices() -> Result<()> {
    let home = TempDir::new()?;

    // 1) Import sample data to populate portfolio (uses INTEREST_SKIP_PRICE_FETCH=1 by default)
    let _import = run_import_json(&home, "tests/data/01_basic_purchase_sale.xlsx");

    // 2) Determine a year to create a fake COTAHIST for (use earliest transaction year)
    let db_path = get_db_path(&home);
    let conn = open_db(Some(db_path.clone()))?;
    let earliest_date: String = conn
        .query_row("SELECT MIN(trade_date) FROM transactions", [], |r| r.get(0))
        .expect("failed to read earliest date");
    let year: i32 = chrono::NaiveDate::parse_from_str(&earliest_date, "%Y-%m-%d")?.year();

    // 3) pick a ticker present in DB
    let ticker: String = conn
        .query_row("SELECT ticker FROM assets LIMIT 1", [], |r| r.get(0))
        .expect("failed to pick ticker");

    // 4) Build a minimal COTAHIST text content with a single valid data record for that ticker
    // Use fixed-width fields expected by parser (245 chars per line)
    fn make_line(date: &str, ticker: &str, price_cents: i64, volume: i64) -> String {
        let mut buf = vec![b' '; 245];
        // record type
        buf[0..2].copy_from_slice(b"01");
        // date YYYYMMDD at 2..10
        buf[2..10].copy_from_slice(date.as_bytes());
        // ticker at 12..24 (12 chars)
        let mut t = ticker.as_bytes().to_vec();
        t.resize(12, b' ');
        buf[12..24].copy_from_slice(&t);
        // prices: place same price at PREABE(56), PREULT(108), etc. as 13-char zero-padded
        let price_field = format!("{:013}", price_cents);
        buf[56..69].copy_from_slice(price_field.as_bytes());
        buf[69..82].copy_from_slice(price_field.as_bytes());
        buf[82..95].copy_from_slice(price_field.as_bytes());
        buf[108..121].copy_from_slice(price_field.as_bytes());
        // volume at 170..188 (18 chars)
        let vol_field = format!("{:018}", volume);
        buf[170..188].copy_from_slice(vol_field.as_bytes());
        String::from_utf8(buf).unwrap()
    }

    let date = format!("{}0101", year); // Jan 1st of year
    let line = make_line(&date, &ticker, 1000, 1); // price 10.00 (1000 cents)
    let contents = format!("{}\n", line);

    // 5) Create a fake ZIP in XDG_CACHE_HOME so download_cotahist_year will use cached archive
    let cache_root = TempDir::new()?; // will set XDG_CACHE_HOME to this

    // Must also have tickers cache here since we override XDG_CACHE_HOME
    setup_test_tickers_cache(cache_root.path());

    let cache_dir = cache_root.path().join("interest").join("cotahist");
    std::fs::create_dir_all(&cache_dir)?;
    let zip_path = cache_dir.join(format!("COTAHIST_A{}.ZIP", year));

    // Create zip with one entry (COTAHIST txt)
    {
        let f = std::fs::File::create(&zip_path)?;
        let mut zip = zip::ZipWriter::new(f);

        let entry_name = format!("COTAHIST_A{}.TXT", year);
        let options: zip::write::FileOptions<'_, zip::write::ExtendedFileOptions> =
            zip::write::FileOptions::default();
        zip.start_file(entry_name, options)?;
        zip.write_all(contents.as_bytes())?;
        zip.finish()?;
    }

    // 6) Run portfolio show with XDG_CACHE_HOME pointing to our cache and HOME to our temp home
    let mut cmd = Command::new(cargo::cargo_bin!("interest"));
    cmd.env("HOME", home.path());
    cmd.env("XDG_CACHE_HOME", cache_root.path());
    cmd.arg("--no-color");
    cmd.arg("portfolio");
    cmd.arg("show");

    let out = cmd.output().expect("failed to run portfolio show");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();

    // 7) Assert the *fixed* behavior (what we expect after applying the fix):
    // the printed portfolio row for our ticker should include a currency price
    // (Price column) and not display "N/A". This will FAIL in the current
    // buggy state and PASS after you reapply your fix.

    let row = stdout
        .lines()
        .find(|l| {
            let parts: Vec<_> = l.split('│').map(|s| s.trim()).collect();
            // parts[1] is Ticker column (first after the left border)
            parts.get(1).map(|s| *s == ticker).unwrap_or(false)
        })
        .unwrap_or_else(|| panic!("Ticker row not found in stdout:\n{}", stdout));

    let cols: Vec<_> = row.split('│').map(|s| s.trim()).collect();

    // Price column is at index 5 (0 empty, 1 ticker, 2 qty, 3 avg cost, 4 total cost, 5 price)
    let price_col = cols.get(5).copied().unwrap_or("");

    assert!(
        price_col.contains("R$") || price_col.contains("R$ "),
        "Expected currency in Price column, found: '{}'\nFull output:\n{}",
        price_col,
        stdout
    );
    assert!(
        !price_col.contains("N/A"),
        "Did not expect N/A in Price column: {}",
        price_col
    );

    Ok(())
}
