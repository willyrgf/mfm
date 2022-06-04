use crate::asset::Asset;
use crate::shared;
// use crate::config::asset::{Asset, Assets};

use std::path::Path;
use std::str::FromStr;
use std::time::UNIX_EPOCH;
use std::{collections::HashMap, time::SystemTime};

use rustc_hex::FromHexError;
use serde::{Deserialize, Serialize};
use web3::Web3;
use web3::{
    contract::{Contract, Options},
    ethabi::Token,
    transports::Http,
    types::{Address, H160, U256},
};

use super::network::Network;
use super::wallet::Wallet;
use super::Config;

pub mod swap_eth_for_tokens;
pub mod swap_tokens_for_tokens;

const ZERO_ADDRESS: &str = "0x0000000000000000000000000000000000000000";

//TODO: validate the fields in the new mod initialization
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Exchange {
    name: String,
    router_address: String,
    factory_address: String,
    network_id: String,
}

impl Exchange {
    pub fn name(&self) -> &str {
        self.name.as_str()
    }

    pub fn network_id(&self) -> &str {
        self.network_id.as_str()
    }

    pub fn router_address(&self) -> &str {
        self.router_address.as_str()
    }

    pub fn factory_address(&self) -> &str {
        self.router_address.as_str()
    }

    pub fn as_router_address(&self) -> Result<H160, FromHexError> {
        Address::from_str(self.factory_address())
    }

    pub fn as_factory_address(&self) -> H160 {
        Address::from_str(self.factory_address()).unwrap()
    }

    pub fn router_abi_path(&self) -> String {
        let path = format!("./res/exchanges/{}/abi.json", self.name.as_str());
        // TODO: move it to const static
        let fallback_path = "./res/exchanges/uniswap_v2_router_abi.json".to_string();
        if Path::new(&path).exists() {
            return path;
        }
        fallback_path
    }

    pub fn factory_abi_path(&self) -> String {
        let path = format!("./res/exchanges/{}/factory_abi.json", self.name.as_str());
        // TODO: move it to const static
        let fallback_path = "./res/exchanges/uniswap_v2_factory_abi.json".to_string();
        if Path::new(&path).exists() {
            return path;
        }
        fallback_path
    }

    pub fn router_abi_json_string(&self) -> String {
        let reader = std::fs::File::open(self.router_abi_path()).unwrap();
        let json: serde_json::Value = serde_json::from_reader(reader).unwrap();
        json.to_string()
    }

    pub fn factory_abi_json_string(&self) -> String {
        let reader = std::fs::File::open(self.factory_abi_path()).unwrap();
        let json: serde_json::Value = serde_json::from_reader(reader).unwrap();
        json.to_string()
    }

    pub fn router_contract(&self) -> Contract<Http> {
        let client = self.get_web3_client_http();
        let contract_address = self.as_router_address().unwrap();
        let json_abi = self.router_abi_json_string();
        Contract::from_json(client.eth(), contract_address, json_abi.as_bytes()).unwrap()
    }

    pub fn factory_contract(&self) -> Contract<Http> {
        let client = self.get_web3_client_http();
        let contract_address = self.as_factory_address();
        let json_abi = self.factory_abi_json_string();
        Contract::from_json(client.eth(), contract_address, json_abi.as_bytes()).unwrap()
    }

    pub async fn wrapper_address(&self) -> H160 {
        let router_contract = self.router_contract();

        let wrapped_addr = router_contract
            .query("WETH", (), None, Options::default(), None)
            .await
            .unwrap();

        wrapped_addr
    }

    pub async fn get_factory_pair(
        &self,
        input_asset: &Asset,
        output_asset: &Asset,
    ) -> Option<Address> {
        let contract = self.factory_contract();

        let result = contract.query(
            "getPair",
            (
                input_asset.as_address().unwrap(),
                output_asset.as_address().unwrap(),
            ),
            None,
            Options::default(),
            None,
        );

        let address = match result.await {
            Ok(a) => Some(a),
            _ => None,
        };

        address
    }

    pub async fn build_route_for(&self, input_asset: &Asset, output_asset: &Asset) -> Vec<H160> {
        //TODO: use path_asset from input and output asset
        // input -> path_asset -> path_asset from output -> output
        //TODO: check liquidity of directly path
        let mut v = vec![];
        let network = self.get_network();
        let wrapped_asset = network.get_wrapped_asset();
        let wrapped_is_output = wrapped_asset.address() == output_asset.address();
        let wrapped_is_input = wrapped_asset.address() == input_asset.address();
        let has_direct_route = match self.get_factory_pair(input_asset, output_asset).await {
            Some(a) => (a.to_string().as_str() != ZERO_ADDRESS),
            _ => false,
        };

        v.push(input_asset.as_address().unwrap());
        if !has_direct_route && !wrapped_is_output && !wrapped_is_input {
            v.push(wrapped_asset.as_address().unwrap());
        }
        v.push(output_asset.as_address().unwrap());

        v
    }

    //TODO: return error with value vec![zero]
    pub async fn get_amounts_out(
        &self,
        // decimals: u8,
        amount: U256,
        assets_path: Vec<H160>,
    ) -> Vec<U256> {
        let zero = U256::from(0_i32);

        //TODO: check if the amount is sufficient
        if amount == zero {
            return vec![zero];
        }

        let contract = self.router_contract();
        // let quantity = 1;
        // let amount: U256 = (quantity * 10_i32.pow(decimals.into())).into();
        let result = contract.query(
            "getAmountsOut",
            (amount, assets_path),
            None,
            Options::default(),
            None,
        );
        let result_amounts_out: Vec<U256> = match result.await {
            Ok(a) => a,
            Err(e) => {
                log::error!(
                    "get_amounts_out(): result err: {:?}, return zeroed value",
                    e
                );
                vec![zero]
            }
        };
        result_amounts_out
    }

    fn get_valid_timestamp(&self, future_millis: u128) -> u128 {
        let start = SystemTime::now();
        let since_epoch = start.duration_since(UNIX_EPOCH).unwrap();
        since_epoch.as_millis().checked_add(future_millis).unwrap()
    }

    pub fn get_network<'a>(&self) -> &'a Network {
        Config::global()
            .networks
            .get(self.network_id.as_str())
            .unwrap()
    }

    pub fn get_web3_client_http(&self) -> Web3<Http> {
        self.get_network().get_web3_client_http()
    }

    pub async fn swap_tokens_for_tokens(
        &self,
        from_wallet: &Wallet,
        amount_in: U256,
        amount_min_out: U256,
        input_asset: Asset,
        output_asset: Asset,
        slippage_opt: Option<U256>,
    ) {
        let asset_path = Token::Array(
            self.build_route_for(&input_asset, &output_asset)
                .await
                .into_iter()
                .map(Token::Address)
                .collect::<Vec<_>>(),
        );

        let asset_path_out = self.build_route_for(&output_asset, &input_asset).await;
        let asset_path_in = self.build_route_for(&input_asset, &output_asset).await;

        let input_asset_decimals = input_asset.decimals().await;
        let output_asset_decimals = output_asset.decimals().await;

        //TODO: review this model of use slippage
        let slippage = match slippage_opt {
            Some(s) => s,
            None => {
                let ais = input_asset.slippage_u256(input_asset_decimals);
                let aos = output_asset.slippage_u256(output_asset_decimals);

                ais + aos
            }
        };

        // TODO: move it to a func an test it
        // check what input asset max tx amount limit is lower to use
        //
        // anonq 10000  = 1USD
        // anonqv2 1000 = 1USD
        // (anonq/anonqv2)*10000 = 10000
        // (anonqv2/anonqv2)*1000 = 1000 // use it
        //
        // anonq 10000  = 1USD
        // anonqv2 1000 = 20USD
        // (anonq/anonqv2)*10000 = 500 // use it
        // (anonqv2/anonqv2)*1000 = 1000
        //
        // amount_min_out = 0,05
        // amount_min_out*1000 = 50
        let i_max_tx_amount = input_asset.max_tx_amount().await;
        let o_max_tx_amount = output_asset.max_tx_amount().await;
        log::debug!("cmd::swap() i_max_tx_amount: {:?}", i_max_tx_amount);
        log::debug!("cmd::swap() o_max_tx_amount: {:?}", o_max_tx_amount);
        let limit_max_input = match (i_max_tx_amount, o_max_tx_amount) {
            (Some(il), Some(ol)) => {
                // anonq =        10_000 = 10000anonq
                // safemoon = 10_000_000 = 249410anonq
                // 10000*11000 = 111_000
                // 10_000*11000 = (10000*11000)*6,17 = 678_000

                // 10000000*6,17 = 61.7MM
                // 10000*1 = 10000
                let limit_amount_out: U256 = self
                    .get_amounts_out(ol, asset_path_out.clone())
                    .await
                    .last()
                    .unwrap()
                    .into();

                log::debug!(
                    "cmd::swap(): limit_amount_out: {:?}, limit_amount_out: {:?}",
                    limit_amount_out,
                    shared::blockchain_utils::display_amount_to_float(
                        limit_amount_out,
                        input_asset_decimals
                    )
                );

                if il > limit_amount_out {
                    Some(limit_amount_out)
                } else {
                    Some(il)
                }
            }
            (None, Some(ol)) => {
                let limit_amount_in: U256 = self
                    .get_amounts_out(ol, asset_path_out.clone())
                    .await
                    .last()
                    .unwrap()
                    .into();
                log::debug!("cmd::swap() limit_amount_in: {:?}", limit_amount_in);
                Some(limit_amount_in)
            }
            (Some(il), None) => Some(il),
            (None, None) => None,
        };

        log::debug!("cmd::swap() limit_max_output: {:?}", limit_max_input);

        match limit_max_input {
            Some(limit) if amount_in > limit => {
                // TODO: resolv this calc with U256 exp10  or numbigint
                let mut total = amount_in;
                let amount_in_plus_two_decimals = amount_in * U256::exp10(2);
                let number_hops = (((amount_in_plus_two_decimals / limit).as_u128() as f64)
                    / 100_f64)
                    .ceil() as u64;

                for _ in 0..number_hops {
                    let ai: U256;
                    let ao: U256;

                    if total > limit {
                        //TODO: calc amount_min_out
                        total -= limit;
                        log::debug!(
                            "cmd::swap() inside total > limit: total: {:?}, limit: {:?}",
                            total,
                            limit
                        );

                        ai = limit;
                        ao = self
                            .get_amounts_out(limit, asset_path_in.clone())
                            .await
                            .last()
                            .unwrap()
                            .into();

                        log::debug!("cmd::swap() inside total > limit: ao: {:?}", ao);
                    } else {
                        ai = total;
                        ao = self
                            .get_amounts_out(total, asset_path_in.clone())
                            .await
                            .last()
                            .unwrap()
                            .into();

                        log::debug!("cmd::swap() inside total > limit: ao: {:?}", ao);
                    }

                    let slippage_amount =
                        (ao * slippage) / U256::exp10(output_asset_decimals.into());
                    let amount_min_out_slippage = ao - slippage_amount;
                    log::debug!("slippage_amount {:?}", slippage_amount);

                    swap_tokens_for_tokens::swap(
                        self,
                        from_wallet,
                        ai,
                        amount_min_out_slippage,
                        asset_path.clone(),
                    )
                    .await;
                }
            }
            _ => {
                swap_tokens_for_tokens::swap(
                    self,
                    from_wallet,
                    amount_in,
                    amount_min_out,
                    asset_path,
                )
                .await;
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Exchanges(HashMap<String, Exchange>);
impl Exchanges {
    pub fn get(&self, key: &str) -> &Exchange {
        match self.0.get(key) {
            Some(e) => e,
            None => {
                log::error!("get(): key {} doesnt exist", key);
                panic!();
            }
        }
    }
}
