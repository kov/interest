use crate::db::AssetType;
use crate::reports::portfolio::PositionSummary;
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
