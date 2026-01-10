use anyhow::Result;
use rusqlite::Connection;
use rust_decimal::Decimal;
use serde::Serialize;
use std::collections::HashMap;
use tracing::{info, warn};

use crate::corporate_actions;
use crate::db;
use crate::importers::MovimentacaoEntry;
use serde_json::json;

#[derive(Debug, Clone, Copy, Serialize)]
pub struct ImportStats {
    pub imported_trades: usize,
    pub skipped_trades: usize,
    pub skipped_trades_old: usize,
    pub imported_actions: usize,
    pub skipped_actions: usize,
    pub skipped_actions_old: usize,
    #[allow(dead_code)]
    pub auto_applied_actions: usize,
    pub imported_income: usize,
    pub skipped_income: usize,
    pub skipped_income_old: usize,
    pub errors: usize,
}

pub fn import_movimentacao_entries(
    conn: &Connection,
    entries: Vec<MovimentacaoEntry>,
    track_state: bool,
) -> Result<ImportStats> {
    let receipt_index = build_subscription_receipts_index(&entries);
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
            warn!(
                "Skipping corporate action with no ticker: {:?}",
                entry.product
            );
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

        if entry.movement_type == "Bonificação em Ativos" {
            use rust_decimal::RoundingStrategy;

            let qty = match entry.quantity {
                Some(qty) if qty > Decimal::ZERO => qty,
                _ => {
                    skipped_actions += 1;
                    continue;
                }
            };
            if let Some(last_date) = last_action_date {
                if entry.date <= last_date {
                    skipped_actions_old += 1;
                    continue;
                }
            }

            let integer_qty = qty.round_dp_with_strategy(0, RoundingStrategy::ToZero);
            let fractional_qty = qty - integer_qty;
            if integer_qty > Decimal::ZERO {
                let mut notes = format!(
                    "Bonificação em Ativos credit from movimentacao: {}",
                    entry.product
                );
                if fractional_qty > Decimal::ZERO {
                    notes = format!("{}; fractional remainder: {}", notes, fractional_qty);
                }
                let bonus_tx = db::Transaction {
                    id: None,
                    asset_id,
                    transaction_type: db::TransactionType::Buy,
                    trade_date: entry.date,
                    settlement_date: Some(entry.date),
                    quantity: integer_qty,
                    price_per_unit: Decimal::ZERO,
                    total_cost: Decimal::ZERO,
                    fees: Decimal::ZERO,
                    is_day_trade: false,
                    quota_issuance_date: None,
                    notes: Some(notes),
                    source: "MOVIMENTACAO".to_string(),
                    created_at: chrono::Utc::now(),
                };
                match db::insert_transaction(conn, &bonus_tx) {
                    Ok(_) => {
                        imported_actions += 1;
                        max_action_date = Some(match max_action_date {
                            Some(current) if current >= entry.date => current,
                            _ => entry.date,
                        });
                    }
                    Err(e) => {
                        warn!("Error inserting Bonificação em Ativos transaction: {}", e);
                        errors += 1;
                    }
                }
            } else {
                skipped_actions += 1;
            }
            continue;
        }

        if entry.movement_type == "Desdobro" {
            let qty = match entry.quantity {
                Some(qty) if qty > Decimal::ZERO => qty,
                _ => {
                    skipped_actions += 1;
                    continue;
                }
            };
            if let Some(last_date) = last_action_date {
                if entry.date <= last_date {
                    skipped_actions_old += 1;
                    continue;
                }
            }

            if let Some((ratio_from, ratio_to)) =
                infer_split_ratio_from_credit(conn, asset_id, entry, qty)?
            {
                let mut action = match entry.to_corporate_action(asset_id) {
                    Ok(a) => a,
                    Err(e) => {
                        warn!("Failed to convert entry to corporate action: {}", e);
                        errors += 1;
                        continue;
                    }
                };
                action.ratio_from = ratio_from;
                action.ratio_to = ratio_to;
                let note_suffix = format!("inferred ratio {}:{}", ratio_from, ratio_to);
                action.notes = Some(match action.notes.take() {
                    Some(existing) if !existing.is_empty() => {
                        format!("{} | {}", existing, note_suffix)
                    }
                    _ => note_suffix,
                });

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
                            ticker, action.event_date, e
                        );
                        errors += 1;
                    }
                }
            } else {
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
                    notes: Some(format!(
                        "Desdobro credit from movimentacao: {}",
                        entry.product
                    )),
                    source: "MOVIMENTACAO".to_string(),
                    created_at: chrono::Utc::now(),
                };
                match db::insert_transaction(conn, &bonus_tx) {
                    Ok(_) => {
                        imported_actions += 1;
                        max_action_date = Some(match max_action_date {
                            Some(current) if current >= entry.date => current,
                            _ => entry.date,
                        });
                    }
                    Err(e) => {
                        warn!("Error inserting Desdobro transaction: {}", e);
                        errors += 1;
                    }
                }
            }
            continue;
        }

        if entry.movement_type == "Atualização" {
            let qty = match entry.quantity {
                Some(qty) if qty > Decimal::ZERO => qty,
                _ => {
                    skipped_actions += 1;
                    continue;
                }
            };

            let receipt_match = match_subscription_receipt(&receipt_index, entry, qty);
            if let Some(receipt_match) = receipt_match {
                let notes = format!(
                    "Subscription receipt conversion from {} ({})",
                    receipt_match.tickers.join(", "),
                    entry.product
                );
                let issue = db::Inconsistency {
                    id: None,
                    issue_type: db::InconsistencyType::MissingCostBasis,
                    status: db::InconsistencyStatus::Open,
                    severity: db::InconsistencySeverity::Blocking,
                    asset_id: Some(asset_id),
                    transaction_id: None,
                    ticker: Some(ticker.to_string()),
                    trade_date: Some(entry.date),
                    quantity: Some(qty),
                    source: Some("MOVIMENTACAO".to_string()),
                    source_ref: None,
                    missing_fields_json: Some(
                        json!({
                            "price_per_unit": null,
                            "total_cost": null,
                            "fees": null
                        })
                        .to_string(),
                    ),
                    context_json: Some(
                        json!({
                            "notes": notes,
                            "movement_type": entry.movement_type,
                            "product": entry.product,
                            "receipt_tickers": receipt_match.tickers
                        })
                        .to_string(),
                    ),
                    resolution_action: None,
                    resolution_json: None,
                    created_at: None,
                    resolved_at: None,
                };
                match db::insert_inconsistency(conn, &issue) {
                    Ok(_) => {
                        skipped_actions += 1;
                        max_action_date = Some(match max_action_date {
                            Some(current) if current >= entry.date => current,
                            _ => entry.date,
                        });
                    }
                    Err(e) => {
                        warn!("Error inserting Atualização inconsistency: {}", e);
                        errors += 1;
                    }
                }
            } else {
                skipped_actions += 1;
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
                        ticker, action.event_date, e
                    );
                    errors += 1;
                }
            }
        } else {
            info!(
                "Skipping auto-apply for {} on {} (ratio 1:1)",
                ticker, action.event_date
            );
        }
    }

    if track_state {
        if let Some(last_date) = max_action_date {
            db::set_last_import_date(conn, "MOVIMENTACAO", "corporate_actions", last_date)?;
        }
    }

    // Process income events
    let income_events: Vec<_> = entries.iter().filter(|e| e.is_income_event()).collect();

    let mut imported_income = 0;
    let mut skipped_income = 0;
    let mut skipped_income_old = 0;
    let mut max_income_date: Option<chrono::NaiveDate> = None;

    let last_income_date = if track_state {
        db::get_last_import_date(conn, "MOVIMENTACAO", "income")?
    } else {
        None
    };

    for entry in income_events {
        if entry.ticker.is_none() {
            warn!("Skipping income event with no ticker: {:?}", entry.product);
            skipped_income += 1;
            continue;
        }

        let ticker = entry.ticker.as_ref().unwrap();
        let asset_type = db::AssetType::detect_from_ticker(ticker).unwrap_or(db::AssetType::Stock);
        let asset_id = match db::upsert_asset(conn, ticker, &asset_type, None) {
            Ok(id) => id,
            Err(e) => {
                warn!("Error upserting asset {} for income event: {}", ticker, e);
                errors += 1;
                continue;
            }
        };

        // Skip if older than last import date
        if let Some(last_date) = last_income_date {
            if entry.date <= last_date {
                skipped_income_old += 1;
                continue;
            }
        }

        let income_event = match entry.to_income_event(asset_id) {
            Ok(ie) => ie,
            Err(e) => {
                warn!("Failed to convert entry to income event: {}", e);
                errors += 1;
                continue;
            }
        };

        // Check for duplicate (same asset, date, type, amount)
        match db::income_event_exists(
            conn,
            asset_id,
            income_event.event_date,
            &income_event.event_type,
            income_event.total_amount,
        ) {
            Ok(true) => {
                skipped_income += 1;
                continue;
            }
            Ok(false) => {}
            Err(e) => {
                warn!("Error checking for duplicate income event: {}", e);
                errors += 1;
                continue;
            }
        }

        match db::insert_income_event(conn, &income_event) {
            Ok(_) => {
                imported_income += 1;
                max_income_date = Some(match max_income_date {
                    Some(current) if current >= income_event.event_date => current,
                    _ => income_event.event_date,
                });
            }
            Err(e) => {
                warn!("Error inserting income event: {}", e);
                errors += 1;
            }
        }
    }

    if track_state {
        if let Some(last_date) = max_income_date {
            db::set_last_import_date(conn, "MOVIMENTACAO", "income", last_date)?;
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
        imported_income,
        skipped_income,
        skipped_income_old,
        errors,
    })
}

#[derive(Clone)]
struct ReceiptEntry {
    date: chrono::NaiveDate,
    quantity: Decimal,
    ticker: String,
}

struct ReceiptMatch {
    tickers: Vec<String>,
}

fn build_subscription_receipts_index(
    entries: &[MovimentacaoEntry],
) -> HashMap<String, Vec<ReceiptEntry>> {
    let mut receipts: HashMap<String, Vec<ReceiptEntry>> = HashMap::new();
    let subscription_tickers = collect_subscription_tickers(entries);

    for entry in entries {
        if entry.direction != "Credito" {
            continue;
        }
        if !is_subscription_receipt_entry(entry, &subscription_tickers) {
            continue;
        }
        let qty = match entry.quantity {
            Some(qty) if qty > Decimal::ZERO => qty,
            _ => continue,
        };
        let ticker = match entry.ticker.as_deref() {
            Some(ticker) => ticker.to_string(),
            None => continue,
        };
        let key = normalized_product_description(&entry.product);
        receipts.entry(key).or_default().push(ReceiptEntry {
            date: entry.date,
            quantity: qty,
            ticker,
        });
    }

    receipts
}

fn collect_subscription_tickers(entries: &[MovimentacaoEntry]) -> Vec<String> {
    let mut tickers = Vec::new();
    for entry in entries {
        if !is_subscription_related_movement_type(&entry.movement_type) {
            continue;
        }
        if let Some(ticker) = entry.ticker.as_deref() {
            if !tickers.iter().any(|t| t == ticker) {
                tickers.push(ticker.to_string());
            }
        }
    }
    tickers
}

fn is_subscription_related_movement_type(movement_type: &str) -> bool {
    movement_type.contains("Subscrição")
        || movement_type.contains("Direito")
        || movement_type.contains("Cessão de Direitos")
}

fn is_subscription_receipt_entry(
    entry: &MovimentacaoEntry,
    subscription_tickers: &[String],
) -> bool {
    if entry.movement_type == "Recibo de Subscrição" {
        return true;
    }
    if is_subscription_related_movement_type(&entry.movement_type) {
        return true;
    }
    if entry.movement_type == "Transferência" {
        if let Some(ticker) = entry.ticker.as_deref() {
            return subscription_tickers.iter().any(|t| t == ticker);
        }
    }
    if entry.movement_type == "Transferência - Liquidação" {
        if let Some(ticker) = entry.ticker.as_deref() {
            return is_receipt_like_ticker(ticker);
        }
    }
    false
}

fn is_receipt_like_ticker(ticker: &str) -> bool {
    let ticker = ticker.trim().to_uppercase();
    if ticker.len() < 2 {
        return false;
    }
    if ticker.ends_with('9') {
        return true;
    }
    ticker.ends_with("12")
        || ticker.ends_with("13")
        || ticker.ends_with("14")
        || ticker.ends_with("15")
}

fn match_subscription_receipt(
    receipt_index: &HashMap<String, Vec<ReceiptEntry>>,
    entry: &MovimentacaoEntry,
    qty: Decimal,
) -> Option<ReceiptMatch> {
    let key = normalized_product_description(&entry.product);
    let receipts = receipt_index.get(&key)?;
    let lookback_days = 120;

    let candidates: Vec<_> = receipts
        .iter()
        .filter(|receipt| {
            receipt.date <= entry.date && (entry.date - receipt.date).num_days() <= lookback_days
        })
        .collect();

    if candidates.is_empty() {
        return None;
    }

    if let Some(receipt) = candidates.iter().find(|receipt| receipt.quantity == qty) {
        return Some(ReceiptMatch {
            tickers: vec![receipt.ticker.clone()],
        });
    }

    let sum: Decimal = candidates.iter().map(|receipt| receipt.quantity).sum();
    if sum == qty {
        let tickers = candidates
            .iter()
            .map(|receipt| receipt.ticker.clone())
            .collect::<Vec<_>>();
        return Some(ReceiptMatch { tickers });
    }

    None
}

fn normalized_product_description(product: &str) -> String {
    let desc = product.split_once(" - ").map(|x| x.1).unwrap_or(product);
    desc.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_uppercase()
}

fn infer_split_ratio_from_credit(
    conn: &Connection,
    asset_id: i64,
    entry: &MovimentacaoEntry,
    credit_qty: Decimal,
) -> Result<Option<(i32, i32)>> {
    use rust_decimal::prelude::ToPrimitive;

    if entry.movement_type != "Desdobro" {
        return Ok(None);
    }

    let old_qty = db::get_asset_position_before_date(conn, asset_id, entry.date)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn entry(
        date: (i32, u32, u32),
        movement_type: &str,
        product: &str,
        ticker: &str,
        direction: &str,
        quantity: i64,
    ) -> MovimentacaoEntry {
        MovimentacaoEntry {
            direction: direction.to_string(),
            date: NaiveDate::from_ymd_opt(date.0, date.1, date.2).unwrap(),
            movement_type: movement_type.to_string(),
            product: product.to_string(),
            ticker: Some(ticker.to_string()),
            institution: "TEST".to_string(),
            quantity: Some(Decimal::from(quantity)),
            unit_price: None,
            operation_value: None,
        }
    }

    #[test]
    fn matches_subscription_receipt_for_update() {
        let entries = vec![
            entry(
                (2020, 7, 13),
                "Recibo de Subscrição",
                "BRCR13 - FDO INV IMOB - FII BTG PACTUAL CORPORATE OFFICE FUND",
                "BRCR13",
                "Credito",
                22,
            ),
            entry(
                (2020, 9, 14),
                "Atualização",
                "BRCR11 - FDO INV IMOB - FII BTG PACTUAL CORPORATE OFFICE FUND",
                "BRCR11",
                "Credito",
                22,
            ),
        ];

        let receipt_index = build_subscription_receipts_index(&entries);
        let update_entry = &entries[1];
        let match_result =
            match_subscription_receipt(&receipt_index, update_entry, Decimal::from(22));

        assert!(match_result.is_some());
        assert_eq!(match_result.unwrap().tickers, vec!["BRCR13".to_string()]);
    }

    #[test]
    fn matches_receipt_like_transfer_for_update() {
        let entries = vec![
            entry(
                (2023, 6, 30),
                "Transferência - Liquidação",
                "CDII12 - SPARTA INFRA CDI FIC FI INFRA RENDA FIXA CP",
                "CDII12",
                "Credito",
                304,
            ),
            entry(
                (2023, 6, 28),
                "Direito de Subscrição",
                "CDII12 - SPARTA INFRA CDI FIC FI INFRA RENDA FIXA CP",
                "CDII12",
                "Credito",
                3,
            ),
            entry(
                (2023, 7, 11),
                "Direitos de Subscrição - Exercido",
                "CDII12 - SPARTA INFRA CDI FIC FI INFRA RENDA FIXA CP",
                "CDII12",
                "Debito",
                307,
            ),
            entry(
                (2023, 8, 7),
                "Atualização",
                "CDII11 - SPARTA INFRA CDI FIC FI INFRA RENDA FIXA CP",
                "CDII11",
                "Credito",
                307,
            ),
        ];

        let receipt_index = build_subscription_receipts_index(&entries);
        let update_entry = &entries[2];
        let match_result =
            match_subscription_receipt(&receipt_index, update_entry, Decimal::from(307));

        assert!(match_result.is_some());
    }

    #[test]
    fn ignores_snapshot_updates_without_receipt_match() {
        let entries = vec![
            entry(
                (2024, 4, 12),
                "Rendimento",
                "BRCR11 - FDO INV IMOB - FII BTG PACTUAL CORP. OFFICE FUND",
                "BRCR11",
                "Credito",
                802,
            ),
            entry(
                (2024, 4, 16),
                "Atualização",
                "BRCR11 - FDO INV IMOB - FII BTG PACTUAL CORP. OFFICE FUND",
                "BRCR11",
                "Credito",
                1433,
            ),
        ];

        let receipt_index = build_subscription_receipts_index(&entries);
        let update_entry = &entries[1];
        let match_result =
            match_subscription_receipt(&receipt_index, update_entry, Decimal::from(1433));

        assert!(match_result.is_none());
    }
}
