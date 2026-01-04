# Repository Guidelines

## Project Overview
Interest is a Rust CLI for tracking Brazilian B3 investments. It imports broker files, applies corporate actions, and calculates FIFO cost basis and tax (swing/day trade, FIIs, FIAGRO, FI-Infra), with DARF payment info.

## Architecture & Data Flow
- **Import**: File → importer → normalized transactions in SQLite (`~/.interest/data.db`).
- **Corporate actions**: Actions are applied to pre‑ex‑date transactions and tracked in a junction table to keep adjustments idempotent.
- **Tax**: FIFO matcher computes cost basis, then tax rules and loss carryforward apply.

## Project Structure & Module Organization
- `src/cli/`: clap command definitions.
- `src/db/`: schema, models, queries; decimals stored as TEXT.
- `src/importers/`: CEI/Negociação, Movimentação, IRPF PDF, ofertas públicas.
- `src/corporate_actions/`: split/bonus adjustment engine.
- `src/tax/`: cost basis, swing/day trade rules, DARF, IRPF.
- `src/reports/`: portfolio and performance reports.
- `tests/`: integration tests and generated import fixtures.

## Build, Test, and Development Commands
```bash
cargo build
cargo build --release
cargo test
cargo test irpf
cargo test corporate_actions
./target/debug/interest portfolio show
cargo run -- import <file.xlsx>
```
Use `RUST_LOG=debug` for verbose output when debugging import issues.

## Coding Style & Naming Conventions
- Rust 2021, 4‑space indentation, `snake_case` for functions/vars.
- **Money/quantity must use `rust_decimal::Decimal`** (never `f64`).
- Keep transaction processing ordered by `trade_date ASC` for FIFO accuracy.
- Prefer content-based file detection (`file_detector.rs`) over extensions.

## Testing Guidelines
- Unit tests live with modules; integration tests in `tests/`.
- Use `tempfile::NamedTempFile` for isolated DBs.
- Add fixtures via `tests/generate_test_files.rs` when new import formats are added.

## Commit & Pull Request Guidelines
- Commit messages are short, imperative, and scoped (e.g., “Add ofertas públicas import...”).
- PRs should include: summary, test command run, and any data assumptions (e.g., split ratios, unit vs FII).

## Critical Invariants
- `quantity × price` must remain constant after corporate actions.
- Corporate actions must be idempotent (tracked via junction table).
- No negative positions; sells must be fully covered by prior buys.
