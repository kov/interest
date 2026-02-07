# Income Enhancements - Visual Overview

## Current vs Proposed Command Structure

### Current Commands (4 subcommands)
```
income
├── show [--year]          → List income by asset (grouped by type)
├── detail [--year] [-a]   → Individual income events
├── summary [--year]       → Monthly/yearly breakdown
└── add {...}              → Manually add income event
```

### Proposed Commands (9 subcommands)
```
income
├── show [--year] [--sort-by yield|amount]     → ✨ Enhanced with yield sorting
├── detail [--year] [-a]                       → (Unchanged)
├── summary [--year] [--categorize] [--tax-aware] → ✨ Enhanced with categories
├── add {...}                                  → (Unchanged)
├── yield [--ticker] [--asset-type] [--period] → 🆕 LTM yield calculations
├── trends [--ticker] [--period]               → 🆕 Growth/decline analysis
├── forecast [--year] [--conservative]         → 🆕 Income projections
├── calendar [--month]                         → 🆕 Upcoming payments
└── alerts                                     → 🆕 Anomaly detection
```

Legend: ✨ = Enhanced, 🆕 = New command

---

## Data Flow Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                      Income Events                          │
│                      (Database Table)                       │
│  ┌───────────────────────────────────────────────────┐     │
│  │ Fields: asset_id, event_date, event_type,         │     │
│  │         amount, withholding_tax, notes, etc.      │     │
│  └───────────────────────────────────────────────────┘     │
└───────────────────────┬─────────────────────────────────────┘
                        │
        ┌───────────────┼───────────────┐
        │               │               │
        ▼               ▼               ▼
┌─────────────┐ ┌─────────────┐ ┌─────────────┐
│ Aggregation │ │   Current   │ │  Historical │
│   (Group,   │ │  Portfolio  │ │  Snapshots  │
│    Sum)     │ │  Positions  │ │  (Backfill) │
└──────┬──────┘ └──────┬──────┘ └──────┬──────┘
       │               │               │
       └───────────────┼───────────────┘
                       │
                       ▼
       ┌───────────────────────────────┐
       │   Income Analytics Engine     │
       │  (NEW: income_analytics.rs)   │
       ├───────────────────────────────┤
       │ • calculate_ltm_yield()       │
       │ • analyze_trends()            │
       │ • detect_baseline()           │
       │ • forecast_income()           │
       │ • detect_anomalies()          │
       └───────────────┬───────────────┘
                       │
                       ▼
       ┌───────────────────────────────┐
       │   Formatters & Output         │
       ├───────────────────────────────┤
       │ • Tables (ASCII)              │
       │ • JSON (machine-readable)     │
       │ • Charts (visual)             │
       │ • Export (Excel/CSV)          │
       └───────────────────────────────┘
```

---

## Feature Relationship Map

```
                    ┌─────────────────────┐
                    │   Income Events     │
                    │   (Raw Data)        │
                    └──────────┬──────────┘
                               │
                ┌──────────────┼──────────────┐
                │              │              │
        ┌───────▼─────┐  ┌────▼────┐  ┌─────▼─────┐
        │   Current   │  │  Trends │  │ Forecast  │
        │    Yield    │  │         │  │           │
        └───────┬─────┘  └────┬────┘  └─────┬─────┘
                │             │              │
                │      ┌──────▼──────┐       │
                │      │  Baseline   │       │
                │      │  Detection  │       │
                │      └──────┬──────┘       │
                │             │              │
                └─────────────┼──────────────┘
                              │
                    ┌─────────▼─────────┐
                    │   Calendar &      │
                    │     Alerts        │
                    └───────────────────┘
```

**Relationships**:
- **Yield** uses current portfolio values
- **Trends** feeds into **Forecast** (slope calculation)
- **Baseline Detection** improves **Forecast** accuracy
- **Calendar** predicts based on **Trends** patterns
- **Alerts** triggers on deviation from **Baseline**

---

## Phase Implementation Timeline

```
Week 1-2: PHASE 1 - Core Value
  ┌─────────────────────────────────────────┐
  │ ✓ Create income_analytics.rs module     │
  │ ✓ Implement yield calculations          │
  │ ✓ Integrate with portfolio system       │
  │ ✓ Implement trend analysis              │
  │ ✓ Add CLI commands + formatters         │
  └─────────────────────────────────────────┘
  Deliverable: `income yield` and `income trends`

Week 3-4: PHASE 2 - Intelligence
  ┌─────────────────────────────────────────┐
  │ ✓ Baseline detection heuristics         │
  │ ✓ Exceptional event flagging            │
  │ ✓ Forecasting engine (3 methods)        │
  │ ✓ Confidence scoring                    │
  │ ✓ Enhanced summary with categorization  │
  └─────────────────────────────────────────┘
  Deliverable: `income forecast` and `--categorize`

Week 5-6: PHASE 3 - Polish
  ┌─────────────────────────────────────────┐
  │ ✓ Calendar prediction logic             │
  │ ✓ Anomaly detection (alerts)            │
  │ ✓ Tax-aware reporting                   │
  │ ✓ Export to Excel/CSV                   │
  │ ✓ Documentation updates                 │
  └─────────────────────────────────────────┘
  Deliverable: Full suite complete

Phase 4: ADVANCED (Future)
  ┌─────────────────────────────────────────┐
  │ ○ Monte Carlo simulations               │
  │ ○ Benchmarking vs indices               │
  │ ○ External API integration              │
  │ ○ Interactive TUI widgets               │
  └─────────────────────────────────────────┘
  Deliverable: Power-user features
```

---

## Key Calculations Explained

### 1. LTM Yield
```
                      Sum of income (last 12 months)
LTM Yield (%) = ─────────────────────────────────────── × 100
                     Current portfolio value
```

**Example**:
- Last 12 months income: R$ 12,500
- Current portfolio: R$ 250,000
- **Yield: 5.00%**

### 2. Yield on Cost
```
                      Sum of income (last 12 months)
Yield on Cost (%) = ─────────────────────────────────── × 100
                     Total cost basis (avg cost)
```

**Example**:
- Last 12 months income: R$ 2,100
- Original cost: R$ 22,000
- **Yield on Cost: 9.55%** (vs Market Yield: 7.00%)

### 3. Trend (Linear Regression)
```
For monthly income series [y₁, y₂, ..., yₙ]:
  slope = Σ((xᵢ - x̄)(yᵢ - ȳ)) / Σ((xᵢ - x̄)²)
  
  If slope > 0: Growing ▲
  If slope < 0: Declining ▼
  If slope ≈ 0: Stable ─
```

### 4. Baseline Detection
```
For each income event:
  mean = average(last_12_payments)
  stddev = standard_deviation(last_12_payments)
  
  IF amount > mean + 2×stddev
    → Flag as "Exceptional"
  ELSE IF event_type = AMORTIZATION
    → Flag as "Exceptional"
  ELSE
    → Mark as "Baseline"
```

### 5. Forecast (Trend-Adjusted)
```
baseline = mean(last_12_months_income)
trend = linear_regression_slope(last_24_months)

annual_forecast = (baseline × 12) × (1 + trend)
```

**Example**:
- Baseline: R$ 1,000/month
- Trend: +5% per year
- **Forecast: R$ 12,600** = (1,000 × 12) × 1.05

---

## User Workflow Examples

### Workflow 1: Annual Review
```
┌──────────────────────────────────────────┐
│  Step 1: Check overall performance       │
│  $ interest income summary               │
│  → See yearly totals and growth          │
└────────────────┬─────────────────────────┘
                 │
┌────────────────▼─────────────────────────┐
│  Step 2: Analyze yield                   │
│  $ interest income yield                 │
│  → Compare to target (e.g., 6%)          │
└────────────────┬─────────────────────────┘
                 │
┌────────────────▼─────────────────────────┐
│  Step 3: Review trends                   │
│  $ interest income trends                │
│  → Is income growing or declining?       │
└────────────────┬─────────────────────────┘
                 │
┌────────────────▼─────────────────────────┐
│  Step 4: Plan for next year              │
│  $ interest income forecast 2027         │
│  → Set expectations and budget           │
└──────────────────────────────────────────┘
```

### Workflow 2: Asset Evaluation
```
┌──────────────────────────────────────────┐
│  Considering buying XPLG11...            │
│                                          │
│  Step 1: Check historical yield          │
│  $ interest income yield --ticker XPLG11 │
│  → 7.00% (good!)                         │
└────────────────┬─────────────────────────┘
                 │
┌────────────────▼─────────────────────────┐
│  Step 2: Check payment consistency       │
│  $ interest income trends --ticker XPLG11│
│  → Monthly, consistent (████████░)        │
└────────────────┬─────────────────────────┘
                 │
┌────────────────▼─────────────────────────┐
│  Step 3: Review detailed history         │
│  $ interest income detail -a XPLG11      │
│  → See actual payments over time         │
└────────────────┬─────────────────────────┘
                 │
                 ▼
         [Make informed decision]
```

### Workflow 3: Monthly Monitoring
```
┌──────────────────────────────────────────┐
│  Beginning of month...                   │
│                                          │
│  Step 1: Check expected income           │
│  $ interest income calendar --month 3    │
│  → XPLG11 on Mar 5 (R$ 180)             │
│  → MXRF11 on Mar 10 (R$ 150)            │
└────────────────┬─────────────────────────┘
                 │
     [Wait for payments to arrive...]
                 │
┌────────────────▼─────────────────────────┐
│  Mid-month check:                        │
│  $ interest income alerts                │
│  → ⚠ XPLG11: Missed expected payment     │
│  → Action: Check fund announcements      │
└──────────────────────────────────────────┘
```

---

## Success Criteria

### Phase 1 Success
✅ `income yield` command functional  
✅ `income trends` command functional  
✅ Output matches design specifications  
✅ Tests pass for yield calculations  
✅ Performance <1s for 50 assets  
✅ Integration with portfolio works  

### Phase 2 Success
✅ Baseline detection accuracy >90%  
✅ Forecast within ±15% for 12mo+ history  
✅ `income forecast` command functional  
✅ Enhanced `summary` with categorization  

### Phase 3 Success
✅ All 9 commands working  
✅ Export to Excel/CSV functional  
✅ Alert detection working  
✅ Documentation complete  
✅ User feedback positive  

### Overall Success
✅ Adoption: >50% of users try new commands  
✅ Value: Users report better portfolio decisions  
✅ Accuracy: Forecasts validated against actuals  
✅ Performance: <1s for typical portfolios  

---

## Risk Mitigation

| Risk | Mitigation |
|------|------------|
| **Forecast inaccuracy** | Provide confidence intervals, don't overstate |
| **Performance issues** | Cache computed metrics, optimize queries |
| **Complexity creep** | Start simple (Phase 1), iterate based on feedback |
| **Data quality** | Validate income events, flag anomalies |
| **User confusion** | Clear help text, examples in docs |

---

## Questions & Answers

**Q: Why not use machine learning for forecasting?**  
A: Statistical methods (trends, averages) are simpler, more transparent, and sufficient for 95% of use cases. Can add ML later if needed.

**Q: How to handle sold positions?**  
A: Filter out assets with zero current positions for yield calculations. Show warning: "Income from X sold assets excluded."

**Q: What about currency conversion (USD dividends)?**  
A: Convert to BRL at event date (current behavior). Advanced users can check notes field.

**Q: How to update forecasts when new data arrives?**  
A: Forecasts are computed on-demand. No caching needed initially. Can add cache in Phase 4.

---

**For full details, see**: [INCOME_ENHANCEMENTS.md](./INCOME_ENHANCEMENTS.md)
