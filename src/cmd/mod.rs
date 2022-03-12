use crate::config::{asset::Asset, exchange::Exchange, network::Network, wallet::Wallet, Config};
use clap::{ArgMatches, Command};
use web3::{transports::Http, Web3};

pub mod allowance;
pub mod approve;
pub mod swap;
pub mod wrap;

pub struct CmdState {
    config: &Config,
    exchange: Option(&Exchange),
    network: Option(&Network),
    client: Option(&Web3<Http>),
    wallet: Option(&Wallet),
    asset: Option(&Asset),
}

impl CmdState {
    pub fn from_args(args: &ArgMatches, config: &Config) -> Self {
        let exchange_obj = match args.value_of("exchange") {
            Some(n) => Some(config.exchanges.get(n)),
            None => None,
        };
        log::debug!("load_exchange_to: {:?}", exchange_obj);
        let network_obj = Some(exchange_obj.get_network(&config.networks));

        let client =
            Some(Web3::new(web3::transports::Http::new(network_obj.rpc_url()).unwrap()).to_owned());

        let asset_obj = match args.value_of("asset") {
            Some(i) => Some(config.assets.get(i)),
            None => panic!("--asset not supported"),
        };
        let wallet_obj = match args.value_of("wallet") {
            Some(w) => Some(config.wallets.get(w)),
            None => panic!("--wallet doesnt exist"),
        };

        CmdState {
            config: config,
            exchange: exchange_obj,
            network: network_obj,
            client: client,
            wallet: wallet_obj,
            asset: asset_obj,
        }
    }
}

pub const CLI_NAME: &'static str = "mfm";

pub fn new() -> clap::Command<'static> {
    Command::new(CLI_NAME)
        .bin_name(CLI_NAME)
        .arg(
            clap::arg!(-c - -config_filename <PATH> "Config file path")
                .required(false)
                .default_value("config.yaml"),
        )
        .subcommand_required(true)
        .subcommand(
            Command::new(wrap::WRAP_COMMAND)
                .about("Wrap a coin to a token")
                .arg(
                    clap::arg!(--"network" <bsc> "Network to wrap coin to token")
                        .required(true),
                )
                .arg(
                    clap::arg!(--"wallet" <WALLET_NAME> "Wallet id from config file")
                        .required(true),
                )
                .arg(
                    clap::arg!(--"amount" <AMMOUNT> "Amount to wrap coin into token, default: (balance-min_balance_coin)")
                        .required(false)
                        ,
                ),
        )
        .subcommand(
            Command::new(swap::SWAP_COMMAND)
                .about("Swap Tokens for Tokens supporting fees on transfer")
                .arg(
                    clap::arg!(-e --"exchange" <pancake_swap_v2> "Exchange to use router")
                        .required(true),
                )
                .arg(
                    clap::arg!(-w --"wallet" <WALLET_NAME> "Wallet id from config file")
                        .required(true),
                )
                .arg(
                    clap::arg!(-a --"amount" <AMMOUNT> "Amount of TokenA to swap to TokenB")
                        .required(false)
                )
                .arg(
                    clap::arg!(-i --"token_input" <TOKEN_INPUT> "Asset of input token")
                        .required(false)
                )
                .arg(
                    clap::arg!(-o --"token_output" <TOKEN_OUTPUT> "Asset of output token")
                        .required(false)
                )
                .arg(
                    clap::arg!(-s --"slippage" <SLIPPAGE> "Slippage (default 0.5)")
                        .required(false)
                        .default_value("0.5")
                )
        )
        .subcommand(
            Command::new("allowance")
                .about("Get allowance for an token")
                .arg(
                    clap::arg!(-e --"exchange" <pancake_swap_v2> "Exchange to use router")
                        .required(true),
                )
                .arg(
                    clap::arg!(-w --"wallet" <WALLET_NAME> "Wallet id from config file")
                        .required(true),
                )
                .arg(
                    clap::arg!(-a --"asset" <ASSET> "Asset to check allowance")
                        .required(true)
                )
        )
        .subcommand(
            Command::new("approve_spender")
                .about("Approve token spending (needed to swap tokens)")
                .arg(
                    clap::arg!(-e --"exchange" <pancake_swap_v2> "Exchange to use router as spender")
                        .required(true),
                )
                .arg(
                    clap::arg!(-w --"wallet" <WALLET_NAME> "Wallet id from config file")
                        .required(true),
                )
                .arg(
                    clap::arg!(-a --"asset" <ASSET> "Asset to approve spender")
                        .required(true)
                )
                .arg(
                    clap::arg!(-v --"value" <VALUE> "Value to allow spending")
                        .required(true)
                )
        )
}

pub async fn call_sub_commands(matches: &ArgMatches, config: &Config) {
    match matches.subcommand() {
        Some((wrap::WRAP_COMMAND, sub_matches)) => {
            wrap::call_sub_commands(sub_matches, config).await;
        }
        Some((swap::SWAP_COMMAND, sub_matches)) => {
            swap::call_sub_commands(sub_matches, config).await;
        }
        Some((allowance::ALLOWANCE_COMMAND, sub_matches)) => {
            allowance::call_sub_commands(sub_matches, config).await;
        }
        Some((approve::APPROVE_COMMAND, sub_matches)) => {
            approve::call_sub_commands(sub_matches, config).await;
        }
        _ => panic!("command not registred"),
    }
}

pub async fn run(cmd: clap::Command<'static>) {
    let cmd_matches = cmd.get_matches();
    log::debug!("matches: {:?}", cmd_matches);

    let config = match cmd_matches.value_of("config_filename") {
        Some(f) => Config::from_file(f),
        None => panic!("--config_filename is invalid"),
    };

    call_sub_commands(&cmd_matches, &config).await
}

pub fn get_exchange_client_wallet_asset<'a>(
    args: &'a ArgMatches,
    config: &'a Config,
) -> (&'a Exchange, Web3<Http>, &'a Wallet, &'a Asset) {
    let exchange_obj = match args.value_of("exchange") {
        Some(n) => config.exchanges.get(n),
        None => panic!("--exchange not supported"),
    };
    log::debug!("exchange: {:?}", exchange_obj);
    let network_obj = exchange_obj.get_network(&config.networks);

    //TODO: understand better the borrowed that happens when try return &Web3<Http>
    let client = Web3::new(web3::transports::Http::new(network_obj.rpc_url()).unwrap()).to_owned();

    let asset_obj = match args.value_of("asset") {
        Some(i) => config.assets.get(i),
        None => panic!("--asset not supported"),
    };
    let wallet_obj = match args.value_of("wallet") {
        Some(w) => config.wallets.get(w),
        None => panic!("--wallet doesnt exist"),
    };
    (exchange_obj, client.clone(), wallet_obj, asset_obj)
}

pub fn get_exchange_client_wallet_asset_network<'a>(
    args: &'a ArgMatches,
    config: &'a Config,
) -> (&'a Exchange, Web3<Http>, &'a Wallet, &'a Asset, &'a Network) {
    let network = match args.value_of("network") {
        Some(n) => config.networks.get(n),
        None => panic!("--network not supported"),
    };

    let (e, c, w, a) = get_exchange_client_wallet_asset(args, config);
    (e, c, w, a, network)
}
