use clap::{Parser, Subcommand, ValueEnum};
use std::ffi::OsStr;

/// EVM workflow commands.
mod evm;
/// Keystore-oriented CLI commands.
mod keystore;
/// Portfolio feature commands.
mod portfolio;
/// Shared command result types.
pub(crate) mod result;
/// Run lifecycle and artifact commands.
mod run;

/// Output encodings supported by the CLI library.
#[derive(Debug, Clone, ValueEnum, Default)]
pub(crate) enum OutputFormat {
    /// Human-readable text output (default)
    #[default]
    Text,
    /// Machine-readable JSON output
    Json,
}

/// Detects the caller's requested output format when clap parsing itself failed.
pub(crate) fn detect_requested_output_format_for_parse_error<I, S>(
    args: I,
    env_output_format: Option<&OsStr>,
) -> OutputFormat
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let env_requests_json = env_output_format
        .and_then(OsStr::to_str)
        .map(|value| value.eq_ignore_ascii_case("json"))
        .unwrap_or(false);
    let mut iter = args.into_iter();

    while let Some(arg) = iter.next() {
        let Some(arg) = arg.as_ref().to_str() else {
            continue;
        };
        if arg == "--" {
            break;
        }

        if let Some(value) = arg.strip_prefix("--output-format=") {
            return output_format_from_parse_value(value).unwrap_or({
                if env_requests_json {
                    OutputFormat::Json
                } else {
                    OutputFormat::Text
                }
            });
        }

        if arg == "--output-format" {
            let value = iter.next().and_then(|next| {
                next.as_ref()
                    .to_str()
                    .map(output_format_from_parse_value)
                    .unwrap_or(None)
            });
            return value.unwrap_or({
                if env_requests_json {
                    OutputFormat::Json
                } else {
                    OutputFormat::Text
                }
            });
        }
    }

    if env_requests_json {
        OutputFormat::Json
    } else {
        OutputFormat::Text
    }
}

fn output_format_from_parse_value(value: &str) -> Option<OutputFormat> {
    match value {
        value if value.eq_ignore_ascii_case("json") => Some(OutputFormat::Json),
        value if value.eq_ignore_ascii_case("text") => Some(OutputFormat::Text),
        _ => None,
    }
}

/// Context passed to CLI commands containing shared output settings.
#[derive(Debug, Clone)]
struct CommandContext {
    /// Output format requested by the caller.
    output_format: OutputFormat,
}

impl CommandContext {
    /// Builds a command context for the supplied output format.
    fn new(output_format: OutputFormat) -> Self {
        Self { output_format }
    }
}

/// Root CLI parser for the `mfm` binary.
#[derive(Parser)]
#[command(name = "mfm")]
#[command(about = "MFM - On-chain operations tool")]
#[command(version)]
pub(crate) struct Cli {
    /// Output format for command results
    #[arg(long = "output-format", value_enum, env = "MFM_OUTPUT_FORMAT", default_value_t = OutputFormat::Text)]
    output_format: OutputFormat,

    /// Top-level command selected by the caller.
    #[command(subcommand)]
    command: Commands,
}

/// Top-level CLI command tree.
#[derive(Subcommand)]
enum Commands {
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
    /// EVM lifecycle workflow operations
    Evm {
        /// Nested EVM command to execute.
        #[command(subcommand)]
        command: evm::EvmCommand,
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
    pub(crate) async fn execute(&self) -> ! {
        let ctx = CommandContext::new(self.output_format.clone());

        match &self.command {
            Commands::Keystore { command } => {
                command.execute(&ctx).await;
            }
            Commands::Portfolio { command } => {
                command.execute(&ctx).await;
            }
            Commands::Evm { command } => {
                command.execute(&ctx).await;
            }
            Commands::Run { command } => {
                command.execute(&ctx).await;
            }
        }
    }
}
