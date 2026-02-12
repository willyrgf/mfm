use clap::{Parser, Subcommand, ValueEnum};

pub mod keystore;
pub mod portfolio;
pub mod result;
pub mod run;

#[derive(Debug, Clone, ValueEnum, Default)]
pub enum OutputFormat {
    /// Human-readable text output (default)
    #[default]
    Text,
    /// Machine-readable JSON output
    Json,
}

/// Context passed to all CLI commands containing shared configuration and state
#[derive(Debug, Clone)]
pub struct CommandContext {
    pub output_format: OutputFormat,
}

impl CommandContext {
    pub fn new(output_format: OutputFormat) -> Self {
        Self { output_format }
    }
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
    /// Portfolio monitoring operations
    Portfolio {
        #[command(subcommand)]
        command: portfolio::PortfolioCommand,
    },
    /// Run operations (start/resume/inspect)
    Run {
        #[command(subcommand)]
        command: run::RunCommand,
    },
}

impl Cli {
    pub async fn execute(&self) -> ! {
        let ctx = CommandContext::new(self.output_format.clone());

        match &self.command {
            Commands::Keystore { command } => {
                command.execute(&ctx).await;
            }
            Commands::Portfolio { command } => {
                command.execute(&ctx).await;
            }
            Commands::Run { command } => {
                command.execute(&ctx).await;
            }
        }
    }
}
