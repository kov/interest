# Repository Guidelines

## Project Structure & Module Organization
- `src/` holds the Rust CLI and core logic: `cli/` (clap commands), `db/` (SQLite models/schema), `importers/` (CEI/Movimentacao/IRPF PDF), `corporate_actions/`, `tax/`, `pricing/`, `reports/`, plus `main.rs`.
- `tests/` contains integration tests, fixtures in `tests/data/`, and guidance in `tests/README.md`.
- `target/` is Cargo output.

## Architecture & Functionality Overview
- **Data flow**: Import file → parser → DB → reports/tax/portfolio. Corporate actions adjust transactions before ex-date and are tracked for idempotency.
- **Key invariants**:
  - Use `rust_decimal::Decimal` for all money/quantities (never `f64`).
  - FIFO cost basis processes transactions in `trade_date ASC`.
  - Corporate actions must preserve total cost (`qty × price`), and record adjustments in the junction table.
  - Manual transactions are auto-adjusted for later corporate actions.
- **Database**: SQLite at `~/.interest/data.db`. Schema in `src/db/schema.sql` with foreign keys enabled.

## Build, Test, and Development Commands
- `cargo build` / `cargo build --release`: build debug or optimized binaries.
- `cargo run -- <command>`: run CLI (e.g., `cargo run -- portfolio show`).
- `cargo test`: run all tests.
- `cargo test irpf` / `cargo test corporate_actions` / `cargo test tax_integration_tests`: focused suites.
- `cargo test --test generate_test_files -- --ignored`: regenerate XLS fixtures in `tests/data/`.
- `RUST_LOG=debug cargo test test_name -- --nocapture`: debug a single test.

## Coding Style & Naming Conventions
- Standard Rust formatting; use `cargo fmt` when needed.
- Naming: `snake_case` for fns/modules, `CamelCase` for types, `SCREAMING_SNAKE_CASE` for constants.
- Avoid float math; store decimals as TEXT and read via SQLite `ValueRef` matching.

## Testing Guidelines
- Integration tests live in `tests/integration_tests.rs` and `tests/tax_integration_tests.rs`.
- New import scenarios should add fixtures via `tests/generate_test_files.rs`.
- Follow existing test naming (e.g., `test_04_stock_split`).

## Commit & Pull Request Guidelines
- Commit messages use imperative mood (e.g., “Add …”, “Implement …”).
- PRs should include a brief summary, tests run (with commands), and any notable data/CLI output changes.
