use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq, Deserialize, Serialize)]
struct AssetConfig {
    asset_id: String,
    percent: f64,
}

#[derive(Debug, PartialEq, Deserialize, Serialize)]
struct Portfolio(HashMap<String, AssetConfig>);

#[derive(Debug, PartialEq, Deserialize, Serialize)]
pub struct Rebalancer {
    name: String,
    wallet_id: String,
    threshold_percent: f64,
    quoted_in: String,
    portfolio: Portfolio,
}

#[derive(Debug, PartialEq, Deserialize, Serialize)]
pub struct Rebalancers(HashMap<String, Rebalancer>);
impl Rebalancers {
    pub fn get(&self, key: &str) -> &Rebalancer {
        self.0.get(key).unwrap()
    }
}
