use std::fmt;
use std::path::PathBuf;

use crate::commands::result::{CommandError, CommandOutput, CommandResult};
use crate::commands::CommandContext;
use crate::presentation::output::handle_command_result;
use crate::support::run_store::RunStoresArgs;
use clap::{Args, Subcommand};
use mfm_app::{CatalogListCursor, CatalogValueIdentity};
use mfm_ids::{ContentDigest, SchemaId};
use serde::Serialize;
use tokio::io::AsyncWriteExt;

/// Setup catalog commands.
#[derive(Subcommand)]
pub(crate) enum SetupCommand {
    /// Import a strict TOML setup document into the immutable catalog.
    Import {
        #[command(flatten)]
        args: ImportArgs,
    },
    /// List immutable catalog identities.
    List {
        #[command(flatten)]
        args: ListArgs,
    },
    /// Export one exact canonical catalog value to a new file.
    Export {
        #[command(flatten)]
        args: ExportArgs,
    },
}

/// Arguments for setup import.
#[derive(Args)]
pub(crate) struct ImportArgs {
    /// TOML setup document path.
    #[arg(long, value_name = "PATH")]
    pub file: PathBuf,
    #[command(flatten)]
    pub stores: RunStoresArgs,
}

/// Arguments for catalog listing.
#[derive(Args)]
pub(crate) struct ListArgs {
    /// Maximum identities to return (at most 100).
    #[arg(long, default_value_t = 100)]
    pub limit: u32,
    /// Last name from the previous page.
    #[arg(long)]
    pub after_name: Option<String>,
    /// Last schema id from the previous page.
    #[arg(long)]
    pub after_schema_id: Option<String>,
    /// Last digest from the previous page.
    #[arg(long)]
    pub after_digest: Option<String>,
    #[command(flatten)]
    pub stores: RunStoresArgs,
}

/// Arguments for catalog export.
#[derive(Args)]
pub(crate) struct ExportArgs {
    /// Exact catalog name.
    #[arg(long)]
    pub name: String,
    /// Exact typed schema id.
    #[arg(long)]
    pub schema_id: String,
    /// Exact content digest.
    #[arg(long)]
    pub digest: String,
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
            Self::Import { args } => import(args).await,
            Self::List { args } => list(args).await,
            Self::Export { args } => export(args).await,
        };
        handle_command_result(result, &ctx.output_format);
    }
}

#[derive(Debug, Clone, Serialize)]
struct SetupOutput {
    values: Vec<CatalogValueIdentity>,
}

impl fmt::Display for SetupOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for value in &self.values {
            writeln!(f, "{} {} {}", value.name, value.schema_id, value.digest)?;
        }
        Ok(())
    }
}

async fn import(args: &ImportArgs) -> CommandResult<SetupOutput> {
    let bytes = tokio::fs::read(&args.file)
        .await
        .map_err(|_| CommandError::backend("SetupFileReadFailed", "Failed to read setup file"))?;
    let store = mfm_app::connect_production_store(args.stores.database_url.as_deref()).await?;
    let values = mfm_app::import_setup_toml(&store, &bytes).await?;
    Ok(CommandOutput::new(SetupOutput { values }))
}

async fn list(args: &ListArgs) -> CommandResult<SetupOutput> {
    let cursor = match (&args.after_name, &args.after_schema_id, &args.after_digest) {
        (None, None, None) => None,
        (Some(name), Some(schema_id), Some(digest)) => Some(CatalogListCursor {
            name: name.clone(),
            schema_id: SchemaId::parse(schema_id)
                .map_err(|_| CommandError::new("InvalidSchemaId", "Schema id is invalid"))?,
            digest: ContentDigest::parse(digest).map_err(|_| {
                CommandError::new("InvalidContentDigest", "Content digest is invalid")
            })?,
        }),
        _ => {
            return Err(CommandError::new(
                "CatalogCursorInvalid",
                "after-name, after-schema-id, and after-digest must be supplied together",
            ));
        }
    };
    let store = mfm_app::connect_production_store(args.stores.database_url.as_deref()).await?;
    let values = mfm_app::list_catalog_values(&store, cursor.as_ref(), args.limit).await?;
    Ok(CommandOutput::new(SetupOutput { values }))
}

async fn export(args: &ExportArgs) -> CommandResult<SetupOutput> {
    let schema_id = SchemaId::parse(&args.schema_id)
        .map_err(|_| CommandError::new("InvalidSchemaId", "Schema id is invalid"))?;
    let digest = ContentDigest::parse(&args.digest)
        .map_err(|_| CommandError::new("InvalidContentDigest", "Content digest is invalid"))?;
    let identity = CatalogValueIdentity {
        name: args.name.clone(),
        schema_id,
        digest,
    };
    let store = mfm_app::connect_production_store(args.stores.database_url.as_deref()).await?;
    let bytes = mfm_app::export_catalog_value(&store, &identity).await?;
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&args.output)
        .await
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                CommandError::new(
                    "CatalogExportPathExists",
                    "Export output path already exists",
                )
            } else {
                CommandError::backend(
                    "CatalogExportPathInvalid",
                    "Export output path could not be opened",
                )
            }
        })?;
    file.write_all(&bytes).await.map_err(|_| {
        CommandError::backend("CatalogExportWriteFailed", "Export could not be written")
    })?;
    Ok(CommandOutput::new(SetupOutput {
        values: vec![identity],
    }))
}
