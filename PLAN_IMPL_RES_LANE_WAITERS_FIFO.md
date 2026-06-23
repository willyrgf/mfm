# Resource Lane FIFO Waiters Implementation Plan

> **For Hermes:** Use subagent-driven-development skill to implement this plan task-by-task.

**Goal:** Add minimal FIFO fairness for single-lane exclusive resource-lane claims across independent `mfm run --op ...` processes.

**Architecture:** Add a durable, mutable, Postgres-backed per-lane waiter queue that is enforced inside resource-lane claim admission under the existing per-lane advisory transaction lock. The waiter queue is operational admission state only; `ResourceLaneClaimed`, `ResourceLaneReleased`, and `resource_lane_transitions` remain the only semantic lane ownership/replay authority. No queue-position or cosmetic response metadata is required.

**Tech Stack:** Rust, Cargo workspace, `sqlx`, PostgreSQL, existing MFM typed runtime/store/app/CLI crates.

---

## Scope

Implement FIFO fairness for the current practical case: a claim commit that attempts to acquire exactly one exclusive resource lane.

Do not implement:

- Durable FIFO fairness for multi-lane atomic claims.
- Queue-position hints, approximate queue position, or other cosmetic UX metadata.
- Domain/run-stream `ResourceLaneBlocked` events.
- A central dispatcher/daemon requirement.
- Ownership grants outside normal `ResourceLaneClaimed` admission.

## Contract

A single-lane exclusive claim may be materialized only when:

1. no active authoritative holder exists for the lane, and
2. either no live waiter exists for the lane, or the claimant is the oldest live waiter for that lane.

A later live waiter must not bypass an earlier live waiter. A later waiter may pass an earlier waiter only when the earlier waiter has been marked `claimed`, `cancelled`, or `expired`.

`ResourceLaneClaimBlocked` must be reworded from “no rows persisted” to:

> no run event, commit, resource-lane claim, resource-lane release, lane-transition, or other MFM domain authority row was persisted; the store may have inserted or refreshed a non-authoritative operational waiter row used only for FIFO admission.

## Current code anchors

- Store API and resource lane block type: `crates/kernel/store/src/lib.rs:2187`, `crates/kernel/store/src/lib.rs:2202`, `crates/kernel/store/src/lib.rs:5060`.
- Pure staging currently returns blocked on active holder: `crates/kernel/store/src/lib.rs:5435`.
- Postgres append path already locks lanes before staging: `crates/storages/stream-store-postgres/src/run_store.rs:193`, `crates/storages/stream-store-postgres/src/run_store.rs:2299`.
- Current lane authority schema: `crates/storages/stream-store-postgres/migrations/0001_run_store.sql:160`.
- Runtime currently converts blocked claims to `BlockedOnResourceLane`: `crates/kernel/runtime/src/attempt.rs:298`, `crates/kernel/runtime/src/attempt.rs:431`.
- Runtime currently drops holder/waiter details from witness: `crates/kernel/runtime/src/attempt.rs:651`.
- Scheduler currently collapses lane block to generic `SchedulerStatus::Blocked`: `crates/kernel/runtime/src/scheduler.rs:183`, `crates/kernel/runtime/src/scheduler.rs:217`.
- CLI drive modes: `bin/cli/src/support/run_store.rs:14`, `bin/cli/src/commands/run/start.rs:39`.
- App drive modes: `crates/app/src/lib.rs:462`, `crates/app/src/lib.rs:1228`.

---

## Design details

### New operational schema

Add two mutable operational tables to the initial migration and schema validation lists.

```sql
CREATE TABLE resource_lane_waiter_counters (
  lane_id BYTEA PRIMARY KEY,
  next_ticket BIGINT NOT NULL,
  updated_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
  CONSTRAINT resource_lane_waiter_counters_lane_id_len CHECK (octet_length(lane_id) = 32),
  CONSTRAINT resource_lane_waiter_counters_next_ticket_positive CHECK (next_ticket >= 1)
);

CREATE TABLE resource_lane_waiters (
  waiter_id TEXT PRIMARY KEY,
  lane_id BYTEA NOT NULL,
  lane_ticket BIGINT NOT NULL,
  run_id TEXT NOT NULL,
  node_id TEXT NOT NULL,
  attempt_id TEXT NOT NULL,
  ledger_key TEXT NOT NULL,
  invocation_epoch INTEGER NOT NULL,
  claim_fingerprint TEXT NOT NULL,
  status TEXT NOT NULL CHECK (status IN ('waiting', 'claimed', 'cancelled', 'expired')),
  enqueued_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp(),
  lease_expires_at TIMESTAMPTZ NOT NULL,
  CONSTRAINT resource_lane_waiters_lane_id_len CHECK (octet_length(lane_id) = 32),
  CONSTRAINT resource_lane_waiters_lane_ticket_positive CHECK (lane_ticket >= 1),
  CONSTRAINT resource_lane_waiters_invocation_epoch_nonnegative CHECK (invocation_epoch >= 0),
  UNIQUE (lane_id, lane_ticket),
  UNIQUE (lane_id, claim_fingerprint)
);

CREATE INDEX resource_lane_waiters_live_fifo_idx
ON resource_lane_waiters (lane_id, status, lane_ticket)
WHERE status = 'waiting';

CREATE INDEX resource_lane_waiters_expiry_idx
ON resource_lane_waiters (status, lease_expires_at)
WHERE status = 'waiting';
```

Notes:

- These tables are mutable operational state, not append-only domain authority.
- `lane_ticket` is lane-local and assigned under the existing resource-lane advisory transaction lock.
- `claim_fingerprint` makes blocked retries idempotent.
- No `queue_position`, `queue_ahead`, `holder diagnostics`, or other cosmetic fields are required.

### Claim fingerprint

Derive a stable `claim_fingerprint` from the prepared claim identity. Include only stable, non-store-filled inputs:

- `lane_id`
- `run_id`
- `node_id`
- `attempt_id`
- `ledger_key`
- `invocation_epoch`
- `requirement_digest`
- `resolved_by_capability_impl`
- resource key identity already represented by `lane_id`

Do not include store-filled fencing token, claim id, lane transition seq, commit id, or expected next seq.

Use the repository’s existing canonical JSON/hash helpers and add tests to prove identical retries produce identical fingerprints.

### Waiter id

Derive `waiter_id` deterministically from `claim_fingerprint`, e.g. `resource_lane_waiter:<sha256...>`, rather than generating random ids. This keeps retry behavior simple and avoids extra lookup complexity.

### Lease policy

Use a conservative default lease long enough to avoid false expiry during normal local scheduling delays.

Initial constants can live in the Postgres store implementation:

```text
RESOURCE_LANE_WAITER_LEASE_SECS = 60
RESOURCE_LANE_WAITER_EXPIRE_GRACE_SECS = 0
```

The first implementation may refresh the lease only when the same blocked claim retries. If a later CLI wait loop is added, it can either retry periodically or call a targeted renewal API. Do not add a standalone lease API unless the app/CLI wait loop in this change needs it.

### FIFO admission rule

Inside Postgres `append_prepared_commit_bundle`, under the existing per-lane advisory transaction lock:

```text
expire stale waiting rows for this lane
load active authoritative lane holder
compute claim_fingerprint

if active holder exists and it is not this holder:
    enqueue_or_refresh_waiter
    commit transaction
    return ResourceLaneClaimBlocked { lane_key, holder, waiter }

head_waiter = oldest waiting row for lane ordered by lane_ticket

if head_waiter exists and head_waiter.claim_fingerprint != this claim_fingerprint:
    enqueue_or_refresh_waiter
    commit transaction
    return ResourceLaneClaimBlocked { lane_key, holder: optional/none, waiter }

if head_waiter is this claim:
    stage/materialize ResourceLaneClaimed as today
    insert normal authority rows
    mark waiter claimed in the same transaction
    commit transaction
    return Appended/Idempotent

if no head_waiter:
    stage/materialize ResourceLaneClaimed as today
    commit transaction
    return Appended/Idempotent
```

Important: The free-lane-but-not-head case has no active holder. The existing `ResourceLaneClaimBlock` currently requires `holder`; that type must change so holder is optional.

### Release behavior

Keep release authority exactly as today, with one addition:

- under the same lane lock, opportunistically expire stale waiters for that lane;
- after a successful commit, optionally emit `NOTIFY` as a wake hint in a later task.

Do not mark the next waiter as granted on release. The next waiter claims only by retrying the normal prepared claim path and passing FIFO admission.

### Pure staging vs Postgres admission

Current pure staging in `mfm_store` only sees `CommitBase`; it does not know the durable waiter queue. Keep pure staging responsible for semantic lane correctness, but add a Postgres pre-gate around claim staging.

Recommended split:

1. Postgres append path derives lane ids from claim intents and locks them as today.
2. Postgres queue gate runs before `stage_prepared_commit_plan` for single-lane claim commits.
3. If queue gate blocks, return `CommitOutcome::ResourceLaneClaimBlocked` before staging/materializing authority.
4. If queue gate allows, call `stage_prepared_commit_plan` as today.
5. If staging returns `ResourceLaneClaimBlocked` due to an active holder, insert/refresh waiter and return blocked.
6. If staging succeeds and the commit contains an admitted claim for a waiter, mark that waiter `claimed` in the same DB transaction after authority insertion succeeds.

This avoids putting operational waiter state into pure `mfm_store` staging while preserving strict semantic validation.

### Multi-lane claims

For this implementation, detect claim commits with more than one distinct `lane_id` and do not apply FIFO waiter admission. Choose one of these explicit behaviors:

Preferred:

- return a typed store error for multi-lane FIFO unsupported if such a commit would need FIFO gating.

Acceptable if existing tests depend on multi-lane staging:

- preserve existing non-FIFO all-or-nothing behavior for multi-lane claims and document that FIFO applies only to single-lane exclusive claims.

Do not claim FIFO fairness for multi-lane claims.

---

## Task plan

### Task 1: Update design documentation first

**Objective:** Record the FIFO contract and authority split before changing behavior.

**Files:**

- Modify: `RFC_REFAC_PG_TRANS.md`
- Modify: `docs/design.md`
- Modify: `docs/saga.md`
- Modify: `docs/persisted-public-surfaces.md`
- Modify as needed: `docs/architecture.md`

**Steps:**

1. In `RFC_REFAC_PG_TRANS.md`, replace the “fairness deferred” wording for v1 single-lane exclusive claims with the new durable waiter queue contract.
2. State explicitly that waiter rows are mutable operational admission state and not semantic replay authority.
3. State explicitly that `ResourceLaneClaimBlocked` may persist/refresh a waiter row but no run event or lane authority row.
4. State that FIFO is guaranteed only among live non-expired waiters for single-lane exclusive claims.
5. State that notifications are hints only and are not required for correctness.
6. In `docs/design.md`, add the same authority split to the resource-lane lifecycle section.
7. In `docs/saga.md`, update any “fairness deferred” language to match the new scoped FIFO contract.
8. In `docs/persisted-public-surfaces.md`, classify `resource_lane_waiter_counters` and `resource_lane_waiters` as mutable operational coordination surfaces, not append-only authority.
9. Run documentation grep checks:
   - `rg "fairness|FIFO|ResourceLaneClaimBlocked|resource_lane_wait" RFC_REFAC_PG_TRANS.md docs`

Expected result: docs consistently describe single-lane FIFO waiter admission and do not describe waiter rows as lane ownership authority.

### Task 2: Add waiter types to the store crate

**Objective:** Represent minimal waiter metadata without cosmetic queue-position fields.

**Files:**

- Modify: `crates/kernel/store/src/lib.rs`
- Test: existing store tests under `crates/kernel/store/tests/`

**Steps:**

1. Add a public `ResourceLaneWaiter` or `ResourceLaneWaiterBlock` struct near `ResourceLaneClaimBlock`:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceLaneWaiterBlock {
    pub waiter_id: String,
    pub lane_ticket: u64,
    pub lease_expires_at_unix_ms: i64,
}
```

Use the project’s existing timestamp type if one already exists in `mfm_store`; otherwise keep the field internal to Postgres-specific types until needed.

2. Change `ResourceLaneClaimBlock` to:

```rust
pub struct ResourceLaneClaimBlock {
    pub lane_key: ResourceLaneKey,
    pub holder: Option<SideEffectLedgerRef>,
    pub waiter: Option<ResourceLaneWaiterBlock>,
}
```

3. Update doc comments:
   - remove “no rows persisted” wording;
   - say no domain authority rows are persisted;
   - waiter metadata is non-authoritative.

4. Update compile errors in tests and call sites by wrapping existing `holder` values in `Some(...)` and `waiter: None`.

5. Run:
   - `cargo test -p mfm-store --test commit_contract`
   - or the exact package name from `cargo metadata` if the package name differs.

Expected result: store crate compiles and existing non-FIFO tests pass with the optional holder/waiter shape.

### Task 3: Add Postgres schema

**Objective:** Persist lane-local FIFO waiter state as operational tables.

**Files:**

- Modify: `crates/storages/stream-store-postgres/migrations/0001_run_store.sql`
- Modify: `crates/storages/stream-store-postgres/src/schema.rs`
- Test: Postgres schema/metadata tests under `crates/storages/stream-store-postgres` and integration metadata contracts.

**Steps:**

1. Add `resource_lane_waiter_counters` and `resource_lane_waiters` after `resource_lane_transitions` or near other lane tables.
2. Add indexes for live FIFO lookup and expiry.
3. Update schema allowlists/checks in `schema.rs` so the new mutable tables and indexes are expected.
4. Ensure immutability triggers are not applied to these tables. They are intentionally mutable operational state.
5. Run focused schema tests:
   - `cargo test -p mfm-stream-store-postgres schema`
   - `cargo test -p mfm-integration-tests --test cargo_metadata_contract`

Expected result: migration/schema checks accept the new tables and do not treat them as append-only authority tables.

### Task 4: Add Postgres waiter helper functions

**Objective:** Implement deterministic waiter id/fingerprint, enqueue/refresh, expiry, head lookup, and claimed marking.

**Files:**

- Modify: `crates/storages/stream-store-postgres/src/run_store.rs`
- Test: unit tests in the same crate if existing style supports them.

**Steps:**

1. Add constants:

```rust
const RESOURCE_LANE_WAITER_FINGERPRINT_DOMAIN: &[u8] = b"mfm.resource_lane.waiter.fingerprint.v1";
const RESOURCE_LANE_WAITER_ID_DOMAIN: &[u8] = b"mfm.resource_lane.waiter.id.v1";
const RESOURCE_LANE_WAITER_LEASE_SECS: i64 = 60;
```

2. Add `resource_lane_claim_fingerprint(...)` using canonical JSON and existing `sha256_digest_bytes` helper.
3. Add `resource_lane_waiter_id(...)` from the fingerprint.
4. Add `expire_stale_waiters_tx(tx, lane_id)`:

```sql
UPDATE resource_lane_waiters
SET status = 'expired', updated_at = statement_timestamp()
WHERE lane_id = $1
  AND status = 'waiting'
  AND lease_expires_at <= statement_timestamp()
```

5. Add `oldest_live_waiter_tx(tx, lane_id)` ordered by `lane_ticket ASC`.
6. Add `enqueue_or_refresh_waiter_tx(...)`:
   - if `(lane_id, claim_fingerprint)` exists with `waiting`, refresh `lease_expires_at` and return existing `waiter_id/lane_ticket`;
   - if exists with `expired` or `cancelled`, decide whether to reactivate with a new ticket or reject/reinsert. Prefer new ticket to avoid reclaiming old priority after expiry/cancel;
   - if absent, allocate `lane_ticket` under lane lock using `resource_lane_waiter_counters` and insert a `waiting` row.
7. Add `mark_waiter_claimed_tx(tx, lane_id, claim_fingerprint)` that updates only a matching `waiting` row to `claimed`.
8. Add focused tests for deterministic fingerprint/id if practical.

Expected result: helper functions compile and can be used from admission code.

### Task 5: Gate Postgres claim admission by FIFO waiter state

**Objective:** Enforce FIFO in the production Postgres append path.

**Files:**

- Modify: `crates/storages/stream-store-postgres/src/run_store.rs`
- Test: existing Postgres store tests in this crate.

**Steps:**

1. Add a helper to extract distinct claim lane ids and claim intents from `CommitRequest`.
2. For requests with exactly one `ResourceLaneClaimIntent`, run FIFO pre-gate after `lock_resource_lanes_for_request_tx` and before `stage_prepared_commit_plan`.
3. Pre-gate behavior:
   - expire stale waiters;
   - compute fingerprint;
   - load active resource lanes or enough lane authority to determine holder;
   - if active holder exists and differs, enqueue/refresh and return blocked with `holder: Some(...)`;
   - if no active holder but oldest live waiter exists for another fingerprint, enqueue/refresh and return blocked with `holder: None`;
   - otherwise allow staging.
4. Preserve existing semantic staging. If `stage_prepared_commit_plan` still returns `ResourceLaneClaimBlocked`, enqueue/refresh and return blocked.
5. After successful authority insertion for a claim whose fingerprint had a waiter, mark the waiter `claimed` in the same transaction before commit.
6. Ensure idempotent appended/idempotent commit handling does not create duplicate waiter rows.
7. Run focused Postgres tests.

Expected result: a later claimant cannot claim a free lane while an earlier live waiter exists.

### Task 6: Update in-memory/test store behavior enough for unit tests

**Objective:** Keep non-Postgres tests compiling and add store-contract tests for FIFO where appropriate.

**Files:**

- Modify: `crates/kernel/store/src/lib.rs` test-support store code if present.
- Modify: `crates/kernel/store/tests/commit_contract.rs`
- Modify: `crates/kernel/runtime/src/tests.rs` only as needed for new optional fields.

**Steps:**

1. Update existing `ResourceLaneClaimBlock` construction in tests to use `holder: Some(holder), waiter: None`.
2. Do not force the pure in-memory store to implement durable FIFO unless there is already a mutable test-store abstraction suitable for it.
3. Add tests at the store-contract level only if they can model the new waiter queue honestly.
4. Prefer Postgres integration tests for real FIFO semantics because the feature is Postgres-backed.

Expected result: existing unit tests continue to pass and do not falsely claim FIFO coverage where no durable queue exists.

### Task 7: Add FIFO Postgres integration tests

**Objective:** Prove cross-process FIFO admission at the store boundary.

**Files:**

- Modify/add tests under `crates/storages/stream-store-postgres/src/run_store.rs` if existing tests live there.
- Or add integration tests under the existing Postgres test crate/path used by this repository.

**Required scenarios:**

1. **Enqueue B then C while A holds lane**
   - A claims lane.
   - B claim returns blocked with waiter metadata.
   - C claim returns blocked with waiter metadata.
   - Assert B and C have distinct tickets and B ticket < C ticket.

2. **C cannot bypass B after release**
   - A releases lane.
   - C retries first.
   - C receives `ResourceLaneClaimBlocked` with `holder: None`.
   - B retries.
   - B claim appends `ResourceLaneClaimed`.

3. **C claims after B releases**
   - B releases lane.
   - C retries.
   - C claim appends.

4. **Blocked retry is idempotent**
   - B retries while still blocked.
   - Assert same waiter id and same lane ticket are returned/refreshed.
   - Assert no duplicate waiter row.

5. **Expired head waiter is skipped**
   - B is head waiter.
   - Force B lease expiry using SQL in test setup or a short test-only lease.
   - C retries and can claim after B is marked expired.

6. **No cosmetics in API**
   - Assert block metadata has no queue position/ahead count fields.

**Commands:**

Use the repository’s focused Postgres test command for this crate. If it requires `DATABASE_URL`, document the command in the test module or docs.

Expected result: FIFO behavior is enforced by store admission, not by test ordering or client-side checks.

### Task 8: Surface lane-block reports through runtime/app without queue cosmetics

**Objective:** Let app/CLI wait policy know a run is resource-lane blocked, without adding queue position fields.

**Files:**

- Modify: `crates/kernel/runtime/src/attempt.rs`
- Modify: `crates/kernel/runtime/src/scheduler.rs`
- Modify: `crates/kernel/runtime/src/lib.rs`
- Modify: `crates/app/src/lib.rs`

**Steps:**

1. Extend `ResourceLaneBlockWitness` only as needed to carry non-cosmetic wait identity:
   - `node_id`
   - `lane_key`
   - optional `waiter_id`
   - optional `lane_ticket`
2. Do not include queue position or queue-ahead count.
3. Add a richer non-waiting scheduler report if needed, e.g. `SchedulerDriveReport`, so app code can distinguish resource-lane blocked from generic blocked.
4. Keep existing `drive_once` and `drive_until_blocked` compatibility wrappers returning `SchedulerStatus`.
5. Update app `RunResponse` only if required for the CLI wait loop. Keep response metadata minimal and redacted.

Expected result: runtime/app can tell when waiting/retry is appropriate, but no cosmetic queue position is exposed.

### Task 9: Add explicit app/CLI waiting mode if implementing end-to-end liveness now

**Objective:** Allow `mfm run --op ...` processes to remain alive and retry their FIFO waiter until admitted or budget expires.

**Files:**

- Modify: `crates/app/src/lib.rs`
- Modify: `bin/cli/src/support/run_store.rs`
- Modify: `bin/cli/src/commands/run/start.rs`
- Modify: `bin/cli/src/commands/run/resume.rs`
- Modify: `bin/cli/README.md`

**Steps:**

1. Add an explicit drive mode such as `WaitResourceLane` or `UntilBlockedWithLaneWait`.
2. Add CLI flags with bounded default behavior. Example:

```text
--drive until-blocked-with-lane-wait
--lane-wait-budget-ms <MS>
```

3. Implement wait loop:
   - drive non-waiting scheduler;
   - if not resource-lane blocked, return;
   - if resource-lane blocked, sleep with capped backoff/jitter and retry until budget expires;
   - retrying the same claim refreshes the waiter lease.
4. Do not rely on queue position or holder display.
5. Treat future LISTEN/NOTIFY as an optimization only; polling/backoff must be sufficient.

Expected result: multiple CLI processes can make FIFO progress while alive, even without notifications.

### Task 10: Optional wake hints after polling FIFO works

**Objective:** Reduce latency without changing correctness.

**Files:**

- Modify: `crates/storages/stream-store-postgres/src/run_store.rs`
- Possibly modify: app/CLI Postgres service construction if a listener connection is added.

**Steps:**

1. Add `pg_notify` after successful release commit with a payload containing only lane id/digest, not raw key material.
2. Add a listener/wake path only after polling FIFO tests pass.
3. Ensure wait loop always re-drives/re-reads after notification.
4. Add a test or documented manual verification that missed notifications still progress via polling.

Expected result: wake hints improve latency but cannot grant ownership or bypass FIFO.

### Task 11: Documentation updates after implementation

**Objective:** Keep public and contributor docs accurate.

**Files:**

- Modify: `bin/cli/README.md`
- Modify: `docs/design.md`
- Modify: `docs/saga.md`
- Modify: `docs/persisted-public-surfaces.md`
- Modify: `RFC_REFAC_PG_TRANS.md`
- Modify rustdoc in public store/runtime/app APIs touched by the change.

**Steps:**

1. Document the new CLI drive/wait option if Task 9 is implemented.
2. Document that FIFO applies only to single-lane exclusive resource claims.
3. Document that waiters are live only while their lease is refreshed.
4. Document that expired waiters can be skipped.
5. Document that waiter rows are operational state and are excluded from replay authority.
6. Document `ResourceLaneClaimBlocked` revised persistence semantics.

Expected result: docs and rustdoc agree with the implementation.

### Task 12: Final verification

**Objective:** Prove implementation is correct and does not break existing contracts.

**Commands:**

Run focused checks first:

```bash
cargo fmt --all -- --check
cargo test -p mfm-store --test commit_contract
cargo test -p mfm-runtime
cargo test -p mfm-app
cargo test -p mfm-stream-store-postgres
cargo test -p mfm-integration-tests --test cargo_metadata_contract
cargo test -p mfm-integration-tests --test architecture_namespace_contract
```

Then, if feasible:

```bash
cargo check --workspace
cargo test --workspace
```

For Postgres-specific parity tests, start Postgres manually according to existing project conventions and run the focused Postgres tests with explicit `DATABASE_URL`.

Expected result: focused checks pass. If workspace-wide tests are too slow or require unavailable services, report exactly which focused checks were run and which were not.

---

## Risks and mitigations

### Risk: waiter rows accidentally become semantic authority

Mitigation: keep all ownership/release legality tied to `ResourceLaneClaimed`, `ResourceLaneReleased`, and `resource_lane_transitions`; document waiter rows as operational coordination state only.

### Risk: dead CLI blocks queue forever

Mitigation: waiter leases and opportunistic expiry under lane lock.

### Risk: retry creates duplicate waiters

Mitigation: deterministic `claim_fingerprint` and `UNIQUE (lane_id, claim_fingerprint)`.

### Risk: later process wakes faster and bypasses earlier process

Mitigation: store-side FIFO gate checks oldest live waiter before materializing any claim.

### Risk: multi-lane claims get an accidental fairness claim

Mitigation: explicitly scope FIFO to single-lane claims; reject or preserve non-FIFO behavior for multi-lane claims and document it.

### Risk: exposing raw resource keys leaks sensitive data

Mitigation: waiter metadata uses waiter id/ticket and existing redacted lane identifiers only. Do not add queue position or raw key display.

### Risk: pure store staging cannot see waiter queue

Mitigation: implement FIFO gate in Postgres append admission while keeping pure staging for semantic authority validation.

---

## Acceptance criteria

- A blocked single-lane claim inserts or refreshes exactly one waiter row for its stable claim fingerprint.
- `ResourceLaneClaimBlocked` no longer promises that no operational rows were persisted.
- A free lane cannot be claimed by a later live waiter while an earlier live waiter exists.
- The head live waiter can claim the lane after the prior holder releases.
- Expired/cancelled waiters do not block later waiters.
- Successful claim marks the matching waiter `claimed` in the same transaction as the authoritative `ResourceLaneClaimed` rows.
- No new domain `ResourceLaneBlocked` event exists.
- No queue-position or queue-ahead cosmetic metadata is exposed.
- Documentation clearly states FIFO scope, lease semantics, and authority split.
