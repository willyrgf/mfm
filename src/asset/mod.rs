use self::config::AssetConfig;
use crate::config::{exchange::Exchange, Config};
use web3::types::H160;

pub mod config;

pub struct Asset {
    name: String,
    network_id: String,
    address: String,
    address_h160: H160,
    exchange_id: String,
    exchange: Exchange,
    slippage: f64,
}

impl Asset {
    fn new(config: &Config, config_asset: AssetConfig) -> Self {
        Self {
            name: config_asset.name,
            network_id: config_asset.network_id,
            address: config_asset.address,
            address_h160: config_asset.as_address().unwrap(),
            exchange_id: config_asset.exchange_id,
            exchange: *config_asset.get_exchange(config).clone(),
            slippage: config_asset.slippage,
        }
    }
}
