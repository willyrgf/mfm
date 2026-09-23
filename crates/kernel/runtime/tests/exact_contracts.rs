//! Exact codec and State ownership is qualified during construction, before Runtime admission.
use mfm_ids::{DigestBytes, EntryPointId, RunId, SemanticTypeId, StableId};
use mfm_program::{
    compile, load, Identity, Never, ProgramEnvironment, ProgramError, ProgramLimits,
    ProposedStateOutcome, Pure, PureState, State,
};
use mfm_program_derive::MfmValue;
use mfm_runtime::{RunViewState, Runtime};
use mfm_store::MemoryStore;
use mfm_values::{InvocationDiagnostic, MfmValue as MfmValueTrait, SchemaAudit, SchemaDescriptor};
use serde::{Deserialize, Serialize};
use std::{
    marker::PhantomData,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
};
struct Resources<S>(PhantomData<fn() -> S>);
impl<S> ProgramEnvironment for Resources<S> {
    type Sources = S;
}
impl<S> Default for Resources<S> {
    fn default() -> Self {
        Self(PhantomData)
    }
}
static DESCRIPTOR_CALL: AtomicUsize = AtomicUsize::new(0);
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
            if DESCRIPTOR_CALL
                .fetch_add(1, Ordering::SeqCst)
                .is_multiple_of(2)
            {
                "MutableDescriptorNumber::alternate"
            } else {
                "MutableDescriptorNumber::changed-again"
            }
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
    ) -> std::result::Result<ProposedStateOutcome<Self::Output, Self::Failure>, InvocationDiagnostic>
    {
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
    ) -> std::result::Result<ProposedStateOutcome<Self::Output, Self::Failure>, InvocationDiagnostic>
    {
        Ok(ProposedStateOutcome::Success { output: input })
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
    ) -> std::result::Result<ProposedStateOutcome<Self::Output, Self::Failure>, InvocationDiagnostic>
    {
        Ok(ProposedStateOutcome::Success {
            output: Number {
                value: input.value + 1,
            },
        })
    }
}

struct AliasState;
impl State for AliasState {
    type Input = Number;
    type Output = NumberAlias;
    type Failure = Never;
    fn state_id() -> mfm_program::Result<StableId> {
        Ok(StableId::new("mfm.test.runtime/alias@1")?)
    }
}
impl PureState for AliasState {
    fn evaluate(
        _: Number,
    ) -> Result<ProposedStateOutcome<NumberAlias, Never>, InvocationDiagnostic> {
        panic!("construction must reject ambiguous codecs before evaluation")
    }
}

#[test]
fn exact_value_and_state_abi_collisions_are_rejected_during_construction() {
    let input = Number { value: 4 };
    let entry = EntryPointId::new("mfm.test.runtime/ownership@1").unwrap();
    let program = compile(
        entry.clone(),
        &Identity::<Number>::default(),
        &input,
        &Resources::<Identity<Number>>::default(),
        ProgramLimits::new(0),
    )
    .unwrap();
    let incompatible = Resources::<(Identity<Number>, Identity<NumberAlias>)>::default();
    let error = compile(
        entry.clone(),
        &Pure::<AliasState>::default(),
        &input,
        &incompatible,
        ProgramLimits::new(0),
    )
    .err()
    .unwrap();
    let diagnostic = serde_json::to_string(&error).unwrap();
    assert!(diagnostic.contains("claim_value"), "{diagnostic}");
    assert!(diagnostic.contains("conflicting_owner"), "{diagnostic}");
    assert!(load(program.canonical_bytes(), &incompatible).is_err());
    // A failed construction cannot poison another inventory.
    assert!(load(
        program.canonical_bytes(),
        &Resources::<Identity<Number>>::default()
    )
    .is_ok());

    let mutable = Resources::<Identity<MutableDescriptorNumber>>::default();
    ALTERNATE_DESCRIPTOR_AUDIT.store(false, Ordering::SeqCst);
    compile(
        entry.clone(),
        &Identity::<MutableDescriptorNumber>::default(),
        &MutableDescriptorNumber { value: 1 },
        &mutable,
        ProgramLimits::new(0),
    )
    .unwrap();
    ALTERNATE_DESCRIPTOR_AUDIT.store(true, Ordering::SeqCst);
    let error = compile(
        entry.clone(),
        &Identity::<MutableDescriptorNumber>::default(),
        &MutableDescriptorNumber { value: 1 },
        &mutable,
        ProgramLimits::new(0),
    )
    .err()
    .unwrap();
    ALTERNATE_DESCRIPTOR_AUDIT.store(false, Ordering::SeqCst);
    let diagnostic = serde_json::to_string(&error).unwrap();
    assert!(diagnostic.contains("select_value"), "{diagnostic}");

    let incompatible = Resources::<(
        Pure<GenericState<FirstGenericValue>>,
        Pure<ConflictingGenericState<FirstGenericValue>>,
    )>::default();
    let error = compile(
        entry,
        &(
            Pure::<GenericState<FirstGenericValue>>::default(),
            Pure::<ConflictingGenericState<FirstGenericValue>>::default(),
        ),
        &GenericStateValue {
            value: FirstGenericValue { first: 1 },
        },
        &incompatible,
        ProgramLimits::new(0),
    )
    .err()
    .unwrap();
    let diagnostic = serde_json::to_string(&error).unwrap();
    assert!(diagnostic.contains("claim_state"), "{diagnostic}");
}

#[tokio::test]
async fn one_semantic_family_executes_multiple_exact_schemas_hot_and_cold() {
    assert_eq!(
        GenericStateValue::<FirstGenericValue>::semantic_id().unwrap(),
        GenericStateValue::<SecondGenericValue>::semantic_id().unwrap()
    );
    assert_ne!(
        mfm_program::nominal_contract_ref::<GenericStateValue<FirstGenericValue>>().unwrap(),
        mfm_program::nominal_contract_ref::<GenericStateValue<SecondGenericValue>>().unwrap()
    );
    let installed = Resources::<(
        Pure<GenericState<FirstGenericValue>>,
        Pure<GenericState<SecondGenericValue>>,
    )>::default();
    let store = Arc::new(MemoryStore::new());
    let runtime = Runtime::new(store.clone());
    let first = GenericStateValue {
        value: FirstGenericValue { first: 11 },
    };
    let second = GenericStateValue {
        value: SecondGenericValue {
            second: "two".into(),
        },
    };
    let first_program = compile(
        EntryPointId::new("mfm.test.runtime/generic-first@1").unwrap(),
        &Pure::<GenericState<FirstGenericValue>>::default(),
        &first,
        &installed,
        ProgramLimits::new(0),
    )
    .unwrap();
    let second_program = compile(
        EntryPointId::new("mfm.test.runtime/generic-second@1").unwrap(),
        &Pure::<GenericState<SecondGenericValue>>::default(),
        &second,
        &installed,
        ProgramLimits::new(0),
    )
    .unwrap();
    let first_run = RunId::from_digest(DigestBytes::from_array([60; 32]));
    let second_run = RunId::from_digest(DigestBytes::from_array([61; 32]));
    let first_hot = runtime
        .start(first_run.clone(), &first_program, &first)
        .await
        .unwrap();
    let second_hot = runtime
        .start(second_run.clone(), &second_program, &second)
        .await
        .unwrap();
    assert_eq!(
        first_hot.success().unwrap().canonical_bytes(),
        br#"{"value":{"first":11}}"#
    );
    assert_eq!(
        second_hot.success().unwrap().canonical_bytes(),
        br#"{"value":{"second":"two"}}"#
    );
    drop(first_program);
    drop(second_program);
    drop(runtime);
    let cold = Runtime::new(store);
    for (run, hot) in [(first_run, first_hot), (second_run, second_hot)] {
        let document = cold.program_document(&run).await.unwrap();
        let program = load(document.canonical_bytes(), &installed).unwrap();
        let view = cold.read(&run, &program).await.unwrap();
        assert_eq!(view.head_digest(), hot.head_digest());
        assert_eq!(view.success().unwrap(), hot.success().unwrap());
    }
}

// Unused recovery allowances must not prevent ordinary completion; restored inputs and outputs
// must retain their exact identities.
#[tokio::test]
async fn pure_and_zero_state_programs_restore_without_reserving_future_recovery_capacity() {
    let store = Arc::new(MemoryStore::new());
    let runtime = Runtime::new(store);
    let installed = Resources::<Pure<Increment>>::default();
    let run_id = RunId::from_digest(DigestBytes::from_array([1; 32]));
    let program = compile(
        EntryPointId::new("mfm.test.runtime/pure@1").expect("entry point"),
        &Pure::<Increment>::default(),
        &Number { value: 4 },
        &installed,
        ProgramLimits::new(u32::MAX),
    )
    .expect("Program");

    let hot = runtime
        .start(run_id.clone(), &program, &Number { value: 4 })
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
    assert_eq!(mismatch.code(), "value_error");
    assert_eq!(
        mismatch.details().as_value(),
        &serde_json::json!("invalid_schema_identity")
    );

    let document = runtime.program_document(&run_id).await.unwrap();
    drop(program);
    let program = load(document.canonical_bytes(), &installed).unwrap();
    let cold = runtime.read(&run_id, &program).await.expect("cold read");
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

    let empty = Runtime::new(Arc::new(MemoryStore::new()));
    let empty_program = compile(
        EntryPointId::new("mfm.test.runtime/empty@1").expect("entry point"),
        &Identity::<Number>::default(),
        &Number { value: 9 },
        &Resources::<Identity<Number>>::default(),
        ProgramLimits::new(0),
    )
    .expect("empty Program");
    let empty_view = empty
        .start(
            RunId::from_digest(DigestBytes::from_array([2; 32])),
            &empty_program,
            &Number { value: 9 },
        )
        .await
        .expect("empty start");
    let RunViewState::Succeeded(value) = empty_view.state() else {
        panic!("zero-State Program did not succeed");
    };
    assert_eq!(empty_view.head_sequence(), 1);
    assert_eq!(value.canonical_bytes(), br#"{"value":9}"#);
}
