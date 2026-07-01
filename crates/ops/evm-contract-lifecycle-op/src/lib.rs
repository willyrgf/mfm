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

use std::collections::BTreeMap;

use mfm_authored_config::{EntryPointDescriptor, TOML_JSON_AUTHORED_CONFIG_FORMATS};
use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_evm_contract_config::{
    ConfigurePhaseConfig, DeployPhaseConfig, EvmNetworkIntent, ValidatePhaseConfig,
};
use mfm_evm_contract_model::{ConfiguredContract, DeployedContract, ValidationReport};
use mfm_ids::{DigestAlgorithm, OperationKind, OperationVersion, SeedId};
use mfm_program::{
    build_root_with_registries, CanonicalSeed, Handle, Operation, OperationExpansion, OperationKey,
    PublicOutputKey, RootBound, RootBuilder, ScopeKey, SeedKey, SideEffectSagaPolicy,
    SideEffectVerificationSpec, StateKey, TypedProgramLaunchPlan,
};
use mfm_program_derive::{MfmConfig, OperationOutput, PublicOutputs};
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
                SideEffectVerificationSpec::Receipt,
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
                SideEffectVerificationSpec::Receipt,
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
                SideEffectVerificationSpec::Receipt,
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
                SideEffectVerificationSpec::Receipt,
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

mfm_certify::define_program_descriptor_registry! {
    state_registry: pub contract_lifecycle_state_registry,
    operation_registry: pub contract_lifecycle_operation_registry,
    certification: pub register_contract_lifecycle_certification_descriptors,
    states: [DeployContractState, ConfigureContractState, ValidateContractState],
    operations: [
        DeployContractOperation,
        ConfigureContractOperation,
        ValidateContractOperation,
        ContractLifecycleOperation,
    ],
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
    ensure_networks_match(
        config.network(),
        &deployed.network_id,
        deployed.expected_chain_id,
        "seed",
    )?;
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
    ensure_networks_match(
        config.network(),
        &configured.deployed.network_id,
        configured.deployed.expected_chain_id,
        "seed",
    )?;
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
    Ok(TypedProgramLaunchPlan::from_draft(
        deploy_contract_program_draft(config)?,
    )?)
}

/// Plans a configure-only EVM contract entry-point program with its launch seed.
pub fn plan_contract_configure_entry_point(
    config: ContractConfigureEntryPointConfig,
) -> Result<TypedProgramLaunchPlan, ContractLifecyclePlanError> {
    let seed = CanonicalSeed::from_value(&config.deployed)?;
    let draft = configure_contract_program_draft(config.config, config.deployed)?;
    let seeds = seed_bytes_by_key(&draft, DEPLOY_SEED_KEY, seed.canonical_json().clone())?;
    Ok(TypedProgramLaunchPlan::from_draft_and_seed_material(
        draft, seeds,
    )?)
}

/// Plans a validate-only EVM contract entry-point program with its launch seed.
pub fn plan_contract_validate_entry_point(
    config: ContractValidateEntryPointConfig,
) -> Result<TypedProgramLaunchPlan, ContractLifecyclePlanError> {
    let seed = CanonicalSeed::from_value(&config.configured)?;
    let draft = validate_contract_program_draft(config.config, config.configured)?;
    let seeds = seed_bytes_by_key(&draft, CONFIGURED_SEED_KEY, seed.canonical_json().clone())?;
    Ok(TypedProgramLaunchPlan::from_draft_and_seed_material(
        draft, seeds,
    )?)
}

/// Plans a full lifecycle EVM contract entry-point program.
pub fn plan_contract_lifecycle_entry_point(
    config: ContractLifecycleConfig,
) -> Result<TypedProgramLaunchPlan, ContractLifecyclePlanError> {
    Ok(TypedProgramLaunchPlan::from_draft(
        contract_lifecycle_program_draft(config)?,
    )?)
}

/// Error returned while planning a contract lifecycle entry point.
#[derive(Debug, thiserror::Error)]
pub enum ContractLifecyclePlanError {
    /// Program planning failed.
    #[error("contract lifecycle planning failed: {0}")]
    Plan(#[from] mfm_program::PlanError),
}

fn seed_bytes_by_key(
    draft: &mfm_program::TypedProgramDraft,
    seed_key: &'static str,
    bytes: PlainCanonicalJsonBytes,
) -> mfm_program::Result<BTreeMap<SeedId, PlainCanonicalJsonBytes>> {
    let seed = draft
        .seeds()
        .iter()
        .find(|seed| seed.key.as_str() == seed_key)
        .ok_or_else(|| {
            mfm_program::PlanError::Key(format!(
                "entry-point seed key `{seed_key}` was not present in the draft"
            ))
        })?;
    Ok(BTreeMap::from([(seed.seed_id.clone(), bytes)]))
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
    ensure_networks_match(
        deploy.network(),
        configure.network().network_id(),
        configure.network().expected_chain_id(),
        "deploy/configure",
    )?;
    ensure_networks_match(
        deploy.network(),
        validate.network().network_id(),
        validate.network().expected_chain_id(),
        "deploy/validate",
    )
}

fn ensure_networks_match(
    left: &EvmNetworkIntent,
    network_id: &str,
    expected_chain_id: u64,
    label: &'static str,
) -> mfm_program::Result<()> {
    if left.network_id() != network_id {
        return Err(network_mismatch_error(label, "network id"));
    }
    if left.expected_chain_id() != expected_chain_id {
        return Err(network_mismatch_error(label, "expected chain id"));
    }
    Ok(())
}

fn network_mismatch_error(label: &'static str, field: &'static str) -> mfm_program::PlanError {
    mfm_program::PlanError::Key(format!("contract lifecycle {label} {field} mismatch"))
}
