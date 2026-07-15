use mfm_canonical::sha256_digest_bytes;
use mfm_catalog_model::CatalogRef;
use mfm_certify::CertificationRegistry;
use mfm_evm_contract_model::EvmContractContext;
use mfm_ids::{DigestAlgorithm, SchemaId, StoreScopeId};
use mfm_op_evm_contract_lifecycle::{
    build_configure_entry_config, build_deploy_entry_config, build_lifecycle_entry_config,
    build_validate_entry_config,
};
use mfm_program::TypedProgramLaunchPlan;
use mfm_state_evm_contracts::{
    ConfigureAction, DeployAction, ImportConfiguredSpec, ImportDeployedSpec, ValidateAction,
};
use mfm_storage_postgres::{CatalogValueKey, PostgresStore};
use mfm_values::{MfmConfig, ValidatedConfig};
use serde::Deserialize;
use serde_json::Value;

use crate::{
    certify_launch_plan, entry_point_launch_internal_error, invocation_key_digest_or_mint,
    prepare_certified_run_launch, AppError, CertifiedRunLaunchInput, ErrorClass, InvocationKey,
    RunLaunchRequest,
};

const CONTRACT_DEPLOY_ID: &str = "mfm.evm.contract/deploy@1";
const CONTRACT_CONFIGURE_ID: &str = "mfm.evm.contract/configure@1";
const CONTRACT_VALIDATE_ID: &str = "mfm.evm.contract/validate@1";
const CONTRACT_LIFECYCLE_ID: &str = "mfm.evm.contract/lifecycle@1";
const CONTRACT_DEPLOY_REQUEST_SCHEMA: (&str, &[u8]) = (
    "mfm.app.request.evm_contract_deploy",
    b"mfm.app.request:evm_contract_deploy:v1",
);
const CONTRACT_CONFIGURE_REQUEST_SCHEMA: (&str, &[u8]) = (
    "mfm.app.request.evm_contract_configure",
    b"mfm.app.request:evm_contract_configure:v1",
);
const CONTRACT_VALIDATE_REQUEST_SCHEMA: (&str, &[u8]) = (
    "mfm.app.request.evm_contract_validate",
    b"mfm.app.request:evm_contract_validate:v1",
);
const CONTRACT_LIFECYCLE_REQUEST_SCHEMA: (&str, &[u8]) = (
    "mfm.app.request.evm_contract_lifecycle",
    b"mfm.app.request:evm_contract_lifecycle:v1",
);

/// One exact public entry point and the request schema accepted by it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct EntryPointSummary {
    /// Exact entry-point id, including namespace and version.
    pub entry_point_id: &'static str,
    /// Stable schema id for the strict JSON request object.
    #[serde(serialize_with = "serialize_schema_id")]
    pub request_schema_id: SchemaId,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContractDeployRequest {
    context: CatalogRef<EvmContractContext>,
    deploy_action: CatalogRef<DeployAction>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContractConfigureRequest {
    context: CatalogRef<EvmContractContext>,
    import_deployed: CatalogRef<ImportDeployedSpec>,
    configure_action: CatalogRef<ConfigureAction>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContractValidateRequest {
    context: CatalogRef<EvmContractContext>,
    import_configured: CatalogRef<ImportConfiguredSpec>,
    validate_action: CatalogRef<ValidateAction>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContractLifecycleRequest {
    context: CatalogRef<EvmContractContext>,
    deploy_action: CatalogRef<DeployAction>,
    configure_action: CatalogRef<ConfigureAction>,
    validate_action: CatalogRef<ValidateAction>,
}

#[derive(Debug, Clone, Copy)]
enum EntryPoint {
    Deploy,
    Configure,
    Validate,
    Lifecycle,
}

impl EntryPoint {
    fn parse(value: &str) -> Result<Self, AppError> {
        match value {
            CONTRACT_DEPLOY_ID => Ok(Self::Deploy),
            CONTRACT_CONFIGURE_ID => Ok(Self::Configure),
            CONTRACT_VALIDATE_ID => Ok(Self::Validate),
            CONTRACT_LIFECYCLE_ID => Ok(Self::Lifecycle),
            _ => Err(AppError::new(
                ErrorClass::BadRequest,
                "EntryPointNotFound",
                "The exact entry-point id is not registered",
            )),
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::Deploy => CONTRACT_DEPLOY_ID,
            Self::Configure => CONTRACT_CONFIGURE_ID,
            Self::Validate => CONTRACT_VALIDATE_ID,
            Self::Lifecycle => CONTRACT_LIFECYCLE_ID,
        }
    }

    fn request_schema_id(self) -> Result<SchemaId, AppError> {
        let (name, seed) = match self {
            Self::Deploy => CONTRACT_DEPLOY_REQUEST_SCHEMA,
            Self::Configure => CONTRACT_CONFIGURE_REQUEST_SCHEMA,
            Self::Validate => CONTRACT_VALIDATE_REQUEST_SCHEMA,
            Self::Lifecycle => CONTRACT_LIFECYCLE_REQUEST_SCHEMA,
        };
        SchemaId::new(
            name,
            "1",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(seed),
        )
        .map_err(|_| {
            AppError::backend(
                ErrorClass::Internal,
                "EntryPointRequestSchemaInvalid",
                "An entry-point request schema is invalid",
            )
        })
    }

    async fn prepare(
        self,
        store: &PostgresStore,
        request: &Value,
    ) -> Result<PreparedEntryPoint, AppError> {
        match self {
            Self::Deploy => {
                let request: ContractDeployRequest = decode_request(request)?;
                let (context, context_source) = resolve_catalog(store, &request.context).await?;
                let (deploy, deploy_source) =
                    resolve_catalog(store, &request.deploy_action).await?;
                let config = build_deploy_entry_config(context, deploy)
                    .map_err(|_| plan_error("EvmContractPlanFailed"))?;
                let plan = TypedProgramLaunchPlan::from_draft(
                    mfm_op_evm_contract_lifecycle::deploy_contract_program_draft(config)
                        .map_err(|_| plan_error("EvmContractPlanFailed"))?,
                )
                .map_err(|_| plan_error("EvmContractPlanFailed"))?;
                Ok(PreparedEntryPoint::new(
                    plan,
                    [context_source, deploy_source],
                ))
            }
            Self::Configure => {
                let request: ContractConfigureRequest = decode_request(request)?;
                let (context, context_source) = resolve_catalog(store, &request.context).await?;
                let (import_deployed, import_source) =
                    resolve_catalog(store, &request.import_deployed).await?;
                let (configure, configure_source) =
                    resolve_catalog(store, &request.configure_action).await?;
                let config = build_configure_entry_config(context, import_deployed, configure)
                    .map_err(|_| plan_error("EvmContractPlanFailed"))?;
                let plan = TypedProgramLaunchPlan::from_draft(
                    mfm_op_evm_contract_lifecycle::configure_contract_program_draft(config)
                        .map_err(|_| plan_error("EvmContractPlanFailed"))?,
                )
                .map_err(|_| plan_error("EvmContractPlanFailed"))?;
                Ok(PreparedEntryPoint::new(
                    plan,
                    [context_source, import_source, configure_source],
                ))
            }
            Self::Validate => {
                let request: ContractValidateRequest = decode_request(request)?;
                let (context, context_source) = resolve_catalog(store, &request.context).await?;
                let (import_configured, import_source) =
                    resolve_catalog(store, &request.import_configured).await?;
                let (validate, validate_source) =
                    resolve_catalog(store, &request.validate_action).await?;
                let config = build_validate_entry_config(context, import_configured, validate)
                    .map_err(|_| plan_error("EvmContractPlanFailed"))?;
                let plan = TypedProgramLaunchPlan::from_draft(
                    mfm_op_evm_contract_lifecycle::validate_contract_program_draft(config)
                        .map_err(|_| plan_error("EvmContractPlanFailed"))?,
                )
                .map_err(|_| plan_error("EvmContractPlanFailed"))?;
                Ok(PreparedEntryPoint::new(
                    plan,
                    [context_source, import_source, validate_source],
                ))
            }
            Self::Lifecycle => {
                let request: ContractLifecycleRequest = decode_request(request)?;
                let (context, context_source) = resolve_catalog(store, &request.context).await?;
                let (deploy, deploy_source) =
                    resolve_catalog(store, &request.deploy_action).await?;
                let (configure, configure_source) =
                    resolve_catalog(store, &request.configure_action).await?;
                let (validate, validate_source) =
                    resolve_catalog(store, &request.validate_action).await?;
                let config = build_lifecycle_entry_config(context, deploy, configure, validate)
                    .map_err(|_| plan_error("EvmContractPlanFailed"))?;
                let plan = TypedProgramLaunchPlan::from_draft(
                    mfm_op_evm_contract_lifecycle::contract_lifecycle_program_draft(config)
                        .map_err(|_| plan_error("EvmContractPlanFailed"))?,
                )
                .map_err(|_| plan_error("EvmContractPlanFailed"))?;
                Ok(PreparedEntryPoint::new(
                    plan,
                    [
                        context_source,
                        deploy_source,
                        configure_source,
                        validate_source,
                    ],
                ))
            }
        }
    }
}

struct PreparedEntryPoint {
    plan: TypedProgramLaunchPlan,
    sources: Vec<mfm_events::v1::CatalogSourceEvidence>,
}

impl PreparedEntryPoint {
    fn new<const N: usize>(
        plan: TypedProgramLaunchPlan,
        sources: [mfm_events::v1::CatalogSourceEvidence; N],
    ) -> Self {
        Self {
            plan,
            sources: sources.into_iter().collect(),
        }
    }
}

/// Returns the exact compiled entry-point discovery surface.
pub fn entry_point_summaries() -> Result<Vec<EntryPointSummary>, AppError> {
    let mut summaries: Vec<EntryPointSummary> = [
        EntryPoint::Deploy,
        EntryPoint::Configure,
        EntryPoint::Validate,
        EntryPoint::Lifecycle,
    ]
    .into_iter()
    .map(|entry_point| {
        Ok::<EntryPointSummary, AppError>(EntryPointSummary {
            entry_point_id: entry_point.id(),
            request_schema_id: entry_point.request_schema_id()?,
        })
    })
    .collect::<Result<Vec<_>, AppError>>()?;
    summaries.sort_by_key(|summary| summary.entry_point_id);
    Ok(summaries)
}

/// Validates an exact public entry-point id without resolving catalog values.
pub fn validate_entry_point_id(entry_point_id: &str) -> Result<(), AppError> {
    EntryPoint::parse(entry_point_id).map(|_| ())
}

/// Prepares an exact catalog-backed entry-point launch.
pub async fn prepare_entry_point_run_launch(
    store: &PostgresStore,
    entry_point_id: &str,
    request: &Value,
    certification_registry: &CertificationRegistry,
    store_scope_id: StoreScopeId,
    invocation_key: Option<InvocationKey>,
) -> Result<RunLaunchRequest, AppError> {
    let entry_point = EntryPoint::parse(entry_point_id)?;
    let prepared = entry_point.prepare(store, request).await?;
    let (certified_spec, scoped_registry, config_inputs, seed_inputs) =
        certify_launch_plan(&prepared.plan, certification_registry)?;
    let invocation_key_digest = invocation_key_digest_or_mint(invocation_key.as_ref())?;
    let entry_point_evidence =
        mfm_events::v1::EntryPointLaunchEvidence::new(entry_point.id(), prepared.sources).map_err(
            |_| {
                entry_point_launch_internal_error(
                    "EntryPointLaunchEvidenceInvalid",
                    "entry-point launch evidence is invalid",
                )
            },
        )?;
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

async fn resolve_catalog<T: MfmConfig>(
    store: &PostgresStore,
    reference: &CatalogRef<T>,
) -> Result<(T, mfm_events::v1::CatalogSourceEvidence), AppError> {
    let schema_id = T::schema_id().map_err(|_| {
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
    let config: T = serde_json::from_slice(&row.canonical_json).map_err(|_| {
        AppError::backend(
            ErrorClass::BadRequest,
            "CatalogValueTypeInvalid",
            "The catalog value does not match the requested type",
        )
    })?;
    let validated = ValidatedConfig::new(config).map_err(|_| {
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

fn plan_error(code: &'static str) -> AppError {
    AppError::backend(ErrorClass::BadRequest, code, "Entry-point planning failed")
}

fn serialize_schema_id<S>(value: &SchemaId, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(value.as_str())
}
