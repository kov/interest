use anyhow::Result;
use chrono::NaiveDate;
use rusqlite::Connection;
use rust_decimal::Decimal;
use std::collections::HashMap;

use crate::db::AssetType;
use super::swing_trade::{MonthlyTaxCalculation, calculate_monthly_tax};

/// Monthly summary for IRPF
#[derive(Debug, Clone)]
pub struct MonthlyIrpfSummary {
    pub month: u32,
    pub month_name: &'static str,
    pub total_sales: Decimal,
    pub total_profit: Decimal,
    pub total_loss: Decimal,
    pub tax_due: Decimal,
    pub by_asset_type: HashMap<AssetType, AssetTypeMonthSummary>,
}

/// Asset type summary for a month
#[derive(Debug, Clone)]
pub struct AssetTypeMonthSummary {
    pub sales: Decimal,
    pub profit_loss: Decimal,
    pub tax_due: Decimal,
}

/// Annual IRPF tax report
#[derive(Debug, Clone)]
pub struct AnnualTaxReport {
    pub year: i32,
    pub monthly_summaries: Vec<MonthlyIrpfSummary>,
    pub annual_total_sales: Decimal,
    pub annual_total_profit: Decimal,
    pub annual_total_loss: Decimal,
    pub annual_total_tax: Decimal,
    pub losses_to_carry_forward: HashMap<AssetType, Decimal>,
}

/// Generate annual IRPF report for a year
pub fn generate_annual_report(
    conn: &Connection,
    year: i32,
) -> Result<AnnualTaxReport> {
    let mut monthly_summaries = Vec::new();
    let mut annual_total_sales = Decimal::ZERO;
    let mut annual_total_profit = Decimal::ZERO;
    let mut annual_total_loss = Decimal::ZERO;
    let mut annual_total_tax = Decimal::ZERO;

    // Track accumulated losses by asset type for carryforward
    let mut accumulated_losses: HashMap<AssetType, Decimal> = HashMap::new();

    // Process each month
    for month in 1..=12 {
        let month_calculations = calculate_monthly_tax(conn, year, month)?;

        if month_calculations.is_empty() {
            continue;
        }

        let mut month_sales = Decimal::ZERO;
        let mut month_profit = Decimal::ZERO;
        let mut month_loss = Decimal::ZERO;
        let mut month_tax = Decimal::ZERO;
        let mut by_asset_type: HashMap<AssetType, AssetTypeMonthSummary> = HashMap::new();

        for calc in month_calculations {
            month_sales += calc.total_sales;
            month_tax += calc.tax_due;

            let net_pl = calc.net_profit;
            if net_pl > Decimal::ZERO {
                month_profit += net_pl;
            } else {
                month_loss += net_pl.abs();

                // Track losses for carryforward
                let loss_entry = accumulated_losses.entry(calc.asset_type.clone())
                    .or_insert(Decimal::ZERO);
                *loss_entry += net_pl.abs();
            }

            by_asset_type.insert(
                calc.asset_type.clone(),
                AssetTypeMonthSummary {
                    sales: calc.total_sales,
                    profit_loss: calc.net_profit,
                    tax_due: calc.tax_due,
                },
            );
        }

        annual_total_sales += month_sales;
        annual_total_profit += month_profit;
        annual_total_loss += month_loss;
        annual_total_tax += month_tax;

        monthly_summaries.push(MonthlyIrpfSummary {
            month,
            month_name: get_month_name(month),
            total_sales: month_sales,
            total_profit: month_profit,
            total_loss: month_loss,
            tax_due: month_tax,
            by_asset_type,
        });
    }

    Ok(AnnualTaxReport {
        year,
        monthly_summaries,
        annual_total_sales,
        annual_total_profit,
        annual_total_loss,
        annual_total_tax,
        losses_to_carry_forward: accumulated_losses,
    })
}

/// Get month name in Portuguese
fn get_month_name(month: u32) -> &'static str {
    match month {
        1 => "Janeiro",
        2 => "Fevereiro",
        3 => "Março",
        4 => "Abril",
        5 => "Maio",
        6 => "Junho",
        7 => "Julho",
        8 => "Agosto",
        9 => "Setembro",
        10 => "Outubro",
        11 => "Novembro",
        12 => "Dezembro",
        _ => "Unknown",
    }
}

/// Export annual report to CSV format
pub fn export_to_csv(report: &AnnualTaxReport) -> String {
    let mut csv = String::new();

    // Header
    csv.push_str("Mês,Vendas Totais,Lucro,Prejuízo,Imposto Devido\n");

    // Monthly rows
    for summary in &report.monthly_summaries {
        csv.push_str(&format!(
            "{},{:.2},{:.2},{:.2},{:.2}\n",
            summary.month_name,
            summary.total_sales,
            summary.total_profit,
            summary.total_loss,
            summary.tax_due
        ));
    }

    // Total row
    csv.push_str(&format!(
        "\nTOTAL ANUAL,{:.2},{:.2},{:.2},{:.2}\n",
        report.annual_total_sales,
        report.annual_total_profit,
        report.annual_total_loss,
        report.annual_total_tax
    ));

    // Losses to carry forward
    if !report.losses_to_carry_forward.is_empty() {
        csv.push_str("\nPREJUÍZOS A COMPENSAR\n");
        for (asset_type, loss) in &report.losses_to_carry_forward {
            csv.push_str(&format!("{},{:.2}\n", asset_type.as_str(), loss));
        }
    }

    csv
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_month_names() {
        assert_eq!(get_month_name(1), "Janeiro");
        assert_eq!(get_month_name(12), "Dezembro");
    }

    #[test]
    fn test_csv_export() {
        let report = AnnualTaxReport {
            year: 2025,
            monthly_summaries: vec![],
            annual_total_sales: Decimal::from(100000),
            annual_total_profit: Decimal::from(15000),
            annual_total_loss: Decimal::from(2000),
            annual_total_tax: Decimal::from(1950),
            losses_to_carry_forward: HashMap::new(),
        };

        let csv = export_to_csv(&report);
        assert!(csv.contains("TOTAL ANUAL"));
        assert!(csv.contains("100000.00"));
    }
}
