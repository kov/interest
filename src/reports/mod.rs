// Reports module - Portfolio and tax report generators

pub mod aggregation;
pub mod cashflow;
pub mod enrichment;
pub mod income_analytics;
pub mod performance;
pub mod period;
pub mod portfolio;
pub mod scope;

pub use performance::calculate_performance;
pub use period::parse_period;
#[cfg(test)]
pub use period::Period;
pub use portfolio::{
    calculate_portfolio, calculate_portfolio_at_date, invalidate_snapshots_after, PortfolioReport,
};
