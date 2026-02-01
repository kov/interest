use crate::db::AssetType;
use crate::reports::portfolio::PositionSummary;
use anyhow::Result;
use chrono::NaiveDate;
use rusqlite::Connection;
use rust_decimal::Decimal;
use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct AssetTypeTotals {
    pub cost_basis: Decimal,
    pub market_value: Decimal,
    pub unrealized_pl: Decimal,
    pub return_pct: Decimal,
}

pub fn aggregate_positions_by_asset_type(
    positions: &[PositionSummary],
) -> HashMap<AssetType, AssetTypeTotals> {
    let mut raw_totals: HashMap<AssetType, (Decimal, Decimal)> = HashMap::new();

    for position in positions {
        let entry = raw_totals
            .entry(position.asset.asset_type)
            .or_insert((Decimal::ZERO, Decimal::ZERO));
        entry.0 += position.total_cost;
        if let Some(value) = position.current_value {
            entry.1 += value;
        }
    }

    let mut totals = HashMap::new();
    for (asset_type, (cost_basis, market_value)) in raw_totals {
        let unrealized_pl = market_value - cost_basis;
        let return_pct = if cost_basis > Decimal::ZERO {
            (unrealized_pl / cost_basis) * Decimal::from(100)
        } else {
            Decimal::ZERO
        };
        totals.insert(
            asset_type,
            AssetTypeTotals {
                cost_basis,
                market_value,
                unrealized_pl,
                return_pct,
            },
        );
    }

    totals
}

pub fn normalize_positions_with_prices(
    conn: &Connection,
    date: NaiveDate,
    positions: Vec<PositionSummary>,
) -> Result<(Vec<PositionSummary>, Decimal, Decimal)> {
    let mut normalized = Vec::new();
    let mut total_cost = Decimal::ZERO;
    let mut total_value = Decimal::ZERO;

    for mut position in positions {
        total_cost += position.total_cost;

        let asset_id = match position.asset.id {
            Some(id) => id,
            None => {
                position.current_price = None;
                position.current_value = None;
                position.unrealized_pl = None;
                position.unrealized_pl_pct = None;
                normalized.push(position);
                continue;
            }
        };

        let price = crate::db::get_price_on_or_before(conn, asset_id, date)?;
        match price {
            Some(p) => {
                let current_price = p.close_price;
                let current_value = current_price * position.quantity;
                let unrealized_pl = current_value - position.total_cost;
                let unrealized_pl_pct = if position.total_cost > Decimal::ZERO {
                    Some((unrealized_pl / position.total_cost) * Decimal::from(100))
                } else {
                    Some(Decimal::ZERO)
                };

                position.current_price = Some(current_price);
                position.current_value = Some(current_value);
                position.unrealized_pl = Some(unrealized_pl);
                position.unrealized_pl_pct = unrealized_pl_pct;
                total_value += current_value;
            }
            None => {
                position.current_price = None;
                position.current_value = None;
                position.unrealized_pl = None;
                position.unrealized_pl_pct = None;
            }
        }

        normalized.push(position);
    }

    Ok((normalized, total_cost, total_value))
}
