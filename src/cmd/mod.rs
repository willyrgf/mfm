use clap::{ArgEnum, Command};

// clap::arg_enum!(
//     #[derive(Debug)]
//     pub enum Foo {
//         Bar,
//         Baz,
//         Qx,
//     }
// );

pub struct Wrap {}

impl Runnable for Wrap {
    fn run() {}
}

pub trait Runnable {
    fn run();
}

// pub enum CommandRun<R: Runnable> {
//     Wrap(R),
pub enum CommandRun {
    Wrap(Wrap),
}

pub fn test_new_enum_cmd() {
    let r = CommandRun::Wrap(Wrap {});
}

pub fn new() -> clap::Command<'static> {
    Command::new("mfm")
        .bin_name("mfm")
        .arg(
            clap::arg!(-c - -config_filename <PATH> "Config file path")
                .required(false)
                .default_value("config.yaml"),
        )
        .subcommand_required(true)
        .subcommand(
            Command::new("wrap")
                .about("Wrap a coin to a token")
                .arg(
                    clap::arg!(--"network" <bsc> "Network to wrap coin to token")
                        .required(true),
                )
                .arg(
                    clap::arg!(--"wallet" <WALLET_NAME> "Wallet id from config file")
                        .required(true),
                )
                .arg(
                    clap::arg!(--"amount" <AMMOUNT> "Amount to wrap coin into token, default: (balance-min_balance_coin)")
                        .required(false)
                        ,
                ),
        )
        .subcommand(
            Command::new("swaptt")
                .about("Swap Tokens for Tokens")
                .arg(
                    clap::arg!(-e --"exchange" <pancake_swap_v2> "Exchange to use router")
                        .required(true),
                )
                .arg(
                    clap::arg!(-w --"wallet" <WALLET_NAME> "Wallet id from config file")
                        .required(true),
                )
                .arg(
                    clap::arg!(-a --"amount" <AMMOUNT> "Amount of TokenA to swap to TokenB")
                        .required(false)
                )
                .arg(
                    clap::arg!(-i --"token_input" <TOKEN_INPUT> "Asset of input token")
                        .required(false)
                )
                .arg(
                    clap::arg!(-o --"token_output" <TOKEN_OUTPUT> "Asset of output token")
                        .required(false)
                ),

        )
}
