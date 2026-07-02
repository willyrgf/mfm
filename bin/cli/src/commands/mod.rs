use clap::{Parser, Subcommand, ValueEnum};
use std::ffi::OsStr;

/// Public fact query commands.
mod facts;
/// Keystore-oriented CLI commands.
mod keystore;
/// Shared command result types.
pub(crate) mod result;
/// Run lifecycle and artifact commands.
mod run;

pub(crate) const OUTPUT_FORMAT_ENV: &str = "MFM_OUTPUT_FORMAT";
const DEFAULT_OUTPUT_FORMAT: OutputFormat = OutputFormat::Text;

/// Output encodings supported by the CLI library.
#[derive(Debug, Clone, Copy, ValueEnum, Default)]
pub(crate) enum OutputFormat {
    /// Human-readable text output (default)
    #[default]
    Text,
    /// Machine-readable JSON output
    Json,
}

impl OutputFormat {
    pub(crate) fn is_json(self) -> bool {
        matches!(self, Self::Json)
    }

    fn from_cli_value(value: &str) -> Option<Self> {
        <Self as ValueEnum>::from_str(value, true).ok()
    }

    fn from_env_value(value: Option<&OsStr>) -> Self {
        value
            .and_then(OsStr::to_str)
            .and_then(Self::from_cli_value)
            .unwrap_or(DEFAULT_OUTPUT_FORMAT)
    }
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
    let fallback_format = OutputFormat::from_env_value(env_output_format);
    let mut iter = args.into_iter();

    while let Some(arg) = iter.next() {
        let Some(arg) = arg.as_ref().to_str() else {
            continue;
        };
        if arg == "--" {
            break;
        }

        if let Some(value) = arg.strip_prefix("--output-format=") {
            return OutputFormat::from_cli_value(value).unwrap_or(fallback_format);
        }

        if arg == "--output-format" {
            return iter
                .next()
                .and_then(|next| {
                    next.as_ref()
                        .to_str()
                        .and_then(OutputFormat::from_cli_value)
                })
                .unwrap_or(fallback_format);
        }
    }

    fallback_format
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
    #[arg(long = "output-format", value_enum, env = OUTPUT_FORMAT_ENV, default_value_t = DEFAULT_OUTPUT_FORMAT)]
    output_format: OutputFormat,

    /// Top-level command selected by the caller.
    #[command(subcommand)]
    command: Commands,
}

/// Top-level CLI command tree.
#[derive(Subcommand)]
enum Commands {
    /// Public fact discovery and query operations
    Facts {
        /// Nested facts command to execute.
        #[command(subcommand)]
        command: facts::FactsCommand,
    },
    /// Keystore management operations
    Keystore {
        /// Nested keystore command to execute.
        #[command(subcommand)]
        command: keystore::KeystoreCommand,
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
        let ctx = CommandContext::new(self.output_format);

        match &self.command {
            Commands::Facts { command } => command.execute(&ctx).await,
            Commands::Keystore { command } => command.execute(&ctx).await,
            Commands::Run { command } => command.execute(&ctx).await,
        }
    }
}
