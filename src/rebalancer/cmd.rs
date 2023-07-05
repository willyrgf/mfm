use chrono::prelude::*;
use std::ops::{Div, Mul};

use crate::{
    cmd,
    rebalancer::{
        self, config::Strategy, generate_asset_rebalances, get_total_quoted_balance, AssetBalances, move_asset_with_slippage
    },
    track,
    utils::{blockchain::display_amount_to_float, scalar::BigDecimal},
};
use clap::{ArgMatches, Command};
use prettytable::{row, Table};
use web3::types::U256;

pub const REBALANCER_COMMAND: &str = "rebalancer";
pub const REBALANCER_RUN_COMMAND: &str = "run";
pub const REBALANCER_INFO_COMMAND: &str = "info";
pub const REBALANCER_EXIT_COMMAND: &str = "exit";

pub fn generate_info_cmd() -> Command {
    Command::new(REBALANCER_INFO_COMMAND)
        .about("Infos about rebalancer")
        .arg(
            clap::arg!(-n --"name" <REBALANCER_NAME> "Rebalancer name from config file")
                .required(true),
        )
        .arg(
            clap::arg!(-s --"run-every" <TIME_IN_SECONDS> "Run continuously at every number of seconds")
            .value_parser(clap::value_parser!(u32)),
        )
        .arg(
            clap::arg!(-t --"track" <TRUE_FALSE> "Run track command after the info")
            .required(false)
            .default_value("false"),
        )
}

pub fn generate_run_cmd() -> Command {
    Command::new(REBALANCER_RUN_COMMAND)
        .about("Run rebalancer")
        .arg(
            clap::arg!(-n --"name" <REBALANCER_NAME> "Rebalancer name from config file")
                .required(true),
        )
}

pub fn generate_exit_cmd() -> Command {
    Command::new(REBALANCER_EXIT_COMMAND)
        .about("Exit all positions of the portfolio to parking asset")
        .arg(
            clap::arg!(-n --"name" <REBALANCER_NAME> "Rebalancer name from config file")
                .required(true),
        )
}

pub fn generate() -> Command {
    Command::new(REBALANCER_COMMAND)
        .about("Fires a rebalancer")
        .subcommand(generate_run_cmd())
        .subcommand(generate_info_cmd())
        .subcommand(generate_exit_cmd())
}

pub async fn call_sub_commands(args: &ArgMatches) -> Result<(), anyhow::Error> {
    match args.subcommand() {
        Some((REBALANCER_RUN_COMMAND, sub_args)) => cmd_run(sub_args).await,
        Some((REBALANCER_INFO_COMMAND, sub_args)) => cmd_info(sub_args).await,
        Some((REBALANCER_EXIT_COMMAND, sub_args)) => cmd_exit(sub_args).await,
        _ => Err(anyhow::anyhow!("no sub cmd found")),
    }
}

async fn cmd_exit(args: &ArgMatches) -> Result<(), anyhow::Error> {
    let mut config = cmd::helpers::get_rebalancer(args);
    tracing::debug!(
        "rebalancer::cmd::call_sub_commands(): rebalancer_config: {:?}",
        config
    );

    // TODO: a guarantee to try to move
    config.parking_asset_min_move = -0.1;

    let assets = config.get_assets()?;
    let assets_balances = rebalancer::get_assets_balances(&config, assets).await?;

    {
        let rebalancer_config = &config;
        let assets_balances = &assets_balances;
        let parking_asset = rebalancer_config.get_parking_asset();

        //TODO: do it to until all the balance are in the parking asset
        for ab in assets_balances.iter() {
            if ab.asset.name() == rebalancer_config.parking_asset_id() {
                continue;
            }

            let exchange = ab.asset
                .get_network()
                .get_exchange_by_liquidity(&ab.asset, &parking_asset, ab.balance)
                .await.unwrap_or_else(||{
                    tracing::error!(
                        "move_assets_to_parking(): network.get_exchange_by_liquidity(): None, asset_in: {:?}, asset_out: {:?}",
                        ab.asset,
                        parking_asset
                    );
                    panic!()
                });

            let parking_asset_path = exchange.build_route_for(&ab.asset, &parking_asset).await;

            let parking_amount_out: U256 = exchange
                .get_amounts_out(ab.balance, parking_asset_path.clone())
                .await
                .last()
                .unwrap()
                .into();

            let min_move =
                rebalancer_config.parking_asset_min_move_u256(parking_asset.decimals().await.unwrap());
            if min_move >= parking_amount_out {
                tracing::warn!(
                    "min_move not satisfied: min_move {}, parking_amount_out {}",
                    min_move,
                    parking_amount_out
                );
                //TODO: return this error?
                continue;
            }

            move_asset_with_slippage(
                rebalancer_config,
                &ab.asset,
                &parking_asset,
                ab.balance,
                parking_amount_out,
            )
            .await;
        }
    };
    Ok(())
}

async fn cmd_run(args: &ArgMatches) -> Result<(), anyhow::Error> {
    let config = cmd::helpers::get_rebalancer(args);
    tracing::debug!(
        "rebalancer::cmd::call_sub_commands(): rebalancer_config: {:?}",
        config
    );

    rebalancer::validate(&config).await?;

    match config.strategy() {
        Strategy::FullParking => {
            tracing::debug!("rebalancer::cmd::call_sub_commands() Strategy::FullParking");
            rebalancer::run_full_parking(&config).await
        }
        Strategy::DiffParking => {
            tracing::debug!("rebalancer::cmd::call_sub_commands() Strategy::DiffParking");
            tracing::info!("running run_diff_parking by Strategy::DiffParking");
            rebalancer::run_diff_parking(&config).await
        }
    }
}

async fn wrapped_cmd_info(args: &ArgMatches) -> Result<(), anyhow::Error> {
    let config = cmd::helpers::get_rebalancer(args);
    let asset_quoted = &config.get_quoted_asset();
    let asset_quoted_decimals = asset_quoted.decimals().await.unwrap();
    let mut portifolio_balance = U256::default();

    let mut table = Table::new();
    let mut wallet_table = Table::new();

    let utc_time = Local::now();

    wallet_table.add_row(row![
        "wallet",
        format!("{:?}", config.get_wallet().address()),
        "running_at",
        utc_time.to_rfc2822()
    ]);

    table.add_row(row![
        "Asset",
        "Percent",
        "Price",
        "Balance",
        "Quoted In",
        "Balance in quoted",
        "Amount to trade",
        "Quoted amount to trade",
        "Current Percent"
    ]);

    let asset_rebalances = generate_asset_rebalances(&config)
        .await
        .unwrap_or_else(|e| {
            tracing::error!(error = %e);
            panic!()
        });

    let assets_balances: Vec<AssetBalances> = asset_rebalances
        .clone()
        .iter()
        .map(|ar| ar.asset_balances.clone())
        .collect();

    let total_quoted = get_total_quoted_balance(assets_balances.as_slice());

    asset_rebalances.clone().iter().for_each(|ar| {
        let balance_of = ar.asset_balances.balance;
        let asset = ar.asset_balances.asset.clone();
        let decimals = ar.asset_balances.asset_decimals;
        let amount_in_quoted = ar.asset_balances.quoted_balance;
        let asset_quoted_decimals = ar.asset_balances.quoted_asset_decimals;

        let amount_in_quoted_bd = BigDecimal::from_unsigned_u256(
            &ar.asset_balances.quoted_balance,
            asset_quoted_decimals.into(),
        );

        let balance_of_bd = BigDecimal::from_unsigned_u256(&balance_of, decimals.into());
        portifolio_balance += amount_in_quoted;

        let price = {
            if balance_of_bd == BigDecimal::zero() {
                BigDecimal::from_unsigned_u256(
                    &ar.asset_balances.quoted_unit_price,
                    asset_quoted_decimals.into(),
                )
                .to_f64()
                .unwrap()
            } else {
                amount_in_quoted_bd
                    .clone()
                    .div(balance_of_bd.clone())
                    .with_scale(asset_quoted_decimals.into())
                    .to_f64()
                    .unwrap()
            }
        };

        table.add_row(row![
            asset.name(),
            ar.asset_balances.percent(),
            price,
            balance_of_bd.to_f64().unwrap(),
            config.quoted_in(),
            amount_in_quoted_bd.to_f64().unwrap(),
            ar.amount_f64_with_sign(ar.asset_amount_to_trade, decimals),
            ar.amount_f64_with_sign(ar.quoted_amount_to_trade, asset_quoted_decimals),
            (amount_in_quoted_bd.to_f64().unwrap()
                / BigDecimal::from_unsigned_u256(&total_quoted, asset_quoted_decimals.into())
                    .to_f64()
                    .unwrap())
                * 100.0
        ]);
    });

    let mut info_table = Table::new();
    info_table.add_row(row!["Min threshold", config.threshold_percent, "%"]);

    let asset_balances: Vec<AssetBalances> = asset_rebalances
        .clone()
        .into_iter()
        .map(|ar| ar.asset_balances)
        .collect();

    info_table.add_row(row![
        "Current threshold",
        config
            .current_percent_diff(&asset_balances)
            .mul(BigDecimal::try_from(100.0_f64).unwrap())
            .with_scale(4),
        "%"
    ]);

    info_table.add_row(row![
        "Current amount to trade",
        config.current_total_amount_to_trade(&asset_rebalances),
        asset_quoted.name()
    ]);

    let network = config.get_network();
    let client = network.get_web3_client_rpc();
    let rebalancer_wallet = config.get_wallet();
    let coin_balance = rebalancer_wallet.coin_balance(client.clone()).await;
    let mut balances_table = Table::new();

    balances_table.add_row(row![
        "Portifolio balance",
        display_amount_to_float(portifolio_balance, asset_quoted_decimals),
        asset_quoted.name()
    ]);

    balances_table.add_row(row![
        "Coin balance",
        display_amount_to_float(coin_balance, network.coin_decimals()),
        network.symbol()
    ]);

    let parking_asset = config.get_parking_asset();
    let from_wallet = config.get_wallet();

    // TODO: handle with the possibility to have just one asset.
    // By example, using a rebalancer exit command and just have a parking asset.
    // May a network default wrapped asset.
    let input_asset = match asset_rebalances
        .clone()
        .iter()
        .filter(|ar| {
            (ar.asset_balances.asset.name() != parking_asset.name())
                && ar.asset_balances.max_tx_amount.is_none()
                && ar.asset_balances.balance > U256::from(0)
        })
        .last()
    {
        Some(ar) => Some(ar.asset_balances.asset.clone()),
        None => {
            tracing::warn!("No input asset to calculate swap cost");
            None
        }
    };

    if let Some(input_asset) = input_asset {
        let amount_in = input_asset.balance_of(from_wallet.address()).await?;
        let parking_asset_exchange = input_asset
        .get_network()
        .get_exchange_by_liquidity(&input_asset, &parking_asset, amount_in)
        .await.unwrap_or_else(||{
            tracing::error!(
                "cmd_info(): network.get_exchange_by_liquidity(): None, asset_in: {:?}, asset_out: {:?}",
                input_asset.clone(),
                parking_asset
            );
            panic!()
        });

        let gas_price = client.clone().eth().gas_price().await.unwrap();
        let swap_cost = parking_asset_exchange
            .estimate_swap_cost(from_wallet, &input_asset, &parking_asset)
            .await?;

        let total_ops = U256::from(asset_rebalances.len());

        balances_table.add_row(row![
            "Total Swap cost",
            display_amount_to_float((swap_cost * gas_price) * total_ops, network.coin_decimals()),
            network.symbol()
        ]);
        balances_table.add_row(row![
            "Swap cost",
            display_amount_to_float(swap_cost * gas_price, network.coin_decimals()),
            network.symbol()
        ]);
        balances_table.add_row(row![
            "Gas price",
            display_amount_to_float(gas_price, network.coin_decimals()),
            network.symbol()
        ]);
    }

    wallet_table.printstd();
    table.printstd();
    info_table.printstd();
    balances_table.printstd();

    match cmd::helpers::get_track(args) {
        true => track::wrapped_run(args).await,
        false => Ok(()),
    }
}

async fn cmd_info(args: &ArgMatches) -> Result<(), anyhow::Error> {
    tracing::debug!("cmd_info()");

    let run_every = cmd::helpers::get_run_every(args);

    match run_every {
        Some(every_seconds) => loop {
            match wrapped_cmd_info(args).await {
                Ok(_) => {}
                Err(e) => tracing::error!(
                    "ignored by run_every config, error running wrapped_cmd_info(): {:?}",
                    e
                ),
            }

            let duration = std::time::Duration::from_secs((*every_seconds).into());
            std::thread::sleep(duration);

            // breaking line in the terminal to the possible next call
            print!("\n\n");
        },
        None => wrapped_cmd_info(args).await,
    }
}
