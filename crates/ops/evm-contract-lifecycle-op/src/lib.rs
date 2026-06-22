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

use mfm_authored_config::{EntryPointDescriptor, TOML_JSON_AUTHORED_CONFIG_FORMATS};
use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_evm_contract_config::{
    ConfigurePhaseConfig, DeployPhaseConfig, EvmNetworkIntent, ValidatePhaseConfig,
};
use mfm_evm_contract_model::{ConfiguredContract, DeployedContract, ValidationReport};
use mfm_ids::{DigestAlgorithm, OperationKind, OperationVersion};
use mfm_program::{
    build_root_with_registries, CanonicalSeed, Handle, Operation, OperationExpansion, OperationKey,
    OperationRegistryBuilder, PublicOutputKey, RootBound, RootBuilder, ScopeKey, SeedKey,
    SideEffectSagaPolicy, StateKey, StateRegistryBuilder, TypedProgramConfigMaterial,
    TypedProgramLaunchPlan, TypedProgramSeedMaterial,
};
use mfm_program_derive::{MfmConfig, OperationOutput, PublicOutputs};
use mfm_spec::v1::MediaType;
use mfm_state_evm_contracts::{
    account_nonce_resource_claim, ConfigureContractInputHandles, ConfigureContractState,
    ContractLifecyclePublicOutputs, DeployContractState, ValidateContractInputHandles,
    ValidateContractState,
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

/// Public deploy-only EVM contract entry-point descriptor.
pub const CONTRACT_DEPLOY_ENTRY_POINT: EntryPointDescriptor = EntryPointDescriptor {
    namespace: OP_NAMESPACE,
    name: DEPLOY_OP_KEY,
    public_name: "evm_contract_deploy",
    version: 1,
    accepted_config_formats: TOML_JSON_AUTHORED_CONFIG_FORMATS,
};

/// Public configure-only EVM contract entry-point descriptor.
pub const CONTRACT_CONFIGURE_ENTRY_POINT: EntryPointDescriptor = EntryPointDescriptor {
    namespace: OP_NAMESPACE,
    name: CONFIGURE_OP_KEY,
    public_name: "evm_contract_configure",
    version: 1,
    accepted_config_formats: TOML_JSON_AUTHORED_CONFIG_FORMATS,
};

/// Public validate-only EVM contract entry-point descriptor.
pub const CONTRACT_VALIDATE_ENTRY_POINT: EntryPointDescriptor = EntryPointDescriptor {
    namespace: OP_NAMESPACE,
    name: VALIDATE_OP_KEY,
    public_name: "evm_contract_validate",
    version: 1,
    accepted_config_formats: TOML_JSON_AUTHORED_CONFIG_FORMATS,
};

/// Public full lifecycle EVM contract entry-point descriptor.
pub const CONTRACT_LIFECYCLE_ENTRY_POINT: EntryPointDescriptor = EntryPointDescriptor {
    namespace: OP_NAMESPACE,
    name: LIFECYCLE_OP_KEY,
    public_name: "evm_contract_lifecycle",
    version: 1,
    accepted_config_formats: TOML_JSON_AUTHORED_CONFIG_FORMATS,
};

/// Public EVM contract entry-point descriptors exported by this operation crate.
pub const CONTRACT_ENTRY_POINTS: [EntryPointDescriptor; 4] = [
    CONTRACT_DEPLOY_ENTRY_POINT,
    CONTRACT_CONFIGURE_ENTRY_POINT,
    CONTRACT_VALIDATE_ENTRY_POINT,
    CONTRACT_LIFECYCLE_ENTRY_POINT,
];

/// Aggregate full lifecycle authored config owned by the operation layer.
#[derive(Debug, Clone, Serialize, PartialEq, Eq, MfmConfig)]
#[mfm(
    schema = "mfm.evm.contract.operation.config.lifecycle",
    validate = "validate_contract_lifecycle_config"
)]
pub struct ContractLifecycleConfig {
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
            deploy,
            configure,
            validate,
        }
    }
}

impl<'de> Deserialize<'de> for ContractLifecycleConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct RawContractLifecycleConfig {
            deploy: DeployPhaseConfig,
            configure: ConfigurePhaseConfig,
            validate: ValidatePhaseConfig,
        }

        let raw = RawContractLifecycleConfig::deserialize(deserializer)?;
        Ok(Self::new(raw.deploy, raw.configure, raw.validate))
    }
}

fn validate_contract_lifecycle_config(config: &ContractLifecycleConfig) -> Result<(), String> {
    ensure_phase_networks_match(&config.deploy, &config.configure, &config.validate)
        .map_err(|error| error.to_string())
}

/// Configure entry-point config carrying the phase config and deployed typestate seed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractConfigureEntryPointConfig {
    /// Configure phase config.
    pub config: ConfigurePhaseConfig,
    /// Deployed contract typestate seed.
    pub deployed: DeployedContract,
}

/// Validate entry-point config carrying the phase config and configured typestate seed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractValidateEntryPointConfig {
    /// Validate phase config.
    pub config: ValidatePhaseConfig,
    /// Configured contract typestate seed.
    pub configured: ConfiguredContract,
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
        config: mfm_program::ValidatedConfig<Self::Config>,
        _input: Self::Input<'program, 'scope>,
        builder: &mut OperationExpansion<'program, 'scope>,
        _dispatch: mfm_program::OperationExpansionDispatch<Self>,
    ) -> mfm_program::Result<Self::Output<'program, 'scope>> {
        let config = config.into_inner();
        let deployed = builder
            .side_effect::<DeployContractState, _>(
                StateKey::new("deploy")?,
                config,
                (),
                account_nonce_resource_claim()?,
            )?
            .into_handle();
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
        config: mfm_program::ValidatedConfig<Self::Config>,
        deployed: Self::Input<'program, 'scope>,
        builder: &mut OperationExpansion<'program, 'scope>,
        _dispatch: mfm_program::OperationExpansionDispatch<Self>,
    ) -> mfm_program::Result<Self::Output<'program, 'scope>> {
        let config = config.into_inner();
        let configured = builder
            .side_effect::<ConfigureContractState, _>(
                StateKey::new("configure")?,
                config,
                ConfigureContractInputHandles { deployed },
                account_nonce_resource_claim()?,
            )?
            .into_handle();
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
        config: mfm_program::ValidatedConfig<Self::Config>,
        configured: Self::Input<'program, 'scope>,
        builder: &mut OperationExpansion<'program, 'scope>,
        _dispatch: mfm_program::OperationExpansionDispatch<Self>,
    ) -> mfm_program::Result<Self::Output<'program, 'scope>> {
        let config = config.into_inner();
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
        config: mfm_program::ValidatedConfig<Self::Config>,
        _input: Self::Input<'program, 'scope>,
        builder: &mut OperationExpansion<'program, 'scope>,
        _dispatch: mfm_program::OperationExpansionDispatch<Self>,
    ) -> mfm_program::Result<Self::Output<'program, 'scope>> {
        let config = config.into_inner();
        let deployed = builder
            .side_effect::<DeployContractState, _>(
                StateKey::new("deploy")?,
                config.deploy,
                (),
                account_nonce_resource_claim()?,
            )?
            .into_handle();
        let configured = builder
            .side_effect::<ConfigureContractState, _>(
                StateKey::new("configure")?,
                config.configure,
                ConfigureContractInputHandles {
                    deployed: deployed.clone(),
                },
                account_nonce_resource_claim()?,
            )?
            .into_handle();
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
        root.set_saga_policy(SideEffectSagaPolicy::FailWithoutAcdcClaim)?;
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
        root.set_saga_policy(SideEffectSagaPolicy::FailWithoutAcdcClaim)?;
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
        root.set_saga_policy(SideEffectSagaPolicy::FailWithoutAcdcClaim)?;
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

/// Plans a deploy-only EVM contract entry-point program.
pub fn plan_contract_deploy_entry_point(
    config: DeployPhaseConfig,
) -> Result<TypedProgramLaunchPlan, ContractLifecyclePlanError> {
    plan_draft(deploy_contract_program_draft(config)?, Vec::new())
}

/// Plans a configure-only EVM contract entry-point program with its launch seed.
pub fn plan_contract_configure_entry_point(
    config: ContractConfigureEntryPointConfig,
) -> Result<TypedProgramLaunchPlan, ContractLifecyclePlanError> {
    let seed = CanonicalSeed::from_value(&config.deployed)?;
    plan_draft(
        configure_contract_program_draft(config.config, config.deployed)?,
        vec![seed.canonical_json().clone()],
    )
}

/// Plans a validate-only EVM contract entry-point program with its launch seed.
pub fn plan_contract_validate_entry_point(
    config: ContractValidateEntryPointConfig,
) -> Result<TypedProgramLaunchPlan, ContractLifecyclePlanError> {
    let seed = CanonicalSeed::from_value(&config.configured)?;
    plan_draft(
        validate_contract_program_draft(config.config, config.configured)?,
        vec![seed.canonical_json().clone()],
    )
}

/// Plans a full lifecycle EVM contract entry-point program.
pub fn plan_contract_lifecycle_entry_point(
    config: ContractLifecycleConfig,
) -> Result<TypedProgramLaunchPlan, ContractLifecyclePlanError> {
    plan_draft(contract_lifecycle_program_draft(config)?, Vec::new())
}

/// Error returned while planning a contract lifecycle entry point.
#[derive(Debug, thiserror::Error)]
pub enum ContractLifecyclePlanError {
    /// Program planning failed.
    #[error("contract lifecycle planning failed: {0}")]
    Plan(#[from] mfm_program::PlanError),
}

fn contract_lifecycle_draft_config_material(
    draft: &mfm_program::TypedProgramDraft,
) -> mfm_program::Result<Vec<TypedProgramConfigMaterial>> {
    let media_type = MediaType::new("application/json")
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?;
    Ok(draft
        .state_nodes()
        .iter()
        .map(|node| &node.config)
        .chain(draft.operation_lineage().iter().map(|frame| &frame.config))
        .map(|config| TypedProgramConfigMaterial {
            schema_id: config.schema_id.clone(),
            bytes: config.canonical_json.clone(),
            media_type: media_type.clone(),
        })
        .collect())
}

fn plan_draft(
    draft: mfm_program::TypedProgramDraft,
    seed_bytes: Vec<PlainCanonicalJsonBytes>,
) -> Result<TypedProgramLaunchPlan, ContractLifecyclePlanError> {
    let config_material = contract_lifecycle_draft_config_material(&draft)?;
    let seed_material = seed_material_for_draft(&draft, seed_bytes)?;
    Ok(TypedProgramLaunchPlan {
        draft,
        config_material,
        seed_material,
    })
}

fn seed_material_for_draft(
    draft: &mfm_program::TypedProgramDraft,
    seed_bytes: Vec<PlainCanonicalJsonBytes>,
) -> mfm_program::Result<Vec<TypedProgramSeedMaterial>> {
    if draft.seeds().len() != seed_bytes.len() {
        return Err(mfm_program::PlanError::Key(format!(
            "entry-point seed material count mismatch: draft requires {}, supplied {}",
            draft.seeds().len(),
            seed_bytes.len()
        )));
    }
    let media_type = MediaType::new("application/json")
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?;
    draft
        .seeds()
        .iter()
        .zip(seed_bytes)
        .map(|(seed, bytes)| {
            let digest = bytes.content_digest();
            let byte_len = bytes.as_bytes().len() as u64;
            if digest != seed.content_digest || byte_len != seed.byte_len as u64 {
                return Err(mfm_program::PlanError::Canonical(format!(
                    "entry-point seed material did not match draft seed {}",
                    seed.seed_id
                )));
            }
            Ok(TypedProgramSeedMaterial {
                seed_id: seed.seed_id.clone(),
                bytes,
                media_type: media_type.clone(),
            })
        })
        .collect()
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

#[cfg(test)]
mod tests {
    use super::*;
    use mfm_certify::certify_program_draft;

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

    #[test]
    fn lifecycle_config_rejects_stale_version_field() {
        let value = serde_json::json!({
            "lifecycle_version": 1,
            "deploy": serde_json::to_value(deploy_config()).expect("deploy json"),
            "configure": serde_json::to_value(configure_config()).expect("configure json"),
            "validate": serde_json::to_value(validate_config()).expect("validate json"),
        });

        let error = serde_json::from_value::<ContractLifecycleConfig>(value)
            .expect_err("stale lifecycle version field");

        assert!(error.to_string().contains("unknown field"));
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
    fn entry_point_plan_helpers_are_draft_only_and_preserve_seeds() {
        let deploy = plan_contract_deploy_entry_point(deploy_config()).expect("deploy plan");
        let configure = plan_contract_configure_entry_point(ContractConfigureEntryPointConfig {
            config: configure_config(),
            deployed: deployed_contract(),
        })
        .expect("configure plan");
        let validate = plan_contract_validate_entry_point(ContractValidateEntryPointConfig {
            config: validate_config(),
            configured: configured_contract(),
        })
        .expect("validate plan");
        let lifecycle =
            plan_contract_lifecycle_entry_point(lifecycle_config()).expect("lifecycle plan");

        assert_eq!(deploy.draft.state_nodes().len(), 1);
        assert!(deploy.seed_material.is_empty());
        assert_eq!(configure.draft.state_nodes().len(), 1);
        assert_eq!(configure.seed_material.len(), 1);
        assert_eq!(
            configure.seed_material[0].seed_id,
            configure.draft.seeds()[0].seed_id
        );
        assert_eq!(validate.draft.state_nodes().len(), 1);
        assert_eq!(validate.seed_material.len(), 1);
        assert_eq!(
            validate.seed_material[0].seed_id,
            validate.draft.seeds()[0].seed_id
        );
        assert_eq!(lifecycle.draft.state_nodes().len(), 3);
        assert!(lifecycle.seed_material.is_empty());
        assert!(!lifecycle.config_material.is_empty());
    }

    #[test]
    fn entry_point_descriptors_are_public_launch_surface() {
        assert_eq!(
            CONTRACT_ENTRY_POINTS
                .iter()
                .map(|descriptor| descriptor.public_name)
                .collect::<Vec<_>>(),
            vec![
                "evm_contract_deploy",
                "evm_contract_configure",
                "evm_contract_validate",
                "evm_contract_lifecycle"
            ]
        );
        assert!(CONTRACT_ENTRY_POINTS
            .iter()
            .all(|descriptor| descriptor.namespace == OP_NAMESPACE
                && descriptor.version == 1
                && descriptor.accepted_config_formats == TOML_JSON_AUTHORED_CONFIG_FORMATS));
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
