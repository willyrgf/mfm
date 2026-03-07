use crate::commands::result::CommandError;
use mfm_app::{AppError, AppServices, EngineBundle};
use mfm_machine::engine::Stores;
use mfm_sdk::unstable::SingleOpReportError;

/// Builds the default engine bundle used by CLI commands.
pub fn make_engine_bundle() -> EngineBundle {
    mfm_app::make_engine_bundle()
}

/// Builds the shared app service facade from CLI-selected stores.
pub fn make_app_services(stores: Stores) -> AppServices {
    AppServices::new(make_engine_bundle(), stores.events, stores.artifacts)
}

/// Converts an app-layer error into the CLI command error contract.
pub fn command_error_from_app_error(err: AppError) -> CommandError {
    CommandError::new(err.code, err.message)
}

/// Converts a single-op report error into the CLI command error contract.
pub fn command_error_from_single_op_report_error(err: SingleOpReportError) -> CommandError {
    let mut out = CommandError::new(err.code, err.message);
    if out.code == "OperationCancelled" {
        out = out.with_exit_code(0);
    }
    out
}
