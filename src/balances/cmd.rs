use clap::{ArgMatches, Command};

pub fn generate<'a>() -> Command<'a> {
    Command::new("balances")
        .about("Check balances from all assets listed on config")
        .arg(clap::arg!(-w --"wallet" <WALLET_NAME> "Wallet id from config file").required(true))
        .arg(
            clap::arg!(-z --"hide-zero" <TRUE_FALSE> "hide zero balances")
                .required(false)
                .default_value("false"),
        )
}

#[tracing::instrument(name = "balances call command")]
pub async fn call_sub_commands(args: &ArgMatches) {
    super::run(args).await
}
