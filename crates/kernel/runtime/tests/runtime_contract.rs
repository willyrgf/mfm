use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use mfm_capabilities::{CapabilityError, ReadCapabilityContract};
use mfm_ids::{DigestBytes, EntryPointId, RunId, StableId};
use mfm_journal::{EncodedRunFrame, JournalHistory, OutcomeKind, StoredRunBytes};
use mfm_program::{
    capability_contract_ref, expand_program, nominal_contract_ref, state_implementation_ref,
    CapabilityInjection, Never, Operation, OperationExpansion, Program, ProgramError,
    ProposedStateOutcome, PureState, ReadPreparationError, ReadState, State,
};
use mfm_program_derive::MfmValue;
use mfm_runtime::{ReadAdapterError, RunViewState, Runtime, RuntimeAssemblyBuilder, RuntimeError};
use mfm_store::{AppendResult, MemoryStore, Store, StoreError};
use mfm_values::canonicalize_mfm_value;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct Value {
    value: u64,
}

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct OtherValue {
    value: String,
}

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct RecoveryPayload {
    value: u64,
}

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
enum RecoverySelector {
    Recover(RecoveryPayload),
    Terminal(RecoveryPayload),
}

struct Increment;

impl State for Increment {
    type Input = Value;
    type Output = Value;
    type Failure = Never;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.runtime/increment@1").map_err(|_| ProgramError::InvalidContract)
    }
}

impl PureState for Increment {
    fn evaluate(input: Self::Input) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        ProposedStateOutcome::Success {
            output: Value {
                value: input.value + 1,
            },
        }
    }
}

static PURE_RETRY_EVALUATIONS: AtomicUsize = AtomicUsize::new(0);

struct CountingRetryIncrement;

impl State for CountingRetryIncrement {
    type Input = Value;
    type Output = Value;
    type Failure = Never;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.runtime/counting-retry-increment@1")
            .map_err(|_| ProgramError::InvalidContract)
    }
}

impl PureState for CountingRetryIncrement {
    fn evaluate(input: Self::Input) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        PURE_RETRY_EVALUATIONS.fetch_add(1, Ordering::SeqCst);
        ProposedStateOutcome::Success {
            output: Value {
                value: input.value + 1,
            },
        }
    }
}

struct ConflictingIncrement;

impl State for ConflictingIncrement {
    type Input = Value;
    type Output = OtherValue;
    type Failure = Never;

    fn state_id() -> mfm_program::Result<StableId> {
        Increment::state_id()
    }
}

impl PureState for ConflictingIncrement {
    fn evaluate(_input: Self::Input) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        ProposedStateOutcome::Success {
            output: OtherValue {
                value: "conflict".to_owned(),
            },
        }
    }
}

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
enum Selector {
    Left(BranchValue),
    Right(BranchValue),
}

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
#[allow(clippy::redundant_allocation)] // Exercises recursive Box Match projection parity.
enum NestedSelector {
    Nested(Box<Box<BranchValue>>),
}

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
enum ManualSelectorDescriptor {
    Branch(BranchValue),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ManualSelector {
    Branch(BranchValue),
}

impl mfm_values::MfmValue for ManualSelector {
    fn schema_descriptor() -> mfm_values::Result<mfm_values::SchemaDescriptor> {
        <ManualSelectorDescriptor as mfm_values::MfmValue>::schema_descriptor()
    }

    fn semantic_id() -> mfm_values::Result<mfm_ids::SemanticTypeId> {
        <ManualSelectorDescriptor as mfm_values::MfmValue>::semantic_id()
    }
}

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct BranchValue {
    value: u64,
}

struct Choose;

impl State for Choose {
    type Input = Value;
    type Output = Selector;
    type Failure = Never;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.runtime/choose@1").map_err(|_| ProgramError::InvalidContract)
    }
}

impl PureState for Choose {
    fn evaluate(input: Self::Input) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        ProposedStateOutcome::Success {
            output: if input.value % 2 == 0 {
                Selector::Left(BranchValue { value: input.value })
            } else {
                Selector::Right(BranchValue { value: input.value })
            },
        }
    }
}

struct ChooseNested;

impl State for ChooseNested {
    type Input = Value;
    type Output = NestedSelector;
    type Failure = Never;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.runtime/choose-nested@1").map_err(|_| ProgramError::InvalidContract)
    }
}

impl PureState for ChooseNested {
    fn evaluate(input: Self::Input) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        ProposedStateOutcome::Success {
            output: NestedSelector::Nested(Box::new(Box::new(BranchValue { value: input.value }))),
        }
    }
}

struct ChooseManual;

impl State for ChooseManual {
    type Input = Value;
    type Output = ManualSelector;
    type Failure = Never;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.runtime/choose-manual@1").map_err(|_| ProgramError::InvalidContract)
    }
}

impl PureState for ChooseManual {
    fn evaluate(input: Self::Input) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        ProposedStateOutcome::Success {
            output: ManualSelector::Branch(BranchValue { value: input.value }),
        }
    }
}

struct IncrementBranch;

impl State for IncrementBranch {
    type Input = BranchValue;
    type Output = BranchValue;
    type Failure = Never;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.runtime/increment-branch@1")
            .map_err(|_| ProgramError::InvalidContract)
    }
}

struct RightBranch;

impl State for RightBranch {
    type Input = BranchValue;
    type Output = BranchValue;
    type Failure = Never;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.runtime/right-branch@1").map_err(|_| ProgramError::InvalidContract)
    }
}

impl PureState for RightBranch {
    fn evaluate(input: Self::Input) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        ProposedStateOutcome::Success {
            output: BranchValue {
                value: input.value + 100,
            },
        }
    }
}

impl PureState for IncrementBranch {
    fn evaluate(input: Self::Input) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        ProposedStateOutcome::Success {
            output: BranchValue {
                value: input.value + 1,
            },
        }
    }
}

struct Fail;

impl State for Fail {
    type Input = Value;
    type Output = Value;
    type Failure = Value;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.runtime/fail@1").map_err(|_| ProgramError::InvalidContract)
    }
}

impl PureState for Fail {
    fn evaluate(input: Self::Input) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        ProposedStateOutcome::Failure { failure: input }
    }
}

struct Rejoin;

impl State for Rejoin {
    type Input = Value;
    type Output = Value;
    type Failure = Value;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.runtime/rejoin@1").map_err(|_| ProgramError::InvalidContract)
    }
}

impl PureState for Rejoin {
    fn evaluate(input: Self::Input) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        if input.value % 2 == 0 {
            ProposedStateOutcome::Success { output: input }
        } else {
            ProposedStateOutcome::Failure { failure: input }
        }
    }
}

struct BlockingSignals {
    entered: tokio::sync::Notify,
    finished: tokio::sync::Notify,
    release: AtomicBool,
    evaluations: AtomicUsize,
}

static BLOCKING_SIGNALS: OnceLock<Mutex<Option<Arc<BlockingSignals>>>> = OnceLock::new();

struct BlockingIncrement;

impl State for BlockingIncrement {
    type Input = Value;
    type Output = Value;
    type Failure = Never;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.runtime/blocking-increment@1")
            .map_err(|_| ProgramError::InvalidContract)
    }
}

impl PureState for BlockingIncrement {
    fn evaluate(input: Self::Input) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        let signals = BLOCKING_SIGNALS
            .get()
            .and_then(|slot| slot.lock().ok()?.as_ref().cloned())
            .expect("test installs blocking signals");
        signals.evaluations.fetch_add(1, Ordering::SeqCst);
        signals.entered.notify_one();
        while !signals.release.load(Ordering::SeqCst) {
            std::thread::yield_now();
        }
        signals.finished.notify_one();
        ProposedStateOutcome::Success {
            output: Value {
                value: input.value + 1,
            },
        }
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
    type Intent = Intent;
    type Evidence = Evidence;

    fn contract_id() -> mfm_capabilities::Result<StableId> {
        StableId::new("mfm.test.runtime/observation@1")
            .map_err(|_| CapabilityError::InvalidContract)
    }

    fn bind_evidence(
        intent: &Self::Intent,
        evidence: &Self::Evidence,
    ) -> mfm_capabilities::Result<()> {
        (intent.value == evidence.value)
            .then_some(())
            .ok_or(CapabilityError::EvidenceBinding)
    }
}

struct Observe;

impl State for Observe {
    type Input = Value;
    type Output = Value;
    type Failure = Value;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.runtime/observe@1").map_err(|_| ProgramError::InvalidContract)
    }
}

impl ReadState<Observation> for Observe {
    fn prepare(input: &Self::Input) -> Result<Intent, ReadPreparationError> {
        (input.value != 999)
            .then_some(Intent { value: input.value })
            .ok_or(ReadPreparationError)
    }

    fn interpret(
        input: Self::Input,
        evidence: &Evidence,
    ) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        if evidence.accepted {
            ProposedStateOutcome::Success { output: input }
        } else {
            ProposedStateOutcome::Failure { failure: input }
        }
    }
}

struct AliasObservation;

impl ReadCapabilityContract for AliasObservation {
    type Intent = Intent;
    type Evidence = Evidence;

    fn contract_id() -> mfm_capabilities::Result<StableId> {
        Observation::contract_id()
    }

    fn bind_evidence(
        intent: &Self::Intent,
        evidence: &Self::Evidence,
    ) -> mfm_capabilities::Result<()> {
        Observation::bind_evidence(intent, evidence)
    }
}

struct DriftObservation;

impl ReadCapabilityContract for DriftObservation {
    type Intent = OtherValue;
    type Evidence = Evidence;

    fn contract_id() -> mfm_capabilities::Result<StableId> {
        Observation::contract_id()
    }

    fn bind_evidence(
        _intent: &Self::Intent,
        _evidence: &Self::Evidence,
    ) -> mfm_capabilities::Result<()> {
        Ok(())
    }
}

struct AlternateObservation;

impl ReadCapabilityContract for AlternateObservation {
    type Intent = Intent;
    type Evidence = Evidence;

    fn contract_id() -> mfm_capabilities::Result<StableId> {
        StableId::new("mfm.test.runtime/alternate-observation@1")
            .map_err(|_| CapabilityError::InvalidContract)
    }

    fn bind_evidence(
        intent: &Self::Intent,
        evidence: &Self::Evidence,
    ) -> mfm_capabilities::Result<()> {
        Observation::bind_evidence(intent, evidence)
    }
}

struct AliasObserve;

impl State for AliasObserve {
    type Input = Value;
    type Output = Value;
    type Failure = Value;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.runtime/alias-observe@1").map_err(|_| ProgramError::InvalidContract)
    }
}

impl ReadState<AliasObservation> for AliasObserve {
    fn prepare(input: &Self::Input) -> Result<Intent, ReadPreparationError> {
        Ok(Intent { value: input.value })
    }

    fn interpret(
        input: Self::Input,
        _evidence: &Evidence,
    ) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        ProposedStateOutcome::Success { output: input }
    }
}

struct DriftObserve;

impl State for DriftObserve {
    type Input = Value;
    type Output = Value;
    type Failure = Value;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.runtime/drift-observe@1").map_err(|_| ProgramError::InvalidContract)
    }
}

impl ReadState<DriftObservation> for DriftObserve {
    fn prepare(input: &Self::Input) -> Result<OtherValue, ReadPreparationError> {
        Ok(OtherValue {
            value: input.value.to_string(),
        })
    }

    fn interpret(
        input: Self::Input,
        _evidence: &Evidence,
    ) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        ProposedStateOutcome::Success { output: input }
    }
}

struct ReadIncrementCollision;

impl State for ReadIncrementCollision {
    type Input = Value;
    type Output = Value;
    type Failure = Never;

    fn state_id() -> mfm_program::Result<StableId> {
        Increment::state_id()
    }
}

impl ReadState<Observation> for ReadIncrementCollision {
    fn prepare(input: &Self::Input) -> Result<Intent, ReadPreparationError> {
        Ok(Intent { value: input.value })
    }

    fn interpret(
        input: Self::Input,
        _evidence: &Evidence,
    ) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        ProposedStateOutcome::Success { output: input }
    }
}

struct ReadCollisionOne;
struct ReadCollisionTwo;

impl State for ReadCollisionOne {
    type Input = Value;
    type Output = Value;
    type Failure = Never;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.runtime/read-collision@1")
            .map_err(|_| ProgramError::InvalidContract)
    }
}

impl State for ReadCollisionTwo {
    type Input = Value;
    type Output = Value;
    type Failure = Never;

    fn state_id() -> mfm_program::Result<StableId> {
        ReadCollisionOne::state_id()
    }
}

impl ReadState<Observation> for ReadCollisionOne {
    fn prepare(input: &Self::Input) -> Result<Intent, ReadPreparationError> {
        Ok(Intent { value: input.value })
    }

    fn interpret(
        input: Self::Input,
        _evidence: &Evidence,
    ) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        ProposedStateOutcome::Success { output: input }
    }
}

impl ReadState<AlternateObservation> for ReadCollisionTwo {
    fn prepare(input: &Self::Input) -> Result<Intent, ReadPreparationError> {
        Ok(Intent { value: input.value })
    }

    fn interpret(
        input: Self::Input,
        _evidence: &Evidence,
    ) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        ProposedStateOutcome::Success { output: input }
    }
}

struct InvalidRuntimeState;

impl State for InvalidRuntimeState {
    type Input = Value;
    type Output = Value;
    type Failure = Never;

    fn state_id() -> mfm_program::Result<StableId> {
        Err(ProgramError::InvalidContract)
    }
}

impl PureState for InvalidRuntimeState {
    fn evaluate(input: Self::Input) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        ProposedStateOutcome::Success { output: input }
    }
}

struct InvalidRuntimeCapability;

impl ReadCapabilityContract for InvalidRuntimeCapability {
    type Intent = Intent;
    type Evidence = Evidence;

    fn contract_id() -> mfm_capabilities::Result<StableId> {
        Err(CapabilityError::InvalidContract)
    }

    fn bind_evidence(
        _intent: &Self::Intent,
        _evidence: &Self::Evidence,
    ) -> mfm_capabilities::Result<()> {
        Ok(())
    }
}

struct InvalidRuntimeRead;

impl State for InvalidRuntimeRead {
    type Input = Value;
    type Output = Value;
    type Failure = Never;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.runtime/invalid-runtime-read@1")
            .map_err(|_| ProgramError::InvalidContract)
    }
}

impl ReadState<InvalidRuntimeCapability> for InvalidRuntimeRead {
    fn prepare(input: &Self::Input) -> Result<Intent, ReadPreparationError> {
        Ok(Intent { value: input.value })
    }

    fn interpret(
        input: Self::Input,
        _evidence: &Evidence,
    ) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        ProposedStateOutcome::Success { output: input }
    }
}

struct ObserveNever;

impl State for ObserveNever {
    type Input = Value;
    type Output = Value;
    type Failure = Never;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.runtime/observe-never@1").map_err(|_| ProgramError::InvalidContract)
    }
}

impl ReadState<Observation> for ObserveNever {
    fn prepare(input: &Self::Input) -> Result<Intent, ReadPreparationError> {
        Ok(Intent { value: input.value })
    }

    fn interpret(
        input: Self::Input,
        _evidence: &Evidence,
    ) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        ProposedStateOutcome::Success { output: input }
    }
}

static ASSOCIATION_STATE_ID_CALLS: AtomicUsize = AtomicUsize::new(0);
static ASSOCIATION_CAPABILITY_ID_CALLS: AtomicUsize = AtomicUsize::new(0);

struct AssociationCountingCapability;

impl ReadCapabilityContract for AssociationCountingCapability {
    type Intent = Intent;
    type Evidence = Evidence;

    fn contract_id() -> mfm_capabilities::Result<StableId> {
        ASSOCIATION_CAPABILITY_ID_CALLS.fetch_add(1, Ordering::SeqCst);
        StableId::new("mfm.test.runtime/association-counting@1")
            .map_err(|_| CapabilityError::InvalidContract)
    }

    fn bind_evidence(
        intent: &Self::Intent,
        evidence: &Self::Evidence,
    ) -> mfm_capabilities::Result<()> {
        Observation::bind_evidence(intent, evidence)
    }
}

struct AssociationCountingRead;

impl State for AssociationCountingRead {
    type Input = Value;
    type Output = Value;
    type Failure = Never;

    fn state_id() -> mfm_program::Result<StableId> {
        ASSOCIATION_STATE_ID_CALLS.fetch_add(1, Ordering::SeqCst);
        StableId::new("mfm.test.runtime/association-counting-read@1")
            .map_err(|_| ProgramError::InvalidContract)
    }
}

impl ReadState<AssociationCountingCapability> for AssociationCountingRead {
    fn prepare(input: &Self::Input) -> Result<Intent, ReadPreparationError> {
        Ok(Intent { value: input.value })
    }

    fn interpret(
        input: Self::Input,
        _evidence: &Evidence,
    ) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        ProposedStateOutcome::Success { output: input }
    }
}

struct HookSupport;

impl State for HookSupport {
    type Input = Value;
    type Output = Value;
    type Failure = Never;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.runtime/hook-support@1").map_err(|_| ProgramError::InvalidContract)
    }
}

impl PureState for HookSupport {
    fn evaluate(input: Self::Input) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        ProposedStateOutcome::Success { output: input }
    }
}

struct HookCapability;

impl ReadCapabilityContract for HookCapability {
    type Intent = Intent;
    type Evidence = Evidence;

    fn contract_id() -> mfm_capabilities::Result<StableId> {
        StableId::new("mfm.test.runtime/hook-capability@1")
            .map_err(|_| CapabilityError::InvalidContract)
    }

    fn bind_evidence(
        intent: &Self::Intent,
        evidence: &Self::Evidence,
    ) -> mfm_capabilities::Result<()> {
        Observation::bind_evidence(intent, evidence)
    }
}

struct HookRead;

impl State for HookRead {
    type Input = Value;
    type Output = Value;
    type Failure = Never;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.runtime/hook-read@1").map_err(|_| ProgramError::InvalidContract)
    }
}

impl ReadState<HookCapability> for HookRead {
    fn prepare(input: &Self::Input) -> Result<Intent, ReadPreparationError> {
        Ok(Intent { value: input.value })
    }

    fn interpret(
        input: Self::Input,
        _evidence: &Evidence,
    ) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        ProposedStateOutcome::Success { output: input }
    }
}

#[derive(Default)]
struct HookCounters {
    before: AtomicUsize,
    binding: AtomicUsize,
    after: AtomicUsize,
}

struct HookSetup {
    binding: Binding,
    counters: Arc<HookCounters>,
}

impl CapabilityInjection<HookRead> for HookCapability {
    type Setup = HookSetup;
    type ExpandedInput = Value;
    type ExpandedOutput = Value;

    fn original_binding_ref(setup: &Self::Setup) -> mfm_program::Result<mfm_ids::ContentRef> {
        setup.counters.binding.fetch_add(1, Ordering::SeqCst);
        canonicalize_mfm_value(&setup.binding)
            .map(|(_, binding_ref)| binding_ref)
            .map_err(|_| ProgramError::InvalidContract)
    }

    fn write_before(
        setup: &Self::Setup,
        writer: &mut mfm_program::InjectionWriter,
    ) -> mfm_program::Result<()> {
        setup.counters.before.fetch_add(1, Ordering::SeqCst);
        writer.pure::<HookSupport>()
    }

    fn write_after(
        setup: &Self::Setup,
        writer: &mut mfm_program::InjectionWriter,
    ) -> mfm_program::Result<()> {
        setup.counters.after.fetch_add(1, Ordering::SeqCst);
        writer.pure::<HookSupport>()
    }
}

struct ClassifiedFailure;

impl State for ClassifiedFailure {
    type Input = Value;
    type Output = Value;
    type Failure = Value;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.runtime/classified-failure@1")
            .map_err(|_| ProgramError::InvalidContract)
    }
}

impl PureState for ClassifiedFailure {
    fn evaluate(input: Self::Input) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        if input.value % 2 == 0 {
            ProposedStateOutcome::Success { output: input }
        } else {
            ProposedStateOutcome::Failure { failure: input }
        }
    }
}

struct ClassifyRecovery;

impl State for ClassifyRecovery {
    type Input = Value;
    type Output = RecoverySelector;
    type Failure = Never;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.runtime/classify-recovery@1")
            .map_err(|_| ProgramError::InvalidContract)
    }
}

impl PureState for ClassifyRecovery {
    fn evaluate(input: Self::Input) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        let output = if input.value == 3 {
            RecoverySelector::Terminal(RecoveryPayload { value: input.value })
        } else {
            RecoverySelector::Recover(RecoveryPayload { value: input.value })
        };
        ProposedStateOutcome::Success { output }
    }
}

struct RecoverValue;

impl State for RecoverValue {
    type Input = RecoveryPayload;
    type Output = Value;
    type Failure = Never;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.runtime/recover-value@1").map_err(|_| ProgramError::InvalidContract)
    }
}

impl PureState for RecoverValue {
    fn evaluate(input: Self::Input) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        ProposedStateOutcome::Success {
            output: Value { value: input.value },
        }
    }
}

struct TerminalRecovery;

impl State for TerminalRecovery {
    type Input = RecoveryPayload;
    type Output = OtherValue;
    type Failure = Never;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.runtime/terminal-recovery@1")
            .map_err(|_| ProgramError::InvalidContract)
    }
}

impl PureState for TerminalRecovery {
    fn evaluate(input: Self::Input) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        ProposedStateOutcome::Success {
            output: OtherValue {
                value: format!("terminal-{}", input.value),
            },
        }
    }
}

struct FinalizeRecovery;

impl State for FinalizeRecovery {
    type Input = Value;
    type Output = OtherValue;
    type Failure = Never;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.runtime/finalize-recovery@1")
            .map_err(|_| ProgramError::InvalidContract)
    }
}

impl PureState for FinalizeRecovery {
    fn evaluate(input: Self::Input) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        ProposedStateOutcome::Success {
            output: OtherValue {
                value: format!("final-{}", input.value),
            },
        }
    }
}

fn run(byte: u8) -> RunId {
    RunId::from_digest(DigestBytes::from_array([byte; 32]))
}

struct EmptyValue;

impl Operation for EmptyValue {
    type Input = Value;
    type Output = Value;
    type Failure = Never;

    fn expand(
        &self,
        _body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        Ok(())
    }
}

struct ForeignEmpty;

impl Operation for ForeignEmpty {
    type Input = OtherValue;
    type Output = OtherValue;
    type Failure = Never;

    fn expand(
        &self,
        _body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        Ok(())
    }
}

struct PureOperation<S>(std::marker::PhantomData<fn() -> S>);

impl<S: PureState> Operation for PureOperation<S> {
    type Input = S::Input;
    type Output = S::Output;
    type Failure = S::Failure;

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        body.pure::<S>()
    }
}

struct MissingRootCodec;

impl Operation for MissingRootCodec {
    type Input = Value;
    type Output = Value;
    type Failure = OtherValue;

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        body.pure::<Increment>()
    }
}

struct RuntimeMatch;

impl Operation for RuntimeMatch {
    type Input = Value;
    type Output = BranchValue;
    type Failure = Never;

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        body.pure::<Choose>()?;
        body.match_join::<Selector, BranchValue>(|arms| {
            arms.arm::<BranchValue>(
                StableId::new("left").map_err(|_| ProgramError::InvalidContract)?,
                |branch| branch.pure::<IncrementBranch>(),
            )?;
            arms.arm::<BranchValue>(
                StableId::new("right").map_err(|_| ProgramError::InvalidContract)?,
                |branch| branch.pure::<RightBranch>(),
            )
        })
    }
}

struct RuntimeNestedMatch;

impl Operation for RuntimeNestedMatch {
    type Input = Value;
    type Output = BranchValue;
    type Failure = Never;

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        body.pure::<ChooseNested>()?;
        body.match_join::<NestedSelector, BranchValue>(|arms| {
            arms.arm::<BranchValue>(
                StableId::new("nested").map_err(|_| ProgramError::InvalidContract)?,
                |branch| branch.pure::<IncrementBranch>(),
            )
        })
    }
}

struct RuntimeManualMatch;

impl Operation for RuntimeManualMatch {
    type Input = Value;
    type Output = BranchValue;
    type Failure = Never;

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        body.pure::<ChooseManual>()?;
        body.match_join::<ManualSelector, BranchValue>(|arms| {
            arms.arm::<BranchValue>(
                StableId::new("branch").map_err(|_| ProgramError::InvalidContract)?,
                |branch| branch.pure::<IncrementBranch>(),
            )
        })
    }
}

macro_rules! identity_injection {
    ($capability:ty, $state:ty) => {
        impl CapabilityInjection<$state> for $capability {
            type Setup = Binding;
            type ExpandedInput = <$state as State>::Input;
            type ExpandedOutput = <$state as State>::Output;

            fn original_binding_ref(
                setup: &Self::Setup,
            ) -> mfm_program::Result<mfm_ids::ContentRef> {
                canonicalize_mfm_value(setup)
                    .map(|(_, binding_ref)| binding_ref)
                    .map_err(|_| ProgramError::InvalidContract)
            }
        }
    };
}

identity_injection!(Observation, Observe);
identity_injection!(Observation, ObserveNever);
identity_injection!(AssociationCountingCapability, AssociationCountingRead);

struct RuntimeRead;

impl Operation for RuntimeRead {
    type Input = Value;
    type Output = Value;
    type Failure = Value;

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        body.read::<Observe, Observation>(&binding())
    }
}

struct RuntimeReadNever;

impl Operation for RuntimeReadNever {
    type Input = Value;
    type Output = Value;
    type Failure = Never;

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        body.read::<ObserveNever, Observation>(&binding())
    }
}

struct RuntimeAssociationCountingRead;

impl Operation for RuntimeAssociationCountingRead {
    type Input = Value;
    type Output = Value;
    type Failure = Never;

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        body.read::<AssociationCountingRead, AssociationCountingCapability>(&binding())
    }
}

struct RuntimeHookRead {
    setup: HookSetup,
}

impl Operation for RuntimeHookRead {
    type Input = Value;
    type Output = Value;
    type Failure = Never;

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        body.read::<HookRead, HookCapability>(&self.setup)
    }
}

struct RuntimeClassifiedRecovery;

impl Operation for RuntimeClassifiedRecovery {
    type Input = Value;
    type Output = OtherValue;
    type Failure = Never;

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        body.with_failure_handler::<Value, Value>(
            |protected| protected.pure::<ClassifiedFailure>(),
            |handler| {
                handler.pure::<ClassifyRecovery>()?;
                handler.match_join::<RecoverySelector, Value>(|arms| {
                    arms.arm::<RecoveryPayload>(
                        StableId::new("recover").map_err(|_| ProgramError::InvalidContract)?,
                        |branch| branch.pure::<RecoverValue>(),
                    )?;
                    arms.arm::<RecoveryPayload>(
                        StableId::new("terminal").map_err(|_| ProgramError::InvalidContract)?,
                        |branch| branch.pure::<TerminalRecovery>(),
                    )
                })
            },
        )?;
        body.pure::<FinalizeRecovery>()
    }
}

fn zero_program() -> Program {
    expand_program(
        EntryPointId::new("mfm.test.runtime/zero@1").expect("entry"),
        &EmptyValue,
    )
    .expect("zero Program")
}

fn pure_program() -> Program {
    expand_program(
        EntryPointId::new("mfm.test.runtime/pure@1").expect("entry"),
        &PureOperation::<Increment>(std::marker::PhantomData),
    )
    .expect("pure Program")
}

fn counting_retry_program() -> Program {
    expand_program(
        EntryPointId::new("mfm.test.runtime/counting-retry@1").expect("entry"),
        &PureOperation::<CountingRetryIncrement>(std::marker::PhantomData),
    )
    .expect("counting Program")
}

fn missing_root_codec_program() -> Program {
    expand_program(
        EntryPointId::new("mfm.test.runtime/missing-root-codec@1").expect("entry"),
        &MissingRootCodec,
    )
    .expect("missing root codec Program")
}

fn match_program() -> Program {
    expand_program(
        EntryPointId::new("mfm.test.runtime/match@1").expect("entry"),
        &RuntimeMatch,
    )
    .expect("Match Program")
}

fn nested_match_program() -> Program {
    expand_program(
        EntryPointId::new("mfm.test.runtime/nested-match@1").expect("entry"),
        &RuntimeNestedMatch,
    )
    .expect("nested Match Program")
}

fn manual_match_program() -> Program {
    expand_program(
        EntryPointId::new("mfm.test.runtime/manual-match@1").expect("entry"),
        &RuntimeManualMatch,
    )
    .expect("manual Match Program")
}

fn failure_program() -> Program {
    expand_program(
        EntryPointId::new("mfm.test.runtime/failure@1").expect("entry"),
        &PureOperation::<Fail>(std::marker::PhantomData),
    )
    .expect("failure Program")
}

fn rejoin_program() -> Program {
    let value = nominal_contract_ref::<Value>().expect("value");
    let never = nominal_contract_ref::<Never>().expect("never");
    decode_program(serde_json::json!({
        "entry_point_id": "mfm.test.runtime/rejoin@1",
        "admitted_context_contract_ref": value,
        "root_success_contract_ref": value,
        "root_failure_contract_ref": never,
        "declarations": [
            state_wire(
                state_implementation_ref::<Rejoin>().expect("rejoin"),
                value.clone(), value.clone(), value.clone(),
                serde_json::json!({ "kind": "pure" }), Some(1), Some(1),
            ),
            state_wire(
                state_implementation_ref::<Increment>().expect("increment"),
                value.clone(), value.clone(), never.clone(),
                serde_json::json!({ "kind": "pure" }), None, None,
            ),
        ],
    }))
}

fn blocking_program() -> Program {
    expand_program(
        EntryPointId::new("mfm.test.runtime/blocking@1").expect("entry"),
        &PureOperation::<BlockingIncrement>(std::marker::PhantomData),
    )
    .expect("blocking Program")
}

fn foreign_zero_program() -> Program {
    expand_program(
        EntryPointId::new("mfm.test.runtime/foreign@1").expect("entry"),
        &ForeignEmpty,
    )
    .expect("foreign Program")
}

fn unsupported_match_program() -> Program {
    let selector = nominal_contract_ref::<OtherValue>().expect("selector");
    let value = nominal_contract_ref::<Value>().expect("value");
    let never = nominal_contract_ref::<Never>().expect("never");
    decode_program(serde_json::json!({
        "entry_point_id": "mfm.test.runtime/unsupported-match@1",
        "admitted_context_contract_ref": selector,
        "root_success_contract_ref": value,
        "root_failure_contract_ref": never,
        "declarations": [
            {
                "kind": "match",
                "value": {
                    "selector_contract_ref": selector,
                    "variants": [{ "tag": "left", "entry_index": 1 }],
                },
            },
            state_wire(
                state_implementation_ref::<Increment>().expect("increment"),
                value.clone(), value.clone(), never.clone(),
                serde_json::json!({ "kind": "pure" }), None, None,
            ),
        ],
    }))
}

fn binding() -> Binding {
    Binding { route: 7 }
}

fn read_program() -> Program {
    expand_program(
        EntryPointId::new("mfm.test.runtime/read@1").expect("entry"),
        &RuntimeRead,
    )
    .expect("read Program")
}

fn read_never_program() -> Program {
    expand_program(
        EntryPointId::new("mfm.test.runtime/read-never@1").expect("entry"),
        &RuntimeReadNever,
    )
    .expect("read Never Program")
}

fn missing_capability_program() -> Program {
    let value = nominal_contract_ref::<Value>().expect("value");
    let (_, binding_ref) = canonicalize_mfm_value(&binding()).expect("binding ref");
    decode_program(serde_json::json!({
        "entry_point_id": "mfm.test.runtime/missing-capability@1",
        "admitted_context_contract_ref": value,
        "root_success_contract_ref": value,
        "root_failure_contract_ref": value,
        "declarations": [state_wire(
            state_implementation_ref::<Observe>().expect("observe"),
            value.clone(), value.clone(), value.clone(),
            serde_json::json!({
                "kind": "read",
                "capability_contract_ref": capability_contract_ref::<AlternateObservation>().expect("capability"),
                "intent_contract_ref": nominal_contract_ref::<Intent>().expect("intent"),
                "evidence_contract_ref": nominal_contract_ref::<Evidence>().expect("evidence"),
                "binding_ref": binding_ref,
            }),
            None, None,
        )],
    }))
}

fn association_counting_program() -> Program {
    expand_program(
        EntryPointId::new("mfm.test.runtime/association-counting@1").expect("entry"),
        &RuntimeAssociationCountingRead,
    )
    .expect("association counting Program")
}

fn classified_recovery_program() -> Program {
    expand_program(
        EntryPointId::new("mfm.test.runtime/classified-recovery@1").expect("entry"),
        &RuntimeClassifiedRecovery,
    )
    .expect("classified recovery Program")
}

#[allow(clippy::too_many_arguments)]
fn state_wire(
    implementation: mfm_ids::ContentRef,
    input: mfm_ids::ContentRef,
    output: mfm_ids::ContentRef,
    failure: mfm_ids::ContentRef,
    execution: serde_json::Value,
    next_index: Option<u16>,
    failure_next_index: Option<u16>,
) -> serde_json::Value {
    serde_json::json!({
        "kind": "state",
        "value": {
            "state_implementation_ref": implementation,
            "input_contract_ref": input,
            "output_contract_ref": output,
            "failure_contract_ref": failure,
            "execution": execution,
            "next_index": next_index,
            "failure_next_index": failure_next_index,
        },
    })
}

fn decode_program(wire: serde_json::Value) -> Program {
    let canonical = mfm_canonical::PlainCanonicalJsonBytes::from_json_str(
        &serde_json::to_string(&wire).expect("wire JSON"),
    )
    .expect("canonical Program JSON");
    Program::decode_canonical(canonical.as_bytes()).expect("retained Program")
}

fn assert_same_view(left: &mfm_runtime::RunView, right: &mfm_runtime::RunView) {
    assert_eq!(left.run_id(), right.run_id());
    assert_eq!(left.head_sequence(), right.head_sequence());
    assert_eq!(left.head_digest(), right.head_digest());
    match (left.state(), right.state()) {
        (RunViewState::Runnable, RunViewState::Runnable) => {}
        (RunViewState::Succeeded(left), RunViewState::Succeeded(right))
        | (RunViewState::Failed(left), RunViewState::Failed(right)) => {
            assert_eq!(left.contract_ref(), right.contract_ref());
            assert_eq!(left.value_ref(), right.value_ref());
            assert_eq!(left.canonical_bytes(), right.canonical_bytes());
        }
        _ => panic!("run states differ"),
    }
}

#[tokio::test]
async fn runtime_hot_cold_pure_and_zero_state_paths_match() {
    let mut builder = RuntimeAssemblyBuilder::new();
    builder.register_value::<Value>().expect("value");
    builder.register_pure::<Increment>().expect("pure");
    let assembly = builder.finish().expect("assembly");
    let store = Arc::new(MemoryStore::new());
    let runtime = Runtime::new(assembly, store);

    let zero_id = run(1);
    let zero = runtime
        .start(zero_id.clone(), zero_program(), Value { value: 4 })
        .await
        .expect("zero start");
    assert_eq!(zero.head_sequence(), 1);
    assert!(matches!(zero.state(), RunViewState::Succeeded(_)));
    let zero_cold = runtime.read(&zero_id).await.expect("zero cold");
    assert_same_view(&zero, &zero_cold);

    let pure_id = run(2);
    let pure = runtime
        .start(pure_id.clone(), pure_program(), Value { value: 4 })
        .await
        .expect("pure start");
    assert_eq!(pure.head_sequence(), 2);
    let RunViewState::Succeeded(value) = pure.state() else {
        panic!("pure success");
    };
    assert_eq!(value.canonical_bytes(), br#"{"value":5}"#);
    let pure_cold = runtime.read(&pure_id).await.expect("pure cold");
    assert_same_view(&pure, &pure_cold);
}

#[tokio::test]
async fn runtime_hot_cold_match_and_failure_roots_match() {
    let mut builder = RuntimeAssemblyBuilder::new();
    builder.register_pure::<Choose>().expect("choose");
    builder
        .register_pure::<ChooseNested>()
        .expect("choose nested");
    builder
        .register_pure::<IncrementBranch>()
        .expect("increment branch");
    builder
        .register_pure::<RightBranch>()
        .expect("right branch");
    builder.register_pure::<Fail>().expect("fail");
    builder.register_pure::<Rejoin>().expect("rejoin");
    builder.register_pure::<Increment>().expect("increment");
    let runtime = Runtime::new(
        builder.finish().expect("assembly"),
        Arc::new(MemoryStore::new()),
    );

    for (byte, input) in [(30, 4), (31, 5)] {
        let id = run(byte);
        let hot = runtime
            .start(id.clone(), match_program(), Value { value: input })
            .await
            .expect("match start");
        assert_eq!(hot.head_sequence(), 3);
        let RunViewState::Succeeded(output) = hot.state() else {
            panic!("match success");
        };
        assert_eq!(
            output.canonical_bytes(),
            format!(
                "{{\"value\":{}}}",
                if input % 2 == 0 {
                    input + 1
                } else {
                    input + 100
                }
            )
            .as_bytes()
        );
        let cold = runtime.read(&id).await.expect("match read");
        assert_same_view(&hot, &cold);
    }

    let nested_id = run(29);
    let nested_hot = runtime
        .start(
            nested_id.clone(),
            nested_match_program(),
            Value { value: 7 },
        )
        .await
        .expect("nested match start");
    assert_eq!(nested_hot.head_sequence(), 3);
    let RunViewState::Succeeded(nested_value) = nested_hot.state() else {
        panic!("nested match success");
    };
    assert_eq!(nested_value.canonical_bytes(), br#"{"value":8}"#);
    assert_same_view(
        &nested_hot,
        &runtime.read(&nested_id).await.expect("nested match cold"),
    );

    let id = run(32);
    let hot = runtime
        .start(id.clone(), failure_program(), Value { value: 9 })
        .await
        .expect("failure start");
    let RunViewState::Failed(failure) = hot.state() else {
        panic!("failure root");
    };
    assert_eq!(failure.canonical_bytes(), br#"{"value":9}"#);
    assert_same_view(&hot, &runtime.read(&id).await.expect("cold"));

    for (byte, input) in [(33, 4), (34, 5)] {
        let id = run(byte);
        let hot = runtime
            .start(id.clone(), rejoin_program(), Value { value: input })
            .await
            .expect("rejoin start");
        assert_eq!(hot.head_sequence(), 3);
        let RunViewState::Succeeded(value) = hot.state() else {
            panic!("rejoin success");
        };
        assert_eq!(
            value.canonical_bytes(),
            format!("{{\"value\":{}}}", input + 1).as_bytes()
        );
        assert_same_view(&hot, &runtime.read(&id).await.expect("rejoin cold"));
    }
}

#[tokio::test]
async fn classified_failure_recovery_and_terminal_paths_are_hot_cold_exact() {
    let mut builder = RuntimeAssemblyBuilder::new();
    builder
        .register_pure::<ClassifiedFailure>()
        .expect("fallible State");
    builder
        .register_pure::<ClassifyRecovery>()
        .expect("classifier State");
    builder
        .register_pure::<RecoverValue>()
        .expect("recover State");
    builder
        .register_pure::<TerminalRecovery>()
        .expect("terminal State");
    builder
        .register_pure::<FinalizeRecovery>()
        .expect("final State");
    let runtime = Runtime::new(
        builder.finish().expect("assembly"),
        Arc::new(MemoryStore::new()),
    );

    for (byte, input, expected, sequence) in [
        (70, 0, br#"{"value":"final-0"}"#.as_slice(), 3),
        (71, 1, br#"{"value":"final-1"}"#.as_slice(), 5),
        (72, 3, br#"{"value":"terminal-3"}"#.as_slice(), 4),
    ] {
        let id = run(byte);
        let hot = runtime
            .start(
                id.clone(),
                classified_recovery_program(),
                Value { value: input },
            )
            .await
            .expect("classified recovery start");
        assert_eq!(hot.head_sequence(), sequence);
        let RunViewState::Succeeded(output) = hot.state() else {
            panic!("classified path must succeed");
        };
        assert_eq!(output.canonical_bytes(), expected);
        assert_same_view(
            &hot,
            &runtime.read(&id).await.expect("classified cold read"),
        );
    }
}

fn register_read_runtime<F>(store: Arc<dyn Store>, callback: F) -> Runtime
where
    F: for<'a> Fn(
            &'a Intent,
        ) -> Pin<
            Box<dyn Future<Output = std::result::Result<Evidence, ReadAdapterError>> + Send + 'a>,
        > + Send
        + Sync
        + 'static,
{
    let mut builder = RuntimeAssemblyBuilder::new();
    builder
        .register_read::<Observe, Observation>()
        .expect("read");
    builder
        .register_adapter::<Observation, _, _>(binding(), callback)
        .expect("adapter");
    Runtime::new(builder.finish().expect("assembly"), store)
}

async fn retained_head(store: &MemoryStore, id: &RunId) -> u64 {
    let retained = store.load_run(id).await.expect("load").expect("present");
    mfm_journal::JournalHistory::qualify(id, retained)
        .expect("history")
        .head_sequence()
}

struct CountingStore {
    inner: Arc<MemoryStore>,
    loads: AtomicUsize,
    appends: AtomicUsize,
}

impl CountingStore {
    fn new(inner: Arc<MemoryStore>) -> Self {
        Self {
            inner,
            loads: AtomicUsize::new(0),
            appends: AtomicUsize::new(0),
        }
    }

    fn reset(&self) {
        self.loads.store(0, Ordering::SeqCst);
        self.appends.store(0, Ordering::SeqCst);
    }
}

impl Store for CountingStore {
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
        Box::pin(async move {
            self.loads.fetch_add(1, Ordering::SeqCst);
            self.inner.load_run(run_id).await
        })
    }

    fn append_run<'a>(
        &'a self,
        frame: &'a EncodedRunFrame,
    ) -> Pin<Box<dyn Future<Output = std::result::Result<AppendResult, StoreError>> + Send + 'a>>
    {
        Box::pin(async move {
            self.appends.fetch_add(1, Ordering::SeqCst);
            self.inner.append_run(frame).await
        })
    }
}

#[tokio::test]
async fn assembly_registration_is_idempotent_and_inconsistency_is_rejected() {
    let mut builder = RuntimeAssemblyBuilder::new();
    builder.register_value::<Value>().expect("value");
    builder.register_value::<Value>().expect("idempotent value");
    builder.register_pure::<Increment>().expect("state");
    builder
        .register_pure::<Increment>()
        .expect("idempotent state");
    assert_eq!(
        builder.register_pure::<ConflictingIncrement>(),
        Err(RuntimeError::IncompatibleAssembly)
    );

    let mut adapters = RuntimeAssemblyBuilder::new();
    adapters
        .register_adapter::<Observation, _, _>(binding(), |_| {
            Box::pin(async {
                Ok(Evidence {
                    value: 1,
                    accepted: true,
                })
            })
        })
        .expect("adapter");
    assert_eq!(
        adapters.register_adapter::<Observation, _, _>(binding(), |_| {
            Box::pin(async {
                Ok(Evidence {
                    value: 1,
                    accepted: true,
                })
            })
        }),
        Err(RuntimeError::IncompatibleAssembly)
    );

    let mut capability_type_collision = RuntimeAssemblyBuilder::new();
    capability_type_collision
        .register_read::<Observe, Observation>()
        .expect("original capability type");
    assert_eq!(
        capability_type_collision.register_read::<AliasObserve, AliasObservation>(),
        Err(RuntimeError::IncompatibleAssembly)
    );

    let mut capability_abi_collision = RuntimeAssemblyBuilder::new();
    capability_abi_collision
        .register_read::<Observe, Observation>()
        .expect("original capability ABI");
    assert_eq!(
        capability_abi_collision.register_read::<DriftObserve, DriftObservation>(),
        Err(RuntimeError::IncompatibleAssembly)
    );

    let mut state_mode_collision = RuntimeAssemblyBuilder::new();
    state_mode_collision
        .register_pure::<Increment>()
        .expect("pure state");
    assert_eq!(
        state_mode_collision.register_read::<ReadIncrementCollision, Observation>(),
        Err(RuntimeError::IncompatibleAssembly)
    );

    let mut state_capability_collision = RuntimeAssemblyBuilder::new();
    state_capability_collision
        .register_read::<ReadCollisionOne, Observation>()
        .expect("first state capability");
    assert_eq!(
        state_capability_collision.register_read::<ReadCollisionTwo, AlternateObservation>(),
        Err(RuntimeError::IncompatibleAssembly)
    );

    let mut invalid_static_identity = RuntimeAssemblyBuilder::new();
    assert_eq!(
        invalid_static_identity.register_pure::<InvalidRuntimeState>(),
        Err(RuntimeError::IncompatibleAssembly)
    );
    let mut invalid_capability_identity = RuntimeAssemblyBuilder::new();
    assert_eq!(
        invalid_capability_identity.register_read::<InvalidRuntimeRead, InvalidRuntimeCapability>(),
        Err(RuntimeError::IncompatibleAssembly)
    );
}

#[tokio::test]
async fn read_state_with_never_failure_registers_associates_and_executes() {
    let mut builder = RuntimeAssemblyBuilder::new();
    builder
        .register_read::<ObserveNever, Observation>()
        .expect("read never");
    builder
        .register_adapter::<Observation, _, _>(binding(), |intent| {
            Box::pin(async move {
                Ok(Evidence {
                    value: intent.value,
                    accepted: true,
                })
            })
        })
        .expect("adapter");
    let runtime = Runtime::new(
        builder.finish().expect("assembly"),
        Arc::new(MemoryStore::new()),
    );
    let id = run(39);
    let hot = runtime
        .start(id.clone(), read_never_program(), Value { value: 7 })
        .await
        .expect("read never start");
    assert!(matches!(hot.state(), RunViewState::Succeeded(_)));
    assert_same_view(&hot, &runtime.read(&id).await.expect("cold read never"));
}

#[tokio::test]
async fn association_and_exact_retry_io_order_is_frozen() {
    let memory = Arc::new(MemoryStore::new());
    let store = Arc::new(CountingStore::new(memory));
    let empty = Runtime::new(
        RuntimeAssemblyBuilder::new()
            .finish()
            .expect("empty assembly"),
        store.clone(),
    );
    let unsupported_id = run(35);
    assert!(matches!(
        empty
            .start(unsupported_id.clone(), pure_program(), Value { value: 1 })
            .await,
        Err(RuntimeError::IncompatibleAssembly)
    ));
    assert_eq!(store.loads.load(Ordering::SeqCst), 0);
    assert_eq!(store.appends.load(Ordering::SeqCst), 0);

    let provider_calls = Arc::new(AtomicUsize::new(0));
    let mut missing_capability_builder = RuntimeAssemblyBuilder::new();
    missing_capability_builder
        .register_read::<Observe, Observation>()
        .expect("read state");
    missing_capability_builder
        .register_adapter::<Observation, _, _>(binding(), {
            let provider_calls = Arc::clone(&provider_calls);
            move |intent| {
                provider_calls.fetch_add(1, Ordering::SeqCst);
                Box::pin(async move {
                    Ok(Evidence {
                        value: intent.value,
                        accepted: true,
                    })
                })
            }
        })
        .expect("adapter");
    let missing_capability = Runtime::new(
        missing_capability_builder.finish().expect("assembly"),
        store.clone(),
    );
    assert!(matches!(
        missing_capability
            .start(run(59), missing_capability_program(), Value { value: 1 })
            .await,
        Err(RuntimeError::IncompatibleAssembly)
    ));
    assert_eq!(provider_calls.load(Ordering::SeqCst), 0);
    assert_eq!(store.loads.load(Ordering::SeqCst), 0);
    assert_eq!(store.appends.load(Ordering::SeqCst), 0);

    let mut missing_root_builder = RuntimeAssemblyBuilder::new();
    missing_root_builder
        .register_pure::<Increment>()
        .expect("reachable state");
    let missing_root = Runtime::new(
        missing_root_builder.finish().expect("assembly"),
        store.clone(),
    );
    assert!(matches!(
        missing_root
            .start(run(56), missing_root_codec_program(), Value { value: 1 },)
            .await,
        Err(RuntimeError::IncompatibleAssembly)
    ));
    assert_eq!(store.loads.load(Ordering::SeqCst), 0);
    assert_eq!(store.appends.load(Ordering::SeqCst), 0);

    let mut complete_root_builder = RuntimeAssemblyBuilder::new();
    complete_root_builder
        .register_pure::<Increment>()
        .expect("reachable state");
    complete_root_builder
        .register_value::<OtherValue>()
        .expect("root failure codec");
    let complete_root = Runtime::new(
        complete_root_builder.finish().expect("assembly"),
        store.clone(),
    );
    let complete = complete_root
        .start(run(57), missing_root_codec_program(), Value { value: 1 })
        .await
        .expect("complete roots");
    assert!(matches!(complete.state(), RunViewState::Succeeded(_)));
    assert_eq!(store.loads.load(Ordering::SeqCst), 0);
    assert_eq!(store.appends.load(Ordering::SeqCst), 2);
    store.reset();

    let mut unsupported_match = RuntimeAssemblyBuilder::new();
    unsupported_match
        .register_value::<OtherValue>()
        .expect("selector codec");
    unsupported_match
        .register_pure::<Increment>()
        .expect("target");
    let runtime = Runtime::new(unsupported_match.finish().expect("assembly"), store.clone());
    assert!(matches!(
        runtime
            .start(
                run(37),
                unsupported_match_program(),
                OtherValue {
                    value: "left".to_owned(),
                },
            )
            .await,
        Err(RuntimeError::IncompatibleAssembly)
    ));
    assert_eq!(store.loads.load(Ordering::SeqCst), 0);
    assert_eq!(store.appends.load(Ordering::SeqCst), 0);

    let mut manual_match = RuntimeAssemblyBuilder::new();
    manual_match
        .register_pure::<ChooseManual>()
        .expect("manual selector state");
    manual_match
        .register_pure::<IncrementBranch>()
        .expect("manual selector target");
    let runtime = Runtime::new(manual_match.finish().expect("assembly"), store.clone());
    let manual_id = run(38);
    let manual = runtime
        .start(
            manual_id.clone(),
            manual_match_program(),
            Value { value: 1 },
        )
        .await
        .expect("manual selector execution");
    let RunViewState::Succeeded(value) = manual.state() else {
        panic!("manual selector success");
    };
    assert_eq!(value.canonical_bytes(), br#"{"value":2}"#);
    assert_same_view(
        &manual,
        &runtime.read(&manual_id).await.expect("manual cold"),
    );
    assert_eq!(store.loads.load(Ordering::SeqCst), 1);
    assert_eq!(store.appends.load(Ordering::SeqCst), 3);
    store.reset();

    let mut builder = RuntimeAssemblyBuilder::new();
    builder.register_pure::<Increment>().expect("increment");
    let runtime = Runtime::new(builder.finish().expect("assembly"), store.clone());
    let id = run(36);
    let first = runtime
        .start(id.clone(), pure_program(), Value { value: 1 })
        .await
        .expect("inserted start");
    assert_eq!(first.head_sequence(), 2);
    assert_eq!(store.loads.load(Ordering::SeqCst), 0);
    assert_eq!(store.appends.load(Ordering::SeqCst), 2);

    store.reset();
    let retry = runtime
        .start(id.clone(), pure_program(), Value { value: 1 })
        .await
        .expect("exact retry");
    assert_same_view(&first, &retry);
    assert_eq!(store.loads.load(Ordering::SeqCst), 1);
    assert_eq!(store.appends.load(Ordering::SeqCst), 1);

    store.reset();
    assert!(matches!(
        empty.read(&id).await,
        Err(RuntimeError::IncompatibleAssembly)
    ));
    assert_eq!(store.loads.load(Ordering::SeqCst), 1);
    assert_eq!(store.appends.load(Ordering::SeqCst), 0);
    store.reset();
    assert!(matches!(
        empty.resume(&id).await,
        Err(RuntimeError::IncompatibleAssembly)
    ));
    assert_eq!(store.loads.load(Ordering::SeqCst), 1);
    assert_eq!(store.appends.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn association_uses_persisted_refs_without_rerunning_static_id_functions() {
    let mut builder = RuntimeAssemblyBuilder::new();
    builder
        .register_read::<AssociationCountingRead, AssociationCountingCapability>()
        .expect("counting read");
    builder
        .register_adapter::<AssociationCountingCapability, _, _>(binding(), |intent| {
            Box::pin(async move {
                Ok(Evidence {
                    value: intent.value,
                    accepted: true,
                })
            })
        })
        .expect("adapter");
    let program = association_counting_program();
    ASSOCIATION_STATE_ID_CALLS.store(0, Ordering::SeqCst);
    ASSOCIATION_CAPABILITY_ID_CALLS.store(0, Ordering::SeqCst);
    let runtime = Runtime::new(
        builder.finish().expect("assembly"),
        Arc::new(MemoryStore::new()),
    );

    let view = runtime
        .start(run(60), program, Value { value: 4 })
        .await
        .expect("associated execution");
    assert!(matches!(view.state(), RunViewState::Succeeded(_)));
    assert_eq!(ASSOCIATION_STATE_ID_CALLS.load(Ordering::SeqCst), 0);
    assert_eq!(ASSOCIATION_CAPABILITY_ID_CALLS.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn injection_hooks_run_only_during_source_authoring() {
    fn assert_authored_once(counters: &HookCounters) {
        assert_eq!(counters.before.load(Ordering::SeqCst), 1);
        assert_eq!(counters.binding.load(Ordering::SeqCst), 1);
        assert_eq!(counters.after.load(Ordering::SeqCst), 1);
    }

    let counters = Arc::new(HookCounters::default());
    let authored = expand_program(
        EntryPointId::new("mfm.test.runtime/hook-read-program@1").expect("entry"),
        &RuntimeHookRead {
            setup: HookSetup {
                binding: binding(),
                counters: Arc::clone(&counters),
            },
        },
    )
    .expect("hook-authored Program");
    assert_authored_once(&counters);

    let decoded = Program::decode_canonical(authored.canonical_bytes()).expect("decode Program");
    assert_authored_once(&counters);

    let provider_calls = Arc::new(AtomicUsize::new(0));
    let mut builder = RuntimeAssemblyBuilder::new();
    builder
        .register_pure::<HookSupport>()
        .expect("hook support State");
    builder
        .register_read::<HookRead, HookCapability>()
        .expect("hook Read State");
    builder
        .register_adapter::<HookCapability, _, _>(binding(), {
            let provider_calls = Arc::clone(&provider_calls);
            move |intent| {
                provider_calls.fetch_add(1, Ordering::SeqCst);
                Box::pin(async move {
                    Ok(Evidence {
                        value: intent.value,
                        accepted: true,
                    })
                })
            }
        })
        .expect("hook adapter");
    assert_authored_once(&counters);

    let runtime = Runtime::new(
        builder.finish().expect("hook assembly"),
        Arc::new(MemoryStore::new()),
    );
    assert_authored_once(&counters);

    let id = run(73);
    let hot = runtime
        .start(id.clone(), decoded, Value { value: 9 })
        .await
        .expect("hook Program start");
    assert_eq!(hot.head_sequence(), 4);
    assert_eq!(provider_calls.load(Ordering::SeqCst), 1);
    assert_authored_once(&counters);

    let resumed = runtime.resume(&id).await.expect("completed resume");
    assert_same_view(&hot, &resumed);
    assert_eq!(provider_calls.load(Ordering::SeqCst), 1);
    assert_authored_once(&counters);

    let cold = runtime.read(&id).await.expect("cold read");
    assert_same_view(&hot, &cold);
    assert_eq!(provider_calls.load(Ordering::SeqCst), 1);
    assert_authored_once(&counters);
}

#[tokio::test]
async fn fused_read_hot_cold_and_fail_closed_paths_are_exact() {
    let store = Arc::new(MemoryStore::new());
    let calls = Arc::new(AtomicUsize::new(0));
    let runtime = register_read_runtime(store.clone(), {
        let calls = Arc::clone(&calls);
        move |intent| {
            calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {
                Ok(Evidence {
                    value: intent.value,
                    accepted: true,
                })
            })
        }
    });
    let id = run(40);
    let hot = runtime
        .start(id.clone(), read_program(), Value { value: 7 })
        .await
        .expect("read start");
    assert_eq!(hot.head_sequence(), 2);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_same_view(&hot, &runtime.read(&id).await.expect("cold"));
    assert_eq!(calls.load(Ordering::SeqCst), 1, "read is observational");

    let failure_store = Arc::new(MemoryStore::new());
    let failure_calls = Arc::new(AtomicUsize::new(0));
    let failure_runtime = register_read_runtime(failure_store, {
        let calls = Arc::clone(&failure_calls);
        move |intent| {
            calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {
                Ok(Evidence {
                    value: intent.value,
                    accepted: false,
                })
            })
        }
    });
    let failure_id = run(39);
    let failure_hot = failure_runtime
        .start(failure_id.clone(), read_program(), Value { value: 6 })
        .await
        .expect("ordinary read failure");
    assert!(matches!(failure_hot.state(), RunViewState::Failed(_)));
    assert_same_view(
        &failure_hot,
        &failure_runtime
            .read(&failure_id)
            .await
            .expect("cold ordinary read failure"),
    );
    assert_eq!(failure_calls.load(Ordering::SeqCst), 1);

    let preparation_store = Arc::new(MemoryStore::new());
    let preparation_calls = Arc::new(AtomicUsize::new(0));
    let preparation_runtime = register_read_runtime(preparation_store.clone(), {
        let calls = Arc::clone(&preparation_calls);
        move |_| {
            calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Err(ReadAdapterError::Internal) })
        }
    });
    let preparation_id = run(41);
    assert!(matches!(
        preparation_runtime
            .start(preparation_id.clone(), read_program(), Value { value: 999 })
            .await,
        Err(RuntimeError::Internal)
    ));
    assert_eq!(preparation_calls.load(Ordering::SeqCst), 0);
    assert_eq!(retained_head(&preparation_store, &preparation_id).await, 1);

    for (byte, adapter_error, expected) in [
        (42, ReadAdapterError::Unavailable, RuntimeError::Unavailable),
        (43, ReadAdapterError::Internal, RuntimeError::Internal),
    ] {
        let store = Arc::new(MemoryStore::new());
        let runtime = register_read_runtime(store.clone(), move |_| {
            Box::pin(async move { Err(adapter_error) })
        });
        let id = run(byte);
        assert!(matches!(
            runtime.start(id.clone(), read_program(), Value { value: 1 }).await,
            Err(error) if error == expected
        ));
        assert_eq!(retained_head(&store, &id).await, 1);
    }

    let bind_store = Arc::new(MemoryStore::new());
    let bind_runtime = register_read_runtime(bind_store.clone(), |intent| {
        Box::pin(async move {
            Ok(Evidence {
                value: intent.value + 1,
                accepted: true,
            })
        })
    });
    let bind_id = run(44);
    assert!(matches!(
        bind_runtime
            .start(bind_id.clone(), read_program(), Value { value: 1 })
            .await,
        Err(RuntimeError::Internal)
    ));
    assert_eq!(retained_head(&bind_store, &bind_id).await, 1);
}

#[tokio::test]
async fn adapter_panics_are_redacted_before_any_conclusion_append() {
    let construction_store = Arc::new(MemoryStore::new());
    let construction = register_read_runtime(construction_store.clone(), |_| {
        panic!("construction payload must not escape")
    });
    let construction_id = run(45);
    let error = construction
        .start(construction_id.clone(), read_program(), Value { value: 1 })
        .await
        .err()
        .expect("construction panic");
    assert_eq!(error, RuntimeError::Internal);
    assert_eq!(error.to_string(), "runtime internal failure");
    assert_eq!(
        retained_head(&construction_store, &construction_id).await,
        1
    );

    let poll_store = Arc::new(MemoryStore::new());
    let polling = register_read_runtime(poll_store.clone(), |_| {
        Box::pin(async { panic!("poll payload must not escape") })
    });
    let poll_id = run(46);
    assert!(matches!(
        polling
            .start(poll_id.clone(), read_program(), Value { value: 1 })
            .await,
        Err(RuntimeError::Internal)
    ));
    assert_eq!(retained_head(&poll_store, &poll_id).await, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_read_observations_have_one_exact_durable_winner() {
    let store = Arc::new(MemoryStore::new());
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let calls = Arc::new(AtomicUsize::new(0));
    let make_runtime = || {
        register_read_runtime(store.clone(), {
            let barrier = Arc::clone(&barrier);
            let calls = Arc::clone(&calls);
            move |intent| {
                let barrier = Arc::clone(&barrier);
                calls.fetch_add(1, Ordering::SeqCst);
                Box::pin(async move {
                    barrier.wait().await;
                    Ok(Evidence {
                        value: intent.value,
                        accepted: true,
                    })
                })
            }
        })
    };
    let left = make_runtime();
    let right = make_runtime();
    let id = run(47);
    let (left, right) = tokio::join!(
        left.start(id.clone(), read_program(), Value { value: 4 }),
        right.start(id.clone(), read_program(), Value { value: 4 })
    );
    let left = left.expect("left");
    let right = right.expect("right");
    assert_same_view(&left, &right);
    assert_eq!(left.head_sequence(), 2);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(retained_head(&store, &id).await, 2);
}

#[tokio::test]
async fn cancellation_at_provider_await_leaves_only_the_acknowledged_prefix() {
    let store = Arc::new(MemoryStore::new());
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let runtime = Arc::new(register_read_runtime(store.clone(), {
        let entered = Arc::clone(&entered);
        let release = Arc::clone(&release);
        move |intent| {
            let entered = Arc::clone(&entered);
            let release = Arc::clone(&release);
            Box::pin(async move {
                entered.notify_one();
                release.notified().await;
                Ok(Evidence {
                    value: intent.value,
                    accepted: true,
                })
            })
        }
    }));
    let id = run(48);
    let task = {
        let runtime = Arc::clone(&runtime);
        let id = id.clone();
        tokio::spawn(async move { runtime.start(id, read_program(), Value { value: 4 }).await })
    };
    entered.notified().await;
    task.abort();
    match task.await {
        Err(error) => assert!(error.is_cancelled()),
        Ok(_) => panic!("provider task was not cancelled"),
    }
    release.notify_waiters();
    let view = runtime.read(&id).await.expect("durable prefix");
    assert_eq!(view.head_sequence(), 1);
    assert!(matches!(view.state(), RunViewState::Runnable));
}

#[test]
fn cancellation_before_a_queued_blocking_job_performs_no_store_io() {
    let executor = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .max_blocking_threads(1)
        .enable_all()
        .build()
        .expect("test executor");
    executor.block_on(async {
        let blocker_entered = Arc::new(tokio::sync::Notify::new());
        let blocker_release = Arc::new(AtomicBool::new(false));
        let blocker = tokio::task::spawn_blocking({
            let entered = Arc::clone(&blocker_entered);
            let release = Arc::clone(&blocker_release);
            move || {
                entered.notify_one();
                while !release.load(Ordering::SeqCst) {
                    std::thread::yield_now();
                }
            }
        });
        blocker_entered.notified().await;

        let memory = Arc::new(MemoryStore::new());
        let counted = Arc::new(CountingStore::new(memory.clone()));
        let mut builder = RuntimeAssemblyBuilder::new();
        builder.register_pure::<Increment>().expect("increment");
        let runtime = Arc::new(Runtime::new(
            builder.finish().expect("assembly"),
            counted.clone(),
        ));
        let id = run(59);
        let queued = Arc::new(tokio::sync::Notify::new());
        let task = {
            let runtime = Arc::clone(&runtime);
            let id = id.clone();
            let queued = Arc::clone(&queued);
            tokio::spawn(async move {
                let mut start = Box::pin(runtime.start(id, pure_program(), Value { value: 4 }));
                std::future::poll_fn(|context| match start.as_mut().poll(context) {
                    std::task::Poll::Pending => {
                        queued.notify_one();
                        std::task::Poll::Ready(())
                    }
                    std::task::Poll::Ready(_) => {
                        panic!("blocking admission unexpectedly completed")
                    }
                })
                .await;
                start.await
            })
        };
        queued.notified().await;
        assert_eq!(counted.loads.load(Ordering::SeqCst), 0);
        assert_eq!(counted.appends.load(Ordering::SeqCst), 0);
        task.abort();
        match task.await {
            Err(error) => assert!(error.is_cancelled()),
            Ok(_) => panic!("queued blocking task was not cancelled"),
        }

        blocker_release.store(true, Ordering::SeqCst);
        blocker.await.expect("release blocking lane");
        tokio::task::spawn_blocking(|| {})
            .await
            .expect("drain blocking lane");
        assert_eq!(counted.loads.load(Ordering::SeqCst), 0);
        assert_eq!(counted.appends.load(Ordering::SeqCst), 0);
        assert!(memory.load_run(&id).await.expect("memory load").is_none());
    });
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancellation_during_blocking_state_evaluation_cannot_start_dependent_io() {
    let signals = Arc::new(BlockingSignals {
        entered: tokio::sync::Notify::new(),
        finished: tokio::sync::Notify::new(),
        release: AtomicBool::new(false),
        evaluations: AtomicUsize::new(0),
    });
    let slot = BLOCKING_SIGNALS.get_or_init(|| Mutex::new(None));
    *slot.lock().expect("signals lock") = Some(Arc::clone(&signals));

    let memory = Arc::new(MemoryStore::new());
    let counted = Arc::new(CountingStore::new(memory.clone()));
    let mut builder = RuntimeAssemblyBuilder::new();
    builder
        .register_pure::<BlockingIncrement>()
        .expect("blocking state");
    let runtime = Arc::new(Runtime::new(
        builder.finish().expect("assembly"),
        counted.clone(),
    ));
    let id = run(54);
    let task = {
        let runtime = Arc::clone(&runtime);
        let id = id.clone();
        tokio::spawn(async move {
            runtime
                .start(id, blocking_program(), Value { value: 4 })
                .await
        })
    };
    signals.entered.notified().await;
    assert_eq!(counted.appends.load(Ordering::SeqCst), 1);
    task.abort();
    match task.await {
        Err(error) => assert!(error.is_cancelled()),
        Ok(_) => panic!("blocking evaluation task was not cancelled"),
    }
    signals.release.store(true, Ordering::SeqCst);
    signals.finished.notified().await;
    tokio::task::yield_now().await;

    assert_eq!(counted.appends.load(Ordering::SeqCst), 1);
    assert_eq!(retained_head(&memory, &id).await, 1);
    let resumed = runtime
        .resume(&id)
        .await
        .expect("resume after cancellation");
    assert_eq!(signals.evaluations.load(Ordering::SeqCst), 2);
    assert_eq!(resumed.head_sequence(), 2);
    assert!(matches!(resumed.state(), RunViewState::Succeeded(_)));
    *slot.lock().expect("signals lock") = None;
}

struct BlockingSecondAppendStore {
    inner: Arc<MemoryStore>,
    appends: AtomicUsize,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}

struct CommitThenBlockAcknowledgementStore {
    inner: Arc<MemoryStore>,
    appends: AtomicUsize,
    committed: tokio::sync::Notify,
    release: tokio::sync::Notify,
}

impl Store for CommitThenBlockAcknowledgementStore {
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
    ) -> Pin<Box<dyn Future<Output = std::result::Result<AppendResult, StoreError>> + Send + 'a>>
    {
        Box::pin(async move {
            let result = self.inner.append_run(frame).await?;
            if self.appends.fetch_add(1, Ordering::SeqCst) + 1 == 2
                && result == AppendResult::Inserted
            {
                self.committed.notify_one();
                self.release.notified().await;
            }
            Ok(result)
        })
    }
}

impl Store for BlockingSecondAppendStore {
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
    ) -> Pin<Box<dyn Future<Output = std::result::Result<AppendResult, StoreError>> + Send + 'a>>
    {
        Box::pin(async move {
            if self.appends.fetch_add(1, Ordering::SeqCst) + 1 == 2 {
                self.entered.notify_one();
                self.release.notified().await;
            }
            self.inner.append_run(frame).await
        })
    }
}

#[tokio::test]
async fn cancellation_at_append_await_cannot_publish_the_candidate() {
    let memory = Arc::new(MemoryStore::new());
    let store = Arc::new(BlockingSecondAppendStore {
        inner: memory.clone(),
        appends: AtomicUsize::new(0),
        entered: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
    });
    let mut builder = RuntimeAssemblyBuilder::new();
    builder.register_pure::<Increment>().expect("increment");
    let runtime = Arc::new(Runtime::new(
        builder.finish().expect("assembly"),
        store.clone(),
    ));
    let id = run(49);
    let task = {
        let runtime = Arc::clone(&runtime);
        let id = id.clone();
        tokio::spawn(async move { runtime.start(id, pure_program(), Value { value: 4 }).await })
    };
    store.entered.notified().await;
    task.abort();
    match task.await {
        Err(error) => assert!(error.is_cancelled()),
        Ok(_) => panic!("append task was not cancelled"),
    }
    store.release.notify_waiters();
    assert_eq!(retained_head(&memory, &id).await, 1);
    let view = runtime.read(&id).await.expect("prefix");
    assert!(matches!(view.state(), RunViewState::Runnable));
}

#[tokio::test]
async fn cancellation_after_atomic_append_commit_only_withholds_acknowledgement() {
    let memory = Arc::new(MemoryStore::new());
    let store = Arc::new(CommitThenBlockAcknowledgementStore {
        inner: memory.clone(),
        appends: AtomicUsize::new(0),
        committed: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
    });
    let mut builder = RuntimeAssemblyBuilder::new();
    builder.register_pure::<Increment>().expect("increment");
    let runtime = Arc::new(Runtime::new(
        builder.finish().expect("assembly"),
        store.clone(),
    ));
    let id = run(55);
    let task = {
        let runtime = Arc::clone(&runtime);
        let id = id.clone();
        tokio::spawn(async move { runtime.start(id, pure_program(), Value { value: 4 }).await })
    };
    store.committed.notified().await;
    task.abort();
    match task.await {
        Err(error) => assert!(error.is_cancelled()),
        Ok(_) => panic!("append acknowledgement task was not cancelled"),
    }
    store.release.notify_waiters();

    assert_eq!(retained_head(&memory, &id).await, 2);
    let view = runtime.read(&id).await.expect("committed conclusion");
    assert_eq!(view.head_sequence(), 2);
    assert!(matches!(view.state(), RunViewState::Succeeded(_)));
}

struct CommitThenIndeterminateStore {
    inner: Arc<MemoryStore>,
}

struct AbsentIndeterminateConclusionStore {
    inner: Arc<MemoryStore>,
    conclusion_attempts: AtomicUsize,
}

impl Store for AbsentIndeterminateConclusionStore {
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
    ) -> Pin<Box<dyn Future<Output = std::result::Result<AppendResult, StoreError>> + Send + 'a>>
    {
        Box::pin(async move {
            if frame.run_sequence() > 1
                && self.conclusion_attempts.fetch_add(1, Ordering::SeqCst) == 0
            {
                return Err(StoreError::Indeterminate);
            }
            self.inner.append_run(frame).await
        })
    }
}

impl Store for CommitThenIndeterminateStore {
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
    ) -> Pin<Box<dyn Future<Output = std::result::Result<AppendResult, StoreError>> + Send + 'a>>
    {
        Box::pin(async move {
            let result = self.inner.append_run(frame).await?;
            if frame.run_sequence() > 1 && result == AppendResult::Inserted {
                Err(StoreError::Indeterminate)
            } else {
                Ok(result)
            }
        })
    }
}

#[tokio::test]
async fn an_earlier_view_remains_a_snapshot_after_indeterminate_commit() {
    let memory = Arc::new(MemoryStore::new());
    let unavailable = register_read_runtime(memory.clone(), |_| {
        Box::pin(async { Err(ReadAdapterError::Unavailable) })
    });
    let id = run(50);
    assert!(matches!(
        unavailable
            .start(id.clone(), read_program(), Value { value: 8 })
            .await,
        Err(RuntimeError::Unavailable)
    ));
    let earlier = unavailable.read(&id).await.expect("earlier snapshot");
    assert_eq!(earlier.head_sequence(), 1);

    let ambiguous = register_read_runtime(
        Arc::new(CommitThenIndeterminateStore {
            inner: memory.clone(),
        }),
        |intent| {
            Box::pin(async move {
                Ok(Evidence {
                    value: intent.value,
                    accepted: true,
                })
            })
        },
    );
    assert!(matches!(
        ambiguous.resume(&id).await,
        Err(RuntimeError::Indeterminate)
    ));
    assert_eq!(earlier.head_sequence(), 1);
    assert!(matches!(earlier.state(), RunViewState::Runnable));

    let later = unavailable.read(&id).await.expect("later snapshot");
    assert_eq!(later.head_sequence(), 2);
    assert!(matches!(later.state(), RunViewState::Succeeded(_)));
}

#[tokio::test]
async fn absent_indeterminate_pure_conclusion_recomputes_and_commits_on_resume() {
    PURE_RETRY_EVALUATIONS.store(0, Ordering::SeqCst);
    let memory = Arc::new(MemoryStore::new());
    let store = Arc::new(AbsentIndeterminateConclusionStore {
        inner: memory,
        conclusion_attempts: AtomicUsize::new(0),
    });
    let mut builder = RuntimeAssemblyBuilder::new();
    builder
        .register_pure::<CountingRetryIncrement>()
        .expect("counting pure");
    let runtime = Runtime::new(builder.finish().expect("assembly"), store);
    let id = run(58);

    assert!(matches!(
        runtime
            .start(id.clone(), counting_retry_program(), Value { value: 4 })
            .await,
        Err(RuntimeError::Indeterminate)
    ));
    assert_eq!(PURE_RETRY_EVALUATIONS.load(Ordering::SeqCst), 1);
    let prefix = runtime.read(&id).await.expect("genesis prefix");
    assert_eq!(prefix.head_sequence(), 1);
    assert!(matches!(prefix.state(), RunViewState::Runnable));

    let resumed = runtime.resume(&id).await.expect("resumed conclusion");
    assert_eq!(PURE_RETRY_EVALUATIONS.load(Ordering::SeqCst), 2);
    assert_eq!(resumed.head_sequence(), 2);
    assert!(matches!(resumed.state(), RunViewState::Succeeded(_)));
    assert_same_view(&resumed, &runtime.read(&id).await.expect("cold final"));
}

#[tokio::test]
async fn cold_read_binding_failure_is_invalid_history_without_adapter_io() {
    let id = run(51);
    let program = read_program();
    let input = Value { value: 4 };
    let (input_bytes, input_ref) = canonicalize_mfm_value(&input).expect("input");
    let genesis = EncodedRunFrame::admission(
        &id,
        program.content_ref(),
        program.canonical_bytes(),
        &input_ref,
        input_bytes.as_bytes(),
    )
    .expect("genesis");
    let genesis_for_store = EncodedRunFrame::admission(
        &id,
        program.content_ref(),
        program.canonical_bytes(),
        &input_ref,
        input_bytes.as_bytes(),
    )
    .expect("stored genesis");
    let history = JournalHistory::from_genesis(genesis).expect("history");
    let intent = Intent { value: 4 };
    let evidence = Evidence {
        value: 5,
        accepted: true,
    };
    let outcome = Value { value: 4 };
    let (intent_bytes, intent_ref) = canonicalize_mfm_value(&intent).expect("intent");
    let (evidence_bytes, evidence_ref) = canonicalize_mfm_value(&evidence).expect("evidence");
    let (outcome_bytes, outcome_ref) = canonicalize_mfm_value(&outcome).expect("outcome");
    let conclusion = history
        .encode_read_conclusion(
            &intent_ref,
            intent_bytes.as_bytes(),
            &evidence_ref,
            evidence_bytes.as_bytes(),
            OutcomeKind::Success,
            &outcome_ref,
            outcome_bytes.as_bytes(),
        )
        .expect("conclusion");
    let store = Arc::new(MemoryStore::new());
    assert_eq!(
        store
            .append_run(&genesis_for_store)
            .await
            .expect("append genesis"),
        AppendResult::Inserted
    );
    assert_eq!(
        store
            .append_run(&conclusion)
            .await
            .expect("append conclusion"),
        AppendResult::Inserted
    );
    let calls = Arc::new(AtomicUsize::new(0));
    let runtime = register_read_runtime(store, {
        let calls = Arc::clone(&calls);
        move |_| {
            calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Err(ReadAdapterError::Internal) })
        }
    });
    assert!(matches!(
        runtime.read(&id).await,
        Err(RuntimeError::InvalidHistory)
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn valid_foreign_genesis_collision_precedes_foreign_assembly_association() {
    let id = run(56);
    let foreign = foreign_zero_program();
    let foreign_value = OtherValue {
        value: "foreign".to_owned(),
    };
    let (foreign_bytes, foreign_ref) =
        canonicalize_mfm_value(&foreign_value).expect("foreign value");
    let genesis = EncodedRunFrame::admission(
        &id,
        foreign.content_ref(),
        foreign.canonical_bytes(),
        &foreign_ref,
        foreign_bytes.as_bytes(),
    )
    .expect("foreign genesis");
    let store = Arc::new(MemoryStore::new());
    assert_eq!(
        store.append_run(&genesis).await.expect("append foreign"),
        AppendResult::Inserted
    );

    let mut builder = RuntimeAssemblyBuilder::new();
    builder.register_pure::<Increment>().expect("increment");
    let runtime = Runtime::new(builder.finish().expect("assembly"), store);
    assert!(matches!(
        runtime.start(id, pure_program(), Value { value: 1 }).await,
        Err(RuntimeError::AdmissionConflict)
    ));
}

struct FixedHistoryStore {
    frames: Vec<Vec<u8>>,
}

impl Store for FixedHistoryStore {
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
        let frames = self.frames.clone();
        Box::pin(async move {
            Ok(Some(
                StoredRunBytes::new(frames).expect("bounded fixed history"),
            ))
        })
    }

    fn append_run<'a>(
        &'a self,
        _frame: &'a EncodedRunFrame,
    ) -> Pin<Box<dyn Future<Output = std::result::Result<AppendResult, StoreError>> + Send + 'a>>
    {
        Box::pin(async { Err(StoreError::Unavailable) })
    }
}

#[tokio::test]
async fn a_store_history_for_the_wrong_requested_run_id_is_invalid() {
    let retained_id = run(57);
    let program = zero_program();
    let value = Value { value: 3 };
    let (bytes, value_ref) = canonicalize_mfm_value(&value).expect("value");
    let genesis = EncodedRunFrame::admission(
        &retained_id,
        program.content_ref(),
        program.canonical_bytes(),
        &value_ref,
        bytes.as_bytes(),
    )
    .expect("genesis");
    let store = Arc::new(FixedHistoryStore {
        frames: vec![genesis.canonical_bytes().to_vec()],
    });
    let mut builder = RuntimeAssemblyBuilder::new();
    builder.register_value::<Value>().expect("value codec");
    let runtime = Runtime::new(builder.finish().expect("assembly"), store);
    let requested_id = run(58);
    assert!(matches!(
        runtime.read(&requested_id).await,
        Err(RuntimeError::InvalidHistory)
    ));
    assert!(matches!(
        runtime.resume(&requested_id).await,
        Err(RuntimeError::InvalidHistory)
    ));
}

struct NotInsertedAbsentStore;

impl Store for NotInsertedAbsentStore {
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
        Box::pin(async { Ok(None) })
    }

    fn append_run<'a>(
        &'a self,
        _frame: &'a EncodedRunFrame,
    ) -> Pin<Box<dyn Future<Output = std::result::Result<AppendResult, StoreError>> + Send + 'a>>
    {
        Box::pin(async { Ok(AppendResult::NotInserted) })
    }
}

struct InsertThenNotInsertedAbsentStore {
    appends: AtomicUsize,
}

impl Store for InsertThenNotInsertedAbsentStore {
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
        Box::pin(async { Ok(None) })
    }

    fn append_run<'a>(
        &'a self,
        _frame: &'a EncodedRunFrame,
    ) -> Pin<Box<dyn Future<Output = std::result::Result<AppendResult, StoreError>> + Send + 'a>>
    {
        Box::pin(async move {
            Ok(if self.appends.fetch_add(1, Ordering::SeqCst) == 0 {
                AppendResult::Inserted
            } else {
                AppendResult::NotInserted
            })
        })
    }
}

#[tokio::test]
async fn not_inserted_followed_by_absence_is_internal() {
    let mut builder = RuntimeAssemblyBuilder::new();
    builder.register_value::<Value>().expect("value");
    let runtime = Runtime::new(
        builder.finish().expect("assembly"),
        Arc::new(NotInsertedAbsentStore),
    );
    assert!(matches!(
        runtime
            .start(run(52), zero_program(), Value { value: 1 })
            .await,
        Err(RuntimeError::Internal)
    ));

    let mut builder = RuntimeAssemblyBuilder::new();
    builder.register_pure::<Increment>().expect("increment");
    let runtime = Runtime::new(
        builder.finish().expect("assembly"),
        Arc::new(InsertThenNotInsertedAbsentStore {
            appends: AtomicUsize::new(0),
        }),
    );
    assert!(matches!(
        runtime
            .start(run(53), pure_program(), Value { value: 1 })
            .await,
        Err(RuntimeError::Internal)
    ));
}

#[derive(Clone, Copy)]
enum Failure {
    Capacity,
    Corrupt,
    Unavailable,
    Indeterminate,
}

struct FaultStore {
    failure: Failure,
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
        Box::pin(async move { Err(store_error(self.failure)) })
    }

    fn append_run<'a>(
        &'a self,
        _frame: &'a EncodedRunFrame,
    ) -> Pin<Box<dyn Future<Output = std::result::Result<AppendResult, StoreError>> + Send + 'a>>
    {
        Box::pin(async move { Err(store_error(self.failure)) })
    }
}

fn store_error(failure: Failure) -> StoreError {
    match failure {
        Failure::Capacity => StoreError::Capacity,
        Failure::Corrupt => StoreError::CorruptPhysicalState,
        Failure::Unavailable => StoreError::Unavailable,
        Failure::Indeterminate => StoreError::Indeterminate,
    }
}

fn runtime_with_fault(failure: Failure) -> Runtime {
    let mut builder = RuntimeAssemblyBuilder::new();
    builder.register_value::<Value>().expect("value");
    Runtime::new(
        builder.finish().expect("assembly"),
        Arc::new(FaultStore { failure }),
    )
}

#[tokio::test]
async fn store_error_mapping_is_operation_specific_and_exhaustive() {
    let cases = [
        (
            Failure::Capacity,
            RuntimeError::Internal,
            RuntimeError::Capacity,
        ),
        (
            Failure::Corrupt,
            RuntimeError::InvalidHistory,
            RuntimeError::InvalidHistory,
        ),
        (
            Failure::Unavailable,
            RuntimeError::Unavailable,
            RuntimeError::Unavailable,
        ),
        (
            Failure::Indeterminate,
            RuntimeError::Internal,
            RuntimeError::Indeterminate,
        ),
    ];
    for (offset, (failure, load_expected, append_expected)) in cases.into_iter().enumerate() {
        let runtime = runtime_with_fault(failure);
        let id = run(u8::try_from(offset + 10).expect("byte"));
        assert!(matches!(
            runtime.read(&id).await,
            Err(error) if error == load_expected
        ));
        assert!(matches!(
            runtime.start(id, zero_program(), Value { value: 1 }).await,
            Err(error) if error == append_expected
        ));
    }
}
