use clap::{ArgMatches, Command};

pub fn generate() -> Command {
    Command::new("track")
        .about("Track all information to server")
        .arg(
            clap::arg!(-s --"run-every" <TIME_IN_SECONDS> "Run continuously at every number of seconds")
            .value_parser(clap::value_parser!(u32)),
        )
}

#[tracing::instrument(name = "track call command")]
pub async fn call_sub_commands(args: &ArgMatches) -> Result<(), anyhow::Error> {
    super::run(args).await
}
