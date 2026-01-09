//! Utility functions for formatting and common operations
//!
//! This module provides centralized formatting utilities for consistent
//! display of currency and decimal values throughout the application.

use rust_decimal::Decimal;

/// Currency symbol options for formatting
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CurrencySymbol {
    /// Include "R$ " prefix (Brazilian Real)
    BRL,
    /// No currency symbol (for table cells, calculations display)
    #[allow(dead_code)]
    None,
}

/// Core formatting function with full control over output.
///
/// Formats a Decimal value using Brazilian locale conventions:
/// - Thousands separator: `.` (period)
/// - Decimal separator: `,` (comma)
///
/// # Arguments
/// * `value` - The decimal value to format
/// * `width` - Minimum width for padding (0 for no padding, right-aligned)
/// * `symbol` - Whether to include currency symbol
///
/// # Examples
/// ```
/// use interest::utils::{format_currency_with_width, CurrencySymbol};
/// use rust_decimal_macros::dec;
///
/// assert_eq!(
///     format_currency_with_width(dec!(1234.56), 0, CurrencySymbol::BRL),
///     "R$ 1.234,56"
/// );
///
/// assert_eq!(
///     format_currency_with_width(dec!(1234), 15, CurrencySymbol::None),
///     "       1.234,00"
/// );
/// ```
pub fn format_currency_with_width(value: Decimal, width: usize, symbol: CurrencySymbol) -> String {
    let is_negative = value < Decimal::ZERO;
    let abs_value = value.abs();

    // Round to 2 decimal places and format
    let formatted = format!("{:.2}", abs_value);
    let parts: Vec<&str> = formatted.split('.').collect();

    let integer_part = parts[0];
    let decimal_part = parts.get(1).unwrap_or(&"00");

    // Add thousands separators (.) to integer part
    let with_separators: String = integer_part
        .chars()
        .rev()
        .enumerate()
        .flat_map(|(i, c)| {
            if i > 0 && i % 3 == 0 {
                vec!['.', c]
            } else {
                vec![c]
            }
        })
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();

    let sign = if is_negative { "-" } else { "" };
    let prefix = match symbol {
        CurrencySymbol::BRL => "R$ ",
        CurrencySymbol::None => "",
    };

    let result = format!("{}{}{},{}", prefix, sign, with_separators, decimal_part);

    // Apply width padding (right-align)
    if width > 0 && result.len() < width {
        format!("{:>width$}", result, width = width)
    } else {
        result
    }
}

// ============ Convenience functions ============

/// Format as Brazilian Real with symbol: "R$ 1.234,56"
///
/// # Examples
/// ```
/// use interest::utils::format_currency;
/// use rust_decimal_macros::dec;
///
/// assert_eq!(format_currency(dec!(1234.56)), "R$ 1.234,56");
/// assert_eq!(format_currency(dec!(-500)), "R$ -500,00");
/// ```
pub fn format_currency(value: Decimal) -> String {
    format_currency_with_width(value, 0, CurrencySymbol::BRL)
}

/// Format as Brazilian Real, right-aligned to specified width.
///
/// # Examples
/// ```
/// use interest::utils::format_currency_aligned;
/// use rust_decimal_macros::dec;
///
/// let result = format_currency_aligned(dec!(100), 15);
/// assert_eq!(result, "      R$ 100,00");
/// ```
pub fn format_currency_aligned(value: Decimal, width: usize) -> String {
    format_currency_with_width(value, width, CurrencySymbol::BRL)
}

/// Format number only (no symbol): "1.234,56"
///
/// # Examples
/// ```
/// use interest::utils::format_decimal_br;
/// use rust_decimal_macros::dec;
///
/// assert_eq!(format_decimal_br(dec!(1234.56)), "1.234,56");
/// ```
#[allow(dead_code)]
pub fn format_decimal_br(value: Decimal) -> String {
    format_currency_with_width(value, 0, CurrencySymbol::None)
}

/// Format number only, right-aligned to specified width.
///
/// # Examples
/// ```
/// use interest::utils::format_decimal_br_aligned;
/// use rust_decimal_macros::dec;
///
/// let result = format_decimal_br_aligned(dec!(1234.56), 12);
/// assert_eq!(result, "    1.234,56");
/// ```
#[allow(dead_code)]
pub fn format_decimal_br_aligned(value: Decimal, width: usize) -> String {
    format_currency_with_width(value, width, CurrencySymbol::None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_format_currency_basic() {
        assert_eq!(format_currency(dec!(1234.56)), "R$ 1.234,56");
        assert_eq!(format_currency(dec!(0.99)), "R$ 0,99");
        assert_eq!(format_currency(dec!(1000000)), "R$ 1.000.000,00");
    }

    #[test]
    fn test_format_currency_small_values() {
        assert_eq!(format_currency(dec!(0)), "R$ 0,00");
        assert_eq!(format_currency(dec!(0.01)), "R$ 0,01");
        assert_eq!(format_currency(dec!(1)), "R$ 1,00");
        assert_eq!(format_currency(dec!(12)), "R$ 12,00");
        assert_eq!(format_currency(dec!(123)), "R$ 123,00");
        assert_eq!(format_currency(dec!(999.99)), "R$ 999,99");
    }

    #[test]
    fn test_format_currency_large_values() {
        assert_eq!(format_currency(dec!(1000)), "R$ 1.000,00");
        assert_eq!(format_currency(dec!(12345)), "R$ 12.345,00");
        assert_eq!(format_currency(dec!(123456)), "R$ 123.456,00");
        assert_eq!(format_currency(dec!(1234567)), "R$ 1.234.567,00");
        assert_eq!(format_currency(dec!(12345678.90)), "R$ 12.345.678,90");
    }

    #[test]
    fn test_format_currency_negative() {
        assert_eq!(format_currency(dec!(-1234.56)), "R$ -1.234,56");
        assert_eq!(format_currency(dec!(-0.01)), "R$ -0,01");
        assert_eq!(format_currency(dec!(-1000000)), "R$ -1.000.000,00");
    }

    #[test]
    fn test_format_decimal_br() {
        assert_eq!(format_decimal_br(dec!(1234.56)), "1.234,56");
        assert_eq!(format_decimal_br(dec!(0)), "0,00");
        assert_eq!(format_decimal_br(dec!(-500)), "-500,00");
    }

    #[test]
    fn test_format_with_width() {
        // "R$ 100,00" is 10 chars, padding to 15 adds 5 spaces
        let result = format_currency_aligned(dec!(100), 15);
        assert_eq!(result.len(), 15);
        assert_eq!(result, "      R$ 100,00");

        // Test decimal_br_aligned
        let result2 = format_decimal_br_aligned(dec!(1234.56), 12);
        assert_eq!(result2.len(), 12);
        assert_eq!(result2, "    1.234,56");
    }

    #[test]
    fn test_format_with_width_no_padding_needed() {
        // If result is already >= width, no padding added
        let result = format_currency_aligned(dec!(1000000), 5);
        assert_eq!(result, "R$ 1.000.000,00");
    }

    #[test]
    fn test_precision() {
        // Values with more than 2 decimal places are truncated to 2 places
        // by the {:.2} format specifier
        assert_eq!(format_currency(dec!(1.234)), "R$ 1,23");
        // 1.999 truncates to 1.99 (not rounded to 2.00)
        assert_eq!(format_currency(dec!(1.99)), "R$ 1,99");
        // Exact values display correctly
        assert_eq!(format_currency(dec!(2.00)), "R$ 2,00");
    }
}
