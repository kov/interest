# Interest - B3 Investment Tracker

A Rust CLI tool to track Brazilian B3 stock exchange investments (stocks, FIIs, FIAGROs, FI-INFRAs) with automatic performance tracking, historical analysis, and Brazilian tax calculations.

## Features

- 📊 **Portfolio Tracking**: Real-time portfolio value and performance
- 📈 **Price Updates**: Automatic price fetching from Yahoo Finance and Brapi.dev
- 📄 **Transaction Import**: Import from B3 Área Logada do Investidor Excel/CSV exports
- 💰 **Tax Calculations**: Brazilian tax rules (swing trade, IRPF, amortização)
- 🔄 **Corporate Actions**: Handle stock splits, bonuses, and amortization automatically
- 💾 **Local Storage**: SQLite-compatible database (Limbo) for fast, reliable storage

## Supported Asset Types

- **Stocks** (Ações): Brazilian equities with swing trade tax calculations
- **FII** (Fundos Imobiliários): Real estate investment funds
- **FIAGRO**: Agribusiness investment funds
- **FI-INFRA**: Infrastructure investment funds
- **Bonds**: Government and corporate bonds (future enhancement)

## Installation

```bash
cargo install --path .
```

## Usage

```bash
# Import transactions from B3 export
interest import ~/Downloads/cei_export.xlsx

# Update all prices
interest prices update

# View portfolio
interest portfolio show

# Calculate monthly tax
interest tax calculate --month 12/2025

# Generate annual tax report for IRPF
interest tax report --year 2025
```

## Project Status

🚧 **Under Development** - See [PLAN.md](./PLAN.md) for implementation details.

## Architecture

- **Language**: Rust 2024 edition
- **Database**: Limbo (Rust SQLite rewrite) - async-native, memory-safe
- **CLI**: clap with derive macros
- **Financial Math**: rust_decimal for precise calculations
- **APIs**: Yahoo Finance, Brapi.dev

## Tax Features

### Swing Trade Tax
- 15% on monthly net profits (17.5% if new law passes)
- R$20,000/month exemption for stocks
- FIFO cost basis calculations
- Loss carryforward within asset classes

### IRPF Support
- Annual transaction summaries
- Capital gains/losses reporting
- Amortização tracking (cost basis reduction)
- 2025 vs 2026 quota tax rule handling

### Corporate Actions
- Stock splits (desdobramento)
- Reverse splits (grupamento)
- Bonus shares (bonificação)
- Amortização (capital return from funds)

## Development

See [PLAN.md](./PLAN.md) for the complete implementation plan with phases, database schema, and technical details.

## License

MIT
