#![allow(clippy::disallowed_methods)]

use std::path::PathBuf;

use crate::commands::result::CommandError;
use clap::{Args, ValueEnum};
use mfm_app::{DriveMode, RunServices};
use mfm_artifact_store_fs::FsTypedArtifactStore;
use mfm_ids::{RunId, SchemaId};
use mfm_stream_store_postgres::PostgresTypedRunEventStore;

/// Shared typed run store selection arguments.
#[derive(Args, Debug, Clone)]
pub(crate) struct TypedRunStoresArgs {
    /// Root directory for typed run artifacts (default: $MFM_TYPED_ARTIFACT_ROOT or ~/.mfm/typed_run_artifacts)
    #[arg(long)]
    pub(crate) typed_artifact_root: Option<PathBuf>,

    /// PostgreSQL connection string for the typed run-event store (default: $DATABASE_URL)
    #[arg(long)]
    pub(crate) database_url: Option<String>,
}

/// Scheduler driving policy accepted by typed CLI commands that can execute work.
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum TypedDriveArg {
    /// Append or inspect only, without driving runnable states.
    AppendOnly,
    /// Drive at most one scheduler step.
    Once,
    /// Drive runnable states until the typed scheduler blocks.
    UntilBlocked,
}

/// Builds the typed filesystem artifact store selected by CLI arguments.
pub(crate) fn make_typed_artifact_store(args: &TypedRunStoresArgs) -> FsTypedArtifactStore {
    FsTypedArtifactStore::new(
        args.typed_artifact_root
            .clone()
            .unwrap_or_else(mfm_app::default_typed_artifact_root),
    )
}

/// Connects to the certified typed PostgreSQL run-event store.
pub(crate) async fn make_typed_run_store(
    args: &TypedRunStoresArgs,
) -> Result<PostgresTypedRunEventStore, CommandError> {
    let database_url = match args
        .database_url
        .clone()
        .or_else(|| std::env::var("DATABASE_URL").ok())
    {
        Some(database_url) => database_url,
        None => {
            return Err(CommandError::new(
                "MissingDatabaseUrl",
                "Missing DATABASE_URL (or pass --database-url)",
            ))
        }
    };

    Ok(PostgresTypedRunEventStore::connect(&database_url).await?)
}

/// Builds typed app services for CLI commands backed by the certified typed stores.
pub(crate) async fn connect_run_services(
    args: &TypedRunStoresArgs,
) -> Result<RunServices<PostgresTypedRunEventStore>, CommandError> {
    let store = make_typed_run_store(args).await?;
    let artifacts = make_typed_artifact_store(args);
    let runners = mfm_app::production_typed_runner_registry(artifacts.clone())?;
    let certification_registry = mfm_app::production_certification_registry()?;
    Ok(
        mfm_app::make_async_typed_services_with_certification_registry(
            runners,
            store,
            artifacts,
            certification_registry,
        ),
    )
}

/// Converts a CLI drive enum into the typed app drive mode.
pub(crate) fn drive_mode(arg: TypedDriveArg) -> DriveMode {
    match arg {
        TypedDriveArg::AppendOnly => DriveMode::AppendOnly,
        TypedDriveArg::Once => DriveMode::Once,
        TypedDriveArg::UntilBlocked => DriveMode::UntilBlocked,
    }
}

/// Parses a typed run id from the persisted typed-kernel identity grammar.
pub(crate) fn parse_typed_run_id(value: &str) -> Result<RunId, CommandError> {
    RunId::parse(value).map_err(|_| {
        CommandError::new(
            "InvalidRunId",
            "Run id must use the typed run identity format `run:<algorithm>:<digest>`",
        )
    })
}

/// Parses a typed public output schema id.
pub(crate) fn parse_typed_schema_id(value: &str) -> Result<SchemaId, CommandError> {
    SchemaId::parse(value).map_err(|_| {
        CommandError::new(
            "InvalidSchemaId",
            "Schema id must use the typed schema identity format",
        )
    })
}
