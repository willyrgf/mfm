use crate::commands::result::CommandError;
use clap::{Args, ValueEnum};
use mfm_app::{DriveMode, ProductionRunServices};
use mfm_ids::{RunId, SchemaId};

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

/// Builds typed app services for CLI commands backed by the certified postgres run stores.
pub(crate) async fn connect_run_services(
    args: &RunStoresArgs,
) -> Result<ProductionRunServices, CommandError> {
    Ok(mfm_app::connect_production_run_services(args.database_url.as_deref()).await?)
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
