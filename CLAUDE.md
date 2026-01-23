# Agent guidance

## Project Overview

Interest is a dual-mode (CLI + interactive TUI) tool for tracking Brazilian B3 stock exchange investments with automatic price updates, average cost basis calculations, performance tracking, and tax reporting. Written in Rust, it handles complex Brazilian tax rules including swing trade/day trade distinctions, fund quota vintage tracking (pre/post-2026), corporate action adjustments, and historical portfolio snapshots.

The project has undergone a significant architectural overhaul to support both traditional CLI commands and an interactive terminal UI (TUI) mode. The TUI provides a REPL-style interface with readline support, while sharing the same business logic core with the CLI.

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
# Use the `interactive` subcommand to start the TUI
cargo run -- interactive

# Traditional CLI mode - provide a command
cargo run -- portfolio show
cargo run -- import negociacao.xlsx
cargo run -- import-irpf irpf.pdf 2018 --dry-run
cargo run -- tax report 2024
cargo run -- performance show YTD

# JSON output for scripting
cargo run -- portfolio show --json
cargo run -- tax report 2024 --json

# Disable colors (automatic when piped)
cargo run -- portfolio show --no-color
```

### Price Data (COTAHIST)

```bash
# Import B3 COTAHIST for a year (downloads or uses cache)
cargo run -- prices import-b3 2024

# Import from a local ZIP (manual download)
cargo run -- prices import-b3-file ~/Downloads/COTAHIST_A2024.ZIP

# Clear cached COTAHIST ZIPs (optionally a year)
cargo run -- prices clear-cache 2024
```

**Offline mode:** set `INTEREST_OFFLINE=1` to prevent network usage. When enabled, cached COTAHIST ZIPs are used directly and missing cache files return an error.

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
├── cli/          - Legacy CLI interface (clap-based, being phased out)
│   ├── mod.rs    - Command definitions using clap macros
│   └── formatters.rs - Output formatting utilities
├── commands.rs   - Command parser (replaces clap for TUI/CLI dual mode)
├── dispatcher.rs - Command routing to handlers (shared by TUI and CLI)
│   └── performance.rs - Performance command handlers
├── db/           - Database models, schema, and operations
│   ├── models.rs - Core types (Asset, Transaction, CorporateAction, etc.)
│   └── schema.sql - SQLite schema with junction tables
├── importers/    - File parsers (CEI Excel/CSV, Movimentação, IRPF PDF, Ofertas Públicas)
│   ├── file_detector.rs - Auto-detects file format from content
│   ├── cei_excel.rs      - B3/CEI Excel parser
│   ├── cei_csv.rs        - B3/CEI CSV parser
│   ├── movimentacao_excel.rs - B3 Movimentação Excel parser
│   ├── movimentacao_import.rs - Movimentação import logic
│   ├── ofertas_publicas_excel.rs - Ofertas Públicas parser
│   ├── irpf_pdf.rs       - IRPF tax declaration PDF parser
│   └── validation.rs     - Transaction validation logic
├── corporate_actions/ - Split/reverse-split/bonus handling with idempotency
├── tax/          - Brazilian tax calculations
│   ├── cost_basis.rs     - Average cost matching algorithm
│   ├── swing_trade.rs    - 15% tax, R$20k exemption for stocks
│   ├── darf.rs           - DARF payment generation
│   ├── irpf.rs           - Annual IRPF report
│   └── loss_carryforward.rs - Loss offset tracking
├── pricing/      - Price fetching from Yahoo Finance
│   └── yahoo.rs  - Yahoo Finance integration
├── reports/      - Portfolio and performance reports
│   ├── portfolio.rs - Portfolio calculation with snapshot support
│   └── performance.rs - Performance tracking with TWR calculation
├── ui/           - Interactive TUI components
│   ├── mod.rs           - TUI entry point and REPL loop
│   ├── readline.rs      - Rustyline wrapper with completion
│   ├── crossterm_engine.rs - Rendering helpers (tables, spinners)
│   ├── event_loop.rs    - Event loop skeleton (TODO: full implementation)
│   └── overlays.rs      - Overlay system (TODO: file pickers, dialogs)
├── scraping/     - Web scraping utilities
│   └── maisretorno.rs - maisretorno.com scraper
├── error.rs      - Custom error types
├── term_contracts.rs - Term contract handling
├── utils/        - Shared utilities
├── lib.rs        - Library entry point (exports core modules)
└── main.rs       - Application entry point (routes to TUI or CLI)
```

### Data Flow

1. **User Input**:
   - TUI Mode (default): `cargo run` → `launch_tui()` → readline REPL → `parse_command()` → `dispatch_command()`
   - CLI Mode: `cargo run -- <cmd>` → clap parsing → `main()` → legacy handlers → calls same business logic

2. **Import**: File → `file_detector` → Parser → `RawTransaction` → `validation` → Database → Invalidate snapshots

3. **Corporate Actions**: Add action → Apply (adjust transactions) → Track in junction table → Invalidate snapshots

4. **Tax Calculation**: Transactions → average-cost matcher → Cost basis → Tax calculation

5. **Portfolio**: Database → Current positions → Fetch prices → Calculate P&L → (optionally) Save snapshot

6. **Performance**: Load/create snapshots → Calculate TWR → Asset breakdown → Format report

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

**Note**: This junction table approach is from an older implementation that physically modified transactions in the database. The current system applies corporate actions at query-time during portfolio/tax calculations via `apply_forward_qty_adjustments()`, so transactions in the database stay unadjusted. See `designs/FIXEDSPLITS.md` for details.

#### 3. Average Cost Basis Matching

Algorithm in `tax/cost_basis.rs`:

```rust
// Maintains running total quantity and cost
// For each sale:
//   1. Compute avg_cost = total_cost / total_qty
//   2. cost_basis = avg_cost * sold_qty
//   3. Reduce total_cost by cost_basis and total_qty by sold_qty
```

#### 10. Asset Metadata, Registry, and Synthetic Tickers

**Asset type resolution order**: B3 CSV cache → Mais Retorno registry → Ambima scrape fallback. This is implemented in `src/tickers/mod.rs::resolve_asset_type_with_name()` and relies on the registry being populated in `asset_registry`.

**Mais Retorno registry**:

- Sync is shared between explicit `assets sync-maisretorno` and auto-refresh triggered by unknown asset lookups.
- Refresh is throttled via metadata key `registry_maisretorno_refreshed_at` (24h).
- Progress is reported via the shared spinner/progress channel when running in a TTY.

**Bond name parsing (debentures)**:

- Mais Retorno list entries for debentures use a full name like `ELET23 - DEBENTURE ...`.
- We split on `" - "` and store:
  - `ticker`: prefix (e.g., `ELET23`)
  - `name`: remainder (full debenture name + maturity)

**Tesouro Direto synthetic tickers**:

- Synthetic ticker is derived from name via `src/tesouro.rs::ticker_from_name()`.
- Normalization drops month components like `01/2005` → `2005`.
- Example: `Tesouro Prefixado 01/2005` → `TESOURO_PREFIXADO_2005`.

**Critical**: Process transactions in chronological order (`ORDER BY trade_date ASC`).

#### 4. Query-Time Corporate Action Application

**Current implementation:** Corporate actions are applied during calculations (portfolio, tax, performance), not when transactions are added.

**How it works:**

```rust
// When calculating portfolio (src/reports/portfolio.rs):
// 1. Load transactions in chronological order
// 2. Load corporate actions for the asset
// 3. For each transaction date, apply forward adjustments:
crate::corporate_actions::apply_forward_qty_adjustments(
    &mut position.quantity,
    &actions,
    &mut action_idx,
    tx.trade_date,
);
// 4. Database transactions stay unchanged
```

**User experience**: Enter original pre-split quantities. When viewing portfolio or generating reports, the system automatically applies adjustments on-the-fly. No manual "apply" step needed.

#### 5. Import Format Auto-Detection

`importers/file_detector.rs` checks file content, not just extension:

```rust
pub fn detect_file_type(path: &Path) -> Result<FileType> {
    // CSV/TXT → Always CEI
    // Excel → Check sheet names:
    //   - "Movimentação" → Movimentacao format
    //   - "negociação", "ativos" → CEI format
    //   - "Ofertas Públicas" → Ofertas Públicas format
}
```

**Why**: B3 exports have inconsistent naming; content is more reliable than filename.

#### 6. Dual-Mode Architecture (TUI + CLI)

**Pattern**: Single command parser + dispatcher shared by both interfaces.

```rust
// commands.rs - Platform-agnostic command representation
pub enum Command {
    Import { path: String, dry_run: bool },
    PortfolioShow { filter: Option<String> },
    PerformanceShow { period: String },
    // ...
}

pub fn parse_command(input: &str) -> Result<Command, CommandParseError> {
    // Parse both "/import file.xlsx" and "import file.xlsx"
    // Works for readline input AND traditional CLI args
}

// dispatcher.rs - Routes commands to handlers
pub async fn dispatch_command(command: Command, json_output: bool) -> Result<()> {
    match command {
        Command::PortfolioShow { filter } => dispatch_portfolio_show(filter, json_output).await,
        // ... all commands route to same business logic
    }
}
```

**Benefits**:

- Zero code duplication
- Same validation/formatting logic for both modes
- Easy to test (just test the command handlers)
- Gradual migration path (can keep clap while building TUI)

#### 7. Portfolio Snapshot System with Fingerprint Invalidation

**Pattern**: Inspired by IRPF caching system, snapshots are stored with fingerprints for invalidation.

```rust
// reports/portfolio.rs
pub fn compute_snapshot_fingerprint(conn: &Connection, as_of_date: NaiveDate) -> Result<String> {
    // Hash: transaction IDs + quantities + prices + trade_dates
    // For all transactions WHERE trade_date <= as_of_date
    // Similar to IRPF's compute_year_fingerprint()
}

pub fn save_portfolio_snapshot(conn: &Connection, date: NaiveDate, label: Option<String>) -> Result<()> {
    // 1. Calculate portfolio at date
    // 2. Compute fingerprint
    // 3. Save positions + fingerprint to position_snapshots table
}

pub fn get_valid_snapshot(conn: &Connection, date: NaiveDate) -> Result<Option<PortfolioReport>> {
    // 1. Load snapshot from database
    // 2. Compute current fingerprint for that date
    // 3. Compare: if match → return snapshot (cache hit), else → return None
}

pub fn invalidate_snapshots_after(conn: &Connection, earliest_changed_date: NaiveDate) -> Result<()> {
    // Called when transactions added/modified
    // Deletes all snapshots WHERE snapshot_date >= earliest_changed_date
}
```

**Integration hooks**: Add invalidation calls after every transaction modification:

- After import: `invalidate_snapshots_after(earliest_trade_date)`
- After corporate action: `invalidate_snapshots_after(action.ex_date)`
- After manual transaction add/edit: `invalidate_snapshots_after(transaction.trade_date)`

**Why**: Enables fast performance calculations without recalculating full portfolio history every time.

#### 8. Time-Weighted Return (TWR) Calculation

**Purpose**: Measure investment performance independent of cash flows (contributions/withdrawals).

```rust
// reports/performance.rs
pub fn calculate_performance(conn: &mut Connection, period: Period) -> Result<PerformanceReport> {
    // 1. Get start/end dates from period
    // 2. Ensure snapshots exist (compute if missing)
    // 3. Calculate TWR: (end_value - start_value) / start_value * 100
    // 4. Break down by asset type
    // 5. (Future) Account for cash flows for true TWR
}
```

**Periods supported**:

- MTD (Month-to-date)
- QTD (Quarter-to-date)
- YTD (Year-to-date)
- OneYear (last 365 days)
- AllTime (since first transaction)
- Custom (from:to date range)

#### 9. Output Ordering (CLI/TUI)

**Preferred order for date-based lists**: show earlier first (ascending), so later entries appear last.

- Use `ORDER BY <date> ASC` (and tie-breakers like ticker/id ASC) for list-style outputs in CLI/TUI and JSON.
- Keep this consistent across commands such as `actions ... list` and `income detail`.

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

### Adding a New Command

**New pattern** (TUI + CLI dual mode):

1. Add variant to `Command` enum in `src/commands.rs`:

```rust
pub enum Command {
    // ... existing commands
    MyNewCommand { arg1: String, arg2: bool },
}
```

2. Add parsing logic in `parse_command()`:

```rust
"mynewcommand" => {
    let arg1 = parts.next().ok_or(...)?.to_string();
    let arg2 = parts.any(|p| p == "--flag");
    Ok(Command::MyNewCommand { arg1, arg2 })
}
```

3. Add handler in `src/dispatcher.rs`:

```rust
pub async fn dispatch_my_new_command(arg1: &str, arg2: bool, json_output: bool) -> Result<()> {
    // Business logic here
    // Format output based on json_output flag
}
```

4. Wire up in `dispatch_command()`:

```rust
Command::MyNewCommand { arg1, arg2 } => {
    dispatch_my_new_command(&arg1, arg2, json_output).await
}
```

5. (Optional) Add to readline completion patterns in `src/ui/mod.rs`

6. (Optional) Add legacy clap command in `src/cli/mod.rs` for backwards compatibility

**Old pattern** (pure CLI, deprecated):

- Only add to `src/cli/mod.rs` via clap macros
- Add handler in `main.rs`
- Cannot be used from TUI

### Adding a New Importer

1. Create parser in `src/importers/new_format.rs`
2. Return `Vec<RawTransaction>` (for trades) or custom type
3. Add variant to `ImportResult` enum in `importers/mod.rs`
4. Update `file_detector.rs` if auto-detection needed
5. Add handler in `dispatcher.rs` (not `main.rs`)
6. **Important**: Call `invalidate_snapshots_after()` after successful import

See `irpf_pdf.rs` for reference implementation with custom `IrpfPosition` type.

### Adding a New Tax Calculation

1. Define category in `TaxCategory` enum if needed
2. Implement calculation in `tax/` module
3. Use average-cost matcher from `cost_basis.rs` for gains/losses
4. Add DARF payment generation in `darf.rs`
5. Write integration test in `tests/tax_integration_tests.rs`

### Adding a New Performance Metric

1. Add field to `PerformanceReport` struct in `reports/performance.rs`
2. Calculate metric in `calculate_performance()` function
3. Add formatting in `cli/formatters.rs` (for terminal output)
4. Add to JSON serialization (automatic if field is added to struct)

### Handling Corporate Actions

**Key constraint**: `total_cost = quantity × price` must remain constant.

Splits are represented using absolute quantity adjustments (the B3 files already provide the final quantity delta). Apply the adjustment forward-only and recompute the average price to preserve total cost:

```rust
// Example: pre-split 50 @ R$5.00 (total R$250); B3 provides +50 quantity adjustment
let new_quantity = old_quantity + quantity_adjustment; // 50 + 50 = 100
let new_price = total_cost / new_quantity;             // R$250 / 100 = R$2.50
// Total cost stays R$250 ✓
```

Always record the adjustment in the junction table to keep idempotency:

```rust
db::insert_corporate_action_adjustment(&conn, action_id, tx_id,
    old_quantity, new_quantity, old_price, new_price)?;
```

**Important**: Call `invalidate_snapshots_after(action.ex_date)` after applying actions.

## Testing Strategy

**→ For comprehensive test documentation, see [`tests/README.md`](tests/README.md)**

The `tests/README.md` file contains complete documentation on:

- **Test Infrastructure**: Test harness architecture, fixtures, isolation strategies
- **Binary-Driven Integration Tests**: Patterns for testing the real CLI
- **Test Patterns & Best Practices**: Decimal comparisons, table parsing, JSON assertions
- **Debugging with Asciinema**: Terminal cast recordings for UI/progress debugging
- **Test Data Files**: Fixture generation and maintenance

**Quick reference**:

### Unit Tests

Located in `#[cfg(test)] mod tests` within each module:

```bash
cargo test --bin interest           # Run all unit tests
cargo test tax::                    # Run specific module
cargo test                          # Run all unit + integration tests
```

### Integration Tests

Located in `tests/`:

```bash
cargo test --test integration_tests           # All integration tests
cargo test --test tax_integration_tests       # Tax scenarios
cargo run --bin generate_test_fixtures        # Generate fixtures
cargo test --test integration_tests test_portfolio_filters_by_asset_type_fii
cargo test --test tax_integration_tests -- --list
```

**Pattern**: Use isolated `TempDir` with fixtures (see `tests/README.md` for details).

### Corporate Actions: Split Handling (Brazil-specific)

- Splits are modeled with fixed absolute quantity adjustments (`quantity_adjustment`), exactly as provided by B3 files. We do **not** use ratios or multipliers.
- Example: a 1:2 split is stored as `quantity_adjustment = 50` when the pre-split position was 50 → post-split 100. The model applies the absolute adjustment forward-only from the ex-date.
- Total cost is preserved: new quantity increases, average price decreases proportionally (cost unchanged).
- Rationale: Brazilian tax flows use average-cost basis (not FIFO). Fixed absolute adjustments match what CEI/Movimentação exports provide and keep average-cost math correct for tax and portfolio.
- **Query-time application**: Database transactions stay unadjusted. Forward-only adjustments are applied during portfolio/tax/performance calculations via `apply_forward_qty_adjustments()`, ensuring idempotency and no double-application.
- See `designs/FIXEDSPLITS.md` for the design rationale behind this approach.

### Testing TUI in Development

```bash
# Launch TUI for manual testing
cargo run

# Test specific command parsing
cargo test commands::parse_command

# Test dispatcher without TUI
cargo test dispatcher::

# Check readline completion (requires manual inspection)
cargo run
# Then type: /p<TAB> → should complete to /portfolio
```

## Critical Invariants

1. **Decimal precision**: All money/quantity calculations use `Decimal`, never `f64`
2. **Ordering**: Always process transactions by `trade_date ASC`
3. **Total cost preservation**: After corporate actions, `quantity × price` must equal original total
4. **Idempotent actions**: Junction table prevents double-adjustment
5. **No negative positions**: Selling more than owned should error (not short selling)
6. **Snapshot invalidation**: Always call `invalidate_snapshots_after()` when modifying transactions
7. **Command dispatcher isolation**: Never put business logic in `main.rs` or `ui/mod.rs` - only in `dispatcher.rs`

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

### metadata vs import_state Tables

**Critical distinction**: Two separate tables for different purposes.

#### `metadata` Table - Application Settings

**Purpose**: General key-value store for app-level settings and cache timestamps.

**Schema**:
```sql
CREATE TABLE metadata (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
);
```

**Examples**:
- `schema_version` - Database schema version
- `db_created_at` - When database was created
- `registry_maisretorno_refreshed_at` - Last Mais Retorno sync timestamp
- `tesouro_csv_imported_mtime` - File modification time for Tesouro data
- Various cache timestamps and file mtimes

**Functions**: `get_metadata(key)`, `set_metadata(key, value)`, `delete_metadata_by_key(key)`

#### `import_state` Table - Import Cutoff Tracking

**Purpose**: Tracks the latest imported date for each source/type combination to enable incremental imports (avoid re-importing old data).

**Schema**:
```sql
CREATE TABLE import_state (
    source TEXT NOT NULL,           -- 'CEI', 'MOVIMENTACAO', 'OFERTAS_PUBLICAS'
    entry_type TEXT NOT NULL,       -- 'trades', 'corporate_actions', 'income', 'allocations'
    last_date DATE NOT NULL,        -- Latest date successfully imported
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (source, entry_type)
);
```

**Examples**:
- `('MOVIMENTACAO', 'trades', '2026-01-02')` - Latest trade from Movimentação
- `('CEI', 'trades', '2025-12-31')` - Latest trade from CEI
- `('MOVIMENTACAO', 'income', '2026-01-15')` - Latest income event

**How it works**:
1. `get_last_import_date(conn, source, entry_type)` returns the cutoff date
2. Importer skips entries with `date <= last_date` (already imported)
3. `set_last_import_date(conn, source, entry_type, date)` updates cutoff after successful import
4. If `import_state` is empty, falls back to `MAX(date)` from actual data tables

**Functions**: `get_last_import_date(source, entry_type)`, `set_last_import_date(source, entry_type, date)`

**Force reimport**: Delete from `import_state WHERE source = ?` to reset cutoff and allow re-importing old dates.

#### When to Use Which

- **Use `metadata`**: App settings, cache timestamps, schema version, feature flags
- **Use `import_state`**: Import deduplication, tracking what's been imported from each source

**Common mistake**: Trying to store import cutoffs in `metadata` with keys like `last_import_date_MOVIMENTACAO_trades`. This is wrong - use the structured `import_state` table instead.

## External Dependencies

### Price APIs

1. **Yahoo Finance**: Primary, `ticker.SA` format (e.g., `PETR4.SA`)

Rate limiting handled by client code (no auth tokens needed as of 2026).

### PDF Parsing

`pdf-extract` crate extracts text from IRPF PDFs. Format varies by year; regex patterns may need updates for different IRPF versions.

Current support: IRPF 2019 (year 2018 data).

## Recent Architectural Decisions (January 2026)

### 1. TUI vs Pure CLI

**Decision**: Build CLI as primary interface, TUI will evolve as we go.

**Rationale for adding TUI**:

- Better UX for everyday use (no need to remember exact command syntax)
- Readline completion reduces typing
- Future: overlays for file picking, validation, data entry
- Both modes share 100% of business logic (zero duplication)

### 2. Custom Command Parser vs Clap

**Decision**: Replace clap with custom `parse_command()` in `commands.rs`.

**Rationale**:

- Clap designed for traditional CLI, doesn't work with readline input
- Custom parser handles both `/import file.xlsx` (TUI) and `import file.xlsx` (CLI)
- Simpler error handling (return `CommandParseError` instead of exiting)
- Less code (clap macros generate lots of boilerplate)
- Easier to add commands (just add enum variant + match arm)

**Migration path**: Keep clap in `cli/mod.rs` for backwards compatibility during transition.

### 3. Snapshot-Based Performance Tracking

**Decision**: Store portfolio snapshots with fingerprint invalidation (similar to IRPF caching).

**Rationale**:

- Recalculating full portfolio history for every performance query is slow
- Snapshots enable fast date-range queries (MTD, YTD, etc.)
- Fingerprint validation ensures snapshots stay accurate after data changes
- Proven pattern (IRPF caching already works this way)
- Enables future features: backfilling, charting, comparing periods

**Trade-off**: Extra storage (position_snapshots table), but negligible vs. speed gain.

### 4. Dispatcher Pattern for Command Routing

**Decision**: Central `dispatcher.rs` module routes commands to handlers.

**Rationale**:

- Single source of truth for command execution
- Both TUI and CLI call same `dispatch_command()` function
- Easy to add logging/metrics/error handling in one place
- Testable without UI (just test dispatcher functions)
- Clear separation: `main.rs` = entry point, `ui/mod.rs` = UI loop, `dispatcher.rs` = business logic

### 5. Performance Module Structure

**Decision**: Separate `reports/performance.rs` from `reports/portfolio.rs`.

**Rationale**:

- Portfolio = current positions + P&L (snapshot in time)
- Performance = TWR calculation over date ranges (requires 2+ snapshots)
- Different use cases, different inputs/outputs
- Avoids bloating portfolio module with time-series logic

## Files to Check Before Modifying

- **Tax calculations**: Review `README.md` section on Brazilian tax rules
- **Database schema**: Check `src/db/schema.sql` before adding columns/tables
- **Corporate actions**: Review `README.md` section on idempotency design
- **Import formats**: Check `src/importers/` for similar parsers
- **Command routing**: Always add new commands to `commands.rs` + `dispatcher.rs`, not just `main.rs`
