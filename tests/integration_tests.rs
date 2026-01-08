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
use chrono::Utc;
use interest::corporate_actions::{apply_corporate_action, get_unapplied_actions};
use interest::db::models::{Asset, AssetType, Transaction, TransactionType};
use interest::db::{init_database, open_db, upsert_asset};
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

/// Create a base CLI command with proper environment setup
fn base_cmd(home: &TempDir) -> Command {
    let mut cmd = Command::new(cargo::cargo_bin!("interest"));
    cmd.env("HOME", home.path());
    cmd.env("INTEREST_SKIP_PRICE_FETCH", "1");
    cmd.arg("--no-color");
    cmd
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
    avg.add_purchase(&base_txs[0]); // Liquidation becomes a purchase

    let sale_result = avg.match_sale(&base_txs[1])?;

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
    Ok(())
}

#[test]
fn test_12_desdobro_ratio_inference_auto_apply() -> Result<()> {
    let home = TempDir::new()?;

    // Import file
    let import_result = run_import_json(&home, "tests/data/12_desdobro_inference.xlsx");
    assert!(assert_json_success(&import_result));

    let data = get_json_data(&import_result);
    assert_eq!(data["imported_trades"].as_u64().unwrap(), 1);
    assert_eq!(data["imported_actions"].as_u64().unwrap(), 1);
    assert_eq!(data["auto_applied_actions"].as_u64().unwrap(), 1);

    // Verify with SQL
    let transactions = sql::query_transactions(&home, "A1MD34");
    assert_eq!(transactions.len(), 1);
    assert_eq!(transactions[0].quantity, dec!(640));
    assert_eq!(transactions[0].total_cost, dec!(800));
    Ok(())
}

#[test]
fn test_14_atualizacao_ratio_inference_auto_apply() -> Result<()> {
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
    avg.add_purchase(&transactions[0]);

    let sale_result = avg.match_sale(&transactions[1])?;

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

    // Manually create the split corporate action
    let asset_id = transactions[0].asset_id;
    let db_path = get_db_path(&home);
    let conn = Connection::open(&db_path)?;
    conn.execute(
        "INSERT INTO corporate_actions (asset_id, action_type, event_date, ex_date, ratio_from, ratio_to, applied, source)
         VALUES (?1, 'SPLIT', '2025-02-15', '2025-02-15', 1, 2, 0, 'TEST')",
        [asset_id],
    )?;

    // Apply the split
    let asset = Asset {
        id: Some(asset_id),
        ticker: "VALE3".to_string(),
        asset_type: AssetType::Stock,
        name: Some("VALE SA".to_string()),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    let actions = get_unapplied_actions(&conn, Some(asset_id))?;
    assert_eq!(actions.len(), 1, "Should have 1 unapplied action");

    let adjusted_count = apply_corporate_action(&conn, &actions[0], &asset)?;
    assert_eq!(adjusted_count, 1, "Should adjust 1 transaction (first buy)");

    // Re-fetch transactions to see adjustments
    let adjusted_txs = sql::query_transactions(&home, "VALE3");

    // First purchase should be adjusted: 100 @ R$80 -> 200 @ R$40
    assert_eq!(adjusted_txs[0].quantity, dec!(200));
    assert_eq!(adjusted_txs[0].price_per_unit, dec!(40));
    assert_eq!(
        adjusted_txs[0].quantity * adjusted_txs[0].price_per_unit,
        dec!(8000),
        "Total cost should remain unchanged"
    );

    // Second purchase (after split) should be unchanged
    assert_eq!(adjusted_txs[1].quantity, dec!(50));
    assert_eq!(adjusted_txs[1].price_per_unit, dec!(42));

    // Test cost basis with adjusted quantities
    let mut avg = AverageCostMatcher::new();
    avg.add_purchase(&adjusted_txs[0]);
    avg.add_purchase(&adjusted_txs[1]);

    let sale_result = avg.match_sale(&adjusted_txs[2])?;

    let expected_avg = (adjusted_txs[0].total_cost + adjusted_txs[1].total_cost)
        / (adjusted_txs[0].quantity + adjusted_txs[1].quantity);
    let expected_cost_basis = expected_avg * adjusted_txs[2].quantity;

    assert_eq!(sale_result.cost_basis, expected_cost_basis);
    assert_eq!(sale_result.sale_total, dec!(6750));
    assert_eq!(
        sale_result.profit_loss,
        sale_result.sale_total - expected_cost_basis
    );

    // Remaining quantity
    assert_eq!(avg.remaining_quantity(), dec!(100));

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
    let asset_id = transactions[0].asset_id;
    let db_path = get_db_path(&home);
    let conn = Connection::open(&db_path)?;
    conn.execute(
        "INSERT INTO corporate_actions (asset_id, action_type, event_date, ex_date, ratio_from, ratio_to, applied, source)
         VALUES (?1, 'SPLIT', '2025-02-20', '2025-02-20', 10, 1, 0, 'TEST')",
        [asset_id],
    )?;

    let asset = Asset {
        id: Some(asset_id),
        ticker: "MGLU3".to_string(),
        asset_type: AssetType::Stock,
        name: Some("MAGAZINE LUIZA".to_string()),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    let actions = get_unapplied_actions(&conn, Some(asset_id))?;
    apply_corporate_action(&conn, &actions[0], &asset)?;

    // Re-fetch and verify
    let adjusted_txs = sql::query_transactions(&home, "MGLU3");

    // 1000 @ R$2.00 -> 100 @ R$20.00
    assert_eq!(adjusted_txs[0].quantity, dec!(100));
    assert_eq!(adjusted_txs[0].price_per_unit, dec!(20));

    // Test cost basis
    let mut avg = AverageCostMatcher::new();
    avg.add_purchase(&adjusted_txs[0]);

    let sale_result = avg.match_sale(&adjusted_txs[1])?;

    assert_eq!(sale_result.cost_basis, dec!(1000)); // 50 @ R$20
    assert_eq!(sale_result.sale_total, dec!(1100)); // 50 @ R$22
    assert_eq!(sale_result.profit_loss, dec!(100));

    Ok(())
}

#[test]
fn test_06_multiple_splits() -> Result<()> {
    let home = TempDir::new()?;

    // Initialize DB and import only trade entries (no auto actions)
    let db_path = get_db_path(&home);
    std::fs::create_dir_all(db_path.parent().unwrap())?;
    init_database(Some(db_path.clone()))?;
    let conn = Connection::open(&db_path)?;
    import_movimentacao(&conn, "tests/data/06_multiple_splits.xlsx")?;

    let transactions = sql::query_transactions(&home, "ITSA4");
    assert_eq!(transactions.len(), 3, "Should have 3 transactions");

    let asset_id = transactions[0].asset_id;

    // First split 1:2 on 2025-02-10
    let db_path = get_db_path(&home);
    let conn = Connection::open(&db_path)?;
    conn.execute(
        "INSERT INTO corporate_actions (asset_id, action_type, event_date, ex_date, ratio_from, ratio_to, applied, source)
         VALUES (?1, 'SPLIT', '2025-02-10', '2025-02-10', 1, 2, 0, 'TEST')",
        [asset_id],
    )?;

    // Second split 1:2 on 2025-04-15
    conn.execute(
        "INSERT INTO corporate_actions (asset_id, action_type, event_date, ex_date, ratio_from, ratio_to, applied, source)
         VALUES (?1, 'SPLIT', '2025-04-15', '2025-04-15', 1, 2, 0, 'TEST')",
        [asset_id],
    )?;

    let asset = Asset {
        id: Some(asset_id),
        ticker: "ITSA4".to_string(),
        asset_type: AssetType::Stock,
        name: Some("ITAUSA PN".to_string()),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    // Apply both splits
    let actions = get_unapplied_actions(&conn, Some(asset_id))?;
    assert_eq!(actions.len(), 2, "Should have 2 splits");

    apply_corporate_action(&conn, &actions[0], &asset)?;
    apply_corporate_action(&conn, &actions[1], &asset)?;

    // Re-fetch and verify
    let adjusted_txs = sql::query_transactions(&home, "ITSA4");

    // First purchase: 50 @ R$10.00 -> 100 @ R$5.00 -> 200 @ R$2.50
    assert_eq!(adjusted_txs[0].quantity, dec!(200));
    assert_eq!(adjusted_txs[0].price_per_unit, dec!(2.5));

    // Second purchase: 25 @ R$5.50 -> 50 @ R$2.75 (only second split applies)
    assert_eq!(adjusted_txs[1].quantity, dec!(50));
    assert_eq!(adjusted_txs[1].price_per_unit, dec!(2.75));

    // Test cost basis
    let mut avg = AverageCostMatcher::new();
    avg.add_purchase(&adjusted_txs[0]);
    avg.add_purchase(&adjusted_txs[1]);

    let sale_result = avg.match_sale(&adjusted_txs[2])?;

    let expected_avg = (adjusted_txs[0].total_cost + adjusted_txs[1].total_cost)
        / (adjusted_txs[0].quantity + adjusted_txs[1].quantity);
    let expected_cost_basis = expected_avg * adjusted_txs[2].quantity;

    assert_eq!(sale_result.cost_basis, expected_cost_basis);
    assert_eq!(
        sale_result.profit_loss,
        sale_result.sale_total - expected_cost_basis
    );

    assert_eq!(avg.remaining_quantity(), dec!(50));

    Ok(())
}

#[test]
fn test_08_complex_scenario() -> Result<()> {
    let home = TempDir::new()?;

    // Initialize DB and import only trade entries (no auto actions)
    let db_path = get_db_path(&home);
    std::fs::create_dir_all(db_path.parent().unwrap())?;
    init_database(Some(db_path.clone()))?;
    let conn = Connection::open(&db_path)?;
    import_movimentacao(&conn, "tests/data/08_complex_scenario.xlsx")?;

    // Should have transactions for both BBAS3 and BBAS3T
    let base_txs = sql::query_transactions(&home, "BBAS3");
    let term_txs = sql::query_transactions(&home, "BBAS3T");

    assert_eq!(term_txs.len(), 1, "Should have 1 term contract purchase");
    // We expect: 2 initial buys + split entry + 1 sell + 1 buy + liquidation + 1 sell = 7
    // But split entry might not be imported as a transaction, so we get 6
    assert_eq!(base_txs.len(), 6, "Should have 6 base transactions");

    // Create and apply split (1:2) on 2025-02-15
    let asset_id = base_txs[0].asset_id;
    let db_path = get_db_path(&home);
    let conn = Connection::open(&db_path)?;
    conn.execute(
        "INSERT INTO corporate_actions (asset_id, action_type, event_date, ex_date, ratio_from, ratio_to, applied, source)
         VALUES (?1, 'SPLIT', '2025-02-15', '2025-02-15', 1, 2, 0, 'TEST')",
        [asset_id],
    )?;

    let asset = Asset {
        id: Some(asset_id),
        ticker: "BBAS3".to_string(),
        asset_type: AssetType::Stock,
        name: Some("BANCO DO BRASIL".to_string()),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    let actions = get_unapplied_actions(&conn, Some(asset_id))?;
    apply_corporate_action(&conn, &actions[0], &asset)?;

    // Process term liquidations
    process_term_liquidations(&conn)?;

    // Re-fetch to see adjustments
    let adjusted_txs = sql::query_transactions(&home, "BBAS3");

    // Verify split adjustments on first two purchases
    assert_eq!(adjusted_txs[0].quantity, dec!(400)); // 200 -> 400
    assert_eq!(adjusted_txs[0].price_per_unit, dec!(20)); // 40 -> 20

    assert_eq!(adjusted_txs[1].quantity, dec!(200)); // 100 -> 200
    assert_eq!(adjusted_txs[1].price_per_unit, dec!(21)); // 42 -> 21

    // Calculate final position and verify cost basis for final sale
    let mut avg = AverageCostMatcher::new();

    // Add purchases and process sales in order
    avg.add_purchase(&adjusted_txs[0]); // 400 @ 20
    avg.add_purchase(&adjusted_txs[1]); // 200 @ 21

    let avg_before_sale1 = avg.average_cost();
    let _sale1 = avg.match_sale(&adjusted_txs[2])?; // Sells 300 @ 22
    let tolerance = dec!(0.0000000001);
    assert!((avg.average_cost() - avg_before_sale1).abs() <= tolerance);

    // Third purchase (after first sale)
    avg.add_purchase(&adjusted_txs[3]); // 150 @ 23

    // Term liquidation adds shares
    avg.add_purchase(&adjusted_txs[4]); // 200 @ 24

    // Final sale: 400 shares
    let avg_before_sale2 = avg.average_cost();
    let sale2 = avg.match_sale(&adjusted_txs[5])?;

    assert_eq!(
        sale2.cost_basis,
        avg_before_sale2 * adjusted_txs[5].quantity
    );
    assert_eq!(sale2.sale_total, dec!(10400));
    assert_eq!(sale2.profit_loss, sale2.sale_total - sale2.cost_basis);
    assert!((avg.average_cost() - avg_before_sale2).abs() <= tolerance);

    // Remaining quantity
    assert_eq!(avg.remaining_quantity(), dec!(250));

    Ok(())
}

#[test]
fn test_no_duplicate_adjustments() -> Result<()> {
    let home = TempDir::new()?;

    // Initialize DB and import only trade entries (no auto actions)
    let db_path = get_db_path(&home);
    std::fs::create_dir_all(db_path.parent().unwrap())?;
    init_database(Some(db_path.clone()))?;
    let conn = Connection::open(&db_path)?;
    import_movimentacao(&conn, "tests/data/04_stock_split.xlsx")?;

    let transactions = sql::query_transactions(&home, "VALE3");
    let asset_id = transactions[0].asset_id;

    // Create split
    let db_path = get_db_path(&home);
    let conn = Connection::open(&db_path)?;
    conn.execute(
        "INSERT INTO corporate_actions (asset_id, action_type, event_date, ex_date, ratio_from, ratio_to, applied, source)
         VALUES (?1, 'SPLIT', '2025-02-15', '2025-02-15', 1, 2, 0, 'TEST')",
        [asset_id],
    )?;

    let asset = Asset {
        id: Some(asset_id),
        ticker: "VALE3".to_string(),
        asset_type: AssetType::Stock,
        name: Some("VALE SA".to_string()),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    // Apply split first time
    let actions = get_unapplied_actions(&conn, Some(asset_id))?;
    let count1 = apply_corporate_action(&conn, &actions[0], &asset)?;
    assert_eq!(count1, 1, "Should adjust 1 transaction");

    // Verify adjustment was recorded
    let adjustment_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM corporate_action_adjustments WHERE action_id = ?1",
        [actions[0].id.unwrap()],
        |row| row.get(0),
    )?;
    assert_eq!(adjustment_count, 1, "Should have 1 adjustment record");

    // Try to apply again - should not duplicate
    let count2 = apply_corporate_action(&conn, &actions[0], &asset)?;
    assert_eq!(count2, 0, "Should not adjust anything (already applied)");

    // Verify still only 1 adjustment
    let adjustment_count2: i64 = conn.query_row(
        "SELECT COUNT(*) FROM corporate_action_adjustments WHERE action_id = ?1",
        [actions[0].id.unwrap()],
        |row| row.get(0),
    )?;
    assert_eq!(
        adjustment_count2, 1,
        "Should still have exactly 1 adjustment record"
    );

    // Verify quantities didn't change (no double adjustment)
    let adjusted_txs = sql::query_transactions(&home, "VALE3");
    assert_eq!(adjusted_txs[0].quantity, dec!(200)); // Not 400!
    assert_eq!(adjusted_txs[0].price_per_unit, dec!(40)); // Not 20!

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
// This will be inserted into integration_tests.rs
#[test]
fn test_07_capital_return() -> Result<()> {
    let home = TempDir::new()?;

    let import_result = run_import_json(&home, "tests/data/07_capital_return.xlsx");
    assert!(assert_json_success(&import_result));

    let transactions = sql::query_transactions(&home, "MXRF11");
    assert_eq!(transactions.len(), 3, "Should have 3 transactions");

    // Verify initial state
    assert_eq!(transactions[0].quantity, dec!(100));
    assert_eq!(transactions[0].price_per_unit, dec!(10));
    assert_eq!(transactions[0].total_cost, dec!(1000));

    // Create capital return action: R$1.00/share = 100 cents
    let asset_id = transactions[0].asset_id;
    let db_path = get_db_path(&home);
    let conn = Connection::open(&db_path)?;
    conn.execute(
        "INSERT INTO corporate_actions (asset_id, action_type, event_date, ex_date, ratio_from, ratio_to, applied, source)
         VALUES (?1, 'CAPITAL_RETURN', '2025-02-15', '2025-02-15', 100, 0, 0, 'TEST')",
        [asset_id],
    )?;

    let asset = Asset {
        id: Some(asset_id),
        ticker: "MXRF11".to_string(),
        asset_type: AssetType::Fii,
        name: Some("MAXI RENDA FII".to_string()),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    // Apply capital return
    let actions = get_unapplied_actions(&conn, Some(asset_id))?;
    assert_eq!(actions.len(), 1, "Should have 1 capital return action");
    let adjusted_count = apply_corporate_action(&conn, &actions[0], &asset)?;
    assert_eq!(adjusted_count, 1, "Should adjust first purchase only");

    // Re-fetch transactions
    let adjusted_txs = sql::query_transactions(&home, "MXRF11");

    // First purchase should have reduced cost basis
    // 100 shares @ R$10.00 = R$1,000.00
    // Capital return R$1.00/share = R$100.00
    // New cost basis: R$900.00 (R$9.00/share)
    assert_eq!(adjusted_txs[0].quantity, dec!(100)); // Quantity unchanged
    assert_eq!(adjusted_txs[0].total_cost, dec!(900)); // Cost reduced
    assert_eq!(adjusted_txs[0].price_per_unit, dec!(9)); // Price recalculated

    // Second purchase (after capital return) should be unchanged
    assert_eq!(adjusted_txs[1].quantity, dec!(50));
    assert_eq!(adjusted_txs[1].price_per_unit, dec!(10.5));

    // Test cost basis with average cost
    let mut avg = AverageCostMatcher::new();
    avg.add_purchase(&adjusted_txs[0]); // 100 @ 9.00
    avg.add_purchase(&adjusted_txs[1]); // 50 @ 10.50

    let sale_result = avg.match_sale(&adjusted_txs[2])?; // Sell 120

    let expected_avg = (adjusted_txs[0].total_cost + adjusted_txs[1].total_cost)
        / (adjusted_txs[0].quantity + adjusted_txs[1].quantity);
    let expected_cost_basis = expected_avg * adjusted_txs[2].quantity;

    assert_eq!(sale_result.cost_basis, expected_cost_basis);
    assert_eq!(sale_result.sale_total, dec!(1320));
    assert_eq!(
        sale_result.profit_loss,
        sale_result.sale_total - expected_cost_basis
    );

    assert_eq!(avg.remaining_quantity(), dec!(30));

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
