use clap::{ArgMatches, Command};

pub fn generate() -> Command {
    Command::new("quote")
        .about("Get a quote for tokens to tokens")
        .arg(clap::arg!(-n --"network" <bsc> "Network to use, ex (bsc, polygon)").required(true))
        .arg(clap::arg!(-e --"exchange" <pancake_swap_v2> "Exchange to use router").required(false))
        .arg(
            clap::arg!(-a --"amount" <AMMOUNT> "Amount of TokenA to swap to TokenB")
                .required(false),
        )
        .arg(clap::arg!(-i --"token_input" <TOKEN_INPUT> "Asset of input token").required(false))
        .arg(clap::arg!(-o --"token_output" <TOKEN_OUTPUT> "Asset of output token").required(false))
        .arg(
            clap::arg!(-s --"slippage" <SLIPPAGE> "Slippage (default 0.5)")
                .required(false)
                .default_value("0.5")
                .value_parser(clap::value_parser!(f64)),
        )
}

#[tracing::instrument(name = "quote call command")]
pub async fn call_sub_commands(args: &ArgMatches) -> Result<(), anyhow::Error> {
    super::run(args).await
}
