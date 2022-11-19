use clap::{ArgMatches, Command};

pub fn generate() -> Command {
    Command::new("withdraw")
    .about("Withdraw to a wallet")
    .arg(
        clap::arg!(-w --"wallet" <WALLET_NAME> "Wallet id from config file")
            .required(true),
    )
    .arg(clap::arg!(-n --"network" <bsc> "Network to use, ex (bsc, polygon)").required(true))
    .arg(
        clap::arg!(-t --"withdraw-wallet" <WITHDRAW_WALLET_NAME> "Withdraw wallet to receive the transfer")
            .required(true),
    )
    .arg(
        clap::arg!(-a --"asset" <ASSET> "Asset to withdraw")
            .required(true)
    )
    .arg(
        clap::arg!(-v --"amount" <VALUE> "Amount to withdraw")
            .required(true)
            .value_parser(clap::value_parser!(f64))
    )
}

#[tracing::instrument(name = "withdraw call command")]
pub async fn call_sub_commands(args: &ArgMatches) -> Result<(), anyhow::Error> {
    super::run(args).await
}
