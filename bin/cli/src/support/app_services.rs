use crate::commands::result::CommandError;
use mfm_app::{AppError, AppServices, EngineBundle};
use mfm_machine::engine::Stores;
use mfm_sdk::unstable::SingleOpReportError;

pub fn make_engine_bundle() -> EngineBundle {
    mfm_app::make_engine_bundle()
}

pub fn make_app_services(stores: Stores) -> AppServices {
    AppServices::new(make_engine_bundle(), stores.events, stores.artifacts)
}

pub fn command_error_from_app_error(err: AppError) -> CommandError {
    CommandError::new(err.code, err.message)
}

pub fn command_error_from_single_op_report_error(err: SingleOpReportError) -> CommandError {
    let mut out = CommandError::new(err.code, err.message);
    if out.code == "OperationCancelled" {
        out = out.with_exit_code(0);
    }
    out
}
