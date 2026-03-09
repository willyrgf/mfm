use crate::commands::result::CommandError;
use mfm_app::{AppError, AppServices, EngineBundle};
use mfm_machine::engine::Stores;
use mfm_sdk::unstable::SingleOpReportError;
use serde::Serialize;

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

/// Serializes a CLI-built op config into the JSON shape expected by the app layer.
pub fn serialize_op_config<T: Serialize>(
    op_config: &T,
    op_name: &'static str,
) -> Result<serde_json::Value, CommandError> {
    serde_json::to_value(op_config).map_err(|err| {
        CommandError::new(
            "SerializationError",
            format!("Failed to serialize {op_name} op config: {err}"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::ser::SerializeMap;

    struct NonStringKeyMap;

    impl Serialize for NonStringKeyMap {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry(&vec![1_u8], "value")?;
            map.end()
        }
    }

    #[test]
    fn serialize_op_config_returns_command_error() {
        let err = serialize_op_config(&NonStringKeyMap, "keystore list").unwrap_err();
        assert_eq!(err.code, "SerializationError");
        assert!(err.message.contains("keystore list"));
    }
}
