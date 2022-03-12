use crate::{cmd, config};
use clap::ArgMatches;

pub const REBALANCER_COMMAND: &'static str = "rebalancer";

pub async fn call_sub_commands(args: &ArgMatches, config: &config::Config) {
    let rebalancer = cmd::get_rebalancer(args, config);
    log::debug!("rebalancer: {:?}", rebalancer);
}
