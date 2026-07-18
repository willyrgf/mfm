use super::*;
use mfm_capabilities::{CapabilitySpec, ExternalMutationAuthorityRole, NoCaps};
use mfm_ids::{
    AdapterKind, AdapterVersion, CapabilityKind, CapabilityVersion, ContextRef,
    ContextResourceKind, ContextStage, OperationKind, OperationVersion,
};
use mfm_program::{
    build_root_with_registries, CanonicalSeed, MfmContext, MfmFactType as _, NoContext, Operation,
    OperationKey, OperationRegistryBuilder, PublicOutputKey, PureState, ResourceClaim, RootBuilder,
    ScopeKey, SideEffectState, StateKey, StateRegistryBuilder, StateResult, StateSpec,
};
use mfm_program_derive::{MfmConfig, MfmFactType, MfmValue, OperationOutput, PublicOutputs};
use serde::{Deserialize, Serialize};

#[path = "tests/lifecycle_support.rs"]
mod lifecycle_support;

#[path = "tests/behavior.rs"]
mod behavior;

fn certify_untrusted_typed_spec(
    typed: spec::TypedExecutionSpec,
    registry: &CertificationRegistry,
) -> Result<CertifiedTypedSpec> {
    certify_typed_spec(spec::UntrustedTypedSpec::from_raw_spec(typed), registry)
}

fn digest_byte(byte: u8) -> DigestBytes {
    DigestBytes::from_array([byte; 32])
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.certify.test",
    name = "value",
    version = "1",
    schema = "mfm.certify.test.value"
)]
struct TestValue {
    amount: u64,
}

fn context_resource_kind() -> ContextResourceKind {
    ContextResourceKind::new("mfm.certify.test.contract_instance").expect("resource kind")
}

fn context_stage() -> ContextStage {
    ContextStage::new("deployed").expect("context stage")
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.certify.test",
    name = "contract_context",
    version = "1",
    schema = "mfm.certify.test.contract_context"
)]
struct ContractContext {
    network: String,
}

impl MfmContext for ContractContext {}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.certify.test",
    name = "chain_head_subject",
    version = "1",
    schema = "mfm.certify.test.chain_head_subject"
)]
struct ChainHeadSubject {
    chain: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.certify.test",
    name = "chain_head_response",
    version = "1",
    schema = "mfm.certify.test.chain_head_response"
)]
struct ChainHeadResponse {
    height: u64,
}

#[allow(clippy::duplicated_attributes)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue, MfmFactType)]
#[mfm(
    namespace = "mfm.certify.test",
    name = "chain_head_fact",
    version = "1",
    schema = "mfm.certify.test.chain_head_fact"
)]
#[mfm_fact(kind = "chain.head")]
#[mfm_fact(field(
    id = "subject.chain",
    source = "subject",
    path = "chain",
    value_type = "string",
    exposure = "returnable"
))]
#[mfm_fact(field(
    id = "result.height",
    source = "result",
    path = "height",
    value_type = "unsigned_integer",
    operators(equal, greater_than_or_equal),
    exposure = "returnable",
    sortable
))]
#[mfm_fact(ordering(
    name = "result.height.desc",
    term(field = "result.height", direction = "descending", nulls = "last")
))]
struct ChainHeadFact {
    subject: ChainHeadSubject,
    response: ChainHeadResponse,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmConfig)]
struct TestConfig {
    multiplier: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmConfig)]
struct OperationConfig {
    multiplier: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmConfig)]
struct DefaultedConfig {
    #[serde(default)]
    multiplier: Option<u64>,
}

#[derive(PublicOutputs)]
#[mfm(schema = "mfm.certify.test.public_outputs")]
struct TestPublicOutputs<'p, 's> {
    result: mfm_program::Handle<'p, 's, TestValue>,
}

#[derive(OperationOutput)]
#[mfm(schema = "mfm.certify.test.operation_outputs")]
struct TestOperationOutputs<'p, 's> {
    result: mfm_program::Handle<'p, 's, TestValue>,
}

macro_rules! impl_test_state_spec {
    (
        $state:ty,
        effect = $effect:ty,
        caps = $caps:ty,
        kind = $kind:literal,
        version = $version:literal,
        name = $name:literal,
        digest = $digest:literal $(,)?
    ) => {
        impl StateSpec for $state {
            type Config = TestConfig;
            type Context = NoContext;
            type Input = TestValue;
            type Output = TestValue;
            type Effect = $effect;
            type Caps = $caps;

            fn kind() -> program::Result<StateKind> {
                StateKind::new(
                    "mfm.certify.test",
                    $kind,
                    DigestAlgorithm::Sha256JcsV1,
                    digest_byte($digest),
                )
                .map_err(|error| program::PlanError::Key(error.to_string()))
            }

            fn version() -> program::Result<StateVersion> {
                StateVersion::new($version)
                    .map_err(|error| program::PlanError::Key(error.to_string()))
            }

            fn name() -> &'static str {
                $name
            }

            fn new(config: program::ValidatedConfig<Self::Config>) -> program::Result<Self> {
                Ok(Self {
                    config: config.into_inner(),
                })
            }
        }
    };
}

macro_rules! impl_multiplying_pure_state {
    ($state:ty) => {
        impl PureState for $state {
            fn run(
                &self,
                input: Self::Input,
                _context: &program::CertifiedContext<Self::Context>,
            ) -> StateResult<Self::Output> {
                Ok(TestValue {
                    amount: input.amount * self.config.multiplier,
                })
            }
        }
    };
}

struct MultiplyState {
    config: TestConfig,
}

impl_test_state_spec!(
    MultiplyState,
    effect = Pure,
    caps = NoCaps,
    kind = "multiply",
    version = "mfm.certify.test.multiply.v1",
    name = "mfm.certify.test.multiply",
    digest = 0x11,
);

impl_multiplying_pure_state!(MultiplyState);

struct ConflictingMultiplyState {
    config: TestConfig,
}

impl_test_state_spec!(
    ConflictingMultiplyState,
    effect = Pure,
    caps = NoCaps,
    kind = "multiply",
    version = "mfm.certify.test.multiply.v1",
    name = "mfm.certify.test.conflicting_multiply",
    digest = 0x11,
);

impl_multiplying_pure_state!(ConflictingMultiplyState);

struct ContextSourceState {
    config: TestConfig,
}

impl StateSpec for ContextSourceState {
    type Config = TestConfig;
    type Context = ContractContext;
    type Input = TestValue;
    type Output = TestValue;
    type Effect = Pure;
    type Caps = NoCaps;

    fn kind() -> program::Result<StateKind> {
        StateKind::new(
            "mfm.certify.test",
            "context-source",
            DigestAlgorithm::Sha256JcsV1,
            digest_byte(0x13),
        )
        .map_err(|error| program::PlanError::Key(error.to_string()))
    }

    fn version() -> program::Result<StateVersion> {
        StateVersion::new("mfm.certify.test.context_source.v1")
            .map_err(|error| program::PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.certify.test.context_source"
    }

    fn output_context_contract() -> program::Result<spec::StateOutputContextContractSpec> {
        Ok(spec::StateOutputContextContractSpec::Produces {
            resource_kind: context_resource_kind(),
            stage: context_stage(),
        })
    }

    fn new(config: program::ValidatedConfig<Self::Config>) -> program::Result<Self> {
        Ok(Self {
            config: config.into_inner(),
        })
    }
}

impl_multiplying_pure_state!(ContextSourceState);

struct ContextConsumerState {
    config: TestConfig,
}

impl StateSpec for ContextConsumerState {
    type Config = TestConfig;
    type Context = ContractContext;
    type Input = TestValue;
    type Output = TestValue;
    type Effect = Pure;
    type Caps = NoCaps;

    fn kind() -> program::Result<StateKind> {
        StateKind::new(
            "mfm.certify.test",
            "context-consumer",
            DigestAlgorithm::Sha256JcsV1,
            digest_byte(0x14),
        )
        .map_err(|error| program::PlanError::Key(error.to_string()))
    }

    fn version() -> program::Result<StateVersion> {
        StateVersion::new("mfm.certify.test.context_consumer.v1")
            .map_err(|error| program::PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.certify.test.context_consumer"
    }

    fn input_context_contract() -> program::Result<spec::StateInputContextContractSpec> {
        Ok(spec::StateInputContextContractSpec::Required {
            resource_kind: context_resource_kind(),
            stage: context_stage(),
            producer: Box::new(spec::ContextProducerSpec {
                producer_descriptor_ids: vec![context_source_descriptor_id()?],
                seed_producers_allowed: false,
            }),
        })
    }

    fn new(config: program::ValidatedConfig<Self::Config>) -> program::Result<Self> {
        Ok(Self {
            config: config.into_inner(),
        })
    }
}

impl_multiplying_pure_state!(ContextConsumerState);

fn context_source_descriptor_id() -> program::Result<DescriptorId> {
    Ok(program::state_descriptor::<ContextSourceState>()?
        .descriptor_id()
        .clone())
}

struct FactEmittingState {
    config: TestConfig,
}

impl StateSpec for FactEmittingState {
    type Config = TestConfig;
    type Context = NoContext;
    type Input = TestValue;
    type Output = TestValue;
    type Effect = Pure;
    type Caps = NoCaps;

    fn kind() -> program::Result<StateKind> {
        StateKind::new(
            "mfm.certify.test",
            "fact-emitting",
            DigestAlgorithm::Sha256JcsV1,
            digest_byte(0xa2),
        )
        .map_err(|error| program::PlanError::Key(error.to_string()))
    }

    fn version() -> program::Result<StateVersion> {
        StateVersion::new("mfm.certify.test.fact_emitting.v1")
            .map_err(|error| program::PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.certify.test.fact_emitting"
    }

    fn emitted_fact_descriptors() -> program::Result<Vec<program::FactDescriptorRef>> {
        Ok(vec![mfm_program::fact_descriptor_ref::<ChainHeadFact>()?])
    }

    fn new(config: program::ValidatedConfig<Self::Config>) -> program::Result<Self> {
        Ok(Self {
            config: config.into_inner(),
        })
    }
}

impl_multiplying_pure_state!(FactEmittingState);

struct MutationCap;

impl CapabilitySpec for MutationCap {
    type Role = ExternalMutationAuthorityRole;

    fn kind() -> mfm_capabilities::Result<CapabilityKind> {
        CapabilityKind::new(
            "mfm.certify.test",
            "mutation",
            DigestAlgorithm::Sha256JcsV1,
            digest_byte(0x61),
        )
        .map_err(|error| mfm_capabilities::CapabilityError::Identity(error.to_string()))
    }

    fn version() -> mfm_capabilities::Result<CapabilityVersion> {
        CapabilityVersion::new("mfm.certify.test.mutation.v1")
            .map_err(|error| mfm_capabilities::CapabilityError::Identity(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.certify.test.mutation"
    }
}

struct MutatingState {
    config: TestConfig,
}

impl_test_state_spec!(
    MutatingState,
    effect = ApplySideEffect,
    caps = (MutationCap,),
    kind = "mutating",
    version = "mfm.certify.test.mutating.v1",
    name = "mfm.certify.test.mutating",
    digest = 0x62,
);

impl SideEffectState for MutatingState {
    type Intent = TestValue;
    type IdempotencyInput = TestValue;
    type PreparedInvocation = TestValue;
    type Submission = TestValue;
    type RecoveryEvidence = TestValue;
    type Receipt = TestValue;
    type Confirmation = TestValue;

    fn intent(
        &self,
        input: &Self::Input,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<mfm_program::SideEffectIntent<Self::Intent, Self::IdempotencyInput>> {
        let intent = TestValue {
            amount: input.amount * self.config.multiplier,
        };
        Ok(mfm_program::SideEffectIntent::new(intent.clone(), intent))
    }

    fn output_from_receipt(
        &self,
        _input: &Self::Input,
        _prepared: &Self::PreparedInvocation,
        _submission: &Self::Submission,
        receipt: &Self::Receipt,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output> {
        Ok(receipt.clone())
    }

    fn output_from_confirmation(
        &self,
        _input: &Self::Input,
        _prepared: &Self::PreparedInvocation,
        _submission: &Self::Submission,
        _receipt: &Self::Receipt,
        confirmation: &Self::Confirmation,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output> {
        Ok(confirmation.clone())
    }
}

struct MultiplyOperation;

impl Operation for MultiplyOperation {
    type Config = OperationConfig;
    type Input<'p, 's> = mfm_program::Handle<'p, 's, TestValue>;
    type Output<'p, 's> = TestOperationOutputs<'p, 's>;

    fn kind() -> program::Result<OperationKind> {
        OperationKind::new(
            "mfm.certify.test",
            "operation-multiply",
            DigestAlgorithm::Sha256JcsV1,
            digest_byte(0x12),
        )
        .map_err(|error| program::PlanError::Key(error.to_string()))
    }

    fn version() -> program::Result<OperationVersion> {
        OperationVersion::new("mfm.certify.test.operation_multiply.v1")
            .map_err(|error| program::PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.certify.test.operation_multiply"
    }

    fn expand<'p, 's>(
        &self,
        config: program::ValidatedConfig<Self::Config>,
        input: Self::Input<'p, 's>,
        builder: &mut mfm_program::OperationExpansion<'p, 's>,
        _dispatch: mfm_program::OperationExpansionDispatch<Self>,
    ) -> program::Result<Self::Output<'p, 's>> {
        let result = builder.state::<MultiplyState, _>(
            StateKey::new("multiply-state")?,
            NoContext,
            TestConfig {
                multiplier: config.as_ref().multiplier,
            },
            input,
        )?;
        Ok(TestOperationOutputs { result })
    }
}

struct ConflictingMultiplyOperation;

impl Operation for ConflictingMultiplyOperation {
    type Config = OperationConfig;
    type Input<'p, 's> = mfm_program::Handle<'p, 's, TestValue>;
    type Output<'p, 's> = TestOperationOutputs<'p, 's>;

    fn kind() -> program::Result<OperationKind> {
        OperationKind::new(
            "mfm.certify.test",
            "operation-multiply",
            DigestAlgorithm::Sha256JcsV1,
            digest_byte(0x12),
        )
        .map_err(|error| program::PlanError::Key(error.to_string()))
    }

    fn version() -> program::Result<OperationVersion> {
        OperationVersion::new("mfm.certify.test.operation_multiply.v1")
            .map_err(|error| program::PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.certify.test.conflicting_operation_multiply"
    }

    fn expand<'p, 's>(
        &self,
        config: program::ValidatedConfig<Self::Config>,
        input: Self::Input<'p, 's>,
        builder: &mut mfm_program::OperationExpansion<'p, 's>,
        _dispatch: mfm_program::OperationExpansionDispatch<Self>,
    ) -> program::Result<Self::Output<'p, 's>> {
        let result = builder.state::<MultiplyState, _>(
            StateKey::new("multiply-state")?,
            NoContext,
            TestConfig {
                multiplier: config.as_ref().multiplier,
            },
            input,
        )?;
        Ok(TestOperationOutputs { result })
    }
}

fn reference_draft() -> program::TypedProgramDraft {
    let mut states = StateRegistryBuilder::new();
    states
        .register::<MultiplyState>()
        .expect("state registration");
    let mut operations = OperationRegistryBuilder::new();
    operations
        .register::<MultiplyOperation>()
        .expect("operation registration");
    build_root_with_registries(
        ScopeKey::new("root").expect("root key"),
        states.snapshot(),
        operations.snapshot(),
        |root: &mut RootBuilder<'_, '_>| {
            let seed = root.seed(
                mfm_program::SeedKey::new("initial").expect("seed key"),
                CanonicalSeed::from_value(&TestValue { amount: 2 }).expect("seed"),
            )?;
            let result = root.scope().call::<MultiplyOperation, _>(
                OperationKey::new("multiply-operation")?,
                MultiplyOperation,
                OperationConfig { multiplier: 3 },
                seed,
            )?;
            root.bind_public_outputs(
                PublicOutputKey::new("terminal")?,
                &TestPublicOutputs {
                    result: result.result,
                },
            )
        },
    )
    .expect("reference draft")
}

fn context_bound_draft(declare_second_context: bool) -> program::TypedProgramDraft {
    let mut states = StateRegistryBuilder::new();
    states
        .register::<ContextSourceState>()
        .expect("source state registration");
    states
        .register::<ContextConsumerState>()
        .expect("consumer state registration");
    build_root_with_registries(
        ScopeKey::new("root").expect("root key"),
        states.snapshot(),
        OperationRegistryBuilder::new().snapshot(),
        |root: &mut RootBuilder<'_, '_>| {
            let seed = root.seed(
                mfm_program::SeedKey::new("initial").expect("seed key"),
                CanonicalSeed::from_value(&TestValue { amount: 2 }).expect("seed"),
            )?;
            let primary = root.scope().declare_context(ContractContext {
                network: "primary".to_owned(),
            })?;
            if declare_second_context {
                let _secondary = root.scope().declare_context(ContractContext {
                    network: "secondary".to_owned(),
                })?;
            }
            let produced = root.scope().state::<ContextSourceState, _>(
                StateKey::new("context-source")?,
                &primary,
                TestConfig { multiplier: 3 },
                seed,
            )?;
            let consumed = root.scope().state::<ContextConsumerState, _>(
                StateKey::new("context-consumer")?,
                &primary,
                TestConfig { multiplier: 5 },
                produced,
            )?;
            root.bind_public_outputs(
                PublicOutputKey::new("terminal")?,
                &TestPublicOutputs { result: consumed },
            )
        },
    )
    .expect("context-bound draft")
}

fn fact_emitting_draft() -> program::TypedProgramDraft {
    let mut states = StateRegistryBuilder::new();
    states
        .register::<FactEmittingState>()
        .expect("state registration");
    build_root_with_registries(
        ScopeKey::new("root").expect("root key"),
        states.snapshot(),
        OperationRegistryBuilder::new().snapshot(),
        |root: &mut RootBuilder<'_, '_>| {
            let seed = root.seed(
                mfm_program::SeedKey::new("initial").expect("seed key"),
                CanonicalSeed::from_value(&TestValue { amount: 2 }).expect("seed"),
            )?;
            let result = root.scope().state::<FactEmittingState, _>(
                StateKey::new("fact-emitter")?,
                NoContext,
                TestConfig { multiplier: 3 },
                seed,
            )?;
            root.bind_public_outputs(
                PublicOutputKey::new("terminal")?,
                &TestPublicOutputs { result },
            )
        },
    )
    .expect("fact emitting draft")
}

fn side_effect_draft() -> program::TypedProgramDraft {
    side_effect_draft_with_verification(program::SideEffectVerificationSpec::Receipt)
}

fn side_effect_draft_with_verification(
    verification: program::SideEffectVerificationSpec,
) -> program::TypedProgramDraft {
    let mut states = StateRegistryBuilder::new();
    states
        .register::<MutatingState>()
        .expect("state registration");
    build_root_with_registries(
        ScopeKey::new("root").expect("root key"),
        states.snapshot(),
        OperationRegistryBuilder::new().snapshot(),
        |root: &mut RootBuilder<'_, '_>| {
            root.set_saga_policy(program::SideEffectSagaPolicy::FailWithoutAcdcClaim)?;
            let seed = root.seed(
                mfm_program::SeedKey::new("initial").expect("seed key"),
                CanonicalSeed::from_value(&TestValue { amount: 2 }).expect("seed"),
            )?;
            let result = root.scope().side_effect::<MutatingState, _>(
                StateKey::new("mutating-state")?,
                NoContext,
                TestConfig { multiplier: 3 },
                seed,
                ResourceClaim::manual_only(),
                verification,
            )?;
            root.bind_public_outputs(
                PublicOutputKey::new("terminal")?,
                &TestPublicOutputs {
                    result: result.into_handle(),
                },
            )
        },
    )
    .expect("side effect draft")
}

fn compensating_draft() -> program::TypedProgramDraft {
    let mut states = StateRegistryBuilder::new();
    states
        .register::<MutatingState>()
        .expect("state registration");
    build_root_with_registries(
        ScopeKey::new("root").expect("root key"),
        states.snapshot(),
        OperationRegistryBuilder::new().snapshot(),
        |root: &mut RootBuilder<'_, '_>| {
            root.set_saga_policy(program::SideEffectSagaPolicy::CompensateCompleted {
                on_remediation_unresolved: program::RemediationUnresolved::FailWithoutAcdcClaim,
            })?;
            let seed = root.seed(
                mfm_program::SeedKey::new("initial").expect("seed key"),
                CanonicalSeed::from_value(&TestValue { amount: 2 }).expect("seed"),
            )?;
            let (forward, _remediation) = root
                .scope()
                .side_effect_with_compensation::<MutatingState, MutatingState, _, _, _>(
                    NoContext,
                    NoContext,
                    program::SideEffectNodeParams {
                        key: StateKey::new("mutating-state")?,
                        config: TestConfig { multiplier: 3 },
                        input: seed,
                        resource_claim: ResourceClaim::manual_only(),
                        verification: program::SideEffectVerificationSpec::Receipt,
                    },
                    program::RemediationNodeParams {
                        key: StateKey::new("compensating-state")?,
                        config: TestConfig { multiplier: 1 },
                        resource_claim: ResourceClaim::manual_only(),
                        verification: program::SideEffectVerificationSpec::Receipt,
                    },
                    Ok,
                )?;
            root.bind_public_outputs(
                PublicOutputKey::new("terminal")?,
                &TestPublicOutputs {
                    result: forward.into_handle(),
                },
            )
        },
    )
    .expect("compensating draft")
}

fn config_ref_for_bytes<C: mfm_values::MfmConfig>(
    bytes: &PlainCanonicalJsonBytes,
) -> spec::ConfigRef {
    let digest = bytes.content_digest();
    spec::ConfigRef {
        schema_id: C::schema_id().expect("config schema"),
        artifact_id: ArtifactId::from_digest(digest.algorithm(), *digest.digest()),
        digest,
        byte_len: bytes.as_bytes().len() as u64,
        media_type: spec::MediaType::new("application/json").expect("media type"),
    }
}

fn registered_multiply_certification_registry() -> CertificationRegistry {
    let mut registry = CertificationRegistry::new();
    registry
        .register_state::<MultiplyState>()
        .expect("certification state registration");
    registry
}
