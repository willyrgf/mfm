use clap::{ArgMatches, Command};

pub fn generate() -> Command {
    Command::new("watcher")
        .about("Watch an address")
        .arg(
            clap::arg!(-n --"network" <NETWORK> "Network to use, ex (bsc, polygon)").required(true),
        )
        .arg(clap::arg!(-a --"address" <ADDRESS> "Address to watch").required(true))
}

#[tracing::instrument(name = "track call command")]
pub async fn call_sub_commands(args: &ArgMatches) -> Result<(), anyhow::Error> {
    super::run(args).await
}
