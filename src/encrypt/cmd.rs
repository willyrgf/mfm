use clap::{ArgMatches, Command};

pub fn generate() -> Command {
    Command::new("encrypt").about("Encrypt data with a password")
}

#[tracing::instrument(name = "encrypt call command")]
pub async fn call_sub_commands(args: &ArgMatches) -> Result<(), anyhow::Error> {
    super::run(args).await
}
