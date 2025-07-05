use clap::{Parser, Subcommand};

pub mod keystore;
pub mod utils;

#[derive(Parser)]
#[command(name = "mfm")]
#[command(about = "MFM - On-chain operations tool")]
#[command(version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Keystore management operations
    Keystore {
        #[command(subcommand)]
        command: keystore::KeystoreCommand,
    },
}

impl Cli {
    pub async fn execute(&self) -> Result<(), Box<dyn std::error::Error>> {
        match &self.command {
            Commands::Keystore { command } => {
                command.execute().await?;
            }
        }
        Ok(())
    }
}
