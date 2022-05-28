use crate::{
    cmd,
    rebalancer::{self, config::Strategy},
};
use clap::{ArgMatches, Command};

pub const REBALANCER_COMMAND: &str = "rebalancer";

pub fn generate_cmd<'a>() -> Command<'a> {
    Command::new(REBALANCER_COMMAND)
        .about("Fires a rebalancer")
        .arg(
            clap::arg!(-n --"name" <REBALANCER_NAME> "Rebalancer name from config file")
                .required(true),
        )
}

pub async fn call_sub_commands(args: &ArgMatches) {
    let config = cmd::helpers::get_rebalancer(args);
    log::debug!(
        "rebalancer::cmd::call_sub_commands(): rebalancer_config: {:?}",
        config
    );

    rebalancer::validate(config).await;

    match config.strategy() {
        Strategy::FullParking => {
            log::debug!("rebalancer::cmd::call_sub_commands() Strategy::FullParking");
            rebalancer::run_full_parking(config).await;
        }
        Strategy::DiffParking => {
            log::debug!("rebalancer::cmd::call_sub_commands() Strategy::DiffParking");
            rebalancer::run_diff_parking(config).await;
        }
    }
}
