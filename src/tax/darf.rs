use anyhow::Result;
use chrono::NaiveDate;
use rust_decimal::Decimal;

use super::swing_trade::{MonthlyTaxCalculation, TaxCategory};
use crate::utils::format_currency;

/// DARF payment information
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct DarfPayment {
    pub year: i32,
    pub month: u32,
    pub category: TaxCategory,
    pub darf_code: String,
    pub description: String,
    pub tax_due: Decimal,
    pub due_date: NaiveDate,
}

/// Generate DARF payments from monthly tax calculations
pub fn generate_darf_payments(
    calculations: Vec<MonthlyTaxCalculation>,
    year: i32,
    month: u32,
) -> Result<Vec<DarfPayment>> {
    let mut payments = Vec::new();

    // Calculate due date (last business day of the following month)
    let due_date = calculate_darf_due_date(year, month)?;

    for calc in calculations {
        // Skip if no tax due or exempt category
        if calc.tax_due <= Decimal::ZERO {
            continue;
        }

        if let Some(darf_code) = calc.category.darf_code() {
            payments.push(DarfPayment {
                year,
                month,
                category: calc.category.clone(),
                darf_code: darf_code.to_string(),
                description: calc.category.darf_description().to_string(),
                tax_due: calc.tax_due,
                due_date,
            });
        }
    }

    Ok(payments)
}

/// Calculate DARF due date
/// Tax is due on the last business day of the month following the transaction month
/// For simplicity, we use the last day of the month (business day check can be added later)
fn calculate_darf_due_date(year: i32, month: u32) -> Result<NaiveDate> {
    // Get the following month
    let (due_year, due_month) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };

    // Get last day of the due month
    let last_day = if due_month == 12 {
        NaiveDate::from_ymd_opt(due_year + 1, 1, 1)
            .unwrap()
            .pred_opt()
            .unwrap()
    } else {
        NaiveDate::from_ymd_opt(due_year, due_month + 1, 1)
            .unwrap()
            .pred_opt()
            .unwrap()
    };

    Ok(last_day)
}

/// Format DARF payment for display
#[allow(dead_code)]
pub fn format_darf_payment(payment: &DarfPayment) -> String {
    format!(
        "DARF {code} - {description}\n  Vencimento: {due_date}\n  Valor: {amount}",
        code = payment.darf_code,
        description = payment.description,
        due_date = payment.due_date.format("%d/%m/%Y"),
        amount = format_currency(payment.tax_due)
    )
}

/// Format all DARF payments for a month
#[allow(dead_code)]
pub fn format_monthly_darf_summary(payments: &[DarfPayment], year: i32, month: u32) -> String {
    if payments.is_empty() {
        return format!("Nenhum DARF a pagar para {}/{}", month, year);
    }

    let mut output = format!("DARFs a pagar referente a {}/{}:\n\n", month, year);

    for payment in payments {
        output.push_str(&format_darf_payment(payment));
        output.push_str("\n\n");
    }

    let total: Decimal = payments.iter().map(|p| p.tax_due).sum();
    output.push_str(&format!("Total: {}", format_currency(total)));

    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_darf_due_date_calculation() {
        // Tax for January (month 1) is due at end of February
        let due_date = calculate_darf_due_date(2024, 1).unwrap();
        assert_eq!(due_date, NaiveDate::from_ymd_opt(2024, 2, 29).unwrap()); // 2024 is leap year

        // Tax for February 2023 (non-leap year) is due at end of March
        let due_date = calculate_darf_due_date(2023, 2).unwrap();
        assert_eq!(due_date, NaiveDate::from_ymd_opt(2023, 3, 31).unwrap());

        // Tax for December is due at end of January next year
        let due_date = calculate_darf_due_date(2024, 12).unwrap();
        assert_eq!(due_date, NaiveDate::from_ymd_opt(2025, 1, 31).unwrap());
    }

    #[test]
    fn test_darf_code_mapping() {
        assert_eq!(TaxCategory::StockSwingTrade.darf_code(), Some("6015"));
        assert_eq!(TaxCategory::StockDayTrade.darf_code(), Some("6015"));
        assert_eq!(TaxCategory::FiiSwingTrade.darf_code(), Some("6015"));
        assert_eq!(TaxCategory::FiInfra.darf_code(), None); // Exempt
    }

    #[test]
    fn test_format_darf_payment() {
        let payment = DarfPayment {
            year: 2024,
            month: 1,
            category: TaxCategory::StockSwingTrade,
            darf_code: "6015".to_string(),
            description: "Renda Variável - Operações Comuns".to_string(),
            tax_due: dec!(1500.00),
            due_date: NaiveDate::from_ymd_opt(2024, 2, 29).unwrap(),
        };

        let formatted = format_darf_payment(&payment);
        assert!(formatted.contains("DARF 6015"));
        assert!(formatted.contains("Renda Variável"));
        assert!(formatted.contains("29/02/2024"));
        assert!(formatted.contains("R$ 1.500,00")); // Brazilian locale format
    }
}
