// Tax module - Brazilian tax calculations (FIFO, swing trade, IRPF)

pub mod cost_basis;
pub mod swing_trade;
pub mod irpf;

pub use cost_basis::{FifoMatcher, SaleCostBasis};
pub use swing_trade::{MonthlyTaxCalculation, calculate_monthly_tax};
pub use irpf::{AnnualTaxReport, generate_annual_report};
