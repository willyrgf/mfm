# RFC: simplify error classification and recovery handlers

Status: proposed. This document records the agreed design direction and sketches its implementation;
it does not change the current executable or persistence contract. Until implementation lands,
[docs/design.md](docs/design.md) and [docs/architecture.md](docs/architecture.md) remain authoritative.

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

## Motivation and current design

The current [recovery contracts](crates/kernel/program/src/recovery.rs) already expose static
`Classifier::classify` and `Handler::handle` functions. The complexity lies around those functions:

- Independently inherited `Classifiers` and `Handlers` families select bindings by exact incident ABI.
- Classifier bindings include domain and adapter-context maps, their parameters, classifier
  parameters, and source/mapped contract relationships.
- Runtime registers and associates those combinations, constructs a mapped incident, and passes it
  between separately registered classifier and handler callbacks.
- Operation authors must account for heterogeneous child incident types in both families.

The current implementation supports independently replacing classification and mapping foreign
domain errors into a policy-specific vocabulary. This RFC deliberately replaces that flexibility
with intrinsic classification of exact error types and independently configurable recovery actions.

The shipping Portfolio currently selects `NoRecovery`/`Stop` with zero allowances. Its default
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
unbounded retry loops, or a new Effect execution protocol. It does not define durable records for
every pending Effect attempt; that separate audit requirement is discussed below.

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

All Rust below is an API sketch, not code that compiles against the current repository. Existing
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
eligible restart targets. For the minimal generic refresh policy, the handler binding permits at
most one explicitly selected invalidated-input target. Absence means no generic restart; the
handler does not guess the nearest checkpoint or infer a target from `InputInvalidated`.

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
                // Checked binding; returned only when active and eligible.
                context.invalidated_input_target()
                    .map(Restart)
                    .unwrap_or(Stop)
            }
            _ => Stop,
        })
    }
}
```

`invalidated_input_target()` is a proposed context accessor backed by the binding's checked target,
not an ambient lookup. General custom handlers may still use the existing explicitly bound target
list; this RFC does not require deleting supported multi-checkpoint scheduling. If a scope needs
several different semantic refresh regions, use narrower handler bindings or a reviewed custom
policy. Do not automatically expand the common classification into a registry of domain labels.

The framework default remains the generic `Stop` handler, now implementing `Handler` once.
`StandardRecovery` is explicitly selected in these examples, not a new shipping default.

## Operation inheritance and occurrence overrides

```rust
// Illustrative OperationExpansion authoring; omitted root maps remain required.
let refresh_anchor = scope.checkpoint::<CollectionInput>()?;

scope.handler(
    HandlerBinding::new::<StandardRecovery>(NoParams)?
        .invalidated_input_target(&refresh_anchor)?,
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

## Execution and Effect semantics

```text
typed State failure / typed adapter operational error
    -> Runtime preserves original cause and State context
    -> statically associated error classifier produces IncidentSummary
    -> selected handler requests Stop / RetryState / Restart(target)
    -> Runtime authorizes against phase, committed history and allowances
    -> existing conclusion append or pending-Effect invocation disposition
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
| Qualified nonacceptance timeout, EffectPending | RetryState | Yield with the same acknowledged command and EffectId; a later progress call attempts it again |
| Unknown dispatch outcome, EffectPending | Stop | Stop this invocation, expose typed cause, retain pending command for explicit resume |
| Retryable cause, EffectSettled | Stop | Settled Effects cannot be repeated through recovery |
| Retryable cause, Pure | Stop | Repeating deterministic execution with identical input cannot help |

Runtime independently rejects invalid requests even if a custom handler proposes them. An
`OutcomeUnknown` classification supplies no retry-safety evidence; a custom handler must not
reinterpret it as such. It is not a classifier veto: a caller-selected custom handler may request
`RetryState` for an unknown pending outcome under its retained-command recovery policy. Runtime
then yields with the same command authority. That choice does not establish nonacceptance or
permit a new command. Deliberate explicit resume of a retained Effect likewise remains an existing
caller-driven protocol operation, not a claim that the previous attempt did nothing.

This RFC preserves current pending-Effect behavior: a retry request yields; it neither re-enters
the adapter in a loop nor appends a retry frame, advances the visit, or spends a committed-decision
allowance. Consequently, existing allowances do not bound repeated caller-driven pending attempts.
The API/docs must not describe them as a durable automatic attempt budget.

An acknowledged Effect still blocks restart across its position, including when the current
failure is in a subsequent Pure or Read State. Settled Effect recovery remains disallowed. Store
receives no classification, policy or scheduling responsibility.

## Auditability and persistence

The original error is the audit fact. `Classification::OutcomeUnknown` is insufficient to explain
which adapter failed, its precise variant, or its reviewed public details. Preserve exact qualified
error values and State-owned context wherever the existing contract retains or exposes incidents.

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

There is an important existing limitation: pending Effect operational errors append nothing.
Stopped invocations expose the original typed incident, but pending attempts are not a complete
durable audit trail. A retry-yield currently does not promise a retained per-attempt error report
either. This RFC does not claim that preserving typed causes fixes that persistence limitation.

If durable auditability of every pending attempt is required, a separate complete design must add
attempt records, their exact wire and adjacency rules, cancellation/ambiguous-append semantics,
retention/capacity bounds, observation behavior, and durable retry accounting. That design must
preserve the same pending command authority. It must not be slipped into classifier removal as an
unreviewed change or described as already implemented.

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
| Runtime execution | Use common summary, preserve internal-error bypass, typed originals, exact pending command behavior and sole semantic fold |
| Persistence and reports | Update changed descriptors/codes and bounds, retain original/root failures, reject old incompatible formats, document pending audit limitations |
| Consumers | Update Portfolio, Application, transport rendering, fixtures, rustdoc and consuming tests; shipping defaults stay Stop/zero |

Do not mechanically delete all `MapAbi`, `ValueMap`, `Identity`, or erased value machinery: inspect
remaining root-map and codec responsibilities. Delete only superseded owners and paths, with no
compatibility shims or deprecation layer. Similarly, public `IncidentAbi` can be removed if no
remaining exact contract needs it; retain necessary incident qualification privately rather than
preserving an obsolete family-selection API.

Update [design](docs/design.md), [architecture](docs/architecture.md), affected crate READMEs,
transport docs and [recovery validation](docs/recovery-validation.md) in the implementation cutover.
The current contract documents are intentionally not rewritten by this proposal-only change.

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
7. Retryable pending Effect failure yields with exactly the same command, EffectId and head;
   StandardRecovery stops on OutcomeUnknown with the typed incident and preserves that same
   authority. A custom handler requesting RetryState for OutcomeUnknown yields with the same
   retained authority, proving there is no classifier veto. Explicit resume does not regenerate a
   new command. No unbounded in-call retry loop is introduced.
8. Cancellation and ambiguous append acknowledgement preserve all-or-nothing history. Hot and cold
   outcomes agree, and completed reconstruction never invokes classifiers or handlers.
9. Handler identity/parameter mismatch and superseded Program descriptors are rejected. Program
   identity changes with changed selected handler/configuration and revised failure semantics.
10. Bounds account for the final descriptor and report shapes; existing original/root value and
    report-overflow coverage remains meaningful. Shipping Portfolio still stops with zero budgets.

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
This RFC-only change requires local link/command review and `git diff --check`, not Rust gates.

## Logical commit sequence

1. `document classification and handler simplification`: this proposed RFC, with links and
   documentation verification. Existing contracts remain current.
2. `replace recovery families with intrinsic error classification`: one inseparable kernel,
   domain, authoring, registration, wire/report, consumer, test and documentation cutover. Delete
   superseded APIs in this commit. Do not split it into intermediate incompatible public designs.

Independent additional scenarios may land later only if they are not required to validate the
cutover. Durable pending-attempt auditing, if selected, needs its own complete design and coherent
implementation sequence; it is not prerequisite scaffolding for classifier simplification.

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

| Assumption | Why uncertain | Consequence if wrong | Validation |
| --- | --- | --- | --- |
| Four common classifications cover current recovery decisions | Existing custom policies can inspect mapped domain/context payloads | Some policy behavior cannot be expressed by the generic summary | Translate current EVM/Portfolio and nested Runtime policies before fixing the public enum; add facts only for demonstrated needs |
| Intrinsic classification can replace independently selectable classification | Repository production defaults do not establish downstream library usage | Downstream callers lose a customization point | Review known consumers and document the intentional breaking API; do not retain a compatibility family |
| One bound invalidated-input target suffices for the standard handler | Nested Operations may have several semantic refresh regions | The standard handler would restore the wrong input if allowed to guess | Exercise nested anchor acquisition and use explicit narrower bindings or a custom handler with checked targets |
| Pending-attempt auditability may remain at the current observation boundary for this simplification | Original-error retention was agreed, but durable recording of every pending attempt was not | This RFC alone would not meet a requirement for a complete durable attempt audit | Decide that requirement before claiming complete audit coverage; if needed, design the separate Journal/Runtime attempt-record cutover |
| Concrete adapters can support the illustrated timeout distinctions | The example server's nonacceptance guarantee is hypothetical, not established for current EVM providers | Misclassification could request unsafe repetition | Qualify actual protocol outcomes with adapter tests; use OutcomeUnknown when guarantees are absent |
