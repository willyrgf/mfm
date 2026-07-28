use std::path::PathBuf;

use crate::commands::result::PublicError;
use clap::Args;
use mfm_ids::RunId;

/// Shared production application connection inputs.
#[derive(Args)]
pub(crate) struct ApplicationConnectionArgs {
    /// Production database connection.
    #[command(flatten)]
    database: DatabaseArgs,

    /// Explicit runtime configuration for live drive capabilities.
    #[arg(long, value_name = "PATH")]
    runtime_config: Option<PathBuf>,
}

/// Shared production database connection arguments.
#[derive(Args)]
struct DatabaseArgs {
    /// PostgreSQL connection string (default: $DATABASE_URL).
    #[arg(long)]
    database_url: Option<String>,
}

/// Builds one opaque application facade over the production database.
pub(crate) async fn connect_application(
    args: &ApplicationConnectionArgs,
) -> Result<mfm_app::Application, PublicError> {
    let _connection_inputs = (&args.database.database_url, &args.runtime_config);
    Err(PublicError::new(
        mfm_app::ErrorClass::ServiceUnavailable,
        "AuthoritativeWriterFenceUnavailable",
        "A deployment-owned authoritative-writer fence is required",
    ))
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
