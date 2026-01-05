use anyhow::Result;
use rusqlite::Connection;
use rust_decimal::Decimal;
use tracing::{info, warn};

use crate::corporate_actions;
use crate::db;
use crate::importers::MovimentacaoEntry;

#[derive(Debug, Clone, Copy)]
pub struct ImportStats {
    pub imported_trades: usize,
    pub skipped_trades: usize,
    pub skipped_trades_old: usize,
    pub imported_actions: usize,
    pub skipped_actions: usize,
    pub skipped_actions_old: usize,
    pub auto_applied_actions: usize,
    pub errors: usize,
}

pub fn import_movimentacao_entries(
    conn: &Connection,
    entries: Vec<MovimentacaoEntry>,
    track_state: bool,
) -> Result<ImportStats> {
    let trades: Vec<_> = entries.iter().filter(|e| e.is_trade()).collect();
    let mut actions: Vec<_> = entries.iter().filter(|e| e.is_corporate_action()).collect();
    actions.sort_by_key(|e| e.date);

    let mut imported_trades = 0;
    let mut skipped_trades = 0;
    let mut skipped_trades_old = 0;
    let mut errors = 0;
    let mut max_trade_date: Option<chrono::NaiveDate> = None;

    let last_trade_date = if track_state {
        db::get_last_import_date(conn, "MOVIMENTACAO", "trades")?
    } else {
        None
    };

    for entry in trades {
        if entry.ticker.is_none() {
            warn!("Skipping trade with no ticker: {:?}", entry.product);
            skipped_trades += 1;
            continue;
        }

        let ticker = entry.ticker.as_ref().unwrap();
        let asset_type = db::AssetType::detect_from_ticker(ticker).unwrap_or(db::AssetType::Stock);
        let asset_id = match db::upsert_asset(conn, ticker, &asset_type, None) {
            Ok(id) => id,
            Err(e) => {
                warn!("Error upserting asset {}: {}", ticker, e);
                errors += 1;
                continue;
            }
        };

        let transaction = match entry.to_transaction(asset_id) {
            Ok(tx) => tx,
            Err(e) => {
                warn!("Failed to convert entry to transaction: {}", e);
                errors += 1;
                continue;
            }
        };
        if let Some(last_date) = last_trade_date {
            if transaction.trade_date <= last_date {
                skipped_trades_old += 1;
                continue;
            }
        }

        match db::insert_transaction(conn, &transaction) {
            Ok(_) => {
                imported_trades += 1;
                max_trade_date = Some(match max_trade_date {
                    Some(current) if current >= transaction.trade_date => current,
                    _ => transaction.trade_date,
                });
            }
            Err(e) => {
                warn!("Error inserting transaction: {}", e);
                errors += 1;
            }
        }
    }

    if track_state {
        if let Some(last_date) = max_trade_date {
            db::set_last_import_date(conn, "MOVIMENTACAO", "trades", last_date)?;
        }
    }

    let mut imported_actions = 0;
    let mut skipped_actions = 0;
    let mut skipped_actions_old = 0;
    let mut auto_applied_actions = 0;
    let mut max_action_date: Option<chrono::NaiveDate> = None;

    let last_action_date = if track_state {
        db::get_last_import_date(conn, "MOVIMENTACAO", "corporate_actions")?
    } else {
        None
    };

    for entry in actions {
        if entry.ticker.is_none() {
            warn!("Skipping corporate action with no ticker: {:?}", entry.product);
            skipped_actions += 1;
            continue;
        }

        let ticker = entry.ticker.as_ref().unwrap();
        let asset_type = db::AssetType::detect_from_ticker(ticker).unwrap_or(db::AssetType::Stock);
        let asset_id = match db::upsert_asset(conn, ticker, &asset_type, None) {
            Ok(id) => id,
            Err(e) => {
                warn!("Error upserting asset {}: {}", ticker, e);
                errors += 1;
                continue;
            }
        };

        if entry.movement_type == "Atualização" {
            if let Some(qty) = entry.quantity {
                if qty > Decimal::ZERO {
                    let bonus_tx = db::Transaction {
                        id: None,
                        asset_id,
                        transaction_type: db::TransactionType::Buy,
                        trade_date: entry.date,
                        settlement_date: Some(entry.date),
                        quantity: qty,
                        price_per_unit: Decimal::ZERO,
                        total_cost: Decimal::ZERO,
                        fees: Decimal::ZERO,
                        is_day_trade: false,
                        quota_issuance_date: None,
                        notes: Some(format!("Bonus shares from Atualização ({})", entry.product)),
                        source: "MOVIMENTACAO".to_string(),
                        created_at: chrono::Utc::now(),
                    };
                    match db::insert_transaction(conn, &bonus_tx) {
                        Ok(_) => {
                            imported_trades += 1;
                        }
                        Err(e) => {
                            warn!("Error inserting Atualização bonus transaction: {}", e);
                            errors += 1;
                        }
                    }
                }
            }
            continue;
        }

        let mut action = match entry.to_corporate_action(asset_id) {
            Ok(a) => a,
            Err(e) => {
                warn!("Failed to convert entry to corporate action: {}", e);
                errors += 1;
                continue;
            }
        };

        if entry.movement_type == "Desdobro" && action.ratio_from == 1 && action.ratio_to == 1 {
            if let Some((ratio_from, ratio_to)) =
                infer_split_ratio_from_credit(conn, asset_id, entry)?
            {
                action.ratio_from = ratio_from;
                action.ratio_to = ratio_to;
                let note_suffix = format!("inferred ratio {}:{}", ratio_from, ratio_to);
                action.notes = Some(match action.notes.take() {
                    Some(existing) if !existing.is_empty() => {
                        format!("{} | {}", existing, note_suffix)
                    }
                    _ => note_suffix,
                });
            }
        }
        if entry.movement_type == "Desdobro" && action.ratio_from == 1 && action.ratio_to == 1 {
            warn!(
                "Desdobro ratio unknown for {} on {}; defaulting to 1:1",
                ticker,
                entry.date
            );
        }

        if let Some(last_date) = last_action_date {
            if action.event_date <= last_date {
                skipped_actions_old += 1;
                continue;
            }
        }

        let action_id = match db::insert_corporate_action(conn, &action) {
            Ok(id) => id,
            Err(e) => {
                warn!("Error inserting corporate action: {}", e);
                errors += 1;
                continue;
            }
        };
        action.id = Some(action_id);
        imported_actions += 1;
        max_action_date = Some(match max_action_date {
            Some(current) if current >= action.event_date => current,
            _ => action.event_date,
        });

        if action.ratio_from != action.ratio_to {
            let asset = db::Asset {
                id: Some(asset_id),
                ticker: ticker.to_string(),
                asset_type,
                name: None,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            };
            match corporate_actions::apply_corporate_action(conn, &action, &asset) {
                Ok(adjusted) => {
                    if adjusted > 0 {
                        auto_applied_actions += 1;
                    }
                }
                Err(e) => {
                    warn!(
                        "Failed to auto-apply corporate action for {} on {}: {}",
                        ticker,
                        action.event_date,
                        e
                    );
                    errors += 1;
                }
            }
        } else {
            info!(
                "Skipping auto-apply for {} on {} (ratio 1:1)",
                ticker,
                action.event_date
            );
        }
    }

    if track_state {
        if let Some(last_date) = max_action_date {
            db::set_last_import_date(conn, "MOVIMENTACAO", "corporate_actions", last_date)?;
        }
    }

    Ok(ImportStats {
        imported_trades,
        skipped_trades,
        skipped_trades_old,
        imported_actions,
        skipped_actions,
        skipped_actions_old,
        auto_applied_actions,
        errors,
    })
}

fn infer_split_ratio_from_credit(
    conn: &Connection,
    asset_id: i64,
    entry: &MovimentacaoEntry,
) -> Result<Option<(i32, i32)>> {
    use rust_decimal::prelude::ToPrimitive;

    if entry.movement_type != "Desdobro" {
        return Ok(None);
    }

    let credit_qty = match entry.quantity {
        Some(qty) if qty > Decimal::ZERO => qty,
        _ => return Ok(None),
    };

    let mut old_qty = db::get_asset_position_before_date(conn, asset_id, entry.date)?;
    if let Some(ticker) = entry.ticker.as_deref() {
        let carryover = rename_carryover_quantity(conn, ticker, entry.date)?;
        if carryover > Decimal::ZERO {
            old_qty += carryover;
        }
    }
    if old_qty <= Decimal::ZERO {
        warn!(
            "Cannot infer split ratio for {} on {}: position before date is {}",
            entry.ticker.as_deref().unwrap_or("?"),
            entry.date,
            old_qty
        );
        return Ok(None);
    }

    let ratio_from_increment = (old_qty + credit_qty) / old_qty;
    let ratio_from_total = credit_qty / old_qty;

    let ratio_increment_i32 = ratio_from_increment.to_i32().and_then(|r| {
        if r > 1 && Decimal::from(r) == ratio_from_increment {
            Some(r)
        } else {
            None
        }
    });

    let ratio_total_i32 = ratio_from_total.to_i32().and_then(|r| {
        if r > 1 && Decimal::from(r) == ratio_from_total {
            Some(r)
        } else {
            None
        }
    });

    let selected_ratio = match (ratio_increment_i32, ratio_total_i32) {
        (Some(increment), _) => Some(increment),
        (None, Some(total)) => Some(total),
        _ => None,
    };

    if selected_ratio.is_none() {
        warn!(
            "Cannot infer split ratio for {} on {}: computed ratios {} and {}",
            entry.ticker.as_deref().unwrap_or("?"),
            entry.date,
            ratio_from_increment,
            ratio_from_total
        );
        return Ok(None);
    }

    Ok(Some((1, selected_ratio.unwrap())))
}

fn rename_carryover_quantity(
    conn: &Connection,
    target_ticker: &str,
    before_date: chrono::NaiveDate,
) -> Result<Decimal> {
    use rusqlite::OptionalExtension;

    let mut total = Decimal::ZERO;

    for (source_ticker, effective_date) in db::rename_sources_for(target_ticker) {
        if effective_date >= before_date {
            continue;
        }

        let source_id: Option<i64> = conn
            .query_row(
                "SELECT id FROM assets WHERE ticker = ?1",
                [source_ticker],
                |row| row.get(0),
            )
            .optional()?;

        if let Some(id) = source_id {
            let source_qty = db::get_asset_position_before_date(conn, id, effective_date)?;
            if source_qty > Decimal::ZERO {
                total += source_qty;
            }
        }
    }

    Ok(total)
}
