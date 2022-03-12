use std::collections::HashMap;

struct AssetConfig {
    asset_id: String,
    percent: f64,
}

struct Portfolio(HashMap<String, AssetConfig>);

pub struct Rebalancer {
    name: String,
    wallet_id: String,
    threshold_percent: f64,
    quoted_in: String,
    portfolio: Portfolio,
}
