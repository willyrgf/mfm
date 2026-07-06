use super::*;
use mfm_capabilities::{CapabilitySpec, ExternalMutationAuthorityRole};
use mfm_ids::{
    CapabilityKind, CapabilityVersion, ContextResourceKind, ContextStage, DigestBytes, SchemaId,
};
use mfm_program_derive::{
    MfmConfig, MfmValue, OperationOutput as OperationOutputDerive,
    PublicOutputs as PublicOutputsDerive, StateInput as StateInputDerive,
};
use serde::{Deserialize, Serialize};

fn manual_resource_claim() -> ResourceClaim {
    ResourceClaim::manual_only()
}

fn schema_id(name: &'static str, byte: u8) -> SchemaId {
    SchemaId::new(
        name,
        "1",
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([byte; 32]),
    )
    .expect("schema id")
}

fn exclusive_resource_claim(name: &'static str, byte: u8) -> ResourceClaim {
    ResourceClaim::exclusive(
        ResourceNamespace::new(name).expect("resource namespace"),
        schema_id(name, byte),
    )
}

fn seed_id(byte: u8) -> SeedId {
    SeedId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([byte; 32]),
    )
}

fn operator_member(name: &str, byte: u8) -> OperatorAuthorityMemberSpec {
    OperatorAuthorityMemberSpec {
        operator_id: OperatorId::new(format!("mfm.program.test.operator.{name}"))
            .expect("operator id"),
        public_identity: OperatorPublicIdentity::new(format!(
            "mfm.program.test.operator.identity.{byte:02x}"
        ))
        .expect("operator identity"),
    }
}

#[test]
fn manual_authority_builders_reject_empty_duplicates_and_oversized_quorum() {
    assert!(matches!(
        NonEmptyUniqueOperators::try_from_vec(Vec::new()),
        Err(PlanError::ManualPolicy(_))
    ));

    let duplicate = operator_member("duplicate", 0x11);
    assert!(matches!(
        NonEmptyUniqueOperators::try_from_vec(vec![duplicate.clone(), duplicate]),
        Err(PlanError::ManualPolicy(_))
    ));

    assert!(matches!(
        ThresholdQuorum::new(0),
        Err(PlanError::ManualPolicy(_))
    ));

    let operators =
        NonEmptyUniqueOperators::new(operator_member("a", 0x21), vec![operator_member("b", 0x22)])
            .expect("operators");
    let authority = OperatorAuthoritySnapshotDraft::new(
        OperatorAuthorityId::new("mfm.program.test.manual.authority").expect("authority"),
        operators,
    );
    let oversized = ThresholdQuorum::new(3).expect("quorum");
    assert!(matches!(
        ManualAuthorizationDraft::threshold(
            ManualAuthorizationVerifierId::new("mfm.program.test.manual.verifier")
                .expect("verifier"),
            ManualSigningSchemeSpec::new("mfm.manual_resolution.digest_signature.v1")
                .expect("signing scheme"),
            authority.clone(),
            oversized,
        ),
        Err(PlanError::ManualPolicy(_))
    ));

    let auth_policy = ManualAuthorizationDraft::threshold(
        ManualAuthorizationVerifierId::new("mfm.program.test.manual.verifier").expect("verifier"),
        ManualSigningSchemeSpec::new("mfm.manual_resolution.digest_signature.v1")
            .expect("signing scheme"),
        authority,
        ThresholdQuorum::new(2).expect("quorum"),
    )
    .expect("auth policy");
    let policy = ManualResolutionPolicyDraft::new(
        schema_id("mfm.program.test.manual_evidence", 0x23),
        auth_policy,
    );
    let lowered = policy.to_spec();
    assert_eq!(
        lowered.evidence_schema,
        schema_id("mfm.program.test.manual_evidence", 0x23)
    );
}

#[test]
fn resource_claim_constructors_lower_to_spec_shapes() {
    assert!(matches!(
        ResourceClaim::manual_only().as_spec(),
        mfm_spec::v1::ResourceClaimSpec::ManualOnly
    ));
    assert!(matches!(
        exclusive_resource_claim("mfm.program.test.exclusive", 0x31).as_spec(),
        mfm_spec::v1::ResourceClaimSpec::Exclusive { .. }
    ));
    assert!(matches!(
        ResourceClaim::exact_touched_set(
            ResourceNamespace::new("mfm.program.test.touched").expect("namespace"),
            schema_id("mfm.program.test.touched", 0x32),
        )
        .as_spec(),
        mfm_spec::v1::ResourceClaimSpec::ExactTouchedSet { .. }
    ));
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.program.test",
    name = "launch_value",
    version = "1",
    schema = "mfm.program.test.launch_value"
)]
struct LaunchValue {
    amount: u64,
    label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.program.test",
    name = "domain_key",
    version = "1",
    schema = "mfm.program.test.domain_key"
)]
struct TestDomainKey {
    source: String,
    index: u64,
}

impl StableDomainKey for TestDomainKey {}

#[derive(PublicOutputsDerive)]
#[mfm(schema = "mfm.program.test.public_outputs")]
struct LaunchPublicOutputs<'p, 's> {
    result: Handle<'p, 's, LaunchValue>,
}

fn launch_seed(amount: u64, label: &str) -> CanonicalSeed<LaunchValue> {
    CanonicalSeed::from_value(&LaunchValue {
        amount,
        label: label.to_owned(),
    })
    .expect("seed")
}

fn single_seed_public_output_draft(seed: CanonicalSeed<LaunchValue>) -> TypedProgramDraft {
    build_root(ScopeKey::new("root").expect("scope key"), |root| {
        let handle = root.seed(SeedKey::new("first")?, seed)?;
        root.bind_public_outputs(
            PublicOutputKey::new("terminal")?,
            &LaunchPublicOutputs { result: handle },
        )
    })
    .expect("root builds")
}

#[derive(OperationOutputDerive)]
#[mfm(schema = "mfm.program.test.operation_outputs")]
struct LaunchOperationOutputs<'p, 's> {
    result: Handle<'p, 's, LaunchValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, StateInputDerive)]
#[serde(rename_all = "camelCase")]
struct LaunchInput {
    primary_value: LaunchValue,
    optional_value: mfm_values::MaybeValue<LaunchValue>,
    ordered_values: Vec<LaunchValue>,
    required_values: NonEmpty<LaunchValue>,
    artifact_value: mfm_values::ArtifactRef<LaunchValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmConfig)]
#[mfm(schema = "mfm.program.test.launch_config")]
struct LaunchConfig {
    multiplier: u64,
}

fn test_state_kind(name: &str, digest: &[u8]) -> Result<StateKind> {
    StateKind::new(
        "mfm.program.test.state",
        name,
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(digest),
    )
    .map_err(|error| PlanError::Key(error.to_string()))
}

fn test_operation_kind(name: &str, digest: &[u8]) -> Result<OperationKind> {
    OperationKind::new(
        "mfm.program.test.operation",
        name,
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(digest),
    )
    .map_err(|error| PlanError::Key(error.to_string()))
}

#[derive(Debug, Clone)]
struct MultiplyState {
    config: LaunchConfig,
}

impl StateSpec for MultiplyState {
    type Config = LaunchConfig;
    type Context = NoContext;
    type Input = LaunchValue;
    type Output = LaunchValue;
    type Effect = Pure;
    type Caps = NoCaps;

    fn kind() -> Result<StateKind> {
        test_state_kind("multiply", b"mfm.program.test.state:multiply")
    }

    fn version() -> Result<StateVersion> {
        StateVersion::new("mfm.program.test.state.multiply.v1")
            .map_err(|error| PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "multiply"
    }

    fn new(config: ValidatedConfig<Self::Config>) -> Result<Self> {
        Ok(Self {
            config: config.into_inner(),
        })
    }
}

impl PureState for MultiplyState {
    fn run(
        &self,
        input: Self::Input,
        _context: &CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output> {
        Ok(LaunchValue {
            amount: input.amount * self.config.multiplier,
            label: input.label,
        })
    }
}

fn context_resource_kind() -> ContextResourceKind {
    ContextResourceKind::new("mfm.program.test.balance").expect("context resource kind")
}

fn context_stage() -> ContextStage {
    ContextStage::new("resolved").expect("context stage")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.program.test",
    name = "chain_context",
    version = "1",
    schema = "mfm.program.test.chain_context"
)]
struct ChainContext {
    chain_id: u64,
    network: String,
}

impl MfmContext for ChainContext {}

#[derive(Debug, Clone)]
struct ContextualMultiplyState {
    config: LaunchConfig,
}

impl StateSpec for ContextualMultiplyState {
    type Config = LaunchConfig;
    type Context = ChainContext;
    type Input = LaunchValue;
    type Output = LaunchValue;
    type Effect = Pure;
    type Caps = NoCaps;

    fn kind() -> Result<StateKind> {
        test_state_kind(
            "contextual_multiply",
            b"mfm.program.test.state:contextual_multiply",
        )
    }

    fn version() -> Result<StateVersion> {
        StateVersion::new("mfm.program.test.state.contextual_multiply.v1")
            .map_err(|error| PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "contextual_multiply"
    }

    fn output_context_contract() -> Result<StateOutputContextContractSpec> {
        Ok(StateOutputContextContractSpec::Produces {
            resource_kind: context_resource_kind(),
            stage: context_stage(),
        })
    }

    fn new(config: ValidatedConfig<Self::Config>) -> Result<Self> {
        Ok(Self {
            config: config.into_inner(),
        })
    }
}

impl PureState for ContextualMultiplyState {
    fn run(
        &self,
        input: Self::Input,
        _context: &CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output> {
        Ok(LaunchValue {
            amount: input.amount * self.config.multiplier,
            label: input.label,
        })
    }
}

struct TestMutationCap;

impl CapabilitySpec for TestMutationCap {
    type Role = ExternalMutationAuthorityRole;

    fn kind() -> mfm_capabilities::Result<CapabilityKind> {
        CapabilityKind::new(
            "mfm.program.test",
            "mutation",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"mfm.program.test.capability:mutation"),
        )
        .map_err(|error| mfm_capabilities::CapabilityError::Identity(error.to_string()))
    }

    fn version() -> mfm_capabilities::Result<CapabilityVersion> {
        CapabilityVersion::new("mfm.program.test.capability.mutation.v1")
            .map_err(|error| mfm_capabilities::CapabilityError::Identity(error.to_string()))
    }

    fn name() -> &'static str {
        "mutation"
    }
}

#[derive(Debug, Clone)]
struct ForwardMutationState {
    config: LaunchConfig,
}

#[derive(Debug, Clone)]
struct CompensationMutationState {
    config: LaunchConfig,
}

#[derive(Debug, Clone)]
struct ContextualMutationState {
    config: LaunchConfig,
}

macro_rules! impl_side_effect_state_spec {
    ($state:ty, $kind:literal, $version:literal, $name:literal, $digest:literal) => {
        impl StateSpec for $state {
            type Config = LaunchConfig;
            type Context = NoContext;
            type Input = LaunchValue;
            type Output = LaunchValue;
            type Effect = ApplySideEffect;
            type Caps = (TestMutationCap,);

            fn kind() -> Result<StateKind> {
                test_state_kind($kind, $digest)
            }

            fn version() -> Result<StateVersion> {
                StateVersion::new($version).map_err(|error| PlanError::Key(error.to_string()))
            }

            fn name() -> &'static str {
                $name
            }

            fn new(config: ValidatedConfig<Self::Config>) -> Result<Self> {
                Ok(Self {
                    config: config.into_inner(),
                })
            }
        }

        impl SideEffectState for $state {
            type Intent = LaunchValue;
            type IdempotencyInput = LaunchValue;
            type Submission = LaunchValue;
            type Receipt = LaunchValue;
            type Confirmation = LaunchValue;
            type SubmitFuture<'a> = std::future::Ready<StateResult<Self::Submission>>;

            fn prepare_intent(
                &self,
                input: &Self::Input,
                _context: &CertifiedContext<Self::Context>,
            ) -> StateResult<Self::Intent> {
                Ok(LaunchValue {
                    amount: input.amount + self.config.multiplier,
                    label: input.label.clone(),
                })
            }

            fn idempotency_input(
                &self,
                _input: &Self::Input,
                intent: &Self::Intent,
                _context: &CertifiedContext<Self::Context>,
            ) -> StateResult<Self::IdempotencyInput> {
                Ok(intent.clone())
            }

            fn submit<'a>(
                &'a self,
                intent: &'a Self::Intent,
                _key: &'a IdempotencyKey<Self::IdempotencyInput>,
                _caps: &'a Self::Caps,
                _context: &'a CertifiedContext<Self::Context>,
            ) -> Self::SubmitFuture<'a> {
                std::future::ready(Ok(intent.clone()))
            }

            fn output_from_receipt(
                &self,
                _input: &Self::Input,
                _intent: &Self::Intent,
                receipt: &Self::Receipt,
                _context: &CertifiedContext<Self::Context>,
            ) -> StateResult<Self::Output> {
                Ok(receipt.clone())
            }

            fn output_from_confirmation(
                &self,
                _input: &Self::Input,
                _intent: &Self::Intent,
                confirmation: &Self::Confirmation,
                _context: &CertifiedContext<Self::Context>,
            ) -> StateResult<Self::Output> {
                Ok(confirmation.clone())
            }
        }
    };
}

impl_side_effect_state_spec!(
    ForwardMutationState,
    "forward_mutation",
    "mfm.program.test.state.forward_mutation.v1",
    "forward_mutation",
    b"mfm.program.test.state:forward_mutation"
);
impl_side_effect_state_spec!(
    CompensationMutationState,
    "compensation_mutation",
    "mfm.program.test.state.compensation_mutation.v1",
    "compensation_mutation",
    b"mfm.program.test.state:compensation_mutation"
);

impl StateSpec for ContextualMutationState {
    type Config = LaunchConfig;
    type Context = ChainContext;
    type Input = LaunchValue;
    type Output = LaunchValue;
    type Effect = ApplySideEffect;
    type Caps = (TestMutationCap,);

    fn kind() -> Result<StateKind> {
        test_state_kind(
            "contextual_mutation",
            b"mfm.program.test.state:contextual_mutation",
        )
    }

    fn version() -> Result<StateVersion> {
        StateVersion::new("mfm.program.test.state.contextual_mutation.v1")
            .map_err(|error| PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "contextual_mutation"
    }

    fn output_context_contract() -> Result<StateOutputContextContractSpec> {
        Ok(StateOutputContextContractSpec::Produces {
            resource_kind: context_resource_kind(),
            stage: context_stage(),
        })
    }

    fn new(config: ValidatedConfig<Self::Config>) -> Result<Self> {
        Ok(Self {
            config: config.into_inner(),
        })
    }
}

impl SideEffectState for ContextualMutationState {
    type Intent = LaunchValue;
    type IdempotencyInput = LaunchValue;
    type Submission = LaunchValue;
    type Receipt = LaunchValue;
    type Confirmation = LaunchValue;
    type SubmitFuture<'a> = std::future::Ready<StateResult<Self::Submission>>;

    fn prepare_intent(
        &self,
        input: &Self::Input,
        _context: &CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Intent> {
        Ok(LaunchValue {
            amount: input.amount + self.config.multiplier,
            label: input.label.clone(),
        })
    }

    fn idempotency_input(
        &self,
        _input: &Self::Input,
        intent: &Self::Intent,
        _context: &CertifiedContext<Self::Context>,
    ) -> StateResult<Self::IdempotencyInput> {
        Ok(intent.clone())
    }

    fn submit<'a>(
        &'a self,
        intent: &'a Self::Intent,
        _key: &'a IdempotencyKey<Self::IdempotencyInput>,
        _caps: &'a Self::Caps,
        _context: &'a CertifiedContext<Self::Context>,
    ) -> Self::SubmitFuture<'a> {
        std::future::ready(Ok(intent.clone()))
    }

    fn output_from_receipt(
        &self,
        _input: &Self::Input,
        _intent: &Self::Intent,
        receipt: &Self::Receipt,
        _context: &CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output> {
        Ok(receipt.clone())
    }

    fn output_from_confirmation(
        &self,
        _input: &Self::Input,
        _intent: &Self::Intent,
        confirmation: &Self::Confirmation,
        _context: &CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output> {
        Ok(confirmation.clone())
    }
}

#[derive(Debug, Clone)]
struct MultiplyOperation;

impl Operation for MultiplyOperation {
    type Config = LaunchConfig;
    type Input<'program, 'scope> = Handle<'program, 'scope, LaunchValue>;
    type Output<'program, 'scope> = LaunchOperationOutputs<'program, 'scope>;

    fn kind() -> Result<OperationKind> {
        test_operation_kind("multiply", b"mfm.program.test.operation:multiply")
    }

    fn version() -> Result<OperationVersion> {
        OperationVersion::new("mfm.program.test.operation.multiply.v1")
            .map_err(|error| PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "multiply"
    }

    fn expand<'program, 'scope>(
        &self,
        config: ValidatedConfig<Self::Config>,
        input: Self::Input<'program, 'scope>,
        builder: &mut OperationExpansion<'program, 'scope>,
        _dispatch: OperationExpansionDispatch<Self>,
    ) -> Result<Self::Output<'program, 'scope>> {
        let result = builder.state::<MultiplyState, _>(
            StateKey::new("multiply-operation/state")?,
            NoContext,
            config.into_inner(),
            input,
        )?;
        Ok(LaunchOperationOutputs { result })
    }
}

#[derive(Debug, Clone)]
struct ContextualMutationOperation;

impl Operation for ContextualMutationOperation {
    type Config = LaunchConfig;
    type Input<'program, 'scope> = Handle<'program, 'scope, LaunchValue>;
    type Output<'program, 'scope> = LaunchOperationOutputs<'program, 'scope>;

    fn kind() -> Result<OperationKind> {
        test_operation_kind(
            "contextual_mutation",
            b"mfm.program.test.operation:contextual_mutation",
        )
    }

    fn version() -> Result<OperationVersion> {
        OperationVersion::new("mfm.program.test.operation.contextual_mutation.v1")
            .map_err(|error| PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "contextual_mutation"
    }

    fn expand<'program, 'scope>(
        &self,
        config: ValidatedConfig<Self::Config>,
        input: Self::Input<'program, 'scope>,
        builder: &mut OperationExpansion<'program, 'scope>,
        _dispatch: OperationExpansionDispatch<Self>,
    ) -> Result<Self::Output<'program, 'scope>> {
        let context = builder.declare_context(ChainContext {
            chain_id: 31337,
            network: "local".to_owned(),
        })?;
        let result = builder
            .side_effect::<ContextualMutationState, _>(
                StateKey::new("contextual-mutation-operation/state")?,
                &context,
                config.into_inner(),
                input,
                manual_resource_claim(),
                SideEffectVerificationSpec::Receipt,
            )?
            .into_handle();
        Ok(LaunchOperationOutputs { result })
    }
}

#[derive(Debug, Clone)]
struct FailingOperation;

impl Operation for FailingOperation {
    type Config = LaunchConfig;
    type Input<'program, 'scope> = Handle<'program, 'scope, LaunchValue>;
    type Output<'program, 'scope> = LaunchOperationOutputs<'program, 'scope>;

    fn kind() -> Result<OperationKind> {
        test_operation_kind("failing", b"mfm.program.test.operation:failing")
    }

    fn version() -> Result<OperationVersion> {
        OperationVersion::new("mfm.program.test.operation.failing.v1")
            .map_err(|error| PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "failing"
    }

    fn expand<'program, 'scope>(
        &self,
        config: ValidatedConfig<Self::Config>,
        input: Self::Input<'program, 'scope>,
        builder: &mut OperationExpansion<'program, 'scope>,
        _dispatch: OperationExpansionDispatch<Self>,
    ) -> Result<Self::Output<'program, 'scope>> {
        let _planned = builder.state::<MultiplyState, _>(
            StateKey::new("rollback/state")?,
            NoContext,
            config.into_inner(),
            input,
        )?;
        builder.child_scope(ScopeKey::new("rollback/child")?, |child| {
            child.bridge_to_parent(())
        })?;
        Err(PlanError::Key("forced expansion failure".to_owned()))
    }
}

fn register_multiply_state(registry: &mut StateRegistryBuilder) {
    registry
        .register::<MultiplyState>()
        .expect("state registers");
}

fn register_contextual_multiply_state(registry: &mut StateRegistryBuilder) {
    registry
        .register::<ContextualMultiplyState>()
        .expect("contextual state registers");
}

fn multiply_state_registry() -> StateRegistrySnapshot {
    let mut registry = StateRegistryBuilder::new();
    register_multiply_state(&mut registry);
    registry.into_snapshot()
}

fn register_multiply_operation(registry: &mut OperationRegistryBuilder) {
    registry
        .register::<MultiplyOperation>()
        .expect("operation registers");
}

fn multiply_registries() -> (StateRegistrySnapshot, OperationRegistrySnapshot) {
    let mut operations = OperationRegistryBuilder::new();
    register_multiply_operation(&mut operations);
    (multiply_state_registry(), operations.into_snapshot())
}

fn register_forward_mutation_state(registry: &mut StateRegistryBuilder) {
    registry
        .register::<ForwardMutationState>()
        .expect("forward registers");
}

fn register_compensation_mutation_state(registry: &mut StateRegistryBuilder) {
    registry
        .register::<CompensationMutationState>()
        .expect("compensation registers");
}

fn register_contextual_mutation_state(registry: &mut StateRegistryBuilder) {
    registry
        .register::<ContextualMutationState>()
        .expect("contextual mutation registers");
}

fn forward_mutation_registry() -> StateRegistrySnapshot {
    let mut registry = StateRegistryBuilder::new();
    register_forward_mutation_state(&mut registry);
    registry.into_snapshot()
}

fn compensation_registry() -> StateRegistrySnapshot {
    let mut registry = StateRegistryBuilder::new();
    register_forward_mutation_state(&mut registry);
    register_compensation_mutation_state(&mut registry);
    registry.into_snapshot()
}

fn contextual_mutation_registries() -> (StateRegistrySnapshot, OperationRegistrySnapshot) {
    let mut states = StateRegistryBuilder::new();
    register_contextual_mutation_state(&mut states);
    let mut operations = OperationRegistryBuilder::new();
    operations
        .register::<ContextualMutationOperation>()
        .expect("contextual operation registers");
    (states.into_snapshot(), operations.into_snapshot())
}

fn set_compensating_policy(root: &mut RootBuilder<'_, '_>) -> Result<()> {
    root.set_saga_policy(SideEffectSagaPolicy::CompensateCompleted {
        on_remediation_unresolved: RemediationUnresolved::FailWithoutAcdcClaim,
    })
}

#[test]
fn root_builder_binds_seed_and_public_output_specs() {
    let draft = build_root(
        ScopeKey::new("portfolio/root").expect("scope key"),
        |root| {
            let seed = launch_seed(42, "cash");
            let handle = root.seed(SeedKey::new("launch-input")?, seed)?;
            let outputs = LaunchPublicOutputs { result: handle };
            root.bind_public_outputs(PublicOutputKey::new("terminal")?, &outputs)
        },
    )
    .expect("root builds");

    assert_eq!(draft.root_key().as_str(), "portfolio/root");
    assert_eq!(draft.scopes().len(), 1);
    assert_eq!(draft.scopes()[0].key.as_str(), "portfolio/root");
    assert_eq!(draft.scopes()[0].scope_id, *draft.root_scope_id());
    assert_eq!(draft.scopes()[0].parent_scope_id, None);
    assert_eq!(draft.seeds().len(), 1);
    assert_eq!(draft.seeds()[0].key.as_str(), "launch-input");
    assert_eq!(draft.seeds()[0].byte_len, 28);
    assert_eq!(
        draft.seeds()[0].content_digest.as_str(),
        "content:sha256-jcs-v1:3247e9eea57a8b2cc5058f1915144ae32ad6142e75b412c530f7d581d5e4c457"
    );
    assert_eq!(
        draft.seeds()[0].schema_id,
        LaunchValue::schema_id().expect("schema id")
    );
    assert_eq!(
        draft.seeds()[0].semantic_type_id,
        LaunchValue::semantic_id().expect("semantic id")
    );

    let public = draft.public_output_spec();
    assert_eq!(public.key().as_str(), "terminal");
    assert_eq!(public.outputs().len(), 1);
    assert_eq!(public.outputs()[0].public_field_path().as_str(), "result");
    assert_eq!(
        public.outputs()[0].cell().cell_id(),
        &draft.seeds()[0].cell_id
    );
    assert_eq!(public.outputs()[0].cell().scope_id(), draft.root_scope_id());
}

#[test]
fn typed_program_launch_plan_collects_config_material() {
    let (state_registry, operation_registry) = multiply_registries();
    let seed = launch_seed(4, "operation");
    let seed_bytes = seed.canonical_json().clone();
    let draft = build_root_with_registries(
        ScopeKey::new("portfolio/root").expect("scope key"),
        state_registry,
        operation_registry,
        |root| {
            let input = root.seed(SeedKey::new("launch-input")?, seed)?;
            let result = root.scope().call::<MultiplyOperation, _>(
                OperationKey::new("multiply-operation")?,
                MultiplyOperation,
                LaunchConfig { multiplier: 8 },
                input,
            )?;
            root.bind_public_outputs(
                PublicOutputKey::new("terminal")?,
                &LaunchPublicOutputs {
                    result: result.result,
                },
            )
        },
    )
    .expect("root builds");
    let expected_len = draft.state_nodes().len() + draft.operation_lineage().len();
    let seed_material =
        std::collections::BTreeMap::from([(draft.seeds()[0].seed_id.clone(), seed_bytes)]);

    let plan = TypedProgramLaunchPlan::from_draft_and_seed_material(draft.clone(), seed_material)
        .expect("launch plan");

    assert_eq!(plan.config_material.len(), expected_len);
    for node in draft.state_nodes() {
        assert!(plan
            .config_material
            .iter()
            .any(|material| material.schema_id == node.config.schema_id
                && material.bytes == node.config.canonical_json));
    }
    for frame in draft.operation_lineage() {
        assert!(plan
            .config_material
            .iter()
            .any(|material| material.schema_id == frame.config.schema_id
                && material.bytes == frame.config.canonical_json));
    }
    assert_eq!(plan.seed_material.len(), 1);
}

#[test]
fn typed_program_launch_plan_matches_seed_material_by_seed_id() {
    let first_seed = launch_seed(1, "first");
    let first_bytes = first_seed.canonical_json().clone();
    let second_seed = launch_seed(2, "second");
    let second_bytes = second_seed.canonical_json().clone();
    let draft = build_root(ScopeKey::new("root").expect("scope key"), |root| {
        let first = root.seed(SeedKey::new("first")?, first_seed)?;
        let _second = root.seed(SeedKey::new("second")?, second_seed)?;
        root.bind_public_outputs(
            PublicOutputKey::new("terminal")?,
            &LaunchPublicOutputs { result: first },
        )
    })
    .expect("root builds");
    let first_id = draft
        .seeds()
        .iter()
        .find(|seed| seed.key.as_str() == "first")
        .expect("first seed spec")
        .seed_id
        .clone();
    let second_id = draft
        .seeds()
        .iter()
        .find(|seed| seed.key.as_str() == "second")
        .expect("second seed spec")
        .seed_id
        .clone();
    let seeds =
        std::collections::BTreeMap::from([(second_id, second_bytes), (first_id, first_bytes)]);

    let plan = TypedProgramLaunchPlan::from_draft_and_seed_material(draft.clone(), seeds)
        .expect("launch plan");

    let planned_seed_ids = plan
        .seed_material
        .iter()
        .map(|material| material.seed_id.clone())
        .collect::<Vec<_>>();
    let draft_seed_ids = draft
        .seeds()
        .iter()
        .map(|seed| seed.seed_id.clone())
        .collect::<Vec<_>>();
    assert_eq!(planned_seed_ids, draft_seed_ids);
}

#[test]
fn typed_program_launch_plan_rejects_invalid_seed_material() {
    #[derive(Clone, Copy)]
    enum InvalidSeedMaterialCase {
        Missing,
        Unknown,
        Mismatched,
    }

    for (name, case) in [
        ("missing seed material", InvalidSeedMaterialCase::Missing),
        ("unknown seed material", InvalidSeedMaterialCase::Unknown),
        (
            "mismatched seed material",
            InvalidSeedMaterialCase::Mismatched,
        ),
    ] {
        let seed = launch_seed(1, "first");
        let seed_bytes = seed.canonical_json().clone();
        let draft = single_seed_public_output_draft(seed);
        let expected_id = draft.seeds()[0].seed_id.clone();
        let seeds = match case {
            InvalidSeedMaterialCase::Missing => std::collections::BTreeMap::new(),
            InvalidSeedMaterialCase::Unknown => std::collections::BTreeMap::from([
                (expected_id, seed_bytes.clone()),
                (seed_id(0xab), seed_bytes),
            ]),
            InvalidSeedMaterialCase::Mismatched => std::collections::BTreeMap::from([(
                expected_id,
                launch_seed(2, "wrong").canonical_json().clone(),
            )]),
        };

        let error =
            TypedProgramLaunchPlan::from_draft_and_seed_material(draft, seeds).expect_err(name);

        match case {
            InvalidSeedMaterialCase::Missing => assert!(
                matches!(&error, PlanError::Key(message) if message.contains("missing entry-point seed material")),
                "{name}: {error:?}"
            ),
            InvalidSeedMaterialCase::Unknown => assert!(
                matches!(&error, PlanError::Key(message) if message.contains("unknown entry-point seed material")),
                "{name}: {error:?}"
            ),
            InvalidSeedMaterialCase::Mismatched => assert!(
                matches!(&error, PlanError::Canonical(message) if message.contains("did not match draft seed")),
                "{name}: {error:?}"
            ),
        }
    }
}

#[test]
fn registered_state_registry_plans_state_node() {
    let mut registry = StateRegistryBuilder::new();
    register_multiply_state(&mut registry);
    let descriptor = registry
        .snapshot()
        .state_descriptor::<MultiplyState>()
        .expect("registered state descriptor resolves");
    assert_eq!(descriptor.runner(), RunnerKind::Pure);

    let draft = build_root_with_registry(
        ScopeKey::new("portfolio/root").expect("scope key"),
        registry.snapshot(),
        |root| {
            let seed = launch_seed(7, "gross");
            let input = root.seed(SeedKey::new("launch-input")?, seed)?;
            let result = root.scope().state::<MultiplyState, _>(
                StateKey::new("multiply")?,
                NoContext,
                LaunchConfig { multiplier: 3 },
                input,
            )?;
            root.bind_public_outputs(
                PublicOutputKey::new("terminal")?,
                &LaunchPublicOutputs { result },
            )
        },
    )
    .expect("root builds");

    assert_eq!(draft.saga_policy(), &SagaPolicy::NoSideEffects);
    assert_eq!(draft.state_nodes().len(), 1);
    let node = &draft.state_nodes()[0];
    assert_eq!(node.key.as_str(), "multiply");
    assert_eq!(node.scope_id, *draft.root_scope_id());
    assert_eq!(&node.state_descriptor_id, descriptor.descriptor_id());
    assert_eq!(node.runner, RunnerKind::Pure);
    assert_eq!(
        node.config.schema_id,
        LaunchConfig::schema_id().expect("config schema id")
    );
    assert_eq!(
        node.input.input_schema_id,
        LaunchValue::input_schema_id().expect("input schema id")
    );
    assert_eq!(
        node.output_schema_id,
        LaunchValue::schema_id().expect("output schema id")
    );
    assert_eq!(
        node.output_semantic_type_id,
        LaunchValue::semantic_id().expect("output semantic id")
    );
    assert_eq!(
        draft.public_output_spec().outputs()[0].cell().cell_id(),
        &node.output_cell_id
    );
}

#[test]
fn declared_context_refs_are_stable_and_duplicates_reject() {
    fn draft_with_context(chain_id: u64) -> TypedProgramDraft {
        build_root(ScopeKey::new("root").expect("scope key"), |root| {
            let input = root.seed(SeedKey::new("input")?, launch_seed(1, "context"))?;
            let _context = root.scope().declare_context(ChainContext {
                chain_id,
                network: "local".to_owned(),
            })?;
            root.bind_public_outputs(
                PublicOutputKey::new("terminal")?,
                &LaunchPublicOutputs { result: input },
            )
        })
        .expect("root builds")
    }

    let first = draft_with_context(31337);
    let second = draft_with_context(31337);
    let different = draft_with_context(1);
    assert_eq!(first.contexts().len(), 1);
    assert_eq!(
        first.contexts()[0].context_ref,
        second.contexts()[0].context_ref
    );
    assert_ne!(
        first.contexts()[0].context_ref,
        different.contexts()[0].context_ref
    );

    build_root(ScopeKey::new("root").expect("scope key"), |root| {
        let input = root.seed(SeedKey::new("input")?, launch_seed(1, "duplicate"))?;
        let context = ChainContext {
            chain_id: 31337,
            network: "local".to_owned(),
        };
        let _first = root.scope().declare_context(context.clone())?;
        let error = root
            .scope()
            .declare_context(context)
            .expect_err("duplicate context ref rejects");
        assert!(matches!(error, PlanError::DuplicateContextRef(_)));
        root.bind_public_outputs(
            PublicOutputKey::new("terminal")?,
            &LaunchPublicOutputs { result: input },
        )
    })
    .expect("root still builds after handled duplicate");
}

#[test]
fn state_with_declared_context_emits_context_node_output_and_input_metadata() {
    let mut registry = StateRegistryBuilder::new();
    register_contextual_multiply_state(&mut registry);
    let descriptor = registry
        .snapshot()
        .state_descriptor::<ContextualMultiplyState>()
        .expect("registered state descriptor resolves");
    register_multiply_state(&mut registry);

    let draft = build_root_with_registry(
        ScopeKey::new("root").expect("scope key"),
        registry.snapshot(),
        |root| {
            let input = root.seed(SeedKey::new("input")?, launch_seed(2, "contextual"))?;
            let context = root.scope().declare_context(ChainContext {
                chain_id: 31337,
                network: "local".to_owned(),
            })?;
            let context_bound = root.scope().state::<ContextualMultiplyState, _>(
                StateKey::new("contextual")?,
                &context,
                LaunchConfig { multiplier: 5 },
                input,
            )?;
            let result = root.scope().state::<MultiplyState, _>(
                StateKey::new("consume-contextual")?,
                NoContext,
                LaunchConfig { multiplier: 2 },
                context_bound,
            )?;
            root.bind_public_outputs(
                PublicOutputKey::new("terminal")?,
                &LaunchPublicOutputs { result },
            )
        },
    )
    .expect("root builds");

    assert_eq!(draft.contexts().len(), 1);
    let context = &draft.contexts()[0];
    match descriptor.context() {
        StateContextDescriptorSpec::Required(requirement) => {
            assert_eq!(
                &context.context_descriptor_id,
                &requirement.context_descriptor_id
            );
            assert_eq!(&context.schema_id, &requirement.schema_id);
            assert_eq!(&context.semantic_type_id, &requirement.semantic_type_id);
            assert_eq!(
                &context.canonicalizer_identity,
                &requirement.canonicalizer_identity
            );
        }
        StateContextDescriptorSpec::NoContext => panic!("contextual state must require context"),
    }

    let context_node = draft
        .state_nodes()
        .iter()
        .find(|node| node.key.as_str() == "contextual")
        .expect("contextual node");
    assert_eq!(&context_node.context_descriptor, descriptor.context());
    assert_eq!(
        &context_node.output_context_contract,
        descriptor.output_context()
    );
    assert!(matches!(
        &context_node.context,
        NodeContextSpec::Required { context_ref } if context_ref == &context.context_ref
    ));
    match &context_node.output_context {
        CellContextSpec::Bound {
            context_ref,
            resource_kind,
            stage,
            producer,
        } => {
            assert_eq!(context_ref, &context.context_ref);
            assert_eq!(resource_kind, &context_resource_kind());
            assert_eq!(stage, &context_stage());
            assert_eq!(
                producer.producer_descriptor_ids.as_slice(),
                std::slice::from_ref(&context_node.state_descriptor_id)
            );
            assert!(!producer.seed_producers_allowed);
        }
        CellContextSpec::NoContext => panic!("contextual output must be bound"),
    }

    let consumer = draft
        .state_nodes()
        .iter()
        .find(|node| node.key.as_str() == "consume-contextual")
        .expect("consumer node");
    match consumer.input.root.as_ref() {
        InputBindingNodeRef::Cell(cell) => {
            assert_eq!(cell.cell_id(), &context_node.output_cell_id);
            match cell.context() {
                InputContextSpec::Required {
                    context_ref,
                    resource_kind,
                    stage,
                    producer,
                } => {
                    assert_eq!(context_ref, &context.context_ref);
                    assert_eq!(resource_kind, &context_resource_kind());
                    assert_eq!(stage, &context_stage());
                    assert_eq!(
                        producer.producer_descriptor_ids.as_slice(),
                        std::slice::from_ref(&context_node.state_descriptor_id)
                    );
                    assert!(!producer.seed_producers_allowed);
                }
                InputContextSpec::NoContext => panic!("consumer input must carry context"),
            }
        }
        other => panic!("unexpected consumer input binding: {other:?}"),
    }
}

#[test]
fn operation_expansion_can_declare_context_and_plan_context_side_effect() {
    let (states, operations) = contextual_mutation_registries();
    let draft = build_root_with_registries(
        ScopeKey::new("root").expect("scope key"),
        states,
        operations,
        |root| {
            root.set_saga_policy(SideEffectSagaPolicy::FailWithoutAcdcClaim)?;
            let input = root.seed(SeedKey::new("input")?, launch_seed(2, "contextual"))?;
            let result = root.scope().call::<ContextualMutationOperation, _>(
                OperationKey::new("contextual-mutation-operation")?,
                ContextualMutationOperation,
                LaunchConfig { multiplier: 5 },
                input,
            )?;
            root.bind_public_outputs(
                PublicOutputKey::new("terminal")?,
                &LaunchPublicOutputs {
                    result: result.result,
                },
            )
        },
    )
    .expect("root builds");

    assert_eq!(draft.contexts().len(), 1);
    let node = draft
        .state_nodes()
        .iter()
        .find(|node| node.key.as_str() == "contextual-mutation-operation/state")
        .expect("contextual side-effect node");
    assert!(matches!(
        &node.context,
        NodeContextSpec::Required { context_ref } if context_ref == &draft.contexts()[0].context_ref
    ));
    assert!(node.side_effect_resource_claim.is_some());
    assert_eq!(
        node.side_effect_verification.as_ref(),
        Some(&SideEffectVerificationSpec::Receipt)
    );
    assert!(matches!(
        &node.output_context,
        CellContextSpec::Bound { context_ref, .. } if context_ref == &draft.contexts()[0].context_ref
    ));
}

#[test]
fn linked_compensation_authoring_keeps_remediation_out_of_forward_nodes() {
    let draft = build_root_with_registry(
        ScopeKey::new("root").expect("scope key"),
        compensation_registry(),
        |root| {
            set_compensating_policy(root)?;
            let seed = launch_seed(2, "linked");
            let input = root.seed(SeedKey::new("input")?, seed)?;
            let forward_claim = exclusive_resource_claim("mfm.program.test.forward_nonce", 0x61);
            let remediation_claim =
                exclusive_resource_claim("mfm.program.test.remediation_nonce", 0x62);
            let (forward, remediation) = root
                .scope()
                .side_effect_with_compensation::<
                    ForwardMutationState,
                    CompensationMutationState,
                    _,
                    _,
                    _,
                >(
                    NoContext,
                    NoContext,
                    SideEffectNodeParams {
                        key: StateKey::new("forward")?,
                        config: LaunchConfig { multiplier: 5 },
                        input,
                        resource_claim: forward_claim.clone(),
                        verification: SideEffectVerificationSpec::Receipt,
                    },
                    RemediationNodeParams {
                        key: StateKey::new("compensate-forward")?,
                        config: LaunchConfig { multiplier: 7 },
                        resource_claim: remediation_claim.clone(),
                        verification: SideEffectVerificationSpec::Receipt,
                    },
                    |forward| Ok(forward.clone()),
                )?;
            assert_ne!(forward.node_id(), remediation.node_id());
            root.bind_public_outputs(
                PublicOutputKey::new("terminal")?,
                &LaunchPublicOutputs {
                    result: forward.into_handle(),
                },
            )
        },
    )
    .expect("root builds");

    assert!(matches!(
        draft.saga_policy(),
        SagaPolicy::CompensateCompleted { .. }
    ));
    assert_eq!(draft.state_nodes().len(), 1);
    let forward = &draft.state_nodes()[0];
    assert_eq!(forward.key.as_str(), "forward");
    assert_eq!(forward.runner, RunnerKind::ApplySideEffect);
    let expected_forward_claim = exclusive_resource_claim("mfm.program.test.forward_nonce", 0x61);
    assert_eq!(
        forward.side_effect_resource_claim.as_ref(),
        Some(expected_forward_claim.as_spec())
    );
    assert_eq!(
        forward.side_effect_verification.as_ref(),
        Some(&SideEffectVerificationSpec::Receipt)
    );
    let verify = forward
        .side_effect_verify
        .as_ref()
        .expect("forward side-effect verify pair");
    assert_ne!(verify.node_id, forward.node_id);
    assert_ne!(verify.output_cell_id, forward.output_cell_id);
    assert_eq!(
        draft.public_output_spec().outputs()[0].cell().cell_id(),
        &verify.output_cell_id
    );
    let remediation = draft
        .remediation_nodes()
        .get(&forward.node_id)
        .expect("linked remediation");
    assert_eq!(remediation.key.as_str(), "compensate-forward");
    assert_eq!(remediation.runner, RunnerKind::ApplySideEffect);
    let expected_remediation_claim =
        exclusive_resource_claim("mfm.program.test.remediation_nonce", 0x62);
    assert_eq!(
        remediation.side_effect_resource_claim.as_ref(),
        Some(expected_remediation_claim.as_spec())
    );
    assert_eq!(
        remediation.side_effect_verification.as_ref(),
        Some(&SideEffectVerificationSpec::Receipt)
    );
    let remediation_verify = remediation
        .side_effect_verify
        .as_ref()
        .expect("remediation side-effect verify pair");
    assert_ne!(remediation_verify.node_id, remediation.node_id);
    assert_ne!(
        remediation_verify.output_cell_id,
        remediation.output_cell_id
    );
    assert!(
        draft
            .state_nodes()
            .iter()
            .all(|node| node.node_id != remediation.node_id),
        "remediation node must not be in forward node collection"
    );
}

#[test]
fn linked_compensation_rejects_same_forward_and_remediation_key() {
    let err = build_root_with_registry(
        ScopeKey::new("root").expect("scope key"),
        compensation_registry(),
        |root| {
            set_compensating_policy(root)?;
            let seed = launch_seed(2, "same-key");
            let input = root.seed(SeedKey::new("input")?, seed)?;
            let _ = root
                .scope()
                .side_effect_with_compensation::<
                    ForwardMutationState,
                    CompensationMutationState,
                    _,
                    _,
                    _,
                >(
                    NoContext,
                    NoContext,
                    SideEffectNodeParams {
                        key: StateKey::new("forward")?,
                        config: LaunchConfig { multiplier: 5 },
                        input,
                        resource_claim: manual_resource_claim(),
                        verification: SideEffectVerificationSpec::Receipt,
                    },
                    RemediationNodeParams {
                        key: StateKey::new("forward")?,
                        config: LaunchConfig { multiplier: 7 },
                        resource_claim: manual_resource_claim(),
                        verification: SideEffectVerificationSpec::Receipt,
                    },
                    |forward| Ok(forward.clone()),
                )?;
            unreachable!("same-key linked compensation should reject");
        },
    )
    .expect_err("same forward/remediation key rejects");

    assert!(matches!(err, PlanError::DuplicateStateKey(key) if key == "forward"));
}

#[test]
fn compensating_policy_requires_every_forward_side_effect_linked() {
    let err = build_root_with_registry(
        ScopeKey::new("root").expect("scope key"),
        forward_mutation_registry(),
        |root| {
            set_compensating_policy(root)?;
            let seed = launch_seed(2, "gap");
            let input = root.seed(SeedKey::new("input")?, seed)?;
            let result = root.scope().side_effect::<ForwardMutationState, _>(
                StateKey::new("forward")?,
                NoContext,
                LaunchConfig { multiplier: 5 },
                input,
                manual_resource_claim(),
                SideEffectVerificationSpec::Receipt,
            )?;
            root.bind_public_outputs(
                PublicOutputKey::new("terminal")?,
                &LaunchPublicOutputs {
                    result: result.into_handle(),
                },
            )
        },
    )
    .expect_err("coverage gap rejects");

    assert!(
        matches!(err, PlanError::SagaCoverageGap(message) if message.contains("no linked remediation"))
    );
}

#[test]
fn out_of_scope_remediation_binding_fails_finalize() {
    let mut registry = StateRegistryBuilder::new();
    register_multiply_state(&mut registry);
    register_forward_mutation_state(&mut registry);
    register_compensation_mutation_state(&mut registry);

    let err = build_root_with_registry(
        ScopeKey::new("root").expect("scope key"),
        registry.snapshot(),
        |root| {
            set_compensating_policy(root)?;
            let seed = launch_seed(2, "scope");
            let input = root.seed(SeedKey::new("input")?, seed)?;
            let sibling = root.scope().state::<MultiplyState, _>(
                StateKey::new("sibling")?,
                NoContext,
                LaunchConfig { multiplier: 3 },
                input.clone(),
            )?;
            let _ = root
                .scope()
                .side_effect_with_compensation::<
                    ForwardMutationState,
                    CompensationMutationState,
                    _,
                    _,
                    _,
                >(
                    NoContext,
                    NoContext,
                    SideEffectNodeParams {
                        key: StateKey::new("forward")?,
                        config: LaunchConfig { multiplier: 5 },
                        input,
                        resource_claim: manual_resource_claim(),
                        verification: SideEffectVerificationSpec::Receipt,
                    },
                    RemediationNodeParams {
                        key: StateKey::new("compensate-forward")?,
                        config: LaunchConfig { multiplier: 7 },
                        resource_claim: manual_resource_claim(),
                        verification: SideEffectVerificationSpec::Receipt,
                    },
                    |_forward| Ok(sibling),
                )?;
            unreachable!("remediation binding should have rejected");
        },
    )
    .expect_err("out-of-scope remediation input rejects");

    assert!(
        matches!(err, PlanError::RemediationBindingScope(message) if message.contains("outside linked forward"))
    );
}

#[test]
fn state_output_domain_keys_are_lineage_evidence() {
    let registry = multiply_state_registry();

    let build = |domain_key: TestDomainKey| {
        build_root_with_registry(
            ScopeKey::new("root").expect("scope key"),
            registry.clone(),
            |root| {
                let seed = launch_seed(2, "domain");
                let input = root.seed(SeedKey::new("input")?, seed)?;
                let result = root.scope().state_with_domain_keys::<MultiplyState, _, _>(
                    StateKey::new("multiply")?,
                    NoContext,
                    LaunchConfig { multiplier: 5 },
                    input,
                    vec![domain_key],
                )?;
                root.bind_public_outputs(
                    PublicOutputKey::new("terminal")?,
                    &LaunchPublicOutputs { result },
                )
            },
        )
        .expect("root builds")
    };

    let first = build(TestDomainKey {
        source: "portfolio".to_owned(),
        index: 1,
    });
    let second = build(TestDomainKey {
        source: "portfolio".to_owned(),
        index: 2,
    });

    let first_node = &first.state_nodes()[0];
    let second_node = &second.state_nodes()[0];
    assert_eq!(first_node.output_domain_keys.len(), 1);
    assert_eq!(first_node.node_id, second_node.node_id);
    assert_eq!(first_node.output_cell_id, second_node.output_cell_id);
    assert_ne!(
        first_node.output_value_lineage, second_node.output_value_lineage,
        "stable domain keys must be hash-defining for state output lineage"
    );
}

#[test]
fn registered_operation_registry_records_lineage_frame() {
    let state_registry = multiply_state_registry();
    let mut operation_registry = OperationRegistryBuilder::new();
    register_multiply_operation(&mut operation_registry);
    let operation_descriptor = operation_registry
        .snapshot()
        .operation_descriptor::<MultiplyOperation>()
        .expect("registered operation descriptor resolves");

    let draft = build_root_with_registries(
        ScopeKey::new("portfolio/root").expect("scope key"),
        state_registry,
        operation_registry.snapshot(),
        |root| {
            let seed = launch_seed(4, "operation");
            let input = root.seed(SeedKey::new("launch-input")?, seed)?;
            let result = root.scope().call::<MultiplyOperation, _>(
                OperationKey::new("multiply-operation")?,
                MultiplyOperation,
                LaunchConfig { multiplier: 8 },
                input,
            )?;
            root.bind_public_outputs(
                PublicOutputKey::new("terminal")?,
                &LaunchPublicOutputs {
                    result: result.result,
                },
            )
        },
    )
    .expect("root builds");

    assert_eq!(draft.state_nodes().len(), 1);
    assert_eq!(draft.operation_lineage().len(), 1);
    let frame = &draft.operation_lineage()[0];
    assert_eq!(frame.key.as_str(), "multiply-operation");
    assert_eq!(frame.scope_id, *draft.root_scope_id());
    assert_eq!(
        &frame.operation_descriptor_id,
        operation_descriptor.descriptor_id()
    );
    assert_eq!(
        frame.output_schema_id,
        <LaunchOperationOutputs<'static, 'static> as OperationOutput<'static, 'static>>::output_schema_id()
            .expect("output schema id")
    );
    assert_eq!(frame.output_handles.len(), 1);
    assert_eq!(
        frame.output_handles[0].cell_id(),
        &draft.state_nodes()[0].output_cell_id
    );
    assert!(frame
        .lineage_digest
        .as_str()
        .starts_with("content:sha256-jcs-v1:"));
}

#[test]
fn stable_ids_and_value_lineage_golden_vectors() {
    let (state_registry, operation_registry) = multiply_registries();

    let draft = build_root_with_registries(
        ScopeKey::new("portfolio/root").expect("scope key"),
        state_registry,
        operation_registry,
        |root| {
            let seed = launch_seed(4, "operation");
            let input = root.seed(SeedKey::new("launch-input")?, seed)?;
            let result = root.scope().call::<MultiplyOperation, _>(
                OperationKey::new("multiply-operation")?,
                MultiplyOperation,
                LaunchConfig { multiplier: 8 },
                input,
            )?;
            root.bind_public_outputs(
                PublicOutputKey::new("terminal")?,
                &LaunchPublicOutputs {
                    result: result.result,
                },
            )
        },
    )
    .expect("root builds");

    let seed = &draft.seeds()[0];
    let node = &draft.state_nodes()[0];
    let frame = &draft.operation_lineage()[0];
    let empty_lineage = OperationLineage::empty().expect("empty lineage");
    let active_lineage =
        OperationLineage::from_parts(vec![frame.operation_instance_id.clone()], Vec::new())
            .expect("active lineage");
    let alternate_lowering_node_id = NodeId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        canonical_digest_bytes(serde_json::json!({
            "alg": DigestAlgorithm::Sha256JcsV1.as_str(),
            "config_digest": node.config.config_ref_digest.as_str(),
            "input_binding_digest": node.input.digest.as_str(),
            "local_node_key": node.key.as_str(),
            "lowering_version": "mfm.typed.lowering.v1",
            "scope_id": node.scope_id.as_str(),
            "state_kind": node.state_kind.as_str(),
            "state_version": node.state_version.as_str(),
        }))
        .expect("alternate lowering digest"),
    );

    assert_eq!(
        draft.root_scope_id().as_str(),
        "scope:sha256-jcs-v1:b773e3f182b4866afd06c3948883ad1545bd26152fa9ebc9a9df4f67ae7151b9"
    );
    assert_eq!(
        empty_lineage.digest.as_str(),
        "content:sha256-jcs-v1:c593e42bace983cf3dd7de37223c5f442e305ec8e5a37e828d23c4a9e8584f22"
    );
    assert_eq!(
        seed.seed_id.as_str(),
        "seed:sha256-jcs-v1:b5a83431eb098643947f99b1962264571edec7395ac616391543e829bf5c5b4e"
    );
    assert_eq!(
        seed.cell_id.as_str(),
        "cell:sha256-jcs-v1:ee1c049a22d8dfa96c72d97d539d1dbd00de32d8b97beee7d5f33640e071b3d9"
    );
    assert_eq!(
        seed.value_lineage.digest().as_str(),
        "content:sha256-jcs-v1:8bc9b7d9cd6230a162050711b9e2f524c51595835e98105ac0de96fbfa7b4806"
    );
    assert_eq!(
        frame.config.config_ref_digest.as_str(),
        "content:sha256-jcs-v1:4efd60dfc94f22725016efa4bef18e7571683bb788d685c943b1e44266575555"
    );
    assert_eq!(
        node.config.config_ref_digest,
        frame.config.config_ref_digest
    );
    assert_eq!(
        frame.input.digest.as_str(),
        "content:sha256-jcs-v1:31ec5fdd663005e338311043edf8cd20d4f0458087879322c3986d77aa5d8847"
    );
    assert_eq!(node.input.digest, frame.input.digest);
    assert_eq!(
        frame.operation_instance_id.as_str(),
        "op:sha256-jcs-v1:c050bfea69048432a471bd87a76cf268c28ef44158f2b1612bdd84e30ee64190"
    );
    assert_eq!(
        active_lineage.digest.as_str(),
        "content:sha256-jcs-v1:68d4be45f0125749d4d04a8a44a0923e6238a1d0ad98671e389803b60ccc425d"
    );
    assert_eq!(
        node.node_id.as_str(),
        "node:sha256-jcs-v1:dd367a529f7a8d1a7f6bcefd688941942649409bf8e28f3a528af0495c99af89"
    );
    assert_ne!(
        node.node_id, alternate_lowering_node_id,
        "lowering version must be hash-defining for node ids"
    );
    assert_eq!(
        node.output_cell_id.as_str(),
        "cell:sha256-jcs-v1:a0a425a1ed0ef57383e669b3c789759b51ce17d6e77b4821fd149649d83b12cc"
    );
    assert_eq!(
        node.output_value_lineage.digest().as_str(),
        "content:sha256-jcs-v1:e00a1480e94ca6d223f96afdbf1a036960cdc3757314da1aa3e8b7af78077af0"
    );
    assert_eq!(
        frame.lineage_digest.as_str(),
        "content:sha256-jcs-v1:b5340c50d16a162c62ef1bdecdf1340de59f1a188e2a57be337ce068d7e5aa53"
    );
}

#[test]
fn domain_keyed_handles_sort_canonically_and_reject_duplicates() {
    build_root(ScopeKey::new("root").expect("scope key"), |root| {
        let late = root.seed(SeedKey::new("late")?, launch_seed(3, "late"))?;
        let early = root.seed(SeedKey::new("early")?, launch_seed(1, "early"))?;
        let late_ref = late.typed_ref();
        let early_ref = early.typed_ref();
        let keyed = DomainKeyedHandles::new(vec![
            (
                TestDomainKey {
                    source: "z".to_owned(),
                    index: 2,
                },
                late.clone(),
            ),
            (
                TestDomainKey {
                    source: "a".to_owned(),
                    index: 1,
                },
                early.clone(),
            ),
        ])?;
        let reversed_keyed = DomainKeyedHandles::new(vec![
            (
                TestDomainKey {
                    source: "a".to_owned(),
                    index: 1,
                },
                early.clone(),
            ),
            (
                TestDomainKey {
                    source: "z".to_owned(),
                    index: 2,
                },
                late.clone(),
            ),
        ])?;
        let binding: InputBinding<Vec<LaunchValue>> = keyed.into_binding()?;
        let reversed_binding: InputBinding<Vec<LaunchValue>> = reversed_keyed.into_binding()?;
        assert_eq!(
            binding.digest(),
            reversed_binding.digest(),
            "canonical domain-key order must make input order irrelevant"
        );
        let InputBindingNodeKind::Vec {
            ordering,
            domain_keys,
            elements,
        } = &binding.root().kind
        else {
            panic!("domain-keyed handles should bind as vector");
        };
        assert_eq!(ordering, &OrderingEvidence::StableDomainKey);
        assert_eq!(domain_keys.len(), 2);
        assert_eq!(elements.len(), 2);
        assert_eq!(
            binding.digest().as_str(),
            "content:sha256-jcs-v1:422c5aedbcf6740a6de87d7efeebe548ab81245c4572ec1c76b8e322678e07e4"
        );
        assert_eq!(
            domain_keys[0].content_digest.as_str(),
            "content:sha256-jcs-v1:dcf7ba6724b36bcbb4945ba25c50e8fadc7b56b00be10009321a918484de9187"
        );
        assert_eq!(
            domain_keys[1].content_digest.as_str(),
            "content:sha256-jcs-v1:201a76ed8dddaee1cc860ade5d752a9b6e710be9f1ccece57a7c69f6a062e64f"
        );
        let InputBindingNodeKind::Cell(first_cell) = &elements[0].kind else {
            panic!("first domain-keyed element should be a cell");
        };
        let InputBindingNodeKind::Cell(second_cell) = &elements[1].kind else {
            panic!("second domain-keyed element should be a cell");
        };
        assert_eq!(first_cell.cell_id, early_ref.cell_id);
        assert_eq!(second_cell.cell_id, late_ref.cell_id);

        let non_empty_keyed = DomainKeyedNonEmptyHandles::new(vec![
            (
                TestDomainKey {
                    source: "z".to_owned(),
                    index: 2,
                },
                late.clone(),
            ),
            (
                TestDomainKey {
                    source: "a".to_owned(),
                    index: 1,
                },
                early.clone(),
            ),
        ])?;
        let non_empty_binding: InputBinding<NonEmpty<LaunchValue>> =
            non_empty_keyed.into_binding()?;
        let InputBindingNodeKind::NonEmptyVec {
            ordering,
            domain_keys,
            elements,
        } = &non_empty_binding.root().kind
        else {
            panic!("domain-keyed non-empty handles should bind as non-empty vector");
        };
        assert_eq!(ordering, &OrderingEvidence::StableDomainKey);
        assert_eq!(domain_keys.len(), 2);
        assert_eq!(elements.len(), 2);
        assert!(matches!(
            DomainKeyedNonEmptyHandles::<TestDomainKey, LaunchValue>::new(Vec::new()),
            Err(PlanError::EmptyNonEmptyInput)
        ));

        let duplicate = DomainKeyedHandles::new(vec![
            (
                TestDomainKey {
                    source: "dup".to_owned(),
                    index: 1,
                },
                early.clone(),
            ),
            (
                TestDomainKey {
                    source: "dup".to_owned(),
                    index: 1,
                },
                early.clone(),
            ),
        ]);
        assert!(matches!(duplicate, Err(PlanError::DuplicateDomainKey(_))));

        root.bind_public_outputs(
            PublicOutputKey::new("terminal")?,
            &LaunchPublicOutputs { result: early },
        )
    })
    .expect("root builds");
}

#[test]
fn same_scope_same_type_lineage_mismatch_rejects_for_certification() {
    let draft = build_root(ScopeKey::new("root").expect("scope key"), |root| {
        let first = root.seed(SeedKey::new("first")?, launch_seed(1, "first"))?;
        let _second = root.seed(SeedKey::new("second")?, launch_seed(2, "second"))?;
        root.bind_public_outputs(
            PublicOutputKey::new("terminal")?,
            &LaunchPublicOutputs { result: first },
        )
    })
    .expect("root builds");

    let first = draft.seeds()[0].typed_ref();
    let second = draft.seeds()[1].typed_ref();
    assert!(matches!(
        draft.validate_same_scope_same_type_lineage_for_certification(&first, &second),
        Err(PlanError::LineageMismatch(_))
    ));
    assert!(draft
        .validate_same_scope_same_type_lineage_for_certification(&first, &first)
        .is_ok());
}

#[test]
fn failed_operation_expansion_rolls_back_scope_mutations_and_lineage() {
    let state_registry = multiply_state_registry();
    let mut operation_registry = OperationRegistryBuilder::new();
    operation_registry
        .register::<FailingOperation>()
        .expect("failing operation registers");
    register_multiply_operation(&mut operation_registry);

    let draft = build_root_with_registries(
        ScopeKey::new("root").expect("scope key"),
        state_registry,
        operation_registry.into_snapshot(),
        |root| {
            let seed = launch_seed(5, "rollback");
            let input = root.seed(SeedKey::new("input")?, seed)?;
            let failed = root.scope().call::<FailingOperation, _>(
                OperationKey::new("rollback-operation")?,
                FailingOperation,
                LaunchConfig { multiplier: 2 },
                input.clone(),
            );
            let Err(error) = failed else {
                panic!("expansion should fail before returning outputs");
            };
            assert_eq!(error, PlanError::Key("forced expansion failure".to_owned()));

            let _direct = root.scope().state::<MultiplyState, _>(
                StateKey::new("rollback/state")?,
                NoContext,
                LaunchConfig { multiplier: 3 },
                input.clone(),
            )?;
            root.scope()
                .child_scope(ScopeKey::new("rollback/child")?, |child| {
                    child.bridge_to_parent(())
                })?;
            let retry = root.scope().call::<MultiplyOperation, _>(
                OperationKey::new("rollback-operation")?,
                MultiplyOperation,
                LaunchConfig { multiplier: 4 },
                input,
            )?;
            root.bind_public_outputs(
                PublicOutputKey::new("terminal")?,
                &LaunchPublicOutputs {
                    result: retry.result,
                },
            )
        },
    )
    .expect("root builds after failed expansion rollback");

    assert_eq!(draft.operation_lineage().len(), 1);
    assert_eq!(
        draft.operation_lineage()[0].key.as_str(),
        "rollback-operation"
    );
    assert_eq!(
        draft
            .scopes()
            .iter()
            .filter(|scope| scope.key.as_str() == "rollback/child")
            .count(),
        1
    );
    assert_eq!(
        draft
            .state_nodes()
            .iter()
            .filter(|node| node.key.as_str() == "rollback/state")
            .count(),
        1
    );
    let direct_state = draft
        .state_nodes()
        .iter()
        .find(|node| node.key.as_str() == "rollback/state")
        .expect("direct state reused failed expansion state key");
    assert!(direct_state.planning_lineage.active_instances.is_empty());
    assert!(direct_state.planning_lineage.completed_frames.is_empty());
}

#[test]
fn unregistered_operation_and_state_are_rejected_at_registry_boundary() {
    enum Case {
        Operation,
        State,
    }

    for case in [Case::Operation, Case::State] {
        let result = match case {
            Case::Operation => build_root_with_registry(
                ScopeKey::new("root").expect("scope key"),
                multiply_state_registry(),
                |root| {
                    let seed = launch_seed(1, "unregistered-operation");
                    let input = root.seed(SeedKey::new("input")?, seed)?;
                    let _ = root.scope().call::<MultiplyOperation, _>(
                        OperationKey::new("multiply-operation")?,
                        MultiplyOperation,
                        LaunchConfig { multiplier: 2 },
                        input,
                    )?;
                    unreachable!(
                        "unregistered operation planning must fail before public output binding"
                    )
                },
            ),
            Case::State => build_root(ScopeKey::new("root").expect("scope key"), |root| {
                let seed = launch_seed(1, "unregistered");
                let input = root.seed(SeedKey::new("input")?, seed)?;
                let _ = root.scope().state::<MultiplyState, _>(
                    StateKey::new("multiply")?,
                    NoContext,
                    LaunchConfig { multiplier: 2 },
                    input,
                )?;
                unreachable!("unregistered state planning must fail before public output binding")
            }),
        };

        let Err(PlanError::Registry(message)) = result else {
            panic!("expected unregistered registry error, got {result:?}");
        };
        if matches!(case, Case::Operation) {
            assert!(message.contains("operation"));
        }
        assert!(message.contains("is not registered"));
    }
}

#[test]
fn duplicate_state_key_is_rejected_without_partial_node() {
    let result = build_root_with_registry(
        ScopeKey::new("root").expect("scope key"),
        multiply_state_registry(),
        |root| {
            let first = root.seed(SeedKey::new("first")?, launch_seed(1, "first"))?;
            let second = root.seed(SeedKey::new("second")?, launch_seed(2, "second"))?;
            let _ = root.scope().state::<MultiplyState, _>(
                StateKey::new("multiply")?,
                NoContext,
                LaunchConfig { multiplier: 2 },
                first,
            )?;
            let _ = root.scope().state::<MultiplyState, _>(
                StateKey::new("multiply")?,
                NoContext,
                LaunchConfig { multiplier: 3 },
                second,
            )?;
            unreachable!("duplicate state key must fail")
        },
    );

    assert_eq!(
        result.expect_err("duplicate key rejects"),
        PlanError::DuplicateStateKey("multiply".to_owned())
    );
}

#[test]
fn single_handle_input_binding_records_identity_and_lineage() {
    build_root(ScopeKey::new("root").expect("scope key"), |root| {
        let seed = launch_seed(10, "single");
        let handle = root.seed(SeedKey::new("single")?, seed)?;
        let binding: InputBinding<LaunchValue> = handle.clone().into_binding()?;

        let InputBindingNodeKind::Cell(cell) = &binding.root().kind else {
            panic!("single handle should bind as a cell");
        };

        assert_eq!(cell.field_path.as_str(), "root");
        assert_eq!(cell.cell_id, handle.typed_ref().cell_id);
        assert_eq!(
            cell.semantic_type_id,
            LaunchValue::semantic_id().expect("semantic id")
        );
        assert_eq!(cell.schema_id, LaunchValue::schema_id().expect("schema id"));
        assert_eq!(cell.required_terminal, RequiredTerminal::ProducedOnly);
        assert_eq!(&cell.value_lineage, handle.typed_ref().value_lineage());
        assert!(binding
            .digest()
            .as_str()
            .starts_with("content:sha256-jcs-v1:"));

        root.bind_public_outputs(
            PublicOutputKey::new("terminal")?,
            &LaunchPublicOutputs { result: handle },
        )
    })
    .expect("root builds");
}

#[test]
fn optional_and_artifact_handles_bind_as_typed_value_cells() {
    build_root(ScopeKey::new("root").expect("scope key"), |root| {
        let optional_seed =
            CanonicalSeed::from_value(&mfm_values::MaybeValue::Skipped(mfm_values::SkipReason {
                code: mfm_values::SkipCode::Filtered,
                explanation: "filtered by test fixture".to_owned(),
            }))?;
        let optional = root.seed(SeedKey::new("optional")?, optional_seed)?;
        let optional_binding: InputBinding<mfm_values::MaybeValue<LaunchValue>> =
            optional.into_binding()?;

        let InputBindingNodeKind::Cell(cell) = &optional_binding.root().kind else {
            panic!("optional handle should bind as a cell");
        };
        assert_eq!(cell.required_terminal, RequiredTerminal::MaybeSkipped);
        assert_eq!(
            cell.semantic_type_id,
            mfm_values::MaybeValue::<LaunchValue>::semantic_id().expect("maybe semantic id")
        );

        let artifact_ref = artifact_ref_fixture(0xaa)?;
        let artifact = root.seed(
            SeedKey::new("artifact")?,
            CanonicalSeed::from_value(&artifact_ref)?,
        )?;
        let artifact_binding: InputBinding<mfm_values::ArtifactRef<LaunchValue>> =
            artifact.into_binding()?;
        let InputBindingNodeKind::Cell(cell) = &artifact_binding.root().kind else {
            panic!("artifact handle should bind as a cell");
        };
        assert_eq!(cell.required_terminal, RequiredTerminal::ProducedOnly);
        assert_eq!(
            cell.semantic_type_id,
            mfm_values::ArtifactRef::<LaunchValue>::semantic_id().expect("artifact semantic id")
        );

        let final_seed = launch_seed(11, "terminal");
        let final_handle = root.seed(SeedKey::new("terminal-seed")?, final_seed)?;
        root.bind_public_outputs(
            PublicOutputKey::new("terminal")?,
            &LaunchPublicOutputs {
                result: final_handle,
            },
        )
    })
    .expect("root builds");
}

#[test]
fn tuple_vector_and_non_empty_inputs_preserve_author_order() {
    build_root(ScopeKey::new("root").expect("scope key"), |root| {
        let first = root.seed(SeedKey::new("first")?, launch_seed(1, "first"))?;
        let second = root.seed(SeedKey::new("second")?, launch_seed(2, "second"))?;

        let tuple_binding: InputBinding<(LaunchValue, LaunchValue)> =
            (first.clone(), second.clone()).into_binding()?;
        let InputBindingNodeKind::Tuple { elements } = &tuple_binding.root().kind else {
            panic!("tuple input should bind as tuple");
        };
        assert_eq!(elements.len(), 2);
        assert_eq!(cell_path(elements.first().expect("first tuple cell")), "0");
        assert_eq!(cell_path(elements.get(1).expect("second tuple cell")), "1");

        let vector_binding: InputBinding<Vec<LaunchValue>> =
            vec![first.clone(), second.clone()].into_binding()?;
        let reversed_binding: InputBinding<Vec<LaunchValue>> =
            vec![second.clone(), first.clone()].into_binding()?;
        assert_ne!(
            vector_binding.digest(),
            reversed_binding.digest(),
            "explicit vector order is part of the binding digest"
        );
        let InputBindingNodeKind::Vec {
            elements, ordering, ..
        } = &vector_binding.root().kind
        else {
            panic!("vector input should bind as vector");
        };
        assert_eq!(ordering, &OrderingEvidence::ExplicitAuthorOrder);
        assert_eq!(cell_path(elements.first().expect("first vector cell")), "0");
        assert_eq!(cell_path(elements.get(1).expect("second vector cell")), "1");

        assert_eq!(
            NonEmptyHandles::<LaunchValue>::try_from_vec(Vec::new()).expect_err("empty rejects"),
            PlanError::EmptyNonEmptyInput
        );
        let non_empty_binding: InputBinding<NonEmpty<LaunchValue>> =
            NonEmptyHandles::try_from_vec(vec![first.clone(), second.clone()])?.into_binding()?;
        let InputBindingNodeKind::NonEmptyVec {
            elements, ordering, ..
        } = &non_empty_binding.root().kind
        else {
            panic!("non-empty input should bind as non-empty vector");
        };
        assert_eq!(ordering, &OrderingEvidence::ExplicitAuthorOrder);
        assert_eq!(elements.len(), 2);

        let empty_node = InputBindingNode::non_empty_vector(
            Vec::new(),
            OrderingEvidence::ExplicitAuthorOrder,
            Vec::new(),
        );
        assert_eq!(
            InputBinding::<NonEmpty<LaunchValue>>::from_root(empty_node)
                .expect_err("public empty NonEmptyVec rejects"),
            PlanError::EmptyNonEmptyInput
        );

        root.bind_public_outputs(
            PublicOutputKey::new("terminal")?,
            &LaunchPublicOutputs { result: first },
        )
    })
    .expect("root builds");
}

#[test]
fn derive_backed_state_input_handles_build_canonical_struct_bindings() {
    build_root(ScopeKey::new("root").expect("scope key"), |root| {
        let primary = root.seed(SeedKey::new("primary")?, launch_seed(1, "primary"))?;
        let optional = root.seed(
            SeedKey::new("optional")?,
            CanonicalSeed::from_value(&mfm_values::MaybeValue::Produced(LaunchValue {
                amount: 2,
                label: "optional".to_owned(),
            }))?,
        )?;
        let ordered_first = root.seed(
            SeedKey::new("ordered-first")?,
            launch_seed(3, "ordered-first"),
        )?;
        let ordered_second = root.seed(
            SeedKey::new("ordered-second")?,
            launch_seed(4, "ordered-second"),
        )?;
        let artifact = root.seed(
            SeedKey::new("artifact")?,
            CanonicalSeed::from_value(&artifact_ref_fixture(0xbb)?)?,
        )?;

        let handles = LaunchInputHandles {
            primary_value: primary.clone(),
            optional_value: optional,
            ordered_values: vec![ordered_first.clone(), ordered_second.clone()],
            required_values: NonEmptyHandles::new(ordered_first, vec![ordered_second]),
            artifact_value: artifact,
        };
        let binding: InputBinding<LaunchInput> = handles.into_binding()?;
        assert_eq!(
            binding.input_schema_id(),
            &LaunchInput::input_schema_id().map_err(|error| PlanError::Value(error.to_string()))?
        );

        let InputBindingNodeKind::Struct { fields } = &binding.root().kind else {
            panic!("derive-backed state input should bind as a struct");
        };
        let paths = fields
            .iter()
            .map(|field| field.field_path.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            paths,
            vec![
                "artifactValue",
                "optionalValue",
                "orderedValues",
                "primaryValue",
                "requiredValues",
            ]
        );
        let ordered_field = fields
            .iter()
            .find(|field| field.field_path.as_str() == "orderedValues")
            .expect("ordered field");
        let InputBindingNodeKind::Vec {
            elements, ordering, ..
        } = &ordered_field.node.kind
        else {
            panic!("orderedValues should bind as a vector");
        };
        assert_eq!(ordering, &OrderingEvidence::ExplicitAuthorOrder);
        assert_eq!(
            cell_path(elements.first().expect("first ordered cell")),
            "orderedValues.0"
        );

        root.bind_public_outputs(
            PublicOutputKey::new("terminal")?,
            &LaunchPublicOutputs { result: primary },
        )
    })
    .expect("root builds");
}

#[test]
fn duplicate_input_field_paths_reject() {
    let first = NamedInputBinding::new(
        InputFieldPath::new("duplicate").expect("path"),
        InputBindingNode::Unit,
    );
    let second = NamedInputBinding::new(
        InputFieldPath::new("duplicate").expect("path"),
        InputBindingNode::Unit,
    );

    assert_eq!(
        InputBindingNode::struct_fields(vec![first.clone(), second.clone()])
            .expect_err("constructor rejects duplicate paths"),
        PlanError::DuplicateInputFieldPath("duplicate".to_owned())
    );
    assert_eq!(
        InputBinding::<()>::from_root(InputBindingNode::struct_fields_unchecked(vec![
            first, second,
        ]))
        .expect_err("public struct node rejects duplicate paths"),
        PlanError::DuplicateInputFieldPath("duplicate".to_owned())
    );
}

#[test]
fn input_binding_root_must_match_declared_state_input_shape() {
    let error = InputBinding::<LaunchValue>::from_root(InputBindingNode::Unit)
        .expect_err("non-unit input cannot use unit binding root");

    assert!(matches!(
        error,
        PlanError::InputBindingShape(message)
            if message.contains("expected value_ref but got unit")
    ));
}

#[test]
fn child_scope_exports_bridge_nodes_and_validates_refs() {
    let draft = build_root(
        ScopeKey::new("portfolio/root").expect("scope key"),
        |root| {
            let seed = launch_seed(42, "cash");
            let parent_handle = root.seed(SeedKey::new("launch-input")?, seed)?;
            let child_output = root
                .scope()
                .child_scope(ScopeKey::new("execution")?, |child| {
                    let child_handle = child.import_from_parent(
                        BridgeKey::new("import-launch")?,
                        parent_handle.clone(),
                        BridgePolicy::same_run_same_value(),
                    )?;
                    let parent_output = child.export_to_parent(
                        BridgeKey::new("export-result")?,
                        child_handle,
                        BridgePolicy::same_run_same_value(),
                    )?;
                    child.bridge_to_parent(parent_output)
                })?;
            let outputs = LaunchPublicOutputs {
                result: child_output,
            };
            root.bind_public_outputs(PublicOutputKey::new("terminal")?, &outputs)
        },
    )
    .expect("root builds");

    assert_eq!(draft.scopes().len(), 2);
    assert_eq!(draft.scopes()[1].key.as_str(), "execution");
    assert_eq!(
        draft.scopes()[1].parent_scope_id.as_ref(),
        Some(draft.root_scope_id())
    );
    assert_eq!(draft.bridge_nodes().len(), 2);
    assert_eq!(
        draft.bridge_nodes()[0].bridge_kind,
        BridgeKind::ImportFromParent
    );
    assert_eq!(
        draft.bridge_nodes()[1].bridge_kind,
        BridgeKind::ExportToParent
    );
    assert_eq!(
        draft.bridge_nodes()[0].source_scope_id,
        *draft.root_scope_id()
    );
    assert_eq!(
        draft.bridge_nodes()[0].target_scope_id,
        draft.scopes()[1].scope_id
    );
    assert_eq!(
        draft.bridge_nodes()[1].source_scope_id,
        draft.scopes()[1].scope_id
    );
    assert_eq!(
        draft.bridge_nodes()[1].target_scope_id,
        *draft.root_scope_id()
    );

    let export_ref = draft.bridge_nodes()[1].bridge_ref();
    let certified = draft
        .validate_bridge_ref_for_certification(&export_ref)
        .expect("emitted bridge ref certifies");
    assert_eq!(certified.node_id, export_ref.bridge_node_id);
    assert_eq!(
        draft.public_output_spec().outputs()[0].cell().cell_id(),
        &export_ref.target_cell_id
    );
}

#[test]
fn already_parent_bridged_handle_can_flow_through_later_child_scope() {
    let draft = build_root(ScopeKey::new("root").expect("scope key"), |root| {
        let seed = launch_seed(42, "cash");
        let parent_handle = root.seed(SeedKey::new("launch-input")?, seed)?;
        let first_output = root.scope().child_scope(ScopeKey::new("first")?, |child| {
            let child_handle = child.import_from_parent(
                BridgeKey::new("import-launch")?,
                parent_handle.clone(),
                BridgePolicy::same_run_same_value(),
            )?;
            let parent_output = child.export_to_parent(
                BridgeKey::new("export-result")?,
                child_handle,
                BridgePolicy::same_run_same_value(),
            )?;
            child.bridge_to_parent(parent_output)
        })?;
        let second_output = root
            .scope()
            .child_scope(ScopeKey::new("second")?, |child| {
                child.bridge_to_parent(first_output.clone())
            })?;
        let outputs = LaunchPublicOutputs {
            result: second_output,
        };
        root.bind_public_outputs(PublicOutputKey::new("terminal")?, &outputs)
    })
    .expect("root builds");

    assert_eq!(draft.scopes().len(), 3);
    assert_eq!(
        draft.bridge_nodes().len(),
        2,
        "returning an already parent-visible handle must not invent a bridge"
    );
}

#[test]
fn forged_or_stale_bridge_refs_do_not_certify() {
    let left = build_root(ScopeKey::new("left").expect("scope key"), |root| {
        let seed = launch_seed(1, "left");
        let parent_handle = root.seed(SeedKey::new("input")?, seed)?;
        let child_output = root.scope().child_scope(ScopeKey::new("child")?, |child| {
            let child_handle = child.import_from_parent(
                BridgeKey::new("import")?,
                parent_handle.clone(),
                BridgePolicy::same_run_same_value(),
            )?;
            let exported = child.export_to_parent(
                BridgeKey::new("export")?,
                child_handle,
                BridgePolicy::same_run_same_value(),
            )?;
            child.bridge_to_parent(exported)
        })?;
        root.bind_public_outputs(
            PublicOutputKey::new("terminal")?,
            &LaunchPublicOutputs {
                result: child_output,
            },
        )
    })
    .expect("left builds");
    let right = build_root(ScopeKey::new("right").expect("scope key"), |root| {
        let seed = launch_seed(2, "right");
        let parent_handle = root.seed(SeedKey::new("input")?, seed)?;
        let outputs = LaunchPublicOutputs {
            result: parent_handle,
        };
        root.bind_public_outputs(PublicOutputKey::new("terminal")?, &outputs)
    })
    .expect("right builds");

    let stale_ref = left.bridge_nodes()[1].bridge_ref();
    assert_eq!(
        right.validate_bridge_ref_for_certification(&stale_ref),
        Err(PlanError::UnknownBridgeRef)
    );

    let mut forged_ref = stale_ref;
    forged_ref.target_cell_id = right.public_output_spec().outputs()[0]
        .cell()
        .cell_id()
        .clone();
    assert_eq!(
        left.validate_bridge_ref_for_certification(&forged_ref),
        Err(PlanError::UnknownBridgeRef)
    );
}

#[test]
fn duplicate_seed_keys_reject() {
    let error = build_root(ScopeKey::new("root").expect("scope key"), |root| {
        let first = launch_seed(1, "a");
        let second = launch_seed(2, "b");
        let _ = root.seed(SeedKey::new("same")?, first)?;
        let _ = root.seed(SeedKey::new("same")?, second)?;
        unreachable!("duplicate seed must reject before binding outputs")
    })
    .expect_err("duplicate seed rejects");

    assert_eq!(error, PlanError::DuplicateSeedKey("same".to_owned()));
}

#[test]
fn canonical_seed_from_canonical_json_accepts_matching_shape_and_rejects_mismatch() {
    let valid = mfm_canonical::PlainCanonicalJsonBytes::from_json_str(
        "{\"amount\":7,\"label\":\"canonical\"}",
    )
    .expect("canonical json");
    let seed = CanonicalSeed::<LaunchValue>::from_canonical_json(valid).expect("seed");

    assert_eq!(seed.byte_len(), 32);
    assert_eq!(
        seed.content_digest().as_str(),
        "content:sha256-jcs-v1:2b2d92093ac043c94672798bbc5c79761eec80b4aed995400305e8f8a06927e2"
    );
    assert_eq!(
        seed.canonical_json().as_str(),
        "{\"amount\":7,\"label\":\"canonical\"}"
    );

    let invalid = mfm_canonical::PlainCanonicalJsonBytes::from_json_str("{\"amount\":7}")
        .expect("canonical json");
    let error =
        CanonicalSeed::<LaunchValue>::from_canonical_json(invalid).expect_err("missing label");

    assert!(matches!(error, PlanError::Canonical(message) if message.contains("missing field")));
}

#[test]
fn root_scope_id_is_stable_for_key() {
    let left = build_root(ScopeKey::new("same").expect("scope key"), |root| {
        let seed = launch_seed(1, "a");
        let handle = root.seed(SeedKey::new("input")?, seed)?;
        let outputs = LaunchPublicOutputs { result: handle };
        root.bind_public_outputs(PublicOutputKey::new("terminal")?, &outputs)
    })
    .expect("left");
    let right = build_root(ScopeKey::new("same").expect("scope key"), |root| {
        let seed = launch_seed(2, "b");
        let handle = root.seed(SeedKey::new("input")?, seed)?;
        let outputs = LaunchPublicOutputs { result: handle };
        root.bind_public_outputs(PublicOutputKey::new("terminal")?, &outputs)
    })
    .expect("right");

    assert_eq!(left.root_scope_id(), right.root_scope_id());
}

fn cell_path(node: &InputBindingNode) -> &str {
    let InputBindingNodeKind::Cell(cell) = &node.kind else {
        panic!("expected cell node");
    };
    cell.field_path.as_str()
}

fn artifact_ref_fixture(byte: u8) -> Result<mfm_values::ArtifactRef<LaunchValue>> {
    let artifact_id = mfm_ids::ArtifactId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([byte; 32]),
    );
    let content_digest = ContentDigest::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([byte.wrapping_add(1); 32]),
    );
    mfm_values::ArtifactRef::<LaunchValue>::new(artifact_id, content_digest)
        .map_err(|error| PlanError::Value(error.to_string()))
}
