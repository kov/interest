use crate::reports::cashflow;
use crate::reports::scope::Scope;
use crate::{db, formatters, options, reports};
use anyhow::Result;

pub async fn dispatch_cashflow(
    action: &crate::cli::CashFlowCommands,
    options: options::OutputOptions,
) -> Result<()> {
    match action {
        crate::cli::CashFlowCommands::Show { period } => {
            let period_str = period.as_deref().unwrap_or("ALL");
            dispatch_cashflow_scoped(period_str, Scope::Portfolio, options).await
        }
        crate::cli::CashFlowCommands::Type { asset_type, period } => {
            let period_str = period.as_deref().unwrap_or("ALL");
            dispatch_cashflow_scoped(period_str, Scope::AssetType(*asset_type), options).await
        }
        crate::cli::CashFlowCommands::Asset { ticker, period } => {
            let period_str = period.as_deref().unwrap_or("ALL");
            dispatch_cashflow_scoped(period_str, Scope::SingleAsset(ticker.clone()), options).await
        }
        crate::cli::CashFlowCommands::Stats { period } => {
            let period_str = period.as_deref().unwrap_or("ALL");
            dispatch_cashflow_stats(period_str, options).await
        }
    }
}

fn filter_entries(
    entries: Vec<cashflow::CashFlowEntry>,
    scope: &Scope,
) -> Vec<cashflow::CashFlowEntry> {
    match scope {
        Scope::Portfolio => entries,
        Scope::AssetType(at) => entries
            .into_iter()
            .filter(|e| e.asset_type == *at)
            .collect(),
        Scope::SingleAsset(ticker) => {
            let upper = ticker.to_uppercase();
            entries.into_iter().filter(|e| e.ticker == upper).collect()
        }
    }
}

async fn dispatch_cashflow_scoped(
    period_str: &str,
    scope: Scope,
    options: options::OutputOptions,
) -> Result<()> {
    db::init_database(None)?;
    let conn = db::open_db(None)?;

    let period = reports::parse_period(period_str)?;
    let today = chrono::Local::now().date_naive();
    let (from_date, to_date) =
        crate::reports::performance::get_period_dates(period, Some(&conn), today)?;

    // Check if this is a single-year request (show monthly breakdown)
    let is_single_year = period_str
        .parse::<i32>()
        .map(|year| (1900..=2100).contains(&year))
        .unwrap_or(false);

    // Get all entries and filter by scope
    let all_entries = cashflow::cash_flow_entries(&conn, from_date, to_date)?;
    let entries = filter_entries(all_entries, &scope);

    let report = cashflow::build_report_from_entries(&entries, from_date, to_date);

    if report.years.is_empty() && !options.is_json() {
        println!("\nℹ No cash flow data found for the selected period.\n");
        return Ok(());
    }

    if is_single_year && !options.is_json() {
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

    let period = reports::parse_period(period_str)?;
    let today = chrono::Local::now().date_naive();
    let (from_date, to_date) =
        crate::reports::performance::get_period_dates(period, Some(&conn), today)?;

    let stats = cashflow::calculate_cash_flow_stats(&conn, from_date, to_date)?;

    // Format dates for table output
    let from_str = from_date.format("%Y-%m-%d").to_string();
    let to_str = to_date.format("%Y-%m-%d").to_string();

    // Print cash flow stats output (JSON or table)
    let output = formatters::cashflow::format_cashflow_stats(&stats, &from_str, &to_str, options)?;
    println!("{}", output);

    Ok(())
}
