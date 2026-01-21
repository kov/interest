//! Integration tests for income event functionality
//!
//! Tests:
//! - Parsing income events from movimentacao Excel
//! - Income event database operations (insert, duplicate detection)
//! - Income event querying with filters
//! - Income import stats tracking

use anyhow::Result;
use chrono::NaiveDate;
use interest::db::{
    get_income_events_with_assets, income_event_exists, init_database, insert_asset,
    insert_income_event, open_db, upsert_asset, AssetType, IncomeEvent, IncomeEventType,
};
use interest::importers::import_movimentacao_entries;
use interest::importers::movimentacao_excel::MovimentacaoEntry;
use rust_decimal_macros::dec;
use tempfile::NamedTempFile;

// =============================================================================
// Test Helpers
// =============================================================================

/// Create a test database in a temporary file
fn create_test_db() -> Result<(NamedTempFile, rusqlite::Connection)> {
    let temp_file = NamedTempFile::new()?;
    let db_path = temp_file.path().to_path_buf();
    init_database(Some(db_path.clone()))?;
    let conn = open_db(Some(db_path))?;
    Ok((temp_file, conn))
}

/// Create a sample income event for testing
fn sample_income_event(asset_id: i64, event_type: IncomeEventType) -> IncomeEvent {
    IncomeEvent {
        id: None,
        asset_id,
        event_date: NaiveDate::from_ymd_opt(2024, 6, 15).unwrap(),
        ex_date: Some(NaiveDate::from_ymd_opt(2024, 6, 10).unwrap()),
        event_type,
        amount_per_quota: dec!(0.85),
        total_amount: dec!(850.00),
        withholding_tax: dec!(0.00),
        is_quota_pre_2026: Some(true),
        source: "TEST".to_string(),
        notes: None,
        created_at: chrono::Utc::now(),
    }
}

/// Create a sample MovimentacaoEntry for testing
fn sample_movimentacao_entry(
    date: NaiveDate,
    movement_type: &str,
    ticker: &str,
) -> MovimentacaoEntry {
    MovimentacaoEntry {
        direction: "Credito".to_string(),
        date,
        movement_type: movement_type.to_string(),
        product: format!("{} - Some Description", ticker),
        institution: "B3".to_string(),
        quantity: Some(dec!(1000.0)),
        unit_price: Some(dec!(0.85)),
        operation_value: Some(dec!(850.00)),
        ticker: Some(ticker.to_string()),
    }
}

// =============================================================================
// Database Operation Tests
// =============================================================================

#[test]
fn test_insert_income_event() -> Result<()> {
    let (_temp, conn) = create_test_db()?;

    // Create test asset
    let asset_id = upsert_asset(&conn, "XPLG11", &AssetType::Fii, None)?;

    // Insert income event
    let event = sample_income_event(asset_id, IncomeEventType::Dividend);
    let event_id = insert_income_event(&conn, &event)?;

    assert!(event_id > 0, "Expected positive event ID");

    Ok(())
}

#[test]
fn test_income_event_exists_duplicate_detection() -> Result<()> {
    let (_temp, conn) = create_test_db()?;

    let asset_id = upsert_asset(&conn, "MXRF11", &AssetType::Fii, None)?;
    let event = sample_income_event(asset_id, IncomeEventType::Dividend);

    // Should not exist before insertion
    assert!(
        !income_event_exists(
            &conn,
            asset_id,
            event.event_date,
            &event.event_type,
            event.total_amount
        )?,
        "Event should not exist initially"
    );

    // Insert event
    insert_income_event(&conn, &event)?;

    // Should exist after insertion
    assert!(
        income_event_exists(
            &conn,
            asset_id,
            event.event_date,
            &event.event_type,
            event.total_amount
        )?,
        "Event should exist after insertion"
    );

    // Different amount should not be detected as duplicate
    assert!(
        !income_event_exists(
            &conn,
            asset_id,
            event.event_date,
            &event.event_type,
            dec!(999.99)
        )?,
        "Different amount should not be duplicate"
    );

    // Different type should not be detected as duplicate
    assert!(
        !income_event_exists(
            &conn,
            asset_id,
            event.event_date,
            &IncomeEventType::Jcp,
            event.total_amount
        )?,
        "Different event type should not be duplicate"
    );

    Ok(())
}

#[test]
fn test_get_income_events_with_assets_no_filter() -> Result<()> {
    let (_temp, conn) = create_test_db()?;

    // Create multiple assets and events
    let xplg_id = upsert_asset(&conn, "XPLG11", &AssetType::Fii, None)?;
    let mxrf_id = upsert_asset(&conn, "MXRF11", &AssetType::Fii, None)?;

    let event1 = IncomeEvent {
        event_date: NaiveDate::from_ymd_opt(2024, 6, 15).unwrap(),
        ..sample_income_event(xplg_id, IncomeEventType::Dividend)
    };
    let event2 = IncomeEvent {
        event_date: NaiveDate::from_ymd_opt(2024, 7, 10).unwrap(),
        event_type: IncomeEventType::Jcp,
        ..sample_income_event(mxrf_id, IncomeEventType::Jcp)
    };

    insert_income_event(&conn, &event1)?;
    insert_income_event(&conn, &event2)?;

    // Query all events
    let results = get_income_events_with_assets(&conn, None, None, None)?;

    assert_eq!(results.len(), 2, "Should return both events");
    assert_eq!(
        results[0].1.ticker, "XPLG11",
        "Should be ordered by date ASC"
    );
    assert_eq!(results[1].1.ticker, "MXRF11");

    Ok(())
}

#[test]
fn test_get_income_events_with_date_filter() -> Result<()> {
    let (_temp, conn) = create_test_db()?;

    let asset_id = upsert_asset(&conn, "XPLG11", &AssetType::Fii, None)?;

    // Insert events in different months
    let event1 = IncomeEvent {
        event_date: NaiveDate::from_ymd_opt(2024, 5, 15).unwrap(),
        ..sample_income_event(asset_id, IncomeEventType::Dividend)
    };
    let event2 = IncomeEvent {
        event_date: NaiveDate::from_ymd_opt(2024, 6, 15).unwrap(),
        ..sample_income_event(asset_id, IncomeEventType::Dividend)
    };
    let event3 = IncomeEvent {
        event_date: NaiveDate::from_ymd_opt(2024, 7, 15).unwrap(),
        ..sample_income_event(asset_id, IncomeEventType::Dividend)
    };

    insert_income_event(&conn, &event1)?;
    insert_income_event(&conn, &event2)?;
    insert_income_event(&conn, &event3)?;

    // Query with from_date filter
    let from_date = NaiveDate::from_ymd_opt(2024, 6, 1).unwrap();
    let results = get_income_events_with_assets(&conn, Some(from_date), None, None)?;

    assert_eq!(results.len(), 2, "Should return events from June onwards");
    assert!(results[0].0.event_date >= from_date);
    assert!(results[1].0.event_date >= from_date);

    // Query with to_date filter
    let to_date = NaiveDate::from_ymd_opt(2024, 6, 30).unwrap();
    let results = get_income_events_with_assets(&conn, None, Some(to_date), None)?;

    assert_eq!(results.len(), 2, "Should return events up to June");
    assert!(results[0].0.event_date <= to_date);
    assert!(results[1].0.event_date <= to_date);

    // Query with both filters
    let results = get_income_events_with_assets(&conn, Some(from_date), Some(to_date), None)?;

    assert_eq!(results.len(), 1, "Should return only June event");
    assert_eq!(results[0].0.event_date, event2.event_date);

    Ok(())
}

#[test]
fn test_get_income_events_with_asset_filter() -> Result<()> {
    let (_temp, conn) = create_test_db()?;

    let xplg_id = upsert_asset(&conn, "XPLG11", &AssetType::Fii, None)?;
    let mxrf_id = upsert_asset(&conn, "MXRF11", &AssetType::Fii, None)?;

    let event1 = sample_income_event(xplg_id, IncomeEventType::Dividend);
    let event2 = sample_income_event(mxrf_id, IncomeEventType::Dividend);

    insert_income_event(&conn, &event1)?;
    insert_income_event(&conn, &event2)?;

    // Filter by ticker
    let results = get_income_events_with_assets(&conn, None, None, Some("XPLG11"))?;

    assert_eq!(results.len(), 1, "Should return only XPLG11 events");
    assert_eq!(results[0].1.ticker, "XPLG11");

    // Case-insensitive filter
    let results = get_income_events_with_assets(&conn, None, None, Some("mxrf11"))?;

    assert_eq!(results.len(), 1, "Should be case-insensitive");
    assert_eq!(results[0].1.ticker, "MXRF11");

    Ok(())
}

#[test]
fn test_different_income_event_types() -> Result<()> {
    let (_temp, conn) = create_test_db()?;

    let asset_id = upsert_asset(&conn, "XPLG11", &AssetType::Fii, None)?;

    // Insert different event types
    let dividend = sample_income_event(asset_id, IncomeEventType::Dividend);
    let jcp = IncomeEvent {
        event_type: IncomeEventType::Jcp,
        total_amount: dec!(500.00),
        ..sample_income_event(asset_id, IncomeEventType::Jcp)
    };
    let amortization = IncomeEvent {
        event_type: IncomeEventType::Amortization,
        total_amount: dec!(1200.00),
        ..sample_income_event(asset_id, IncomeEventType::Amortization)
    };

    insert_income_event(&conn, &dividend)?;
    insert_income_event(&conn, &jcp)?;
    insert_income_event(&conn, &amortization)?;

    let results = get_income_events_with_assets(&conn, None, None, None)?;

    assert_eq!(results.len(), 3, "Should have all three event types");

    // Verify each event type is stored correctly
    let types: Vec<&IncomeEventType> = results.iter().map(|(e, _)| &e.event_type).collect();
    assert!(types.contains(&&IncomeEventType::Dividend));
    assert!(types.contains(&&IncomeEventType::Jcp));
    assert!(types.contains(&&IncomeEventType::Amortization));

    Ok(())
}

// =============================================================================
// Parser Tests
// =============================================================================

#[test]
fn test_movimentacao_entry_is_income_event() {
    // This tests the various movement types that should be recognized as income events
    let income_types = vec![
        "Rendimento",
        "Dividendo",
        "Juros Sobre Capital Próprio",
        "AMORTIZAÇÃO",
        "Amortização",
        "Reembolso",
        "PAGAMENTO DE JUROS",
        "INCORPORAÇÃO DE JUROS",
        "Juros",
        "Rendimento - Transferido",
        "Dividendo - Transferido",
        "Juros Sobre Capital Próprio - Transferido",
    ];

    for movement_type in income_types {
        let entry = sample_movimentacao_entry(
            NaiveDate::from_ymd_opt(2024, 6, 15).unwrap(),
            movement_type,
            "XPLG11",
        );

        assert!(
            entry.is_income_event(),
            "Movement type '{}' should be recognized as income event",
            movement_type
        );
    }
}

#[test]
fn test_movimentacao_entry_not_income_event() {
    let non_income_types = vec!["Compra", "Venda", "Transferência - Liquidação"];

    for movement_type in non_income_types {
        let mut entry = sample_movimentacao_entry(
            NaiveDate::from_ymd_opt(2024, 6, 15).unwrap(),
            movement_type,
            "XPLG11",
        );
        entry.unit_price = Some(dec!(100.00));
        entry.operation_value = Some(dec!(10000.00));

        assert!(
            !entry.is_income_event(),
            "Movement type '{}' should NOT be recognized as income event",
            movement_type
        );
    }
}

#[test]
fn test_to_income_event_dividend() -> Result<()> {
    let entry = sample_movimentacao_entry(
        NaiveDate::from_ymd_opt(2024, 6, 15).unwrap(),
        "Rendimento",
        "XPLG11",
    );

    let income_event = entry.to_income_event(1)?;

    assert_eq!(income_event.event_type, IncomeEventType::Dividend);
    assert_eq!(income_event.total_amount, dec!(850.00));
    assert_eq!(income_event.amount_per_quota, dec!(0.85));
    assert_eq!(income_event.source, "MOVIMENTACAO");
    assert_eq!(income_event.notes, None);

    Ok(())
}

#[test]
fn test_to_income_event_jcp() -> Result<()> {
    let mut entry = sample_movimentacao_entry(
        NaiveDate::from_ymd_opt(2024, 7, 20).unwrap(),
        "Juros Sobre Capital Próprio",
        "MXRF11",
    );
    entry.quantity = Some(dec!(500.0));
    entry.unit_price = Some(dec!(0.12));
    entry.operation_value = Some(dec!(60.00));

    let income_event = entry.to_income_event(2)?;

    assert_eq!(income_event.event_type, IncomeEventType::Jcp);
    assert_eq!(income_event.total_amount, dec!(60.00));
    assert_eq!(income_event.amount_per_quota, dec!(0.12));

    Ok(())
}

#[test]
fn test_to_income_event_amortization() -> Result<()> {
    let mut entry = sample_movimentacao_entry(
        NaiveDate::from_ymd_opt(2024, 8, 10).unwrap(),
        "AMORTIZAÇÃO",
        "XPLG11",
    );
    entry.unit_price = Some(dec!(1.20));
    entry.operation_value = Some(dec!(1200.00));

    let income_event = entry.to_income_event(1)?;

    assert_eq!(income_event.event_type, IncomeEventType::Amortization);
    assert_eq!(income_event.total_amount, dec!(1200.00));
    assert_eq!(income_event.amount_per_quota, dec!(1.20));

    Ok(())
}

#[test]
fn test_to_income_event_with_notes() -> Result<()> {
    // Test "Transferido" suffix adds notes
    let mut entry = sample_movimentacao_entry(
        NaiveDate::from_ymd_opt(2024, 6, 15).unwrap(),
        "Dividendo - Transferido",
        "XPLG11",
    );
    entry.quantity = Some(dec!(100.0));
    entry.operation_value = Some(dec!(85.00));

    let income_event = entry.to_income_event(1)?;

    assert_eq!(income_event.event_type, IncomeEventType::Dividend);
    assert_eq!(income_event.notes, Some("Transferido".to_string()));

    Ok(())
}

#[test]
fn test_to_income_event_no_quantity() -> Result<()> {
    // Test when quantity is None but operation_value and unit_price exist
    let mut entry = sample_movimentacao_entry(
        NaiveDate::from_ymd_opt(2024, 6, 15).unwrap(),
        "Rendimento",
        "XPLG11",
    );
    entry.quantity = None;

    let income_event = entry.to_income_event(1)?;

    assert_eq!(income_event.total_amount, dec!(850.00));
    assert_eq!(income_event.amount_per_quota, dec!(0.85)); // Falls back to unit_price

    Ok(())
}

#[test]
fn test_to_income_event_no_operation_value() -> Result<()> {
    // Test when operation_value is None but quantity and unit_price exist
    let mut entry = sample_movimentacao_entry(
        NaiveDate::from_ymd_opt(2024, 6, 15).unwrap(),
        "Dividendo",
        "XPLG11",
    );
    entry.operation_value = None;

    let income_event = entry.to_income_event(1)?;

    assert_eq!(income_event.total_amount, dec!(850.00)); // Calculated from quantity * unit_price
    assert_eq!(income_event.amount_per_quota, dec!(0.85));

    Ok(())
}

#[test]
fn test_to_income_event_invalid_movement_type() {
    let mut entry = sample_movimentacao_entry(
        NaiveDate::from_ymd_opt(2024, 6, 15).unwrap(),
        "Compra",
        "XPLG11",
    );
    entry.quantity = Some(dec!(100.0));
    entry.unit_price = Some(dec!(100.00));
    entry.operation_value = Some(dec!(10000.00));

    let result = entry.to_income_event(1);
    assert!(result.is_err(), "Should fail for non-income movement type");
}

// =============================================================================
// Import Integration Tests
// =============================================================================

#[test]
fn test_import_movimentacao_with_income_events() -> Result<()> {
    let (_temp, conn) = create_test_db()?;

    // Create sample entries with income events
    let entry1 = sample_movimentacao_entry(
        NaiveDate::from_ymd_opt(2024, 6, 15).unwrap(),
        "Rendimento",
        "XPLG11",
    );
    let mut entry2 = sample_movimentacao_entry(
        NaiveDate::from_ymd_opt(2024, 7, 20).unwrap(),
        "Juros Sobre Capital Próprio",
        "MXRF11",
    );
    entry2.quantity = Some(dec!(500.0));
    entry2.unit_price = Some(dec!(0.12));
    entry2.operation_value = Some(dec!(60.00));

    let entries = vec![entry1, entry2];

    let stats = import_movimentacao_entries(&conn, entries, false)?;

    assert_eq!(stats.imported_income, 2, "Should import 2 income events");
    assert_eq!(stats.skipped_income, 0, "Should not skip any");
    assert_eq!(stats.errors, 0, "Should have no errors");

    // Verify events were actually inserted
    let results = get_income_events_with_assets(&conn, None, None, None)?;
    assert_eq!(results.len(), 2, "Should have 2 events in database");

    Ok(())
}

#[test]
fn test_import_movimentacao_duplicate_income_detection() -> Result<()> {
    let (_temp, conn) = create_test_db()?;

    let entry = sample_movimentacao_entry(
        NaiveDate::from_ymd_opt(2024, 6, 15).unwrap(),
        "Rendimento",
        "XPLG11",
    );

    // First import
    let stats1 = import_movimentacao_entries(&conn, vec![entry.clone()], false)?;
    assert_eq!(stats1.imported_income, 1, "First import should succeed");

    // Second import (duplicate)
    let stats2 = import_movimentacao_entries(&conn, vec![entry], false)?;
    assert_eq!(stats2.imported_income, 0, "Should not import duplicate");
    assert_eq!(stats2.skipped_income, 1, "Should skip duplicate");

    // Verify only one event in database
    let results = get_income_events_with_assets(&conn, None, None, None)?;
    assert_eq!(results.len(), 1, "Should have only 1 event in database");

    Ok(())
}

#[test]
fn test_import_movimentacao_mixed_entries() -> Result<()> {
    let (_temp, conn) = create_test_db()?;

    // Mix of trades, income events, and corporate actions
    let mut trade_entry = sample_movimentacao_entry(
        NaiveDate::from_ymd_opt(2024, 6, 10).unwrap(),
        "Compra",
        "XPLG11",
    );
    trade_entry.direction = "Debito".to_string();
    trade_entry.unit_price = Some(dec!(100.00));
    trade_entry.operation_value = Some(dec!(100000.00));

    let income_entry = sample_movimentacao_entry(
        NaiveDate::from_ymd_opt(2024, 6, 15).unwrap(),
        "Rendimento",
        "XPLG11",
    );

    let entries = vec![trade_entry, income_entry];

    let stats = import_movimentacao_entries(&conn, entries, false)?;

    assert_eq!(stats.imported_trades, 1, "Should import 1 trade");
    assert_eq!(stats.imported_income, 1, "Should import 1 income event");
    assert_eq!(stats.errors, 0, "Should have no errors");

    Ok(())
}

#[test]
fn test_import_movimentacao_resgate_only_for_bonds() -> Result<()> {
    let (_temp, conn) = create_test_db()?;

    insert_asset(&conn, "PETR4", &AssetType::Stock, None)?;
    insert_asset(&conn, "TESOURO_PREFIXADO_2027", &AssetType::GovBond, None)?;

    let mut stock_resgate = sample_movimentacao_entry(
        NaiveDate::from_ymd_opt(2024, 6, 12).unwrap(),
        "Resgate",
        "PETR4",
    );
    stock_resgate.direction = "Debito".to_string();
    stock_resgate.quantity = Some(dec!(10));
    stock_resgate.unit_price = Some(dec!(30.00));
    stock_resgate.operation_value = Some(dec!(300.00));

    let mut bond_resgate = sample_movimentacao_entry(
        NaiveDate::from_ymd_opt(2024, 6, 12).unwrap(),
        "Resgate",
        "TESOURO_PREFIXADO_2027",
    );
    bond_resgate.direction = "Debito".to_string();
    bond_resgate.quantity = Some(dec!(3.8));
    bond_resgate.unit_price = Some(dec!(1000.00));
    bond_resgate.operation_value = Some(dec!(3800.00));

    let stats = import_movimentacao_entries(&conn, vec![stock_resgate, bond_resgate], false)?;

    assert_eq!(stats.imported_trades, 1, "Should import bond resgate");
    assert_eq!(stats.skipped_trades, 1, "Should skip non-bond resgate");
    assert_eq!(stats.errors, 0, "Should have no errors");

    Ok(())
}

#[test]
fn test_import_income_without_ticker() -> Result<()> {
    let (_temp, conn) = create_test_db()?;

    let mut entry = sample_movimentacao_entry(
        NaiveDate::from_ymd_opt(2024, 6, 15).unwrap(),
        "Rendimento",
        "XPLG11",
    );
    entry.ticker = None; // No ticker
    entry.product = "Unknown Product".to_string();
    entry.quantity = Some(dec!(100.0));
    entry.operation_value = Some(dec!(85.00));

    let stats = import_movimentacao_entries(&conn, vec![entry], false)?;

    assert_eq!(stats.imported_income, 0, "Should not import without ticker");
    assert_eq!(stats.skipped_income, 1, "Should skip entry without ticker");

    Ok(())
}

// =============================================================================
// IncomeEventType Parsing Tests
// =============================================================================

#[test]
fn test_income_event_type_parsing() {
    use std::str::FromStr;

    // Dividend variations
    assert_eq!(
        IncomeEventType::from_str("DIVIDEND").unwrap(),
        IncomeEventType::Dividend
    );
    assert_eq!(
        IncomeEventType::from_str("dividendo").unwrap(),
        IncomeEventType::Dividend
    );
    assert_eq!(
        IncomeEventType::from_str("Rendimento").unwrap(),
        IncomeEventType::Dividend
    );

    // Amortization variations
    assert_eq!(
        IncomeEventType::from_str("AMORTIZATION").unwrap(),
        IncomeEventType::Amortization
    );
    // Note: to_ascii_uppercase() converts accented characters to ASCII
    // So "Amortização" becomes "AMORTIZACAO" (ã -> A)
    assert_eq!(
        IncomeEventType::from_str("AMORTIZACAO").unwrap(),
        IncomeEventType::Amortization
    );
    assert_eq!(
        IncomeEventType::from_str("Amortizacao").unwrap(),
        IncomeEventType::Amortization
    );

    // JCP
    assert_eq!(
        IncomeEventType::from_str("JCP").unwrap(),
        IncomeEventType::Jcp
    );
    assert_eq!(
        IncomeEventType::from_str("jcp").unwrap(),
        IncomeEventType::Jcp
    );

    // Invalid
    assert!(IncomeEventType::from_str("invalid").is_err());
}

#[test]
fn test_income_event_type_as_str() {
    assert_eq!(IncomeEventType::Dividend.as_str(), "DIVIDEND");
    assert_eq!(IncomeEventType::Amortization.as_str(), "AMORTIZATION");
    assert_eq!(IncomeEventType::Jcp.as_str(), "JCP");
}
