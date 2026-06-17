# RFC: Refactor Runtime Scheduler Into Explicit FSM Lifecycles

Status: draft

This RFC proposes a breaking refactor of the typed runtime scheduler. The goal is to make runtime
execution easier to reason about by splitting the current broad scheduler orchestration into small,
explicit lifecycle protocols.

The design target is:

```text
authored run material
  -> run admission lifecycle certifies, binds, and commits run authority
  -> verified history + certified spec + bound runtime context
  -> transition lifecycle chooses one pure decision
  -> attempt lifecycle starts durable attempt authority
  -> runner/state handler executes under that attempt
  -> terminal evidence is planned
  -> terminal evidence is committed or recovery takes ownership
```

The scheduler should not be a bag of hidden runtime responsibilities. It should be a thin dispatch
surface over named lifecycle components with narrow authority.

## Problem

The current runtime model is architecturally sound but too much of it is concentrated behind
`SerialTypedScheduler`. Today the scheduler path owns or coordinates:

- verified history rebuild
- frontier decision
- invocation preparation
- runner lookup and execution
- framework lifecycle special cases
- artifact staging
- runner-output commit planning
- resource-lane retry behavior
- manual resolution terminal proof handling
- public-output, retention, and completion framework paths

That makes the runtime harder to audit than the design contract suggests. The design says the
frontier scheduler is pure. The implementation should make that fact obvious in the type and module
boundaries.

The bigger issue is failure auditability. MFM wants complete durable evidence for every execution
attempt that enters the semantic runtime. If a selected state attempt starts and then materialization,
handler execution, output validation, artifact staging, or terminal commit planning fails, the run
stream should not silently advance or lose the attempt. The attempt must either commit terminal
evidence or be recovered later from the open attempt.

## Goals

- Make frontier scheduling pure and small: certified spec plus bound runtime context plus verified
  history in, one transition decision out.
- Make transition dispatch explicit: starting a state, continuing an attempt, projecting retention,
  rendering public output, completing, retrying, remediating, resolving saga terminal outcomes, or
  blocking are named decisions.
- Make attempt execution a first-class lifecycle with fixed typed phases.
- Persist `StateAttemptStarted` before runner/state-handler execution for every semantic attempt.
- Require every started attempt to reach terminal evidence, interruption, or recovery ownership
  before dependent runtime work can advance past it.
- Capture observed failures as redacted terminal attempt evidence whenever storage is available.
- Preserve append-only run stream authority, content addressing, canonical JSON, replay evidence,
  and no-secret persistence.
- Keep state handlers and runners unable to write the store directly.
- Expose small responsible components so tests can target each authority boundary.

## Non-Goals

- Do not introduce arbitrary dynamic pre/post hooks.
- Do not let state handlers, adapters, or transports append semantic run events directly.
- Do not treat rendered public output, projection rows, logs, or operational diagnostics as runtime
  authority.
- Do not make replay construct live capabilities or signer providers.
- Do not use this refactor to weaken side-effect uncertainty handling.
- Do not try to audit arbitrary in-process CPU steps. MFM can audit runtime phase boundaries,
  typed inputs/outputs, facts, artifacts, side-effect evidence, and redacted failures.

## Core Design

The refactor introduces one pre-FSM admission protocol and four FSM lifecycle protocols:

```text
RunAdmissionLifecycle
TransitionLifecycle
AttemptLifecycle
AttemptRecoveryLifecycle
SideEffectLifecycle
```

These are protocols, not plugin hook chains. Each phase has typed input authority, typed output, and
bounded commit authority.

### Run Admission Lifecycle

The run admission lifecycle exists before the FSM runtime is allowed to make transition decisions.
Its job is to turn authored operation/state material into a certified, bound, append-only run root.

Input:

- authored operation/state material
- deterministic expansion output
- certified spec and certificate artifacts
- configured runner, capability, and framework registries
- staged genesis artifacts needed to establish the run root

Output:

- atomic run genesis authority, including `RunStarted`
- `CertifiedRuntimeSpec`
- `BoundRuntimeContext`
- first verified run history view

Rules:

- Run admission is not a state attempt.
- Run admission does not execute domain state logic.
- Run admission must verify spec/certificate material before any semantic run lifecycle exists.
- Run admission must prove runner, capability, and framework bindings are available before the FSM
  can construct executable attempts.
- Bootstrap/genesis behavior belongs here, not as a scheduler exception inside the FSM.
- If admission fails, the failure is an ingress, corruption, or deployment diagnostic. It is not a
  semantic run event.

This is the replacement direction for the historical `Bootstrap` special case. Bootstrap has been
an attractive shortcut because it can atomically seed run state, completion state, artifacts, and
retention references. That atomicity is still useful, but it should be modeled as pre-FSM run
admission authority rather than as a framework state that sometimes bypasses normal lifecycle rules.

### Bound Runtime Context

`BoundRuntimeContext` is the authority object that proves a certified spec is executable in the
current runtime assembly.

It is constructed from:

- `CertifiedRuntimeSpec`
- runner registry
- capability registry
- framework handler registry
- deployment/runtime configuration relevant to executable binding

It proves:

- every certified executable state has a runner binding
- every certified capability reference can be bound to an allowed capability implementation
- framework lifecycle handlers needed by the certified spec are available
- binding identities match the certified spec and deployment policy

`TransitionLifecycle` and `AttemptLifecycle` should require this bound context, not raw registries.
That makes missing runner or capability bindings unrepresentable after run admission. A missing
binding is a pre-authority deployment/configuration failure unless the bound context was constructed
incorrectly, in which case the implementation has violated its own invariant.

### Transition Lifecycle

The transition lifecycle decides what the runtime should do next.

Input:

- `CertifiedRuntimeSpec`
- `VerifiedRunHistory` or a shared verified run view
- `BoundRuntimeContext`
- currently blocked resource-lane hints, if any

Output:

```rust
enum TransitionDecision {
    StartNode,
    ContinueAttempt,
    StartRemediation,
    AwaitManualResolution,
    ProjectPublicOutput,
    ProjectRetentionManifest,
    ResolveSagaTerminal,
    CompleteRun,
    Blocked,
}
```

Rules:

- The decision is pure.
- The decision does not write the store.
- The decision does not stage artifacts.
- The decision does not invoke runners, handlers, capabilities, transports, or signers.
- Saga retry, remediation, manual-resolution, public-output, retention, and completion routing
  belongs here as explicit transition decisions.
- Terminal saga decisions must not imply a stronger AC/DC claim than the verified evidence proves.
  Concrete variants may split further during implementation, for example compensated completion,
  failed-without-ACDC-claim, and manually resolved terminal states.

The transition lifecycle dispatches exactly one lifecycle action. It does not execute state logic
itself.

### Attempt Lifecycle

The attempt lifecycle executes one selected certified node attempt.

```text
selected transition decision + bound runtime context
  -> minimal preflight
  -> pre-commit StateAttemptStarted
  -> materialize invocation
  -> execute runner/state handler
  -> plan terminal evidence
  -> stage terminal artifacts
  -> post-commit terminal evidence
```

The important invariant is:

```text
Once StateAttemptStarted is committed, the run must not advance past that attempt until a terminal
attempt outcome is committed or recovery takes ownership.
```

Minimal preflight exists only to derive stable attempt identity, prove that the selected node is
still startable from the verified history, and select already-bound executable authority from
`BoundRuntimeContext`. Expensive or failure-prone work that belongs to the attempt should happen
after `StateAttemptStarted`, so failures can be captured as terminal evidence.

Examples of work that should happen after attempt start:

- input materialization
- config artifact materialization
- bound runner context construction
- recorded fact materialization
- state handler or runner execution
- output validation
- artifact and retention binding validation

The runner/state handler still cannot write the store. It returns typed output proposals:

- typed cell output
- read facts
- side-effect phase evidence
- public-output evidence
- staged artifacts
- staged retention refs
- redacted failure details

Runtime validates those proposals and builds store-owned prepared commits.

### Terminal Attempt Outcomes

A started attempt must eventually reach one terminal attempt outcome:

- `StateAttemptCompleted`
- `StateAttemptFailed`
- `StateAttemptInterrupted`

`StateAttemptInterrupted` is a distinct event. It is not an alias for `StateAttemptFailed`.
`StateAttemptFailed` means the attempt reached a semantic/runtime-evaluable failure.
`StateAttemptInterrupted` means the attempt started but the runtime could not observe or complete
the lifecycle cleanly.

Interruption terminalizes attempt bookkeeping, but it does not by itself:

- engage saga remediation
- prove a compensated outcome
- prove an AC/DC terminal claim
- close side-effect uncertainty after `InvocationStarted`
- authorize manual saga resolution

Failure terminalization must use a failure-safe path. If a runner returns invalid output, runtime
must not depend on that invalid output to terminalize the attempt. It should be able to produce a
redacted diagnostic artifact and a `StateAttemptFailed` event from:

- run id
- spec hash
- node id
- attempt id
- attempt number
- redacted error class
- optional redacted diagnostic artifact evidence

Terminal evidence must not contain secrets, raw signed transactions, private keys, mnemonics,
passwords, authorization headers, local endpoint details, or bearer mutation material.

### Failure Taxonomy

Not every runtime problem after selection should become the same semantic event.

| Failure class | Runtime surface |
|---|---|
| Domain/state handler failure observed inside a valid started attempt | `StateAttemptFailed` |
| Runner contract violation, invalid output, schema mismatch, or artifact evidence mismatch inside a valid started attempt | Redacted `StateAttemptFailed` if failure-safe terminalization is possible |
| Process kill, panic, machine loss, or store/artifact outage before terminal evidence commits | Open attempt recovery, usually ending in retry, terminal evidence, `StateAttemptInterrupted`, or operational block |
| Missing runner/capability/framework binding | Pre-authority deployment diagnostic from `RunAdmissionLifecycle` or `BoundRuntimeContext` construction |
| Corrupt spec, invalid certificate, unverifiable history, missing retained bytes needed for authority | Redacted ingress/corruption diagnostic, not a semantic run event |
| Side-effect uncertainty after `InvocationStarted` | `SideEffectLifecycle` recovery; never generic interruption by itself |

The invariant is:

```text
only failures inside a valid started attempt can become semantic terminal attempt evidence.
authority, corruption, and deployment failures remain outside the typed run stream.
```

### Attempt Recovery Lifecycle

Some failures cannot be terminalized at the moment they occur:

- process kill
- machine loss
- runtime panic
- store outage
- artifact-store outage before terminal commit
- crash after artifact staging but before store append

The recovery lifecycle owns open attempts found in verified history.

Input:

- certified spec
- verified run history
- open attempt projection
- retained artifacts, if any

Output:

- resume the attempt
- retry terminalization
- commit `StateAttemptInterrupted`
- block for manual recovery
- delegate to side-effect recovery when the open attempt crossed a mutation uncertainty boundary

Rules:

- An open attempt must have explicit ownership disposition before the scheduler can choose other
  work. Unrelated work is legal only when verified history provides a scoped independence witness,
  such as the selected per-node or resource-lane policy. Otherwise recovery owns the next step.
- Orphan artifact-store bytes are not authority. Only admitted run-store artifact evidence tied to
  the append-only stream is authority.
- Recovery is evidence-driven and must not invent successful output.

### Side-Effect Lifecycle

Side-effecting states need a specialized lifecycle because external mutation uncertainty is not the
same as pure/read state failure.

Side-effect phases remain first-class:

- intent persisted
- claim acquired or taken over
- invocation prepared
- invocation started
- not-submitted proven
- submission observed
- submission unknown
- receipt observed
- confirmation observed
- ambiguity recorded
- terminal side-effect failure

Rules:

- `InvocationStarted` remains the durable uncertainty boundary.
- After that boundary, generic runtime failure cannot simply become ordinary `StateAttemptFailed`
  unless recovery evidence proves no external mutation ambiguity remains.
- `StateAttemptInterrupted` cannot close an attempt that crossed `InvocationStarted` by itself.
  Recovery must first prove one of the side-effect outcomes below.
- Signed raw transactions and bearer mutation material remain transient and must not be retained as
  typed semantic artifacts.
- Resource lanes, idempotency input, claim owner, fencing token, invocation epoch, ledger purpose,
  and replay verifier identity remain explicit evidence.

The generic attempt lifecycle delegates side-effect phase advancement and recovery to
`SideEffectLifecycle` rather than hiding it in scheduler branches.

After `InvocationStarted`, side-effect recovery must reach one of these evidence-backed outcomes:

- not submitted was proven
- submission, receipt, or confirmation was recovered
- ambiguity was recorded as `SideEffectAmbiguous` and paired with a non-retryable
  `StateAttemptFailed` in the same atomic commit

`SideEffectLifecycle` is orchestration over store/runtime ledger validators. It is not a second
source of ledger law. Active-attempt checks, legal side-effect phase transitions,
confirmation-before-output rules, ambiguity pairing, and terminal failure pairing remain enforced by
store/runtime admission.

### Saga And ACDC Invariants

Saga and AC/DC guarantees must compose with the new lifecycle split rather than sit beside it as
scheduler special cases.

Rules:

- Saga decisions are derived from certified spec, verified run history, and admitted side-effect
  evidence.
- `StateAttemptInterrupted` does not engage saga remediation and does not prove any terminal AC/DC
  claim.
- Operational manual recovery is not saga manual resolution. It may unblock an interrupted runtime
  lifecycle, but it must not emit `ManualResolutionRecorded` or mark a saga manually resolved.
- Manual saga resolution requires signed, prefix-bound `ManualResolutionProofAuthority` over a
  quiescent manually blocked prefix with no open semantic attempts.
- `ResolveSagaTerminal` must rebuild its `SagaTerminalProof` from the current `VerifiedRunHistory`
  immediately before commit. A cached proof cannot authorize terminalization after the run prefix
  changes.
- Transition decisions should use `StartRemediation` for executable recovery work. `Compensated` is
  a terminal outcome proven by admitted evidence, not a generic compensation lifecycle that can be
  started by name.
- After saga engagement, `ContinueAttempt` must not cross a new forward `InvocationStarted`
  boundary. It may only advance already-past-boundary forward ledgers toward quiescence or record
  safe pre-boundary failure/not-submitted evidence.
- Persisted diagnostics, including diagnostic artifacts, are non-authority. They affect replay,
  status, or terminal interpretation only when referenced by admitted semantic events in the same
  valid commit.

## Type Enforcement And Runtime Validation

The lifecycle rules should be encoded in types wherever possible. The target is not to rely on
convention or comments for phase ordering, authority boundaries, or capability access. Types should
make illegal control flow unrepresentable, while focused runtime validation checks facts that come
from persisted streams, artifact bytes, certified spec contents, registries, or external evidence.

### Type-Enforced Boundaries

The strongest type boundaries should be:

- `RunAdmissionLifecycle::admit` is the only path from authored run material to `RunStarted`
  genesis authority.
- `BoundRuntimeContext::construct` is the only path from raw runner/capability/framework registries
  to executable runtime authority.
- `FrontierScheduler::decide` accepts only certified spec authority, bound runtime authority, a
  verified run view, and pure scheduling hints. It does not receive stores, artifact stores,
  runners, capabilities, transports, or signers.
- Transition output is a closed `TransitionDecision` enum, not implicit control flow hidden in
  scheduler branches.
- Attempt execution uses typestate phases such as:

```rust
Attempt<Selected>
Attempt<Started>
Attempt<Invoked>
Attempt<TerminalPlanned>
Attempt<TerminalCommitted>
```

- Only a selected node plus `BoundRuntimeContext` can construct `Attempt<Selected>`.
- Only `Attempt<Started>` can build a sealed invocation.
- Only `Attempt<TerminalPlanned>` can stage terminal artifacts and submit terminal commit authority.
- Runner contexts do not carry store handles or artifact-store mutation handles.
- Runners return proposals, not committed events.
- Terminal outcomes are closed, for example:

```rust
enum TerminalAttemptOutcome {
    Completed,
    Failed,
    Interrupted,
}
```

- Failure terminalization has a failure-safe constructor that depends only on minimal trusted
  attempt authority, not on runner-provided output.
- Recovery APIs require explicit authority such as `OpenAttemptAuthority`.
- Starting unrelated work requires a `NoOpenAttempt`, scoped independence witness, or equivalent
  proof from verified history.
- Side-effect phases use typestate or sealed authorities such as:

```rust
SideEffect<IntentPersisted>
SideEffect<InvocationPrepared>
SideEffect<InvocationStarted>
SideEffect<Submitted>
SideEffect<Confirmed>
SideEffect<Ambiguous>
```

- After `SideEffect<InvocationStarted>`, generic attempt failure is not constructible without
  side-effect recovery authority proving that ordinary failure is safe.

### Validation-Backed Boundaries

Some invariants cannot be proven by Rust types alone because they depend on data loaded at runtime.
They require explicit validation even when the API shape is type-safe:

- The chosen `TransitionDecision` is legal for the certified spec and verified history.
- A node is still startable at the current stream head.
- Attempt id, attempt number, and open-attempt state match committed history.
- Saga retry, remediation, manual resolution, public-output, retention, and completion decisions
  match the certified spec plus stream projection.
- Runner output proposals match the certified output cell, schema id, semantic id, value lineage,
  capability binding, and recorded facts.
- Artifact evidence matches bytes, digest, byte length, media type, role, schema id, semantic id,
  and producer scope.
- Retained artifact evidence was admitted in the run stream; orphan artifact-store bytes are not
  authority.
- Side-effect phase transitions are legal from the committed ledger state.
- Resource lanes, idempotency input, claim owner, fencing token, invocation epoch, ledger purpose,
  and replay verifier identity match certified and committed evidence.
- Recovery evidence after process interruption or external mutation uncertainty is sufficient for
  the requested recovery action.
- Redacted diagnostics, terminal failure evidence, public outputs, artifacts, and errors contain no
  secrets or bearer mutation material.

The no-secret rule can be strengthened with redacted wrapper types and private constructors, but it
still needs tests and validation because arbitrary strings and bytes can accidentally carry secret
material.

### Rule Classification

| Rule | Type-enforced shape | Still requires validation |
|---|---|---|
| Run authority exists before FSM execution | `RunAdmissionLifecycle` is the only constructor for run genesis authority | Spec/certificate/artifact bytes and genesis commit must be verified |
| Runtime bindings exist before attempts | `BoundRuntimeContext` required to select executable attempts | Registries and deployment policy are runtime-loaded data |
| Transition decision is pure | No store, runner, artifact, transport, signer, or capability handles in the decision API | Correct decision still depends on certified spec, bound runtime context, and verified history |
| Transition does not write/stage/invoke | Capability-limited input type | No, if forbidden handles are absent |
| All transition routes are explicit | Closed `TransitionDecision` enum | Legal variant depends on history and saga state |
| Attempt start precedes handler execution | `Attempt<Started>` required to build invocation | Stream head must still allow that attempt |
| Started attempts must terminalize or recover | `OpenAttemptAuthority` and `NoOpenAttempt` witnesses | Open attempt discovery comes from verified history |
| Handler cannot write store | Runner context excludes store handles | No |
| Runner returns proposals, not commits | Proposal output type distinct from prepared commits | Proposals must be validated against spec/history |
| Terminal outcomes are closed | Closed terminal outcome enum | Terminal payload must match active attempt |
| Failure terminalization is failure-safe | Constructor from minimal trusted attempt authority | Redacted diagnostic evidence must be valid and non-secret |
| Orphan artifact bytes are not authority | Commit APIs accept only admitted/verified artifact evidence | Evidence must be checked against committed stream |
| Side-effect uncertainty boundary is enforced | `SideEffect<InvocationStarted>` cannot construct generic failure | Ledger state and recovery evidence must be validated |
| Raw signed/bearer material is not retained | Secret-bearing types are absent from semantic artifacts/events | Redaction/no-secret tests and byte/string validation remain required |

## Responsibility Split

The refactor should make these components explicit:

| Component | Owns | Must not own |
|---|---|---|
| `RunAdmissionLifecycle` | Certify authored run material, bind executable authority, commit atomic run genesis | Domain state execution, transition decisions |
| `BoundRuntimeContextLoader` | Construct executable runner/capability/framework binding authority from certified spec and registries | Store writes, transition decisions, handler execution |
| `VerifiedRunContextLoader` | Load stream, verify spec/certificate/artifacts, construct verified history view | Runner execution, transition decisions |
| `FrontierScheduler` | Pure transition decision from certified spec, bound runtime context, and verified history | Store writes, artifact staging, live IO |
| `TransitionLifecycle` | Dispatch one transition decision to the correct lifecycle | State execution details |
| `AttemptLifecycle` | Start, run, terminalize, or hand off one attempt | Pure frontier selection |
| `AttemptRecoveryLifecycle` | Resolve open attempts after interruption | Inventing output or bypassing side-effect evidence |
| `InvocationBuilder` | Materialized inputs, config, recorded facts, sealed runner context from bound authority | Store commits, registry binding lookup |
| `RunnerExecutor` | Call one sealed runner/state handler | Event validation, store writes |
| `RunnerOutputPlanner` | Validate runner proposals against certified spec and history | Live IO, artifact persistence |
| `ArtifactStager` | Stage verified artifact bytes before commit | Admitting run-store evidence independently |
| `CommitPlanner` | Build purpose-specific prepared commits | Executing handlers |
| `SideEffectLifecycle` | Orchestrate mutation phase recovery through store/runtime ledger validators | Generic scheduler decisions, independent ledger law |
| `FrameworkLifecycle` | Public output, retention, completion, and saga terminal framework attempts after run admission | Bootstrap/genesis authority, domain state semantics |

`SerialTypedScheduler` can remain as a facade during migration, but its implementation should become
thin orchestration over these components.

## Storage And Event Semantics

### Semantic Run Stream Boundary

Before verified run authority exists, no semantic runtime lifecycle exists. Transition, attempt,
recovery, and commit-planning APIs should require authority objects such as `CertifiedRuntimeSpec`
and `VerifiedRunHistory` or a shared verified run context. That makes semantic appends from
unverified persisted bytes unrepresentable in typed runtime APIs.

Stored spec/certificate/history verification failures are ingress or corruption failures, not run
events. They may be reported through redacted operational diagnostics, but they do not enter the
typed run stream.

Failures after `StateAttemptStarted` is committed belong to attempt lifecycle ownership and must be
terminalized, interrupted, or recovered when storage is available.

### Started Attempt Rule

Every executable FSM lifecycle should commit `StateAttemptStarted` before running its handler,
including ordinary domain nodes and post-admission framework lifecycle nodes such as:

- `PublicOutputRender`
- `ProjectRetentionManifest`
- `CompleteRun`
- `ResolveSagaTerminal`

This is a breaking event-order change where any current post-admission framework path starts and
terminalizes in one commit. The new event order is more explicit and gives crash recovery the same
model for framework and domain attempts.

Bootstrap/genesis is different. It should move to `RunAdmissionLifecycle` and remain an atomic
run-root commit unless a later RFC introduces a separate pre-run authority model. It should not be
used as evidence that the FSM scheduler needs special-case attempt ordering.

### Terminal Failure Planning

Fallible phases inside a valid started attempt should map to one of the terminal/recovery paths from
the failure taxonomy:

- invocation materialization failure
- handler error
- invalid runner output
- artifact binding mismatch
- public-output render failure
- retention manifest projection failure

Missing or incompatible runner/capability/framework bindings are not in this list because
`BoundRuntimeContext` must prove them before an attempt can start.

If terminal failure planning or commit fails because storage is unavailable, recovery owns the open
attempt on resume. If the attempt crossed `InvocationStarted`, side-effect recovery rules decide
which terminal evidence is legal.

### Blocking Is Not Failure

Resource-lane contention, waiting for manual resolution, or waiting for side-effect recovery can
leave an attempt open or block a transition without terminalizing as failure. The invariant is not
"every selected attempt finishes immediately." The invariant is:

```text
the run cannot treat an unresolved open attempt as satisfied, bypass its required terminal evidence,
or cross dependent work except through that attempt's lifecycle, recovery authority, or an explicit
independence witness.
```

## Proposed Module Shape

One possible runtime crate layout:

```text
crates/kernel/runtime/src/
  scheduler.rs              // facade, thin public API
  admission.rs              // RunAdmissionLifecycle and genesis authority
  binding.rs                // BoundRuntimeContext construction
  transition.rs             // TransitionLifecycle and TransitionDecision
  frontier.rs               // pure scheduler only
  attempt.rs                // AttemptLifecycle
  recovery.rs               // AttemptRecoveryLifecycle
  side_effect_lifecycle.rs  // SideEffectLifecycle wrapper over store ledger authority
  invocation.rs             // sealed runner context construction
  runner_output.rs          // runner proposal validation
  commit.rs                 // prepared commit builders
  artifacts.rs              // artifact evidence/staging helpers
  framework.rs              // framework node handlers, no scheduling
  history.rs                // verified history/view construction
```

Names can change during implementation. The important part is that each module has one authority
boundary.

## Migration Plan

### Phase 0: Baselines

Add or identify tests for:

- current Bootstrap/genesis event ordering and atomicity
- ordinary state success
- ordinary state handler failure
- invocation materialization failure after attempt start
- invalid runner output after attempt start
- framework public-output, retention, completion, and manual terminal paths
- missing runner/capability binding before attempt start
- resource-lane blocking with open-attempt ownership and independence witnesses
- open attempt recovery on resume
- side-effect recovery before and after `InvocationStarted`
- saga remediation and manual-resolution AC/DC paths
- no secrets in terminal failure events, artifacts, public outputs, or errors
- event-order goldens for old and new attempt lifecycle behavior

### Phase 1: Extract Run Admission And Binding Authority

- Extract the current Bootstrap/genesis behavior behind `RunAdmissionLifecycle`.
- Preserve atomic run-root behavior while moving it out of scheduler/framework attempt semantics.
- Add `BoundRuntimeContext` construction before transition or attempt execution.
- Make missing runner/capability/framework binding fail as a redacted deployment/admission
  diagnostic, not as a semantic attempt event.
- Keep existing runtime behavior behind the old scheduler facade while the authority boundary is
  introduced.

### Phase 2: Extract Pure Transition Decision

- Rename or wrap the existing frontier decision as `FrontierScheduler`.
- Ensure it accepts only certified spec, bound runtime context, verified history, and pure hints.
- Make saga retry, remediation, manual-resolution, public-output, retention, completion, and
  blocked outcomes explicit `TransitionDecision` variants.
- Decide explicitly whether open-attempt exclusion is global, per node, or resource-lane scoped.
  Do not silently change current scheduling semantics while extracting the lifecycle.
- Keep existing behavior behind the old scheduler facade.

### Phase 3: Introduce Attempt Lifecycle Types

- Add `AttemptLifecycle`, `AttemptStart`, `AttemptInvocation`, `AttemptTerminalPlan`, and
  `AttemptTerminalCommit` types.
- Route ordinary state attempts through the lifecycle with no intended event-shape change yet.
- Keep store writes in `CommitPlanner` and store admission APIs.

### Phase 4: Migrate Event Schema For Interruption

- Add `StateAttemptInterrupted` as a durable event, not a structured failure alias.
- Update event structs, stream store admission, projections, replay, app status, Postgres storage,
  and tests in one compatibility-aware slice.
- Define public status, retryability, and resource-lane behavior for interrupted attempts.
- Assert that interruption does not engage saga remediation and does not prove an AC/DC terminal
  outcome.

### Phase 5: Terminalize Observed Failures

- Add the failure-safe terminalization path.
- Convert handler errors, materialization errors, and output validation errors inside a valid
  started attempt into redacted terminal attempt evidence.
- Add recovery behavior for failures that happen before terminal commit succeeds.
- Keep authority/corruption/deployment failures outside the typed run stream.

### Phase 6: Classify Framework Lifecycles

- Keep Bootstrap/genesis in `RunAdmissionLifecycle`.
- Route public-output, retention, completion, and saga terminal framework states through the same
  post-admission attempt lifecycle where doing so preserves existing validators.
- Split current same-commit post-admission framework attempts into started-before-run and terminal
  commits only with coordinated replay/projection/storage tests.
- Update replay, status, and public-output tests for the new event order.

### Phase 7: Attempt Recovery Lifecycle

- Detect open attempts from verified history.
- Enforce the chosen open-attempt exclusion rule from Phase 2.
- Resume, terminalize, mark interrupted, or delegate to side-effect recovery based on certified
  state effect and recorded evidence.
- Keep operational manual recovery separate from saga manual resolution.

### Phase 8: Side-Effect And Saga/ACDC Boundary

- Extract side-effect phase handling behind `SideEffectLifecycle`.
- Keep store ledger typestate as the transition authority.
- Reject generic interruption/failure after `InvocationStarted` unless side-effect recovery proves
  not-submitted, recovers submission/receipt/confirmation, or records ambiguity with paired
  non-retryable failure.
- Rebuild saga terminal proofs from current verified history immediately before terminal commits.
- Add forward-fence tests so `ContinueAttempt` after saga engagement cannot cross a new
  `InvocationStarted` boundary.
- Add recovery tests for every uncertainty phase.
- Only after this lands should a generic side-effect adapter driver be considered.

### Phase 9: Remove Scheduler Bulk

- Reduce `SerialTypedScheduler` to a public facade over context loading, transition dispatch, and
  lifecycle execution.
- Delete duplicated sync/async orchestration paths where the shared lifecycle supports both.
- Update runtime docs to describe the new lifecycle protocols.

## Verification

Focused checks for implementation slices:

```bash
cargo test -p mfm-runtime
cargo test -p mfm-store --test commit_contract
cargo test -p mfm-replay
cargo test -p mfm-app
cargo test -p mfm-integration-tests --test architecture_namespace_contract
```

Additional required tests:

- run admission failure does not append semantic run events
- missing runner/capability binding fails before attempt start
- open attempt without an independence witness blocks unrelated work
- open attempt recovery after process interruption
- `StateAttemptInterrupted` is projected and replayed distinctly from `StateAttemptFailed`
- `StateAttemptInterrupted` does not engage saga remediation
- side-effect open attempt after `InvocationStarted` cannot be generically failed without recovery
  evidence
- side-effect ambiguity must pair with non-retryable terminal failure in the same atomic commit
- operational manual recovery cannot emit saga manual-resolution events
- stale `SagaTerminalProof` is rejected after the verified prefix changes
- `ContinueAttempt` after saga engagement cannot cross a new `InvocationStarted` boundary
- post-admission framework lifecycle nodes follow the same start/run/terminal model
- Bootstrap/genesis remains covered by run admission atomicity tests
- storage failure before terminal commit leaves recoverable open attempt state
- invalid runner output becomes redacted failure evidence, not a panic or silent block

## First-Cut Decisions

The following decisions define the first implementation direction:

- Interruption should be a new `StateAttemptInterrupted` event, not a structured
  `StateAttemptFailed` reason. `StateAttemptFailed` means the attempt reached a
  semantic/runtime-evaluable failure. `StateAttemptInterrupted` means the attempt started but the
  runtime could not observe or complete the lifecycle cleanly.
- Bootstrap/genesis belongs to `RunAdmissionLifecycle`, not the FSM attempt lifecycle.
- Missing runner or capability binding should be unrepresentable after `BoundRuntimeContext`
  construction. If binding cannot be proven, run admission or resume fails with a redacted
  deployment/configuration diagnostic before semantic attempt start.
- Post-admission framework lifecycle states should move toward the same start/run/terminal model as
  domain states, but only with explicit replay, projection, store, and validator migration tests.
- Operational manual recovery is separate from saga manual resolution. Saga manual resolution
  remains a signed, prefix-bound protocol over a quiescent manually blocked prefix.
- Pre-authority failures are not semantic runtime states. If certified runtime authority or
  verified run history cannot be constructed, no transition, attempt, recovery, or semantic append
  API is reachable. These failures may be reported as redacted ingress/corruption diagnostics, but
  they do not enter the typed run stream.
- Sync/async service collapse is not a goal of this RFC. Lifecycle components should be designed so
  a later async-primary cleanup can share the same lifecycle semantics, but this refactor should not
  mix lifecycle authority changes with broad service plumbing changes.

## Bottom Line

The runtime should expose three simple truths in code:

```text
Run admission mints run authority before the FSM exists.
Transition lifecycle decides what should happen next.
Attempt lifecycle makes started execution durable, terminal, interrupted, or recoverable.
```

That gives MFM the simpler scheduler model originally intended while preserving the hard authority
contracts: certified specs, append-only streams, guarded commits, replay from evidence, explicit
side-effect uncertainty, and no secret persistence.
