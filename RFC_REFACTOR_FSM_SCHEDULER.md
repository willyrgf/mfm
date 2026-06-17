# RFC: Refactor Runtime Scheduler Into Explicit FSM Lifecycles

Status: draft

This RFC proposes a breaking refactor of the typed runtime scheduler. The goal is to make runtime
execution easier to reason about by splitting the current broad scheduler orchestration into small,
explicit lifecycle protocols.

The design target is:

```text
authored run material
  -> run admission lifecycle verifies, binds, and commits run authority
  -> verified history + certified spec + bound runtime context
  -> transition lifecycle chooses one pure decision
  -> attempt lifecycle starts durable attempt authority
  -> runner/state handler executes under that attempt
  -> terminal evidence is planned
  -> terminal evidence is committed or recovery takes ownership
```

The scheduler should not be a bag of hidden runtime responsibilities. It should be a thin dispatch
surface over named lifecycle components with narrow authority.

## Change Classes

This RFC is an umbrella over three distinct change classes with different blast radii and review
obligations. They are sequenced so the low-risk work can land first and the high-risk work is never
mislabeled as "just a scheduler refactor":

- **Runtime decomposition.** Splitting `SerialTypedScheduler` into named lifecycle components behind
  the existing facade. Backward-compatible; no event-schema, certification, or storage change. This
  is the bulk of the RFC.
- **Event-schema migration.** Adding `StateAttemptInterrupted` and the failure-safe terminal path.
  Touches events, store admission/projection, replay, Postgres storage, and public status. Requires
  the interruption legality matrix and public-status semantics (below) before any code lands.
- **Certification/admission migration.** Anything that changes what the certified spec contains or
  what `RunStarted` means on the wire, including demoting `BootstrapRun` from the certified graph.
  Changes the spec hash and certificate and therefore replay/projection of persisted runs. This RFC
  deliberately does **not** perform this migration; it only consolidates the existing genesis path
  and records the decision boundary (see "Bootstrap Compatibility And Certification Scope").

The migration phases map to these classes as:

- Runtime decomposition: Phases 1, 2, 3, 7, 8, 9.
- Event-schema migration: Phases 4, 5, 6.
- Certification/admission migration: out of scope here; only the decision boundary is recorded.

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

That makes the runtime harder to audit than the design contract suggests. The frontier decision is
already pure today (`frontier::scheduler_decision_with_blocked_nodes` takes certified spec, a
verified view, and a blocked-node set, with no IO). The problem is not frontier purity; it is that
dispatch, attempt orchestration, framework special cases, resource-lane retry, and manual-proof
handling are concentrated in the scheduler facade. The refactor should isolate those responsibilities
so each authority boundary is obvious in the type and module structure.

The bigger issue is failure auditability. MFM wants complete durable evidence for every execution
attempt that enters the semantic runtime. If a selected state attempt starts and then materialization,
handler execution, output validation, artifact staging, or terminal commit planning fails, the run
stream should not silently advance or lose the attempt. The attempt must either commit terminal
evidence or be recovered later from the open attempt.

## Goals

- Make frontier scheduling pure and small: certified spec plus bound runtime context plus verified
  history in, one transition decision out.
- Make transition dispatch explicit: starting a node (domain or framework), continuing an attempt,
  remediating, awaiting manual resolution, resolving saga terminal outcomes, or blocking are named
  decisions. Framework specialization (public output, retention, completion) is derived from the
  selected node's `FrameworkNodeSpec`, not from a separate decision variant.
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

### Execution And Concurrency Model

The runtime advances one run through a single active driver at a time. This RFC preserves today's
strictly serial execution: within a run, the transition lifecycle selects exactly one action and the
attempt lifecycle drives it before the next decision. No intra-run parallelism is introduced.

- **Single writer per run.** At most one driver advances a given run at a time. Cross-driver safety
  does not depend on inferring process liveness; it relies on store optimistic concurrency: every
  lifecycle commit carries the expected next stream sequence as a precondition. A stale or crashed
  driver's appends fail that precondition.
- **Liveness is not a stream fact.** An open attempt (`StateAttemptStarted` with no terminal) is
  indistinguishable in verified history between "currently executing" and "crashed mid-attempt." The
  runtime never guesses. Under single-writer, a driver that loads a run and finds an open attempt it
  did not just start may take recovery ownership, because any competing live writer would lose the
  expected-seq race.
- **Precondition conflict is re-decide, not failure.** A failed stream-position precondition means
  the view is stale. The driver reloads verified history and re-runs the transition decision. The
  resource-lane retry loop is one instance of this general pattern.
- **Open-attempt exclusion is global per run.** If any semantic attempt is open, the only legal
  decisions are `ContinueAttempt` for that attempt or recovery ownership; no unrelated `StartNode`.
  The typestate witness is therefore `NoOpenAttempt` (global). A finer-grained witness (per-node or
  resource-lane scoped) that would allow independent concurrent work is explicitly deferred and must
  not be introduced by silently changing scheduling semantics.
- **Cross-run resource lanes are unchanged.** Lanes remain a store-admission concern across runs; a
  run blocks on a lane held by another run via `ResourceLaneBlocked`. That is the only cross-run
  concurrency and it is not expanded here.

### Lifecycle Entry Points

Three entry points reach the lifecycle protocols. They share the same `TransitionLifecycle` and
attempt machinery; they differ only in how verified authority is first established.

- **Start (genesis).** `RunAdmissionLifecycle` verifies the certified bundle, constructs
  `BoundRuntimeContext`, prepares the atomic genesis commit (`RunStarted` plus the bundled genesis
  events), appends it, and returns `RunAdmissionAuthority`, `CertifiedRuntimeSpec`,
  `BoundRuntimeContext`, and the first `VerifiedRunHistory` at the genesis head. To preserve today's
  prepare/append separation, admission may expose verify+bind+prepare and the genesis append as two
  steps; the post-append step is what yields the first verified history.
- **Resume / recover.** There is no genesis. `VerifiedRunContextLoader` loads the stream and builds
  `VerifiedRunHistory` (verifying spec/certificate/artifacts); `BoundRuntimeContextLoader` builds
  `BoundRuntimeContext` from the registries (a missing binding here is the same redacted
  deployment/configuration diagnostic as at admission). A recovery sweep then classifies any open
  attempt (see "Attempt Recovery Lifecycle") before the first transition decision, so the frontier's
  "open attempt exists" branch is well defined.
- **Replay.** Read-only and out of this RFC's execution scope. Replay consumes the same events and
  must accept the new event schema and ordering (see the replay surface notes in the migration plan).
  It constructs live capabilities or signers under no circumstances.

Only `RunAdmissionLifecycle` and the resume context loaders hold store/registry access; the frontier
decision never does.

### Run Admission Lifecycle

The run admission lifecycle exists before the FSM runtime is allowed to make transition decisions.
Its job is to turn a verified certified bundle into a bound, append-only run root.

Flow:

```text
certified spec + certificate bundle (produced upstream by mfm-certify)
  -> admission verification
  -> executable binding validation
  -> atomic genesis commit
  -> verified run authority for the FSM
```

Input:

- certified spec and certificate bundle produced upstream by `mfm-certify`
- persisted spec, certificate, config, and seed artifact bytes
- configured runner, capability, and framework registries
- staged genesis artifacts needed to establish the run root

Admission validation verifies:

- spec hash and certificate hash
- registry digest used by certification
- descriptor identities and descriptor digests
- lowering and canonicalizer identity
- saga policy digest
- public-output schema authority
- config and seed artifact content hashes
- typed decodability and validation of config and seed artifacts
- launch artifact evidence and artifact roles
- no-secret constraints for admitted launch material

Executable binding validation verifies:

- every certified executable state has a runner binding
- every certified capability reference can be bound to an allowed capability implementation
- framework handlers required by the certified lifecycle are available
- binding identities match the certified spec and deployment policy
- bindings are runtime assembly facts, not semantic stream facts

Output:

- `RunAdmissionAuthority`
- `CertifiedRuntimeSpec`
- `BoundRuntimeContext`
- atomic run genesis authority, including `RunStarted`
- first `VerifiedRunHistory` view at the genesis head

Rules:

- Run admission is not a state attempt.
- Run admission does not execute domain state logic.
- Run admission does not lower, expand, or certify. `mfm-certify` produces `CertifiedTypedSpec` and
  the certificate upstream; admission only verifies that bundle against the production registry,
  preserving the certify -> runtime crate boundary.
- Run admission must verify spec/certificate material before any semantic run lifecycle exists.
- Run admission must prove runner, capability, and framework bindings are available before the FSM
  can construct executable attempts.
- Bootstrap/genesis behavior belongs here, not as a scheduler exception inside the FSM.
- If admission fails, the failure is an ingress, corruption, or deployment diagnostic. It is not a
  semantic run event.
- `RunStarted` means the run authority was admitted. It does not mean a framework state executed.

This is the long-term direction for the historical `Bootstrap` special case, but it must be scoped
carefully. Bootstrap atomicity is useful because it seeds run state, completion state, artifacts, and
retention references in one commit. Today `BootstrapRun` is modeled as a certified graph node, yet it
does not execute as an ordinary framework runner: its runner stub errors if invoked, and the genesis
commit is synthesized by commit-planner middleware (`prepare_run_launch`). So genesis is already
"pre-FSM" in practice while still being represented as a certified node.

Because of that, this RFC consolidates the existing genesis path behind `RunAdmissionLifecycle`
without changing the certified topology. Whether `BootstrapRun` is later demoted or removed from the
certified graph is a separate certification/spec migration decision, scoped below.

### Bootstrap Compatibility And Certification Scope

`BootstrapRun` removal is a certification change, not a runtime change, and this RFC does not perform
it. The boundary is explicit:

- **Phase 1 keeps `BootstrapRun` certified.** `RunAdmissionLifecycle` first wraps and consolidates
  the existing genesis path. The certified spec, lowering, spec hash, certificate, and `RunStarted`
  shape are unchanged; admission is an internal reorganization of who mints genesis authority.
- **`RunStarted` semantics are clarified, not redefined on the wire.** "Admitted run authority, not
  framework-state execution" describes intent. The persisted `RunStarted` payload, its bundled
  genesis attempt events, and its retention refs are preserved in Phase 1.
- **Demoting or removing `BootstrapRun` from certified topology is deferred** to a separate
  certification/spec migration. It must not be done implicitly while extracting admission.
- **If `BootstrapRun` is later removed**, that migration must plan for: spec-hash change, certificate
  re-issuance, replay verification of historical runs whose `RunStarted` pins the old spec hash,
  projection rebuild compatibility, validators that currently require a certified `BootstrapRun` node,
  and resume compatibility for in-flight persisted runs. Until that plan exists, `BootstrapRun` stays
  in the certified graph.

This keeps the RFC honest: genesis atomicity moves behind a named admission authority now, but the
certified topology does not silently change.

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
That makes a missing runner/capability/framework *binding* unrepresentable after run admission: a
missing binding is a pre-authority deployment/configuration failure surfaced during admission or
resume, unless the bound context was constructed incorrectly, in which case the implementation has
violated its own invariant.

`BoundRuntimeContext` proves only that registry/identity bindings *exist and match the certified
spec*. It does not and cannot prove that live transport, signer, or capability *execution* will
succeed. A bound capability can still fail at attempt time (endpoint down, signer unavailable, RPC
error). Those are live-execution failures inside a started attempt — they become `StateAttemptFailed`
or are owned by side-effect recovery, never pre-authority binding failures. The type proves "a
binding was selected," not "the call will work."

### Transition Lifecycle

Deciding and dispatching are two roles. `FrontierScheduler::decide` is the pure function that maps
certified spec, bound runtime context, verified history, and pure hints to one `TransitionDecision`.
`TransitionLifecycle::dispatch` consumes that decision and routes it to exactly one lifecycle action
(attempt, recovery, side-effect, or framework). This section defines the decision; dispatch targets
are in the Responsibility Split.

Input:

- `CertifiedRuntimeSpec`
- `VerifiedRunHistory` or a shared verified run view
- `BoundRuntimeContext`
- currently blocked resource-lane hints, if any

Output:

```rust
enum TransitionDecision {
    StartNode { node_id: NodeId },
    ContinueAttempt { attempt_id: AttemptId },
    StartRemediation { node_id: NodeId },
    AwaitManualResolution,
    ResolveSagaTerminal,
    Blocked,
}
```

`StartNode` covers ordinary domain attempts *and* post-admission framework attempts
(`PublicOutputRender`, `ProjectRetentionManifest`, `CompleteRun`). The framework specialization is
derived from the selected node's `FrameworkNodeSpec`, not from a parallel decision variant, so the
certified node stays the single source of truth for "what this node is." This avoids an earlier shape
where `ProjectPublicOutput`/`ProjectRetentionManifest`/`CompleteRun` duplicated node identity in the
decision enum.

`StartRemediation` and `ResolveSagaTerminal` are kept explicit because they are selected by the saga
projection (obligation classification and quiescence), not by ordinary input-readiness/topology like
`StartNode`. They still execute as normal node attempts (`StateAttemptStarted` -> terminal); in fact
`ResolveSagaTerminal` and the forward-success `CompleteRun` emit the same `RunCompleted` batch and
differ only in the committed outcome. The explicit variants document the *selection authority*, not a
different execution path: `CompleteRun` is reached via `StartNode` because forward readiness selects
it, whereas saga-engaged terminals and remediation are selected by the saga pipeline. If saga
selection later folds into ordinary selection, these variants can be removed.

Rules:

- The decision is pure.
- The decision does not write the store.
- The decision does not stage artifacts.
- The decision does not invoke runners, handlers, capabilities, transports, or signers.
- Saga retry, remediation, manual-resolution, and saga-terminal routing belong here as transition
  decisions. Public-output, retention, and completion are ordinary `StartNode` selections specialized
  by `FrameworkNodeSpec`, not separate decision variants.
- Saga routing is derived through the saga projection pipeline, not by mutating run mode directly.
- Terminal saga decisions must not imply a stronger AC/DC claim than the verified evidence proves.
  Concrete variants may split further during implementation, for example compensated completion,
  failed-without-ACDC-claim, and manually resolved terminal states.
- Cross-run resource-lane admission is not decided here. The pure decision may *predict* a held lane
  from the current projection and skip a blocked node, but lane ownership can change between decision
  and commit and spans runs (`StoreError::ResourceLaneBlocked` is the authority). The lifecycle must
  preserve: decide -> try commit -> handle `ResourceLaneBlocked` -> re-decide with that node marked
  blocked. A `Blocked` decision is therefore provisional, not a proof that no node can ever run.

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
`BoundRuntimeContext`. Preflight checks input *readiness* (the required cells exist), not input
*materialization* (loading and decoding bytes). Expensive or failure-prone work that belongs to the
attempt, materialization included, happens after `StateAttemptStarted`, so failures can be captured
as terminal evidence.

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

An attempt is in one of these dispositions:

- **Open**: `StateAttemptStarted` is committed and no terminal outcome exists yet. The run may not
  advance past it except through its own lifecycle, recovery authority, or an independence witness.
- **Completed**: `StateAttemptCompleted` with matching terminal cell evidence.
- **Failed**: `StateAttemptFailed` with a semantic/runtime-evaluable failure and an explicit
  `retryable` flag.
- **Interrupted**: `StateAttemptInterrupted`, meaning the attempt started but the runtime could not
  observe or complete the lifecycle cleanly.

`Open` and `Interrupted` are distinct. Open is "not yet resolved"; Interrupted is a terminal
disposition that records "the runtime gave up observing this attempt cleanly." Recovery decides which
applies; a dying attempt cannot author its own interruption — `StateAttemptInterrupted` is always
written by `AttemptRecoveryLifecycle` after observing an open attempt in verified history.

`StateAttemptInterrupted` is a distinct event, not an alias for `StateAttemptFailed`. Interruption
terminalizes attempt bookkeeping for the attempts where it is legal, but it does not by itself:

- engage saga remediation
- prove a compensated outcome
- prove an AC/DC terminal claim
- close side-effect uncertainty after `InvocationStarted`
- authorize manual saga resolution

#### Interruption Legality Matrix

Interruption is not legal for every open attempt. The boundary is the side-effect uncertainty
boundary:

| Open attempt kind | `StateAttemptInterrupted` legal as standalone closure? |
|---|---|
| Pure or read attempt | Yes |
| Side-effect attempt before `InvocationStarted` | Yes (no external mutation could have happened) |
| Side-effect attempt at or after `InvocationStarted` | No. The attempt stays **Open** and is owned by `SideEffectLifecycle` recovery until it proves not-submitted, recovers submission/receipt/confirmation, or records ambiguity paired with a non-retryable `StateAttemptFailed` |

So "interruption terminalizes bookkeeping" applies only to the legal rows. A post-`InvocationStarted`
side-effect attempt is never closed by interruption alone; it remains open until side-effect recovery
reaches an evidence-backed outcome. This removes the earlier ambiguity between "interruption is always
terminal" and "interruption cannot close a post-boundary attempt."

#### Failure-Safe Terminalization

Failure terminalization must use a failure-safe path. If a runner returns invalid output, runtime
must not depend on that invalid output to terminalize the attempt. It should produce a redacted
diagnostic artifact and a `StateAttemptFailed` event from minimal trusted attempt authority:

- run id
- spec hash
- node id
- attempt id
- attempt number
- `retryable` (derived from certified policy plus runtime failure classification, never from runner
  output)
- redacted error class
- optional redacted diagnostic artifact evidence

`retryable` is a required trusted input, not an afterthought: `retryable: false` is exactly what
engages saga, and the store requires it to match any paired side-effect terminal retryability in the
same commit. A failure-safe path that cannot set `retryable` deterministically would either fail to
engage saga when policy requires it or engage it spuriously. The failure classification (for example:
runner contract violation and invalid output are non-retryable by classification; transient
runtime/storage outages are not even semantic failures and go to recovery) determines `retryable`
from trusted inputs, independent of whatever the runner returned.

Terminal evidence must not contain secrets, raw signed transactions, private keys, mnemonics,
passwords, authorization headers, local endpoint details, or bearer mutation material.

#### Public Status For Interruption

Interruption is an attempt-level disposition, not a run-level mode:

- It does not add a variant to the public `RunMode` enum (`forward`, `remediating`, `manual_blocked`,
  `completed`, `compensated`, `manually_resolved`, `failed_without_acdc_claim`).
- A run with an interrupted-but-retryable attempt remains in its current `RunMode` (typically
  `forward`); recovery re-attempts it.
- Public status may expose attempt disposition (started / completed / failed / interrupted)
  separately from `RunMode`, but interruption never by itself produces a terminal run outcome.
- Retry/resume policy for interrupted attempts is driven by `AttemptRecoveryLifecycle`, not by saga.
  Saga engagement is reserved for non-retryable `StateAttemptFailed` and forward ambiguity.

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

Recovery is not a separate frontier decision. The transition decision for an open attempt is always
`ContinueAttempt`; the attempt lifecycle consults recovery to choose its disposition from evidence.
Recovery is triggered two ways: a resume-time sweep that classifies open attempts before the first
decision, and continuation of an open attempt this driver owns. Because liveness is not a stream fact
(see "Execution And Concurrency Model"), recovery relies on single-writer ownership, not on detecting
that a process died.

`StateAttemptInterrupted` is how recovery closes an open attempt that will not be cleanly continued —
for example, to reach a quiescent prefix with no open semantic attempts, which manual saga resolution
requires. It is only legal where the interruption legality matrix allows it.

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

#### Open Framework Attempts

Splitting post-admission framework nodes into started-before-run and terminal commits makes them
crash-recoverable mid-attempt — a state the current scheduler forbids (it errors on a framework node
found mid-attempt). Recovery must define disposition for each open framework attempt:

- **`PublicOutputRender`**: re-render from verified history. The terminal commit must still pair the
  output receipt, terminal cell, and `StateAttemptCompleted` in the same commit (the store enforces
  this pairing).
- **`ProjectRetentionManifest`**: rebuild the manifest from current verified evidence; do not reuse a
  manifest computed before interruption.
- **`CompleteRun` / `ResolveSagaTerminal`**: rebuild the terminal proof from the current verified
  prefix before commit. No cached proof authority may carry across the interruption; a proof built
  against a stale prefix cannot authorize terminalization (see the manual-proof cache note under "Saga
  And ACDC Invariants").

In all cases the started marker must make re-execution side-effect-free at the framework level:
re-running a framework attempt after `StateAttemptStarted` must recompute terminal evidence, not
double-apply it.

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

The forward fence (no new forward `InvocationStarted` boundary after saga engagement) is the clearest
example: the store rejects forward boundary events once saga has engaged. The transition lifecycle and
`SideEffectLifecycle` may early-reject a `ContinueAttempt`/`StartNode` that would cross a new forward
boundary, but that is only a fail-fast convenience. Store admission remains the source of truth, and
forward-fence tests must assert the store rejects the append, not only that the runtime declines to
attempt it.

### Saga And ACDC Invariants

Saga and AC/DC guarantees must compose with the new lifecycle split rather than sit beside it as
scheduler special cases.

The transition lifecycle should derive saga routing through an explicit pipeline:

```text
certified spec + verified history + admitted side-effect evidence
  -> saga projection
  -> allowed saga transition
  -> lifecycle dispatch
```

The saga projection derives:

- whether saga handling has engaged
- whether any forward side-effect ledger has crossed `InvocationStarted`
- whether every past-boundary forward ledger is quiescent
- owed remediation obligations
- manual block reason and unresolved-obligation digest
- whether a terminal saga proof can be built from the current prefix

Rules:

- Saga decisions are derived from certified spec, verified run history, and admitted side-effect
  evidence.
- `StateAttemptFailed` can engage saga only when it records a real semantic non-retryable failure
  under certified policy.
- `StateAttemptInterrupted` does not engage saga remediation and does not prove any terminal AC/DC
  claim.
- Forward side-effect ambiguity engages saga only through the paired ambiguity plus non-retryable
  attempt failure rule.
- Operational manual recovery is not saga manual resolution. It may unblock an interrupted runtime
  lifecycle, but it must not emit `ManualResolutionRecorded` or mark a saga manually resolved.
- Manual saga resolution requires signed, prefix-bound `ManualResolutionProofAuthority` over a
  quiescent manually blocked prefix with no open semantic attempts.
- `ResolveSagaTerminal` must rebuild its `SagaTerminalProof` from the current `VerifiedRunHistory`
  immediately before commit. A cached proof cannot authorize terminalization after the run prefix
  changes. Implementation note: this requires removing the scheduler-level in-memory
  `manual_terminal_proofs` cache, which today retains a `VerifiedManualResolutionForPrefix` keyed only
  by run id. Runtime should rebuild and re-verify manual-resolution proof authority from the current
  verified prefix at terminalization. (The store already re-derives the terminal completion outcome at
  admission, so the narrow remaining freshness risk is exactly this cached manual proof.)
- Transition decisions should use `StartRemediation` for executable recovery work. `Compensated` is
  a terminal outcome proven by admitted evidence, not a generic compensation lifecycle that can be
  started by name.
- After saga engagement, `ContinueAttempt` must not cross a new forward `InvocationStarted`
  boundary. It may only advance already-past-boundary forward ledgers toward quiescence or record
  safe pre-boundary failure/not-submitted evidence.
- Persisted diagnostics, including diagnostic artifacts, are non-authority. They affect replay,
  status, or terminal interpretation only when referenced by admitted semantic events in the same
  valid commit.

The allowed transition shape is:

```text
forward runnable and saga not engaged
  -> StartNode
open attempt exists
  -> ContinueAttempt (the attempt lifecycle consults recovery for disposition)
saga engaged and forward ledgers are not quiescent
  -> ContinueAttempt only for already-past-boundary ledgers, or side-effect recovery
owed remediation remains
  -> StartRemediation
manual policy requires operator decision
  -> AwaitManualResolution
terminal proof can be derived from the current prefix
  -> ResolveSagaTerminal
```

This keeps `RunMode` a projection and keeps terminal saga outcomes proof-backed. The scheduler may
route to remediation or manual resolution, but it does not append a saga-control event merely to
declare a mode change.

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
  attempt authority — including a `retryable` flag derived from certified policy and failure
  classification — not on runner-provided output.
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

Bootstrap/genesis is different. Its authority moves to `RunAdmissionLifecycle` and remains an atomic
run-root commit; `BootstrapRun` stays in the certified graph per "Bootstrap Compatibility And
Certification Scope" unless a later certification migration removes it. It should not be used as
evidence that the FSM scheduler needs special-case attempt ordering.

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

### Design Contract Updates

`docs/design.md`, `docs/saga.md`, and `docs/architecture.md` are the authoritative contract, and
`docs/design.md` mandates updating it whenever event schemas, store/projection authority, resume or
replay semantics, or certified saga authority change. This RFC changes several of those, so doc
updates are first-class migration work, landed in the same phase as the change they describe — not
deferred to the end:

- **Phase 1** updates the runtime/admission description and the meaning of `RunStarted` (admitted run
  authority), while noting `BootstrapRun` remains certified.
- **Phase 2** updates the documented frontier decision set (it is no longer exactly "run / block /
  complete").
- **Phase 4** updates typed event schemas and public status for `StateAttemptInterrupted`, including
  the attempt-disposition vs `RunMode` distinction.
- **Phase 6** updates the framework-node lifecycle description (started-before-run and terminal
  commits).
- **Phase 8** updates the side-effect/saga authority description, including the forward-fence
  authority note and the manual-proof freshness rule.

A phase that changes authority or event semantics without the matching doc update is incomplete.

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
- Keep `BootstrapRun` in the certified graph. Phase 1 consolidates the genesis path only; it does not
  change certified topology, spec hash, certificate, or the persisted `RunStarted` shape.
- Preserve atomic run-root behavior while moving it out of scheduler/framework attempt semantics.
- Verify spec hash, certificate hash, registry digest, descriptor identities/digests, saga policy
  digest, public-output schema authority, and config/seed artifact evidence during admission.
- Clarify that `RunStarted` *means* admitted run authority (not framework-state execution). In Phase 1
  this is an intent/documentation clarification; the persisted payload and bundled genesis events are
  unchanged.
- Add `BoundRuntimeContext` construction (registry/identity binding existence only) before transition
  or attempt execution.
- Make missing runner/capability/framework binding fail as a redacted deployment/admission
  diagnostic, not as a semantic attempt event.
- Keep existing runtime behavior behind the old scheduler facade while the authority boundary is
  introduced.
- Update `docs/design.md` runtime/admission sections in the same phase.

### Phase 2: Extract Pure Transition Decision

- Rename or wrap the existing frontier decision as `FrontierScheduler`.
- Ensure it accepts only certified spec, bound runtime context, verified history, and pure hints.
- Make remediation, manual-resolution, saga-terminal, and blocked outcomes explicit
  `TransitionDecision` variants. Public-output, retention, and completion remain `StartNode`
  selections specialized by `FrameworkNodeSpec` (not separate variants).
- Introduce a saga projection step that derives engagement, quiescence, obligations, manual block,
  and terminal proof availability from certified spec plus verified history.
- Implement global per-run open-attempt exclusion (`NoOpenAttempt`) per the Execution And Concurrency
  Model; finer-grained (per-node or resource-lane) witnesses are deferred. Do not silently change
  current scheduling semantics while extracting the lifecycle.
- Keep existing behavior behind the old scheduler facade.

### Phase 3: Introduce Attempt Lifecycle Types

- Add `AttemptLifecycle`, `AttemptStart`, `AttemptInvocation`, `AttemptTerminalPlan`, and
  `AttemptTerminalCommit` types.
- Route ordinary state attempts through the lifecycle with no intended event-shape change yet.
- Keep store writes in `CommitPlanner` and store admission APIs.

### Phase 4: Migrate Event Schema For Interruption

- Add `StateAttemptInterrupted` as a durable event, not a structured failure alias.
- Add the `Interrupted` attempt disposition to the store projection, distinct from `Open` and
  `Failed`, and enforce the interruption legality matrix (no standalone interruption at or after
  `InvocationStarted`).
- Update event structs, stream store admission, projections, replay, app status, Postgres storage,
  and tests in one compatibility-aware slice.
- In Postgres storage, add the new event to the codec and the `AttemptStatus` projection variant;
  projection tables are rebuildable indexes (no destructive migration), but rebuild must handle
  streams with and without the new event.
- Implement the public-status, retryability, and resource-lane behavior for interrupted attempts as
  defined in "Terminal Attempt Outcomes": attempt-level disposition, no new public `RunMode`,
  retry/resume driven by recovery.
- Assert that interruption does not engage saga remediation and does not prove an AC/DC terminal
  outcome.
- Update `docs/design.md` and `docs/saga.md` event-schema and status sections in the same slice.

### Phase 5: Terminalize Observed Failures

- Add the failure-safe terminalization path.
- Derive `retryable` on the failure-safe path from certified policy plus failure classification,
  never from runner output, and ensure it matches any paired side-effect terminal retryability.
- Convert handler errors, materialization errors, and output validation errors inside a valid
  started attempt into redacted terminal attempt evidence.
- Add recovery behavior for failures that happen before terminal commit succeeds.
- Keep authority/corruption/deployment failures outside the typed run stream.

### Phase 6: Classify Framework Lifecycles

- Keep Bootstrap/genesis in `RunAdmissionLifecycle` with `BootstrapRun` still certified.
- Route public-output, retention, completion, and saga terminal framework states through the same
  post-admission attempt lifecycle where doing so preserves existing validators.
- Split current same-commit post-admission framework attempts into started-before-run and terminal
  commits only with coordinated replay/projection/storage tests.
- Define open-framework-attempt recovery per "Open Framework Attempts" (re-render, rebuild manifest,
  rebuild terminal proof; preserve the store's same-commit pairings).
- Treat `mfm-replay` as a first-class surface: its verified-history reconstruction and golden
  fixtures change with the new event order, and pre-migration streams must still replay.
- Update replay, status, and public-output tests for the new event order.
- Update the `docs/design.md` framework-lifecycle description for the new event order.

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
- Remove the scheduler-level `manual_terminal_proofs` cache; rebuild and re-verify manual-resolution
  proof authority from the current verified prefix at terminalization.
- Add forward-fence tests so a forward `InvocationStarted` boundary after saga engagement is rejected
  by store admission (runtime may early-reject, but the test asserts the store is the authority).
- Add recovery tests for every uncertainty phase.
- Update the `docs/design.md` / `docs/saga.md` side-effect and saga authority sections in the same
  phase.
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
- interruption is rejected as a standalone closure for an attempt at or after `InvocationStarted`
- failure-safe `StateAttemptFailed` sets `retryable` from policy/classification and matches paired
  side-effect terminal retryability
- operational manual recovery cannot emit saga manual-resolution events
- stale `SagaTerminalProof` is rejected after the verified prefix changes
- `ContinueAttempt` after saga engagement cannot cross a new `InvocationStarted` boundary
- post-admission framework lifecycle nodes follow the same start/run/terminal model
- Bootstrap/genesis remains covered by run admission atomicity tests
- historical pre-migration streams (no `StateAttemptInterrupted`, old framework event order) replay
  and project unchanged
- storage failure before terminal commit leaves recoverable open attempt state
- invalid runner output becomes redacted failure evidence, not a panic or silent block

## First-Cut Decisions

The following decisions define the first implementation direction:

- Interruption should be a new `StateAttemptInterrupted` event, not a structured
  `StateAttemptFailed` reason. `StateAttemptFailed` means the attempt reached a
  semantic/runtime-evaluable failure. `StateAttemptInterrupted` means the attempt started but the
  runtime could not observe or complete the lifecycle cleanly.
- Bootstrap/genesis authority belongs to `RunAdmissionLifecycle`, not the FSM attempt lifecycle. But
  `BootstrapRun` stays in the certified graph in this RFC; demoting or removing it is a separate
  certification/spec migration (see "Bootstrap Compatibility And Certification Scope").
- Missing runner or capability binding should be unrepresentable after `BoundRuntimeContext`
  construction. If binding cannot be proven, run admission or resume fails with a redacted
  deployment/configuration diagnostic before semantic attempt start. This covers binding *existence*
  only; live transport/signer/capability execution can still fail inside a started attempt and is
  terminalized there or owned by side-effect recovery.
- Post-admission framework lifecycle states should move toward the same start/run/terminal model as
  domain states, but only with explicit replay, projection, store, and validator migration tests.
- Operational manual recovery is separate from saga manual resolution. Saga manual resolution
  remains a signed, prefix-bound protocol over a quiescent manually blocked prefix.
- Pre-authority failures are not semantic runtime states. If certified runtime authority or
  verified run history cannot be constructed, no transition, attempt, recovery, or semantic append
  API is reachable. These failures may be reported as redacted ingress/corruption diagnostics, but
  they do not enter the typed run stream.
- Sync/async service collapse is not a goal. Lifecycle authority logic (frontier decision, attempt
  planning, invocation build, output validation, recovery classification, commit planning) is IO-free
  and shared; only the thin driver seam that loads the stream, awaits runners, stages artifacts, and
  appends commits remains split sync/async. A later async-primary cleanup can drop the sync driver
  without touching lifecycle semantics.
- Execution stays strictly serial per run with global open-attempt exclusion (`NoOpenAttempt`).
  Single-writer-per-run plus expected-seq commit preconditions provide cross-driver safety; liveness
  is never inferred from the stream. Finer-grained independence witnesses and intra-run concurrency
  are deferred.

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
