# Plan: Implement RFC_REFACTOR_FSM_SCHEDULER

Status: implementation reviewed; merge readiness has known follow-up items

This file is now a closeout record for `RFC_REFACTOR_FSM_SCHEDULER.md`. The original commit-by-commit
plan was useful while the branch was under construction, but it is no longer the contract reviewers
should read to understand the final implementation. The authoritative runtime contract is
`docs/design.md`; the contributor-facing placement contract is `docs/architecture.md`; detailed
requirement traceability is in `docs/validation/fsm-scheduler-rfc-traceability.md`.

## Implemented Slices

| Slice | Final branch behavior | Primary evidence |
|---|---|---|
| Admission | `RunAdmitted` is the single run-root event. Admission verifies launch material and bound executable evidence before the FSM scheduler can drive attempts. There is no synthetic certified admission node and no admission attempt evidence. | `crates/kernel/runtime/src/admission.rs`, `crates/kernel/runtime/src/binding.rs`, `crates/kernel/runtime/src/history.rs`, `docs/design.md` |
| Verified context loading | Scheduler drive paths load `VerifiedRunContext` through `VerifiedRunContextLoader`, combining committed stream/view authority with `BoundRuntimeContext` runner, capability, and framework-handler authority. Resume compares live bound runner executable identities against the evidence admitted at run start. | `crates/kernel/runtime/src/history.rs`, `crates/kernel/runtime/src/scheduler.rs`, runtime tests |
| Stream-model guard | `ProjectionSnapshot::validate_run_stream` is the centralized ingress guard for unsupported old lifecycle streams. Runtime, replay, and Postgres-backed loads reject attempt-bound terminal payloads that are not preceded by a separate `StateAttemptStarted` commit. | `crates/kernel/store/src/lib.rs`, `crates/kernel/replay/src/lib.rs`, `crates/storages/stream-store-postgres/src/typed.rs` |
| Transition lifecycle | `TransitionDecision` is explicit for starting attempts, continuing open attempts, remediation, manual waits, saga terminal resolution, and blocked status. Public-output, retention, and completion work are framework node selections; an already-projected public output is a scheduler facade status, not a frontier decision. | `crates/kernel/runtime/src/transition.rs`, `crates/kernel/runtime/src/scheduler.rs`, `docs/design.md` |
| Attempt lifecycle | Ordinary and framework attempts append `StateAttemptStarted` before sealed invocation construction. Post-start materialization, runner-output validation, and runtime validation failures terminalize into redacted diagnostic evidence when no side-effect authority has been acquired. | `crates/kernel/runtime/src/attempt.rs`, `crates/kernel/runtime/src/framework_lifecycle.rs` |
| Recovery lifecycle | Open-attempt recovery classifies continue, retry terminalization, interruption, side-effect recovery delegation, and operational block dispositions outside the scheduler facade. | `crates/kernel/runtime/src/recovery.rs`, architecture namespace tests |
| Side-effect lifecycle | Standalone interruption is phase-aware in runtime and store validation: intent-persisted and claimed phases may be interrupted; no-projection open side-effect attempts continue the same attempt; `SideEffectInvocationPrepared` and later phases are owned by side-effect recovery. | `crates/kernel/runtime/src/side_effect_lifecycle.rs`, `crates/kernel/store/src/v1/projection.rs`, `crates/kernel/store/tests/commit_contract.rs` |
| Saga/manual proof authority | Manual-resolution and saga terminal proofs bind the current prefix sequence and digest. Runtime rebuilds proof authority from current verified history; store validation rejects stale proof/request prefixes. | `crates/kernel/runtime/src/manual_resolution.rs`, `crates/kernel/runtime/src/framework_lifecycle.rs`, `crates/kernel/store/src/lib.rs` |
| Postgres projection authority | PostgreSQL loads/rebuilds the selected run snapshot from authoritative event rows and rebuilds active global resource lanes from authoritative run streams instead of trusting persisted projection rows. | `crates/storages/stream-store-postgres/src/typed.rs` |
| Public status | Public status separates `run_mode`, attempt dispositions, and `scheduler_status`. Resource-lane status is filtered to lane evidence referenced by the target run's persisted live side-effect ledgers; no-append scheduler waiters are not public status yet. | `crates/app/src/lib.rs`, `bin/cli/README.md`, `bin/rest-api/README.md` |

## Completed Nuances

- The `RunnerOutputPlanner` responsibility from the RFC is implemented by
  `CommitPlanner::prepare_runner_output` plus framework lifecycle planning, not by a separate
  `runner_output.rs` module.
- Storage and artifact-authority outages before durable terminal evidence commits remain
  recovery/ingress failures and leave started attempts open; they are not converted into semantic
  failure events.
- Open-attempt recovery has an explicit operational-block disposition for cases such as terminal
  side-effect ledger evidence without matching attempt-terminal evidence. Normal store/history
  validation rejects those malformed streams before scheduler recovery.
- A no-projection open side-effect attempt has not acquired ledger evidence, so recovery continues the
  same attempt; intent-persisted and claimed phases remain eligible for ordinary attempt
  interruption.
- Sync and async attempt lifecycles now precheck resource lanes against store-owned status projection
  before staging runner artifacts.
- The sync and async driver loops still duplicate some IO choreography. Shared lifecycle authority
  lives in pure helpers; full driver collapse is deferred.

## Verification Evidence

Recorded closeout evidence lives in `docs/validation/fsm-scheduler-closeout-2026-06-18.md` and
`docs/validation/fsm-scheduler-closeout-2026-06-18-run-summary.json`. Requirement-by-requirement
evidence and reviewer findings live in `docs/validation/fsm-scheduler-rfc-traceability.md`.

Focused checks that should remain green before merge:

```bash
cargo fmt --all -- --check
cargo check --workspace --all-targets --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test -p mfm-runtime
cargo test -p mfm-store --test commit_contract
cargo test -p mfm-replay
cargo test -p mfm-app
cargo test -p mfm-integration-tests --test cargo_metadata_contract
cargo test -p mfm-integration-tests --test architecture_namespace_contract
cargo test -p mfm-integration-tests --test rest_api_run_control
cargo test -p mfm --test json_output_integration test_run_status_json_contract_exposes_attempt_dispositions_and_saga_blocks
```

Service-backed checks before merge:

```bash
NIXFIED_STATE_DIR="$(mktemp -d)" nix -L -v --log-format bar-with-logs run .#ci
```

If not relying on Nixfied-managed services, run the parity targets with explicit service
environment variables such as `DATABASE_URL`, `RETH_HTTP_PORT`, and `MFM_EVM_RPC_SOURCES_JSON`.

## Remaining Merge-Readiness Items

- The current EVM contract runner binding and side-effect runner behavior still live in
  `crates/app/src/evm_contracts.rs`. That conflicts with the architecture placement rule that app
  assembly wires adapters but does not own runner behavior. Before merge, either move that runner
  layer into `crates/adapters/evm-contracts` or deliberately update the architecture contract with
  the new boundary and tests that enforce it.
- Typed-core hardening remains broader than this scheduler RFC: persisted/public no-secret checks
  and manual persisted-surface trait provenance need a separate focused hardening pass if the branch
  is expected to make those guarantees mechanically complete.

## Deferred Scope

- Public no-append resource-lane waiter status is not implemented. Status exposes persisted live
  lane evidence, not scheduler waiters that blocked before appending lane evidence.
- Dedicated public CLI/REST/app fixture coverage for `ResolveSagaTerminal` remains deferred to a
  public-surface follow-up.
- Full sync/async driver collapse is deferred to a later async-primary cleanup.
