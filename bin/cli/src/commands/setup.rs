use std::fmt;
use std::path::{Path, PathBuf};

use crate::commands::result::{CommandError, CommandOutput, CommandResult};
use crate::commands::CommandContext;
use crate::presentation::output::handle_command_result;
use crate::support::output_file::{self, CreateNewFileError};
use crate::support::run_store::RunStoresArgs;
use clap::{Args, Subcommand};
use mfm_app::SetupConfigPublication;
use serde::Serialize;
use tokio::io::AsyncReadExt;

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
    let bytes = read_setup_file(&args.file).await?;
    let store = mfm_app::connect_production_store(args.stores.database_url.as_deref()).await?;
    let configs = mfm_app::import_setup_toml(&store, &bytes).await?;
    Ok(CommandOutput::new(SetupImportOutput { configs }))
}

async fn read_setup_file(path: &Path) -> Result<Vec<u8>, CommandError> {
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|_| setup_file_read_error())?;
    let mut chunk = [0_u8; 16 * 1024];
    let mut bytes = Vec::with_capacity(chunk.len());

    while bytes.len() < mfm_app::MAX_SETUP_FILE_BYTES {
        let remaining = mfm_app::MAX_SETUP_FILE_BYTES - bytes.len();
        let read_len = remaining.min(chunk.len());
        let read = file
            .read(&mut chunk[..read_len])
            .await
            .map_err(|_| setup_file_read_error())?;
        if read == 0 {
            return Ok(bytes);
        }
        bytes.extend_from_slice(&chunk[..read]);
    }

    let mut extra = [0_u8; 1];
    if file
        .read(&mut extra)
        .await
        .map_err(|_| setup_file_read_error())?
        != 0
    {
        return Err(CommandError::new(
            "SetupFileTooLarge",
            "The setup file exceeds the permitted size",
        ));
    }
    Ok(bytes)
}

fn setup_file_read_error() -> CommandError {
    CommandError::backend("SetupFileReadFailed", "Failed to read setup file")
}

async fn list(args: &ListArgs) -> CommandResult<SetupListOutput> {
    let store = mfm_app::connect_production_store(args.stores.database_url.as_deref()).await?;
    let targets = mfm_app::list_setup_targets(&store).await?;
    Ok(CommandOutput::new(SetupListOutput { targets }))
}

async fn export(args: &ExportArgs) -> CommandResult<SetupExportOutput> {
    let store = mfm_app::connect_production_store(args.stores.database_url.as_deref()).await?;
    let bytes = mfm_app::export_setup_target(&store, &args.target).await?;
    let output = args.output.clone();
    tokio::task::spawn_blocking(move || output_file::create_new_atomic(&output, &bytes))
        .await
        .map_err(|_| setup_export_write_error())?
        .map_err(map_setup_export_error)?;
    Ok(CommandOutput::new(SetupExportOutput {
        target: args.target.clone(),
    }))
}

fn map_setup_export_error(error: CreateNewFileError) -> CommandError {
    match error {
        CreateNewFileError::TargetExists => {
            CommandError::new("SetupExportPathExists", "Export output path already exists")
        }
        CreateNewFileError::InvalidPath => CommandError::backend(
            "SetupExportPathInvalid",
            "Export output path could not be opened",
        ),
        CreateNewFileError::WriteFailed => setup_export_write_error(),
    }
}

fn setup_export_write_error() -> CommandError {
    CommandError::backend("SetupExportWriteFailed", "Export could not be written")
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::read_setup_file;

    #[tokio::test]
    async fn setup_reader_accepts_the_limit_and_rejects_one_byte_more() {
        let directory = TempDir::new().expect("temporary setup directory");
        let boundary_path = directory.path().join("boundary.toml");
        let boundary_file = std::fs::File::create(&boundary_path).expect("create boundary file");
        boundary_file
            .set_len(mfm_app::MAX_SETUP_FILE_BYTES as u64)
            .expect("size boundary setup file");

        assert_eq!(
            read_setup_file(&boundary_path)
                .await
                .expect("boundary-sized setup input")
                .len(),
            mfm_app::MAX_SETUP_FILE_BYTES
        );

        let path = directory.path().join("oversized.toml");
        let file = std::fs::File::create(&path).expect("create oversized setup file");
        file.set_len((mfm_app::MAX_SETUP_FILE_BYTES + 1) as u64)
            .expect("size oversized setup file");

        let error = read_setup_file(&path)
            .await
            .expect_err("oversized setup input must be rejected");
        assert_eq!(error.code, "SetupFileTooLarge");
    }
}
