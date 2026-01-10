// Reports module - Portfolio and tax report generators

pub mod performance;
pub mod portfolio;

pub use performance::{calculate_performance, Period};
pub use portfolio::{
    calculate_allocation, calculate_portfolio, calculate_portfolio_at_date,
    invalidate_snapshots_after, PortfolioReport,
};
