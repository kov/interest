use anyhow::{anyhow, Result};
use chrono::{Datelike, NaiveDate};

/// Time period for reports
#[derive(Debug, Clone)]
pub enum Period {
    Mtd,     // Month-to-date
    Qtd,     // Quarter-to-date
    Ytd,     // Year-to-date
    OneYear, // Last 365 days
    AllTime, // Since first transaction
    Custom { from: NaiveDate, to: NaiveDate },
}

/// Parse a period string into a Period enum.
///
/// Accepted formats:
/// - Named: `MTD`, `QTD`, `YTD`, `1Y`, `ALL` (case-insensitive)
/// - Year: `2025` (full year range: Jan 1 to Dec 31)
/// - Custom range: `2024-01-01:2024-06-30`
/// - Single date: `2024-06-15` or `2024-06` (point-in-time)
pub fn parse_period(period: &str) -> Result<Period> {
    let upper = period.to_uppercase();
    match upper.as_str() {
        "MTD" => return Ok(Period::Mtd),
        "QTD" => return Ok(Period::Qtd),
        "YTD" => return Ok(Period::Ytd),
        "1Y" | "ONEYEAR" => return Ok(Period::OneYear),
        "ALL" | "ALLTIME" => return Ok(Period::AllTime),
        _ => {}
    }

    // Year shorthand: YYYY → full year range
    if let Ok(year) = period.parse::<i32>() {
        if (1900..=2100).contains(&year) {
            let from = NaiveDate::from_ymd_opt(year, 1, 1)
                .ok_or_else(|| anyhow!("Invalid year: {}", year))?;
            let to = NaiveDate::from_ymd_opt(year, 12, 31)
                .ok_or_else(|| anyhow!("Invalid year: {}", year))?;
            return Ok(Period::Custom { from, to });
        }
    }

    // Custom range: from:to
    if let Some((from_str, to_str)) = period.split_once(':') {
        let from = parse_date_flexible(from_str)
            .map_err(|_| anyhow!("Invalid from date: {}. Use YYYY-MM-DD format.", from_str))?;
        let to = parse_date_flexible(to_str)
            .map_err(|_| anyhow!("Invalid to date: {}. Use YYYY-MM-DD format.", to_str))?;
        return Ok(Period::Custom { from, to });
    }

    // Single date (point-in-time)
    if let Ok(date) = parse_date_flexible(period) {
        return Ok(Period::Custom {
            from: date,
            to: date,
        });
    }

    Err(anyhow!(
        "Invalid period '{}'. Use: MTD, QTD, YTD, 1Y, ALL, YYYY, or from:to (YYYY-MM-DD:YYYY-MM-DD)",
        period
    ))
}

/// Parse a date string in flexible formats.
///
/// Supported:
/// - `YYYY-MM-DD` → exact date
/// - `YYYY-MM` → last day of month
/// - `YYYY` → December 31
pub fn parse_date_flexible(s: &str) -> Result<NaiveDate> {
    // YYYY-MM-DD (exact date)
    if let Ok(date) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Ok(date);
    }

    // YYYY-MM (last day of month)
    if let Ok(ym) = NaiveDate::parse_from_str(&format!("{}-01", s), "%Y-%m-%d") {
        let next_month = if ym.month() == 12 {
            NaiveDate::from_ymd_opt(ym.year() + 1, 1, 1)
        } else {
            NaiveDate::from_ymd_opt(ym.year(), ym.month() + 1, 1)
        };
        if let Some(nm) = next_month {
            return Ok(nm.pred_opt().unwrap_or(ym));
        }
    }

    // YYYY (December 31)
    if let Ok(year) = s.parse::<i32>() {
        if (1900..=2100).contains(&year) {
            return NaiveDate::from_ymd_opt(year, 12, 31)
                .ok_or_else(|| anyhow!("Invalid year: {}", year));
        }
    }

    Err(anyhow!(
        "Invalid date '{}'. Use YYYY-MM-DD, YYYY-MM, or YYYY",
        s
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_period_named() {
        assert!(matches!(parse_period("MTD").unwrap(), Period::Mtd));
        assert!(matches!(parse_period("mtd").unwrap(), Period::Mtd));
        assert!(matches!(parse_period("QTD").unwrap(), Period::Qtd));
        assert!(matches!(parse_period("YTD").unwrap(), Period::Ytd));
        assert!(matches!(parse_period("ytd").unwrap(), Period::Ytd));
        assert!(matches!(parse_period("1Y").unwrap(), Period::OneYear));
        assert!(matches!(parse_period("ALL").unwrap(), Period::AllTime));
    }

    #[test]
    fn test_parse_period_year() {
        let period = parse_period("2024").unwrap();
        if let Period::Custom { from, to } = period {
            assert_eq!(from, NaiveDate::from_ymd_opt(2024, 1, 1).unwrap());
            assert_eq!(to, NaiveDate::from_ymd_opt(2024, 12, 31).unwrap());
        } else {
            panic!("Expected Custom period for year");
        }
    }

    #[test]
    fn test_parse_period_custom_range() {
        let period = parse_period("2024-01-01:2024-12-31").unwrap();
        if let Period::Custom { from, to } = period {
            assert_eq!(from, NaiveDate::from_ymd_opt(2024, 1, 1).unwrap());
            assert_eq!(to, NaiveDate::from_ymd_opt(2024, 12, 31).unwrap());
        } else {
            panic!("Expected Custom period");
        }
    }

    #[test]
    fn test_parse_period_single_date() {
        let period = parse_period("2024-06-15").unwrap();
        if let Period::Custom { from, to } = period {
            let expected = NaiveDate::from_ymd_opt(2024, 6, 15).unwrap();
            assert_eq!(from, expected);
            assert_eq!(to, expected);
        } else {
            panic!("Expected Custom period for single date");
        }
    }

    #[test]
    fn test_parse_period_single_month() {
        let period = parse_period("2024-06").unwrap();
        if let Period::Custom { from, to } = period {
            let expected = NaiveDate::from_ymd_opt(2024, 6, 30).unwrap();
            assert_eq!(from, expected);
            assert_eq!(to, expected);
        } else {
            panic!("Expected Custom period for single month");
        }
    }

    #[test]
    fn test_parse_period_invalid() {
        assert!(parse_period("INVALID").is_err());
    }

    #[test]
    fn test_parse_date_flexible_full() {
        let date = parse_date_flexible("2024-06-15").unwrap();
        assert_eq!(date, NaiveDate::from_ymd_opt(2024, 6, 15).unwrap());
    }

    #[test]
    fn test_parse_date_flexible_month() {
        let date = parse_date_flexible("2024-06").unwrap();
        assert_eq!(date, NaiveDate::from_ymd_opt(2024, 6, 30).unwrap());
    }

    #[test]
    fn test_parse_date_flexible_year() {
        let date = parse_date_flexible("2024").unwrap();
        assert_eq!(date, NaiveDate::from_ymd_opt(2024, 12, 31).unwrap());
    }

    #[test]
    fn test_parse_date_flexible_february_leap_year() {
        let date = parse_date_flexible("2024-02").unwrap();
        assert_eq!(date, NaiveDate::from_ymd_opt(2024, 2, 29).unwrap());
    }

    #[test]
    fn test_parse_date_flexible_february_non_leap() {
        let date = parse_date_flexible("2023-02").unwrap();
        assert_eq!(date, NaiveDate::from_ymd_opt(2023, 2, 28).unwrap());
    }

    #[test]
    fn test_parse_date_flexible_invalid() {
        assert!(parse_date_flexible("not-a-date").is_err());
    }
}
