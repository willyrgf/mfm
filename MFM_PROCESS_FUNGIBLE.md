# MFM: Process-Fungible Execution

Status: **working design discussion — not yet normative.** Captures the architecture
dialogue plus code-grounded analysis on a question the platform was built around but never
decided explicitly: is MFM one service running many runs, or many processes each running
runs? Conclusion: neither framing is architectural — **processes are fungible workers over
Postgres-owned coordination.** This document is the resume point for turning the conclusion
into a normative `docs/design.md` section and an execution-layer roadmap.

Date opened: 2026-06-23.
Revised: 2026-06-23 (**R2**, after a large Postgres refactor landed — see §0);
2026-06-24 (**R3**, roadmap #1 decided — the unified admission-lane design, §6.1; remaining
decisions in §6.2).
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

**#2 — External-effect reconciliation (critical path; the doctrine's hard core).**
- **D1 nonce concurrency:** keep **serial per-account** `Exclusive` lanes (recommend) vs pipelined
  nonces with gap recovery (defer).
- **D2 reconciliation algorithm + idempotency anchor:** how takeover decides landed-vs-never-sent
  (deterministic signed-tx hash + receipt + `getTransactionCount`), what on-chain fact identifies
  "our" tx, and the **exclusive-signer precondition** (does MFM assume it solely owns the account,
  or handle a nonce occupied out-of-band?).
- **D3 finality / reorg policy + recorded evidence:** confirmations-to-final (per-chain config) and
  what typed evidence terminalizes each `submission observed/unknown/not-submitted` case so replay
  can verify it.

**#3 — Liveness & recovery.**
- **D4 side-effect holder-death trigger:** the resource-lane holder is an *event* (authority), not
  a lease, so waiter reaping does not free it. Decide the sweeper linking a dead execution lease to
  side-effect `ClaimTakenOver`/recovery (recommend: dispatch surfaces open side-effect ledgers with
  no live driver).
- **D5 recovery model:** recovery is **ordinary fungible dispatch** of a run in a recovering state
  (recommend) vs a dedicated recovery-worker class.
- **D6 timed wakeups:** add a `due_at` signal to the dispatch layer for confirmation polling /
  backoff — the one dispatch piece deferred in §5; reconciliation (D2) needs it.

**#4 — Execution-tenure policy (closes out #1).**
- **D7 lease granularity:** **per-run** now (recommend); per-attempt (intra-run parallelism) later.
- **D8 long external waits:** a worker waiting minutes on a tx confirmation **releases execution
  tenure** and lets recovery/re-dispatch own it (recommend) vs holding + heartbeating.
- **D9 fairness / sharding:** defaults now (oldest-first over the feed, single feed); shard by
  `hash(run_id)` later.

**#5 — Performance / ops.**
- **D10 materialize the lane-state projection** (rebuildable cache, not authority; the §5c residual);
  sequence after #2/#3.
- **D11 connection budget at N workers** — pool sizing + pooler choice. We are pooler-safe
  (xact-scoped locks + lease tables; no session advisory locks), so transaction-mode poolers are OK.

Critical path: **#2** is the biggest remaining correctness gap and exactly where the doctrine
always said the hard problem lives. #3 makes the system self-healing; #4 is policy you can default
and revisit; #5 is scale.

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

Roadmap #1 is decided (§6.1). Two tracks from here: (a) ratify the doctrine (§2) as a normative
`docs/design.md` "Process-Fungible Execution" section and land §6.1 + §6.2 as the execution-layer
roadmap; (b) work the §6.2 decision agenda, starting with the critical path — **#2, the
external-effect reconciliation seam** (the doctrine's hard core and the biggest remaining
correctness gap). The first calls to make are D1 (nonce concurrency model) and D2 (reconciliation
algorithm + exclusive-signer precondition). Still no code until the direction is signed off.

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
