use std::path::Path;

use crate::commands::result::PublicError;
use clap::Args;
use mfm_ids::{RunId, SchemaId};

/// Shared production database connection arguments.
#[derive(Args, Debug, Clone)]
pub(crate) struct DatabaseArgs {
    /// PostgreSQL connection string (default: $DATABASE_URL).
    #[arg(long)]
    pub(crate) database_url: Option<String>,
}

/// Builds one opaque application facade over the production database.
pub(crate) async fn connect_application(
    args: &DatabaseArgs,
    runtime_config_path: Option<&Path>,
) -> Result<mfm_app::Application, PublicError> {
    mfm_app::connect_production_application(args.database_url.as_deref(), runtime_config_path).await
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
