use clap::{ArgMatches, Command};

pub fn generate() -> Command {
    Command::new("unwrap")
        .about("Unwrap a wrapped coin to coin")
        .arg(clap::arg!(-n --"network" <bsc> "Network to unwrap token to coin").required(true))
        .arg(clap::arg!(-w --"wallet" <WALLET_NAME> "Wallet id from config file").required(true))
        .arg(clap::arg!(-a --"amount" <AMMOUNT> "Amount to unwrap token into coin").required(false))
}

#[tracing::instrument(name = "unwrap call command")]
pub async fn call_sub_commands(args: &ArgMatches) -> Result<(), anyhow::Error> {
    super::run(args).await
}
