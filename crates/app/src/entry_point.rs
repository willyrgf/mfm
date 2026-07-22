use mfm_certify::CertificationRegistry;
use mfm_ids::{StableAuthorKey, StoreScopeId};
use mfm_op_portfolio_snapshot::portfolio_snapshot_program_launch_plan;
use mfm_portfolio_model::ids::PortfolioId;
use mfm_portfolio_model::portfolio::PortfolioConfig;
use mfm_storage_postgres::PostgresStore;
use mfm_values::{MfmConfig, ValidatedConfig};

use crate::{
    certify_launch_plan, entry_point_launch_internal_error, invocation_key_digest_or_mint,
    prepare_certified_run_launch, CertifiedRunLaunchInput, ErrorClass, InvocationKey, PublicError,
    RunLaunchRequest,
};

const PORTFOLIO_SNAPSHOT_ID: &str = "mfm.portfolio/snapshot@2";
const ENTRY_POINT_IDS: &[&str] = &[PORTFOLIO_SNAPSHOT_ID];

/// Returns the exact compiled entry-point discovery surface.
pub fn entry_point_ids() -> &'static [&'static str] {
    ENTRY_POINT_IDS
}

/// Prepares one configured-target-backed run launch at admission.
///
/// The mutable configuration store is consulted only here. The resulting certified plan contains
/// a concrete normalized configuration and retains its exact target, schema, and digest as
/// admission evidence. Resume and replay use only retained run material.
pub async fn prepare_entry_point_run_launch(
    store: &PostgresStore,
    entry_point_id: &str,
    target: &str,
    certification_registry: &CertificationRegistry,
    store_scope_id: StoreScopeId,
    invocation_key: Option<InvocationKey>,
) -> Result<RunLaunchRequest, PublicError> {
    match entry_point_id {
        PORTFOLIO_SNAPSHOT_ID => {
            prepare_portfolio_snapshot_run_launch(
                store,
                target,
                certification_registry,
                store_scope_id,
                invocation_key,
            )
            .await
        }
        _ => Err(entry_point_not_found()),
    }
}

async fn prepare_portfolio_snapshot_run_launch(
    store: &PostgresStore,
    target: &str,
    certification_registry: &CertificationRegistry,
    store_scope_id: StoreScopeId,
    invocation_key: Option<InvocationKey>,
) -> Result<RunLaunchRequest, PublicError> {
    let (portfolio, source) = resolve_portfolio_target(store, target).await?;
    let plan = portfolio_snapshot_program_launch_plan(portfolio).map_err(|_| {
        PublicError::backend(
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

fn entry_point_not_found() -> PublicError {
    PublicError::new(
        ErrorClass::BadRequest,
        "EntryPointNotFound",
        "The exact entry-point id is not registered",
    )
}

async fn resolve_portfolio_target(
    store: &PostgresStore,
    target: &str,
) -> Result<(PortfolioConfig, mfm_events::v1::ConfiguredTargetEvidence), PublicError> {
    let portfolio_id = PortfolioId::new(target.to_owned()).map_err(|_| {
        PublicError::new(
            ErrorClass::BadRequest,
            "ConfiguredTargetInvalid",
            "The portfolio target is invalid",
        )
    })?;
    let stable_target = StableAuthorKey::new(portfolio_id.as_str()).map_err(|_| {
        PublicError::backend(
            ErrorClass::Internal,
            "ConfiguredTargetInvalid",
            "The portfolio target cannot be represented as a stable target",
        )
    })?;
    let row = store
        .load_configured_value(&stable_target)
        .await
        .map_err(PublicError::from)?
        .ok_or_else(|| {
            PublicError::not_found(
                "ConfiguredValueNotFound",
                "The current configuration target was not found",
            )
        })?;
    let expected_schema_id = PortfolioConfig::schema_id().map_err(|_| {
        PublicError::backend(
            ErrorClass::Internal,
            "ConfiguredValueSchemaInvalid",
            "The portfolio configuration schema is invalid",
        )
    })?;
    if row.schema_id != expected_schema_id {
        return Err(PublicError::new(
            ErrorClass::BadRequest,
            "ConfiguredValueSchemaInvalid",
            "The current configuration target has the wrong schema",
        ));
    }
    let config: PortfolioConfig = serde_json::from_slice(&row.canonical_json).map_err(|_| {
        PublicError::new(
            ErrorClass::BadRequest,
            "ConfiguredValueTypeInvalid",
            "The current configuration does not match the expected type",
        )
    })?;
    let validated = ValidatedConfig::new(config.normalized()).map_err(|_| {
        PublicError::new(
            ErrorClass::BadRequest,
            "ConfiguredValueValidationFailed",
            "The current configuration failed semantic validation",
        )
    })?;
    if validated.as_ref().portfolio_id != portfolio_id {
        return Err(PublicError::new(
            ErrorClass::BadRequest,
            "ConfiguredTargetMismatch",
            "The configured portfolio id does not match the selected target",
        ));
    }
    let canonical = validated.canonical_json().map_err(|_| {
        PublicError::backend(
            ErrorClass::Internal,
            "ConfiguredValueCanonicalizationFailed",
            "The current configuration could not be canonicalized",
        )
    })?;
    if canonical.as_bytes() != row.canonical_json.as_slice()
        || canonical.content_digest() != row.digest
    {
        return Err(PublicError::backend(
            ErrorClass::Internal,
            "ConfiguredValueCanonicalMismatch",
            "The current configuration failed canonical integrity verification",
        ));
    }
    let source = mfm_events::v1::ConfiguredTargetEvidence::new(
        stable_target,
        expected_schema_id,
        row.digest,
    );
    Ok((validated.into_inner(), source))
}

#[cfg(test)]
mod tests {
    use super::{entry_point_ids, PORTFOLIO_SNAPSHOT_ID};
    use mfm_portfolio_model::ids::PortfolioId;

    #[test]
    fn discovery_exposes_only_the_portfolio_snapshot_objective() {
        assert_eq!(entry_point_ids(), &[PORTFOLIO_SNAPSHOT_ID]);
    }

    #[test]
    fn portfolio_target_parser_accepts_only_the_domain_id_grammar() {
        for valid in ["acme/primary", "portfolio-main", "a"] {
            PortfolioId::new(valid).expect("valid target");
        }
        for invalid in ["", "Mfm/primary", "mfm.reserved", "acme//primary"] {
            PortfolioId::new(invalid).expect_err("invalid target");
        }
    }
}
