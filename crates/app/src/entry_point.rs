use mfm_certify::CertificationRegistry;
use mfm_ids::{StableAuthorKey, StoreScopeId};
use mfm_op_portfolio_snapshot::portfolio_snapshot_program_launch_plan;
use mfm_portfolio_model::ids::PortfolioId;
use mfm_portfolio_model::portfolio::PortfolioConfig;
use mfm_storage_postgres::PostgresStore;
use mfm_values::{MfmConfig, ValidatedConfig};

use crate::{
    certify_launch_plan, entry_point_launch_internal_error, invocation_key_digest_or_mint,
    prepare_certified_run_launch, AppError, CertifiedRunLaunchInput, ErrorClass, InvocationKey,
    RunLaunchRequest,
};

const PORTFOLIO_SNAPSHOT_ID: &str = "mfm.portfolio/snapshot@1";

/// One exact public entry-point summary.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct EntryPointSummary {
    /// Exact entry-point id, including namespace and version.
    pub entry_point_id: &'static str,
}

/// Returns the exact compiled entry-point discovery surface.
pub fn entry_point_summaries() -> Vec<EntryPointSummary> {
    vec![EntryPointSummary {
        entry_point_id: PORTFOLIO_SNAPSHOT_ID,
    }]
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
) -> Result<RunLaunchRequest, AppError> {
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
) -> Result<RunLaunchRequest, AppError> {
    let (portfolio, source) = resolve_portfolio_target(store, target).await?;
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

fn entry_point_not_found() -> AppError {
    AppError::new(
        ErrorClass::BadRequest,
        "EntryPointNotFound",
        "The exact entry-point id is not registered",
    )
}

async fn resolve_portfolio_target(
    store: &PostgresStore,
    target: &str,
) -> Result<(PortfolioConfig, mfm_events::v1::ConfiguredTargetEvidence), AppError> {
    let portfolio_id = PortfolioId::new(target.to_owned()).map_err(|_| {
        AppError::new(
            ErrorClass::BadRequest,
            "ConfiguredTargetInvalid",
            "The portfolio target is invalid",
        )
    })?;
    let stable_target = StableAuthorKey::new(portfolio_id.as_str()).map_err(|_| {
        AppError::backend(
            ErrorClass::Internal,
            "ConfiguredTargetInvalid",
            "The portfolio target cannot be represented as a stable target",
        )
    })?;
    let row = store
        .load_configured_value(&stable_target)
        .await
        .map_err(AppError::from)?
        .ok_or_else(|| {
            AppError::not_found(
                "ConfiguredValueNotFound",
                "The current configuration target was not found",
            )
        })?;
    let expected_schema_id = PortfolioConfig::schema_id().map_err(|_| {
        AppError::backend(
            ErrorClass::Internal,
            "ConfiguredValueSchemaInvalid",
            "The portfolio configuration schema is invalid",
        )
    })?;
    if row.schema_id != expected_schema_id {
        return Err(AppError::new(
            ErrorClass::BadRequest,
            "ConfiguredValueSchemaInvalid",
            "The current configuration target has the wrong schema",
        ));
    }
    let config: PortfolioConfig = serde_json::from_slice(&row.canonical_json).map_err(|_| {
        AppError::new(
            ErrorClass::BadRequest,
            "ConfiguredValueTypeInvalid",
            "The current configuration does not match the expected type",
        )
    })?;
    let validated = ValidatedConfig::new(config.normalized()).map_err(|_| {
        AppError::new(
            ErrorClass::BadRequest,
            "ConfiguredValueValidationFailed",
            "The current configuration failed semantic validation",
        )
    })?;
    if validated.as_ref().portfolio_id != portfolio_id {
        return Err(AppError::new(
            ErrorClass::BadRequest,
            "ConfiguredTargetMismatch",
            "The configured portfolio id does not match the selected target",
        ));
    }
    let canonical = validated.canonical_json().map_err(|_| {
        AppError::backend(
            ErrorClass::Internal,
            "ConfiguredValueCanonicalizationFailed",
            "The current configuration could not be canonicalized",
        )
    })?;
    if canonical.as_bytes() != row.canonical_json.as_slice()
        || canonical.content_digest() != row.digest
    {
        return Err(AppError::backend(
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
    use super::{entry_point_summaries, PORTFOLIO_SNAPSHOT_ID};
    use mfm_portfolio_model::ids::PortfolioId;

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

    #[test]
    fn discovery_exposes_only_the_portfolio_snapshot_objective() {
        let summaries = entry_point_summaries();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].entry_point_id, PORTFOLIO_SNAPSHOT_ID);
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

    #[test]
    fn deleted_entry_points_are_not_discoverable() {
        let discovered = entry_point_summaries()
            .into_iter()
            .map(|summary| summary.entry_point_id)
            .collect::<Vec<_>>();
        for entry_point in DELETED_ENTRY_POINTS {
            assert!(!discovered.contains(&entry_point));
        }
    }
}
