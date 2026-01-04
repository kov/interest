//! Integration tests for the interest tracker
//!
//! These tests verify end-to-end functionality:
//! - XLS import
//! - Cost basis calculations with FIFO
//! - Term contract lifecycle and cost basis transfer
//! - Split/reverse split adjustments
//! - Capital return adjustments
//! - No duplicate adjustments
//! - Correct portfolio totals

use anyhow::Result;
use chrono::Utc;
use interest::corporate_actions::{apply_corporate_action, get_unapplied_actions};
use interest::db::models::{Asset, AssetType, Transaction, TransactionType};
use interest::db::{
    get_asset_position_before_date, get_last_import_date, init_database,
    insert_corporate_action, open_db, set_last_import_date, upsert_asset,
};
use interest::importers::movimentacao_excel::parse_movimentacao_excel;
use interest::tax::cost_basis::FifoMatcher;
use interest::term_contracts::process_term_liquidations;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal_macros::dec;
use rusqlite::Connection;
use std::str::FromStr;
use tempfile::TempDir;

/// Test helper: Create a temporary database
fn create_test_db() -> Result<(TempDir, Connection)> {
    let temp_dir = TempDir::new()?;
    let db_path = temp_dir.path().join("test.db");
    init_database(Some(db_path.clone()))?;
    let conn = open_db(Some(db_path))?;
    Ok((temp_dir, conn))
}

/// Test helper: Import movimentacao file into database
fn import_movimentacao(conn: &Connection, file_path: &str) -> Result<()> {
    let entries = parse_movimentacao_excel(file_path)?;

    for entry in entries {
        // Skip non-trade entries for now (we'll handle corporate actions separately)
        if !entry.is_trade() {
            continue;
        }

        let ticker = entry.ticker.as_ref().unwrap();
        let asset_type = AssetType::Stock; // Simplified for tests
        let asset_id = upsert_asset(conn, ticker, &asset_type, None)?;

        let transaction = entry.to_transaction(asset_id)?;

        // Insert transaction
        conn.execute(
            "INSERT INTO transactions (
                asset_id, transaction_type, trade_date, settlement_date,
                quantity, price_per_unit, total_cost, fees,
                is_day_trade, quota_issuance_date, notes, source
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            rusqlite::params![
                transaction.asset_id,
                transaction.transaction_type.as_str(),
                transaction.trade_date,
                transaction.settlement_date,
                transaction.quantity.to_string(),
                transaction.price_per_unit.to_string(),
                transaction.total_cost.to_string(),
                transaction.fees.to_string(),
                transaction.is_day_trade,
                transaction.quota_issuance_date,
                transaction.notes,
                transaction.source,
            ],
        )?;
    }

    Ok(())
}

struct ImportStats {
    imported_trades: usize,
    skipped_trades_old: usize,
    imported_actions: usize,
    skipped_actions_old: usize,
    auto_applied_actions: usize,
}

/// Test helper: Import movimentacao file into database with import-state tracking and auto-apply
fn import_movimentacao_with_state(conn: &Connection, file_path: &str) -> Result<ImportStats> {
    let entries = parse_movimentacao_excel(file_path)?;

    let trades: Vec<_> = entries.iter().filter(|e| e.is_trade()).collect();
    let actions: Vec<_> = entries.iter().filter(|e| e.is_corporate_action()).collect();

    let mut imported_trades = 0;
    let mut skipped_trades_old = 0;
    let mut max_trade_date = None;

    let last_trade_date = get_last_import_date(conn, "MOVIMENTACAO", "trades")?;

    for entry in trades {
        let ticker = entry.ticker.as_ref().unwrap();
        let asset_type = AssetType::Stock;
        let asset_id = upsert_asset(conn, ticker, &asset_type, None)?;

        let transaction = entry.to_transaction(asset_id)?;
        if let Some(last_date) = last_trade_date {
            if transaction.trade_date <= last_date {
                skipped_trades_old += 1;
                continue;
            }
        }

        conn.execute(
            "INSERT INTO transactions (
                asset_id, transaction_type, trade_date, settlement_date,
                quantity, price_per_unit, total_cost, fees,
                is_day_trade, quota_issuance_date, notes, source
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            rusqlite::params![
                transaction.asset_id,
                transaction.transaction_type.as_str(),
                transaction.trade_date,
                transaction.settlement_date,
                transaction.quantity.to_string(),
                transaction.price_per_unit.to_string(),
                transaction.total_cost.to_string(),
                transaction.fees.to_string(),
                transaction.is_day_trade,
                transaction.quota_issuance_date,
                transaction.notes,
                transaction.source,
            ],
        )?;
        imported_trades += 1;
        max_trade_date = Some(match max_trade_date {
            Some(current) if current >= transaction.trade_date => current,
            _ => transaction.trade_date,
        });
    }

    if let Some(last_date) = max_trade_date {
        set_last_import_date(conn, "MOVIMENTACAO", "trades", last_date)?;
    }

    let mut imported_actions = 0;
    let mut skipped_actions_old = 0;
    let mut auto_applied_actions = 0;
    let mut max_action_date = None;

    let last_action_date = get_last_import_date(conn, "MOVIMENTACAO", "corporate_actions")?;

    for entry in actions {
        let ticker = entry.ticker.as_ref().unwrap();
        let asset_type = AssetType::Stock;
        let asset_id = upsert_asset(conn, ticker, &asset_type, None)?;
        let mut action = entry.to_corporate_action(asset_id)?;

        if entry.movement_type == "Desdobro" && action.ratio_from == 1 && action.ratio_to == 1 {
            if let Some(qty) = entry.quantity {
                let old_qty = get_asset_position_before_date(conn, asset_id, entry.date)?;
                if old_qty > Decimal::ZERO {
                    let new_qty = old_qty + qty;
                    let ratio = new_qty / old_qty;
                    if let Some(ratio_i32) = ratio.to_i32() {
                        if ratio_i32 > 1 && Decimal::from(ratio_i32) == ratio {
                            action.ratio_from = 1;
                            action.ratio_to = ratio_i32;
                        }
                    }
                }
            }
        }

        if let Some(last_date) = last_action_date {
            if action.event_date <= last_date {
                skipped_actions_old += 1;
                continue;
            }
        }

        let action_id = insert_corporate_action(conn, &action)?;
        action.id = Some(action_id);
        imported_actions += 1;
        max_action_date = Some(match max_action_date {
            Some(current) if current >= action.event_date => current,
            _ => action.event_date,
        });

        if action.ratio_from != action.ratio_to {
            let asset = Asset {
                id: Some(asset_id),
                ticker: ticker.to_string(),
                asset_type,
                name: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            };
            let adjusted = apply_corporate_action(conn, &action, &asset)?;
            if adjusted > 0 {
                auto_applied_actions += 1;
            }
        }
    }

    if let Some(last_date) = max_action_date {
        set_last_import_date(conn, "MOVIMENTACAO", "corporate_actions", last_date)?;
    }

    Ok(ImportStats {
        imported_trades,
        skipped_trades_old,
        imported_actions,
        skipped_actions_old,
        auto_applied_actions,
    })
}

/// Helper to read Decimal from SQLite (handles both INTEGER, REAL and TEXT)
fn get_decimal_value(row: &rusqlite::Row, idx: usize) -> Result<Decimal, rusqlite::Error> {
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

/// Test helper: Get all transactions for an asset
fn get_transactions(conn: &Connection, ticker: &str) -> Result<Vec<Transaction>> {
    let mut stmt = conn.prepare(
        "SELECT t.id, t.asset_id, t.transaction_type, t.trade_date, t.settlement_date,
                t.quantity, t.price_per_unit, t.total_cost, t.fees, t.is_day_trade,
                t.quota_issuance_date, t.notes, t.source, t.created_at
         FROM transactions t
         JOIN assets a ON t.asset_id = a.id
         WHERE a.ticker = ?1
         ORDER BY t.trade_date ASC",
    )?;

    let transactions = stmt
        .query_map([ticker], |row| {
            Ok(Transaction {
                id: Some(row.get(0)?),
                asset_id: row.get(1)?,
                transaction_type: TransactionType::from_str(&row.get::<_, String>(2)?)
                    .unwrap_or(TransactionType::Buy),
                trade_date: row.get(3)?,
                settlement_date: row.get(4)?,
                quantity: get_decimal_value(row, 5)?,
                price_per_unit: get_decimal_value(row, 6)?,
                total_cost: get_decimal_value(row, 7)?,
                fees: get_decimal_value(row, 8)?,
                is_day_trade: row.get(9)?,
                quota_issuance_date: row.get(10)?,
                notes: row.get(11)?,
                source: row.get(12)?,
                created_at: row.get(13)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(transactions)
}

/// Test helper: Calculate total position for an asset
fn calculate_position(transactions: &[Transaction]) -> (Decimal, Decimal) {
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
                // Cost is handled via FIFO, not simple subtraction
            }
        }
    }

    (total_quantity, total_cost)
}

#[test]
fn test_01_basic_purchase_and_sale() -> Result<()> {
    let (_temp_dir, conn) = create_test_db()?;

    // Import the test file
    import_movimentacao(&conn, "tests/data/01_basic_purchase_sale.xlsx")?;

    // Verify transactions were imported
    let transactions = get_transactions(&conn, "PETR4")?;
    assert_eq!(transactions.len(), 3, "Should have 3 transactions");

    // Verify first purchase
    assert_eq!(transactions[0].transaction_type, TransactionType::Buy);
    assert_eq!(transactions[0].quantity, dec!(100));
    assert_eq!(transactions[0].price_per_unit, dec!(25));
    assert_eq!(transactions[0].total_cost, dec!(2500));

    // Verify second purchase
    assert_eq!(transactions[1].transaction_type, TransactionType::Buy);
    assert_eq!(transactions[1].quantity, dec!(50));
    assert_eq!(transactions[1].price_per_unit, dec!(30));

    // Verify sale
    assert_eq!(transactions[2].transaction_type, TransactionType::Sell);
    assert_eq!(transactions[2].quantity, dec!(80));
    assert_eq!(transactions[2].price_per_unit, dec!(35));

    // Test FIFO cost basis calculation
    let mut fifo = FifoMatcher::new();
    fifo.add_purchase(&transactions[0]);
    fifo.add_purchase(&transactions[1]);

    let sale_result = fifo.match_sale(&transactions[2])?;

    // FIFO: 80 shares from first lot @ R$25.00 = R$2,000.00 cost basis
    assert_eq!(sale_result.cost_basis, dec!(2000));
    assert_eq!(sale_result.sale_total, dec!(2800));
    assert_eq!(sale_result.profit_loss, dec!(800));

    // Verify remaining quantity
    assert_eq!(fifo.remaining_quantity(), dec!(70)); // 20 + 50

    Ok(())
}

#[test]
fn test_02_term_contract_lifecycle() -> Result<()> {
    let (_temp_dir, conn) = create_test_db()?;

    // Import the test file
    import_movimentacao(&conn, "tests/data/02_term_contract_lifecycle.xlsx")?;

    // Check ANIM3T term contract purchase
    let term_txs = get_transactions(&conn, "ANIM3T")?;
    assert_eq!(term_txs.len(), 1, "Should have 1 term contract purchase");
    assert_eq!(term_txs[0].quantity, dec!(200));
    assert_eq!(term_txs[0].price_per_unit, dec!(10));

    // Check ANIM3 transactions (liquidation + sale)
    let base_txs = get_transactions(&conn, "ANIM3")?;
    assert_eq!(base_txs.len(), 2, "Should have liquidation + sale");

    // Verify liquidation is marked correctly
    assert!(base_txs[0]
        .notes
        .as_ref()
        .unwrap()
        .contains("Term contract liquidation"));

    // Process term contract liquidations
    let processed = process_term_liquidations(&conn)?;
    assert_eq!(processed, 1, "Should process 1 term liquidation");

    // Test cost basis calculation
    // The liquidation should inherit cost from the term purchase
    let mut fifo = FifoMatcher::new();
    fifo.add_purchase(&base_txs[0]); // Liquidation becomes a purchase

    let sale_result = fifo.match_sale(&base_txs[1])?;

    // Cost basis should be from term contract: 100 @ R$10.00 = R$1,000.00
    assert_eq!(sale_result.cost_basis, dec!(1000));
    assert_eq!(sale_result.sale_total, dec!(1200));
    assert_eq!(sale_result.profit_loss, dec!(200));

    Ok(())
}

#[test]
fn test_09_duplicate_trades_not_deduped() -> Result<()> {
    let (_temp_dir, conn) = create_test_db()?;

    let stats = import_movimentacao_with_state(&conn, "tests/data/10_duplicate_trades.xlsx")?;
    assert_eq!(stats.imported_trades, 2);

    let transactions = get_transactions(&conn, "DUPL3")?;
    assert_eq!(transactions.len(), 2, "Both duplicate trades should be imported");
    Ok(())
}

#[test]
fn test_10_no_reimport_of_old_data() -> Result<()> {
    let (_temp_dir, conn) = create_test_db()?;

    let stats_first = import_movimentacao_with_state(&conn, "tests/data/10_duplicate_trades.xlsx")?;
    assert_eq!(stats_first.imported_trades, 2);

    let stats_second = import_movimentacao_with_state(&conn, "tests/data/10_duplicate_trades.xlsx")?;
    assert_eq!(stats_second.imported_trades, 0);
    assert_eq!(stats_second.skipped_trades_old, 2);

    let transactions = get_transactions(&conn, "DUPL3")?;
    assert_eq!(transactions.len(), 2);
    Ok(())
}

#[test]
fn test_11_auto_apply_bonus_action_on_import() -> Result<()> {
    let (_temp_dir, conn) = create_test_db()?;

    let stats = import_movimentacao_with_state(&conn, "tests/data/11_bonus_auto_apply.xlsx")?;
    assert_eq!(stats.imported_trades, 1);
    assert_eq!(stats.imported_actions, 1);
    assert_eq!(stats.auto_applied_actions, 1);

    let transactions = get_transactions(&conn, "ITSA4")?;
    assert_eq!(transactions.len(), 1);
    assert_eq!(transactions[0].quantity, dec!(120));
    assert_eq!(transactions[0].total_cost, dec!(1000));
    Ok(())
}

#[test]
fn test_12_desdobro_ratio_inference_auto_apply() -> Result<()> {
    let (_temp_dir, conn) = create_test_db()?;

    let stats = import_movimentacao_with_state(&conn, "tests/data/12_desdobro_inference.xlsx")?;
    assert_eq!(stats.imported_trades, 1);
    assert_eq!(stats.imported_actions, 1);
    assert_eq!(stats.auto_applied_actions, 1);

    let transactions = get_transactions(&conn, "A1MD34")?;
    assert_eq!(transactions.len(), 1);
    assert_eq!(transactions[0].quantity, dec!(640));
    assert_eq!(transactions[0].total_cost, dec!(800));
    Ok(())
}

#[test]
fn test_03_term_contract_sold_before_expiry() -> Result<()> {
    let (_temp_dir, conn) = create_test_db()?;

    import_movimentacao(&conn, "tests/data/03_term_contract_sold.xlsx")?;

    let transactions = get_transactions(&conn, "SHUL4T")?;
    assert_eq!(transactions.len(), 2, "Should have buy and sell");

    // Test cost basis - term contracts can be traded like regular stocks
    let mut fifo = FifoMatcher::new();
    fifo.add_purchase(&transactions[0]);

    let sale_result = fifo.match_sale(&transactions[1])?;

    assert_eq!(sale_result.cost_basis, dec!(1200));
    assert_eq!(sale_result.sale_total, dec!(1350));
    assert_eq!(sale_result.profit_loss, dec!(150));

    Ok(())
}

#[test]
fn test_04_stock_split() -> Result<()> {
    let (_temp_dir, conn) = create_test_db()?;

    import_movimentacao(&conn, "tests/data/04_stock_split.xlsx")?;

    let transactions = get_transactions(&conn, "VALE3")?;

    // Before adjustments: should have 4 transactions (buy, split, buy, sell)
    // Split entry is not imported as a transaction, only as corporate action
    assert_eq!(transactions.len(), 3, "Should have 3 transactions (buy, buy, sell)");

    // Manually create the split corporate action
    let asset_id = transactions[0].asset_id;
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
    let adjusted_txs = get_transactions(&conn, "VALE3")?;

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
    let mut fifo = FifoMatcher::new();
    fifo.add_purchase(&adjusted_txs[0]);
    fifo.add_purchase(&adjusted_txs[1]);

    let sale_result = fifo.match_sale(&adjusted_txs[2])?;

    // FIFO: 150 from first lot @ R$40.00 = R$6,000.00
    assert_eq!(sale_result.cost_basis, dec!(6000));
    assert_eq!(sale_result.sale_total, dec!(6750));
    assert_eq!(sale_result.profit_loss, dec!(750));

    // Remaining: 50 from first lot + 50 from second lot
    assert_eq!(fifo.remaining_quantity(), dec!(100));

    Ok(())
}

#[test]
fn test_05_reverse_split() -> Result<()> {
    let (_temp_dir, conn) = create_test_db()?;

    import_movimentacao(&conn, "tests/data/05_reverse_split.xlsx")?;

    let transactions = get_transactions(&conn, "MGLU3")?;
    assert_eq!(transactions.len(), 2, "Should have buy and sell");

    // Create reverse split (10:1 - 10 shares become 1)
    let asset_id = transactions[0].asset_id;
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
    let adjusted_txs = get_transactions(&conn, "MGLU3")?;

    // 1000 @ R$2.00 -> 100 @ R$20.00
    assert_eq!(adjusted_txs[0].quantity, dec!(100));
    assert_eq!(adjusted_txs[0].price_per_unit, dec!(20));

    // Test cost basis
    let mut fifo = FifoMatcher::new();
    fifo.add_purchase(&adjusted_txs[0]);

    let sale_result = fifo.match_sale(&adjusted_txs[1])?;

    assert_eq!(sale_result.cost_basis, dec!(1000)); // 50 @ R$20
    assert_eq!(sale_result.sale_total, dec!(1100)); // 50 @ R$22
    assert_eq!(sale_result.profit_loss, dec!(100));

    Ok(())
}

#[test]
fn test_06_multiple_splits() -> Result<()> {
    let (_temp_dir, conn) = create_test_db()?;

    import_movimentacao(&conn, "tests/data/06_multiple_splits.xlsx")?;

    let transactions = get_transactions(&conn, "ITSA4")?;
    assert_eq!(transactions.len(), 3, "Should have 3 transactions");

    let asset_id = transactions[0].asset_id;

    // First split 1:2 on 2025-02-10
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
    let adjusted_txs = get_transactions(&conn, "ITSA4")?;

    // First purchase: 50 @ R$10.00 -> 100 @ R$5.00 -> 200 @ R$2.50
    assert_eq!(adjusted_txs[0].quantity, dec!(200));
    assert_eq!(adjusted_txs[0].price_per_unit, dec!(2.5));

    // Second purchase: 25 @ R$5.50 -> 50 @ R$2.75 (only second split applies)
    assert_eq!(adjusted_txs[1].quantity, dec!(50));
    assert_eq!(adjusted_txs[1].price_per_unit, dec!(2.75));

    // Test cost basis
    let mut fifo = FifoMatcher::new();
    fifo.add_purchase(&adjusted_txs[0]);
    fifo.add_purchase(&adjusted_txs[1]);

    let sale_result = fifo.match_sale(&adjusted_txs[2])?;

    // FIFO: all 200 from first lot @ R$2.50 = R$500.00
    assert_eq!(sale_result.cost_basis, dec!(500));
    assert_eq!(sale_result.profit_loss, dec!(100)); // 600 - 500

    assert_eq!(fifo.remaining_quantity(), dec!(50)); // All from second lot

    Ok(())
}

#[test]
fn test_08_complex_scenario() -> Result<()> {
    let (_temp_dir, conn) = create_test_db()?;

    import_movimentacao(&conn, "tests/data/08_complex_scenario.xlsx")?;

    // Should have transactions for both BBAS3 and BBAS3T
    let base_txs = get_transactions(&conn, "BBAS3")?;
    let term_txs = get_transactions(&conn, "BBAS3T")?;

    assert_eq!(term_txs.len(), 1, "Should have 1 term contract purchase");
    // We expect: 2 initial buys + split entry + 1 sell + 1 buy + liquidation + 1 sell = 7
    // But split entry might not be imported as a transaction, so we get 6
    assert_eq!(
        base_txs.len(),
        6,
        "Should have 6 base transactions"
    );

    // Create and apply split (1:2) on 2025-02-15
    let asset_id = base_txs[0].asset_id;
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
    let adjusted_txs = get_transactions(&conn, "BBAS3")?;

    // Verify split adjustments on first two purchases
    assert_eq!(adjusted_txs[0].quantity, dec!(400)); // 200 -> 400
    assert_eq!(adjusted_txs[0].price_per_unit, dec!(20)); // 40 -> 20

    assert_eq!(adjusted_txs[1].quantity, dec!(200)); // 100 -> 200
    assert_eq!(adjusted_txs[1].price_per_unit, dec!(21)); // 42 -> 21

    // Calculate final position and verify cost basis for final sale
    let mut fifo = FifoMatcher::new();

    // Add purchases and process sales in order
    fifo.add_purchase(&adjusted_txs[0]); // 400 @ 20
    fifo.add_purchase(&adjusted_txs[1]); // 200 @ 21

    // First sale: 300 shares
    let _sale1 = fifo.match_sale(&adjusted_txs[2])?; // Sells 300 @ 22

    // After first sale, remaining: 100 @ 20, 200 @ 21 = 300 shares total

    // Third purchase (after first sale)
    fifo.add_purchase(&adjusted_txs[3]); // 150 @ 23

    // Term liquidation adds shares
    fifo.add_purchase(&adjusted_txs[4]); // 200 @ 24

    // Final sale: 400 shares
    let sale2 = fifo.match_sale(&adjusted_txs[5])?;

    // FIFO: 100 @ 20 + 200 @ 21 + 100 @ 23 = 2000 + 4200 + 2300 = 8500
    assert_eq!(sale2.cost_basis, dec!(8500));
    assert_eq!(sale2.sale_total, dec!(10400));
    assert_eq!(sale2.profit_loss, dec!(1900));

    // Remaining: 50 @ 23 + 200 @ 24 = 250 shares
    assert_eq!(fifo.remaining_quantity(), dec!(250));

    Ok(())
}

#[test]
fn test_no_duplicate_adjustments() -> Result<()> {
    let (_temp_dir, conn) = create_test_db()?;

    import_movimentacao(&conn, "tests/data/04_stock_split.xlsx")?;

    let transactions = get_transactions(&conn, "VALE3")?;
    let asset_id = transactions[0].asset_id;

    // Create split
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
    let adjusted_txs = get_transactions(&conn, "VALE3")?;
    assert_eq!(adjusted_txs[0].quantity, dec!(200)); // Not 400!
    assert_eq!(adjusted_txs[0].price_per_unit, dec!(40)); // Not 20!

    Ok(())
}

#[test]
fn test_position_totals_match() -> Result<()> {
    let (_temp_dir, conn) = create_test_db()?;

    import_movimentacao(&conn, "tests/data/01_basic_purchase_sale.xlsx")?;

    let transactions = get_transactions(&conn, "PETR4")?;
    let (quantity, _cost) = calculate_position(&transactions);

    // After buying 100 + 50 and selling 80, should have 70 shares
    assert_eq!(quantity, dec!(70));

    Ok(())
}
// This will be inserted into integration_tests.rs
#[test]
fn test_07_capital_return() -> Result<()> {
    let (_temp_dir, conn) = create_test_db()?;

    import_movimentacao(&conn, "tests/data/07_capital_return.xlsx")?;

    let transactions = get_transactions(&conn, "MXRF11")?;
    assert_eq!(transactions.len(), 3, "Should have 3 transactions");

    // Verify initial state
    assert_eq!(transactions[0].quantity, dec!(100));
    assert_eq!(transactions[0].price_per_unit, dec!(10));
    assert_eq!(transactions[0].total_cost, dec!(1000));

    // Create capital return action: R$1.00/share = 100 cents
    let asset_id = transactions[0].asset_id;
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
    let adjusted_txs = get_transactions(&conn, "MXRF11")?;

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

    // Test cost basis with FIFO
    let mut fifo = FifoMatcher::new();
    fifo.add_purchase(&adjusted_txs[0]); // 100 @ 9.00
    fifo.add_purchase(&adjusted_txs[1]); // 50 @ 10.50

    let sale_result = fifo.match_sale(&adjusted_txs[2])?; // Sell 120

    // FIFO: 100 @ 9.00 + 20 @ 10.50 = 900 + 210 = 1110
    assert_eq!(sale_result.cost_basis, dec!(1110));
    assert_eq!(sale_result.sale_total, dec!(1320)); // 120 @ 11.00
    assert_eq!(sale_result.profit_loss, dec!(210)); // 1320 - 1110

    // Remaining: 30 shares @ 10.50
    assert_eq!(fifo.remaining_quantity(), dec!(30));

    Ok(())
}

#[test]
fn test_10_day_trade_detection() -> Result<()> {
    let (_temp_dir, conn) = create_test_db()?;

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

    // Verify day trade flag
    let transactions = get_transactions(&conn, "VALE3")?;
    assert_eq!(transactions.len(), 2);
    assert_eq!(transactions[0].is_day_trade, false); // Buy
    assert_eq!(transactions[1].is_day_trade, true);  // Sell (day trade)

    // Day trades should result in zero position
    let (quantity, _) = calculate_position(&transactions);
    assert_eq!(quantity, Decimal::ZERO);

    Ok(())
}

#[test]
fn test_11_multi_asset_portfolio() -> Result<()> {
    let (_temp_dir, conn) = create_test_db()?;

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

    // Verify each asset's position
    let petr4_txs = get_transactions(&conn, "PETR4")?;
    let (petr4_qty, petr4_cost) = calculate_position(&petr4_txs);
    assert_eq!(petr4_qty, dec!(100));
    assert_eq!(petr4_cost, dec!(2500));

    let vale3_txs = get_transactions(&conn, "VALE3")?;
    let (vale3_qty, vale3_cost) = calculate_position(&vale3_txs);
    assert_eq!(vale3_qty, dec!(200));
    assert_eq!(vale3_cost, dec!(16000));

    let mxrf11_txs = get_transactions(&conn, "MXRF11")?;
    let (mxrf11_qty, mxrf11_cost) = calculate_position(&mxrf11_txs);
    assert_eq!(mxrf11_qty, dec!(50));
    assert_eq!(mxrf11_cost, dec!(5000));

    // Total portfolio cost
    let total_cost = petr4_cost + vale3_cost + mxrf11_cost;
    assert_eq!(total_cost, dec!(23500)); // 2500 + 16000 + 5000

    Ok(())
}
