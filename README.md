# Interest - Brazilian B3 Investment Tracker

A command-line tool for tracking Brazilian stock exchange investments (B3) with automatic price updates, performance analysis, and tax calculations.

## Features

### ✅ Implemented

- **Transaction Import**: Import from B3/CEI Excel/CSV exports
- **Manual Transaction Entry**: Add buy/sell transactions manually with auto-adjustment
- **Portfolio Tracking**: Real-time portfolio with P&L calculations using average cost
- **Price Updates**: Fetch current prices from Yahoo Finance and Brapi.dev
- **Corporate Actions**: Manual entry and automatic adjustment of splits/reverse splits/bonuses
- **Idempotent Application**: Safe to reapply corporate actions without double-adjustment
- **Auto-Adjustment**: New historical transactions automatically adjusted for splits
- **Average Cost Basis**: Accurate cost basis calculations for tax purposes
- **Tax Calculations**: Swing trade tax calculations (15% on net profit)
- **Asset Types**: Stocks, Real Estate Funds (FII), Agribusiness Funds (FIAGRO), Infrastructure Funds (FI-INFRA)

### 🚧 Planned (See TODO)

- Day trade tax calculations (20%)
- IRPF annual report generation
- Dividend/income tracking with amortization
- Historical price fetching
- Headless browser for web scraping corporate actions from investing.com
- Import from movimentacao files
- IRPF PDF import for pre-CEI data

## Installation

### Prerequisites

- Rust 1.70+ (for building from source)
- SQLite 3.x (for database)

### Building

```bash
git clone <repository-url>
cd interest
cargo build --release
```

The binary will be at `target/release/interest`.

## Quick Start

```bash
# 1. Import transactions from CEI export
interest import negociacao-2026-01-01.xlsx

# 2. View your portfolio
interest portfolio show

# 3. Add a corporate action (if needed)
interest actions add A1MD34 reverse-split 10:1 2022-11-22 --notes "From investing.com"
interest actions apply A1MD34

# 4. Add a missing historical transaction (enter original pre-split values)
interest transactions add A1MD34 buy 1000 50 2020-01-15 --notes "Pre-CEI purchase"
# → Auto-adjusted to 100 @ R$500 based on the 10:1 reverse split

# 5. Update prices
interest prices update
```

## Database

**Location:** `~/.interest/data.db` (SQLite database)

### Schema Overview

**Core Tables:**
- `assets` - Ticker definitions (ticker, asset_type, name)
- `transactions` - Buy/sell records with average cost tracking
- `corporate_actions` - Splits, reverse splits, bonuses
- `corporate_action_adjustments` - **Junction table** tracking which transactions were adjusted
- `price_history` - Daily OHLCV price data
- `positions` - Current holdings (cached for performance)
- `tax_events` - Monthly tax calculations
- `income_events` - Dividends, amortization, JCP

### Key Design Decisions

1. **Decimal Precision**
   - All monetary values stored as TEXT (Decimal strings) for exact precision
   - Never use floating point (`f64`) for financial calculations
   - SQLite type affinity handled with `ValueRef` pattern matching

2. **Corporate Action Tracking**
   - Junction table prevents double-adjustment
   - Tracks: action_id, transaction_id, old/new quantity/price
   - Enables idempotent application

3. **Foreign Keys**
   - Enabled for referential integrity
   - Cascading deletes where appropriate

4. **Indexes**
   - On all frequently queried columns (asset_id, trade_date, ticker)
   - Composite indexes for common join patterns

## Commands

### Import

```bash
# Import from Excel/CSV
interest import <file.xlsx>

# Dry run (preview without saving)
interest import <file.xlsx> --dry-run
```

**Supported formats:**
- B3/CEI "Negociação de Ativos" Excel exports
- CSV exports from CEI

**Duplicate detection:** Automatically skips duplicate transactions based on `(asset_id, trade_date, transaction_type, quantity)`.

### Manual Transactions

```bash
interest transactions add <ticker> <buy|sell> <quantity> <price> <date> [OPTIONS]

Options:
  --fees <amount>    Transaction fees/brokerage (default: 0)
  --notes <text>     Optional notes about the transaction

Examples:
  # Simple purchase
  interest transactions add PETR4 buy 100 25.50 2018-06-15

  # With fees and notes
  interest transactions add MXRF11 sell 50 120.00 2023-08-20 --fees 5.00 --notes "Partial exit"

  # Pre-split purchase (will be auto-adjusted if split exists)
  interest transactions add A1MD34 buy 1000 50 2020-01-15
  # → Automatically adjusted to 100 @ R$500 if 10:1 reverse split was applied
```

**Important:** When adding historical transactions:
- **Enter the original quantities and prices** (as you remember them)
- The system automatically applies any corporate actions that occurred after the trade date
- No need to calculate post-split values yourself

### Corporate Actions

```bash
interest actions add <ticker> <type> <ratio> <ex-date> [--notes <text>]

Types:
  split           Stock split (e.g., 1:2 means each share becomes 2)
  reverse-split   Reverse split (e.g., 10:1 means 10 shares become 1)
  bonus           Bonus shares (e.g., 10:11 means 10% bonus)

Examples:
  # Regular split (1 share becomes 2)
  interest actions add PETR4 split 1:2 2022-03-15

  # Reverse split (10 shares become 1)
  interest actions add A1MD34 reverse-split 10:1 2022-11-22 --notes "From investing.com"

  # Bonus shares (10% increase)
  interest actions add ITSA4 bonus 10:11 2023-05-10

# Apply corporate actions (adjusts all relevant transactions)
interest actions apply <ticker>    # Apply to specific ticker
interest actions apply             # Apply all unapplied actions

# List corporate actions
interest actions list              # All actions
interest actions list PETR4        # Filter by ticker
```

**How Corporate Actions Work:**

1. **Add the action** with the ex-date (when it becomes effective)
2. **Apply it** - adjusts all transactions before ex-date:
   - `new_quantity = old_quantity × (ratio_to / ratio_from)`
   - `new_price = old_price × (ratio_from / ratio_to)`
   - **Total cost remains unchanged** (critical for tax calculations)
3. **New transactions are auto-adjusted** - when you add historical transactions later, they're automatically adjusted based on their date
4. **Idempotent** - safe to reapply, won't double-adjust (tracked via junction table)

**Example: 10:1 Reverse Split**

```bash
# You bought 1000 shares at R$50 before the split
# Then a 10:1 reverse split happened on 2022-11-22

# Step 1: Add and apply the corporate action
interest actions add A1MD34 reverse-split 10:1 2022-11-22
interest actions apply A1MD34

# Step 2: Add your historical purchase (original values)
interest transactions add A1MD34 buy 1000 50 2020-01-15
# ℹ Auto-applied 1 corporate action(s) to this transaction
# Database now shows: 100 shares @ R$500 (total cost R$50,000 unchanged)

# Step 3: Safe to reapply (idempotency test)
interest actions apply A1MD34
# ℹ No unapplied corporate actions found (no double-adjustment!)
```

### Portfolio

```bash
# Show complete portfolio
interest portfolio show

# Filter by asset type
interest portfolio show --asset-type FII
interest portfolio show --asset-type STOCK

# Performance over time (TODO)
interest portfolio performance --period 1y
```

**Output includes:**
- Ticker, quantity, average cost, current price
- Current value, unrealized P&L (amount and %)
- Total portfolio value and P&L

**Calculations:**
- Uses average cost basis for sales
- Automatically accounts for corporate actions
- Real-time prices from Yahoo Finance/Brapi.dev

### Prices

```bash
# Update all prices from Yahoo Finance and Brapi.dev
interest prices update

# Fetch historical prices for a ticker (TODO)
interest prices history PETR4 --from 2023-01-01 --to 2023-12-31
```

**Price sources:**
1. **Yahoo Finance** - Primary (ticker.SA suffix)
2. **Brapi.dev** - Fallback for Brazilian stocks
3. **Note:** BDRs may not have corporate action data in Brapi.dev

### Tax

```bash
# Calculate monthly swing trade tax
interest tax calculate 12/2025

# Generate annual IRPF report (TODO)
interest tax report 2025

# Show monthly tax summary for a year (TODO)
interest tax summary 2025
```

## Asset Types

The tool auto-detects asset types from ticker suffixes:

| Type | Ticker Pattern | Example | Tax Treatment |
|------|---------------|---------|---------------|
| **STOCK** | Ends in 3-6 | PETR4, VALE3 | Swing: 15%, R$20k/month exempt |
| **FII** | Ends in 11 | MXRF11, HGLG11 | Pre-2026 quotas: exempt dividends, 20% gains<br>Post-2026: 5% dividends, 17.5% gains |
| **FIAGRO** | Ends in 11 | AGRO11 | Same as FII |
| **FI-INFRA** | Ends in 11 | IFRA11 | Same as FII |
| **BDR** | Ends in 34 | A1MD34 (AMD) | Special handling, limited API data |

**Manual override:** Asset type can be modified directly in the database if auto-detection is incorrect.

## Brazilian Tax Rules (2025-2026)

### Swing Trade Tax

**Rate:** 15% on monthly net profit (may become 17.5% if new law passes)

**Exemption:**
- **Stocks only**: Sales below R$20,000/month
- **FII/FIAGRO/FI-INFRA**: No exemption

**Calculation method:**
1. Sum all sales for the month by asset type
2. Calculate profit/loss for each sale using average cost basis
3. Net profit = total profits - total losses
4. Apply exemption (stocks only)
5. Tax = net profit × 15%

**Loss carryforward:** Losses can offset future gains within the same asset class (stocks, FII, etc.) but not across classes.

**Payment:** DARF code 6015, due by last business day of following month.

### Day Trade Tax (TODO)

**Rate:** 20% flat on net profit, no exemption threshold

**Detection:** Buy and sell of same ticker on same trade_date

### IRPF Annual Report (TODO)

**Required reporting:**
- All transactions (buys/sells) with dates and values
- Monthly tax paid (DARFs submitted)
- Current holdings as of December 31 (Bens e Direitos section)
- Dividend and rental income received
- Amortization received (reduces cost basis)

### Fund Income Tax (2026 New Rules)

**FII/FIAGRO/FI-INFRA:**
- **Quotas issued ≤ 2025:**
  - Dividends: Tax-exempt
  - Capital gains: 20%
- **Quotas issued ≥ 2026:**
  - Dividends: 5% (withheld at source)
  - Capital gains: 17.5%

**Amortization (Capital Return):**
- 20% tax withheld at source
- **Reduces cost basis** of your investment
- Not counted as income, but lowers future capital gains

**Tracking:** The `quota_issuance_date` field (or `settlement_date` for purchases) determines which tax rules apply.

## Design Decisions & Assumptions

### 1. Data Precision

**Assumption:** Financial calculations require exact decimal precision to avoid rounding errors in tax calculations.

**Implementation:**
- All prices, quantities, and monetary values use `rust_decimal::Decimal`
- Never use `f64` for financial calculations
- Database stores decimals as TEXT to preserve precision
- SQLite type affinity handled with `ValueRef` pattern matching

**Why:** Even 0.01 cent errors compound over thousands of transactions and can lead to incorrect tax calculations.

### 2. Average Cost Basis

**Assumption:** Cost basis is calculated using average cost per asset.

**Implementation:**
- Transactions processed in chronological order (by `trade_date`)
- Earliest purchases matched against sales first
- Proportional cost basis calculated for partial sales

**Example:**
```
Portfolio: Buy 100 @ R$10 = R$1,000
           Buy 50 @ R$15 = R$750

Sell 75 shares:
  ├─ Takes all 100 from first lot? No, only 75 needed
  ├─ Cost basis: (75/100) × R$1,000 = R$750
  └─ Remaining: 25 shares @ R$10, 50 shares @ R$15
```

### 3. Corporate Actions - Junction Table Tracking

**Problem:** Previously, reapplying a corporate action would double-adjust transactions (e.g., 1000 → 100 → 10).

**Solution:** Junction table `corporate_action_adjustments` records every adjustment:
- Tracks: `(action_id, transaction_id)` pair
- Stores: old/new quantity, old/new price
- Prevents: Double-adjustment on reapply

**Benefits:**
- **Idempotent:** Can reapply actions safely
- **Auditable:** Know exactly which transactions were adjusted
- **Reversible:** Could implement undo if needed

### 4. Auto-Adjustment of Manual Transactions

**Assumption:** Users naturally remember original (pre-split) quantities when adding historical transactions.

**Implementation:**
When adding a manual transaction:
1. Insert with user-provided quantity/price
2. Find all applied corporate actions with `ex_date > trade_date`
3. Apply them in chronological order
4. Update transaction with final adjusted values
5. Record adjustments in junction table

**Example:**
```bash
# User adds pre-split purchase
interest transactions add A1MD34 buy 1000 50 2020-01-15

# System finds 10:1 reverse split (ex-date 2022-11-22)
# Automatically adjusts: 1000 @ R$50 → 100 @ R$500
# User sees: "ℹ Auto-applied 1 corporate action(s)"
```

**Why:** Users shouldn't need to mentally calculate post-split values or remember which splits have been applied.

### 5. CEI Data Limitations

**Assumption:** CEI only provides 5 years of transaction history (nothing before ~2019).

**Workarounds:**
1. **Manual transaction entry** for pre-2019 data
2. **Potential IRPF PDF import** (TODO) - annual tax declarations have full history
3. **Other sources** (TODO) - broker statements, personal records

**Impact:** Users with long positions need to manually add old transactions.

### 6. Transaction Source Tracking

**Implementation:** `source` field on every transaction:
- `CEI` - Imported from CEI export
- `MANUAL` - Manually added by user
- `B3_PORTAL` - Future: Direct B3 integration
- `IRPF` - Future: Extracted from tax declaration PDFs

**Why:**
- Debug data quality issues
- Understand data provenance
- Prioritize which sources to trust on duplicates

### 7. Corporate Actions Data Sources

**Challenges:**
- **Brapi.dev:** Has data for Brazilian stocks, **but NOT for BDRs** (e.g., A1MD34 = AMD)
- **Investing.com:** Has comprehensive data but **no API** (Cloudflare protection blocks scraping)
- **Yahoo Finance:** Provides adjusted prices but not raw corporate action events

**Current solution:** Manual entry (`interest actions add`)

**Future solutions (TODO):**
- Headless browser (Puppeteer/Playwright) to scrape investing.com
- B3 official data API (if they provide one)
- CSV import for bulk entry from investing.com exports

### 8. Duplicate Detection

**Assumption:** Same asset, date, type, and quantity = duplicate transaction.

**Implementation:** Before inserting, check:
```sql
SELECT COUNT(*) FROM transactions
WHERE asset_id = ? AND trade_date = ?
  AND transaction_type = ? AND quantity = ?
```

**Edge case:** Buying the same quantity of the same stock twice on the same day is rare enough to treat as duplicate.

**Future improvement:** Could add `--force` flag to override duplicate check.

### 9. Settlement Date and Quota Vintage

**Brazilian stock settlement:** T+2 (trade date + 2 business days)

**Why it matters for funds:**
- Quotas issued in 2025 or earlier: Old tax rules (exempt dividends)
- Quotas issued in 2026 or later: New tax rules (5% dividend tax)
- Determined by `settlement_date` or `quota_issuance_date`

**Implementation:**
- CEI imports include `settlement_date` from export
- Manual entries default to same as `trade_date`
- TODO: Track quota vintage for accurate tax calculations

### 10. Day Trading Detection

**Assumption:** Buy and sell of same ticker on same `trade_date` = day trade.

**Implementation:** `is_day_trade` boolean flag on transactions.

**Future:** Separate tax calculation (20% flat rate, no exemption).

**Edge case:** Multiple buys/sells same day with net position = swing trade? (Research needed)

### 11. Database Schema Versioning

**Current approach:**
- Schema version tracked in `metadata` table (`schema_version = 1`)
- Schema SQL in `src/db/schema.sql`
- Executed on first run or when database doesn't exist

**Future migrations:**
1. Increment `schema_version` in new schema.sql
2. Add migration SQL in `src/db/migrations/v2.sql`
3. Check version on startup, run migrations if needed
4. Update metadata table

**Current schema version:** 1

### 12. Price Data Caching

**Implementation:** Prices stored in `price_history` table with `price_date`.

**Cache strategy:**
- Check `price_history` for today's date first
- Only fetch from API if not cached or stale
- TODO: Add configurable TTL (time-to-live)

**Why:** Avoid excessive API calls, respect rate limits, faster portfolio calculations.

## Troubleshooting

### "Insufficient purchase history" Error

```
Error: A1MD34: Insufficient purchase history: Selling 100 units but only 6.1 available.
```

**Causes:**
1. Missing pre-2019 transactions (CEI limitation)
2. Shares from term contracts/transfers not in export
3. Corporate action not applied (quantities still pre-split)
4. Short selling (not supported)

**Solutions:**

```bash
# Solution 1: Add missing historical purchases (enter original quantities)
interest transactions add A1MD34 buy 1000 50 2018-01-15 --notes "Pre-CEI purchase"

# Solution 2: Apply corporate action if you haven't yet
interest actions add A1MD34 reverse-split 10:1 2022-11-22
interest actions apply A1MD34

# Solution 3: Check if action was already applied
sqlite3 ~/.interest/data.db "SELECT * FROM corporate_actions WHERE asset_id = (SELECT id FROM assets WHERE ticker = 'A1MD34')"
```

### Double-Adjusted Transactions

**Symptoms:** Prices/quantities way off (e.g., R$5000 instead of R$50).

**Cause:** Corporate action applied multiple times in older versions without junction table tracking.

**Solution:** Reset and reimport with current version:

```bash
# Backup current database
cp ~/.interest/data.db ~/.interest/data.db.backup

# Delete and reimport
rm ~/.interest/data.db
interest import negociacao-2026-01-01.xlsx

# Re-add corporate actions
interest actions add A1MD34 reverse-split 10:1 2022-11-22
interest actions apply A1MD34

# Re-add manual transactions (they'll auto-adjust correctly)
interest transactions add A1MD34 buy 1000 50 2020-01-15
```

### Price Fetch Failures

**Yahoo Finance 404:**
```
Error: Failed to fetch price for PETR4: 404 Not Found
```

**Solutions:**
- Verify ticker format (should be TICKER.SA, e.g., PETR4.SA) - automatic in code
- Some tickers may not be listed on Yahoo Finance
- Try Brapi.dev as fallback (automatic)

**Brapi.dev errors:**
- No authentication required as of 2026-01
- BDRs may not have complete data (especially corporate actions)

### SQLite Type Affinity Issues

**Problem:** Decimal values read as wrong type from database.

**Cause:** SQLite stores numbers differently based on value:
- Whole numbers (100) → INTEGER
- Decimals (100.50) → TEXT or REAL

**Solution:** All readers use `ValueRef` pattern matching to handle all three types:

```rust
match row.get_ref(idx)? {
    ValueRef::Text(bytes) => Decimal::from_str(s)?,
    ValueRef::Integer(i) => Decimal::from(i),
    ValueRef::Real(f) => Decimal::try_from(f)?,
    _ => Err(...)
}
```

**Prevention:** Store all decimals as TEXT strings.

## Contributing

See `TODO` file for planned features.

When contributing:
1. **Never use `f64`** for financial calculations - always use `Decimal`
2. **Write tests** for tax calculations and average cost logic
3. **Document assumptions** in code comments
4. **Update this README** for new features or design decisions

## Architecture

**Language:** Rust (2021 edition)

**Key Dependencies:**
- `rusqlite` - SQLite database (considered Limbo but stuck with SQLite)
- `clap` - CLI framework with derive macros
- `rust_decimal` - Precise financial arithmetic
- `reqwest` - HTTP client for price APIs
- `calamine` - Excel file parsing
- `csv` - CSV parsing
- `chrono` - Date/time handling
- `tabled` - CLI table formatting
- `colored` - Terminal colors

## License

MIT

## Credits

Built with Claude Code (Anthropic).
