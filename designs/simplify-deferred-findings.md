# Code Quality Findings

Codebase-wide review organized by priority. Findings 1-6 from the original
income analytics review; 7-11 were income-specific low-priority items.
Findings 12+ come from a broader codebase review.

Items marked **(DONE)** have been addressed.

---

## High Priority

### 1. N+1 DB query patterns in analytics dispatch functions **(DONE)**

**Where:** `src/dispatcher/income.rs`

**Problem:** Each analytics dispatch function looped over every portfolio
position and issued 1-3 DB queries per asset.

**Fixed:** Batch fetch + in-memory partition by ticker. Added `_from_events`
variants to analytics functions.

### 2. Redundant LTM income computation in `dispatch_income_yield` **(DONE)**

**Where:** `src/dispatcher/income.rs`

**Problem:** `calculate_portfolio` already populated `position.ltm_income`,
then `dispatch_income_yield` re-fetched all LTM events from the DB.

**Fixed:** Build `LtmYieldResult` and `AssetYield` directly from enriched
`PositionSummary` fields.

### 3. Duplicated transaction enrichment logic across 3 modules

**Where:** `src/reports/portfolio.rs:179-315`,
`src/reports/performance.rs:349-498`, `src/tax/swing_trade.rs:190-364`

**Problem:** All three contain a nearly identical ~120-line block that:
fetches transactions, builds rename carryover transactions, builds exchange
synthetic buy transactions, sorts, then iterates with interleaved
amortization/exchange-source/corporate-action application. Any bug fix or
feature change must be replicated in three places.

**Fix:** Extract a shared `build_enriched_transactions_for_asset(conn,
asset_id, as_of, assets_by_id)` that returns the fully sorted, enriched
transaction list. All three callers invoke it instead of duplicating the
logic.

**Should address:** Yes. This is the single largest source of duplication
and divergence risk in the codebase.

### 4. Divergent `build_rename_carryover_transaction` implementations

**Where:** `src/reports/portfolio.rs:494-555`,
`src/tax/swing_trade.rs:512-616`

**Problem:** Two implementations with different behavior — portfolio
intentionally skips corporate-action adjustments to the carryover, while
tax applies them via `apply_actions_to_carryover`. This suggests either a
bug in one or an intentional behavioral difference that is undocumented.

**Fix:** Unify into a single implementation with a clear, tested decision
about whether corporate actions should be applied. If the difference is
intentional, document why and add a test covering both behaviors.

**Should address:** Yes. Potential correctness issue.

### 5. `commands.rs` is entirely dead code

**Where:** `src/commands.rs` (1400+ lines)

**Problem:** Declared via `mod commands;` but nothing imports from it. All
enums are `#[allow(dead_code)]`. The file is a hand-rolled TUI command
parser that mirrors the clap `cli::*Commands` enums 1:1. All dispatchers
use the clap enums directly.

**Fix:** Delete `src/commands.rs` and remove `mod commands;` from `main.rs`.

**Should address:** Yes. Over 1400 lines of fully unused code.

---

## Medium Priority

### 6. N+1 DB queries in portfolio/performance/tax asset loops

**Where:** `src/reports/portfolio.rs:161-326`,
`src/reports/performance.rs:349-498`, `src/tax/swing_trade.rs:190-364`

**Problem:** The main asset processing loop issues at least 6 per-item DB
queries per asset (transactions, renames, exchanges, amortizations,
corporate actions, prices). With typical portfolio sizes this is tolerable
due to SQLite's low latency, but architecturally suboptimal.

**Fix:** Batch-fetch transactions for all relevant asset IDs in one query
and group in a HashMap. Natural companion to finding #3 (shared enrichment
function).

**Should address:** Yes, when addressing finding #3.

### 7. N+1 price lookups in `normalize_positions_with_prices`

**Where:** `src/reports/aggregation.rs:63-108`

**Problem:** One `get_price_on_or_before` query per position inside a loop.
Called from both portfolio and performance codepaths.

**Fix:** Batch-fetch prices for all asset IDs with a single query returning
`(asset_id, close_price)` for the latest price on or before the date.

**Should address:** Yes.

### 8. Duplicated `apply_exchange_source_effect` (3 copies)

**Where:** `src/reports/portfolio.rs:557-570`,
`src/reports/performance.rs:503-516`, `src/tax/swing_trade.rs:672-685`

**Problem:** Three identical copies operating on `AvgCostPosition` vs
`AverageCostMatcher`. Logic (spinoff reduces cost, merger clears position)
is the same.

**Fix:** Introduce a trait (e.g., `CostTracker`) that both types implement,
then write one generic function.

**Should address:** Yes. Natural part of finding #3 refactor.

### 9. Duplicated `get_decimal_value` (3 extra copies)

**Where:** `src/reports/portfolio.rs:578-601`,
`src/tax/swing_trade.rs:743-766`, `src/tax/loss_carryforward.rs:301-324`

**Problem:** Three copies of an older implementation; `db::get_decimal_value`
is already `pub` and uses a cleaner `ValueRef` approach.

**Fix:** Delete the three copies and use `crate::db::get_decimal_value`.

**Should address:** Yes. Mechanical cleanup.

### 10. Duplicated transaction fetch functions

**Where:** `src/reports/portfolio.rs:410-470`,
`src/tax/swing_trade.rs:471-510,688-740`

**Problem:** Identical SQL queries and row-mapping logic duplicated between
portfolio and tax modules.

**Fix:** Move to `db/mod.rs` as public functions.

**Should address:** Yes. Natural part of finding #3 refactor.

### 11. Duplicated price-fetch progress logic

**Where:** `src/dispatcher/portfolio.rs:182-297`,
`src/dispatcher/performance.rs:38-118`

**Problem:** Nearly identical blocks for price resolution with progress
reporting, privacy masking closure, env var check, and json-vs-tty
branching.

**Fix:** Extract `ensure_prices_with_ui(conn, assets, price_range, options)`.

**Should address:** Yes.

### 12. Duplicated inconsistency resolution arms

**Where:** `src/dispatcher/inconsistencies.rs:487-546,549-610`

**Problem:** `MissingCostBasis` and `MissingPurchaseHistory` arms are
functionally identical except for the notes string.

**Fix:** Extract a shared `create_resolution_transaction(conn, issue,
resolution, notes_suffix)` helper.

**Should address:** Yes, but low urgency.

### 13. `println!` instead of `options.writer()` in cashflow

**Where:** `src/dispatcher/cashflow.rs:74,79,83,105`

**Problem:** Breaks the output abstraction used everywhere else.

**Fix:** Replace with `options.writer().writeln(&output)?`.

**Blocked:** The `format_cashflow_*` functions take `OutputOptions` by value
(consuming it), so calling `options.writer()` afterward fails the borrow
checker. Fixing this requires changing the format function signatures to
accept `&OutputOptions` — which is a broader change since `println!` is
the pattern used across all dispatchers. Should be addressed as part of a
codebase-wide `println! → writer()` migration that also updates format
function signatures.

### 14. Dead file `src/cli/formatters.rs`

**Where:** `src/cli/formatters.rs` (665 lines)

**Problem:** Never declared as a module in `src/cli/mod.rs`, never compiled.
Stale leftover from before `src/formatters/portfolio.rs` was created.

**Fix:** Delete the file.

**Should address:** Yes.

### 15. Duplicated `asset_type_name()` / missing `AssetType::display_name()`

**Where:** `src/formatters/portfolio.rs:344` (+ dead copy in
`src/cli/formatters.rs`)

**Problem:** Private function mapping `AssetType` to display names. Adding a
new variant requires updating multiple places.

**Fix:** Add `pub fn display_name(&self) -> &'static str` on `AssetType` in
`src/db/models.rs`.

**Should address:** Yes.

### 16. Stringly-typed `asset_type` across CLI (11 occurrences)

**Where:** `src/cli/mod.rs` — `PortfolioCommands::Type`,
`PerformanceCommands::Type`, `CashFlowCommands::Type`,
`IncomeCommands::Type`, `IncomeCommands::Yield`, `TickersCommands::Resolve`,
`AssetsCommands::*`

**Problem:** `String` where a `ValueEnum` enum would give tab-completion,
help text listing valid values, and instant validation at parse time.

**Fix:** Derive `clap::ValueEnum` on `AssetType` (or a wrapper) and use it
throughout.

**Should address:** Yes. Cross-cutting improvement.

### 17. `get_period_dates` uses `Local::now()`

**Where:** `src/reports/performance.rs:83`

**Problem:** Hardcodes `Local::now()` instead of accepting an `as_of`
parameter. Not testable with deterministic dates.

**Fix:** Add an `as_of: NaiveDate` parameter.

**Should address:** Yes.

### 18. `priceable_assets` misuse in performance dispatcher

**Where:** `src/dispatcher/performance.rs:39`

**Problem:** `priceable_assets` is computed for progress count but actual
fetch uses all assets — misleading progress indicator and possibly
unnecessary network calls.

**Fix:** Pass `&priceable_assets` to `ensure_prices_available_with_progress`.

**Should address:** Yes.

### 19. `predict_payment_dates` fetched events twice **(DONE)**

Resolved as part of finding #1 (`_from_events` refactor).

### 20. Formatter categorization duplication **(DONE)**

Extracted `append_categorization_block` helper.

### 21. `categorize_income_event` O(N²) **(DONE)**

Pre-grouped events by `(ticker, event_type)` in `compute_categorized_totals`.

### 22. `asset_type_totals` aggregation duplicated **(DONE)**

Extracted `aggregate_by_asset_type` helper.

---

## Low Priority

### 23. Calendar arithmetic with `Duration::days(365/730/30*N)`

**Where:** `src/reports/income_analytics.rs` (9 places),
`src/reports/portfolio.rs:386`

**Problem:** Doesn't account for leap years. `months_back * 30` drifts
~2 months over 24 months.

**Fix:** Use `chrono::Months` and `checked_sub_months` for proper calendar
subtraction.

**Should address:** Eventually. The imprecision is usually acceptable but
can cause missed or double-counted events at boundaries.

### 24. Duplicated asset type ordering arrays

**Where:** `src/formatters/cashflow.rs:276-289`,
`src/dispatcher/income.rs:197-211`

**Problem:** Two inline arrays with different subsets of the canonical type
ordering.

**Fix:** Define `AssetType::display_order()` as single source of truth.

**Should address:** Eventually. Natural companion to finding #15.

### 25. Empty-state boilerplate duplication

**Where:** All dispatcher modules (11 copies in income alone)

**Problem:** `if is_json { render default } else { writeln info }` repeated
everywhere.

**Fix:** Extract `emit_empty_state(msg, options)`.

**Should address:** Yes, as a codebase-wide refactor.

### 26. Stringly-typed `event_type`, `format`, `transaction_type`, `status`

**Where:** `src/cli/mod.rs:386,487,653,806`

**Problem:** Same class of issue as finding #16 but narrower scope.

**Fix:** `ValueEnum` enums for each.

**Should address:** Natural companion to finding #16.

### 27. Duplicate `parse_asset_type` helper

**Where:** `src/dispatcher/tickers.rs:168`, `src/dispatcher/assets.rs:211`

**Problem:** Identical function in two files.

**Fix:** Use `.parse::<AssetType>()` with consistent `.map_err()` inline,
or move to a shared helper.

**Should address:** Trivial fix.

### 28. Duplicate `is_supported_portfolio_ticker` check

**Where:** `src/tax/swing_trade.rs:191,195`

**Problem:** Same check appears twice in sequence — second is dead code.

**Fix:** Remove the duplicate.

**Should address:** Trivial fix.

### 29. `TaxCategory` missing `Copy` derive

**Where:** `src/tax/swing_trade.rs:12`

**Problem:** Simple enum requiring unnecessary `.clone()` calls.

**Fix:** Add `Copy` to derive list.

**Should address:** Trivial fix.

### 30. `TaxCategory` display name duplicated

**Where:** `src/tax/irpf.rs:382-393`

**Problem:** Manual match block instead of using existing
`TaxCategory::display_name()` method.

**Fix:** Replace with `category.display_name()`.

**Should address:** Trivial fix.

### 31. Duplicated interactive prompt functions in inconsistencies

**Where:** `src/dispatcher/inconsistencies.rs:347-478`

**Problem:** `prompt_missing_cost_basis` and `prompt_missing_purchase_history`
are nearly identical.

**Fix:** Merge into one function with a label parameter.

**Should address:** Low urgency.

### 32. Inconsistent `Income Summary` year-only interface

**Where:** `src/cli/mod.rs:422-434`

**Problem:** Uses `year: Option<i32>` while all other reports use unified
`period: Option<String>` after the command overhaul.

**Fix:** Accept the unified `period` parameter for consistency.

**Should address:** Eventually, for consistency.

### 33. Redundant future-date validation in portfolio

**Where:** `src/dispatcher/portfolio.rs:39-44,118-129`

**Problem:** `resolve_as_of_date` already validates; dispatcher checks again.

**Fix:** Validate in one place only.

**Should address:** Low urgency.

### 34. N+1 pattern in `dispatch_price_update`

**Where:** `src/dispatcher/prices.rs:175-216`

**Problem:** Fetches prices for ALL assets in the database sequentially,
including those with no open positions.

**Fix:** Filter to assets with open positions. Batch DB inserts in a
transaction.

**Should address:** Low urgency. The command is explicitly "update all."
