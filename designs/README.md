# Income Subcommand Enhancement - Design Index

This directory contains the complete design proposal for enhancing the `income` subcommand in the Interest portfolio tracker.

## 📚 Documentation Files

### Quick Start
👉 **Start here**: [INCOME_ENHANCEMENTS_SUMMARY.md](./INCOME_ENHANCEMENTS_SUMMARY.md)
- Executive summary (5 minutes read)
- Problem statement and proposed solution
- Key features overview
- Use cases and benefits

### Visual Overview
📊 **Next**: [INCOME_ENHANCEMENTS_VISUAL.md](./INCOME_ENHANCEMENTS_VISUAL.md)
- Command structure diagrams
- Data flow architecture
- User workflow examples
- Calculation formulas explained

### Full Specification
📘 **Detailed Reference**: [INCOME_ENHANCEMENTS.md](./INCOME_ENHANCEMENTS.md)
- Complete technical specification
- Detailed feature descriptions
- Implementation plan (4 phases)
- Testing strategy
- Open questions and alternatives

---

## 🎯 What's Being Proposed

Transform the income subcommand from a **passive data logger** into an **active portfolio management tool**.

### New Capabilities

| Feature | What it does | Value |
|---------|--------------|-------|
| **Yield Calculations** | LTM yield % by portfolio/asset/type | "Is my 5% yield target being met?" |
| **Trend Analysis** | Growth/decline tracking with charts | "Is my income growing over time?" |
| **Forecasting** | Predict next year's income | "What can I expect to earn in 2027?" |
| **Baseline Detection** | Recurring vs one-time income | "Was this payment normal or special?" |
| **Calendar** | Predicted payment dates | "When will I receive my next dividend?" |
| **Alerts** | Anomaly detection | "Why didn't XPLG11 pay this month?" |

### 5 New Commands
```bash
interest income yield              # Calculate portfolio yield
interest income trends             # Show growth/decline over time
interest income forecast 2027      # Project future income
interest income calendar           # Upcoming payment predictions
interest income alerts             # Detect anomalies
```

### Enhanced Existing Commands
```bash
interest income show --sort-by yield        # Rank by yield, not amount
interest income summary --categorize        # Baseline vs exceptional
interest income summary --tax-aware         # Pre-tax vs post-tax
```

---

## 🏗️ Implementation Roadmap

**Phase 1** (Weeks 1-2): Core Value
- ✅ `income yield` command
- ✅ `income trends` command
- ✅ Portfolio integration

**Phase 2** (Weeks 3-4): Intelligence  
- ✅ Baseline detection
- ✅ `income forecast` command
- ✅ Confidence scoring

**Phase 3** (Weeks 5-6): Polish
- ✅ `income calendar` + `income alerts`
- ✅ Export (Excel/CSV)
- ✅ Tax-aware reporting

**Phase 4** (Future): Advanced
- ⭕ Monte Carlo simulations
- ⭕ Benchmarking vs indices
- ⭕ Interactive TUI widgets

---

## 💡 Key Design Decisions

✅ **No database schema changes** - builds on existing tables  
✅ **Simple statistics** - no complex ML, easy to understand  
✅ **Incremental rollout** - Phase 1 delivers immediate value  
✅ **Backward compatible** - existing commands unchanged  
✅ **Fast performance** - <1 second for typical portfolios  

---

## 🎓 Use Cases

**Retiree Seeking Income Stability**:
```bash
$ interest income yield          # Check 6% target
$ interest income trends         # Verify stability
$ interest income forecast       # Plan expenses
```

**Growth Investor Evaluating Assets**:
```bash
$ interest income yield --ticker XPLG11   # Check yield
$ interest income trends --ticker XPLG11  # Review consistency
$ interest income detail -a XPLG11        # See payment history
```

**Tax Planning**:
```bash
$ interest income summary --tax-aware --year 2025
$ interest income detail --year 2025 --json > income_2025.json
```

---

## 📊 Success Metrics

1. **Adoption**: >50% of users try new commands within 3 months
2. **Value**: Positive feedback on decision-making support
3. **Accuracy**: Forecasts within ±10% for 12mo+ history
4. **Performance**: <1 second response time

---

## 🚀 Current Status

✅ **Design Complete** - All documentation delivered  
✅ **Three Documents** - Summary, Visual, Full Spec (52KB total)  
✅ **Ready for Review** - Awaiting stakeholder feedback  

**Next Step**: Review design, refine priorities, begin Phase 1 implementation

---

## 📞 Questions?

For questions or feedback:
- Comment on the GitHub issue/PR
- Review the detailed specifications in the files above
- Contact the maintainers

---

## 📈 Document Stats

| Document | Size | Lines | Purpose |
|----------|------|-------|---------|
| INCOME_ENHANCEMENTS.md | 30KB | 801 | Full technical spec |
| INCOME_ENHANCEMENTS_SUMMARY.md | 5KB | 167 | Executive summary |
| INCOME_ENHANCEMENTS_VISUAL.md | 17KB | 364 | Diagrams & workflows |
| **TOTAL** | **52KB** | **1,332** | Complete design |

---

**Last Updated**: 2026-02-07  
**Status**: Design Complete ✅  
**Next Phase**: Stakeholder Review → Implementation Planning
