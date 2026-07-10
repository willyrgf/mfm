use super::*;
use mfm_capabilities::{CapabilitySpec, ExternalMutationAuthorityRole, NoCaps};
use mfm_ids::{
    AdapterKind, AdapterVersion, CapabilityKind, CapabilityVersion, ContextRef,
    ContextResourceKind, ContextStage, OperationKind, OperationVersion,
};
use mfm_program::{
    build_root_with_registries, CanonicalSeed, IdempotencyKey, MfmContext, MfmFactType as _,
    NoContext, Operation, OperationKey, OperationRegistryBuilder, PublicOutputKey, PureState,
    ResourceClaim, RootBuilder, ScopeKey, SideEffectState, StateKey, StateRegistryBuilder,
    StateResult, StateSpec,
};
use mfm_program_derive::{MfmConfig, MfmFactType, MfmValue, OperationOutput, PublicOutputs};
use serde::{Deserialize, Serialize};

fn certify_untrusted_typed_spec(
    typed: spec::TypedExecutionSpec,
    registry: &CertificationRegistry,
) -> Result<CertifiedTypedSpec> {
    certify_typed_spec(spec::UntrustedTypedSpec::from_raw_spec(typed), registry)
}

macro_rules! assert_rejection_cases {
    ($fixture:ident, $($expected:expr => $mutate:expr),+ $(,)?) => {{
        let (registry, base) = $fixture();
        $(assert_rejects(&registry, &base, $expected, $mutate);)+
    }};
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

impl PureState for MultiplyState {
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

impl PureState for ConflictingMultiplyState {
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

impl PureState for ContextSourceState {
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

impl PureState for ContextConsumerState {
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

impl PureState for FactEmittingState {
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
    type Submission = TestValue;
    type Receipt = TestValue;
    type Confirmation = TestValue;
    type SubmitFuture<'a> = std::future::Ready<StateResult<Self::Submission>>;

    fn prepare_intent(
        &self,
        input: &Self::Input,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Intent> {
        Ok(TestValue {
            amount: input.amount * self.config.multiplier,
        })
    }

    fn idempotency_input(
        &self,
        _input: &Self::Input,
        intent: &Self::Intent,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::IdempotencyInput> {
        Ok(intent.clone())
    }

    fn submit<'a>(
        &'a self,
        intent: &'a Self::Intent,
        _key: &'a IdempotencyKey<Self::IdempotencyInput>,
        _caps: &'a Self::Caps,
        _context: &'a mfm_program::CertifiedContext<Self::Context>,
    ) -> Self::SubmitFuture<'a> {
        std::future::ready(Ok(intent.clone()))
    }

    fn output_from_receipt(
        &self,
        _input: &Self::Input,
        _intent: &Self::Intent,
        receipt: &Self::Receipt,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output> {
        Ok(receipt.clone())
    }

    fn output_from_confirmation(
        &self,
        _input: &Self::Input,
        _intent: &Self::Intent,
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

#[test]
fn certification_registry_rejects_duplicate_state_kind_version() {
    let mut registry = CertificationRegistry::new();
    registry
        .register_state::<MultiplyState>()
        .expect("first state descriptor registers");

    let error = registry
        .register_state::<ConflictingMultiplyState>()
        .expect_err("conflicting state kind/version rejects");

    assert_invalid_semantic_contains(error, "state kind/version");
}

#[test]
fn certification_registry_rejects_duplicate_operation_kind_version() {
    let mut registry = CertificationRegistry::new();
    registry
        .register_operation::<MultiplyOperation>()
        .expect("first operation descriptor registers");

    let error = registry
        .register_operation::<ConflictingMultiplyOperation>()
        .expect_err("conflicting operation kind/version rejects");

    assert_invalid_semantic_contains(error, "operation kind/version");
}

#[test]
fn registered_config_validator_rejects_invalid_config_bytes() {
    enum Case {
        SchemaShapeMismatch,
        NoncanonicalBytes,
        NonAuthoritativeCanonicalEncoding,
    }

    for case in [
        Case::SchemaShapeMismatch,
        Case::NoncanonicalBytes,
        Case::NonAuthoritativeCanonicalEncoding,
    ] {
        match case {
            Case::SchemaShapeMismatch => {
                let registry = registered_multiply_certification_registry();
                let invalid = PlainCanonicalJsonBytes::from_json_str(r#"{"multiplier":"bad"}"#)
                    .expect("canonical invalid config");
                let config_ref = config_ref_for_bytes::<TestConfig>(&invalid);

                let err = registry
                    .validate_config_ref_bytes(&config_ref, invalid.as_bytes())
                    .expect_err("invalid typed config shape rejects");
                assert!(err
                    .to_string()
                    .contains("typed config did not match registered schema"));
            }
            Case::NoncanonicalBytes => {
                let registry = registered_multiply_certification_registry();
                let canonical = PlainCanonicalJsonBytes::from_json_str(r#"{"multiplier":2}"#)
                    .expect("canonical config");
                let config_ref = config_ref_for_bytes::<TestConfig>(&canonical);

                let err = registry
                    .validate_config_ref_bytes(&config_ref, br#"{ "multiplier": 2 }"#)
                    .expect_err("noncanonical config rejects");
                let rendered = err.to_string();
                assert!(rendered.contains("typed config"));
                assert!(rendered.contains("not normalized canonical JSON"));
            }
            Case::NonAuthoritativeCanonicalEncoding => {
                let mut registry = CertificationRegistry::new();
                registry
                    .insert_config_validator(
                        config_validator_for::<DefaultedConfig>().expect("validator"),
                    )
                    .expect("insert validator");
                let supplied = PlainCanonicalJsonBytes::from_json_str(r#"{}"#)
                    .expect("canonical but not authoritative config");
                let config_ref = config_ref_for_bytes::<DefaultedConfig>(&supplied);

                let err = registry
                    .validate_config_ref_bytes(&config_ref, supplied.as_bytes())
                    .expect_err("non-authoritative canonical config rejects");
                assert!(err
                    .to_string()
                    .contains("did not match registered canonical encoding"));
            }
        }
    }
}

#[test]
fn trusted_draft_config_ref_accepts_exact_bytes_without_descriptor_validator() {
    let draft = reference_draft();
    let registry = CertificationRegistry::from_program_draft(&draft).expect("draft registry");
    let config = draft
        .state_nodes()
        .first()
        .expect("state node")
        .config
        .clone();
    let config_ref = spec::ConfigRef {
        schema_id: config.schema_id.clone(),
        artifact_id: ArtifactId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            *config.content_digest.digest(),
        ),
        digest: config.content_digest.clone(),
        byte_len: config.byte_len as u64,
        media_type: spec::MediaType::new("application/json").expect("media type"),
    };

    let source = registry
        .validate_config_ref_bytes(&config_ref, config.canonical_json.as_bytes())
        .expect("trusted exact config validates");
    assert_eq!(source, Some(ConfigValidationSource::TrustedExactRef));
}

#[test]
fn certifies_reference_program_draft() {
    let draft = reference_draft();
    let expected_public_schema = draft.public_output_spec().public_schema_id().clone();
    let certified = certify_program_draft(&draft).expect("certified");
    certified.envelope().verify_hash().expect("hash verifies");
    certified
        .certificate()
        .verify_hash()
        .expect("certificate hash verifies");
    assert_eq!(
        certified.certificate().evidence.certifier_algorithm,
        CERTIFIER_ALGORITHM
    );
    assert_eq!(
        certified.certificate().evidence.spec_hash,
        *certified.spec_hash()
    );
    assert_eq!(
        certified.certificate_hash().as_str(),
        "content:sha256-jcs-v1:9df4bb9724a7c4c452319e57e897cfee33c88120ce9a2f98a25a16f8f42c6b97"
    );
    assert_eq!(
        certified.envelope().spec.public_outputs.public_schema_id,
        expected_public_schema
    );
    assert!(certified.envelope().spec.nodes.iter().any(|node| matches!(
        node.framework,
        Some(spec::FrameworkNodeSpec::PublicOutputRender(_))
    )));
    assert!(certified.envelope().spec.nodes.iter().any(|node| matches!(
        node.framework,
        Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(_))
    )));
    assert!(certified.envelope().spec.nodes.iter().any(|node| matches!(
        node.framework,
        Some(spec::FrameworkNodeSpec::CompleteRun(_))
    )));
    assert!(certified.envelope().spec.nodes.iter().any(|node| matches!(
        node.framework,
        Some(spec::FrameworkNodeSpec::ResolveSagaTerminal(_))
    )));
}

#[test]
fn certifies_context_bound_transition_graph() {
    let draft = context_bound_draft(false);
    let certified = certify_program_draft(&draft).expect("context-bound draft certifies");
    let typed = certified.validated_spec().spec();
    assert_eq!(typed.contexts.len(), 1);
    let source = node_with_descriptor_name(typed, "mfm.certify.test.context_source");
    let source_output = typed
        .cells
        .iter()
        .find(|cell| cell.cell_id == source.output_cell)
        .expect("source output cell");
    assert!(matches!(
        &source_output.context,
        spec::CellContextSpec::Bound {
            resource_kind,
            stage,
            producer,
            ..
        } if resource_kind == &context_resource_kind()
            && stage == &context_stage()
            && producer.producer_descriptor_ids.as_slice()
                == std::slice::from_ref(&source.descriptor_id)
            && !producer.seed_producers_allowed
    ));
    let consumer = node_with_descriptor_name(typed, "mfm.certify.test.context_consumer");
    assert!(matches!(
        &consumer.input_bindings.root,
        spec::InputBindingNodeSpec::Cell(cell)
            if matches!(cell.context, spec::InputContextSpec::Required { .. })
    ));
}

#[test]
fn certification_rejects_context_table_entry_that_fails_registered_typed_decode() {
    let (registry, mut typed) = context_bound_registry_and_spec(false);
    let invalid_context =
        PlainCanonicalJsonBytes::from_json_str(r#"{"network":7}"#).expect("canonical context");
    let old_context_ref = typed.contexts[0].context_ref.clone();
    let new_context_ref = {
        let context = &mut typed.contexts[0];
        context.canonical_context = invalid_context;
        context.canonical_context_digest = context.canonical_context.content_digest();
        context.canonical_context_byte_len = context.canonical_context.as_bytes().len() as u64;
        context.context_ref = spec::CertifiedContextSpec::derive_context_ref(
            &context.context_descriptor_id,
            &context.schema_id,
            &context.semantic_type_id,
            &context.canonicalizer_identity,
            &context.canonical_context,
        )
        .expect("context ref");
        context.context_ref.clone()
    };
    retarget_context_ref(&mut typed, &old_context_ref, &new_context_ref);

    let error = certify_untrusted_typed_spec(typed, &registry)
        .expect_err("registered context descriptor must decode context payload");

    assert_problem_contains(
        error,
        ProblemClass::InvalidDataShape,
        "registered descriptor",
    );
}

#[test]
fn certification_rejects_context_bound_output_under_wrong_node_context() {
    let (registry, mut typed) = context_bound_registry_and_spec(true);
    let other_context_ref = typed.contexts[1].context_ref.clone();
    let source_output_cell = node_with_descriptor_name(&typed, "mfm.certify.test.context_source")
        .output_cell
        .clone();
    let output = typed
        .cells
        .iter_mut()
        .find(|cell| cell.cell_id == source_output_cell)
        .expect("source output cell");
    let spec::CellContextSpec::Bound { context_ref, .. } = &mut output.context else {
        panic!("source output must be context-bound");
    };
    *context_ref = other_context_ref;

    let error = certify_untrusted_typed_spec(typed, &registry)
        .expect_err("wrong output context must reject");
    assert_invalid_semantic_contains(error, "output context");
}

#[test]
fn certification_rejects_no_context_state_consuming_context_bound_resource() {
    let mut states = StateRegistryBuilder::new();
    states
        .register::<ContextSourceState>()
        .expect("source state registration");
    states
        .register::<MultiplyState>()
        .expect("multiply state registration");
    let draft = build_root_with_registries(
        ScopeKey::new("root").expect("root key"),
        states.snapshot(),
        OperationRegistryBuilder::new().snapshot(),
        |root: &mut RootBuilder<'_, '_>| {
            let seed = root.seed(
                mfm_program::SeedKey::new("initial").expect("seed key"),
                CanonicalSeed::from_value(&TestValue { amount: 2 }).expect("seed"),
            )?;
            let context = root.scope().declare_context(ContractContext {
                network: "primary".to_owned(),
            })?;
            let produced = root.scope().state::<ContextSourceState, _>(
                StateKey::new("context-source")?,
                &context,
                TestConfig { multiplier: 3 },
                seed,
            )?;
            let consumed = root.scope().state::<MultiplyState, _>(
                StateKey::new("plain-consumer")?,
                NoContext,
                TestConfig { multiplier: 5 },
                produced,
            )?;
            root.bind_public_outputs(
                PublicOutputKey::new("terminal")?,
                &TestPublicOutputs { result: consumed },
            )
        },
    )
    .expect("draft builds before certification");

    let error = certify_program_draft(&draft).expect_err("no-context consumer must reject");
    assert_invalid_semantic_contains(error, "without a descriptor contract");
}

#[test]
fn certification_rejects_input_context_that_does_not_match_source_cell() {
    let (registry, mut typed) = context_bound_registry_and_spec(false);
    let consumer = node_with_descriptor_name_mut(&mut typed, "mfm.certify.test.context_consumer");
    let spec::InputBindingNodeSpec::Cell(cell) = &mut consumer.input_bindings.root else {
        panic!("consumer input must be a cell");
    };
    cell.context = spec::InputContextSpec::no_context();
    consumer.input_bindings.digest =
        content_digest_json(input_node_json(&consumer.input_bindings.root)).expect("input digest");

    let error =
        certify_untrusted_typed_spec(typed, &registry).expect_err("input context must reject");
    assert_eq!(
        error.problem_class(),
        Some(ProblemClass::InvalidInterfaceWiring),
        "{error}"
    );
    assert!(error.to_string().contains("cell context"), "{error}");
}

#[test]
fn certification_rejects_unauthorized_context_bound_seed() {
    let (registry, mut typed) = context_bound_registry_and_spec(false);
    let context_ref = typed.contexts[0].context_ref.clone();
    let seed_cell_id = typed.seeds[0].cell_id.clone();
    let seed_cell = typed
        .cells
        .iter_mut()
        .find(|cell| cell.cell_id == seed_cell_id)
        .expect("seed cell");
    seed_cell.context = spec::CellContextSpec::Bound {
        context_ref,
        resource_kind: context_resource_kind(),
        stage: context_stage(),
        producer: Box::new(spec::ContextProducerSpec {
            producer_descriptor_ids: vec![context_source_descriptor_id().expect("descriptor id")],
            seed_producers_allowed: false,
        }),
    };

    let error =
        certify_untrusted_typed_spec(typed, &registry).expect_err("seed context must reject");
    assert_invalid_semantic_contains(error, "not authorized to produce context-bound cell");
}

#[test]
fn verifies_persisted_spec_certificate_parts() {
    let (registry, certified, persisted_parts) = reference_persisted_spec_certificate_parts();
    let verified = verify_persisted_spec_certificate(
        persisted_parts.spec_bytes(),
        persisted_parts.certificate_bytes(),
        &registry,
    )
    .expect("verified persisted spec/certificate");
    assert_eq!(verified.spec_hash(), certified.spec_hash());
    assert_eq!(verified.certificate_hash(), certified.certificate_hash());
}

#[test]
fn certificate_evidence_mismatches_reject_persisted_parts() {
    #[derive(Clone, Copy)]
    enum Mismatch {
        RegistryDigest,
        DescriptorIdentity,
        DescriptorDigest,
        SpecHash,
    }

    let (registry, certified, persisted_parts) = reference_persisted_spec_certificate_parts();
    for (case, expected) in [
        (Mismatch::RegistryDigest, "registry digest mismatch"),
        (Mismatch::DescriptorIdentity, "descriptor identity mismatch"),
        (Mismatch::DescriptorDigest, "descriptor digest mismatch"),
        (Mismatch::SpecHash, "spec hash mismatch"),
    ] {
        let mut evidence = certified.certificate().evidence.clone();
        match case {
            Mismatch::RegistryDigest => {
                evidence.registry_digest =
                    ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, digest_byte(0x72));
            }
            Mismatch::DescriptorIdentity => {
                evidence.descriptor_identities[0].descriptor_id =
                    DescriptorId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_byte(0x73));
            }
            Mismatch::DescriptorDigest => {
                evidence.descriptor_identities[0].descriptor_digest =
                    ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, digest_byte(0x74));
            }
            Mismatch::SpecHash => {
                evidence.spec_hash =
                    SpecHash::from_digest(DigestAlgorithm::Sha256JcsV1, digest_byte(0x75));
            }
        }
        let certificate = CertifiedSpecCertificate::from_evidence(evidence).expect("certificate");
        let error = verify_persisted_spec_certificate(
            persisted_parts.spec_bytes(),
            &certificate_bytes(&certificate),
            &registry,
        )
        .expect_err(expected);
        assert!(matches!(error, CertifyError::Certificate(_)), "{error}");
    }
}

#[test]
fn parsed_persisted_parts_are_untrusted_until_verifier_succeeds() {
    let (_registry, _certified, persisted_parts) = reference_persisted_spec_certificate_parts();
    let untrusted = persisted_parts
        .parse_untrusted()
        .expect("parsed untrusted persisted parts");
    assert_eq!(
        untrusted.spec().spec_hash().expect("untrusted spec hash"),
        untrusted.certificate().evidence.spec_hash
    );
    let error = verify_persisted_spec_certificate(
        persisted_parts.spec_bytes(),
        persisted_parts.certificate_bytes(),
        &CertificationRegistry::new(),
    )
    .expect_err("parsed persisted parts need registry-backed verifier success");
    assert!(matches!(error, CertifyError::Certificate(_)), "{error}");
}

#[test]
fn typed_certification_rejects_problem_taxonomy() {
    assert_rejection_cases!(
        reference_registry_and_spec,
        ProblemClass::InvalidTopology => |spec| {
            spec.cells.push(spec.cells[0].clone());
        },
        ProblemClass::InvalidInterfaceWiring => |spec| {
            let node = spec
                .nodes
                .iter_mut()
                .find(|node| {
                    matches!(
                        node.input_bindings.root,
                        spec::InputBindingNodeSpec::Cell(_)
                    )
                })
                .expect("cell-input node");
            let first_input = match &mut node.input_bindings.root {
                spec::InputBindingNodeSpec::Cell(cell) => cell,
                _ => panic!("expected cell input"),
            };
            first_input.schema_id = SchemaId::new(
                "mfm.certify.test.wrong",
                "1",
                DigestAlgorithm::Sha256JcsV1,
                digest_byte(0x44),
            )
            .expect("schema id");
            node.input_bindings.digest =
                content_digest_json(input_node_json(&node.input_bindings.root))
                    .expect("input digest");
        },
        ProblemClass::InvalidSemanticTransition => |spec| {
            spec.nodes[0].effect_kind = ApplySideEffect::descriptor()
                .expect("side effect descriptor")
                .kind;
        },
        ProblemClass::InvalidDataShape => |spec| {
            spec.config_refs.clear();
        },
        ProblemClass::InvalidDataMeaning => |spec| {
            let seed_lineage = spec
                .value_lineages
                .iter_mut()
                .find(|lineage| matches!(lineage.producer, spec::CellProducer::Seed(_)))
                .expect("seed lineage");
            seed_lineage.config_ref_digest = Some(ContentDigest::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                digest_byte(0x55),
            ));
        },
        ProblemClass::InvalidTerminalShape => |spec| {
            spec.public_outputs.outputs.clear();
        },
    );
}

#[test]
fn typed_spec_requires_registry_authority() {
    let spec = certify_program_draft(&reference_draft())
        .expect("certified")
        .validated_spec()
        .spec()
        .clone();
    let error =
        certify_untrusted_typed_spec(spec, &CertificationRegistry::new()).expect_err("must reject");
    assert_eq!(
        error.problem_class(),
        Some(ProblemClass::InvalidSemanticTransition)
    );
}

#[test]
fn certification_rejects_node_fact_descriptor_outside_registered_descriptor() {
    assert_reference_rejects(ProblemClass::InvalidSemanticTransition, |spec| {
        spec.nodes[0]
            .fact_descriptor_allowlist
            .push(spec::FactDescriptorRef {
                descriptor_hash: ContentDigest::from_digest(
                    DigestAlgorithm::Sha256JcsV1,
                    digest_byte(0xa1),
                ),
            });
    });
}

#[test]
fn certification_carries_derive_fact_descriptor_allowlist_authority() {
    let draft = fact_emitting_draft();
    let registry = CertificationRegistry::from_program_draft(&draft).expect("registry");
    let certified = certify_program_draft(&draft).expect("certified");
    let expected_ref = mfm_program::fact_descriptor_ref::<ChainHeadFact>().expect("descriptor ref");
    let derived_descriptor = ChainHeadFact::descriptor().expect("derived descriptor");
    assert_eq!(
        mfm_program::facts::fact_descriptor_hash(&derived_descriptor).expect("descriptor hash"),
        expected_ref.descriptor_hash
    );

    let spec = certified.validated_spec().spec();
    let fact_node = spec
        .nodes
        .iter()
        .find(|node| {
            node.fact_descriptor_allowlist.as_slice() == std::slice::from_ref(&expected_ref)
        })
        .expect("fact-emitting node");
    let descriptor_identity = spec
        .descriptor_identities
        .iter()
        .find_map(|identity| match identity {
            spec::DescriptorIdentity::State(state)
                if state.descriptor_id == fact_node.descriptor_id =>
            {
                Some(state.as_ref())
            }
            _ => None,
        })
        .expect("fact-emitting descriptor identity");
    assert_eq!(
        descriptor_identity.emitted_fact_descriptors.as_slice(),
        std::slice::from_ref(&expected_ref)
    );

    let baseline_hash = certified.spec_hash().clone();
    let mut mutated = spec.clone();
    let mutated_node = mutated
        .nodes
        .iter_mut()
        .find(|node| node.descriptor_id == fact_node.descriptor_id)
        .expect("mutated fact node");
    mutated_node.fact_descriptor_allowlist.clear();
    let mutated_descriptor = mutated
        .descriptor_identities
        .iter_mut()
        .find_map(|identity| match identity {
            spec::DescriptorIdentity::State(state)
                if state.descriptor_id == fact_node.descriptor_id =>
            {
                Some(state.as_mut())
            }
            _ => None,
        })
        .expect("mutated descriptor identity");
    mutated_descriptor.emitted_fact_descriptors.clear();

    assert_ne!(
        mutated.spec_hash().expect("mutated spec hash"),
        baseline_hash
    );
    let error = certify_untrusted_typed_spec(mutated, &registry)
        .expect_err("tampered allow-list authority rejects");
    assert_eq!(
        error.problem_class(),
        Some(ProblemClass::InvalidSemanticTransition)
    );
}

#[test]
fn certification_rejects_forged_framework_descriptor_bypass() {
    assert_reference_rejects(ProblemClass::InvalidSemanticTransition, |spec| {
        let mut forged = spec
            .descriptor_identities
            .iter()
            .find_map(|descriptor| match descriptor {
                spec::DescriptorIdentity::State(state) => Some(state.as_ref().clone()),
                _ => None,
            })
            .expect("state descriptor");
        forged.name = "mfm.framework.forged".to_owned();
        forged.state_kind = state_kind_json("forged", serde_json::json!({ "framework": "forged" }))
            .expect("state kind");
        forged.state_version =
            StateVersion::new("mfm.framework.state.forged.v1").expect("state version");
        forged.descriptor_id = state_descriptor_id_from_spec(&forged).expect("descriptor id");
        spec.descriptor_identities
            .push(spec::DescriptorIdentity::State(Box::new(forged)));
    });
}

#[test]
fn certification_accepts_valid_lifecycle_framework_node_shapes() {
    let (registry, spec) = reference_registry_and_spec();

    certify_untrusted_typed_spec(spec, &registry).expect("lifecycle framework nodes certify");
}

#[test]
fn certification_mints_descriptor_set_and_lifecycle_views() {
    let certified = certify_program_draft(&reference_draft()).expect("certified");
    let spec = certified.validated_spec().spec();
    let graph = certified.validated_spec().graph();
    let descriptors = certified.descriptor_set();
    let lifecycle = certified.framework_lifecycle();

    assert_eq!(graph.scope_count(), spec.scopes.len());
    assert_eq!(graph.config_ref_count(), spec.config_refs.len());
    assert_eq!(graph.value_lineage_count(), spec.value_lineages.len());
    assert_eq!(graph.cell_count(), spec.cells.len());
    assert_eq!(graph.forward_node_count(), spec.nodes.len());
    assert_eq!(graph.remediation_count(), spec.remediations.len());
    assert_eq!(graph.descriptors(), descriptors);
    assert!(descriptors.state_descriptors().count() > 0);
    assert!(descriptors
        .renderer(&spec.public_outputs.renderer_descriptor.descriptor_id)
        .is_some());
    assert!(descriptors
        .state(lifecycle.render().descriptor_id())
        .is_some());
    assert!(descriptors
        .state(lifecycle.retention().descriptor_id())
        .is_some());
    assert!(descriptors
        .state(lifecycle.complete().descriptor_id())
        .is_some());
    assert!(descriptors
        .state(lifecycle.resolve().descriptor_id())
        .is_some());
    assert!(graph.cell(lifecycle.render().output_cell()).is_some());
    assert!(graph.forward_node(lifecycle.render().node_id()).is_some());
    assert_ne!(
        lifecycle.render().node_id(),
        lifecycle.retention().node_id()
    );
    assert_ne!(
        lifecycle.retention().output_cell(),
        lifecycle.complete().output_cell()
    );
}

#[test]
fn certification_rejects_missing_retention_lifecycle_node() {
    assert_reference_rejects(ProblemClass::InvalidTopology, |spec| {
        spec.nodes.retain(|node| {
            !matches!(
                node.framework,
                Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(_))
            )
        });
    });
}

#[test]
fn certification_rejects_forged_lifecycle_framework_variant() {
    assert_reference_rejects(ProblemClass::InvalidSemanticTransition, |spec| {
        let forged = spec
            .nodes
            .iter()
            .find_map(|node| match &node.framework {
                Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(retention)) => {
                    Some(retention.clone())
                }
                _ => None,
            })
            .expect("retention metadata");
        let node = spec
            .nodes
            .iter_mut()
            .find(|node| node.framework.is_none())
            .expect("user node");
        node.framework = Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(forged));
    });
}

#[test]
fn certification_rejects_lifecycle_descriptor_mismatch() {
    assert_reference_rejects(ProblemClass::InvalidSemanticTransition, |spec| {
        let mut forged = spec
            .descriptor_identities
            .iter()
            .find_map(|descriptor| match descriptor {
                spec::DescriptorIdentity::State(state) => Some(state.as_ref().clone()),
                _ => None,
            })
            .expect("state descriptor");
        forged.name = "mfm.framework.complete_run".to_owned();
        forged.state_kind = state_kind_json(
            "complete_run",
            serde_json::json!({ "framework": "complete_run" }),
        )
        .expect("complete state kind");
        forged.state_version = StateVersion::new("mfm.framework.state.complete_run.v1")
            .expect("complete state version");
        forged.runner = "forged".to_owned();
        forged.descriptor_id = state_descriptor_id_from_spec(&forged).expect("descriptor id");
        spec.descriptor_identities
            .push(spec::DescriptorIdentity::State(Box::new(forged)));
    });
}

#[test]
fn certification_rejects_invalid_lifecycle_ordering() {
    assert_reference_rejects(ProblemClass::InvalidTopology, |spec| {
        let public_cell = spec
            .public_outputs
            .outputs
            .first()
            .expect("public output")
            .cell_id
            .clone();
        append_retention_lifecycle_node(spec, public_cell);
    });
}

#[test]
fn certification_rejects_lifecycle_framework_permissions() {
    assert_reference_rejects(ProblemClass::InvalidSemanticTransition, |spec| {
        let node = find_lifecycle_node_mut::<spec::ProjectRetentionManifestNodeSpec>(spec);
        node.adapter_bindings.push(spec::AdapterBinding {
            adapter_kind: AdapterKind::new(
                "mfm.certify.test",
                "forged-adapter",
                DigestAlgorithm::Sha256JcsV1,
                digest_byte(0xa1),
            )
            .expect("adapter kind"),
            adapter_version: AdapterVersion::new("mfm.certify.test.adapter.v1")
                .expect("adapter version"),
            binding_digest: None,
        });
    });
}

#[test]
fn certification_rejects_user_consumers_of_lifecycle_receipts() {
    assert_rejection_cases!(
        reference_registry_and_spec,
        ProblemClass::InvalidTopology => |spec| {
            let render_receipt = lifecycle_render_receipt_cell(spec);
            append_user_receipt_consumer(spec, render_receipt, "user/render-receipt");
        },
        ProblemClass::InvalidTopology => |spec| {
            let retention_receipt = lifecycle_retention_receipt_cell(spec);
            append_user_receipt_consumer(spec, retention_receipt, "user/retention-receipt");
        },
    );
}

#[test]
fn certification_rejects_executable_nodes_outside_lifecycle_tail() {
    assert_reference_rejects(ProblemClass::InvalidTopology, |spec| {
        append_independent_user_node(spec, "user/outside-lifecycle-tail");
    });
}

#[test]
fn certification_rejects_forged_lifecycle_config_ref() {
    assert_reference_rejects(ProblemClass::InvalidSemanticTransition, |spec| {
        let node = find_lifecycle_node_mut::<spec::ProjectRetentionManifestNodeSpec>(spec);
        node.config_ref.byte_len += 1;
    });
}

#[test]
fn certification_rejects_builtin_lifecycle_descriptors_without_framework_metadata() {
    assert_rejection_cases!(
        reference_registry_and_spec,
        ProblemClass::InvalidSemanticTransition => |spec| {
            clear_lifecycle_framework_metadata::<spec::ProjectRetentionManifestNodeSpec>(spec);
        },
        ProblemClass::InvalidSemanticTransition => |spec| {
            clear_lifecycle_framework_metadata::<spec::CompleteRunNodeSpec>(spec);
        },
        ProblemClass::InvalidSemanticTransition => |spec| {
            clear_lifecycle_framework_metadata::<spec::ResolveSagaTerminalNodeSpec>(spec);
        },
    );
}

#[test]
fn certification_rejects_forged_lifecycle_input_binding() {
    assert_rejection_cases!(
        reference_registry_and_spec,
        ProblemClass::InvalidInterfaceWiring => |spec| {
            let node = find_lifecycle_node_mut::<spec::ProjectRetentionManifestNodeSpec>(spec);
            let current = match &node.input_bindings.root {
                spec::InputBindingNodeSpec::Cell(cell) => cell.clone(),
                _ => panic!("expected cell input"),
            };
            node.input_bindings.root =
                spec::InputBindingNodeSpec::Struct(vec![spec::NamedInputBindingSpec {
                    field_path: spec::PublicFieldPath::new("public_output_receipt")
                        .expect("field path"),
                    node: spec::InputBindingNodeSpec::Cell(current),
                }]);
            node.input_bindings.digest =
                content_digest_json(input_node_json(&node.input_bindings.root))
                    .expect("input digest");
        },
        ProblemClass::InvalidInterfaceWiring => |spec| {
            let node = find_lifecycle_node_mut::<spec::CompleteRunNodeSpec>(spec);
            let current = match &node.input_bindings.root {
                spec::InputBindingNodeSpec::Cell(cell) => cell.clone(),
                _ => panic!("expected cell input"),
            };
            node.input_bindings.root =
                spec::InputBindingNodeSpec::Struct(vec![spec::NamedInputBindingSpec {
                    field_path: spec::PublicFieldPath::new("retention_manifest_receipt")
                        .expect("field path"),
                    node: spec::InputBindingNodeSpec::Cell(current),
                }]);
            node.input_bindings.digest =
                content_digest_json(input_node_json(&node.input_bindings.root))
                    .expect("input digest");
        },
        ProblemClass::InvalidInterfaceWiring => |spec| {
            let input_cell = spec.cells.first().expect("input cell").clone();
            let node = find_lifecycle_node_mut::<spec::ResolveSagaTerminalNodeSpec>(spec);
            node.input_bindings.root =
                spec::InputBindingNodeSpec::Struct(vec![spec::NamedInputBindingSpec {
                    field_path: spec::PublicFieldPath::new("retention_manifest_receipt")
                        .expect("field path"),
                    node: spec::InputBindingNodeSpec::Cell(Box::new(spec::InputBindingCellSpec {
                        field_path: spec::PublicFieldPath::new("retention_manifest_receipt")
                            .expect("field path"),
                        cell_id: input_cell.cell_id,
                        semantic_type_id: input_cell.semantic_type_id,
                        schema_id: input_cell.schema_id,
                        required_terminal: spec::RequiredTerminal::ProducedOnly,
                        value_lineage: input_cell.value_lineage,
                        context: spec::InputContextSpec::no_context(),
                    })),
                }]);
            node.input_bindings.digest =
                content_digest_json(input_node_json(&node.input_bindings.root))
                    .expect("input digest");
        },
    );
}

#[test]
fn certification_rejects_operation_input_binding_tampering() {
    assert_reference_rejects(ProblemClass::InvalidTopology, |spec| {
        let frame = spec
            .planning_lineage
            .first_mut()
            .expect("operation lineage frame");
        let input_cell = match &mut frame.input_bindings.root {
            spec::InputBindingNodeSpec::Cell(cell) => cell,
            _ => panic!("expected operation cell input"),
        };
        input_cell.cell_id = CellId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_byte(0x91));
        frame.input_bindings.digest =
            content_digest_json(input_node_json(&frame.input_bindings.root)).expect("input digest");
        frame.input_binding_digest = frame.input_bindings.digest.clone();
    });
}

#[test]
fn certification_rejects_stable_id_and_scope_tampering() {
    assert_rejection_cases!(
        reference_registry_and_spec,
        ProblemClass::InvalidTopology => |spec| {
            spec.nodes[0].stable_key =
                spec::StableAuthorKey::new("renamed-node").expect("node key");
        },
        ProblemClass::InvalidTopology => |spec| {
            spec.scopes[0].parent_scope_id = Some(spec.scopes[0].scope_id.clone());
        },
    );
}

#[test]
fn certification_rejects_framework_lineage_tampering() {
    assert_rejection_cases!(
        reference_registry_and_spec,
        ProblemClass::InvalidDataMeaning => |spec| {
            spec.planning_lineage.clear();
        },
        ProblemClass::InvalidDataMeaning => |spec| {
            let lineage = spec
                .nodes
                .first_mut()
                .expect("node with operation planning lineage");
            lineage.planning_lineage.active_operation_instances = vec![
                OperationInstanceId::from_digest(
                    DigestAlgorithm::Sha256JcsV1,
                    digest_byte(0x92),
                ),
            ];
            lineage.planning_lineage.lineage_digest = planning_lineage_digest(
                &lineage.planning_lineage.active_operation_instances,
                &lineage.planning_lineage.completed_operation_frames,
            )
            .expect("planning lineage digest");
        },
        ProblemClass::InvalidDataMeaning => |spec| {
            let render_node_id = spec
                .nodes
                .iter()
                .find(|node| {
                    matches!(
                        node.framework,
                        Some(spec::FrameworkNodeSpec::PublicOutputRender(_))
                    )
                })
                .expect("render node")
                .node_id
                .clone();
            let lineage = spec
                .value_lineages
                .iter_mut()
                .find(|lineage| {
                    lineage.producer == spec::CellProducer::Node(render_node_id.clone())
                })
                .expect("render lineage");
            lineage.input_cells.clear();
        },
    );
}

#[test]
fn certification_rejects_side_effect_contract_mismatch() {
    assert_side_effect_rejects(ProblemClass::InvalidSemanticTransition, |spec| {
        let node = spec
            .nodes
            .iter_mut()
            .find(|node| node.side_effect.is_some())
            .expect("side-effect node");
        node.side_effect = Some(spec::SideEffectContractSpec {
            contract_digest: ContentDigest::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                digest_byte(0x63),
            ),
            resource_claim: spec::ResourceClaimSpec::ManualOnly,
            verification: spec::SideEffectVerificationSpec::Receipt,
        });
    });
}

#[test]
fn certification_lowers_side_effect_submit_verify_pair() {
    let (_registry, typed) = side_effect_registry_and_spec();
    let submit = side_effect_submit_node(&typed);
    let (verify_node, verify) = side_effect_verify_node(&typed);
    let contract = submit.side_effect.as_ref().expect("submit side effect");
    let expected_pair =
        spec::side_effect_pair_id(&submit.node_id, &submit.output_cell, contract).expect("pair id");
    let pair = typed
        .side_effect_verify_pair_for_pair_id(&verify.pair_id)
        .expect("side-effect pair");

    assert_eq!(verify.submit_node_id, submit.node_id);
    assert_eq!(pair.submit_output_cell, &submit.output_cell);
    assert_eq!(verify.pair_id, expected_pair);
    assert_eq!(
        verify_node.deterministic_predecessors,
        vec![submit.node_id.clone()]
    );
    assert_ne!(verify_node.output_cell, submit.output_cell);
    assert!(typed
        .public_outputs
        .outputs
        .iter()
        .any(|output| output.cell_id == verify_node.output_cell));
    assert!(!typed
        .public_outputs
        .outputs
        .iter()
        .any(|output| output.cell_id == submit.output_cell));
}

#[test]
fn persisted_side_effect_verify_specs_reject_unknown_submit_output_cell_id() {
    let (_registry, typed) = side_effect_registry_and_spec();
    let submit_output_cell = side_effect_submit_node(&typed).output_cell.clone();
    let mut value: serde_json::Value =
        serde_json::from_str(typed.canonical_json().expect("canonical spec").as_str())
            .expect("typed spec JSON");
    let verify_node = value["nodes"]
        .as_array_mut()
        .expect("nodes")
        .iter_mut()
        .find(|node| node["framework"]["kind"] == "side_effect_verify")
        .expect("verify node");
    verify_node["framework"]["side_effect_verify"]
        .as_object_mut()
        .expect("verify object")
        .insert(
            "submit_output_cell_id".to_owned(),
            serde_json::json!(submit_output_cell.as_str()),
        );
    let input = serde_json::to_string(&value).expect("JSON");

    let error = spec::TypedExecutionSpec::from_json_str(&input)
        .expect_err("unknown submit output anchor must reject");

    assert!(error.to_string().contains("unknown or non-normalized"));
}

#[test]
fn certification_rejects_invalid_side_effect_verify_pair_shapes() {
    assert_rejection_cases!(
        side_effect_registry_and_spec,
        ProblemClass::InvalidTopology => |spec| {
            spec.nodes.retain(|node| {
                !matches!(
                    node.framework,
                    Some(spec::FrameworkNodeSpec::SideEffectVerify(_))
                )
            });
        },
        ProblemClass::InvalidTopology => |spec| {
            spec.nodes.retain(|node| node.side_effect.is_none());
        },
        ProblemClass::InvalidTopology => |spec| {
            let duplicate = side_effect_verify_node(spec).0.clone();
            spec.nodes.push(duplicate);
        },
        ProblemClass::InvalidDataMeaning => |spec| {
            let submit_node_id = side_effect_submit_node(spec).node_id.clone();
            let verify_output_cell = side_effect_verify_node(spec).0.output_cell.clone();
            let cell = spec
                .cells
                .iter_mut()
                .find(|cell| cell.cell_id == verify_output_cell)
                .expect("verify output cell");
            cell.producer = spec::CellProducer::Node(submit_node_id);
        },
        ProblemClass::InvalidTerminalShape => |spec| {
            let submit_output_cell = side_effect_submit_node(spec).output_cell.clone();
            let submit_cell = spec
                .cells
                .iter()
                .find(|cell| cell.cell_id == submit_output_cell)
                .expect("submit output cell")
                .clone();
            let public_output = spec
                .public_outputs
                .outputs
                .first_mut()
                .expect("public output");
            public_output.cell_id = submit_cell.cell_id;
            public_output.producer = submit_cell.producer;
            public_output.scope_id = submit_cell.scope_id;
            public_output.semantic_type_id = submit_cell.semantic_type_id;
            public_output.schema_id = submit_cell.schema_id;
            public_output.value_lineage = submit_cell.value_lineage;
        },
        ProblemClass::InvalidInterfaceWiring => |spec| {
            let submit_output_cell = side_effect_submit_node(spec).output_cell.clone();
            let wrong_cell = spec
                .cells
                .iter()
                .find(|cell| cell.cell_id != submit_output_cell)
                .expect("wrong cell")
                .clone();
            let wrong_predecessors =
                predecessors_for_test_inputs(spec, std::slice::from_ref(&wrong_cell.cell_id));
            let verify_node = spec
                .nodes
                .iter_mut()
                .find(|node| {
                    matches!(
                        node.framework,
                        Some(spec::FrameworkNodeSpec::SideEffectVerify(_))
                    )
                })
                .expect("verify node");
            verify_node.input_bindings = spec::framework_lifecycle_maybe_skipped_cell_input_binding(
                "side_effect_verify",
                "submit_output",
                &wrong_cell,
            )
            .expect("wrong verify input binding");
            verify_node.deterministic_predecessors = wrong_predecessors;
        },
        ProblemClass::InvalidDataMeaning => |spec| {
            let submit_output_cell = side_effect_submit_node(spec).output_cell.clone();
            let wrong_cell = spec
                .cells
                .iter()
                .find(|cell| cell.cell_id != submit_output_cell)
                .expect("wrong cell")
                .cell_id
                .clone();
            let verify_node = side_effect_verify_node(spec).0.clone();
            let lineage = spec
                .value_lineages
                .iter_mut()
                .find(|lineage| {
                    lineage.producer == spec::CellProducer::Node(verify_node.node_id.clone())
                })
                .expect("verify output lineage");
            lineage.input_cells = vec![wrong_cell];
            lineage.lineage_ref.lineage_digest =
                value_lineage_digest(lineage).expect("lineage digest");
            let lineage_ref = lineage.lineage_ref.clone();
            let output = spec
                .cells
                .iter_mut()
                .find(|cell| cell.cell_id == verify_node.output_cell)
                .expect("verify output cell");
            output.value_lineage = lineage_ref;
        },
        ProblemClass::InvalidSemanticTransition => |spec| {
            let submit_output_cell = side_effect_submit_node(spec).output_cell.clone();
            append_user_receipt_consumer(spec, submit_output_cell, "user/submit-output");
        },
    );
}

#[test]
fn side_effect_verification_policy_is_spec_authority_not_registry_authority() {
    let receipt = side_effect_draft_with_verification(program::SideEffectVerificationSpec::Receipt);
    let finalized_1 =
        side_effect_draft_with_verification(program::SideEffectVerificationSpec::Finalized {
            depth: 1,
        });
    let finalized_12 =
        side_effect_draft_with_verification(program::SideEffectVerificationSpec::Finalized {
            depth: 12,
        });

    let receipt_registry =
        CertificationRegistry::from_program_draft(&receipt).expect("receipt registry");
    let finalized_registry =
        CertificationRegistry::from_program_draft(&finalized_12).expect("finalized registry");
    assert_eq!(
        receipt_registry.digest().expect("receipt registry digest"),
        finalized_registry
            .digest()
            .expect("finalized registry digest"),
        "verification policy must not be resolved from registry authority"
    );

    let receipt_spec = certify_program_draft(&receipt)
        .expect("receipt certified")
        .validated_spec()
        .spec()
        .spec_hash()
        .expect("receipt hash");
    let finalized_1_spec = certify_program_draft(&finalized_1)
        .expect("finalized one certified")
        .validated_spec()
        .spec()
        .spec_hash()
        .expect("finalized one hash");
    let finalized_12_spec = certify_program_draft(&finalized_12)
        .expect("finalized twelve certified")
        .validated_spec()
        .spec()
        .spec_hash()
        .expect("finalized twelve hash");
    assert_ne!(receipt_spec, finalized_1_spec);
    assert_ne!(finalized_1_spec, finalized_12_spec);
}

#[test]
fn certifies_compensating_draft_with_separate_remediation_collection() {
    let draft = compensating_draft();
    let certified = certify_program_draft(&draft).expect("certified compensating draft");
    assert!(matches!(
        certified.validated_spec().spec().saga,
        spec::SagaPolicySpec::CompensateCompleted { .. }
    ));
    assert_eq!(certified.validated_spec().spec().remediations.len(), 1);
    let validated = certified.validated_spec().spec();
    let (forward_id, remediation) = validated.remediations.iter().next().expect("remediation");
    assert!(validated
        .nodes
        .iter()
        .any(|node| node.node_id == *forward_id));
    assert!(!validated
        .nodes
        .iter()
        .any(|node| node.node_id == remediation.node_id));
    let pair = validated
        .side_effect_verify_pair_for_submit_node(&remediation.node_id)
        .expect("remediation verify pair");
    assert_eq!(pair.submit_node.node_id, remediation.node_id);
    assert_eq!(pair.submit_output_cell, &remediation.output_cell);
}

#[test]
fn certified_side_effect_contract_validates_resource_evidence() {
    let (_, mut typed) = side_effect_spec_with_manual(manual_resolution_spec(0xc0));
    let node = typed
        .nodes
        .iter_mut()
        .find(|node| node.side_effect.is_some())
        .expect("side-effect node");
    let node_id = node.node_id.clone();
    let namespace = spec::ResourceNamespace::new("mfm.certify.test.wallet").expect("namespace");
    let key_schema = SchemaId::new(
        "mfm.certify.test.resource_key",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        digest_byte(0xc1),
    )
    .expect("key schema");
    node.side_effect
        .as_mut()
        .expect("side effect")
        .resource_claim = spec::ResourceClaimSpec::Exclusive {
        namespace: namespace.clone(),
        key_schema: key_schema.clone(),
    };
    let contract =
        CertifiedSideEffectContract::for_node(&typed, &node_id).expect("certified contract");
    let key = events::ResourceKeyEvidence {
        namespace: namespace.clone(),
        key_schema_id: key_schema,
        key: events::ResourceKey::new("wallet-1").expect("resource key"),
    };
    assert!(contract.validate_resource_key(Some(&key)).is_ok());
    assert!(matches!(
        contract
            .validate_resource_key(None)
            .expect_err("missing key")
            .problem_class(),
        Some(ProblemClass::InvalidSemanticTransition)
    ));
    let touched_set = events::ResourceTouchedSetEvidence {
        namespace,
        evidence_schema_id: SchemaId::new(
            "mfm.certify.test.touched_set",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            digest_byte(0xc2),
        )
        .expect("touched schema"),
        evidence_hash: ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, digest_byte(0xc3)),
        evidence_artifact_id: ArtifactId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            digest_byte(0xc3),
        ),
        evidence_artifact_evidence_hash: ContentDigest::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            digest_byte(0xc4),
        ),
    };
    assert!(contract.validate_touched_set(Some(&touched_set)).is_err());
}

#[test]
fn certified_side_effect_contract_validates_remediation_links() {
    let (_, typed) = compensating_registry_and_spec();
    let (forward_node_id, remediation) = typed.remediations.iter().next().expect("remediation");
    let contract = CertifiedSideEffectContract::for_node(&typed, &remediation.node_id)
        .expect("remediation contract");
    let run_id = RunId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_byte(0xc4));
    let purpose = events::SideEffectLedgerPurpose::Remediation {
        forward_pair_id: mfm_ids::SideEffectPairId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            digest_byte(0xc5),
        ),
    };
    let forward_purpose = events::SideEffectLedgerPurpose::Forward;

    assert!(contract
        .validate_remediation_link(CertifiedRemediationLink {
            remediation_run_id: &run_id,
            ledger_purpose: &purpose,
            forward_run_id: None,
            forward_node_id: None,
            forward_ledger_purpose: None,
            forward_terminal: false,
        })
        .is_err());
    assert!(contract
        .validate_remediation_link(CertifiedRemediationLink {
            remediation_run_id: &run_id,
            ledger_purpose: &purpose,
            forward_run_id: Some(&run_id),
            forward_node_id: Some(forward_node_id),
            forward_ledger_purpose: Some(&forward_purpose),
            forward_terminal: true,
        })
        .is_ok());
}

#[test]
fn certification_rejects_saga_policy_graph_disagreement() {
    let (registry, side_effecting) = side_effect_registry_and_spec();

    assert_rejects(
        &registry,
        &side_effecting,
        ProblemClass::InvalidSemanticTransition,
        |spec| {
            spec.saga = spec::SagaPolicySpec::NoSideEffects;
        },
    );

    let (registry, pure) = reference_registry_and_spec();
    assert_rejects(
        &registry,
        &pure,
        ProblemClass::InvalidSemanticTransition,
        |spec| {
            spec.saga = spec::SagaPolicySpec::FailWithoutAcdcClaim;
        },
    );
}

#[test]
fn certification_rejects_invalid_remediation_keys_and_coverage() {
    assert_rejection_cases!(
        compensating_registry_and_spec,
        ProblemClass::InvalidTopology => |spec| {
            let (_, mut remediation) = spec.remediations.pop_first().expect("remediation");
            remediation.node_id = node_id(0xb1);
            spec.remediations.insert(node_id(0xb0), remediation);
        },
        ProblemClass::InvalidTopology => |spec| {
            let remediation = spec
                .remediations
                .values()
                .next()
                .expect("remediation")
                .clone();
            let mut cloned = remediation;
            cloned.node_id = node_id(0xb2);
            let lifecycle_node = find_lifecycle_node_mut::<spec::PublicOutputRenderNodeSpec>(spec)
                .node_id
                .clone();
            spec.remediations.insert(lifecycle_node, cloned);
        },
        ProblemClass::InvalidTopology => |spec| {
            spec.remediations.clear();
        },
    );
}

#[test]
fn certification_rejects_invalid_remediation_node_shapes() {
    assert_rejection_cases!(
        compensating_registry_and_spec,
        ProblemClass::InvalidSemanticTransition => |spec| {
            spec.saga = spec::SagaPolicySpec::FailWithoutAcdcClaim;
        },
        ProblemClass::InvalidTopology => |spec| {
            let forward_id = spec
                .remediations
                .keys()
                .next()
                .expect("forward key")
                .clone();
            spec.remediations
                .get_mut(&forward_id)
                .expect("remediation")
                .node_id = forward_id.clone();
        },
        ProblemClass::InvalidSemanticTransition => |spec| {
            spec.remediations
                .values_mut()
                .next()
                .expect("remediation")
                .side_effect = None;
        },
        ProblemClass::InvalidSemanticTransition => |spec| {
            spec.remediations
                .values_mut()
                .next()
                .expect("remediation")
                .state_version =
                StateVersion::new("mfm.certify.test.forged.v1").expect("state version");
        },
    );
}

#[test]
fn certification_rejects_out_of_scope_remediation_bindings() {
    assert_compensating_rejects(ProblemClass::InvalidTopology, |spec| {
        let render_receipt = lifecycle_render_receipt_cell(spec);
        retarget_first_remediation_input(spec, render_receipt);
    });
}

#[test]
fn certification_rejects_manual_policy_without_schemas_at_decode_boundary() {
    let (_, typed) = side_effect_spec_with_manual(manual_resolution_spec(0xc1));
    let mut value: serde_json::Value =
        serde_json::from_str(typed.canonical_json().expect("canonical spec").as_str())
            .expect("spec JSON");
    value["saga"]["manual"]
        .as_object_mut()
        .expect("manual object")
        .remove("evidence_schema");
    let input = serde_json::to_string(&value).expect("JSON");
    let error = spec::TypedExecutionSpec::from_json_str(&input)
        .expect_err("missing manual evidence schema rejects");
    assert!(error.to_string().contains("evidence_schema"), "{error}");
}

#[test]
fn certification_records_manual_authority_evidence() {
    let manual = manual_resolution_spec(0xc1);
    let (mut registry, typed) = side_effect_spec_with_manual(manual.clone());
    register_manual_authority(&mut registry, &manual);

    let certified = certify_untrusted_typed_spec(typed, &registry).expect("certified manual spec");

    assert_eq!(
        certified
            .certificate()
            .evidence
            .schema_role_grants
            .as_slice(),
        &[CertifiedSchemaRoleGrantEvidence {
            schema_id: manual.evidence_schema.clone(),
            role: CertifiedSchemaRole::ManualResolutionEvidence,
        }]
    );
    assert_eq!(
        certified
            .certificate()
            .evidence
            .manual_authorization_verifiers
            .as_slice(),
        &[CertifiedManualAuthorizationVerifierEvidence {
            verifier_id: manual.authorization.verifier_id.clone(),
        }]
    );
    assert_eq!(
        certified
            .certificate()
            .evidence
            .operator_authority_snapshots
            .len(),
        1
    );
    assert_eq!(
        certified
            .certificate()
            .evidence
            .operator_authority_snapshots[0]
            .authority_id,
        manual.authorization.authority.authority_id
    );
}

#[test]
fn certification_rejects_invalid_manual_authority_cases() {
    #[derive(Clone, Copy)]
    enum Case {
        UnknownEvidenceSchema,
        WrongSchemaRole,
        UnknownVerifier,
        AuthoritySnapshotMismatch,
        EmptyAuthority,
        UnsupportedSigningScheme,
        QuorumExceeded,
    }

    for (case, byte, expected) in [
        (
            Case::UnknownEvidenceSchema,
            0xc2,
            "unknown manual evidence schema",
        ),
        (Case::WrongSchemaRole, 0xc3, "wrong certified role"),
        (
            Case::UnknownVerifier,
            0xc4,
            "unknown manual authorization verifier",
        ),
        (
            Case::AuthoritySnapshotMismatch,
            0xc5,
            "does not match registry",
        ),
        (Case::EmptyAuthority, 0xc6, "has no operators"),
        (
            Case::UnsupportedSigningScheme,
            0xc7,
            "unsupported manual authorization signing scheme",
        ),
        (Case::QuorumExceeded, 0xc8, "quorum 2 exceeds"),
    ] {
        let mut manual = manual_resolution_spec(byte);
        match case {
            Case::EmptyAuthority => manual.authorization.authority.operators.clear(),
            Case::UnsupportedSigningScheme => {
                manual.authorization.signing_scheme =
                    spec::ManualSigningSchemeSpec::new("mfm.certify.test.unsupported-signing.v1")
                        .expect("signing scheme");
            }
            Case::QuorumExceeded => {
                manual.authorization.quorum =
                    spec::ManualAuthorizationQuorumSpec::new(2).expect("quorum");
            }
            Case::UnknownEvidenceSchema
            | Case::WrongSchemaRole
            | Case::UnknownVerifier
            | Case::AuthoritySnapshotMismatch => {}
        }
        let (mut registry, typed) = side_effect_spec_with_manual(manual.clone());
        match case {
            Case::UnknownEvidenceSchema => {
                register_manual_verifier(&mut registry, &manual);
                register_manual_authority_snapshot(
                    &mut registry,
                    manual.authorization.authority.clone(),
                );
            }
            Case::WrongSchemaRole => {
                register_manual_schema_role(
                    &mut registry,
                    &manual,
                    CertifiedSchemaRole::ManualResolutionAuthorization,
                );
                register_manual_verifier(&mut registry, &manual);
                register_manual_authority_snapshot(
                    &mut registry,
                    manual.authorization.authority.clone(),
                );
            }
            Case::UnknownVerifier => {
                register_manual_evidence_schema(&mut registry, &manual);
                register_manual_authority_snapshot(
                    &mut registry,
                    manual.authorization.authority.clone(),
                );
            }
            Case::AuthoritySnapshotMismatch => {
                register_manual_evidence_schema(&mut registry, &manual);
                register_manual_verifier(&mut registry, &manual);
                let mut mismatched = manual.authorization.authority.clone();
                mismatched.operators[0].public_identity =
                    spec::OperatorPublicIdentity::new("operator-certify-public-mismatch")
                        .expect("operator public identity");
                register_manual_authority_snapshot(&mut registry, mismatched);
            }
            Case::EmptyAuthority | Case::UnsupportedSigningScheme | Case::QuorumExceeded => {
                register_manual_authority(&mut registry, &manual);
            }
        }

        assert_manual_certification_rejects(typed, &registry, expected);
    }
}

#[test]
fn certification_rejects_missing_or_duplicate_resolve_saga_terminal_node() {
    assert_rejection_cases!(
        reference_registry_and_spec,
        ProblemClass::InvalidTopology => |spec| {
            spec.nodes.retain(|node| {
                !matches!(
                    node.framework,
                    Some(spec::FrameworkNodeSpec::ResolveSagaTerminal(_))
                )
            });
        },
        ProblemClass::InvalidTopology => |spec| {
            let duplicate = spec
                .nodes
                .iter()
                .find(|node| {
                    matches!(
                        node.framework,
                        Some(spec::FrameworkNodeSpec::ResolveSagaTerminal(_))
                    )
                })
                .expect("resolve node")
                .clone();
            spec.nodes.push(duplicate);
        },
    );
}

fn assert_rejects(
    registry: &CertificationRegistry,
    base: &spec::TypedExecutionSpec,
    expected: ProblemClass,
    mutate: impl FnOnce(&mut spec::TypedExecutionSpec),
) {
    let mut mutated = base.clone();
    mutate(&mut mutated);
    let error = certify_untrusted_typed_spec(mutated, registry).expect_err("mutation must reject");
    assert_eq!(error.problem_class(), Some(expected), "{error}");
}

fn assert_problem_contains(error: CertifyError, expected: ProblemClass, needle: &str) {
    assert_eq!(error.problem_class(), Some(expected), "{error}");
    assert!(error.to_string().contains(needle), "{error}");
}

fn assert_invalid_semantic_contains(error: CertifyError, needle: &str) {
    assert_problem_contains(error, ProblemClass::InvalidSemanticTransition, needle);
}

fn assert_manual_certification_rejects(
    typed: spec::TypedExecutionSpec,
    registry: &CertificationRegistry,
    needle: &str,
) {
    let error = certify_untrusted_typed_spec(typed, registry).expect_err(needle);
    assert_invalid_semantic_contains(error, needle);
}

fn assert_reference_rejects(
    expected: ProblemClass,
    mutate: impl FnOnce(&mut spec::TypedExecutionSpec),
) {
    let (registry, base) = reference_registry_and_spec();
    assert_rejects(&registry, &base, expected, mutate);
}

fn assert_side_effect_rejects(
    expected: ProblemClass,
    mutate: impl FnOnce(&mut spec::TypedExecutionSpec),
) {
    let (registry, base) = side_effect_registry_and_spec();
    assert_rejects(&registry, &base, expected, mutate);
}

fn assert_compensating_rejects(
    expected: ProblemClass,
    mutate: impl FnOnce(&mut spec::TypedExecutionSpec),
) {
    let (registry, base) = compensating_registry_and_spec();
    assert_rejects(&registry, &base, expected, mutate);
}

fn reference_registry_and_spec() -> (CertificationRegistry, spec::TypedExecutionSpec) {
    let draft = reference_draft();
    let registry = CertificationRegistry::from_program_draft(&draft).expect("registry");
    let spec = certify_program_draft(&draft)
        .expect("certified")
        .validated_spec()
        .spec()
        .clone();
    (registry, spec)
}

fn context_bound_registry_and_spec(
    declare_second_context: bool,
) -> (CertificationRegistry, spec::TypedExecutionSpec) {
    let draft = context_bound_draft(declare_second_context);
    let registry = CertificationRegistry::from_program_draft(&draft).expect("registry");
    let spec = certify_program_draft(&draft)
        .expect("certified")
        .validated_spec()
        .spec()
        .clone();
    (registry, spec)
}

fn retarget_context_ref(typed: &mut spec::TypedExecutionSpec, old: &ContextRef, new: &ContextRef) {
    for node in &mut typed.nodes {
        if let spec::NodeContextSpec::Required { context_ref } = &mut node.context {
            if context_ref == old {
                *context_ref = new.clone();
            }
        }
        retarget_input_binding_context_ref(&mut node.input_bindings.root, old, new);
        node.input_bindings.digest =
            content_digest_json(input_node_json(&node.input_bindings.root)).expect("input digest");
    }
    for cell in &mut typed.cells {
        if let spec::CellContextSpec::Bound { context_ref, .. } = &mut cell.context {
            if context_ref == old {
                *context_ref = new.clone();
            }
        }
    }
}

fn retarget_input_binding_context_ref(
    node: &mut spec::InputBindingNodeSpec,
    old: &ContextRef,
    new: &ContextRef,
) {
    match node {
        spec::InputBindingNodeSpec::Unit => {}
        spec::InputBindingNodeSpec::Cell(cell) => {
            if let spec::InputContextSpec::Required { context_ref, .. } = &mut cell.context {
                if context_ref == old {
                    *context_ref = new.clone();
                }
            }
        }
        spec::InputBindingNodeSpec::Tuple(elements) => {
            for element in elements {
                retarget_input_binding_context_ref(element, old, new);
            }
        }
        spec::InputBindingNodeSpec::Struct(fields) => {
            for field in fields {
                retarget_input_binding_context_ref(&mut field.node, old, new);
            }
        }
        spec::InputBindingNodeSpec::Vec { elements, .. }
        | spec::InputBindingNodeSpec::NonEmptyVec { elements, .. } => {
            for element in elements {
                retarget_input_binding_context_ref(element, old, new);
            }
        }
    }
}

fn descriptor_id_by_name(typed: &spec::TypedExecutionSpec, name: &str) -> DescriptorId {
    typed
        .descriptor_identities
        .iter()
        .find_map(|descriptor| match descriptor {
            spec::DescriptorIdentity::State(state) if state.name == name => {
                Some(state.descriptor_id.clone())
            }
            _ => None,
        })
        .expect("state descriptor by name")
}

fn node_with_descriptor_name<'a>(
    typed: &'a spec::TypedExecutionSpec,
    name: &str,
) -> &'a spec::NodeSpec {
    let descriptor_id = descriptor_id_by_name(typed, name);
    typed
        .nodes
        .iter()
        .find(|node| node.descriptor_id == descriptor_id)
        .expect("node by descriptor name")
}

fn node_with_descriptor_name_mut<'a>(
    typed: &'a mut spec::TypedExecutionSpec,
    name: &str,
) -> &'a mut spec::NodeSpec {
    let descriptor_id = descriptor_id_by_name(typed, name);
    typed
        .nodes
        .iter_mut()
        .find(|node| node.descriptor_id == descriptor_id)
        .expect("node by descriptor name")
}

fn compensating_registry_and_spec() -> (CertificationRegistry, spec::TypedExecutionSpec) {
    let draft = compensating_draft();
    let registry = CertificationRegistry::from_program_draft(&draft).expect("registry");
    let spec = certify_program_draft(&draft)
        .expect("certified")
        .validated_spec()
        .spec()
        .clone();
    (registry, spec)
}

fn side_effect_registry_and_spec() -> (CertificationRegistry, spec::TypedExecutionSpec) {
    let draft = side_effect_draft();
    let registry = CertificationRegistry::from_program_draft(&draft).expect("registry");
    let spec = certify_program_draft(&draft)
        .expect("certified")
        .validated_spec()
        .spec()
        .clone();
    (registry, spec)
}

fn side_effect_submit_node(typed: &spec::TypedExecutionSpec) -> &spec::NodeSpec {
    typed
        .nodes
        .iter()
        .find(|node| node.side_effect.is_some() && node.framework.is_none())
        .expect("side-effect submit node")
}

fn side_effect_verify_node(
    typed: &spec::TypedExecutionSpec,
) -> (&spec::NodeSpec, &spec::SideEffectVerifyNodeSpec) {
    typed
        .nodes
        .iter()
        .find_map(|node| match &node.framework {
            Some(spec::FrameworkNodeSpec::SideEffectVerify(verify)) => Some((node, verify)),
            _ => None,
        })
        .expect("side-effect verify node")
}

fn side_effect_spec_with_manual(
    manual: spec::ManualResolutionEvidenceSpec,
) -> (CertificationRegistry, spec::TypedExecutionSpec) {
    let draft = side_effect_draft();
    let registry = CertificationRegistry::from_program_draft(&draft).expect("registry");
    let mut typed = certify_program_draft(&draft)
        .expect("certified")
        .validated_spec()
        .spec()
        .clone();
    typed.saga = spec::SagaPolicySpec::ManualResolution { manual };
    (registry, typed)
}

fn manual_resolution_spec(byte: u8) -> spec::ManualResolutionEvidenceSpec {
    let evidence_schema = SchemaId::new(
        "mfm.certify.test.manual_evidence",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        digest_byte(byte),
    )
    .expect("manual evidence schema");
    spec::ManualResolutionEvidenceSpec {
        evidence_schema,
        authorization: spec::ManualResolutionAuthorizationSpec {
            verifier_id: spec::ManualAuthorizationVerifierId::new(format!(
                "mfm.certify.test.manual.verifier.{byte:02x}"
            ))
            .expect("verifier id"),
            signing_scheme: spec::ManualSigningSchemeSpec::new(MANUAL_RESOLUTION_SIGNING_SCHEME)
                .expect("signing scheme"),
            authority: spec::OperatorAuthoritySnapshotSpec {
                authority_id: spec::OperatorAuthorityId::new(format!(
                    "mfm.certify.test.manual.authority.{byte:02x}"
                ))
                .expect("authority id"),
                operators: vec![spec::OperatorAuthorityMemberSpec {
                    operator_id: spec::OperatorId::new(format!("operator.certify.{byte:02x}"))
                        .expect("operator id"),
                    public_identity: spec::OperatorPublicIdentity::new(format!(
                        "operator-certify-public-{byte:02x}"
                    ))
                    .expect("operator public identity"),
                }],
            },
            quorum: spec::ManualAuthorizationQuorumSpec::new(1).expect("quorum"),
        },
    }
}

fn register_manual_authority(
    registry: &mut CertificationRegistry,
    manual: &spec::ManualResolutionEvidenceSpec,
) {
    register_manual_evidence_schema(registry, manual);
    register_manual_verifier(registry, manual);
    register_manual_authority_snapshot(registry, manual.authorization.authority.clone());
}

fn register_manual_evidence_schema(
    registry: &mut CertificationRegistry,
    manual: &spec::ManualResolutionEvidenceSpec,
) {
    register_manual_schema_role(
        registry,
        manual,
        CertifiedSchemaRole::ManualResolutionEvidence,
    );
}

fn register_manual_schema_role(
    registry: &mut CertificationRegistry,
    manual: &spec::ManualResolutionEvidenceSpec,
    role: CertifiedSchemaRole,
) {
    registry
        .register_schema_role(manual.evidence_schema.clone(), role)
        .expect("register schema role");
}

fn register_manual_verifier(
    registry: &mut CertificationRegistry,
    manual: &spec::ManualResolutionEvidenceSpec,
) {
    registry
        .register_manual_authorization_verifier(manual.authorization.verifier_id.clone())
        .expect("register verifier");
}

fn register_manual_authority_snapshot(
    registry: &mut CertificationRegistry,
    authority: spec::OperatorAuthoritySnapshotSpec,
) {
    registry
        .register_operator_authority_snapshot(authority)
        .expect("register authority");
}

fn node_id(byte: u8) -> NodeId {
    NodeId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_byte(byte))
}

fn retarget_first_remediation_input(typed: &mut spec::TypedExecutionSpec, cell_id: CellId) {
    let input_cell = typed
        .cells
        .iter()
        .find(|cell| cell.cell_id == cell_id)
        .expect("input cell")
        .clone();
    let root = spec::InputBindingNodeSpec::Cell(Box::new(spec::InputBindingCellSpec {
        field_path: spec::PublicFieldPath::new("input").expect("field path"),
        cell_id: cell_id.clone(),
        semantic_type_id: input_cell.semantic_type_id,
        schema_id: input_cell.schema_id,
        required_terminal: spec::RequiredTerminal::ProducedOnly,
        value_lineage: input_cell.value_lineage,
        context: spec::InputContextSpec::no_context(),
    }));
    let digest = content_digest_json(input_node_json(&root)).expect("input digest");
    let predecessors = predecessors_for_test_inputs(typed, std::slice::from_ref(&cell_id));
    let original_output_cell = typed
        .remediations
        .values()
        .next()
        .expect("remediation")
        .output_cell
        .clone();
    let original_output = typed
        .cells
        .iter()
        .find(|cell| cell.cell_id == original_output_cell)
        .expect("remediation output cell")
        .clone();
    let (
        previous_node_id,
        previous_output_cell,
        node_id,
        output_cell,
        scope_id,
        planning_lineage,
        config_digest,
    ) = {
        let remediation = typed.remediations.values_mut().next().expect("remediation");
        let previous_node_id = remediation.node_id.clone();
        remediation.input_bindings.root = root;
        remediation.input_bindings.digest = digest;
        remediation.deterministic_predecessors = predecessors;
        let config_digest = config_ref_digest(&remediation.config_ref).expect("config digest");
        remediation.node_id =
            state_node_id_from_spec(remediation, &config_digest).expect("remediation node id");
        remediation.output_cell = cell_id_from_parts(
            &remediation.scope_id,
            &spec::CellProducer::Node(remediation.node_id.clone()),
            &original_output.semantic_type_id,
            &original_output.schema_id,
        )
        .expect("remediation output cell");
        (
            previous_node_id,
            original_output_cell,
            remediation.node_id.clone(),
            remediation.output_cell.clone(),
            remediation.scope_id.clone(),
            remediation.planning_lineage.clone(),
            config_digest,
        )
    };
    let lineage = render_value_lineage_ref(
        &scope_id,
        &node_id,
        std::slice::from_ref(&cell_id),
        &planning_lineage,
        &config_digest,
    )
    .expect("lineage");
    typed
        .cells
        .iter_mut()
        .find(|cell| cell.cell_id == previous_output_cell)
        .expect("remediation output cell")
        .cell_id = output_cell.clone();
    let output = typed
        .cells
        .iter_mut()
        .find(|cell| cell.cell_id == output_cell)
        .expect("retargeted remediation output cell");
    output.producer = spec::CellProducer::Node(node_id.clone());
    output.value_lineage = lineage.clone();
    let value_lineage = typed
        .value_lineages
        .iter_mut()
        .find(|lineage| lineage.producer == spec::CellProducer::Node(previous_node_id.clone()))
        .expect("remediation value lineage");
    value_lineage.producer = spec::CellProducer::Node(node_id);
    value_lineage.lineage_ref = lineage;
    value_lineage.input_cells = vec![cell_id];
    value_lineage.config_ref_digest = Some(config_digest);
    value_lineage.planning_lineage = planning_lineage;
}

fn lifecycle_render_receipt_cell(typed: &spec::TypedExecutionSpec) -> CellId {
    typed
        .nodes
        .iter()
        .find(|node| {
            matches!(
                node.framework,
                Some(spec::FrameworkNodeSpec::PublicOutputRender(_))
            )
        })
        .expect("render node")
        .output_cell
        .clone()
}

fn lifecycle_retention_receipt_cell(typed: &spec::TypedExecutionSpec) -> CellId {
    typed
        .nodes
        .iter()
        .find(|node| {
            matches!(
                node.framework,
                Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(_))
            )
        })
        .expect("retention node")
        .output_cell
        .clone()
}

trait LifecycleVariant {
    fn matches(framework: &spec::FrameworkNodeSpec) -> bool;
}

impl LifecycleVariant for spec::ProjectRetentionManifestNodeSpec {
    fn matches(framework: &spec::FrameworkNodeSpec) -> bool {
        matches!(
            framework,
            spec::FrameworkNodeSpec::ProjectRetentionManifest(_)
        )
    }
}

impl LifecycleVariant for spec::CompleteRunNodeSpec {
    fn matches(framework: &spec::FrameworkNodeSpec) -> bool {
        matches!(framework, spec::FrameworkNodeSpec::CompleteRun(_))
    }
}

impl LifecycleVariant for spec::ResolveSagaTerminalNodeSpec {
    fn matches(framework: &spec::FrameworkNodeSpec) -> bool {
        matches!(framework, spec::FrameworkNodeSpec::ResolveSagaTerminal(_))
    }
}

impl LifecycleVariant for spec::PublicOutputRenderNodeSpec {
    fn matches(framework: &spec::FrameworkNodeSpec) -> bool {
        matches!(framework, spec::FrameworkNodeSpec::PublicOutputRender(_))
    }
}

fn find_lifecycle_node_mut<T: LifecycleVariant>(
    typed: &mut spec::TypedExecutionSpec,
) -> &mut spec::NodeSpec {
    typed
        .nodes
        .iter_mut()
        .find(|node| node.framework.as_ref().is_some_and(T::matches))
        .expect("lifecycle node")
}

fn clear_lifecycle_framework_metadata<T: LifecycleVariant>(typed: &mut spec::TypedExecutionSpec) {
    find_lifecycle_node_mut::<T>(typed).framework = None;
}

fn append_retention_lifecycle_node(
    typed: &mut spec::TypedExecutionSpec,
    public_output_receipt_cell: CellId,
) -> CellId {
    let scope_id = typed.scopes[0].scope_id.clone();
    let stable_key = spec::StableAuthorKey::new("framework/project-retention-manifest")
        .expect("retention stable key");
    let retention = spec::ProjectRetentionManifestNodeSpec {
        public_schema_id: typed.public_outputs.public_schema_id.clone(),
        public_output_receipt_cell: public_output_receipt_cell.clone(),
    };
    let node_id =
        project_retention_manifest_node_id_from_spec(&scope_id, stable_key.as_str(), &retention)
            .expect("retention node id");
    let receipt_schema =
        spec::retention_manifest_receipt_schema_id().expect("retention receipt schema");
    let receipt_semantic =
        spec::retention_manifest_receipt_semantic_type_id().expect("retention receipt semantic");
    let output_cell = framework_cell_id(&scope_id, &node_id, &receipt_semantic, &receipt_schema)
        .expect("retention output cell");
    let config_ref =
        framework_config_ref("project_retention_manifest", &node_id).expect("config ref");
    let config_digest = config_ref_digest(&config_ref).expect("config digest");
    let input_cell = typed
        .cells
        .iter()
        .find(|cell| cell.cell_id == public_output_receipt_cell)
        .expect("public-output receipt cell");
    let input_binding = spec::framework_lifecycle_receipt_input_binding(
        "project_retention_manifest",
        "public_output_receipt",
        input_cell,
    )
    .expect("input binding");
    let descriptor = framework_project_retention_manifest_descriptor(
        &receipt_schema,
        &receipt_semantic,
        &config_ref.schema_id,
        &input_binding.input_schema_id,
    )
    .expect("retention descriptor");
    let planning_lineage = typed.scopes[0].planning_lineage.clone();
    let lineage = render_value_lineage_ref(
        &scope_id,
        &node_id,
        std::slice::from_ref(&public_output_receipt_cell),
        &planning_lineage,
        &config_digest,
    )
    .expect("retention lineage");

    push_lifecycle_node(LifecycleNodeParts {
        typed,
        node_id,
        stable_key,
        scope_id,
        descriptor,
        config_ref,
        input_binding,
        output_cell: output_cell.clone(),
        receipt_schema,
        receipt_semantic,
        framework: spec::FrameworkNodeSpec::ProjectRetentionManifest(retention),
        planning_lineage,
        lineage,
        input_cells: vec![public_output_receipt_cell],
    });
    output_cell
}

struct LifecycleNodeParts<'a> {
    typed: &'a mut spec::TypedExecutionSpec,
    node_id: NodeId,
    stable_key: spec::StableAuthorKey,
    scope_id: ScopeId,
    descriptor: spec::StateDescriptorIdentity,
    config_ref: spec::ConfigRef,
    input_binding: spec::InputBindingSpec,
    output_cell: CellId,
    receipt_schema: SchemaId,
    receipt_semantic: SemanticTypeId,
    framework: spec::FrameworkNodeSpec,
    planning_lineage: spec::PlanningLineage,
    lineage: spec::ValueLineageRef,
    input_cells: Vec<CellId>,
}

fn push_lifecycle_node(parts: LifecycleNodeParts<'_>) {
    let LifecycleNodeParts {
        typed,
        node_id,
        stable_key,
        scope_id,
        descriptor,
        config_ref,
        input_binding,
        output_cell,
        receipt_schema,
        receipt_semantic,
        framework,
        planning_lineage,
        lineage,
        input_cells,
    } = parts;
    let predecessors = predecessors_for_test_inputs(typed, &input_cells);
    typed.config_refs.push(config_ref.clone());
    typed
        .descriptor_identities
        .push(spec::DescriptorIdentity::State(Box::new(
            descriptor.clone(),
        )));
    typed.cells.push(spec::CellSpec {
        cell_id: output_cell.clone(),
        producer: spec::CellProducer::Node(node_id.clone()),
        scope_id: scope_id.clone(),
        semantic_type_id: receipt_semantic,
        schema_id: receipt_schema,
        value_lineage: lineage.clone(),
        terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
        storage_policy: spec::StoragePolicy::ContentAddressed,
        redaction_policy: spec::RedactionPolicy::Public,
        context: spec::CellContextSpec::no_context(),
    });
    typed.value_lineages.push(spec::ValueLineage {
        lineage_ref: lineage,
        scope_id: scope_id.clone(),
        producer: spec::CellProducer::Node(node_id.clone()),
        input_cells,
        config_ref_digest: Some(config_ref_digest(&config_ref).expect("config digest")),
        planning_lineage: planning_lineage.clone(),
        domain_keys: Vec::new(),
        transform_policy: spec::LineageTransformPolicy::StateOutput,
    });
    typed.nodes.push(spec::NodeSpec {
        node_id,
        stable_key,
        scope_id,
        state_kind: descriptor.state_kind,
        state_version: descriptor.state_version,
        descriptor_id: descriptor.descriptor_id,
        context: spec::NodeContextSpec::no_context(),
        config_ref,
        input_bindings: input_binding,
        output_cell,
        effect_kind: descriptor.effect_kind,
        capability_bindings: descriptor.capabilities,
        adapter_bindings: Vec::new(),
        fact_descriptor_allowlist: Vec::new(),
        side_effect: None,
        framework: Some(framework),
        planning_lineage,
        deterministic_predecessors: predecessors,
    });
}

fn predecessors_for_test_inputs(
    typed: &spec::TypedExecutionSpec,
    input_cells: &[CellId],
) -> Vec<NodeId> {
    let mut predecessors = BTreeSet::new();
    for input_cell in input_cells {
        let cell = typed
            .cells
            .iter()
            .find(|cell| cell.cell_id == *input_cell)
            .expect("input cell");
        if let spec::CellProducer::Node(node_id) = &cell.producer {
            predecessors.insert(node_id.clone());
        }
    }
    predecessors.into_iter().collect()
}

fn append_user_receipt_consumer(
    typed: &mut spec::TypedExecutionSpec,
    receipt_cell: CellId,
    stable_key: &str,
) -> NodeId {
    let template = typed
        .nodes
        .iter()
        .find(|node| node.framework.is_none())
        .expect("user node template")
        .clone();
    let descriptor = typed
        .descriptor_identities
        .iter()
        .find_map(|descriptor| match descriptor {
            spec::DescriptorIdentity::State(state)
                if state.descriptor_id == template.descriptor_id =>
            {
                Some(state.as_ref().clone())
            }
            _ => None,
        })
        .expect("template descriptor");
    let input_cell = typed
        .cells
        .iter()
        .find(|cell| cell.cell_id == receipt_cell)
        .expect("receipt cell")
        .clone();
    let root = spec::InputBindingNodeSpec::Cell(Box::new(spec::InputBindingCellSpec {
        field_path: spec::PublicFieldPath::new("receipt").expect("field path"),
        cell_id: receipt_cell.clone(),
        semantic_type_id: input_cell.semantic_type_id.clone(),
        schema_id: input_cell.schema_id.clone(),
        required_terminal: spec::RequiredTerminal::ProducedOnly,
        value_lineage: input_cell.value_lineage.clone(),
        context: spec::InputContextSpec::no_context(),
    }));
    let input_binding = spec::InputBindingSpec {
        input_schema_id: descriptor.input_schema_id.clone(),
        input_descriptor_id: descriptor_id_json(serde_json::json!({
            "input": "framework_receipt_consumer",
            "receipt_cell": receipt_cell.as_str(),
            "stable_key": stable_key,
        }))
        .expect("input descriptor"),
        digest: content_digest_json(input_node_json(&root)).expect("input digest"),
        root,
    };
    let config_digest = config_ref_digest(&template.config_ref).expect("config digest");
    let mut node = spec::NodeSpec {
        node_id: NodeId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_byte(0xf0)),
        stable_key: spec::StableAuthorKey::new(stable_key).expect("stable key"),
        scope_id: template.scope_id.clone(),
        state_kind: descriptor.state_kind.clone(),
        state_version: descriptor.state_version.clone(),
        descriptor_id: descriptor.descriptor_id.clone(),
        context: spec::NodeContextSpec::no_context(),
        config_ref: template.config_ref.clone(),
        input_bindings: input_binding,
        output_cell: CellId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_byte(0xf1)),
        effect_kind: descriptor.effect_kind.clone(),
        capability_bindings: descriptor.capabilities.clone(),
        adapter_bindings: template.adapter_bindings.clone(),
        fact_descriptor_allowlist: Vec::new(),
        side_effect: None,
        framework: None,
        planning_lineage: template.planning_lineage.clone(),
        deterministic_predecessors: predecessors_for_test_inputs(
            typed,
            std::slice::from_ref(&receipt_cell),
        ),
    };
    node.node_id = state_node_id_from_spec(&node, &config_digest).expect("consumer node id");
    node.output_cell = cell_id_from_parts(
        &node.scope_id,
        &spec::CellProducer::Node(node.node_id.clone()),
        &descriptor.output_semantic_type_id,
        &descriptor.output_schema_id,
    )
    .expect("consumer output cell");
    let lineage = render_value_lineage_ref(
        &node.scope_id,
        &node.node_id,
        std::slice::from_ref(&receipt_cell),
        &node.planning_lineage,
        &config_digest,
    )
    .expect("consumer lineage");
    typed.cells.push(spec::CellSpec {
        cell_id: node.output_cell.clone(),
        producer: spec::CellProducer::Node(node.node_id.clone()),
        scope_id: node.scope_id.clone(),
        semantic_type_id: descriptor.output_semantic_type_id,
        schema_id: descriptor.output_schema_id,
        value_lineage: lineage.clone(),
        terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
        storage_policy: spec::StoragePolicy::ContentAddressed,
        redaction_policy: spec::RedactionPolicy::Public,
        context: spec::CellContextSpec::no_context(),
    });
    typed.value_lineages.push(spec::ValueLineage {
        lineage_ref: lineage,
        scope_id: node.scope_id.clone(),
        producer: spec::CellProducer::Node(node.node_id.clone()),
        input_cells: vec![receipt_cell],
        config_ref_digest: Some(config_digest),
        planning_lineage: node.planning_lineage.clone(),
        domain_keys: Vec::new(),
        transform_policy: spec::LineageTransformPolicy::StateOutput,
    });
    let node_id = node.node_id.clone();
    typed.nodes.push(node);
    node_id
}

fn append_independent_user_node(typed: &mut spec::TypedExecutionSpec, stable_key: &str) -> NodeId {
    let template = typed
        .nodes
        .iter()
        .find(|node| node.framework.is_none())
        .expect("user node template")
        .clone();
    let descriptor = typed
        .descriptor_identities
        .iter()
        .find_map(|descriptor| match descriptor {
            spec::DescriptorIdentity::State(state)
                if state.descriptor_id == template.descriptor_id =>
            {
                Some(state.as_ref().clone())
            }
            _ => None,
        })
        .expect("template descriptor");
    let input_cells = collect_input_cells(&template.input_bindings.root);
    let config_digest = config_ref_digest(&template.config_ref).expect("config digest");
    let mut node = spec::NodeSpec {
        node_id: NodeId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_byte(0xe0)),
        stable_key: spec::StableAuthorKey::new(stable_key).expect("stable key"),
        scope_id: template.scope_id.clone(),
        state_kind: descriptor.state_kind.clone(),
        state_version: descriptor.state_version.clone(),
        descriptor_id: descriptor.descriptor_id.clone(),
        context: spec::NodeContextSpec::no_context(),
        config_ref: template.config_ref.clone(),
        input_bindings: template.input_bindings.clone(),
        output_cell: CellId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_byte(0xe1)),
        effect_kind: descriptor.effect_kind.clone(),
        capability_bindings: descriptor.capabilities.clone(),
        adapter_bindings: template.adapter_bindings.clone(),
        fact_descriptor_allowlist: template.fact_descriptor_allowlist.clone(),
        side_effect: template.side_effect.clone(),
        framework: None,
        planning_lineage: template.planning_lineage.clone(),
        deterministic_predecessors: predecessors_for_test_inputs(typed, &input_cells),
    };
    node.node_id = state_node_id_from_spec(&node, &config_digest).expect("node id");
    node.output_cell = cell_id_from_parts(
        &node.scope_id,
        &spec::CellProducer::Node(node.node_id.clone()),
        &descriptor.output_semantic_type_id,
        &descriptor.output_schema_id,
    )
    .expect("output cell");
    let lineage = render_value_lineage_ref(
        &node.scope_id,
        &node.node_id,
        &input_cells,
        &node.planning_lineage,
        &config_digest,
    )
    .expect("lineage");
    typed.cells.push(spec::CellSpec {
        cell_id: node.output_cell.clone(),
        producer: spec::CellProducer::Node(node.node_id.clone()),
        scope_id: node.scope_id.clone(),
        semantic_type_id: descriptor.output_semantic_type_id,
        schema_id: descriptor.output_schema_id,
        value_lineage: lineage.clone(),
        terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
        storage_policy: spec::StoragePolicy::ContentAddressed,
        redaction_policy: spec::RedactionPolicy::Public,
        context: spec::CellContextSpec::no_context(),
    });
    typed.value_lineages.push(spec::ValueLineage {
        lineage_ref: lineage,
        scope_id: node.scope_id.clone(),
        producer: spec::CellProducer::Node(node.node_id.clone()),
        input_cells,
        config_ref_digest: Some(config_digest),
        planning_lineage: node.planning_lineage.clone(),
        domain_keys: Vec::new(),
        transform_policy: spec::LineageTransformPolicy::StateOutput,
    });
    let node_id = node.node_id.clone();
    typed.nodes.push(node);
    node_id
}

fn digest_byte(byte: u8) -> DigestBytes {
    DigestBytes::from_array([byte; 32])
}

fn reference_persisted_spec_certificate_parts() -> (
    CertificationRegistry,
    CertifiedTypedSpec,
    PersistedSpecCertificateParts,
) {
    let draft = reference_draft();
    let registry = CertificationRegistry::from_program_draft(&draft).expect("registry");
    let certified = certify_program_draft(&draft).expect("certified");
    assert_eq!(
        certified.certificate().evidence.registry_digest,
        registry.digest().expect("registry digest")
    );
    let persisted_parts = certified
        .to_persisted_parts()
        .expect("persisted spec/certificate parts");
    (registry, certified, persisted_parts)
}

fn certificate_bytes(certificate: &CertifiedSpecCertificate) -> Vec<u8> {
    certificate
        .canonical_json()
        .expect("certificate canonical json")
        .to_vec()
}

#[test]
fn certification_summary_keys_are_stable() {
    let keys = [
        ProblemClass::InvalidTopology.summary_key(),
        ProblemClass::InvalidInterfaceWiring.summary_key(),
        ProblemClass::InvalidSemanticTransition.summary_key(),
        ProblemClass::InvalidDataShape.summary_key(),
        ProblemClass::InvalidDataMeaning.summary_key(),
        ProblemClass::InvalidTerminalShape.summary_key(),
    ];
    assert_eq!(
        keys,
        [
            "invalid_topology_rejected",
            "invalid_interface_wiring_rejected",
            "invalid_semantic_transition_rejected",
            "invalid_data_shape_rejected",
            "invalid_data_meaning_rejected",
            "invalid_terminal_shape_rejected"
        ]
    );
}
