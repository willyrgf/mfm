use clap::{ArgMatches, Command};

pub fn generate<'a>() -> Command<'a> {
    Command::new("quote")
        .about("Get a quote for tokens to tokens")
        .arg(clap::arg!(-e --"exchange" <pancake_swap_v2> "Exchange to use router").required(true))
        .arg(
            clap::arg!(-a --"amount" <AMMOUNT> "Amount of TokenA to swap to TokenB")
                .required(false),
        )
        .arg(clap::arg!(-i --"token_input" <TOKEN_INPUT> "Asset of input token").required(false))
        .arg(clap::arg!(-o --"token_output" <TOKEN_OUTPUT> "Asset of output token").required(false))
        .arg(
            clap::arg!(-s --"slippage" <SLIPPAGE> "Slippage (default 0.5)")
                .required(false)
                .default_value("0.5"),
        )
}

#[tracing::instrument(name = "quote call command")]
pub async fn call_sub_commands(args: &ArgMatches) {
    super::run(args).await.unwrap_or_else(|e| {
        tracing::error!(error = %e);
        panic!()
    });
}
