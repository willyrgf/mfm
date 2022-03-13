use crate::{
    cmd,
    config::{self, asset::Asset, rebalancer::Rebalancer, Config},
};
use clap::ArgMatches;
use std::{thread, time};
use web3::{transports::Http, types::U256};

pub const REBALANCER_COMMAND: &'static str = "rebalancer";

#[derive(Debug)]
pub struct AssetBalances<'a> {
    asset: &'a Asset,
    asset_decimals: u8,
    percent: f64,
    balance: U256,
    quoted_asset: &'a Asset,
    quoted_asset_decimals: u8,
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
            asset_decimals: asset.decimals(client.clone()).await,
            percent: rebalancer.get_asset_config_percent(asset.name()),
            balance: asset
                .balance_of(client.clone(), rebalancer.get_wallet(config).address())
                .await,
            quoted_asset: quoted_asset,
            quoted_asset_decimals: quoted_asset.decimals(client.clone()).await,
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

    pub fn desired_quoted_in_balance(&self, total_quoted_balance: U256) -> U256 {
        //self.quoted_balance / total_quoted_balance
        let p = ((self.percent / 100.0) * 10_f64.powf(self.quoted_asset_decimals.into())) as u128;
        let rb =
            (total_quoted_balance * U256::from(p)) / U256::exp10(self.quoted_asset_decimals.into());
        rb
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

    let total_quoted_balance: U256 = assets_balances
        .iter()
        .fold(U256::from(0_i32), |acc, x| acc + x.quoted_balance);
    log::debug!("total_quoted_balance: {:?}", total_quoted_balance);

    // let _: Vec<bool> = futures::future::join_all(assets_balances.iter().map(|ab| async move {
    for ab in assets_balances.iter() {
        if ab.asset.name() == rebalancer.parking_asset_id() {
            continue;
        }
        let exchange = ab.asset.get_exchange(&config);
        let network = exchange.get_network(&config.networks);
        let client = network.get_web3_client_http();
        let from_wallet = rebalancer.get_wallet(&config);
        let gas_price = client.eth().gas_price().await.unwrap();
        let parking_asset = rebalancer.get_parking_asset(&config.assets);
        let parking_asset_decimals = parking_asset.decimals(client.clone()).await;
        let parking_route = config.routes.search(ab.asset, parking_asset);

        let parking_amount_out: U256 = exchange
            .get_amounts_out(
                client.clone(),
                ab.balance,
                parking_route.build_path(&config.assets),
            )
            .await
            .last()
            .unwrap()
            .into();
        let ab_slip = ab.asset.slippage_u256(parking_asset_decimals);
        let parking_slip = parking_asset.slippage_u256(parking_asset_decimals);
        let slippage = ab_slip + parking_slip;
        let slippage_amount =
            (parking_amount_out * slippage) / U256::exp10(parking_asset_decimals.into());
        let parking_amount_out_slip = parking_amount_out - slippage_amount;
        log::debug!("parking_amount_out: {:?}", parking_amount_out);
        log::debug!("parking_amount_out_slip: {:?}", parking_amount_out_slip);
        let rb = ab.desired_quoted_in_balance(total_quoted_balance);
        log::debug!("asset_balance: {:?}, desired_quoted_in_balance: {}", ab, rb);
        let min_move = rebalancer.parking_asset_min_move_u256(parking_asset_decimals);
        if min_move >= parking_amount_out_slip {
            log::debug!(
                "min_move not sattisfied: min_move {}, parking_amounts_out {}",
                min_move,
                parking_amount_out_slip
            );
        }
        exchange
            .swap_tokens_for_tokens(
                client.clone(),
                from_wallet,
                gas_price,
                ab.balance,
                parking_amount_out_slip,
                parking_route.build_path_using_tokens(&config.assets),
            )
            .await;

        //TODO: implement a new way to watcher the transaction
        let duration = time::Duration::from_secs(180);
        thread::sleep(duration);
    }
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
