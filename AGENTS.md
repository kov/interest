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
- `cargo run`: launch interactive TUI mode (default).
- `cargo run -- interactive`: explicitly launch interactive mode.
- `cargo run -- portfolio show`: view current positions.
- `cargo run -- import <file>`: import CEI/Movimentacao.
- `cargo run -- import-irpf <pdf> <year> --dry-run`: preview IRPF imports.
- DB lives at `~/.interest/data.db` (inspect with `sqlite3 ~/.interest/data.db`).

## Interactive Mode (TUI)

The default mode when running `cargo run` with no arguments. Provides a readline-based REPL:

- Commands: `/import <file>`, `/portfolio show`, `/tax report <year>`, `/tax summary <year>`, `/help`, `/exit`
- Slash prefix is optional: `portfolio show` works the same as `/portfolio show`
- Tab completion available for commands
- Command history saved to `~/.interest/.history`
- Exit with `/exit`, `/quit`, `quit`, or Ctrl+D
- Ctrl+C cancels current input without exiting
- When testing it using tools run the program normally without a pipe and then just send commands as you would to the shell

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
