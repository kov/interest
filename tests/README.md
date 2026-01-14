# Interest Tracker - Test Suite Documentation

This document covers the test harness, infrastructure, patterns, and best practices for testing Interest Tracker. For guidance on adding tests and understanding the codebase architecture, see `CLAUDE.md`.

## Table of Contents

- [Test Philosophy](#test-philosophy)
- [Test Infrastructure](#test-infrastructure)
- [Binary-Driven Integration Tests](#binary-driven-integration-tests)
- [Test Patterns & Best Practices](#test-patterns--best-practices)
- [Debugging with Asciinema](#debugging-with-asciinema)
- [Running Tests](#running-tests)
- [Test Data Files](#test-data-files)

## Test Philosophy

Interest Tracker uses a **binary-driven integration testing** approach:

- **Test the real CLI**, not just library functions
- **Deterministic**: Tests use isolated temp directories and fixtures, no network calls
- **Fast**: Parallel execution, minimal setup, automatic cleanup
- **Robust**: Parse exact table columns and JSON fields, use `Decimal` for money comparisons
- **Cross-validated**: Portfolio, performance, and tax outputs should agree

This approach catches integration issues that unit tests miss while remaining fast and reliable.

## Test Infrastructure

### Test Harness Architecture

All integration tests use a standardized harness in `tests/integration_tests.rs`:

```rust
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
```

**Key features:**

1. **Isolated HOME**: Each test gets a temp directory → writes to `.interest/data.db` under test temp folder
2. **Fixture cache**: B3 tickers CSV copied to test cache → deterministic asset type resolution
3. **No network**: `INTEREST_SKIP_PRICE_FETCH=1` disables live price fetching
4. **No colors**: `--no-color` ensures clean table parsing
5. **Platform-agnostic**: `XDG_CACHE_HOME` standardizes cache location across macOS/Linux

### B3 Tickers Cache Fixture

**Problem**: Asset type detection requires B3's consolidated instruments CSV. Network access in tests is unreliable and slow.

**Solution**: Pre-generated fixture with minimal test tickers.

**Location**: `tests/fixtures/b3_cache/`

- `tickers.csv` - CSV with columns: `TckrSymb`, `SctyCtgyNm`, `CFICd`, `CrpnNm`
- `tickers.meta.json` - Metadata with future timestamp (never stale)

**How it works**:

```rust
fn setup_test_tickers_cache(cache_root: &Path) {
    let tickers_dir = cache_root.join("interest").join("tickers");
    std::fs::create_dir_all(&tickers_dir).expect("failed to create tickers cache dir");
    std::fs::copy(
        "tests/fixtures/b3_cache/tickers.csv",
        tickers_dir.join("tickers.csv"),
    ).expect("failed to copy tickers.csv fixture");
    std::fs::copy(
        "tests/fixtures/b3_cache/tickers.meta.json",
        tickers_dir.join("tickers.meta.json"),
    ).expect("failed to copy tickers.meta.json fixture");
}
```

The future `fetched_at` date (`2099-01-01T00:00:00Z`) ensures cache is never considered stale.

**Fixture tickers** (as of 2026-01):

| Ticker | Type | Name |
|--------|------|------|
| PETR4, VALE3, ITSA4, MGLU3, BBAS3, ANIM3, DUPL3, KPCA3, SHUL4, AMBP3 | SHARES | Stocks |
| A1MD34 | BDR | Brazilian Depositary Receipt |
| MXRF11, BRCR11 | FUNDS (FII keywords) | Real Estate Funds |
| RZTR11 | FUNDS (FI-INFRA keywords) | Infrastructure Fund |

**Adding new tickers**: When test data files include new tickers, add entries to `tests/fixtures/b3_cache/tickers.csv` with appropriate `SctyCtgyNm` and `CrpnNm` values. See `designs/TESTHARNESS.md` for details.

### Cache Directory Structure

With test harness setup:

```
$XDG_CACHE_HOME/ (points to temp dir)
└── interest/
    ├── tickers/
    │   ├── tickers.csv        (copied from fixture)
    │   └── tickers.meta.json  (copied from fixture)
    └── cotahist/              (if test needs COTAHIST)
        └── COTAHIST_A2025.ZIP (created by test)
```

### Tests With Custom Caches

Tests that create custom `XDG_CACHE_HOME` (e.g., COTAHIST test) must also set up tickers cache:

```rust
#[test]
fn test_portfolio_show_fetches_cached_cotahist_and_shows_prices() -> Result<()> {
    let home = TempDir::new()?;

    // Import uses base_cmd() which sets up tickers cache
    let _import = run_import_json(&home, "tests/data/01_basic_purchase_sale.xlsx");

    // Create custom cache for COTAHIST
    let cache_root = TempDir::new()?;

    // Must also have tickers cache here since we override XDG_CACHE_HOME
    setup_test_tickers_cache(cache_root.path());

    // Now create COTAHIST cache...
    let cotahist_dir = cache_root.path().join("interest").join("cotahist");
    // ... set up ZIP ...

    let mut cmd = Command::new(cargo::cargo_bin!("interest"));
    cmd.env("HOME", home.path());
    cmd.env("XDG_CACHE_HOME", cache_root.path());  // Has both caches
    // ...
}
```

## Binary-Driven Integration Tests

### Principles

1. **Use isolated HOME directory** (`TempDir`) so binary writes to `.interest/data.db` under test temp folder
2. **Keep imports deterministic**: Import via fixtures, no live data
3. **Disable live price fetching**: Set `INTEREST_SKIP_PRICE_FETCH=1`
4. **Prefer `--json` for assertions**: More robust than table parsing
5. **Assert precisely**: Use `rust_decimal::Decimal` comparisons, not string equality
6. **Cross-validate**: Portfolio, performance, and tax outputs should agree

### Recommended Test Flow

Modeled after `test_06_multiple_splits`:

#### 1. Setup

```rust
let home = TempDir::new()?;

// Initialize DB
let db_path = get_db_path(&home);
std::fs::create_dir_all(db_path.parent().unwrap())?;
interest::db::init_database(Some(db_path.clone()))?;
let conn = rusqlite::Connection::open(&db_path)?;

// Import movements
import_movimentacao(&conn, "tests/data/my_case.xlsx")?;

// Import corporate actions (if needed)
// Verify raw transactions remain unadjusted in DB
```

#### 2. Portfolio Assertions (CLI Table)

```rust
let out = base_cmd(&home)
    .env("INTEREST_SKIP_PRICE_FETCH", "1")
    .arg("portfolio").arg("show").arg("--at").arg("2025-05-21")
    .output()?;
assert!(out.status.success());

let stdout = String::from_utf8_lossy(&out.stdout);
let row = stdout.lines().find(|l| l.starts_with("│ TICKR"))
    .expect("Ticker row not found");
let cols: Vec<_> = row.split('│')
    .map(|s| s.trim())
    .filter(|s| !s.is_empty())
    .collect();

// Assert all columns: Ticker, Quantity, Avg Cost, Total Cost, Price, Value, P&L, Return %
assert_eq!(cols[1], "50.00");      // Quantity
assert_eq!(cols[2], "R$ 2,55");    // Avg Cost
assert_eq!(cols[3], "R$ 127,50");  // Total Cost
// With INTEREST_SKIP_PRICE_FETCH=1, Price/Value/P&L are "N/A"
```

**Important**: Parse complete rows with exact column positions. Don't use loose `contains()` checks.

#### 3. Performance Assertions (CLI JSON)

```rust
let perf_out = base_cmd(&home)
    .env("INTEREST_SKIP_PRICE_FETCH", "1")
    .arg("--json").arg("performance").arg("show").arg("2025")
    .output()?;
assert!(perf_out.status.success());

let perf_json: serde_json::Value = serde_json::from_slice(&perf_out.stdout)?;
assert_eq!(perf_json["end_value"].as_str().unwrap(), "127.5");
assert_eq!(perf_json["total_return"].as_str().unwrap(), "0");  // No prices
```

#### 4. Tax Assertions (CLI JSON)

```rust
let tax_out = base_cmd(&home)
    .arg("--json").arg("tax").arg("report").arg("2025")
    .output()?;
assert!(tax_out.status.success());

let tax_json: serde_json::Value = serde_json::from_slice(&tax_out.stdout)?;

// Use Decimal for precise comparisons
use rust_decimal::Decimal;
use std::str::FromStr;

let total_sales = Decimal::from_str(
    tax_json["annual_total_sales"].as_str().unwrap()
)?;
let total_profit = Decimal::from_str(
    tax_json["annual_total_profit"].as_str().unwrap()
)?;

assert!(total_sales > Decimal::ZERO);
assert_eq!(total_profit, dec!(200));  // Exact expected profit
```

### Robustness Tips

- **Always assert DB transactions remain unchanged** post-import (corporate actions applied at query time)
- **Validate column count and exact content** for portfolio rows; avoid loose `contains()`
- **Use `Decimal` equality** when comparing money/quantity; never use `f64`
- **Prefer env-controlled determinism**: `INTEREST_SKIP_PRICE_FETCH=1` for stable outputs

## Test Patterns & Best Practices

### Pattern: Test Skeleton

```rust
#[test]
fn test_my_feature() -> anyhow::Result<()> {
    let home = TempDir::new()?;

    // Setup DB and import trades
    let db_path = get_db_path(&home);
    std::fs::create_dir_all(db_path.parent().unwrap())?;
    interest::db::init_database(Some(db_path.clone()))?;
    let conn = rusqlite::Connection::open(&db_path)?;
    import_movimentacao(&conn, "tests/data/my_case.xlsx")?;

    // Portfolio check
    let out = base_cmd(&home)
        .arg("portfolio").arg("show")
        .output()?;
    assert!(out.status.success());
    // ... assertions ...

    Ok(())
}
```

### Pattern: Decimal Comparisons

```rust
// GOOD: Precise Decimal comparison
let actual = Decimal::from_str(json["value"].as_str().unwrap())?;
let expected = dec!(127.50);
assert_eq!(actual, expected);

// BAD: String comparison (fails on scale differences)
assert_eq!(json["value"].as_str().unwrap(), "127.50"); // Fails if "127.5"

// BAD: f64 comparison (precision errors)
let actual_f64 = 0.1 + 0.2;  // Actually 0.30000000000000004
```

### Pattern: Table Parsing

```rust
// Find the ticker row
let row = stdout.lines()
    .find(|l| {
        let parts: Vec<_> = l.split('│').map(|s| s.trim()).collect();
        parts.get(1).map(|s| *s == "PETR4").unwrap_or(false)
    })
    .expect("Ticker row not found");

// Parse all columns
let cols: Vec<_> = row.split('│')
    .map(|s| s.trim())
    .filter(|s| !s.is_empty())
    .collect();

// Assert by column index (not by substring)
assert_eq!(cols[0], "");         // Empty border
assert_eq!(cols[1], "PETR4");    // Ticker
assert_eq!(cols[2], "70.00");    // Quantity
assert_eq!(cols[3], "R$ 26,66"); // Avg Cost
```

### Pattern: JSON Output

```rust
// Always use --json before the command
let out = base_cmd(&home)
    .arg("--json")           // BEFORE command
    .arg("portfolio")
    .arg("show")
    .output()?;

let json: serde_json::Value = serde_json::from_slice(&out.stdout)?;
```

### Critical Invariants to Test

1. **Decimal precision**: All money/quantity use `Decimal`, never `f64`
2. **Total cost preservation**: After corporate actions, `quantity × price` equals original total
3. **Idempotent actions**: Re-applying corporate actions doesn't change results
4. **No negative positions**: Selling more than owned should error
5. **Query-time adjustments**: Database transactions remain unadjusted; adjustments applied on read

## Debugging with Asciinema

### Purpose

Use terminal cast recordings to capture deterministic, timestamped stdout output for diagnosing UI/progress regressions (e.g., "user doesn't see X" or "UI gets stuck on Y").

### Install & Record

**Install** (macOS):
```bash
brew install asciinema
```

**Recommended patterns**:

```bash
# Single command
asciinema rec runs/my-case.cast --command "INTEREST_SKIP_PRICE_FETCH=1 cargo run -- import tests/data/01_basic_purchase_sale.xlsx"

# Interactive session
asciinema rec runs/my-case.cast
# ... run commands interactively ...
# Press Ctrl-D to stop
```

**For determinism**:
- Set environment flags: `INTEREST_SKIP_PRICE_FETCH=1`
- Record in same environment (TERM, locale)
- Include colors/ANSI for accurate reproduction

### What to Capture

Two kinds of terminal output:

1. **Persistent messages** (with newline) - final status, instrumentation summaries
2. **Transient spinner/overwrites** (`\r`) - progress spinners, in-place updates

Treat both as legitimate progress tokens when analyzing.

### Parsing Casts

High-level workflow:

1. Load cast (JSON array of `[time_offset, event_type, data]`)
2. Filter `event_type == 'o'` (stdout)
3. Optionally strip ANSI escapes:
   - CSI/SGR: `\x1b\[[0-9;?]*[A-Za-z]`
   - OSC: `\x1b\].*?(?:\x07|\x1b\\)`
4. Tokenize into types: SPINNER, PROGRESS, PERSISTENT, RESULT, ERROR, SUCCESS
5. Compute timeline: `(timestamp_ms, token_type, text)`

**Metrics**:
- Token distribution (spinner vs progress vs persistent)
- Max gap between tokens (should be < 800ms)
- Concatenated events (indicates buffered writes)

**Thresholds**:
- **800ms** max gap (catches freezes while tolerating jitter)
- Add ±100–200ms slack for CI variability

### Investigation Checklist

When user reports missing/stuck UI:

1. **Reproduce**: Record deterministic cast with `INTEREST_SKIP_PRICE_FETCH=1`
2. **Parse & search** for expected tokens:
   - **Absent** → program never emitted them (add instrumentation)
   - **Present but late** → emitted only as final summary (add intermediate updates)
   - **Present with gaps** → long CPU/IO work (profile & optimize)
   - **Concatenated** → buffered writes (add flushes)
3. **Check filtering**: Spinner updates but text doesn't change? UI layer may filter
4. **Check flushing**: Messages emitted but not seen? Add `stdout().flush()`
5. **Add instrumentation**: Lightweight prints guarded by env var for checkpoints

### Example Parser (Python)

```python
import json, re

CSI_RE = re.compile(r"\x1b\[[0-9;?]*[A-Za-z]")
OSC_RE = re.compile(r"\x1b\].*?(?:\x07|\x1b\\)")
SPINNER_CHARS = '⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏-/\\|'

with open('runs/import.cast') as f:
    events = json.load(f)

tokens = []
for t, typ, data in events:
    if typ != 'o':
        continue
    s = OSC_RE.sub('', CSI_RE.sub('', data))

    parts = re.split(r'(\r|\n)', s)
    for p in parts:
        if not p.strip():
            continue
        if any(ch in p for ch in SPINNER_CHARS):
            tokens.append((t, 'SPINNER', p.strip()))
        elif 'Parsing' in p or 'Fetching' in p or 'Imported' in p:
            tokens.append((t, 'PROGRESS', p.strip()))

# Compute gaps
for i in range(1, len(tokens)):
    delta = tokens[i][0] - tokens[i-1][0]
    if delta > 0.8:
        print(f'Long gap: {delta}s between {tokens[i-1][2]} and {tokens[i][2]}')
```

### Common Pitfalls

- **Buffered writes** → concatenated messages (flush more frequently)
- **Blocking work** → prevents progress events (offload to threads)
- **Overly aggressive filtering** → drops informative updates
- **Terminal differences** → glyph/SGR code mismatches

See `designs/ASCIINEMA.md` for detailed analysis and test plan.

## Running Tests

### Unit Tests

Located in `#[cfg(test)] mod tests` within each module:

```bash
# Run all unit tests
cargo test --lib

# Run specific module tests
cargo test tax::
cargo test importers::irpf_pdf::
```

### Integration Tests

Located in `tests/`:

```bash
# Run all integration tests
cargo test --test integration_tests

# Run specific test
cargo test --test integration_tests test_portfolio_filters_by_asset_type_fii

# Run tax integration tests
cargo test --test tax_integration_tests

# Run with output (nocapture)
cargo test --test integration_tests -- --nocapture
```

### Test Categories

- `integration_tests.rs` - CLI commands, portfolio, corporate actions, filtering
- `tax_integration_tests.rs` - Tax scenarios (exemptions, loss carryforward, categories)
- `generate_test_files.rs` - Generate Excel fixtures (run with `--ignored`)

### Performance

Test suite runs fast due to:
- Isolated temp databases (no shared state)
- No network calls (fixtures + `INTEREST_SKIP_PRICE_FETCH`)
- Parallel execution
- Automatic cleanup

Typical runtime: **< 5 seconds** for full integration suite.

## Test Data Files

### Location

`tests/data/*.xlsx` - Excel files in B3 Movimentação format

### Generation

Test data is generated programmatically:

```bash
# Generate all test XLS files
cargo test --test generate_test_files -- --ignored --nocapture
```

This creates fixtures in `tests/data/` with known scenarios for testing.

### Key Test Files

- **01_basic_purchase_sale.xlsx** - Basic buy/sell, average cost basis
- **04_stock_split.xlsx** - Stock split 1:2 (desdobro)
- **05_reverse_split.xlsx** - Reverse split 10:1 (grupamento)
- **06_multiple_splits.xlsx** - Multiple splits on same asset
- **07_capital_return.xlsx** - FII capital return (amortização)
- **08_complex_scenario.xlsx** - Multi-event integration test
- **11_bonus_auto_apply.xlsx** - Bonus shares (bonificação)
- **12_desdobro_inference.xlsx** - Absolute adjustment inference

See individual test descriptions in `generate_test_files.rs` for details.

### Adding New Test Data

1. Add entry to `generate_test_files.rs`
2. Generate: `cargo test --test generate_test_files -- --ignored`
3. Add tickers to `tests/fixtures/b3_cache/tickers.csv` if new
4. Write integration test in `integration_tests.rs`
5. Verify: `cargo test --test integration_tests test_my_new_feature`

## Contributing

When adding tests:

1. **Follow binary-driven pattern** - test via CLI, not just library functions
2. **Use test harness** - `base_cmd()` provides isolation and fixtures
3. **Assert precisely** - use `Decimal` for money, exact column positions for tables
4. **Test cross-validation** - portfolio, performance, tax should agree
5. **Keep tests fast** - use fixtures, disable network, isolated databases
6. **Document new patterns** - update this README if introducing new test infrastructure

For architectural guidance and implementation details, see `CLAUDE.md`.
