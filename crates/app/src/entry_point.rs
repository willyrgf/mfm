use mfm_canonical::sha256_digest_bytes;
use mfm_catalog_model::CatalogRef;
use mfm_certify::CertificationRegistry;
use mfm_ids::{DigestAlgorithm, SchemaId, StoreScopeId};
use mfm_op_portfolio_snapshot::portfolio_snapshot_program_launch_plan;
use mfm_portfolio_model::portfolio::PortfolioConfig;
use mfm_storage_postgres::{CatalogValueKey, PostgresStore};
use mfm_values::{MfmConfig, ValidatedConfig};
use serde::Deserialize;
use serde_json::Value;

use crate::{
    certify_launch_plan, entry_point_launch_internal_error, invocation_key_digest_or_mint,
    prepare_certified_run_launch, AppError, CertifiedRunLaunchInput, ErrorClass, InvocationKey,
    RunLaunchRequest,
};

const PORTFOLIO_SNAPSHOT_ID: &str = "mfm.portfolio/snapshot@1";
const PORTFOLIO_REQUEST_SCHEMA: (&str, &[u8]) = (
    "mfm.app.request.portfolio_snapshot",
    b"mfm.app.request:portfolio_snapshot:catalog_ref:v1",
);

/// One exact public entry-point summary and its request schema.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct EntryPointSummary {
    /// Exact entry-point id, including namespace and version.
    pub entry_point_id: &'static str,
    /// Stable schema id for the strict JSON request object.
    #[serde(serialize_with = "serialize_schema_id")]
    pub request_schema_id: SchemaId,
}

/// Strict request for the sole portfolio snapshot objective.
///
/// This stays private to app admission: callers provide JSON through the CLI or REST transport,
/// while planning receives only the resolved normalized [`PortfolioConfig`].
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PortfolioSnapshotRequest {
    portfolio: CatalogRef<PortfolioConfig>,
}

/// Returns the exact compiled entry-point discovery surface.
pub fn entry_point_summaries() -> Result<Vec<EntryPointSummary>, AppError> {
    Ok(vec![EntryPointSummary {
        entry_point_id: PORTFOLIO_SNAPSHOT_ID,
        request_schema_id: portfolio_request_schema_id()?,
    }])
}

/// Validates an exact public entry-point id without resolving catalog values.
pub fn validate_entry_point_id(entry_point_id: &str) -> Result<(), AppError> {
    if entry_point_id == PORTFOLIO_SNAPSHOT_ID {
        Ok(())
    } else {
        Err(entry_point_not_found())
    }
}

/// Prepares the sole catalog-backed portfolio snapshot launch at admission.
///
/// The catalog is consulted only here. The resulting certified plan contains the concrete,
/// normalized [`PortfolioConfig`] and retains the exact catalog identity as admission evidence.
pub async fn prepare_entry_point_run_launch(
    store: &PostgresStore,
    entry_point_id: &str,
    request: &Value,
    certification_registry: &CertificationRegistry,
    store_scope_id: StoreScopeId,
    invocation_key: Option<InvocationKey>,
) -> Result<RunLaunchRequest, AppError> {
    validate_entry_point_id(entry_point_id)?;
    let request: PortfolioSnapshotRequest = decode_request(request)?;
    let (portfolio, source) = resolve_portfolio_catalog(store, &request.portfolio).await?;
    let plan = portfolio_snapshot_program_launch_plan(portfolio).map_err(|_| {
        AppError::backend(
            ErrorClass::BadRequest,
            "PortfolioSnapshotPlanFailed",
            "Entry-point planning failed",
        )
    })?;
    let (certified_spec, scoped_registry, config_inputs, seed_inputs) =
        certify_launch_plan(&plan, certification_registry)?;
    let invocation_key_digest = invocation_key_digest_or_mint(invocation_key.as_ref())?;
    let entry_point_evidence =
        mfm_events::v1::EntryPointLaunchEvidence::new(PORTFOLIO_SNAPSHOT_ID, vec![source])
            .map_err(|_| {
                entry_point_launch_internal_error(
                    "EntryPointLaunchEvidenceInvalid",
                    "entry-point launch evidence is invalid",
                )
            })?;
    prepare_certified_run_launch(
        CertifiedRunLaunchInput {
            certified_spec,
            registry: &scoped_registry,
            store_scope_id,
            invocation_key_digest,
            entry_point_evidence,
        },
        config_inputs,
        seed_inputs,
    )
}

fn portfolio_request_schema_id() -> Result<SchemaId, AppError> {
    SchemaId::new(
        PORTFOLIO_REQUEST_SCHEMA.0,
        "1",
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(PORTFOLIO_REQUEST_SCHEMA.1),
    )
    .map_err(|_| {
        AppError::backend(
            ErrorClass::Internal,
            "EntryPointRequestSchemaInvalid",
            "The entry-point request schema is invalid",
        )
    })
}

fn entry_point_not_found() -> AppError {
    AppError::new(
        ErrorClass::BadRequest,
        "EntryPointNotFound",
        "The exact entry-point id is not registered",
    )
}

async fn resolve_portfolio_catalog(
    store: &PostgresStore,
    reference: &CatalogRef<PortfolioConfig>,
) -> Result<(PortfolioConfig, mfm_events::v1::CatalogSourceEvidence), AppError> {
    let schema_id = PortfolioConfig::schema_id().map_err(|_| {
        AppError::backend(
            ErrorClass::Internal,
            "CatalogSchemaInvalid",
            "The catalog value schema is invalid",
        )
    })?;
    let key = CatalogValueKey::new(
        reference.name().as_str(),
        schema_id.clone(),
        reference.digest().clone(),
    )
    .map_err(|_| {
        AppError::backend(
            ErrorClass::BadRequest,
            "CatalogReferenceInvalid",
            "The catalog reference is invalid",
        )
    })?;
    let row = store
        .load_catalog_value(&key)
        .await
        .map_err(AppError::from)?
        .ok_or_else(|| {
            AppError::not_found(
                "CatalogValueNotFound",
                "The exact catalog value was not found",
            )
        })?;
    let config: PortfolioConfig = serde_json::from_slice(&row.canonical_json).map_err(|_| {
        AppError::backend(
            ErrorClass::BadRequest,
            "CatalogValueTypeInvalid",
            "The catalog value does not match the requested type",
        )
    })?;
    let validated = ValidatedConfig::new(config.normalized()).map_err(|_| {
        AppError::backend(
            ErrorClass::BadRequest,
            "CatalogValueValidationFailed",
            "The catalog value failed semantic validation",
        )
    })?;
    let canonical = validated.canonical_json().map_err(|_| {
        AppError::backend(
            ErrorClass::Internal,
            "CatalogValueCanonicalizationFailed",
            "The catalog value could not be canonicalized",
        )
    })?;
    if canonical.as_bytes() != row.canonical_json.as_slice()
        || canonical.content_digest() != *reference.digest()
    {
        return Err(AppError::backend(
            ErrorClass::Internal,
            "CatalogValueCanonicalMismatch",
            "The catalog value failed canonical integrity verification",
        ));
    }
    let source = mfm_events::v1::CatalogSourceEvidence::new(
        reference.name().as_str(),
        schema_id,
        reference.digest().clone(),
    )
    .map_err(|_| {
        AppError::backend(
            ErrorClass::Internal,
            "CatalogSourceInvalid",
            "Catalog launch evidence is invalid",
        )
    })?;
    Ok((validated.into_inner(), source))
}

fn decode_request<T: serde::de::DeserializeOwned>(request: &Value) -> Result<T, AppError> {
    serde_json::from_value(request.clone()).map_err(|_| {
        AppError::new(
            ErrorClass::BadRequest,
            "EntryPointRequestInvalid",
            "The entry-point request JSON is invalid",
        )
    })
}

fn serialize_schema_id<S>(value: &SchemaId, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(value.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const DELETED_ENTRY_POINTS: [&str; 8] = [
        "mfm.portfolio/portfolio_snapshot@1",
        "mfm.portfolio/collect_then_report@1",
        "mfm.bitcoin/btc_address_balance@1",
        "mfm.evm/evm_native_balance@1",
        "mfm.evm.contract/deploy@1",
        "mfm.evm.contract/configure@1",
        "mfm.evm.contract/validate@1",
        "mfm.evm.contract/lifecycle@1",
    ];
    const DIGEST: &str =
        "content:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000001";

    #[test]
    fn discovery_exposes_only_the_portfolio_snapshot_objective() {
        let summaries = entry_point_summaries().expect("entry-point summaries");
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].entry_point_id, PORTFOLIO_SNAPSHOT_ID);
        assert_eq!(
            summaries[0].request_schema_id,
            portfolio_request_schema_id().expect("request schema")
        );
    }

    #[test]
    fn only_the_exact_snapshot_id_is_accepted() {
        validate_entry_point_id(PORTFOLIO_SNAPSHOT_ID).expect("exact snapshot id");
        for id in DELETED_ENTRY_POINTS.into_iter().chain([
            "mfm.portfolio/snapshot",
            "mfm.portfolio/snapshot@latest",
            "mfm.portfolio/snapshot@2",
        ]) {
            let error = validate_entry_point_id(id).expect_err("non-exact id must fail");
            assert_eq!(error.code, "EntryPointNotFound", "{id}");
        }
    }

    #[test]
    fn request_strictly_accepts_one_catalog_reference() {
        let request = json!({
            "portfolio": {"name": "acme/primary", "digest": DIGEST}
        });
        let decoded: PortfolioSnapshotRequest = decode_request(&request).expect("valid request");
        assert_eq!(decoded.portfolio.name().as_str(), "acme/primary");
        assert_eq!(decoded.portfolio.digest().as_str(), DIGEST);

        for malformed in [
            json!({}),
            json!({"portfolio": {"name": "acme/primary", "digest": DIGEST}, "extra": true}),
            json!({"portfolio": {"name": "acme/primary", "digest": DIGEST, "extra": true}}),
            json!({"config": {"name": "acme/primary", "digest": DIGEST}}),
            json!({"portfolio": {"name": "acme/primary", "digest": DIGEST}, "policy": {}}),
            json!({"portfolio": {"name": "acme/primary", "digest": DIGEST}, "collector": {}}),
            json!({"portfolio": {"name": "acme/primary"}}),
        ] {
            let error = decode_request::<PortfolioSnapshotRequest>(&malformed)
                .expect_err("non-contract request must fail");
            assert_eq!(error.code, "EntryPointRequestInvalid");
        }
    }
}
