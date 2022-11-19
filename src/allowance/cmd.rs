use clap::{ArgMatches, Command};

pub fn generate() -> Command {
    Command::new("allowance")
        .about("Get allowance for an network and wallet")
        .arg(clap::arg!(-n --"network" <bsc> "Network to use, ex (bsc, polygon)").required(true))
        .arg(clap::arg!(-w --"wallet" <WALLET_NAME> "Wallet id from config file").required(true))
}

#[tracing::instrument(name = "allowance call command")]
pub async fn call_sub_commands(args: &ArgMatches) -> Result<(), anyhow::Error> {
    super::run(args).await
}
