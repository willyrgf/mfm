use crate::{
    allowance, approve, balances, encrypt, quote, rebalancer, swap, track, unwrap, withdraw, wrap, watcher,
};
use crate::{config::Config, APP_NAME};
use clap::{crate_version, ArgMatches, Command};
use serde::{Deserialize, Serialize};

pub mod helpers;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Commands {
    Balances,
    Wrap,
    Unwrap,
    Swap,
    Allowance,
    Approve,
    Rebalancer,
    Withdraw,
    Quote,
    Track,
    Encrypt,
    Watcher,
}

impl Commands {
    pub async fn run(&self, args: &ArgMatches) -> Result<(), anyhow::Error> {
        match &self {
            Self::Balances => balances::cmd::call_sub_commands(args).await,
            Self::Wrap => wrap::cmd::call_sub_commands(args).await,
            Self::Unwrap => unwrap::cmd::call_sub_commands(args).await,
            Self::Swap => swap::cmd::call_sub_commands(args).await,
            Self::Allowance => allowance::cmd::call_sub_commands(args).await,
            Self::Approve => approve::cmd::call_sub_commands(args).await,
            Self::Rebalancer => rebalancer::cmd::call_sub_commands(args).await,
            Self::Withdraw => withdraw::cmd::call_sub_commands(args).await,
            Self::Encrypt => encrypt::cmd::call_sub_commands(args).await,
            Self::Quote => quote::cmd::call_sub_commands(args).await,
            Self::Track => track::cmd::call_sub_commands(args).await,
            Self::Watcher => watcher::cmd::call_sub_commands(args).await,
        }
    }
}

pub fn new() -> Command {
    Command::new(APP_NAME)
        .bin_name(APP_NAME)
        .version(crate_version!())
        .arg(
            clap::arg!(-c - -config_filename <PATH> "Config file path")
                .required(false)
                .default_value("config.yaml"),
        )
        .subcommand_required(true)
        .subcommand(wrap::cmd::generate())
        .subcommand(unwrap::cmd::generate())
        .subcommand(swap::cmd::generate())
        .subcommand(allowance::cmd::generate())
        .subcommand(approve::cmd::generate())
        .subcommand(balances::cmd::generate())
        .subcommand(rebalancer::cmd::generate())
        .subcommand(encrypt::cmd::generate())
        .subcommand(withdraw::cmd::generate())
        .subcommand(quote::cmd::generate())
        .subcommand(track::cmd::generate())
        .subcommand(watcher::cmd::generate())
}

#[tracing::instrument(name = "lookup command from cli")]
pub fn lookup_command(cmd: &str) -> Result<Commands, anyhow::Error> {
    let json_cmd = format!("\"{}\"", cmd);
    serde_json::from_str(json_cmd.as_str()).map_err(|e| anyhow::anyhow!(e))
}

#[tracing::instrument(name = "call commands")]
pub async fn call_sub_commands(matches: &ArgMatches) -> Result<(), anyhow::Error> {
    match matches.subcommand() {
        Some((cmd, sub_matches)) => lookup_command(cmd)?.run(sub_matches).await,
        _ => Err(anyhow::anyhow!("subcommand is required")),
    }
}

#[tracing::instrument(name = "cli run command", skip(cmd))]
pub async fn run(cmd: Command) -> Result<(), anyhow::Error> {
    let cmd_matches = cmd.get_matches();

    match cmd_matches.get_one::<String>("config_filename") {
        Some(f) => Config::from_file(f)?,
        None => return Err(anyhow::anyhow!("--config_filename is invalid")),
    };

    call_sub_commands(&cmd_matches).await.map_err(|e| {
        tracing::error!(error = %e);
        anyhow::anyhow!("call subcommand failed, err: {}", e)
    })
}
