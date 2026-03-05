use crate::db::AssetType;

/// Scope for reporting commands: full portfolio, by asset type, or single asset.
#[derive(Debug, Clone)]
pub enum Scope {
    Portfolio,
    AssetType(AssetType),
    SingleAsset(String),
}
