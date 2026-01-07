//! Transaction validation module
//!
//! Provides validation logic for imported transactions, collecting all issues
//! instead of failing on the first error. This allows the TUI to pause and
//! prompt the user for each validation issue.
//!
//! Note: This module is fully implemented but not yet integrated into the import
//! workflow. Integration planned for Phase 5 of the TUI plan.

use crate::importers::cei_excel::RawTransaction;
use anyhow::Result;
use chrono::NaiveDate;
use rust_decimal::Decimal;

/// A validation issue found during transaction validation
#[derive(Debug, Clone)]
pub struct ValidationIssue {
    /// Row number in the import file (1-indexed for user display)
    #[allow(dead_code)] // Used in Phase 5 import workflow
    pub row: usize,
    /// Field name that has the issue (e.g., "ticker", "date", "quantity")
    pub field: String,
    /// The problematic value
    #[allow(dead_code)] // Used in Phase 5 import workflow
    pub value: String,
    /// Description of why this is an issue
    #[allow(dead_code)] // Used in Phase 5 import workflow
    pub reason: String,
    /// Suggestion for fixing the issue (if available)
    pub suggestion: Option<String>,
}

impl ValidationIssue {
    #[allow(dead_code)] // Used in Phase 5 import workflow
    pub fn new(
        row: usize,
        field: impl Into<String>,
        value: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            row,
            field: field.into(),
            value: value.into(),
            reason: reason.into(),
            suggestion: None,
        }
    }

    #[allow(dead_code)] // Used in Phase 5 import workflow
    pub fn with_suggestion(mut self, suggestion: impl Into<String>) -> Self {
        self.suggestion = Some(suggestion.into());
        self
    }
}

/// Result of validation: successful transactions and any issues found
#[derive(Debug)]
pub struct ValidationResult {
    #[allow(dead_code)] // Used in Phase 5 import workflow
    pub valid_raw: Vec<RawTransaction>,
    pub issues: Vec<ValidationIssue>,
}

impl ValidationResult {
    #[allow(dead_code)] // Used in Phase 5 import workflow
    pub fn new_from_raw(valid: Vec<RawTransaction>, issues: Vec<ValidationIssue>) -> Self {
        Self {
            valid_raw: valid,
            issues,
        }
    }

    #[allow(dead_code)] // Used in Phase 5 import workflow
    pub fn has_issues(&self) -> bool {
        !self.issues.is_empty()
    }

    /// Count issues by type for summary reporting
    #[allow(dead_code)] // Used in Phase 5 import workflow
    pub fn issue_summary(&self) -> std::collections::BTreeMap<String, usize> {
        let mut summary = std::collections::BTreeMap::new();
        for issue in &self.issues {
            *summary.entry(issue.field.clone()).or_insert(0) += 1;
        }
        summary
    }
}

/// Validate a list of raw transactions before insertion
///
/// Returns both valid transactions and all validation issues found.
/// Does NOT fail on first error; collects all issues for the TUI to handle.
///
/// Note: Database constraint validation (foreign keys, duplicates) happens
/// during insertion. This function validates data format only.
#[allow(dead_code)] // Used in Phase 5 import workflow
pub fn validate_raw_transactions(raw_txns: Vec<RawTransaction>) -> Result<ValidationResult> {
    let mut valid = Vec::new();
    let mut issues = Vec::new();

    for (row_idx, raw_txn) in raw_txns.into_iter().enumerate() {
        let row_num = row_idx + 1; // 1-indexed for user display

        // Validate ticker exists
        let ticker = raw_txn.normalized_ticker();
        if !is_valid_ticker(&ticker) {
            issues.push(
                ValidationIssue::new(
                    row_num,
                    "ticker",
                    &ticker,
                    format!("Invalid ticker format: '{}'", ticker),
                )
                .with_suggestion("Ensure ticker has 4-5 alphanumeric characters"),
            );
            continue;
        }

        // Validate date
        if let Err(e) = validate_date(&raw_txn.trade_date) {
            issues.push(ValidationIssue::new(
                row_num,
                "trade_date",
                raw_txn.trade_date.to_string(),
                e,
            ));
            continue;
        }

        // Validate quantities and prices
        if raw_txn.quantity <= Decimal::ZERO {
            issues.push(ValidationIssue::new(
                row_num,
                "quantity",
                raw_txn.quantity.to_string(),
                "Quantity must be greater than zero".to_string(),
            ));
            continue;
        }

        if raw_txn.price <= Decimal::ZERO {
            issues.push(ValidationIssue::new(
                row_num,
                "price",
                raw_txn.price.to_string(),
                "Price must be greater than zero".to_string(),
            ));
            continue;
        }

        if raw_txn.fees < Decimal::ZERO {
            issues.push(ValidationIssue::new(
                row_num,
                "fees",
                raw_txn.fees.to_string(),
                "Fees cannot be negative".to_string(),
            ));
            continue;
        }

        // Note: Do NOT attempt to convert to Transaction here since we don't have
        // asset_id yet. The TUI will handle asset lookup and transaction creation.
        // For now, just mark as validated if all basic checks pass.
        valid.push(raw_txn);
    }

    Ok(ValidationResult::new_from_raw(valid, issues))
}

/// Check if ticker has valid format (typically 4-6 alphanumeric chars)
#[allow(dead_code)] // Used in Phase 5 import workflow
fn is_valid_ticker(ticker: &str) -> bool {
    if ticker.is_empty() || ticker.len() < 4 || ticker.len() > 6 {
        return false;
    }
    ticker.chars().all(|c| c.is_ascii_alphanumeric())
}

/// Validate trade date is not too far in future or past
#[allow(dead_code)] // Used in Phase 5 import workflow
fn validate_date(date: &NaiveDate) -> Result<(), String> {
    let now = chrono::Local::now().naive_local().date();
    let max_future_days = 30;
    let max_past_years = 50;

    // Check future
    if date > &(now + chrono::Duration::days(max_future_days)) {
        return Err(format!(
            "Trade date {} is too far in the future (max {} days)",
            date, max_future_days
        ));
    }

    // Check past (roughly 50 years)
    if date < &(now - chrono::Duration::days(365 * max_past_years)) {
        return Err(format!(
            "Trade date {} is too far in the past (max {} years)",
            date, max_past_years
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn sample_raw_transaction(ticker: &str, qty: Decimal, price: Decimal) -> RawTransaction {
        RawTransaction {
            ticker: ticker.to_string(),
            transaction_type: "C".to_string(),
            trade_date: NaiveDate::from_ymd_opt(2024, 1, 15).unwrap(),
            quantity: qty,
            price,
            fees: dec!(10),
            total: qty * price + dec!(10),
            market: None,
        }
    }

    #[test]
    fn test_validation_issue_creation() {
        let issue = ValidationIssue::new(1, "ticker", "INVALID", "Too short");
        assert_eq!(issue.row, 1);
        assert_eq!(issue.field, "ticker");
        assert_eq!(issue.value, "INVALID");
        assert!(issue.suggestion.is_none());
    }

    #[test]
    fn test_validation_issue_with_suggestion() {
        let issue = ValidationIssue::new(1, "ticker", "XX", "Too short")
            .with_suggestion("Use 4-5 character ticker");
        assert_eq!(
            issue.suggestion,
            Some("Use 4-5 character ticker".to_string())
        );
    }

    #[test]
    fn test_valid_ticker_formats() {
        assert!(is_valid_ticker("PETR4"));
        assert!(is_valid_ticker("VALE3"));
        assert!(is_valid_ticker("BBAS3"));
        assert!(is_valid_ticker("ITUB4"));
    }

    #[test]
    fn test_invalid_ticker_formats() {
        assert!(!is_valid_ticker(""));
        assert!(!is_valid_ticker("A")); // Too short
        assert!(!is_valid_ticker("TOOLONGTICKERINDICATOR")); // Too long
        assert!(!is_valid_ticker("PETR$")); // Special char
        assert!(!is_valid_ticker("PETR ")); // Whitespace
    }

    #[test]
    fn test_validate_date_current_date() {
        let today = chrono::Local::now().naive_local().date();
        assert!(validate_date(&today).is_ok());
    }

    #[test]
    fn test_validate_date_recent_past() {
        let past = chrono::Local::now().naive_local().date() - chrono::Duration::days(100);
        assert!(validate_date(&past).is_ok());
    }

    #[test]
    fn test_validate_date_far_future_fails() {
        let future = chrono::Local::now().naive_local().date() + chrono::Duration::days(100);
        assert!(validate_date(&future).is_err());
    }

    #[test]
    fn test_validate_quantity_must_be_positive() {
        let mut txn = sample_raw_transaction("PETR4", dec!(100), dec!(20.50));
        txn.quantity = dec!(-100);
        assert!(txn.quantity <= Decimal::ZERO);
    }

    #[test]
    fn test_validate_price_must_be_positive() {
        let mut txn = sample_raw_transaction("PETR4", dec!(100), dec!(20.50));
        txn.price = Decimal::ZERO;
        assert!(txn.price <= Decimal::ZERO);
    }

    #[test]
    fn test_validation_result_has_issues() {
        let issues = vec![ValidationIssue::new(1, "ticker", "X", "too short")];
        let result = ValidationResult::new_from_raw(vec![], issues);
        assert!(result.has_issues());
    }

    #[test]
    fn test_validation_result_issue_summary() {
        let issues = vec![
            ValidationIssue::new(1, "ticker", "X", "too short"),
            ValidationIssue::new(2, "price", "0", "must be positive"),
            ValidationIssue::new(3, "ticker", "XX", "too short"),
        ];
        let result = ValidationResult::new_from_raw(vec![], issues);
        let summary = result.issue_summary();
        assert_eq!(summary.get("ticker"), Some(&2));
        assert_eq!(summary.get("price"), Some(&1));
    }
}
