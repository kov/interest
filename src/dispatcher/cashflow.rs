use crate::commands::CashFlowAction;
use crate::reports::cashflow::{self, TrendDirection};
use crate::utils::format_currency;
use crate::{db, reports};
use anyhow::{anyhow, Result};
use chrono::{Datelike, NaiveDate};
use colored::Colorize;
use rust_decimal::Decimal;
use serde::Serialize;
use std::collections::HashMap;
use tabled::{settings::Style, Table, Tabled};

pub async fn dispatch_cashflow(action: CashFlowAction, json_output: bool) -> Result<()> {
    match action {
        CashFlowAction::Show { period } => dispatch_cashflow_show(&period, json_output).await,
        CashFlowAction::Stats { period } => dispatch_cashflow_stats(&period, json_output).await,
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

async fn dispatch_cashflow_show(period_str: &str, json_output: bool) -> Result<()> {
    db::init_database(None)?;
    let conn = db::open_db(None)?;

    let period = parse_period_string(period_str)?;
    let (from_date, to_date) = crate::reports::performance::get_period_dates(period, Some(&conn))?;

    let report = cashflow::calculate_cash_flow_report(&conn, from_date, to_date)?;

    if report.years.is_empty() {
        println!(
            "\n{} No cash flow data found for the selected period.\n",
            "ℹ".blue().bold()
        );
        return Ok(());
    }

    if json_output {
        #[derive(Serialize)]
        struct AssetTypeJson {
            #[serde(rename = "in")]
            in_amount: String,
            out_sells: String,
            out_income: String,
        }

        #[derive(Serialize)]
        struct YearJson {
            year: i32,
            money_in: String,
            money_out: String,
            net_flow: String,
            by_asset_type: HashMap<String, AssetTypeJson>,
        }

        let years = report
            .years
            .iter()
            .map(|year| {
                let mut by_asset_type = HashMap::new();
                for (asset_type, values) in &year.by_asset_type {
                    by_asset_type.insert(
                        asset_type.as_str().to_string(),
                        AssetTypeJson {
                            in_amount: values.money_in.to_string(),
                            out_sells: values.money_out_sells.to_string(),
                            out_income: values.money_out_income.to_string(),
                        },
                    );
                }

                YearJson {
                    year: year.year,
                    money_in: year.money_in.to_string(),
                    money_out: year.money_out.to_string(),
                    net_flow: year.net_flow.to_string(),
                    by_asset_type,
                }
            })
            .collect::<Vec<_>>();

        let payload = serde_json::json!({
            "from_date": report.from_date,
            "to_date": report.to_date,
            "total_in": report.total_in,
            "total_out": report.total_out,
            "net_flow": report.net_flow,
            "years": years,
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }

    println!(
        "\n{} Cash Flow Summary ({} - {})\n",
        "💸".cyan().bold(),
        report.from_date,
        report.to_date
    );

    let type_order = [
        db::AssetType::Stock,
        db::AssetType::Bdr,
        db::AssetType::Fii,
        db::AssetType::Fiagro,
        db::AssetType::FiInfra,
        db::AssetType::Etf,
        db::AssetType::Fidc,
        db::AssetType::Fip,
        db::AssetType::Bond,
        db::AssetType::GovBond,
        db::AssetType::Option,
        db::AssetType::TermContract,
        db::AssetType::Unknown,
    ];

    let is_single_year = period_str
        .parse::<i32>()
        .map(|year| (1900..=2100).contains(&year))
        .unwrap_or(false);

    if is_single_year {
        let entries = cashflow::cash_flow_entries(&conn, from_date, to_date)?;
        let mut by_month: HashMap<(i32, u32), HashMap<db::AssetType, CashFlowTotals>> =
            HashMap::new();

        for entry in entries {
            let key = (entry.date.year(), entry.date.month());
            let month_map = by_month.entry(key).or_default();
            let totals = month_map.entry(entry.asset_type).or_insert((
                Decimal::ZERO,
                Decimal::ZERO,
                Decimal::ZERO,
            ));
            totals.0 += entry.money_in;
            totals.1 += entry.money_out_sells;
            totals.2 += entry.money_out_income;
        }

        let mut months: Vec<_> = by_month.into_iter().collect();
        months.sort_by_key(|(key, _)| *key);

        for ((year, month), asset_map) in months {
            let label = format!("{} {}", month_name_pt(month), year);
            println!("\n{}", label.bold().white());

            let mut total = Decimal::ZERO;
            let mut total_in = Decimal::ZERO;
            let mut total_out_sells = Decimal::ZERO;
            let mut total_out_income = Decimal::ZERO;
            for asset_type in &type_order {
                if let Some((money_in, money_out_sells, money_out_income)) =
                    asset_map.get(asset_type)
                {
                    let net = *money_in - *money_out_sells - *money_out_income;
                    if net == Decimal::ZERO {
                        continue;
                    }
                    total += net;
                    total_in += *money_in;
                    total_out_sells += *money_out_sells;
                    total_out_income += *money_out_income;
                    let net_label = format_currency(net);
                    let colored_net = if net >= Decimal::ZERO {
                        net_label.cyan()
                    } else {
                        net_label.yellow()
                    };
                    println!("  {} {}", asset_type.as_str(), colored_net);
                }
            }

            let total_label = format_currency(total);
            let colored_total = if total >= Decimal::ZERO {
                total_label.cyan()
            } else {
                total_label.yellow()
            };
            println!("  {} {}", "Total new money".bold(), colored_total);
        }
    } else {
        let mut years = report.years.clone();
        years.sort_by_key(|y| y.year);

        for year in &years {
            println!("\n{}", year.year.to_string().bold().white());

            let mut year_in = Decimal::ZERO;
            let mut year_out_sells = Decimal::ZERO;
            let mut year_out_income = Decimal::ZERO;
            for asset_type in &type_order {
                if let Some(values) = year.by_asset_type.get(asset_type) {
                    if values.net_flow == Decimal::ZERO {
                        continue;
                    }

                    year_in += values.money_in;
                    year_out_sells += values.money_out_sells;
                    year_out_income += values.money_out_income;

                    let net_label = format_currency(values.net_flow);
                    let colored_net = if values.net_flow >= Decimal::ZERO {
                        net_label.cyan()
                    } else {
                        net_label.yellow()
                    };
                    println!("  {} {}", asset_type.as_str(), colored_net);
                }
            }

            let total_label = format_currency(year.net_flow);
            let colored_total = if year.net_flow >= Decimal::ZERO {
                total_label.cyan()
            } else {
                total_label.yellow()
            };
            println!("  {} {}", "Total new money".bold(), colored_total);
        }
    }

    let mut overall_by_type: HashMap<db::AssetType, Decimal> = HashMap::new();
    for year in &report.years {
        for (asset_type, values) in &year.by_asset_type {
            *overall_by_type.entry(*asset_type).or_insert(Decimal::ZERO) += values.net_flow;
        }
    }

    println!(
        "\nTotal new money: {} ({} in, {} out)",
        format_currency(report.net_flow),
        format_currency(report.total_in),
        format_currency(report.total_out)
    );

    println!("\n{}", "Net Flow Breakdown".bold().white());

    for asset_type in &type_order {
        if let Some(net) = overall_by_type.get(asset_type) {
            if *net == Decimal::ZERO {
                continue;
            }
            let net_label = format_currency(*net);
            let colored_net = if *net >= Decimal::ZERO {
                net_label.cyan()
            } else {
                net_label.yellow()
            };
            println!("  {} {}", asset_type.as_str(), colored_net);
        }
    }

    let total_label = format_currency(report.net_flow);
    let colored_total = if report.net_flow >= Decimal::ZERO {
        total_label.cyan()
    } else {
        total_label.yellow()
    };
    println!("  {} {}", "Total new money".bold(), colored_total);

    Ok(())
}

fn month_name_pt(month: u32) -> &'static str {
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
        _ => "Mês inválido",
    }
}

async fn dispatch_cashflow_stats(period_str: &str, json_output: bool) -> Result<()> {
    db::init_database(None)?;
    let conn = db::open_db(None)?;

    let period = parse_period_string(period_str)?;
    let (from_date, to_date) = crate::reports::performance::get_period_dates(period, Some(&conn))?;

    let stats = cashflow::calculate_cash_flow_stats(&conn, from_date, to_date)?;

    if json_output {
        #[derive(Serialize)]
        struct YearlyChangeJson {
            year: i32,
            net_flow: String,
            growth_pct: Option<String>,
        }

        let yearly_changes = stats
            .trend
            .yearly_changes
            .iter()
            .map(|change| YearlyChangeJson {
                year: change.year,
                net_flow: change.curr_year_net.to_string(),
                growth_pct: change.growth_rate_pct.map(|g| g.to_string()),
            })
            .collect::<Vec<_>>();

        let trend = serde_json::json!({
            "direction": match stats.trend.direction {
                TrendDirection::Increasing => "increasing",
                TrendDirection::Decreasing => "decreasing",
                TrendDirection::Stable => "stable",
            },
        });

        let payload = serde_json::json!({
            "avg_monthly_in": stats.avg_monthly_in,
            "avg_monthly_out": stats.avg_monthly_out,
            "avg_monthly_net": stats.avg_monthly_net,
            "avg_yearly_in": stats.avg_yearly_in,
            "avg_yearly_out": stats.avg_yearly_out,
            "avg_yearly_net": stats.avg_yearly_net,
            "months_with_data": stats.months_with_data,
            "years_with_data": stats.years_with_data,
            "avg_yoy_growth_rate": stats.trend.avg_yearly_growth_rate.map(|g| g.to_string()),
            "trend": trend,
            "yearly_changes": yearly_changes,
        });

        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }

    println!(
        "\n{} Cash Flow Statistics ({} - {})\n",
        "📊".cyan().bold(),
        from_date,
        to_date
    );

    #[derive(Tabled)]
    struct MetricRow {
        #[tabled(rename = "Metric")]
        metric: String,
        #[tabled(rename = "Value")]
        value: String,
    }

    let avg_growth = stats
        .trend
        .avg_yearly_growth_rate
        .map(|g| format!("{:+.2}%", g))
        .unwrap_or_else(|| "-".to_string());

    let rows = vec![
        MetricRow {
            metric: "Avg Monthly In".to_string(),
            value: format_currency(stats.avg_monthly_in),
        },
        MetricRow {
            metric: "Avg Monthly Out".to_string(),
            value: format_currency(stats.avg_monthly_out),
        },
        MetricRow {
            metric: "Avg Monthly Net".to_string(),
            value: format_currency(stats.avg_monthly_net),
        },
        MetricRow {
            metric: "Avg Yearly In".to_string(),
            value: format_currency(stats.avg_yearly_in),
        },
        MetricRow {
            metric: "Avg Yearly Out".to_string(),
            value: format_currency(stats.avg_yearly_out),
        },
        MetricRow {
            metric: "Avg Yearly Net".to_string(),
            value: format_currency(stats.avg_yearly_net),
        },
        MetricRow {
            metric: "Months with Data".to_string(),
            value: stats.months_with_data.to_string(),
        },
        MetricRow {
            metric: "Years with Data".to_string(),
            value: stats.years_with_data.to_string(),
        },
        MetricRow {
            metric: "Avg YoY Growth".to_string(),
            value: avg_growth,
        },
    ];

    let mut table = Table::new(rows);
    table.with(Style::modern());
    println!("{}", table);

    if !stats.trend.yearly_changes.is_empty() {
        #[derive(Tabled)]
        struct YearRow {
            #[tabled(rename = "Year")]
            year: String,
            #[tabled(rename = "Net Flow")]
            net_flow: String,
            #[tabled(rename = "vs Prior Year")]
            growth: String,
        }

        let changes = stats
            .trend
            .yearly_changes
            .iter()
            .map(|change| YearRow {
                year: change.year.to_string(),
                net_flow: format_currency(change.curr_year_net),
                growth: change
                    .growth_rate_pct
                    .map(|g| format!("{:+.2}%", g))
                    .unwrap_or_else(|| "(baseline)".to_string()),
            })
            .collect::<Vec<_>>();

        println!("\n{}", "Year-over-Year Changes:".bold());
        let mut table = Table::new(changes);
        table.with(Style::modern());
        println!("{}", table);
    }

    let trend_text = match stats.trend.direction {
        TrendDirection::Increasing => "Increasing contributions over time",
        TrendDirection::Decreasing => "Decreasing contributions over time",
        TrendDirection::Stable => "Stable contributions over time",
    };

    println!("\nTrend: {}", trend_text);

    Ok(())
}
type CashFlowTotals = (Decimal, Decimal, Decimal);
