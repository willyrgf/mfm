use super::side_effect_helpers::{side_effect_capability_kind, side_effect_capability_version};
use super::*;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.runtime.test",
    name = "value",
    version = "1",
    schema = "mfm.runtime.test.value"
)]
pub(super) struct CertifierValue {
    pub(super) amount: u64,
}

pub(super) fn runtime_context_resource_kind() -> ContextResourceKind {
    ContextResourceKind::new("mfm.runtime.test.contract_instance").expect("context resource kind")
}

pub(super) fn runtime_context_stage() -> ContextStage {
    ContextStage::new("deployed").expect("context stage")
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.runtime.test",
    name = "contract_context",
    version = "1",
    schema = "mfm.runtime.test.contract_context"
)]
pub(super) struct RuntimeContractContext {
    pub(super) network: String,
}

impl MfmContext for RuntimeContractContext {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RuntimeContextOutput {
    pub(super) amount: u64,
    pub(super) context_ref: ContextRef,
    pub(super) resource_kind: ContextResourceKind,
    pub(super) stage: ContextStage,
}

impl Serialize for RuntimeContextOutput {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut state = serializer.serialize_struct("RuntimeContextOutput", 4)?;
        state.serialize_field("amount", &self.amount)?;
        state.serialize_field("context_ref", self.context_ref.as_str())?;
        state.serialize_field("resource_kind", &self.resource_kind)?;
        state.serialize_field("stage", &self.stage)?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for RuntimeContextOutput {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct RawRuntimeContextOutput {
            amount: u64,
            context_ref: String,
            resource_kind: ContextResourceKind,
            stage: ContextStage,
        }

        let raw = RawRuntimeContextOutput::deserialize(deserializer)?;
        Ok(Self {
            amount: raw.amount,
            context_ref: ContextRef::parse(raw.context_ref).map_err(serde::de::Error::custom)?,
            resource_kind: raw.resource_kind,
            stage: raw.stage,
        })
    }
}

impl mfm_values::MfmValue for RuntimeContextOutput {
    fn schema_descriptor() -> mfm_values::Result<mfm_values::SchemaDescriptor> {
        <CertifierValue as mfm_values::MfmValue>::schema_descriptor()
    }

    fn schema_id() -> mfm_values::Result<SchemaId> {
        <CertifierValue as mfm_values::MfmValue>::schema_id()
    }

    fn semantic_id() -> mfm_values::Result<SemanticTypeId> {
        <CertifierValue as mfm_values::MfmValue>::semantic_id()
    }
}

impl ContextBoundOutput for RuntimeContextOutput {
    fn context_ref(&self) -> &ContextRef {
        &self.context_ref
    }

    fn context_resource_kind(&self) -> &ContextResourceKind {
        &self.resource_kind
    }

    fn context_stage(&self) -> &ContextStage {
        &self.stage
    }
}

#[allow(clippy::duplicated_attributes)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue, MfmFactType)]
#[mfm(
    namespace = "mfm.runtime.test",
    name = "fact",
    version = "1",
    schema = "mfm.runtime.test.fact"
)]
#[mfm_fact(kind = "mfm.runtime.test.fact")]
#[mfm_fact(field(
    id = "subject.amount",
    source = "subject",
    path = "amount",
    value_type = "unsigned_integer",
    operator = "equal",
    exposure = "returnable"
))]
#[mfm_fact(field(
    id = "result.amount",
    source = "result",
    path = "amount",
    value_type = "unsigned_integer",
    operators(equal, greater_than),
    exposure = "returnable",
    sortable
))]
#[mfm_fact(ordering(
    name = "result.amount.asc",
    term(field = "result.amount", direction = "ascending", nulls = "last")
))]
pub(super) struct RuntimeTestFact {
    pub(super) subject: CertifierValue,
    pub(super) response: CertifierValue,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct FixtureOutputValue {
    pub(super) amount: u64,
    pub(super) node_id: String,
    pub(super) attempt_id: String,
}

pub(super) fn fixture_output_value(
    amount: u64,
    node_id: impl Into<String>,
    attempt_id: impl Into<String>,
) -> FixtureOutputValue {
    FixtureOutputValue {
        amount,
        node_id: node_id.into(),
        attempt_id: attempt_id.into(),
    }
}

impl mfm_values::MfmValue for FixtureOutputValue {
    fn schema_descriptor() -> mfm_values::Result<mfm_values::SchemaDescriptor> {
        <CertifierValue as mfm_values::MfmValue>::schema_descriptor()
    }

    fn schema_id() -> mfm_values::Result<SchemaId> {
        Ok(fixture_value_schema_id())
    }

    fn semantic_id() -> mfm_values::Result<SemanticTypeId> {
        Ok(fixture_value_semantic_id())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.runtime.test",
    name = "side_effect_evidence",
    version = "1",
    schema = "mfm.runtime.test.side_effect_evidence"
)]
pub(super) struct FixtureSideEffectEvidence {
    pub(super) amount: u64,
    pub(super) node_id: String,
    pub(super) attempt_id: String,
}

pub(super) fn fixture_side_effect_evidence(
    amount: u64,
    node_id: impl Into<String>,
    attempt_id: impl Into<String>,
) -> FixtureSideEffectEvidence {
    FixtureSideEffectEvidence {
        amount,
        node_id: node_id.into(),
        attempt_id: attempt_id.into(),
    }
}

pub(super) fn fixture_side_effect_evidence_for_ctx(
    ctx: &ErasedRunCtx<'_>,
    amount: u64,
) -> FixtureSideEffectEvidence {
    fixture_side_effect_evidence(
        amount,
        ctx.node().node_id.as_str(),
        ctx.attempt_id().as_str(),
    )
}

impl From<&FixtureSideEffectEvidence> for FixtureOutputValue {
    fn from(evidence: &FixtureSideEffectEvidence) -> Self {
        fixture_output_value(evidence.amount, &evidence.node_id, &evidence.attempt_id)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmConfig)]
pub(super) struct CertifierConfig {
    pub(super) multiplier: u64,
}

#[derive(PublicOutputs)]
#[mfm(schema = "mfm.runtime.test.public_outputs")]
pub(super) struct CertifierPublicOutputs<'p, 's> {
    pub(super) result: mfm_program::Handle<'p, 's, CertifierValue>,
}

pub(super) struct CertifierState {
    config: CertifierConfig,
}

impl StateSpec for CertifierState {
    type Config = CertifierConfig;
    type Context = NoContext;
    type Input = CertifierValue;
    type Output = CertifierValue;
    type Effect = mfm_capabilities::Pure;
    type Caps = mfm_capabilities::NoCaps;

    fn kind() -> mfm_program::Result<StateKind> {
        StateKind::new(
            "mfm.runtime.test",
            "multiply",
            DigestAlgorithm::Sha256JcsV1,
            D1,
        )
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn version() -> mfm_program::Result<StateVersion> {
        StateVersion::new("mfm.runtime.test.multiply.v1")
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.runtime.test.multiply"
    }

    fn new(config: mfm_program::ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self {
            config: config.into_inner(),
        })
    }
}

impl PureState for CertifierState {
    fn run(
        &self,
        input: Self::Input,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output> {
        Ok(CertifierValue {
            amount: input.amount * self.config.multiplier,
        })
    }
}

pub(super) struct RuntimeContextSourceState {
    config: CertifierConfig,
}

impl StateSpec for RuntimeContextSourceState {
    type Config = CertifierConfig;
    type Context = RuntimeContractContext;
    type Input = CertifierValue;
    type Output = RuntimeContextOutput;
    type Effect = mfm_capabilities::Pure;
    type Caps = mfm_capabilities::NoCaps;

    fn kind() -> mfm_program::Result<StateKind> {
        runtime_state_kind("context-source", DigestBytes::from_array([0xc1; 32]))
    }

    fn version() -> mfm_program::Result<StateVersion> {
        StateVersion::new("mfm.runtime.test.context_source.v1")
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.runtime.test.context_source"
    }

    fn output_context_contract() -> mfm_program::Result<spec::StateOutputContextContractSpec> {
        Ok(spec::StateOutputContextContractSpec::Produces {
            resource_kind: runtime_context_resource_kind(),
            stage: runtime_context_stage(),
        })
    }

    fn new(config: mfm_program::ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self {
            config: config.into_inner(),
        })
    }
}

impl PureState for RuntimeContextSourceState {
    fn run(
        &self,
        input: Self::Input,
        context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output> {
        Ok(RuntimeContextOutput {
            amount: input.amount * self.config.multiplier,
            context_ref: context.context_ref().clone(),
            resource_kind: runtime_context_resource_kind(),
            stage: runtime_context_stage(),
        })
    }
}

pub(super) struct RuntimeContextConsumerState {
    config: CertifierConfig,
}

impl StateSpec for RuntimeContextConsumerState {
    type Config = CertifierConfig;
    type Context = RuntimeContractContext;
    type Input = RuntimeContextOutput;
    type Output = CertifierValue;
    type Effect = mfm_capabilities::Pure;
    type Caps = mfm_capabilities::NoCaps;

    fn kind() -> mfm_program::Result<StateKind> {
        runtime_state_kind("context-consumer", DigestBytes::from_array([0xc2; 32]))
    }

    fn version() -> mfm_program::Result<StateVersion> {
        StateVersion::new("mfm.runtime.test.context_consumer.v1")
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.runtime.test.context_consumer"
    }

    fn input_context_contract() -> mfm_program::Result<spec::StateInputContextContractSpec> {
        Ok(spec::StateInputContextContractSpec::Required {
            resource_kind: runtime_context_resource_kind(),
            stage: runtime_context_stage(),
            producer: Box::new(spec::ContextProducerSpec {
                producer_descriptor_ids: vec![runtime_context_source_descriptor_id()?],
                seed_producers_allowed: false,
            }),
        })
    }

    fn new(config: mfm_program::ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self {
            config: config.into_inner(),
        })
    }
}

impl PureState for RuntimeContextConsumerState {
    fn run(
        &self,
        input: Self::Input,
        context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output> {
        assert_eq!(context.value().network, "primary");
        Ok(CertifierValue {
            amount: input.amount * self.config.multiplier,
        })
    }
}

pub(super) fn runtime_context_source_descriptor_id() -> mfm_program::Result<DescriptorId> {
    Ok(
        mfm_program::state_descriptor::<RuntimeContextSourceState>()?
            .descriptor_id()
            .clone(),
    )
}

#[derive(PublicOutputs)]
#[mfm(schema = "mfm.runtime.test.fixture_outputs")]
pub(super) struct FixturePublicOutputs<'p, 's> {
    pub(super) result: mfm_program::Handle<'p, 's, FixtureOutputValue>,
}

#[derive(PublicOutputs)]
#[mfm(schema = "mfm.runtime.test.dual_fixture_outputs")]
pub(super) struct DualFixturePublicOutputs<'p, 's> {
    pub(super) result: mfm_program::Handle<'p, 's, FixtureOutputValue>,
    pub(super) side_effect: mfm_program::Handle<'p, 's, FixtureOutputValue>,
}

pub(super) struct RuntimeReadCap;

impl CapabilitySpec for RuntimeReadCap {
    type Role = ReadExternalRole;

    fn kind() -> mfm_capabilities::Result<CapabilityKind> {
        CapabilityKind::new("mfm.test", "read-db", DigestAlgorithm::Sha256JcsV1, D0)
            .map_err(|error| mfm_capabilities::CapabilityError::Identity(error.to_string()))
    }

    fn version() -> mfm_capabilities::Result<CapabilityVersion> {
        CapabilityVersion::new("mfm.cap.read_db.v1")
            .map_err(|error| mfm_capabilities::CapabilityError::Identity(error.to_string()))
    }

    fn name() -> &'static str {
        "read-db"
    }
}

pub(super) struct RuntimeMutationCap;

impl CapabilitySpec for RuntimeMutationCap {
    type Role = ExternalMutationAuthorityRole;

    fn kind() -> mfm_capabilities::Result<CapabilityKind> {
        Ok(side_effect_capability_kind())
    }

    fn version() -> mfm_capabilities::Result<CapabilityVersion> {
        Ok(side_effect_capability_version())
    }

    fn name() -> &'static str {
        "external-mutation"
    }
}

pub(super) fn runtime_adapter_binding() -> AdapterBindingSpec {
    AdapterBindingSpec {
        adapter_kind: AdapterKind::new("mfm.test", "adapter", DigestAlgorithm::Sha256JcsV1, D1)
            .expect("adapter kind"),
        adapter_version: AdapterVersion::new("mfm.adapter.v1").expect("adapter version"),
    }
}

pub(super) fn runtime_state_kind(
    name: &str,
    digest: DigestBytes,
) -> mfm_program::Result<StateKind> {
    StateKind::new(
        "mfm.runtime.test",
        name,
        DigestAlgorithm::Sha256JcsV1,
        digest,
    )
    .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
}

macro_rules! impl_runtime_read_state {
    ($state:ident, $input:ty, $kind:literal, $version:literal, $name:literal, $digest:expr) => {
        pub(super) struct $state {
            config: CertifierConfig,
        }

        impl StateSpec for $state {
            type Config = CertifierConfig;
            type Context = NoContext;
            type Input = $input;
            type Output = FixtureOutputValue;
            type Effect = mfm_capabilities::ReadExternal;
            type Caps = (RuntimeReadCap,);

            fn kind() -> mfm_program::Result<StateKind> {
                runtime_state_kind($kind, $digest)
            }

            fn version() -> mfm_program::Result<StateVersion> {
                StateVersion::new($version)
                    .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
            }

            fn name() -> &'static str {
                $name
            }

            fn new(
                config: mfm_program::ValidatedConfig<Self::Config>,
            ) -> mfm_program::Result<Self> {
                Ok(Self {
                    config: config.into_inner(),
                })
            }
        }

        impl ReadState for $state {
            type Plan = FixtureOutputValue;
            type Evidence = FixtureOutputValue;
            type Facts = ();

            fn plan(
                &self,
                _input: &Self::Input,
                _context: &mfm_program::CertifiedContext<Self::Context>,
            ) -> StateResult<Self::Plan> {
                Ok(fixture_output_value(
                    self.config.multiplier,
                    $name,
                    "typed-read",
                ))
            }

            fn reduce(
                &self,
                input: &Self::Input,
                evidence: &mfm_program::ExternalReadEvidenceSet<Self::Evidence>,
                context: &mfm_program::CertifiedContext<Self::Context>,
            ) -> StateResult<(Self::Output, Self::Facts)> {
                if !evidence.fact_query_evidence().is_empty()
                    || evidence.primary_evidence() != &self.plan(input, context)?
                {
                    return Err(mfm_program::StateError::Message(
                        "runtime fixture read evidence did not match its plan".to_owned(),
                    ));
                }
                Ok((evidence.primary_evidence().clone(), ()))
            }
        }
    };
}

macro_rules! impl_runtime_side_effect_state {
    ($state:ident, $input:ty, $kind:literal, $version:literal, $name:literal, $digest:expr) => {
        pub(super) struct $state {
            config: CertifierConfig,
        }

        impl StateSpec for $state {
            type Config = CertifierConfig;
            type Context = NoContext;
            type Input = $input;
            type Output = FixtureOutputValue;
            type Effect = mfm_capabilities::ApplySideEffect;
            type Caps = (RuntimeMutationCap,);

            fn kind() -> mfm_program::Result<StateKind> {
                runtime_state_kind($kind, $digest)
            }

            fn version() -> mfm_program::Result<StateVersion> {
                StateVersion::new($version)
                    .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
            }

            fn name() -> &'static str {
                $name
            }

            fn adapter_bindings() -> mfm_program::Result<Vec<AdapterBindingSpec>> {
                Ok(vec![runtime_adapter_binding()])
            }

            fn new(
                config: mfm_program::ValidatedConfig<Self::Config>,
            ) -> mfm_program::Result<Self> {
                Ok(Self {
                    config: config.into_inner(),
                })
            }
        }

        impl SideEffectState for $state {
            type Intent = FixtureSideEffectEvidence;
            type IdempotencyInput = FixtureSideEffectEvidence;
            type PreparedInvocation = FixtureSideEffectEvidence;
            type Submission = FixtureSideEffectEvidence;
            type RecoveryEvidence = FixtureSideEffectEvidence;
            type Receipt = FixtureSideEffectEvidence;
            type Confirmation = FixtureSideEffectEvidence;

            fn intent(
                &self,
                input: &Self::Input,
                _context: &mfm_program::CertifiedContext<Self::Context>,
            ) -> StateResult<mfm_program::SideEffectIntent<Self::Intent, Self::IdempotencyInput>>
            {
                let amount = serde_json::to_value(input)
                    .ok()
                    .and_then(|value| value.get("amount").and_then(serde_json::Value::as_u64))
                    .unwrap_or(self.config.multiplier);
                let intent = fixture_side_effect_evidence(amount, $name, "typed-intent");
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
                Ok(receipt.into())
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
                Ok(confirmation.into())
            }
        }
    };
}

impl_runtime_side_effect_state!(
    RuntimeSubmitAState,
    CertifierValue,
    "submit-a",
    "mfm.runtime.test.submit_a.v1",
    "mfm.runtime.test.submit_a",
    DigestBytes::from_array([0xa1; 32])
);
impl_runtime_side_effect_state!(
    RuntimeSubmitBState,
    FixtureOutputValue,
    "submit-b",
    "mfm.runtime.test.submit_b.v1",
    "mfm.runtime.test.submit_b",
    DigestBytes::from_array([0xa2; 32])
);
impl_runtime_read_state!(
    RuntimeReadState,
    FixtureOutputValue,
    "read-output",
    "mfm.runtime.test.read_output.v1",
    "mfm.runtime.test.read_output",
    DigestBytes::from_array([0xa3; 32])
);
impl_runtime_read_state!(
    RuntimeSeedReadState,
    CertifierValue,
    "read-seed",
    "mfm.runtime.test.read_seed.v1",
    "mfm.runtime.test.read_seed",
    DigestBytes::from_array([0xa4; 32])
);

pub(super) struct RuntimeTailState {
    config: CertifierConfig,
}

impl StateSpec for RuntimeTailState {
    type Config = CertifierConfig;
    type Context = NoContext;
    type Input = FixtureOutputValue;
    type Output = FixtureOutputValue;
    type Effect = mfm_capabilities::Pure;
    type Caps = mfm_capabilities::NoCaps;

    fn kind() -> mfm_program::Result<StateKind> {
        runtime_state_kind("tail", DigestBytes::from_array([0xa5; 32]))
    }

    fn version() -> mfm_program::Result<StateVersion> {
        StateVersion::new("mfm.runtime.test.tail.v1")
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.runtime.test.tail"
    }

    fn new(config: mfm_program::ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self {
            config: config.into_inner(),
        })
    }
}

impl PureState for RuntimeTailState {
    fn run(
        &self,
        input: Self::Input,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output> {
        Ok(fixture_output_value(
            input.amount * self.config.multiplier,
            input.node_id,
            input.attempt_id,
        ))
    }
}

pub(super) fn certifier_backed_runtime_authority() -> (
    mfm_certify::CertifiedTypedSpec,
    mfm_certify::CertificationRegistry,
) {
    let mut states = StateRegistryBuilder::new();
    states
        .register::<CertifierState>()
        .expect("state registration");
    let mut registry = mfm_certify::CertificationRegistry::new();
    registry
        .register_state::<CertifierState>()
        .expect("certification registry");
    let draft = build_root_with_registries(
        ScopeKey::new("root").expect("root key"),
        states.snapshot(),
        mfm_program::OperationRegistryBuilder::new().snapshot(),
        |root: &mut RootBuilder<'_, '_>| {
            let seed = root.seed(
                mfm_program::SeedKey::new("initial").expect("seed key"),
                CanonicalSeed::from_value(&CertifierValue { amount: 2 }).expect("seed"),
            )?;
            let result = root.scope().state::<CertifierState, _>(
                StateKey::new("multiply-state")?,
                NoContext,
                CertifierConfig { multiplier: 3 },
                seed,
            )?;
            root.bind_public_outputs(
                PublicOutputKey::new("terminal")?,
                &CertifierPublicOutputs { result },
            )
        },
    )
    .expect("program draft");
    (
        mfm_certify::certify_program_draft(&draft).expect("certified program"),
        registry,
    )
}
