// Reports module - Portfolio and tax report generators

pub mod performance;
pub mod portfolio;

pub use performance::{
    backfill_daily_snapshots, calculate_performance, get_period_dates, AssetPerformance,
    PerformanceReport, Period,
};
pub use portfolio::{
    calculate_allocation, calculate_portfolio, calculate_portfolio_at_date,
    compute_snapshot_fingerprint, get_valid_snapshot, invalidate_snapshots_after,
    save_portfolio_snapshot, PortfolioReport,
};
