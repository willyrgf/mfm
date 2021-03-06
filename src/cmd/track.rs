use crate::{
    asset::Asset,
    config::Config,
    rebalancer::{generate_asset_rebalances, AssetRebalancer},
    shared::blockchain_utils::display_amount_to_float,
};
use clap::{ArgMatches, Command};
use serde::{Deserialize, Serialize};
use web3::types::U256;

pub const TRACK_COMMAND: &str = "track";

//TODO: change the name of the tables like this
#[derive(Deserialize, Serialize, Clone)]
struct TrackAsset {
    asset: Asset,
    price: f64,
    balance: f64,
    quoted_asset: Asset,
    quoted_balance: f64,
    amount_to_trade: f64,
    quoted_amount_to_trade: f64,
}

#[derive(Deserialize, Serialize, Clone)]
pub struct TrackPortfolioStateData {
    portfolio_name: String,
    track_assets: Vec<TrackAsset>,
    quoted_portfolio_asset: Asset,
    quoted_portfolio_balance: f64,
    coin_balance: f64,
    total_estimate_swap_cost: f64,
    estimate_swap_cost: f64,
    gas_price: f64,
}

#[derive(Deserialize, Serialize, Clone)]
pub struct TrackPortfolioState {
    token_id: String,
    rebalancer_label: String,
    data: TrackPortfolioStateData,
}

pub fn generate_cmd() -> Command<'static> {
    Command::new(TRACK_COMMAND).about("Track all information to server")
}

pub async fn call_sub_commands(args: &ArgMatches) {
    cmd_run(args).await;
}

async fn cmd_run(_args: &ArgMatches) {
    let config = Config::global();

    let (api_token, api_address) = match &config.server {
        Some(s) => (s.api_token.clone(), s.api_url.clone()),
        None => {
            log::error!("track::cmd_run() config.server missing");
            panic!()
        }
    };

    for (rebalancer_name, rebalancer_config) in config.rebalancers.hashmap().iter() {
        log::info!(
            "track::cmd_run(): rebalancer_name: {:?}, rebalancer_config: {:?}",
            rebalancer_name,
            rebalancer_config
        );
        // TODO: back it when finish the break of validate
        // rebalancer::validate(rebalancer_config).await;

        let quoted_portfolio_asset = rebalancer_config.get_quoted_asset();
        let asset_quoted_decimals = quoted_portfolio_asset.decimals().await;

        let asset_rebalancers = generate_asset_rebalances(rebalancer_config).await;

        let track_assets = asset_rebalancers
            .clone()
            .iter()
            .map(|ar| TrackAsset {
                asset: ar.asset_balances.asset.clone(),
                price: display_amount_to_float(
                    ar.asset_balances.quoted_unit_price,
                    asset_quoted_decimals,
                ),
                balance: display_amount_to_float(
                    ar.asset_balances.balance,
                    ar.asset_balances.asset_decimals,
                ),
                quoted_asset: ar.rebalancer_config.get_quoted_asset(),
                quoted_balance: display_amount_to_float(
                    ar.asset_balances.quoted_balance,
                    asset_quoted_decimals,
                ),
                amount_to_trade: display_amount_to_float(
                    ar.asset_amount_to_trade,
                    ar.asset_balances.asset_decimals,
                ),
                quoted_amount_to_trade: display_amount_to_float(
                    ar.quoted_amount_to_trade,
                    asset_quoted_decimals,
                ),
            })
            .collect();

        let quoted_portfolio_balance_u256 =
            asset_rebalancers
                .clone()
                .iter()
                .fold(U256::from(0_u32), |acc, ar| {
                    let amount_in_quoted = ar.asset_balances.quoted_balance;
                    acc + amount_in_quoted
                });
        let quoted_portfolio_balance =
            display_amount_to_float(quoted_portfolio_balance_u256, asset_quoted_decimals);

        let network = rebalancer_config.get_network();
        let client = network.get_web3_client_http();
        let rebalancer_wallet = rebalancer_config.get_wallet();
        let coin_balance_u256 = rebalancer_wallet.coin_balance(client.clone()).await;
        let coin_balance = display_amount_to_float(coin_balance_u256, network.coin_decimals());

        let parking_asset = rebalancer_config.get_parking_asset();
        let from_wallet = rebalancer_config.get_wallet();

        let input_asset = match asset_rebalancers
            .clone()
            .iter()
            .filter(|ar| {
                (ar.asset_balances.asset.name() != parking_asset.name())
                    && ar.asset_balances.max_tx_amount.is_none()
                    && ar.asset_balances.balance > U256::from(0)
            })
            .last()
        {
            Some(ar) => ar.asset_balances.asset.clone(),
            None => panic!("No input asset to calculate swap cost"),
        };

        let amount_in = input_asset.balance_of(from_wallet.address()).await;
        let parking_asset_exchange = input_asset
            .get_network()
            .get_exchange_by_liquidity(&input_asset, &parking_asset, amount_in)
            .await
            .unwrap_or_else(|| {
                log::error!(
					"cmd_info(): network.get_exchange_by_liquidity(): None, asset_in: {:?}, asset_out: {:?}",
					input_asset.clone(),
					parking_asset
				);
                panic!()
            });

        let gas_price_u256 = client.clone().eth().gas_price().await.unwrap();
        let swap_cost = parking_asset_exchange
            .estimate_swap_cost(from_wallet, &input_asset, &parking_asset)
            .await;
        let total_ops = U256::from(asset_rebalancers.len());

        let total_estimate_swap_cost = display_amount_to_float(
            (swap_cost * gas_price_u256) * total_ops,
            network.coin_decimals(),
        );
        let estimate_swap_cost =
            display_amount_to_float(swap_cost * gas_price_u256, network.coin_decimals());
        let gas_price = display_amount_to_float(gas_price_u256, network.coin_decimals());

        let track_portfolio_state = {
            let data = TrackPortfolioStateData {
                portfolio_name: rebalancer_name.to_string(),
                track_assets,
                quoted_portfolio_asset,
                quoted_portfolio_balance,
                coin_balance,
                total_estimate_swap_cost,
                estimate_swap_cost,
                gas_price,
            };
            TrackPortfolioState {
                token_id: api_token.clone(),
                rebalancer_label: rebalancer_name.to_string(),
                data,
            }
        };

        let string_body = serde_json::to_string(&track_portfolio_state).unwrap();

        match {
            let client = reqwest::Client::new();
            client
                .post(&format!("{}/portfolio_state", api_address))
                .header("Content-Type", "application/json")
                .body(string_body)
                .send()
                .await
        } {
            Ok(response) => {
                log::info!("track::cmd_run() http request response: {:?}", response);
            }
            Err(e) => {
                log::error!("track::cmd_run() http request error: {:?}", e);
                panic!()
            }
        }
    }
}
