use clap::{Parser, Subcommand, ValueEnum};

/// Keystore-oriented CLI commands.
pub mod keystore;
/// Portfolio feature commands.
pub mod portfolio;
/// Shared command result types.
pub mod result;
/// Run lifecycle and artifact commands.
pub mod run;

/// Output encodings supported by the CLI library.
#[derive(Debug, Clone, ValueEnum, Default)]
pub enum OutputFormat {
    /// Human-readable text output (default)
    #[default]
    Text,
    /// Machine-readable JSON output
    Json,
}

/// Context passed to CLI commands containing shared output settings.
#[derive(Debug, Clone)]
pub struct CommandContext {
    /// Output format requested by the caller.
    pub output_format: OutputFormat,
}

impl CommandContext {
    /// Builds a command context for the supplied output format.
    pub fn new(output_format: OutputFormat) -> Self {
        Self { output_format }
    }
}

/// Root CLI parser for the `mfm` binary.
#[derive(Parser)]
#[command(name = "mfm")]
#[command(about = "MFM - On-chain operations tool")]
#[command(version)]
pub struct Cli {
    /// Output format for command results
    #[arg(long = "output-format", value_enum, env = "MFM_OUTPUT_FORMAT", default_value_t = OutputFormat::Text)]
    pub output_format: OutputFormat,

    /// Top-level command selected by the caller.
    #[command(subcommand)]
    pub command: Commands,
}

/// Top-level CLI command tree.
#[derive(Subcommand)]
pub enum Commands {
    /// Keystore management operations
    Keystore {
        /// Nested keystore command to execute.
        #[command(subcommand)]
        command: keystore::KeystoreCommand,
    },
    /// Portfolio monitoring operations
    Portfolio {
        /// Nested portfolio command to execute.
        #[command(subcommand)]
        command: portfolio::PortfolioCommand,
    },
    /// Run operations (start/resume/inspect)
    Run {
        /// Nested run command to execute.
        #[command(subcommand)]
        command: run::RunCommand,
    },
}

impl Cli {
    /// Dispatches the parsed command and terminates the process with the command's exit code.
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
