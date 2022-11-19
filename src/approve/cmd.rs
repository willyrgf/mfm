use clap::{ArgMatches, Command};

pub fn generate() -> Command {
    Command::new("approve")
        .about("Approve token spending (needed to swap tokens)")
        //TODO: add a custom spender arg to add another spenders lide yield-farms
        .arg(
            clap::arg!(-e --"exchange" <pancake_swap_v2> "Exchange to use router as spender")
                .required(true),
        )
        .arg(clap::arg!(-w --"wallet" <WALLET_NAME> "Wallet id from config file").required(true))
        .arg(clap::arg!(-a --"asset" <ASSET> "Asset to approve spender").required(true))
        .arg(
            clap::arg!(-v --"amount" <VALUE> "Amount to allow spending")
                .required(true)
                .value_parser(clap::value_parser!(f64)),
        )
}

#[tracing::instrument(name = "approve call command")]
pub async fn call_sub_commands(args: &ArgMatches) -> Result<(), anyhow::Error> {
    super::run(args).await
}
