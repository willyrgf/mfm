use crate::commands::result::CommandError;
use crate::support::run_stores::{make_ephemeral_stores, make_stores, RunStoresArgs};
use mfm_app::{AppError, AppServices, EngineBundle};
use mfm_machine::engine::Stores;

/// Builds the default engine bundle used by CLI commands.
pub(crate) fn make_engine_bundle() -> EngineBundle {
    mfm_app::make_engine_bundle()
}

/// Builds the shared app service facade from CLI-selected stores.
pub(crate) fn make_app_services(stores: Stores) -> AppServices {
    AppServices::new(make_engine_bundle(), stores.streams, stores.artifacts)
}

/// Builds app services backed by in-memory stream storage for one-shot commands.
pub(crate) fn make_ephemeral_app_services() -> AppServices {
    make_app_services(make_ephemeral_stores(None))
}

/// Builds app services from shared run-store arguments.
pub(crate) async fn make_app_services_from_args(
    stores_args: &RunStoresArgs,
) -> Result<AppServices, CommandError> {
    let stores = make_stores(
        stores_args.artifact_root.clone(),
        stores_args.database_url.clone(),
    )
    .await?;

    Ok(make_app_services(stores))
}

/// Converts an app-layer error into the CLI command error contract.
pub(crate) fn command_error_from_app_error(err: AppError) -> CommandError {
    let mut out = CommandError::new(err.code, err.message);
    if out.code == "OperationCancelled" {
        out = out.with_exit_code(0);
    }
    out
}
