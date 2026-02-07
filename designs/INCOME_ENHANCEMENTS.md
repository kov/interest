# Income Subcommand Enhancement Design Proposal

**Status**: Draft  
**Date**: 2026-02-07  
**Author**: Design Analysis for Issue #XX

## Executive Summary

This document proposes enhancements to the `income` subcommand to provide more actionable insights including yield metrics, trend analysis, baseline vs exceptional income detection, and forecasting capabilities. The goal is to make the income tracking feature significantly more valuable for portfolio management and decision-making.

## Current State

### Existing Subcommands

1. **`income show [--year YYYY]`**
   - Groups income by asset type and ticker
   - Shows breakdown by income type (Dividends, JCP, Amortization)
   - Sorted by total amount within each asset type
   - Defaults to current year

2. **`income detail [--year YYYY] [--asset TICKER]`**
   - Lists individual income events chronologically
   - Shows date, ticker, asset type, event type, gross amount, tax, net amount, notes
   - Filterable by year and specific asset
   - Includes summary totals

3. **`income summary [--year YYYY]`**
   - With year: Monthly breakdown of income by type
   - Without year: Yearly breakdown across all years
   - Includes statistics: periods with income, average per period
   - Shows subtotals by asset type

4. **`income add`**
   - Manually add income events
   - Fields: ticker, event_type, total_amount, date, ex_date, withholding, amount_per_quota, notes

### Data Model

**Database Schema**: `income_events` table
- Core fields: `asset_id`, `event_date`, `ex_date`, `event_type`, `amount_per_quota`, `total_amount`, `withholding_tax`
- Tax tracking: `is_quota_pre_2026` (for 2026 Brazilian tax law changes)
- Metadata: `source` (YAHOO, CEI, MANUAL), `notes`, `created_at`
- Indexed on: asset_id, event_date, event_type

**Event Types**:
- `DIVIDEND` (dividends, dividendos, rendimento)
- `JCP` (Juros sobre Capital Próprio - Interest on Equity)
- `AMORTIZATION` (amortização)

### Current Capabilities

✅ **What works well:**
- Comprehensive data capture (gross, tax, net, per-quota amounts)
- Multi-year tracking with flexible filtering
- Asset type grouping (stocks, FII, FIAGRO, bonds, etc.)
- Source tracking (imported vs manual entries)
- Time-series aggregation (monthly/yearly)
- Basic statistics (periods with income, averages)

⚠️ **Limitations:**
- No yield calculations (yield % relative to position value/cost)
- No trend analysis or pattern detection
- No distinction between "baseline" recurring income vs exceptional events
- No forecasting or forward-looking projections
- No LTM (Last Twelve Months) metrics
- Limited correlation with portfolio positions (no income/position relationship)
- No comparison across assets or benchmarks
- No visualization or graphical trend indicators

## Problem Statement

Investors need to answer questions like:
1. **"What is my portfolio's current yield?"** (LTM income / current portfolio value)
2. **"Which assets are my best dividend payers?"** (yield %, consistency, growth)
3. **"Is my income growing, stable, or declining?"** (trend analysis)
4. **"What can I expect to earn this year?"** (forecasting based on history)
5. **"Was this month's income normal or exceptional?"** (baseline detection)
6. **"How does my FII income compare to stocks?"** (comparative analysis)
7. **"Which holdings should I prioritize for income?"** (yield-weighted insights)

The current implementation provides raw data aggregation but lacks the analytical layer needed to answer these strategic questions.

## Proposed Enhancements

### 1. Yield Calculations (High Priority)

#### 1.1 Last Twelve Months (LTM) Yield

**New Command**: `income yield [--ticker TICKER] [--asset-type TYPE]`

**Calculation Methodology**:
```
LTM Yield = (Sum of income in last 12 months) / (Current position value) * 100
```

**Output Format**:
```
Income Yield Analysis (Last 12 Months)

Portfolio Overview:
  Total LTM Income:     R$ 12,500.00
  Current Portfolio:    R$ 250,000.00
  Portfolio Yield:      5.00%

By Asset Type:
┌────────────┬───────────────┬──────────────┬────────┐
│ Type       │ LTM Income    │ Position Val │ Yield  │
├────────────┼───────────────┼──────────────┼────────┤
│ FII        │ R$ 8,400.00   │ R$ 140,000   │ 6.00%  │
│ Stock      │ R$ 3,200.00   │ R$ 90,000    │ 3.56%  │
│ FIAGRO     │ R$ 900.00     │ R$ 20,000    │ 4.50%  │
└────────────┴───────────────┴──────────────┴────────┘

Top Yielding Assets:
┌──────────┬───────────────┬──────────────┬────────┬─────────────┐
│ Ticker   │ LTM Income    │ Avg Position │ Yield  │ Consistency │
├──────────┼───────────────┼──────────────┼────────┼─────────────┤
│ XPLG11   │ R$ 2,100.00   │ R$ 30,000    │ 7.00%  │ ████████░   │
│ MXRF11   │ R$ 1,800.00   │ R$ 28,000    │ 6.43%  │ ██████████  │
│ PETR4    │ R$ 1,200.00   │ R$ 25,000    │ 4.80%  │ ████░░░░░   │
└──────────┴───────────────┴──────────────┴────────┴─────────────┘
```

**Data Requirements**:
- Income events from last 12 months (already available)
- Current portfolio positions with values (from `portfolio show`)
- Historical position values for average calculation
- Integration with `reports/portfolio.rs`

**Implementation Notes**:
- Use `get_income_events_with_assets()` with date filter (today - 365 days)
- Integrate with portfolio snapshot system for position values
- Handle assets with zero positions (sold) - show warning or filter out
- "Consistency" metric: `(months with income / 12) * 10` bars

#### 1.2 Trailing Yield (TTM, 6M, 3M)

**Extension**: Add time period flags
```
income yield --period LTM    # Last 12 months (default)
income yield --period 6M     # Last 6 months
income yield --period 3M     # Last 3 months
income yield --period YTD    # Year to date
```

**Calculation**:
- Annualize shorter periods: `(income_period / days_in_period) * 365 / position_value * 100`

#### 1.3 Yield on Cost

**Calculation**:
```
Yield on Cost = (LTM income) / (Average cost basis) * 100
```

**Value**: Shows true return on original investment, useful for long-held positions where market value has appreciated significantly.

**Add to output**:
```
│ XPLG11   │ 7.00% (mkt) │ 9.50% (cost) │ +35% gain │
```

### 2. Trend Analysis (High Priority)

#### 2.1 Income Trend Report

**New Command**: `income trends [--ticker TICKER] [--period PERIOD]`

**Default Period**: Last 3 years, displayed monthly

**Output Format**:
```
Income Trends (Last 36 Months)

Monthly Income (R$):
  ┌─────────────────────────────────────────────────┐
  │                                            ▲    │
2500│                                      ▲   ███   │
  │                          ▲       ▲   ███ ████   │
2000│                    ▲   ███     ███ ████ ████ ▲ │
  │              ▲     ███  ████ ▲  ████ ████ ████ █ │
1500│        ▲   ███ ▲  ████ ████ ██ ████ ████ ████ █ │
  │    ▲  ███  ████ ██ ████ ████ ██ ████ ████ ████ █ │
1000│ ▲ ███ ████ ████ ██ ████ ████ ██ ████ ████ ████ █ │
  │ █ ███ ████ ████ ██ ████ ████ ██ ████ ████ ████ █ │
 500│ █ ███ ████ ████ ██ ████ ████ ██ ████ ████ ████ █ │
  └─────────────────────────────────────────────────┘
    2024-01    2024-07    2025-01    2025-07    2026-01

Statistics:
  3-Year Trend:          +45% (Growing ▲)
  YoY Growth:            +12%
  MoM Volatility:        18% (Moderate)
  Seasonal Pattern:      Q1 Strong, Q3 Weak

Growth Decomposition:
  ┌────────────────┬────────────┬────────────┬──────────┐
  │ Component      │ 2024       │ 2025       │ Growth   │
  ├────────────────┼────────────┼────────────┼──────────┤
  │ Position Growth│ +R$ 800    │ -          │ +35%     │
  │ Yield Expansion│ +R$ 400    │ -          │ +15%     │
  │ New Holdings   │ +R$ 200    │ -          │ +8%      │
  │ TOTAL          │ +R$ 1,400  │ -          │ +58%     │
  └────────────────┴────────────┴────────────┴──────────┘
```

**Calculations**:
- **Linear regression** on monthly totals: slope → trend direction
- **Year-over-year (YoY)**: `(this_year_total / last_year_total - 1) * 100`
- **Month-over-month volatility**: `stddev(monthly_pct_changes)`
- **Seasonal pattern**: Compare Q1, Q2, Q3, Q4 averages

**Growth Decomposition** (advanced):
- Requires correlation with transaction history
- **Position Growth**: Income from increased holdings (more shares/quotas)
- **Yield Expansion**: Higher payouts per share (dividend increases)
- **New Holdings**: Income from newly acquired assets

#### 2.2 Per-Asset Trend

**Extension**: `income trends --ticker XPLG11`

Show micro-trends for individual assets:
- Payment frequency (monthly, quarterly, etc.)
- Amount per payment trend
- Consistency score (regularity of payments)
- Distribution policy changes detection

### 3. Baseline vs Exceptional Income (Medium Priority)

#### 3.1 Income Categorization

**Concept**: Distinguish between recurring/predictable income and one-off events.

**Heuristics**:

**Baseline (Recurring)**:
- Regular dividends from FII (typically monthly)
- Consistent stock dividends (quarterly/semi-annual patterns)
- JCP that occurs regularly (e.g., annually from same company)

**Exceptional (Non-recurring)**:
- Amortization events (quota capital returns, usually one-time)
- "Special dividends" (significantly larger than historical average)
- First-time dividends from new holdings
- Final distributions from closed funds

**Detection Algorithm**:
```python
For each asset:
  1. Calculate historical payment frequency (monthly, quarterly, etc.)
  2. Calculate mean and stddev of payment amounts
  3. Flag as exceptional if:
     - event_type == AMORTIZATION (nearly always exceptional)
     - amount > mean + 2*stddev (unusually large)
     - First payment ever from this asset
     - Payment after >6 months gap (broken pattern)
```

**New Command**: `income summary --categorize`

**Output Addition**:
```
Income Summary 2025

Baseline Income:       R$ 10,200.00 (82%)
Exceptional Income:    R$ 2,300.00 (18%)
  ├─ Amortization:     R$ 1,500.00
  ├─ Special Divs:     R$ 600.00
  └─ New Holdings:     R$ 200.00

Monthly Baseline Avg:  R$ 850.00
Expected FY 2026:      R$ 10,200.00 (based on baseline)
```

**Use Cases**:
- More accurate forecasting (exclude one-offs)
- Better understanding of sustainable income
- Portfolio planning (distinguish growth from recurring income)

### 4. Forecasting (Medium Priority)

#### 4.1 Income Forecast Report

**New Command**: `income forecast [--year YYYY] [--conservative]`

**Methodology**:

**Approach 1: Historical Average (Baseline Model)**
```
For each asset with position:
  monthly_avg = mean(last_12_months_income)
  annual_forecast = monthly_avg * 12
```

**Approach 2: Trend-Adjusted**
```
For each asset:
  baseline = mean(last_12_months_income)
  trend_factor = linear_regression_slope(last_24_months)
  annual_forecast = (baseline * 12) * (1 + trend_factor)
```

**Approach 3: Seasonal Decomposition**
```
For each asset:
  Extract: trend, seasonal_pattern, residual
  Forecast = (trend[next_period] * seasonal_pattern[month]) + residual_avg
```

**Conservative Mode**:
- Use `mean - 0.5*stddev` instead of mean (downside protection)
- Only include assets with 12+ months history
- Exclude exceptional events from baseline

**Output Format**:
```
Income Forecast 2026

Methodology: Trend-Adjusted Historical (12-month baseline)

Expected Annual Income: R$ 13,200.00
  ├─ Baseline Growth:   R$ 12,500.00 (from 2025)
  ├─ Trend Adjustment:  +R$ 700.00 (+5.6%)
  └─ Confidence:        Medium (67% interval: R$ 11,800 - R$ 14,600)

By Quarter:
  ┌─────────┬───────────────┬─────────────┬─────────┐
  │ Quarter │ Forecast      │ vs 2025 Q   │ Drivers │
  ├─────────┼───────────────┼─────────────┼─────────┤
  │ Q1 2026 │ R$ 3,600      │ +8%         │ Strong  │
  │ Q2 2026 │ R$ 3,100      │ +4%         │ Normal  │
  │ Q3 2026 │ R$ 2,900      │ +2%         │ Weak    │
  │ Q4 2026 │ R$ 3,600      │ +7%         │ Strong  │
  └─────────┴───────────────┴─────────────┴─────────┘

Top Contributors:
  ┌──────────┬───────────────┬─────────┬───────────────────┐
  │ Ticker   │ 2026 Forecast │ Conf.   │ Basis             │
  ├──────────┼───────────────┼─────────┼───────────────────┤
  │ XPLG11   │ R$ 2,400      │ High    │ 12mo avg + trend  │
  │ MXRF11   │ R$ 2,100      │ High    │ 12mo avg          │
  │ PETR4    │ R$ 1,800      │ Medium  │ Volatile history  │
  │ VALE3    │ R$ 1,200      │ Low     │ Only 6mo history  │
  └──────────┴───────────────┴─────────┴───────────────────┘

Assumptions:
  ✓ Current holdings maintained
  ✓ No position size changes
  ✓ Historical payment patterns continue
  ⚠ Commodity price volatility (PETR4, VALE3)
  ⚠ New holdings have limited history
```

**Confidence Levels**:
- **High**: 12+ months history, low volatility (CV < 0.2), consistent frequency
- **Medium**: 6-12 months history, or moderate volatility (CV 0.2-0.4)
- **Low**: <6 months history, or high volatility (CV > 0.4)

#### 4.2 Monte Carlo Simulation (Advanced/Optional)

For portfolios with many assets, run simulations:
```
income forecast --monte-carlo --simulations 10000
```

- Generate probability distribution of outcomes
- Show 10th, 50th, 90th percentiles
- Account for correlation between assets (e.g., oil stocks move together)

### 5. Additional Useful Features

#### 5.1 Benchmarking

**Command**: `income benchmark [--index IBOV|IDIV]`

Compare portfolio income yield against market indices:
```
Portfolio vs Benchmarks (LTM Yield)

Your Portfolio:        5.00%
IDIV (Dividend Index): 5.80%  [+0.80pp, Underperforming]
IBOV (Bovespa Index):  3.20%  [-1.80pp, Outperforming]
FII Average:           6.50%  [+1.50pp vs your FII segment]
```

**Data Source**: Would require external data integration (maybe hardcoded averages)

#### 5.2 Dividend Calendar

**Command**: `income calendar [--month MM]`

Show expected payment dates based on historical patterns:
```
Upcoming Income Events (March 2026)

┌────────────┬──────────┬──────────────┬──────────────┬────────────┐
│ Date (Est) │ Ticker   │ Type         │ Amount (Est) │ Confidence │
├────────────┼──────────┼──────────────┼──────────────┼────────────┤
│ 2026-03-05 │ XPLG11   │ Dividend     │ R$ 180       │ High       │
│ 2026-03-10 │ MXRF11   │ Dividend     │ R$ 150       │ High       │
│ 2026-03-15 │ HGLG11   │ Dividend     │ R$ 120       │ Medium     │
│ 2026-03-28 │ PETR4    │ JCP          │ R$ 300       │ Low        │
└────────────┴──────────┴──────────────┴──────────────┴────────────┘

Total Expected: R$ 750
```

**Logic**:
- Calculate typical payment day-of-month from history
- Estimate amount from recent average
- Confidence based on consistency

#### 5.3 Tax-Aware Reporting

**Command**: `income summary --tax-aware`

Show pre-tax vs post-tax income:
```
Income Summary 2025 (Tax-Aware)

Gross Income:          R$ 12,500.00
Withholding Tax:       R$ 1,250.00  (10%)
Net Income:            R$ 11,250.00

By Tax Treatment:
  ┌────────────────┬───────────┬────────┬─────────┐
  │ Source         │ Gross     │ Tax    │ Net     │
  ├────────────────┼───────────┼────────┼─────────┤
  │ FII (Pre-2026) │ R$ 8,400  │ R$ 0   │ R$ 8,400│ (Exempt)
  │ Stocks         │ R$ 3,200  │ R$ 480 │ R$ 2,720│ (15% WHT)
  │ FII (Post-2026)│ R$ 900    │ R$ 45  │ R$ 855  │ (5% WHT)
  └────────────────┴───────────┴────────┴─────────┘
```

Leverages existing `withholding_tax` and `is_quota_pre_2026` fields.

#### 5.4 Alerts and Anomalies

**Command**: `income alerts`

Detect noteworthy patterns:
```
Income Alerts

⚠ XPLG11: Missed expected payment in Feb 2026
  Last payment: 2026-01-05
  Typical frequency: Monthly
  Action: Check fund announcements

✓ PETR4: Dividend increased 25% vs last year
  Previous: R$ 0.80/share → Current: R$ 1.00/share
  
⚠ Overall income down 15% MoM
  Jan 2026: R$ 1,100 → Feb 2026: R$ 935
  Main driver: Missing XPLG11 payment
```

**Detection Rules**:
- Missed payment: Expected date + 7 days, no payment recorded
- Amount change: >20% vs previous payment
- Overall income change: >10% MoM or YoY

#### 5.5 Export to Spreadsheet

**Command**: `income export 2025 --format xlsx`

Export detailed income data for external analysis:
- All events with computed fields (yield, running totals, etc.)
- Monthly/yearly summary sheets
- Pivot table-ready format

### 6. Visualization Enhancements (Low Priority / Future)

For TUI mode or terminal output:

**ASCII Charts** (like the trend output above):
- Monthly bar charts
- Sparklines for quick trends
- Heat maps for seasonal patterns

**Color Coding**:
- 🟢 Green: Growth trends, high yields
- 🟡 Yellow: Moderate/stable
- 🔴 Red: Declining trends, low yields

## Implementation Plan

### Phase 1: Foundation (Weeks 1-2)
**Goal**: Add yield calculations and basic trends

Tasks:
1. **Create `reports/income_analytics.rs`** module
   - `calculate_ltm_yield()`: Portfolio-wide and per-asset
   - `calculate_yield_on_cost()`: Using cost basis from portfolio
   - `analyze_trends()`: Linear regression, YoY, volatility
   
2. **Add new dispatcher handlers**
   - `dispatch_income_yield()`
   - `dispatch_income_trends()`
   
3. **Integrate with portfolio system**
   - Cross-reference income with position snapshots
   - Fetch current/average position values
   
4. **Add formatters**
   - `formatters/income.rs`: `format_yield_report()`, `format_trends_report()`
   
5. **CLI additions**
   - Add `Yield` and `Trends` to `IncomeCommands` enum
   
**Deliverables**:
- `income yield` command working
- `income trends` command with basic stats

### Phase 2: Intelligence (Weeks 3-4)
**Goal**: Add baseline detection and forecasting

Tasks:
1. **Baseline vs Exceptional Logic**
   - Implement detection heuristics
   - Add `is_exceptional` flag computation
   - Update `income summary` to categorize
   
2. **Forecasting Engine**
   - Historical average baseline
   - Trend adjustment
   - Confidence scoring
   - Add `dispatch_income_forecast()`
   
3. **Enhanced Statistics**
   - Seasonal pattern detection
   - Consistency scoring
   - Payment frequency analysis
   
**Deliverables**:
- `income summary --categorize` working
- `income forecast` command working

### Phase 3: Polish & Extras (Weeks 5-6)
**Goal**: Add remaining features and refinements

Tasks:
1. **Calendar Feature**
   - Payment date prediction
   - `income calendar` command
   
2. **Alerts System**
   - Anomaly detection
   - `income alerts` command
   
3. **Tax-Aware Reporting**
   - Add `--tax-aware` flag to summary
   - Net income calculations
   
4. **Export Functionality**
   - XLSX export with multiple sheets
   - CSV export option
   
5. **Documentation**
   - Update README with new commands
   - Add examples and use cases
   - Update CLAUDE.md with design patterns

**Deliverables**:
- All proposed commands working
- Comprehensive documentation
- Tests for all new features

### Phase 4: Advanced Features (Future)
**Goal**: Optional enhancements for power users

Tasks:
1. **Monte Carlo Simulation**
2. **Benchmarking** (requires external data)
3. **Interactive TUI widgets** (charts, drill-downs)
4. **API integration** for real-time dividend announcements

## Technical Considerations

### Database Changes

**No schema changes required!** All features can be built on existing `income_events` table.

**Optional additions** (for future optimization):
```sql
-- Cache computed metrics (optional)
CREATE TABLE income_metrics_cache (
    asset_id INTEGER,
    metric_type TEXT,  -- 'ltm_yield', 'trend', 'baseline_avg'
    value DECIMAL(15,4),
    computed_at DATETIME,
    PRIMARY KEY (asset_id, metric_type)
);
```

### Dependencies

**Required**:
- Integration with `reports/portfolio.rs` for position values
- Access to `position_snapshots` for historical data

**New Libraries** (minimal):
- Linear regression: Use `linregress` crate or implement simple least-squares
- ASCII charts: `tui-rs` or manual formatting (lightweight)

### Performance

**Concerns**:
- Yield calculation requires joining income + portfolio data
- Trend analysis with 36 months of data → should be fast (<100ms)
- Forecasting simulations (Monte Carlo) → may be slow, needs caching

**Optimizations**:
- Cache portfolio snapshots (already exists)
- Pre-compute common metrics (LTM totals) → store in metadata
- Lazy loading: only compute detailed analytics on request

### Testing

**Unit Tests**:
- Yield calculation accuracy (various position scenarios)
- Trend detection (growth, decline, flat)
- Baseline categorization (regular vs exceptional)
- Forecast algorithms (verify against known data)

**Integration Tests**:
- Full commands with test data
- Edge cases: new holdings, sold positions, missing data
- Multi-year scenarios

**Test Data**:
- Create fixture with realistic income patterns
- Include seasonal variations, special dividends, amortization

## User Experience

### Command Hierarchy

```
income
  ├── show [--year] [--sort-by yield|amount]     # Enhanced with yield
  ├── detail [--year] [--asset]                  # No change
  ├── summary [--year] [--categorize] [--tax-aware]  # Enhanced
  ├── add {...}                                  # No change
  ├── yield [--ticker] [--asset-type] [--period] # NEW
  ├── trends [--ticker] [--period]               # NEW
  ├── forecast [--year] [--conservative]         # NEW
  ├── calendar [--month]                         # NEW
  ├── alerts                                     # NEW
  └── export [year] [--format xlsx|csv]          # NEW
```

### Output Modes

All new commands support:
- **Terminal table** (default): Human-readable with colors
- **JSON** (`--json`): Machine-readable for scripting
- **Quiet** (`--quiet`): Minimal output (return codes only)

### Incremental Rollout

1. Start with `income yield` and `income trends` (core value)
2. Add `forecast` and `summary --categorize` (intelligence)
3. Layer on extras (calendar, alerts, export) as refinements

Users get immediate value from Phase 1, while advanced features come later.

## Alternatives Considered

### Alternative 1: Separate `yield` top-level command
**Rejected**: Yield is fundamentally about income, grouping under `income` is more intuitive

### Alternative 2: AI/ML-based forecasting
**Deferred**: Too complex for MVP. Simple statistical methods (trends, averages) are:
- Easier to implement and test
- More transparent to users
- Sufficient for most use cases
- Can add ML later as enhancement

### Alternative 3: Web dashboard for visualization
**Out of Scope**: Would require:
- Web server infrastructure
- Frontend development
- Significant complexity
Better to focus on CLI/TUI with optional export to Excel for visualization

## Success Metrics

How do we know these enhancements are successful?

1. **Adoption**: % of users who use `income yield` and `income trends` commands
2. **Value**: User feedback that these features help with portfolio decisions
3. **Accuracy**: Forecast accuracy within ±10% for assets with 12+ months history
4. **Performance**: All commands execute in <1 second for typical portfolios (<100 assets)

## Open Questions

1. **Dividend announcements**: Should we integrate with external APIs (e.g., B3, Yahoo) for upcoming dividend dates? Or rely purely on historical patterns?
   - **Recommendation**: Start with historical patterns (Phase 3), add API integration in Phase 4 if there's demand

2. **Reinvestment tracking**: Should forecasts account for dividend reinvestment plans?
   - **Recommendation**: No for MVP. Assume dividends are distributed, not reinvested. Can add flag later.

3. **Currency handling**: For BDRs paying USD dividends, should we track in original currency?
   - **Recommendation**: Convert to BRL at event date (current behavior). Advanced users can check notes field for original amount.

4. **Historical adjustments**: How to handle retroactive corrections (e.g., fund revises distribution amount)?
   - **Recommendation**: Edit event and regenerate reports. No versioning needed for MVP.

## Conclusion

This proposal significantly enhances the income tracking capabilities of the tool, transforming it from a passive data logger to an active portfolio management aid. The phased approach ensures quick wins (yield calculations, trends) while leaving room for sophisticated features (forecasting, alerts) in later iterations.

**Key Principles**:
- **Build on existing data**: No schema changes needed
- **Incremental value**: Each phase delivers standalone benefits
- **User-centric**: Answer real investor questions
- **Maintainable**: Simple algorithms, clear code structure

**Next Steps**:
1. Review this proposal with stakeholders
2. Refine scope and priorities
3. Begin Phase 1 implementation
4. Iterate based on user feedback

---

**Appendix A: Example Use Cases**

**Retiree seeking income stability**:
```bash
$ interest income yield
# Check overall portfolio yield is meeting 6% target

$ interest income trends
# Verify income is stable or growing

$ interest income forecast --conservative
# Plan for next year's expenses

$ interest income summary --categorize
# Understand baseline vs one-time events
```

**Growth investor adding dividend stocks**:
```bash
$ interest income show --sort-by yield
# Identify best dividend payers

$ interest income yield --ticker PETR4
# Evaluate new position's contribution

$ interest income trends --ticker PETR4
# Check dividend growth track record
```

**Tax planning**:
```bash
$ interest income summary --tax-aware --year 2025
# Prepare for tax filing, verify withholding

$ interest income detail --year 2025 --json > income_2025.json
# Export for accountant
```

**Appendix B: Data Flow Diagram**

```
┌─────────────────┐
│ income_events   │
│ (database)      │
└────────┬────────┘
         │
         ├─────────────────────────┐
         │                         │
         ▼                         ▼
┌─────────────────┐       ┌───────────────────┐
│ Aggregation     │       │ Portfolio         │
│ (sum, group)    │       │ (positions, cost) │
└────────┬────────┘       └────────┬──────────┘
         │                         │
         │    ┌────────────────────┘
         │    │
         ▼    ▼
┌──────────────────────────┐
│ Income Analytics Engine  │
│ - Yield calculations     │
│ - Trend analysis         │
│ - Baseline detection     │
│ - Forecasting            │
└────────────┬─────────────┘
             │
             ▼
┌──────────────────────────┐
│ Formatters & Output      │
│ - Tables                 │
│ - JSON                   │
│ - Charts (ASCII)         │
└──────────────────────────┘
```
