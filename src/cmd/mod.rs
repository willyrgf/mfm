use crate::{balances, quote, rebalancer, unwrap, wrap};
use crate::{config::Config, APP_NAME};
use clap::{crate_version, ArgMatches, Command};
use serde::{Deserialize, Serialize};

pub mod allowance;
pub mod approve;
pub mod helpers;
pub mod swap;
pub mod track;
pub mod withdraw;

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
}

impl Commands {
    pub async fn run(&self, args: &ArgMatches) {
        match &self {
            Self::Balances => balances::cmd::call_sub_commands(args).await,
            Self::Wrap => wrap::cmd::call_sub_commands(args).await,
            Self::Unwrap => unwrap::cmd::call_sub_commands(args).await,
            Self::Swap => swap::call_sub_commands(args).await,
            Self::Allowance => allowance::call_sub_commands(args).await,
            Self::Approve => approve::call_sub_commands(args).await,
            Self::Rebalancer => rebalancer::cmd::call_sub_commands(args).await,
            Self::Withdraw => withdraw::call_sub_commands(args).await,
            Self::Quote => quote::cmd::call_sub_commands(args).await,
            Self::Track => track::call_sub_commands(args).await,
        }
    }
}

pub fn new() -> clap::Command<'static> {
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
        .subcommand(swap::generate_cmd())
        .subcommand(allowance::generate_cmd())
        .subcommand(approve::generate_cmd())
        .subcommand(balances::cmd::generate())
        .subcommand(rebalancer::cmd::generate())
        .subcommand(withdraw::generate_cmd())
        .subcommand(quote::cmd::generate())
        .subcommand(track::generate_cmd())
}

#[tracing::instrument(name = "lookup command from cli")]
pub fn lookup_command(cmd: &str) -> Result<Commands, anyhow::Error> {
    let json_cmd = format!("\"{}\"", cmd);
    serde_json::from_str(json_cmd.as_str()).map_err(|e| anyhow::anyhow!(e))
}

#[tracing::instrument(name = "call commands")]
pub async fn call_sub_commands(matches: &ArgMatches) -> Result<(), anyhow::Error> {
    match matches.subcommand() {
        Some((cmd, sub_matches)) => {
            lookup_command(cmd)?.run(sub_matches).await;
            Ok(())
        }
        _ => Err(anyhow::anyhow!("subcommand is required")),
    }
}

#[tracing::instrument(name = "cli run command", skip(cmd))]
pub async fn run(cmd: clap::Command<'static>) -> Result<(), anyhow::Error> {
    let cmd_matches = cmd.get_matches();

    match cmd_matches.value_of("config_filename") {
        Some(f) => Config::from_file(f)?,
        None => return Err(anyhow::anyhow!("--config_filename is invalid")),
    };

    match call_sub_commands(&cmd_matches).await {
        Ok(_) => Ok(()),
        Err(e) => Err(anyhow::anyhow!("call subcommand failed, err: {}", e)),
    }
}
