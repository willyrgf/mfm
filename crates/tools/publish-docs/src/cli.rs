use clap::{Args, Parser, Subcommand};

use crate::model::OutputFormat;

/// CLI parser for the `mfm-publish-docs` binary.
#[derive(Debug, Parser)]
#[command(name = "mfm-publish-docs")]
#[command(about = "Planner, reconciler, and lifecycle tool for the docs.rs publish wave")]
#[command(
    after_help = "Logging: use LOG_LEVEL or RUST_LOG to set the filter. Use LOG_FORMAT for text or json logs."
)]
#[command(version)]
pub(crate) struct Cli {
    /// Output format for command responses.
    #[arg(
        long = "output-format",
        value_enum,
        env = "MFM_OUTPUT_FORMAT",
        default_value_t = OutputFormat::Text
    )]
    pub output_format: OutputFormat,

    /// Top-level command to execute.
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Allow dirty working trees and forward `--allow-dirty` to `cargo publish`.
    #[arg(long)]
    pub allow_dirty: bool,

    /// Start processing from the named package in the wave.
    #[arg(long)]
    pub from: Option<String>,

    /// Process exactly one package from the wave.
    #[arg(long)]
    pub only: Option<String>,
}

/// Top-level subcommands supported by the tool.
#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Build a plan without publishing crates.
    Plan(CommandArgs),
    /// Execute publishable actions.
    Apply(CommandArgs),
    /// Resume a previous apply attempt by reusing its stored selection.
    Resume(ResumeArgs),
    /// Regenerate `crates/docs/README.md` from the desired-state catalog.
    SyncUmbrella(SyncUmbrellaArgs),
    /// Yank an explicitly requested published version.
    Yank(YankArgs),
}

/// Shared arguments used by command variants that only need `--json`.
#[derive(Debug, Clone, Args, Default)]
pub(crate) struct CommandArgs {
    /// Emit machine-readable JSON.
    #[arg(long)]
    pub json: bool,
}

/// Arguments for `resume`.
#[derive(Debug, Clone, Args)]
pub(crate) struct ResumeArgs {
    /// Prior run identifier whose selection should be replayed.
    pub run_id: String,

    /// Emit machine-readable JSON.
    #[arg(long)]
    pub json: bool,
}

/// Arguments for `sync-umbrella`.
#[derive(Debug, Clone, Args)]
pub(crate) struct SyncUmbrellaArgs {
    /// Emit machine-readable JSON.
    #[arg(long)]
    pub json: bool,

    /// Report whether the README is stale without writing it.
    #[arg(long)]
    pub check: bool,
}

/// Arguments for `yank`.
#[derive(Debug, Clone, Args)]
pub(crate) struct YankArgs {
    /// Cargo package name to yank.
    pub package: String,

    /// Version to yank. Defaults to the local workspace version.
    #[arg(long)]
    pub version: Option<String>,

    /// Emit machine-readable JSON.
    #[arg(long)]
    pub json: bool,
}
