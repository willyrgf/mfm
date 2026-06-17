# RFC: Refactor Runtime Scheduler Into Explicit FSM Lifecycles

Status: draft

This RFC proposes a breaking refactor of the typed runtime scheduler. The goal is to make runtime
execution easier to reason about by splitting the current broad scheduler orchestration into small,
explicit lifecycle protocols.

The design target is:

```text
verified history + certified spec
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

- Make frontier scheduling pure and small: certified spec plus verified history in, one transition
  decision out.
- Make transition dispatch explicit: starting a state, continuing an attempt, projecting retention,
  rendering public output, completing, retrying, compensating, remediating, or blocking are named
  decisions.
- Make attempt execution a first-class lifecycle with fixed typed phases.
- Persist `StateAttemptStarted` before runner/state-handler execution for every semantic attempt.
- Require every started attempt to reach terminal evidence or be owned by recovery before the run
  can advance past it.
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

The refactor introduces four lifecycle protocols:

```text
TransitionLifecycle
AttemptLifecycle
AttemptRecoveryLifecycle
SideEffectLifecycle
```

These are protocols, not plugin hook chains. Each phase has typed input authority, typed output, and
bounded commit authority.

### Transition Lifecycle

The transition lifecycle decides what the runtime should do next.

Input:

- `CertifiedRuntimeSpec`
- `VerifiedRunHistory` or a shared verified run view
- currently blocked resource-lane hints, if any

Output:

```rust
enum TransitionDecision {
    StartNode,
    ContinueAttempt,
    StartCompensation,
    StartRemediation,
    AwaitManualResolution,
    ProjectPublicOutput,
    ProjectRetentionManifest,
    CompleteRun,
    Blocked,
}
```

Rules:

- The decision is pure.
- The decision does not write the store.
- The decision does not stage artifacts.
- The decision does not invoke runners, handlers, capabilities, transports, or signers.
- Saga, retry, compensation, remediation, public-output, retention, and completion routing belongs
  here as explicit transition decisions.

The transition lifecycle dispatches exactly one lifecycle action. It does not execute state logic
itself.

### Attempt Lifecycle

The attempt lifecycle executes one selected certified node attempt.

```text
selected transition decision
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

Minimal preflight exists only to derive stable attempt identity and prove that the selected node is
still startable from the verified history. Expensive or failure-prone work should happen after
`StateAttemptStarted`, so failures can be captured as terminal evidence.

Examples of work that should happen after attempt start:

- input materialization
- config artifact materialization
- runner binding lookup
- executable identity checks
- capability binding checks
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
- `StateAttemptInterrupted` or an equivalent explicit failure reason

`StateAttemptInterrupted` can be a new event, or it can be represented as `StateAttemptFailed` with
a stable structured reason. The important property is that interruption and recovery are
distinguishable from a domain-state failure when operators, replay, and tests need that distinction.

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

- A run with an open attempt cannot choose unrelated next work until recovery resolves ownership.
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
- Signed raw transactions and bearer mutation material remain transient and must not be retained as
  typed semantic artifacts.
- Resource lanes, idempotency input, claim owner, fencing token, invocation epoch, ledger purpose,
  and replay verifier identity remain explicit evidence.

The generic attempt lifecycle delegates side-effect phase advancement and recovery to
`SideEffectLifecycle` rather than hiding it in scheduler branches.

## Type Enforcement And Runtime Validation

The lifecycle rules should be encoded in types wherever possible. The target is not to rely on
convention or comments for phase ordering, authority boundaries, or capability access. Types should
make illegal control flow unrepresentable, while focused runtime validation checks facts that come
from persisted streams, artifact bytes, certified spec contents, registries, or external evidence.

### Type-Enforced Boundaries

The strongest type boundaries should be:

- `FrontierScheduler::decide` accepts only certified spec authority, a verified run view, and pure
  scheduling hints. It does not receive stores, artifact stores, runners, capabilities, transports,
  or signers.
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
- Starting unrelated work requires a `NoOpenAttempt` or equivalent witness from verified history.
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
- Saga retry, compensation, remediation, manual resolution, public-output, retention, and
  completion decisions match the certified spec plus stream projection.
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
| Transition decision is pure | No store, runner, artifact, transport, signer, or capability handles in the decision API | Correct decision still depends on certified spec plus verified history |
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
| `VerifiedRunContextLoader` | Load stream, verify spec/certificate/artifacts, construct verified history view | Runner execution, transition decisions |
| `FrontierScheduler` | Pure transition decision from certified spec plus verified history | Store writes, artifact staging, live IO |
| `TransitionLifecycle` | Dispatch one transition decision to the correct lifecycle | State execution details |
| `AttemptLifecycle` | Start, run, terminalize, or hand off one attempt | Pure frontier selection |
| `AttemptRecoveryLifecycle` | Resolve open attempts after interruption | Inventing output or bypassing side-effect evidence |
| `InvocationBuilder` | Materialized inputs, config, capabilities, recorded facts, sealed runner context | Store commits |
| `RunnerExecutor` | Call one sealed runner/state handler | Event validation, store writes |
| `RunnerOutputPlanner` | Validate runner proposals against certified spec and history | Live IO, artifact persistence |
| `ArtifactStager` | Stage verified artifact bytes before commit | Admitting run-store evidence independently |
| `CommitPlanner` | Build purpose-specific prepared commits | Executing handlers |
| `SideEffectLifecycle` | Mutation phase legality, recovery, uncertainty handling | Generic scheduler decisions |
| `FrameworkLifecycle` | Bootstrap, public output, retention, completion, manual terminal framework attempts | Domain state semantics |

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

Failures after `StateAttemptStarted` is committed are semantic attempt failures and must be
terminalized in the run stream when storage is available.

### Started Attempt Rule

Every executable semantic lifecycle should commit `StateAttemptStarted` before running its handler,
including framework lifecycle nodes such as:

- `PublicOutputRender`
- `ProjectRetentionManifest`
- `CompleteRun`
- `ResolveSagaTerminal`

This is a breaking event-order change where any current framework path starts and terminalizes in
one commit. The new event order is more explicit and gives crash recovery the same model for
framework and domain attempts.

### Terminal Failure Planning

Any fallible phase after attempt start should map to terminal failure planning:

- invocation materialization failure
- missing or incompatible runner binding
- capability mismatch
- handler error
- invalid runner output
- artifact binding mismatch
- public-output render failure
- retention manifest projection failure

If terminal failure planning or commit fails because storage is unavailable, recovery handles the
open attempt on resume.

### Blocking Is Not Failure

Resource-lane contention, waiting for manual resolution, or waiting for side-effect recovery can
leave an attempt open or block a transition without terminalizing as failure. The invariant is not
"every selected attempt finishes immediately." The invariant is:

```text
the run cannot advance past an open attempt except through that attempt's lifecycle or recovery
authority.
```

## Proposed Module Shape

One possible runtime crate layout:

```text
crates/kernel/runtime/src/
  scheduler.rs              // facade, thin public API
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

- ordinary state success
- ordinary state handler failure
- invocation materialization failure after attempt start
- invalid runner output after attempt start
- framework public-output, retention, completion, and manual terminal paths
- resource-lane blocking with open attempt ownership
- open attempt recovery on resume
- side-effect recovery before and after `InvocationStarted`
- no secrets in terminal failure events, artifacts, public outputs, or errors
- event-order goldens for old and new attempt lifecycle behavior

### Phase 1: Extract Pure Transition Decision

- Rename or wrap the existing frontier decision as `FrontierScheduler`.
- Ensure it accepts only certified spec plus verified history.
- Make saga, retry, compensation, remediation, public-output, retention, completion, and blocked
  outcomes explicit `TransitionDecision` variants.
- Keep existing behavior behind the old scheduler facade.

### Phase 2: Introduce Attempt Lifecycle Types

- Add `AttemptLifecycle`, `AttemptStart`, `AttemptInvocation`, `AttemptTerminalPlan`, and
  `AttemptTerminalCommit` types.
- Route ordinary state attempts through the lifecycle with no intended event-shape change yet.
- Keep store writes in `CommitPlanner` and store admission APIs.

### Phase 3: Terminalize Observed Failures

- Add the failure-safe terminalization path.
- Convert handler errors, materialization errors, and output validation errors after attempt start
  into redacted terminal attempt evidence.
- Add recovery behavior for failures that happen before terminal commit succeeds.

### Phase 4: Unify Framework Lifecycles

- Route public-output, retention, completion, and manual terminal framework states through the same
  attempt lifecycle.
- Split current same-commit framework attempts into started-before-run and terminal commits, or
  deliberately document any framework exception that remains.
- Update replay, status, and public-output tests for the new event order.

### Phase 5: Attempt Recovery Lifecycle

- Detect open attempts from verified history.
- Prevent unrelated transition decisions while an open attempt is unresolved.
- Resume, terminalize, mark interrupted, or delegate to side-effect recovery based on certified
  state effect and recorded evidence.

### Phase 6: Side-Effect Lifecycle Boundary

- Extract side-effect phase handling behind `SideEffectLifecycle`.
- Keep store ledger typestate as the transition authority.
- Add recovery tests for every uncertainty phase.
- Only after this lands should a generic side-effect adapter driver be considered.

### Phase 7: Remove Scheduler Bulk

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

- open attempt blocks unrelated work
- open attempt recovery after process interruption
- side-effect open attempt after `InvocationStarted` cannot be generically failed without recovery
  evidence
- framework lifecycle nodes follow the same start/run/terminal model
- storage failure before terminal commit leaves recoverable open attempt state
- invalid runner output becomes redacted failure evidence, not a panic or silent block

## Open Questions

No open design questions remain for the first cut. The following decisions define the implementation
direction:

- Interruption should be a new `StateAttemptInterrupted` event, not a structured
  `StateAttemptFailed` reason. `StateAttemptFailed` means the attempt reached a
  semantic/runtime-evaluable failure. `StateAttemptInterrupted` means the attempt started but the
  runtime could not observe or complete the lifecycle cleanly.
- Missing runner or capability binding should be unrepresentable before attempt start. Constructing
  `Attempt<Started>` should require executable and capability binding authority. If a missing
  binding somehow becomes observable after start, terminalize it as a redacted
  `StateAttemptFailed` with a stable configuration/binding error class; this is a code or
  deployment fix, not a workflow retry.
- Framework lifecycle states should not have same-commit exceptions in the first design. Bootstrap,
  public-output rendering, retention projection, completion, and manual terminal resolution should
  follow the same start/run/terminal lifecycle as domain states unless a later RFC justifies a
  specific exception.
- Pre-authority failures are not semantic runtime states. If certified runtime authority or
  verified run history cannot be constructed, no transition, attempt, recovery, or semantic append
  API is reachable. These failures may be reported as redacted ingress/corruption diagnostics, but
  they do not enter the typed run stream.
- Sync/async service collapse is not a goal of this RFC. Lifecycle components should be designed so
  a later async-primary cleanup can share the same lifecycle semantics, but this refactor should not
  mix lifecycle authority changes with broad service plumbing changes.

## Bottom Line

The runtime should expose two simple truths in code:

```text
Transition lifecycle decides what should happen next.
Attempt lifecycle makes every started execution attempt durable, terminal, or recoverable.
```

That gives MFM the simpler scheduler model originally intended while preserving the hard authority
contracts: certified specs, append-only streams, guarded commits, replay from evidence, explicit
side-effect uncertainty, and no secret persistence.
