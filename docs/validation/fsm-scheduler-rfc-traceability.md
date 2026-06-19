# FSM scheduler RFC traceability matrix

Source of truth: `RFC_REFACTOR_FSM_SCHEDULER.md`, not any implementation plan.

This file records branch-local evidence for the explicit FSM lifecycle refactor. Status values:
`met`, `partial`, `missing`, `conflict`, or `deferred-by-RFC`.

## Requirement: no old-model compatibility readers

RFC: `RFC_REFACTOR_FSM_SCHEDULER.md:25-56`, `1117-1128`, `1231-1232`, `1238-1241`

Status: met

Expected behavior: runtime, replay, projection, and Postgres-backed loads reject unsupported old
stream orders with a typed diagnostic instead of silently accepting mixed lifecycle streams.

Expected module/API shape: one stream-model guard is reached by store projection rebuild and
therefore by runtime, replay, and Postgres projection rebuild.

Required tests:
- old-model terminal payload without a prior `StateAttemptStarted` commit is rejected
- old same-commit framework terminal order is rejected
- Postgres rebuild does not project old-model rows

Current evidence:
- `ProjectionSnapshot::rebuild_from_run_stream` calls the stream-model guard:
  `crates/kernel/store/src/lib.rs:6110`
- attempt-bound payloads require a prior started attempt:
  `crates/kernel/store/src/lib.rs:6117`, `crates/kernel/store/src/lib.rs:6130`,
  `crates/kernel/store/src/lib.rs:6138`
- replay has an unsupported old stream model test:
  `crates/kernel/replay/src/lib.rs:2586`
- Postgres maps old-model stream projection failures to corruption diagnostics:
  `crates/storages/stream-store-postgres/src/typed.rs:1417`,
  `crates/storages/stream-store-postgres/src/typed.rs:2350`,
  `crates/storages/stream-store-postgres/src/typed.rs:2355`

Blocker: none currently known.

Required fix: none.

Acceptance tests: keep the existing store/replay/Postgres old-stream rejection tests green.

Owner commit: already implemented.

## Requirement: RunAdmitted is the only admission root

RFC: superseded by the flat admission migration.

Status: met

Expected behavior: run admission is a pre-FSM boundary represented by exactly one `RunAdmitted`
event. The certified graph contains only real executable and terminal framework nodes; admission
does not append synthetic attempts, receipt cells, artifact-reference payloads, or retention-ref
payloads.

Expected module/API shape: `RunAdmissionLifecycle` prepares and verifies run launch authority;
the scheduler's first candidate after admission is a real certified node.

Required tests:
- run admission atomicity
- admission appends exactly one root payload
- admission failure does not append semantic events

Current evidence:
- `RunAdmissionLifecycle` wraps launch preparation and admitted authority:
  `crates/kernel/runtime/src/admission.rs:44`
- launch preparation validates staged spec, certificate, config, and seed evidence:
  `crates/kernel/runtime/src/commit.rs:141`
- launch commits exactly one `RunAdmitted` root payload:
  `crates/kernel/runtime/src/commit.rs`
- design documents run admission as a single root event outside certified topology:
  `docs/design.md:390`

Blocker: none currently known.

Required fix: none.

Acceptance tests: keep run launch/admission atomicity tests green.

Owner commit: already implemented.

## Requirement: run admission verifies launch authority before FSM execution

RFC: `RFC_REFACTOR_FSM_SCHEDULER.md:191-207`, `225-259`, `261-274`, `867`

Status: met

Expected behavior: spec/certificate/artifact and executable binding authority are verified before
semantic transition or attempt APIs are reachable. Admission failures are ingress/corruption or
deployment diagnostics, not semantic run events.

Expected module/API shape: admission exposes preparation and post-append verified authority; resume
uses verified context loaders.

Required tests:
- spec/certificate/config/seed evidence mismatch rejects admission
- missing executable binding rejects before attempt start
- admission failure leaves no semantic run events

Current evidence:
- `prepare_run_launch` requires a bound context:
  `crates/kernel/runtime/src/admission.rs:48`
- `admitted_run_authority` rebuilds a verified `RuntimeRunView` after append:
  `crates/kernel/runtime/src/admission.rs:66`
- scheduler start paths reload admitted authority after admission append:
  `crates/kernel/runtime/src/scheduler.rs:108`
- `VerifiedRunContextLoader` combines committed stream authority with bound context:
  `crates/kernel/runtime/src/history.rs:159`

Blocker: none currently known.

Required fix: none.

Acceptance tests: admission-failure and missing-binding regression tests must stay green.

Owner commit: already implemented.

## Requirement: BoundRuntimeContext proves executable bindings only

RFC: `RFC_REFACTOR_FSM_SCHEDULER.md:245-251`, `312-343`, `780-783`, `868`, `1074-1077`, `1248-1252`

Status: met

Expected behavior: missing runner, capability, or framework-handler bindings fail before a semantic
attempt starts. Live transport/signer/capability failures remain attempt-time behavior.

Expected module/API shape: `BoundRuntimeContext` is constructed from certified spec plus registries
and is required by transition/attempt execution.

Required tests:
- missing runner binding fails before attempt start
- capability implementation mismatch fails before attempt start
- framework handler kind mismatch fails before attempt start

Current evidence:
- `BoundRuntimeContext::bind` checks all topological and remediation nodes:
  `crates/kernel/runtime/src/binding.rs:82`
- node authority requires runner binding, capability authority, and framework handler authority:
  `crates/kernel/runtime/src/binding.rs:169`
- framework handler kinds are derived from certified framework node specs:
  `crates/kernel/runtime/src/binding.rs:311`
- scheduler loads bound context before launch and before each drive step:
  `crates/kernel/runtime/src/scheduler.rs:86`, `crates/kernel/runtime/src/scheduler.rs:349`

Blocker: none currently known.

Required fix: none.

Acceptance tests: runtime binding identity and missing-capability tests must stay green.

Owner commit: already implemented.

## Requirement: transition decision is pure and explicit

RFC: `RFC_REFACTOR_FSM_SCHEDULER.md:345-412`, `784-789`, `869-871`, `1084-1090`

Status: met

Expected behavior: the pure transition decision receives certified spec, verified history, and pure
hints only; it returns one closed decision. Public output, retention, and completion are selected as
ordinary `StartNode` framework nodes, not parallel decision variants.

Expected module/API shape: `TransitionLifecycle::decide` is IO-free. Dispatch routes the decision to
the owning lifecycle.

Required tests:
- architecture test forbids store/artifact/runner/capability handles in transition decision code
- transition decision variants do not duplicate framework node identity
- public output, retention, and completion are selected through node identity

Current evidence:
- `TransitionLifecycle::decide` takes spec, run id, verified view, and blocked-node hints:
  `crates/kernel/runtime/src/transition.rs:42`
- explicit variants exist for start, continue, remediation, manual resolution, saga terminal, and
  blocked decisions: `crates/kernel/runtime/src/transition.rs:16`
- completed frontier/public-output projection is mapped to `TransitionDecision::Blocked`:
  `crates/kernel/runtime/src/transition.rs:109`
- public-output-projected remains only a scheduler facade status via a pure helper:
  `crates/kernel/runtime/src/transition.rs:115`,
  `crates/kernel/runtime/src/scheduler.rs:377`
- static runtime guard rejects reintroducing `TransitionDecision::PublicOutputProjected`:
  `crates/kernel/runtime/src/tests.rs:10080`

Blocker: none currently known.

Required fix: none.

Acceptance tests:
- static decision-shape test covering the closed transition enum
- status test proving already-projected public output returns public-output status without adding a
  separate framework selection variant

Owner commit:
- runtime: keep public-output-projected as scheduler status, not transition decision

## Requirement: scheduler facade is thin and recovery dispatch is single-owned

RFC: `RFC_REFACTOR_FSM_SCHEDULER.md:347-350`, `411-412`, `582-589`, `903-904`, `1188-1190`

Status: met

Expected behavior: transition dispatch consumes one decision and routes it to the owning lifecycle.
The scheduler facade should not duplicate recovery policy.

Expected module/API shape: recovery disposition handling lives in transition/attempt/recovery
lifecycle code, with scheduler limited to loading context and invoking a lifecycle entry point.

Required tests:
- architecture test that `scheduler.rs` does not match on `OpenAttemptDisposition`
- behavioral tests proving each recovery disposition is handled once through lifecycle dispatch

Current evidence:
- transition classifies open attempts into `ContinueAttempt`:
  `crates/kernel/runtime/src/transition.rs:48`
- recovery-owned sync dispatch lives in `AttemptRecoveryLifecycle`:
  `crates/kernel/runtime/src/recovery.rs:154`
- recovery-owned async dispatch lives in `AttemptRecoveryLifecycle`:
  `crates/kernel/runtime/src/recovery.rs:174`
- scheduler invokes recovery dispatch without matching `OpenAttemptDisposition`:
  `crates/kernel/runtime/src/scheduler.rs:454`, `crates/kernel/runtime/src/scheduler.rs:482`
- static runtime guard rejects scheduler direct disposition matching:
  `crates/kernel/runtime/src/tests.rs:10080`

Blocker: none currently known.

Required fix: none.

Acceptance tests:
- static namespace/architecture test forbids scheduler recovery-disposition matching
- runtime tests for interrupt, side-effect delegation, operational block, and retry terminalization
  through the shared lifecycle

Owner commit:
- runtime: move recovery dispatch out of scheduler facade

## Requirement: full sync/async driver collapse is deferred

RFC: `RFC_REFACTOR_FSM_SCHEDULER.md:54-56`, `1191-1196`, `1266-1271`

Status: deferred-by-RFC

Expected behavior: this RFC removes duplicated scheduler-owned recovery/transition authority, but it
does not require eliminating every duplicated sync/async driver loop. Existing sync and async
drivers may still duplicate IO choreography for stream loading, runner await, artifact staging, and
append calls. A later async-primary cleanup may collapse those drivers if lifecycle semantics stay
unchanged.

Expected module/API shape: shared IO-free helpers own frontier decisions, transition and recovery
classification, attempt-id/start planning, invocation build, output validation, and commit planning.
Sync/async wrappers remain acceptable where the store/runner/artifact IO shape differs.

Required tests:
- architecture test that scheduler does not own recovery disposition routing
- behavioral parity tests for representative sync/async lifecycle paths

Current evidence:
- recovery dispatch authority is centralized in `AttemptRecoveryLifecycle`:
  `crates/kernel/runtime/src/recovery.rs:154`, `crates/kernel/runtime/src/recovery.rs:174`
- scheduler delegates recovery dispatch without matching recovery dispositions:
  `crates/kernel/runtime/src/scheduler.rs:454`, `crates/kernel/runtime/src/scheduler.rs:482`
- ordinary/framework lifecycle sync and async paths still exist as separate IO drivers:
  `crates/kernel/runtime/src/attempt.rs:158`, `crates/kernel/runtime/src/attempt.rs:363`,
  `crates/kernel/runtime/src/framework_lifecycle.rs:49`,
  `crates/kernel/runtime/src/framework_lifecycle.rs:135`

Blocker: none; full driver collapse is explicitly outside this RFC.

Required fix: none in this RFC.

Acceptance tests:
- scheduler/recovery structural guard
- sync/async lifecycle parity tests remain focused on behavior, not driver deduplication

Owner commit:
- runtime/docs: document sync/async driver collapse as deferred outside this RFC

## Requirement: serial execution with resource-lane-scoped independence witness

RFC: `RFC_REFACTOR_FSM_SCHEDULER.md:157-184`, `612-614`, `819-820`, `1091-1093`, `1162-1164`, `1266-1269`

Status: met

Expected behavior: open-attempt exclusion is the default. The only interleaving is a node parked on a
cross-run resource lane, where independent ready work may advance only with a scoped independence
witness. The witness is conservative before another side-effect node has concrete lane evidence:
same-namespace side-effect work waits until the parked lane releases, while different namespaces can
advance.

Expected module/API shape: `NoOpenAttempt` or equivalent for default scheduling; resource-lane
witness carries lane/resource identity instead of a bare node skip and falls back to certified
resource namespace when a not-yet-run side-effect node has no concrete lane key.

Required tests:
- two ready side-effect nodes targeting the same externally held lane: the second attempt must not
  start after the first is parked
- two ready side-effect nodes in the same resource namespace but with different eventual keys: the
  second attempt must wait until the parked lane releases
- a truly independent node/lane still advances while the first node is parked

Current evidence:
- lane block status carries a `ResourceLaneBlockWitness` plus `advanced`:
  `crates/kernel/runtime/src/attempt.rs:29`
- witness carries node and lane key and consults verified projection state so concrete different-key
  same-namespace attempts can continue:
  `crates/kernel/runtime/src/attempt.rs:45`, `crates/kernel/runtime/src/attempt.rs:54`
- attempt lifecycle constructs witnesses from projected lane conflicts and typed store errors:
  `crates/kernel/runtime/src/attempt.rs:808`, `crates/kernel/runtime/src/attempt.rs:848`
- scheduler carries `BTreeSet<ResourceLaneBlockWitness>` hints:
  `crates/kernel/runtime/src/scheduler.rs:347`
- transition/recovery filter blocked work by scoped witness against verified projections:
  `crates/kernel/runtime/src/transition.rs:129`, `crates/kernel/runtime/src/recovery.rs:108`
- same-namespace side-effect work parks until the lane releases:
  `crates/kernel/runtime/src/tests.rs:6918`
- same-namespace side-effect work with already projected different concrete lane evidence can
  continue:
  `crates/kernel/runtime/src/tests.rs:7083`

Blocker: none currently known.

Required fix: none.

Acceptance tests:
- same-lane blocked side-effect attempt prevents another same-lane start
- same-namespace different-key side-effect work waits until the parked lane releases
- same-namespace different-key side-effect work with concrete projected lane evidence continues
- independent-lane/non-conflicting node still advances

Owner commit:
- runtime: carry resource-lane independence witness through blocked scheduling

## Requirement: started attempt precedes semantic execution and materialization

RFC: `RFC_REFACTOR_FSM_SCHEDULER.md:414-463`, `919-939`, `1131-1143`

Status: met

Expected behavior: every semantic attempt appends `StateAttemptStarted` before materialization,
runner execution, validation, artifact staging, or terminal commit planning. Run admission is not a
semantic attempt.

Expected module/API shape: attempt lifecycle has selected/started/invoked/terminal planned/terminal
committed phases.

Required tests:
- ordinary attempt emits start before runner execution
- materialization failure after start becomes terminal attempt failure
- post-admission framework nodes emit started-before-run ordering

Current evidence:
- ordinary attempt typestate structs model selected, started, invoked, terminal planned, committed:
  `crates/kernel/runtime/src/attempt.rs:53`
- ordinary attempt appends start before invocation construction:
  `crates/kernel/runtime/src/attempt.rs:146`
- framework attempt appends start before terminal output preparation:
  `crates/kernel/runtime/src/framework_lifecycle.rs:72`
- design documents started-before-run for ordinary and framework attempts:
  `docs/design.md:377`

Blocker: none currently known.

Required fix: none.

Acceptance tests: keep runtime lifecycle ordering and failure terminalization tests green.

Owner commit: already implemented.

## Requirement: runner contexts cannot write store and runner output is proposal-only

RFC: `RFC_REFACTOR_FSM_SCHEDULER.md:125-127`, `453-463`, `803-805`, `874-875`, `895-897`

Status: met

Expected behavior: runners receive sealed invocation context only. They return proposal payloads and
staged artifacts; runtime validates and prepares store-owned commits.

Expected module/API shape: invocation builder constructs sealed runner context from bound authority;
commit planner converts validated proposals into prepared commits.

Required tests:
- compile-fail tests prevent runners from returning lifecycle-owned payload batches directly
- invalid proposal payloads are rejected by commit planning

Current evidence:
- `AttemptLifecycle` builds `PreparedRunnerInvocation` before `run_erased`:
  `crates/kernel/runtime/src/attempt.rs:184`
- runner output is passed to `CommitPlanner::prepare_runner_output`:
  `crates/kernel/runtime/src/attempt.rs:239`
- scheduler-owned payloads from runners are rejected:
  `crates/kernel/runtime/src/commit.rs:2080`
- `StateAttemptInterrupted` from runners is rejected as recovery-owned:
  `crates/kernel/runtime/src/commit.rs:1992`

Blocker: none currently known.

Required fix: none.

Acceptance tests: keep runtime trybuild runner-output authority tests green.

Owner commit: already implemented.

## Requirement: interruption is distinct, recovery-owned, and attempt-level only

RFC: `RFC_REFACTOR_FSM_SCHEDULER.md:467-490`, `537-549`, `591-593`, `1108-1129`, `1241-1244`

Status: met

Expected behavior: `StateAttemptInterrupted` is not a failure alias, carries no retryable flag, does
not engage saga, and is exposed as attempt disposition separately from `RunMode`.

Expected module/API shape: closed event enum includes `StateAttemptInterrupted`; store projection has
`AttemptStatus::Interrupted`; public app/CLI/REST responses expose attempt dispositions.

Required tests:
- interruption projects as interrupted, not failed
- interruption does not engage saga remediation
- public status includes interrupted disposition and no run-mode interruption variant

Current evidence:
- event enum and schema include `StateAttemptInterrupted`:
  `crates/kernel/events/src/lib.rs:352`
- payload contains spec hash, node id, and attempt id, with no retryable field:
  `crates/kernel/events/src/lib.rs:788`
- projection maps it to `AttemptStatus::Interrupted`:
  `crates/kernel/store/src/v1/projection.rs:180`
- public app status maps it to disposition `interrupted` and no retryability:
  `crates/app/src/lib.rs:2371`
- CLI and REST docs list attempt disposition separately from run mode:
  `bin/cli/README.md:341`, `bin/rest-api/README.md:254`

Blocker: none currently known.

Required fix: none.

Acceptance tests:
- store interrupted projection test
- app/CLI/REST JSON status tests for interrupted disposition

Owner commit: already implemented.

## Requirement: interruption legality matrix

RFC: `RFC_REFACTOR_FSM_SCHEDULER.md:491-508`, `663-666`, `1112-1114`, `1216-1217`, `1229-1230`

Status: met

Expected behavior: pure/read attempts can be interrupted. Side-effect attempts can be interrupted
before `SideEffectInvocationPrepared`. Side-effect attempts at/after `SideEffectInvocationPrepared`
remain open for side-effect recovery.

Expected module/API shape: store projection admission and runtime recovery both enforce the matrix.

Required tests:
- interrupt side-effect after `IntentPersisted` before `InvocationPrepared`
- interrupt side-effect after `Claimed` before `InvocationPrepared`
- reject interruption after `SideEffectInvocationPrepared`
- reject interruption after open lane/ledger at later phases

Current evidence:
- projection rejects interruption after prepared side-effect authority:
  `crates/kernel/store/src/v1/projection.rs:226`
- runtime standalone interruption allows only no projection, intent, or claimed phases:
  `crates/kernel/runtime/src/side_effect_lifecycle.rs:84`
- recovery classifies intent/claimed side-effect attempts as interruptible:
  `crates/kernel/runtime/src/side_effect_lifecycle.rs:115`
- store tests cover intent/claimed allowed and prepared rejected:
  `crates/kernel/store/tests/commit_contract.rs:2372`,
  `crates/kernel/store/tests/commit_contract.rs:2438`,
  `crates/kernel/store/tests/commit_contract.rs:2504`

Blocker: none currently known.

Required fix: none.

Acceptance tests: keep side-effect interruption legality tests green.

Owner commit: already implemented.

## Requirement: failure-safe terminalization uses trusted failure classification

RFC: `RFC_REFACTOR_FSM_SCHEDULER.md:510-535`, `554-572`, `1133-1148`, `1218-1219`, `1263-1266`

Status: met

Expected behavior: invalid output, materialization, and validation failures inside a valid started
attempt terminalize via redacted trusted evidence. `retryable` is derived from certified policy plus
runtime failure classification, never from runner output. Authority, corruption, unverifiable
history, and deployment failures remain outside the typed run stream and do not become
`StateAttemptFailed`.

Expected module/API shape: failure-safe constructor receives minimal trusted attempt authority plus a
policy/classification retryability decision. Terminalizable runtime validation has a distinct error
surface from corrupt or unverifiable run history.

Required tests:
- invalid runner output terminalizes with redacted diagnostic evidence
- materialization failure terminalizes after start
- runtime validation failure terminalizes after start
- corrupt/unverifiable `InvalidRunStream` after start returns the corruption error without appending
  `StateAttemptFailed`
- failure-safe retryability follows policy/classification
- paired side-effect terminal failure retryability matches attempt failure retryability

Current evidence:
- runtime distinguishes terminalizable validation from invalid stream authority:
  `crates/kernel/runtime/src/error.rs:23`,
  `crates/kernel/runtime/src/attempt.rs:736`
- observed post-start failures terminalize through runtime-owned failure commit:
  `crates/kernel/runtime/src/attempt.rs:585`
- diagnostic artifact uses redacted fields only:
  `crates/kernel/runtime/src/attempt.rs:749`
- `ObservedFailureRetryabilityPolicy` derives retryability from certified runtime/node authority:
  `crates/kernel/runtime/src/attempt.rs:118`
- failure-safe error info uses the derived retryability decision:
  `crates/kernel/runtime/src/attempt.rs:703`
- runtime tests cover policy/classification-derived retryability:
  `crates/kernel/runtime/src/attempt.rs:984`
- runtime integration tests assert emitted failure-safe retryability:
  `crates/kernel/runtime/src/tests.rs:10508`
- runtime integration tests prove terminalizable runtime validation and non-terminal
  `InvalidRunStream` authority failures:
  `crates/kernel/runtime/src/tests.rs:3794`,
  `crates/kernel/runtime/src/tests.rs:3843`

Blocker: none currently known.

Required fix: none.

Acceptance tests:
- policy/classification-derived failure-safe retryability
- post-start runtime validation terminalizes as redacted `StateAttemptFailed`
- post-start `InvalidRunStream` remains a corruption error and appends no terminal attempt evidence
- paired side-effect terminal retryability match

Owner commit:
- runtime: derive failure-safe retryability from policy and failure class
- runtime: split terminalizable validation from stream corruption

## Requirement: side-effect terminal evidence releases resource lanes

RFC: `RFC_REFACTOR_FSM_SCHEDULER.md:502`, `663-666`, `675-680`

Status: met

Expected behavior: when side-effect recovery records terminal side-effect evidence, any held
resource lane is released before or as part of closing the attempt. Ambiguity plus paired
non-retryable `StateAttemptFailed` is one terminal recovery outcome.

Expected module/API shape: store projection releases resource lanes on all terminal side-effect
phases that close lane authority; rebuilt projections match live append projections.

Required tests:
- ambiguous side-effect terminal evidence releases the resource lane
- rebuilt projection after ambiguity has no active lane
- a second run can acquire the same lane after ambiguity without waiting for `RunCompleted`
- Postgres projection rebuild preserves the lane release

Current evidence:
- confirmation releases the resource lane:
  `crates/kernel/store/src/v1/projection.rs:606`
- side-effect failure releases the resource lane:
  `crates/kernel/store/src/v1/projection.rs:669`
- ambiguity releases the resource lane:
  `crates/kernel/store/src/v1/projection.rs:634`
- store tests cover live append, rebuild, and second-run reacquisition after ambiguity:
  `crates/kernel/store/tests/commit_contract.rs:3575`
- runtime tests cover peer progress after ambiguous holder release:
  `crates/kernel/runtime/src/tests.rs:6844`
- Postgres tests cover authoritative rebuild preserving cross-run lane projection behavior:
  `crates/storages/stream-store-postgres/src/typed.rs:2286`

Blocker: none currently known.

Required fix: none.

Acceptance tests:
- store append and rebuild test for ambiguity lane release
- cross-run same-lane acquisition after ambiguity
- Postgres rebuild test for persisted/rebuilt projection

Owner commit:
- store: release resource lanes on side-effect ambiguity

## Requirement: failure taxonomy keeps pre-authority failures outside stream

RFC: `RFC_REFACTOR_FSM_SCHEDULER.md:551-569`, `910-920`, `953-958`, `1257-1260`

Status: met

Expected behavior: binding, corruption, unverifiable history, and deployment failures are reported as
diagnostics and never become semantic run events. Only failures inside a valid started attempt become
semantic terminal evidence.

Expected module/API shape: verified context loader and bound context construction must run before
transition; observed failure terminalization checks active started attempt authority.

Required tests:
- missing binding produces no attempt start
- corrupt stream rejects before transition
- observed post-start failure terminalizes only when attempt is still started

Current evidence:
- scheduler drive loads verified context before decision:
  `crates/kernel/runtime/src/scheduler.rs:349`
- observed failure terminalization requires projected started attempt:
  `crates/kernel/runtime/src/attempt.rs:614`
- side-effect attempts with acquired ledger authority are excluded from generic observed failure:
  `crates/kernel/runtime/src/attempt.rs:625`

Blocker: none currently known.

Required fix: none.

Acceptance tests: keep binding, corrupt history, and post-start failure tests green.

Owner commit: already implemented.

## Requirement: attempt recovery owns open attempts

RFC: `RFC_REFACTOR_FSM_SCHEDULER.md:571-618`, `1160-1167`

Status: met

Expected behavior: open attempts are explicitly classified before other work is chosen. Recovery may
continue, retry terminalization, interrupt, block operationally, or delegate side-effect recovery.

Expected module/API shape: `AttemptRecoveryLifecycle` classifies verified open attempts and exposes
typed dispositions. Scheduler does not duplicate that policy.

Required tests:
- open attempt without independence witness blocks unrelated work
- process interruption can be recovered through interruption
- operational manual recovery remains separate from saga manual resolution

Current evidence:
- `AttemptRecoveryLifecycle` exposes open-attempt dispositions:
  `crates/kernel/runtime/src/recovery.rs:23`
- it detects started attempts from verified projections:
  `crates/kernel/runtime/src/recovery.rs:323`
- it validates frontier open-attempt consistency:
  `crates/kernel/runtime/src/recovery.rs:189`
- recovery-owned dispatch owns sync and async open-attempt execution:
  `crates/kernel/runtime/src/recovery.rs:154`, `crates/kernel/runtime/src/recovery.rs:174`
- recovery filters parked work with scoped resource-lane witnesses:
  `crates/kernel/runtime/src/recovery.rs:108`

Blocker: none currently known.

Required fix: none.

Acceptance tests:
- see scheduler-facade and resource-lane witness rows.

Owner commit:
- runtime: centralize open-attempt recovery dispatch and scoped independence

## Requirement: open framework attempts are recoverable and recompute evidence

RFC: `RFC_REFACTOR_FSM_SCHEDULER.md:619-637`, `1145-1158`, `1223-1224`

Status: met

Expected behavior: post-admission framework attempts append `StateAttemptStarted`, can be resumed
mid-attempt, and recompute terminal evidence from current verified history.

Expected module/API shape: framework lifecycle handles `PublicOutputRender`,
`ProjectRetentionManifest`, `CompleteRun`, and `ResolveSagaTerminal`; run admission remains outside
framework dispatch.

Required tests:
- open public-output render re-renders from verified history
- retention manifest rebuilds from current evidence
- completion/saga terminal proof is rebuilt against current prefix

Current evidence:
- framework lifecycle owns post-admission framework nodes only:
  `crates/kernel/runtime/src/framework_lifecycle.rs:36`
- framework lifecycle appends start before terminal output preparation:
  `crates/kernel/runtime/src/framework_lifecycle.rs:72`
- saga terminal proof is rebuilt from the current view:
  `crates/kernel/runtime/src/framework_lifecycle.rs:399`
- design documents the framework started-before-run model:
  `docs/design.md:405`

Blocker: none currently known.

Required fix: none.

Acceptance tests: keep framework lifecycle ordering/replay tests green.

Owner commit: already implemented.

## Requirement: side-effect lifecycle owns mutation uncertainty

RFC: `RFC_REFACTOR_FSM_SCHEDULER.md:639-692`, `1171-1184`

Status: met

Expected behavior: `InvocationStarted` remains the durable uncertainty boundary. After that boundary,
generic interruption/failure is not allowed unless side-effect recovery supplies evidence. Store
admission remains the source of ledger law and forward-fence authority.

Expected module/API shape: runtime `SideEffectLifecycle` classifies/resumes ledger phases while
store projection/admission enforces legal transitions, terminal failure pairing, and forward fence.

Required tests:
- side-effect after `InvocationStarted` cannot be generically interrupted/failed
- ambiguity pairs with non-retryable `StateAttemptFailed` in same commit
- forward `InvocationStarted` after saga engagement is rejected by store admission
- recovery tests cover each uncertainty phase

Current evidence:
- runtime side-effect lifecycle classifies prepared/started/submission/receipt/confirmed phases:
  `crates/kernel/runtime/src/side_effect_lifecycle.rs:102`
- generic observed failure refuses side-effect attempts with acquired ledger authority:
  `crates/kernel/runtime/src/attempt.rs:625`
- store requires terminal side-effect evidence to pair with attempt failure:
  `crates/kernel/store/src/lib.rs:6052`
- store requires side-effect attempt failures to carry terminal side-effect evidence:
  `crates/kernel/store/src/lib.rs:6115`
- design states store is source of truth for forward fence:
  `docs/design.md:449`

Blocker: none currently known.

Required fix: none.

Acceptance tests: keep side-effect recovery and forward-fence tests green.

Owner commit: already implemented.

## Requirement: saga routing and AC/DC claims are evidence-derived

RFC: `RFC_REFACTOR_FSM_SCHEDULER.md:694-767`, `1176-1180`, `1212-1222`

Status: met

Expected behavior: saga routing derives from certified spec, verified history, and admitted
side-effect evidence. Interruption does not engage saga. Manual resolution requires signed
prefix-bound proof over a quiescent manually blocked prefix with no open semantic attempts. Saga
terminal proofs are rebuilt from current verified history.

Expected module/API shape: store saga projection derives run mode and obligations; runtime/manual
resolution proof code verifies prefix authority; no scheduler-local proof cache remains.

Required tests:
- interruption does not engage saga
- manual resolution before manual-blocked prefix is rejected
- stale manual/SagaTerminalProof is rejected after prefix changes
- `ContinueAttempt` after saga engagement cannot cross new forward `InvocationStarted`

Current evidence:
- saga projection derives mode and obligations from side-effect projections:
  `crates/kernel/store/src/v1/saga.rs:3`
- interruption projection does not call `note_saga_engagement`; only non-retryable failed attempts do:
  `crates/kernel/store/src/v1/projection.rs:195`
- manual resolution commit checks run mode, expected sequence, prefix digest, block reason, and
  obligations digest: `crates/kernel/runtime/src/manual_resolution.rs:92`
- runtime manual prefix construction rejects same-run open semantic attempts before proof generation:
  `crates/kernel/runtime/src/manual_resolution.rs:36`
- store projection carries attempt run identity and exposes a run-scoped open-attempt guard:
  `crates/kernel/store/src/lib.rs:3638`, `crates/kernel/store/src/lib.rs:4054`
- store admission and projection apply the run-scoped guard for manual resolution:
  `crates/kernel/store/src/v1/admission.rs:16`, `crates/kernel/store/src/v1/projection.rs:806`
- `PreparedCommit<ManualResolution>::new` requires verified manual proof authority:
  `crates/kernel/store/src/lib.rs:1863`
- proof validation checks run id, spec hash, expected sequence, outcome, artifact refs, and policy:
  `crates/kernel/store/src/lib.rs:5697`
- terminal saga proof is rebuilt from current view and retained artifacts:
  `crates/kernel/runtime/src/framework_lifecycle.rs:399`
- no `manual_terminal_proofs` cache remains in runtime source (`rg manual_terminal_proofs` returns
  only RFC references).

Blocker: none currently known.

Required fix: none.

Acceptance tests:
- manually blocked prefix plus same-run open `StateAttemptStarted` rejects manual-resolution append
  and runtime `record_manual_resolution`
- manually blocked prefix plus unrelated-run open `StateAttemptStarted` admits manual resolution
- historical runtime/replay streams reject `ManualResolutionRecorded` over a same-run open-attempt
  prefix even if the attempt is terminalized later
- side-effect lane remains held when manual resolution is rejected over an open attempt
- manual-resolution prepared commit without verified proof fails
- stale/mismatched manual proof fails

Owner commit:
- store/runtime: require proof-backed manual resolution over no-open-attempt prefix

## Requirement: store remains semantic authority for resource lanes

RFC: `RFC_REFACTOR_FSM_SCHEDULER.md:405-409`, `682-692`, `1215-1217`

Status: met

Expected behavior: cross-run resource lane acquisition is store-admitted. A stale prediction is
handled by store `ResourceLaneBlocked` and re-decision, not by treating the frontier as authority.

Expected module/API shape: store projection owns lane acquisition/release and returns typed
`ResourceLaneBlocked`; runtime handles it as a blocked scheduling condition.

Required tests:
- store rejects conflicting lane acquisition across runs
- runtime handles `ResourceLaneBlocked` by re-deciding with a blocked hint
- public status exposes lane holder/blocker evidence

Current evidence:
- store lane acquisition returns `StoreError::ResourceLaneBlocked`:
  `crates/kernel/store/src/v1/resource_lanes.rs:46`
- runtime only treats typed `ResourceLaneBlocked` as a lane block:
  `crates/kernel/runtime/src/attempt.rs:757`
- app status exposes active and blocked lane holder data:
  `crates/app/src/lib.rs:2560`

Blocker: none currently known.

Required fix: none.

Acceptance tests: keep store lane-block tests and public status lane tests green.

Owner commit: already implemented for store authority.

## Requirement: public status separates run mode from attempt disposition

RFC: `RFC_REFACTOR_FSM_SCHEDULER.md:537-549`, `1124-1128`, `1232-1233`

Status: met

Expected behavior: no public `RunMode::Interrupted`; interruption is attempt-level and retryable by
recovery policy. Status exposes attempt dispositions separately.

Expected module/API shape: app response includes `run_mode`, saga status, and
`attempt_dispositions`; CLI/REST docs and real status tests cover the shape.

Required tests:
- app maps interrupted attempt to `disposition = interrupted` with no `retryable`
- CLI JSON includes started/interrupted/failed/completed dispositions
- REST JSON includes interrupted disposition

Current evidence:
- `TypedRunMode` has only the RFC run-mode variants:
  `crates/app/src/lib.rs:416`
- app response includes `attempt_dispositions`:
  `crates/app/src/lib.rs:667`
- app maps store interruption to public interrupted disposition:
  `crates/app/src/lib.rs:2535`
- CLI JSON fixture asserts interrupted:
  `bin/cli/tests/json_output_integration.rs:563`
- REST status route integration asserts interrupted:
  `tests/integration/tests/rest_api_run_control.rs:553`
- app status test appends a real `StateAttemptInterrupted` and observes post-admission framework
  attempts, then asserts public app stream order including `CompleteRun`:
  `crates/app/src/lib.rs:4076`, `crates/app/src/lib.rs:4159`,
  `crates/app/src/lib.rs:5074`
- REST status route test starts a real run, appends `StateAttemptInterrupted`, resumes, and calls
  `/v1/runs/:id/status` and `/v1/runs/:id/stream`, requiring `CompleteRun` disposition/order:
  `tests/integration/tests/rest_api_run_control.rs:483`,
  `tests/integration/tests/rest_api_run_control.rs:613`,
  `tests/integration/tests/rest_api_run_control.rs:835`
- CLI Postgres parity status test starts a real run, appends `StateAttemptInterrupted`, resumes,
  and calls `mfm run status` plus `mfm run stream`, requiring `CompleteRun` disposition/order:
  `bin/cli/tests/status_contract_postgres.rs:16`,
  `bin/cli/tests/status_contract_postgres.rs:159`,
  `bin/cli/tests/status_contract_postgres.rs:330`

Blocker: none currently known.

Required fix: none.

Acceptance tests:
- `mfm run status --output-format json` reports `interrupted` in `attempt_dispositions` with no
  retryable flag and no interruption run mode
- `GET /v1/runs/:id/status` reports the same shape
- public-output, retention, and complete-run framework attempts appear in dispositions with the new
  event order

Owner commit:
- tests: add real cli/rest run-status contract coverage

## Requirement: public transport ResolveSagaTerminal framework lifecycle fixture

RFC: `RFC_REFACTOR_FSM_SCHEDULER.md:379-383`, `1229`, `1238-1239`

Status: deferred-by-RFC

Expected behavior: runtime handles `ResolveSagaTerminal` as a post-admission framework node with
started-before-terminal ordering. This RFC does not add a dedicated app/CLI/REST saga-terminal
fixture or require public transport status/stream coverage for that saga terminal path in this
slice.

Expected module/API shape: runtime framework lifecycle remains authoritative for
`ResolveSagaTerminal`; public transport tests for the completed-run fixture require
`PublicOutputRender`, `ProjectRetentionManifest`, and `CompleteRun`.

Required tests:
- future public transport saga fixture should assert `ResolveSagaTerminal` appears in
  `attempt_dispositions` and its `StateAttemptStarted` precedes saga terminal `RunCompleted`
  evidence.

Current evidence:
- runtime framework lifecycle rebuilds saga terminal proof from the current view:
  `crates/kernel/runtime/src/framework_lifecycle.rs:399`
- app/REST/CLI completed-run public contracts explicitly require `CompleteRun` and skip
  `ResolveSagaTerminal` until a saga-terminal transport fixture is added:
  `crates/app/src/lib.rs:5071`,
  `tests/integration/tests/rest_api_run_control.rs:836`,
  `bin/cli/tests/status_contract_postgres.rs:334`

Blocker: none; deferred by RFC/public fixture scope.

Required fix: none for this RFC loop.

Acceptance tests:
- none until a future RFC/product slice adds a public saga-terminal transport fixture.

Owner commit:
- validation: record saga-terminal public transport fixture as deferred

## Requirement: public status filters resource-lane projection by target live lane interest

RFC: `RFC_REFACTOR_FSM_SCHEDULER.md:182-184`, `405-409`, `1124-1126`, `1231-1233`

Status: met

Expected behavior: public status uses store-owned resource-lane projection authority for derived
resource-lane fields while preserving semantic run mode. Public `saga.resource_lanes` is limited to
lanes referenced by target-run side-effect ledgers that are still in a live lane-interest phase; it
is not a store-wide lane inventory, and terminal/released ledgers do not report stale lane holders or
stale blockers.

Expected module/API shape: app status has access to cross-run lane projection authority, not only the
target run's rebuilt stream projection, but global lane authority remains internal to blocker
derivation unless a lane is target-owned or target-referenced. Async app launch, resume, and status
responses use the same store-owned status projection authority as CLI/REST status.

Required tests:
- RFC-required lane behavior is covered by runtime/store tests.
- non-RFC public transport coverage should assert global lane projection preservation when a run has
  ledger/resource-key projection evidence.
- public status coverage should assert unrelated global lane holders are not serialized as
  `saga.resource_lanes` for the queried run.
- public status coverage should assert released/terminal target ledgers do not report stale
  `blocked_by_lane` holders or referenced public lanes.
- service-path coverage should exercise `run_status_with_projection`/status projection merging, not
  only direct `TypedRunResponse` construction.

Current evidence:
- app status merges the target run projection with global store resource-lane projection:
  `crates/app/src/lib.rs:938`, `crates/app/src/lib.rs:1222`, `crates/app/src/lib.rs:2439`
- async app launch/resume/status responses read store-owned status projection authority before
  rendering:
  `crates/app/src/lib.rs:1138`, `crates/app/src/lib.rs:1168`,
  `crates/app/src/lib.rs:1174`, `crates/app/src/lib.rs:1242`
- async store trait exposes status projection authority and Postgres implements it with its
  cross-run projection snapshot:
  `crates/kernel/store/src/lib.rs:4765`, `crates/storages/stream-store-postgres/src/typed.rs:371`
- public saga status filters resource ledgers and `resource_lanes` through the queried run id and
  live lane-interest phase:
  `crates/app/src/lib.rs:2585`, `crates/app/src/lib.rs:2736`,
  `crates/app/src/lib.rs:2796`, `crates/app/src/lib.rs:2825`
- CLI status passes a store-owned projection snapshot into app status rendering:
  `bin/cli/src/commands/run/status.rs:48`
- REST status route passes a store-owned projection snapshot into app status rendering:
  `bin/rest-api/src/lib.rs:910`
- `blocked_by_lane` reads only the supplied projection:
  `crates/app/src/lib.rs:2736`
- actual lane conflict authority is cross-run store projection and Postgres append admission stages
  against stream-derived global lane projection:
  `crates/kernel/store/src/v1/resource_lanes.rs:46`
- app public JSON coverage asserts stream-admissible active lane status, rejects an unrelated global
  lane in `saga.resource_lanes`, and verifies released terminal phases do not report stale lane
  state:
  `crates/app/src/lib.rs:3286`, `crates/app/src/lib.rs:3501`,
  `crates/app/src/lib.rs:3725`
- app service-path coverage exercises `run_status_with_projection` with supplied global lanes and
  verifies public filtering:
  `crates/app/src/lib.rs:3733`, `crates/app/src/lib.rs:3797`,
  `crates/app/src/lib.rs:3801`

Required fix: none remaining for this RFC. If product wants live blocked-waiter status without
target-run ledger evidence, add a new RFC/design row for persisted waiter evidence or an explicit
live scheduler-status API.

Acceptance tests:
- app public JSON test with active target-owned lane status, no unrelated public `resource_lanes`,
  and no stale lane state after lane-release phase
- app service-path test for `run_status_with_projection` preserving global lane projection while
  filtering public `resource_lanes`
- runtime/store/Postgres tests for cross-run `ResourceLaneBlocked` behavior

Owner commit:
- app/store: preserve global lane projection for blocker derivation while filtering public lane
  inventory by run and live lane-interest phase

## Requirement: no-append resource-lane waiter public status

RFC: `RFC_REFACTOR_FSM_SCHEDULER.md:182-184`, `405-409`, `1124-1126`, `1231-1233`

Status: deferred-by-RFC

Expected behavior: the RFC defines lane admission, replay, and re-decision authority, but does not
require public status to report a no-append `ResourceLaneBlocked` prediction. Because the blocked
prepare is rejected before target-run side-effect resource-key evidence is appended, an end-to-end
`blocked_by_lane` status for that exact no-append state would require additional persisted waiter
authority or an explicit live scheduler-status authority not specified by the RFC.

Expected module/API shape: no public waiter API is required by this RFC. Runtime/store/Postgres
still enforce and test cross-run lane conflicts at admission authority boundaries.

Required tests:
- none for a public no-append waiter surface until a future RFC/design row adds persisted waiter
  evidence or a live scheduler-status API.
- keep runtime/store/Postgres tests for cross-run `ResourceLaneBlocked` behavior green.

Current evidence:
- runtime/store/Postgres tests cover cross-run resource-lane blocking at scheduler/admission
  boundaries:
  `crates/kernel/runtime/src/tests.rs:6918`,
  `crates/storages/stream-store-postgres/src/typed.rs:2440`
- validation no longer claims a public no-append waiter status as RFC-complete.

Blocker: none; deferred by RFC scope.

Required fix: none for this RFC.

Acceptance tests:
- if a future RFC adds public waiter status, add an end-to-end app/CLI/REST test proving non-null
  `blocked_by_lane` from persisted waiter evidence or live scheduler-status authority.

Owner commit:
- validation: defer no-append blocked-waiter public status to a future RFC/design decision

## Requirement: replay never constructs live capabilities or signers

RFC: `RFC_REFACTOR_FSM_SCHEDULER.md:203-205`, `134-135`, `415-417`

Status: met

Expected behavior: replay consumes certified spec plus stream evidence and retained artifacts only;
attempts to construct live capability access are rejected.

Expected module/API shape: replay authority exposes verifier-only contexts, not live capability or
signer providers.

Required tests:
- replay rejects live capability request
- replay rejects uncertified capability/fact/public-output evidence

Current evidence:
- replay crate docs state no live capability handles:
  `crates/kernel/replay/src/lib.rs:7`
- replay context exposes `reject_live_capability_request`:
  `crates/kernel/replay/src/lib.rs:734`
- replay handles `StateAttemptInterrupted` distinctly:
  `crates/kernel/replay/src/lib.rs:937`

Blocker: none currently known.

Required fix: none.

Acceptance tests: keep replay authority tests green.

Owner commit: already implemented.

## Requirement: Postgres codec and projections handle new event model

RFC: `RFC_REFACTOR_FSM_SCHEDULER.md:1115-1123`, `1234-1235`

Status: met

Expected behavior: Postgres storage round-trips `StateAttemptInterrupted`, rebuilds projections for
new-model streams, and rejects old-model rows with typed diagnostics.

Expected module/API shape: codec handles the new event; projection snapshots are rebuilt from the
authoritative stream and include global resource lanes.

Required tests:
- interrupted attempt projection persists and rebuilds from events
- projection snapshot ignores stale projection rows and rebuilds from stream
- stale resource-lane projection rows are repaired by stream authority and cannot veto free-lane
  appends
- old-model rows reject rather than project

Current evidence:
- test helper creates `StateAttemptInterrupted` payload:
  `crates/storages/stream-store-postgres/src/typed.rs:1450`
- Postgres test covers interrupted projection persistence/rebuild:
  `crates/storages/stream-store-postgres/src/typed.rs:2098`
- Postgres snapshot rebuilds projection from stream:
  `crates/storages/stream-store-postgres/src/typed.rs:651`
- global resource lanes are rebuilt from all streams:
  `crates/storages/stream-store-postgres/src/typed.rs:661`
- Postgres append admission uses stream-derived global lane authority and rejects stale/deleted
  projection-row conflicts as typed `ResourceLaneBlocked`:
  `crates/storages/stream-store-postgres/src/typed.rs:191`,
  `crates/storages/stream-store-postgres/src/typed.rs:2440`
- Postgres projection writer repairs stale free-lane projection rows by lane key from stream
  authority:
  `crates/storages/stream-store-postgres/src/typed.rs:977`,
  `crates/storages/stream-store-postgres/src/typed.rs:2615`

Blocker: none currently known.

Required fix: none.

Acceptance tests: keep `cargo test -p mfm-stream-store-postgres` green.

Owner commit: already implemented.

## Requirement: docs/design and public docs are updated with authority changes

RFC: `RFC_REFACTOR_FSM_SCHEDULER.md:1023-1042`, `1181-1191`

Status: met

Expected behavior: design docs reflect admission, transition, event schema, framework lifecycle,
side-effect/saga authority, and public status. CLI/REST docs reflect public JSON contracts.

Expected module/API shape: `docs/design.md` is the normative runtime authority contract and public
README files document stable API surfaces.

Required tests:
- docs are reviewed with code changes
- public JSON fixtures match docs

Current evidence:
- design covers admission/bound runtime context/old-model guard:
  `docs/design.md:335`
- design covers transition/recovery:
  `docs/design.md:366`
- design covers started-before-run and framework lifecycle:
  `docs/design.md:377`, `docs/design.md:405`
- design covers side-effect interruption and forward fence:
  `docs/design.md:440`
- CLI/REST docs cover run mode and attempt dispositions:
  `bin/cli/README.md:341`, `bin/rest-api/README.md:254`
- design documents `RunAdmissionLifecycle`, single-root admission, and post-admission framework
  runners:
  `docs/design.md`
- runtime README documents `RunAdmissionLifecycle` and `RunCompleted` authority from `CompleteRun`
  or `ResolveSagaTerminal`:
  `crates/kernel/runtime/README.md`
- architecture namespace contract rejects stale synthetic-admission wording:
  `tests/integration/tests/architecture_namespace_contract.rs`

Blocker: none currently known.

Required fix: none.

Acceptance tests:
- docs guard test fails on stale admission-only and complete-run-only wording
- runtime tests prove admission launch succeeds with exactly one `RunAdmitted` root payload and no
  synthetic admission attempt

Owner commit:
- docs: publish flat run admission model

## Requirement: final validation evidence is recorded

RFC: `RFC_REFACTOR_FSM_SCHEDULER.md:1193-1235`

Status: met

Expected behavior: focused and broad verification evidence is recorded under `docs/validation/`.

Expected module/API shape: validation artifact lists commands, pass/fail status, and any residual
risk.

Required tests:
- `cargo fmt --all -- --check`
- `cargo test -p mfm-runtime`
- `cargo test -p mfm-store --test commit_contract`
- `cargo test -p mfm-replay`
- `cargo test -p mfm-stream-store-postgres`
- `cargo test -p mfm-app`
- `cargo test -p mfm-integration-tests --test architecture_namespace_contract`

Current evidence:
- prior closeout exists at `docs/validation/fsm-scheduler-closeout-2026-06-18.md`
- this loop added focused runtime/store/app/public-status coverage and ran the requested gate:
  - `cargo fmt --all -- --check` passed
  - `cargo test -p mfm-runtime` passed
  - `cargo test -p mfm-store --test commit_contract` passed
  - `cargo test -p mfm-replay` passed
  - `cargo test -p mfm-stream-store-postgres` passed
  - `cargo test -p mfm-app` passed
  - `cargo test -p mfm-integration-tests --test architecture_namespace_contract` passed
- supporting public-surface checks also passed:
  - `cargo test -p mfm-integration-tests --test rest_api_run_control`
  - `cargo test -p mfm --test json_output_integration test_run_status_json_contract_exposes_attempt_dispositions_and_saga_blocks`
  - `cargo test -p mfm-app async_status_reports_interrupted_attempt_and_framework_attempts_from_history`
  - `cargo test -p mfm-integration-tests --test rest_api_run_control status_route_reports_interrupted_attempt_and_framework_attempts_from_history`
- supporting re-audit blocker checks also passed:
  - `cargo test -p mfm-runtime post_start_`
  - `cargo test -p mfm-integration-tests --test architecture_namespace_contract design_docs_do_not_make_public_output_projected_a_transition_decision`
  - `cargo test -p mfm-stream-store-postgres --features parity-tests --no-run`
  - `cargo test -p mfm --features parity-tests --test status_contract_postgres --no-run`
- after the post-review cleanup, full service-backed `.#ci` passed with managed Postgres and Reth:
  `docs/validation/fsm-scheduler-closeout-2026-06-18.md`
- cross-run no-append blocked-waiter status is recorded as `deferred-by-RFC` rather than claimed as
  complete, because the RFC specifies lane admission/re-decision but not persisted waiter status.

Blocker: none currently known.

Required fix: none.

Acceptance tests: all verification gate commands pass or are recorded with honest blocker details.

Owner commit:
- validation: record fsm scheduler RFC traceability and verification evidence

## Architect blocker findings

### Runtime scheduler/lifecycle architecture

Reviewer: Architect A

Status: cleared by re-audit

Findings:
- Cleared: same-namespace side-effect work is conservatively parked until concrete
  lane evidence exists or the parked lane releases. The RFC and design now record this stricter rule;
  see the requirement row "serial execution with resource-lane-scoped independence witness".
- Cleared: recovery disposition dispatch is owned by `AttemptRecoveryLifecycle`. See the
  requirement row "scheduler facade is thin and recovery dispatch is single-owned".
- Cleared: failure-safe terminalization derives retryability from certified policy plus failure
  classification and now keeps corrupt/unverifiable `InvalidRunStream` authority outside semantic
  `StateAttemptFailed`. See the requirement row "failure-safe terminalization uses trusted failure
  classification".

### Store/events/replay/Postgres authority

Reviewer: Architect B

Status: cleared by final re-audit

Findings:
- Cleared: `SideEffectAmbiguous` releases a held resource lane. See the requirement row
  "side-effect terminal evidence releases resource lanes".
- Cleared: Postgres append admission and projection repair use stream-derived global resource-lane
  authority. Stale free-lane projection rows no longer veto valid appends or rebuilds. See the
  requirement row "Postgres codec and projections handle new event model".

### Saga/side-effect/manual proof semantics

Reviewer: Architect C

Status: cleared by re-audit

Findings:
- Cleared: manual saga resolution rejects same-run open semantic attempts in runtime prefix
  construction, store admission, and historical/replay validation while admitting unrelated-run open
  attempts. See the requirement row "saga routing and AC/DC claims are evidence-derived".
- Cleared: `ManualResolutionRecorded` requires proof-bound manual authority. See the requirement row
  "saga routing and AC/DC claims are evidence-derived".
- RFC ambiguity requiring human decision, non-blocking: whether `SideEffectFailed.retryable` itself
  must be policy-derived evidence, or whether this RFC only requires paired `StateAttemptFailed`
  retryability to match it. Current implementation enforces the pairing/match.

### Public API/docs/test contracts

Reviewer: Architect D

Status: cleared/deferred-by-RFC

Findings:
- Cleared: `docs/design.md` now lists only RFC transition decisions and guards against
  `PublicOutputProjected`/blocked-completion wording returning.
- Cleared: app/REST/CLI public status and stream contracts require `CompleteRun` framework attempt
  disposition and start-before-`RunCompleted` ordering for completed runs. See the requirement row
  "public status separates run mode from attempt disposition".
- Deferred by RFC: `ResolveSagaTerminal` public transport fixture coverage is explicitly deferred
  until a future saga-terminal transport fixture slice. See the requirement row "public transport
  ResolveSagaTerminal framework lifecycle fixture".
- Deferred by RFC: no-append cross-run lane blocked-waiter public status is not specified by the
  RFC and would require new persisted waiter or live scheduler-status authority. See the requirement
  row "no-append resource-lane waiter public status".
- Cleared: validation evidence no longer overclaims no-append blocked-waiter status, no longer
  claims live `blocked_by_lane` coverage, and records the gate commands actually run. See the
  requirement row "final validation evidence is recorded".

### Cleanup/dead-code/compatibility sweep

Reviewer: Architect E

Status: cleared/deferred-by-RFC after final re-audit

Findings:
- Cleared: recovery dispatch authority is centralized outside the scheduler facade, and sync/async
  driver paths share the same transition/recovery/attempt lifecycle classification. Runtime tests
  cover sync and async resource-lane behavior plus framework/failure-safe paths.
- Cleared: runtime README assigns public-output rendering to `framework_lifecycle`, with an
  architecture guard for the stale wording.
- Cleared: final cleanup sweep found no stale compatibility paths or stale docs in the re-audited
  diff.
- Deferred by RFC: full sync/async driver collapse is outside this loop. See the requirement row
  "full sync/async driver collapse is deferred".

## Coordinator implementation queue

1. runtime: carry resource-lane independence witness through blocked scheduling — completed
   - fixes: resource-lane scoped witness blocker
   - tests: same-lane blocked side-effect cannot start second same-lane node; independent lane/node
     still advances

2. runtime: centralize open-attempt recovery dispatch outside scheduler facade — completed
   - fixes: duplicated recovery dispatch and scheduler facade bulk
   - tests: architecture/namespace guard plus recovery disposition behavior tests

3. store: release resource lanes on side-effect ambiguity — completed
   - fixes: ambiguity terminal evidence lane release
   - tests: store append/rebuild, cross-run reacquisition, Postgres rebuild

4. store/runtime: require proof-backed manual resolution over no-open-attempt prefix — completed
   - fixes: manual resolution open-attempt guard and proof-backed store admission
   - tests: store admission rejects open attempts and missing/stale/mismatched proof; runtime helper
     fails before append

5. runtime/docs: derive failure-safe retryability from policy and failure class — completed
   - fixes: hard-coded retryability conflict
   - tests: classifier coverage and paired side-effect retryability match

6. app/cli/rest/tests: preserve global lane projection for blocker derivation — completed
   - fixes: status projection no longer drops cross-run lane holders when ledger evidence exists,
     while public `resource_lanes` omits unrelated global holders and stale released-ledger
     references
   - tests: app public JSON coverage for active target-owned lanes, filtered `resource_lanes`, and
     released-phase suppression; app service-path coverage for `run_status_with_projection`;
     no-append blocked-waiter status remains a future RFC/design decision

7. runtime/docs: align `PublicOutputProjected` with RFC transition decision shape — completed
   - fixes: non-blocking decision-shape variance unless architect re-audit escalates it
   - tests: decision-shape/status behavior coverage

8. runtime/docs: record sync/async driver collapse as deferred-by-RFC — completed
   - fixes: sync/async duplicated driver blocker by narrowing RFC scope
   - tests: parity over ordinary success, framework lifecycle, failure-safe materialization failure,
     and resource-lane block/stale reload

9. validation: record final verification evidence — completed
   - fixes: final loop stop condition
   - tests: full verification gate listed above
