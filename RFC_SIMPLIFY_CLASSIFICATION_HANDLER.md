# RFC: simplify error classification and recovery handlers

Status: implemented. This document records the cutover rationale and illustrative API sketches.
[docs/design.md](docs/design.md) and [docs/architecture.md](docs/architecture.md) own the current
executable and persistence contracts; [recovery validation](docs/recovery-validation.md) maps
the acceptance criteria to consuming coverage.

## Summary

States and adapters return precise, typed, redaction-safe failures. Each recoverable-path error type
implements one deterministic classifier that projects its original cause into a common
`Classification`. One static handler consumes that classification and Runtime-owned recovery
context. An Operation selects a default handler; nested Operations inherit it unless they explicitly
replace it, and individual State occurrences can override it.

Classification belongs to the error contract. Handler selection belongs to the caller's Program.
Recovery authorization and scheduling belong exclusively to Runtime. Original errors remain
available for audit and reporting; classification never substitutes for them.

There is no additional `RecoveryAdvice` abstraction, no separately selected classifier registry, and
no handler implementation requirement for every different State incident type.

## Motivation and superseded design

Before this cutover, the recovery contracts exposed static
`Classifier::classify` and `Handler::handle` functions. The complexity lies around those functions:

- Independently inherited `Classifiers` and `Handlers` families select bindings by exact incident ABI.
- Classifier bindings include domain and adapter-context maps, their parameters, classifier
  parameters, and source/mapped contract relationships.
- Runtime registers and associates those combinations, constructs a mapped incident, and passes it
  between separately registered classifier and handler callbacks.
- Operation authors must account for heterogeneous child incident types in both families.

The superseded implementation supported independently replacing classification and mapping foreign
domain errors into a policy-specific vocabulary. This RFC deliberately replaces that flexibility
with intrinsic classification of exact error types and independently configurable recovery actions.

Before this cutover, shipping Portfolio selected `NoRecovery`/`Stop` with zero allowances. Its default
behavior must remain stop with zero allowances after this cutover. Removing `NoRecovery` does not
silently enable recovery.

## Goals

- One common handler interface across heterogeneous States and nested Operations.
- Error-specific classification without runtime classifier lookup or policy-specific error maps.
- Operation defaults and explicit State-occurrence overrides.
- Precise distinctions between failure scenarios, including different timeout outcomes.
- Original typed causes and State-owned adapter context retained at existing observation boundaries.
- Exact Program policy identity, deterministic reconstruction, finite committed recovery budgets,
  and all existing Effect safety constraints.
- Complete deletion of superseded abstractions when implementation lands.

This RFC does not introduce ambient State IO, autonomous background recovery, automatic sleeps,
or unbounded retry loops. Durable recording of pending Effect operational failures is required in
this cutover: auditability is a core contract, not an optional follow-up. The protocol and its
crash/cancellation limits are specified below.

The implementation objective is to reduce concepts, public types, independent configuration sites,
branches and LOC. Merely wrapping the existing recovery families in a shorter authoring API does
not meet that objective. New audit code must be justified separately from the simplification;
required audit guarantees must not be weakened to obtain a smaller diff.

## Ownership

| Owner | Responsibility |
| --- | --- |
| Capability/domain error owner | Precise operational or domain error variants and their classification semantics |
| Live adapter | Convert provider/client outcomes into the appropriate reviewed operational error variant |
| State | Deterministic domain outcomes and adapter context derived from explicit input/intent/command |
| Program | Common classification contract, static handler contract, selected handler descriptors, parameters, allowances and scoped recovery targets |
| Runtime | Compose incidents, associate typed classification functions, supply execution facts, authorize recovery and perform the sole fold |
| Journal | Exact committed failure/decision wire and history qualification |
| Store | Mechanical complete-prefix load and atomic exact-head append |

`ClassifyError` and `Classification` belong in `mfm-program`. State failure types implement the
trait. Program authoring and Runtime registration require `C::OperationalError: ClassifyError` for
capabilities used by executable States. Those bounds belong at the existing typed integration
boundaries; they do not require the capability crate itself to depend on Program.

`mfm-capabilities` retains its reusable `OperationalError: MfmValue` contract and its existing
dependency direction. No new crate or dependency is needed. Domain-owned error types can implement
the Program trait in their owning domain crate. `Never` has an unreachable classification
implementation.

## Classification is intrinsic error semantics

Rust examples below are illustrative API sketches, with some bodies and integration details omitted. Existing
checked identifiers, value derives, canonical codecs, fallible constructors and documentation
requirements still apply. Shortened signatures omit unrelated generic bounds and root failure maps.

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Classification {
    /// Another attempt with unchanged intent/command is supported by this error's semantics.
    Retryable,
    /// The outcome is unresolved; this error supplies no basis for automatic repetition.
    OutcomeUnknown,
    /// Recovery requires refreshing an input; repeating it unchanged cannot help.
    InputInvalidated,
    /// This cause does not support recovery through the available generic actions.
    Permanent,
}

pub trait ClassifyError: MfmValue {
    fn classify(&self) -> Classification;
}

impl ClassifyError for Never {
    fn classify(&self) -> Classification {
        match *self {}
    }
}
```

The method is a statically selected, pure projection of the typed error. Borrowing `self` means
borrowing the cause, not retaining a stateful classifier service. Runtime's typed registration
adapter can monomorphize it into an ordinary callback. It performs no IO, mutation, registry lookup,
or fallible domain conversion.

Classification is derived from the cause, not stored as an independently writable sibling field.
This prevents constructing a timeout cause with an unrelated classification flag. If classification
needs additional facts, the reviewed error variant must retain those facts explicitly.

Different error types can have different classifiers. Callers cannot replace the classifier for
the same exact error contract independently of that contract. They customize the handler instead.
If two adapter protocols give different meaning to otherwise similar failures, their typed errors
must distinguish those semantics; a process-local adapter identity cannot silently change them.

`Retryable` is neither a command to retry nor permission to bypass Runtime. It describes the failure
semantics. Phase, retained command identity, budgets and checkpoint barriers still restrict actions.

## Example: balance adapter and deterministic State

```rust
pub enum BalanceAdapterError {
    ProviderTimeout,
    LocalReadDeadline { deadline_ms: u64 },
    RateLimited,
    Unavailable,
}

impl ClassifyError for BalanceAdapterError {
    fn classify(&self) -> Classification {
        match self {
            // This capability is a duplicate-safe observational Read.
            Self::ProviderTimeout
            | Self::LocalReadDeadline { .. }
            | Self::RateLimited
            | Self::Unavailable => Classification::Retryable,
        }
    }
}

pub enum BalanceStateError {
    AnchorChanged { expected: BlockHash, observed: BlockHash },
    UnsupportedAsset { asset: AssetId },
}

impl ClassifyError for BalanceStateError {
    fn classify(&self) -> Classification {
        match self {
            Self::AnchorChanged { .. } => Classification::InputInvalidated,
            Self::UnsupportedAsset { .. } => Classification::Permanent,
        }
    }
}
```

An internal deadline on this duplicate-safe Read can be retryable. The same conclusion does not
automatically apply to an Effect adapter. Classification uses the capability's actual semantics,
not the word "timeout" alone.

The adapter and State continue to expose distinct execution boundaries:

```rust
impl BalanceAdapter {
    async fn read(
        &self,
        intent: &BalanceIntent,
    ) -> Result<BalanceEvidence, AdapterError<BalanceAdapterError>> {
        // Perform bounded IO and qualify/map provider outcomes.
        // Operational errors contain reviewed causes, never raw response text.
        todo!()
    }
}

impl ReadBalance {
    fn interpret(
        input: BalanceInput,
        evidence: &BalanceEvidence,
    ) -> Result<ProposedStateOutcome<BalanceOutput, BalanceStateError>, StateExecutionError> {
        if evidence.anchor() != input.anchor() {
            return Ok(ProposedStateOutcome::Failure {
                failure: BalanceStateError::AnchorChanged {
                    expected: input.anchor().clone(),
                    observed: evidence.anchor().clone(),
                },
            });
        }
        Ok(ProposedStateOutcome::Success {
            output: BalanceOutput::from_evidence(input, evidence)?,
        })
    }
}
```

`todo!()` marks an omitted sketch body, not an implementation proposal. Evidence binding and
checked output construction retain their current owners. Local mismatches, preparation errors,
`StateExecutionError` and `AdapterError::Invariant` remain outside classified recovery; they map to
the current internal failure boundary and cannot manufacture a durable domain/provider incident.

## Example: two materially different Effect timeouts

```rust
pub enum SubmitAdapterError {
    /// Qualified response guarantees this request was not accepted.
    ServerTimeoutBeforeAcceptance { request_id: PublicRequestId },
    /// Client deadline expired after dispatch; remote acceptance is unknown.
    LocalDeadlineAfterDispatch { deadline_ms: u64 },
    /// A reviewed protocol rejection, not authenticated Effect settlement evidence.
    Rejected { code: SubmitRejectionCode },
}

impl ClassifyError for SubmitAdapterError {
    fn classify(&self) -> Classification {
        match self {
            Self::ServerTimeoutBeforeAcceptance { .. } => Classification::Retryable,
            Self::LocalDeadlineAfterDispatch { .. } => Classification::OutcomeUnknown,
            Self::Rejected { .. } => Classification::Permanent,
        }
    }
}
```

This example assumes a specific server contract: its qualified timeout response proves
nonacceptance. A generic HTTP timeout response does not establish that fact. The adapter must use
`OutcomeUnknown` when its actual protocol cannot establish the claimed distinction.

Conversely, an adapter whose protocol guarantees safe repetition of the same command identity can
classify an internal deadline as `Retryable`. The error contract must encode that guarantee. This
RFC does not impose a universal "server timeout retries, local timeout stops" rule or relabel the
current EVM errors without concrete protocol evidence.

Authenticated transaction reversion remains settlement evidence interpreted into a domain outcome;
it must not be recast as a pre-settlement operational rejection to obtain another attempt.

## Original incident and common handler input

Keep the current distinction between domain failures and adapter failures with State context:

```rust
pub enum Incident<D, E, X> {
    Domain(D),
    Adapter { original: E, context: X },
}

#[derive(Clone, Copy)]
pub enum IncidentSource { State, Adapter }

pub struct IncidentSummary {
    pub source: IncidentSource,
    pub classification: Classification,
}

impl<D: ClassifyError, E: ClassifyError, X> Incident<D, E, X> {
    pub fn summary(&self) -> IncidentSummary {
        match self {
            Self::Domain(error) => IncidentSummary {
                source: IncidentSource::State,
                classification: error.classify(),
            },
            Self::Adapter { original, .. } => IncidentSummary {
                source: IncidentSource::Adapter,
                classification: original.classify(),
            },
        }
    }
}
```

Runtime composes this incident. The State does not call an adapter to return a combined IO result.
For an operational error, Runtime obtains the existing deterministic State-owned adapter context
and preserves it alongside the original cause.

The handler consumes the common summary. The complete original incident remains separately owned
by the execution/reporting path. There is no policy-facing erased payload, downcast API, or mapped
incident exchange between classifier and handler. Runtime's existing private typed codec erasure
can remain where heterogeneous executable association requires it.

## One static handler interface

```rust
pub trait Handler: Send + Sync + 'static {
    type Params: MfmValue;

    fn implementation_id() -> Result<StableId>;

    fn handle(
        params: &Self::Params,
        incident: &IncidentSummary,
        context: &RecoveryContext<'_>,
    ) -> Result<RecoveryRequest, StateExecutionError>;
}
```

There is no incident type parameter. One implementation serves all error types that project into
the common classification. The only typed handler configuration is its immutable `Params`.
Handler implementation errors remain internal failures before conclusion append.

`RecoveryContext` continues to expose authoritative execution phase, remaining allowances and
eligible restart targets. Keep the existing checked target list on the selected handler binding,
but remove its association with an incident-family key. `StandardRecovery` restarts only when
exactly one target is bound and it is currently eligible. Zero or multiple bound targets mean
Stop for this standard policy. Custom handlers can select among explicitly bound targets using
the existing context. There is no additional named refresh-target field or checkpoint service.

```rust
pub struct StandardRecovery;

impl Handler for StandardRecovery {
    type Params = NoParams;

    fn implementation_id() -> Result<StableId> {
        StableId::new("mfm.recovery.standard@1")
            .map_err(|_| ProgramError::InvalidContract)
    }

    fn handle(
        _: &NoParams,
        incident: &IncidentSummary,
        context: &RecoveryContext<'_>,
    ) -> Result<RecoveryRequest, StateExecutionError> {
        use Classification::*;
        use ExecutionPhase::*;
        use RecoveryRequest::*;

        Ok(match (incident.classification, context.phase()) {
            (Retryable, Read | EffectPending) => RetryState,
            (InputInvalidated, Pure | Read) => {
                // Exactly one declared target, and that target is active/eligible.
                context.single_restart_target()
                    .map(Restart)
                    .unwrap_or(Stop)
            }
            _ => Stop,
        })
    }
}
```

`single_restart_target()` is an accessor over the existing declared/eligible targets. It
does not pick one merely because only one of several declared targets is currently active. General
custom handlers retain the existing explicitly bound target list. If a scope needs several
different semantic refresh regions, use narrower bindings or a reviewed custom policy. Do not add
a registry of domain labels or a second representation of checkpoint ownership.

The framework default remains the generic `Stop` handler, now implementing `Handler` once.
`StandardRecovery` is explicitly selected in these examples, not a new shipping default.

## Operation inheritance and occurrence overrides

```rust
// Illustrative OperationExpansion authoring; omitted root maps remain required.
let refresh_anchor = scope.checkpoint::<CollectionInput>()?;

scope.handler(
    HandlerBinding::new::<StandardRecovery>(NoParams)?
        .checkpoint(&refresh_anchor)?,
)?;
scope.allowances(RecoveryAllowances::new(3, 1))?;

scope.read::<AcquireAnchor>(Occurrence::new())?;
scope.read::<ReadNativeBalance>(Occurrence::new())?;
scope.operation(&CollectTokenBalances::new())?;

scope.read::<ReadOptionalMetadata>(
    Occurrence::new()
        .handler(HandlerBinding::new::<Stop>(NoParams)?)
        .retries(0)
        .restarts(0),
)?;
```

The selected handler binding is inherited as ordinary authoring data; the Operation does not
become generic over a policy type. Child States need classification support for their errors,
not individual `Handler<ChildIncident>` implementations.

Selection order is occurrence override, nearest explicit enclosing Operation setting, then
framework `Stop`. Nested Operations inherit the selected binding unless they explicitly replace
it. Replacing a binding replaces its parameters and target bindings together; it does not merge
fields from unrelated policies. Allowances retain their separately documented nearest-setting and
occurrence-override rules, including explicit zero.

Installed inherited checkpoint bindings retain their owning scope for lowering. A child cannot
capture and reinstall another scope's raw checkpoint token to bypass scope checks. Scope exit and
symbolic relocation retain their current Program-owned contracts.

Runtime registers handlers once by implementation and exact parameter contract:

```rust
builder.register_handler::<StandardRecovery>()?;
builder.register_handler::<Stop>()?;
builder.register_read_state::<ReadNativeBalance, BalanceRead>()?;
```

State registration installs the typed error classification projections together with the existing
codecs and executable callbacks. There is no separate `register_classifier` call and no handler
registration per incident ABI. No registry lookup occurs during the semantic fold.

## What a checkpoint means, and what can be simplified

A checkpoint is an explicitly selected boundary before a State, together with the typed input
Runtime retains when execution reaches that boundary. Restarting there executes that State and
its successors again using the retained input. A checkpoint is not a database snapshot, a new run,
or permission to delete acknowledged history.

For example:

```text
collection input
    |
    +-- checkpoint: retain collection input here
    |
AcquireAnchor            -> anchor A
ReadNativeBalance(A)     -> balance at A
ReadTokenBalance(A)      -> AnchorChanged { expected: A, observed: B }
    |
    +-- RetryState: repeat ReadTokenBalance with the same anchor A
    |              (does not repair an invalid anchor)
    |
    +-- Restart(checkpoint): use retained collection input
                            run AcquireAnchor again, then recollect balances
```

The failed attempt and restart decision stay in Journal. New visits append new frames. Runtime
restores a previously retained input for subsequent execution; it never rolls history back. An
acknowledged Effect is a barrier: restart cannot cross it and repeat earlier irreversible work.

Why keep a checkpoint at all? A handler needs a precise answer to "which input should be refreshed?"
An Operation can include several Reads, and retrying the last one is different from rerunning
anchor acquisition. A caller-supplied typed boundary answers that without teaching Runtime about
balances or anchors. Removing restart support entirely would change supported behavior rather
than simplify its implementation.

Keep the small existing [typed checkpoint token and scope owner](crates/kernel/program/src/recovery/scope.rs),
checked lowering, retained input and Effect barrier. Remove incident-type parameters from attaching
a checkpoint to a handler: the selected binding already identifies the policy. Keep one list of
checked targets, rather than adding a separate generic refresh-target configuration. This is the
target API; classification-specific checkpoint selection can be reviewed case by case later.

## Execution and Effect semantics

```text
typed State failure / typed adapter operational error
    -> Runtime preserves original cause and State context
    -> statically associated error classifier produces IncidentSummary
    -> selected handler requests Stop / RetryState / Restart(target)
    -> Runtime authorizes against phase, committed history and allowances
    -> atomic conclusion or pending-Effect failure append
    -> expose only the acknowledged result
```

Execution phase belongs to Runtime. A cause may describe protocol stages, but it cannot declare
that an Effect is unacknowledged or settled. "Pre/during/post Effect" is insufficient: an adapter
may fail before broadcasting while a prepare is already acknowledged, and a later Pure State can
still be constrained by an earlier acknowledged Effect.

| Cause and phase | Standard handler request | Runtime meaning |
| --- | --- | --- |
| Duplicate-safe Read timeout | RetryState | Commit a Read failure/retry conclusion, spend allowance, advance visit and yield |
| Balance anchor invalidated during Read | Restart(bound target) | Commit permitted restart, retain original failure, restore checkpoint input |
| Unsupported asset | Stop | Conclude failure through the current typed reporting path |
| Qualified nonacceptance timeout, EffectPending | RetryState | Append failure/retry, spend allowance, yield with the same command and EffectId for a later progress call |
| Unknown dispatch outcome, EffectPending | Stop | Append failure/stop, expose typed cause, retain pending command for explicit resume |
| Retryable cause, EffectSettled | Stop | Settled Effects cannot be repeated through recovery |
| Retryable cause, Pure | Stop | Repeating deterministic execution with identical input cannot help |

Runtime independently rejects invalid requests even if a custom handler proposes them. An
`OutcomeUnknown` classification supplies no retry-safety evidence; a custom handler must not
reinterpret it as such. It is not a classifier veto: a caller-selected custom handler may request
`RetryState` for an unknown pending outcome under its retained-command recovery policy. Runtime
then commits the failure and authorized retry before yielding with the same command authority.
That choice does not establish nonacceptance or
permit a new command. Deliberate explicit resume of a retained Effect likewise remains an existing
caller-driven protocol operation, not a claim that the previous attempt did nothing.

This cutover replaces the former no-append pending failure path. A pending retry commits a failure
record and spends local/global recovery allowance before yielding. It never re-enters the adapter
in a loop, changes the State visit, or derives a new EffectId. A pending Stop also commits the
failure but does not spend recovery allowance. Both consume the separate finite failure-record
capacity described below. Existing retry budgets alone cannot bound Stop followed by explicit
resume, so they cannot substitute for that capacity.

An acknowledged Effect still blocks restart across its position, including when the current
failure is in a subsequent Pure or Read State. Settled Effect recovery remains disallowed. Store
receives no classification, policy or scheduling responsibility.

## Auditability and persistence

The original error is the audit fact. `Classification::OutcomeUnknown` is insufficient to explain
which adapter failed, its precise variant, or its reviewed public details. Preserve exact qualified
error values and State-owned context. Every qualified operational failure acknowledged to the
caller must be committed, including pending Effect failures that request retry or stop.

For example, an invocation report must retain the equivalent of:

```text
original cause:
  SubmitAdapterError::LocalDeadlineAfterDispatch { deadline_ms: 5000 }
State context:
  public transaction identity and route reference
execution:
  acknowledged pending command and EffectId
handler result:
  Stop
```

Public request identifiers must have checked size and redaction-safe semantics; arbitrary provider
headers, bodies, URLs, credentials and client error strings are not acceptable substitutes for a
reviewed error contract. No new provider payload logging is introduced.

Keep original domain failures separately from mapped root failures. Root failure conversion is
still needed for Operation output contracts and terminal reports; it is not classifier mapping.
Removing classifier maps must not remove the original-to-root audit relationship.

Committed Pure/Read conclusions continue to retain original failure facts and the committed
decision. Completed cold reconstruction uses those decisions; it never reruns classification,
handler logic, or completed State interpretation. The common classification is a hot projection,
not a new independently mutable persisted payload. If a future report exposes it durably, that
requires an explicit wire contract rather than recomputing it from whichever binary is installed.

### Required pending Effect failure record

The current implementation appends nothing on pending Effect operational failure. Replace that
path with one new Journal record variant and a decision type that cannot represent restart:

```rust
pub enum PendingDecision {
    Retry,
    Stop { reason: StopCode },
}

// Variant of the existing Journal record sum; object references use existing qualification.
EffectAdapterFailed {
    position: ExecutionPosition,
    original: T,
    state_context: T,
    decision: PendingDecision,
}
```

The enclosing record's execution position must match the unmatched prepare. The Journal sequence
identifies this audit event; do not add an attempt UUID, new run identity, separate attempt table
or duplicate command payload. The original pending prepare supplies command and EffectId.

Journal's allowed structural sequence becomes:

```text
EffectPrepared -> EffectAdapterFailed* -> EffectConcluded
```

A valid complete prefix can end after prepare or any failure record. No unrelated State record
can intervene, and no failure record may follow settlement for the same prepare. Journal owns
structural qualification; Runtime owns semantic position, counters and outcome validation.

Runtime must qualify the original/context, compute the classification and requested action,
authorize it, and append the failure/decision atomically before reporting an operational outcome.
Retry and Stop both preserve pending position, visit, command and EffectId. Retry increments
committed local/global recovery usage; exhausted recovery allowance converts the proposal to a
recorded Stop with the appropriate reason. Stop ends this invocation, not the pending command's
authority. Explicit later progress can resume while failure-record capacity remains.

Cold fold restores counters and the latest audited failure from those records without callbacks.
Pending observations expose that latest original cause/context and decision, including after a
retry-yield. Earlier failures remain in Journal. They need no duplicate public all-history array or
new transport endpoint. A subsequent settled conclusion resumes the existing settled behavior.

### Finite capacity and admission

Extend each Effect occurrence's existing bounds with `max_pending_failures` and a positive complete
`failure_frame_bytes` bound. Every committed pending failure counts, including Stop. Derive the
count from history; do not persist a second mutable attempt counter. Before each adapter entry,
Runtime must have admitted capacity for another failure record and eventual settlement. If the
failure-record cap is exhausted, reject the invocation before provider IO and preserve the pending
authority. An executable Effect requires at least one such failure slot.

The conservative per-Effect lifecycle bound becomes:

```text
frames = 2 + max_pending_failures
bytes  = prepare_bytes
       + max_pending_failures * failure_frame_bytes
       + conclusion_bytes
```

Include those lifecycles in the existing conservative repeated-sequence bound. For `P` Pure and
`R` Read occurrences, `E` Effect occurrences and global recovery limit `G`, a conservative frame
ceiling is `1 + (G + 1) * (P + R + sum_effects(2 + max_pending_failures))`. All arithmetic remains
checked against the existing object/frame/history limits. The independent record cap is necessary
because stopped attempts spend no retry allowance and callers may resume them.

Exhaustion can leave an Effect unresolved and prevent further adapter entry through this run.
Finite immutable admission requires that limit; do not hide it, silently increase it during resume,
or delete old failures to create room. A caller must select adequate finite bounds when authoring.
This is record capacity, not a new scheduler, retry policy, or permission to abandon remote effects.

### Cancellation, ambiguity and the audit guarantee

No successful operational acknowledgement is returned before the failure append is known inserted.
An ambiguous append uses the existing indeterminate result and exact-head recovery protocol;
reload establishes whether the record committed before proceeding from the qualified head. Stop
and Retry reports never assert that an unacknowledged candidate is durable.

The guarantee is durable acknowledged operational outcomes. A process may die after provider IO
but before recording its result; cancellation, local qualification or handler errors can also
prevent a qualified failure record. Such an interrupted attempt remains outcome-unknown under the
retained command. The system must not claim a complete audit of every physical network call.
Logging physical attempt starts and representing abandoned attempts would require a pre-call
record protocol; that is a different guarantee, not something a post-result frame can provide.

Store's exact-head append prevents competing frames from both winning the same head; it does not
serialize concurrent provider calls. Concurrent callers retain the same Effect authority, and
losing or interrupted attempts must not be fabricated as acknowledged history. Existing adapter
duplicate-safety and command reconciliation contracts remain necessary.

## Program identity and association

Program retains the selected handler implementation identity, exact parameter ABI and canonical
parameter value, permitted checkpoint bindings, and allowances. Function addresses and callbacks
are never serialized or hashed. Changing handler parameters or selection changes Program identity.

Classification semantics belong to the exact State/capability error contract and executable
implementation. A change to the classifier's meaning requires an intentional identity revision in
the affected contract/implementation and consuming Program identity; unchanged enum serialization
does not make a semantic classifier change compatible. An implementation ID is a reviewed contract
commitment, not an automatic hash of Rust code.

Runtime association rejects missing handlers, conflicting registrations, incompatible parameter
codecs and incompatible State/capability contracts before execution. It installs classification
projections through the concrete State/capability registration. There is no fallback classifier
selected by semantic type name.

Replacing classifier/handler descriptors changes the Program contract. Implementation must update
the current Program wire identity and reject superseded descriptors. Do not add a legacy decoder,
migration, dual policy path or history rewrite. Update Journal wire identity if its actual encoded
contract changes; removing old assessment-derived stop codes requires an explicit coordinated
decision rather than silently changing the meaning of existing codes.

The old `Nonrecoverable` classifier veto disappears. The handler requests an action from the new
classification; Runtime applies its safety checks. Remove obsolete assessment-specific public
stop reasons and update CLI/API codes in the same cutover. Keep handler-requested stop distinct
from exhausted allowances and Runtime-denied actions.

## Complete cutover and deletion scope

| Area | Required result |
| --- | --- |
| Program recovery | Add intrinsic classification and common-summary handler; remove `Classifier`, `Assessment`, `NoRecovery`, `Classifiers`, classifier ABIs/bindings/parameters, and incident-keyed `Handlers` families |
| Authoring | One inherited handler binding with occurrence override; preserve scoped checkpoints and independent allowance semantics |
| Runtime assembly | Remove classifier registry and registration API, mapped policy incident plumbing, and per-incident handler registration; associate classification with typed executable registration |
| Maps | Delete policy-only domain/context conversions and their parameters; preserve explicit root failure and expansion maps that still serve public output contracts |
| Domain/live producers | Implement exhaustive classifiers for exact errors; refine variants only where protocol evidence supports the distinction; remove selectable `EvmBalanceClassifier` |
| Runtime execution | Use common summary, preserve internal-error bypass and sole fold; replace no-append pending operational outcomes with atomic audited decisions preserving the command |
| Persistence and reports | Add pending failure wire/adjacency, accounting and latest-failure observation; update descriptors/codes/bounds, retain original/root failures, reject incompatible formats |
| Consumers | Update Portfolio, Application, transport rendering, fixtures, rustdoc and consuming tests; shipping defaults stay Stop/zero |

Do not mechanically delete all `MapAbi`, `ValueMap`, `Identity`, or erased value machinery: inspect
remaining root-map and codec responsibilities. Delete only superseded owners and paths, with no
compatibility shims or deprecation layer. Similarly, public `IncidentAbi` can be removed if no
remaining exact contract needs it; retain necessary incident qualification privately rather than
preserving an obsolete family-selection API.

Update [design](docs/design.md), [architecture](docs/architecture.md), affected crate READMEs,
transport docs and [recovery validation](docs/recovery-validation.md) in the implementation cutover.
The current contract documents are intentionally not rewritten by this proposal-only change.

### Concrete simplification inventory

This inventory identifies current code to delete or shrink. It is an implementation requirement,
not a suggestion to keep both designs while adding a convenience layer.

| Current location / mechanism | Simplification or deletion | Necessary responsibility retained |
| --- | --- | --- |
| [recovery.rs](crates/kernel/program/src/recovery.rs): `Classifier<I>`, `Assessment`, sealed `IncidentContract` association | Delete the selectable classifier trait and assessment. Delete the sealed incident contract abstraction once handler/binding users disappear; direct typed `Incident<D,E,X>` needs no associated-type facade | Intrinsic error classification, original incident sum |
| [bindings.rs](crates/kernel/program/src/recovery/bindings.rs): `ClassifierAbi`, `ClassifierBinding` | Delete source/mapped derivation, map identities, map parameter contracts, classifier identity/parameters and constructors | Exact State/capability error contracts already identify the intrinsic semantics |
| Same file: `Classifiers(BTreeMap<IncidentAbi, ...>)` | Delete family construction, duplicate-source checks, exact family lookup and missing-family-source errors | Typed integration checks that each executable error implements classification |
| Same file: `Handlers(BTreeMap<IncidentAbi, HandlerSetting>)` | Replace with one handler binding; delete incident-keyed selection, duplicate bindings and checkpoint attachment lookup by incident type | Handler identity, parameters and checked checkpoint list |
| Same file: `HandlerAbi::of<I,H>` and `HandlerBinding::stop(input)` | Remove incident input from ABI and generic parameters; one Stop binding works for all States | Exact handler implementation and parameter codec association |
| Same file: `IncidentAbi` | Delete public policy-dispatch ABI once no remaining consumer needs it; do not retain it solely to mirror the removed registries | Original payload codecs and contract references |
| [defaults.rs](crates/kernel/program/src/recovery/defaults.rs): `NoRecovery`, generic default registrations | Delete NoRecovery and per-incident Stop specialization; install one default Stop handler | Shipping Stop/zero behavior |
| Same file: `Occurrence`, `RecoveryDefaults`, `SelectedRecovery::resolve<D,E,X>` | Replace two optional families and typed source/mapped resolution with one optional handler binding and nearest-default selection | Independent allowance overrides, explicit zero, target ownership |
| [authoring.rs](crates/kernel/program/src/authoring.rs): classifiers/handlers setters and typed policy resolution | Remove classifier setter and family APIs; lower one selected binding per occurrence | Typed State expansion, root maps, scope relocation and immutable Program |
| [assembly/recovery.rs](crates/kernel/runtime/src/assembly/recovery.rs): classifier registry and `register_classifier<E,DM,XM,K>` | Delete registry, composite TypeId key, registration, identity-map registration for classifiers and classifier-specific codec setup | Existing value registry and error codecs used by executable States |
| Same file: `AssociatedRecovery` classifier/domain/context parameters | Remove three parameter objects and classifier-map association checks | One selected handler callback and its typed parameters |
| Same file: `ErasedIncident`, `classify<E,DM,XM,K>`, `handle<I,H>` | Delete mapped incident allocation/downcast exchange and conversion qualification; pass the small common summary directly | Private heterogeneous value erasure still needed by codecs/Runtime |
| Same file: map callbacks/registrations | Remove registrations and conversions used exclusively by classifiers | `AssociatedRootMap`, root-map composition and typed terminal failure qualification |
| [assembly.rs](crates/kernel/runtime/src/assembly.rs): default classifier registration during State registration | Replace repeated NoRecovery/Identity registrations with direct error-classification callbacks installed with the executable | Exact State/capability ABI validation |
| [engine.rs](crates/kernel/runtime/src/engine.rs): classifier veto in `decide` | Remove assessment-return tuple and Nonrecoverable branch; authorize the handler request once | Phase restrictions, recovery counters and Effect barriers |
| Same file: pending operational error no-append path | Replace special ephemeral Retry/Stopped construction with audited frame candidate and shared append/fold discipline | Same command/EffectId and caller-driven progression |
| [fold.rs](crates/kernel/runtime/src/engine/fold.rs) | Reuse one fold for new failure records and recovery counters; avoid a second pending-attempt reducer or mutable audit store | Original retained inputs, exact history validation and command authority |
| [conclusion.rs](crates/kernel/journal/src/conclusion.rs): `Nonrecoverable` stop code | Delete the obsolete classifier-veto reason across wire and renderers | Requested, exhausted and Runtime-denied reasons; pending decisions exclude Restart structurally |
| [report.rs](crates/kernel/runtime/src/report.rs), [run_view.rs](crates/app/src/run_view.rs) | Remove old reason translation and any policy-mapped reporting dependencies; reuse qualified original/context ownership for latest pending failure | Original/root domain failure report and redacted public codes |
| [EVM recovery](crates/domains/evm/src/recovery.rs): `EvmBalanceClassifier` | Delete selectable classifier and its identity; move exhaustive semantics to domain/capability error implementations | Precise typed error variants and State context |
| [Portfolio recovery tests](crates/domains/portfolio/tests/recovery_policy.rs) and [Runtime recovery tests](crates/kernel/runtime/src/assembly/recovery/tests.rs) | Replace family/registration scaffolding and synthetic policy context maps; delete tests solely asserting removed machinery | Consuming inheritance/override behavior, root-map tests, original causes, retry/restart safety |

Do not remove a mechanism merely because its name contains "recovery" or "map". In particular,
the [checkpoint scope implementation](crates/kernel/program/src/recovery/scope.rs) is already small
and enforces a real boundary. Keep it and simplify its callers. Preserve size checks, atomicity,
secret redaction and runtime authorization even if they account for substantial LOC.

### LOC and complexity evidence expected from implementation

At RFC revision time the main files contain the following physical line counts, including comments
and any inline tests. These are inspection baselines, not promised deletion totals:

| File | Lines |
| --- | ---: |
| Program `recovery.rs` | 362 |
| Program `recovery/bindings.rs` | 456 |
| Program `recovery/defaults.rs` | 132 |
| Runtime `assembly/recovery.rs` | 311 |
| Program `recovery/scope.rs` | 64 |
| Program `recovery/bounds.rs` | 184 |

The first four files total 1,261 lines but contain retained contracts too. Do not claim all those
lines are removable. Report actual `git diff --numstat` evidence at implementation review, split
into production code, tests, and documentation, and explain code moved between files.

Report classifier/handler simplification separately from the required audit additions. The former
should reduce production LOC and public concepts; the latter may add necessary Journal/fold code.
A net total can conceal either retaining unnecessary machinery or weakening audit requirements.
No numerical savings target justifies code compression, giant functions, omitted tests or weaker
contracts.

The reviewer should be able to verify that classifier-family APIs, per-incident handler APIs,
policy mapping callbacks, duplicate inheritance paths and classifier-veto codes no longer exist
in executable code. Search their concrete old symbols, inspect each remaining match, and preserve
references only where necessary to explain the historical proposal. Each old responsibility must
have one current owner, not a compatibility wrapper around the old implementation.

## Verification and acceptance criteria

Tests must exercise observable contracts rather than freeze the proposed helper names.

1. A real Operation with heterogeneous child State/adapter errors inherits one handler; no
   per-incident handler registration is required. A State occurrence override changes only that
   occurrence, and nested default replacement has deterministic scope.
2. Different exact adapter errors classify server nonacceptance and uncertain local timeout
   differently. A duplicate-safe Read may classify a local deadline as retryable. Unknown provider
   response semantics never produce unsupported nonacceptance evidence.
3. Missing classification at executable integration is rejected at compile time. Capability
   contracts remain usable without depending on Program.
4. Original typed error variants and reviewed payloads survive conclusion/report construction;
   State adapter context and original/root domain failure distinctions remain intact.
5. Internal State/adapter/preparation failures bypass classification and handler recovery, preserve
   the current head or acknowledged prepare, and never durably masquerade as provider failures.
6. Read retry and checkpoint restart spend the appropriate committed budgets. Pure retry, settled
   Effect retry, invalid checkpoints and restart across an earlier Effect remain denied.
7. Pending failure Retry and Stop both append the exact original/context and authorized decision.
   Retry spends local/global recovery allowance without changing position, visit, command or
   EffectId. StandardRecovery stops on OutcomeUnknown; a custom pending retry still records that
   original cause, proving there is no classifier veto. Explicit resume uses the retained command.
8. Cancellation and ambiguous append acknowledgement preserve all-or-nothing history. Hot and cold
   outcomes agree, and completed reconstruction never invokes classifiers or handlers.
9. Handler identity/parameter mismatch and superseded Program descriptors are rejected. Program
   identity changes with changed selected handler/configuration and revised failure semantics.
10. Failure-record capacity counts Stop as well as Retry and prevents adapter entry on exhaustion;
    retry exhaustion records Stop. Bounds include failure frames and preserve settlement capacity.
    Existing original/root value and report-overflow coverage remains meaningful. Shipping
    Portfolio still stops with zero recovery budgets.
11. Journal accepts prepare/failure*/settlement and all complete prefixes, rejects post-settlement
    failure records, and verifies object closure and byte bounds. Runtime fold rejects mismatched
    execution positions. Cold pending views expose the latest original failure and decision
    without invoking policy.
12. Cancellation, ambiguous failure append and competing exact-head candidates never acknowledge
    an uncommitted failure or change the pending command. Tests distinguish durable acknowledged
    outcomes from physical attempts interrupted before their results can be recorded.

Classification coverage is reviewed case by case while translating each current consumer. That
review is normal implementation work, not a prerequisite for handing off the architecture. Do not
silently drop a required policy behavior; document any demonstrated need to refine the common
classification before extending it.

Follow [docs/build-and-verification.md](docs/build-and-verification.md). For implementation, start
with the affected Program/Runtime/domain tests in the default Nix shell, then cover Journal,
Application and transports when their contracts change. Representative focused commands are:

```sh
nix develop -c cargo test -p mfm-program -p mfm-journal --all-targets
nix develop -c cargo test -p mfm-runtime --all-targets
nix develop -c cargo test -p mfm-evm -p mfm-portfolio -p mfm-evm-live --all-targets
```

The cross-crate contract cutover requires one final `nix run .#ci` on the exact candidate after
focused failures are resolved. Do not run broad component gates redundantly just before CI.
The earlier RFC-only commits required local link/command review and `git diff --check`; the
implementation requires the complete verification selection above.

## Logical commit sequence

1. `document classification and handler simplification`: this proposed RFC, with links and
   documentation verification. Existing contracts remain current.
2. `replace recovery families with intrinsic error classification`: one inseparable kernel,
   domain, authoring, registration, wire/report, consumer, test and documentation cutover. Delete
   superseded APIs in this commit. Do not split it into intermediate incompatible public designs.

The second commit includes the required pending failure Journal variant, admission bounds,
fold/accounting, reports and tests. Those are inseparable from the new acknowledged-failure audit
contract. Independent additional scenarios may land later only if they are not required to
validate the cutover. Do not defer a required deletion or audit guarantee to a follow-up.

## Alternatives considered

- Per-State arbitrary function pairs: simple locally, but heterogeneous Operation defaults need
  another dispatch or conversion mechanism. Intrinsic classification supplies the common input.
- `RecoveryPolicy<Incident>` implemented for each child type: statically typed, but repeats handler
  implementations and makes inherited Operations generic over the policy. This RFC avoids it.
- Shared Operation error enum: permits one typed handler but requires child-to-Operation error
  conversion and extra policy-facing types. Original errors already provide the necessary facts.
- `RecoveryAdvice` alongside classification: duplicates the role of the classifier and adds no
  necessary responsibility.
- Blanket stop for EffectPending: ignores protocol-specific evidence that the identical command
  can be attempted again. Runtime phase and typed classification must be considered together.
- Blanket retry for timeout: confuses deadline location with remote acceptance and duplicate
  safety. Concrete adapter contracts must establish the distinction.
- Keep selectable classifiers and merely hide family maps: preserves the old complexity and two
  current APIs. This RFC selects one replacement design and deletes the superseded mechanism.

## Material uncertainties

none for the implemented cutover. Known consumers fit the four classifications. Current EVM
submission errors do not establish server nonacceptance and retain `OutcomeUnknown` where dispatch
is uncertain. The protocol-specific test uses an explicitly qualified nonacceptance contract.
The breaking API deliberately removes independently selectable classifiers. Audit guarantees cover
acknowledged operational results, with finite admitted capacity; they do not claim to identify every
physical attempt interrupted before its result can be committed.
