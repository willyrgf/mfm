use std::collections::VecDeque;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use mfm_capabilities::{
    AdapterError, AdapterInvariantError, CapabilityError, EffectCapabilityContract,
    ReadCapabilityContract,
};
use mfm_ids::{
    ContentRef, DigestBytes, EffectId, EntryPointId, ExecutionPosition, RunId, SemanticTypeId,
    StableId, StatePosition, VisitId,
};
use mfm_journal::{
    EffectConclusion, EncodedRunFrame, JournalHistory, JournalObject, StoredRunBytes,
};
use mfm_program::{
    expand_program, CapabilityInjection, ConclusionBound, EffectBounds, EffectState, FromNever,
    Identity, Never, NoContext, NoParams, Occurrence, Operation, OperationExpansion,
    PreparationError, ProgramError, ProgramLimits, ProposedStateOutcome, PureState, ReadState,
    State,
};
use mfm_program_derive::MfmValue;
use mfm_runtime::{
    EffectAdapterOutcome, InvocationFailure, RunViewState, Runtime, RuntimeAssemblyBuilder,
    RuntimeError,
};
use mfm_store::{AppendResult, MemoryStore, Store, StoreError};
use mfm_values::{
    canonicalize_mfm_value, MfmValue as MfmValueTrait, SchemaAudit, SchemaDescriptor,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct Number {
    value: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NumberAlias {
    value: u64,
}

impl MfmValueTrait for NumberAlias {
    fn schema_descriptor() -> mfm_values::Result<SchemaDescriptor> {
        Number::schema_descriptor()
    }

    fn semantic_id() -> mfm_values::Result<SemanticTypeId> {
        Number::semantic_id()
    }
}

static ALTERNATE_DESCRIPTOR_AUDIT: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MutableDescriptorNumber {
    value: u64,
}

impl MfmValueTrait for MutableDescriptorNumber {
    fn schema_descriptor() -> mfm_values::Result<SchemaDescriptor> {
        let mut descriptor = Number::schema_descriptor()?;
        let rust_type = if ALTERNATE_DESCRIPTOR_AUDIT.load(Ordering::SeqCst) {
            "MutableDescriptorNumber::alternate"
        } else {
            "MutableDescriptorNumber"
        };
        descriptor.audit =
            SchemaAudit::__derive_generated("mfm-runtime-tests", rust_type, "test-only");
        Ok(descriptor)
    }

    fn semantic_id() -> mfm_values::Result<SemanticTypeId> {
        Number::semantic_id()
    }
}

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct FirstGenericValue {
    first: u64,
}

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct SecondGenericValue {
    second: String,
}

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.test.runtime",
    name = "generic-state-value",
    version = "1",
    schema = "mfm.test.runtime-generic-state-value"
)]
struct GenericStateValue<K> {
    value: K,
}

struct GenericState<K>(PhantomData<fn() -> K>);

impl<K: MfmValueTrait> State for GenericState<K> {
    type Input = GenericStateValue<K>;
    type Output = GenericStateValue<K>;
    type Failure = Never;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.runtime/generic-state@1").map_err(|_| ProgramError::InvalidContract)
    }
}

impl<K: MfmValueTrait> PureState for GenericState<K> {
    fn evaluate(
        input: Self::Input,
    ) -> std::result::Result<
        ProposedStateOutcome<Self::Output, Self::Failure>,
        mfm_program::StateExecutionError,
    > {
        Ok(ProposedStateOutcome::Success { output: input })
    }
}

struct ConflictingGenericState<K>(PhantomData<fn() -> K>);

impl<K: MfmValueTrait> State for ConflictingGenericState<K> {
    type Input = GenericStateValue<K>;
    type Output = GenericStateValue<K>;
    type Failure = Never;

    fn state_id() -> mfm_program::Result<StableId> {
        GenericState::<K>::state_id()
    }
}

impl<K: MfmValueTrait> PureState for ConflictingGenericState<K> {
    fn evaluate(
        input: Self::Input,
    ) -> std::result::Result<
        ProposedStateOutcome<Self::Output, Self::Failure>,
        mfm_program::StateExecutionError,
    > {
        Ok(ProposedStateOutcome::Success { output: input })
    }
}

struct GenericProgram<K>(PhantomData<fn() -> K>);

impl<K: MfmValueTrait> Operation for GenericProgram<K> {
    type Input = GenericStateValue<K>;
    type Output = GenericStateValue<K>;
    type Failure = Never;

    fn validate_input(&self, _: &Self::Input) -> mfm_program::Result<()> {
        Ok(())
    }

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        body.pure::<GenericState<K>, Identity<Never>>(
            NoParams,
            Occurrence::new(),
            ConclusionBound::new(65536)?,
        )
    }
}

struct Increment;

impl State for Increment {
    type Input = Number;
    type Output = Number;
    type Failure = Never;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.runtime/increment@1").map_err(|_| ProgramError::InvalidContract)
    }
}

impl PureState for Increment {
    fn evaluate(
        input: Self::Input,
    ) -> std::result::Result<
        ProposedStateOutcome<Self::Output, Self::Failure>,
        mfm_program::StateExecutionError,
    > {
        Ok(ProposedStateOutcome::Success {
            output: Number {
                value: input.value + 1,
            },
        })
    }
}

struct PureProgram;

impl Operation for PureProgram {
    type Input = Number;
    type Output = Number;
    type Failure = Never;

    fn validate_input(&self, _: &Self::Input) -> mfm_program::Result<()> {
        Ok(())
    }

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        body.pure::<Increment, Identity<Never>>(
            NoParams,
            Occurrence::new(),
            ConclusionBound::new(65536)?,
        )
    }
}

struct EmptyProgram;

impl Operation for EmptyProgram {
    type Input = Number;
    type Output = Number;
    type Failure = Never;

    fn validate_input(&self, _: &Self::Input) -> mfm_program::Result<()> {
        Ok(())
    }

    fn expand(
        &self,
        _body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct Intent {
    value: u64,
}

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct Evidence {
    intent_value_ref: ContentRef,
    value: u64,
    accepted: bool,
}

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct Binding {
    route: u64,
}

struct Observation;

impl ReadCapabilityContract for Observation {
    type OperationalError = NoContext;
    type Intent = Intent;
    type Evidence = Evidence;

    fn contract_id() -> mfm_capabilities::Result<StableId> {
        StableId::new("mfm.test.runtime/observation@1")
            .map_err(|_| CapabilityError::InvalidContract)
    }

    fn bind_evidence(
        intent_value_ref: &ContentRef,
        intent: &Self::Intent,
        evidence: &Self::Evidence,
    ) -> mfm_capabilities::Result<()> {
        let expected_value_ref = canonicalize_mfm_value(intent)
            .map(|(_, value_ref)| value_ref)
            .map_err(|_| CapabilityError::EvidenceBinding)?;
        (intent_value_ref == &expected_value_ref
            && intent_value_ref == &evidence.intent_value_ref
            && intent.value == evidence.value)
            .then_some(())
            .ok_or(CapabilityError::EvidenceBinding)
    }
}

struct Observe;

impl State for Observe {
    type Input = Number;
    type Output = Number;
    type Failure = Number;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.runtime/observe@1").map_err(|_| ProgramError::InvalidContract)
    }
}

impl ReadState<Observation> for Observe {
    type AdapterContext = NoContext;
    fn adapter_context(
        _: &Self::Input,
        _: &Intent,
        _: &NoContext,
    ) -> Result<NoContext, mfm_program::StateExecutionError> {
        Ok(NoContext)
    }
    fn prepare(input: &Self::Input) -> Result<Intent, PreparationError> {
        Ok(Intent { value: input.value })
    }

    fn interpret(
        input: Self::Input,
        evidence: &Evidence,
    ) -> std::result::Result<
        ProposedStateOutcome<Self::Output, Self::Failure>,
        mfm_program::StateExecutionError,
    > {
        if evidence.accepted {
            Ok(ProposedStateOutcome::Success { output: input })
        } else {
            Ok(ProposedStateOutcome::Failure { failure: input })
        }
    }
}

impl CapabilityInjection<Observe> for Observation {
    type FailureMap = Identity<Number>;
    fn failure_map_params(_: &Self::Setup) -> mfm_program::Result<NoParams> {
        Ok(NoParams)
    }
    type Setup = Binding;
    type ExpandedInput = Number;
    type ExpandedOutput = Number;
    type ExpandedFailure = <Observe as mfm_program::State>::Failure;

    fn original_binding_ref(setup: &Self::Setup) -> mfm_program::Result<mfm_ids::ContentRef> {
        canonicalize_mfm_value(setup)
            .map(|(_, reference)| reference)
            .map_err(|_| ProgramError::InvalidContract)
    }
}

struct ReadProgram;

impl Operation for ReadProgram {
    type Input = Number;
    type Output = Number;
    type Failure = Number;

    fn validate_input(&self, _: &Self::Input) -> mfm_program::Result<()> {
        Ok(())
    }

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        body.read::<Observe, Observation, Identity<Number>>(
            &Binding { route: 7 },
            NoParams,
            Occurrence::new(),
            ConclusionBound::new(65536)?,
        )
    }
}

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct Command {
    value: u64,
}

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct EffectEvidence {
    effect_id: EffectId,
    value: u64,
    accepted: bool,
}

struct Mutation;

impl EffectCapabilityContract for Mutation {
    type OperationalError = NoContext;
    type Command = Command;
    type Evidence = EffectEvidence;

    fn contract_id() -> mfm_capabilities::Result<StableId> {
        StableId::new("mfm.test.runtime/mutation@1").map_err(|_| CapabilityError::InvalidContract)
    }

    fn bind_evidence(
        effect_id: &EffectId,
        command: &Self::Command,
        evidence: &Self::Evidence,
    ) -> mfm_capabilities::Result<()> {
        (effect_id == &evidence.effect_id && command.value == evidence.value)
            .then_some(())
            .ok_or(CapabilityError::EvidenceBinding)
    }
}

struct ConflictingReadCapability;

impl ReadCapabilityContract for ConflictingReadCapability {
    type OperationalError = NoContext;
    type Intent = Command;
    type Evidence = EffectEvidence;

    fn contract_id() -> mfm_capabilities::Result<StableId> {
        Mutation::contract_id()
    }

    fn bind_evidence(
        _intent_value_ref: &ContentRef,
        intent: &Self::Intent,
        evidence: &Self::Evidence,
    ) -> mfm_capabilities::Result<()> {
        (intent.value == evidence.value)
            .then_some(())
            .ok_or(CapabilityError::EvidenceBinding)
    }
}

struct Mutate;

const PREPARATION_FAILURE_SENTINEL: u64 = u64::MAX;

impl State for Mutate {
    type Input = Number;
    type Output = Number;
    type Failure = Number;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.runtime/mutate@1").map_err(|_| ProgramError::InvalidContract)
    }
}

impl EffectState<Mutation> for Mutate {
    type AdapterContext = NoContext;
    fn adapter_context(
        _: &Self::Input,
        _: &Command,
        _: &NoContext,
    ) -> Result<NoContext, mfm_program::StateExecutionError> {
        Ok(NoContext)
    }
    fn prepare(input: &Self::Input) -> Result<Command, PreparationError> {
        if input.value == PREPARATION_FAILURE_SENTINEL {
            return Err(PreparationError);
        }
        Ok(Command { value: input.value })
    }

    fn interpret(
        input: Self::Input,
        evidence: &EffectEvidence,
    ) -> std::result::Result<
        ProposedStateOutcome<Self::Output, Self::Failure>,
        mfm_program::StateExecutionError,
    > {
        if evidence.accepted {
            Ok(ProposedStateOutcome::Success { output: input })
        } else {
            Ok(ProposedStateOutcome::Failure { failure: input })
        }
    }
}

impl CapabilityInjection<Mutate> for Mutation {
    type FailureMap = Identity<Number>;
    fn failure_map_params(_: &Self::Setup) -> mfm_program::Result<NoParams> {
        Ok(NoParams)
    }
    type Setup = Binding;
    type ExpandedInput = Number;
    type ExpandedOutput = Number;
    type ExpandedFailure = <Mutate as mfm_program::State>::Failure;

    fn original_binding_ref(setup: &Self::Setup) -> mfm_program::Result<mfm_ids::ContentRef> {
        canonicalize_mfm_value(setup)
            .map(|(_, reference)| reference)
            .map_err(|_| ProgramError::InvalidContract)
    }
}

struct EffectProgram;

impl Operation for EffectProgram {
    type Input = Number;
    type Output = Number;
    type Failure = Number;

    fn validate_input(&self, _: &Self::Input) -> mfm_program::Result<()> {
        Ok(())
    }

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        body.effect::<Mutate, Mutation, Identity<Number>>(
            &Binding { route: 8 },
            NoParams,
            Occurrence::new(),
            EffectBounds::new(65536, 65536)?,
        )
    }
}

#[test]
fn exact_value_and_state_abi_collisions_are_rejected() {
    let mut different_type = RuntimeAssemblyBuilder::new().expect("builder");
    different_type
        .register_value::<Number>()
        .expect("number codec");
    assert_eq!(
        different_type.register_value::<NumberAlias>(),
        Err(RuntimeError::IncompatibleAssembly)
    );
    different_type
        .register_value::<Number>()
        .expect("failed registration does not poison the builder");
    different_type.finish();

    ALTERNATE_DESCRIPTOR_AUDIT.store(false, Ordering::SeqCst);
    let mut different_descriptor = RuntimeAssemblyBuilder::new().expect("builder");
    different_descriptor
        .register_value::<MutableDescriptorNumber>()
        .expect("first descriptor");
    ALTERNATE_DESCRIPTOR_AUDIT.store(true, Ordering::SeqCst);
    assert_eq!(
        different_descriptor.register_value::<MutableDescriptorNumber>(),
        Err(RuntimeError::IncompatibleAssembly)
    );
    ALTERNATE_DESCRIPTOR_AUDIT.store(false, Ordering::SeqCst);

    let mut different_state_type = RuntimeAssemblyBuilder::new().expect("builder");
    different_state_type
        .register_pure::<GenericState<FirstGenericValue>>()
        .expect("generic state");
    assert_eq!(
        different_state_type.register_pure::<ConflictingGenericState<FirstGenericValue>>(),
        Err(RuntimeError::IncompatibleAssembly)
    );
}

#[tokio::test]
async fn one_semantic_family_executes_multiple_exact_schemas_hot_and_cold() {
    assert_eq!(
        GenericStateValue::<FirstGenericValue>::semantic_id().expect("first semantic id"),
        GenericStateValue::<SecondGenericValue>::semantic_id().expect("second semantic id")
    );
    assert_ne!(
        mfm_program::nominal_contract_ref::<GenericStateValue<FirstGenericValue>>()
            .expect("first contract"),
        mfm_program::nominal_contract_ref::<GenericStateValue<SecondGenericValue>>()
            .expect("second contract")
    );

    let store = Arc::new(MemoryStore::new());
    let mut builder = RuntimeAssemblyBuilder::new().expect("builder");
    builder
        .register_pure::<GenericState<FirstGenericValue>>()
        .expect("first generic State ABI");
    builder
        .register_pure::<GenericState<SecondGenericValue>>()
        .expect("second generic State ABI");
    let runtime = Runtime::new(builder.finish(), store.clone());

    let first_run_id = RunId::from_digest(DigestBytes::from_array([60; 32]));
    let first_hot = runtime
        .start(
            first_run_id.clone(),
            expand_program(
                EntryPointId::new("mfm.test.runtime/generic-first@1").expect("first entry point"),
                &GenericProgram::<FirstGenericValue>(PhantomData),
                &GenericStateValue {
                    value: FirstGenericValue { first: 11 },
                },
                ProgramLimits::new(0),
            )
            .expect("first Program"),
            GenericStateValue {
                value: FirstGenericValue { first: 11 },
            },
        )
        .await
        .expect("first hot execution");
    let RunViewState::Succeeded(first_hot_value) = first_hot.state() else {
        panic!("first generic Program did not succeed");
    };
    assert_eq!(
        first_hot_value.canonical_bytes(),
        br#"{"value":{"first":11}}"#
    );

    let second_run_id = RunId::from_digest(DigestBytes::from_array([61; 32]));
    let second_hot = runtime
        .start(
            second_run_id.clone(),
            expand_program(
                EntryPointId::new("mfm.test.runtime/generic-second@1").expect("second entry point"),
                &GenericProgram::<SecondGenericValue>(PhantomData),
                &GenericStateValue {
                    value: SecondGenericValue {
                        second: "two".to_owned(),
                    },
                },
                ProgramLimits::new(0),
            )
            .expect("second Program"),
            GenericStateValue {
                value: SecondGenericValue {
                    second: "two".to_owned(),
                },
            },
        )
        .await
        .expect("second hot execution");
    let RunViewState::Succeeded(second_hot_value) = second_hot.state() else {
        panic!("second generic Program did not succeed");
    };
    assert_eq!(
        second_hot_value.canonical_bytes(),
        br#"{"value":{"second":"two"}}"#
    );

    let mut cold_builder = RuntimeAssemblyBuilder::new().expect("builder");
    cold_builder
        .register_pure::<GenericState<FirstGenericValue>>()
        .expect("cold first generic State ABI");
    cold_builder
        .register_pure::<GenericState<SecondGenericValue>>()
        .expect("cold second generic State ABI");
    let cold_runtime = Runtime::new(cold_builder.finish(), store);

    let first_cold = cold_runtime
        .read(&first_run_id)
        .await
        .expect("first cold read");
    let RunViewState::Succeeded(first_cold_value) = first_cold.state() else {
        panic!("first cold generic Program did not succeed");
    };
    assert_eq!(first_cold.head_digest(), first_hot.head_digest());
    assert_eq!(
        first_cold_value.canonical_bytes(),
        first_hot_value.canonical_bytes()
    );

    let second_cold = cold_runtime
        .read(&second_run_id)
        .await
        .expect("second cold read");
    let RunViewState::Succeeded(second_cold_value) = second_cold.state() else {
        panic!("second cold generic Program did not succeed");
    };
    assert_eq!(second_cold.head_digest(), second_hot.head_digest());
    assert_eq!(
        second_cold_value.canonical_bytes(),
        second_hot_value.canonical_bytes()
    );
}

#[test]
fn effect_registration_rejects_duplicates_and_distinguishes_capability_modes() {
    let mut builder = RuntimeAssemblyBuilder::new().expect("builder");
    builder
        .register_effect::<Mutate, Mutation>()
        .expect("Effect State");
    builder
        .register_effect_adapter::<Mutation, _, _>(
            Binding { route: 8 },
            |effect_id, _command_value_ref, command| {
                let effect_id = effect_id.clone();
                let value = command.value;
                Box::pin(async move {
                    Ok(EffectAdapterOutcome::Settled(EffectEvidence {
                        effect_id,
                        value,
                        accepted: true,
                    }))
                })
            },
        )
        .expect("Effect adapter");
    assert_eq!(
        builder.register_effect_adapter::<Mutation, _, _>(
            Binding { route: 8 },
            |effect_id, _command_value_ref, command| {
                let effect_id = effect_id.clone();
                let value = command.value;
                Box::pin(async move {
                    Ok(EffectAdapterOutcome::Settled(EffectEvidence {
                        effect_id,
                        value,
                        accepted: true,
                    }))
                })
            },
        ),
        Err(RuntimeError::IncompatibleAssembly)
    );
    builder
        .register_adapter::<ConflictingReadCapability, _, _>(Binding { route: 8 }, |_, _| {
            Box::pin(async { Err(AdapterError::Invariant(AdapterInvariantError)) })
        })
        .expect("mode participates in the capability contract identity");
}

#[tokio::test]
async fn missing_effect_adapter_is_rejected_before_store_io() {
    let mut builder = RuntimeAssemblyBuilder::new().expect("builder");
    builder
        .register_effect::<Mutate, Mutation>()
        .expect("Effect State");
    let runtime = Runtime::new(builder.finish(), Arc::new(MemoryStore::new()));
    let run_id = RunId::from_digest(DigestBytes::from_array([29; 32]));
    assert!(matches!(
        runtime
            .start(
                run_id.clone(),
                expand_program(
                    EntryPointId::new("mfm.test.runtime/missing-effect-adapter@1")
                        .expect("entry point"),
                    &EffectProgram,
                    &Number { value: 1 },
                    ProgramLimits::new(0),
                )
                .expect("Program"),
                Number { value: 1 },
            )
            .await,
        Err(InvocationFailure::Execution {
            error: RuntimeError::IncompatibleAssembly,
            ..
        })
    ));
    assert!(matches!(
        runtime.read(&run_id).await,
        Err(InvocationFailure::Execution {
            error: RuntimeError::Absent,
            ..
        })
    ));
}

#[tokio::test]
async fn pure_and_zero_state_programs_are_identical_hot_and_cold() {
    let store = Arc::new(MemoryStore::new());
    let mut builder = RuntimeAssemblyBuilder::new().expect("builder");
    builder.register_pure::<Increment>().expect("Pure State");
    let runtime = Runtime::new(builder.finish(), store);
    let run_id = RunId::from_digest(DigestBytes::from_array([1; 32]));
    let program = expand_program(
        EntryPointId::new("mfm.test.runtime/pure@1").expect("entry point"),
        &PureProgram,
        &Number { value: 4 },
        ProgramLimits::new(0),
    )
    .expect("Program");

    let hot = runtime
        .start(run_id.clone(), program, Number { value: 4 })
        .await
        .expect("start");
    let RunViewState::Succeeded(hot_value) = hot.state() else {
        panic!("Pure Program did not succeed");
    };
    assert_eq!(hot.head_sequence(), 2);
    assert_eq!(hot_value.canonical_bytes(), br#"{"value":5}"#);
    assert_eq!(hot_value.decode::<Number>().unwrap().value, 5);
    assert!(matches!(
        hot_value.decode::<FirstGenericValue>(),
        Err(mfm_values::ValueError::SchemaShapeMismatch)
    ));

    let cold = runtime.read(&run_id).await.expect("cold read");
    let RunViewState::Succeeded(cold_value) = cold.state() else {
        panic!("cold Program did not succeed");
    };
    assert_eq!(cold.head_sequence(), hot.head_sequence());
    assert_eq!(cold.head_digest(), hot.head_digest());
    assert_eq!(cold_value.canonical_bytes(), hot_value.canonical_bytes());
    assert_eq!(cold_value.decode::<Number>().unwrap().value, 5);

    let mut empty_builder = RuntimeAssemblyBuilder::new().expect("builder");
    empty_builder
        .register_value::<Number>()
        .expect("root value");
    let empty = Runtime::new(empty_builder.finish(), Arc::new(MemoryStore::new()));
    let empty_program = expand_program(
        EntryPointId::new("mfm.test.runtime/empty@1").expect("entry point"),
        &EmptyProgram,
        &Number { value: 9 },
        ProgramLimits::new(0),
    )
    .expect("empty Program");
    let empty_view = empty
        .start(
            RunId::from_digest(DigestBytes::from_array([2; 32])),
            empty_program,
            Number { value: 9 },
        )
        .await
        .expect("empty start");
    let RunViewState::Succeeded(value) = empty_view.state() else {
        panic!("zero-State Program did not succeed");
    };
    assert_eq!(empty_view.head_sequence(), 1);
    assert_eq!(value.canonical_bytes(), br#"{"value":9}"#);
}

#[tokio::test]
async fn a_fused_read_is_replayable_and_local_invariant_failure_preserves_the_prefix() {
    let calls = Arc::new(AtomicUsize::new(0));
    let store = Arc::new(MemoryStore::new());
    let mut builder = RuntimeAssemblyBuilder::new().expect("builder");
    builder
        .register_read::<Observe, Observation>()
        .expect("Read State");
    builder
        .register_adapter::<Observation, _, _>(Binding { route: 7 }, {
            let calls = Arc::clone(&calls);
            move |intent_value_ref, intent| {
                calls.fetch_add(1, Ordering::SeqCst);
                let intent_value_ref = intent_value_ref.clone();
                Box::pin(async move {
                    Ok(Evidence {
                        intent_value_ref,
                        value: intent.value,
                        accepted: true,
                    })
                })
            }
        })
        .expect("adapter");
    let runtime = Runtime::new(builder.finish(), store);
    let run_id = RunId::from_digest(DigestBytes::from_array([3; 32]));
    let program = expand_program(
        EntryPointId::new("mfm.test.runtime/read@1").expect("entry point"),
        &ReadProgram,
        &Number { value: 12 },
        ProgramLimits::new(0),
    )
    .expect("Program");
    let hot = runtime
        .start(run_id.clone(), program, Number { value: 12 })
        .await
        .expect("Read execution");
    assert!(matches!(hot.state(), RunViewState::Succeeded(_)));
    assert_eq!(hot.head_sequence(), 2);
    let cold = runtime.read(&run_id).await.expect("cold read");
    assert_eq!(cold.head_digest(), hot.head_digest());
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    let unavailable_store = Arc::new(MemoryStore::new());
    let mut unavailable_builder = RuntimeAssemblyBuilder::new().expect("builder");
    unavailable_builder
        .register_read::<Observe, Observation>()
        .expect("Read State");
    unavailable_builder
        .register_adapter::<Observation, _, _>(Binding { route: 7 }, |_, _| {
            Box::pin(async { Err(AdapterError::Invariant(AdapterInvariantError)) })
        })
        .expect("unavailable adapter");
    let unavailable = Runtime::new(unavailable_builder.finish(), unavailable_store.clone());
    let interrupted_run_id = RunId::from_digest(DigestBytes::from_array([4; 32]));
    let interrupted_program = expand_program(
        EntryPointId::new("mfm.test.runtime/read@1").expect("entry point"),
        &ReadProgram,
        &Number { value: 21 },
        ProgramLimits::new(0),
    )
    .expect("Program");
    assert!(matches!(
        unavailable
            .start(
                interrupted_run_id.clone(),
                interrupted_program,
                Number { value: 21 },
            )
            .await,
        Err(InvocationFailure::Execution {
            error: RuntimeError::Internal,
            ..
        })
    ));
    let prefix = unavailable
        .read(&interrupted_run_id)
        .await
        .expect("durable prefix");
    assert_eq!(prefix.head_sequence(), 1);
    assert!(matches!(prefix.state(), RunViewState::Runnable { .. }));

    let mut resumed_builder = RuntimeAssemblyBuilder::new().expect("builder");
    resumed_builder
        .register_read::<Observe, Observation>()
        .expect("Read State");
    resumed_builder
        .register_adapter::<Observation, _, _>(Binding { route: 7 }, |intent_value_ref, intent| {
            let intent_value_ref = intent_value_ref.clone();
            Box::pin(async move {
                Ok(Evidence {
                    intent_value_ref,
                    value: intent.value,
                    accepted: false,
                })
            })
        })
        .expect("replacement adapter");
    let resumed = Runtime::new(resumed_builder.finish(), unavailable_store)
        .resume(&interrupted_run_id)
        .await
        .expect("resume");
    let RunViewState::Failed(value) = resumed.state() else {
        panic!("rejected observation did not follow the Program failure path");
    };
    assert_eq!(resumed.head_sequence(), 2);
    let mfm_runtime::FailureCauseView::Domain { original, root } = value.cause() else {
        panic!("domain result")
    };
    assert_eq!(original.decode::<Number>().unwrap().value, 21);
    assert_eq!(root.decode::<Number>().unwrap().value, 21);
}

#[tokio::test]
async fn cancellation_during_observation_preserves_a_runnable_prefix() {
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let store = Arc::new(MemoryStore::new());
    let mut builder = RuntimeAssemblyBuilder::new().expect("builder");
    builder
        .register_read::<Observe, Observation>()
        .expect("Read State");
    builder
        .register_adapter::<Observation, _, _>(Binding { route: 7 }, {
            let entered = Arc::clone(&entered);
            let release = Arc::clone(&release);
            move |intent_value_ref, intent| {
                let entered = Arc::clone(&entered);
                let release = Arc::clone(&release);
                let intent_value_ref = intent_value_ref.clone();
                Box::pin(async move {
                    entered.notify_one();
                    release.notified().await;
                    Ok(Evidence {
                        intent_value_ref,
                        value: intent.value,
                        accepted: true,
                    })
                })
            }
        })
        .expect("adapter");
    let runtime = Arc::new(Runtime::new(builder.finish(), store));
    let run_id = RunId::from_digest(DigestBytes::from_array([5; 32]));
    let task = {
        let runtime = Arc::clone(&runtime);
        let run_id = run_id.clone();
        tokio::spawn(async move {
            runtime
                .start(
                    run_id,
                    expand_program(
                        EntryPointId::new("mfm.test.runtime/cancel@1").expect("entry point"),
                        &ReadProgram,
                        &Number { value: 8 },
                        ProgramLimits::new(0),
                    )
                    .expect("Program"),
                    Number { value: 8 },
                )
                .await
        })
    };
    entered.notified().await;
    task.abort();
    match task.await {
        Err(error) => assert!(error.is_cancelled()),
        Ok(_) => panic!("observation task was not cancelled"),
    }
    release.notify_waiters();

    let prefix = runtime.read(&run_id).await.expect("durable prefix");
    assert_eq!(prefix.head_sequence(), 1);
    assert!(matches!(prefix.state(), RunViewState::Runnable { .. }));
}

#[tokio::test]
async fn effect_prepare_is_durable_before_adapter_entry_and_cold_resume_reuses_identity() {
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let observed = Arc::new(std::sync::Mutex::new(Vec::new()));
    let store = Arc::new(MemoryStore::new());
    let mut builder = RuntimeAssemblyBuilder::new().expect("builder");
    builder
        .register_effect::<Mutate, Mutation>()
        .expect("Effect State");
    builder
        .register_effect_adapter::<Mutation, _, _>(Binding { route: 8 }, {
            let entered = Arc::clone(&entered);
            let release = Arc::clone(&release);
            let observed = Arc::clone(&observed);
            move |effect_id, command_value_ref, command| {
                let effect_id = effect_id.clone();
                let command_value_ref = command_value_ref.clone();
                let value = command.value;
                observed.lock().expect("observations").push((
                    effect_id.clone(),
                    command_value_ref,
                    value,
                ));
                let entered = Arc::clone(&entered);
                let release = Arc::clone(&release);
                Box::pin(async move {
                    entered.notify_one();
                    release.notified().await;
                    Ok(EffectAdapterOutcome::Settled(EffectEvidence {
                        effect_id,
                        value,
                        accepted: true,
                    }))
                })
            }
        })
        .expect("Effect adapter");
    let runtime = Arc::new(Runtime::new(builder.finish(), store.clone()));
    let run_id = RunId::from_digest(DigestBytes::from_array([30; 32]));
    let task = {
        let runtime = Arc::clone(&runtime);
        let run_id = run_id.clone();
        tokio::spawn(async move {
            runtime
                .start(
                    run_id,
                    expand_program(
                        EntryPointId::new("mfm.test.runtime/effect@1").expect("entry point"),
                        &EffectProgram,
                        &Number { value: 34 },
                        ProgramLimits::new(0),
                    )
                    .expect("Program"),
                    Number { value: 34 },
                )
                .await
        })
    };
    entered.notified().await;

    let pending = runtime.read(&run_id).await.expect("pending view");
    assert_eq!(pending.head_sequence(), 2);
    assert!(matches!(
        pending.state(),
        RunViewState::EffectPending { .. }
    ));
    task.abort();
    match task.await {
        Err(error) => assert!(error.is_cancelled()),
        Ok(_) => panic!("Effect task was not cancelled"),
    }
    release.notify_waiters();
    let (first_effect, first_command_value_ref) = {
        let observations = observed.lock().expect("observations");
        (observations[0].0.clone(), observations[0].1.clone())
    };
    assert_eq!(
        first_command_value_ref,
        canonicalize_mfm_value(&Command { value: 34 })
            .map(|(_, value_ref)| value_ref)
            .expect("command value ref")
    );

    let resumed_observed = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut resumed_builder = RuntimeAssemblyBuilder::new().expect("builder");
    resumed_builder
        .register_effect::<Mutate, Mutation>()
        .expect("Effect State");
    resumed_builder
        .register_effect_adapter::<Mutation, _, _>(Binding { route: 8 }, {
            let resumed_observed = Arc::clone(&resumed_observed);
            move |effect_id, command_value_ref, command| {
                let effect_id = effect_id.clone();
                let command_value_ref = command_value_ref.clone();
                let value = command.value;
                resumed_observed
                    .lock()
                    .expect("resumed observations")
                    .push((effect_id.clone(), command_value_ref, value));
                Box::pin(async move {
                    Ok(EffectAdapterOutcome::Settled(EffectEvidence {
                        effect_id,
                        value,
                        accepted: true,
                    }))
                })
            }
        })
        .expect("Effect adapter");
    let resumed_runtime = Runtime::new(resumed_builder.finish(), store);
    let resumed = resumed_runtime.resume(&run_id).await.expect("cold resume");
    assert_eq!(resumed.head_sequence(), 3);
    assert!(matches!(resumed.state(), RunViewState::Succeeded(_)));
    {
        let resumed_calls = resumed_observed.lock().expect("resumed observations");
        assert_eq!(
            resumed_calls.as_slice(),
            &[(first_effect, first_command_value_ref, 34)]
        );
    }

    let cold = resumed_runtime.read(&run_id).await.expect("cold view");
    assert_eq!(cold.head_digest(), resumed.head_digest());
    assert_eq!(
        resumed_observed.lock().expect("resumed observations").len(),
        1
    );
}

#[tokio::test]
async fn pending_yields_once_and_a_later_settlement_closes_the_same_prepare() {
    let calls = Arc::new(AtomicUsize::new(0));
    let store = Arc::new(MemoryStore::new());
    let mut builder = RuntimeAssemblyBuilder::new().expect("builder");
    builder
        .register_effect::<Mutate, Mutation>()
        .expect("Effect State");
    builder
        .register_effect_adapter::<Mutation, _, _>(Binding { route: 8 }, {
            let calls = Arc::clone(&calls);
            move |effect_id, _command_value_ref, command| {
                let invocation = calls.fetch_add(1, Ordering::SeqCst);
                let effect_id = effect_id.clone();
                let value = command.value;
                Box::pin(async move {
                    if invocation == 0 {
                        Ok(EffectAdapterOutcome::Pending)
                    } else {
                        Ok(EffectAdapterOutcome::Settled(EffectEvidence {
                            effect_id,
                            value,
                            accepted: true,
                        }))
                    }
                })
            }
        })
        .expect("Effect adapter");
    let runtime = Runtime::new(builder.finish(), store);
    let run_id = RunId::from_digest(DigestBytes::from_array([45; 32]));

    let pending = runtime
        .start(
            run_id.clone(),
            expand_program(
                EntryPointId::new("mfm.test.runtime/pending-effect@1").expect("entry point"),
                &EffectProgram,
                &Number { value: 21 },
                ProgramLimits::new(0),
            )
            .expect("Program"),
            Number { value: 21 },
        )
        .await
        .expect("pending is normal progress");
    assert_eq!(pending.head_sequence(), 2);
    assert!(matches!(
        pending.state(),
        RunViewState::EffectPending { .. }
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    let cold = runtime.read(&run_id).await.expect("cold pending view");
    assert_eq!(cold.head_digest(), pending.head_digest());
    assert!(matches!(cold.state(), RunViewState::EffectPending { .. }));
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    let settled = runtime.resume(&run_id).await.expect("later settlement");
    assert_eq!(settled.head_sequence(), 3);
    assert!(matches!(settled.state(), RunViewState::Succeeded(_)));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn effect_preparation_and_evidence_failures_append_no_conclusion() {
    let calls = Arc::new(AtomicUsize::new(0));
    let store = Arc::new(MemoryStore::new());
    let mut builder = RuntimeAssemblyBuilder::new().expect("builder");
    builder
        .register_effect::<Mutate, Mutation>()
        .expect("Effect State");
    builder
        .register_effect_adapter::<Mutation, _, _>(Binding { route: 8 }, {
            let calls = Arc::clone(&calls);
            move |effect_id, _command_value_ref, command| {
                calls.fetch_add(1, Ordering::SeqCst);
                let effect_id = effect_id.clone();
                let value = command.value;
                Box::pin(async move {
                    Ok(EffectAdapterOutcome::Settled(EffectEvidence {
                        effect_id,
                        value,
                        accepted: true,
                    }))
                })
            }
        })
        .expect("adapter");
    let runtime = Runtime::new(builder.finish(), store);
    let run_id = RunId::from_digest(DigestBytes::from_array([31; 32]));
    assert!(matches!(
        runtime
            .start(
                run_id.clone(),
                expand_program(
                    EntryPointId::new("mfm.test.runtime/reject-effect@1").expect("entry point"),
                    &EffectProgram,
                    &Number {
                        value: PREPARATION_FAILURE_SENTINEL
                    },
                    ProgramLimits::new(0),
                )
                .expect("Program"),
                Number {
                    value: PREPARATION_FAILURE_SENTINEL,
                },
            )
            .await,
        Err(InvocationFailure::Execution {
            error: RuntimeError::Internal,
            ..
        })
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        runtime
            .read(&run_id)
            .await
            .expect("genesis")
            .head_sequence(),
        1
    );

    let invalid_store = Arc::new(MemoryStore::new());
    let mut invalid_builder = RuntimeAssemblyBuilder::new().expect("builder");
    invalid_builder
        .register_effect::<Mutate, Mutation>()
        .expect("Effect State");
    invalid_builder
        .register_effect_adapter::<Mutation, _, _>(
            Binding { route: 8 },
            |_effect_id, _command_value_ref, command| {
                let value = command.value;
                Box::pin(async move {
                    Ok(EffectAdapterOutcome::Settled(EffectEvidence {
                        effect_id: EffectId::from_digest(DigestBytes::from_array([99; 32])),
                        value,
                        accepted: true,
                    }))
                })
            },
        )
        .expect("adapter");
    let invalid = Runtime::new(invalid_builder.finish(), invalid_store);
    let invalid_run_id = RunId::from_digest(DigestBytes::from_array([32; 32]));
    assert!(matches!(
        invalid
            .start(
                invalid_run_id.clone(),
                expand_program(
                    EntryPointId::new("mfm.test.runtime/invalid-evidence@1").expect("entry point"),
                    &EffectProgram,
                    &Number { value: 2 },
                    ProgramLimits::new(0),
                )
                .expect("Program"),
                Number { value: 2 },
            )
            .await,
        Err(InvocationFailure::Execution {
            error: RuntimeError::Internal,
            ..
        })
    ));
    assert_eq!(
        invalid
            .read(&invalid_run_id)
            .await
            .expect("pending")
            .head_sequence(),
        2
    );
}

#[derive(Clone, Copy)]
enum AppendAction {
    RetainThenNotInserted,
    Indeterminate,
    RetainThenIndeterminate,
}

struct ScriptedStore {
    inner: MemoryStore,
    actions: std::sync::Mutex<VecDeque<(u64, AppendAction)>>,
    frames: std::sync::Mutex<Vec<Vec<u8>>>,
}

impl ScriptedStore {
    fn new(actions: impl IntoIterator<Item = (u64, AppendAction)>) -> Self {
        Self {
            inner: MemoryStore::new(),
            actions: std::sync::Mutex::new(actions.into_iter().collect()),
            frames: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn recording() -> Self {
        Self::new([])
    }

    fn snapshot(&self) -> Vec<Vec<u8>> {
        self.frames.lock().expect("recorded frames").clone()
    }

    fn record_if_inserted(&self, frame: &EncodedRunFrame, result: AppendResult) {
        if result == AppendResult::Inserted {
            self.frames
                .lock()
                .expect("recorded frames")
                .push(frame.canonical_bytes().to_vec());
        }
    }
}

impl Store for ScriptedStore {
    fn load_run<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> Pin<
        Box<
            dyn Future<Output = std::result::Result<Option<StoredRunBytes>, StoreError>>
                + Send
                + 'a,
        >,
    > {
        self.inner.load_run(run_id)
    }

    fn append_run<'a>(
        &'a self,
        frame: &'a EncodedRunFrame,
    ) -> Pin<Box<dyn Future<Output = Result<AppendResult, StoreError>> + Send + 'a>> {
        Box::pin(async move {
            let action = {
                let mut actions = self.actions.lock().expect("append actions");
                actions
                    .iter()
                    .position(|(sequence, _)| *sequence == frame.run_sequence())
                    .and_then(|index| actions.remove(index))
                    .map(|(_, action)| action)
            };
            match action {
                Some(AppendAction::RetainThenNotInserted) => {
                    let result = self.inner.append_run(frame).await?;
                    self.record_if_inserted(frame, result);
                    Ok(AppendResult::NotInserted)
                }
                Some(AppendAction::Indeterminate) => Err(StoreError::Indeterminate),
                Some(AppendAction::RetainThenIndeterminate) => {
                    let result = self.inner.append_run(frame).await?;
                    self.record_if_inserted(frame, result);
                    Err(StoreError::Indeterminate)
                }
                None => {
                    let result = self.inner.append_run(frame).await?;
                    self.record_if_inserted(frame, result);
                    Ok(result)
                }
            }
        })
    }
}

struct RetainedStore(Vec<Vec<u8>>);

impl Store for RetainedStore {
    fn load_run<'a>(
        &'a self,
        _run_id: &'a RunId,
    ) -> Pin<Box<dyn Future<Output = Result<Option<StoredRunBytes>, StoreError>> + Send + 'a>> {
        Box::pin(async move {
            StoredRunBytes::new(self.0.clone())
                .map(Some)
                .map_err(|_| StoreError::CorruptPhysicalState)
        })
    }

    fn append_run<'a>(
        &'a self,
        _frame: &'a EncodedRunFrame,
    ) -> Pin<Box<dyn Future<Output = Result<AppendResult, StoreError>> + Send + 'a>> {
        Box::pin(async { Err(StoreError::CorruptPhysicalState) })
    }
}

fn effect_runtime_with_counting_adapter(
    store: Arc<dyn Store>,
    calls: Arc<AtomicUsize>,
    effect_ids: Arc<std::sync::Mutex<Vec<EffectId>>>,
) -> Runtime {
    let mut builder = RuntimeAssemblyBuilder::new().expect("builder");
    builder
        .register_effect::<Mutate, Mutation>()
        .expect("Effect State");
    builder
        .register_effect_adapter::<Mutation, _, _>(
            Binding { route: 8 },
            move |effect_id, _command_value_ref, command| {
                calls.fetch_add(1, Ordering::SeqCst);
                let effect_id = effect_id.clone();
                effect_ids
                    .lock()
                    .expect("effect ids")
                    .push(effect_id.clone());
                let value = command.value;
                Box::pin(async move {
                    Ok(EffectAdapterOutcome::Settled(EffectEvidence {
                        effect_id,
                        value,
                        accepted: true,
                    }))
                })
            },
        )
        .expect("adapter");
    Runtime::new(builder.finish(), store)
}

fn retained_effect_reader(frames: Vec<Vec<u8>>, calls: Arc<AtomicUsize>) -> Runtime {
    effect_runtime_with_counting_adapter(
        Arc::new(RetainedStore(frames)),
        calls,
        Arc::new(std::sync::Mutex::new(Vec::new())),
    )
}

fn qualify_recorded_prefix(run_id: &RunId, frames: &[Vec<u8>]) -> JournalHistory {
    JournalHistory::qualify(
        run_id,
        StoredRunBytes::new(frames.to_vec()).expect("recorded Store prefix"),
    )
    .expect("qualified recorded prefix")
}

#[tokio::test]
async fn ambiguous_effect_appends_recover_from_exact_retained_facts() {
    // Expected retained heads and adapter counts are independent of the Store script.
    let cases = [
        ("prepare-absent", 2, AppendAction::Indeterminate, 1, 0, 1),
        (
            "prepare-retained",
            2,
            AppendAction::RetainThenIndeterminate,
            2,
            0,
            1,
        ),
        ("conclusion-absent", 3, AppendAction::Indeterminate, 2, 1, 2),
        (
            "conclusion-retained",
            3,
            AppendAction::RetainThenIndeterminate,
            3,
            1,
            1,
        ),
    ];
    for (
        offset,
        (
            name,
            sequence,
            action,
            expected_head_after_start,
            expected_calls_after_start,
            expected_calls_after_resume,
        ),
    ) in cases.into_iter().enumerate()
    {
        let calls = Arc::new(AtomicUsize::new(0));
        let effect_ids = Arc::new(std::sync::Mutex::new(Vec::new()));
        let store = Arc::new(ScriptedStore::new([(sequence, action)]));
        let runtime = effect_runtime_with_counting_adapter(
            store,
            Arc::clone(&calls),
            Arc::clone(&effect_ids),
        );
        let run_id = RunId::from_digest(DigestBytes::from_array(
            [u8::try_from(33 + offset).expect("RunId byte"); 32],
        ));
        let program = expand_program(
            EntryPointId::new(format!("mfm.test.runtime/ambiguous-{}@1", name))
                .expect("entry point"),
            &EffectProgram,
            &Number {
                value: u64::try_from(offset + 3).expect("input value"),
            },
            ProgramLimits::new(0),
        )
        .expect("Program");

        assert!(matches!(
            runtime
                .start(
                    run_id.clone(),
                    program,
                    Number {
                        value: u64::try_from(offset + 3).expect("input value"),
                    },
                )
                .await,
            Err(InvocationFailure::Execution {
                error: RuntimeError::Store(StoreError::Indeterminate),
                ..
            })
        ));
        assert_eq!(
            calls.load(Ordering::SeqCst),
            expected_calls_after_start,
            "{} adapter entries after start",
            name
        );
        let after_start = runtime.read(&run_id).await.expect("retained prefix");
        assert_eq!(
            after_start.head_sequence(),
            expected_head_after_start,
            "{} retained head after start",
            name
        );
        match expected_head_after_start {
            1 => assert!(matches!(after_start.state(), RunViewState::Runnable { .. })),
            2 => assert!(matches!(
                after_start.state(),
                RunViewState::EffectPending { .. }
            )),
            3 => assert!(matches!(after_start.state(), RunViewState::Succeeded(_))),
            _ => unreachable!("fixture head"),
        }

        let completed = runtime.resume(&run_id).await.expect("ambiguous recovery");
        assert_eq!(completed.head_sequence(), 3, "{} terminal head", name);
        assert!(
            matches!(completed.state(), RunViewState::Succeeded(_)),
            "{} terminal state",
            name
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            expected_calls_after_resume,
            "{} adapter entries after resume",
            name
        );
        let effect_ids = effect_ids.lock().expect("effect ids");
        assert!(effect_ids.windows(2).all(|pair| pair[0] == pair[1]));
    }
}

#[tokio::test]
async fn effect_not_inserted_returns_the_winner_without_entering_its_new_visit() {
    for (offset, sequence) in [2_u64, 3].into_iter().enumerate() {
        let calls = Arc::new(AtomicUsize::new(0));
        let store = Arc::new(ScriptedStore::new([(
            sequence,
            AppendAction::RetainThenNotInserted,
        )]));
        let runtime = effect_runtime_with_counting_adapter(
            store,
            Arc::clone(&calls),
            Arc::new(std::sync::Mutex::new(Vec::new())),
        );
        let run_id = RunId::from_digest(DigestBytes::from_array(
            [u8::try_from(38 + offset).expect("RunId byte"); 32],
        ));
        let completed = runtime
            .start(
                run_id.clone(),
                expand_program(
                    EntryPointId::new(format!("mfm.test.runtime/not-inserted-{sequence}@1"))
                        .expect("entry point"),
                    &EffectProgram,
                    &Number { value: 8 },
                    ProgramLimits::new(0),
                )
                .expect("Program"),
                Number { value: 8 },
            )
            .await
            .expect("converged Effect");
        assert_eq!(completed.head_sequence(), sequence);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            if sequence == 2 { 0 } else { 1 }
        );
        if sequence == 2 {
            assert!(matches!(
                completed.state(),
                RunViewState::EffectPending { .. }
            ));
        } else {
            assert!(matches!(completed.state(), RunViewState::Succeeded(_)));
        }
        let resumed = runtime.resume(&run_id).await.unwrap();
        assert_eq!(resumed.head_sequence(), 3);
        assert!(matches!(resumed.state(), RunViewState::Succeeded(_)));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn every_adapter_failure_leaves_one_pending_prepare() {
    #[derive(Clone, Copy)]
    enum FailureMode {
        Unavailable,
        Internal,
        Panic,
    }

    for (offset, mode) in [
        FailureMode::Unavailable,
        FailureMode::Internal,
        FailureMode::Panic,
    ]
    .into_iter()
    .enumerate()
    {
        let calls = Arc::new(AtomicUsize::new(0));
        let store = Arc::new(MemoryStore::new());
        let mut builder = RuntimeAssemblyBuilder::new().expect("builder");
        builder
            .register_effect::<Mutate, Mutation>()
            .expect("Effect State");
        builder
            .register_effect_adapter::<Mutation, _, _>(Binding { route: 8 }, {
                let calls = Arc::clone(&calls);
                move |_effect_id, _command_value_ref, _command| {
                    calls.fetch_add(1, Ordering::SeqCst);
                    match mode {
                        FailureMode::Unavailable => {
                            Box::pin(async { Err(AdapterError::Operational(NoContext)) })
                        }
                        FailureMode::Internal => {
                            Box::pin(async { Err(AdapterError::Invariant(AdapterInvariantError)) })
                        }
                        FailureMode::Panic => panic!("adapter panic"),
                    }
                }
            })
            .expect("adapter");
        let runtime = Runtime::new(builder.finish(), store);
        let run_id = RunId::from_digest(DigestBytes::from_array(
            [u8::try_from(40 + offset).expect("RunId byte"); 32],
        ));
        let result = runtime
            .start(
                run_id.clone(),
                expand_program(
                    EntryPointId::new(format!("mfm.test.runtime/adapter-failure-{offset}@1"))
                        .expect("entry point"),
                    &EffectProgram,
                    &Number { value: 9 },
                    ProgramLimits::new(0),
                )
                .expect("Program"),
                Number { value: 9 },
            )
            .await;
        match mode {
            FailureMode::Unavailable => assert!(matches!(
                result,
                Err(InvocationFailure::RecoveryStopped { .. })
            )),
            FailureMode::Internal | FailureMode::Panic => assert!(matches!(
                result,
                Err(InvocationFailure::Execution {
                    error: RuntimeError::Internal,
                    ..
                })
            )),
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let pending = runtime.read(&run_id).await.expect("pending view");
        assert_eq!(pending.head_sequence(), 2);
        assert!(matches!(
            pending.state(),
            RunViewState::EffectPending { .. }
        ));
    }
}

#[tokio::test]
async fn retained_effect_facts_are_validated_without_adapter_io() {
    let pending_store = Arc::new(ScriptedStore::recording());
    let pending_effect_ids = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut pending_builder = RuntimeAssemblyBuilder::new().expect("builder");
    pending_builder
        .register_effect::<Mutate, Mutation>()
        .expect("Effect State");
    pending_builder
        .register_effect_adapter::<Mutation, _, _>(Binding { route: 8 }, {
            let pending_effect_ids = Arc::clone(&pending_effect_ids);
            move |effect_id, _, _| {
                pending_effect_ids
                    .lock()
                    .expect("pending effect ids")
                    .push(effect_id.clone());
                Box::pin(async { Err(AdapterError::Operational(NoContext)) })
            }
        })
        .expect("adapter");
    let pending_runtime = Runtime::new(pending_builder.finish(), pending_store.clone());
    let run_id = RunId::from_digest(DigestBytes::from_array([36; 32]));
    assert!(matches!(
        pending_runtime
            .start(
                run_id.clone(),
                expand_program(
                    EntryPointId::new("mfm.test.runtime/retained-effect@1").expect("entry point"),
                    &EffectProgram,
                    &Number { value: 7 },
                    ProgramLimits::new(0),
                )
                .expect("Program"),
                Number { value: 7 },
            )
            .await,
        Err(InvocationFailure::RecoveryStopped { .. })
    ));
    let pending_frames = pending_store.snapshot();
    assert_eq!(pending_frames.len(), 2);
    let retained_effect_id = pending_effect_ids
        .lock()
        .expect("pending effect ids")
        .first()
        .expect("retained effect id")
        .clone();

    let read_calls = Arc::new(AtomicUsize::new(0));
    let read_runtime = retained_effect_reader(pending_frames.clone(), Arc::clone(&read_calls));
    let pending = read_runtime.read(&run_id).await.expect("pending view");
    assert_eq!(pending.head_sequence(), 2);
    assert!(matches!(
        pending.state(),
        RunViewState::EffectPending { .. }
    ));
    assert_eq!(read_calls.load(Ordering::SeqCst), 0);

    let genesis = qualify_recorded_prefix(&run_id, &pending_frames[..1]);
    let (command, command_ref) =
        canonicalize_mfm_value(&Command { value: 7 }).expect("retained command");
    let wrong_id = genesis
        .encode_effect_prepare(
            ExecutionPosition {
                state: StatePosition::new(0).unwrap(),
                visit: VisitId::new(0),
            },
            &EffectId::from_digest(DigestBytes::from_array([88; 32])),
            JournalObject::new(&command_ref, command.as_bytes()).unwrap(),
        )
        .expect("wrong-id prepare")
        .canonical_bytes()
        .to_vec();
    let wrong_id_frames = vec![pending_frames[0].clone(), wrong_id];

    let (replacement_command, replacement_command_ref) =
        canonicalize_mfm_value(&Command { value: 99 }).expect("replacement command");
    let wrong_command = genesis
        .encode_effect_prepare(
            ExecutionPosition {
                state: StatePosition::new(0).unwrap(),
                visit: VisitId::new(0),
            },
            &retained_effect_id,
            JournalObject::new(&replacement_command_ref, replacement_command.as_bytes()).unwrap(),
        )
        .expect("wrong-command prepare")
        .canonical_bytes()
        .to_vec();
    let wrong_command_frames = vec![pending_frames[0].clone(), wrong_command];

    let settled_store = Arc::new(ScriptedStore::recording());
    let settled_calls = Arc::new(AtomicUsize::new(0));
    let settled_effect_ids = Arc::new(std::sync::Mutex::new(Vec::new()));
    let settled_runtime = effect_runtime_with_counting_adapter(
        settled_store.clone(),
        Arc::clone(&settled_calls),
        Arc::clone(&settled_effect_ids),
    );
    let settled_run_id = RunId::from_digest(DigestBytes::from_array([37; 32]));
    settled_runtime
        .start(
            settled_run_id.clone(),
            expand_program(
                EntryPointId::new("mfm.test.runtime/retained-settlement@1").expect("entry point"),
                &EffectProgram,
                &Number { value: 7 },
                ProgramLimits::new(0),
            )
            .expect("Program"),
            Number { value: 7 },
        )
        .await
        .expect("settled Effect");
    assert_eq!(settled_calls.load(Ordering::SeqCst), 1);
    let settled_frames = settled_store.snapshot();
    assert_eq!(settled_frames.len(), 3);
    let settled_effect_id = settled_effect_ids
        .lock()
        .expect("settled effect ids")
        .first()
        .expect("settled effect id")
        .clone();

    let settled_read_calls = Arc::new(AtomicUsize::new(0));
    let settled_reader =
        retained_effect_reader(settled_frames.clone(), Arc::clone(&settled_read_calls));
    let settled = settled_reader
        .read(&settled_run_id)
        .await
        .expect("settled view");
    assert_eq!(settled.head_sequence(), 3);
    assert!(matches!(settled.state(), RunViewState::Succeeded(_)));
    assert_eq!(settled_read_calls.load(Ordering::SeqCst), 0);

    let prepared = qualify_recorded_prefix(&settled_run_id, &settled_frames[..2]);
    let (outcome, outcome_ref) =
        canonicalize_mfm_value(&Number { value: 7 }).expect("valid outcome");
    let (swapped_evidence, swapped_evidence_ref) = canonicalize_mfm_value(&EffectEvidence {
        effect_id: EffectId::from_digest(DigestBytes::from_array([89; 32])),
        value: 7,
        accepted: true,
    })
    .expect("swapped evidence");
    let swapped_evidence = prepared
        .encode_effect_conclusion(
            JournalObject::new(&swapped_evidence_ref, swapped_evidence.as_bytes()).unwrap(),
            EffectConclusion::Success {
                output: JournalObject::new(&outcome_ref, outcome.as_bytes()).unwrap(),
            },
        )
        .expect("swapped-evidence conclusion")
        .canonical_bytes()
        .to_vec();
    let swapped_evidence_frames = vec![
        settled_frames[0].clone(),
        settled_frames[1].clone(),
        swapped_evidence,
    ];

    let (valid_evidence, valid_evidence_ref) = canonicalize_mfm_value(&EffectEvidence {
        effect_id: settled_effect_id,
        value: 7,
        accepted: true,
    })
    .expect("valid evidence");
    let (wrong_outcome, wrong_outcome_ref) =
        canonicalize_mfm_value(&Command { value: 7 }).expect("wrong outcome type");
    let wrong_outcome = prepared
        .encode_effect_conclusion(
            JournalObject::new(&valid_evidence_ref, valid_evidence.as_bytes()).unwrap(),
            EffectConclusion::Success {
                output: JournalObject::new(&wrong_outcome_ref, wrong_outcome.as_bytes()).unwrap(),
            },
        )
        .expect("wrong-outcome conclusion")
        .canonical_bytes()
        .to_vec();
    let wrong_outcome_frames = vec![
        settled_frames[0].clone(),
        settled_frames[1].clone(),
        wrong_outcome,
    ];

    for (name, corrupt_run_id, frames) in [
        ("wrong prepare effect id", run_id.clone(), wrong_id_frames),
        ("replaced command", run_id, wrong_command_frames),
        (
            "swapped evidence effect id",
            settled_run_id.clone(),
            swapped_evidence_frames,
        ),
        (
            "wrong conclusion outcome schema",
            settled_run_id,
            wrong_outcome_frames,
        ),
    ] {
        let calls = Arc::new(AtomicUsize::new(0));
        let runtime = retained_effect_reader(frames, Arc::clone(&calls));
        assert!(
            matches!(
                runtime.read(&corrupt_run_id).await,
                Err(InvocationFailure::Execution {
                    error: RuntimeError::InvalidHistory,
                    ..
                }),
            ),
            "{name}"
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "{name} must not enter the adapter"
        );
    }
}

#[tokio::test]
async fn concurrent_pending_effect_callers_converge_on_one_conclusion() {
    let store = Arc::new(MemoryStore::new());
    let mut unavailable_builder = RuntimeAssemblyBuilder::new().expect("builder");
    unavailable_builder
        .register_effect::<Mutate, Mutation>()
        .expect("Effect State");
    unavailable_builder
        .register_effect_adapter::<Mutation, _, _>(Binding { route: 8 }, |_, _, _| {
            Box::pin(async { Err(AdapterError::Operational(NoContext)) })
        })
        .expect("adapter");
    let unavailable = Runtime::new(unavailable_builder.finish(), store.clone());
    let run_id = RunId::from_digest(DigestBytes::from_array([35; 32]));
    assert!(matches!(
        unavailable
            .start(
                run_id.clone(),
                expand_program(
                    EntryPointId::new("mfm.test.runtime/concurrent-effect@1").expect("entry point"),
                    &EffectProgram,
                    &Number { value: 5 },
                    ProgramLimits::new(0),
                )
                .expect("Program"),
                Number { value: 5 },
            )
            .await,
        Err(InvocationFailure::RecoveryStopped { .. })
    ));

    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let ids = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut builder = RuntimeAssemblyBuilder::new().expect("builder");
    builder
        .register_effect::<Mutate, Mutation>()
        .expect("Effect State");
    builder
        .register_effect_adapter::<Mutation, _, _>(Binding { route: 8 }, {
            let barrier = Arc::clone(&barrier);
            let ids = Arc::clone(&ids);
            move |effect_id, _command_value_ref, command| {
                let effect_id = effect_id.clone();
                let value = command.value;
                ids.lock().expect("ids").push(effect_id.clone());
                let barrier = Arc::clone(&barrier);
                Box::pin(async move {
                    barrier.wait().await;
                    Ok(EffectAdapterOutcome::Settled(EffectEvidence {
                        effect_id,
                        value,
                        accepted: true,
                    }))
                })
            }
        })
        .expect("adapter");
    let runtime = Arc::new(Runtime::new(builder.finish(), store));
    let left = {
        let runtime = Arc::clone(&runtime);
        let run_id = run_id.clone();
        tokio::spawn(async move { runtime.resume(&run_id).await })
    };
    let right = {
        let runtime = Arc::clone(&runtime);
        let run_id = run_id.clone();
        tokio::spawn(async move { runtime.resume(&run_id).await })
    };
    let left = left.await.expect("left task").expect("left resume");
    let right = right.await.expect("right task").expect("right resume");
    assert_eq!(left.head_sequence(), 3);
    assert_eq!(right.head_digest(), left.head_digest());
    let ids = ids.lock().expect("ids");
    assert_eq!(ids.len(), 2);
    assert_eq!(ids[0], ids[1]);
}

struct FaultStore {
    failure: StoreError,
}

impl Store for FaultStore {
    fn load_run<'a>(
        &'a self,
        _run_id: &'a RunId,
    ) -> Pin<
        Box<
            dyn Future<Output = std::result::Result<Option<StoredRunBytes>, StoreError>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move { Err(self.failure) })
    }

    fn append_run<'a>(
        &'a self,
        _frame: &'a EncodedRunFrame,
    ) -> Pin<Box<dyn Future<Output = Result<AppendResult, StoreError>> + Send + 'a>> {
        Box::pin(async move { Err(self.failure) })
    }
}

#[tokio::test]
async fn store_failures_preserve_mechanical_source_and_unknown_observation() {
    for (offset, failure) in [
        StoreError::Capacity,
        StoreError::CorruptPhysicalState,
        StoreError::Unavailable,
        StoreError::Indeterminate,
    ]
    .into_iter()
    .enumerate()
    {
        let mut builder = RuntimeAssemblyBuilder::new().expect("builder");
        builder.register_value::<Number>().expect("root value");
        let runtime = Runtime::new(builder.finish(), Arc::new(FaultStore { failure }));
        let run_id = RunId::from_digest(DigestBytes::from_array(
            [u8::try_from(offset + 10).expect("RunId byte"); 32],
        ));
        assert!(matches!(
            runtime.read(&run_id).await,
            Err(InvocationFailure::Execution { error: RuntimeError::Store(source), last_observed: None, .. }) if source == failure
        ));
        let program = expand_program(
            EntryPointId::new("mfm.test.runtime/fault@1").expect("entry point"),
            &EmptyProgram,
            &Number { value: 1 },
            ProgramLimits::new(0),
        )
        .expect("Program");
        assert!(matches!(
            runtime.start(run_id, program, Number { value: 1 }).await,
            Err(InvocationFailure::Execution { error: RuntimeError::Store(source), last_observed: None, .. }) if source == failure
        ));
    }
}

#[path = "support/injection.rs"]
mod injection;

#[path = "support/callback_errors.rs"]
mod callback_errors;
