use crate::commands::result::PublicError;
use clap::Args;
use mfm_app::{ProductionRunReadServices, ProductionRunServices};
use mfm_ids::{RunId, SchemaId};
use std::path::Path;

/// Shared run-store selection arguments.
#[derive(Args, Debug, Clone)]
pub(crate) struct RunStoresArgs {
    /// PostgreSQL connection string for the run store (default: $DATABASE_URL)
    #[arg(long)]
    pub(crate) database_url: Option<String>,
}

/// Builds typed app services for CLI commands backed by the certified postgres run stores.
pub(crate) async fn connect_run_services(
    args: &RunStoresArgs,
    runtime_config_path: Option<&Path>,
) -> Result<ProductionRunServices, PublicError> {
    mfm_app::connect_production_run_services(args.database_url.as_deref(), runtime_config_path)
        .await
}

/// Builds evidence-only typed app services for CLI read commands.
pub(crate) async fn connect_run_read_services(
    args: &RunStoresArgs,
) -> Result<ProductionRunReadServices, PublicError> {
    mfm_app::connect_production_run_read_services(args.database_url.as_deref()).await
}

/// Parses a typed run id from the persisted typed-kernel identity grammar.
pub(crate) fn parse_run_id(value: &str) -> Result<RunId, PublicError> {
    RunId::parse(value).map_err(|_| {
        PublicError::bad_request(
            "InvalidRunId",
            "Run id must use the typed run identity format `run:<algorithm>:<digest>`",
        )
    })
}

/// Parses a typed public output schema id.
pub(crate) fn parse_schema_id(value: &str) -> Result<SchemaId, PublicError> {
    SchemaId::parse(value).map_err(|_| {
        PublicError::bad_request(
            "InvalidSchemaId",
            "Schema id must use the typed schema identity format",
        )
    })
}
