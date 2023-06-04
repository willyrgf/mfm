use clap::{ArgMatches, Command};

// TODO: refactor flagable args for the command like hide-zero to use arg.get_flag()
pub fn generate() -> Command {
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
pub async fn call_sub_commands(args: &ArgMatches) -> Result<(), anyhow::Error> {
    super::run(args).await
}
