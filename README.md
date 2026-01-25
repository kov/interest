# Interest - Brazilian B3 Investment Tracker

[![CI](https://github.com/kov/interest/actions/workflows/ci.yml/badge.svg)](https://github.com/kov/interest/actions/workflows/ci.yml)

A command-line tool for tracking investments on the Brazilian B3 stock exchange. Interest handles your complete investment workflow: import transactions from B3 exports, track your portfolio in real-time, calculate performance metrics, manage corporate actions (splits, renames, spin-offs), and generate accurate tax reports following Brazilian IRPF rules.

**Key Features:**

- 📊 Real-time portfolio tracking with automatic price updates
- 📈 Performance analytics (MTD, QTD, YTD, custom periods)
- 💰 Income tracking (dividends, JCP, amortization)
- 🧾 Brazilian tax calculations (swing trade, day trade, IRPF reports)
- 🔄 Corporate action management (splits, renames, mergers, spin-offs)
- 📥 Import from B3/CEI Excel exports (Negociação, Movimentação, IRPF PDFs)
- 🎯 Interactive TUI with command history and tab completion

**Target Audience:** Brazilian investors trading on B3 who need accurate cost basis tracking and tax reporting.

---

## Installation

### Prerequisites

- **Rust 1.70+** for building from source ([Install Rust](https://rustup.rs/))
- **SQLite 3.x** (usually pre-installed on Linux/macOS)

### Building

```bash
git clone https://github.com/your-username/interest
cd interest
cargo build --release
```

The compiled binary will be at `./target/release/interest`.

### Quick Test

```bash
# Use the `interactive` subcommand to start the TUI
./target/release/interest interactive

# Or test a command
./target/release/interest help
```

---

## Getting Started: Complete Setup Workflow

Follow these 6 steps to set up Interest with your investment data. This workflow handles the common case where you have pre-2020 positions (before B3's complete digital records began).

### Step 1: Add Opening Balances

**Why:** B3's "Negociação" exports only have complete data from 2020 onwards. For any positions you held before 2020, you'll need to add opening balances manually.

**Choose a reference date:** Pick a date in 2019 (e.g., `2019-12-31`) and use it consistently for all opening balances.

**Add your positions:**

```bash
# Syntax: interest transactions add <TICKER> buy <QUANTITY> <PRICE> <DATE>

# Example: Add opening balances for stocks and FIIs
interest transactions add PETR4 buy 200 28.50 2019-12-31
interest transactions add VALE3 buy 150 52.30 2019-12-31
interest transactions add XPLG11 buy 50 120.00 2019-12-31
interest transactions add HGLG11 buy 75 135.50 2019-12-31
```

**Attention:** the price should be your average purchase price, not the market price.

### Step 2: Export Data from B3

**Navigate to B3 Investor Portal:**

1. Go to https://www.investidor.b3.com.br/
2. Log in with your CPF and password
3. Go to **"Extratos e Informativos"** → **"Negociação de Ativos"**

**Export both files:**

**File 1: Negociação de Ativos** (Trades)

- Set date range: From your opening balance date (e.g., 2020-01-01) to today
- Click **"Exportar"** and choose **Excel** format
- Save - let's call this `negociacao.xlsx`

**File 2: Movimentação** (Corporate Actions & Income)

- Go to **"Extratos e Informativos"** → **"Movimentação"**
- Set the same date range
- Click **"Exportar"** and choose **Excel** format
- Save - let's call this `movimentacao.xlsx`

### Step 3: Import Negociação (Trades)

Import your trades first to establish your transaction history.

**Preview first (recommended):**

```bash
interest import negociacao.xlsx --dry-run
```

This shows what would be imported without actually saving anything.

**Import for real:**

```bash
interest import negociacao.xlsx
```

**What gets imported:**

- All buy/sell transactions
- Trade dates and settlement dates
- Fees and brokerage costs
- Asset types (auto-detected from ticker suffixes)

**Duplicate detection:** The tool automatically skips duplicate transactions, so it's safe to re-import the same file.

### Step 4: Import Movimentação (Corporate Events)

Now import corporate actions, dividends, and other events.

```bash
interest import movimentacao.xlsx
```

**What gets imported:**

- Dividends and JCP (Juros sobre Capital Próprio)
- Stock splits and bonuses
- Subscription rights and conversions
- Transfers and other corporate events

**Note:** Some events (like subscription conversions without cost basis) may create **inconsistencies** that you'll need to resolve in the next step.

### Step 5: Resolve Inconsistencies

Some imported events may have missing information. Interest tracks these as "inconsistencies" that you can resolve interactively.

**Resolve with guided experience (recommended):**

```bash
interest inconsistencies resolve
```

The tool will prompt you interactively for required fields (price, fees, dates, etc.). This is easier than trying to guess which fields are needed.

**Check for open issues:**

```bash
interest inconsistencies list --open
```

**Common issue types:**

- **MissingCostBasis**: Subscription conversions where the original cost isn't in the B3 export
- **MissingPurchaseHistory**: Sales without matching purchase records (usually pre-2020 positions)
- **InvalidTicker**: Tickers that couldn't be auto-detected

**View details for a specific issue:**

```bash
interest inconsistencies show 42
```

**Advanced: Set fields directly if you know what's needed:**

```bash
interest inconsistencies resolve 42 --set price_per_unit=18.75 --set fees=5.00
```

**Ignore if not relevant:**

```bash
interest inconsistencies ignore 42 --reason "Duplicate entry from old statement"
```

### Step 6: Add Corporate Actions Manually (If Needed)

**Good news:** Most corporate actions are automatically imported from B3 Movimentação files. You typically only need to add corporate actions manually for **rare events** that B3 doesn't track well.

**Common cases requiring manual entry:**

**Ticker Renames:**

```bash
# Via Varejo became Casas Bahia and changed ticker from VIIA3 to BHIA3
interest actions rename add VIIA3 BHIA3 2023-01-15
```

**Spin-offs:**

```bash
# GPA (Pão de Açúcar) spun off Assaí as a separate company
# You need to specify how many ASAI3 shares you got and allocate cost (value per share * number of shares)
interest actions spinoff add PCAR3 ASAI3 2021-03-01 100 5000
```

**Mergers:**

```bash
# B2W (BTOW3) and Americanas (AMER3) merged into Lojas Americanas (LAME3)
# You need to specify how many LAME3 shares you got and allocate cost (value per share * number of shares)
interest actions merger add BTOW3 LAME3 2021-05-01 200 12000
interest actions merger add AMER3 LAME3 2021-05-01 150 8000
```

**Verify:**

```bash
interest actions rename list
interest actions spinoff list
interest actions merger list
```

---

## Daily Operations

### View Your Portfolio

**Full portfolio with current prices:**

```bash
interest portfolio show
```

**Filter by asset type:**

```bash
interest portfolio show --asset-type fii
interest portfolio show --asset-type stock
interest portfolio show --asset-type fiagro
```

**Historical snapshot (portfolio as of a specific date):**

```bash
interest portfolio show --at 2024-12-31
interest portfolio show --at 2024-06
interest portfolio show --at 2023
```

The output includes:

- Current quantity and average cost basis
- Current market price
- Position value and unrealized P&L (amount and %)
- Total portfolio value and summary by asset type

### Check Performance

**Common time periods:**

```bash
# Year-to-date
interest performance show YTD

# Month-to-date
interest performance show MTD

# Quarter-to-date
interest performance show QTD

# Last 12 months
interest performance show 1Y

# All time (since first transaction)
interest performance show ALL

# Specific year
interest performance show 2024
```

**Custom date range:**

```bash
interest performance show 2024-01-01:2024-12-31
interest performance show 2024-06:2024-12
```

Performance metrics include Time-Weighted Return (TWR), absolute gains, and breakdown by asset type.

### View Income (Dividends & JCP)

**Summary by asset:**

```bash
interest income show
interest income show 2024
```

**Detailed events for a year:**

```bash
interest income detail 2024
```

**Filter by specific asset:**

```bash
interest income detail 2024 --asset XPLG11
```

**Monthly breakdown:**

```bash
# Monthly totals for a year
interest income summary 2024

# All years
interest income summary
```

### Generate Tax Reports

**Annual IRPF report:**

```bash
interest tax report 2024
```

This generates a complete report including:

- Monthly swing trade tax calculations
- Loss carryforward tracking
- Bens e Direitos (assets held on Dec 31)
- Income received (dividends, JCP)
- Transactions summary

**Export to CSV for spreadsheet import:**

```bash
interest tax report 2024 --export
```

**Quick summary (condensed view):**

```bash
interest tax summary 2024
```

---

## Common Operations

### Manage Assets

**List all assets:**

```bash
interest assets list
```

**Filter by type:**

```bash
interest assets list --type fii
interest assets list --type stock
interest assets list --type bdr
```

**Show details for a specific asset:**

```bash
interest assets show PETR4
```

**Set or update asset type:**

```bash
interest assets set-type XPLG11 fii
```

**Set or update asset name:**

```bash
interest assets set-name XPLG11 "XP Logística FII"
```

**Sync with Mais Retorno registry:**

This is usually performed automatically for you as needed.

```bash
# Preview what would be synced
interest assets sync-maisretorno --dry-run

# Actually sync
interest assets sync-maisretorno

# Sync only specific asset type
interest assets sync-maisretorno --type fii
```

### Update Ticker Registry

The ticker registry caches metadata about B3 tickers (asset types, names). It refreshes automatically if needed, but you can manually update it.

**Check cache status:**

```bash
interest tickers status
```

**Force refresh:**

```bash
interest tickers refresh --force
```

**List unknown tickers:**

```bash
interest tickers list-unknown
```

**Manually resolve a ticker:**

```bash
interest tickers resolve XPTO11 --type fii
```

### Import Historical Prices (B3 COTAHIST)

For accurate historical performance calculations, complete price history is imported on demand from B3's COTAHIST files and cached (see relevant directories at the bottom). You can also manage that manually.

**Import specific year:**

```bash
interest prices import-b3 2024
```

The tool downloads the COTAHIST file from B3 and imports all daily prices.

**Import from local file:**

```bash
interest prices import-b3-file ~/Downloads/COTAHIST_A2024.ZIP
```

**Clear cached price data:**

```bash
interest prices clear-cache 2024
```

---

## Corporate Actions Reference

Quick reference for all corporate action types. Remember: most splits are imported automatically from B3 Movimentação files, so manual entry is typically only needed for renames, spin-offs, and mergers.

### Splits & Reverse-Splits

**Add a split (quantity increases):**

```bash
# Add 100 shares per share held
interest actions split add PETR4 100 2022-03-15
```

**Add a reverse-split (quantity decreases):**

```bash
# 10:1 reverse split (1000 shares become 100, so -900 adjustment)
interest actions split add A1MD34 -900 2022-11-22
```

**List all splits:**

```bash
interest actions split list
```

**List splits for specific ticker:**

```bash
interest actions split list PETR4
```

**Remove a split:**

```bash
interest actions split remove 5
```

### Renames

**Add a ticker rename:**

```bash
interest actions rename add VIIA3 BHIA3 2023-01-15
```

**List all renames:**

```bash
interest actions rename list
```

**List renames for specific ticker:**

```bash
interest actions rename list VIIA3
```

**Remove a rename:**

```bash
interest actions rename remove 3
```

### Bonuses

**Add bonus shares:**

```bash
# 10% bonus (50 additional shares per 100 held)
interest actions bonus add ITSA4 50 2023-05-10 --notes "10% bonus declared"
```

**List bonuses:**

```bash
interest actions bonus list
interest actions bonus list ITSA4
```

**Remove a bonus:**

```bash
interest actions bonus remove 7
```

### Spin-offs & Mergers

**Add a spin-off (company splits into two entities):**

```bash
# Syntax: spinoff add <FROM> <TO> <DATE> <QUANTITY> <ALLOCATED_COST>
interest actions spinoff add PCAR3 ASAI3 2021-03-01 100 5000 --notes "Assaí spin-off"

# Optional: add cash component
interest actions spinoff add PCAR3 ASAI3 2021-03-01 100 5000 --cash 250.00
```

**Add a merger (two companies combine):**

```bash
# Syntax: merger add <FROM> <TO> <DATE> <QUANTITY> <ALLOCATED_COST>
interest actions merger add BTOW3 LAME3 2021-05-01 200 12000 --notes "B2W merger"
interest actions merger add AMER3 LAME3 2021-05-01 150 8000 --notes "Americanas merger"

# Optional: add cash component
interest actions merger add BTOW3 LAME3 2021-05-01 200 12000 --cash 500.00
```

**List spin-offs and mergers:**

```bash
interest actions spinoff list
interest actions merger list

# Filter by ticker
interest actions spinoff list PCAR3
interest actions merger list BTOW3
```

**Remove:**

```bash
interest actions spinoff remove 8
interest actions merger remove 9
```

### How Corporate Actions Work

Corporate actions are applied **automatically** during portfolio and tax calculations. When you view your portfolio or generate a tax report, the system:

1. Reads your transactions from the database (unchanged)
2. Applies split/rename/merger adjustments in chronological order
3. Shows you the adjusted quantities and prices

**Key benefits:**

- No separate "apply" step needed - just add the action and it works
- Database transactions stay unchanged (easier to debug and audit)
- No risk of double-adjustment bugs
- Automatic recalculation whenever you view reports

---

## Files & Directories

### Database Location

```
~/.interest/data.db
```

This SQLite database contains all your data:

- Transactions (buys, sells)
- Assets (tickers, types, names)
- Corporate actions (splits, renames, mergers)
- Price history
- Income events (dividends, JCP)
- Portfolio snapshots
- Tax calculations

**Backup your database regularly:**

```bash
# Create timestamped backup
cp ~/.interest/data.db ~/.interest/data.db.backup-$(date +%Y%m%d)

# Before major changes (reimport, bulk edits)
cp ~/.interest/data.db ~/.interest/data.db.backup-pre-import
```

**Inspect with SQLite CLI:**

```bash
sqlite3 ~/.interest/data.db "SELECT * FROM assets LIMIT 10"
sqlite3 ~/.interest/data.db "SELECT * FROM transactions ORDER BY trade_date DESC LIMIT 20"
sqlite3 ~/.interest/data.db "SELECT ticker, quantity, trade_date FROM transactions WHERE ticker = 'PETR4'"
```

### Cache Directories

Cache location varies by operating system (following XDG standards via the `dir_spec` crate):

- **Linux**: `~/.cache/interest/`
- **macOS**: `~/Library/Caches/interest/`
- **Windows**: `%LOCALAPPDATA%\interest\cache\`

**Cache subdirectories:**

- `tickers/` - B3 ticker registry (CSV from B3 website, refreshed daily)
- `cotahist/` - B3 historical price data (yearly COTAHIST ZIP files)
- `tesouro/` - Tesouro Direto bond pricing data

**Clearing cache:**
It's safe to delete cache directories at any time. Data will be re-downloaded automatically when needed.

```bash
# Linux
rm -rf ~/.cache/interest/

# macOS
rm -rf ~/Library/Caches/interest/

# Windows PowerShell
Remove-Item -Recurse -Force "$env:LOCALAPPDATA\interest\cache"
```

**Reference:** See https://docs.rs/dir_spec/latest/dir_spec/fn.cache_home.html for details on platform-specific cache paths.

---

## Troubleshooting

### "Insufficient Purchase History" Error

**Error message:**

```
Error: PETR4: Insufficient purchase history: Selling 100 units but only 50 available.
```

**Causes:**

1. Missing pre-2020 transactions (B3 Negociação limitation)
2. Corporate action not applied (quantities still pre-split)
3. Subscription rights or transfers not imported
4. Pre-CEI data not entered manually

**Solutions:**

**Add missing historical purchases:**

```bash
interest transactions add PETR4 buy 100 25.50 2018-06-15
```

**Check if corporate actions are recorded:**

```bash
# List splits for this ticker
interest actions split list PETR4

# If the split exists, it's already being applied automatically during calculations
# If it doesn't exist, add it with: interest actions split add PETR4 <adjustment> <date>
```

**Review inconsistencies:**

```bash
interest inconsistencies list --open --asset PETR4
```

### Unknown Ticker Error

**Error message:**

```
Error: Unknown ticker: XPTO11
```

**Causes:**

- Ticker not in B3 registry cache
- Ticker delisted or recently listed
- Typo in ticker symbol

**Solutions:**

**Refresh ticker cache:**

```bash
interest tickers refresh --force
```

**Manually resolve ticker:**

```bash
# If you know it's a FII
interest tickers resolve XPTO11 --type fii

# Or add it directly as an asset
interest assets add XPTO11 --type fii --name "XPTO Fundo Imobiliário"
```

**Check for typos:**

```bash
# List all known assets
interest assets list | grep XPTO
```

### Price Fetch Failures

**Warning message:**

```
Warning: Failed to fetch price for PETR4: 404 Not Found
```

**Causes:**

- Ticker delisted, suspended, or renamed
- Temporary API issue (Yahoo Finance)
- BDR without Brazilian listing data

**Solutions:**

**Retry (APIs can be temporarily unavailable):**

```bash
interest portfolio show
```

**Use historical prices from COTAHIST:**

```bash
interest prices import-b3 2024
```

**Check ticker validity:**

```bash
interest tickers status
interest assets show PETR4
```

**For delisted tickers:**

```bash
# You can still view historical data
interest portfolio show --at 2023-12-31
```

### Inconsistency Won't Resolve

**Error message:**

```
Error: Missing required field: price_per_unit
```

**Cause:** Tried to resolve an inconsistency without providing all required data.

**Solution:**

**View full details to see what's needed:**

```bash
interest inconsistencies show 42
```

**Use guided resolution (recommended):**

```bash
interest inconsistencies resolve 42
```

The tool will prompt for each required field.

**Or provide all fields at once:**

```bash
interest inconsistencies resolve 42 \
  --set price_per_unit=18.75 \
  --set fees=12.34 \
  --set trade_date=2023-08-02
```

### Import Detects Duplicates

**Message:**

```
Skipped 15 duplicate transactions
```

**Cause:** You're re-importing a file that was already imported, or importing overlapping date ranges.

**Behavior:** This is **normal and safe**. The tool automatically skips duplicate transactions based on:

- Same ticker
- Same trade date
- Same transaction type (buy/sell)
- Same quantity

**No action needed** - duplicates are silently skipped to avoid double-counting.

---

## Advanced Usage

### Interactive TUI Mode

**Launch interactive mode, when installed:**

```bash
interest interactive

# Or through cargo
cargo run -- interactive
```

**Features:**

- Command history (use ↑/↓ arrows)
- Tab completion for commands and tickers
- Progress indicators for long operations (imports, price fetches)
- Multi-line editing

**Exit:**

```
exit
quit
q
```

### JSON Output for Scripting

Most commands support `--json` flag for machine-readable output:

**Portfolio as JSON:**

```bash
interest portfolio show --json > portfolio.json
```

**Tax report as JSON:**

```bash
interest tax report 2024 --json > tax-2024.json
```

**Income details as JSON:**

```bash
interest income detail 2024 --json > income-2024.json
```

**Parse with jq:**

```bash
# Extract only FII positions
interest portfolio show --json | jq '.positions[] | select(.asset_type == "FII")'

# Get total portfolio value
interest portfolio show --json | jq '.summary.total_value'
```

### Dry-Run Mode

Preview changes before committing:

**Import preview:**

```bash
interest import negociacao.xlsx --dry-run
```

Shows what would be imported (transactions, new assets, duplicates) without saving to database.

**Asset sync preview:**

```bash
interest assets sync-maisretorno --dry-run
```

Shows what assets would be added/updated from Mais Retorno registry.

### Cash Flow Analysis

Track money in/out of your portfolio:

**Show cash flows for a period:**

```bash
# Specific year
interest cash-flow show 2024

# Year-to-date
interest cash-flow show YTD

# All time
interest cash-flow show ALL

# Custom range
interest cash-flow show 2024-01:2024-06
```

**Get statistics:**

```bash
interest cash-flow stats YTD
```

Shows:

- Total inflows (purchases)
- Total outflows (sales, fees)
- Net cash flow
- Number of transactions

---

## Tips & Best Practices

1. **Always use `--dry-run` first** when importing large files to preview what will change

2. **Backup your database regularly** before major operations:

   ```bash
   cp ~/.interest/data.db ~/.interest/data.db.backup-$(date +%Y%m%d)
   ```

3. **Resolve inconsistencies promptly** - they can block accurate tax calculations and portfolio valuations

4. **Keep corporate actions up-to-date** - check [Investing.com](https://investing.com) or B3 announcements for splits, mergers, and spin-offs

5. **Use historical dates carefully** - Brazilian tax rules changed in 2026 for FII/FIAGRO quotas (5% dividend tax on post-2026 quotas)

6. **Verify your portfolio after imports** - check that quantities and values make sense:

   ```bash
   interest portfolio show
   ```

7. **Export tax reports early** - don't wait until the April IRPF deadline. Generate reports monthly to catch issues early:

   ```bash
   interest tax report 2024 --export
   ```

8. **Use JSON output for automation** - integrate with scripts, dashboards, or spreadsheets:
   ```bash
   interest portfolio show --json | jq '.summary.total_value'
   ```

---

## Getting Help

**Show command help:**

```bash
interest help
```

**In interactive mode:**

```
help
?
```

**Report issues or request features:**

- GitHub Issues: https://github.com/your-username/interest/issues
- Check existing issues before creating new ones
- Include error messages and relevant command output

---

## What's Not Included in This Guide

This README focuses on **using** the Interest tool. For **developers**:

> **For Developers:** Architecture details, design decisions, and contribution guidelines are in [`CLAUDE.md`](CLAUDE.md). That file covers:
>
> - Module structure and data flow
> - Design patterns (decimal precision, idempotency, average cost basis)
> - Database schema and migration strategy
> - Testing strategy and fixtures
> - TUI development workflow
> - Common pitfalls and how to avoid them

---

## License

MIT

---

## Credits

Built by [Gustavo Noronha Silva](https://github.com/kov) with assistance from:
Claude Code (Anthropic)
Codex (OpenAI)
