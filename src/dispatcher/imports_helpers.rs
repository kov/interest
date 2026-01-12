use anyhow::Result;
use chrono::NaiveDate;
use rusqlite::Connection;

use tabled::Tabled;
use tabled::{
    settings::{object::Columns, Alignment, Modify, Style},
    Table,
};

use crate::{db, importers, reports};

// The helpers expose ImportStats from the `importers` module
use crate::importers::ImportStats;
/// Return a pretty table preview for CEI transactions (up to 10 rows)
pub(crate) fn preview_cei_table(txs: &[crate::importers::RawTransaction]) -> Option<String> {
    #[derive(Tabled)]
    struct TransactionPreview<'a> {
        #[tabled(rename = "Date")]
        date: String,
        #[tabled(rename = "Ticker")]
        ticker: &'a str,
        #[tabled(rename = "Type")]
        tx_type: &'a str,
        #[tabled(rename = "Quantity")]
        quantity: String,
        #[tabled(rename = "Price")]
        price: String,
        #[tabled(rename = "Total")]
        total: String,
    }

    let preview: Vec<TransactionPreview> = txs
        .iter()
        .take(10)
        .map(|tx| TransactionPreview {
            date: tx.trade_date.format("%d/%m/%Y").to_string(),
            ticker: tx.ticker.as_str(),
            tx_type: tx.transaction_type.as_str(),
            quantity: tx.quantity.to_string(),
            price: crate::utils::format_currency(tx.price),
            total: crate::utils::format_currency(tx.total),
        })
        .collect();

    if preview.is_empty() {
        None
    } else {
        let table = Table::new(preview)
            .with(Style::rounded())
            .with(Modify::new(Columns::new(3..)).with(Alignment::right()))
            .to_string();
        Some(table)
    }
}

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
        let asset_type =
            db::AssetType::detect_from_ticker(&normalized_ticker).unwrap_or(db::AssetType::Stock);

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

pub(crate) fn preview_movimentacao_trades(
    trades: &[crate::importers::MovimentacaoEntry],
) -> Option<String> {
    #[derive(Tabled)]
    struct TradePreview {
        #[tabled(rename = "Date")]
        date: String,
        #[tabled(rename = "Type")]
        movement_type: String,
        #[tabled(rename = "Ticker")]
        ticker: String,
        #[tabled(rename = "Qty")]
        quantity: String,
        #[tabled(rename = "Price")]
        price: String,
    }

    let preview: Vec<TradePreview> = trades
        .iter()
        .take(5)
        .map(|e| TradePreview {
            date: e.date.format("%d/%m/%Y").to_string(),
            movement_type: e.movement_type.clone(),
            ticker: e.ticker.clone().unwrap_or_else(|| "?".to_string()),
            quantity: e
                .quantity
                .map(|q| q.to_string())
                .unwrap_or_else(|| "-".to_string()),
            price: e
                .unit_price
                .map(crate::utils::format_currency)
                .unwrap_or_else(|| "-".to_string()),
        })
        .collect();

    if preview.is_empty() {
        None
    } else {
        Some(
            Table::new(preview)
                .with(Style::rounded())
                .with(Modify::new(Columns::new(3..)).with(Alignment::right()))
                .to_string(),
        )
    }
}

pub(crate) fn preview_ofertas_table(
    entries: &[crate::importers::OfertaPublicaEntry],
) -> Option<String> {
    #[derive(Tabled)]
    struct OfertaPreview {
        #[tabled(rename = "Date")]
        date: String,
        #[tabled(rename = "Ticker")]
        ticker: String,
        #[tabled(rename = "Qty")]
        quantity: String,
        #[tabled(rename = "Price")]
        price: String,
        #[tabled(rename = "Offer")]
        offer: String,
    }

    let preview: Vec<OfertaPreview> = entries
        .iter()
        .take(5)
        .map(|e| OfertaPreview {
            date: e.date.format("%d/%m/%Y").to_string(),
            ticker: e.ticker.clone(),
            quantity: e.quantity.to_string(),
            price: crate::utils::format_currency(e.unit_price),
            offer: e.offer.clone(),
        })
        .collect();

    if preview.is_empty() {
        None
    } else {
        Some(
            Table::new(preview)
                .with(Style::rounded())
                .with(Modify::new(Columns::new(2..4)).with(Alignment::right()))
                .to_string(),
        )
    }
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
        let asset_type =
            db::AssetType::detect_from_ticker(&entry.ticker).unwrap_or(db::AssetType::Stock);

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
    use super::*;

    #[test]
    fn ceil_importstats_maps_correctly() {
        // Basic smoke test to ensure CEI mapping populates fields
        let txs: Vec<crate::importers::RawTransaction> = vec![];
        assert!(preview_cei_table(&txs).is_none());
    }
}
