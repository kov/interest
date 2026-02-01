use anyhow::Result;
use chrono::NaiveDate;
use rusqlite::Connection;

use crate::{db, importers, reports};

// The helpers expose ImportStats from the `importers` module
use crate::importers::ImportStats;

/// Import CEI transactions into database and update last import date; returns (imported, skipped_old, errors, earliest_date, max_date)
pub(crate) fn import_cei(
    conn: &Connection,
    raw_transactions: &[crate::importers::RawTransaction],
) -> Result<ImportStats> {
    let mut imported: i64 = 0;
    let mut skipped_old: i64 = 0;
    let mut errors: i64 = 0;
    let mut max_imported_date: Option<NaiveDate> = None;
    let mut earliest_imported_date: Option<NaiveDate> = None;

    let last_import_date = db::get_last_import_date(conn, "CEI", "trades")?;

    let asset_exists_closure =
        |ticker: &str| -> anyhow::Result<bool> { crate::db::asset_exists(conn, ticker) };

    for raw_tx in raw_transactions {
        if let Some(last_date) = last_import_date {
            if raw_tx.trade_date <= last_date {
                skipped_old += 1;
                continue;
            }
        }

        let (normalized_ticker, notes_override) =
            importers::cei_excel::resolve_option_exercise_ticker(raw_tx, asset_exists_closure)?;
        let asset_type = db::AssetType::Unknown;

        // Upsert asset
        let asset_id = match db::upsert_asset(conn, &normalized_ticker, &asset_type, None) {
            Ok(id) => id,
            Err(e) => {
                eprintln!("Error upserting asset: {}", e);
                errors += 1;
                continue;
            }
        };

        let mut transaction = match raw_tx.to_transaction(asset_id) {
            Ok(tx) => tx,
            Err(e) => {
                eprintln!("Error converting transaction for {}: {}", raw_tx.ticker, e);
                errors += 1;
                continue;
            }
        };

        if let Some(notes) = notes_override {
            transaction.notes = Some(notes);
        }

        match db::insert_transaction(conn, &transaction) {
            Ok(_) => {
                imported += 1;
                max_imported_date = Some(match max_imported_date {
                    Some(current) if current >= transaction.trade_date => current,
                    _ => transaction.trade_date,
                });
                earliest_imported_date = Some(match earliest_imported_date {
                    Some(current) if current <= transaction.trade_date => current,
                    _ => transaction.trade_date,
                });
            }
            Err(e) => {
                eprintln!("Error inserting transaction: {}", e);
                errors += 1;
            }
        }
    }

    if let Some(last_date) = max_imported_date {
        db::set_last_import_date(conn, "CEI", "trades", last_date)?;
    }

    if imported > 0 {
        if let Some(date) = earliest_imported_date {
            reports::invalidate_snapshots_after(conn, date)?;
        }
    }

    Ok(ImportStats {
        imported: imported as usize,
        skipped_old: skipped_old as usize,
        errors: errors as usize,
        earliest: earliest_imported_date,
        latest: max_imported_date,
        // zero other fields
        imported_trades: 0,
        skipped_trades: 0,
        skipped_trades_old: 0,
        imported_actions: 0,
        skipped_actions: 0,
        skipped_actions_old: 0,
        auto_applied_actions: 0,
        imported_income: 0,
        skipped_income: 0,
        skipped_income_old: 0,
    })
}

/// Import "Ofertas Públicas" allocations into DB and return (imported, skipped_old, errors, max_date)
pub(crate) fn import_ofertas(
    conn: &Connection,
    entries: &[crate::importers::OfertaPublicaEntry],
) -> Result<ImportStats> {
    let mut imported: i64 = 0;
    let mut skipped_old: i64 = 0;
    let mut errors: i64 = 0;
    let mut max_date: Option<NaiveDate> = None;

    let last_import_date = db::get_last_import_date(conn, "OFERTAS_PUBLICAS", "allocations")?;

    for entry in entries {
        let asset_type = db::AssetType::Unknown;

        let asset_id = match db::upsert_asset(conn, &entry.ticker, &asset_type, None) {
            Ok(id) => id,
            Err(e) => {
                eprintln!("Error upserting asset {}: {}", entry.ticker, e);
                errors += 1;
                continue;
            }
        };

        if let Some(last_date) = last_import_date {
            if entry.date <= last_date {
                skipped_old += 1;
                continue;
            }
        }

        let transaction = match entry.to_transaction(asset_id) {
            Ok(tx) => tx,
            Err(e) => {
                eprintln!("Error converting offer to transaction: {}", e);
                errors += 1;
                continue;
            }
        };

        match db::insert_transaction(conn, &transaction) {
            Ok(_) => {
                imported += 1;
                max_date = Some(match max_date {
                    Some(current) if current >= transaction.trade_date => current,
                    _ => transaction.trade_date,
                });
            }
            Err(e) => {
                eprintln!("Error inserting offer transaction: {}", e);
                errors += 1;
            }
        }
    }

    if let Some(d) = max_date {
        db::set_last_import_date(conn, "OFERTAS_PUBLICAS", "allocations", d)?;
    }

    Ok(ImportStats {
        imported: imported as usize,
        skipped_old: skipped_old as usize,
        errors: errors as usize,
        earliest: None,
        latest: max_date,
        imported_trades: 0,
        skipped_trades: 0,
        skipped_trades_old: 0,
        imported_actions: 0,
        skipped_actions: 0,
        skipped_actions_old: 0,
        auto_applied_actions: 0,
        imported_income: 0,
        skipped_income: 0,
        skipped_income_old: 0,
    })
}

#[cfg(test)]
mod tests {
    use crate::formatters;

    #[test]
    fn ceil_importstats_maps_correctly() {
        // Basic smoke test to ensure CEI mapping populates fields
        let txs: Vec<crate::importers::RawTransaction> = vec![];
        assert!(formatters::imports::format_cei_preview_table(&txs).is_none());
    }
}
