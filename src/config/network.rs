use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use web3::{
    transports::{Http, WebSocket},
    types::U256,
    Web3,
};

use super::{wallet::Wallet, Config};
use crate::{asset::Asset, exchange::Exchange, utils::scalar::BigDecimal};

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Network {
    name: String,
    symbol: String,
    decimals: Option<u8>,
    chain_id: u32,
    rpc_url: String,
    node_url_http: Option<String>,
    blockexplorer_url: Option<String>,
    min_balance_coin: f64,
    wrapped_asset: Option<String>,
}

impl Default for Network {
    fn default() -> Self {
        Self {
            ..Default::default()
        }
    }
}

impl Network {
    pub fn rpc_url(&self) -> &str {
        self.rpc_url.as_str()
    }

    pub fn name(&self) -> &str {
        self.name.as_str()
    }

    pub fn symbol(&self) -> &str {
        self.symbol.as_str()
    }

    pub fn node_url(&self) -> Option<String> {
        self.node_url_http.clone()
    }

    // TODO: try get this value from some request in the blockchain
    pub fn coin_decimals(&self) -> u8 {
        match self.decimals {
            Some(n) => n,
            _ => 18_u8,
        }
    }

    pub fn get_wrapped_asset(&self) -> Result<Asset, anyhow::Error> {
        match &self.wrapped_asset {
            Some(wrapped_asset) => Config::global()
                .assets
                .find_by_name_and_network(wrapped_asset.as_str(), self.name.as_str()),
            None => Err(anyhow::anyhow!("wrapped_asset not found")),
        }
    }

    //TODO: validate min_balance_coin in the build of the type
    pub fn get_min_balance_coin(&self, decimals: u8) -> U256 {
        BigDecimal::try_from(self.min_balance_coin)
            .unwrap()
            .with_scale(decimals.into())
            .to_unsigned_u256()
    }

    pub fn get_web3_client_rpc(&self) -> Web3<Http> {
        // FIXME: handle propery with rpc
        self.get_web3_client_http(self.rpc_url()).unwrap()
    }

    pub fn get_web3_client_http(&self, url: &str) -> Result<Web3<Http>, anyhow::Error> {
        let http = Http::new(url).map_err(|e| anyhow::anyhow!(e))?;
        Ok(Web3::new(http))
    }

    pub async fn get_web3_client_ws(&self) -> Result<Web3<WebSocket>, anyhow::Error> {
        match self.node_url() {
            Some(n) => Ok(Web3::new(WebSocket::new(n.as_str()).await.unwrap())),
            None => Err(anyhow::anyhow!("missing network.node_url configuration")),
        }
    }

    pub fn get_exchanges(&self) -> Vec<&Exchange> {
        Config::global()
            .exchanges
            .hashmap()
            .values()
            .filter(|exchange_config| exchange_config.network_id() == self.name)
            .map(|config| &Exchange::new(config))
            .collect()
    }

    pub async fn balance_coin(&self, wallet: &Wallet) -> Result<U256, anyhow::Error> {
        self.get_web3_client_rpc()
            .eth()
            .balance(wallet.address(), None)
            .await
            .map_err(|e| anyhow::anyhow!("error fetch balance from network: {:?}", e))
    }

    pub async fn get_exchange_by_liquidity(
        &self,
        input_asset: &Asset,
        output_asset: &Asset,
        amount_in: U256,
    ) -> Option<&Exchange> {
        match self.get_exchanges().split_first() {
            Some((h, t)) if t.is_empty() => Some(*h),
            Some((h, t)) => {
                let mut current_amount_out = {
                    let path = h.build_route_for(input_asset, output_asset).await;
                    *h.get_amounts_out(amount_in, path).await.last().unwrap()
                };

                futures::future::join_all(t.iter().map(|e| async move {
                    let current_amount = {
                        let path = e.build_route_for(input_asset, output_asset).await;
                        *e.get_amounts_out(amount_in, path).await.last().unwrap()
                    };
                    (Some(*e), current_amount)
                }))
                .await
                .into_iter()
                .fold(
                    Some(*h),
                    |current_exchange, (next_exchange, next_amount)| {
                        if next_amount > current_amount_out {
                            current_amount_out = next_amount;
                            next_exchange
                        } else {
                            current_exchange
                        }
                    },
                )
            }
            _ => {
                tracing::debug!("Network::get_exchange_by_liquidity(): not exchange found, input_asset: {:?}, output_asset: {:?}", input_asset, output_asset);
                None
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Networks(HashMap<String, Network>);
impl Networks {
    pub fn get(&self, key: &str) -> Option<&Network> {
        self.0.get(key)
    }
    pub fn hashmap(&self) -> &HashMap<String, Network> {
        &self.0
    }
}
