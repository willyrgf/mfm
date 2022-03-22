use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use web3::types::U256;

use crate::cmd::rebalancer::AssetBalances;

use super::{asset::Asset, wallet::Wallet, Config};

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
struct AssetConfig {
    // asset_id: String,
    percent: f64,
}

// TODO: validate portfolio max percent
// TODO: validate that all routes (n-to-n) to the assets exist
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
struct Portfolio(HashMap<String, AssetConfig>);

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Rebalancer {
    name: String,
    wallet_id: String,
    threshold_percent: f64,
    quoted_in: String,
    network_id: String,
    parking_asset_id: String,
    portfolio: Portfolio,
    parking_asset_min_move: f64,
}

impl Rebalancer {
    pub fn parking_asset_min_move_u256(&self, decimals: u8) -> U256 {
        //TODO: review u128
        let qe = (self.parking_asset_min_move * 10_f64.powf(decimals.into())) as u128;
        U256::from(qe)
    }

    pub fn network_id(&self) -> &str {
        self.network_id.as_str()
    }

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

    pub fn get_assets<'a>(&self) -> Vec<&'a Asset> {
        self.portfolio
            .0
            .iter()
            .map(|(name, _)| {
                Config::global()
                    .assets
                    .find_by_name_and_network(name.as_str(), self.network_id.as_str())
                    .unwrap()
            })
            .collect()
    }

    pub fn quoted_in(&self) -> &str {
        self.quoted_in.as_str()
    }

    pub fn parking_asset_id(&self) -> &str {
        self.parking_asset_id.as_str()
    }

    pub fn get_quoted_asset<'a>(&self) -> &'a Asset {
        Config::global()
            .assets
            .find_by_name_and_network(self.quoted_in(), self.network_id.as_str())
            .unwrap()
    }

    pub fn get_parking_asset<'a>(&self) -> &'a Asset {
        Config::global().assets.get(self.parking_asset_id())
    }

    pub fn get_wallet<'a>(&self) -> &'a Wallet {
        Config::global().wallets.get(&self.wallet_id)
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
    pub fn reach_min_threshold(&self, assets_balances: &[AssetBalances]) -> bool {
        // TODO: abstract this
        // abs for U256
        let u256_abs_diff = |qap: U256, pn: U256| {
            if qap.ge(&pn) {
                qap - pn
            } else {
                pn - qap
            }
        };

        let quoted_asset_decimals = assets_balances.last().unwrap().quoted_asset_decimals();
        let max_percent_u256 = U256::from(100_i32) * U256::exp10(quoted_asset_decimals.into());

        let thresold_percent_u256 = U256::from(
            (self.threshold_percent * 10_f64.powf(quoted_asset_decimals.into())) as u128,
        );
        log::debug!(
            "reach_min_threshold(): thresold_percent_u256: {:?}",
            thresold_percent_u256
        );

        let total_quoted = assets_balances
            .iter()
            .fold(U256::from(0_i32), |acc, x| acc + x.quoted_balance());
        log::debug!("reach_min_threshold(): total_quoted: {:?}", total_quoted);

        let sum_percent_diff =
            assets_balances
                .iter()
                .fold(U256::from(0_i32), |acc, asset_balances| {
                    if asset_balances.quoted_balance() <= U256::from(0_i32) {
                        return acc;
                    }

                    let quoted_asset_percent = asset_balances.quoted_asset_percent_u256();
                    log::debug!(
                        "reach_min_threshold(): quoted_asset_percent_u256: {:?}",
                        quoted_asset_percent
                    );

                    let p_now = (asset_balances.quoted_balance()
                        * U256::exp10(asset_balances.quoted_asset_decimals().into()))
                        / total_quoted;
                    log::debug!("reach_min_threshold(): p_now: {:?}", p_now);

                    let p_diff = u256_abs_diff(quoted_asset_percent, p_now);

                    log::debug!("reach_min_threshold(): p_diff: {:?}", p_diff);
                    acc + p_diff
                });

        log::debug!(
            "reach_min_threshold(): sum_percent_diff: {:?}",
            sum_percent_diff
        );

        let percent_diff = max_percent_u256 - sum_percent_diff;
        log::debug!("reach_min_threshold(): percent_diff: {:?}", percent_diff);

        percent_diff.gt(&thresold_percent_u256)
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Rebalancers(HashMap<String, Rebalancer>);
impl Rebalancers {
    pub fn get(&self, key: &str) -> &Rebalancer {
        self.0.get(key).unwrap()
    }
}
