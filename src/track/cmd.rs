use clap::{ArgMatches, Command};

pub fn generate() -> Command {
    Command::new("track").about("Track all information to server")
}

#[tracing::instrument(name = "track call command")]
pub async fn call_sub_commands(args: &ArgMatches) -> Result<(), anyhow::Error> {
    super::run(args).await
}
