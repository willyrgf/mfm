use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use mfm_capabilities::{
    AdapterError, CapabilityError, EffectCapabilityContract, ReadCapabilityContract,
};
use mfm_ids::{ContentRef, DigestBytes, EffectId, EntryPointId, RunId, SemanticTypeId, StableId};
use mfm_journal::{decode_frame, seal_frame, EncodedRunFrame};
use mfm_program::{
    expand_program, CapabilityInjection, EffectState, FromNever, Identity, Never, NoParams,
    Occurrence, Operation, OperationExpansion, ProgramError, ProgramLimits, ProposedStateOutcome,
    PureState, ReadState, State,
};
use mfm_program_derive::MfmValue;
use mfm_runtime::{
    EffectAdapterOutcome, InvocationFailure, RunViewState, Runtime, RuntimeAssemblyBuilder,
    RuntimeError,
};
use mfm_store::{AppendResult, LoadedRun, MemoryStore, RunSummary, Store, StoreError};
use mfm_values::{
    canonicalize_mfm_value, MfmValue as MfmValueTrait, NativeCause, SchemaAudit, SchemaDescriptor,
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
    ) -> std::result::Result<ProposedStateOutcome<Self::Output, Self::Failure>, NativeCause> {
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
    ) -> std::result::Result<ProposedStateOutcome<Self::Output, Self::Failure>, NativeCause> {
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
        body.pure::<GenericState<K>, Identity<Never>>(NoParams, Occurrence::new())
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
    ) -> std::result::Result<ProposedStateOutcome<Self::Output, Self::Failure>, NativeCause> {
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
        body.pure::<Increment, Identity<Never>>(NoParams, Occurrence::new())
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
    type OperationalError = OperationalFailure;
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
    ) -> Result<(), NativeCause> {
        let expected_value_ref = canonicalize_mfm_value(intent)
            .map(|(_, value_ref)| value_ref)
            .map_err(NativeCause::from_error)?;
        (intent_value_ref == &expected_value_ref
            && intent_value_ref == &evidence.intent_value_ref
            && intent.value == evidence.value)
            .then_some(())
            .ok_or_else(|| NativeCause::from_error(CapabilityError::EvidenceBinding))
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
    fn prepare(input: &Self::Input) -> Result<Intent, NativeCause> {
        Ok(Intent { value: input.value })
    }

    fn interpret(
        input: Self::Input,
        evidence: &Evidence,
    ) -> std::result::Result<ProposedStateOutcome<Self::Output, Self::Failure>, NativeCause> {
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
    type OperationalError = OperationalFailure;
    type Command = Command;
    type Evidence = EffectEvidence;

    fn contract_id() -> mfm_capabilities::Result<StableId> {
        StableId::new("mfm.test.runtime/mutation@1").map_err(|_| CapabilityError::InvalidContract)
    }

    fn bind_evidence(
        effect_id: &EffectId,
        command: &Self::Command,
        evidence: &Self::Evidence,
    ) -> Result<(), NativeCause> {
        (effect_id == &evidence.effect_id && command.value == evidence.value)
            .then_some(())
            .ok_or_else(|| NativeCause::from_error(CapabilityError::EvidenceBinding))
    }
}

struct ConflictingReadCapability;

impl ReadCapabilityContract for ConflictingReadCapability {
    type OperationalError = OperationalFailure;
    type Intent = Command;
    type Evidence = EffectEvidence;

    fn contract_id() -> mfm_capabilities::Result<StableId> {
        Mutation::contract_id()
    }

    fn bind_evidence(
        _intent_value_ref: &ContentRef,
        intent: &Self::Intent,
        evidence: &Self::Evidence,
    ) -> Result<(), NativeCause> {
        (intent.value == evidence.value)
            .then_some(())
            .ok_or_else(|| NativeCause::from_error(CapabilityError::EvidenceBinding))
    }
}

struct Mutate;

const PREPARATION_FAILURE_SENTINEL: u64 = u64::MAX;

#[derive(Debug, Serialize, thiserror::Error)]
#[error("test command preparation rejected input")]
struct PreparationRejected {
    input: u64,
}

#[derive(Debug, Serialize, thiserror::Error)]
#[error("test adapter rejected invocation")]
struct AdapterRejected;

impl State for Mutate {
    type Input = Number;
    type Output = Number;
    type Failure = Number;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.runtime/mutate@1").map_err(|_| ProgramError::InvalidContract)
    }
}

impl EffectState<Mutation> for Mutate {
    fn prepare(input: &Self::Input) -> Result<Command, NativeCause> {
        if input.value == PREPARATION_FAILURE_SENTINEL {
            return Err(NativeCause::from_error(PreparationRejected {
                input: input.value,
            }));
        }
        Ok(Command { value: input.value })
    }

    fn interpret(
        input: Self::Input,
        evidence: &EffectEvidence,
    ) -> std::result::Result<ProposedStateOutcome<Self::Output, Self::Failure>, NativeCause> {
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
        )
    }
}

#[test]
fn exact_value_and_state_abi_collisions_are_rejected() {
    let mut different_type = RuntimeAssemblyBuilder::new().expect("builder");
    different_type
        .register_value::<Number>()
        .expect("number codec");
    assert!(matches!(
        different_type.register_value::<NumberAlias>(),
        Err(RuntimeError::IncompatibleAssembly)
    ));
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
    assert!(matches!(
        different_descriptor.register_value::<MutableDescriptorNumber>(),
        Err(RuntimeError::IncompatibleAssembly)
    ));
    ALTERNATE_DESCRIPTOR_AUDIT.store(false, Ordering::SeqCst);

    let mut different_state_type = RuntimeAssemblyBuilder::new().expect("builder");
    different_state_type
        .register_pure::<GenericState<FirstGenericValue>>()
        .expect("generic state");
    assert!(matches!(
        different_state_type.register_pure::<ConflictingGenericState<FirstGenericValue>>(),
        Err(RuntimeError::IncompatibleAssembly)
    ));
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
    assert!(matches!(
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
    ));
    builder
        .register_adapter::<ConflictingReadCapability, _, _>(Binding { route: 8 }, |_, _| {
            Box::pin(async {
                Err(AdapterError::Invariant(NativeCause::from_error(
                    AdapterRejected,
                )))
            })
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
async fn pure_and_zero_state_programs_restore_without_reserving_future_recovery_capacity() {
    let store = Arc::new(MemoryStore::new());
    let mut builder = RuntimeAssemblyBuilder::new().expect("builder");
    builder.register_pure::<Increment>().expect("Pure State");
    let runtime = Runtime::new(builder.finish(), store);
    let run_id = RunId::from_digest(DigestBytes::from_array([1; 32]));
    let program = expand_program(
        EntryPointId::new("mfm.test.runtime/pure@1").expect("entry point"),
        &PureProgram,
        &Number { value: 4 },
        ProgramLimits::new(u32::MAX),
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
    assert_eq!(hot.admitted_context().decode::<Number>().unwrap().value, 4);
    assert_eq!(hot_value.canonical_bytes(), br#"{"value":5}"#);
    assert_eq!(hot_value.decode::<Number>().unwrap().value, 5);
    let mismatch = hot_value.decode::<FirstGenericValue>().err().unwrap();
    assert!(matches!(
        mismatch.downcast_ref::<mfm_values::ValueError>(),
        Some(mfm_values::ValueError::InvalidSchemaIdentity)
    ));

    let cold = runtime.read(&run_id).await.expect("cold read");
    let RunViewState::Succeeded(cold_value) = cold.state() else {
        panic!("cold Program did not succeed");
    };
    assert_eq!(cold.head_sequence(), hot.head_sequence());
    assert_eq!(
        cold.admitted_context().value_ref(),
        hot.admitted_context().value_ref()
    );
    assert_eq!(
        cold.admitted_context().canonical_bytes(),
        hot.admitted_context().canonical_bytes()
    );
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
async fn read_success_and_separate_failure_recovery_are_restorable_without_repeating_io() {
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
            Box::pin(async {
                Err(AdapterError::Invariant(NativeCause::from_error(
                    AdapterRejected,
                )))
            })
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
            error: RuntimeError::Native {
                operation: mfm_runtime::Operation::ReadAdapter,
                stage: mfm_runtime::Stage::Execute,
                cause,
            },
            ..
        }) if cause.downcast_ref::<AdapterRejected>().is_some()
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
    assert_eq!(resumed.head_sequence(), 3);
    let mfm_runtime::Failure::Domain { original, .. } = value.failure() else {
        panic!("domain result")
    };
    let root = value.root().unwrap();
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
    assert_eq!(resumed.head_sequence(), 4);
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
    assert_eq!(settled.head_sequence(), 4);
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
            error: RuntimeError::Native {
                operation: mfm_runtime::Operation::EffectPrepare,
                stage: mfm_runtime::Stage::Execute,
                cause,
            },
            ..
        }) if cause.downcast_ref::<PreparationRejected>().is_some_and(|error| error.input == PREPARATION_FAILURE_SENTINEL)
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
            error: RuntimeError::Native {
                operation: mfm_runtime::Operation::EffectBind,
                stage: mfm_runtime::Stage::Execute,
                cause,
            },
            ..
        }) if matches!(cause.downcast_ref::<CapabilityError>(), Some(CapabilityError::EvidenceBinding))
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

struct RetainedStore(Vec<Vec<u8>>);

impl Store for RetainedStore {
    fn load_run<'a>(
        &'a self,
        run_id: &'a RunId,
        probe_sequence: Option<u64>,
    ) -> Pin<Box<dyn Future<Output = Result<Option<LoadedRun>, StoreError>> + Send + 'a>> {
        Box::pin(async move {
            let Some(latest) = self.0.last() else {
                return Ok(None);
            };
            let frame = decode_frame(latest).map_err(|_| StoreError::CorruptPhysicalState)?;
            let head = RunSummary::new(
                run_id.clone(),
                frame.run_sequence(),
                frame.head_digest().clone(),
                self.0.iter().map(|bytes| bytes.len() as u64).sum(),
            )
            .map_err(|_| StoreError::CorruptPhysicalState)?;
            let probe = probe_sequence
                .and_then(|sequence| self.0.get(sequence.checked_sub(1)? as usize))
                .map(|bytes| Arc::from(bytes.as_slice()));
            LoadedRun::new(
                head,
                Arc::from(self.0[0].as_slice()),
                Arc::from(latest.as_slice()),
                probe,
            )
            .map(Some)
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
        ("settlement-absent", 3, AppendAction::Indeterminate, 2, 1, 2),
        (
            "settlement-retained",
            3,
            AppendAction::RetainThenIndeterminate,
            3,
            1,
            1,
        ),
        (
            "interpretation-absent",
            4,
            AppendAction::Indeterminate,
            3,
            1,
            1,
        ),
        (
            "interpretation-retained",
            4,
            AppendAction::RetainThenIndeterminate,
            4,
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

        assert_store_recording(
            runtime
                .start(
                    run_id.clone(),
                    program,
                    Number {
                        value: u64::try_from(offset + 3).expect("input value"),
                    },
                )
                .await
                .err()
                .unwrap(),
            StoreError::Indeterminate,
            sequence,
        );
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
            3 => assert!(matches!(
                after_start.state(),
                RunViewState::AwaitingInterpretation { .. }
            )),
            4 => assert!(matches!(after_start.state(), RunViewState::Succeeded(_))),
            _ => unreachable!("fixture head"),
        }

        let completed = runtime.resume(&run_id).await.expect("ambiguous recovery");
        assert_eq!(completed.head_sequence(), 4, "{} terminal head", name);
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
    for (offset, sequence) in [2_u64, 3, 4].into_iter().enumerate() {
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
        } else if sequence == 3 {
            assert!(matches!(
                completed.state(),
                RunViewState::AwaitingInterpretation { .. }
            ));
        } else {
            assert!(matches!(completed.state(), RunViewState::Succeeded(_)));
        }
        let resumed = runtime.resume(&run_id).await.unwrap();
        assert_eq!(resumed.head_sequence(), 4);
        assert!(matches!(resumed.state(), RunViewState::Succeeded(_)));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn operational_failures_are_audited_while_internal_failures_preserve_prepare() {
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
                        FailureMode::Unavailable => Box::pin(async {
                            Err(AdapterError::Operational(OperationalFailure::Unavailable))
                        }),
                        FailureMode::Internal => Box::pin(async {
                            Err(AdapterError::Invariant(NativeCause::from_error(
                                AdapterRejected,
                            )))
                        }),
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
            FailureMode::Internal | FailureMode::Panic => {
                let Err(InvocationFailure::Execution {
                    error:
                        RuntimeError::Native {
                            operation: mfm_runtime::Operation::EffectAdapter,
                            stage: mfm_runtime::Stage::Execute,
                            cause,
                        },
                    ..
                }) = result
                else {
                    panic!("adapter failure context")
                };
                match mode {
                    FailureMode::Internal => {
                        assert!(cause.downcast_ref::<AdapterRejected>().is_some())
                    }
                    FailureMode::Panic => assert!(matches!(
                        cause.downcast_ref::<mfm_runtime::TaskFailure>(),
                        Some(mfm_runtime::TaskFailure::Panicked)
                    )),
                    FailureMode::Unavailable => unreachable!(),
                }
            }
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let pending = runtime.read(&run_id).await.expect("pending view");
        assert_eq!(
            pending.head_sequence(),
            if matches!(mode, FailureMode::Unavailable) {
                4
            } else {
                2
            }
        );
        assert!(matches!(
            pending.state(),
            RunViewState::EffectPending { .. }
        ));
    }
}

#[tokio::test]
async fn retained_effect_facts_are_validated_without_adapter_io() {
    for settled in [false, true] {
        let actions = if settled {
            vec![]
        } else {
            vec![(2, AppendAction::RetainThenNotInserted)]
        };
        let store = Arc::new(if settled {
            ScriptedStore::recording()
        } else {
            ScriptedStore::new(actions)
        });
        let runtime = effect_runtime_with_counting_adapter(
            store.clone(),
            Arc::new(AtomicUsize::new(0)),
            Arc::new(std::sync::Mutex::new(Vec::new())),
        );
        let run_id =
            RunId::from_digest(DigestBytes::from_array([if settled { 37 } else { 36 }; 32]));
        let program = expand_program(
            EntryPointId::new("mfm.test.runtime/retained-effect@1").unwrap(),
            &EffectProgram,
            &Number { value: 7 },
            ProgramLimits::new(0),
        )
        .unwrap();
        let hot = runtime
            .start(run_id.clone(), program, Number { value: 7 })
            .await
            .unwrap();
        let frames = store.snapshot();
        let calls = Arc::new(AtomicUsize::new(0));
        let reader = retained_effect_reader(frames.clone(), calls.clone());
        assert_eq!(
            reader.read(&run_id).await.unwrap().head_digest(),
            hot.head_digest()
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let latest = decode_frame(frames.last().unwrap()).unwrap();
        let original: serde_json::Value =
            serde_json::from_slice(latest.payload().as_bytes()).unwrap();
        for mutation in 0..if settled { 1 } else { 2 } {
            let mut payload = original.clone();
            if settled {
                let wrong_output = serde_json::to_value(
                    mfm_values::Object::from_value(&Command { value: 7 }).unwrap(),
                )
                .unwrap();
                payload["operation"]["succeeded"]["output"] = wrong_output;
            } else {
                let (name, replacement) = if mutation == 0 {
                    (
                        "effect_id",
                        serde_json::to_value(EffectId::from_digest(DigestBytes::from_array(
                            [88; 32],
                        )))
                        .unwrap(),
                    )
                } else {
                    (
                        "command",
                        serde_json::to_value(
                            mfm_values::Object::from_value(&Command { value: 99 }).unwrap(),
                        )
                        .unwrap(),
                    )
                };
                payload["operation"]["effect_prepared"][name] = replacement;
            }
            let payload = mfm_canonical::PlainCanonicalJsonBytes::from_json_str(
                &serde_json::to_string(&payload).unwrap(),
            )
            .unwrap();
            let changed = seal_frame(
                &run_id,
                latest.run_sequence(),
                latest.previous_head_digest(),
                &payload,
            )
            .unwrap();
            let mut changed_frames = frames.clone();
            *changed_frames.last_mut().unwrap() = changed.canonical_bytes().to_vec();
            let calls = Arc::new(AtomicUsize::new(0));
            let reader = retained_effect_reader(changed_frames, calls.clone());
            assert!(
                reader.read(&run_id).await.is_err(),
                "settled={settled}, mutation={mutation}"
            );
            assert_eq!(calls.load(Ordering::SeqCst), 0);
        }
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
            Box::pin(async { Err(AdapterError::Operational(OperationalFailure::Unavailable)) })
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
    for view in [&left, &right] {
        assert!(matches!(view.head_sequence(), 5 | 6));
        if view.head_sequence() == 5 {
            assert!(matches!(
                view.state(),
                RunViewState::AwaitingInterpretation { .. }
            ));
        } else {
            assert!(matches!(view.state(), RunViewState::Succeeded(_)));
        }
    }
    assert!(left.head_sequence() == 6 || right.head_sequence() == 6);
    assert_eq!(runtime.read(&run_id).await.unwrap().head_sequence(), 6);
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
        _probe_sequence: Option<u64>,
    ) -> Pin<Box<dyn Future<Output = std::result::Result<Option<LoadedRun>, StoreError>> + Send + 'a>>
    {
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
        StoreError::ArithmeticOverflow,
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
        assert_store_recording(
            runtime
                .start(run_id, program, Number { value: 1 })
                .await
                .err()
                .unwrap(),
            failure,
            1,
        );
    }
}

#[path = "support/injection.rs"]
mod injection;

#[path = "support/callback_errors.rs"]
mod callback_errors;

#[path = "support/scripted_store.rs"]
mod scripted_store;
use scripted_store::{AppendAction, ScriptedStore};

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
enum OperationalFailure {
    Unavailable,
}
impl mfm_program::ClassifyError for OperationalFailure {
    fn classify(&self) -> mfm_program::Classification {
        mfm_program::Classification::Permanent
    }
}
impl mfm_program::ClassifyError for Number {
    fn classify(&self) -> mfm_program::Classification {
        mfm_program::Classification::Permanent
    }
}

fn assert_store_recording(failure: InvocationFailure, expected: StoreError, sequence: u64) {
    let InvocationFailure::Execution {
        error: RuntimeError::Recording { failure, .. },
        ..
    } = failure
    else {
        panic!("recording custody")
    };
    let mfm_runtime::RecordingFailure::Append {
        original: None,
        candidate,
        outcome: mfm_runtime::AppendFailure::Store(source),
        observation: None,
        reload_cause: None,
    } = failure.as_ref()
    else {
        panic!("Store outcome without speculative probe")
    };
    assert_eq!(source, &expected);
    assert_eq!(candidate.run_sequence(), sequence);
}
