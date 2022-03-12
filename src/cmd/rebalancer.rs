use crate::{
    cmd,
    config::{self, asset::Asset, rebalancer::Rebalancer, Config},
};
use clap::ArgMatches;
use web3::{transports::Http, types::U256};

pub const REBALANCER_COMMAND: &'static str = "rebalancer";

#[derive(Debug)]
pub struct AssetBalances<'a> {
    asset: &'a Asset,
    percent: f64,
    balance: U256,
    quoted_asset: &'a Asset,
    quoted_balance: U256,
}

impl<'a> AssetBalances<'a> {
    pub async fn new(
        client: web3::Web3<Http>,
        config: &'a Config,
        rebalancer: &'a Rebalancer,
        asset: &'a Asset,
    ) -> AssetBalances<'a> {
        let quoted_asset = rebalancer.get_quoted_asset(&config.assets);
        Self {
            asset: asset,
            percent: rebalancer.get_asset_config_percent(asset.name()),
            balance: asset
                .balance_of(client.clone(), rebalancer.get_wallet(config).address())
                .await,
            quoted_asset: quoted_asset,
            quoted_balance: asset
                .balance_of_quoted_in(
                    client.clone(),
                    config,
                    rebalancer.get_wallet(config),
                    quoted_asset,
                )
                .await,
        }
    }
}

pub async fn call_sub_commands(args: &ArgMatches, config: &config::Config) {
    let rebalancer = cmd::get_rebalancer(args, config);
    log::debug!("rebalancer: {:?}", rebalancer);

    if !rebalancer.is_valid_portfolio_total_percentage() {
        log::error!(
            "rebalancer: {}, sum of portfolio percent should be 100, current is: {}",
            rebalancer.name(),
            rebalancer.total_percentage()
        );

        panic!()
    }

    let assets = rebalancer.get_assets(&config.assets);

    assets.iter().for_each(|&a| log::debug!("asset: {:?}", &a));

    let assets_balances: Vec<AssetBalances> =
        futures::future::join_all(assets.iter().map(|&asset| {
            let exchange = asset.get_exchange(config);
            let client = exchange
                .get_network(&config.networks)
                .get_web3_client_http();

            AssetBalances::new(client, config, rebalancer, asset)
        }))
        .await;

    log::debug!("assets_balances: {:?}", assets_balances);

    // get balance per asset
    // get balance quoted_in asset

    // get total_balance

    // calc how much we need for each asset
    // calc the diff of expect with current balance per asset
    // swap the balances of all assets to the parking_asset
    // calc quoted_balance of the parking_asset
    // buy from parking_asset the percent of each asset
    // check if the portfolio is balanced
}

/*

wbnb, wbtc, weth

busd

 */
