use clap::{Parser, Subcommand, ValueEnum};

pub mod keystore;
pub mod utils;

#[derive(Debug, Clone, ValueEnum, Default)]
pub enum OutputFormat {
    /// Human-readable text output (default)
    #[default]
    Text,
    /// Machine-readable JSON output
    Json,
}

#[derive(Parser)]
#[command(name = "mfm")]
#[command(about = "MFM - On-chain operations tool")]
#[command(version)]
pub struct Cli {
    /// Output format for command results
    #[arg(long = "output-format", value_enum, env = "MFM_OUTPUT_FORMAT", default_value_t = OutputFormat::Text)]
    pub output_format: OutputFormat,

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
                command.execute(&self.output_format).await?;
            }
        }
        Ok(())
    }
}
