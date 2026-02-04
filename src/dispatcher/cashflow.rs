use crate::reports::cashflow;
use crate::{db, formatters, options, reports};
use anyhow::{anyhow, Result};
use chrono::NaiveDate;

pub async fn dispatch_cashflow(
    action: &crate::cli::CashFlowCommands,
    options: options::OutputOptions,
) -> Result<()> {
    match action {
        crate::cli::CashFlowCommands::Show { period } => {
            let period_str = period.as_deref().unwrap_or("ALL");
            dispatch_cashflow_show(period_str, options).await
        }
        crate::cli::CashFlowCommands::Stats { period } => {
            let period_str = period.as_deref().unwrap_or("ALL");
            dispatch_cashflow_stats(period_str, options).await
        }
    }
}

/// Parse a period string (MTD, QTD, YTD, 1Y, ALL, YYYY, or from:to)
fn parse_period_string(period: &str) -> Result<reports::Period> {
    let upper = period.to_uppercase();
    match upper.as_str() {
        "MTD" => Ok(reports::Period::Mtd),
        "QTD" => Ok(reports::Period::Qtd),
        "YTD" => Ok(reports::Period::Ytd),
        "1Y" | "ONEYEAR" => Ok(reports::Period::OneYear),
        "ALL" | "ALLTIME" => Ok(reports::Period::AllTime),
        _ => {
            if let Ok(year) = period.parse::<i32>() {
                if (1900..=2100).contains(&year) {
                    let from = NaiveDate::from_ymd_opt(year, 1, 1)
                        .ok_or_else(|| anyhow!("Invalid year: {}", year))?;
                    let to = NaiveDate::from_ymd_opt(year, 12, 31)
                        .ok_or_else(|| anyhow!("Invalid year: {}", year))?;
                    return Ok(reports::Period::Custom { from, to });
                }
            }

            if let Some((from_str, to_str)) = period.split_once(':') {
                let from = NaiveDate::parse_from_str(from_str, "%Y-%m-%d").map_err(|_| {
                    anyhow!("Invalid from date: {}. Use YYYY-MM-DD format.", from_str)
                })?;
                let to = NaiveDate::parse_from_str(to_str, "%Y-%m-%d")
                    .map_err(|_| anyhow!("Invalid to date: {}. Use YYYY-MM-DD format.", to_str))?;
                Ok(reports::Period::Custom { from, to })
            } else {
                Err(anyhow!(
                    "Invalid period '{}'. Use: MTD, QTD, YTD, 1Y, ALL, YYYY, or from:to (YYYY-MM-DD:YYYY-MM-DD)",
                    period
                ))
            }
        }
    }
}

async fn dispatch_cashflow_show(period_str: &str, options: options::OutputOptions) -> Result<()> {
    db::init_database(None)?;
    let conn = db::open_db(None)?;

    let period = parse_period_string(period_str)?;
    let (from_date, to_date) = crate::reports::performance::get_period_dates(period, Some(&conn))?;

    // Check if this is a single-year request (show monthly breakdown)
    let is_single_year = period_str
        .parse::<i32>()
        .map(|year| (1900..=2100).contains(&year))
        .unwrap_or(false);

    let report = cashflow::calculate_cash_flow_report(&conn, from_date, to_date)?;

    if report.years.is_empty() && !options.is_json() {
        println!("\nℹ No cash flow data found for the selected period.\n");
        return Ok(());
    }

    // Print cash flow output (JSON or table)
    // Even if there are no years, output valid JSON for json_output mode
    // For table output, check if we should show monthly breakdown
    if is_single_year && !options.is_json() {
        let entries = cashflow::cash_flow_entries(&conn, from_date, to_date)?;
        let output =
            formatters::cashflow::format_cashflow_show_monthly(&report, &entries, options)?;
        println!("{}", output);
    } else {
        let output = formatters::cashflow::format_cashflow_show(&report, options)?;
        println!("{}", output);
    }

    Ok(())
}

async fn dispatch_cashflow_stats(period_str: &str, options: options::OutputOptions) -> Result<()> {
    db::init_database(None)?;
    let conn = db::open_db(None)?;

    let period = parse_period_string(period_str)?;
    let (from_date, to_date) = crate::reports::performance::get_period_dates(period, Some(&conn))?;

    let stats = cashflow::calculate_cash_flow_stats(&conn, from_date, to_date)?;

    // Format dates for table output
    let from_str = from_date.format("%Y-%m-%d").to_string();
    let to_str = to_date.format("%Y-%m-%d").to_string();

    // Print cash flow stats output (JSON or table)
    let output = formatters::cashflow::format_cashflow_stats(&stats, &from_str, &to_str, options)?;
    println!("{}", output);

    Ok(())
}
