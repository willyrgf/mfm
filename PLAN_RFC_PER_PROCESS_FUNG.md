# Implementation Plan: RFC Per-Process Fungibility

Status: planning artifact for engineering execution
Source RFC: `RFC_PER_PROCESS_FUNGIBILITY.md`
Architect review: completed with a read-only architect-agent pass

## Ground Rules

This plan assumes the RFC is the implementation target and that the repository is still allowed to
make breaking changes. Engineers should not preserve old behavior behind flags, compatibility
shims, fallback branches, or alternate legacy paths. If a shape is replaced, delete the old shape in
the same commit that lands the new one.

Non-negotiable constraints:

- No backward compatibility work.
- No dual old/new behavior.
- No fallback to random run ids on normal launch.
- No timeout release of ambiguous side-effect lanes.
- No feed-driven dispatch in v1.
- No mutable registry, oracle, or worker-local policy lookup during admission, drive, verify, or
  replay.
- No raw signed transaction persistence.
- Tests and docs move with the code that changes the contract.

The implementation must keep the codebase taxonomy intact:

- typed contracts in `crates/kernel/*`
- Postgres SQL and schema checks in `crates/storages/stream-store-postgres`
- app assembly in `crates/app`
- CLI/REST request and response surfaces in `bin/cli` and `bin/rest-api`
- domain side-effect semantics in state crates
- protocol IO and signer binding in adapters/transports/signers

## Architect Alignment

The architect-agent reviewed the RFC plus `docs/design.md`, `docs/architecture.md`,
`docs/saga.md`, and `docs/code-quality.md`. The implementation plan below incorporates that review.

Agreed architectural points:

- Certified typed specs plus append-only run streams remain semantic authority.
- Process identity never becomes authority.
- Postgres transaction boundaries, per-run CAS, and side-effect fencing remain the safety floor.
- Leases are liveness only.
- Process fungibility is executable-scoped: same run identity plus different executables attaches
  and reports, but does not fork or drive.
- v1 is invoker-driven and manual-resumable. Automatic takeover is deferred.
- The observation feed remains status/watch only.
- `RunIdentityMaterialV1` replaces random default run ids.
- Admission lanes are operational coordination, not authority.
- Resource lanes are single-lane in v1.
- Side-effect verification is a real submit/verify pair.
- The one-ledger `SideEffectPair` redesign is the chosen end state.
- The verify node owns the downstream-visible output cell.
- The submission anchor is adapter/prepared-invocation evidence, not state-owned semantics.
- Receipt verification is final-at-risk by explicit op design choice.

## Decisions To Ratify Before Code

These decisions should be committed first as design-contract updates and then treated as fixed
engineering inputs. Do not start the broad refactors until these are resolved.

1. Deployment trust scope.
   Recommended decision: add `trust_scope_id` to `store_metadata`, initialized by the Postgres
   baseline migration as `mfm.trust_scope.v1:<random-hex>`, protected by the existing no-update
   trigger, and read through a typed store contract. It is store-owned, never caller-controlled, and
   enters `RunIdentityMaterialV1`.

2. Migration posture.
   Recommended decision: rewrite the current baseline migration and schema checks destructively.
   Delete old waiter tables and old event/spec shapes. Local databases must be reset. Do not add
   compatibility migrations that preserve old rows.

3. Finality authority.
   Recommended decision: finality lives only in hash-defining `SideEffectContractSpec` config.
   `RunAdmitted` may record launch audit evidence, but it must not become a second finality policy
   authority.

4. Canonical identity material.
   Recommended decision: define exact canonical domains and bytes for:
   `RunIdentityMaterialV1`, `distinct_run_key_digest`, admission lane fingerprints,
   execution-claim tokens, and `SideEffectPair` ids. Add tests in the same commit that freeze the
   bytes.

5. Execution claim defaults.
   Recommended decision: choose a concrete v1 TTL and heartbeat interval, define renewal failure as
   an operational stop/report, and define attach/report response shapes. A reasonable starting point
   is TTL 60s and heartbeat every 20s, but the commit must make this explicit in code and docs.

6. Short receipt waits.
   Recommended decision: the invoker loop heartbeats while the scheduler has runnable submit/verify
   work and short receipt polling is active. Long finalized waits, `due_at`, and tenure release on
   wait remain deferred.

7. `NotSubmittedProven` threshold.
   Recommended decision: never emit it from one transient RPC miss. v1 should terminalize only from
   explicit verifier evidence that the adapter can defend with tests; otherwise leave the ledger
   ambiguous or manual-blocked according to certified policy.

8. Receipt output shape.
   Recommended decision: add a domain-owned `output_from_receipt` contract beside
   `output_from_confirmation`, then update EVM state outputs explicitly. Runtime invokes the domain
   contract and never constructs domain output itself.

9. WS-E scope.
   Recommended decision: lane-state materialization is an optimization wave after the functional RFC
   lands. It is not required for initial ratification unless resource-lane folds become a measured
   bottleneck.

## Commit Stack

The commits are intentionally ordered so each commit has one clear outcome. Every commit below should
be reviewable on its own. Commit subjects are lower case to match repository convention.

### Commit 1: `docs: ratify per-process fungibility implementation defaults`

Purpose:

- Convert the decisions above into explicit implementation defaults.
- Narrow `docs/saga.md` from multi-lane claims to the actual single-lane v1 contract.
- State that automatic takeover, `due_at`, long-wait tenure release, pipelined nonces, and
  multi-lane admission are deferred.
- State that `RunAdmitted` does not carry independent finality authority.
- Remove or resolve the RFC's vague "D5-D11 defaults" note.

Files:

- `RFC_PER_PROCESS_FUNGIBILITY.md`
- `docs/design.md`
- `docs/saga.md`
- `docs/architecture.md` if placement text needs tightening

Delete:

- Any wording that implies multi-lane v1 atomicity.
- Any wording that implies registry-resolved finality at admission.
- Any wording that implies automatic v1 dead-driver takeover.

Verification:

- `rg -n "every requested lane|multi-lane|registry-resolved|automatic takeover|worker pool" docs RFC_PER_PROCESS_FUNGIBILITY.md`
- `cargo test -p mfm-integration-tests --test architecture_namespace_contract` if available.

### Commit 2: `store: add admission lane contract`

Purpose:

- Add the typed admission-lane contract to `mfm-store`.
- Represent lane class, lane id, mode, token, lease, waiter, and admission result.
- Make mode-by-class unrepresentable where practical:
  resource lanes use `WaitFifo`; execution claims use `NowaitSkip`.
- Define typed fingerprint helpers and 64-bit advisory-lock key derivation inputs.

Files:

- `crates/kernel/store/src/lib.rs`
- `crates/kernel/store/src/v1/*`
- `crates/kernel/store/tests/commit_contract.rs`

Delete:

- Any public contract that exposes resource-lane waiters as a resource-lane-specific primitive.

Verification:

- `cargo test -p mfm-store`
- Compile-fail or unit tests proving invalid mode/class combinations cannot be built.

### Commit 3: `postgres: replace resource waiters with admission lanes`

Purpose:

- Rewrite the Postgres baseline schema from `resource_lane_waiter_counters` and
  `resource_lane_waiters` to `admission_lane` and `admission_waiter`.
- Port existing resource-lane FIFO behavior to the generic `WaitFifo` implementation.
- Keep resource-lane holders event-derived. The admission tables remain operational only.
- Use the 64-bit `pg_advisory_xact_lock(int8)` form or equivalent full-lane fingerprint locking.
- Update schema validation and SQLx query metadata.

Files:

- `crates/storages/stream-store-postgres/migrations/0001_run_store.sql`
- `crates/storages/stream-store-postgres/src/schema.rs`
- `crates/storages/stream-store-postgres/src/run_store/resource_lanes.rs`
- new Postgres admission-lane module if useful
- `docs/persisted-public-surfaces.md`

Delete:

- `resource_lane_waiter_counters`
- `resource_lane_waiters`
- resource-lane-specific waiter allocation helpers after porting

Verification:

- `cargo test -p mfm-stream-store-postgres`
- With `DATABASE_URL`: focused Postgres resource-lane FIFO tests.
- `rg -n "resource_lane_waiter|resource_lane_waiters|resource_lane_waiter_counters" crates docs`
  should only find intentional historical notes, or none if docs are fully updated.

### Commit 4: `postgres: add execution claim lanes`

Purpose:

- Implement `NowaitSkip` admission over the same Postgres primitive.
- Key execution claim lanes by derived run id.
- Add acquire, renew, release, expired-claim read, and explicit stale-claim reaping APIs.
- Do not add background sweeping or automatic resume.

Files:

- `crates/kernel/store/src/v1/*`
- `crates/storages/stream-store-postgres/src/run_store/*`
- `crates/storages/stream-store-postgres/src/schema.rs`

Delete:

- Any placeholder execution-owner concept if introduced during development.

Verification:

- `cargo test -p mfm-store`
- `cargo test -p mfm-stream-store-postgres`
- Focused tests for busy, renew, release, expiry, and false-reap safety assumptions.

### Commit 5: `store: persist deployment trust scope`

Purpose:

- Add store-owned `trust_scope_id`.
- Load it through a typed store/app contract.
- Ensure callers cannot supply it on launch.
- Treat it as non-secret identity material in persisted/public surface docs.

Files:

- `crates/storages/stream-store-postgres/migrations/0001_run_store.sql`
- `crates/storages/stream-store-postgres/src/schema.rs`
- `crates/storages/stream-store-postgres/src/run_store/observations.rs` or a new metadata module
- `crates/kernel/store/src/v1/*`
- `docs/persisted-public-surfaces.md`

Delete:

- Any process-local or CLI-supplied trust-scope input.

Verification:

- `cargo test -p mfm-store`
- `cargo test -p mfm-stream-store-postgres`
- Test that the trust scope survives store reconnects and cannot be changed by runtime credentials.

### Commit 6: `events: record run identity material`

Purpose:

- Add `RunIdentityMaterialV1` to typed event/spec authority.
- Record identity material in `RunAdmitted`.
- Validate `run_id == sha256-jcs-v1(identity_material)` during admission, attach, resume, replay,
  status, and public-output authority construction.
- Fail closed on identity mismatch.

Files:

- `crates/kernel/events/src/lib.rs`
- `crates/kernel/spec/src/lib.rs` if reusable identity structs belong there
- `crates/kernel/runtime/src/commit.rs`
- `crates/kernel/runtime/src/history.rs`
- `crates/kernel/replay/src/lib.rs`
- `crates/app/src/lib.rs`

Delete:

- Any normal launch path that can admit a run without identity material.

Verification:

- `cargo test -p mfm-events`
- `cargo test -p mfm-runtime`
- `cargo test -p mfm-replay`
- Tests for hash mismatch, missing identity material, and identity collision/corruption.

### Commit 7: `app: derive launch run ids from certified specs`

Purpose:

- Remove `mfm_app::new_run_id()` from normal start flows.
- Derive run id only after planning and certification, from:
  `certified_spec_hash`, store `trust_scope_id`, and optional `distinct_run_key_digest`.
- Add `--distinct-run-key` and REST equivalent.
- Remove normal `--run-id` from start. If raw run ids remain, put them behind explicit
  admin/import/test-only APIs, not public normal launch.
- Add typed launch outcomes:
  `Admitted`, `Attached`, `AlreadyDriving`, `IncompatibleExecutable`, `IdentityMismatch`.

Files:

- `crates/app/src/lib.rs`
- `crates/app/src/entry_points.rs`
- `bin/cli/src/commands/run/start.rs`
- `bin/rest-api/src/lib.rs`
- `bin/cli/README.md`
- `bin/rest-api/README.md`

Delete:

- `mfm_app::new_run_id()` if no test/admin-only use remains.
- Public `run start --run-id`.
- REST normal-start `run_id`.
- Tests that expect random launch ids.

Verification:

- `cargo test -p mfm-app`
- `cargo test -p mfm`
- `cargo test -p mfm-rest-api`
- Tests:
  same certified spec + same trust scope -> same run id
  distinct key -> different run id
  raw key is not persisted
  changed config/finality -> different certified spec and different run id

### Commit 8: `runtime: attach duplicate launches without driving`

Purpose:

- Implement try-admit then attach-on-existing semantics.
- If existing identity material mismatches, fail closed.
- If executable bindings mismatch, return `IncompatibleExecutable` and do not drive.
- If a live execution claim exists, return `AlreadyDriving`/attached status.
- Keep compatible same-build duplicates idempotent.

Files:

- `crates/app/src/lib.rs`
- `crates/kernel/runtime/src/binding.rs`
- `crates/kernel/runtime/src/history.rs`
- `crates/kernel/runtime/src/commit.rs`
- CLI/REST response types

Delete:

- Any duplicate-launch path that silently creates another random run.

Verification:

- `cargo test -p mfm-runtime`
- `cargo test -p mfm-app`
- Concurrent duplicate-launch tests with N launchers.
- Same identity with different executable binding attaches/reports and never creates a second run.

### Commit 9: `runtime: drive through execution claims`

Purpose:

- Add the claim-backed invoker drive loop.
- Acquire execution claim before driving.
- Heartbeat while driving.
- Release on terminal.
- Stop/report on renewal failure.
- Keep manual `run resume <run_id>` as the v1 recovery trigger.
- Do not dispatch from observation feed.

Files:

- `crates/app/src/lib.rs`
- `crates/kernel/runtime/src/frontier.rs`
- `crates/kernel/runtime/src/scheduler.rs`
- `crates/kernel/runtime/src/recovery.rs`
- CLI/REST response status types if needed

Delete:

- Any new or old worker-pool/observation-feed dispatch experiment.

Verification:

- `cargo test -p mfm-runtime`
- `cargo test -p mfm-app`
- Tests for heartbeat, release, busy attach, renewal failure report, and manual resume after stale
  claim.

### Commit 10: `spec: add side effect verification policy`

Purpose:

- Add `verification: Receipt | Finalized(depth)` to `SideEffectContractSpec`.
- Make it hash-defining certified config.
- Ensure no registry lookup resolves finality at admission or runtime.
- Add terminal-evidence model types.
- Add `output_from_receipt` beside `output_from_confirmation`.

Files:

- `crates/kernel/spec/src/lib.rs`
- `crates/kernel/certify/src/lib.rs`
- `crates/kernel/program/src/lib.rs`
- `crates/states/evm-contracts/src/lib.rs`
- `crates/collectors/proof/src/lib.rs`
- `crates/transports/proof/src/lib.rs` if proof side effects need test adaptation

Delete:

- Any unconditional confirmation-only terminal-output assumption.

Verification:

- `cargo test -p mfm-spec`
- `cargo test -p mfm-certify`
- `cargo test -p mfm-program`
- `cargo test -p mfm-state-evm-contracts`
- Tests proving finality depth changes the certified spec hash.

### Commit 11: `program: lower side effects into verify pairs`

Purpose:

- Add `FrameworkNodeSpec::SideEffectVerify`.
- Lower every side-effect node into a submit node plus verify framework node.
- Mint deterministic certified `SideEffectPair` ids.
- Re-point the original output cell producer to verify.
- Ensure submit binds no downstream-visible output.
- Add certifier pairing validation:
  exactly one verify per submit, no orphan verify, no submit without verify, evidence-flow binding,
  and post-rewrite single-producer checks.

Files:

- `crates/kernel/spec/src/lib.rs`
- `crates/kernel/program/src/lib.rs`
- `crates/kernel/certify/src/framework_lifecycle.rs`
- `crates/kernel/certify/src/node_contract.rs`
- tests under `crates/kernel/program` and `crates/kernel/certify`

Delete:

- Any lowering/certification path that allows a side-effect node without a verify pair.

Verification:

- `cargo test -p mfm-program`
- `cargo test -p mfm-certify`
- Compile-fail/golden tests for missing verify, orphan verify, duplicate verify, and bad output
  producer.

### Commit 12: `store: rekey side effect ledgers by pair`

Purpose:

- Replace node/attempt-shaped side-effect ledger authority with certified pair authority.
- Make event payload attribution explicit:
  `(pair_id, role: submit|verify)` plus the phase-specific authority.
- Move projection lookup to pair ledger key.
- Bind resource-lane claim to submit role and release to verify role.
- Retarget remediation linkage to pair identity.
- Treat the forward fence atomically over the pair's terminal phase.

Files:

- `crates/kernel/events/src/lib.rs`
- `crates/kernel/store/src/v1/*`
- `crates/kernel/runtime/src/side_effect_driver.rs`
- `crates/kernel/runtime/src/side_effect_lifecycle.rs`
- `crates/kernel/runtime/src/side_effects.rs`
- `crates/storages/stream-store-postgres/src/run_store/*`

Delete:

- Ledger-key derivation from `node_id + attempt_id`.
- Store admission rules that treat submit and verify as unrelated ledgers.
- Resource-lane release validation keyed to the old single node/attempt shape.

Verification:

- `cargo test -p mfm-events`
- `cargo test -p mfm-store`
- `cargo test -p mfm-runtime`
- `cargo test -p mfm-replay`
- Tests for pair-bound claim/release, remediation targeting, and forward-fence behavior.

### Commit 13: `runtime: execute side effect verify nodes`

Purpose:

- Make submit role stop at submitted/unknown/not-submitted evidence.
- Make verify role poll and terminalize to the certified verification level.
- Invoke domain `output_from_receipt` or `output_from_confirmation`.
- Bind the verify-owned output cell.
- Release the lane only at proven terminal evidence or certified manual-resolution terminal.
- Preserve replay from recorded evidence only.

Files:

- `crates/kernel/runtime/src/framework.rs`
- `crates/kernel/runtime/src/framework_lifecycle.rs`
- `crates/kernel/runtime/src/side_effect_driver.rs`
- `crates/kernel/runtime/src/side_effect_lifecycle.rs`
- `crates/kernel/replay/src/lib.rs`
- `crates/adapter-contracts/src/*`

Delete:

- Any runtime code that constructs EVM/domain output directly.
- Any unconditional `is_confirmed()` requirement for all side-effect outputs.

Verification:

- `cargo test -p mfm-runtime`
- `cargo test -p mfm-replay`
- Kill-mid-submit and kill-mid-verify unit tests.
- Replay test proving verify evidence uses no live IO.

### Commit 14: `evm: record submission anchors`

Purpose:

- Record the deterministic signed transaction hash in the prepared-invocation artifact before
  broadcast.
- Reconstruct the anchor from committed prepared-invocation artifacts during resume.
- Keep raw signed transaction bytes transient below the semantic boundary.
- Keep state-owned `IdempotencyInput` separate from adapter-owned submission anchor.

Files:

- `crates/adapters/evm-contracts/src/lib.rs`
- `crates/transports/evm/src/lib.rs`
- `crates/states/evm-contracts/src/lib.rs` for any trait shape changes, not signing
- `docs/persisted-public-surfaces.md`

Delete:

- Any assumption that nonce alone is the recovery anchor.
- Any attempted state-layer signing/hash computation.

Verification:

- `cargo test -p mfm-adapters-evm-contracts`
- `cargo test -p mfm-transports-evm`
- Tests that prepared-invocation reconstruction yields the same expected hash.
- Redaction tests proving raw signed tx bytes are not persisted.

### Commit 15: `evm: reconcile before resubmission`

Purpose:

- Delete the current bare always-`Observed` recovery behavior.
- Add live `SideEffectVerifier` read capability for EVM.
- Reconcile chain truth by recorded submission anchor before any rebroadcast.
- Enforce re-sign equals recorded anchor before rebroadcast.
- Treat signer mismatch as a defined terminal path or manual block according to certified policy.
- Emit `NotSubmittedProven` only from defended investigation evidence.

Files:

- `crates/adapter-contracts/src/*`
- `crates/adapters/evm-contracts/src/lib.rs`
- `crates/transports/evm/src/lib.rs`
- `crates/kernel/runtime/src/side_effect_driver.rs`

Delete:

- `submit_or_recover_submission` behavior that blindly rebroadcasts and returns observed.
- Any transient-RPC-miss terminalization.

Verification:

- `cargo test -p mfm-adapters-evm-contracts`
- `cargo test -p mfm-transports-evm`
- With Reth: focused parity tests using explicit `RETH_HTTP_PORT`.
- Tests:
  landed tx confirms by anchor
  unlanded tx can rebroadcast only after re-sign equality
  foreign nonce occupancy does not double-submit
  transient RPC failure does not release the lane

### Commit 16: `replay: verify side effect pairs from evidence`

Purpose:

- Update replay authority for pair-keyed ledgers.
- Verify receipt/confirmation/not-submitted evidence from recorded facts only.
- Enforce no live registry, transport, signer, keystore, environment, or mutable oracle access.
- Add regression coverage for DEC-39.

Files:

- `crates/kernel/replay/src/lib.rs`
- `crates/kernel/replay/tests/*`
- `crates/kernel/runtime/src/history.rs`

Delete:

- Any replay path that can construct live capabilities for side-effect verification.

Verification:

- `cargo test -p mfm-replay`
- `cargo test -p mfm-runtime`
- Boundary tests that fail if replay/drive/verify depends on live registry/oracle after
  certification.

### Commit 17: `docs: update run and side effect public contracts`

Purpose:

- Update public docs to match implemented behavior.
- Document derived default run identity and `--distinct-run-key`.
- Remove normal `--run-id` start documentation.
- Document attach/report outcomes.
- Document manual resume as v1 recovery trigger.
- Document receipt final-at-risk behavior.
- Document single-lane admission and admission tables as operational coordination only.

Files:

- `bin/cli/README.md`
- `bin/rest-api/README.md`
- `docs/design.md`
- `docs/saga.md`
- `docs/persisted-public-surfaces.md`
- `README.md` only if user-facing summary needs adjustment

Delete:

- Stale random-id, confirmation-only, multi-lane, old waiter-table, and automatic-recovery language.

Verification:

- `rg -n "random UUID|new typed digest id|--run-id|resource_lane_waiters|confirmation-only|worker pool|automatic takeover" README.md docs bin`
- `cargo test -p mfm -p mfm-rest-api`

### Commit 18: `tests: add process fungibility integration coverage`

Purpose:

- Add cross-crate integration tests that exercise the RFC as a user-visible behavior set.
- Keep these tests focused on externally important contracts rather than implementation details.

Coverage:

- N concurrent launchers of identical certified work converge on one run.
- Duplicate launch with live driver attaches/reports.
- Same identity with incompatible executable binding reports and does not drive.
- Distinct-run key forces a separate run.
- Manual resume after a stale execution claim can continue a run.
- Side-effect lane remains held while ambiguous and releases at proven terminal.
- Receipt-level terminalization releases at receipt and is documented as final-at-risk.
- Replay of verify evidence uses recorded evidence only.

Files:

- `crates/app/tests/*`
- `bin/cli/tests/*`
- `crates/storages/stream-store-postgres/src/run_store/tests.rs`
- `crates/kernel/runtime/src/tests.rs`
- `crates/kernel/replay/tests/*`
- `tests/integration/*`

Verification:

- `cargo test --workspace`
- `cargo test -p mfm-integration-tests --test cargo_metadata_contract`
- `cargo test -p mfm-integration-tests --test architecture_namespace_contract`
- Use manually started Postgres/Reth for parity tests that need them.

### Commit 19: `ops: materialize lane state projection`

Purpose:

- Optional post-core optimization from WS-E.
- Materialize an event-derived lane-state projection to remove O(history) folds where measured.
- Keep it rebuildable cache, never authority.

Files:

- `crates/kernel/store/src/v1/*`
- `crates/storages/stream-store-postgres/migrations/0001_run_store.sql`
- `crates/storages/stream-store-postgres/src/run_store/*`
- `docs/persisted-public-surfaces.md`

Delete:

- Any code path that treats materialized projection as lane ownership authority.

Verification:

- `cargo test -p mfm-store`
- `cargo test -p mfm-stream-store-postgres`
- Rebuild projection from stream and compare to materialized rows.

This commit is not required before the functional RFC stack lands unless performance measurements
show the existing fold is blocking engineering progress.

## Recommended Execution Phases

Phase 0: ratify decisions.

- Commit 1.

Phase 1: admission and identity foundation.

- Commits 2 through 9.
- This phase should finish with content-addressed run identity, attach-on-existing behavior, and
  active-driver execution claims, but without side-effect pair refactoring.

Phase 2: side-effect pair authority.

- Commits 10 through 13.
- This is the largest blast radius. Do not start it until Phase 1 is green.

Phase 3: EVM verification and reconciliation.

- Commits 14 through 16.
- This phase makes manual resume safe for ambiguous EVM submissions.

Phase 4: public contracts and integration coverage.

- Commits 17 and 18.

Phase 5: optional performance work.

- Commit 19 only if needed.

## Final Verification Gate

Use focused Cargo checks during the stack. Before declaring the RFC implementation merge-ready:

```bash
cargo fmt --all -- --check
cargo check --workspace
cargo test --workspace
cargo test -p mfm-integration-tests --test cargo_metadata_contract
cargo test -p mfm-integration-tests --test architecture_namespace_contract
```

For live-service parity:

- Start Postgres manually and run focused Postgres tests with `DATABASE_URL`.
- Start Reth manually and run focused EVM parity tests with `RETH_HTTP_PORT` or
  `MFM_EVM_RPC_SOURCES_JSON`.
- Use `nix run .#test-db` or `nix run .#ci` only when final managed validation is explicitly needed
  or when the change is about Nixfied behavior.

## Engineering Review Checklist

Before each commit is reviewed:

- Does it delete the old path it replaces?
- Does it avoid compatibility switches and fallback behavior?
- Are docs and persisted/public surface inventories updated in the same commit?
- Are new public items documented?
- Are secrets absent from events, artifacts, public output, diagnostics, fixtures, and logs?
- Does the implementation preserve crate boundaries from `docs/architecture.md`?
- Does replay avoid live IO, signer, registry, keystore, environment, and mutable oracle access?
- Does any lease affect only liveness, never semantic safety?
- Does any side-effect lane release only from proven terminal evidence or certified manual
  resolution?
- Does every new persisted shape have a focused test and a deletion of the old shape?
