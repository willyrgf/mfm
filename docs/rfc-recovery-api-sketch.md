# Recoverable Runtime: API sketch and engineer handoff

This appendix belongs to the proposed [linear recovery RFC](../RFC_REFACT_RUNTIME_TO_RECOV_SM.md).
It specifies the target API shape; it does not introduce another implementation or supersede the
current [design](design.md) before the complete cutover. Type names below are proposed names.
Existing checked IDs, value codecs, canonicalization, async adapter callbacks, and mechanical Store
interfaces remain the foundation. Private representation and exhaustive rustdoc are omitted from
signature sketches.

The engineer should implement one current design. Do not introduce an experimental Runtime mode,
public erased error workflow, compatibility decoder, or generic policy-instantiation service.

## Capability errors and State context

Keep the existing `State::{Input, Output, Failure, state_id}`, `PureState::evaluate`, and Read/Effect
`prepare` and `interpret` contracts. Add a capability-owned operational error type and a
State-owned context type for adapter incidents. Keep invariant errors outside that recoverable
alternative.

```rust,ignore
// mfm-capabilities
pub enum AdapterError<E> {
    Operational(E),
    Invariant(AdapterInvariantError),
}

pub trait ReadCapabilityContract: Send + Sync + 'static {
    type Intent: MfmValue;
    type Evidence: MfmValue;
    type OperationalError: MfmValue;
    // Existing contract_id and bind_evidence signatures remain.
}

pub trait EffectCapabilityContract: Send + Sync + 'static {
    type Command: MfmValue;
    type Evidence: MfmValue;
    type OperationalError: MfmValue;
    // Existing contract_id and bind_evidence signatures remain.
}

// mfm-program; additions to the existing mode traits
pub trait ReadState<C: ReadCapabilityContract>: State {
    type AdapterContext: MfmValue;

    fn adapter_context(
        input: &Self::Input,
        intent: &C::Intent,
        error: &C::OperationalError,
    ) -> Result<Self::AdapterContext, StateExecutionError>;
    // Existing prepare and interpret signatures remain.
}

pub trait EffectState<C: EffectCapabilityContract>: State {
    type AdapterContext: MfmValue;

    fn adapter_context(
        input: &Self::Input,
        command: &C::Command,
        error: &C::OperationalError,
    ) -> Result<Self::AdapterContext, StateExecutionError>;
    // Existing prepare and interpret signatures remain.
}

pub enum Incident<D, E, X> {
    Domain(D),
    Adapter { original: E, context: X },
}

pub trait IncidentContract: private::Sealed + Send + Sync + 'static {
    type Domain: MfmValue;
    type Error: MfmValue;
    type Context: MfmValue;
}
// Implemented by Program only for Incident<D, E, X> with checked component types.
```

`AdapterInvariantError` is one redaction-safe error type in Capabilities. It replaces the current
Runtime-owned adapter `Internal` variant, not a new provider failure vocabulary. Operational error
values are checked, bounded, domain/capability-defined data; use the existing value derivation and
reviewed error implementations. They contain no raw RPC body, URL, exception chain, or secret.
Add the exact operational-error schema to capability identity/association, and the exact context
schema to State mode identity/association. Changing context semantics changes the State
implementation identity just as changing interpretation does.

Pure incidents use `Incident<S::Failure, Never, NoContext>`; the impossible adapter alternative is
never encoded. `NoContext` and `NoParams` below are checked framework unit values, not ambient
configuration maps. A domain failure already carries its domain context in `S::Failure`; it needs
no second augmentation callback. Only operational adapter errors invoke `adapter_context`.

`IncidentContract` exposes structural component types for exact ABI construction. It is sealed
because the incident always has those two alternatives; downstream authors extend its typed
components and policy implementations. It introduces no value erasure or dynamic introspection.
The transient incident wrapper itself does not implement a new persisted value schema.

Runtime retains the original error and wraps the returned context. The callback cannot replace
that error, declare an Effect settled, or change visit/budget facts. A failed callback is an internal
invocation stop. There is no `Send` or `Sync` change to keystore/signer custody.

## Classification, mapping, and handling

Both callback traits and the small shared recovery vocabulary belong in Program. Reuse
`StateExecutionError` for failure of trusted deterministic callbacks; callers see the fixed Runtime
internal-error boundary, not recursively classified policy errors.

```rust,ignore
#[derive(Clone, Copy)]
pub enum Assessment { Recoverable, Nonrecoverable }

pub trait Classifier<I: IncidentContract>: Send + Sync + 'static {
    type Params: MfmValue;
    fn implementation_id() -> Result<StableId, ProgramError>;
    fn classify(
        params: &Self::Params,
        incident: &I,
        context: &RecoveryContext<'_>,
    ) -> Result<Assessment, StateExecutionError>;
}

pub trait Handler<I: IncidentContract>: Send + Sync + 'static {
    type Params: MfmValue;
    fn implementation_id() -> Result<StableId, ProgramError>;
    fn handle(
        params: &Self::Params,
        incident: &I,
        assessment: Assessment,
        context: &RecoveryContext<'_>,
    ) -> Result<RecoveryRequest, StateExecutionError>;
}

pub trait ValueMap: Send + Sync + 'static {
    type Input: MfmValue;
    type Output: MfmValue;
    type Params: MfmValue;
    fn implementation_id() -> Result<StableId, ProgramError>;
    fn apply(
        params: &Self::Params,
        value: Self::Input,
    ) -> Result<Self::Output, StateExecutionError>;
}

pub enum RecoveryRequest {
    RetryState,
    Restart(RecoveryTarget),
    Stop,
}

#[derive(Clone, Copy)]
pub enum ExecutionPhase {
    Pure,
    Read,
    EffectPending,
    EffectSettled,
}

pub enum StopReason {
    Nonrecoverable,
    Requested,
    Exhausted(RecoveryLimit),
    Disallowed(RecoveryDenial),
}

pub enum RecoveryLimit { StateRetry, StateRestart, Run }
pub enum RecoveryDenial { PureRetry, CheckpointUnavailable, EffectBarrier, EffectSettled }

impl RecoveryContext<'_> {
    pub fn phase(&self) -> ExecutionPhase;
    pub fn remaining(&self) -> RecoveryAllowances;
    pub fn remaining_run_decisions(&self) -> u32;
    pub fn eligible_restart_targets(&self) -> &[RecoveryTarget];
}

impl<'a> RecoveryContext<'a> {
    pub fn new(
        phase: ExecutionPhase,
        remaining: RecoveryAllowances,
        remaining_run_decisions: u32,
        eligible: &'a [RecoveryTarget],
    ) -> Self;
}
```

Mapping consumes its typed input. `Identity<T>` can return that input directly without imposing
`Clone` on `MfmValue`; `FromNever<T>` has no reachable input. Runtime qualifies and retains the
original canonical incident before applying transient policy mappings. Only the original domain
failure or State context is passed to a mapper. Framework composition carries the adapter error
unchanged, structurally:

```rust,ignore
fn map_incident<E, DM: ValueMap, XM: ValueMap>(
    incident: Incident<DM::Input, E, XM::Input>,
    domain_params: &DM::Params,
    context_params: &XM::Params,
) -> Result<Incident<DM::Output, E, XM::Output>, StateExecutionError> {
    Ok(match incident {
        Incident::Domain(value) => Incident::Domain(DM::apply(domain_params, value)?),
        Incident::Adapter { original, context } => Incident::Adapter {
            original,
            context: XM::apply(context_params, context)?,
        },
    })
}
```

The selected classifier and handler borrow the resulting typed incident. Assessments and mapped
policy inputs are transient. On domain Stop, an explicit root-failure mapping path produces the
root value from the retained original; no root mapping runs for an adapter stop. The same mapper
implementation may appear in classification and root reporting, but the paths have distinct
purposes. Each is declared in Program and associated once, with no conversion search.

Incident binding keys use the exact component schema refs `(domain failure, operational error,
State context)`, the mapping input/output/parameter refs, and implementation identities. Existing
generic value descriptors may describe the transient incident, but do not serialize an additional
assessment or policy-facing incident into the frame. Reuse existing codecs for its components.

`RecoveryContext` contains inspection data, not authority. Its public constructor lets downstream
authors test pure policies without Runtime. Runtime constructs every context used in execution
from its own associated Program and reconstructed position, and exposes no API accepting a
caller-supplied context or recovery decision. It always validates the callback's request.

## Finite typed defaults and occurrence overrides

Keep `Operation::expand(&mut OperationExpansion<Input, Output, Failure>)` non-generic over policy
types. A Rust generic handler cannot be instantiated later from an arbitrary runtime type ID.
Instead, an author supplies a finite checked family of the exact generic instantiations supported
by that setting. The family contains descriptors and immutable parameters, not executable callbacks
or a registry service. Only the selected descriptor is lowered into each State declaration.

```rust,ignore
impl Classifiers {
    pub fn new() -> Self;
    pub fn bind<E, DM, XM, K>(
        &mut self,
        domain_params: DM::Params,
        context_params: XM::Params,
        classifier_params: K::Params,
    ) -> Result<(), ProgramError>
    where
        E: MfmValue,
        DM: ValueMap,
        XM: ValueMap,
        K: Classifier<Incident<DM::Output, E, XM::Output>>;
}

impl Handlers {
    pub fn new() -> Self;
    pub fn bind<I: IncidentContract, H: Handler<I>>(
        &mut self,
        params: H::Params,
    ) -> Result<(), ProgramError>;

    pub fn checkpoint<I: IncidentContract, T: MfmValue>(
        &mut self,
        checkpoint: &Checkpoint<T>,
    ) -> Result<(), ProgramError>;
}

impl<I: MfmValue, O: MfmValue, F: MfmValue> OperationExpansion<I, O, F> {
    pub fn classifiers(&mut self, family: Classifiers) -> Result<(), ProgramError>;
    pub fn handlers(&mut self, family: Handlers) -> Result<(), ProgramError>;
    pub fn allowances(&mut self, limits: RecoveryAllowances) -> Result<(), ProgramError>;

    pub fn pure<S, M>(
        &mut self, root_map: M::Params, policy: Occurrence, bound: ConclusionBound,
    )
        -> Result<(), ProgramError>
    where S: PureState, M: ValueMap<Input = S::Failure, Output = F>;

    pub fn read<S, C, M>(
        &mut self, setup: &<C as CapabilityInjection<S>>::Setup, root_map: M::Params,
        policy: Occurrence, bound: ConclusionBound,
    ) -> Result<(), ProgramError>
    where C: ReadCapabilityContract + CapabilityInjection<S>, S: ReadState<C>,
          M: ValueMap<Input = <C as CapabilityInjection<S>>::ExpandedFailure, Output = F>;

    pub fn effect<S, C, M>(
        &mut self, setup: &<C as CapabilityInjection<S>>::Setup, root_map: M::Params,
        policy: Occurrence, bounds: EffectBounds,
    ) -> Result<(), ProgramError>
    where C: EffectCapabilityContract + CapabilityInjection<S>, S: EffectState<C>,
          M: ValueMap<Input = <C as CapabilityInjection<S>>::ExpandedFailure, Output = F>;

    pub fn operation<C, M>(&mut self, child: &C, root_map: M::Params)
        -> Result<(), ProgramError>
    where C: Operation, M: ValueMap<Input = C::Failure, Output = F>;
}
```

`Occurrence` is a checked authoring-only builder with optional classifier family, handler family,
retry allowance, and restart allowance overrides. Missing options inherit; explicit zero overrides
an inherited allowance. When an Operation explicitly replaces a classifier or handler family,
that family must cover the selected exact input; a missing entry is an error, not a fallback to an
outer family. Reject duplicate source bindings and handlers for duplicate input contracts.

The framework's initial no-recovery defaults are installed for each concrete State/capability
incident during typed authoring and State registration. This needs no dynamic generic
instantiation. User-defined families then follow ordinary nearest-explicit-setting precedence.
Global allowance is a separate checked Program-admission parameter and cannot be overridden by
child scopes. Adjacent State input/output equality and every mapping ABI are checked before
producing Program; the `OperationExpansion` type parameters do not themselves prove each adjacent
boundary.

For example, a Portfolio handler family explicitly binds generic `Stop` for Portfolio incidents
and EVM incidents. A child may replace only its classifier family with `EvmClassifier` and still
inherit that handler. One Read occurrence can replace only its handler with `RetryRead`. A second
child that supplies no defaults selects the parent's explicit EVM-to-Portfolio classifier binding.
See the [executable signature proof](examples/recovery-api-signatures.rs) for all four selections.

`operation<C, M>` composes the child's explicit local root mapping paths with `M`. Those are
bounded exact paths of already registered mapper implementations; State registration does not
need to know every parent Operation. This preserves typed root failures without retaining graph
failure handlers or mapping States.

Read and Effect emission preserve the existing typed capability-injection setup. The compiler
resolves the designated binding from that setup once and expands before/designated/after through
the same scope machinery. A raw binding reference cannot replace the setup contract for injected
transaction States.

Injection owns the designated State's explicit root conversion into `ExpandedFailure`:

```rust,ignore
pub trait CapabilityInjection<S: State> {
    // Existing setup, expanded input/output/failure and hook contracts remain.
    type FailureMap: ValueMap<Input = S::Failure, Output = Self::ExpandedFailure>;
    fn failure_map_params(setup: &Self::Setup)
        -> Result<<Self::FailureMap as ValueMap>::Params, ProgramError>;
}
```

The designated root path is `S::Failure -> ExpandedFailure -> F`. Hook States already map
their local failures into `ExpandedFailure`, then receive the same caller map into `F`.
Transaction injection uses `FromNever<ExpandedFailure>` for its infallible designated State;
unchanged-failure injection uses `Identity<ExpandedFailure>`. This is root reporting only:
classification still begins with each State's original incident.

## Checkpoints, limits, and immutable declarations

```rust,ignore
pub struct Checkpoint<T> { /* private scope ID, draft boundary, PhantomData<T> */ }
#[derive(Clone, Copy)]
pub struct RecoveryTarget { /* private checked StatePosition */ }

impl<I: MfmValue, O: MfmValue, F: MfmValue> OperationExpansion<I, O, F> {
    pub fn checkpoint<T: MfmValue>(&mut self) -> Result<Checkpoint<T>, ProgramError>;
}

impl RecoveryTarget {
    pub fn position(&self) -> StatePosition;
}

impl StateDeclaration {
    pub fn recovery_targets(&self) -> &[RecoveryTarget];
}

pub struct RecoveryAllowances { /* private u32 retries and restarts */ }
impl RecoveryAllowances {
    pub const fn new(retries: u32, restarts: u32) -> Self;
    pub fn retries(&self) -> u32;
    pub fn restarts(&self) -> u32;
}

pub struct ProgramLimits { /* private global recovery-decision allowance */ }
impl ProgramLimits {
    pub const fn new(max_recovery_decisions: u32) -> Self;
}

pub struct ConclusionBound { /* private maximum complete-frame bytes */ }
impl ConclusionBound {
    pub fn new(max_frame_bytes: u64) -> Result<Self, ProgramError>;
}

pub struct EffectBounds { /* private prepare and conclusion maximum frame bytes */ }
impl EffectBounds {
    pub fn new(prepare_bytes: u64, conclusion_bytes: u64) -> Result<Self, ProgramError>;
}

pub fn expand_program<O: Operation>(
    entry_point: EntryPointId,
    operation: &O,
    input: &O::Input,
    limits: ProgramLimits,
) -> Result<Program, ProgramError>;

pub struct RecoveryUsage {
    pub state_retries: u32,
    pub state_restarts: u32,
    pub run_decisions: u32,
}
```

Creating `checkpoint<T>` checks the current boundary's exact input type. A checkpoint token is
authoring-only, privately constructed, and neither serializable nor an arbitrary integer jump.
`Handlers::checkpoint<I, T>` attaches that token to the already bound handler for exact incident
input `I`; missing entries are rejected. Installing the family checks that the author owns the
token's scope. Descendants may inherit the installed binding, but cannot capture and directly
rebind a parent token. Injection is expanded before checking the complete recovery region.

Runtime receives only the lowered permitted State positions. Its borrowed recovery context
provides checked eligible targets derived from the current history, preserving their declared
order. `RecoveryTarget` has no public position constructor. Handlers request a supplied target;
Runtime validates it again against the selected occurrence's permissions and active checkpoints.
Program constructs these copyable target tokens during checked lowering; Runtime filters the
declaration's tokens to active eligible targets. This avoids requiring private cross-crate
constructors or letting Program acquire history semantics.
An Effect inside a declared region does not by itself invalidate the Program. After full injection,
Program checks scope ownership and exact boundary contracts; Runtime makes earlier checkpoints
ineligible when an Effect prepare is acknowledged. A checkpoint after the Effect may still recover
the following Read suffix. Occurrence overrides are subject to the same installation ownership
checks as Operation handler families.
The built-in `RestartRegion` handler requires exactly one bound checkpoint. More general custom
handlers must make their selection explicit; absence of an eligible target yields Stop. Neither
the authoring token nor its scope ID appears in Journal.

Checkpoint scoping is enforced by checked construction, not a claim of generative Rust lifetime
isolation. Test legal parent-installed inherited recovery and rejected direct child capture. The
typed token proves the requested boundary type only after checked creation; it does not grant IO
or append authority.

StatePosition is a bounded declaration index; VisitId is a checked run-wide `u64` counter. Both,
and their `ExecutionPosition` pair below, belong with checked IDs and contain no scheduling logic.
Genesis selects visit zero for a nonempty Program; a committed transition
selecting another execution uses checked increment. Effect prepare/conclusion share one visit.
Recovery allowances and usage are counts of committed retry/restart decisions, never timestamps or
invocation counters. The global limit remains distinct from per-occurrence allowances.

Retain one private mode-specific declaration representation with these exact data dependencies:

| Declaration data | Bound into Program identity |
| --- | --- |
| State ABI | Implementation, input, output, domain failure; operational error and context for IO modes. |
| Execution mode | Pure, Read capability/intent/evidence/binding, or Effect capability/command/evidence/binding. |
| Classifier binding | Exact source and mapped incident component refs, mapper/classifier identities and checked parameter values. |
| Handler binding | Exact mapped incident input, handler identity/parameters, permitted lowered checkpoints. |
| Root failure path | Ordered exact mapper ABIs and parameter values ending at the root domain failure contract. |
| Recovery bounds | Per-occurrence allowances and the Program-wide total decision allowance. |
| Size bounds | Maximum complete conclusion bytes for Pure/Read; separate maximum prepare and conclusion bytes for Effects. |

No successor edges remain. Check adjacent types, all parameter schemas and sizes, full callback
association requirements, checkpoint references, and checked capacity arithmetic before genesis.
Use one Read conclusion maximum covering both accepted evidence and contextualized adapter errors.
Do not create a resource optimizer or a separate stored reservation. Parameters and transient map
outputs obey canonical value bounds even when their values do not become frame objects.

Occurrence emission takes one checked size bound appropriate to its mode: `ConclusionBound`
for Pure/Read, or `EffectBounds` for Effect, with private fields and checked constructors. Bounds
must cover the complete closure and participate in Program identity. Registration validates compatibility
and execution enforces them. The exact numeric bounds for real EVM/Portfolio values are an
implementation handoff gate, not numbers invented by this signature sketch.

Journal remains the sole owner of frame/count/history format ceilings. Program's size-bound
constructors check positive representable lifecycle costs and its conservative history arithmetic
checks overflow. Finite `u32` allowances need no additional constructor restriction. Runtime
admission compares the derived cost and every complete-frame bound with Journal's exported
ceilings before genesis append. A structurally valid Program may therefore be inadmissible under
the current Journal capacity. Program does not import Journal or duplicate its numeric ceilings.

## Checked root input specialization

`Operation` requires `fn validate_input(&self, input: &Self::Input) -> Result<(), ProgramError>`.
`expand_program(entry, operation, input, limits)` invokes this deterministic authoring check before
expansion, qualifies the input, and commits its exact value reference into Program. The check proves
agreement with the root Operation's checked planning assumptions; schema equality alone does not
prove source order or asset shape. Child input agreement is owned by parent planning and deterministic
State contracts. No authoring callback enters Runtime.

Runtime checks exact input identity before genesis/provider entry, and cold reconstruction checks
genesis against the same commitment. Reusing a Program under another RunId requires the same initial
value. Programs contain the reference only; genesis retains the value bytes. Native/token collection
specialization stores the checked ordered request, and planning validates it against initial demand.
Test both inconsistent authoring (request A with input B) and later input substitution separately.

## Assembly association and adapter signatures

Authoring descriptors never carry Runtime functions. Application/live composition registers the
same concrete generic implementations in the immutable assembly:

```rust,ignore
impl RuntimeAssemblyBuilder {
    pub fn register_classifier<E, DM, XM, K>(&mut self) -> Result<(), RuntimeError>
    where E: MfmValue, DM: ValueMap, XM: ValueMap,
          K: Classifier<Incident<DM::Output, E, XM::Output>>;

    pub fn register_handler<I: IncidentContract, H: Handler<I>>(&mut self)
        -> Result<(), RuntimeError>;

    pub fn register_map<M: ValueMap>(&mut self) -> Result<(), RuntimeError>;

    // Existing register_pure/register_read/register_effect remain typed by State/capability.
    // Their mode registration now includes operational-error and State-context codecs/callbacks.
}

// Callback outputs in the existing async registration signatures change to:
type ReadResult<C: ReadCapabilityContract> = Result<
    <C as ReadCapabilityContract>::Evidence,
    AdapterError<<C as ReadCapabilityContract>::OperationalError>,
>;

type EffectResult<C: EffectCapabilityContract> = Result<
    EffectAdapterOutcome<<C as EffectCapabilityContract>::Evidence>,
    AdapterError<<C as EffectCapabilityContract>::OperationalError>,
>;
```

Keep the callback arguments unchanged: Reads receive the exact qualified intent ref and borrowed
intent; Effects receive EffectId, exact command ref, and borrowed command. Keep boxed async `Send`
futures and immutable pre-bound callbacks. `Pending` remains an ordinary Effect outcome.

`register_classifier` installs one monomorphized mapping/classification adapter with exact source,
mapped input, implementation, and parameter ABIs. Handler and root-map registration do the same for
their contracts. Registration is idempotent only for the same exact valid association; reject
conflicting contracts. Program association pre-resolves every callback and parameter codec. Live
execution uses those associated functions and does not perform registry lookups or resolve
Operation defaults. `new` stays fallible and `finish` freezes a valid assembly infallibly.

## Public results and operational stops

Keep the three existing Runtime entry points. A successful API call returns an observation of
committed history, which may itself describe a failed run. An invocation error is a separate result
and never implies a terminal history append.

```rust,ignore
impl Runtime {
    pub async fn start<T: MfmValue>(
        &self,
        run_id: RunId,
        program: Program,
        input: T,
    ) -> Result<RunView, InvocationFailure>;

    pub async fn resume(&self, run_id: &RunId)
        -> Result<RunView, InvocationFailure>;

    pub async fn read(&self, run_id: &RunId)
        -> Result<RunView, InvocationFailure>;
}

impl RunView {
    pub fn entry_point(&self) -> &EntryPointId;
    pub fn admitted_context(&self) -> &ValueView;
}

pub enum RunViewState {
    Runnable {
        position: ExecutionPosition,
        reason: RunnableReason,
    },
    EffectPending {
        position: ExecutionPosition,
        effect_id: EffectId,
    },
    Succeeded(ValueView),
    Failed(FailureReport),
}

pub struct ExecutionPosition {
    pub state: StatePosition,
    pub visit: VisitId,
}

pub enum RunnableReason {
    Advance,
    Retry,
    Restart { checkpoint: StatePosition },
}

pub enum InvocationFailure {
    Execution {
        run_id: RunId,
        error: RuntimeError,
        last_observed: Option<RunView>,
    },
    RecoveryStopped {
        observed: RunView,
        incident: Box<AdapterIncidentView>,
        reason: StopReason,
    },
}

pub struct FailureReport { /* private checked fields and derived canonical output */ }

impl FailureReport {
    pub fn position(&self) -> &ExecutionPosition;
    pub fn reason(&self) -> &StopReason;
    pub fn usage(&self) -> &RecoveryUsage;
    pub fn cause(&self) -> &FailureCauseView;
    pub fn value_ref(&self) -> &ContentRef;
    pub fn canonical_bytes(&self) -> &[u8];
}

pub enum FailureCauseView {
    Domain {
        original: ValueView,
        root: ValueView,
    },
    Adapter(AdapterIncidentView),
}

pub struct AdapterIncidentView {
    pub error: ValueView,
    pub state_context: ValueView,
}

impl ValueView {
    pub fn decode<T: MfmValue>(&self) -> Result<T, ValueError>;
}
```

`RunView` keeps its RunId, exact head sequence/digest, and checked private construction. The new
status alternatives replace the current undifferentiated `Runnable` status. `RecoveryStopped`
describes an observed pending Effect and the unrecorded adapter incident that stopped automatic
progression. It schedules no work and introduces no durable stopped-Effect flag. `last_observed`
is explicitly a historical observation, not proof of the current head after an uncertain append.

Keep Runtime's existing safe setup/association/history/internal error distinctions. Replace its
flattened Store-unavailability mapping with a source-preserving `RuntimeError::Store(StoreError)`.
Store `Indeterminate` remains distinguishable from definite noncompletion. Context construction,
policy, mapping, and adapter-invariant failures take the fixed internal stop path. No Runtime or
Store error is a classifier input.

`ValueView::decode` checks the requested exact schema contract before decoding. This extends
the existing terminal-value boundary for typed library reuse; it is not a way to pass arbitrary
untyped values into execution. Pending invocation incidents are qualified, bounded values even
though they were not appended. Rename the existing `RetainedValueView` to `ValueView` consistently
so its name does not claim persistence. Do not maintain separate retained and unretained codec systems.

`FailureReport` is constructed from retained original causes, mapped root failure, stop reason, and
derived position/counters. Its canonical output reference is reproducible without a report
callback or duplicate report object in each frame. An empty identity-success Program has no failure
position; failures before State execution use `InvocationFailure::Execution` instead.

## Journal wire shape and Store boundary

The following are structural record shapes, not new public domain APIs. Journal qualifies canonical
bytes and exact frame-local object closure; Runtime validates position, policy association,
checkpoint eligibility, budgets, and Effect barriers. All `Object` fields below mean the existing
qualified `JournalObject<'a>` representation.

```rust,ignore
enum Decision<T> {
    Retry,
    Restart { checkpoint: StatePosition },
    Stop { reason: StopCode, terminal: T },
}

enum DomainConclusion<'a> {
    Success { output: JournalObject<'a> },
    Failure {
        original: JournalObject<'a>,
        decision: Decision<JournalObject<'a>>, // terminal is the mapped root failure
    },
}

enum ReadConclusion<'a> {
    Observed {
        evidence: JournalObject<'a>,
        outcome: DomainConclusion<'a>,
    },
    AdapterFailed {
        error: JournalObject<'a>,
        state_context: JournalObject<'a>,
        decision: Decision<()>, // no manufactured domain failure
    },
}

enum EffectConclusion<'a> {
    Success { output: JournalObject<'a> },
    Failure {
        original: JournalObject<'a>,
        root: JournalObject<'a>,
        reason: StopCode,
    },
}

enum Record<'a> {
    Admitted {
        program: JournalObject<'a>,
        input: JournalObject<'a>,
    },
    PureConcluded {
        position: ExecutionPosition,
        outcome: DomainConclusion<'a>,
    },
    ReadConcluded {
        position: ExecutionPosition,
        intent: JournalObject<'a>,
        outcome: ReadConclusion<'a>,
    },
    EffectPrepared {
        position: ExecutionPosition,
        effect_id: EffectId,
        command: JournalObject<'a>,
    },
    EffectConcluded {
        evidence: JournalObject<'a>,
        outcome: EffectConclusion<'a>,
    },
}
```

`ExecutionPosition` is the shared checked identity pair, not a Runtime state object. `StopCode` is
Journal's closed structural wire vocabulary corresponding to the public stop reasons; Runtime owns
the explicit conversion. Journal imports no Program policy, Runtime view, or recovery callback.

The Effect conclusion inherits its position, command, and identity from the immediately preceding
prepare. Its outcome cannot encode retry or restart. Pure/Read recovery decisions include a new
position only through deterministic transition interpretation: the recorded occurrence identifies
the concluded visit, while the next visit is derived. There is no separately supplied destination
visit, checkpoint activation/input, assessment, capacity reservation, or duplicated report.

The domain original and mapped terminal value can be the same exact object reference. Object
closure retains it once. `Decision<()>` has no serialized terminal payload; its Stop wire variant
contains the reason only. These generics describe shared structural alternatives, not a generic
value interpreter in Journal. Freeze the exact new canonical tags, hashing domains, and hostile
vectors together with the implementation; old graph bytes are rejected.

Keep these existing Store signatures unchanged:

```rust,ignore
fn load_run<'a>(&'a self, run_id: &'a RunId)
    -> BoxFuture<'a, Result<Option<StoredRunBytes>, StoreError>>;

fn append_run<'a>(&'a self, frame: &'a EncodedRunFrame)
    -> BoxFuture<'a, Result<AppendResult, StoreError>>;
```

`BoxFuture` denotes the existing boxed `Send` future signature. No State ID, policy, checkpoint,
configuration, terminal status, or retry counter enters Store's API. PostgreSQL's snapshot,
atomicity, exact-head locking, and acknowledgement contracts remain unchanged.

## Engineer handoff gates

The API sketch is a proposed contract, not a claim that a complete engine has been implemented.
The implementation should begin with the smallest consuming examples, then complete the one
Program/Runtime/Journal cutover described in the parent RFC. Keep its dependent changes coherent;
do not ship a parallel graph and linear engine while proving the API.

Before freezing the implementation wire, demonstrate:

- Nested Portfolio/EVM classifier defaults, an independent handler override, and explicit typed
  mappings with the original incident preserved.
- State context construction for a Read error and a pending Effect error; neither can change the
  original adapter cause or authoritative execution facts.
- Exact association of all selected State, capability-error, context, mapper, classifier, handler,
  parameter, and root-failure contracts without runtime conversion search.
- A real 64-source capacity calculation under the proposed conservative rule, including maximum
  valid conclusions and the pending Effect lifecycle.
- Public reconstruction tests, including competing recovery appends, uncertain acknowledgement,
  stopped pending Effects, completed preparation callbacks not rerun, and decoded default reports.

Stage the first validation in the production Program authoring and existing Runtime assembly,
with real codec-backed descriptors, parameter qualification, and associated callbacks. Internal
association tests may run before the new Journal wire exists; public consuming tests also exercise
the real generic registration API. Registration alone does not prove selected association. Keep
this work in the uncommitted replacement candidate until the inseparable cutover is coherent;
do not ship a graph/recovery intermediate design or create a separate validation Runtime.

Capacity evidence must bound every valid complete closure, not just measure successful examples.
In particular, a terminal Read domain failure can retain intent, evidence, original failure, and
mapped root failure. Four independent maximum-size objects exceed the current frame ceiling.
Use checked domain bounds and maximum alternative conclusions for native, token, mixed 64-source,
and fully injected transaction fixtures before selecting useful nonzero recovery allowances.

Concrete enrichment remains a subsequent commit: user-requested discovery, exact immutable DB
configuration publication, and admission from the selected revision. Do not mix a discovery source
or new enrichment-specific transport API into the core signature proof.

Use [scope-driven verification](build-and-verification.md). The final implementation candidate
requires the affected focused and managed boundary tests followed by one `nix run .#ci`. A
documentation signature check alone does not satisfy those implementation gates.

## Signature proof and validation scope

The [standalone example](examples/recovery-api-signatures.rs) lives outside the Cargo workspace.
It checks Rust generic instantiation and finite selection of explicitly bound policies, including
parent/child defaults, occurrence overrides, domain mapping, non-`Clone` original error preservation,
and rejection of missing/duplicate bindings. Its assembly methods and root-map lowering are
signature stubs. It has no State executor, provider, Store, Journal decoder, or second Runtime.

For this proof only, `MfmValue` is a marker and `TypeId` substitutes for exact component schema
refs. `type_name` labels make selected implementations visible to the example assertions; neither
identifier is a persisted ABI or a production fallback. The sealed `IncidentContract` models the
actual structural bound. Real codec registration, parameter canonicalization, checkpoint scopes,
adjacent IO types, capacity, async cancellation, and persistence are not proved by this example.

Run it without host Rust or a Cargo workspace change:

```sh
proof_dir=$(mktemp -d /tmp/mfm-recovery-signatures.XXXXXX)
nix develop -c rustc --edition=2021 docs/examples/recovery-api-signatures.rs -o "$proof_dir/check"
"$proof_dir/check"
```

The signatures and finite-selection example compile and its assertions pass in the pinned Nix
shell. Rustdoc-style `rust,ignore` blocks above specify proposed interfaces and deliberately omit
implementation bodies; the claim applies to the linked example, not every fragment as a standalone
compilation unit. No implementation CI result is implied.

## Material uncertainties

| Assumption | Why uncertain | Consequence if wrong | Validation before implementation freeze |
| --- | --- | --- | --- |
| The finite typed binding design integrates with existing checked values without additional public abstractions. | The proof substitutes marker values and process-local type IDs for real codecs/content refs. | Codec/association bounds could require revising signatures. | Port the four consuming cases to real MfmValue types and Runtime assembly, including exact mismatch rejection and typed report decoding. |
| Typed checkpoint tokens with checked scope IDs suffice. | The signature proof does not implement scope bookkeeping or injection. | A scope bug could permit unintended parent capture. | Exercise parent-installed inherited recovery and rejected child reinstallation after complete injection. |
| Complete frame bounds admit useful recovery allowances. | Real canonical closure sizes have not been measured. | The conservative rule could reject useful portfolios. | Measure the 64-source and transaction fixtures, including maximum error/context and conclusion sizes. |
