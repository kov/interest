# Agent guidance

## Project Overview

Interest is a Rust CLI/TUI tool for tracking B3 investments, including imports, portfolio/performance reporting, corporate actions, and Brazilian tax calculations.

## Core Invariants (Always)

1. Use `rust_decimal::Decimal` for money/quantity; never use `f64`.
2. Process transactions in chronological order (`ORDER BY trade_date ASC`).
3. Do not allow negative positions (no short selling).
4. Corporate actions are applied at query-time (portfolio/tax/performance), not persisted into transactions.
5. Preserve total cost through quantity adjustments (`quantity * avg_price` remains constant).
6. Invalidate snapshots after transaction/corporate-action changes (`invalidate_snapshots_after`).
7. Keep business logic in dispatch/report/tax/import modules, not in UI/entrypoint glue.

## High-Signal Architecture

- Command surface: `src/commands.rs`
- Command routing: `src/dispatcher.rs`
- Data model/schema: `src/db/models.rs`, `src/db/schema.sql`
- Imports: `src/importers/`
- Corporate actions: `src/corporate_actions/`
- Tax logic: `src/tax/`
- Portfolio/performance: `src/reports/`
- UI layer: `src/ui/`

## Before Modifying

- Check existing module patterns in the target area before introducing new structure.
- If schema/data-flow changes, verify `src/db/schema.sql` and dependent queries.
- If tax/corporate-action behavior changes, add or update integration tests in `tests/`.
- Use `tests/README.md` for test strategy details.

## Useful Commands (Minimal)

```bash
cargo build
cargo test
cargo run -- <command>
cargo run -- interactive
```

For command syntax/details, prefer `--help` and quick source exploration over duplicating docs here.
