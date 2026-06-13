#![warn(missing_docs)]
//! Typed EVM contract lifecycle operation planning.
//!
//! This crate owns deterministic operation topology for EVM contract deploy,
//! configure, validate, and full lifecycle flows. It builds typed state graphs
//! only; live transports, signer providers, artifact stores, and adapters are
//! supplied by app/runtime assembly.
//!
//! ```rust
//! use mfm_op_evm_contract_lifecycle::{
//!     contract_lifecycle_program_draft, ContractLifecycleConfig,
//! };
//!
//! # fn demo(config: ContractLifecycleConfig) -> mfm_program::Result<()> {
//! let draft = contract_lifecycle_program_draft(config)?;
//! assert_eq!(draft.state_nodes().len(), 3);
//! # Ok(())
//! # }
//! ```

use mfm_certify::{certify_program_draft, CertifiedTypedSpec};
use mfm_evm_contract_config::{
    ConfigurePhaseConfig, DeployPhaseConfig, EvmNetworkIntent, ValidatePhaseConfig,
};
use mfm_evm_contract_model::{ConfiguredContract, DeployedContract, ValidationReport};
use mfm_ids::{
    ArtifactId, ContentDigest, DigestAlgorithm, OperationKind, OperationVersion, SchemaId,
};
use mfm_program::{
    build_root_with_registries, CanonicalSeed, Handle, Operation, OperationExpansion, OperationKey,
    OperationRegistryBuilder, PublicOutputKey, RootBound, RootBuilder, SagaPolicy, ScopeKey,
    SeedKey, StateKey, StateRegistryBuilder,
};
use mfm_program_derive::{MfmConfig, OperationOutput, PublicOutputs};
use mfm_spec::v1 as spec;
use mfm_state_evm_contracts::{
    ConfigureContractInputHandles, ConfigureContractState, ContractLifecyclePublicOutputs,
    DeployContractState, ValidateContractInputHandles, ValidateContractState,
};
use serde::{Deserialize, Serialize};

const OP_NAMESPACE: &str = "mfm.evm.contract";
const ROOT_SCOPE: &str = "evm_contract";
const DEPLOY_OP_KEY: &str = "contract_deploy";
const CONFIGURE_OP_KEY: &str = "contract_configure";
const VALIDATE_OP_KEY: &str = "contract_validate";
const LIFECYCLE_OP_KEY: &str = "contract_lifecycle";
const DEPLOY_SEED_KEY: &str = "deployed_contract";
const CONFIGURED_SEED_KEY: &str = "configured_contract";
const PUBLIC_OUTPUT_KEY: &str = "contract";

/// Aggregate full lifecycle authored config owned by the operation layer.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmConfig)]
#[mfm(schema = "mfm.evm.contract.operation.config.lifecycle")]
pub struct ContractLifecycleConfig {
    /// Config contract version.
    pub lifecycle_version: u64,
    /// Deploy phase config.
    pub deploy: DeployPhaseConfig,
    /// Configure phase config.
    pub configure: ConfigurePhaseConfig,
    /// Validate phase config.
    pub validate: ValidatePhaseConfig,
}

impl ContractLifecycleConfig {
    /// Creates a validated full lifecycle config.
    pub fn new(
        deploy: DeployPhaseConfig,
        configure: ConfigurePhaseConfig,
        validate: ValidatePhaseConfig,
    ) -> Self {
        Self {
            lifecycle_version: 1,
            deploy,
            configure,
            validate,
        }
    }

    fn validate(&self) -> mfm_program::Result<()> {
        if self.lifecycle_version != 1 {
            return Err(mfm_program::PlanError::Key(
                "unsupported contract lifecycle config version".to_owned(),
            ));
        }
        ensure_phase_networks_match(&self.deploy, &self.configure, &self.validate)?;
        Ok(())
    }
}

/// Output handles produced by deploy-only operation planning.
#[derive(OperationOutput)]
#[mfm(schema = "mfm.evm.contract.operation_outputs.deploy")]
pub struct ContractDeployOperationOutputs<'program, 'scope> {
    /// Deployed contract typestate.
    pub deployed: Handle<'program, 'scope, DeployedContract>,
}

/// Public output handles for deploy-only programs.
#[derive(PublicOutputs)]
#[mfm(schema = "mfm.evm.contract.public_outputs.deploy")]
pub struct ContractDeployPublicOutputs<'program, 'scope> {
    /// Deployed contract typestate.
    pub deployed: Handle<'program, 'scope, DeployedContract>,
}

/// Output handles produced by configure-only operation planning.
#[derive(OperationOutput)]
#[mfm(schema = "mfm.evm.contract.operation_outputs.configure")]
pub struct ContractConfigureOperationOutputs<'program, 'scope> {
    /// Configured contract typestate.
    pub configured: Handle<'program, 'scope, ConfiguredContract>,
}

/// Public output handles for configure-only programs.
#[derive(PublicOutputs)]
#[mfm(schema = "mfm.evm.contract.public_outputs.configure")]
pub struct ContractConfigurePublicOutputs<'program, 'scope> {
    /// Configured contract typestate.
    pub configured: Handle<'program, 'scope, ConfiguredContract>,
}

/// Output handles produced by validate-only operation planning.
#[derive(OperationOutput)]
#[mfm(schema = "mfm.evm.contract.operation_outputs.validate")]
pub struct ContractValidateOperationOutputs<'program, 'scope> {
    /// Validation report.
    pub validation_report: Handle<'program, 'scope, ValidationReport>,
}

/// Public output handles for validate-only programs.
#[derive(PublicOutputs)]
#[mfm(schema = "mfm.evm.contract.public_outputs.validate")]
pub struct ContractValidatePublicOutputs<'program, 'scope> {
    /// Validation report.
    pub validation_report: Handle<'program, 'scope, ValidationReport>,
}

/// Output handles produced by full lifecycle operation planning.
#[derive(OperationOutput)]
#[mfm(schema = "mfm.evm.contract.operation_outputs.lifecycle")]
pub struct ContractLifecycleOperationOutputs<'program, 'scope> {
    /// Deployed contract typestate.
    pub deployed: Handle<'program, 'scope, DeployedContract>,
    /// Configured contract typestate.
    pub configured: Handle<'program, 'scope, ConfiguredContract>,
    /// Validation report.
    pub validation_report: Handle<'program, 'scope, ValidationReport>,
}

/// Deploy-only EVM contract planning operation.
pub struct DeployContractOperation;

impl Operation for DeployContractOperation {
    type Config = DeployPhaseConfig;
    type Input<'program, 'scope> = ();
    type Output<'program, 'scope> = ContractDeployOperationOutputs<'program, 'scope>;

    fn kind() -> mfm_program::Result<OperationKind> {
        operation_kind("deploy", b"mfm.evm.contract.operation:deploy")
    }

    fn version() -> mfm_program::Result<OperationVersion> {
        operation_version("deploy")
    }

    fn name() -> &'static str {
        "mfm.evm.contract.deploy"
    }

    fn expand<'program, 'scope>(
        &self,
        config: Self::Config,
        _input: Self::Input<'program, 'scope>,
        builder: &mut OperationExpansion<'program, 'scope>,
        _dispatch: mfm_program::OperationExpansionDispatch<Self>,
    ) -> mfm_program::Result<Self::Output<'program, 'scope>> {
        let deployed =
            builder.state::<DeployContractState, _>(StateKey::new("deploy")?, config, ())?;
        Ok(ContractDeployOperationOutputs { deployed })
    }
}

/// Configure-only EVM contract planning operation.
pub struct ConfigureContractOperation;

impl Operation for ConfigureContractOperation {
    type Config = ConfigurePhaseConfig;
    type Input<'program, 'scope> = Handle<'program, 'scope, DeployedContract>;
    type Output<'program, 'scope> = ContractConfigureOperationOutputs<'program, 'scope>;

    fn kind() -> mfm_program::Result<OperationKind> {
        operation_kind("configure", b"mfm.evm.contract.operation:configure")
    }

    fn version() -> mfm_program::Result<OperationVersion> {
        operation_version("configure")
    }

    fn name() -> &'static str {
        "mfm.evm.contract.configure"
    }

    fn expand<'program, 'scope>(
        &self,
        config: Self::Config,
        deployed: Self::Input<'program, 'scope>,
        builder: &mut OperationExpansion<'program, 'scope>,
        _dispatch: mfm_program::OperationExpansionDispatch<Self>,
    ) -> mfm_program::Result<Self::Output<'program, 'scope>> {
        let configured = builder.state::<ConfigureContractState, _>(
            StateKey::new("configure")?,
            config,
            ConfigureContractInputHandles { deployed },
        )?;
        Ok(ContractConfigureOperationOutputs { configured })
    }
}

/// Validate-only EVM contract planning operation.
pub struct ValidateContractOperation;

impl Operation for ValidateContractOperation {
    type Config = ValidatePhaseConfig;
    type Input<'program, 'scope> = Handle<'program, 'scope, ConfiguredContract>;
    type Output<'program, 'scope> = ContractValidateOperationOutputs<'program, 'scope>;

    fn kind() -> mfm_program::Result<OperationKind> {
        operation_kind("validate", b"mfm.evm.contract.operation:validate")
    }

    fn version() -> mfm_program::Result<OperationVersion> {
        operation_version("validate")
    }

    fn name() -> &'static str {
        "mfm.evm.contract.validate"
    }

    fn expand<'program, 'scope>(
        &self,
        config: Self::Config,
        configured: Self::Input<'program, 'scope>,
        builder: &mut OperationExpansion<'program, 'scope>,
        _dispatch: mfm_program::OperationExpansionDispatch<Self>,
    ) -> mfm_program::Result<Self::Output<'program, 'scope>> {
        let validation_report = builder.state::<ValidateContractState, _>(
            StateKey::new("validate")?,
            config,
            ValidateContractInputHandles { configured },
        )?;
        Ok(ContractValidateOperationOutputs { validation_report })
    }
}

/// Full contract lifecycle planning operation.
pub struct ContractLifecycleOperation;

impl Operation for ContractLifecycleOperation {
    type Config = ContractLifecycleConfig;
    type Input<'program, 'scope> = ();
    type Output<'program, 'scope> = ContractLifecycleOperationOutputs<'program, 'scope>;

    fn kind() -> mfm_program::Result<OperationKind> {
        operation_kind("lifecycle", b"mfm.evm.contract.operation:lifecycle")
    }

    fn version() -> mfm_program::Result<OperationVersion> {
        operation_version("lifecycle")
    }

    fn name() -> &'static str {
        "mfm.evm.contract.lifecycle"
    }

    fn expand<'program, 'scope>(
        &self,
        config: Self::Config,
        _input: Self::Input<'program, 'scope>,
        builder: &mut OperationExpansion<'program, 'scope>,
        _dispatch: mfm_program::OperationExpansionDispatch<Self>,
    ) -> mfm_program::Result<Self::Output<'program, 'scope>> {
        config.validate()?;
        let deployed =
            builder.state::<DeployContractState, _>(StateKey::new("deploy")?, config.deploy, ())?;
        let configured = builder.state::<ConfigureContractState, _>(
            StateKey::new("configure")?,
            config.configure,
            ConfigureContractInputHandles {
                deployed: deployed.clone(),
            },
        )?;
        let validation_report = builder.state::<ValidateContractState, _>(
            StateKey::new("validate")?,
            config.validate,
            ValidateContractInputHandles {
                configured: configured.clone(),
            },
        )?;
        Ok(ContractLifecycleOperationOutputs {
            deployed,
            configured,
            validation_report,
        })
    }
}

/// Builds the contract lifecycle state registry used for authoring and certification.
pub fn contract_lifecycle_state_registry() -> mfm_program::Result<mfm_program::StateRegistrySnapshot>
{
    let mut states = StateRegistryBuilder::new();
    states.register::<DeployContractState>()?;
    states.register::<ConfigureContractState>()?;
    states.register::<ValidateContractState>()?;
    Ok(states.into_snapshot())
}

/// Builds the contract lifecycle operation registry used for authoring and certification.
pub fn contract_lifecycle_operation_registry(
) -> mfm_program::Result<mfm_program::OperationRegistrySnapshot> {
    let mut operations = OperationRegistryBuilder::new();
    operations.register::<DeployContractOperation>()?;
    operations.register::<ConfigureContractOperation>()?;
    operations.register::<ValidateContractOperation>()?;
    operations.register::<ContractLifecycleOperation>()?;
    Ok(operations.into_snapshot())
}

/// Adds lifecycle operation descriptors to a trusted certification registry.
pub fn register_contract_lifecycle_certification_descriptors(
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
            .register::<DeployContractOperation>()
            .map_err(|error| mfm_certify::CertifyError::Lowering(error.to_string()))?,
    )?;
    registry.register_operation(
        &operations
            .register::<ConfigureContractOperation>()
            .map_err(|error| mfm_certify::CertifyError::Lowering(error.to_string()))?,
    )?;
    registry.register_operation(
        &operations
            .register::<ValidateContractOperation>()
            .map_err(|error| mfm_certify::CertifyError::Lowering(error.to_string()))?,
    )?;
    registry.register_operation(
        &operations
            .register::<ContractLifecycleOperation>()
            .map_err(|error| mfm_certify::CertifyError::Lowering(error.to_string()))?,
    )?;
    Ok(())
}

/// Builds a typed deploy-only program draft.
pub fn deploy_contract_program_draft(
    config: DeployPhaseConfig,
) -> mfm_program::Result<mfm_program::TypedProgramDraft> {
    build_program(|root| {
        root.set_saga_policy(SagaPolicy::FailWithoutAcdcClaim)?;
        let result = root.scope().call::<DeployContractOperation, _>(
            OperationKey::new(DEPLOY_OP_KEY)?,
            DeployContractOperation,
            config,
            (),
        )?;
        root.bind_public_outputs(
            PublicOutputKey::new(PUBLIC_OUTPUT_KEY)?,
            &ContractDeployPublicOutputs {
                deployed: result.deployed,
            },
        )
    })
}

/// Builds a typed configure-only program draft from a launch seed deployment.
pub fn configure_contract_program_draft(
    config: ConfigurePhaseConfig,
    deployed: DeployedContract,
) -> mfm_program::Result<mfm_program::TypedProgramDraft> {
    ensure_network_matches_deployed(config.network(), &deployed)?;
    build_program(|root| {
        root.set_saga_policy(SagaPolicy::FailWithoutAcdcClaim)?;
        let deployed = root.seed(
            SeedKey::new(DEPLOY_SEED_KEY)?,
            CanonicalSeed::from_value(&deployed)?,
        )?;
        let result = root.scope().call::<ConfigureContractOperation, _>(
            OperationKey::new(CONFIGURE_OP_KEY)?,
            ConfigureContractOperation,
            config,
            deployed,
        )?;
        root.bind_public_outputs(
            PublicOutputKey::new(PUBLIC_OUTPUT_KEY)?,
            &ContractConfigurePublicOutputs {
                configured: result.configured,
            },
        )
    })
}

/// Builds a typed validate-only program draft from a launch seed configured contract.
pub fn validate_contract_program_draft(
    config: ValidatePhaseConfig,
    configured: ConfiguredContract,
) -> mfm_program::Result<mfm_program::TypedProgramDraft> {
    ensure_network_matches_configured(config.network(), &configured)?;
    build_program(|root| {
        root.set_saga_policy(SagaPolicy::NoSideEffects)?;
        let configured = root.seed(
            SeedKey::new(CONFIGURED_SEED_KEY)?,
            CanonicalSeed::from_value(&configured)?,
        )?;
        let result = root.scope().call::<ValidateContractOperation, _>(
            OperationKey::new(VALIDATE_OP_KEY)?,
            ValidateContractOperation,
            config,
            configured,
        )?;
        root.bind_public_outputs(
            PublicOutputKey::new(PUBLIC_OUTPUT_KEY)?,
            &ContractValidatePublicOutputs {
                validation_report: result.validation_report,
            },
        )
    })
}

/// Builds a typed full lifecycle program draft.
pub fn contract_lifecycle_program_draft(
    config: ContractLifecycleConfig,
) -> mfm_program::Result<mfm_program::TypedProgramDraft> {
    build_program(|root| {
        root.set_saga_policy(SagaPolicy::FailWithoutAcdcClaim)?;
        let result = root.scope().call::<ContractLifecycleOperation, _>(
            OperationKey::new(LIFECYCLE_OP_KEY)?,
            ContractLifecycleOperation,
            config,
            (),
        )?;
        root.bind_public_outputs(
            PublicOutputKey::new(PUBLIC_OUTPUT_KEY)?,
            &ContractLifecyclePublicOutputs {
                deployed: result.deployed,
                configured: result.configured,
                validation_report: result.validation_report,
            },
        )
    })
}

/// Builds and certifies the typed full lifecycle program.
pub fn certified_contract_lifecycle_spec(
    config: ContractLifecycleConfig,
) -> mfm_certify::Result<CertifiedTypedSpec> {
    let draft = contract_lifecycle_program_draft(config)
        .map_err(|error| mfm_certify::CertifyError::Lowering(error.to_string()))?;
    certify_program_draft(&draft)
}

/// Fully compiled contract lifecycle launch program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledContractLifecycleProgram {
    /// Certifier-backed typed spec authority.
    pub certified_spec: CertifiedTypedSpec,
    /// Public output schema id exposed by the certified spec.
    pub public_schema_id: SchemaId,
    /// Config artifacts required to launch the certified spec.
    pub config_artifacts: Vec<ContractLifecycleConfigArtifact>,
}

/// Error returned while compiling a contract lifecycle program.
#[derive(Debug, thiserror::Error)]
pub enum ContractLifecycleCompileError {
    /// Program planning failed.
    #[error("contract lifecycle planning failed: {0}")]
    Plan(#[from] mfm_program::PlanError),
    /// Certification failed.
    #[error("contract lifecycle certification failed: {0}")]
    Certify(#[from] mfm_certify::CertifyError),
}

/// Builds, certifies, and gathers launch config artifacts for a deploy-only program.
pub fn compile_contract_deploy_program(
    config: DeployPhaseConfig,
) -> Result<CompiledContractLifecycleProgram, ContractLifecycleCompileError> {
    compile_draft(deploy_contract_program_draft(config)?)
}

/// Builds, certifies, and gathers launch config artifacts for a configure-only program.
pub fn compile_contract_configure_program(
    config: ConfigurePhaseConfig,
    deployed: DeployedContract,
) -> Result<CompiledContractLifecycleProgram, ContractLifecycleCompileError> {
    compile_draft(configure_contract_program_draft(config, deployed)?)
}

/// Builds, certifies, and gathers launch config artifacts for a validate-only program.
pub fn compile_contract_validate_program(
    config: ValidatePhaseConfig,
    configured: ConfiguredContract,
) -> Result<CompiledContractLifecycleProgram, ContractLifecycleCompileError> {
    compile_draft(validate_contract_program_draft(config, configured)?)
}

/// Builds, certifies, and gathers launch config artifacts for a full lifecycle program.
pub fn compile_contract_lifecycle_program(
    config: ContractLifecycleConfig,
) -> Result<CompiledContractLifecycleProgram, ContractLifecycleCompileError> {
    compile_draft(contract_lifecycle_program_draft(config)?)
}

/// Canonical bytes for one config artifact required by a typed lifecycle spec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractLifecycleConfigArtifact {
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

/// Returns all author-emitted config artifacts from a lifecycle draft.
pub fn contract_lifecycle_draft_config_artifacts(
    draft: &mfm_program::TypedProgramDraft,
) -> mfm_program::Result<Vec<ContractLifecycleConfigArtifact>> {
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
pub fn contract_lifecycle_framework_config_artifacts(
    typed_spec: &spec::TypedExecutionSpec,
) -> mfm_program::Result<Vec<ContractLifecycleConfigArtifact>> {
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

/// Returns only config artifacts required by the certified spec.
pub fn contract_lifecycle_config_artifacts_for_spec(
    draft: &mfm_program::TypedProgramDraft,
    typed_spec: &spec::TypedExecutionSpec,
) -> mfm_program::Result<Vec<ContractLifecycleConfigArtifact>> {
    let mut candidates = contract_lifecycle_draft_config_artifacts(draft)?;
    candidates.extend(contract_lifecycle_framework_config_artifacts(typed_spec)?);
    let mut selected = Vec::new();
    for config_ref in &typed_spec.config_refs {
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

fn compile_draft(
    draft: mfm_program::TypedProgramDraft,
) -> Result<CompiledContractLifecycleProgram, ContractLifecycleCompileError> {
    let certified_spec = certify_program_draft(&draft)?;
    let public_schema_id = certified_spec
        .envelope()
        .spec
        .public_outputs
        .public_schema_id
        .clone();
    let config_artifacts =
        contract_lifecycle_config_artifacts_for_spec(&draft, &certified_spec.envelope().spec)?;
    Ok(CompiledContractLifecycleProgram {
        certified_spec,
        public_schema_id,
        config_artifacts,
    })
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

fn ensure_phase_networks_match(
    deploy: &DeployPhaseConfig,
    configure: &ConfigurePhaseConfig,
    validate: &ValidatePhaseConfig,
) -> mfm_program::Result<()> {
    ensure_network_pair_matches(deploy.network(), configure.network(), "deploy/configure")?;
    ensure_network_pair_matches(deploy.network(), validate.network(), "deploy/validate")
}

fn ensure_network_pair_matches(
    left: &EvmNetworkIntent,
    right: &EvmNetworkIntent,
    label: &'static str,
) -> mfm_program::Result<()> {
    if left.network_id() != right.network_id() {
        return Err(mfm_program::PlanError::Key(format!(
            "contract lifecycle {label} network id mismatch"
        )));
    }
    if left.expected_chain_id() != right.expected_chain_id() {
        return Err(mfm_program::PlanError::Key(format!(
            "contract lifecycle {label} expected chain id mismatch"
        )));
    }
    Ok(())
}

fn ensure_network_matches_deployed(
    network: &EvmNetworkIntent,
    deployed: &DeployedContract,
) -> mfm_program::Result<()> {
    if network.network_id() != deployed.network_id {
        return Err(mfm_program::PlanError::Key(
            "contract lifecycle seed network id mismatch".to_owned(),
        ));
    }
    if network.expected_chain_id() != deployed.expected_chain_id {
        return Err(mfm_program::PlanError::Key(
            "contract lifecycle seed expected chain id mismatch".to_owned(),
        ));
    }
    Ok(())
}

fn ensure_network_matches_configured(
    network: &EvmNetworkIntent,
    configured: &ConfiguredContract,
) -> mfm_program::Result<()> {
    ensure_network_matches_deployed(network, &configured.deployed)
}

fn config_artifact(
    bytes: mfm_canonical::PlainCanonicalJsonBytes,
    schema_id: SchemaId,
    media_type: spec::MediaType,
) -> ContractLifecycleConfigArtifact {
    let digest = bytes.content_digest();
    let artifact_id = ArtifactId::from_digest(digest.algorithm(), *digest.digest());
    let byte_len = bytes.as_bytes().len() as u64;
    ContractLifecycleConfigArtifact {
        artifact_id,
        digest,
        byte_len,
        bytes: bytes.to_vec(),
        schema_id,
        media_type,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn network_json() -> serde_json::Value {
        network_json_for_chain(1)
    }

    fn network_json_for_chain(chain_id: u64) -> serde_json::Value {
        serde_json::json!({
            "network_id": "ethereum-mainnet",
            "expected_chain_id": chain_id,
        })
    }

    fn signer_json() -> serde_json::Value {
        serde_json::json!({
            "signer_ref": "deployer",
            "expected_signer_address": "0x000000000000000000000000000000000000dead",
        })
    }

    fn deploy_config() -> DeployPhaseConfig {
        serde_json::from_value(serde_json::json!({
            "network": network_json(),
            "signer": signer_json(),
        }))
        .expect("deploy config")
    }

    fn configure_config() -> ConfigurePhaseConfig {
        configure_config_for_chain(1)
    }

    fn configure_config_for_chain(chain_id: u64) -> ConfigurePhaseConfig {
        serde_json::from_value(serde_json::json!({
            "network": network_json_for_chain(chain_id),
            "signer": signer_json(),
            "calls": [],
        }))
        .expect("configure config")
    }

    fn validate_config() -> ValidatePhaseConfig {
        validate_config_for_chain(1)
    }

    fn validate_config_for_chain(chain_id: u64) -> ValidatePhaseConfig {
        serde_json::from_value(serde_json::json!({
            "network": network_json_for_chain(chain_id),
        }))
        .expect("validate config")
    }

    fn lifecycle_config() -> ContractLifecycleConfig {
        ContractLifecycleConfig::new(deploy_config(), configure_config(), validate_config())
    }

    fn deployed_contract() -> DeployedContract {
        deployed_contract_on_chain(1)
    }

    fn deployed_contract_on_chain(chain_id: u64) -> DeployedContract {
        DeployedContract {
            lifecycle_version: 1,
            network_id: "ethereum-mainnet".to_owned(),
            expected_chain_id: chain_id,
            contract_address: "0x000000000000000000000000000000000000dead".to_owned(),
            deploy_tx_hash: "0x01".to_owned(),
            deploy_receipt_evidence: None,
            deployed_block_number: Some(1),
        }
    }

    fn configured_contract() -> ConfiguredContract {
        configured_contract_on_chain(1)
    }

    fn configured_contract_on_chain(chain_id: u64) -> ConfiguredContract {
        ConfiguredContract {
            lifecycle_version: 1,
            deployed: deployed_contract_on_chain(chain_id),
            configure_calls: Vec::new(),
            confirmation_read_assertions: Vec::new(),
            confirmation_event_assertions: Vec::new(),
            configure_tx_hashes: Vec::new(),
            configure_receipt_evidence: Vec::new(),
            configured_block_number: Some(2),
        }
    }

    #[test]
    fn full_lifecycle_program_rejects_phase_network_mismatch() {
        let config = ContractLifecycleConfig::new(
            deploy_config(),
            configure_config_for_chain(2),
            validate_config(),
        );

        let error = contract_lifecycle_program_draft(config).expect_err("network mismatch");

        assert!(error
            .to_string()
            .contains("contract lifecycle deploy/configure expected chain id mismatch"));
    }

    #[test]
    fn configure_program_rejects_seed_typestate_network_mismatch() {
        let error =
            configure_contract_program_draft(configure_config(), deployed_contract_on_chain(2))
                .expect_err("seed mismatch");

        assert!(error
            .to_string()
            .contains("contract lifecycle seed expected chain id mismatch"));
    }

    #[test]
    fn validate_program_rejects_seed_typestate_network_mismatch() {
        let error =
            validate_contract_program_draft(validate_config(), configured_contract_on_chain(2))
                .expect_err("seed mismatch");

        assert!(error
            .to_string()
            .contains("contract lifecycle seed expected chain id mismatch"));
    }

    #[test]
    fn full_lifecycle_program_lowers_to_three_ordered_states() {
        let draft = contract_lifecycle_program_draft(lifecycle_config()).expect("draft");

        assert_eq!(draft.state_nodes().len(), 3);
        assert_eq!(draft.state_nodes()[0].key.as_str(), "deploy");
        assert_eq!(draft.state_nodes()[1].key.as_str(), "configure");
        assert_eq!(draft.state_nodes()[2].key.as_str(), "validate");
        assert!(draft
            .state_nodes()
            .iter()
            .all(|node| node.state_descriptor_name.starts_with("mfm.evm.contract.")));

        let certified = certify_program_draft(&draft).expect("certified");
        certified.envelope().verify_hash().expect("hash verifies");
        assert!(certified
            .envelope()
            .spec
            .nodes
            .iter()
            .filter(|node| {
                matches!(
                    node.stable_key.as_str(),
                    "deploy" | "configure" | "validate"
                )
            })
            .all(|node| !node.adapter_bindings.is_empty() || node.stable_key.as_str() == "root"));
    }

    #[test]
    fn phase_program_helpers_are_planning_only() {
        let deploy = deploy_contract_program_draft(deploy_config()).expect("deploy draft");
        let configure = configure_contract_program_draft(configure_config(), deployed_contract())
            .expect("configure draft");
        let validate = validate_contract_program_draft(validate_config(), configured_contract())
            .expect("validate draft");

        assert_eq!(deploy.state_nodes().len(), 1);
        assert_eq!(configure.state_nodes().len(), 1);
        assert_eq!(configure.seeds().len(), 1);
        assert_eq!(validate.state_nodes().len(), 1);
        assert_eq!(validate.seeds().len(), 1);
    }

    #[test]
    fn compile_lifecycle_program_gathers_config_artifacts() {
        let compiled =
            compile_contract_lifecycle_program(lifecycle_config()).expect("compiled lifecycle");

        assert!(!compiled.config_artifacts.is_empty());
        assert!(compiled
            .config_artifacts
            .iter()
            .all(|artifact| artifact.media_type.as_str() == "application/json"));
    }

    #[test]
    fn operation_ids_use_contract_lifecycle_namespace() {
        let names = [
            DeployContractOperation::name(),
            ConfigureContractOperation::name(),
            ValidateContractOperation::name(),
            ContractLifecycleOperation::name(),
        ];
        assert!(names
            .iter()
            .all(|name| name.starts_with("mfm.evm.contract.")));

        let versions = [
            DeployContractOperation::version().expect("deploy"),
            ConfigureContractOperation::version().expect("configure"),
            ValidateContractOperation::version().expect("validate"),
            ContractLifecycleOperation::version().expect("lifecycle"),
        ];
        assert!(versions
            .iter()
            .all(|version| version.as_str().starts_with("mfm.evm.contract.operation.")));
    }

    #[test]
    fn operation_crate_has_no_live_runtime_dependencies() {
        let source = include_str!("lib.rs");
        for forbidden in [
            ["mfm", "_transports"].concat(),
            ["mfm", "_adapters"].concat(),
            ["mfm", "_signers"].concat(),
            ["artifact", "_store"].concat(),
            ["rpc", "_url"].concat(),
            ["key", "store"].concat(),
            ["private", "_key"].concat(),
            ["pass", "word"].concat(),
        ] {
            assert!(
                !source.contains(&forbidden),
                "operation source contains live runtime dependency term {forbidden}"
            );
        }
    }
}
