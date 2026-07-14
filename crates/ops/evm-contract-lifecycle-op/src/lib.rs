#![warn(missing_docs)]
//! Typed EVM contract lifecycle operation planning.
//!
//! This crate owns deterministic operation topology for EVM contract deploy,
//! configure, validate, and full lifecycle flows. It builds typed state graphs
//! only; live transports, signer providers, artifact stores, and adapters are
//! supplied by app/runtime assembly.
//!
//! ```rust
//! use mfm_op_evm_contract_lifecycle::contract_lifecycle_program_draft;
//! use mfm_op_evm_contract_lifecycle::EvmContractLifecycleEntryConfig;
//!
//! # fn demo(config: EvmContractLifecycleEntryConfig) -> mfm_program::Result<()> {
//! let draft = contract_lifecycle_program_draft(config)?;
//! assert_eq!(draft.state_nodes().len(), 3);
//! # Ok(())
//! # }
//! ```

use std::collections::BTreeSet;

use mfm_evm_contract_model::{
    AcceptedContextPolicy, AdoptExternalAddress, ConfiguredContractInstance,
    ContextBoundValidationReport, ContractLifecycleStage, ContractProfileDigestRef,
    DeployedContractInstance, EvmContractContext, ImportFromMfmRun, ImportFromMfmRunEvidence,
};
use mfm_ids::{DigestAlgorithm, OperationKind, OperationVersion};
use mfm_program::{
    build_root_with_registries, Handle, Operation, OperationExpansion, OperationKey,
    PublicOutputKey, RootBound, RootBuilder, ScopeKey, SideEffectSagaPolicy,
    SideEffectVerificationSpec, StateKey, TypedProgramLaunchPlan,
};
use mfm_program_derive::{MfmConfig, MfmValue, OperationOutput, PublicOutputs};
use mfm_state_evm_contracts::{
    account_nonce_resource_claim, ConfigureAction, ContextBoundConfigureContractState,
    ContextBoundDeployContractState, ContextBoundValidateContractState,
    ContextConfigureContractInputHandles, ContextValidateContractInputHandles, DeployAction,
    ImportConfiguredContractState, ImportConfiguredSpec, ImportDeployedContractState,
    ImportDeployedSpec, ValidateAction,
};
use serde::{Deserialize, Serialize};

const OP_NAMESPACE: &str = "mfm.evm.contract";
const ROOT_SCOPE: &str = "evm_contract";
const CONTEXT_DEPLOY_OP_KEY: &str = "contract_context_deploy";
const CONTEXT_CONFIGURE_OP_KEY: &str = "contract_context_configure";
const CONTEXT_VALIDATE_OP_KEY: &str = "contract_context_validate";
const CONTEXT_LIFECYCLE_OP_KEY: &str = "contract_context_lifecycle";
const PUBLIC_OUTPUT_KEY: &str = "contract";

/// Typed planning failures for complete EVM lifecycle entry configs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum EvmContractPlanError {
    /// The authored context could not be certified by the program context authority.
    #[error("EVM contract context is invalid")]
    InvalidContext,
    /// An import requested a lifecycle stage different from the entry operation.
    #[error("EVM contract import stage does not match the entry operation")]
    ImportStageMismatch,
    /// An import source context is not admitted by its explicit context policy.
    #[error("EVM contract import context is not accepted")]
    ImportContextMismatch,
    /// An accepted-context policy is empty or repeats a context reference.
    #[error("EVM contract accepted-context policy is invalid")]
    InvalidAcceptedContextPolicy,
    /// Source-run import evidence does not match its typed import request.
    #[error("EVM contract source-run import evidence does not match its request")]
    ImportEvidenceMismatch,
    /// Source-run import evidence does not describe the required lifecycle value type.
    #[error("EVM contract source-run import evidence type is invalid")]
    ImportEvidenceTypeMismatch,
    /// External adoption assertions contain a runtime-incompatible block range.
    #[error("EVM contract external adoption event assertions cannot set block ranges")]
    InvalidExternalAdoptionPolicy,
}

/// Builds and validates a deploy-only EVM contract entry config.
pub fn build_deploy_entry_config(
    context: EvmContractContext,
    deploy: DeployAction,
) -> Result<EvmContractDeployEntryConfig, EvmContractPlanError> {
    let config = EvmContractDeployEntryConfig { context, deploy };
    validate_deploy_entry_config(&config)?;
    Ok(config)
}

/// Builds and validates a configure-only EVM contract entry config.
pub fn build_configure_entry_config(
    context: EvmContractContext,
    import_deployed: ImportDeployedSpec,
    configure: ConfigureAction,
) -> Result<EvmContractConfigureEntryConfig, EvmContractPlanError> {
    let config = EvmContractConfigureEntryConfig {
        context,
        import_deployed,
        configure,
    };
    validate_configure_entry_config(&config)?;
    Ok(config)
}

/// Builds and validates a validate-only EVM contract entry config.
pub fn build_validate_entry_config(
    context: EvmContractContext,
    import_configured: ImportConfiguredSpec,
    validate: ValidateAction,
) -> Result<EvmContractValidateEntryConfig, EvmContractPlanError> {
    let config = EvmContractValidateEntryConfig {
        context,
        import_configured,
        validate,
    };
    validate_validate_entry_config(&config)?;
    Ok(config)
}

/// Builds and validates a full EVM contract lifecycle entry config.
pub fn build_lifecycle_entry_config(
    context: EvmContractContext,
    deploy: DeployAction,
    configure: ConfigureAction,
    validate: ValidateAction,
) -> Result<EvmContractLifecycleEntryConfig, EvmContractPlanError> {
    let config = EvmContractLifecycleEntryConfig {
        context,
        deploy,
        configure,
        validate,
    };
    validate_lifecycle_entry_config(&config)?;
    Ok(config)
}

/// Complete config for deploy-only EVM contract planning.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue, MfmConfig)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "deploy-entry-config",
    schema = "mfm.evm.contract.config.deploy_entry",
    validate = "validate_deploy_entry_config"
)]
pub struct EvmContractDeployEntryConfig {
    context: EvmContractContext,
    deploy: DeployAction,
}

impl EvmContractDeployEntryConfig {
    /// Returns the certified lifecycle context.
    pub const fn context(&self) -> &EvmContractContext {
        &self.context
    }

    /// Returns the deploy action.
    pub const fn deploy(&self) -> &DeployAction {
        &self.deploy
    }
}

/// Complete config for configure-only EVM contract planning.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue, MfmConfig)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "configure-entry-config",
    schema = "mfm.evm.contract.config.configure_entry",
    validate = "validate_configure_entry_config"
)]
pub struct EvmContractConfigureEntryConfig {
    context: EvmContractContext,
    import_deployed: ImportDeployedSpec,
    configure: ConfigureAction,
}

impl EvmContractConfigureEntryConfig {
    /// Returns the certified lifecycle context.
    pub const fn context(&self) -> &EvmContractContext {
        &self.context
    }

    /// Returns the deployed-stage import spec.
    pub const fn import_deployed(&self) -> &ImportDeployedSpec {
        &self.import_deployed
    }

    /// Returns the configure action.
    pub const fn configure(&self) -> &ConfigureAction {
        &self.configure
    }
}

/// Complete config for validate-only EVM contract planning.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue, MfmConfig)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "validate-entry-config",
    schema = "mfm.evm.contract.config.validate_entry",
    validate = "validate_validate_entry_config"
)]
pub struct EvmContractValidateEntryConfig {
    context: EvmContractContext,
    import_configured: ImportConfiguredSpec,
    validate: ValidateAction,
}

impl EvmContractValidateEntryConfig {
    /// Returns the certified lifecycle context.
    pub const fn context(&self) -> &EvmContractContext {
        &self.context
    }

    /// Returns the configured-stage import spec.
    pub const fn import_configured(&self) -> &ImportConfiguredSpec {
        &self.import_configured
    }

    /// Returns the validate action.
    pub const fn validate(&self) -> &ValidateAction {
        &self.validate
    }
}

/// Complete config for full EVM contract lifecycle planning.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue, MfmConfig)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "lifecycle-entry-config",
    schema = "mfm.evm.contract.config.lifecycle_entry",
    validate = "validate_lifecycle_entry_config"
)]
pub struct EvmContractLifecycleEntryConfig {
    context: EvmContractContext,
    deploy: DeployAction,
    configure: ConfigureAction,
    validate: ValidateAction,
}

impl EvmContractLifecycleEntryConfig {
    /// Returns the certified lifecycle context.
    pub const fn context(&self) -> &EvmContractContext {
        &self.context
    }

    /// Returns the deploy action.
    pub const fn deploy(&self) -> &DeployAction {
        &self.deploy
    }

    /// Returns the configure action.
    pub const fn configure(&self) -> &ConfigureAction {
        &self.configure
    }

    /// Returns the validate action.
    pub const fn validate(&self) -> &ValidateAction {
        &self.validate
    }
}

fn validate_deploy_entry_config(
    config: &EvmContractDeployEntryConfig,
) -> Result<(), EvmContractPlanError> {
    validate_context(&config.context)
}

fn validate_configure_entry_config(
    config: &EvmContractConfigureEntryConfig,
) -> Result<(), EvmContractPlanError> {
    validate_context(&config.context)?;
    validate_import_deployed(&config.context, &config.import_deployed)
}

fn validate_validate_entry_config(
    config: &EvmContractValidateEntryConfig,
) -> Result<(), EvmContractPlanError> {
    validate_context(&config.context)?;
    validate_import_configured(&config.context, &config.import_configured)
}

fn validate_lifecycle_entry_config(
    config: &EvmContractLifecycleEntryConfig,
) -> Result<(), EvmContractPlanError> {
    validate_context(&config.context)
}

fn validate_context(context: &EvmContractContext) -> Result<(), EvmContractPlanError> {
    mfm_program::context_ref_for(context)
        .map(|_| ())
        .map_err(|_| EvmContractPlanError::InvalidContext)
}

fn validate_import_deployed(
    context: &EvmContractContext,
    import: &ImportDeployedSpec,
) -> Result<(), EvmContractPlanError> {
    match import {
        ImportDeployedSpec::FromMfmRun { source, evidence } => {
            validate_mfm_run_import::<DeployedContractInstance>(
                context,
                source,
                evidence,
                ContractLifecycleStage::Deployed,
            )
        }
        ImportDeployedSpec::AdoptExternalAddress { adoption } => {
            validate_external_adoption(adoption)
        }
    }
}

fn validate_import_configured(
    context: &EvmContractContext,
    import: &ImportConfiguredSpec,
) -> Result<(), EvmContractPlanError> {
    match import {
        ImportConfiguredSpec::FromMfmRun { source, evidence } => {
            validate_mfm_run_import::<ConfiguredContractInstance>(
                context,
                source,
                evidence,
                ContractLifecycleStage::Configured,
            )
        }
        ImportConfiguredSpec::AdoptExternalAddress { adoption } => {
            validate_external_adoption(adoption)
        }
    }
}

fn validate_external_adoption(adoption: &AdoptExternalAddress) -> Result<(), EvmContractPlanError> {
    if adoption
        .evidence_policy
        .initial_event_assertions
        .iter()
        .any(|assertion| assertion.from_block.is_some() || assertion.to_block.is_some())
    {
        return Err(EvmContractPlanError::InvalidExternalAdoptionPolicy);
    }
    Ok(())
}

fn validate_mfm_run_import<T>(
    context: &EvmContractContext,
    source: &ImportFromMfmRun,
    evidence: &ImportFromMfmRunEvidence,
    expected_stage: ContractLifecycleStage,
) -> Result<(), EvmContractPlanError>
where
    T: mfm_values::MfmValue,
{
    if source.required_stage != expected_stage {
        return Err(EvmContractPlanError::ImportStageMismatch);
    }

    let context_ref =
        mfm_program::context_ref_for(context).map_err(|_| EvmContractPlanError::InvalidContext)?;
    match &source.accepted_context_policy {
        AcceptedContextPolicy::ExactContext {} => {
            if source.source_context_ref.as_context_ref() != &context_ref {
                return Err(EvmContractPlanError::ImportContextMismatch);
            }
        }
        AcceptedContextPolicy::AcceptedContextRefs { context_refs } => {
            let unique = context_refs.iter().cloned().collect::<BTreeSet<_>>();
            if context_refs.is_empty() || unique.len() != context_refs.len() {
                return Err(EvmContractPlanError::InvalidAcceptedContextPolicy);
            }
            if !unique.contains(&source.source_context_ref) {
                return Err(EvmContractPlanError::ImportContextMismatch);
            }
        }
    }

    if evidence.source_spec_hash != source.source_spec_hash
        || evidence.source_cell_or_output_id != source.source_cell_or_output_id
        || evidence.source_stage != expected_stage
        || evidence.source_context_ref != source.source_context_ref
        || evidence.source_value_digest != source.source_value_digest
        || evidence.import_policy_digest != canonical_digest(source)?
    {
        return Err(EvmContractPlanError::ImportEvidenceMismatch);
    }

    if evidence
        .source_value_artifact_ref_or_inline_canonical_value
        .content_digest()
        .map_err(|_| EvmContractPlanError::ImportEvidenceMismatch)?
        != source
            .source_value_digest
            .typed()
            .map_err(|_| EvmContractPlanError::ImportEvidenceMismatch)?
    {
        return Err(EvmContractPlanError::ImportEvidenceMismatch);
    }

    let schema_id = T::schema_id().map_err(|_| EvmContractPlanError::ImportEvidenceTypeMismatch)?;
    let semantic_type_id =
        T::semantic_id().map_err(|_| EvmContractPlanError::ImportEvidenceTypeMismatch)?;
    if evidence
        .source_cell_schema_id
        .typed()
        .map_err(|_| EvmContractPlanError::ImportEvidenceTypeMismatch)?
        != schema_id
        || evidence
            .source_cell_semantic_type_id
            .typed()
            .map_err(|_| EvmContractPlanError::ImportEvidenceTypeMismatch)?
            != semantic_type_id
    {
        return Err(EvmContractPlanError::ImportEvidenceTypeMismatch);
    }

    if source.source_context_ref.as_context_ref() == &context_ref {
        let descriptor = <EvmContractContext as mfm_program::StateContext>::descriptor()
            .map_err(|_| EvmContractPlanError::InvalidContext)?;
        let mfm_program::StateContextDescriptorSpec::Required(requirement) = descriptor else {
            return Err(EvmContractPlanError::InvalidContext);
        };
        if evidence
            .source_context_descriptor_id
            .typed()
            .map_err(|_| EvmContractPlanError::ImportEvidenceMismatch)?
            != requirement.context_descriptor_id
        {
            return Err(EvmContractPlanError::ImportEvidenceMismatch);
        }
    }

    Ok(())
}

fn canonical_digest<T: Serialize>(
    value: &T,
) -> Result<ContractProfileDigestRef, EvmContractPlanError> {
    let json =
        serde_json::to_string(value).map_err(|_| EvmContractPlanError::ImportEvidenceMismatch)?;
    let canonical = mfm_canonical::PlainCanonicalJsonBytes::from_json_str(&json)
        .map_err(|_| EvmContractPlanError::ImportEvidenceMismatch)?;
    Ok(ContractProfileDigestRef::from(canonical.content_digest()))
}

/// Output handles produced by context-bound deploy-only operation planning.
#[derive(OperationOutput)]
#[mfm(schema = "mfm.evm.contract.operation_outputs.context_deploy")]
pub struct ContextContractDeployOperationOutputs<'program, 'scope> {
    /// Context-bound deployed contract instance.
    pub deployed: Handle<'program, 'scope, DeployedContractInstance>,
}

#[derive(PublicOutputs)]
#[mfm(schema = "mfm.evm.contract.public_outputs.context_deploy")]
struct ContextContractDeployPublicOutputs<'program, 'scope> {
    deployed: Handle<'program, 'scope, DeployedContractInstance>,
}

/// Output handles produced by context-bound configure-only operation planning.
#[derive(OperationOutput)]
#[mfm(schema = "mfm.evm.contract.operation_outputs.context_configure")]
pub struct ContextContractConfigureOperationOutputs<'program, 'scope> {
    /// Context-bound configured contract instance.
    pub configured: Handle<'program, 'scope, ConfiguredContractInstance>,
}

#[derive(PublicOutputs)]
#[mfm(schema = "mfm.evm.contract.public_outputs.context_configure")]
struct ContextContractConfigurePublicOutputs<'program, 'scope> {
    configured: Handle<'program, 'scope, ConfiguredContractInstance>,
}

/// Output handles produced by context-bound validate-only operation planning.
#[derive(OperationOutput)]
#[mfm(schema = "mfm.evm.contract.operation_outputs.context_validate")]
pub struct ContextContractValidateOperationOutputs<'program, 'scope> {
    /// Context-bound validation report.
    pub validation_report: Handle<'program, 'scope, ContextBoundValidationReport>,
}

#[derive(PublicOutputs)]
#[mfm(schema = "mfm.evm.contract.public_outputs.context_validate")]
struct ContextContractValidatePublicOutputs<'program, 'scope> {
    validation_report: Handle<'program, 'scope, ContextBoundValidationReport>,
}

/// Output handles produced by context-bound full lifecycle operation planning.
#[derive(OperationOutput)]
#[mfm(schema = "mfm.evm.contract.operation_outputs.context_lifecycle")]
pub struct ContextContractLifecycleOperationOutputs<'program, 'scope> {
    /// Context-bound deployed contract instance.
    pub deployed: Handle<'program, 'scope, DeployedContractInstance>,
    /// Context-bound configured contract instance.
    pub configured: Handle<'program, 'scope, ConfiguredContractInstance>,
    /// Context-bound validation report.
    pub validation_report: Handle<'program, 'scope, ContextBoundValidationReport>,
}

#[derive(PublicOutputs)]
#[mfm(schema = "mfm.evm.contract.public_outputs.context_lifecycle")]
struct ContextContractLifecyclePublicOutputs<'program, 'scope> {
    deployed: Handle<'program, 'scope, DeployedContractInstance>,
    configured: Handle<'program, 'scope, ConfiguredContractInstance>,
    validation_report: Handle<'program, 'scope, ContextBoundValidationReport>,
}

/// Context-bound deploy-only EVM contract planning operation.
pub struct ContextDeployContractOperation;

impl Operation for ContextDeployContractOperation {
    type Config = EvmContractDeployEntryConfig;
    type Input<'program, 'scope> = ();
    type Output<'program, 'scope> = ContextContractDeployOperationOutputs<'program, 'scope>;

    fn kind() -> mfm_program::Result<OperationKind> {
        operation_kind(
            "context_deploy",
            b"mfm.evm.contract.operation:context_deploy",
        )
    }

    fn version() -> mfm_program::Result<OperationVersion> {
        operation_version("context_deploy")
    }

    fn name() -> &'static str {
        "mfm.evm.contract.context_deploy"
    }

    fn expand<'program, 'scope>(
        &self,
        config: mfm_program::ValidatedConfig<Self::Config>,
        _input: Self::Input<'program, 'scope>,
        builder: &mut OperationExpansion<'program, 'scope>,
        _dispatch: mfm_program::OperationExpansionDispatch<Self>,
    ) -> mfm_program::Result<Self::Output<'program, 'scope>> {
        let config = config.into_inner();
        let context = builder.declare_context(config.context().clone())?;
        let deployed = builder
            .side_effect::<ContextBoundDeployContractState, _>(
                StateKey::new("deploy")?,
                &context,
                config.deploy().clone(),
                (),
                account_nonce_resource_claim()?,
                SideEffectVerificationSpec::Receipt,
            )?
            .into_handle();
        Ok(ContextContractDeployOperationOutputs { deployed })
    }
}

/// Context-bound configure-only EVM contract planning operation.
pub struct ContextConfigureContractOperation;

impl Operation for ContextConfigureContractOperation {
    type Config = EvmContractConfigureEntryConfig;
    type Input<'program, 'scope> = ();
    type Output<'program, 'scope> = ContextContractConfigureOperationOutputs<'program, 'scope>;

    fn kind() -> mfm_program::Result<OperationKind> {
        operation_kind(
            "context_configure",
            b"mfm.evm.contract.operation:context_configure",
        )
    }

    fn version() -> mfm_program::Result<OperationVersion> {
        operation_version("context_configure")
    }

    fn name() -> &'static str {
        "mfm.evm.contract.context_configure"
    }

    fn expand<'program, 'scope>(
        &self,
        config: mfm_program::ValidatedConfig<Self::Config>,
        _input: Self::Input<'program, 'scope>,
        builder: &mut OperationExpansion<'program, 'scope>,
        _dispatch: mfm_program::OperationExpansionDispatch<Self>,
    ) -> mfm_program::Result<Self::Output<'program, 'scope>> {
        let config = config.into_inner();
        let context = builder.declare_context(config.context().clone())?;
        let deployed = builder.state::<ImportDeployedContractState, _>(
            StateKey::new("import_deployed")?,
            &context,
            config.import_deployed().clone(),
            (),
        )?;
        let configured = builder
            .side_effect::<ContextBoundConfigureContractState, _>(
                StateKey::new("configure")?,
                &context,
                config.configure().clone(),
                ContextConfigureContractInputHandles { deployed },
                account_nonce_resource_claim()?,
                SideEffectVerificationSpec::Receipt,
            )?
            .into_handle();
        Ok(ContextContractConfigureOperationOutputs { configured })
    }
}

/// Context-bound validate-only EVM contract planning operation.
pub struct ContextValidateContractOperation;

impl Operation for ContextValidateContractOperation {
    type Config = EvmContractValidateEntryConfig;
    type Input<'program, 'scope> = ();
    type Output<'program, 'scope> = ContextContractValidateOperationOutputs<'program, 'scope>;

    fn kind() -> mfm_program::Result<OperationKind> {
        operation_kind(
            "context_validate",
            b"mfm.evm.contract.operation:context_validate",
        )
    }

    fn version() -> mfm_program::Result<OperationVersion> {
        operation_version("context_validate")
    }

    fn name() -> &'static str {
        "mfm.evm.contract.context_validate"
    }

    fn expand<'program, 'scope>(
        &self,
        config: mfm_program::ValidatedConfig<Self::Config>,
        _input: Self::Input<'program, 'scope>,
        builder: &mut OperationExpansion<'program, 'scope>,
        _dispatch: mfm_program::OperationExpansionDispatch<Self>,
    ) -> mfm_program::Result<Self::Output<'program, 'scope>> {
        let config = config.into_inner();
        let context = builder.declare_context(config.context().clone())?;
        let configured = builder.state::<ImportConfiguredContractState, _>(
            StateKey::new("import_configured")?,
            &context,
            config.import_configured().clone(),
            (),
        )?;
        let validation_report = builder.state::<ContextBoundValidateContractState, _>(
            StateKey::new("validate")?,
            &context,
            config.validate().clone(),
            ContextValidateContractInputHandles { configured },
        )?;
        Ok(ContextContractValidateOperationOutputs { validation_report })
    }
}

/// Context-bound full contract lifecycle planning operation.
pub struct ContextContractLifecycleOperation;

impl Operation for ContextContractLifecycleOperation {
    type Config = EvmContractLifecycleEntryConfig;
    type Input<'program, 'scope> = ();
    type Output<'program, 'scope> = ContextContractLifecycleOperationOutputs<'program, 'scope>;

    fn kind() -> mfm_program::Result<OperationKind> {
        operation_kind(
            "context_lifecycle",
            b"mfm.evm.contract.operation:context_lifecycle",
        )
    }

    fn version() -> mfm_program::Result<OperationVersion> {
        operation_version("context_lifecycle")
    }

    fn name() -> &'static str {
        "mfm.evm.contract.context_lifecycle"
    }

    fn expand<'program, 'scope>(
        &self,
        config: mfm_program::ValidatedConfig<Self::Config>,
        _input: Self::Input<'program, 'scope>,
        builder: &mut OperationExpansion<'program, 'scope>,
        _dispatch: mfm_program::OperationExpansionDispatch<Self>,
    ) -> mfm_program::Result<Self::Output<'program, 'scope>> {
        let config = config.into_inner();
        let context = builder.declare_context(config.context().clone())?;
        let deployed = builder
            .side_effect::<ContextBoundDeployContractState, _>(
                StateKey::new("deploy")?,
                &context,
                config.deploy().clone(),
                (),
                account_nonce_resource_claim()?,
                SideEffectVerificationSpec::Receipt,
            )?
            .into_handle();
        let configured = builder
            .side_effect::<ContextBoundConfigureContractState, _>(
                StateKey::new("configure")?,
                &context,
                config.configure().clone(),
                ContextConfigureContractInputHandles {
                    deployed: deployed.clone(),
                },
                account_nonce_resource_claim()?,
                SideEffectVerificationSpec::Receipt,
            )?
            .into_handle();
        let validation_report = builder.state::<ContextBoundValidateContractState, _>(
            StateKey::new("validate")?,
            &context,
            config.validate().clone(),
            ContextValidateContractInputHandles {
                configured: configured.clone(),
            },
        )?;
        Ok(ContextContractLifecycleOperationOutputs {
            deployed,
            configured,
            validation_report,
        })
    }
}

mfm_certify::define_program_descriptor_registry! {
    state_registry: pub contract_lifecycle_state_registry,
    operation_registry: pub contract_lifecycle_operation_registry,
    certification: pub register_contract_lifecycle_certification_descriptors,
    states: [
        ContextBoundDeployContractState,
        ContextBoundConfigureContractState,
        ContextBoundValidateContractState,
        ImportDeployedContractState,
        ImportConfiguredContractState,
    ],
    operations: [
        ContextDeployContractOperation,
        ContextConfigureContractOperation,
        ContextValidateContractOperation,
        ContextContractLifecycleOperation,
    ],
}

/// Builds a typed context-bound deploy-only program draft.
pub fn deploy_contract_program_draft(
    config: EvmContractDeployEntryConfig,
) -> mfm_program::Result<mfm_program::TypedProgramDraft> {
    build_program(|root| {
        root.set_saga_policy(SideEffectSagaPolicy::FailWithoutAcdcClaim)?;
        let result = root.scope().call::<ContextDeployContractOperation, _>(
            OperationKey::new(CONTEXT_DEPLOY_OP_KEY)?,
            ContextDeployContractOperation,
            config,
            (),
        )?;
        root.bind_public_outputs(
            PublicOutputKey::new(PUBLIC_OUTPUT_KEY)?,
            &ContextContractDeployPublicOutputs {
                deployed: result.deployed,
            },
        )
    })
}

/// Builds a typed context-bound configure-only program draft through a deployed import node.
pub fn configure_contract_program_draft(
    config: EvmContractConfigureEntryConfig,
) -> mfm_program::Result<mfm_program::TypedProgramDraft> {
    build_program(|root| {
        root.set_saga_policy(SideEffectSagaPolicy::FailWithoutAcdcClaim)?;
        let result = root.scope().call::<ContextConfigureContractOperation, _>(
            OperationKey::new(CONTEXT_CONFIGURE_OP_KEY)?,
            ContextConfigureContractOperation,
            config,
            (),
        )?;
        root.bind_public_outputs(
            PublicOutputKey::new(PUBLIC_OUTPUT_KEY)?,
            &ContextContractConfigurePublicOutputs {
                configured: result.configured,
            },
        )
    })
}

/// Builds a typed context-bound validate-only program draft through a configured import node.
pub fn validate_contract_program_draft(
    config: EvmContractValidateEntryConfig,
) -> mfm_program::Result<mfm_program::TypedProgramDraft> {
    build_program(|root| {
        let result = root.scope().call::<ContextValidateContractOperation, _>(
            OperationKey::new(CONTEXT_VALIDATE_OP_KEY)?,
            ContextValidateContractOperation,
            config,
            (),
        )?;
        root.bind_public_outputs(
            PublicOutputKey::new(PUBLIC_OUTPUT_KEY)?,
            &ContextContractValidatePublicOutputs {
                validation_report: result.validation_report,
            },
        )
    })
}

/// Builds a typed context-bound full lifecycle program draft.
pub fn contract_lifecycle_program_draft(
    config: EvmContractLifecycleEntryConfig,
) -> mfm_program::Result<mfm_program::TypedProgramDraft> {
    build_program(|root| {
        root.set_saga_policy(SideEffectSagaPolicy::FailWithoutAcdcClaim)?;
        let result = root.scope().call::<ContextContractLifecycleOperation, _>(
            OperationKey::new(CONTEXT_LIFECYCLE_OP_KEY)?,
            ContextContractLifecycleOperation,
            config,
            (),
        )?;
        root.bind_public_outputs(
            PublicOutputKey::new(PUBLIC_OUTPUT_KEY)?,
            &ContextContractLifecyclePublicOutputs {
                deployed: result.deployed,
                configured: result.configured,
                validation_report: result.validation_report,
            },
        )
    })
}

/// Builds a launch plan for a deploy-only EVM contract program.
pub fn deploy_contract_program_launch_plan(
    config: EvmContractDeployEntryConfig,
) -> mfm_program::Result<TypedProgramLaunchPlan> {
    TypedProgramLaunchPlan::from_draft(deploy_contract_program_draft(config)?)
}

/// Builds a launch plan for a configure-only EVM contract program through a deployed import node.
pub fn configure_contract_program_launch_plan(
    config: EvmContractConfigureEntryConfig,
) -> mfm_program::Result<TypedProgramLaunchPlan> {
    TypedProgramLaunchPlan::from_draft(configure_contract_program_draft(config)?)
}

/// Builds a launch plan for a validate-only EVM contract program through a configured import node.
pub fn validate_contract_program_launch_plan(
    config: EvmContractValidateEntryConfig,
) -> mfm_program::Result<TypedProgramLaunchPlan> {
    TypedProgramLaunchPlan::from_draft(validate_contract_program_draft(config)?)
}

/// Builds a launch plan for a full EVM contract lifecycle program.
pub fn contract_lifecycle_program_launch_plan(
    config: EvmContractLifecycleEntryConfig,
) -> mfm_program::Result<TypedProgramLaunchPlan> {
    TypedProgramLaunchPlan::from_draft(contract_lifecycle_program_draft(config)?)
}

fn build_program(
    build: impl for<'program, 'scope> FnOnce(
        &mut RootBuilder<'program, 'scope>,
    ) -> mfm_program::Result<RootBound<'program, 'scope>>,
) -> mfm_program::Result<mfm_program::TypedProgramDraft> {
    build_root_with_registries(
        ScopeKey::new(ROOT_SCOPE)?,
        contract_lifecycle_state_registry()?,
        contract_lifecycle_operation_registry()?,
        build,
    )
}

fn operation_kind(name: &'static str, seed: &[u8]) -> mfm_program::Result<OperationKind> {
    OperationKind::new(
        OP_NAMESPACE,
        name,
        DigestAlgorithm::Sha256JcsV1,
        mfm_canonical::sha256_digest_bytes(seed),
    )
    .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
}

fn operation_version(name: &'static str) -> mfm_program::Result<OperationVersion> {
    OperationVersion::new(format!("mfm.evm.contract.operation.{name}.v1"))
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
}
