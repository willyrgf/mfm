# RFC: Per-Process Fungibility

Status: **Proposed** (pre-ratification; no code changed)
Date: 2026-06-24
Scope: MFM execution model, coordination layer, and side-effect lifecycle
Derivation / reasoning record: `MFM_PROCESS_FUNGIBLE.md` (R1–R4). This RFC is the consolidated,
authoritative specification; the working doc holds the dialogue and the code-grounded findings.
On ratification, §4 / §6 land in `docs/design.md` and the side-effect contract change lands in
`docs/saga.md`.

---

## 1. Summary

MFM was built without an explicit decision on how it is executed: one service driving many runs,
or many processes each driving runs. This RFC resolves that: **MFM is a Postgres-coordinated
durable execution fabric in which processes are fungible.** Any worker can drive any run; no
process owns anything; correctness lives in the database transaction boundary, never in process
topology.

Most of the substrate already exists (per-run serialization, append-only authority, sharded
resource lanes, a durable observation/watch feed). The remaining work is three coherent pieces:

1. **A unified admission-lane primitive** — one durable mechanism for fair, fenced, lease-reaped
   exclusive admission that serves both *resource lanes* (wallet/nonce contention) and *execution
   claims* (which worker drives a run), plus any future coordination need.
2. **Execution discovery & dispatch** — turn the existing read-only observation feed into an
   execution fabric: workers discover, claim, and drive runnable runs.
3. **Side-effect verification as a paired framework state** — every side-effect node lowers into a
   `submit` node plus an auto-inserted `verify` framework node. This productizes receipt/finality
   verification uniformly *and* makes external-effect recovery fall out of ordinary dispatch,
   collapsing the bespoke reconciliation machinery the earlier design implied.

The net is more capability with less bespoke code, and a system that scales to many processes and
many agents/sessions without any process-level coordination.

## 2. Motivation

### 2.1 The unanswered question

> Is MFM a service that runs many runs inside one process, or a per-execution platform where many
> processes run at once, competing for resources (EVM wallets / Postgres / transports)?

This is a false fork. The semantic unit is a **run** (a durable certified workflow instance), not
a process. Whether one process drives a thousand runs or a thousand processes each drive one is a
deployment knob — *provided coordination does not live in any process.*

### 2.2 Why now

MFM is intended to be operated by **multiple agents across multiple sessions**. Coordination
therefore cannot be process-local: it must be shared, durable, and reconstructable across as many
processes as needed. The shared durable coordinator already exists — Postgres plus the typed,
append-only, certified design.

### 2.3 What is missing today

- **No execution dispatch.** Execution is per-run and caller-driven; a worker must already hold a
  `run_id`. The observation feed is strictly read/observe — there is no "lease me a runnable run."
- **No external-effect reconciliation.** A worker that submits a transaction and dies leaves an
  ambiguous nonce; nothing reads chain truth to resolve landed-vs-never-sent.
- **Verification is not productized.** Receipt/finality checking is embedded ad hoc inside the
  side-effect runner rather than configurable per state and reusable across domains.

## 3. Goals and non-goals

### Goals

- A correctness model that depends only on the Postgres transaction boundary, never on process
  identity or lifetime.
- Many disposable workers (CLI, daemon, REST host, agent session) coordinating only through
  Postgres, with at-least-once execution and exactly-once *effect* semantics.
- One reusable coordination primitive for all exclusive-admission needs.
- Uniform, configurable side-effect verification (receipt vs finality) across all domains.
- External-effect recovery that requires no bespoke machinery beyond ordinary dispatch.

### Non-goals

- Pipelined nonces (multiple in-flight transactions per account). Deferred; serial-within-hold is
  sufficient and far simpler (DEC-1).
- Cross-run priority/fairness beyond observation order, and feed sharding. Deferred (DEC-9).
- Full AC/DC semantics for arbitrary external systems (unchanged from `docs/saga.md`).
- Changing the semantic core (certified specs, typed values, replay) — none of this RFC touches it.

## 4. Guiding doctrine

> **Correctness is the Postgres transaction boundary. Process topology is operational, never
> architectural — workers are disposable and interchangeable. Leases and liveness are progress
> mechanisms, never safety. The only correctness that escapes the transaction boundary is external
> mutation, which is cornered into the side-effect ledger and made safe by lane-exclusivity +
> idempotency + chain reconciliation — never by process identity.**

Three layers follow:

- **Semantic layer** (typed design): certified specs, state graph, typed values, side-effect
  contracts, saga rules, replay rules. *Unchanged by this RFC.*
- **Coordination layer** (Postgres-backed store contract): atomic appends, admission leases/fences,
  resource lanes, idempotency, durable wakeups. *Authority is the typed store contract, not raw SQL
  rows.*
- **Execution layer** (disposable workers): dispatch loops, local concurrency, capability pools,
  transport clients, signer adapters. *May optimize execution; cannot define correctness.*

## 5. Current state (code-grounded baseline)

The safety substrate is already complete; the gaps are in liveness/scale/discovery.

- **Per-run serialization + optimistic seq.** Every append takes a transaction-scoped
  `pg_advisory_xact_lock` on the run id (`run_store/stream.rs:54`), backed by `commits` PK
  `(run_id, seq)` and `expected_next_seq` (`run_store/mod.rs:157`). Two workers driving one run:
  one commits each step; the other no-ops (commit-key idempotent) or retries. **Leaseless is
  already safe.**
- **DB-enforced append-only authority.** `mfm_reject_authority_mutation` triggers reject
  UPDATE/DELETE/TRUNCATE on every authority table (`migrations/0001_run_store.sql`).
- **Sharded resource-lane admission.** Per-lane `pg_advisory_xact_lock` on only the touched lanes
  plus a `resource_lane_waiters` FIFO queue with ticketing and 60s waiter-lease reaping
  (`run_store/resource_lanes.rs`).
- **Durable observation/watch feed.** Cursored, globally ordered by `(append_xid, commit_sort_key)`
  past a sealed `frontier_xid`, woken by `LISTEN/NOTIFY mfm_run_observation`
  (`run_store/observations.rs`); exposed via app/REST/CLI. **Read-only.**
- **Exclusive nonce lane.** EVM submit declares `ResourceClaim::exclusive` per signer account
  (`states/evm-contracts/src/lib.rs:107`).
- **Side-effect phases incl. receipt/confirmation.** The ledger runs `IntentPersisted → Claimed →
  InvocationPrepared → InvocationStarted → {SubmissionObserved | SubmissionUnknown |
  NotSubmittedProven} → ReceiptObserved → ConfirmationObserved → (Failed)`
  (`events.rs:413-439`; `SideEffectPhase`). Fencing (`claim_fencing_token`, `claim_generation`,
  `ClaimTakenOver`) intact.
- **Typed idempotency + verifier pattern.** `IdempotencyKey`/`idempotency_input`
  (`program/src/lib.rs:1506,1531`); `SideEffectReplayVerifier` (`replay/src/lib.rs:470`); framework
  nodes inserted/validated at lowering (`certify/src/framework_lifecycle.rs`).

## 6. Design — the end state

### 6.1 Process-fungible execution model

A run is persisted authority (certified spec) plus a current frontier. Any worker with the registry,
capabilities, and store authority may drive it. Safety is the store (§5). A **worker lease is
operational authority only**: "this process currently has fenced permission to *attempt* an action
under durable preconditions," never "this process owns the run." The semantic result exists only
after a valid store-admitted append.

### 6.2 The admission-lane primitive

Resource-lane admission and execution claims are two instances of one abstraction: **fair, durable,
fenced, lease-reaped exclusive admission to a named lane.** They unify into one primitive.

Governing principle: **the primitive owns admission *order* + an operational *lease*; the caller
owns *authority*; they never share a row.** The shared table is liveness-only and structurally
cannot be authority.

One table-pair, one `admit(mode)` code path; a `class` discriminator **binds** the mode:

```
admission_lane   (class, lane_id) PK, next_ticket,            -- FIFO ticket source
                  holder_token, lease_expires_at              -- NowaitSkip holder lease ONLY
admission_waiter (class, lane_id, ticket) PK, token,          -- WaitFifo queue ONLY
                  status, lease_expires_at
```

```rust
enum Mode { WaitFifo, NowaitSkip }
enum Admission { Admitted(Lease), Blocked(Waiter), Busy }

// all take &mut Transaction<Postgres> → identical code runs inside a commit txn (resource lanes)
// and in a standalone txn (execution dispatch). Shared *down* into a common component, never
// called *across* layers.
async fn lock_lane(tx, lane);
async fn admit(tx, lane, token, mode, holder_is_free, ttl) -> Admission;
async fn renew / release / reap_expired / head_of_line(...);
```

Two instantiations exercise disjoint subsets, so neither stores the other's authority:

- **Resource lane (`WaitFifo`)** — uses only waiter rows. Admission gates the commit, so "admitted"
  and the `ResourceLaneClaimed` authority event are atomic in one txn; the steady-state holder is
  never stored here. The caller supplies `holder_is_free` from the event fold.
- **Execution claim (`NowaitSkip`, lane_id = run_id)** — uses only the lane row's lease; writes no
  waiters; the lease *is* the holder, harmless because safety = the store's per-run lock +
  `expected_next_seq`. A worker that can't claim run X skips to run Y; it never queues.

Non-negotiable invariants:

1. **Mode is bound to class** — `NowaitSkip` is unrepresentable on a FIFO lane and vice-versa.
2. **The table is liveness-only** — no safety decision branches on it; a false reap or two
   simultaneous "holders" cost duplicate work, never corruption.
3. **The materialized lane-state projection (§6.5 / WS-E) stays a SEPARATE event-derived view** —
   never this table; merging them is the one move that manufactures dual authority.

### 6.3 Execution discovery & dispatch

Discovery reuses the observation feed; the only genuinely new coordination is the execution claim
(§6.2). The dispatch loop is library code colocated in *every* worker — **no central dispatcher**
(that would re-introduce process-level coordination):

```
cursor = observe_list(status = non-terminal)        // cold start: seed candidates
loop:
  for run in candidates:                             // feed changes ∪ due_at-elapsed ∪ new head-of-line waiters
    if acquire_execution_claim(run) is None: continue
    while frontier(fold(run)) is Runnable(t):
      renew_claim(); commit(t)                       // release tenure on long external waits (DEC-8)
    release_claim(run)                               // blocked / terminal
  (cursor, candidates) = observe_watch(cursor, wait_ms)   // NOTIFY-woken
```

- **Safety is independent of the claim** — the store's per-run lock + `expected_next_seq` is the
  sole guard. A dead worker's claim expires and is takeable; a double-claim only wastes work.
- **Runnability is derived lazily** by the claim winner folding the run; no new index in v1.
- **`due_at` (DEC-6)** — a durable "blocked until T" the dispatcher polls (for confirmation polling
  / backoff), woken by NOTIFY in between.
- **Lane-release wakeup** — on wake the dispatcher re-checks `head_of_line` for lanes whose holder
  released, surfacing the new head-of-line run as a candidate.

### 6.4 Side-effect verification as a paired framework state

Verification — "did the mutation land (receipt), and is it final (confirmation)?" — is intrinsic to
*every* side effect, not a recovery special case. Lower every side-effect node into a **pair**:

- **Submit node** (the domain side-effect state): drives `Intent → Claim(lane) → Prepared →
  InvocationStarted → Submission*`. Records the deterministic anchor (idempotency key pinning the
  signed-tx hash) *before* the boundary, crosses the boundary once (the single external mutation),
  ends at "submitted." Effect class: apply-side-effect.
- **Verify node** (framework node): drives `Submission* → ReceiptObserved → ConfirmationObserved →
  terminal` to the configured level, then releases the lane and binds the verified output. Effect
  class: read.

Key properties:

- **Auto-inserted at lowering.** Add `FrameworkNodeSpec::SideEffectVerify` beside `Bridge /
  PublicOutputRender / ProjectRetentionManifest / CompleteRun / ResolveSagaTerminal`. Lowering
  inserts exactly one verify node after each side-effect node, cardinality-checked like the other
  framework nodes. Automatic, certified, static, zero per-operation LOC.
- **Level configured on the contract.** Add a hash-defining `verification: Receipt | Finalized`
  field to `SideEffectContractSpec` (beside `resource_claim`). `Receipt` terminalizes at
  `ReceiptObserved` (accepts reorg risk); `Finalized` requires `ConfirmationObserved` past the
  chain's configured depth. The class is semantic/certified; the numeric depth per chain is runtime
  config recorded as evidence at determination time. There is no "skip" level.
- **One ledger across the pair (DEC-14).** The verify node *continues* the submit node's ledger —
  same ledger key, anchor, and lane. The ledger key must be **stable across the pair and not tied
  to a node attempt** (lowering records the shared ledger identity in the verify node's spec). The
  resource lane is claimed in the submit commit and released in the verify terminal commit, on one
  ledger. This makes the **attempt lifecycle / ledger lifecycle** decoupling explicit.
- **Live vs replay verifier.** A new live `SideEffectVerifier` (read capability only; any fungible
  worker can run it) reads external truth by the anchor and emits the existing `ReceiptObserved /
  ConfirmationObserved / NotSubmittedProven` evidence. The existing `SideEffectReplayVerifier`
  checks that recorded evidence with no live IO. Same live-vs-replay symmetry MFM uses elsewhere.

### 6.5 Reconciliation & recovery (emergent — no bespoke machinery)

Because the verify node is an ordinary frontier node, recovery falls out of dispatch:

- A worker that dies mid-verification leaves a **runnable verify node**; ordinary dispatch +
  execution-lease expiry re-drives it.
- A worker that dies mid-submission leaves an **open submit attempt**; ordinary attempt recovery
  re-runs it. Safety floor: the tx is pinned to one nonce, so **at most one tx can land — never a
  double-spend**. Deterministic signing (DEC-15) makes the re-broadcast idempotent.
- The wallet lane releases **only** when the verify node terminalizes, so a dead worker never wedges
  the wallet.
- **Foreign-occupied nonce** (a tx MFM never sent occupies the nonce) → the anchor never appears →
  `NotSubmittedProven` terminal ("superseded"), an honest failure/compensation, never a
  double-spend. Exclusivity is an **availability** expectation, not a safety assumption (DEC-2).

**Key invariant (replaces any holder-death sweeper):** a non-terminal side-effect ledger always has
a corresponding runnable (or `due_at`-scheduled) frontier node. The scheduler guarantees it,
dispatch drives it, the lane releases at terminal.

This subsumes what the earlier design called the "recovery verifier" (now the general live verifier)
and the "resource-lane holder-takeover machine" (now unnecessary — the lane is ledger-lifetime).

### 6.6 Determinism, capabilities, replay

- **Deterministic signing required (DEC-15).** The signer contract must produce deterministic
  signatures (RFC 6979 for ECDSA secp256k1). Enforced, not assumed: the anchor (expected hash) is
  recorded before the boundary, and recovery **asserts the re-signed hash equals the recorded
  anchor** — a determinism-violating signer fails closed instead of double-submitting.
- **Capabilities split semantic vs operational** (unchanged): outcome-affecting authority (signer
  ref, expected chain id, expected address) is pinned to the run and verified at admission;
  transport routing (RPC URL, source id) is per-worker. Verification needs only **read**
  capabilities, so any fungible worker can run it.
- **Replay unchanged in shape.** The verify node records receipt/confirmation evidence; replay
  re-verifies from recorded evidence via `SideEffectReplayVerifier`, no live IO.

## 7. Decision log

| ID | Decision | Rationale |
|---|---|---|
| DEC-0 | Doctrine: correctness = the Postgres transaction boundary; topology operational; leases = liveness, never safety | Enables fungible processes / multi-agent without process-level coordination |
| DEC-1 | Nonce: serial per-account, sequential-within-hold; pipelining deferred | Deterministic nonce block, point-wise recovery; pipelining reopens range/gap recovery |
| DEC-2 | Reconcile on the deterministic idempotency anchor, never the nonce; no exclusive-ownership assumption | Foreign-occupied nonce = safe detectable terminal; exclusivity = availability, not safety |
| DEC-3 | Finality: semantic class (`Receipt`/`Finalized`, hash-defining) + runtime depth recorded as evidence | Replay-deterministic; designer owns the requirement, deployment owns the number |
| DEC-4 | Admission leases + heartbeat; expiry routes to recovery, never silent release | Superseded for resource-lane *holders* by §6.4–6.5 (ledger-lifetime); applies to execution tenure |
| DEC-5 | Recovery = ordinary fungible dispatch of a run in a recovering state | Needs only a read capability; no dedicated recovery worker class |
| DEC-6 | Add a `due_at` dispatch signal for confirmation polling / backoff | Verify nodes awaiting confirmations are `blocked-until-due_at` |
| DEC-7 | Per-run execution lease (not per-attempt) | Simplest; runs are the parallelism unit; per-attempt deferred |
| DEC-8 | Release execution tenure on long external waits | A confirmation wait must not pin a worker |
| DEC-9 | Single feed / oldest-first now; shard by `hash(run_id)` later | Defer scale complexity until needed |
| DEC-10 | Materialize the lane-state projection (rebuildable cache, not authority) | Removes the O(history) `resource_lane:%` fold |
| DEC-11 | Size pools to worker count; pooler-safe (xact-scoped locks + lease tables) | No session advisory locks → transaction-mode poolers OK |
| DEC-12 | Unify resource lanes + execution claims into one admission-lane primitive | One table-pair, one `admit(mode)`; primitive owns order+lease, caller owns authority |
| DEC-13 | Side-effect verification is a paired framework state (submit + verify) | Uniform/configurable verification; recovery-for-free; collapses DEC-2/DEC-4 machinery |
| DEC-14 | One ledger across the verify pair | Ledger key stable across the pair (not attempt-tied); decouples attempt vs ledger lifecycle |
| DEC-15 | Require deterministic signing, enforced via re-sign == recorded anchor | Idempotent re-broadcast; determinism violation fails closed |

## 8. Changes required (implementation plan)

No backward-compatibility constraint (`docs/code-quality.md`): flawed shapes are fixed deliberately
with docs+tests in the same change. Five workstreams.

### WS-A — Admission-lane primitive (refactor)

- **Schema migration:** `admission_lane` + `admission_waiter` (§6.2) with FIFO and expiry indexes;
  retire `resource_lane_waiters` + `resource_lane_waiter_counters`.
- **Generic admission component:** `lock_lane / admit / renew / release / reap_expired /
  head_of_line`, all over `&mut Transaction`; mode bound to `class`; structurally liveness-only.
- **Port resource lanes** onto it (`WaitFifo`, `holder_is_free` from the event fold), relocating
  today's `run_store/resource_lanes.rs` logic.
- **Targets:** `crates/storages/stream-store-postgres` (migration + SQL component); lane-id /
  fingerprint derivation in `kernel/store` backend. *To confirm:* whether the component is a new
  shared crate or a store module reused by app dispatch.
- **Depends on:** nothing (behavior-preserving refactor for resource lanes).

### WS-B — Execution discovery & dispatch

- **Execution lane class** (`NowaitSkip`, `lane_id = run_id`) on the WS-A primitive.
- **Dispatch loop** (library, colocated per worker): cold-start list → NOTIFY-woken watch →
  try-acquire → fold → drive-until-blocked (renew; release on long waits) → release.
- **Runnable granularity:** extend the frontier read so the claim winner gets Runnable/Blocked/
  Terminal (today the feed reports only Started/Completed).
- **`due_at` signal** (DEC-6) and **lane-release wakeup** in the dispatcher.
- **Worker entrypoint** in `bin` + a dispatch service in `crates/app`.
- **Targets:** `crates/app`, `bin/*`, `kernel/runtime` (frontier granularity), `storages` (`due_at`,
  wakeup).
- **Depends on:** WS-A.

### WS-C — Side-effect verification as a paired framework state

- **`FrameworkNodeSpec::SideEffectVerify`** spec types + parse/json (`kernel/spec`) + cardinality
  validation (`kernel/certify/framework_lifecycle.rs`).
- **Lowering** inserts the verify node after each side-effect node and records the shared ledger
  link (`kernel/program` lowering).
- **`verification: Receipt | Finalized`** on `SideEffectContractSpec` (hash-defining; parse/validate
  in `kernel/spec`).
- **One ledger across the pair:** make ledger-key derivation stable across the pair (decoupled from
  the attempt) so the verify node continues the submit ledger; lane claimed by submit, released by
  verify (`kernel/store` ledger-key + phase ownership).
- **Verify-node runner** (`kernel/runtime` framework runner): drive `Submission* → Receipt →
  Confirmation → terminal` to the configured level; bind the verified output cell.
- **Scheduler invariant:** non-terminal side-effect ledger ⇒ runnable / `due_at` frontier node.
- **Live `SideEffectVerifier`** contract (`crates/adapter-contracts`) + EVM impl
  (`crates/adapters/evm-contracts` + `crates/transports/evm`, relocating receipt/confirmation poll),
  behind a read capability.
- **EVM `IdempotencyInput`** shaped to pin the signed-tx hash (`crates/states/evm-contracts`).
- **Depends on:** WS-B (recovery-for-free relies on dispatch + `due_at`).

### WS-D — Determinism & reconciliation (mostly emergent)

- **Deterministic-signing requirement** on the signer contract (`crates/signing`); RFC 6979 in
  `crates/evm-signing`.
- **Re-sign == recorded-anchor assertion** in the submit-node recovery path (`kernel/runtime`);
  fail closed on mismatch.
- **Foreign-occupied-nonce → `NotSubmittedProven`** terminal in the EVM verifier.
- **Depends on:** WS-C.

### WS-E — Performance & ops (later)

- **Materialize the lane-state projection** (DEC-10), rebuildable from events, never authority.
- **Connection budget / pooler config** (DEC-11).
- **Feed sharding** (DEC-9) when a single feed saturates.
- **Depends on:** WS-A/WS-B.

### Sequencing

```
Phase 1: WS-A  (admission lane)            ── foundation, behavior-preserving for resource lanes
Phase 2: WS-B  (execution dispatch)        ── turns the observation feed into an execution fabric
Phase 3: WS-C + WS-D (verification + determinism) ── productizes side effects; delivers recovery
Phase 4: WS-E  (perf / ops)                ── scale optimizations
```

## 9. Migration & compatibility

- **`resource_lane_waiters` → `admission_lane`/`admission_waiter`.** A fresh schema is acceptable;
  resource-lane *holders* are event-derived (rebuilt from the stream), so only the operational
  waiter queue is replaced. No semantic data migration is required for holders.
- **Side-effect contract gains `verification`.** This changes the spec hash; existing certified
  specs are re-lowered/re-certified (no in-place migration of historical runs, per `docs/design.md`
  CLI/REST rules).
- **Public surfaces.** A `mfm worker` (or equivalent) dispatch entrypoint is additive; existing
  `run start/resume/replay/list --watch` are unchanged.

## 10. Risks & mitigations

| Risk | Mitigation |
|---|---|
| Admission table mistaken for authority | Structural: mode∝class, liveness-only, no safety read branches on it (§6.2 inv. 2); lane-state projection kept separate |
| Ledger-key change destabilizes existing side-effect runs | Ledger key derived from `(run, stable side-effect identity)` carried by the certified pair; covered by replay + side-effect integration tests |
| Non-deterministic signer slips in | Enforced by the re-sign == anchor assertion (DEC-15): fails closed, never double-submits |
| Thundering herd at high worker count | `NowaitSkip` losers pay one `UPDATE` and skip; no waiter writes on the Busy path; shard the feed later (DEC-9) |
| Postgres as the coordination/scaling ceiling | Per-run and per-lane locks parallelize disjoint work; materialized lane projection (WS-E) removes the O(history) fold |
| Wallet wedged by a stuck/slow verify | Scheduler invariant (§6.5) keeps a runnable/`due_at` node; `due_at` backoff bounds polling |

## 11. Testing strategy

- **Concurrency:** property/integration tests with N concurrent drivers per run asserting
  single-commit-wins and idempotent re-presentation.
- **Admission:** FIFO fairness, lease-expiry reaping, `NowaitSkip` no-queue, mode∝class compile-fail
  fixtures.
- **Side-effect recovery:** kill-mid-submit and kill-mid-verify integration tests against a local
  chain (reth) asserting at-most-once landing, idempotent re-broadcast, and correct
  `Receipt/Confirmation/NotSubmittedProven` terminalization.
- **Determinism:** assert re-sign reproduces the recorded anchor; a deliberately non-deterministic
  signer fixture must fail closed.
- **Replay:** verify-node evidence replays from recorded facts with no live IO.
- **Boundary:** cargo-metadata + compile-fail fixtures keeping the admission primitive
  domain-agnostic and the verifier read-only.

## 12. Deferred / open

- Pipelined nonces (DEC-1); per-attempt execution leases (DEC-7); feed sharding and cross-run
  priority (DEC-9).
- Confirm the home crate of the admission component (shared crate vs store module).
- D5–D11 defaults to be rubber-stamped at ratification.
- Concrete EVM `IdempotencyInput` shape that pins the signed-tx hash.

## 13. Appendix — key code anchors

| Concern | Location |
|---|---|
| Per-run append lock; head `MAX(seq)` | `run_store/stream.rs:54`, `:25-52` |
| Commit-key idempotency; `expected_next_seq` | `run_store/append.rs:19-55`; `run_store/mod.rs:157` |
| Append-only authority triggers | `migrations/0001_run_store.sql:12-18,200-230` |
| Per-lane advisory lock + FIFO waiters | `run_store/resource_lanes.rs:32-45,134-310` |
| Observation/watch feed + NOTIFY | `run_store/observations.rs:104-268,379-461`; channel `mod.rs:89` |
| Observation surface (app/REST/CLI) | `app/src/lib.rs:1157`; `bin/rest-api/src/lib.rs:502`; `bin/cli/src/commands/run/list.rs` |
| Side-effect fencing | `kernel/store/src/v1/{projection.rs:36,412 , resource_lanes.rs:42,80 , mod.rs:2179-2220}` |
| Side-effect phases/events | `kernel/events/src/lib.rs:413-439`; `SideEffectPhase` in `store/v1` |
| `IdempotencyKey` / `idempotency_input` | `kernel/program/src/lib.rs:1506,1531` |
| `SideEffectContractSpec` / `ResourceClaimSpec` | `kernel/spec/src/lib.rs:1740`, `:1689-1731` |
| `FrameworkNodeSpec` + framework-node validation | `kernel/spec/src/lib.rs:1758`; `kernel/certify/src/framework_lifecycle.rs` |
| `SideEffectReplayVerifier` | `kernel/replay/src/lib.rs:470` |
| EVM exclusive nonce lane; nonce read | `states/evm-contracts/src/lib.rs:107`; `transports/evm/src/lib.rs:469` |
