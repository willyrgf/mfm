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

macro_rules! impl_multiplying_pure_state {
    ($state:ty) => {
        impl PureState for $state {
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
    };
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

impl_multiplying_pure_state!(MultiplyState);

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

impl_multiplying_pure_state!(ContextualMultiplyState);

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

macro_rules! impl_side_effect_state {
    ($state:ty) => {
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

        impl_side_effect_state!($state);
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

impl_side_effect_state!(ContextualMutationState);

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

#[path = "tests/behavior.rs"]
mod behavior;
