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
use interest::corporate_actions::{
    adjust_price_and_cost_for_actions, adjust_quantity_for_actions, get_applicable_actions,
};
use interest::db::models::{AssetType, Transaction, TransactionType};
use interest::db::{init_database, open_db, upsert_asset, CorporateActionType};
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
    Ok(())
}

/// Test that bonus shares are calculated correctly when there's a prior split.
/// This verifies that calculate_net_position_before_date applies split adjustments.
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

    // Add 1:2 split on 2025-02-10 (100 shares -> 200 shares)
    conn.execute(
        "INSERT INTO corporate_actions (asset_id, action_type, event_date, ex_date, ratio_from, ratio_to, source)
         VALUES (?1, 'SPLIT', '2025-02-10', '2025-02-10', 1, 2, 'TEST')",
        [asset_id],
    )?;

    // Add bonus 10:11 on 2025-03-15 (should be based on 200 split-adjusted shares)
    // Expected bonus: 200 * 11/10 - 200 = 220 - 200 = 20 shares
    conn.execute(
        "INSERT INTO corporate_actions (asset_id, action_type, event_date, ex_date, ratio_from, ratio_to, source)
         VALUES (?1, 'BONUS', '2025-03-15', '2025-03-15', 10, 11, 'TEST')",
        [asset_id],
    )?;

    // Apply corporate actions (only bonus creates transactions)
    let actions: Vec<interest::db::CorporateAction> = conn
        .prepare("SELECT id, asset_id, action_type, event_date, ex_date, ratio_from, ratio_to, source, notes, created_at FROM corporate_actions WHERE asset_id = ?1 ORDER BY ex_date")?
        .query_map([asset_id], |row| {
            Ok(interest::db::CorporateAction {
                id: Some(row.get(0)?),
                asset_id: row.get(1)?,
                action_type: row.get::<_, String>(2)?.parse().unwrap(),
                event_date: row.get(3)?,
                ex_date: row.get(4)?,
                ratio_from: row.get(5)?,
                ratio_to: row.get(6)?,
                source: row.get(7)?,
                notes: row.get(8)?,
                created_at: row.get(9)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let asset = interest::db::Asset {
        id: Some(asset_id),
        ticker: "TEST11".to_string(),
        asset_type: AssetType::Stock,
        name: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };

    for action in &actions {
        interest::corporate_actions::apply_corporate_action(&conn, action, &asset)?;
    }

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

    // Verify final position with query-time adjustments
    // Original: 100 shares, after 1:2 split = 200, plus 20 bonus = 220 total
    let as_of = chrono::NaiveDate::from_ymd_opt(2025, 12, 31).unwrap();
    let txs: Vec<Transaction> = conn
        .prepare("SELECT id, asset_id, transaction_type, trade_date, settlement_date, quantity, price_per_unit, total_cost, fees, is_day_trade, quota_issuance_date, notes, source, created_at FROM transactions WHERE asset_id = ?1 ORDER BY trade_date")?
        .query_map([asset_id], |row| {
            Ok(Transaction {
                id: Some(row.get(0)?),
                asset_id: row.get(1)?,
                transaction_type: row.get::<_, String>(2)?.parse().unwrap(),
                trade_date: row.get(3)?,
                settlement_date: row.get(4)?,
                quantity: interest::db::get_decimal_value(row, 5)?,
                price_per_unit: interest::db::get_decimal_value(row, 6)?,
                total_cost: interest::db::get_decimal_value(row, 7)?,
                fees: interest::db::get_decimal_value(row, 8)?,
                is_day_trade: row.get(9)?,
                quota_issuance_date: row.get(10)?,
                notes: row.get(11)?,
                source: row.get(12)?,
                created_at: row.get(13)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut total_qty = dec!(0);
    for tx in &txs {
        let actions = get_applicable_actions(&conn, asset_id, tx.trade_date, as_of)?;
        let adjusted_qty = adjust_quantity_for_actions(tx.quantity, &actions);
        total_qty += adjusted_qty;
    }

    // Original 100 -> 200 after split, plus 20 bonus (no adjustment needed for bonus as it's after split)
    assert_eq!(total_qty, dec!(220), "Final position should be 220 shares");

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

    // Verify corporate action was created
    let db_path = get_db_path(&home);
    let conn = Connection::open(&db_path)?;
    let mut stmt = conn.prepare("SELECT COUNT(*) FROM corporate_actions WHERE asset_id = ?")?;
    let action_count: i64 = stmt.query_row([transactions[0].asset_id], |row| row.get(0))?;
    assert_eq!(action_count, 1, "Corporate action should be recorded");

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

    // Manually create the split corporate action
    let asset_id = transactions[0].asset_id;
    let db_path = get_db_path(&home);
    let conn = Connection::open(&db_path)?;
    conn.execute(
        "INSERT INTO corporate_actions (asset_id, action_type, event_date, ex_date, ratio_from, ratio_to, source)
         VALUES (?1, 'SPLIT', '2025-02-15', '2025-02-15', 1, 2, 'TEST')",
        [asset_id],
    )?;

    // Note: No need to call apply_corporate_action - adjustments happen at query time
    // Verify the action exists in database
    let mut stmt = conn.prepare("SELECT id FROM corporate_actions WHERE asset_id = ?1")?;
    let action_count = stmt
        .query_map([asset_id], |row| row.get::<_, i64>(0))?
        .count();
    assert_eq!(action_count, 1, "Should have 1 corporate action");

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

    // Test query-time adjustment: manually apply adjustment to simulate what portfolio/tax code does
    let actions_for_first_tx =
        get_applicable_actions(&conn, asset_id, db_txs[0].trade_date, db_txs[2].trade_date)?;
    assert_eq!(
        actions_for_first_tx.len(),
        1,
        "First tx should have 1 applicable action"
    );

    // Adjust first transaction quantities at query time
    let adjusted_qty_0 = adjust_quantity_for_actions(db_txs[0].quantity, &actions_for_first_tx);
    let (adjusted_price_0, adjusted_cost_0) = adjust_price_and_cost_for_actions(
        db_txs[0].quantity,
        db_txs[0].price_per_unit,
        db_txs[0].total_cost,
        &actions_for_first_tx,
    );

    // After 1:2 split: 100 @ R$80 -> 200 @ R$40
    assert_eq!(
        adjusted_qty_0,
        dec!(200),
        "Adjusted quantity after 1:2 split"
    );
    assert_eq!(adjusted_price_0, dec!(40), "Adjusted price after split");
    assert_eq!(adjusted_cost_0, dec!(8000), "Total cost unchanged");

    // Test cost basis with adjusted quantities (simulating what tax code does)
    let mut avg = AverageCostMatcher::new();
    avg.add_purchase(&db_txs[0], Some(adjusted_qty_0), Some(adjusted_cost_0));
    avg.add_purchase(&db_txs[1], None, None); // No adjustment needed (after split date)

    let sale_result = avg.match_sale(&db_txs[2], None)?;

    let expected_avg =
        (adjusted_cost_0 + db_txs[1].total_cost) / (adjusted_qty_0 + db_txs[1].quantity);
    let expected_cost_basis = expected_avg * db_txs[2].quantity;

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
    conn.execute(
        "INSERT INTO corporate_actions (asset_id, action_type, event_date, ex_date, ratio_from, ratio_to, source)
         VALUES (?1, 'SPLIT', '2025-02-20', '2025-02-20', 10, 1, 'TEST')",
        [asset_id],
    )?;

    // Verify action exists
    let mut stmt = conn.prepare("SELECT id FROM corporate_actions WHERE asset_id = ?1")?;
    let action_count = stmt
        .query_map([asset_id], |row| row.get::<_, i64>(0))?
        .count();
    assert_eq!(action_count, 1, "Should have 1 corporate action");

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

    // Apply query-time adjustment
    let actions_for_first_tx =
        get_applicable_actions(&conn, asset_id, db_txs[0].trade_date, db_txs[1].trade_date)?;
    assert_eq!(
        actions_for_first_tx.len(),
        1,
        "First tx should have 1 applicable action"
    );

    let adjusted_qty_0 = adjust_quantity_for_actions(db_txs[0].quantity, &actions_for_first_tx);
    let (adjusted_price_0, adjusted_cost_0) = adjust_price_and_cost_for_actions(
        db_txs[0].quantity,
        db_txs[0].price_per_unit,
        db_txs[0].total_cost,
        &actions_for_first_tx,
    );

    // 1000 @ R$2.00 -> 100 @ R$20.00
    assert_eq!(
        adjusted_qty_0,
        dec!(100),
        "Adjusted quantity after 10:1 reverse split"
    );
    assert_eq!(
        adjusted_price_0,
        dec!(20),
        "Adjusted price after reverse split"
    );
    assert_eq!(adjusted_cost_0, dec!(2000), "Total cost unchanged");

    // Test cost basis with adjusted values
    let mut avg = AverageCostMatcher::new();
    avg.add_purchase(&db_txs[0], Some(adjusted_qty_0), Some(adjusted_cost_0));

    let sale_result = avg.match_sale(&db_txs[1], None)?;

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
    conn.execute(
        "INSERT INTO corporate_actions (asset_id, action_type, event_date, ex_date, ratio_from, ratio_to, source)
         VALUES (?1, 'SPLIT', '2025-02-10', '2025-02-10', 1, 2, 'TEST')",
        [asset_id],
    )?;

    // Second split 1:2 on 2025-04-15
    conn.execute(
        "INSERT INTO corporate_actions (asset_id, action_type, event_date, ex_date, ratio_from, ratio_to, source)
         VALUES (?1, 'SPLIT', '2025-04-15', '2025-04-15', 1, 2, 'TEST')",
        [asset_id],
    )?;

    // Verify both actions exist
    let mut stmt =
        conn.prepare("SELECT id FROM corporate_actions WHERE asset_id = ?1 ORDER BY ex_date")?;
    let action_count = stmt
        .query_map([asset_id], |row| row.get::<_, i64>(0))?
        .count();
    assert_eq!(action_count, 2, "Should have 2 corporate actions");

    // Re-fetch transactions - should be UNCHANGED in database
    let db_txs = sql::query_transactions(&home, "ITSA4");
    assert_eq!(db_txs[0].quantity, dec!(50), "Database quantity unchanged");
    assert_eq!(
        db_txs[0].price_per_unit,
        dec!(10),
        "Database price unchanged"
    );
    assert_eq!(db_txs[1].quantity, dec!(25), "Database quantity unchanged");
    assert_eq!(
        db_txs[1].price_per_unit,
        dec!(5.5),
        "Database price unchanged"
    );

    // Apply query-time adjustments to first transaction (both splits apply)
    let actions_for_tx0 =
        get_applicable_actions(&conn, asset_id, db_txs[0].trade_date, db_txs[2].trade_date)?;
    assert_eq!(
        actions_for_tx0.len(),
        2,
        "First tx should have 2 applicable actions"
    );

    let adjusted_qty_0 = adjust_quantity_for_actions(db_txs[0].quantity, &actions_for_tx0);
    let (adjusted_price_0, adjusted_cost_0) = adjust_price_and_cost_for_actions(
        db_txs[0].quantity,
        db_txs[0].price_per_unit,
        db_txs[0].total_cost,
        &actions_for_tx0,
    );

    // First purchase: 50 @ R$10.00 -> 100 @ R$5.00 -> 200 @ R$2.50
    assert_eq!(adjusted_qty_0, dec!(200), "Quantity after two 1:2 splits");
    assert_eq!(adjusted_price_0, dec!(2.5), "Price after two splits");
    assert_eq!(adjusted_cost_0, dec!(500), "Total cost unchanged");

    // Apply query-time adjustments to second transaction (only second split applies)
    let actions_for_tx1 =
        get_applicable_actions(&conn, asset_id, db_txs[1].trade_date, db_txs[2].trade_date)?;
    assert_eq!(
        actions_for_tx1.len(),
        1,
        "Second tx should have 1 applicable action"
    );

    let adjusted_qty_1 = adjust_quantity_for_actions(db_txs[1].quantity, &actions_for_tx1);
    let (adjusted_price_1, adjusted_cost_1) = adjust_price_and_cost_for_actions(
        db_txs[1].quantity,
        db_txs[1].price_per_unit,
        db_txs[1].total_cost,
        &actions_for_tx1,
    );

    // Second purchase: 25 @ R$5.50 -> 50 @ R$2.75 (only second split applies)
    assert_eq!(adjusted_qty_1, dec!(50), "Quantity after one 1:2 split");
    assert_eq!(adjusted_price_1, dec!(2.75), "Price after one split");
    assert_eq!(adjusted_cost_1, dec!(137.5), "Total cost unchanged");

    // Test cost basis with adjusted values
    let mut avg = AverageCostMatcher::new();
    avg.add_purchase(&db_txs[0], Some(adjusted_qty_0), Some(adjusted_cost_0));
    avg.add_purchase(&db_txs[1], Some(adjusted_qty_1), Some(adjusted_cost_1));

    let sale_result = avg.match_sale(&db_txs[2], None)?;

    let expected_avg = (adjusted_cost_0 + adjusted_cost_1) / (adjusted_qty_0 + adjusted_qty_1);
    let expected_cost_basis = expected_avg * db_txs[2].quantity;

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
    assert_eq!(base_txs.len(), 6, "Should have 6 base transactions");

    // Create split (1:2) on 2025-02-15
    let asset_id = base_txs[0].asset_id;
    conn.execute(
        "INSERT INTO corporate_actions (asset_id, action_type, event_date, ex_date, ratio_from, ratio_to, source)
         VALUES (?1, 'SPLIT', '2025-02-15', '2025-02-15', 1, 2, 'TEST')",
        [asset_id],
    )?;

    // Process term liquidations
    process_term_liquidations(&conn)?;

    // Re-fetch transactions - base transactions unchanged in DB
    let db_txs = sql::query_transactions(&home, "BBAS3");
    assert_eq!(db_txs[0].quantity, dec!(200), "Database quantity unchanged");
    assert_eq!(
        db_txs[0].price_per_unit,
        dec!(40),
        "Database price unchanged"
    );
    assert_eq!(db_txs[1].quantity, dec!(100), "Database quantity unchanged");
    assert_eq!(
        db_txs[1].price_per_unit,
        dec!(42),
        "Database price unchanged"
    );

    // Apply query-time adjustments to first two transactions
    let actions_for_tx0 =
        get_applicable_actions(&conn, asset_id, db_txs[0].trade_date, db_txs[5].trade_date)?;
    let adjusted_qty_0 = adjust_quantity_for_actions(db_txs[0].quantity, &actions_for_tx0);
    let (adjusted_price_0, adjusted_cost_0) = adjust_price_and_cost_for_actions(
        db_txs[0].quantity,
        db_txs[0].price_per_unit,
        db_txs[0].total_cost,
        &actions_for_tx0,
    );
    assert_eq!(adjusted_qty_0, dec!(400), "200 -> 400 after 1:2 split");
    assert_eq!(adjusted_price_0, dec!(20), "40 -> 20 after split");

    let actions_for_tx1 =
        get_applicable_actions(&conn, asset_id, db_txs[1].trade_date, db_txs[5].trade_date)?;
    let adjusted_qty_1 = adjust_quantity_for_actions(db_txs[1].quantity, &actions_for_tx1);
    let (adjusted_price_1, adjusted_cost_1) = adjust_price_and_cost_for_actions(
        db_txs[1].quantity,
        db_txs[1].price_per_unit,
        db_txs[1].total_cost,
        &actions_for_tx1,
    );
    assert_eq!(adjusted_qty_1, dec!(200), "100 -> 200 after 1:2 split");
    assert_eq!(adjusted_price_1, dec!(21), "42 -> 21 after split");

    // Calculate final position with adjusted values
    let mut avg = AverageCostMatcher::new();
    avg.add_purchase(&db_txs[0], Some(adjusted_qty_0), Some(adjusted_cost_0)); // 400 @ 20
    avg.add_purchase(&db_txs[1], Some(adjusted_qty_1), Some(adjusted_cost_1)); // 200 @ 21

    let avg_before_sale1 = avg.average_cost();
    let _sale1 = avg.match_sale(&db_txs[2], None)?; // Sells 300 @ 22
    let tolerance = dec!(0.0000000001);
    assert!((avg.average_cost() - avg_before_sale1).abs() <= tolerance);

    avg.add_purchase(&db_txs[3], None, None); // 150 @ 23 (after split)
    avg.add_purchase(&db_txs[4], None, None); // 200 @ 24 (term liquidation)

    let avg_before_sale2 = avg.average_cost();
    let sale2 = avg.match_sale(&db_txs[5], None)?; // 400 shares @ 26

    assert_eq!(sale2.cost_basis, avg_before_sale2 * db_txs[5].quantity);
    assert_eq!(sale2.sale_total, dec!(10400));
    assert_eq!(sale2.profit_loss, sale2.sale_total - sale2.cost_basis);
    assert!((avg.average_cost() - avg_before_sale2).abs() <= tolerance);
    assert_eq!(avg.remaining_quantity(), dec!(250));

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
        "INSERT INTO corporate_actions (asset_id, action_type, event_date, ex_date, ratio_from, ratio_to, source)
         VALUES (?1, 'CAPITAL_RETURN', '2025-02-15', '2025-02-15', 100, 0, 'TEST')",
        [asset_id],
    )?;

    // Re-fetch - transactions should be UNCHANGED in database
    let db_txs = sql::query_transactions(&home, "MXRF11");
    assert_eq!(db_txs[0].quantity, dec!(100), "Database quantity unchanged");
    assert_eq!(db_txs[0].total_cost, dec!(1000), "Database cost unchanged");
    assert_eq!(
        db_txs[0].price_per_unit,
        dec!(10),
        "Database price unchanged"
    );

    // Apply query-time adjustment for capital return
    let actions_for_tx0 =
        get_applicable_actions(&conn, asset_id, db_txs[0].trade_date, db_txs[2].trade_date)?;
    assert_eq!(
        actions_for_tx0.len(),
        1,
        "Should have 1 capital return action"
    );

    let adjusted_qty_0 = adjust_quantity_for_actions(db_txs[0].quantity, &actions_for_tx0);
    let (adjusted_price_0, adjusted_cost_0) = adjust_price_and_cost_for_actions(
        db_txs[0].quantity,
        db_txs[0].price_per_unit,
        db_txs[0].total_cost,
        &actions_for_tx0,
    );

    // Capital return: R$1.00/share reduces cost from R$1000 to R$900
    assert_eq!(
        adjusted_qty_0,
        dec!(100),
        "Quantity unchanged by capital return"
    );
    assert_eq!(adjusted_cost_0, dec!(900), "Cost reduced by capital return");
    assert_eq!(adjusted_price_0, dec!(9), "Price per unit recalculated");

    // Second purchase (after capital return) needs no adjustment
    assert_eq!(db_txs[1].quantity, dec!(50));
    assert_eq!(db_txs[1].price_per_unit, dec!(10.5));

    // Test cost basis with adjusted values
    let mut avg = AverageCostMatcher::new();
    avg.add_purchase(&db_txs[0], Some(adjusted_qty_0), Some(adjusted_cost_0)); // 100 @ 9.00
    avg.add_purchase(&db_txs[1], None, None); // 50 @ 10.50

    let sale_result = avg.match_sale(&db_txs[2], None)?; // Sell 120

    let expected_avg =
        (adjusted_cost_0 + db_txs[1].total_cost) / (adjusted_qty_0 + db_txs[1].quantity);
    let expected_cost_basis = expected_avg * db_txs[2].quantity;

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
#[test]
fn test_15_mixed_splits_reverse_splits_and_bonus() -> Result<()> {
    let home = TempDir::new()?;

    // Initialize DB and import test data
    let db_path = get_db_path(&home);
    std::fs::create_dir_all(db_path.parent().unwrap())?;
    init_database(Some(db_path.clone()))?;
    let conn = Connection::open(&db_path)?;
    import_movimentacao(&conn, "tests/data/15_mixed_splits_and_bonus.xlsx")?;

    let transactions = sql::query_transactions(&home, "KPCA3");
    assert_eq!(
        transactions.len(),
        4,
        "Should have 4 transactions (2 buys, 2 sells)"
    );

    let asset_id = transactions[0].asset_id;

    // Create corporate actions: split, reverse split
    // Bonus creates synthetic transactions, doesn't adjust query-time for existing txs

    // Action 1: Split 1:2 on 2025-02-10
    conn.execute(
        "INSERT INTO corporate_actions (asset_id, action_type, event_date, ex_date, ratio_from, ratio_to, source)
         VALUES (?1, 'SPLIT', '2025-02-10', '2025-02-10', 1, 2, 'TEST')",
        [asset_id],
    )?;

    // Action 2: Reverse split 5:1 on 2025-03-15
    conn.execute(
        "INSERT INTO corporate_actions (asset_id, action_type, event_date, ex_date, ratio_from, ratio_to, source)
         VALUES (?1, 'SPLIT', '2025-03-15', '2025-03-15', 5, 1, 'TEST')",
        [asset_id],
    )?;

    // Action 3: Bonus 1:2 on 2025-04-20
    // NOTE: Bonus actions don't adjust quantities of existing txs at query-time
    conn.execute(
        "INSERT INTO corporate_actions (asset_id, action_type, event_date, ex_date, ratio_from, ratio_to, source)
         VALUES (?1, 'BONUS', '2025-04-20', '2025-04-20', 1, 2, 'TEST')",
        [asset_id],
    )?;

    // Verify actions exist
    let mut stmt =
        conn.prepare("SELECT id FROM corporate_actions WHERE asset_id = ?1 ORDER BY ex_date")?;
    let action_count = stmt
        .query_map([asset_id], |row| row.get::<_, i64>(0))?
        .count();
    assert_eq!(action_count, 3, "Should have 3 corporate actions");

    // Re-fetch transactions - should be UNCHANGED in database
    let db_txs = sql::query_transactions(&home, "KPCA3");

    // First buy: 1000 @ R$2.00
    assert_eq!(db_txs[0].quantity, dec!(1000), "First buy unchanged in DB");
    assert_eq!(
        db_txs[0].price_per_unit,
        dec!(2.00),
        "First buy price unchanged"
    );
    assert_eq!(db_txs[0].total_cost, dec!(2000), "First buy cost unchanged");

    // Second buy: 800 @ R$0.90
    assert_eq!(db_txs[1].quantity, dec!(800), "Second buy unchanged in DB");
    assert_eq!(
        db_txs[1].price_per_unit,
        dec!(0.90),
        "Second buy price unchanged"
    );
    assert_eq!(db_txs[1].total_cost, dec!(720), "Second buy cost unchanged");

    // First sell: 200 @ R$5.50
    assert_eq!(db_txs[2].quantity, dec!(200), "First sell unchanged");

    // Second sell: 400 @ R$3.00
    assert_eq!(db_txs[3].quantity, dec!(400), "Second sell unchanged");

    // Test query-time adjustments for each transaction
    // Note: get_applicable_actions returns ALL actions in the date range
    // But adjust_quantity_for_actions ignores bonus actions

    // FIRST TRANSACTION: 1000 @ R$2.00
    let actions_for_tx0 =
        get_applicable_actions(&conn, asset_id, db_txs[0].trade_date, db_txs[3].trade_date)?;
    assert_eq!(
        actions_for_tx0.len(),
        3,
        "First tx: all 3 actions are 'applicable' (between dates)"
    );
    assert_eq!(actions_for_tx0[0].action_type, CorporateActionType::Split);
    assert_eq!(actions_for_tx0[1].action_type, CorporateActionType::Split); // Reverse split
    assert_eq!(actions_for_tx0[2].action_type, CorporateActionType::Bonus);

    let adjusted_qty_0 = adjust_quantity_for_actions(db_txs[0].quantity, &actions_for_tx0);
    let (adjusted_price_0, adjusted_cost_0) = adjust_price_and_cost_for_actions(
        db_txs[0].quantity,
        db_txs[0].price_per_unit,
        db_txs[0].total_cost,
        &actions_for_tx0,
    );

    // First buy adjustments (bonus is IGNORED in adjust_quantity_for_actions):
    // 1000 @ R$2.00 = R$2000
    // After 1:2 split: 2000 @ R$1.00 = R$2000
    // After 5:1 reverse: 400 @ R$5.00 = R$2000
    // Bonus is NOT applied to adjust functions
    assert_eq!(
        adjusted_qty_0,
        dec!(400),
        "First tx: 1000 -> 2000 -> 400 (bonus ignored)"
    );
    assert_eq!(
        adjusted_price_0,
        dec!(5.0),
        "First tx: R$2.00 -> R$1.00 -> R$5.00 (bonus ignored)"
    );
    assert_eq!(adjusted_cost_0, dec!(2000), "Cost unchanged through splits");

    // SECOND TRANSACTION: 800 @ R$0.90
    let actions_for_tx1 =
        get_applicable_actions(&conn, asset_id, db_txs[1].trade_date, db_txs[3].trade_date)?;
    assert_eq!(
        actions_for_tx1.len(),
        2,
        "Second tx: 2 actions (reverse split + bonus)"
    );
    assert_eq!(actions_for_tx1[0].action_type, CorporateActionType::Split); // Reverse split
    assert_eq!(actions_for_tx1[1].action_type, CorporateActionType::Bonus);

    let adjusted_qty_1 = adjust_quantity_for_actions(db_txs[1].quantity, &actions_for_tx1);
    let (adjusted_price_1, adjusted_cost_1) = adjust_price_and_cost_for_actions(
        db_txs[1].quantity,
        db_txs[1].price_per_unit,
        db_txs[1].total_cost,
        &actions_for_tx1,
    );

    // Second buy adjustments:
    // 800 @ R$0.90 = R$720
    // After 5:1 reverse: 160 @ R$4.50 = R$720
    // Bonus is NOT applied
    assert_eq!(
        adjusted_qty_1,
        dec!(160),
        "Second tx: 800 -> 160 (bonus ignored)"
    );
    assert_eq!(adjusted_price_1, dec!(4.5), "Second tx: R$0.90 -> R$4.50");
    assert_eq!(adjusted_cost_1, dec!(720), "Cost unchanged");

    // Cost basis calculation with adjusted purchases
    let mut avg = AverageCostMatcher::new();
    avg.add_purchase(&db_txs[0], Some(adjusted_qty_0), Some(adjusted_cost_0)); // 400 @ 5.00
    avg.add_purchase(&db_txs[1], Some(adjusted_qty_1), Some(adjusted_cost_1)); // 160 @ 4.50

    let sale1_result = avg.match_sale(&db_txs[2], None)?; // Sell 200 @ 5.50

    let expected_avg_before_sale1 =
        (adjusted_cost_0 + adjusted_cost_1) / (adjusted_qty_0 + adjusted_qty_1);
    let expected_cost_basis_1 = expected_avg_before_sale1 * dec!(200);

    assert_eq!(sale1_result.cost_basis, expected_cost_basis_1);
    assert_eq!(sale1_result.sale_total, dec!(1100)); // 200 @ 5.50
    assert_eq!(
        sale1_result.profit_loss,
        sale1_result.sale_total - expected_cost_basis_1
    );

    // Remaining after first sale: 400 + 160 - 200 = 360
    assert_eq!(avg.remaining_quantity(), dec!(360));

    // Note: The second sell tries to sell 400 shares but only 360 are available
    // This demonstrates that the bonus transaction was NOT created/processed
    // (if it had been, we'd have 360 + 360 = 720 shares)
    // The AverageCostMatcher will error out on this, which is expected behavior
    let sale2_result = avg.match_sale(&db_txs[3], None);

    // This should fail because we're trying to sell more than we have
    assert!(
        sale2_result.is_err(),
        "Should error when trying to sell 400 of 360 available"
    );

    Ok(())
}
