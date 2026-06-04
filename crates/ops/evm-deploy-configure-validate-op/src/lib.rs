#![warn(missing_docs)]
//! Typed EVM deploy/configure/validate workflow operation.
//!
//! The workflow is authored through `mfm-program` and lowers to certified typed state programs.
//! It exposes no legacy dynamic planner or context-key surface. Deploy and configure are
//! side-effect states; validate is a read state that consumes a [`ConfiguredContract`] typestate
//! value.
//!
//! # Examples
//!
//! ```rust
//! use mfm_op_evm_deploy_configure_validate::{
//!     dcv_program_draft, DeployConfigureValidateCanonicalConfig,
//! };
//!
//! # fn demo(config: DeployConfigureValidateCanonicalConfig) -> mfm_program::Result<()> {
//! let draft = dcv_program_draft(config)?;
//! assert_eq!(draft.state_nodes().len(), 3);
//! # Ok(())
//! # }
//! ```

use std::collections::BTreeMap;

use mfm_certify::{certify_program_draft, CertifiedTypedSpec};
use mfm_evm_deploy_configure_validate_config::build_deploy_configure_validate_config;
pub use mfm_evm_deploy_configure_validate_config::{
    canonicalize_deploy_configure_validate_authored_config,
    decode_deploy_configure_validate_built_config,
    decode_deploy_configure_validate_canonical_config,
    parse_deploy_configure_validate_authored_config,
    parse_deploy_configure_validate_authored_config_with_hint, AuthoredConfigFormat,
    DeployConfigureValidateAuthoredConfig, DeployConfigureValidateBuildOutcome,
    DeployConfigureValidateBuildReport, DeployConfigureValidateBuiltConfig,
    DeployConfigureValidateCanonicalConfig, DeployConfigureValidateConfigError,
    DeployConfigureValidateConfigureConfig, DeployConfigureValidateDeployConfig,
    DeployConfigureValidateExecutionConfig, DeployConfigureValidateInput,
    DeployConfigureValidateSignerConfig, DeployConfigureValidateValidateConfig,
    ExistingConfiguredContractValidationConfig,
};
use mfm_ids::{
    ArtifactId, ContentDigest, DigestAlgorithm, OperationKind, OperationVersion, SchemaId, SeedId,
};
use mfm_program::{
    CanonicalSeed, Operation, OperationExpansion, OperationKey, OperationRegistryBuilder,
    PublicOutputKey, RootBuilder, ScopeKey, SeedKey, StateKey, StateRegistryBuilder,
};
use mfm_spec::v1 as spec;
pub use mfm_state_evm_dcv::{
    configure_intent_from_config, deploy_intent_from_config, evm_dcv_adapter_kind,
    evm_dcv_adapter_version, validate_configured_contract_with_backend, ConfigureContractState,
    ConfiguredContract, ConfiguredContractRef, DcvConfigureOperationOutputs,
    DcvConfigurePublicOutputs, DcvDeployOperationOutputs, DcvDeployPublicOutputs,
    DcvOperationOutputs, DcvPublicOutputs, DcvValidateOperationOutputs, DcvValidatePublicOutputs,
    DeployContractState, DeployedContract, EvmDcvConfigureConfirmation,
    EvmDcvConfigureIdempotencyInput, EvmDcvConfigureIntent, EvmDcvConfigureReceipt,
    EvmDcvConfigureReceiptEntry, EvmDcvConfigureSubmission, EvmDcvDeployConfirmation,
    EvmDcvDeployIdempotencyInput, EvmDcvDeployIntent, EvmDcvDeployReceipt, EvmDcvDeploySubmission,
    EvmDcvReadBackend, EvmDcvReadCapability, EvmDcvReadError, EvmDcvReadFuture,
    EvmDcvSignerCapability, EvmDcvTransactionIntent, EvmDcvTransactionSubmitCapability,
    ValidateContractState, ValidationReport,
};

const DCV_OPERATION_KIND_NAME: &str = "deploy_configure_validate_workflow";
const DEPLOY_OPERATION_KIND_NAME: &str = "deploy_contract_workflow";
const CONFIGURE_OPERATION_KIND_NAME: &str = "configure_contract_workflow";
const VALIDATE_OPERATION_KIND_NAME: &str = "validate_contract_workflow";
const DCV_OPERATION_VERSION: &str = "mfm.evm.dcv.operation.workflow.v1";
const DEPLOY_OPERATION_VERSION: &str = "mfm.evm.dcv.operation.deploy.v1";
const CONFIGURE_OPERATION_VERSION: &str = "mfm.evm.dcv.operation.configure.v1";
const VALIDATE_OPERATION_VERSION: &str = "mfm.evm.dcv.operation.validate.v1";
const ROOT_SCOPE: &str = "evm_dcv";
const DEPLOY_ROOT_SCOPE: &str = "evm_dcv_deploy";
const CONFIGURE_ROOT_SCOPE: &str = "evm_dcv_configure";
const VALIDATE_ROOT_SCOPE: &str = "evm_dcv_validate";
const OP_KEY: &str = "deploy_configure_validate";
const DEPLOY_OP_KEY: &str = "deploy_contract";
const CONFIGURE_OP_KEY: &str = "configure_contract";
const VALIDATE_OP_KEY: &str = "validate_contract";
const DEPLOYED_SEED_KEY: &str = "deployed_contract";
const CONFIGURED_SEED_KEY: &str = "configured_contract";
const PUBLIC_OUTPUT_KEY: &str = "evm_dcv";

fn operation_kind(name: &'static str) -> mfm_program::Result<OperationKind> {
    OperationKind::new(
        "mfm.evm.dcv",
        name,
        DigestAlgorithm::Sha256JcsV1,
        mfm_canonical::sha256_digest_bytes(format!("mfm.evm.dcv.operation:{name}").as_bytes()),
    )
    .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
}

fn operation_version(version: &'static str) -> mfm_program::Result<OperationVersion> {
    OperationVersion::new(version).map_err(|error| mfm_program::PlanError::Key(error.to_string()))
}

/// Typed EVM deploy workflow operation.
pub struct DeployContractWorkflowOperation;

impl Operation for DeployContractWorkflowOperation {
    type Config = DeployConfigureValidateDeployConfig;
    type Input<'program, 'scope> = ();
    type Output<'program, 'scope> = DcvDeployOperationOutputs<'program, 'scope>;

    fn kind() -> mfm_program::Result<OperationKind> {
        operation_kind(DEPLOY_OPERATION_KIND_NAME)
    }

    fn version() -> mfm_program::Result<OperationVersion> {
        operation_version(DEPLOY_OPERATION_VERSION)
    }

    fn name() -> &'static str {
        "mfm.evm.dcv.deploy_contract_workflow"
    }

    fn expand<'program, 'scope>(
        &self,
        config: Self::Config,
        _input: Self::Input<'program, 'scope>,
        builder: &mut OperationExpansion<'program, 'scope>,
        _dispatch: mfm_program::OperationExpansionDispatch<Self>,
    ) -> mfm_program::Result<Self::Output<'program, 'scope>> {
        let deployed_contract = builder.state::<DeployContractState, _>(
            StateKey::new("deploy_contract")?,
            config,
            (),
        )?;
        Ok(DcvDeployOperationOutputs { deployed_contract })
    }
}

/// Typed EVM configure workflow operation.
pub struct ConfigureContractWorkflowOperation;

impl Operation for ConfigureContractWorkflowOperation {
    type Config = DeployConfigureValidateConfigureConfig;
    type Input<'program, 'scope> = mfm_program::Handle<'program, 'scope, DeployedContract>;
    type Output<'program, 'scope> = DcvConfigureOperationOutputs<'program, 'scope>;

    fn kind() -> mfm_program::Result<OperationKind> {
        operation_kind(CONFIGURE_OPERATION_KIND_NAME)
    }

    fn version() -> mfm_program::Result<OperationVersion> {
        operation_version(CONFIGURE_OPERATION_VERSION)
    }

    fn name() -> &'static str {
        "mfm.evm.dcv.configure_contract_workflow"
    }

    fn expand<'program, 'scope>(
        &self,
        config: Self::Config,
        input: Self::Input<'program, 'scope>,
        builder: &mut OperationExpansion<'program, 'scope>,
        _dispatch: mfm_program::OperationExpansionDispatch<Self>,
    ) -> mfm_program::Result<Self::Output<'program, 'scope>> {
        let configured_contract = builder.state::<ConfigureContractState, _>(
            StateKey::new("configure_contract")?,
            config,
            input,
        )?;
        Ok(DcvConfigureOperationOutputs {
            configured_contract,
        })
    }
}

/// Typed EVM validate workflow operation.
pub struct ValidateContractWorkflowOperation;

impl Operation for ValidateContractWorkflowOperation {
    type Config = DeployConfigureValidateValidateConfig;
    type Input<'program, 'scope> = mfm_program::Handle<'program, 'scope, ConfiguredContract>;
    type Output<'program, 'scope> = DcvValidateOperationOutputs<'program, 'scope>;

    fn kind() -> mfm_program::Result<OperationKind> {
        operation_kind(VALIDATE_OPERATION_KIND_NAME)
    }

    fn version() -> mfm_program::Result<OperationVersion> {
        operation_version(VALIDATE_OPERATION_VERSION)
    }

    fn name() -> &'static str {
        "mfm.evm.dcv.validate_contract_workflow"
    }

    fn expand<'program, 'scope>(
        &self,
        config: Self::Config,
        input: Self::Input<'program, 'scope>,
        builder: &mut OperationExpansion<'program, 'scope>,
        _dispatch: mfm_program::OperationExpansionDispatch<Self>,
    ) -> mfm_program::Result<Self::Output<'program, 'scope>> {
        let validation_report = builder.state::<ValidateContractState, _>(
            StateKey::new("validate_contract")?,
            config,
            input,
        )?;
        Ok(DcvValidateOperationOutputs { validation_report })
    }
}

/// Typed EVM deploy/configure/validate workflow operation.
pub struct DeployConfigureValidateWorkflowOperation;

impl Operation for DeployConfigureValidateWorkflowOperation {
    type Config = DeployConfigureValidateCanonicalConfig;
    type Input<'program, 'scope> = ();
    type Output<'program, 'scope> = DcvOperationOutputs<'program, 'scope>;

    fn kind() -> mfm_program::Result<OperationKind> {
        operation_kind(DCV_OPERATION_KIND_NAME)
    }

    fn version() -> mfm_program::Result<OperationVersion> {
        operation_version(DCV_OPERATION_VERSION)
    }

    fn name() -> &'static str {
        "mfm.evm.dcv.deploy_configure_validate"
    }

    fn expand<'program, 'scope>(
        &self,
        config: Self::Config,
        _input: Self::Input<'program, 'scope>,
        builder: &mut OperationExpansion<'program, 'scope>,
        _dispatch: mfm_program::OperationExpansionDispatch<Self>,
    ) -> mfm_program::Result<Self::Output<'program, 'scope>> {
        let built = build_deploy_configure_validate_config(config)
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?;
        let mut deploy = built.execution.deploy;
        let mut configure = built.execution.configure;
        let mut validate = built.execution.validate;
        normalize_phase_artifacts(&mut deploy, &mut configure, &mut validate)?;
        validate_phase_alignment(&deploy, &configure, &validate)?;

        let deployed = builder.state::<DeployContractState, _>(
            StateKey::new("deploy_contract")?,
            deploy,
            (),
        )?;
        let configured = builder.state::<ConfigureContractState, _>(
            StateKey::new("configure_contract")?,
            configure,
            deployed.clone(),
        )?;
        let validation_report = builder.state::<ValidateContractState, _>(
            StateKey::new("validate_contract")?,
            validate,
            configured.clone(),
        )?;

        Ok(DcvOperationOutputs {
            deployed: deployed.clone(),
            configured: configured.clone(),
            validation_report,
        })
    }
}

/// Builds the EVM DCV state registry used for authoring and certification.
pub fn dcv_state_registry() -> mfm_program::Result<mfm_program::StateRegistrySnapshot> {
    let mut states = StateRegistryBuilder::new();
    states.register::<DeployContractState>()?;
    states.register::<ConfigureContractState>()?;
    states.register::<ValidateContractState>()?;
    Ok(states.into_snapshot())
}

/// Builds the EVM DCV operation registry used for authoring and certification.
pub fn dcv_operation_registry() -> mfm_program::Result<mfm_program::OperationRegistrySnapshot> {
    let mut operations = OperationRegistryBuilder::new();
    operations.register::<DeployContractWorkflowOperation>()?;
    operations.register::<ConfigureContractWorkflowOperation>()?;
    operations.register::<ValidateContractWorkflowOperation>()?;
    operations.register::<DeployConfigureValidateWorkflowOperation>()?;
    Ok(operations.into_snapshot())
}

/// Adds EVM deploy/configure/validate workflow descriptors to a trusted certification registry.
pub fn register_dcv_certification_descriptors(
    registry: &mut mfm_certify::CertificationRegistry,
) -> mfm_certify::Result<()> {
    let mut states = StateRegistryBuilder::new();
    registry.register_state(
        &states
            .register::<DeployContractState>()
            .map_err(|error| mfm_certify::CertifyError::Lowering(error.to_string()))?,
    )?;
    registry.register_state(
        &states
            .register::<ConfigureContractState>()
            .map_err(|error| mfm_certify::CertifyError::Lowering(error.to_string()))?,
    )?;
    registry.register_state(
        &states
            .register::<ValidateContractState>()
            .map_err(|error| mfm_certify::CertifyError::Lowering(error.to_string()))?,
    )?;
    let mut operations = OperationRegistryBuilder::new();
    registry.register_operation(
        &operations
            .register::<DeployContractWorkflowOperation>()
            .map_err(|error| mfm_certify::CertifyError::Lowering(error.to_string()))?,
    )?;
    registry.register_operation(
        &operations
            .register::<ConfigureContractWorkflowOperation>()
            .map_err(|error| mfm_certify::CertifyError::Lowering(error.to_string()))?,
    )?;
    registry.register_operation(
        &operations
            .register::<ValidateContractWorkflowOperation>()
            .map_err(|error| mfm_certify::CertifyError::Lowering(error.to_string()))?,
    )?;
    registry.register_operation(
        &operations
            .register::<DeployConfigureValidateWorkflowOperation>()
            .map_err(|error| mfm_certify::CertifyError::Lowering(error.to_string()))?,
    )?;
    Ok(())
}

/// Builds a typed EVM deploy/configure/validate program draft.
pub fn dcv_program_draft(
    config: DeployConfigureValidateCanonicalConfig,
) -> mfm_program::Result<mfm_program::TypedProgramDraft> {
    mfm_program::build_root_with_registries(
        ScopeKey::new(ROOT_SCOPE)?,
        dcv_state_registry()?,
        dcv_operation_registry()?,
        |root: &mut RootBuilder<'_, '_>| {
            let result = root
                .scope()
                .call::<DeployConfigureValidateWorkflowOperation, _>(
                    OperationKey::new(OP_KEY)?,
                    DeployConfigureValidateWorkflowOperation,
                    config,
                    (),
                )?;
            root.bind_public_outputs(
                PublicOutputKey::new(PUBLIC_OUTPUT_KEY)?,
                &DcvPublicOutputs {
                    deployed_contract: result.deployed,
                    configured_contract: result.configured,
                    validation_report: result.validation_report,
                },
            )
        },
    )
}

/// Builds a typed EVM deploy-only program draft.
pub fn dcv_deploy_program_draft(
    config: DeployConfigureValidateDeployConfig,
) -> mfm_program::Result<mfm_program::TypedProgramDraft> {
    mfm_program::build_root_with_registries(
        ScopeKey::new(DEPLOY_ROOT_SCOPE)?,
        dcv_state_registry()?,
        dcv_operation_registry()?,
        |root: &mut RootBuilder<'_, '_>| {
            let result = root.scope().call::<DeployContractWorkflowOperation, _>(
                OperationKey::new(DEPLOY_OP_KEY)?,
                DeployContractWorkflowOperation,
                config,
                (),
            )?;
            root.bind_public_outputs(
                PublicOutputKey::new(PUBLIC_OUTPUT_KEY)?,
                &DcvDeployPublicOutputs {
                    deployed_contract: result.deployed_contract,
                },
            )
        },
    )
}

/// Builds a typed EVM configure-only program draft from a deployed-contract launch seed.
pub fn dcv_configure_program_draft(
    config: DeployConfigureValidateConfigureConfig,
    deployed_contract: CanonicalSeed<DeployedContract>,
) -> mfm_program::Result<mfm_program::TypedProgramDraft> {
    mfm_program::build_root_with_registries(
        ScopeKey::new(CONFIGURE_ROOT_SCOPE)?,
        dcv_state_registry()?,
        dcv_operation_registry()?,
        |root: &mut RootBuilder<'_, '_>| {
            let deployed =
                root.seed(SeedKey::new(DEPLOYED_SEED_KEY)?, deployed_contract.clone())?;
            let result = root.scope().call::<ConfigureContractWorkflowOperation, _>(
                OperationKey::new(CONFIGURE_OP_KEY)?,
                ConfigureContractWorkflowOperation,
                config,
                deployed,
            )?;
            root.bind_public_outputs(
                PublicOutputKey::new(PUBLIC_OUTPUT_KEY)?,
                &DcvConfigurePublicOutputs {
                    configured_contract: result.configured_contract,
                },
            )
        },
    )
}

/// Builds a typed EVM validate-only program draft from a configured-contract launch seed.
pub fn dcv_validate_program_draft(
    config: DeployConfigureValidateValidateConfig,
    configured_contract: CanonicalSeed<ConfiguredContract>,
) -> mfm_program::Result<mfm_program::TypedProgramDraft> {
    mfm_program::build_root_with_registries(
        ScopeKey::new(VALIDATE_ROOT_SCOPE)?,
        dcv_state_registry()?,
        dcv_operation_registry()?,
        |root: &mut RootBuilder<'_, '_>| {
            let configured = root.seed(
                SeedKey::new(CONFIGURED_SEED_KEY)?,
                configured_contract.clone(),
            )?;
            let result = root.scope().call::<ValidateContractWorkflowOperation, _>(
                OperationKey::new(VALIDATE_OP_KEY)?,
                ValidateContractWorkflowOperation,
                config,
                configured,
            )?;
            root.bind_public_outputs(
                PublicOutputKey::new(PUBLIC_OUTPUT_KEY)?,
                &DcvValidatePublicOutputs {
                    validation_report: result.validation_report,
                },
            )
        },
    )
}

/// Builds and certifies the typed EVM deploy/configure/validate program.
pub fn certified_dcv_spec(
    config: DeployConfigureValidateCanonicalConfig,
) -> mfm_certify::Result<CertifiedTypedSpec> {
    let draft = dcv_program_draft(config)
        .map_err(|error| mfm_certify::CertifyError::Lowering(error.to_string()))?;
    certify_program_draft(&draft)
}

/// Fully compiled EVM DCV program ready for app launch assembly.
#[derive(Debug, Clone)]
pub struct CompiledDcvProgram {
    /// Certified typed execution spec.
    pub certified_spec: CertifiedTypedSpec,
    /// Public output schema id for this compiled program.
    pub public_schema_id: SchemaId,
    /// Config artifacts required by the certified spec.
    pub config_artifacts: Vec<DcvConfigArtifact>,
    /// Seed artifacts required by phase workflows.
    pub seed_artifacts: Vec<DcvSeedArtifact>,
}

/// Error returned while compiling EVM DCV programs.
#[derive(Debug)]
pub enum DcvCompileError {
    /// Program drafting or config artifact selection failed.
    Plan(mfm_program::PlanError),
    /// Certification failed.
    Certify(mfm_certify::CertifyError),
}

impl std::fmt::Display for DcvCompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Plan(error) => write!(f, "EVM DCV planning failed: {error}"),
            Self::Certify(error) => write!(f, "EVM DCV certification failed: {error}"),
        }
    }
}

impl std::error::Error for DcvCompileError {}

impl From<mfm_program::PlanError> for DcvCompileError {
    fn from(error: mfm_program::PlanError) -> Self {
        Self::Plan(error)
    }
}

impl From<mfm_certify::CertifyError> for DcvCompileError {
    fn from(error: mfm_certify::CertifyError) -> Self {
        Self::Certify(error)
    }
}

/// Canonical bytes for one seed artifact required by a typed EVM DCV phase spec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DcvSeedArtifact {
    /// Seed id assigned by the typed program draft.
    pub seed_id: SeedId,
    /// Canonical content digest.
    pub digest: ContentDigest,
    /// Canonical byte length.
    pub byte_len: u64,
    /// Canonical JSON bytes.
    pub bytes: Vec<u8>,
    /// Seed schema id.
    pub schema_id: SchemaId,
    /// Seed media type.
    pub media_type: spec::MediaType,
}

/// Builds, certifies, and gathers launch artifacts for a deploy-only EVM DCV program.
pub fn compile_dcv_deploy_program(
    config: DeployConfigureValidateDeployConfig,
) -> Result<CompiledDcvProgram, DcvCompileError> {
    compile_dcv_draft(dcv_deploy_program_draft(config)?, Vec::new())
}

/// Builds, certifies, and gathers launch artifacts for a configure-only EVM DCV program.
pub fn compile_dcv_configure_program(
    config: DeployConfigureValidateConfigureConfig,
    deployed_contract: DeployedContract,
) -> Result<CompiledDcvProgram, DcvCompileError> {
    let seed = CanonicalSeed::from_value(&deployed_contract)?;
    let seed_bytes = seed.canonical_json().clone();
    let draft = dcv_configure_program_draft(config, seed)?;
    let seed_artifact = dcv_seed_artifact_for_draft(&draft, DEPLOYED_SEED_KEY, seed_bytes)?;
    compile_dcv_draft(draft, vec![seed_artifact])
}

/// Builds, certifies, and gathers launch artifacts for a validate-only EVM DCV program.
pub fn compile_dcv_validate_program(
    config: DeployConfigureValidateValidateConfig,
    configured_contract: ConfiguredContract,
) -> Result<CompiledDcvProgram, DcvCompileError> {
    let seed = CanonicalSeed::from_value(&configured_contract)?;
    let seed_bytes = seed.canonical_json().clone();
    let draft = dcv_validate_program_draft(config, seed)?;
    let seed_artifact = dcv_seed_artifact_for_draft(&draft, CONFIGURED_SEED_KEY, seed_bytes)?;
    compile_dcv_draft(draft, vec![seed_artifact])
}

/// Builds, certifies, and gathers launch artifacts for the composed EVM DCV program.
pub fn compile_dcv_program(
    config: DeployConfigureValidateCanonicalConfig,
) -> Result<CompiledDcvProgram, DcvCompileError> {
    compile_dcv_draft(dcv_program_draft(config)?, Vec::new())
}

fn compile_dcv_draft(
    draft: mfm_program::TypedProgramDraft,
    seed_artifacts: Vec<DcvSeedArtifact>,
) -> Result<CompiledDcvProgram, DcvCompileError> {
    let certified_spec = certify_program_draft(&draft)?;
    let public_schema_id = certified_spec
        .envelope()
        .spec
        .public_outputs
        .public_schema_id
        .clone();
    let config_artifacts = dcv_config_artifacts_for_spec(&draft, &certified_spec.envelope().spec)?;
    Ok(CompiledDcvProgram {
        certified_spec,
        public_schema_id,
        config_artifacts,
        seed_artifacts,
    })
}

/// Canonical bytes for one config artifact required by a typed EVM DCV spec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DcvConfigArtifact {
    /// Content-addressed artifact id.
    pub artifact_id: ArtifactId,
    /// Canonical content digest.
    pub digest: ContentDigest,
    /// Canonical byte length.
    pub byte_len: u64,
    /// Canonical JSON bytes.
    pub bytes: Vec<u8>,
    /// Config schema id.
    pub schema_id: SchemaId,
    /// Config media type.
    pub media_type: spec::MediaType,
}

/// Returns all author-emitted config artifacts from an EVM DCV draft.
pub fn dcv_draft_config_artifacts(
    draft: &mfm_program::TypedProgramDraft,
) -> mfm_program::Result<Vec<DcvConfigArtifact>> {
    let media_type = spec::MediaType::new("application/json")
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?;
    Ok(draft
        .state_nodes()
        .iter()
        .map(|node| &node.config)
        .chain(draft.operation_lineage().iter().map(|frame| &frame.config))
        .map(|config| {
            config_artifact(
                config.canonical_json.clone(),
                config.schema_id.clone(),
                media_type.clone(),
            )
        })
        .collect())
}

/// Returns framework config artifacts introduced during certification.
pub fn dcv_framework_config_artifacts(
    typed_spec: &spec::TypedExecutionSpec,
) -> mfm_program::Result<Vec<DcvConfigArtifact>> {
    let mut artifacts = Vec::new();
    for node in &typed_spec.nodes {
        let Some(framework) = &node.framework else {
            continue;
        };
        let bytes = spec::framework_config_canonical_json(framework.config_kind(), &node.node_id)
            .map_err(|error| mfm_program::PlanError::Canonical(error.to_string()))?;
        if bytes.content_digest() != node.config_ref.digest
            || bytes.as_bytes().len() as u64 != node.config_ref.byte_len
        {
            return Err(mfm_program::PlanError::Canonical(format!(
                "framework config helper did not match certified config ref for node {}",
                node.node_id
            )));
        }
        artifacts.push(config_artifact(
            bytes,
            node.config_ref.schema_id.clone(),
            node.config_ref.media_type.clone(),
        ));
    }
    Ok(artifacts)
}

/// Returns only config artifacts required by the certified spec, with metadata matching
/// `config_refs`.
pub fn dcv_config_artifacts_for_spec(
    draft: &mfm_program::TypedProgramDraft,
    typed_spec: &spec::TypedExecutionSpec,
) -> mfm_program::Result<Vec<DcvConfigArtifact>> {
    let mut candidates = dcv_draft_config_artifacts(draft)?;
    candidates.extend(dcv_framework_config_artifacts(typed_spec)?);
    let mut selected = Vec::new();
    let mut seen_artifact_ids = BTreeMap::new();
    for config_ref in &typed_spec.config_refs {
        if let Some(previous_schema) =
            seen_artifact_ids.insert(config_ref.artifact_id.clone(), config_ref.schema_id.clone())
        {
            if previous_schema != config_ref.schema_id {
                return Err(mfm_program::PlanError::Key(format!(
                    "certified spec contains duplicate config artifact {} with schemas {} and {}",
                    config_ref.artifact_id, previous_schema, config_ref.schema_id
                )));
            }
            continue;
        }
        let artifact = candidates
            .iter()
            .find(|artifact| {
                artifact.artifact_id == config_ref.artifact_id
                    && artifact.digest == config_ref.digest
                    && artifact.byte_len == config_ref.byte_len
                    && artifact.media_type == config_ref.media_type
                    && artifact.schema_id == config_ref.schema_id
            })
            .cloned()
            .ok_or_else(|| {
                mfm_program::PlanError::Key(format!(
                    "missing typed config artifact for {}",
                    config_ref.artifact_id
                ))
            })?;
        selected.push(artifact);
    }
    Ok(selected)
}

fn normalize_phase_artifacts(
    deploy: &mut DeployConfigureValidateDeployConfig,
    configure: &mut DeployConfigureValidateConfigureConfig,
    validate: &mut DeployConfigureValidateValidateConfig,
) -> mfm_program::Result<()> {
    let Some(artifact) = deploy.artifact.clone() else {
        return Err(mfm_program::PlanError::Key(
            "deploy config must include an inline typed contract artifact; dynamic artifact ports are not part of certified typed EVM DCV execution"
                .to_owned(),
        ));
    };
    if configure.artifact.is_none() {
        configure.artifact = Some(artifact.clone());
    }
    if validate.artifact.is_none() {
        validate.artifact = Some(artifact);
    }
    Ok(())
}

fn validate_phase_alignment(
    deploy: &DeployConfigureValidateDeployConfig,
    configure: &DeployConfigureValidateConfigureConfig,
    validate: &DeployConfigureValidateValidateConfig,
) -> mfm_program::Result<()> {
    if deploy.network_id != configure.network_id || deploy.network_id != validate.network_id {
        return Err(mfm_program::PlanError::Key(
            "deploy/configure/validate network_id values must match".to_owned(),
        ));
    }
    if deploy.control_scope != configure.control_scope
        || deploy.control_scope != validate.control_scope
    {
        return Err(mfm_program::PlanError::Key(
            "deploy/configure/validate control_scope values must match".to_owned(),
        ));
    }
    Ok(())
}

fn config_artifact(
    bytes: mfm_canonical::PlainCanonicalJsonBytes,
    schema_id: SchemaId,
    media_type: spec::MediaType,
) -> DcvConfigArtifact {
    let digest = bytes.content_digest();
    let artifact_id = ArtifactId::from_digest(digest.algorithm(), *digest.digest());
    let byte_len = bytes.as_bytes().len() as u64;
    DcvConfigArtifact {
        artifact_id,
        digest,
        byte_len,
        bytes: bytes.to_vec(),
        schema_id,
        media_type,
    }
}

fn dcv_seed_artifact_for_draft(
    draft: &mfm_program::TypedProgramDraft,
    seed_key: &str,
    bytes: mfm_canonical::PlainCanonicalJsonBytes,
) -> mfm_program::Result<DcvSeedArtifact> {
    let seed = draft
        .seeds()
        .iter()
        .find(|seed| seed.key.as_str() == seed_key)
        .ok_or_else(|| {
            mfm_program::PlanError::Key(format!("missing typed seed artifact for {seed_key}"))
        })?;
    let digest = bytes.content_digest();
    if digest != seed.content_digest || bytes.as_bytes().len() != seed.byte_len {
        return Err(mfm_program::PlanError::Canonical(format!(
            "seed bytes did not match draft seed ref for {seed_key}"
        )));
    }
    Ok(DcvSeedArtifact {
        seed_id: seed.seed_id.clone(),
        digest,
        byte_len: seed.byte_len as u64,
        bytes: bytes.to_vec(),
        schema_id: seed.schema_id.clone(),
        media_type: spec::MediaType::new("application/json")
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mfm_evm_dcv_model::{AbiJson, BytecodeJson, ContractArtifactConfig};

    #[test]
    fn dcv_program_lowers_to_typed_lifecycle_states() {
        let draft = dcv_program_draft(sample_config()).expect("draft");
        let node_keys = draft
            .state_nodes()
            .iter()
            .map(|node| node.key.as_str())
            .collect::<Vec<_>>();
        assert!(node_keys.contains(&"deploy_contract"));
        assert!(node_keys.contains(&"configure_contract"));
        assert!(node_keys.contains(&"validate_contract"));
        assert_eq!(draft.state_nodes().len(), 3);
        assert!(
            draft.state_nodes().iter().all(|node| !node
                .state_descriptor_name
                .contains(&format!("{}{}", "Dyn", "Context"))),
            "EVM DCV state descriptors must not expose dynamic context"
        );

        let certified = certify_program_draft(&draft).expect("certified EVM DCV spec");
        certified.envelope().verify_hash().expect("hash verifies");
        let deploy = certified
            .envelope()
            .spec
            .nodes
            .iter()
            .find(|node| node.stable_key.as_str() == "deploy_contract")
            .expect("deploy node");
        let configure = certified
            .envelope()
            .spec
            .nodes
            .iter()
            .find(|node| node.stable_key.as_str() == "configure_contract")
            .expect("configure node");
        let validate = certified
            .envelope()
            .spec
            .nodes
            .iter()
            .find(|node| node.stable_key.as_str() == "validate_contract")
            .expect("validate node");
        assert!(deploy.side_effect.is_some());
        assert!(configure.side_effect.is_some());
        assert!(validate.side_effect.is_none());
        assert_eq!(deploy.adapter_bindings.len(), 1);
        assert_eq!(configure.adapter_bindings.len(), 1);
        assert_eq!(validate.adapter_bindings.len(), 1);
    }

    #[test]
    fn dcv_program_rejects_missing_inline_deploy_artifact() {
        let mut config = sample_config();
        config.deploy.artifact = None;
        let error = dcv_program_draft(config).expect_err("missing artifact rejected");
        assert!(error
            .to_string()
            .contains("dynamic artifact ports are not part of certified typed EVM DCV execution"));
    }

    #[test]
    fn config_artifacts_match_certified_spec_refs() {
        let draft = dcv_program_draft(sample_config()).expect("draft");
        let certified = certify_program_draft(&draft).expect("certified");
        let artifacts = dcv_config_artifacts_for_spec(&draft, &certified.envelope().spec)
            .expect("config artifacts");
        assert_eq!(artifacts.len(), certified.envelope().spec.config_refs.len());
    }

    #[test]
    fn phase_programs_compile_with_expected_seed_boundaries() {
        let config = sample_config();
        let deploy = compile_dcv_deploy_program(config.deploy.clone()).expect("deploy compiled");
        assert!(deploy.seed_artifacts.is_empty());
        let artifact = config.deploy.artifact.clone();

        let deployed = DeployedContract {
            lifecycle_version: 1,
            network_id: "local".to_owned(),
            control_scope: "shared".to_owned(),
            contract_address: "0x0000000000000000000000000000000000000001".to_owned(),
            deploy_tx_hash: "0x1".to_owned(),
            deploy_receipt_artifact_id: None,
            deployed_block_number: Some(1),
        };
        let mut configure_config = config.configure.clone();
        configure_config.artifact = artifact.clone();
        let configure = compile_dcv_configure_program(configure_config.clone(), deployed.clone())
            .expect("configure compiled");
        assert_eq!(configure.seed_artifacts.len(), 1);

        let configured = ConfiguredContract {
            lifecycle_version: 1,
            deployed,
            configure_calls: configure_config.calls.clone(),
            confirmation_read_assertions: configure_config.confirmation_read_assertions.clone(),
            confirmation_event_assertions: configure_config.confirmation_event_assertions.clone(),
            configure_tx_hashes: Vec::new(),
            configure_receipt_artifact_ids: Vec::new(),
            configured_block_number: Some(2),
        };
        let mut validate_config = config.validate;
        validate_config.artifact = artifact;
        let validate =
            compile_dcv_validate_program(validate_config, configured).expect("validate compiled");
        assert_eq!(validate.seed_artifacts.len(), 1);
    }

    fn sample_config() -> DeployConfigureValidateCanonicalConfig {
        let artifact = sample_artifact();
        DeployConfigureValidateCanonicalConfig {
            machine_id: "evm_deploy_configure_validate".to_owned(),
            pipeline_version: "v1".to_owned(),
            input: DeployConfigureValidateInput::from_json_value(&serde_json::json!({}))
                .expect("input"),
            deploy: DeployConfigureValidateDeployConfig {
                artifact: Some(artifact.clone()),
                network_id: "local".to_owned(),
                control_scope: "shared".to_owned(),
                from: "0x0000000000000000000000000000000000000000".to_owned(),
                constructor_args: Vec::new(),
                value_wei: None,
                signer: sample_signer_config(),
                poll_interval_ms: 1,
                max_receipt_polls: 1,
            },
            configure: DeployConfigureValidateConfigureConfig {
                artifact: None,
                network_id: "local".to_owned(),
                control_scope: "shared".to_owned(),
                from: "0x0000000000000000000000000000000000000000".to_owned(),
                signer: sample_signer_config(),
                calls: vec![mfm_evm_dcv_model::ConfigureCallConfig {
                    function: "configure".to_owned(),
                    args: Vec::new(),
                    value_wei: None,
                }],
                confirmation_read_assertions: vec![mfm_evm_dcv_model::ReadAssertionConfig {
                    function: "getValue".to_owned(),
                    args: Vec::new(),
                    expected: mfm_evm_dcv_model::ExpectedValue::from_json_value(
                        &serde_json::json!(1),
                    )
                    .expect("expected"),
                }],
                confirmation_event_assertions: Vec::new(),
                tx_hashes_export_key: "tx_hashes".to_owned(),
                receipts_export_key: "receipts".to_owned(),
                poll_interval_ms: 1,
                max_receipt_polls: 1,
            },
            validate: DeployConfigureValidateValidateConfig {
                artifact: None,
                network_id: "local".to_owned(),
                control_scope: "shared".to_owned(),
                expected_chain_id: 1,
                require_client_substring: "reth".to_owned(),
                read_assertions: Vec::new(),
                event_assertions: Vec::new(),
            },
        }
    }

    fn sample_signer_config() -> DeployConfigureValidateSignerConfig {
        DeployConfigureValidateSignerConfig::KeystoreEntry {
            entry_id: "550e8400-e29b-41d4-a716-446655440000".to_owned(),
            keystore_path_env: "MFM_TEST_KEYSTORE".to_owned(),
            password_file_env: "MFM_TEST_KEYSTORE_PASSWORD_FILE".to_owned(),
        }
    }

    fn sample_artifact() -> ContractArtifactConfig {
        ContractArtifactConfig {
            abi: AbiJson::from_json_value(&serde_json::json!([
                {"type": "constructor", "inputs": []},
                {"type": "function", "name": "configure", "inputs": [], "outputs": []},
                {"type": "function", "name": "getValue", "inputs": [], "outputs": [{"name": "", "type": "uint256"}]}
            ]))
            .expect("abi"),
            bytecode: BytecodeJson::from_json_value(&serde_json::json!("0x6000"))
                .expect("bytecode"),
        }
    }
}
