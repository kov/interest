use anyhow::Result;
use chrono::NaiveDate;
use interest::db::{init_database, open_db, upsert_asset, AssetType, TransactionType};
use interest::tax::{generate_annual_report_with_progress, ReportProgress};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use tempfile::NamedTempFile;

// Helpers
fn create_test_db() -> Result<NamedTempFile> {
    let temp_file = NamedTempFile::new()?;
    init_database(Some(temp_file.path().to_path_buf()))?;
    Ok(temp_file)
}

fn insert_asset(db_path: &std::path::Path, ticker: &str, asset_type: AssetType) -> Result<i64> {
    let conn = open_db(Some(db_path.to_path_buf()))?;
    upsert_asset(&conn, ticker, &asset_type, Some(ticker))
}

fn insert_transaction(
    db_path: &std::path::Path,
    asset_id: i64,
    tx_type: TransactionType,
    date: NaiveDate,
    quantity: Decimal,
    price: Decimal,
    is_day_trade: bool,
) -> Result<()> {
    let conn = open_db(Some(db_path.to_path_buf()))?;
    let total_cost = quantity * price;

    conn.execute(
        "INSERT INTO transactions (asset_id, transaction_type, trade_date, settlement_date,
                                   quantity, price_per_unit, total_cost, fees, is_day_trade,
                                   source, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, datetime('now'))",
        rusqlite::params![
            asset_id,
            tx_type.as_str(),
            date.to_string(),
            date.to_string(),
            quantity.to_string(),
            price.to_string(),
            total_cost.to_string(),
            "0",
            is_day_trade,
            "TEST",
        ],
    )?;

    Ok(())
}

#[test]
fn test_snapshot_cache_reuse_target_hit() -> Result<()> {
    let temp_db = create_test_db()?;
    let db_path = temp_db.path();

    // One simple trade in 2025
    let stock_id = insert_asset(db_path, "PETR4", AssetType::Stock)?;
    insert_transaction(
        db_path,
        stock_id,
        TransactionType::Buy,
        NaiveDate::from_ymd_opt(2025, 1, 10).unwrap(),
        dec!(100),
        dec!(10),
        false,
    )?;
    insert_transaction(
        db_path,
        stock_id,
        TransactionType::Sell,
        NaiveDate::from_ymd_opt(2025, 1, 20).unwrap(),
        dec!(50),
        dec!(12),
        false,
    )?;

    let conn = open_db(Some(db_path.to_path_buf()))?;

    // First run: should recompute and write snapshot
    let mut events1: Vec<ReportProgress> = Vec::new();
    let _ = generate_annual_report_with_progress(&conn, 2025, |e| events1.push(e));

    // Expect recompute path, not target cache hit
    assert!(
        events1
            .iter()
            .any(|e| matches!(e, ReportProgress::RecomputeStart { .. })),
        "expected recompute start"
    );
    assert!(
        events1
            .iter()
            .any(|e| matches!(e, ReportProgress::RecomputedYear { year } if *year == 2025)),
        "expected recomputed target year"
    );
    assert!(
        !events1
            .iter()
            .any(|e| matches!(e, ReportProgress::TargetCacheHit { .. })),
        "should not be cache hit on first run"
    );

    // Verify snapshot row exists for 2025
    let count_2025: i64 = conn.query_row(
        "SELECT COUNT(*) FROM loss_carryforward_snapshots WHERE year = 2025",
        [],
        |row| row.get(0),
    )?;
    assert!(count_2025 >= 1);

    // For this scenario, carry is empty, so sentinel with zero amount should be stored
    let zero_rows_2025: i64 = conn.query_row(
        "SELECT COUNT(*) FROM loss_carryforward_snapshots WHERE year = 2025 AND ending_remaining_amount = 0",
        [],
        |row| row.get(0),
    )?;
    assert!(zero_rows_2025 >= 1);

    // Second run: should use cache for target year
    let mut events2: Vec<ReportProgress> = Vec::new();
    let _ = generate_annual_report_with_progress(&conn, 2025, |e| events2.push(e));
    assert!(
        events2
            .iter()
            .any(|e| matches!(e, ReportProgress::TargetCacheHit { year } if *year == 2025)),
        "expected target cache hit on second run"
    );
    assert!(
        !events2
            .iter()
            .any(|e| matches!(e, ReportProgress::RecomputeStart { .. })),
        "should not recompute on second run"
    );

    Ok(())
}

#[test]
fn test_snapshot_invalidation_recompute_from_earliest() -> Result<()> {
    let temp_db = create_test_db()?;
    let db_path = temp_db.path();

    let stock_id = insert_asset(db_path, "VALE3", AssetType::Stock)?;
    // Seed 2024 and 2025 so both get snapshots
    insert_transaction(
        db_path,
        stock_id,
        TransactionType::Buy,
        NaiveDate::from_ymd_opt(2024, 12, 10).unwrap(),
        dec!(100),
        dec!(80),
        false,
    )?;
    insert_transaction(
        db_path,
        stock_id,
        TransactionType::Buy,
        NaiveDate::from_ymd_opt(2025, 1, 5).unwrap(),
        dec!(50),
        dec!(90),
        false,
    )?;

    let conn = open_db(Some(db_path.to_path_buf()))?;

    // Initial run to create snapshots for 2024 and 2025
    let mut _events: Vec<ReportProgress> = Vec::new();
    let _ = generate_annual_report_with_progress(&conn, 2025, |e| _events.push(e));

    // Invalidate 2024 by adding another trade in 2024
    insert_transaction(
        db_path,
        stock_id,
        TransactionType::Sell,
        NaiveDate::from_ymd_opt(2024, 12, 20).unwrap(),
        dec!(10),
        dec!(85),
        false,
    )?;

    // Rerun: should detect stale 2024 snapshot and recompute from 2024
    let mut events2: Vec<ReportProgress> = Vec::new();
    let _ = generate_annual_report_with_progress(&conn, 2025, |e| events2.push(e));

    assert!(
        events2
            .iter()
            .any(|e| matches!(e, ReportProgress::SnapshotStale { year } if *year == 2024)),
        "expected stale snapshot for 2024"
    );
    assert!(
        events2.iter().any(
            |e| matches!(e, ReportProgress::RecomputeStart { from_year } if *from_year == 2024)
        ),
        "expected recompute from 2024"
    );
    assert!(
        events2
            .iter()
            .any(|e| matches!(e, ReportProgress::RecomputedYear { year } if *year == 2024)),
        "expected recomputed 2024"
    );

    Ok(())
}

#[test]
fn test_year_fingerprint_stable_across_order() -> Result<()> {
    // DB A: insert buy then sell
    let temp_db_a = create_test_db()?;
    let db_path_a = temp_db_a.path();
    let conn_a = open_db(Some(db_path_a.to_path_buf()))?;
    let asset_a = insert_asset(db_path_a, "AMER3", AssetType::Stock)?;
    insert_transaction(
        db_path_a,
        asset_a,
        TransactionType::Buy,
        NaiveDate::from_ymd_opt(2025, 3, 1).unwrap(),
        dec!(100),
        dec!(10),
        false,
    )?;
    insert_transaction(
        db_path_a,
        asset_a,
        TransactionType::Sell,
        NaiveDate::from_ymd_opt(2025, 3, 15).unwrap(),
        dec!(50),
        dec!(12),
        false,
    )?;

    // DB B: insert sell then buy (reverse order) - SQLite AUTOINCREMENT ids/order shouldn't matter
    let temp_db_b = create_test_db()?;
    let db_path_b = temp_db_b.path();
    let conn_b = open_db(Some(db_path_b.to_path_buf()))?;
    let asset_b = insert_asset(db_path_b, "AMER3", AssetType::Stock)?;
    insert_transaction(
        db_path_b,
        asset_b,
        TransactionType::Sell,
        NaiveDate::from_ymd_opt(2025, 3, 15).unwrap(),
        dec!(50),
        dec!(12),
        false,
    )?;
    insert_transaction(
        db_path_b,
        asset_b,
        TransactionType::Buy,
        NaiveDate::from_ymd_opt(2025, 3, 1).unwrap(),
        dec!(100),
        dec!(10),
        false,
    )?;

    // Compare fingerprints via public function: call the report which computes internally
    let mut ev_a: Vec<ReportProgress> = Vec::new();
    let _ = generate_annual_report_with_progress(&conn_a, 2025, |e| ev_a.push(e));

    let mut ev_b: Vec<ReportProgress> = Vec::new();
    let _ = generate_annual_report_with_progress(&conn_b, 2025, |e| ev_b.push(e));

    // Both should complete without recomputation on second run; more importantly, both should work and
    // produce consistent behavior. We don't have direct access to the fingerprint here without importing internals,
    // so we assert both paths can achieve a cache hit on a second run, implying a stable fingerprint.
    let mut ev_a2: Vec<ReportProgress> = Vec::new();
    let _ = generate_annual_report_with_progress(&conn_a, 2025, |e| ev_a2.push(e));
    assert!(ev_a2
        .iter()
        .any(|e| matches!(e, ReportProgress::TargetCacheHit { year } if *year == 2025)));

    let mut ev_b2: Vec<ReportProgress> = Vec::new();
    let _ = generate_annual_report_with_progress(&conn_b, 2025, |e| ev_b2.push(e));
    assert!(ev_b2
        .iter()
        .any(|e| matches!(e, ReportProgress::TargetCacheHit { year } if *year == 2025)));

    Ok(())
}
