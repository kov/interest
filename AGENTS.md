# Repository Guidelines

## Project Structure & Module Organization
- `src/cli/`: clap command definitions and flags.
- `src/db/`: schema (`schema.sql`), models, and SQLite access helpers.
- `src/importers/`: CEI/Negociação, Movimentação, IRPF PDF, ofertas públicas; `file_detector.rs` auto-detects formats.
- `src/corporate_actions/`: split/bonus handling and adjustment tracking.
- `src/tax/`: average-cost matcher, swing/day trade rules, DARF, IRPF, loss carryforward.
- `src/reports/`: portfolio and performance reports.
- `tests/`: integration tests plus generated fixtures (`tests/generate_test_files.rs`).

## Architecture & Data Flow
1. **Import**: file → importer → normalized `transactions` in SQLite (`~/.interest/data.db`).
2. **Corporate actions**: actions adjust pre‑ex‑date transactions and are tracked in `corporate_action_adjustments` to stay idempotent.
3. **Tax**: average-cost matcher (day trade separated) → gains/losses → exemptions and loss carryforward → DARF payments.
4. **Portfolio**: DB positions + price fetch (Yahoo/Brapi) → P&L report.

## Build, Test, and Development Commands
```bash
cargo build
cargo build --release
cargo test
cargo test irpf
cargo test corporate_actions
./target/debug/interest portfolio show
cargo run -- import .git/negociacao.xlsx
```
Use `RUST_LOG=debug cargo test test_name -- --nocapture` for noisy import debugging.

## Developer Workflow
1. Import data into a fresh DB: `rm ~/.interest/data.db && cargo run -- import <file.xlsx>`.
2. Add missing transactions/corporate actions as needed (use original pre‑split values).
3. Validate results with `interest portfolio show` and targeted `sqlite3` queries.
4. When changing importers, add fixtures and an integration test.

## Coding Style & Naming Conventions
- Rust 2021, 4‑space indentation, `snake_case`.
- **Never use `f64` for money/quantity**; use `rust_decimal::Decimal` end‑to‑end.
- Maintain chronological processing (`ORDER BY trade_date ASC`) for cost basis correctness.
- Units with `11` suffix are not always FIIs; avoid hard‑coding assumptions.

## Testing Guidelines
- Unit tests live in-module; integration tests in `tests/`.
- Use `tempfile::NamedTempFile` for isolated DBs.
- When adding import formats, extend `tests/generate_test_files.rs` and add an integration test.

## Commit & Pull Request Guidelines
- Commit messages are short, imperative, and scoped (e.g., `Add ofertas públicas import...`).
- PRs should include: summary, test command(s), and data assumptions (e.g., split ratios).

## Configuration & Invariants
- Database path: `~/.interest/data.db`; inspect with `sqlite3 ~/.interest/data.db`.
- `quantity × price` must remain constant across corporate action adjustments.
- Corporate actions must be idempotent (use the junction table).
- No negative positions; sells must be fully covered by prior buys.
