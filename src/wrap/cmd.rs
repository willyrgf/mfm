use clap::{ArgMatches, Command};

pub fn generate<'a>() -> Command<'a> {
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
        clap::arg!(-a --"amount" <AMMOUNT> "Amount to wrap coin into token, default: (balance-min_balance_coin)")
            .required(false),
    )
}

#[tracing::instrument(name = "wrap call command")]
pub async fn call_sub_commands(args: &ArgMatches) {
    super::run(args).await.unwrap_or_else(|e| {
        tracing::error!(error = %e);
        panic!()
    });
}
