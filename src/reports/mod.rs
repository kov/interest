// Reports module - Portfolio and tax report generators

pub mod portfolio;

pub use portfolio::{PortfolioReport, PositionSummary, calculate_portfolio, calculate_allocation};
