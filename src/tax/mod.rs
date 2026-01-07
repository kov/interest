// Tax module - Brazilian tax calculations (average cost, swing trade, IRPF)

pub mod cost_basis;
pub mod darf;
pub mod irpf;
pub mod loss_carryforward;
pub mod swing_trade;

#[allow(unused_imports)]
pub use darf::{format_monthly_darf_summary, generate_darf_payments, DarfPayment};
pub use irpf::{generate_annual_report_with_progress, ReportProgress};
#[allow(unused_imports)]
pub use loss_carryforward::{
    apply_losses_to_profit, get_total_losses_by_category, record_loss, upsert_snapshot,
};
pub use swing_trade::calculate_monthly_tax;
