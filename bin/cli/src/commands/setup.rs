use std::fmt;
use std::path::PathBuf;

use crate::commands::result::{CommandError, CommandOutput, CommandResult};
use crate::commands::CommandContext;
use crate::presentation::output::handle_command_result;
use crate::support::run_store::RunStoresArgs;
use clap::{Args, Subcommand};
use mfm_app::SetupConfigPublication;
use serde::Serialize;
use tokio::io::AsyncWriteExt;

/// Current semantic-configuration setup commands.
#[derive(Subcommand)]
pub(crate) enum SetupCommand {
    /// Import a strict TOML setup document into current target-keyed configuration.
    Import {
        #[command(flatten)]
        args: ImportArgs,
    },
    /// List current configuration targets.
    List {
        #[command(flatten)]
        args: ListArgs,
    },
    /// Export one target's current canonical configuration to a new file.
    Export {
        #[command(flatten)]
        args: ExportArgs,
    },
}

/// Arguments for setup import.
#[derive(Args)]
pub(crate) struct ImportArgs {
    /// TOML setup document path.
    #[arg(value_name = "PATH")]
    pub file: PathBuf,
    #[command(flatten)]
    pub stores: RunStoresArgs,
}

/// Arguments for current-target listing.
#[derive(Args)]
pub(crate) struct ListArgs {
    #[command(flatten)]
    pub stores: RunStoresArgs,
}

/// Arguments for current-target export.
#[derive(Args)]
pub(crate) struct ExportArgs {
    /// Stable target whose current configuration should be exported.
    #[arg(value_name = "TARGET")]
    pub target: String,
    /// New output path; existing files are never overwritten.
    #[arg(long)]
    pub output: PathBuf,
    #[command(flatten)]
    pub stores: RunStoresArgs,
}

impl SetupCommand {
    /// Executes the selected setup command.
    pub(crate) async fn execute(&self, ctx: &CommandContext) -> ! {
        let result = match self {
            Self::Import { args } => import(args)
                .await
                .map(|output| CommandOutput::new(SetupOutput::Imported(output.data))),
            Self::List { args } => list(args)
                .await
                .map(|output| CommandOutput::new(SetupOutput::Listed(output.data))),
            Self::Export { args } => export(args)
                .await
                .map(|output| CommandOutput::new(SetupOutput::Exported(output.data))),
        };
        handle_command_result(result, &ctx.output_format);
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
enum SetupOutput {
    /// Import publication details.
    Imported(SetupImportOutput),
    /// Current target list.
    Listed(SetupListOutput),
    /// Export confirmation.
    Exported(SetupExportOutput),
}

impl fmt::Display for SetupOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Imported(output) => output.fmt(f),
            Self::Listed(output) => output.fmt(f),
            Self::Exported(output) => output.fmt(f),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct SetupImportOutput {
    configs: Vec<SetupConfigPublication>,
}

impl fmt::Display for SetupImportOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for config in &self.configs {
            writeln!(
                f,
                "{} {} {} {}",
                config.target,
                config.schema_id,
                config.digest,
                publication_status_text(config.status)
            )?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
struct SetupListOutput {
    targets: Vec<String>,
}

impl fmt::Display for SetupListOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for target in &self.targets {
            writeln!(f, "{target}")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
struct SetupExportOutput {
    target: String,
}

impl fmt::Display for SetupExportOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{}", self.target)
    }
}

fn publication_status_text(status: mfm_app::SetupConfigPublicationStatus) -> &'static str {
    match status {
        mfm_app::SetupConfigPublicationStatus::Created => "created",
        mfm_app::SetupConfigPublicationStatus::Updated => "updated",
        mfm_app::SetupConfigPublicationStatus::Unchanged => "unchanged",
    }
}

async fn import(args: &ImportArgs) -> CommandResult<SetupImportOutput> {
    let bytes = tokio::fs::read(&args.file)
        .await
        .map_err(|_| CommandError::backend("SetupFileReadFailed", "Failed to read setup file"))?;
    let store = mfm_app::connect_production_store(args.stores.database_url.as_deref()).await?;
    let configs = mfm_app::import_setup_toml(&store, &bytes).await?;
    Ok(CommandOutput::new(SetupImportOutput { configs }))
}

async fn list(args: &ListArgs) -> CommandResult<SetupListOutput> {
    let store = mfm_app::connect_production_store(args.stores.database_url.as_deref()).await?;
    let targets = mfm_app::list_setup_targets(&store).await?;
    Ok(CommandOutput::new(SetupListOutput { targets }))
}

async fn export(args: &ExportArgs) -> CommandResult<SetupExportOutput> {
    let store = mfm_app::connect_production_store(args.stores.database_url.as_deref()).await?;
    let bytes = mfm_app::export_setup_target(&store, &args.target).await?;
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&args.output)
        .await
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                CommandError::new("SetupExportPathExists", "Export output path already exists")
            } else {
                CommandError::backend(
                    "SetupExportPathInvalid",
                    "Export output path could not be opened",
                )
            }
        })?;
    file.write_all(&bytes).await.map_err(|_| {
        CommandError::backend("SetupExportWriteFailed", "Export could not be written")
    })?;
    Ok(CommandOutput::new(SetupExportOutput {
        target: args.target.clone(),
    }))
}
