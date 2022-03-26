use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::config::Config;

use super::Asset;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct AssetNetwork {
    pub(crate) name: String,
    pub(crate) network_id: String,
    pub(crate) address: String,
    pub(crate) exchange_id: String,
    pub(crate) slippage: f64,
    pub(crate) path_asset: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct AssetNetworks(HashMap<String, AssetNetwork>);

impl AssetNetworks {
    pub fn get(&self, key: &str) -> &AssetNetwork {
        self.0.get(key).unwrap()
    }
    pub fn hashmap(&self) -> &HashMap<String, AssetNetwork> {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct AssetConfig {
    pub(crate) kind: String,
    pub(crate) networks: AssetNetworks,
}

impl AssetConfig {
    pub fn new_assets_list(&self) -> Vec<&Asset> {
        self.networks
            .hashmap()
            .values()
            .map(|a| {
                let network = Config::global().networks.get(&a.network_id).unwrap();
                &Asset::new(self, network)
            })
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct AssetsConfig(HashMap<String, AssetConfig>);
impl AssetsConfig {
    pub fn hashmap(&self) -> &HashMap<String, AssetConfig> {
        &self.0
    }

    pub fn get(&self, key: &str) -> Option<&AssetConfig> {
        self.0.get(key)
    }

    //TODO: use this function to get assets of the current network
    pub fn find_by_name_and_network(&self, name: &str, network: &str) -> Option<&Asset> {
        let config = Config::global();
        let asset_config = match self.get(name) {
            Some(a) => a,
            None => return None,
        };
        let network = match config.networks.get(network) {
            Some(n) => n,
            None => return None,
        };

        Some(&Asset::new(asset_config, network))
    }
}
