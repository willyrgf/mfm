use clap::{ArgMatches, Command};

pub fn generate() -> Command {
    Command::new("swap")
        .about("Swap Tokens for Tokens supporting fees on transfer")
        .arg(clap::arg!(-n --"network" <bsc> "Network to use, ex (bsc, polygon)").required(true))
        .arg(clap::arg!(-e --"exchange" <pancake_swap_v2> "Exchange to use router. If not provided, try to use best liquidity exchange").required(false))
        .arg(clap::arg!(-w --"wallet" <WALLET_NAME> "Wallet id from config file").required(true))
        .arg(
            clap::arg!(-a --"amount" <AMMOUNT> "Amount of TokenA to swap to TokenB")
                .required(false)
                .value_parser(clap::value_parser!(f64)),
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

#[tracing::instrument(name = "swap call command")]
pub async fn call_sub_commands(args: &ArgMatches) -> Result<(), anyhow::Error> {
    super::run(args).await
}
