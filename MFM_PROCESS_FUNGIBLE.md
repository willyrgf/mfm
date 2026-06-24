# MFM: Process-Fungible Execution

Status: **working design discussion — not yet normative.** Captures the architecture
dialogue plus code-grounded analysis on a question the platform was built around but never
decided explicitly: is MFM one service running many runs, or many processes each running
runs? Conclusion: neither framing is architectural — **processes are fungible workers over
Postgres-owned coordination.** This document is the resume point for turning the conclusion
into a normative `docs/design.md` section and an execution-layer roadmap.

Date opened: 2026-06-23.
Revised: 2026-06-23 (**R2**, after a large Postgres refactor landed — see §0);
2026-06-24 (**R3**, roadmap #1 decided — the unified admission-lane design, §6.1; decisions
D1–D4 resolved, §6.2); 2026-06-24 (**R4**, side-effect design specified — verification as a paired
framework state, §9; refines/simplifies D2–D4).
We have changed no code in this thread. The platform itself advanced: the monolithic
`stream-store-postgres/src/typed.rs` was split into a `run_store/` module tree and the schema
was rewritten (`0001_typed_run_event_store.sql` 83 lines → `0001_run_store.sql` 230 lines),
across commits `2cf867ef` … `39cca870`.

---

## 0. Revision R2 — what the Postgres refactor changed

The refactor implemented most of the §6 roadmap. Delta against the R1 findings:

| R1 finding | Status now | Detail |
|---|---|---|
| Global lane lock = scaling ceiling (§5c.1) | **Superseded** | Global `LOCK TABLE … SHARE ROW EXCLUSIVE` is gone. Now per-lane `pg_advisory_xact_lock` on only the lanes a commit touches, plus a `resource_lane_waiters` FIFO queue (per-lane monotonic tickets, 60s waiter leases, dead-waiter expiry reaping). Disjoint runs/lanes commit concurrently. |
| No cross-run discovery (§5c.2) | **Half closed** | A durable observation/watch fabric landed: cursored, globally ordered by `(append_xid, commit_sort_key)`, sealed-snapshot `frontier_xid` boundary, `LISTEN/NOTIFY` wakeup; exposed via app/REST/CLI. Still strictly read/observe — no execution claim/dispatch/lease. |
| EVM claim kind (§7.1 open) | **Resolved** | EVM submit declares `ResourceClaim::exclusive` per signer account. Per-account nonce safety is real and intentional, not incidental to the old global lock. |
| EVM chain reconciliation (§7.2 open) | **Resolved (negative)** | Not implemented. Only `eth_getTransactionCount` read-at-prepare. Reconciliation-on-takeover is still unbuilt. |
| Safety already complete (§5a) | **Still true, re-mechanized** | Per-run `pg_advisory_xact_lock` + `commits` PK/uniqueness + `expected_next_seq`; side-effect fencing intact. New hardening: authority tables are DB-enforced append-only via `mfm_reject_authority_mutation` triggers. |
| Fencing has no trigger (Gap 2) | **Partial** | Waiter-queue liveness now exists (lease-expiry reaping). Active-holder takeover trigger and an execution-level lease still open. |

Net: the doctrine (§2) is now ~95% realized in code. **R3 decided roadmap #1** — execution
discovery + effort-dedup unify with resource-lane admission into one **admission-lane** primitive
(§6.1). The remaining decision agenda (§6.2): the external-effect reconciliation seam, liveness &
recovery, execution-tenure policy, and performance/ops.

## 1. The question

> Is MFM a service that runs hundreds/thousands of runs simultaneously inside one process,
> or a per-execution platform where hundreds/thousands of processes run at once, competing
> for resources (EVM wallets / Postgres / other transports)?

The sharpened answer is that this is a false fork. The semantic unit is a **run** (a durable
certified workflow instance), not a process. Whether one process drives a thousand runs or a
thousand processes each drive one is a deployment knob, not an architecture — *provided
coordination does not live in any process.*

The decisive reframe (the user's, and the one we committed to): MFM is intended to be used by
**multiple agents across multiple sessions**. Coordination therefore *cannot* be process-level.
It must be shared, durable, and reconstructable across as many processes as needed. The shared
durable coordinator is **Postgres + the typed/durable/inspectable/recoverable design** already
in the codebase.

## 2. The doctrine (the thing to ratify)

> **Correctness is the Postgres transaction boundary. Process topology is operational, never
> architectural — workers are disposable and interchangeable. Leases and liveness are progress
> mechanisms, never safety. The only correctness that escapes the transaction boundary is
> external mutation, which is cornered into the side-effect ledger and made safe by
> lane-exclusivity + idempotency + chain reconciliation — never by process identity.**

After R2 this is ~95% realized in code (see §5). Lane-exclusivity is now concrete (per-account
`Exclusive` lanes with FIFO admission); idempotency is a DB constraint; only chain reconciliation
remains unbuilt. Ratifying the doctrine makes the remaining roadmap narrow and known (§6), and
none of it touches the semantic core.

## 3. Conversation arc (the reasoning we want to keep)

**Position 1 — service-capable run platform.** A run is persisted authority (certified spec)
plus a current frontier, not a process. Any worker with the right registry, capabilities, and
store authority can resume it. Process-per-run makes the hard problems (Postgres connections,
EVM RPC limits, signer/nonce lanes, crash-recovery ownership) worse by pushing coordination
into the OS scheduler and ad-hoc locks. Target model: a multi-run runtime with **leased work**,
separating run identity / semantic authority / execution ownership / external-mutation
ownership / deployment shape.

**Position 2 — the reframe (stronger).** Don't say "one service process owns coordination."
Say coordination authority *lives in Postgres + typed event/ledger semantics*, and processes
are disposable executors that acquire temporary, fenced authority to do work. This is what the
multi-agent / multi-session vision actually requires. Three layers fall out:

- **Semantic layer** (typed design): certified specs, state graph, typed values, side-effect
  contracts, saga rules, replay rules.
- **Coordination layer** (Postgres-backed store contract): atomic appends, leases/fences,
  resource lanes, attempt ownership, idempotency/admission, recovery eligibility, durable
  wakeups. *Authority is the typed store contract, not raw SQL rows.*
- **Execution layer** (disposable workers): async tasks, local concurrency, capability pools,
  transport clients, signer adapters, local retry loops, process lifecycle. **May optimize
  execution; cannot define correctness.**

Key precision retained from the dialogue: **a worker lease is operational authority, not
semantic authority.** It means "this process currently has fenced permission to *attempt* an
action under durable preconditions," never "this process owns the run." The semantic result
exists only after a valid store-admitted append.

**The leak the framing hides.** Postgres gives free coordination for everything inside a
transaction. The hard part is everything that *isn't*: the gap between durable intent →
external fact → durably-recorded result. You cannot put "broadcast this EVM tx" in the same
transaction that records that you broadcast it. The Postgres decision doesn't remove this — it
**corners** it into one seam (the side-effect ledger / saga boundary). Everything else gets to
be a transaction. That cornering is the best property of the decision.

## 4. The three gaps the doctrine forces you to answer

1. **Nonce lane, not lease, makes a wallet safe.** Something durable must hand out and reconcile
   per-`(chain, account)` nonces; a worker can claim a nonce, broadcast, and die before
   recording the hash, and only the chain can disambiguate landed-vs-never-sent.
2. **Fencing protects Postgres, not the chain.** A fencing token makes a stale worker's *commit*
   lose, but the chain never checks the token — a stale worker can still broadcast. So for
   non-idempotent external effects, lease expiry is *liveness*, never *safety*. Safety must come
   from external idempotency or nonce-lane reconciliation. Rule: *lease expiry is never license
   to re-do an external effect — only license to re-examine one.*
3. **Capabilities that affect outcomes pin to the run, not the worker.** If any worker can lease
   any run, an outcome-affecting capability (signer, gas policy, data source) supplied per-process
   makes the run non-deterministic / non-replayable. Capabilities must split into *operational*
   (any worker supplies freely) and *semantic* (pinned to the run, verified against certified
   expected identity, replayed identically).

These three reduce to one principle: **the transaction boundary is your coordination;
everything outside it needs an explicit durable lifecycle + idempotency + reconciliation.**

## 5. Code-grounded findings (current as of R2)

Read for R2: `crates/storages/stream-store-postgres/{migrations/0001_run_store.sql,src/run_store/*}`,
`crates/kernel/store/src/v1/*`, `crates/states/evm-contracts`, `crates/adapters/evm-contracts`,
`crates/transports/evm`, `crates/kernel/spec`, `crates/app`, `bin/{cli,rest-api}`.

### 5a. Safety is still complete — the store *is* the fabric (re-mechanized)

Concurrent execution of one run across any number of processes/sessions remains safe, enforced
by Postgres rather than process topology. The mechanisms changed in the refactor:

- **Per-run serialization.** Each append takes a transaction-scoped `pg_advisory_xact_lock`
  keyed on a hash of the run id (`run_store/stream.rs:54` `lock_run_tx`, class `0x4d465201`),
  with the `commits` primary key `(run_id, seq)` and `expected_next_seq` as the backstop. The
  old `typed_run_heads … FOR UPDATE` row is gone; head is `MAX(seq) FROM commits`
  (`stream.rs:25-52`). Because the lock is *xact-scoped*, it is held only for the append, not
  across node think-time — so duplicate *work* by two drivers is possible, but duplicate
  *commit* is not.
- **Commit-key idempotency.** `commits UNIQUE (run_id, commit_key)`; a re-presented commit with
  the same fingerprint returns `Idempotent`, a different fingerprint returns `CommitConflict`
  (`run_store/append.rs:19-55`; migration `:47`). Check-lock-recheck guards the race.
- **Side-effect fencing intact.** `ClaimTakenOver` + `claim_fencing_token` + `claim_generation`
  now live in `kernel/store/src/v1/{projection.rs:36,412 , resource_lanes.rs:42,80 ,
  mod.rs:2179-2220,2497-2509}`. Ownership can transfer and fence a stale holder's ledger writes.
- **New hardening: DB-enforced append-only authority.** Every authority table
  (`commits`, `run_events`, `artifact_*`, `store_metadata`, `run_observation_cursors`) has a
  `mfm_reject_authority_mutation` trigger that raises on UPDATE/DELETE/TRUNCATE (migration
  `:12-18,200-230`). A per-provisioning `store_epoch` singleton (`:20-31`) scopes cursors. The
  append-only invariant is now a database guarantee, not just a contract convention.

**Elegant consequence (unchanged): you can run completely leaseless and stay correct.** A dead
worker mid-step simply didn't commit; optimistic concurrency makes its late write lose. Leases
remain a future efficiency/liveness optimization, never a correctness requirement.

### 5b. The three gaps, re-judged against current code

- **Gap 3 (semantic vs operational capabilities) — essentially closed (unchanged).** The
  semantic/runtime config split + admission-time identity verification still make any worker
  interchangeable, or admission fails closed. Needs *naming* as the multi-worker determinism
  guarantee.
- **Gap 2 (fencing) — mechanism present, trigger now partial.** Ledger-write fencing is intact.
  Liveness gained a real trigger *for the waiter queue*: `expire_stale_waiters_tx` reaps waiters
  whose 60s lease lapsed (`resource_lanes.rs:163-177`, `RESOURCE_LANE_WAITER_LEASE_SECS`
  `mod.rs:92`). What's still missing: a trigger that decides an *active lane holder* (in-flight
  side effect) is dead and initiates `ClaimTakenOver`, and any execution-level lease at all.
- **Gap 1 (nonce) — narrowed to reconciliation only.** Resolved that the EVM submit declares
  `ResourceClaim::exclusive` on a per-signer-account nonce lane (`states/evm-contracts/src/lib.rs:89,107`
  `account_nonce_resource_claim` → `ResourceClaim::exclusive`; adapter matches
  `ResourceClaimSpec::Exclusive` at `adapters/evm-contracts/src/lib.rs:1622`; claim kinds are
  mandatory and hash-defining, `kernel/spec/src/lib.rs:1689-1731,4259`). So per-account
  serialization is intentional and real. The remaining hole is precisely **reconciliation on
  takeover**: nonce is still read fresh at prepare via `eth_getTransactionCount`
  (`transports/evm/src/lib.rs:469`) with no landed-vs-never-sent reconciliation path.

### 5c. The missing layer now — execution dispatch + reconciliation

1. **Resource-lane admission — sharded and fair (was the "scaling ceiling"; now mostly done).**
   The global lock is gone. Admission takes per-lane `pg_advisory_xact_lock` on only the lanes a
   commit touches (`resource_lanes.rs:32-45` `lock_resource_lane_tx` class `0x4d465202`;
   touched-lane set in `append.rs:70-71`). A `resource_lane_waiters` FIFO queue gives fair
   admission: per-lane monotonic tickets from `resource_lane_waiter_counters`, a head-of-line
   pre-gate (`resource_lane_fifo_pre_gate_tx` `resource_lanes.rs:134-161`), and lease-expiry
   reaping of dead waiters. **Residual:** active held-lane state is still folded from the full
   history of `resource_lane:%` events on every commit/read
   (`projections.rs:87-104` `load_resource_lane_state_tx`, fold `:106-206`) — O(total lane
   transitions), no global lock but still unbounded read work. The remaining optimization is an
   **incremental materialized lane-state projection**.
2. **Cross-run discovery — read/observe done, execution dispatch still open.**
   - *Observation/watch (done):* `RunObservationStore` is a durable, cursored, globally ordered
     change feed. Watch rows are ordered by `(append_xid, commit_sort_key)` past a server-side
     cursor, bounded by a sealed-snapshot `frontier_xid = pg_snapshot_xmin(...)` to avoid xid
     visibility races, and woken by `LISTEN/NOTIFY mfm_run_observation` (`observations.rs`:
     watch `:234-268`, list `:204-232`, listener `:104-134`, notify `:194-202`, cursors
     `:379-461`, frontier `:185-192`; channel `mod.rs:89`). Exposed via
     `app::read_run_observations` ("observation-only", `app/src/lib.rs:1157`),
     REST `GET /v1/runs?cursor&limit&wait_ms` (`bin/rest-api/src/lib.rs:502`), and CLI
     `mfm run list --watch` (`bin/cli/src/commands/run/list.rs`).
   - *Execution dispatch/claim (open):* nothing lets a disposable worker say "find a run with a
     runnable frontier, lease it to me, and let me drive it." Observation reports only
     `Started`/`Completed`, not runnable/blocked granularity; the run advisory lock is
     per-commit, not a durable execution lease across node think-time. So workers can be
     *informed* by the watch feed but not *dispatched* or given exclusive execution tenure. This
     is now the single biggest gap for the multi-agent *execution* vision.
3. **Nonce reconciliation on takeover (open, unchanged).** With per-account `Exclusive` lanes in
   place, a taken-over ambiguous ledger must read chain truth (`getTransactionCount` / receipt)
   to decide landed-vs-never-sent before the account can progress. The
   `submission-observed / unknown / not-submitted` slot models it; the reconciliation path is
   not implemented.

## 6. Roadmap (R3) — #1 decided, #2–#5 a decision agenda

Ratify the doctrine (§2). The execution-safety model stays **leaseless-optimistic** (the store
already covers it; every lease below is liveness only). Roadmap #1 is now decided; the rest is a
set of decisions to make before building.

### 6.1 DECIDED — #1: the unified admission lane + execution discovery

**Discovery** reuses the existing observation feed (cold-start `list` of non-terminal runs, then
NOTIFY-woken `watch`). **Effort-dedup** and **resource-lane fairness** unify into one generalized
primitive — the **admission lane** — replacing both the bespoke `resource_lane_waiters` machinery
and the separately-proposed execution-claim table.

Governing principle: **the primitive owns admission *order* + an operational *lease*; the caller
owns *authority*. They never share a row.** The shared table is liveness-only and structurally
cannot be authority.

One table-pair, one `admit(mode)` code path; a `class` discriminator *binds* the mode:

- `admission_lane(class, lane_id) PK, next_ticket, holder_token, lease_expires_at` — holder-lease
  columns used by `NowaitSkip` only.
- `admission_waiter(class, lane_id, ticket) PK, token, status, lease_expires_at` — used by
  `WaitFifo` only.
- All functions take `&mut Transaction` → the same code runs inside a commit txn (resource lanes)
  and standalone (execution dispatch). Shared *down* into a common component, never called
  *across* layers.

Two instantiations exercise disjoint subsets, so neither stores the other's authority:

- **Resource lane (`WaitFifo`)** — uses only waiter rows. Admission gates the commit, so
  "admitted" and the `ResourceLaneClaimed` authority event are atomic in one txn; there is never a
  steady-state holder-lease to store. The caller supplies `holder_is_free` from the event fold.
- **Execution (`NowaitSkip`)** — uses only the lane row's lease; writes no waiters; the lease *is*
  the holder, harmless because safety = the store's per-run lock + `expected_next_seq`.

Non-negotiable invariants (from a three-architect review):
1. **Mode is bound to class** — `NowaitSkip` is unrepresentable on a FIFO lane and vice-versa.
2. **The table is liveness-only** — no safety decision branches on it; a false reap or two
   simultaneous "holders" cost duplicate work, never corruption.
3. **The lane-state projection (#5) stays a SEPARATE event-derived view** — never this table;
   merging them is the one move that manufactures dual authority.

Cost: a **refactor** (generalize + relocate today's `resource_lane_waiters`), not a greenfield
add. Dividend: one dispatch loop services both runnable runs *and* newly-head-of-line lane waiters
(a lane release → NOTIFY → dispatcher drives the head-of-line run); any future coordination need is
a new `class` + mode.

### 6.2 Decision agenda — #2–#5

**D1–D4 decided** (2026-06-24 discussion); D5–D11 keep their recommended defaults.

**#2 — External-effect reconciliation (critical path; the doctrine's hard core).**
- **D1 nonce concurrency — DECIDED: serial per-account, sequential-within-hold.** Exclusive lanes
  give deterministic nonce assignment; a holder may pre-assign a contiguous block and submit
  *sequentially* (one in-flight → point-wise recovery). Pipelined batches (multiple in-flight) are
  deferred — they reopen range/gap recovery. A batch is N side-effect nodes sharing one lane hold,
  preserving the one-mutation-per-side-effect effect-class rule.
- **D2 reconciliation anchor — DECIDED: anchor on the deterministic idempotency identity, never the
  nonce; no exclusive-ownership assumption.** Exclusivity is an *availability* expectation, not a
  *safety* assumption; a foreign-occupied nonce is a detectable terminal ("superseded"), never a
  double-spend. **Representation (reuses existing machinery — no new authority/event types):**
  - the anchor is the existing typed `IdempotencyKey`/`idempotency_input`
    (`program/src/lib.rs:1506,1531`, recorded at intent in `store/v1/mod.rs:3476-3480`); the EVM
    side-effect's `IdempotencyInput` must deterministically pin the signed-tx hash. The operation's
    "submission responsibility" = supplying that typed input.
  - the output is the existing submission slot `SubmissionUnknown → SubmissionObserved |
    NotSubmittedProven` (`events.rs:427-431`, `store/v1/saga.rs:216-264`).
  - the procedure is one new contract — a `SideEffectRecoveryVerifier` shaped like
    `SideEffectReplayVerifier` (`replay/src/lib.rs:470`), identified and domain-implemented, but
    answering from a live **read** capability instead of recorded evidence (so any fungible worker
    can run it). Reconciliation is to recovery what the replay verifier is to replay; every future
    side-effecting domain implements the same verifier against its own idempotency key + read.
- **D3 finality — DECIDED: two layers.** The *requirement class* (receipt-confirmed vs finalized)
  is **semantic**, the designer's choice, hash-defining in `SideEffectContractSpec`
  (`spec/src/lib.rs:1740`, beside `resource_claim`). The *numeric depth* per chain is **runtime**
  config but **recorded as evidence** at determination time, so replay verifies "final under depth
  D" without a live chain. Reorg-risk acceptance is encoded in the chosen class.

**#3 — Liveness & recovery.**
- **D4 holder lease + death trigger — DECIDED: every admission/holder/waiter row carries
  `lease_expires_at` + heartbeat renewal; expiry routes to recovery, never a silent release.** Today
  only the *waiter* has a lease; the unified admission lane adds a holder lease. Consequence by
  phase: waiter expiry → drop from queue; pre-boundary holder expiry → interrupt; **post-boundary
  holder expiry → takeover (bump fencing token / `ClaimTakenOver`) → reconciliation (D2)**, and the
  lane stays held until the side-effect terminalizes. False reap of a slow-but-alive holder is safe
  — fencing rejects its late writes and the D2 anchor prevents double-submit, so it costs wasted
  reconciliation work, never corruption.
- **D5 recovery model:** recovery is **ordinary fungible dispatch** of a run in a recovering state
  (default); it needs only a read capability, which any worker has.
- **D6 timed wakeups:** add a `due_at` signal to the dispatch layer for confirmation polling /
  backoff — the one dispatch piece deferred in §5; D2 reconciliation needs it.

**#4 — Execution-tenure policy.** D7 **per-run** lease; D8 **release tenure on long external waits**
(a confirmation wait hands the run back to recovery/re-dispatch rather than pinning a worker); D9
single feed / oldest-first now, shard by `hash(run_id)` later. (Recommended defaults; revisit under
load.)

**#5 — Performance / ops.** D10 materialize the lane-state projection (rebuildable cache, not
authority; the §5c residual); D11 size pools to worker count (pooler-safe). Sequence after #2/#3.

**The reconciliation engine — D2+D4 are one machine:** holder-lease expiry (D4) is the *trigger*;
the recovery verifier on the idempotency anchor (D2) is the *procedure*; the certified finality
class (D3) decides *when terminal*; serial-exclusive lanes (D1) make the anchor deterministic.
**§9 specifies the full side-effect design and refines this:** verification becomes a paired
framework *state*, which generalizes the verifier to every side effect and *removes* the bespoke
recovery-verifier and the resource-lane holder-takeover (see §9.7–9.8). Read §9 as the authoritative
side-effect spec; the D2–D4 records above are the decision history it builds on.

## 7. Verification items — resolved in R2

1. **EVM submit `ResourceClaim` kind → resolved: `Exclusive` per signer account.** Per-account
   nonce safety is intentional, not incidental to the old global lock.
   (`states/evm-contracts/src/lib.rs:107`.)
2. **EVM recovery chain reconciliation → resolved: not implemented.** Only read-at-prepare
   exists; folded into roadmap #2.

No open verification items block ratifying the doctrine. The execution-ownership-granularity and
dispatch-source questions raised here are now decided/tracked in §6: dispatch rides the existing
observation feed (§6.1), and granularity is decision D7.

## 8. Next step

Roadmap #1 is decided (§6.1), the #2 reconciliation engine is designed (§6.2: D1–D4), and the
side-effect design is now fully specified (§9 — verification as a paired framework state, which
*simplifies* D2/D4). Two tracks from here: (a) ratify the doctrine (§2) + land §6.1 / §6.2 / §9 as a
normative `docs/design.md` "Process-Fungible Execution" section; (b) start the build design — the
`FrameworkNodeSpec::SideEffectVerify` node + the `SideEffectVerifier` contract + the admission-lane
refactor of `resource_lane_waiters`. Open items to confirm: D5–D11 defaults and the EVM
`IdempotencyInput` shape that pins the signed-tx hash (ledger-continuity §9.5 and
deterministic-signing §9.7 are now settled). Still no code until the direction is signed off.

## 9. Side-effect design — verification as a paired framework state

Authoritative spec for how side effects verify and recover. It refines §6.2 (D2/D3/D4): the
recovery-time reconciliation sketched there generalizes into a normal verification step, which in
turn collapses most of the bespoke recovery machinery.

### 9.1 The insight

Verification — "did the mutation land (receipt), and is it final (confirmation)?" — is intrinsic to
*every* side effect, not a recovery special case. The happy path and the recovery path run the
*same* chain read against the *same* anchor; recovery is just "the same verification, picked up by
another worker." So there is one verification mechanism, not a separate recovery verifier.

The phases already exist. The side-effect ledger runs
`IntentPersisted → Claimed → InvocationPrepared → InvocationStarted` (the uncertainty boundary)
`→ {SubmissionObserved | SubmissionUnknown | NotSubmittedProven} → ReceiptObserved →
ConfirmationObserved → (Failed)` (`events.rs:413-439`; `SideEffectPhase` in `store/v1`). **Receipt
vs confirmation are already distinct phases.** What's missing is only *who drives the post-boundary
phases* and *how the required level is configured*.

### 9.2 The design — split the side-effect node in two

Lower every side-effect node into a **pair**:

- **Submit node** (the domain side-effect state): drives `Intent → Claim(lane) → Prepared →
  InvocationStarted → Submission*`. Records the deterministic anchor (idempotency key pinning the
  signed-tx hash) *before* the boundary, crosses the boundary once (the single external mutation),
  ends at "submitted." It is the only node that mutates (effect class: apply-side-effect).
- **Verify node** (a framework node, 9.3): drives `Submission* → ReceiptObserved →
  ConfirmationObserved → terminal` to the configured level, then releases the lane and binds the
  verified output. It only reads (effect class: read).

The split point is the existing uncertainty boundary. The pair shares **one ledger, one anchor, one
resource-lane hold** (9.5).

### 9.3 The verify node is a framework node, inserted at lowering

Add `FrameworkNodeSpec::SideEffectVerify` beside `Bridge / PublicOutputRender /
ProjectRetentionManifest / CompleteRun / ResolveSagaTerminal` (`spec/src/lib.rs:1758`). Lowering
inserts exactly one verify node immediately after each side-effect node — the same way retention /
completion / resolve-saga nodes are framework-inserted and cardinality-checked in
`certify/src/framework_lifecycle.rs` ("expected exactly one … framework node"). The pairing is
therefore **automatic** (the author writes one side-effect state; lowering produces the pair),
**certified & static** (topology stays hash-defining; no runtime topology change), and **zero
per-operation LOC** (one framework node kind serves every side effect in every domain).

### 9.4 Verification level is configured on the side-effect contract

Add a hash-defining field to `SideEffectContractSpec` (beside `resource_claim`, `spec/src/lib.rs:1740`):

    verification: Receipt | Finalized

`Receipt` terminalizes at `ReceiptObserved` (accepts reorg risk); `Finalized` requires
`ConfirmationObserved` past the chain's configured depth. The *class* is semantic/certified (the
designer's choice, per downstream need and reorg tolerance); the *numeric depth* per chain stays
runtime config but is recorded as evidence at determination time (D3). There is no "skip" level —
the lane and run cannot terminalize without a determination.

### 9.5 One ledger, one lane, across two nodes

The Exclusive resource lane (the wallet/nonce lane) is **claimed by the submit node and released by
the verify node** at terminal. Between them the lane is held by the *ledger* (non-terminal), not by
any worker. This is the decided sequential-within-hold model (D1): the wallet stays held until the
tx is confirmed to the required level, so no second nonce is issued against a still-pending tx.

**Decided: one ledger across the pair.** The verify node continues the producing node's ledger —
same ledger key, anchor, and lane, with phases flowing across the two driving nodes (not a linked
second ledger + lane handoff). Consequence: the ledger key must be **stable across the pair and not
tied to a node attempt** — lowering records the shared ledger identity in the verify node's spec
(the certified link), so both nodes address the same ledger. This makes explicit the decoupling of
**attempt lifecycle** (per-node execution bookkeeping) from **ledger lifecycle** (the side-effect's
durable phase machine): the lane is claimed in the submit commit and released in the verify terminal
commit, on one ledger.

### 9.6 The verifier pair: live vs replay

- **Live** — a new `SideEffectVerifier` contract (the only genuinely new contract), domain-
  implemented, that reads external truth by the anchor (`getTransactionReceipt` + depth /
  `getTransactionCount`) and emits the existing `ReceiptObserved / ConfirmationObserved /
  NotSubmittedProven` evidence. It needs only a **read** capability, so any fungible worker can run
  it. Mostly *relocated* receipt-poll code from today's EVM adapter, not new logic.
- **Replay** — the existing `SideEffectReplayVerifier` (`replay/src/lib.rs:470`) checks recorded
  evidence from recorded facts, no live IO; receipts/confirmations already carry a `replay_verifier_id`.

Live verifier produces evidence; replay verifier checks it — the same live-vs-replay symmetry MFM
uses everywhere.

### 9.7 Recovery falls out for free — the payoff

Because the verify node is an ordinary frontier node:

- A worker that dies mid-verification leaves a **runnable verify node**; ordinary execution dispatch
  (roadmap #1) + execution-lease expiry re-drives it. No bespoke recovery verifier.
- A worker that dies mid-submission leaves an **open submit attempt**; ordinary attempt recovery
  re-runs it. Safety floor: the tx is pinned to one nonce, so **at most one tx can land — never a
  double-spend**. **Decided: require deterministic signing (RFC 6979)** — re-sign → same signed
  payload → same hash → the broadcast is idempotent (the chain dedups). This is a **signer-contract
  requirement, and it is enforced, not assumed:** the anchor (expected hash) is recorded before the
  boundary, and recovery **asserts the re-signed hash equals the recorded anchor** — a
  determinism-violating signer fails closed there instead of double-submitting. The set-of-hashes
  fallback is dropped.
- The wallet lane releases **only** when the verify node terminalizes — so a dead worker never
  wedges the wallet; another worker just finishes the verify node.

**Key invariant (replaces the holder-death sweeper):** a non-terminal side-effect ledger always has
a corresponding runnable (or `due_at`-scheduled) frontier node. The scheduler guarantees it,
dispatch drives it, the lane releases at terminal. No holder-lease on the wallet, no takeover machine.

### 9.8 What this collapses in §6.2

- **D2** — the "recovery verifier" becomes the general live `SideEffectVerifier` (9.6); anchor
  unchanged (idempotency key pinning the tx hash).
- **D3** — the finality decision becomes the `verification: Receipt | Finalized` contract field
  (9.4); unchanged in substance.
- **D4** — the bespoke active-holder takeover for resource lanes is **no longer needed**. The wallet
  lane is ledger-lifetime, released by the verify node; recovery is ordinary dispatch (9.7). The
  only worker lease that remains is **execution tenure** (roadmap #1, NowaitSkip), whose expiry just
  means re-dispatch. "Lease expiry routes to recovery, never silent release" still holds — for
  execution tenure; the resource lane simply isn't worker-leased.
- **D6** — `due_at` is still needed: a verify node awaiting confirmations is a `blocked-until-due_at`
  node. With D8 the worker releases execution tenure during the wait; at `due_at` a worker re-picks
  the verify node, polls, and either terminalizes or re-blocks.

### 9.9 New vs relocated (the LOC-honesty check)

- **New:** one `FrameworkNodeSpec::SideEffectVerify` variant + its runner; one `SideEffectVerifier`
  live contract (+ one EVM impl); one `verification` enum/field on the contract; the lowering
  insertion + cardinality check; the scheduler "non-terminal ledger ⇒ runnable node" invariant.
- **Relocated/reused:** the submission slot, the `ReceiptObserved`/`ConfirmationObserved` phases, the
  replay verifier, the idempotency key, the resource lane, the framework-node-insertion pattern.
- **Removed:** the bespoke recovery verifier and the resource-lane holder-takeover machine.

Net: more capability (uniform, configurable verification for every side effect in every domain) with
*less* bespoke machinery than the D2/D4 sketch.

### 9.10 Bonus alignment

- **Saga obligations:** the verify node's terminal *is* the obligation signal — `Confirmed` ⇒ the
  forward effect definitely happened ⇒ a compensation obligation may be owed; `NotSubmittedProven` ⇒
  no obligation. Verification feeds saga classification exactly (`docs/saga.md`).
- **One-mutation-per-node:** improved — the submit node performs exactly one mutation, the verify
  node none. The effect-class rule gets cleaner, not strained.

## Appendix — key code anchors (R2)

| Concern | Location |
|---|---|
| Per-run append lock (`pg_advisory_xact_lock`) | `run_store/stream.rs:54` (`lock_run_tx`); head `MAX(seq)` `:25-52` |
| Commit-key idempotency | `run_store/append.rs:19-55`; migration `0001_run_store.sql:47` |
| `expected_next_seq` | `run_store/mod.rs:157`; staging in `kernel/store` |
| DB-enforced append-only authority | migration `:12-18` (`mfm_reject_authority_mutation`), `:200-230` (triggers) |
| `commits` table (authority + global order) | migration `:33-58` (`append_xid`, `commit_sort_key`) |
| Per-lane advisory lock (touched lanes only) | `run_store/resource_lanes.rs:32-45`; `append.rs:70-71` |
| FIFO waiter queue + lease reaping | migration `:151-186`; `resource_lanes.rs:134-310`; lease `mod.rs:92` |
| Held-lane fold (residual O(history)) | `run_store/projections.rs:87-104,106-206` |
| Side-effect fencing token / generation / `ClaimTakenOver` | `kernel/store/src/v1/projection.rs:36,412`; `resource_lanes.rs:42,80`; `mod.rs:2179-2220,2497-2509` |
| Observation/watch (cursored, ordered, NOTIFY) | `run_store/observations.rs:104-268,379-461`; channel `mod.rs:89`; cursor table migration `:188-198` |
| Observation surface (app/REST/CLI) | `app/src/lib.rs:1157`; `bin/rest-api/src/lib.rs:502`; `bin/cli/src/commands/run/list.rs` |
| EVM exclusive nonce lane | `states/evm-contracts/src/lib.rs:89,107`; adapter `adapters/evm-contracts/src/lib.rs:1622`; spec `kernel/spec/src/lib.rs:1689-1731` |
| Nonce read-at-prepare (no reconciliation) | `transports/evm/src/lib.rs:469` (`eth_getTransactionCount`) |
| Resource-claim kinds + lane semantics | `docs/saga.md:150-167`; `kernel/spec/src/lib.rs:1689-1731` |
| Side-effect uncertainty boundary + submission slot | `docs/design.md:444-487` |
