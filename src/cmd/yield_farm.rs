use crate::{
    cmd,
    config::{yield_farm::YieldFarm, Config},
    shared,
};
use clap::{ArgMatches, Command};
use prettytable::{cell, row, Table};
use web3::types::U256;

pub const YIELD_FARM_COMMAND: &str = "yield-farm";
pub const YIELD_FARM_RUN_COMMAND: &str = "run";
pub const YIELD_FARM_INFO_COMMAND: &str = "info";
pub const YIELD_FARM_DEPOSIT_COMMAND: &str = "deposit";

pub fn generate_deposit_cmd<'a>() -> Command<'a> {
    Command::new(YIELD_FARM_DEPOSIT_COMMAND)
        .about("Deposit asset into farm")
        .arg(
            clap::arg!(-y --"yield-farm" <YIELD_FARM_NAME> "Yield farm name in config file")
                .required(true),
        )
        .arg(
            clap::arg!(-a --"amount" <AMOUNT> "Amount of yield farm deposit asset to deposit")
                .required(true),
        )
}

pub fn generate_info_cmd<'a>() -> Command<'a> {
    Command::new(YIELD_FARM_INFO_COMMAND)
        .about("Get info about all or one yield farm")
        .arg(
            clap::arg!(-y --"yield-farm" <YIELD_FARM_NAME> "Yield farm name in config file")
                .required(false),
        )
        .arg(
            clap::arg!(-q --"quoted-asset" <busd> "Asset to quote rewards")
                .required(false)
                .default_value("usdc"),
        )
}

pub fn generate_run_cmd<'a>() -> Command<'a> {
    Command::new(YIELD_FARM_RUN_COMMAND)
        .about("Run harvest on all or one yield farm")
        .arg(
            clap::arg!(-y --"yield-farm" <YIELD_FARM_NAME> "Yield farm name in config file")
                .required(false),
        )
        .arg(
            clap::arg!(-f --"force-harvest" <true_or_false> "Will ignore the min rewards required")
                .required(false)
                .default_value("false"),
        )
}

pub fn generate_cmd<'a>() -> Command<'a> {
    Command::new(YIELD_FARM_COMMAND)
        .about("Haverst some YieldFarm")
        .subcommand(generate_info_cmd())
        .subcommand(generate_run_cmd())
        .subcommand(generate_deposit_cmd())
}

pub fn get_farms_to_look(args: &ArgMatches) -> Vec<&YieldFarm> {
    let farms_to_look: Vec<&YieldFarm> = match args.value_of("yield-farm") {
        Some(y) => vec![Config::global().yield_farms.get(y)],
        None => Config::global()
            .yield_farms
            .hashmap()
            .iter()
            .map(|(_k, v)| v)
            .collect::<Vec<&YieldFarm>>(),
    };

    farms_to_look
}

pub async fn call_info_cmd(args: &ArgMatches) {
    let mut table = Table::new();
    table.add_row(row![
        "Network",
        "Farm",
        "Deposit Asset",
        "Deposit Amount",
        "Deposit Amount in Quoted",
        "Pending rewards",
        "Reward Asset",
        "Quoted pending rewards",
        "Quoted Asset",
        "Decimals",
        "Min rewards required"
    ]);

    futures::future::join_all(get_farms_to_look(args).iter().map(|&yield_farm| async {
        let quoted_asset =
            cmd::helpers::get_quoted_asset_in_network_from_args(args, yield_farm.network_id())
                .unwrap();
        let quoted_asset_decimal = quoted_asset.decimals().await;
        let yield_farm_asset = yield_farm.get_reward_asset().unwrap();
        let yield_farm_asset_decimals = yield_farm_asset.decimals().await;
        let deposit_asset = yield_farm.get_deposit_asset();
        let deposit_asset_decimals = match deposit_asset.clone() {
            Some(a) => a.decimals().await,
            None => 0_u8,
        };
        let reward_asset = yield_farm.get_reward_asset();
        let reward_asset_decimals = match reward_asset.clone() {
            Some(a) => a.decimals().await,
            None => 0_u8,
        };

        let pending_rewards = yield_farm.get_pending_rewards().await;

        let exchange = yield_farm_asset
            .get_network()
            .get_exchange_by_liquidity(&yield_farm_asset,&quoted_asset, pending_rewards)
            .await.
            unwrap_or_else(||{
                log::error!(
                    "call_info_cmd(): network.get_exchange_by_liquidity(): None, asset_in: {:?}, asset_out: {:?}",
                    yield_farm_asset,
                    quoted_asset
                );
                panic!()
        });

        let deposit_asset_name: String = match deposit_asset.clone() {
            Some(a) => a.name().into(),
            None => "".to_string(),
        };
        let reward_asset_name: String = match reward_asset.clone() {
            Some(a) => a.name().into(),
            None => "".to_string(),
        };
        let quote_asset_path = exchange
            .build_route_for(&yield_farm_asset, &quoted_asset)
            .await;

        let deposited_quote_asset_path = match deposit_asset.clone() {
            Some(a) => {
                let path = exchange.build_route_for(&a, &quoted_asset).await;
                Some(path)
            }
            None => None,
        };

        let quoted_price = match exchange
            .get_amounts_out(pending_rewards, quote_asset_path)
            .await
            .last()
        {
            Some(&u) => u,
            _ => U256::from(0_i32),
        };

        let deposited_amount = yield_farm.get_deposited_amount().await;

        let deposited_amount_in_quoted = match deposited_amount {
            z if z == U256::from(0_i32) => U256::default(),
            a => match deposited_quote_asset_path.clone() {
                Some(p) => match exchange.get_amounts_out(a, p).await.last() {
                    Some(&u) => u,
                    _ => U256::from(0_i32),
                },
                None => U256::default(),
            },
        };

        let min_rewards_required =
            yield_farm.get_min_rewards_required_u256(yield_farm_asset_decimals);

        let result = YieldFarmInfoCmdOutput {
            deposit_asset_name,
            reward_asset_name,
            deposit_asset_decimals,
            deposited_amount_in_quoted,
            reward_asset_decimals,
            network_id: yield_farm.network_id().into(),
            yield_farm_name: yield_farm.name().into(),
            deposited_amount,
            pending_rewards,
            yield_farm_asset_name: yield_farm_asset.name().into(),
            quoted_price,
            quoted_asset_name: quoted_asset.name().into(),
            yield_farm_asset_decimals,
            quoted_asset_decimal,
            min_rewards_required,
        };

        result
    }))
    .await
    .into_iter()
    .for_each(|cmd_output| {
        table.add_row(row![
            cmd_output.network_id,
            cmd_output.yield_farm_name,
            cmd_output.deposit_asset_name,
            shared::blockchain_utils::display_amount_to_float(
                cmd_output.deposited_amount,
                cmd_output.yield_farm_asset_decimals
            ),
            shared::blockchain_utils::display_amount_to_float(
                cmd_output.deposited_amount_in_quoted,
                cmd_output.quoted_asset_decimal
            ),
            shared::blockchain_utils::display_amount_to_float(
                cmd_output.pending_rewards,
                cmd_output.yield_farm_asset_decimals
            ),
            cmd_output.reward_asset_name,
            shared::blockchain_utils::display_amount_to_float(
                cmd_output.quoted_price,
                cmd_output.quoted_asset_decimal
            ),
            cmd_output.quoted_asset_name,
            cmd_output.yield_farm_asset_decimals,
            cmd_output.min_rewards_required
        ]);
    });
    table.printstd();
}

pub async fn call_deposit_cmd(args: &ArgMatches) {
    let mut table = Table::new();
    table.add_row(row![
        "Deposited Amount",
        "Deposited Asset ",
        "Farm",
        "Decimals",
    ]);
    let yield_farm = cmd::helpers::get_yield_farm(args);
    let yield_farm_asset = yield_farm.get_deposit_asset().unwrap();
    let yield_farm_asset_decimals = yield_farm_asset.decimals().await;
    let amount = cmd::helpers::get_amount(args, yield_farm_asset_decimals);

    yield_farm.deposit(amount).await;

    table.add_row(row![
        amount,
        yield_farm_asset.name(),
        yield_farm.name(),
        yield_farm_asset_decimals
    ]);
    table.printstd();
}

pub async fn call_run_cmd(args: &ArgMatches) {
    let mut table = Table::new();
    let force_harvest = cmd::helpers::get_force_harvest(args);
    table.add_row(row![
        "Harvested",
        "Farm",
        "Pending rewards",
        "Asset",
        "Decimals",
        "Min rewards required"
    ]);
    for yield_farm in get_farms_to_look(args) {
        let yield_farm_asset = yield_farm.get_reward_asset().unwrap();
        let yield_farm_asset_decimals = yield_farm_asset.decimals().await;

        let pending_rewards = yield_farm.get_pending_rewards().await;
        let min_rewards_required =
            yield_farm.get_min_rewards_required_u256(yield_farm_asset_decimals);
        log::info!("yield_farm pending rewards: {:?}", pending_rewards);
        log::info!(
            "yield_farm min rewards required: {:?}",
            min_rewards_required
        );

        let can_harvest = pending_rewards >= min_rewards_required;
        if can_harvest || force_harvest {
            log::info!("harvesting yield farm: {:?}", yield_farm);
            yield_farm.harvest().await;
        }

        table.add_row(row![
            (can_harvest || force_harvest),
            yield_farm.name(),
            pending_rewards,
            yield_farm_asset.name(),
            yield_farm_asset_decimals,
            min_rewards_required
        ]);
    }
    table.printstd();
}

pub async fn call_sub_commands(args: &ArgMatches) {
    match args.subcommand() {
        Some((YIELD_FARM_RUN_COMMAND, sub_args)) => {
            call_run_cmd(sub_args).await;
        }

        Some((YIELD_FARM_INFO_COMMAND, sub_args)) => {
            call_info_cmd(sub_args).await;
        }
        Some((YIELD_FARM_DEPOSIT_COMMAND, sub_args)) => {
            call_deposit_cmd(sub_args).await;
        }
        _ => {
            log::error!("no sub cmd found");
            panic!("sub_cmd_not_found");
        }
    }
}

#[derive(Debug, Clone)]
pub struct YieldFarmInfoCmdOutput {
    deposit_asset_name: String,
    reward_asset_name: String,
    deposit_asset_decimals: u8,
    reward_asset_decimals: u8,
    network_id: String,
    yield_farm_name: String,
    deposited_amount: U256,
    deposited_amount_in_quoted: U256,
    pending_rewards: U256,
    yield_farm_asset_name: String,
    quoted_price: U256,
    quoted_asset_name: String,
    yield_farm_asset_decimals: u8,
    quoted_asset_decimal: u8,
    min_rewards_required: U256,
}

// impl YieldFarmInfoCmdOutput {
//     pub fn new() -> YieldFarmInfoCmdOutput {

//     }
// }

// table.add_row(row![
//     yield_farm.network_id(),
//     yield_farm.name(),
//     shared::blockchain_utils::display_amount_to_float(
//         deposited_amount,
//         yield_farm_asset_decimals
//     ),
//     shared::blockchain_utils::display_amount_to_float(
//         pending_rewards,
//         yield_farm_asset_decimals
//     ),
//     yield_farm_asset.name(),
//     shared::blockchain_utils::display_amount_to_float(quoted_price, quoted_asset_decimal),
//     quoted_asset.name(),
//     yield_farm_asset_decimals,
//     min_rewards_required
// ]);
