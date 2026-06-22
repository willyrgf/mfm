use crate::commands::result::CommandError;
use clap::{Args, ValueEnum};
use mfm_app::{DriveMode, RunServices};
use mfm_ids::{RunId, SchemaId};
use mfm_stream_store_postgres::PostgresRunStore;

/// Shared run-store selection arguments.
#[derive(Args, Debug, Clone)]
pub(crate) struct RunStoresArgs {
    /// PostgreSQL connection string for the run store (default: $DATABASE_URL)
    #[arg(long)]
    pub(crate) database_url: Option<String>,
}

/// Scheduler driving policy accepted by typed CLI commands that can execute work.
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum DriveArg {
    /// Append or inspect only, without driving runnable states.
    AppendOnly,
    /// Drive at most one scheduler step.
    Once,
    /// Drive runnable states until the typed scheduler blocks.
    UntilBlocked,
}

/// Connects to the certified PostgreSQL run store.
pub(crate) async fn make_run_store(args: &RunStoresArgs) -> Result<PostgresRunStore, CommandError> {
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

    Ok(PostgresRunStore::connect(&database_url).await?)
}

/// Builds typed app services for CLI commands backed by the certified postgres run stores.
pub(crate) async fn connect_run_services(
    args: &RunStoresArgs,
) -> Result<RunServices<PostgresRunStore, PostgresRunStore>, CommandError> {
    let store = make_run_store(args).await?;
    let runners = mfm_app::production_runner_registry(std::sync::Arc::new(store.clone()))?;
    let certification_registry = mfm_app::production_certification_registry()?;
    Ok(mfm_app::make_run_services_with_certification_registry(
        runners,
        store.clone(),
        store,
        certification_registry,
    ))
}

/// Converts a CLI drive enum into the typed app drive mode.
pub(crate) fn drive_mode(arg: DriveArg) -> DriveMode {
    match arg {
        DriveArg::AppendOnly => DriveMode::AppendOnly,
        DriveArg::Once => DriveMode::Once,
        DriveArg::UntilBlocked => DriveMode::UntilBlocked,
    }
}

/// Parses a typed run id from the persisted typed-kernel identity grammar.
pub(crate) fn parse_run_id(value: &str) -> Result<RunId, CommandError> {
    RunId::parse(value).map_err(|_| {
        CommandError::new(
            "InvalidRunId",
            "Run id must use the typed run identity format `run:<algorithm>:<digest>`",
        )
    })
}

/// Parses a typed public output schema id.
pub(crate) fn parse_schema_id(value: &str) -> Result<SchemaId, CommandError> {
    SchemaId::parse(value).map_err(|_| {
        CommandError::new(
            "InvalidSchemaId",
            "Schema id must use the typed schema identity format",
        )
    })
}
