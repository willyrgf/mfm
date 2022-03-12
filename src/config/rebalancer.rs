use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::{
    asset::{Asset, Assets},
    wallet::Wallet,
    Config,
};

#[derive(Debug, PartialEq, Deserialize, Serialize)]
struct AssetConfig {
    // asset_id: String,
    percent: f64,
}

// TODO: validate portfolio max percent
// TODO: validate that all routes (n-to-n) to the assets exist
#[derive(Debug, PartialEq, Deserialize, Serialize)]
struct Portfolio(HashMap<String, AssetConfig>);

#[derive(Debug, PartialEq, Deserialize, Serialize)]
pub struct Rebalancer {
    name: String,
    wallet_id: String,
    threshold_percent: f64,
    quoted_in: String,
    parking_asset_id: String,
    portfolio: Portfolio,
}

impl Rebalancer {
    pub fn name(&self) -> String {
        self.name.clone()
    }

    pub fn total_percentage(&self) -> f64 {
        self.portfolio
            .0
            .iter()
            .map(|(_, asset_config)| asset_config.percent)
            .sum()
    }

    pub fn is_valid_portfolio_total_percentage(&self) -> bool {
        const REQUIRED: f64 = 100.0;
        self.total_percentage().eq(&REQUIRED)
    }

    pub fn get_asset_config_percent(&self, name: &str) -> f64 {
        match self.portfolio.0.get(name) {
            Some(a) => a.percent,
            None => {
                log::error!("asset_name {} doesnt exist", name);
                panic!()
            }
        }
    }

    pub fn get_assets<'a>(&self, config_assets: &'a Assets) -> Vec<&'a Asset> {
        self.portfolio
            .0
            .iter()
            .map(|(name, _)| config_assets.get(name))
            .collect()
    }

    pub fn quoted_in(&self) -> &str {
        self.quoted_in.as_str()
    }

    pub fn get_quoted_asset<'a>(&self, config_assets: &'a Assets) -> &'a Asset {
        config_assets.get(self.quoted_in())
    }

    pub fn get_wallet<'a>(&self, config: &'a Config) -> &'a Wallet {
        config.wallets.get(&self.wallet_id)
    }
}

#[derive(Debug, PartialEq, Deserialize, Serialize)]
pub struct Rebalancers(HashMap<String, Rebalancer>);
impl Rebalancers {
    pub fn get(&self, key: &str) -> &Rebalancer {
        self.0.get(key).unwrap()
    }
}
