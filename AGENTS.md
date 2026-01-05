# Repository Guidelines

## Project Structure & Module Organization
- `src/cli/`: CLI definitions (clap).
- `src/db/`: models, schema, and SQLite operations.
- `src/importers/`: CEI Excel/CSV, Movimentacao, IRPF PDF parsers + file detection.
- `src/corporate_actions/`: split/reverse-split/bonus handling with idempotent adjustments.
- `src/tax/`: average-cost basis, swing/day trade rules, loss carryforward, DARF, IRPF.
- `src/pricing/`: Yahoo/Brapi price fetching.
- `src/reports/`: portfolio/performance output.
- `tests/`: integration tests; `tests/generate_test_files.rs` for fixtures.

## Build, Test, and Development Commands
- `cargo build` / `cargo build --release`: compile debug/release.
- `cargo test`: run all tests.
- `cargo test irpf`, `cargo test corporate_actions`, `cargo test tax_integration_tests`: focused suites.
- `cargo run -- portfolio show`: view current positions.
- `cargo run -- import <file>`: import CEI/Movimentacao.
- `cargo run -- import-irpf <pdf> <year> --dry-run`: preview IRPF imports.
- DB lives at `~/.interest/data.db` (inspect with `sqlite3 ~/.interest/data.db`).

## Coding Style & Naming Conventions
- Rust standard style; keep `rustfmt` defaults (4-space indent).
- Use `rust_decimal::Decimal` for all money/quantity values; never `f64`.
- Keep transaction processing ordered by `trade_date`.
- Use clear, short names; follow Rust conventions (`snake_case`, `CamelCase`).

## Testing Guidelines
- Unit tests live alongside modules under `#[cfg(test)]`.
- Integration tests live in `tests/` and should use isolated temp DBs.
- Add tests for import edge cases and corporate action idempotency.

## Architecture Overview
- Import pipeline: file detector -> parser -> DB transactions.
- Corporate actions adjust historical transactions; adjustments are tracked in a junction table to prevent double application.
- Tax calculations use average cost basis (day trade separated) and apply exemptions/loss carryforward per category.

## Commit & Pull Request Guidelines
- Commit messages are short, imperative, and capitalized (see `git log --oneline`).
- PRs should include a concise description, relevant test output, and any data/assumption changes.

## Agent Notes
- Align changes with `CLAUDE.md`, especially the Decimal precision and corporate action invariants.
