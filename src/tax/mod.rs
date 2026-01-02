// Tax module - Brazilian tax calculations (FIFO, swing trade, IRPF)

pub mod cost_basis;
pub mod swing_trade;
pub mod irpf;

pub use swing_trade::calculate_monthly_tax;
pub use irpf::generate_annual_report;
