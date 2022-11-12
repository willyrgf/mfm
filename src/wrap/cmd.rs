use clap::{ArgMatches, Command};

pub fn generate() -> Command {
    Command::new("wrap")
    .about("Wrap a coin to a token")
    .arg(
        clap::arg!(-n --"network" <bsc> "Network to wrap coin to token")
            .required(true),
    )
    .arg(
        clap::arg!(-w --"wallet" <WALLET_NAME> "Wallet id from config file")
            .required(true),
    )
    .arg(
        clap::arg!(-a --"amount" <AMOUNT> "Amount to wrap coin into token, default: (balance-min_balance_coin)")
            .required(false)
            .value_parser(clap::value_parser!(f64)),
    )
}

#[tracing::instrument(name = "wrap call command")]
pub async fn call_sub_commands(args: &ArgMatches) -> Result<(), anyhow::Error> {
    super::run(args).await
}
