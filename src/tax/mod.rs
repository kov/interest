// Tax module - Brazilian tax calculations (average cost, swing trade, IRPF)

pub mod cost_basis;
pub mod swing_trade;
pub mod irpf;
pub mod loss_carryforward;
pub mod darf;

pub use swing_trade::calculate_monthly_tax;
pub use irpf::generate_annual_report;
#[allow(unused_imports)]
pub use loss_carryforward::{apply_losses_to_profit, record_loss, get_total_losses_by_category};
#[allow(unused_imports)]
pub use darf::{generate_darf_payments, format_monthly_darf_summary, DarfPayment};
