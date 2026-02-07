# Income Subcommand Enhancements - Executive Summary

> **Full Design**: See [INCOME_ENHANCEMENTS.md](./INCOME_ENHANCEMENTS.md) for complete specifications

## The Problem

Investors need to answer strategic questions about their income:
- ❓ "What is my portfolio's current yield?"
- ❓ "Is my income growing or declining?"
- ❓ "What can I expect to earn next year?"
- ❓ "Was this payment normal or exceptional?"

**Current state**: The tool tracks income data well but lacks analytical insights.

## Proposed Solution

Transform the `income` subcommand from a **passive data logger** into an **active portfolio management tool**.

### 5 New Commands

| Command | Purpose | Example Output |
|---------|---------|----------------|
| `income yield` | Calculate LTM yield by asset/type | `Portfolio: 5.00% • XPLG11: 7.00%` |
| `income trends` | Show growth/decline over time | ASCII chart + "3-Year: +45% ▲" |
| `income forecast` | Project next year's income | `2026 Expected: R$ 13,200 (±10%)` |
| `income calendar` | Predict upcoming payments | `Mar 5: XPLG11 R$ 180 (High conf)` |
| `income alerts` | Flag anomalies | `⚠ XPLG11: Missed Feb payment` |

### Enhanced Existing Commands

- `income summary --categorize` → Separate baseline (82%) from exceptional (18%) income
- `income summary --tax-aware` → Show pre-tax vs post-tax breakdown
- `income show --sort-by yield` → Rank by yield instead of amount

## Quick Examples

### Yield Analysis
```bash
$ interest income yield

Portfolio Overview (Last 12 Months):
  Total Income:     R$ 12,500
  Portfolio Value:  R$ 250,000
  Yield:            5.00%

Top Yielding Assets:
  XPLG11   7.00%  ████████░  (High consistency)
  MXRF11   6.43%  ██████████ (Very consistent)
  PETR4    4.80%  ████░░░░░  (Volatile)
```

### Trend Analysis
```bash
$ interest income trends

Monthly Income (36 months):
  ┌─────────────────────────────────┐
2500│                          ▲  ███│
2000│                    ▲    ███ ███│
1500│              ▲    ███   ███ ███│
1000│        ▲    ███   ███   ███ ███│
  └─────────────────────────────────┘
    2024-01    2024-07    2025-01

  3-Year Trend: +45% (Growing ▲)
  YoY Growth:   +12%
```

### Income Forecast
```bash
$ interest income forecast 2026

Expected Annual Income: R$ 13,200
  Confidence: Medium (67% interval: R$ 11,800 - R$ 14,600)

By Quarter:
  Q1 2026: R$ 3,600 (+8% vs 2025)
  Q2 2026: R$ 3,100 (+4%)
  Q3 2026: R$ 2,900 (+2%)
  Q4 2026: R$ 3,600 (+7%)
```

## Why This Matters

### For Retirees
✅ Verify income is meeting living expense targets  
✅ Plan for next year with confidence intervals  
✅ Identify which assets are most reliable  

### For Growth Investors
✅ Compare dividend yields across holdings  
✅ Track dividend growth over time  
✅ Make informed allocation decisions  

### For Tax Planning
✅ Understand tax burden by income source  
✅ Export detailed data for accountant  
✅ Leverage pre-2026 quota exemptions  

## Implementation Plan

### Phase 1: Core Value (2 weeks)
- ✅ `income yield` command
- ✅ `income trends` command
- Integration with portfolio system

### Phase 2: Intelligence (2 weeks)
- ✅ Baseline vs exceptional categorization
- ✅ `income forecast` command
- Confidence scoring

### Phase 3: Polish (2 weeks)
- ✅ `income calendar` command
- ✅ `income alerts` command
- Tax-aware reporting
- Export to Excel/CSV

### Phase 4: Advanced (Future)
- Monte Carlo simulations
- Benchmarking vs indices
- Interactive TUI widgets

## Technical Highlights

✅ **No database changes** - builds on existing `income_events` table  
✅ **Fast** - all queries <1 second for typical portfolios  
✅ **Maintainable** - simple statistical methods, no complex ML  
✅ **Backward compatible** - existing commands unchanged  
✅ **Well-tested** - comprehensive test plan included  

## Key Decisions

### ✅ Keep it under `income` (not separate `yield` command)
Yield is fundamentally about income - grouping is intuitive

### ✅ Start with statistical methods (not AI/ML)
- Simpler to implement and test
- More transparent to users  
- Sufficient for 95% of use cases

### ✅ No external API dependencies (for MVP)
- Use historical patterns for predictions
- Can add API integration in Phase 4

### ✅ Integrate with existing portfolio snapshots
- Leverage proven fingerprint invalidation pattern
- Share position/cost data efficiently

## Success Metrics

1. **Adoption**: >50% of active users try `income yield` within 3 months
2. **Value**: Positive user feedback that features aid decision-making
3. **Accuracy**: Forecasts within ±10% for assets with 12+ months history
4. **Performance**: All commands <1 second on portfolios with <100 assets

## Next Steps

1. ✅ Review design proposal
2. Refine priorities based on feedback
3. Begin Phase 1 implementation (`income yield` + `income trends`)
4. Gather user feedback, iterate

---

**Questions?** See full design document: [INCOME_ENHANCEMENTS.md](./INCOME_ENHANCEMENTS.md)

**Want to discuss?** Comment on the GitHub issue or PR.
