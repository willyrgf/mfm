# PROBLEM: AC/DC workflow guarantees for MFM

Status: implementation-aware problem statement for the next MFM architecture discussion.

Source context: Stonebraker, Zhou, Kraft, and Li, "Consistency and Correctness in
Data-Oriented Workflow Systems" (CIDR 2026 draft, local PDF
`/Users/willyrgf/Downloads/p9-stonebraker.pdf`).

Review basis:

- `AGENTS.md`
- `docs/code-quality.md`
- `docs/design.md`
- `docs/architecture.md`
- current typed-core implementation in kernel/runtime/store/replay/app/storage crates
- extracted paper text from `/private/tmp/p9-stonebraker.txt`

## One Sentence

MFM has a strong typed durable-execution foundation, including append-only run streams and durable
forward side-effect ledgers, but it cannot claim AC/DC workflow semantics until failure directives,
compensation/backout, manual resolution, and concurrency correctness become certified typed
execution semantics rather than ordinary user-authored states or app glue.

## Non-Claim

AC/DC is not implemented today.

Current MFM can preserve and resume forward typed execution evidence. It does not yet provide a
platform-owned guarantee that a failed update-oriented workflow either completes on a certified
alternate path, is correctly backed out or compensated, or is durably escalated to a typed manual
resolution state.

## Paper Terms In MFM Vocabulary

The paper uses AC/DC to extend ACID-style expectations from one database transaction to a workflow.
For MFM, the terms should be interpreted through typed-core authority:

| Paper term | MFM typed-core interpretation |
| --- | --- |
| Workflow step | A certified typed node attempt selected by runtime from `CertifiedRuntimeSpec` and verified run history. |
| Durable computing | Append-only typed run streams plus artifact evidence that let runtime/replay rebuild authority and avoid repeating completed work. |
| Operation log | `run:{run_id}` kernel event stream plus retained artifact evidence and rebuildable projections. |
| Step transaction | A node commit admitted through `PreparedTypedCommit`; for external side effects, the external mutation is not inside the store transaction. |
| Directive | Missing certified spec semantics for retry, alternate step, backout/compensation, manual resolution, or terminal failure. |
| Backout | Missing platform-owned remediation mode that appends evidence without erasing forward history. |
| Compensation | Missing typed side-effect-like remedial obligation linked to a completed forward side effect. |
| Manual resolution | Missing durable typed run status and evidence for operator-owned completion or backout. |
| Atomic workflow | A run that reaches successful output, certified alternate output, compensated/backed-out terminal evidence, or manual-resolution terminal evidence. |
| Consistent workflow | A workflow whose declared remediation restores required invariants in single-user mode or explicitly carries a domain assertion/manual path. |
| Correct workflow under concurrency | A workflow whose remediation outcome is equivalent to removing the failed workflow and preserving other completed concurrent workflows that should remain. |

The paper's durable execution maps closely to MFM's implemented forward path. The paper's
atomicity, consistency, and concurrency correctness do not yet map to implemented MFM authority.

## Current Implementation Facts

These are facts from inspected code and docs, not design goals.

- `docs/design.md` makes certified typed specs the only runtime contract, run streams
  append-only and authoritative, store commits atomic at the event-batch level, and replay/resume
  driven by stored certified spec plus authoritative run stream.
- `docs/architecture.md` assigns planning to ops, domain semantics to states, capability binding
  to adapters, IO to transports, append authority/projections to store, scheduling to runtime,
  assembly to app, and transport-only surfaces to binaries.
- `crates/kernel/program/src/lib.rs` has `RunnerKind::ApplySideEffect` and `SideEffectState`.
  `SideEffectState` declares forward intent, idempotency input, submission, receipt,
  confirmation, submit, and `output_from_confirmation`.
- The side-effect descriptor digest generated for `ApplySideEffect` covers forward schema and
  semantic type identities for intent, idempotency input, submission, receipt, and confirmation.
  It does not include a compensation contract, failure directive, correctness class, finality
  policy, or irreversible-effect classification.
- `crates/kernel/spec/src/lib.rs` represents node side-effect metadata as
  `SideEffectContractSpec { contract_digest }`. It has no certified fields for retry,
  alternate paths, compensation, backout, manual resolution, physical backout, or correctness
  assertions.
- `crates/kernel/certify/src/lib.rs` requires `ApplySideEffect` descriptors and nodes to carry a
  matching side-effect contract digest. That is a forward side-effect contract check, not a
  backout-strategy check.
- `crates/kernel/events/src/lib.rs` has forward side-effect events for intent, claim/takeover,
  invocation prepared/started, not-submitted proof, submission observed/unknown, receipt,
  confirmation, ambiguity, and failure.
- `SideEffectFailed` has only two legal failure phases:
  `BeforeInvocationStarted` and `AfterNotSubmittedProven`. The current event vocabulary does not
  model "submitted but later compensated", "confirmed but later compensated", or "manual
  remediation completed".
- `RunCompletionOutcome` has `Completed`, `Failed`, and `Cancelled` variants, but
  `crates/kernel/runtime/src/history.rs` currently accepts historical `RunCompleted` only when the
  outcome matches sealed successful `CompleteRun` evidence. Tests reject forged failed/cancelled
  completion.
- `crates/kernel/store/src/lib.rs` owns commit preconditions, logical keys, run/cell/attempt/fact/
  public-output/retention projections, and a forward `SideEffectProjection`.
- Store run state is only `Absent`, `Started`, or `Completed`. There is no durable run projection
  for unresolved failure, compensating, compensated, backed out, manual resolution, or irreversible
  blocked.
- Store side-effect phases are forward phases:
  intent persisted, claimed, invocation prepared, invocation started, submission observed,
  not-submitted proven, submission unknown, receipt observed, confirmation observed, ambiguous,
  or failed.
- Runtime historical validation requires side-effect terminal cell output to be preceded by
  confirmation evidence. A side-effect node cannot produce output from merely submitted or receipt
  evidence.
- Runtime pairs `SideEffectFailed` with `StateAttemptFailed` for side-effect attempts. A
  non-retryable failed attempt removes that node from the runnable frontier; it does not enter a
  compensation frontier.
- Runtime frontier scheduling blocks when any side-effect projection is ambiguous, including
  independent ready nodes. This is a conservative forward-safety rule.
- Runtime's sealed framework lifecycle nodes are `BootstrapRun`, bridge, `PublicOutputRender`,
  `ProjectRetentionManifest`, and successful `CompleteRun`. There is no remediation lifecycle node.
- `crates/kernel/replay` is evidence-only. `ReplayBroker` indexes recorded facts plus forward
  side-effect intent/submission/receipt/confirmation evidence, and `SideEffectReplayVerifier`
  verifies those phases without live IO. There is no compensation verifier contract.
- `crates/app/src/lib.rs` verifies stored certified spec/certificate artifacts before resume,
  status, public-output rendering, and replay. Public app run phase is still only
  absent/started/completed; blocked is a scheduler status string, not durable AC/DC state.
- `crates/storages/stream-store-postgres/src/typed.rs` persists the same derived projections,
  including the forward side-effect projection JSON and absent/started/completed run state. It adds
  durable storage, not new AC/DC semantics.
- Existing tests cover forward side-effect phase durability, not-submitted resume, ambiguity
  blocking, output-before-confirmation rejection, side-effect failure pairing, failed/cancelled
  `RunCompleted` rejection, replay evidence validation, and app replay/resume authority checks.
  They do not cover compensation, reverse-order remediation, manual resolution, irreversible
  boundaries, or phantom-prone compensation.

## What MFM Already Guarantees

MFM already has reusable AC/DC prerequisites:

- certified typed specs are the only semantic runtime contract;
- append-only run streams are the authority for execution history;
- store commits are atomic at the MFM event-batch level;
- projections are rebuildable and not independent semantic authority;
- side-effect execution has typed forward intent, typed idempotency input, durable claims and
  fencing, invocation epochs, uncertainty boundaries, evidence artifacts, replay verifier ids, and
  ambiguity blocking;
- resume advances only from verified stream/projection state;
- replay cannot construct live transports or capability handles;
- app and binaries are not supposed to own workflow semantics.

These are durable execution foundations. They are not workflow atomicity, consistency, or
concurrency correctness for failed update-oriented workflows.

## What MFM Does Not Yet Guarantee

MFM currently lacks:

- certified failure directives for retry, alternate step, compensation/backout, manual resolution,
  and terminal failure conditions;
- a platform-owned decision point that says a later node failure requires compensation of already
  confirmed side effects;
- compensation contracts linked to forward side-effect contracts;
- typed compensation intent, idempotency input, capability contract, receipt, confirmation,
  ambiguity, and failure evidence;
- reverse dependency-order compensation scheduling from certified graph plus completed ledgers;
- durable run statuses for unresolved failure, compensating, compensated, backed out, irreversible
  blocked, or manually resolved;
- replay authority for compensation evidence;
- public inspect/render surfaces that expose unresolved remediation obligations;
- certification rules that reject side-effecting workflows without a declared remediation strategy;
- correctness classes or assertions for phantoms, cascading backout, commutative updates,
  escrow-like updates, external-service ambiguity, or irreversible effects.

A workflow author can model a "compensation" as another ordinary state today, but that does not
give AC/DC. The platform would not know the state is a mandatory inverse obligation, would not
schedule it automatically after a later failure, would not resume it as remediation, and would not
know what correctness claim it is supposed to prove.

## Primary Architecture Problem

Define a certified typed remediation model for side-effecting workflows.

The model must let MFM answer, from certified spec plus append-only events:

1. What should happen when a node fails?
2. Which completed side effects create remediation obligations?
3. Which obligations are compensatable, physically backout-capable, alternate-path-resolvable,
   manual-only, or irreversible?
4. What evidence proves an obligation was attempted exactly once?
5. What evidence proves the obligation restored the declared invariant or reached a declared manual
   terminal state?
6. What happens when remediation itself fails, is ambiguous, or crosses an irreversible boundary?
7. Which concurrency-correctness claims are framework-enforced, and which are domain assertions
   carried by certified evidence?

This is a kernel/runtime/store/spec/replay architecture problem, not an app cleanup problem.

## Modeling Placement Analysis

The current architecture suggests a mixed model, not a plain user-state pattern.

| Candidate | Fit | Problem if used alone |
| --- | --- | --- |
| Extend `SideEffectState` with compensation metadata | Reuses forward side-effect concepts: intent, idempotency, capability binding, evidence, replay verifier. | A trait extension alone cannot create run remediation mode, reverse scheduling, terminal phases, or certification directives. |
| Add a parallel compensation effect class | Makes remedial mutation distinct from forward mutation and may avoid overloading `ApplySideEffect`. | It would duplicate much of the ledger machinery unless designed as a specialization of the same side-effect protocol. |
| Certified continuation runs | Useful when remediation needs a newly certified graph, operator input, or post-failure planning boundary. | A detached run does not by itself make the original run atomically complete; parent/child causality and terminal authority would be required. |
| Framework lifecycle remediation nodes | Matches existing sealed lifecycle pattern for bootstrap, render, retention, and completion. Runtime can own mode transitions and frontier derivation. | Lifecycle nodes still need domain-declared compensation contracts and typed capability evidence; framework code must not perform domain IO. |
| Ordinary user states or app/binary glue | Easy to author locally. | Rejected for AC/DC: it hides obligations from certification, runtime, store, replay, and public status. |

Working conclusion for discussion:

Compensation should be modeled as certified remediation semantics owned by the framework/runtime,
with remedial actions carrying side-effect-grade typed evidence and being linked to forward
side-effect contracts. Whether that is encoded as a `SideEffectState` extension, a sibling
`ApplyCompensation` effect class, framework-owned lifecycle nodes, certified continuation runs, or
a combination is still open. It should not be hidden in ordinary user states.

## Reusable Foundations Vs New Authority

| Area | Reusable foundation | New authority required |
| --- | --- | --- |
| Spec/certification | Certified typed spec, descriptor identities, side-effect contract digest checks, framework lifecycle nodes. | Failure directives, remediation contract specs, correctness classes, irreversible boundaries, manual-resolution metadata. |
| Events | Append-only event model, side-effect ledger payload pattern, redaction-safe errors, artifact evidence refs. | Compensation/backout/manual-resolution event payloads and terminal run outcomes. |
| Store | Atomic prepared commits, logical keys, preconditions, rebuildable projections, forward side-effect phase rules. | Remediation projections, run remediation state, preconditions for compensation once-and-only-once and terminal resolution. |
| Runtime | Verified history, deterministic scheduler, guarded commit planner, conservative ambiguity blocking. | Failure-mode transition, compensation frontier derivation, reverse dependency ordering, alternate-path/manual-resolution scheduling. |
| Replay | Evidence-only broker and forward side-effect verifier contract. | Compensation evidence indexes and verifier contracts; replay-visible correctness assertions. |
| App/storage | Certified bundle verification, start/resume/replay/render assembly, durable Postgres event/projection storage. | Public status/render surfaces for unresolved, compensating, compensated, manually resolved, and irreversible-blocked states. |
| States/adapters/transports | State-owned intent, adapter evidence phases, reusable transports, secret boundaries. | Domain-declared compensation intent and correctness evidence without moving live IO or topology into the wrong layer. |

## Concrete Correctness Problems For MFM

### External Services

MFM's current side-effect protocol handles uncertainty in the forward direction with idempotency
input, submission unknown, not-submitted proof, ambiguity, receipt, and confirmation evidence.
For AC/DC, an external service call also needs a declared remediation path: cancel, refund, release,
reverse transfer, manual resolution, or irreversible boundary.

If the service cannot prove whether the original request happened, the run must not be marked
compensated unless typed evidence or manual authority resolves the ambiguity.

### On-Chain Effects

EVM contract lifecycle states record transaction intents, submissions, receipts, confirmation
values, block numbers, and receipt status. That is not the same as a finality or reorg policy.

For AC/DC, MFM needs to know whether an on-chain effect is:

- not submitted and safe to retry;
- submitted but not confirmed;
- included but not final under a declared finality rule;
- final and compensatable only through another transaction;
- final and irreversible for the workflow's atomicity claim.

Current typed confirmation evidence should not be interpreted as proof that an irreversible
business boundary can be backed out.

### Replay Evidence

Replay can verify that recorded forward evidence is internally consistent and bound to certified
authority. It cannot prove compensation correctness without recorded compensation evidence,
declared correctness classes, and replay verifier contracts for those classes.

Replay must remain evidence-only. A future compensation replay path must not call live services to
decide whether a run was compensated.

### Phantoms And Cascading Backout

The paper's phantom problem is concrete for MFM whenever a side effect describes a predicate, scope,
or external resource set rather than a single commutative operation. Re-running a predicate during
compensation can touch objects that were not touched originally. Restoring an old value can erase
intervening work.

MFM needs typed evidence for the original affected set or a stronger semantic proof, such as:

- touched keys or object ids;
- predicate snapshot evidence;
- prior/post state evidence;
- service operation ids with cancel/refund semantics;
- escrow/commutative-operation assertions;
- isolation or lock proof;
- explicit manual-only remediation.

If the platform cannot prove or verify the declared correctness claim, the run should block for
manual resolution rather than claim compensated success.

### Irreversible Effects

Some effects cannot be undone in the workflow's semantic model: dispensing funds, final on-chain
transactions, sending irreversible notifications, or crossing a business finality boundary.

The certified spec should force those boundaries to shape topology. Options include placing the
irreversible effect after all fallible compensatable work, splitting the business process into
multiple workflows, or requiring a manual-resolution terminal path for failures after the boundary.

MFM must not model irreversible effects as if running an inverse state can make the workflow look
like it never happened.

## Architecture Constraints

Any AC/DC design must preserve the MFM boundary contract:

- operations plan topology and directives, but do not execute remediation;
- states declare forward and remediation semantics, but do not create live IO;
- adapters bind state intent to capabilities and evidence phases;
- transports implement reusable live/replay capability backends;
- store owns append-only event admission, logical keys, preconditions, and projections;
- runtime owns scheduler decisions, failure-mode transitions, compensation frontier derivation,
  and guarded commits;
- replay verifies only certified recorded evidence;
- app assembles registries, stores, artifacts, runners, and capabilities;
- CLI and REST expose start/resume/replay/inspect/render surfaces only.

Append-only history remains authority. AC/DC cannot mean erasing or mutating forward events. It
must mean appending certified remediation evidence and terminal resolution evidence.

## Success Criteria For Claiming Progress

MFM can start claiming AC/DC progress only when these are true:

- certification rejects side-effecting workflows that omit required failure/remediation semantics;
- later failure after confirmed side effects enters durable remediation or manual-resolution mode;
- remediation obligations are typed, idempotent, replay-verifiable, and crash-resumable;
- runtime derives a remediation frontier from the certified graph and forward ledger evidence;
- store projects forward and remediation obligations from append-only events;
- public status distinguishes unresolved failure, ambiguous side effect, compensating,
  compensated, irreversible blocked, manually resolved, and successful completion;
- replay verifies remediation evidence without live capabilities;
- tests cover failure after one confirmed side effect, failure after multiple confirmed side
  effects, reverse-order compensation, compensation crash-resume, ambiguous compensation, manual
  resolution, irreversible-boundary rejection or blocking, and at least one phantom-prone case.

## Open Questions

- Should failure directives live directly in `TypedExecutionSpec` nodes, in framework lifecycle
  metadata, or in a separate certified remediation spec section?
- Should compensation be encoded as an extension of `SideEffectState`, a sibling effect class, or a
  framework-owned remedial side-effect protocol shared by both?
- When is a certified continuation run the right remediation unit, and how does it become terminal
  authority for the parent run?
- What is the minimal compensation event vocabulary that avoids duplicating the whole forward
  side-effect event model while preserving once-and-only-once evidence?
- Which correctness classes can the kernel name generically without depending on domain semantics?
- What assertion format is acceptable when correctness depends on domain facts that the framework
  cannot prove?
- How should MFM model physical backout for narrow database-local cases without making it the
  default for external-service and on-chain workflows?
- How should public API compatibility be handled when adding durable run phases beyond
  absent/started/completed?
- How should on-chain finality depth, reorg evidence, and irreversible business boundaries be
  represented without putting protocol policy in the wrong layer?

## Immediate Discussion Target

The next design discussion should define the smallest certified vertical slice:

1. A failure directive shape in certified typed specs.
2. A remediation contract linked to a forward side-effect contract.
3. Append-only remediation evidence and store projections.
4. Runtime transition from non-retryable failure with completed side effects into remediation or
   manual-resolution mode.
5. Replay verification for remediation evidence.
6. Public status vocabulary for unresolved, compensating, compensated, manually resolved, and
   irreversible-blocked runs.

The goal is not to design every AC/DC feature at once. The immediate problem is to extend MFM's
typed durable execution core from "we can resume the forward workflow safely" to "we can finish,
compensate, or durably escalate the whole workflow with explicit correctness semantics."
