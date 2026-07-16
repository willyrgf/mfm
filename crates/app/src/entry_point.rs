use mfm_certify::CertificationRegistry;
use mfm_ids::{SchemaId, StoreScopeId};
use mfm_storage_postgres::PostgresStore;
use serde_json::Value;

use crate::{AppError, ErrorClass, InvocationKey, RunLaunchRequest};

/// One exact public entry-point summary and its request schema.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct EntryPointSummary {
    /// Exact entry-point id, including namespace and version.
    pub entry_point_id: &'static str,
    /// Stable schema id for the strict JSON request object.
    #[serde(serialize_with = "serialize_schema_id")]
    pub request_schema_id: SchemaId,
}

/// Returns the exact compiled entry-point discovery surface.
///
/// Contract entry points were intentionally removed before the portfolio snapshot objective is
/// published. No public operation is available during this transition.
pub fn entry_point_summaries() -> Result<Vec<EntryPointSummary>, AppError> {
    Ok(Vec::new())
}

/// Validates an exact public entry-point id without resolving catalog values.
///
/// No entry points are currently published, so this returns `EntryPointNotFound` for every id.
pub fn validate_entry_point_id(_entry_point_id: &str) -> Result<(), AppError> {
    Err(no_public_entry_point())
}

/// Prepares an exact catalog-backed entry-point launch.
///
/// No entry points are currently published, so this returns `EntryPointNotFound` without reading
/// the supplied catalog request.
pub async fn prepare_entry_point_run_launch(
    _store: &PostgresStore,
    _entry_point_id: &str,
    _request: &Value,
    _certification_registry: &CertificationRegistry,
    _store_scope_id: StoreScopeId,
    _invocation_key: Option<InvocationKey>,
) -> Result<RunLaunchRequest, AppError> {
    Err(no_public_entry_point())
}

fn no_public_entry_point() -> AppError {
    AppError::new(
        ErrorClass::BadRequest,
        "EntryPointNotFound",
        "The exact entry-point id is not registered",
    )
}

fn serialize_schema_id<S>(value: &SchemaId, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(value.as_str())
}
