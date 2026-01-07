use anyhow::Result;
use chrono::{NaiveDate, Utc};
use interest::db::{init_database, open_db, upsert_asset, AssetType, TransactionType};
use interest::tax::swing_trade::{calculate_monthly_tax, TaxCategory};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::collections::HashMap;
use tempfile::NamedTempFile;

/// Helper to create a test database
fn create_test_db() -> Result<NamedTempFile> {
    let temp_file = NamedTempFile::new()?;
    init_database(Some(temp_file.path().to_path_buf()))?;
    Ok(temp_file)
}

/// Helper to insert an asset
fn insert_asset(db_path: &std::path::Path, ticker: &str, asset_type: AssetType) -> Result<i64> {
    let conn = open_db(Some(db_path.to_path_buf()))?;
    upsert_asset(&conn, ticker, &asset_type, Some(ticker))
}

/// Helper to insert a transaction
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
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
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
            Utc::now().to_rfc3339(),
        ],
    )?;

    Ok(())
}

#[test]
fn test_stock_swing_trade_under_exemption() -> Result<()> {
    let temp_db = create_test_db()?;
    let db_path = temp_db.path();

    // Create stock asset
    let petr4_id = insert_asset(db_path, "PETR4", AssetType::Stock)?;

    // Buy 100 @ R$25 = R$2,500
    insert_transaction(
        db_path,
        petr4_id,
        TransactionType::Buy,
        NaiveDate::from_ymd_opt(2025, 1, 5).unwrap(),
        dec!(100),
        dec!(25.00),
        false,
    )?;

    // Sell 50 @ R$30 = R$1,500 (under R$20k exemption)
    insert_transaction(
        db_path,
        petr4_id,
        TransactionType::Sell,
        NaiveDate::from_ymd_opt(2025, 1, 20).unwrap(),
        dec!(50),
        dec!(30.00),
        false,
    )?;

    // Calculate tax for January 2025
    let conn = open_db(Some(db_path.to_path_buf()))?;
    let mut carry = HashMap::new();
    let calculations = calculate_monthly_tax(&conn, 2025, 1, &mut carry)?;

    // Should have one calculation for stock swing trade
    assert_eq!(calculations.len(), 1);

    let calc = &calculations[0];
    assert_eq!(calc.category, TaxCategory::StockSwingTrade);
    assert_eq!(calc.total_sales, dec!(1500.00)); // 50 * 30
    assert_eq!(calc.total_cost_basis, dec!(1250.00)); // 50 * 25
    assert_eq!(calc.net_profit, dec!(250.00)); // 1500 - 1250

    // Sales under R$20k should be exempt
    assert_eq!(calc.exemption_applied, dec!(250.00));
    assert_eq!(calc.taxable_amount, dec!(0));
    assert_eq!(calc.tax_due, dec!(0));

    Ok(())
}

#[test]
fn test_stock_swing_trade_over_exemption() -> Result<()> {
    let temp_db = create_test_db()?;
    let db_path = temp_db.path();

    // Create stock asset
    let vale3_id = insert_asset(db_path, "VALE3", AssetType::Stock)?;

    // Buy 1000 @ R$50 = R$50,000
    insert_transaction(
        db_path,
        vale3_id,
        TransactionType::Buy,
        NaiveDate::from_ymd_opt(2025, 2, 1).unwrap(),
        dec!(1000),
        dec!(50.00),
        false,
    )?;

    // Sell 500 @ R$60 = R$30,000 (over R$20k, no exemption)
    insert_transaction(
        db_path,
        vale3_id,
        TransactionType::Sell,
        NaiveDate::from_ymd_opt(2025, 2, 15).unwrap(),
        dec!(500),
        dec!(60.00),
        false,
    )?;

    // Calculate tax for February 2025
    let conn = open_db(Some(db_path.to_path_buf()))?;
    let mut carry = HashMap::new();
    let calculations = calculate_monthly_tax(&conn, 2025, 2, &mut carry)?;

    assert_eq!(calculations.len(), 1);

    let calc = &calculations[0];
    assert_eq!(calc.category, TaxCategory::StockSwingTrade);
    assert_eq!(calc.total_sales, dec!(30000.00));
    assert_eq!(calc.total_cost_basis, dec!(25000.00)); // 500 * 50
    assert_eq!(calc.net_profit, dec!(5000.00));

    // Over R$20k - no exemption
    assert_eq!(calc.exemption_applied, dec!(0));
    assert_eq!(calc.taxable_amount, dec!(5000.00));
    assert_eq!(calc.tax_due, dec!(750.00)); // 15% of 5000

    Ok(())
}

#[test]
fn test_stock_day_trade_always_taxable() -> Result<()> {
    let temp_db = create_test_db()?;
    let db_path = temp_db.path();

    // Create stock asset
    let mglu3_id = insert_asset(db_path, "MGLU3", AssetType::Stock)?;

    // Buy 100 @ R$10 = R$1,000
    insert_transaction(
        db_path,
        mglu3_id,
        TransactionType::Buy,
        NaiveDate::from_ymd_opt(2025, 3, 10).unwrap(),
        dec!(100),
        dec!(10.00),
        true, // Day trade
    )?;

    // Sell 100 @ R$12 = R$1,200 (same day - day trade)
    insert_transaction(
        db_path,
        mglu3_id,
        TransactionType::Sell,
        NaiveDate::from_ymd_opt(2025, 3, 10).unwrap(),
        dec!(100),
        dec!(12.00),
        true, // Day trade
    )?;

    // Calculate tax for March 2025
    let conn = open_db(Some(db_path.to_path_buf()))?;
    let mut carry = HashMap::new();
    let calculations = calculate_monthly_tax(&conn, 2025, 3, &mut carry)?;

    assert_eq!(calculations.len(), 1);

    let calc = &calculations[0];
    assert_eq!(calc.category, TaxCategory::StockDayTrade);
    assert_eq!(calc.total_sales, dec!(1200.00));
    assert_eq!(calc.total_cost_basis, dec!(1000.00));
    assert_eq!(calc.net_profit, dec!(200.00));

    // Day trades have NO exemption, even under R$20k
    assert_eq!(calc.exemption_applied, dec!(0));
    assert_eq!(calc.taxable_amount, dec!(200.00));
    assert_eq!(calc.tax_due, dec!(40.00)); // 20% of 200

    Ok(())
}

#[test]
fn test_fii_always_taxable_20_percent() -> Result<()> {
    let temp_db = create_test_db()?;
    let db_path = temp_db.path();

    // Create FII asset
    let mxrf11_id = insert_asset(db_path, "MXRF11", AssetType::Fii)?;

    // Buy 100 @ R$10 = R$1,000
    insert_transaction(
        db_path,
        mxrf11_id,
        TransactionType::Buy,
        NaiveDate::from_ymd_opt(2025, 4, 1).unwrap(),
        dec!(100),
        dec!(10.00),
        false,
    )?;

    // Sell 50 @ R$12 = R$600 (under R$20k but FII has no exemption)
    insert_transaction(
        db_path,
        mxrf11_id,
        TransactionType::Sell,
        NaiveDate::from_ymd_opt(2025, 4, 15).unwrap(),
        dec!(50),
        dec!(12.00),
        false,
    )?;

    // Calculate tax for April 2025
    let conn = open_db(Some(db_path.to_path_buf()))?;
    let mut carry = HashMap::new();
    let calculations = calculate_monthly_tax(&conn, 2025, 4, &mut carry)?;

    assert_eq!(calculations.len(), 1);

    let calc = &calculations[0];
    assert_eq!(calc.category, TaxCategory::FiiSwingTrade);
    assert_eq!(calc.total_sales, dec!(600.00));
    assert_eq!(calc.total_cost_basis, dec!(500.00)); // 50 * 10
    assert_eq!(calc.net_profit, dec!(100.00));

    // FII has NO exemption threshold
    assert_eq!(calc.exemption_applied, dec!(0));
    assert_eq!(calc.taxable_amount, dec!(100.00));
    assert_eq!(calc.tax_due, dec!(20.00)); // 20% of 100

    Ok(())
}

#[test]
fn test_fiagro_same_as_fii() -> Result<()> {
    let temp_db = create_test_db()?;
    let db_path = temp_db.path();

    // Create FIAGRO asset
    let fiagro_id = insert_asset(db_path, "TEST32", AssetType::Fiagro)?;

    // Buy 100 @ R$100 = R$10,000
    insert_transaction(
        db_path,
        fiagro_id,
        TransactionType::Buy,
        NaiveDate::from_ymd_opt(2025, 5, 1).unwrap(),
        dec!(100),
        dec!(100.00),
        false,
    )?;

    // Sell 50 @ R$110 = R$5,500
    insert_transaction(
        db_path,
        fiagro_id,
        TransactionType::Sell,
        NaiveDate::from_ymd_opt(2025, 5, 20).unwrap(),
        dec!(50),
        dec!(110.00),
        false,
    )?;

    // Calculate tax for May 2025
    let conn = open_db(Some(db_path.to_path_buf()))?;
    let mut carry = HashMap::new();
    let calculations = calculate_monthly_tax(&conn, 2025, 5, &mut carry)?;

    assert_eq!(calculations.len(), 1);

    let calc = &calculations[0];
    assert_eq!(calc.category, TaxCategory::FiagroSwingTrade);
    assert_eq!(calc.total_sales, dec!(5500.00));
    assert_eq!(calc.total_cost_basis, dec!(5000.00));
    assert_eq!(calc.net_profit, dec!(500.00));

    // FIAGRO same as FII: no exemption, 20% tax
    assert_eq!(calc.exemption_applied, dec!(0));
    assert_eq!(calc.taxable_amount, dec!(500.00));
    assert_eq!(calc.tax_due, dec!(100.00)); // 20% of 500

    Ok(())
}

#[test]
fn test_fi_infra_fully_exempt() -> Result<()> {
    let temp_db = create_test_db()?;
    let db_path = temp_db.path();

    // Create FI-Infra asset
    let fiinfra_id = insert_asset(db_path, "INFRA11", AssetType::FiInfra)?;

    // Buy 100 @ R$100 = R$10,000
    insert_transaction(
        db_path,
        fiinfra_id,
        TransactionType::Buy,
        NaiveDate::from_ymd_opt(2025, 6, 1).unwrap(),
        dec!(100),
        dec!(100.00),
        false,
    )?;

    // Sell 100 @ R$150 = R$15,000 (large profit!)
    insert_transaction(
        db_path,
        fiinfra_id,
        TransactionType::Sell,
        NaiveDate::from_ymd_opt(2025, 6, 20).unwrap(),
        dec!(100),
        dec!(150.00),
        false,
    )?;

    // Calculate tax for June 2025
    let conn = open_db(Some(db_path.to_path_buf()))?;
    let mut carry = HashMap::new();
    let calculations = calculate_monthly_tax(&conn, 2025, 6, &mut carry)?;

    // FI-Infra should be skipped entirely
    assert_eq!(
        calculations.len(),
        0,
        "FI-Infra sales should not generate tax calculations"
    );

    Ok(())
}

#[test]
fn test_multi_category_same_month() -> Result<()> {
    let temp_db = create_test_db()?;
    let db_path = temp_db.path();

    // Create multiple assets
    let stock_id = insert_asset(db_path, "PETR4", AssetType::Stock)?;
    let fii_id = insert_asset(db_path, "MXRF11", AssetType::Fii)?;

    // Stock swing trade: Buy 100 @ R$20, Sell 50 @ R$25 = R$1,250 sales
    insert_transaction(
        db_path,
        stock_id,
        TransactionType::Buy,
        NaiveDate::from_ymd_opt(2025, 7, 1).unwrap(),
        dec!(100),
        dec!(20.00),
        false,
    )?;
    insert_transaction(
        db_path,
        stock_id,
        TransactionType::Sell,
        NaiveDate::from_ymd_opt(2025, 7, 15).unwrap(),
        dec!(50),
        dec!(25.00),
        false,
    )?;

    // Stock day trade: Buy 100 @ R$10, Sell 100 @ R$11 = R$1,100 sales
    insert_transaction(
        db_path,
        stock_id,
        TransactionType::Buy,
        NaiveDate::from_ymd_opt(2025, 7, 20).unwrap(),
        dec!(100),
        dec!(10.00),
        true,
    )?;
    insert_transaction(
        db_path,
        stock_id,
        TransactionType::Sell,
        NaiveDate::from_ymd_opt(2025, 7, 20).unwrap(),
        dec!(100),
        dec!(11.00),
        true,
    )?;

    // FII: Buy 100 @ R$10, Sell 50 @ R$12 = R$600 sales
    insert_transaction(
        db_path,
        fii_id,
        TransactionType::Buy,
        NaiveDate::from_ymd_opt(2025, 7, 5).unwrap(),
        dec!(100),
        dec!(10.00),
        false,
    )?;
    insert_transaction(
        db_path,
        fii_id,
        TransactionType::Sell,
        NaiveDate::from_ymd_opt(2025, 7, 25).unwrap(),
        dec!(50),
        dec!(12.00),
        false,
    )?;

    // Calculate tax for July 2025
    let conn = open_db(Some(db_path.to_path_buf()))?;
    let mut carry = HashMap::new();
    let calculations = calculate_monthly_tax(&conn, 2025, 7, &mut carry)?;

    // Should have 3 categories
    assert_eq!(calculations.len(), 3);

    // Find each category
    let stock_swing = calculations
        .iter()
        .find(|c| c.category == TaxCategory::StockSwingTrade)
        .expect("Stock swing trade not found");
    let stock_day = calculations
        .iter()
        .find(|c| c.category == TaxCategory::StockDayTrade)
        .expect("Stock day trade not found");
    let fii_swing = calculations
        .iter()
        .find(|c| c.category == TaxCategory::FiiSwingTrade)
        .expect("FII swing trade not found");

    // Verify stock swing trade (under R$20k exemption)
    assert_eq!(stock_swing.total_sales, dec!(1250.00));
    assert_eq!(stock_swing.net_profit, dec!(250.00)); // (25-20)*50
    assert_eq!(stock_swing.exemption_applied, dec!(250.00));
    assert_eq!(stock_swing.tax_due, dec!(0));

    // Verify stock day trade (20%, no exemption)
    // Average cost uses day trade pool only
    // Cost basis: 100 @ R$10 = R$1000
    // Sales: 100 @ R$11 = R$1100
    // Profit: R$100
    assert_eq!(stock_day.total_sales, dec!(1100.00));
    assert_eq!(stock_day.net_profit, dec!(100.00));
    assert_eq!(stock_day.exemption_applied, dec!(0));
    assert_eq!(stock_day.tax_due, dec!(20.00)); // 20% of 100

    // Verify FII (20%, no exemption)
    assert_eq!(fii_swing.total_sales, dec!(600.00));
    assert_eq!(fii_swing.net_profit, dec!(100.00)); // (12-10)*50
    assert_eq!(fii_swing.exemption_applied, dec!(0));
    assert_eq!(fii_swing.tax_due, dec!(20.00)); // 20% of 100

    Ok(())
}

#[test]
fn test_loss_scenario() -> Result<()> {
    let temp_db = create_test_db()?;
    let db_path = temp_db.path();

    // Create stock asset
    let stock_id = insert_asset(db_path, "AMER3", AssetType::Stock)?;

    // Buy 100 @ R$50 = R$5,000
    insert_transaction(
        db_path,
        stock_id,
        TransactionType::Buy,
        NaiveDate::from_ymd_opt(2025, 8, 1).unwrap(),
        dec!(100),
        dec!(50.00),
        false,
    )?;

    // Sell 100 @ R$40 = R$4,000 (loss of R$1,000)
    insert_transaction(
        db_path,
        stock_id,
        TransactionType::Sell,
        NaiveDate::from_ymd_opt(2025, 8, 20).unwrap(),
        dec!(100),
        dec!(40.00),
        false,
    )?;

    // Calculate tax for August 2025
    let conn = open_db(Some(db_path.to_path_buf()))?;
    let mut carry = HashMap::new();
    let calculations = calculate_monthly_tax(&conn, 2025, 8, &mut carry)?;

    assert_eq!(calculations.len(), 1);

    let calc = &calculations[0];
    assert_eq!(calc.category, TaxCategory::StockSwingTrade);
    assert_eq!(calc.total_sales, dec!(4000.00));
    assert_eq!(calc.total_cost_basis, dec!(5000.00));
    assert_eq!(calc.net_profit, dec!(-1000.00)); // Loss

    // No tax on losses
    assert_eq!(calc.taxable_amount, dec!(0));
    assert_eq!(calc.tax_due, dec!(0));

    Ok(())
}

#[test]
fn test_loss_carryforward_single_category() -> Result<()> {
    let temp_db = create_test_db()?;
    let db_path = temp_db.path();

    // Create stock asset
    let stock_id = insert_asset(db_path, "AMER3", AssetType::Stock)?;

    // Month 1: Buy 100 @ R$50, Sell 100 @ R$40 = Loss of R$1,000
    insert_transaction(
        db_path,
        stock_id,
        TransactionType::Buy,
        NaiveDate::from_ymd_opt(2025, 1, 5).unwrap(),
        dec!(100),
        dec!(50.00),
        false,
    )?;
    insert_transaction(
        db_path,
        stock_id,
        TransactionType::Sell,
        NaiveDate::from_ymd_opt(2025, 1, 20).unwrap(),
        dec!(100),
        dec!(40.00),
        false,
    )?;

    // Month 2: Buy 100 @ R$30, Sell 100 @ R$45 = Profit of R$1,500
    insert_transaction(
        db_path,
        stock_id,
        TransactionType::Buy,
        NaiveDate::from_ymd_opt(2025, 2, 5).unwrap(),
        dec!(100),
        dec!(30.00),
        false,
    )?;
    insert_transaction(
        db_path,
        stock_id,
        TransactionType::Sell,
        NaiveDate::from_ymd_opt(2025, 2, 20).unwrap(),
        dec!(100),
        dec!(45.00),
        false,
    )?;

    // Month 3: Large profit over R$20k (taxable, carry IS consumed)
    insert_transaction(
        db_path,
        stock_id,
        TransactionType::Buy,
        NaiveDate::from_ymd_opt(2025, 3, 5).unwrap(),
        dec!(600),
        dec!(30.00),
        false,
    )?;
    insert_transaction(
        db_path,
        stock_id,
        TransactionType::Sell,
        NaiveDate::from_ymd_opt(2025, 3, 20).unwrap(),
        dec!(600),
        dec!(36.00),
        false,
    )?; // Sales: 21600 (over R$20k), Profit: 3600

    let conn = open_db(Some(db_path.to_path_buf()))?;

    // Month 1 - should record loss
    let mut carry = HashMap::new();
    let calc_month1 = calculate_monthly_tax(&conn, 2025, 1, &mut carry)?;
    assert_eq!(calc_month1.len(), 1);
    let m1 = &calc_month1[0];
    assert_eq!(m1.net_profit, dec!(-1000.00)); // Loss
    assert_eq!(m1.loss_offset_applied, dec!(0)); // No previous losses to apply
    assert_eq!(m1.tax_due, dec!(0)); // No tax on loss

    // Month 2 - should NOT apply loss because profit is exempt (under R$20k)
    let calc_month2 = calculate_monthly_tax(&conn, 2025, 2, &mut carry)?;
    assert_eq!(calc_month2.len(), 1);
    let m2 = &calc_month2[0];
    assert_eq!(m2.net_profit, dec!(1500.00)); // Raw profit
    assert_eq!(m2.loss_offset_applied, dec!(0)); // NO loss applied - profit is exempt
    assert_eq!(m2.exemption_applied, dec!(1500.00)); // Entire profit exempt (under R$20k)
    assert_eq!(m2.taxable_amount, dec!(0)); // Nothing taxable after exemption
    assert_eq!(m2.tax_due, dec!(0));
    // Carry should still be 1000 (not consumed on exempt profit)
    assert_eq!(carry.get(&TaxCategory::StockSwingTrade), Some(&dec!(1000)));

    // Month 3 - NOW sales volume exceeds R$20k, carry should be applied
    let calc_month3 = calculate_monthly_tax(&conn, 2025, 3, &mut carry)?;
    assert_eq!(calc_month3.len(), 1);
    let m3 = &calc_month3[0];
    assert_eq!(m3.net_profit, dec!(3600.00)); // 600 shares * (36 - 30) = R$3600
    assert_eq!(m3.loss_offset_applied, dec!(1000.00)); // Carry IS applied (sales >R$20k)
    assert_eq!(m3.taxable_amount, dec!(2600.00)); // 3600 - 1000
    assert_eq!(m3.exemption_applied, dec!(0)); // No exemption (sales >R$20k)
    assert_eq!(m3.tax_due, dec!(390.00)); // 15% of 2600
                                          // Carry should be consumed
    assert_eq!(carry.get(&TaxCategory::StockSwingTrade), None); // All consumed

    Ok(())
}

#[test]
fn test_loss_carryforward_partial_offset() -> Result<()> {
    let temp_db = create_test_db()?;
    let db_path = temp_db.path();

    // Create stock asset
    let stock_id = insert_asset(db_path, "PETR4", AssetType::Stock)?;

    // Month 1: Loss of R$5,000
    insert_transaction(
        db_path,
        stock_id,
        TransactionType::Buy,
        NaiveDate::from_ymd_opt(2025, 1, 5).unwrap(),
        dec!(100),
        dec!(100.00),
        false,
    )?;
    insert_transaction(
        db_path,
        stock_id,
        TransactionType::Sell,
        NaiveDate::from_ymd_opt(2025, 1, 20).unwrap(),
        dec!(100),
        dec!(50.00),
        false,
    )?;

    // Month 2: Profit of R$2,000 (less than previous loss)
    insert_transaction(
        db_path,
        stock_id,
        TransactionType::Buy,
        NaiveDate::from_ymd_opt(2025, 2, 5).unwrap(),
        dec!(100),
        dec!(30.00),
        false,
    )?;
    insert_transaction(
        db_path,
        stock_id,
        TransactionType::Sell,
        NaiveDate::from_ymd_opt(2025, 2, 20).unwrap(),
        dec!(100),
        dec!(50.00),
        false,
    )?;

    // Month 3: Profit of R$4,000
    insert_transaction(
        db_path,
        stock_id,
        TransactionType::Buy,
        NaiveDate::from_ymd_opt(2025, 3, 5).unwrap(),
        dec!(100),
        dec!(40.00),
        false,
    )?;
    insert_transaction(
        db_path,
        stock_id,
        TransactionType::Sell,
        NaiveDate::from_ymd_opt(2025, 3, 20).unwrap(),
        dec!(100),
        dec!(80.00),
        false,
    )?;

    // Month 4: Large profit over R$20k (taxable, carry IS consumed partially)
    insert_transaction(
        db_path,
        stock_id,
        TransactionType::Buy,
        NaiveDate::from_ymd_opt(2025, 4, 5).unwrap(),
        dec!(400),
        dec!(50.00),
        false,
    )?;
    insert_transaction(
        db_path,
        stock_id,
        TransactionType::Sell,
        NaiveDate::from_ymd_opt(2025, 4, 20).unwrap(),
        dec!(400),
        dec!(57.00),
        false,
    )?; // Sales: 22800 (over R$20k), Profit: 2800

    let conn = open_db(Some(db_path.to_path_buf()))?;

    // Month 1: Record R$5,000 loss
    let mut carry = HashMap::new();
    let calc_month1 = calculate_monthly_tax(&conn, 2025, 1, &mut carry)?;
    assert_eq!(calc_month1[0].net_profit, dec!(-5000.00));

    // Month 2: Profit, but NOT offset (profit is exempt under R$20k)
    let calc_month2 = calculate_monthly_tax(&conn, 2025, 2, &mut carry)?;
    let m2 = &calc_month2[0];
    assert_eq!(m2.net_profit, dec!(2000.00));
    assert_eq!(m2.loss_offset_applied, dec!(0)); // NO offset - profit is exempt
    assert_eq!(m2.taxable_amount, dec!(0)); // Nothing taxable after exemption
    assert_eq!(m2.exemption_applied, dec!(2000.00)); // Entire profit exempt
    assert_eq!(m2.tax_due, dec!(0));

    // Month 3: Profit, also exempt
    let calc_month3 = calculate_monthly_tax(&conn, 2025, 3, &mut carry)?;
    let m3 = &calc_month3[0];
    assert_eq!(m3.net_profit, dec!(4000.00));
    assert_eq!(m3.loss_offset_applied, dec!(0)); // NO offset - profit is exempt
    assert_eq!(m3.taxable_amount, dec!(0)); // Nothing taxable after exemption
    assert_eq!(m3.exemption_applied, dec!(4000.00)); // Entire profit exempt
    assert_eq!(m3.tax_due, dec!(0));
    // Carry should still contain R$5,000 loss
    assert_eq!(carry.get(&TaxCategory::StockSwingTrade), Some(&dec!(5000)));

    // Month 4: Large profit - now carry IS consumed
    let calc_month4 = calculate_monthly_tax(&conn, 2025, 4, &mut carry)?;
    let m4 = &calc_month4[0];
    assert_eq!(m4.net_profit, dec!(2800.00));
    assert_eq!(m4.loss_offset_applied, dec!(2800.00)); // Carry applied (partial)
    assert_eq!(m4.taxable_amount, dec!(0)); // Fully offset
    assert_eq!(m4.exemption_applied, dec!(0)); // No exemption (large sale)
    assert_eq!(m4.tax_due, dec!(0)); // No tax on zeroed profit
                                     // Remaining carry: 5000 - 2800 = 2200
    assert_eq!(carry.get(&TaxCategory::StockSwingTrade), Some(&dec!(2200)));

    Ok(())
}

#[test]
fn test_loss_carryforward_separate_categories() -> Result<()> {
    let temp_db = create_test_db()?;
    let db_path = temp_db.path();

    // Create stock asset
    let stock_id = insert_asset(db_path, "MGLU3", AssetType::Stock)?;

    // Month 1: Swing trade loss of R$1,000
    insert_transaction(
        db_path,
        stock_id,
        TransactionType::Buy,
        NaiveDate::from_ymd_opt(2025, 1, 5).unwrap(),
        dec!(100),
        dec!(20.00),
        false,
    )?;
    insert_transaction(
        db_path,
        stock_id,
        TransactionType::Sell,
        NaiveDate::from_ymd_opt(2025, 1, 20).unwrap(),
        dec!(100),
        dec!(10.00),
        false,
    )?;

    // Month 2: Day trade profit of R$500 (should NOT offset swing trade loss)
    insert_transaction(
        db_path,
        stock_id,
        TransactionType::Buy,
        NaiveDate::from_ymd_opt(2025, 2, 10).unwrap(),
        dec!(100),
        dec!(15.00),
        true,
    )?;
    insert_transaction(
        db_path,
        stock_id,
        TransactionType::Sell,
        NaiveDate::from_ymd_opt(2025, 2, 10).unwrap(),
        dec!(100),
        dec!(20.00),
        true,
    )?;

    // Month 3: Swing trade profit of R$1,500 (exempt, under R$20k sales)
    insert_transaction(
        db_path,
        stock_id,
        TransactionType::Buy,
        NaiveDate::from_ymd_opt(2025, 3, 5).unwrap(),
        dec!(100),
        dec!(25.00),
        false,
    )?;
    insert_transaction(
        db_path,
        stock_id,
        TransactionType::Sell,
        NaiveDate::from_ymd_opt(2025, 3, 20).unwrap(),
        dec!(100),
        dec!(40.00),
        false,
    )?;

    // Month 4: Large swing trade profit (over R$20k sales, carry gets applied)
    insert_transaction(
        db_path,
        stock_id,
        TransactionType::Buy,
        NaiveDate::from_ymd_opt(2025, 4, 5).unwrap(),
        dec!(500),
        dec!(35.00),
        false,
    )?;
    insert_transaction(
        db_path,
        stock_id,
        TransactionType::Sell,
        NaiveDate::from_ymd_opt(2025, 4, 20).unwrap(),
        dec!(500),
        dec!(45.00),
        false,
    )?; // Sales: 22500 (over R$20k), Profit: 5000

    let conn = open_db(Some(db_path.to_path_buf()))?;

    // Month 1: Swing trade loss
    let mut carry = HashMap::new();
    let calc_month1 = calculate_monthly_tax(&conn, 2025, 1, &mut carry)?;
    assert_eq!(calc_month1[0].category, TaxCategory::StockSwingTrade);
    assert_eq!(calc_month1[0].net_profit, dec!(-1000.00));

    // Month 2: Day trade profit - NO offset (different category)
    let calc_month2 = calculate_monthly_tax(&conn, 2025, 2, &mut carry)?;
    let day_trade = calc_month2
        .iter()
        .find(|c| c.category == TaxCategory::StockDayTrade)
        .expect("Day trade not found");
    assert_eq!(day_trade.net_profit, dec!(500.00));
    assert_eq!(day_trade.loss_offset_applied, dec!(0)); // No swing loss applied to day trade
    assert_eq!(day_trade.profit_after_loss_offset, dec!(500.00));
    assert_eq!(day_trade.tax_due, dec!(100.00)); // 20% of 500

    // Month 3: Swing trade profit - still exempt (under R$20k sales)
    let calc_month3 = calculate_monthly_tax(&conn, 2025, 3, &mut carry)?;
    let swing_trade = calc_month3
        .iter()
        .find(|c| c.category == TaxCategory::StockSwingTrade)
        .expect("Swing trade not found");
    assert_eq!(swing_trade.net_profit, dec!(1500.00));
    assert_eq!(swing_trade.loss_offset_applied, dec!(0)); // No offset - profit is exempt
    assert_eq!(swing_trade.taxable_amount, dec!(0)); // Exempt
    assert_eq!(swing_trade.exemption_applied, dec!(1500.00)); // Entire profit exempt
    assert_eq!(swing_trade.tax_due, dec!(0));
    // Carry should still be 1000 (not consumed on exempt profit)
    assert_eq!(carry.get(&TaxCategory::StockSwingTrade), Some(&dec!(1000)));

    // Month 4: Large swing trade profit - now carry IS applied (over R$20k sales)
    let calc_month4 = calculate_monthly_tax(&conn, 2025, 4, &mut carry)?;
    let swing_trade_m4 = calc_month4
        .iter()
        .find(|c| c.category == TaxCategory::StockSwingTrade)
        .expect("Swing trade not found in month 4");
    assert_eq!(swing_trade_m4.net_profit, dec!(5000.00));
    assert_eq!(swing_trade_m4.loss_offset_applied, dec!(1000.00)); // Previous swing loss applied
    assert_eq!(swing_trade_m4.taxable_amount, dec!(4000.00)); // 5000 - 1000
    assert_eq!(swing_trade_m4.exemption_applied, dec!(0)); // No exemption (large sale)
    assert_eq!(swing_trade_m4.tax_due, dec!(600.00)); // 15% of 4000
                                                      // Carry should be consumed
    assert_eq!(carry.get(&TaxCategory::StockSwingTrade), None); // All consumed

    Ok(())
}
