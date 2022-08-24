use clap::{ArgMatches, Command};

pub fn generate<'a>() -> Command<'a> {
    Command::new("unwrap")
        .about("Unwrap a wrapped coin to coin")
        .arg(clap::arg!(-n --"network" <bsc> "Network to unwrap token to coin").required(true))
        .arg(clap::arg!(-w --"wallet" <WALLET_NAME> "Wallet id from config file").required(true))
        .arg(clap::arg!(-a --"amount" <AMMOUNT> "Amount to unwrap token into coin").required(false))
}

#[tracing::instrument(name = "unwrap call command")]
pub async fn call_sub_commands(args: &ArgMatches) {
    super::run(args).await.unwrap_or_else(|e| {
        tracing::error!(error = %e);
        panic!()
    });
}
