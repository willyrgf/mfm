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

use mfm_authored_config::{EntryPointDescriptor, TOML_JSON_AUTHORED_CONFIG_FORMATS};
use mfm_evm_contract_model::{
    ConfiguredContractInstance, ContextBoundValidationReport, DeployedContractInstance,
    EvmContractContext,
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

/// Public deploy-only EVM contract entry-point descriptor retained for the pre-cutover app.
pub const CONTRACT_DEPLOY_ENTRY_POINT: EntryPointDescriptor = EntryPointDescriptor {
    namespace: OP_NAMESPACE,
    name: CONTEXT_DEPLOY_OP_KEY,
    public_name: "evm_contract_deploy",
    version: 1,
    accepted_config_formats: TOML_JSON_AUTHORED_CONFIG_FORMATS,
};

/// Public configure-only EVM contract entry-point descriptor retained for the pre-cutover app.
pub const CONTRACT_CONFIGURE_ENTRY_POINT: EntryPointDescriptor = EntryPointDescriptor {
    namespace: OP_NAMESPACE,
    name: CONTEXT_CONFIGURE_OP_KEY,
    public_name: "evm_contract_configure",
    version: 1,
    accepted_config_formats: TOML_JSON_AUTHORED_CONFIG_FORMATS,
};

/// Public validate-only EVM contract entry-point descriptor retained for the pre-cutover app.
pub const CONTRACT_VALIDATE_ENTRY_POINT: EntryPointDescriptor = EntryPointDescriptor {
    namespace: OP_NAMESPACE,
    name: CONTEXT_VALIDATE_OP_KEY,
    public_name: "evm_contract_validate",
    version: 1,
    accepted_config_formats: TOML_JSON_AUTHORED_CONFIG_FORMATS,
};

/// Public full-lifecycle EVM contract entry-point descriptor retained for the pre-cutover app.
pub const CONTRACT_LIFECYCLE_ENTRY_POINT: EntryPointDescriptor = EntryPointDescriptor {
    namespace: OP_NAMESPACE,
    name: CONTEXT_LIFECYCLE_OP_KEY,
    public_name: "evm_contract_lifecycle",
    version: 1,
    accepted_config_formats: TOML_JSON_AUTHORED_CONFIG_FORMATS,
};

/// Complete config for deploy-only EVM contract planning.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue, MfmConfig)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "deploy-entry-config",
    schema = "mfm.evm.contract.config.deploy_entry"
)]
pub struct EvmContractDeployEntryConfig {
    context: EvmContractContext,
    deploy: DeployAction,
}

impl EvmContractDeployEntryConfig {
    /// Creates a deploy config from its complete typed pieces.
    pub fn new(context: EvmContractContext, deploy: DeployAction) -> Self {
        Self { context, deploy }
    }

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
    schema = "mfm.evm.contract.config.configure_entry"
)]
pub struct EvmContractConfigureEntryConfig {
    context: EvmContractContext,
    import_deployed: ImportDeployedSpec,
    configure: ConfigureAction,
}

impl EvmContractConfigureEntryConfig {
    /// Creates a configure config from its complete typed pieces.
    pub fn new(
        context: EvmContractContext,
        import_deployed: ImportDeployedSpec,
        configure: ConfigureAction,
    ) -> Self {
        Self {
            context,
            import_deployed,
            configure,
        }
    }

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
    schema = "mfm.evm.contract.config.validate_entry"
)]
pub struct EvmContractValidateEntryConfig {
    context: EvmContractContext,
    import_configured: ImportConfiguredSpec,
    validate: ValidateAction,
}

impl EvmContractValidateEntryConfig {
    /// Creates a validate config from its complete typed pieces.
    pub fn new(
        context: EvmContractContext,
        import_configured: ImportConfiguredSpec,
        validate: ValidateAction,
    ) -> Self {
        Self {
            context,
            import_configured,
            validate,
        }
    }

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
    schema = "mfm.evm.contract.config.lifecycle_entry"
)]
pub struct EvmContractLifecycleEntryConfig {
    context: EvmContractContext,
    deploy: DeployAction,
    configure: ConfigureAction,
    validate: ValidateAction,
}

impl EvmContractLifecycleEntryConfig {
    /// Creates a full lifecycle config from its complete typed pieces.
    pub fn new(
        context: EvmContractContext,
        deploy: DeployAction,
        configure: ConfigureAction,
        validate: ValidateAction,
    ) -> Self {
        Self {
            context,
            deploy,
            configure,
            validate,
        }
    }

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

/// Plans a deploy-only EVM contract entry-point program for the pre-cutover app.
pub fn plan_contract_deploy_entry_point(
    config: EvmContractDeployEntryConfig,
) -> mfm_program::Result<TypedProgramLaunchPlan> {
    deploy_contract_program_launch_plan(config)
}

/// Plans a configure-only EVM contract entry-point program for the pre-cutover app.
pub fn plan_contract_configure_entry_point(
    config: EvmContractConfigureEntryConfig,
) -> mfm_program::Result<TypedProgramLaunchPlan> {
    configure_contract_program_launch_plan(config)
}

/// Plans a validate-only EVM contract entry-point program for the pre-cutover app.
pub fn plan_contract_validate_entry_point(
    config: EvmContractValidateEntryConfig,
) -> mfm_program::Result<TypedProgramLaunchPlan> {
    validate_contract_program_launch_plan(config)
}

/// Plans a full lifecycle EVM contract entry-point program for the pre-cutover app.
pub fn plan_contract_lifecycle_entry_point(
    config: EvmContractLifecycleEntryConfig,
) -> mfm_program::Result<TypedProgramLaunchPlan> {
    contract_lifecycle_program_launch_plan(config)
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
