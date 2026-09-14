//! Selected SQLx facts at the PostgreSQL producer boundary.

use serde_json::{json, Value};
use std::{error::Error, io};

pub(super) fn sqlx_fields(
    operation: &'static str,
    stage: &'static str,
    error: &sqlx::Error,
) -> Value {
    let mut details = json!({"operation": operation, "stage": stage});
    let mut sources = Vec::new();
    let mut pointers = Vec::new();
    let mut source: Option<&(dyn Error + 'static)> = Some(error);
    while let Some(error) = source {
        // Interface repetition is observable; concrete-object identity is not.
        let pointer = error as *const dyn Error;
        if pointers
            .iter()
            .any(|previous| std::ptr::eq(*previous, pointer))
        {
            details["source_cycle"] = true.into();
            break;
        }
        pointers.push(pointer);
        let mut layer = json!({"message": error.to_string()});
        source = if let Some(error) = error.downcast_ref::<sqlx::Error>() {
            use sqlx::Error::*;
            layer["kind"] = match error {
                Configuration(_) => "configuration",
                InvalidArgument(_) => "invalid_argument",
                Database(_) => "database",
                Io(_) => "io",
                Tls(_) => "tls",
                Protocol(_) => "protocol",
                RowNotFound => "row_not_found",
                TypeNotFound { type_name } => {
                    layer["type_name"] = type_name.clone().into();
                    "type_not_found"
                }
                ColumnIndexOutOfBounds { index, len } => {
                    layer["index"] = (*index).into();
                    layer["len"] = (*len).into();
                    "column_index_out_of_bounds"
                }
                ColumnNotFound(column) => {
                    layer["column"] = column.clone().into();
                    "column_not_found"
                }
                ColumnDecode { index, .. } => {
                    layer["index"] = index.clone().into();
                    "column_decode"
                }
                Encode(_) => "encode",
                Decode(_) => "decode",
                AnyDriverError(_) => "any_driver_error",
                PoolTimedOut => "pool_timed_out",
                PoolClosed => "pool_closed",
                WorkerCrashed => "worker_crashed",
                Migrate(_) => "migrate",
                InvalidSavePointStatement => "invalid_save_point_statement",
                BeginFailed => "begin_failed",
                ConfigFile(_) => "config_file",
                _ => "other",
            }
            .into();
            match error {
                Database(database) => Some(database.as_error()),
                Io(error) => Some(error as &dyn Error),
                Configuration(source)
                | Tls(source)
                | ColumnDecode { source, .. }
                | Encode(source)
                | Decode(source)
                | AnyDriverError(source) => Some(source.as_ref()),
                _ => error.source(),
            }
        } else if let Some(error) = error.downcast_ref::<sqlx::postgres::PgDatabaseError>() {
            layer["sqlstate"] = error.code().into();
            use sqlx::postgres::PgSeverity;
            layer["severity"] = match error.severity() {
                PgSeverity::Panic => "PANIC",
                PgSeverity::Fatal => "FATAL",
                PgSeverity::Error => "ERROR",
                PgSeverity::Warning => "WARNING",
                PgSeverity::Notice => "NOTICE",
                PgSeverity::Debug => "DEBUG",
                PgSeverity::Info => "INFO",
                PgSeverity::Log => "LOG",
            }
            .into();
            layer["server_message"] = error.message().into();
            layer["detail"] = json!(error.detail());
            layer["hint"] = json!(error.hint());
            layer["schema"] = json!(error.schema());
            layer["table"] = json!(error.table());
            layer["column"] = json!(error.column());
            layer["constraint"] = json!(error.constraint());
            error.source()
        } else if let Some(error) = error.downcast_ref::<io::Error>() {
            layer["os_kind"] = format!("{:?}", error.kind()).into();
            layer["os_code"] = json!(error.raw_os_error());
            error
                .get_ref()
                .map(|child| child as &dyn Error)
                .or_else(|| error.source())
        } else {
            error.source()
        };
        sources.push(layer);
    }
    details["sources"] = sources.into();
    details
}
