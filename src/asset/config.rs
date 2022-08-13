use super::Asset;
use crate::config::Config;
use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct AssetNetwork {
    pub(crate) name: String,
    pub(crate) network_id: String,
    pub(crate) address: String,
    pub(crate) slippage: f64,
    pub(crate) path_asset: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct AssetNetworks(HashMap<String, AssetNetwork>);

impl AssetNetworks {
    pub fn get(&self, key: &str) -> Result<&AssetNetwork, anyhow::Error> {
        self.0.get(key).context("network doesnt exist for asset")
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
    pub fn new_assets_list(&self) -> Result<Vec<Asset>, anyhow::Error> {
        self.networks
            .hashmap()
            .values()
            .map(|a| {
                let network = Config::global()
                    .networks
                    .get(&a.network_id)
                    .context("network not found")?;
                Asset::new(self, network)
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
    pub fn find_by_name_and_network(
        &self,
        name: &str,
        network: &str,
    ) -> Result<Asset, anyhow::Error> {
        let config = Config::global();
        let asset_config = self
            .get(name)
            .context(format!("asset_config not found, key: {}", name))?;

        let network = config
            .networks
            .get(network)
            .context(format!("network not found, key: {}", network))?;

        Asset::new(asset_config, network)
    }
}
