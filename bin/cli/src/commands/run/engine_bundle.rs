use crate::commands::result::CommandError;
use mfm_app::{AppError, EngineBundle};

pub(super) fn make_engine_bundle() -> EngineBundle {
    mfm_app::make_engine_bundle()
}

pub(super) fn command_error_from_app_error(err: AppError) -> CommandError {
    CommandError::new(err.code, err.message)
}
