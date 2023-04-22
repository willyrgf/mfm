use crate::asset::Asset;
use crate::config::network::Network;
use crate::utils::math;
use crate::utils::scalar::BigDecimal;
use crate::{config::wallet::Wallet, config::Config};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::ops::{Add, Div, Sub};
use web3::types::U256;

use super::{AssetBalances, AssetRebalancer, Kind};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Strategy {
    FullParking,
    DiffParking,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
struct AssetConfig {
    // asset_id: String,
    percent: f64,
}

// TODO: validate portfolio max percent
// TODO: validate that all routes (n-to-n) to the assets exist
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Portfolio(HashMap<String, AssetConfig>);

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct RebalancerConfig {
    pub(crate) name: String,
    pub(crate) wallet_id: String,
    pub(crate) network_id: String,
    pub(crate) portfolio: Portfolio,
    pub(crate) strategy: Strategy,
    pub(crate) threshold_percent: f64,
    pub(crate) quoted_in: String,
    pub(crate) parking_asset_id: String,
    // TODO: refactor the move_assets_to_parking function to be reusable and have another function
    // to exit command.
    pub(crate) parking_asset_min_move: f64,
}

impl RebalancerConfig {
    pub fn parking_asset_min_move_u256(&self, decimals: u8) -> U256 {
        math::f64_to_u256(self.parking_asset_min_move, decimals)
    }

    pub fn network_id(&self) -> &str {
        self.network_id.as_str()
    }

    pub fn name(&self) -> String {
        self.name.clone()
    }

    pub fn strategy(&self) -> Strategy {
        self.strategy.clone()
    }

    pub fn total_percentage(&self) -> f64 {
        self.portfolio
            .0
            .values()
            .map(|asset_config| asset_config.percent)
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
                tracing::error!("RebalancerConfig::get_asset_config_percent(): asset_name {} doesnt exist in portfolio", name);
                panic!()
            }
        }
    }

    pub fn get_assets(&self) -> Result<Vec<Asset>, anyhow::Error> {
        self.portfolio
            .0
            .keys()
            .map(|name| {
                Config::global()
                    .assets
                    .find_by_name_and_network(name.as_str(), self.network_id.as_str())
            })
            .collect()
    }

    pub fn quoted_in(&self) -> &str {
        self.quoted_in.as_str()
    }

    pub fn parking_asset_id(&self) -> &str {
        self.parking_asset_id.as_str()
    }

    pub fn get_quoted_asset(&self) -> Asset {
        Config::global()
            .assets
            .find_by_name_and_network(self.quoted_in(), self.network_id.as_str())
            .unwrap()
    }

    pub fn get_parking_asset(&self) -> Asset {
        Config::global()
            .assets
            .find_by_name_and_network(self.parking_asset_id.as_str(), self.network_id.as_str())
            .unwrap()
    }

    // TODO: refactor RebalancerConfig to carry the wallet and avoid panic here
    pub fn get_wallet<'a>(&self) -> &'a Wallet {
        Config::global().wallets.get(&self.wallet_id).unwrap()
    }

    pub fn get_network(&self) -> &Network {
        match Config::global().networks.get(self.network_id()) {
            Some(n) => n,
            _ => panic!("missing network for rebalancer"),
        }
    }

    pub fn threshold_percent(&self) -> f64 {
        self.threshold_percent
    }

    /*
        Returns true if the percentual difference between current portfolio tokes amount
        and the expected.

        A algorithm to get percent diff between current and expected portfolio tokens:
        ```numi.app
            each_percent=0,25 = 0,25

            now_btc_quoted=250 = 250
            now_eth_quoted=250 = 250
            now_anonq_quoted=250 = 250
            now_bnb_quoted=500 = 500
            now_total=now_btc_quoted+now_eth_quoted+now_anonq_quoted+now_bnb_quoted = 1.250

            percent_now_btc = now_btc_quoted/now_total = 0,2
            percent_now_eth = now_eth_quoted/now_total = 0,2
            percent_now_anonq = now_anonq_quoted/now_total = 0,2
            percent_now_bnb = now_bnb_quoted/now_total = 0,4

            percent_diff=abs(percent_now_btc-each_percent)+abs(percent_now_eth-each_percent)+abs(percent_now_anonq-each_percent)+abs(percent_now_bnb-each_percent) = 0,3

            threshold = 0,02 = 0,02

            threshold-percent_diff = -0,28
        ```
    */

    pub fn u256_abs_diff(&self, qap: U256, pn: U256) -> U256 {
        if qap.ge(&pn) {
            qap - pn
        } else {
            pn - qap
        }
    }

    pub fn current_total_amount_to_trade(
        &self,
        assets_rebalances: &[AssetRebalancer],
    ) -> BigDecimal {
        assets_rebalances
            .iter()
            .fold(BigDecimal::from(0_i32), |acc, x| {
                let quoted_amount_to_trade = BigDecimal::from_unsigned_u256(
                    &x.quoted_amount_to_trade,
                    x.asset_balances.quoted_asset_decimals().into(),
                );

                match x.kind {
                    Kind::FromParking => acc.add(quoted_amount_to_trade),
                    _ => acc.add(BigDecimal::zero()),
                }
            })
    }

    pub fn current_percent_diff(&self, assets_balances: &[AssetBalances]) -> BigDecimal {
        let total_quoted = assets_balances
            .iter()
            .fold(BigDecimal::from(0_i32), |acc, x| {
                acc + BigDecimal::from_unsigned_u256(
                    &x.quoted_balance(),
                    x.quoted_asset_decimals().into(),
                )
            });

        let sum_percent_diff =
            assets_balances
                .iter()
                .fold(BigDecimal::from(0_i32), |acc, asset_balances| {
                    let quoted_balance_bd = BigDecimal::from_unsigned_u256(
                        &asset_balances.quoted_balance(),
                        asset_balances.quoted_asset_decimals().into(),
                    );

                    if quoted_balance_bd <= BigDecimal::from(0_i32) {
                        return acc;
                    }

                    let quoted_asset_percent_bd = math::percent_to_bigdecimal(
                        asset_balances.percent(),
                        asset_balances.quoted_asset_decimals(),
                    );

                    let p_now_bd = quoted_balance_bd.div(total_quoted.clone());
                    let p_diff_bd = quoted_asset_percent_bd.sub(p_now_bd).abs();

                    acc + p_diff_bd
                });

        //max_percent.sub(sum_percent_diff).with_scale(4)
        let total_sides = BigDecimal::from(2_i32);
        let round_scale = 4_u8;
        sum_percent_diff
            .div(total_sides)
            .with_scale(round_scale.into())
    }

    // TODO: add tests and refactor this function
    pub fn reach_min_threshold(&self, assets_balances: &[AssetBalances]) -> bool {
        let percent_decimal = 4_u8;
        let threshold_percent =
            math::percent_to_bigdecimal(self.threshold_percent(), percent_decimal);
        let current_percent_diff = self
            .current_percent_diff(assets_balances)
            .with_scale(percent_decimal.into());

        current_percent_diff.ge(&threshold_percent)
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct RebalancersConfig(HashMap<String, RebalancerConfig>);
impl RebalancersConfig {
    pub fn hashmap(&self) -> &HashMap<String, RebalancerConfig> {
        &self.0
    }
    pub fn get(&self, key: &str) -> &RebalancerConfig {
        self.0.get(key).unwrap()
    }
}
