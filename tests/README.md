# Interest Tracker - Test Suite

This directory contains comprehensive integration tests for the Interest Tracker's core functionality.

## Overview

The test suite validates:
- ✅ XLS/Excel file import (Movimentação format)
- ✅ FIFO cost basis calculations
- ✅ Stock splits and reverse splits
- ✅ Corporate action adjustments
- ✅ Adjustment deduplication (no double-counting)
- ✅ Portfolio position calculations
- ✅ Term contract lifecycle (fully implemented)
- ✅ Capital returns / amortization (fully implemented)
- ✅ Day trade detection
- ✅ Multi-asset portfolio calculations

## Test Files

### Test Data Generation

Test XLS files are generated programmatically using Rust:

```bash
# Generate all test data files
cargo test --test generate_test_files -- --ignored
```

This creates 9 test XLS files in `tests/data/`:

1. **01_basic_purchase_sale.xlsx** - Basic buy/sell with FIFO cost basis
   - Buy 100 PETR4 @ R$25.00
   - Buy 50 PETR4 @ R$30.00
   - Sell 80 PETR4 @ R$35.00
   - Tests: FIFO matching, cost basis = R$2,000, profit = R$800

2. **02_term_contract_lifecycle.xlsx** - Term contracts (purchase, expiry, sale)
   - Buy 200 ANIM3T (term) @ R$10.00
   - Liquidation: 200 ANIM3T → ANIM3
   - Sell 100 ANIM3 @ R$12.00
   - Tests: Cost basis transfer, profit = R$200

3. **03_term_contract_sold.xlsx** - Term contract sold before expiry
   - Buy 150 SHUL4T @ R$8.00
   - Sell 150 SHUL4T @ R$9.00 (before liquidation)
   - Tests: Term contracts traded like regular stocks

4. **04_stock_split.xlsx** - Stock split (desdobro) 1:2
   - Buy 100 VALE3 @ R$80.00
   - Split 1:2 → 200 @ R$40.00
   - Buy 50 VALE3 @ R$42.00
   - Sell 150 VALE3 @ R$45.00
   - Tests: Quantity/price adjustments, cost unchanged

5. **05_reverse_split.xlsx** - Reverse split (grupamento) 10:1
   - Buy 1000 MGLU3 @ R$2.00
   - Reverse split 10:1 → 100 @ R$20.00
   - Sell 50 MGLU3 @ R$22.00
   - Tests: Consolidation math, profit = R$100

6. **06_multiple_splits.xlsx** - Multiple splits on same asset
   - Buy 50 ITSA4 @ R$10.00
   - Split 1:2 → 100 @ R$5.00
   - Buy 25 ITSA4 @ R$5.50
   - Split 1:2 → 200 + 50 shares
   - Sell 200 ITSA4 @ R$3.00
   - Tests: Cumulative adjustments

7. **07_capital_return.xlsx** - FII capital return (amortização)
   - Buy 100 MXRF11 @ R$10.00
   - Capital return R$1.00/share → cost basis R$9.00/share
   - Buy 50 MXRF11 @ R$10.50
   - Sell 120 MXRF11 @ R$11.00
   - Tests: Cost basis reduction

8. **08_complex_scenario.xlsx** - Multi-event scenario
   - Multiple purchases, split, sales, term contract liquidation
   - Tests: Integration of all features

9. **09_fi_infra.xlsx** - FI-Infra fund
   - Buy/sell infrastructure fund
   - Tests: Different asset types

10. **10_duplicate_trades.xlsx** - Duplicate trades
   - Two identical buys on the same date/qty/price
   - Tests: Import allows duplicates, no dedup by date/qty

11. **11_bonus_auto_apply.xlsx** - Bonus auto-apply
   - Buy 100 ITSA4
   - Bonus 20% (Bonificação em Ativos)
   - Tests: Auto-apply corporate actions on import

12. **12_desdobro_inference.xlsx** - Desdobro ratio inference
   - Buy 80 A1MD34, Desdobro credit of 560 shares
   - Tests: Infer 1:8 split and auto-apply adjustment

## Running Tests

### Run All Integration Tests

```bash
cargo test --test integration_tests
```

### Run Specific Test

```bash
cargo test --test integration_tests test_04_stock_split
```

### Run with Output

```bash
cargo test --test integration_tests -- --nocapture
```

## Test Results

Current status (as of latest run):

```
running 12 tests
test test_01_basic_purchase_and_sale ............. ok
test test_02_term_contract_lifecycle ............. ok
test test_03_term_contract_sold_before_expiry .... ok
test test_04_stock_split ......................... ok
test test_05_reverse_split ....................... ok
test test_06_multiple_splits ..................... ok
test test_07_capital_return ...................... ok
test test_08_complex_scenario .................... ok
test test_10_day_trade_detection ................. ok
test test_11_multi_asset_portfolio ............... ok
test test_no_duplicate_adjustments ............... ok
test test_position_totals_match .................. ok

test result: ok. 12 passed; 0 failed; 0 ignored
```

## Test Coverage

### ✅ Fully Tested Features

- **Basic Transactions**: Buy/sell with accurate FIFO cost basis
- **Stock Splits**: Both regular (1:2) and reverse (10:1) splits
- **Multiple Splits**: Cumulative adjustments across multiple split events
- **Adjustment Deduplication**: Ensures splits aren't applied twice
- **Portfolio Calculations**: Accurate position totals
- **XLS Import**: Movimentação file format parsing
- **Term Contracts**: Full lifecycle including purchase, liquidation, and cost basis transfer
- **Capital Returns**: Amortization with cost basis reduction
- **Day Trade Detection**: Proper flagging of same-day buy/sell
- **Multi-Asset Portfolio**: Position calculations across different asset types

### 📋 TODO (Future Enhancements)

1. Add tests for:
   - FII/FI-Infra specific tax rules
   - Income events (dividends, JCP)
   - Tax calculations (swing trade, day trade)
   - IRPF reporting
   - Price history tracking

## Test Architecture

### Helper Functions

- `create_test_db()`: Creates temporary SQLite database
- `import_movimentacao()`: Imports XLS file into database
- `get_transactions()`: Fetches transactions with proper decimal handling
- `get_decimal_value()`: Handles INTEGER/REAL/TEXT decimal reading
- `calculate_position()`: Computes total shares and cost

### Test Pattern

Each test follows this structure:

1. Create temporary database
2. Import test XLS file
3. Verify transactions imported correctly
4. Apply corporate actions (if applicable)
5. Calculate cost basis using FIFO
6. Assert expected outcomes (quantities, costs, profits)

## Implementation Highlights

### Recent Improvements

1. **Fixed Decimal Reading** (`src/term_contracts.rs`)
   - Added `get_decimal_value()` helper to properly read decimal values from SQLite
   - Handles INTEGER, REAL, and TEXT types correctly
   - Fixed term contract liquidation processing

2. **Capital Return Support** (`src/corporate_actions/mod.rs`, `src/db/models.rs`)
   - Added `CorporateActionType::CapitalReturn` enum variant
   - Implemented cost basis reduction logic
   - Quantity remains unchanged, cost reduced by amount per share
   - Fully tested with test_07

3. **Comprehensive Test Suite**
   - 12 integration tests covering all core features
   - Test data generated programmatically in Rust
   - All tests passing with ~0.44s execution time

## Contributing

When adding new features:

1. Create test XLS file in `generate_test_files.rs`
2. Add corresponding integration test in `integration_tests.rs`
3. Verify test passes: `cargo test --test integration_tests`
4. Update this README with new test description

## Performance

Test suite runs in ~0.4 seconds:

```
test result: ok. 12 passed; 0 failed; 0 ignored; finished in 0.44s
```

All tests use temporary databases that are automatically cleaned up after completion.

## Test Descriptions

### Core Functionality Tests

- **test_01_basic_purchase_and_sale**: Verifies FIFO cost basis with multiple purchase lots
- **test_02_term_contract_lifecycle**: Tests term contract purchase, liquidation, and cost transfer
- **test_03_term_contract_sold_before_expiry**: Validates term contracts traded before expiry

### Corporate Action Tests

- **test_04_stock_split**: Tests 1:2 split with quantity/price adjustments
- **test_05_reverse_split**: Tests 10:1 reverse split (consolidation)
- **test_06_multiple_splits**: Validates cumulative split adjustments
- **test_07_capital_return**: Tests FII amortization with cost basis reduction
- **test_no_duplicate_adjustments**: Ensures adjustments aren't applied twice

### Complex Scenario Tests

- **test_08_complex_scenario**: Integration test with splits, term contracts, and multiple sales
- **test_10_day_trade_detection**: Validates same-day buy/sell flagging
- **test_11_multi_asset_portfolio**: Tests multi-asset position calculations

### Utility Tests

- **test_position_totals_match**: Verifies portfolio position accuracy
