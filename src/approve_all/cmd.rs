use clap::{ArgMatches, Command};

pub fn generate() -> Command {
    Command::new("approve-all")
        .about("Approve all configured token spending (needed to swap tokens)")
        .arg(
            clap::arg!(-w --"wallet" <WALLET_NAME> "Wallet id from config file")
                .required(true),
        )
        .arg(
            clap::arg!(-n --"network" <bsc> "Network to run all approvals")
                .required(true),
        )
        .arg(
            clap::arg!(-a --"amount" <VALUE> "Amount to allow spending: default is the current balance")
                .required(false)
                .value_parser(clap::value_parser!(f64)),
        )
}

#[tracing::instrument(name = "approve_all call command", level = "debug")]
pub async fn call_sub_commands(args: &ArgMatches) -> Result<(), anyhow::Error> {
    super::run(args).await
}
