use anyhow::Result;
use rusqlite::Connection;
use rust_decimal::Decimal;
use std::collections::HashMap;

use super::loss_carryforward::{
    clear_year_losses, compute_year_fingerprint, earliest_transaction_year, load_snapshots,
    record_loss, upsert_snapshot,
};
use super::swing_trade::{calculate_monthly_tax, TaxCategory};
use tracing::debug;

/// Monthly summary for IRPF
///
#[derive(Debug, Clone)]
pub struct MonthlyIrpfSummary {
    pub month_name: &'static str,
    pub total_sales: Decimal,
    pub total_profit: Decimal,
    pub total_loss: Decimal,
    pub total_loss_offset_applied: Decimal,
    pub tax_due: Decimal,
    #[allow(dead_code)]
    pub by_category: HashMap<TaxCategory, CategoryMonthSummary>,
}

/// Tax category summary for a month
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct CategoryMonthSummary {
    pub sales: Decimal,
    pub profit_loss: Decimal,
    pub loss_offset_applied: Decimal,
    pub exemption_applied: Decimal,
    pub tax_due: Decimal,
}

/// Annual IRPF tax report
#[derive(Debug, Clone)]
pub struct AnnualTaxReport {
    #[allow(dead_code)]
    pub year: i32,
    pub monthly_summaries: Vec<MonthlyIrpfSummary>,
    pub annual_total_sales: Decimal,
    pub annual_total_profit: Decimal,
    pub annual_total_loss: Decimal,
    pub annual_total_tax: Decimal,
    pub previous_losses_carry_forward: HashMap<TaxCategory, Decimal>,
    pub losses_to_carry_forward: HashMap<TaxCategory, Decimal>,
}

/// Progress events emitted while generating an annual report.
#[derive(Debug, Clone)]
pub enum ReportProgress {
    Start {
        target_year: i32,
        #[allow(dead_code)]
        earliest_year: i32,
        #[allow(dead_code)]
        snapshot_years: usize,
    },
    SnapshotHit {
        #[allow(dead_code)]
        year: i32,
    },
    SnapshotMiss {
        #[allow(dead_code)]
        year: i32,
    },
    SnapshotStale {
        #[allow(dead_code)]
        year: i32,
    },
    TargetCacheHit {
        year: i32,
    },
    RecomputeStart {
        from_year: i32,
    },
    RecomputedYear {
        year: i32,
    },
}

fn compute_annual_report_with_carry(
    conn: &Connection,
    year: i32,
    starting_carry: HashMap<TaxCategory, Decimal>,
    record_losses: bool,
) -> Result<(AnnualTaxReport, HashMap<TaxCategory, Decimal>)> {
    let mut monthly_summaries = Vec::new();
    let mut annual_total_sales = Decimal::ZERO;
    let mut annual_total_profit = Decimal::ZERO;
    let mut annual_total_loss = Decimal::ZERO;
    let mut annual_total_tax = Decimal::ZERO;

    let mut carryforward = starting_carry.clone();

    // Process each month
    for month in 1..=12 {
        let month_calculations = calculate_monthly_tax(conn, year, month, &mut carryforward)?;

        if month_calculations.is_empty() {
            continue;
        }

        let mut month_sales = Decimal::ZERO;
        let mut month_profit = Decimal::ZERO;
        let mut month_loss = Decimal::ZERO;
        let mut month_loss_offset = Decimal::ZERO;
        let mut month_tax = Decimal::ZERO;
        let mut by_category: HashMap<TaxCategory, CategoryMonthSummary> = HashMap::new();

        for calc in month_calculations {
            month_sales += calc.total_sales;
            month_tax += calc.tax_due;
            month_loss_offset += calc.loss_offset_applied;

            let net_pl = calc.net_profit;
            if net_pl > Decimal::ZERO {
                month_profit += net_pl;
            } else {
                month_loss += net_pl.abs();
            }

            // Record loss to ledger only during recomputation, not on cache hits
            if record_losses && calc.total_loss > Decimal::ZERO {
                record_loss(conn, year, month, &calc.category, calc.total_loss)?;
            }

            by_category.insert(
                calc.category.clone(),
                CategoryMonthSummary {
                    sales: calc.total_sales,
                    profit_loss: calc.net_profit,
                    loss_offset_applied: calc.loss_offset_applied,
                    exemption_applied: calc.exemption_applied,
                    tax_due: calc.tax_due,
                },
            );
        }

        annual_total_sales += month_sales;
        annual_total_profit += month_profit;
        annual_total_loss += month_loss;
        annual_total_tax += month_tax;

        monthly_summaries.push(MonthlyIrpfSummary {
            month_name: get_month_name(month),
            total_sales: month_sales,
            total_profit: month_profit,
            total_loss: month_loss,
            total_loss_offset_applied: month_loss_offset,
            tax_due: month_tax,
            by_category,
        });
    }

    let ending_carry = carryforward
        .iter()
        .filter_map(|(k, v)| {
            if v.is_zero() {
                None
            } else {
                Some((k.clone(), *v))
            }
        })
        .collect::<HashMap<_, _>>();

    let report = AnnualTaxReport {
        year,
        monthly_summaries,
        annual_total_sales,
        annual_total_profit,
        annual_total_loss,
        annual_total_tax,
        previous_losses_carry_forward: starting_carry
            .iter()
            .filter_map(|(k, v)| {
                if v.is_zero() {
                    None
                } else {
                    Some((k.clone(), *v))
                }
            })
            .collect(),
        losses_to_carry_forward: ending_carry.clone(),
    };

    Ok((report, ending_carry))
}

/// Generate annual IRPF report for a year (deterministic, snapshot-aware)
#[allow(dead_code)]
pub fn generate_annual_report(conn: &Connection, year: i32) -> Result<AnnualTaxReport> {
    generate_annual_report_with_progress(conn, year, |_| {})
}

/// Generate annual IRPF report for a year with progress callbacks.
pub fn generate_annual_report_with_progress<P>(
    conn: &Connection,
    year: i32,
    mut progress: P,
) -> Result<AnnualTaxReport>
where
    P: FnMut(ReportProgress),
{
    let earliest_year = earliest_transaction_year(conn)?.unwrap_or(year);

    let snapshots = load_snapshots(conn)?;

    debug!(
        target_year = year,
        earliest_year,
        snapshot_years = snapshots.len(),
        "Starting IRPF report generation with snapshot cache"
    );
    progress(ReportProgress::Start {
        target_year: year,
        earliest_year,
        snapshot_years: snapshots.len(),
    });

    // Determine starting carry from the latest valid snapshot before or at target year
    // and find the earliest year that needs recomputation.
    let mut recompute_start = earliest_year;
    let mut carry: HashMap<TaxCategory, Decimal> = HashMap::new();
    let mut carry_before_target: HashMap<TaxCategory, Decimal> = HashMap::new();
    let mut target_snapshot_valid = false;

    // Find the latest consecutive snapshot chain with matching fingerprints before target
    for y in earliest_year..=year {
        let fingerprint = compute_year_fingerprint(conn, y)?;
        if let Some(snapshot) = snapshots.get(&y) {
            let match_fingerprint = snapshot.tx_fingerprint == fingerprint;
            debug!(
                target_year = year,
                snapshot_year = y,
                match_fingerprint,
                "Snapshot lookup"
            );
            if match_fingerprint {
                progress(ReportProgress::SnapshotHit { year: y });
                if y == year {
                    // For the target year, we want the starting carry BEFORE applying this snapshot
                    carry_before_target = carry.clone();
                    target_snapshot_valid = true;
                }
                // snapshot matches; we can use its ending carry for next year
                carry = snapshot.ending_carry.clone();
                recompute_start = y + 1;
                continue;
            } else {
                // Snapshot exists but is stale; still honor its carry so imported IRPF losses are not lost
                progress(ReportProgress::SnapshotStale { year: y });
                carry = snapshot.ending_carry.clone();
                recompute_start = y;
                break;
            }
        } else {
            debug!(target_year = year, snapshot_year = y, "Snapshot miss");
            progress(ReportProgress::SnapshotMiss { year: y });
            // missing snapshot: recompute from here with whatever carry we have
            progress(ReportProgress::SnapshotStale { year: y });
            recompute_start = y;
            break;
        }
    }

    if target_snapshot_valid {
        debug!(
            target_year = year,
            "Using cached carry for target year; skipping recomputation"
        );
        progress(ReportProgress::TargetCacheHit { year });
        let (report, _) = compute_annual_report_with_carry(conn, year, carry_before_target, false)?;
        return Ok(report);
    }

    // Recompute from recompute_start through target year
    debug!(
        target_year = year,
        recompute_start, "Recomputing carry snapshots"
    );
    progress(ReportProgress::RecomputeStart {
        from_year: recompute_start,
    });

    // Clear loss ledger for all years being recomputed to avoid stale data
    for y in recompute_start..=year {
        clear_year_losses(conn, y)?;
    }

    let mut last_report = None;
    for y in recompute_start..=year {
        let fingerprint = compute_year_fingerprint(conn, y)?;
        let (report, ending_carry) =
            compute_annual_report_with_carry(conn, y, carry.clone(), true)?;
        upsert_snapshot(conn, y, &fingerprint, &ending_carry)?;
        debug!(
            target_year = year,
            recomputed_year = y,
            "Recomputed year and updated snapshot"
        );
        progress(ReportProgress::RecomputedYear { year: y });
        carry = ending_carry;
        if y == year {
            last_report = Some(report);
        }
    }

    // If no recomputation was needed (all snapshots valid and target year < recompute_start)
    if recompute_start > year {
        // The carry at this point is the ending carry of the last snapshot at or before target
        // We still need to compute the target year report using that carry as starting point
        let (report, _) = compute_annual_report_with_carry(conn, year, carry, false)?;
        return Ok(report);
    }

    last_report.ok_or_else(|| anyhow::anyhow!("Failed to compute annual report for {year}"))
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
    csv.push_str("Mês,Vendas Totais,Lucro,Prejuízo,Prejuízo Compensado,Imposto Devido\n");

    // Monthly rows
    for summary in &report.monthly_summaries {
        csv.push_str(&format!(
            "{},{:.2},{:.2},{:.2},{:.2},{:.2}\n",
            summary.month_name,
            summary.total_sales,
            summary.total_profit,
            summary.total_loss,
            summary.total_loss_offset_applied,
            summary.tax_due
        ));
    }

    // Total row
    let total_loss_offset: Decimal = report
        .monthly_summaries
        .iter()
        .map(|s| s.total_loss_offset_applied)
        .sum();
    csv.push_str(&format!(
        "\nTOTAL ANUAL,{:.2},{:.2},{:.2},{:.2},{:.2}\n",
        report.annual_total_sales,
        report.annual_total_profit,
        report.annual_total_loss,
        total_loss_offset,
        report.annual_total_tax
    ));

    // Losses to carry forward
    if !report.losses_to_carry_forward.is_empty() {
        csv.push_str("\nPREJUÍZOS A COMPENSAR\n");
        csv.push_str("Categoria,Prejuízo\n");
        for (category, loss) in &report.losses_to_carry_forward {
            let category_name = match category {
                TaxCategory::StockSwingTrade => "Ações (Swing Trade)",
                TaxCategory::StockDayTrade => "Ações (Day Trade)",
                TaxCategory::FiiSwingTrade => "FII (Swing Trade)",
                TaxCategory::FiiDayTrade => "FII (Day Trade)",
                TaxCategory::FiagroSwingTrade => "FIAGRO (Swing Trade)",
                TaxCategory::FiagroDayTrade => "FIAGRO (Day Trade)",
                TaxCategory::FiInfra => "FI-Infra (Isento)",
            };
            csv.push_str(&format!("{},{:.2}\n", category_name, loss));
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
            previous_losses_carry_forward: HashMap::new(),
            losses_to_carry_forward: HashMap::new(),
        };

        let csv = export_to_csv(&report);
        assert!(csv.contains("TOTAL ANUAL"));
        assert!(csv.contains("100000.00"));
    }
}
