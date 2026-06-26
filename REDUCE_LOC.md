# LOC Reduction Candidates for Per-Process Fungibility

Date: 2026-06-26

Scope: compare this branch with `dev`, then identify implementation and test code
that can be removed, reused, or abstracted without weakening the RFC.

Current baseline:

- `git diff --shortstat dev`: `93 files changed, 16836 insertions(+), 2968 deletions(-)`.

Research inputs:

- Store/Postgres/admission lane slice.
- Runtime/certify/replay/events/spec slice.
- App/CLI/REST/EVM adapter slice.
- Tests/fixtures slice.
- Local cross-checks with `rg`, `git diff`, and targeted file reads.

The LOC estimates below are approximate net reductions. They include likely helper
additions where an abstraction is needed, so they should be treated as triage ranges,
not exact promises.

## Non-Negotiable RFC Guardrails

Do not reduce LOC by weakening these:

- Keep the RFC's one reusable admission-lane primitive: one table-pair and one
  `admit(mode)` shape for liveness coordination.
- Keep admission tables liveness-only. Resource ownership must remain event-derived
  from committed run stream evidence.
- Keep v1 invoker-driven and manual-resumable. Do not add worker-pool dispatch,
  feed-driven dispatch, automatic dead-driver takeover, or `due_at` retry machinery
  as part of LOC cleanup.
- Keep side effects lowered into submit + verify behavior. The verify side owns
  downstream-visible output and terminal evidence validation.
- Keep one pair-keyed side-effect ledger end state. Node id and attempt id may remain
  attribution, but they must not become ledger authority again.
- Keep finality policy certified in the spec and replayed from the committed stream.
  Do not reintroduce a registry, oracle, or worker-local policy lookup.
- Keep signer/resource lanes released only from proven terminal evidence or certified
  manual-resolution terminal behavior. Never timeout-release a signer lane.
- Keep security-sensitive and replay tests that prove the above. Reducing duplicated
  fixture code is fine; deleting coverage is not.

## Store and Postgres Admission

### S1. Abstract one Postgres `admit(mode)` primitive

Target:

- `crates/storages/stream-store-postgres/src/run_store/admission_lanes.rs:20`
- `crates/storages/stream-store-postgres/src/run_store/admission_lanes.rs:179`
- `crates/storages/stream-store-postgres/src/run_store/resource_lanes.rs:118`

Proof:

- The RFC requires one table-pair and one `admit(mode)` code path over
  lock/admit/renew/release/reap/head-of-line semantics.
- Current Postgres code has separate FIFO waiter procedures and execution-claim
  procedures, plus resource-specific pre-gating.

Safe reduction:

- Factor shared SQL row, lock, lease, expiry, and result-shaping mechanics through
  a single lane helper parameterized by mode.
- Keep resource authority outside SQL rows; resource holders remain event-derived.

Risk and verification:

- Medium.
- Run focused Postgres tests for FIFO fairness, execution claim busy/renew/release,
  expiry/reap, and stale-token behavior.

Impact:

- About 80-160 LOC.

### S2. Remove the duplicated active-holder check in Postgres FIFO pre-gate

Target:

- `crates/storages/stream-store-postgres/src/run_store/append.rs:111`
- `crates/storages/stream-store-postgres/src/run_store/resource_lanes.rs:118`
- `crates/kernel/store/src/v1/mod.rs:5476`

Proof:

- Typed staging already blocks when another committed holder owns the lane.
- Postgres separately checks the same projected holder before FIFO admission.

Safe reduction:

- Let typed staging decide holder ownership.
- Keep Postgres pre-gate focused on FIFO head-of-line admission after staging has
  proven the lane is holder-free.
- Preserve waiter enqueue behavior when staging reports a block.

Risk and verification:

- Medium because concurrency races are the point of this code.
- Re-run holder + waiter tests and Postgres resource-lane parity tests.

Impact:

- About 15-35 LOC plus one fewer repeated branch.

### S3. Reuse central resource-lane release resolution from `mfm-store`

Target:

- `crates/storages/stream-store-postgres/src/run_store/resource_lanes.rs:31`
- `crates/kernel/store/src/v1/mod.rs:5534`
- `crates/kernel/store/src/v1/resource_lanes.rs:99`

Proof:

- Postgres resolves and validates release lane identity before locking.
- Typed staging and projection release already perform similar pair/claim/epoch checks.
- The RFC places resource-lane authority in the typed store contract, not SQL liveness rows.

Safe reduction:

- Move or expose the reusable "which resource lane is this terminal side-effect
  evidence allowed to release?" calculation from `mfm-store`.
- Have Postgres call that helper before locking the lane row.

Risk and verification:

- Medium.
- Preserve pair-bound release and mismatch tests.

Impact:

- About 25-45 LOC.

### S4. Remove unused FIFO grant/result public types

Target:

- `crates/kernel/store/src/v1/admission_lanes.rs:303`
- `crates/kernel/store/src/v1/admission_lanes.rs:310`
- `crates/kernel/store/src/v1/mod.rs:310`

Proof:

- `WaitFifoAdmissionGrant` and `WaitFifoAdmissionResult` are defined and re-exported,
  but current `rg` results show no consumers outside definition/re-export paths.
- The real append surface returns `CommitOutcome::AdmissionBlocked`.

Safe reduction:

- Delete the unused types and the re-export.

Risk and verification:

- Low, unless external public API consumers already depend on these unused types.
- `cargo check --workspace` catches in-repo fallout.

Impact:

- About 12-18 LOC.

### S5. Remove unused `WaitFifoAdmissionBlock.lane`

Target:

- `crates/kernel/store/src/v1/admission_lanes.rs:324`
- `crates/kernel/store/src/v1/mod.rs:5476`
- `crates/storages/stream-store-postgres/src/run_store/resource_lanes.rs:131`
- `crates/kernel/runtime/src/attempt.rs:688`

Proof:

- Consumers read `resource_lane_key` and `waiter`, not `block.lane`.
- The lane value is derivable from `resource_lane_key` where needed.

Safe reduction:

- Delete the field and constructor assignments.

Risk and verification:

- Low-medium because blocked-lane status JSON might indirectly expose it if converted
  elsewhere.
- Run compile and blocked-resource-lane status tests.

Impact:

- About 6-12 LOC.

### S6. Keep `ResourceLaneProjection`; only consider removing duplicated fields

Target:

- `crates/kernel/store/src/v1/mod.rs:3543`
- `crates/storages/stream-store-postgres/src/run_store/projections.rs:158`
- `crates/kernel/store/src/v1/resource_lanes.rs:58`

Proof:

- `ResourceLaneProjection` is the event-derived authority that the RFC requires.
- It currently stores both `holder: SideEffectPairLedgerRef` and duplicated
  pair/node/attempt/invocation fields.

Safe reduction:

- Do not remove the projection.
- A possible later reduction is to remove fields derivable from `holder` if they are
  not part of public projection JSON or diagnostics.

Risk and verification:

- High. This crosses persisted/event projection surfaces.
- Only do it with projection serialization tests and resource-lane release tests.

Impact:

- Unknown, likely small. Treat as lower priority.

## Runtime, Certify, Replay, Events, and Spec

### R1. Replace legacy confirmation-only terminal validation with policy-aware validation

Target:

- `crates/kernel/runtime/src/side_effect_lifecycle.rs:187`
- `crates/kernel/runtime/src/side_effect_lifecycle.rs:450`
- `crates/kernel/runtime/src/side_effects.rs:268`
- `crates/kernel/runtime/src/commit.rs:2569`
- `crates/kernel/spec/src/lib.rs:1847`

Proof:

- The RFC requires terminal evidence to follow certified `Receipt` or `Finalized(depth)`
  policy.
- `side_effect_lifecycle.rs` still has a confirmation-only terminal branch, while
  runtime side-effect validation already has policy-aware paths.

Safe reduction:

- Keep terminal validation, but remove the old confirmation-only validator branch.
- Route submit/verify/frontier/recovery validation through the policy-aware helper.

Risk and verification:

- Medium.
- Keep tests for receipt-terminal verify, finalized-terminal verify, submit skipped
  evidence, frontier validation, recovery validation, and commit validation.

Impact:

- About 20-50 LOC, more if the two validators are unified cleanly.

### R2. Finish pair-only side-effect authority and delete old node/attempt ledger validation

Target:

- `crates/kernel/events/src/lib.rs:247`
- `crates/kernel/events/src/lib.rs:625`
- `crates/kernel/runtime/src/side_effect_lifecycle.rs:352`
- `crates/kernel/runtime/src/side_effects.rs:54`
- `crates/kernel/replay/src/lib.rs:266`

Proof:

- The RFC and plan commit to one ledger keyed by certified pair identity, not by
  `{attempt_id,node_id}`.
- The plan explicitly calls for deleting node/attempt ledger keying and dual-validation
  scaffolding.

Safe reduction:

- Keep node id and attempt id as event attribution where needed.
- Remove any remaining code that treats them as ledger authority.

Risk and verification:

- High.
- Requires event schema, admission, replay, lane release, and recovery regressions.

Impact:

- About 150-300 LOC in focused files, plus test/schema churn.

### R3. Merge runtime and pre-invocation side-effect binding helpers

Target:

- `crates/kernel/runtime/src/side_effect_driver.rs:993`
- `crates/kernel/runtime/src/side_effect_driver.rs:1005`
- `crates/kernel/runtime/src/side_effect_driver.rs:1048`

Proof:

- Runtime and pre-invocation helpers duplicate pair id, ledger purpose, ledger key,
  owner, fencing token, and claim authority construction.
- The RFC requires deterministic pair-bound authority, not two context-specific
  implementations.

Safe reduction:

- Introduce a small local binding input trait or struct that exposes run id, node,
  attempt id, runtime spec, and projections.
- Keep output byte-for-byte equivalent.

Risk and verification:

- Low-medium.
- Add or keep golden tests for claim owner, fencing token, ledger key, and preclaim
  behavior.

Impact:

- About 60-100 LOC.

### R4. Collapse `SideEffectInvocationOutputBuilder` start and non-start variants

Target:

- `crates/kernel/runtime/src/side_effect_driver.rs:1548`
- `crates/kernel/runtime/src/side_effect_driver.rs:1593`
- `crates/kernel/runtime/src/side_effect_driver.rs:1631`
- `crates/kernel/runtime/src/side_effect_driver.rs:1719`
- `crates/kernel/runtime/src/side_effect_driver.rs:1876`

Proof:

- Method pairs such as `submission_observed` and
  `start_prepared_and_submission_observed` differ mostly by optional
  `InvocationStarted`.
- The file already has internal optional-start helpers.

Safe reduction:

- Make optional start a parameter of the shared builder path.
- Preserve exact event ordering.

Risk and verification:

- Low.
- Verify prepared paths emit start once and already-started paths do not duplicate it.

Impact:

- About 40-70 LOC.

### R5. Drive commit artifact binding from `SideEffectEventRef`

Target:

- `crates/kernel/runtime/src/commit.rs:1402`
- `crates/kernel/runtime/src/commit.rs:1544`
- `crates/kernel/events/src/lib.rs:625`

Proof:

- `staged_payload_artifact_binding` repeats side-effect extraction across event variants.
- `events` already exposes `side_effect_ref()`.

Safe reduction:

- Use `SideEffectEventRef` for common extraction.
- Keep source, phase, attempt, pair, artifact role, and mismatch checks intact.

Risk and verification:

- Medium.
- Cover every side-effect artifact role and mismatch source handling.

Impact:

- About 50-90 LOC.

### R6. Centralize replay side-effect evidence lookup and event indexing

Target:

- `crates/kernel/replay/src/lib.rs:752`
- `crates/kernel/replay/src/lib.rs:786`
- `crates/kernel/replay/src/lib.rs:843`
- `crates/kernel/replay/src/lib.rs:881`
- `crates/kernel/replay/src/lib.rs:922`
- `crates/kernel/replay/src/lib.rs:1999`

Proof:

- Replay repeats "find by pair + epoch, verify schema/hash/artifact, verify against
  intent" for submission, not-submitted, receipt, confirmation, and ambiguity evidence.
- The RFC wants replay from certified spec and committed stream only, which a shared
  helper preserves.

Safe reduction:

- Build one indexed evidence lookup helper per pair/epoch/evidence-kind.
- Keep all verifier and artifact hash checks.

Risk and verification:

- Medium.
- Replay tests must cover every evidence kind, duplicate events, wrong verifier, and
  wrong artifact hash.

Impact:

- About 80-150 LOC.

### R7. Shrink redundant fields in `SideEffectEvidenceReplayRequest`

Target:

- `crates/kernel/replay/src/lib.rs:266`
- `crates/kernel/replay/src/lib.rs:378`
- `crates/kernel/replay/src/lib.rs:1400`

Proof:

- `SideEffectEvidenceReplayRequest` carries ledger, node, attempt, capability,
  adapter, schema, and hash fields that `SideEffectReplayFrame` reconstructs from
  recorded intent and then compares back to that same indexed intent.
- Pair authority plus certified stream replay makes pair/epoch/evidence kind plus
  expected verifier sufficient at the lookup boundary.

Safe reduction:

- Keep mismatch checks at frame/index construction.
- Remove self-comparison fields from the request surface.

Risk and verification:

- Medium-high if this request type is public API.
- Preserve tests for wrong pair, wrong evidence hash, wrong verifier, and wrong spec.

Impact:

- About 30-80 LOC.

### R8. Reuse the generic framework output-cell validator for verify output cells

Target:

- `crates/kernel/certify/src/framework_lifecycle.rs:691`
- `crates/kernel/certify/src/framework_lifecycle.rs:721`

Proof:

- `validate_side_effect_verify_output_cell` repeats schema, semantic, and storage checks
  that exist in the generic framework output-cell validator.
- The RFC requires a distinct side-effect verify validation family, but not duplicate
  generic cell-shape code.

Safe reduction:

- Keep side-effect pair/evidence checks separate.
- Delegate only generic output-cell shape validation.

Risk and verification:

- Low.
- Certify tests for bad producer, schema, semantic, and storage constraints.

Impact:

- About 20-30 LOC.

### R9. Reconsider persisted `submit_output_cell_id` on verify node specs

Target:

- `crates/kernel/spec/src/lib.rs:466`
- `crates/kernel/spec/src/lib.rs:1964`
- `crates/kernel/certify/src/framework_lifecycle.rs:118`
- `crates/kernel/certify/src/framework_lifecycle.rs:752`
- `crates/kernel/program/src/lib.rs:4146`
- `crates/kernel/runtime/src/tests.rs:12668`

Proof:

- The RFC says the original side-effect node's downstream-visible output is produced
  by the verify node.
- Current spec stores `submit_output_cell_id` on `SideEffectVerifyNodeSpec`, validates
  it against the submit descriptor, and validates that the submit output only feeds
  the verify node.

Safe reduction:

- If the submit output cell remains a private anchor, derive it from the certified
  submit descriptor instead of persisting it in the verify node spec.
- Keep the "downstream can only observe verify output" invariant.

Risk and verification:

- High. This touches hash-defining spec JSON and certification.
- Only do this with explicit acceptance that the certified spec shape changes.

Impact:

- About 30-100 LOC, but this is a design-alignment cleanup, not a first-pass trim.

### R10. Remove the `verify_pair` lowering branch if remediation side effects must also verify

Target:

- `crates/kernel/program/src/lib.rs:3902`
- `crates/kernel/program/src/lib.rs:4093`
- `crates/kernel/program/src/lib.rs:4131`
- `crates/kernel/program/src/lib.rs:4585`

Proof:

- Normal side-effect lowering passes `verify_pair = true`.
- `side_effect_with_compensation` introduces a special branch that can skip verify-pair
  lowering for remediation.
- The RFC language says each side-effect node lowers into submit + verify behavior.

Safe reduction:

- If remediation side effects are intended to use the same terminal-evidence contract,
  remove the boolean branch and lower them uniformly.
- If remediation is intentionally different, document the exception rather than keeping
  an unexplained branch.

Risk and verification:

- High. This can add or reshape certified nodes and saga obligations.
- Requires program, certify, runtime, and saga tests.

Impact:

- LOC impact is uncertain. It may reduce branching but increase generated spec nodes.

### R11. Remove dead or test-only side-effect view methods after driver refactor

Target:

- `crates/kernel/runtime/src/side_effect_lifecycle.rs:108`
- `crates/kernel/runtime/src/side_effect_lifecycle.rs:113`
- `crates/kernel/runtime/src/side_effect_lifecycle.rs:120`
- `crates/kernel/runtime/src/frontier.rs:292`

Proof:

- `SideEffectAttemptView` exposes ledger-key, ledger-purpose, and empty-state helpers.
- Some uses are test assertions or can become unnecessary if side-effect binding helpers
  are unified around a smaller authority object.

Safe reduction:

- Do not remove these first.
- After R3/R5, re-run `rg` and delete methods that have become test-only or unused.

Risk and verification:

- Low if done after call-site removal and compile.

Impact:

- Small, about 5-20 LOC.

### R12. Keep `SideEffectVerify` as a separate validation family

Target:

- `crates/kernel/certify/src/framework_lifecycle.rs:84`
- `crates/kernel/runtime/src/commit.rs:2480`
- `crates/kernel/runtime/src/commit.rs:2569`

Proof:

- The RFC requires verify-specific binding of terminal evidence, finality policy,
  output ownership, and pair authority.

Safe reduction:

- Reuse generic helpers for common mechanics, but do not collapse verify validation into
  ordinary state validation.

Risk and verification:

- This is a guardrail, not a removal candidate.

Impact:

- None directly; prevents unsafe false reductions.

## App, CLI, and REST

### A1. Remove unreachable `IdentityMismatch` launch outcome

Target:

- `crates/app/src/lib.rs:921`
- `crates/app/src/lib.rs:1355`
- `crates/app/src/lib.rs:1398`
- `crates/app/src/lib.rs:1470`
- `bin/cli/src/commands/run/start.rs:126`
- `bin/rest-api/src/lib.rs:544`

Proof:

- `IdentityMismatch` appears in enum/mapping code but is not constructed as a normal
  launch outcome.
- Actual identity mismatches return `run_identity_material_mismatch()`.
- The RFC treats identity mismatch as collision/corruption that should fail closed.

Safe reduction:

- Delete the outcome variant and transport mappings if product semantics confirm that
  identity mismatch is an error, not a successful attach/report status.
- Make outcome-to-response conversion simpler.

Risk and verification:

- Medium because the plan listed `IdentityMismatch` as a typed outcome.
- Confirm with app contract tests, CLI JSON tests, and REST run-control tests.

Impact:

- About 25-40 LOC.

### A2. Remove redundant preflight stream read before run admission

Target:

- `crates/app/src/lib.rs:1418`
- `crates/app/src/lib.rs:1426`
- `crates/app/src/lib.rs:1445`

Proof:

- `launch_run` loads the stream, then calls `expected_next_seq`, then retries attach
  after `start_run` race failure.
- The RFC launch pseudocode relies on try-admit then attach; safety is CAS/per-run
  append serialization, not a pre-read.

Safe reduction:

- Use `expected_next_seq` and the post-`start_run` race path.
- Keep identity validation before admission.

Risk and verification:

- Medium.
- Re-run duplicate launch, concurrent launch, empty-store, and same-run attach tests.

Impact:

- About 10-15 LOC plus one avoided DB read.

### A3. Share CLI/REST run-start orchestration in `crates/app`

Target:

- `bin/cli/src/commands/run/start.rs:117`
- `bin/cli/src/commands/run/start.rs:126`
- `bin/rest-api/src/lib.rs:535`
- `bin/rest-api/src/lib.rs:544`

Proof:

- CLI and REST both clone run id, discover public schema, call `launch_run`, split
  outcome, and optionally render public output.
- Repository guidance keeps binaries thin and app assembly in reusable crates.

Safe reduction:

- Add an app helper returning launch outcome, run report, and optional public output.
- Keep transport-specific parsing/rendering in CLI/REST.

Risk and verification:

- Low-medium.
- Re-run CLI JSON output tests and REST run-control tests.

Impact:

- About 25-40 LOC net.

### A4. Abstract repeated REST store bounds with private marker traits

Target:

- `bin/rest-api/src/lib.rs:466`
- `bin/rest-api/src/lib.rs:500`
- `bin/rest-api/src/lib.rs:570`

Proof:

- Handlers repeat the same long bounds:
  `RunEventStore + TrustScopeStore + ExecutionClaimStore + RetainedArtifactReadProvider + Clone + Send + Sync + 'static`.

Safe reduction:

- Introduce private marker traits such as `RunCommandStore` and `RunObservationStore`
  inside `bin/rest-api`.

Risk and verification:

- Low.
- `cargo test -p mfm-rest-api`.

Impact:

- About 25-35 LOC.

### A5. Keep executable mismatch as attach/report, not a duplicate driver

Target:

- `crates/app/src/lib.rs:1398`

Proof:

- The RFC says same run identity with incompatible executables attaches/reports and
  does not drive.

Safe reduction:

- Do not remove executable-binding checks while removing unreachable identity mismatch
  paths.

Risk and verification:

- This is a guardrail.

Impact:

- None directly.

## EVM Contract Domain and Adapter

### E1. Remove stale aggregate receipt value

Target:

- `crates/states/evm-contracts/src/lib.rs:270`
- `crates/states/evm-contracts/src/lib.rs:459`
- `crates/states/evm-contracts/src/lib.rs:570`
- `crates/adapters/evm-contracts/src/lib.rs:2425`
- `crates/adapters/evm-contracts/src/lib.rs:3239`

Proof:

- `ContractTransactionReceipts` is no longer the side-effect receipt type.
- Deploy/configure use `ContractDeployReceipt` and `ContractConfigureReceipt`.
- Remaining uses are an adapter wrapper and `single_receipt`.

Safe reduction:

- Construct the typed deploy/configure receipt directly and delete the aggregate wrapper.

Risk and verification:

- Low-medium if public API consumers are in-repo only.
- Run EVM state and adapter tests.

Impact:

- About 20 LOC.

### E2. Remove unused confirmation projection helpers

Target:

- `crates/adapters/evm-contracts/src/lib.rs:747`
- `crates/adapters/evm-contracts/src/lib.rs:767`
- `crates/adapters/evm-contracts/src/lib.rs:2496`
- `crates/adapters/evm-contracts/src/lib.rs:2677`

Proof:

- `confirm_deploy` and `confirm_configure` are definitions only.
- The active verify path already invokes domain output methods for deploy/configure.

Safe reduction:

- Delete the dead helpers.

Risk and verification:

- Low-medium because they are public functions.
- Run adapter tests and workspace check.

Impact:

- About 35 LOC.

### E3. Abstract deploy/configure mutation driver callbacks

Target:

- `crates/adapters/evm-contracts/src/lib.rs:2176`
- `crates/adapters/evm-contracts/src/lib.rs:2279`

Proof:

- Deploy and configure callback implementations duplicate idempotency handling,
  prepared-artifact loading, runtime lookup, submit, and recover mechanics.
- Only phase-specific prepare/reconstruct code differs.

Safe reduction:

- Keep the abstraction inside the EVM adapter.
- Use typed phase hooks for deploy/configure differences.

Risk and verification:

- Medium.
- Re-run adapter recovery tests, re-sign mismatch tests, and foreign nonce occupancy tests.

Impact:

- About 50-90 LOC.

### E4. Abstract deploy/configure verify callback artifact loading

Target:

- `crates/adapters/evm-contracts/src/lib.rs:2396`
- `crates/adapters/evm-contracts/src/lib.rs:2534`

Proof:

- Both paths load prepared invocation artifacts, load submissions, resolve runtime,
  poll receipts, load receipt artifacts, compute finality, and map receipt/confirmation
  through domain state.

Safe reduction:

- Share mechanics up to the point where domain-specific evidence/output mapping begins.

Risk and verification:

- Medium.
- Run receipt-level and finalized-level lifecycle tests.

Impact:

- About 40-80 LOC.

### E5. Keep adapter-owned anchor reconstruction

Target:

- `crates/adapters/evm-contracts/src/lib.rs:2176`
- `crates/adapters/evm-contracts/src/lib.rs:2396`

Proof:

- The RFC requires reconciliation by the recorded submission anchor.

Safe reduction:

- Do not remove anchor reconstruction while abstracting deploy/configure mechanics.

Risk and verification:

- This is a guardrail.

Impact:

- None directly.

## Tests and Fixtures

### T1. Share duplicated CLI/REST status helpers

Target:

- `tests/integration/tests/rest_api_run_control.rs:916`
- `tests/integration/tests/rest_api_run_control.rs:981`
- `tests/integration/tests/rest_api_run_control.rs:1008`
- `tests/integration/tests/rest_api_run_control.rs:1024`
- `bin/cli/tests/status_contract_postgres.rs:371`
- `bin/cli/tests/status_contract_postgres.rs:436`
- `bin/cli/tests/status_contract_postgres.rs:463`
- `bin/cli/tests/status_contract_postgres.rs:479`

Proof:

- `append_interrupted_attempt`, `append_run_commit`, `test_bundle_from_plan`, and
  `fixed_attempt_id` are duplicated across CLI and REST status tests.

Safe reduction:

- Move fixture mechanics to shared test support.
- Keep assertion intent in each test.

Risk and verification:

- Low-medium because one side is CLI/Postgres and one side is REST/in-memory.
- Run both focused test targets.

Impact:

- About 180-250 LOC.

### T2. Reuse HTTP request/response helpers in integration tests

Target:

- `tests/integration/tests/rest_api_run_control.rs:23`
- `tests/integration/tests/support/mod.rs:98`
- `tests/integration/tests/support/mod.rs:108`
- `tests/integration/tests/support/mod.rs:143`
- `tests/integration/tests/parity_evm_contract_lifecycle_reth.rs:649`
- `tests/integration/tests/portfolio_snapshot_local.rs:241`

Proof:

- `json_post`, `empty_post`, and `response_json` are implemented repeatedly.
- Shared support already contains equivalent helpers.

Safe reduction:

- Make the support helpers visible to the tests that duplicate them.

Risk and verification:

- Low.
- Preserve env restore/locking behavior in tests that spawn subprocesses.

Impact:

- About 80-140 LOC.

### T3. Stop hand-rolling side-effect verify lowering in runtime tests

Target:

- `crates/kernel/runtime/src/tests.rs:12422`
- `crates/kernel/runtime/src/tests.rs:12521`
- `crates/kernel/runtime/src/tests.rs:12668`
- `crates/kernel/program/src/lib.rs:4146`
- `crates/kernel/certify/src/framework_lifecycle.rs:127`

Proof:

- Runtime tests rebuild verify nodes, descriptors, cells, lineages, and consumer rewrites.
- Program lowering and certify validation already define the production behavior.
- The RFC says verify lowering is hash-defining certified spec behavior.

Safe reduction:

- Use certified specs or a shared fixture that calls production lowering.
- Keep small malformed-spec builders only for negative runtime tests.

Risk and verification:

- Medium.
- Run runtime, program, and certify side-effect tests.

Impact:

- About 250-350 LOC.

### T4. Share execution-claim lifecycle contract tests

Target:

- `crates/kernel/store/tests/commit_contract.rs:838`
- `crates/storages/stream-store-postgres/src/run_store/tests.rs:188`
- `crates/storages/stream-store-postgres/src/run_store/tests.rs:260`

Proof:

- In-memory and Postgres tests repeat acquire/busy/renew/release assertions.
- Postgres then adds SQL-specific expiry/reap behavior.

Safe reduction:

- Use one generic contract helper over `ExecutionClaimStore`.
- Keep Postgres-only expiry/reap/stale-token tests.

Risk and verification:

- Low-medium.
- Run `cargo test -p mfm-store` and focused Postgres tests with `DATABASE_URL`.

Impact:

- About 30-60 LOC.

### T5. Share sync/async side-effect prepare fixture builders

Target:

- `crates/kernel/store/tests/commit_contract.rs:2834`
- `crates/kernel/store/tests/commit_contract.rs:3022`

Proof:

- Sync and async fixture builders construct the same side-effect intent, claim, lane,
  and prepared-invocation payloads.

Safe reduction:

- Return shared payload/artifact bundles and let each caller append through its store
  specific path.

Risk and verification:

- Low.
- Re-run store contract tests.

Impact:

- About 40-70 LOC.

### T6. Compress Postgres execution-claim test setup

Target:

- `crates/storages/stream-store-postgres/src/run_store/tests.rs:188`
- `crates/storages/stream-store-postgres/src/run_store/tests.rs:375`

Proof:

- Three tests repeat acquire/busy/expire/reap setup and assertions.

Safe reduction:

- Extract token/holder setup helpers while keeping behavior assertions explicit.

Risk and verification:

- Low.
- Run focused Postgres execution claim tests.

Impact:

- About 40-80 LOC.

### T7. Macro or helper for repeated runtime test `ExecutionClaimStore` delegations

Target:

- `crates/kernel/runtime/src/tests.rs:392`
- `crates/kernel/runtime/src/tests.rs:566`
- `crates/kernel/runtime/src/tests.rs:685`
- `crates/kernel/runtime/src/tests.rs:9088`
- `crates/kernel/runtime/src/tests.rs:9219`

Proof:

- Several runtime test store wrappers implement the same delegation methods to `inner`.

Safe reduction:

- Use a local macro or helper trait for delegation in tests only.

Risk and verification:

- Low.
- Compile plus affected runtime tests.

Impact:

- About 90-120 LOC.

## Suggested Reduction Order

1. Dead-code and unused-public-shape cleanup: S4, S5, E1, E2, A1 after confirming
   the launch-outcome contract.
2. Test fixture reuse: T1 through T7. This cuts many lines while preserving behavior
   coverage.
3. Low/medium-risk code abstractions: A3, A4, E3, E4, R3, R4, R5, R6, R8.
4. Higher-risk design-shape simplifications: S1, S2, S3, R1, R2, R7, R9, R10, S6.
   These should each land with focused tests and no compatibility shims.

## Verification Matrix

Use focused Cargo checks first:

- Store changes: `cargo test -p mfm-store`; Postgres tests with explicit
  `DATABASE_URL` when SQL behavior is touched.
- Runtime/spec/certify/replay changes:
  `cargo test -p mfm-runtime -p mfm-program -p mfm-certify -p mfm-replay`.
- App/CLI/REST changes:
  `cargo test -p mfm-app`, `cargo test -p mfm --test status_contract_postgres`
  when `DATABASE_URL` is available, and `cargo test -p mfm-rest-api`.
- EVM adapter/domain changes:
  `cargo test -p mfm-state-evm-contracts -p mfm-adapters-evm-contracts`; use
  explicit Reth env only for parity coverage.
- Architecture guardrails:
  `cargo test -p mfm-integration-tests --test cargo_metadata_contract` and
  `cargo test -p mfm-integration-tests --test architecture_namespace_contract`.
