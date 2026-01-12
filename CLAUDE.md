# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Interest is a dual-mode (CLI + interactive TUI) tool for tracking Brazilian B3 stock exchange investments with automatic price updates, average cost basis calculations, performance tracking, and tax reporting. Written in Rust, it handles complex Brazilian tax rules including swing trade/day trade distinctions, fund quota vintage tracking (pre/post-2026), corporate action adjustments, and historical portfolio snapshots.

**Recent Evolution** (January 2026): The project has undergone a significant architectural overhaul to support both traditional CLI commands and an interactive terminal UI (TUI) mode. The TUI provides a REPL-style interface with readline support, while sharing the same business logic core with the CLI.

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
# Launch interactive TUI mode (default if no command given)
cargo run

# Or explicit TUI launch
./target/debug/interest

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
├── pricing/      - Price fetching from Yahoo Finance & Brapi.dev
│   ├── yahoo.rs  - Yahoo Finance integration
│   └── brapi.rs  - Brapi.dev fallback
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
│   └── investing.rs - Investing.com scraper (TODO)
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

**Why**: Prevents double-adjustment when reapplying actions. Safe to run `actions apply` multiple times.

**Implementation pattern** in `corporate_actions/mod.rs`:

1. Check if adjustment exists in junction table
2. If not, apply adjustment and record it
3. If yes, skip (already adjusted)

#### 3. Average Cost Basis Matching

Algorithm in `tax/cost_basis.rs`:

```rust
// Maintains running total quantity and cost
// For each sale:
//   1. Compute avg_cost = total_cost / total_qty
//   2. cost_basis = avg_cost * sold_qty
//   3. Reduce total_cost by cost_basis and total_qty by sold_qty
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

// main.rs - Entry point
fn main() {
    if no_cli_args {
        launch_tui().await  // Interactive mode
    } else {
        // Traditional CLI mode (via clap, calls same handlers)
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

### Integration Test Playbook (Binary-Driven)

This project favors binary-driven integration tests that exercise the real CLI. Use the interest binary for validation and prefer JSON output for machine-robust assertions.

Principles:

- Use an isolated HOME directory (`TempDir`) so the binary writes to `.interest/data.db` under the test temp folder.
- Keep imports deterministic: import trades via helper(s), import corporate actions via the binary or JSON helpers; do NOT mutate transactions for corporate actions—adjustments occur at query time.
- Disable live price fetching for deterministic outputs by setting `INTEREST_SKIP_PRICE_FETCH=1` when calling the binary.
- Prefer `--json` for programmatic checks; when parsing tables, match complete rows and columns (not substring contains).
- Assert quantities, average cost, and total cost precisely using `rust_decimal::Decimal` comparisons rather than string equality to avoid scale differences.
- Cross-validate portfolio, performance, and tax outputs so they agree on end-state values.

Recommended Flow (modeled after `test_06_multiple_splits`):

- Setup

  - Create `TempDir` and initialize DB using the project helper (`init_database(Some(db_path))`).
  - Import movements into the DB using `import_movimentacao(&conn, file)`; then import corporate actions (via binary or helper) and assert they exist; verify raw transactions remain unadjusted in DB.

- Portfolio Assertions (CLI table)

  - Call: `interest portfolio show --at YYYY-MM-DD` at key dates.
  - Parse the table output: find the ticker row beginning with `│ <TICKER>`; split by `│`, trim, and assert all columns: Quantity, Avg Cost, Total Cost, Price, Value, P&L, Return %.
  - For deterministic runs, set `INTEREST_SKIP_PRICE_FETCH=1` so Price/Value/P&L are `N/A` and cost-driven calculations are stable.

- Performance Assertions (CLI JSON)

  - Call: `interest --json performance show <PERIOD>` (e.g., `2025`).
  - Parse JSON fields: `start_value`, `end_value`, `total_return`, `realized_gains`, `unrealized_gains`.
  - For deterministic tests, set `INTEREST_SKIP_PRICE_FETCH=1` and expect values driven by cost snapshots; confirm period end value matches the final portfolio total for the scenario.

- Tax Assertions (CLI JSON)

  - Call: `interest --json tax report <YEAR>`.
  - Parse JSON fields: `annual_total_sales`, `annual_total_profit`, `annual_total_loss`, `annual_total_tax`, and monthly summaries as needed.
  - Compare numeric values using `rust_decimal::Decimal::from_str(...)` to avoid string-scale mismatches (e.g., `90.000` vs `90.00`). Validate that computed profit/loss aligns with average cost basis for sales.

### Asciinema debugging (terminal cast recordings) 🔍

Purpose

- Use terminal cast recordings to capture deterministic, timestamped stdout output for reproducing and diagnosing UI/progress regressions (e.g., "the user doesn't see X" or "the UI gets stuck on Y"). Casts make it possible to analyze what the program emitted and when, independent of the interactive terminal.

Install & record

- Install (macOS): `brew install asciinema`
- Recommended recording patterns:
  - Single command: `asciinema rec runs/my-case.cast --command "<ENV_VARS> && <your command>"`
    - Example: `asciinema rec runs/import.cast --command "INSTRUMENT_ASCIINEMA=1 ./.git/reimport.sh"`
  - Manual: `asciinema rec runs/my-case.cast` → run the action interactively → press Ctrl-D to stop.
- Record in the same environment (TERM, locale, env vars) you intend to debug; include colors/ANSI so spinner glyphs and formatting are preserved unless you intentionally strip them.
- For deterministic runs, set environment flags that make the run reproducible (e.g., `INTEREST_SKIP_PRICE_FETCH=1`) and add instrumentation flags (if available) to print diagnostics or summaries.

What to capture & why it matters

- Two kinds of terminal output matter:
  1. Persistent messages (written with newline) — easy to find; good for final status lines and instrumentation summaries.
  2. Transient spinner/overwrites (written with `\r`) — used for progress spinners and in-place updates; these are the items that often make the UI look "stuck" when they stop changing.
- Capture both kinds; when analyzing, treat `\r`/overwrite events as legitimate progress tokens rather than noise.

Parsing & tokenization (high level)

1. Load the cast (v3 JSON array of events): each event is typically `[time_offset, event_type, data]`.
2. Filter `event_type == 'o'` (stdout) events, and optionally `event_type == 'e'` for stderr if relevant.
3. Preprocess each `data` value:
   - Optionally strip OSC/CSI/ANSI escapes for timeline analysis. Example regexes:
     - CSI/SGR: `\x1b\[[0-9;?]*[A-Za-z]`
     - OSC: `\x1b\].*?(?:\x07|\x1b\\)`
   - Preserve `\r` and `\n` markers; split on these markers but keep `\r` tokens since they represent overwrite updates.
4. Tokenize into canonical token types (examples):
   - SPINNER: contains spinner glyphs or in-place updates
   - PROGRESS: progress messages like "Parsing X (n/m p%)"
   - PERSISTENT: newline-terminated status or instrumentation summaries (consider using a unique prefix for easy detection)
   - RESULT/TICKER: detailed lines (e.g., per-ticker results)
   - ERROR / SUCCESS: messages prefixed with `❌` or `✓`
5. Produce a timeline: ordered `(timestamp_ms, token_type, short_text)` entries.

Diagnostics & metrics to compute

- Token count and distribution (how many spinner vs progress vs persistent messages)
- Maximum gap between consecutive tokens (max_gap) — useful to detect "stuck" periods
- Gaps above threshold (list them with surrounding tokens)
- Concatenated events: single `data` field containing multiple logical tokens (indicates buffered writes)
- Time from last progress token to next persistent summary (useful for detecting delayed finalization)

Heuristics & thresholds

- Default conservative threshold: **800 ms** for max-gap checks (this catches multi-second freezes while tolerating small scheduling jitter). Add ±100–200 ms slack for CI variability.
- For spinner liveliness: if SPINNER tokens repeat without any PROGRESS or PERSISTENT token for longer than threshold, the UI may appear stuck — investigate the code emitting progress updates or the heartbeat that drives spinner ticks.

Investigation checklist (when user reports missing or stuck UI)

1. Reproduce deterministically:
   - Run the same command with any deterministic env vars and with instrumentation enabled (if available).
   - Record a cast (prefer `--command` style) so the recording is deterministic and repeatable.
2. Parse the cast and search for the expected tokens (the thing the user says they don't see):
   - If tokens are **absent**: the issue is upstream (the program never emitted them). Add instrumentation or more granular progress callbacks at logical steps.
   - If tokens are **present** but only as a final PERSISTENT summary: the program emitted them late; consider emitting intermediate persist messages or forwarding intermediate progress to the UI.
   - If tokens are **present** but there are long gaps between them: inspect the two tokens around the gap to see what work was being done; long CPU/IO operations, buffered writes, or long blocking sections are typical causes.
   - If many logical tokens are concatenated into a single `data` event: this usually means writes were buffered and flushed in a burst; consider writing each logical token separately or adding flushes after important updates.
3. If spinner glyphs update (SPINNER tokens) but the progress text doesn't change, check for filtering in the UI layer (the dispatcher/renderer may suppress non-persistent updates).
4. If messages are emitted but not seen in the TUI, check stdout flush behavior — ensure `stdout().flush()` is called after in-place updates.
5. Add small instrumented prints (or lightweight persistent summaries) guarded by an env var to confirm whether work reached certain checkpoints. These are very helpful because they leave permanent traces in the recording for later analysis.

Tools & implementation notes

- Languages: Python quick-scripts are convenient for ad-hoc checks; Rust helpers (using `serde_json` and `regex`) are preferred for CI tests.
- Regexes for cleaning ANSI/OSC shown above; make stripping optional so tests can assert formatting when necessary.
- Normalize spinner glyphs to a common `SPINNER` token to avoid glyph-set differences across terminals.

Using casts in CI

- Keep golden cast fixtures in `tests/data/` and add unit tests that parse them. Typical assertions:
  - Presence of at least one PROGRESS token for the operation under test.
  - `max_gap` ≤ threshold (800 ms default)
  - No single-event sequences that contain many concatenated progress messages (or assert a maximum concatenated token count)
- Prefer conservative thresholds so CI does not flake; if CI shows intermittent failures, increase slack or investigate bottlenecks that cause large scheduling jitter.

Common pitfalls to watch for

- Buffered writes that produce concatenated messages (fix by flushing more frequently for important updates)
- Blocking work in the same thread/task that prevents progress events from being emitted
- Overly aggressive UI filtering that drops non-persistent but informative progress messages
- Terminal differences (glyphs and SGR codes) that affect token recognition

If you need a starter parser or test harness, we have example scripts and Rust helpers under `scripts/` and `src/` to load and analyze casts; reuse them and extend for the checks you need.

- Example (high-level Python pseudo-code):

```python
import json, re
CSI_RE = re.compile(r"\x1b\[[0-9;?]*[A-Za-z]")
OSC_RE = re.compile(r"\x1b\].*?(?:\x07|\x1b\\)")
SPINNER_CHARS = '⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏-/\\|'

with open('runs/import-b3.cast') as fh:
    events = json.load(fh)

tokens = []
for t, typ, data in events:
    if typ != 'o':
        continue
    s = OSC_RE.sub('', data)
    s = CSI_RE.sub('', s)
    # Split on carriage returns and newlines, but keep \r updates as tokens.
    parts = re.split(r'(\r|\n)', s)
    for p in parts:
        if not p.strip():
            continue
        # detect spinner glyphs or known keywords
        if any(ch in p for ch in SPINNER_CHARS):
            tokens.append((t, 'SPINNER', p.strip()))
        elif 'Parsing COTAHIST' in p or 'Fetching prices' in p or 'Imported' in p:
            tokens.append((t, 'PROGRESS', p.strip()))

# compute deltas and check gaps
for i in range(1, len(tokens)):
    delta = tokens[i][0] - tokens[i-1][0]
    if delta > 0.8:
        print('Long gap', delta, tokens[i-1], tokens[i])
```

- Using casts in tests & CI 🔁
  - Add Rust helpers under `tests/support/asciicast.rs` to parse and normalize tokens into a `Vec<Token>`.
  - Add unit tests that parse `working.cast` and `broken.cast` fixtures and assert:
    - `working` has no gaps > 800 ms
    - `broken` has at least one gap > 800 ms and/or concatenated-token events
  - Add an integration test that runs a deterministic small synthetic import (or uses a small test zip), records a cast, parses it, and asserts the `max_gap` is below threshold.
  - Keep golden normalized timelines in `tests/data/` for easy regression diffs.

For more background and the investigative notes from our COTAHIST debugging session, see `designs/ASCIINEMA.md` which contains analysis examples and a proposed test plan.

- Robustness Tips
  - Always assert database transactions remain unchanged post-import (corporate actions are applied forward-only at query time).
  - Validate column count and exact content for portfolio rows; avoid loose `contains()` checks.
  - When comparing money/quantity, use `Decimal` equality; do not use `f64`.
  - Prefer env-controlled determinism: `INTEREST_SKIP_PRICE_FETCH=1` for tests that do not require market prices.

Example Skeleton:

```rust
#[test]
fn my_integration_test() -> anyhow::Result<()> {
        let home = tempfile::TempDir::new()?;

        // Setup DB and import trades
        let db_path = tests::helpers::get_db_path(&home);
        std::fs::create_dir_all(db_path.parent().unwrap())?;
        interest::db::init_database(Some(db_path.clone()))?;
        let conn = rusqlite::Connection::open(&db_path)?;
        tests::helpers::import_movimentacao(&conn, "tests/data/my_case.xlsx")?;

        // Portfolio check (table)
        let out = tests::helpers::base_cmd(&home)
                .env("INTEREST_SKIP_PRICE_FETCH", "1")
                .arg("portfolio").arg("show").arg("--at").arg("2025-05-21")
                .output()?;
        assert!(out.status.success());
        let stdout = String::from_utf8_lossy(&out.stdout);
        let row = stdout.lines().find(|l| l.starts_with("│ TICKR"))
                .expect("Ticker row not found");
        let cols: Vec<_> = row.split('│').map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
        assert_eq!(cols[1], "50.00"); // Quantity
        assert_eq!(cols[2], "R$ 2,55"); // Avg Cost

        // Performance check (JSON)
        let perf_out = tests::helpers::base_cmd(&home)
                .env("INTEREST_SKIP_PRICE_FETCH", "1")
                .arg("--json").arg("performance").arg("show").arg("2025")
                .output()?;
        assert!(perf_out.status.success());
        let perf_json: serde_json::Value = serde_json::from_slice(&perf_out.stdout)?;
        assert_eq!(perf_json["end_value"].as_str().unwrap(), "127.5");

        // Tax check (JSON)
        let tax_out = tests::helpers::base_cmd(&home)
                .arg("--json").arg("tax").arg("report").arg("2025")
                .output()?;
        assert!(tax_out.status.success());
        let tax_json: serde_json::Value = serde_json::from_slice(&tax_out.stdout)?;
        use std::str::FromStr as _;
        let total_sales = rust_decimal::Decimal::from_str(tax_json["annual_total_sales"].as_str().unwrap())?;
        let total_profit = rust_decimal::Decimal::from_str(tax_json["annual_total_profit"].as_str().unwrap())?;
        assert!(total_sales > rust_decimal::Decimal::ZERO);
        assert!(total_profit >= rust_decimal::Decimal::ZERO);

        Ok(())
}
```

Following this playbook ensures your tests validate the true CLI behavior end-to-end and remain stable across formatting changes.

### Corporate Actions: Split Handling (Brazil-specific)

- Splits are modeled with fixed absolute quantity adjustments (`quantity_adjustment`), exactly as provided by B3 files. We do **not** use ratios or multipliers.
- Example: a 1:2 split is stored as `quantity_adjustment = 50` when the pre-split position was 50 → post-split 100. The model applies the absolute adjustment forward-only from the ex-date.
- Total cost is preserved: new quantity increases, average price decreases proportionally (cost unchanged).
- Rationale: Brazilian tax flows use average-cost basis (not FIFO). Fixed absolute adjustments match what CEI/Movimentação exports provide and keep average-cost math correct for tax and portfolio.
- Query-time application: database transactions stay unadjusted; forward-only adjustments are applied when calculating portfolio, performance, and tax, ensuring idempotency and no double-application.

## TUI Development Workflow

The TUI is being built incrementally following the plan in `INCREMENTAL_TUI_PLAN.md`. Current status:

**Phase 1-2 (COMPLETE)**: Foundation + Command Layer

- ✅ Custom error types in `error.rs`
- ✅ Validation extraction in `importers/validation.rs`
- ✅ Command enum + parser in `commands.rs`
- ✅ Dispatcher in `dispatcher.rs`
- ✅ CLI refactored to use new command layer

**Phase 3 (IN PROGRESS)**: TUI Infrastructure

- ✅ Basic readline REPL in `ui/mod.rs`
- ✅ Readline wrapper with completion in `ui/readline.rs`
- ✅ Crossterm rendering helpers in `ui/crossterm_engine.rs`
- 🚧 Overlays system in `ui/overlays.rs` (skeleton exists, needs file picker)
- 🚧 Event loop in `ui/event_loop.rs` (skeleton exists, needs overlay routing)

**Phase 4 (PLANNED)**: Easy Commands → TUI

- Portfolio show, tax report, performance show
- All will reuse existing dispatcher handlers

**Phase 5 (PLANNED)**: Import Workflow

- Interactive file picker, streaming preview, validation overlays
- Most complex command to port

**Phase 6 (PLANNED)**: Performance Tracking Features

- See `PERFORMANCE_TRACKING_PLAN.md` for details
- Snapshot backfilling, live dashboard, B3 COTAHIST import

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

## External Dependencies

### Price APIs

1. **Yahoo Finance**: Primary, `ticker.SA` format (e.g., `PETR4.SA`)
2. **Brapi.dev**: Fallback, Brazilian focus, **no BDR corporate actions**

Rate limiting handled by client code (no auth tokens needed as of 2026).

### PDF Parsing

`pdf-extract` crate extracts text from IRPF PDFs. Format varies by year; regex patterns may need updates for different IRPF versions.

Current support: IRPF 2019 (year 2018 data).

## Recent Architectural Decisions (January 2026)

### 1. TUI vs Pure CLI

**Decision**: Build interactive TUI as primary interface, keep CLI for scripting/automation.

**Rationale**:

- Better UX for everyday use (no need to remember exact command syntax)
- Readline completion reduces typing
- Future: overlays for file picking, validation, data entry
- CLI still available via `cargo run -- <command>` for scripts/automation
- Both modes share 100% of business logic (zero duplication)

**Implementation**: See `INCREMENTAL_TUI_PLAN.md` for phased rollout plan.

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

- **TUI/CLI architecture**: Review `INCREMENTAL_TUI_PLAN.md` before adding commands
- **Performance tracking**: Review `PERFORMANCE_TRACKING_PLAN.md` before adding metrics
- **Tax calculations**: Review `README.md` section on Brazilian tax rules
- **Database schema**: Check `src/db/schema.sql` before adding columns/tables
- **Corporate actions**: Review `README.md` section on idempotency design
- **Import formats**: Check `src/importers/` for similar parsers
- **Command routing**: Always add new commands to `commands.rs` + `dispatcher.rs`, not just `main.rs`
