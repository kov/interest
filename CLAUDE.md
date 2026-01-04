# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Interest is a CLI tool for tracking Brazilian B3 stock exchange investments with automatic price updates, FIFO cost basis calculations, and tax reporting. Written in Rust, it handles complex Brazilian tax rules including swing trade/day trade distinctions, fund quota vintage tracking (pre/post-2026), and corporate action adjustments.

## Commands

### Build & Test

```bash
# Build debug version
cargo build

# Build release version
cargo build --release

# Run all tests
cargo test

# Run specific test module
cargo test irpf                    # IRPF-related tests
cargo test corporate_actions       # Corporate action tests
cargo test tax_integration_tests   # Tax calculation tests

# Run single test
cargo test test_parse_code_31_line_simple

# Run with logging
RUST_LOG=debug cargo test test_name -- --nocapture
```

### Running the Tool

```bash
# Use debug build during development
./target/debug/interest portfolio show

# Or run directly with cargo
cargo run -- portfolio show

# Import a file
cargo run -- import negociacao.xlsx

# Import IRPF PDF
cargo run -- import-irpf irpf.pdf 2018 --dry-run
```

### Database

```bash
# Database location
~/.interest/data.db

# Inspect database
sqlite3 ~/.interest/data.db

# Common queries
sqlite3 ~/.interest/data.db "SELECT * FROM assets"
sqlite3 ~/.interest/data.db "SELECT * FROM transactions ORDER BY trade_date DESC LIMIT 10"
sqlite3 ~/.interest/data.db "SELECT * FROM corporate_actions"
```

## Architecture Overview

### Module Structure

```
src/
├── cli/          - Command-line interface definitions (clap)
├── db/           - Database models, schema, and operations
│   ├── models.rs - Core types (Asset, Transaction, CorporateAction, etc.)
│   └── schema.sql - SQLite schema with junction tables
├── importers/    - File parsers (CEI Excel/CSV, Movimentação, IRPF PDF)
│   ├── file_detector.rs - Auto-detects file format from content
│   ├── cei_excel.rs      - B3/CEI Excel parser
│   ├── movimentacao_excel.rs - B3 Movimentação parser
│   └── irpf_pdf.rs       - IRPF tax declaration PDF parser
├── corporate_actions/ - Split/reverse-split/bonus handling with idempotency
├── tax/          - Brazilian tax calculations
│   ├── cost_basis.rs     - FIFO matching algorithm
│   ├── swing_trade.rs    - 15% tax, R$20k exemption for stocks
│   ├── darf.rs           - DARF payment generation
│   ├── irpf.rs           - Annual IRPF report
│   └── loss_carryforward.rs - Loss offset tracking
├── pricing/      - Price fetching from Yahoo Finance & Brapi.dev
├── reports/      - Portfolio and performance reports
└── main.rs       - CLI handlers and application entry
```

### Data Flow

1. **Import**: File → Parser → RawTransaction → Database
2. **Corporate Actions**: Add action → Apply (adjust transactions) → Track in junction table
3. **Tax Calculation**: Transactions → FIFO matcher → Cost basis → Tax calculation
4. **Portfolio**: Database → Current positions → Fetch prices → Calculate P&L

### Key Design Patterns

#### 1. Decimal Precision (CRITICAL)

**Never use f64 for money.** All financial values use `rust_decimal::Decimal`:

```rust
use rust_decimal::Decimal;
use std::str::FromStr;

// Good
let price = Decimal::from_str("10.51")?;
let total = price * Decimal::from(1926);

// Bad - DO NOT DO THIS
let price = 10.51_f64;  // Precision errors accumulate
```

**Database storage**: Decimals stored as TEXT strings, read using `ValueRef` pattern matching to handle SQLite type affinity:

```rust
match row.get_ref(idx)? {
    ValueRef::Text(bytes) => Decimal::from_str(std::str::from_utf8(bytes)?)?,
    ValueRef::Integer(i) => Decimal::from(i),
    ValueRef::Real(f) => Decimal::try_from(f)?,
    _ => return Err(...)
}
```

#### 2. Corporate Action Idempotency

Uses junction table `corporate_action_adjustments` to track which transactions have been adjusted:

```sql
CREATE TABLE corporate_action_adjustments (
    action_id INTEGER,
    transaction_id INTEGER,
    old_quantity TEXT,    -- Before adjustment
    new_quantity TEXT,    -- After adjustment
    old_price_per_unit TEXT,
    new_price_per_unit TEXT,
    PRIMARY KEY (action_id, transaction_id)
);
```

**Why**: Prevents double-adjustment when reapplying actions. Safe to run `actions apply` multiple times.

**Implementation pattern** in `corporate_actions/mod.rs`:
1. Check if adjustment exists in junction table
2. If not, apply adjustment and record it
3. If yes, skip (already adjusted)

#### 3. FIFO Cost Basis Matching

Algorithm in `tax/cost_basis.rs`:

```rust
// Maintains queue of purchase lots (earliest first)
// For each sale:
//   1. Match against oldest lot first
//   2. Calculate proportional cost: (sold_qty / lot_qty) * lot_cost
//   3. Reduce lot quantity or remove if fully consumed
//   4. Move to next lot if needed
```

**Critical**: Process transactions in chronological order (`ORDER BY trade_date ASC`).

#### 4. Auto-Adjustment of Manual Transactions

When user adds historical transaction in `handle_transaction_add()`:

```rust
// 1. Insert transaction with original values
db::insert_transaction(&conn, &transaction)?;

// 2. Find corporate actions that occurred AFTER this trade
// 3. Apply them in chronological order
let actions_applied = corporate_actions::apply_actions_to_transaction(&conn, tx_id)?;

// 4. User sees: "Auto-applied 2 corporate action(s)"
```

**User experience**: Enter original pre-split quantities; system handles adjustments automatically.

#### 5. Import Format Auto-Detection

`importers/file_detector.rs` checks file content, not just extension:

```rust
pub fn detect_file_type(path: &Path) -> Result<FileType> {
    // CSV/TXT → Always CEI
    // Excel → Check sheet names:
    //   - "Movimentação" → Movimentacao format
    //   - "negociação", "ativos" → CEI format
}
```

**Why**: B3 exports have inconsistent naming; content is more reliable than filename.

### Brazilian Tax Rules Implementation

#### Tax Categories

Defined in `tax/swing_trade.rs`:

```rust
pub enum TaxCategory {
    StockSwingTrade,      // 15%, R$20k/month exempt
    StockDayTrade,        // 20%, no exemption
    FiiSwingTrade,        // 20%, no exemption (pre-2026 quotas)
    FiiSwingTrade2026,    // 17.5%, no exemption (post-2026 quotas)
    FiiDayTrade,          // 20%, no exemption
    // Same pattern for FIAGRO, FI_INFRA
}
```

#### Quota Vintage Tracking (2026 Tax Changes)

Fund quotas use `settlement_date` or `quota_issuance_date` to determine tax rules:

```rust
// Check if quota is pre-2026 (exempt dividends) or post-2026 (5% tax)
let is_quota_pre_2026 = transaction.settlement_date
    .map(|d| d.year() <= 2025)
    .unwrap_or(false);
```

**Database field**: `quota_issuance_date` in transactions table, populated from CEI settlement dates.

#### Loss Carryforward

`tax/loss_carryforward.rs` tracks losses by category:

- Losses offset future gains **within same category** (stocks vs FII vs FIAGRO)
- Cannot offset across categories
- Tracked in `loss_carryforward` table with `(year, month, category, loss_amount)`

## Common Development Tasks

### Adding a New Importer

1. Create parser in `src/importers/new_format.rs`
2. Return `Vec<RawTransaction>` (for trades) or custom type
3. Add variant to `ImportResult` enum in `importers/mod.rs`
4. Update `file_detector.rs` if auto-detection needed
5. Add handler in `main.rs`

See `irpf_pdf.rs` for reference implementation with custom `IrpfPosition` type.

### Adding a New Tax Calculation

1. Define category in `TaxCategory` enum if needed
2. Implement calculation in `tax/` module
3. Use FIFO matcher from `cost_basis.rs` for gains/losses
4. Add DARF payment generation in `darf.rs`
5. Write integration test in `tests/tax_integration_tests.rs`

### Handling Corporate Actions

**Key constraint**: `total_cost = quantity × price` must remain constant.

Example reverse split adjustment (10:1):

```rust
// Before: 1000 @ R$50 = R$50,000
let new_quantity = old_quantity * ratio_to / ratio_from;  // 1000 * 1 / 10 = 100
let new_price = old_price * ratio_from / ratio_to;        // 50 * 10 / 1 = 500
// After: 100 @ R$500 = R$50,000 ✓
```

Always record adjustment in junction table:

```rust
db::insert_corporate_action_adjustment(&conn, action_id, tx_id,
    old_quantity, new_quantity, old_price, new_price)?;
```

## Testing Strategy

### Unit Tests

Located in `#[cfg(test)] mod tests` within each module.

- `importers/irpf_pdf.rs`: Decimal parsing, ticker extraction, line parsing
- `tax/`: Tax calculation edge cases
- `corporate_actions/`: Split ratio calculations

### Integration Tests

Located in `tests/`:

- `integration_tests.rs`: Corporate action application, duplicate detection
- `tax_integration_tests.rs`: Full tax scenarios (exemptions, loss carryforward, multiple categories)

**Pattern**: Use `tempfile::NamedTempFile` for isolated database per test.

### Test Data Generation

`tests/generate_test_files.rs` creates Excel/CSV files for import testing.

Run: `cargo test --test generate_test_files -- --nocapture`

## Critical Invariants

1. **Decimal precision**: All money/quantity calculations use `Decimal`, never `f64`
2. **FIFO ordering**: Always process transactions by `trade_date ASC`
3. **Total cost preservation**: After corporate actions, `quantity × price` must equal original total
4. **Idempotent actions**: Junction table prevents double-adjustment
5. **No negative positions**: Selling more than owned should error (not short selling)

## Common Pitfalls

### SQLite Type Affinity

SQLite stores `Decimal` as TEXT, but may return INTEGER for whole numbers:

```rust
// Always use ValueRef pattern matching
match row.get_ref(column_index)? {
    ValueRef::Text(bytes) => Decimal::from_str(...)?,
    ValueRef::Integer(i) => Decimal::from(i),
    ValueRef::Real(f) => Decimal::try_from(f)?,
    _ => return Err(...)
}
```

### Corporate Action Order

When applying multiple actions, **chronological order matters**:

```rust
// Get actions sorted by ex_date ASC
let actions = get_unapplied_actions_for_transaction(&conn, trade_date)?;
for action in actions {
    apply_adjustment(...);  // Apply oldest first
}
```

### Asset Type Detection

Suffix-based detection has limitations:

- **11** suffix = FII, FIAGRO, FI-INFRA, **or units** (e.g., SAPR11)
- **34** suffix = BDR, **but not all BDRs follow this pattern**

Manual override via database update if auto-detection fails.

### Import Duplicate Detection

Current logic: Same `(asset_id, trade_date, transaction_type, quantity)` = duplicate.

**Edge case**: Two separate buys of same amount on same day will be treated as duplicate. Use `--force` flag (TODO) to override.

## Database Schema Notes

### Junction Table Pattern

`corporate_action_adjustments` tracks many-to-many relationship:
- One action can adjust many transactions
- One transaction can be adjusted by many actions (if multiple splits occurred)

### Foreign Keys Enabled

```sql
PRAGMA foreign_keys = ON;
```

All foreign key constraints are enforced. Deleting an asset cascades to transactions.

### Indexes

Key indexes for performance:

```sql
CREATE INDEX idx_transactions_asset_date ON transactions(asset_id, trade_date);
CREATE INDEX idx_transactions_date ON transactions(trade_date);
CREATE INDEX idx_corporate_actions_asset ON corporate_actions(asset_id, ex_date);
```

Add more indexes if queries become slow (use `EXPLAIN QUERY PLAN`).

## External Dependencies

### Price APIs

1. **Yahoo Finance**: Primary, `ticker.SA` format (e.g., `PETR4.SA`)
2. **Brapi.dev**: Fallback, Brazilian focus, **no BDR corporate actions**

Rate limiting handled by client code (no auth tokens needed as of 2026).

### PDF Parsing

`pdf-extract` crate extracts text from IRPF PDFs. Format varies by year; regex patterns may need updates for different IRPF versions.

Current support: IRPF 2019 (year 2018 data).

## Files to Check Before Modifying

- **Tax calculations**: Review `README.md` section on Brazilian tax rules
- **Database schema**: Check `src/db/schema.sql` before adding columns
- **Corporate actions**: Review `README.md` section on idempotency design
- **Import formats**: Check `src/importers/` for similar parsers
