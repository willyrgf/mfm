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
durable execution fabric in which processes are fungible.** No process owns a run, and any worker
that **matches a run's bound executables** can drive it — correctness lives in the database
transaction boundary, never in process topology. (v1 is invoker-driven and manual-resumable; full
automatic disposable-worker recovery is deferred — §6.5, §12.)

Most of the substrate already exists (per-run serialization, append-only authority, sharded
resource lanes, a durable observation/watch feed). The remaining work is three coherent pieces:

1. **A unified admission-lane primitive** — one durable mechanism for fair, fenced, lease-reaped
   exclusive admission that serves both *resource lanes* (wallet/nonce contention) and *execution
   claims* (which worker drives a run), plus any future coordination need.
2. **Content-addressed run identity + execution claim** — the invoker expands and certifies the
   runtime state graph and derives a **semantic** run id from it (certified graph + resolved semantic
   inputs/config + admission policy + trust scope; **executables are not in the identity**). Identical
   work converges on one run (idempotent by default); a distinguishing input forces a distinct run.
   The execution claim coordinates active drivers; a worker may only **drive** a run whose bound
   executables it matches (a determinism guard, not run identity) — so a different build
   *attaches/reports*, never duplicates and never double-executes. **v1 is invoker-driven and
   manual-resumable**: automatic disposable-worker recovery (expired-claim takeover, `due_at`) is the
   deferred AC/DC + saga effort (§6.5, §12).
3. **Side-effect verification as a paired framework state** — every side-effect node lowers into a
   `submit` node plus an auto-inserted `verify` framework node. This productizes receipt/finality
   verification uniformly *and* makes external-effect reconciliation follow the ordinary frontier
   once a run is live-driven or manually resumed, collapsing the bespoke reconciliation machinery
   the earlier design implied.

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

- **Random run identity, no execution claim, and no automatic takeover.** Execution is invoker-driven
  (the process that starts a run drives it) — fine — but default run ids are random today
  (`mfm_app::new_run_id()` hashes a UUIDv4, and CLI/REST use it when no run id is supplied). That
  means two identical launches create two independent runs. There is also no durable execution claim
  to coordinate active drivers of the same run, and no automatic way for another process to take over
  a dead driver's run (deferred to the AC/DC + saga recovery effort). The observation feed is
  read/observe only.
- **No external-effect reconciliation.** A worker that submits a transaction and dies leaves an
  ambiguous nonce; nothing reads chain truth to resolve landed-vs-never-sent.
- **Verification is not productized.** Receipt/finality checking is embedded ad hoc inside the
  side-effect runner rather than configurable per state and reusable across domains.

## 3. Goals and non-goals

### Goals

- A correctness model that depends only on the Postgres transaction boundary, never on process
  identity or lifetime.
- Many workers (CLI, daemon, REST host, agent session) coordinating only through Postgres. Processes
  are **safe to lose** (a crash never corrupts; safety = the store), with **at-most-once MFM-authored
  mutation per deterministic anchor** and evidence-backed terminalization/compensation. In v1, *making
  progress* after a driver is lost requires **manual `run resume`** — automatic at-least-once recovery
  is the deferred AC/DC + saga effort (§6.5/§12). Not a full AC/DC / exactly-once claim (see
  `docs/saga.md`).
- One reusable coordination primitive for all exclusive-admission needs.
- Uniform, configurable side-effect verification (receipt vs finality) across all domains.
- External-effect recovery behavior, once a run is live-driven or manually resumed, that requires no
  bespoke machinery beyond ordinary frontier driving.

### Non-goals

- Pipelined nonces (multiple in-flight transactions per account). Deferred; serial-within-hold is
  sufficient and far simpler (DEC-1).
- Cross-run priority/fairness beyond observation order, and feed sharding. Deferred (DEC-9).
- Multi-lane admission (one claim spanning several lanes, with all-or-nothing acquisition, deadlock
  ordering, and starvation policy). v1 is single-lane only — what the **code** provides
  (`single_lane_claim_admission`); `docs/saga.md` currently overstates it as a multi-lane contract
  (`saga.md:161`, "every requested lane acquired together") and must be **narrowed to single-lane at
  ratification** (DEC-33, review #6).
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
  `pg_advisory_xact_lock` on the run id (`run_store/stream.rs:54`), backed by the `commits` PK
  `(run_id, seq)` and a **caller-supplied `expected_next_seq` CAS** validated at staging — a
  precondition, not a stored column (`run_store/mod.rs:157`; `StaleExpectedNextSeq` in `store/v1`).
  Two workers driving one run: one commits each step; the other no-ops (commit-key idempotent) or
  retries. **Leaseless is already safe.**
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

A run is persisted authority (certified spec) plus a current frontier. Any worker that **matches the
run's bound executables** — and has the registry, capabilities, and store authority — may drive it.
Fungibility of *driving* is **executable-scoped**, not unbounded: the drive/resume path rejects a
worker whose runner/adapter executables differ from those in `RunAdmitted` (a **drive-time
determinism guard, not part of run identity** — DEC-29; `binding.rs:113`, `history.rs:212`). v1 is
also **manual-resumable**, not auto-recovering (§6.5). Safety is the store (§5). A **worker lease is
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
- **Execution claim (`NowaitSkip`, lane_id = run id)** — uses only the lane row's lease; writes no
  waiters; the lease *is* the holder, harmless because safety = the store's per-run lock +
  `expected_next_seq`. The content-addressed run id is the **dedup identity**; try-acquire is the
  **active-driver coordination** gate: a second invoker of the same run finds an existing holder and
  attaches/reports instead of doing duplicate driver work (§6.3). Crucially, the execution lease
  confers **no** authority to cross a side-effect boundary — that gate is solely the per-side-effect
  nonce-lane claim + `claim_fencing_token`. So even two processes that both believe they hold the
  execution lease (after a false reap) cannot double-submit: only a valid side-effect claim-holder
  may submit, and nonce serialization + the claim-generation fence make at most one mutation land.
  Execution lease = dispatch liveness; side-effect claim = mutation safety.

Non-negotiable invariants:

1. **Mode is bound to class** — `NowaitSkip` is unrepresentable on a FIFO lane and vice-versa.
2. **The table is liveness-only** — no safety decision branches on it; a false reap or two
   simultaneous "holders" cost duplicate work, never corruption.
3. **The materialized lane-state projection (§6.5 / WS-E) stays a SEPARATE event-derived view** —
   never this table; merging them is the one move that manufactures dual authority.

**Placement.** The primitive is exposed as a **typed contract in `kernel/store`** (lane id,
`admit(mode)`, lease, fence, head-of-line) with a **Postgres implementation in `storages`**.
App/runtime dispatch consumes the *contract*, never the Postgres-specific SQL — preserving the
existing store-contract/implementation boundary (the same way `RunEventStore` is consumed today).
Note (LOW): today's advisory-lock key is a 32-bit truncation of SHA-256 (`run_store/stream.rs:66-74`),
so distinct lanes can falsely contend at scale; the new primitive should use the 64-bit
`pg_advisory_xact_lock(int8)` form derived from the lane fingerprint (or an equivalent full-lane
lock). Safety is unaffected — it holds via PK + CAS.

**Scope (v1) — documents the existing single-lane scope, not a new constraint.** The primitive ships
exactly two lane classes: single-lane `WaitFifo` (resource lanes; one `(namespace, key)` per claim —
the code provides this via `single_lane_claim_admission`; `docs/saga.md` overstates it as multi-lane
and is reconciled to single-lane at ratification, DEC-33) and `NowaitSkip` (execution). Multi-lane
admission stays deferred (§3 Non-goals). The point is only that the RFC's prose must not *imply*
multi-lane atomicity the design never had.

### 6.3 Execution: content-addressed run id, invoker-drives, and manual resume

MFM is **invoker-driven**, not pool-dispatched: `mfm run --op xyz` starts the state-machine runner
*in that process* and drives the run to completion or to a block. There is no pool of idle workers
polling a feed for work; the observation feed (§5) is **status/watch only** and never a dispatch
surface. One process may drive many runs concurrently; the correctness boundary is the run stream,
not the OS process.

Before admission, the launcher expands and certifies the requested work into the runtime state graph
the scheduler will actually execute. The default run id is a **semantic** content address over that
canonical work product:

```
run_id = hash(
  canonical certified graph (lowered node/cell/binding graph)
  + resolved semantic inputs / config
  + hash-defining admission policy
  + trust scope (DEC-31)
)
```

The projection **excludes** the run id itself, non-semantic launch spelling (raw entry-point name,
config-file bytes, local path, unresolved "latest"), the certified spec's audit/provenance fields
(`TypedExecutionSpecAudit` / `non_semantic_provenance`), **and the bound executable identities** — so
it is build-independent and **not** simply the full spec hash. Resolved *semantic* inputs/config **are**
in the preimage (different config/inputs ⇒ different run); only non-semantic spelling is excluded.
Remediation-linked specs carry a `forward_run_id` whose handling the projection must define (WS-B,
addresses MED-3). The **admission policy is resolved here against a pinned registry snapshot** —
finality-policy depth and any registry-resolved descriptors are baked to concrete values, with the
**registry-snapshot digest + resolver identity** folded into the preimage (DEC-3/DEC-36). So a registry
policy change yields a *different* run id: identical launches before and after the change do not
converge, and cannot expect different terminality from one run.

**Run identity is semantic; the executable binding is a drive guard (DEC-29).** Executable identity is
deliberately **out of run identity** — folding it in would make the same semantic work on a different
build a *distinct* run, which **double-executes** the side effect across a rolling upgrade (the nonce
lane serializes the two txs but executes both). Instead, executables stay where the code already
enforces them: a **drive-time determinism guard** — `validate_run_admitted_binding` rejects a worker
whose runner/adapter executables differ from the run's admitted ones (`history.rs:212`,
`binding.rs:113`). So *identical work* always addresses **one** run (no double-execution), and a
**different-build** launcher **attaches/reports** ("this run exists, admitted under build X — needs a
compatible-build driver, or pass an explicit run id for a new run"); it never creates a duplicate and
never hard-errors as a conflict. The first admitter's executables become the run's bound build.
Consequence (rolling upgrade): an in-flight run is driven/resumed only by a matching build until the
deferred AC/DC + saga effort adds cross-build takeover; a *completed* run is recognized, not re-run.
Today's implementation lacks all of this: `mfm_app::new_run_id()` hashes a random UUID and CLI/REST use
it by default; WS-B replaces that default with this derived run id.

**Idempotent by default, distinct-run on request (DEC-28).** Because the run id is the certified-graph
content address, two launches of *identical* certified work converge on **one** run — re-invoking
attaches to / reports the existing run rather than duplicating it. This is the multi-agent safety
default: if two agents or sessions independently request the same work, they coordinate onto one run
and cannot double-execute. To force a **distinct** run of identical work, the caller supplies a
distinguishing input — an explicit `run_id`, or a nonce/timestamp that flows into the certified graph
(e.g. a snapshot-as-of-block input). Most repeatable ops already carry a varying input (a resolved
block height or timestamp) that differentiates them, so dedup collapses only genuinely-identical
launches — which is exactly when convergence is desired.

**Trust scope is normative (DEC-31).** v1 scopes **one Postgres deployment to one trust domain**: all
launchers are co-authorized for the deployment's signers/capabilities, and the **trust scope is a
factor of the run-id preimage** (above). Convergence is therefore a safety win — co-authorized
launchers of identical work converge onto one run. `RunAdmitted` carries no per-launch principal, so
**cross-tenant isolation is a non-goal**: supporting mutually-distrusting tenants in one deployment
would require the launching *principal* (not just the trust scope) to enter the run-id preimage, the
claim lane, and admitted evidence — which would stop identical work by different principals from
converging. Out of scope here.

The execution claim is a `NowaitSkip` admission lane keyed on the derived run id. It does two jobs —
neither of which is discovery and neither of which creates run identity:

- **Active-driver coordination.** A second launch that derives the same run id sees the same stream.
  If a live driver holds the claim, it attaches to / reports that run instead of doing duplicate
  driver work. If two drivers race, the store's per-run lock, commit-key idempotency, and
  `expected_next_seq` remain the safety floor.
- **Recovery substrate.** The driver heartbeats the claim while running (renewing
  `lease_expires_at` forward). If it dies, the lease expiry makes the run visibly stale for future
  recovery machinery — without ever transferring *semantic* ownership (safety is still the store,
  §5).

The driver holds the run through blocks rather than handing it off:

```
expand + certify graph; derive semantic run_id
try-admit RunAdmitted(run_id, my_executables)            // on CommitConflict → run exists → attach
if run exists and my_executables != admitted:            // different build
    report "exists; needs a compatible-build driver"; stop
acquire_execution_claim(run) or attach-to-existing       // active-driver coordination
while frontier(fold(run)) is Runnable(t):
  renew_claim(); commit(t)                                // heartbeat + advance
  // a short Receipt-level side-effect wait (~1 block) is polled in-line while heartbeating
release_claim(run)                                        // terminal
```

**Dead-driver automatic recovery is deferred (to the AC/DC + saga effort).** The execution claim's
lease + expiry is only the *substrate* for recovery. The mechanism that automatically finds stale
runs, takes over expired claims, and reconciles ambiguous in-flight submissions is part of the
larger, separate AC/DC + saga recovery effort, not this RFC. v1 recovery is the existing explicit
operator path: `mfm run resume <run_id>` / REST resume. Consequence: if a driver dies after a wallet
lane is claimed but before the side-effect ledger reaches a proven terminal, that signer lane remains
held until the run is manually resumed and driven to the verify/reconciliation terminal. This is an
availability wedge, not a safety escape hatch; the lane is never timeout-released while the nonce is
ambiguous.

When automatic recovery is built, it should read the **claim table** for expired claims (a fresh
indexed read), not the snapshot-frontier-gated feed.

**Why no feed-driven discovery, and no `due_at` in v1 (review HIGH-2).** The observation feed is
gated behind `pg_snapshot_xmin` (`observations.rs:185-192`), so a pool that discovered work through
it would inherit a cluster-wide liveness lag. That lag is real for the *watch* feed but irrelevant
here: MFM does not dispatch through it — the invoker drives, manual resume is keyed by run id, and
future automatic resume reads the claim table.
Confirmation waits are short at the default `Receipt` level, so the driver simply holds +
heartbeats through them; `due_at` (re-waking a parked, undriven run) and tenure-release-on-wait are
**deferred** (DEC-6/DEC-8) to a long-`Finalized`-at-scale optimization, not a v1 mechanism.

### 6.4 Side-effect verification as a paired framework state

Verification — "did the mutation land (receipt), and is it final (confirmation)?" — is intrinsic to
*every* side effect, not a recovery special case. Lower every side-effect node into a **pair**:

- **Submit node** (the domain side-effect state): drives `Intent → Claim(lane) → Prepared →
  InvocationStarted → Submission*`, crosses the boundary once (the single external mutation), ends
  at "submitted." Effect class: apply-side-effect.
- **Verify node** (framework node): drives `Submission* → ReceiptObserved → ConfirmationObserved →
  terminal` to the configured level, then releases the lane and binds the verified output. Effect
  class: read.

It is **auto-inserted at lowering** — add `FrameworkNodeSpec::SideEffectVerify` beside `Bridge /
PublicOutputRender / ProjectRetentionManifest / CompleteRun / ResolveSagaTerminal`; lowering inserts
exactly one verify node after each side-effect node. Unlike the existing framework nodes — validated
as **singletons** (`certify/src/framework_lifecycle.rs`) — `SideEffectVerify` is **its own validation
family**: an N-cardinality **pairing invariant** (exactly one verify per submit; no orphan verify, no
submit without verify), **evidence-flow binding** (verify consumes the submit's epoch/anchor), and a
**post-rewrite single-producer check** (every downstream consumer of the re-pointed output cell
resolves to verify — §6.4e). Zero per-operation LOC for *authors*, but a real new certifier +
lowering path (WS-C).

This split hides four authority changes the review correctly surfaced; they are specified explicitly
below, not behind "paired framework state."

**(a) Two anchors at two layers — respects the state/signer boundary.** Reconciliation needs a
*submission anchor*, but the state cannot compute it: `idempotency_input(input, intent)` is
deterministic and computed in the state *before* `submit` (`program/src/lib.rs:1546`), and states
must not sign (`docs/architecture.md:143`). So split them:
- *Semantic idempotency* (state, pre-submit) — the existing `idempotency_input → IdempotencyKey`;
  identifies the intended mutation; drives store dedup / ledger identity.
- *Submission anchor* (adapter, prepared-invocation) — the deterministic signed-tx hash, retained in
  the prepared-invocation artifact (which `docs/design.md` already permits to hold "expected hashes")
  *before* broadcast. The anchor is only defined once `InvocationPrepared` is **committed**, because
  it freezes the nonce, fees, and gas — all **live RPC reads** at prepare time
  (`adapters/evm-contracts/src/lib.rs:665-683`). Recovery therefore **reconstructs the anchor from
  the committed prepared-invocation artifact** (`reconstruct_*`, `:1145+`) and must never re-run the
  live `prepare_*` path; a death *before* `InvocationPrepared` committed broadcast nothing (the
  boundary is `InvocationStarted`), so a fresh prepare on a new invocation epoch is safe. RFC 6979
  deterministic signing is necessary for the signing step, but the load-bearing determinism is
  **artifact reconstruction**, not the signature. Reconciliation matches this anchor against chain
  truth.
Two deterministic anchors, two layers; neither crosses the boundary.

**(b) Finality depth is certified, resolved at admission, never process-local.** `verification:
Receipt | Finalized(policy)` is hash-defining on `SideEffectContractSpec` — the semantic *class*
plus a **certified finality-policy descriptor**. Because the numeric depth is outcome-affecting it
is not taken from worker-local config: the certified policy is **resolved against the registry at
run admission** to a concrete depth, recorded in `RunAdmitted` launch evidence **with the
registry-snapshot digest + resolver identity, which are folded into the run-id preimage** (DEC-36), and
read from the run stream by every worker and by replay. (Pure admission-pinning from local config would only move
the process dependence to *which worker admitted* — resolving from certified authority removes it.)
At **initial launch** the invoker must hold the capability to satisfy the policy or the launch
**fails before `RunAdmitted`** (an ingress failure, per `docs/design.md:216`) — an invoker-driven
system has no pool to hand an admitted-but-undrivable run to, so it must not admit one. At
**resume/attach** a worker lacking the capability **declines cleanly** and the run waits for a capable
resumer (a capacity/deployment stall surfaced operationally, never a semantic failure). *(Resolves the
review's depth-model inconsistency and the strand-an-admitted-run concern, review #5.)*

**(c) Terminal evidence by level — enables receipt-level output.** Today terminal output is built
only from confirmation (`output_from_confirmation`, `program/src/lib.rs:1561`) and runtime requires
`is_confirmed()` before output (`side_effect_lifecycle.rs:298`). Generalize to a typed
**terminal-evidence** model: the `SideEffectState` builds output from the terminal evidence at its
level — `output_from_receipt(.. Receipt)` for `Receipt`, `output_from_confirmation(.. Confirmation)`
for `Finalized` (the `Receipt` and `Confirmation` associated types already exist,
`program/src/lib.rs:1534-1537`). Runtime validates terminal evidence against the *configured* level,
not unconditionally `is_confirmed()`. **The verify framework node *invokes* this domain contract; it
never constructs domain output itself (DEC-37).** The framework node owns the generic *lifecycle*
(poll, terminalize, bind the cell, release the lane), but the output *value* is built by the domain
`SideEffectState`'s certified `output_from_*` method, invoked through state/adapter descriptors — so
`kernel/runtime` stays domain-free (`docs/architecture.md:80`).

**(d) One ledger across the pair — a first-class *pair authority* redesign (DEC-14, DEC-17).** The
verify node continues the submit node's ledger. This is *not* a projection-lookup tweak: side-effect
authority is node/attempt-shaped **down to the event schemas** — `IntentPersisted` and its siblings
carry `node_id`/`scope_id`/`attempt_id` (`events.rs:2389`), as do store admission, lane release
(`resource_lanes.rs:75`), and artifact producer evidence (`store/v1/mod.rs:6931`). One ledger across
two nodes therefore requires a **first-class certified `SideEffectPair` authority** that the schema
and validation understand:
1. **Pair identity** — lowering mints a certified pair `(submit_node, verify_node)`; the ledger key
   derives from `(run, pair_id)`, not `{attempt_id, node_id}` (`side_effect_driver.rs:1096`).
2. **Event attribution** — side-effect event payloads are attributed to a `(pair, role ∈ {submit,
   verify})`; store admission accepts an event iff its emitting node is the pair's certified node
   for that phase (replacing single node/attempt checks, `events.rs:2389`).
3. **Projection lookup** — by ledger/pair key, not node+attempt (`side_effect_lifecycle.rs:278`).
4. **Release authority** — bound to the pair: the submit role claims the lane, the verify role
   releases it, validated against the certified pair + fence (`resource_lanes.rs:75`).
5. **Producer evidence** — submit-role intermediate artifacts and the verify-role output artifact
   are both legal within the pair (`store/v1/mod.rs:6931`).
6. **Remediation linkage** — remediation today targets a forward *node*
   (`runtime/src/side_effects.rs:213`) and the saga keys it on `forward_ledger_key` (`docs/saga.md`).
   With a pair, remediation targets the **pair** (canonically the submit role), over the pair-keyed
   ledger.
7. **Forward fence** — saga engagement forbids new forward boundary crossings, derived from per-node
   ledger phase (`docs/saga.md`). The fence must treat the pair **atomically** (over pair-terminal
   phase), so a verify-side transition after engagement is not wrongly admitted.

**Decided: one-ledger** (rounds 1–2 showed the redesign reaches event schemas, store admission, lane
release, and producer evidence — a wide blast radius, accepted on purpose). One ledger is the simpler,
cleaner *end state* (one ledger, one anchor, one lane; attempt lifecycle decoupled from ledger
lifecycle), and we optimize for end-state simplicity over one-time implementation effort
(`docs/code-quality.md`: doing it right over doing it now). *Considered and rejected:* the
two-linked-ledger handoff — smaller blast radius, but a permanently more complex end state (two
ledgers + a handoff protocol).

**(e) Output ownership & lineage — canonical lowering.** Splitting the node moves its
downstream-visible output, so lowering must define ownership. Canonical rule: the original
side-effect node's **output cell is produced by the verify node** (its producer is re-pointed to
verify at lowering); the submit node produces only internal submission evidence and binds **no**
downstream-visible cell. Cell lineage and artifact producer evidence name the **verify** node.
Consequence: downstream nodes depend on the verify node's cell, so they can never observe
"submitted" before "verified," and every cell has exactly one producer — e.g. a deployed contract
address is unusable downstream until verification terminalizes.

**Live vs replay verifier.** A new live `SideEffectVerifier` (read capability only; any fungible
worker) reads chain truth by the submission anchor and emits the existing `ReceiptObserved /
ConfirmationObserved / NotSubmittedProven` evidence; the existing `SideEffectReplayVerifier`
(`replay/src/lib.rs:470`) checks it from recorded facts, no live IO.

### 6.5 Reconciliation & recovery (ordinary driving once resumed)

The verify-pair is designed so recovery *once a run is driven again* follows the ordinary frontier
rather than a bespoke side-effect recovery machine. **The automatic resumption that triggers
cross-process recovery is deferred to the AC/DC + saga effort (§6.3, §12);** this section specifies
how recovery *behaves* once a run is manually resumed in v1, automatically resumed by that future
effort, or re-driven within a live driver. It is not a v1 sweep. Because the verify node is an
ordinary frontier node:

- A worker that dies mid-verification leaves a **runnable verify node**; manual `run resume` in v1
  (or future automatic recovery) re-drives it.
- A worker that dies mid-submission leaves an **open submit attempt**; manual `run resume` in v1
  (or future automatic recovery) re-runs the attempt path. **The safe reconciliation — chain-truth
  read → `Confirmed` / `NotSubmittedProven`, plus the re-sign == recorded-anchor assertion — is
  net-new EVM-verifier work (WS-D), not emergent from frontier driving**: today's recovery callback
  blindly re-broadcasts and returns `Observed` (`adapters/evm-contracts/src/lib.rs:1758-1774`).
  Safety floor *given WS-D*: the tx is pinned to one nonce, so **at most one tx can land — never a
  double-spend**, and recorded-anchor reconstruction makes re-broadcast idempotent. *Without WS-D*,
  bare re-broadcast mishandles a tx dropped from the mempool whose nonce has since advanced (reorg /
  foreign use) — recording `Observed` for a tx that can never land — so WS-D's reconcile-before-resubmit
  is a **hard gate**, not optional.
- The wallet lane releases **only** when the verify node terminalizes, so a dead worker can wedge the
  wallet until manual resume. That is an explicit v1 availability tradeoff; it is safer than
  timeout-releasing an ambiguous nonce.
- **Foreign-occupied nonce** (a tx MFM never sent occupies the nonce) → the anchor never appears →
  `NotSubmittedProven` terminal ("superseded"), an honest failure/compensation, never a
  double-spend. Exclusivity is an **availability** expectation, not a safety assumption (DEC-2).

**Key invariant (replaces any holder-death sweeper):** once the run is being driven, a non-terminal
side-effect ledger has a corresponding runnable frontier node (and, in the future, a
`due_at`-scheduled node for parked long waits). The scheduler decision exposes it, the driver/resume
loop drives it, and the lane releases at terminal.

This subsumes what the earlier design called the "recovery verifier" (now the general live verifier)
and the "resource-lane holder-takeover machine" (now unnecessary for safety — the lane is
ledger-lifetime and is not lease-reaped).

**Lane release tracks the ledger's terminal state, not the run's block.** The wallet lane is held
only while the side-effect is *genuinely ambiguous*. The moment the ledger reaches a **proven
terminal** — `Confirmed` (nonce consumed) or `NotSubmittedProven` (nonce provably free) — the lane
releases, *even if the run then manual-blocks downstream*: only the run is parked, the wallet is free
for other runs. A lane is held past terminal only when the **ambiguity itself** needs an operator
(`ManualResolution`), and the operator's signed resolution is what terminalizes and releases it. The
lane is **never** timeout-released (it is ledger-lifetime, not lease-reaped) — releasing a
still-ambiguous nonce would risk a cross-run nonce conflict. A held nonce lane blocks only
*same-signer* side-effects; unrelated runs proceed (DEC-25).

**Per-signer write throughput ceiling (honest scalability bound).** Because the lane is held
submit→terminal, a signer account executes at most **one side-effect per terminal window**: ≈one per
block at the default `Receipt` level, and ≈one per finality window only for explicitly-`Finalized`
effects. On-chain *reads* are unconstrained. Horizontal scale of side-effecting work comes from
**more signer accounts**, not more workers, until pipelined nonces (DEC-1) are lifted. (A
`Receipt`-level reorg can gap the next nonce on that signer; nonce continuity is **recovered** by the
stuck-tx / nonce-reclaim recovery workflow, not auto-healed — the accepted, recoverable risk of
choosing `Receipt`, see below.) (DEC-26)

**Receipt is final-at-risk — a deliberate, recoverable choice (DEC-32).** `Receipt`-level
terminalization binds output and releases the lane at `ReceiptObserved` — *before* finality. A later
reorg of that receipt leaves no frontier in the terminalized run to repair the original mutation, so
(a) the published output may be invalid and (b) the next nonce on that signer may **wedge** (a
released-and-reorged nonce that a subsequent run already advanced past). Both are the **accepted risk
of choosing `Receipt`** — the op designer selects the verification level per side-effect, and picks
`Finalized` when reorg safety matters. Neither is a correctness violation: the wedge is **recoverable**
by the deferred stuck-tx / nonce-reclaim recovery workflow (cancel/replace the missing nonce to
restore continuity), and a downstream that consumed a reorged-out `Receipt` output is the same
final-at-risk acceptance. Outputs that must survive reorgs use `Finalized`. (Last round's "nonce
continuity self-heals" was inaccurate — it is *recovered*, not automatic.)

**`NotSubmittedProven` is the outcome of recovery *investigation*, not a single read (DEC-35).** "Nonce
advanced + receipt absent" is an **ambiguity**, not a proof — its causes differ and so do the
recoveries: the receipt may exist and merely failed to fetch (recover it from the tx hash); the tx may
be **stuck in the pool** (cancel or bump gas); a **foreign tx** may have taken the nonce (terminal
"superseded"); or it was truly never sent (resubmit). WS-D's chain-truth read resolves the clear cases
in v1; the richer diagnosis-and-recovery (receipt recovery, stuck-tx cancel/bump) is part of the
deferred AC/DC + saga recovery effort. `NotSubmittedProven` is the *terminal outcome* of whichever path
resolves the ambiguity — which is also why a transient/lagging RPC read can never by itself release the
lane: it triggers investigation, not terminalization. So the negative terminal is "not always about
finality" — finality is one input among several.

### 6.6 Determinism, capabilities, replay

- **Recorded-anchor reconstruction is load-bearing (DEC-15).** The production ECDSA signer path
  already uses deterministic RFC 6979-style signing, so this RFC is not adding a new nonce-signing
  mechanism as its primary safety move. The primary invariant is that the submission anchor (the
  prepared-invocation expected hash, §6.4a) is recorded before the boundary; recovery
  **reconstructs it from the committed prepared invocation and asserts the re-signed hash equals
  it** before any re-broadcast. A determinism-violating signer fails closed instead of
  double-submitting. **Disposition on mismatch (review Med-6):** the re-sign check gates
  *resubmission*, not reconciliation — the verify node still reconciles by the *recorded* anchor
  against chain truth. On-chain → confirm + release the lane; provably not on-chain and
  unresubmittable → a **defined terminal**: `NotSubmittedProven` → failure-with-lane-release
  (default `FailWithoutAcdcClaim`), or a `ManualResolution`-policy manual block that holds the lane
  pending operator action. Never a silent ledger/lane wedge.
- **Capabilities split semantic vs operational** (unchanged): outcome-affecting authority (signer
  ref, expected chain id, expected address) is pinned to the run and verified at admission;
  transport routing (RPC URL, source id) is per-worker. Verification needs only **read**
  capabilities, so any worker with the run's bound executable identities (DEC-29) and those read
  capabilities can run it.
- **Replay unchanged in shape.** The verify node records receipt/confirmation evidence; replay
  re-verifies from recorded evidence via `SideEffectReplayVerifier`, no live IO.

## 7. Decision log

| ID | Decision | Rationale |
|---|---|---|
| DEC-0 | Doctrine: correctness = the Postgres transaction boundary; topology operational; leases = liveness, never safety | Enables fungible processes / multi-agent without process-level coordination |
| DEC-1 | Nonce: serial per-account, sequential-within-hold; pipelining deferred | Deterministic nonce block, point-wise recovery; pipelining reopens range/gap recovery |
| DEC-2 | Reconcile on the deterministic **submission anchor** (prepared-invocation signed-tx hash, adapter-computed — distinct from the state's semantic idempotency input), never the nonce; no exclusive-ownership assumption | Respects the state/signer boundary; foreign-occupied nonce = safe detectable terminal; exclusivity = availability, not safety |
| DEC-3 | Finality: semantic class + **certified policy descriptor** resolved against the registry at admission to a depth recorded in `RunAdmitted` | Depth is outcome-affecting → from certified authority, not worker config; **initial launch without the capability fails before `RunAdmitted` (ingress); only resume/attach declines** (review #5) |
| DEC-4 | Admission leases + heartbeat; expiry routes to recovery, never silent release | Superseded for resource-lane *holders* by §6.4–6.5 (ledger-lifetime); applies to execution tenure |
| DEC-5 | Recovery behavior = ordinary driving once a run is resumed; automatic recovery dispatch is deferred | Needs only a read capability; v1 trigger is manual `run resume`, future trigger is expired-claim sweep |
| DEC-6 | `due_at` re-wake for undriven parked runs — **DEFERRED** (v1 holds+heartbeats through short Receipt waits) | Only needed for long `Finalized` waits at scale |
| DEC-7 | Per-run execution lease (not per-attempt) | Simplest; runs are the parallelism unit; per-attempt deferred |
| DEC-8 | Release execution tenure on long external waits — **DEFERRED** (v1 holds+heartbeats) | Pairs with `due_at`; only for long `Finalized` waits |
| DEC-9 | Single feed / oldest-first now; shard by `hash(run_id)` later | Defer scale complexity until needed |
| DEC-10 | Materialize the lane-state projection (rebuildable cache, not authority) | Removes the O(history) `resource_lane:%` fold |
| DEC-11 | Size pools to worker count; pooler-safe (xact-scoped locks + lease tables) | No session advisory locks → transaction-mode poolers OK |
| DEC-12 | Unify resource lanes + execution claims into one admission-lane primitive | One table-pair, one `admit(mode)`; primitive owns order+lease, caller owns authority |
| DEC-13 | Side-effect verification is a paired framework state (submit + verify) | Uniform/configurable verification; once resumed, reconciliation follows the ordinary frontier instead of bespoke machinery |
| DEC-14 | One ledger across the verify pair (**committed**) — re-key by certified pair identity, projection-by-ledger-key, pair-bound release, multi-node payload validation | Simplest end state; decouples attempt vs ledger lifecycle. Two-linked-ledger rejected (less effort, more complex end state) |
| DEC-15 | Recovery reconstructs the anchor from the committed prepared invocation and asserts re-sign == it before re-broadcast | The production signer path is already deterministic; artifact reconstruction is the load-bearing invariant because nonce/fee/gas are live reads |
| DEC-16 | Side-effect terminal output via a typed terminal-evidence model (`output_from_receipt` / `output_from_confirmation`), validated against the configured level | Enables `Receipt`-level terminalization; output is confirmation-only today |
| DEC-17 | One-ledger's first-class certified `SideEffectPair` authority (pair identity, event attribution, store admission, pair-bound release, producer evidence) is accepted as the chosen path | Node/attempt shaping reaches the event schemas (`events.rs:2389`); blast radius accepted, not a reason to switch |
| DEC-18 | Default run id = **semantic** content address: `hash(certified graph + resolved semantic inputs/config + admission policy + trust scope)`, **excluding executables**, audit/provenance, the run id, and launch spelling | Identical work converges (no double-execute); build-independent identity; `NowaitSkip` coordinates active drivers of that identity |
| DEC-19 | Lowering re-points the original output cell's producer to the verify node; submit binds no downstream cell | Downstream never observes "submitted" before "verified"; one producer per cell |
| DEC-20 | Re-sign mismatch gates resubmission only; reconcile by the recorded anchor to a defined terminal (failure-with-lane-release or policy manual-block) | Never a silent ledger/lane wedge |
| DEC-21 | Automatic takeover of a side-effecting run requires reconciliation → deferred to the AC/DC + saga recovery effort | v1 uses explicit manual resume; safety does not depend on single-driver uniqueness |
| DEC-22 | **Invoker-driven** execution (not pool-dispatched): the starting process drives its run; the execution claim = active-driver coordination + future recovery substrate; the observation feed is status/watch only | Corrects a worker-pool misframing; no feed-driven dispatch → no snapshot-frontier liveness coupling |
| DEC-23 | Dead-driver **automatic resume** (background sweep over expired claims) — **DEFERRED** to the AC/DC + saga recovery effort | v1 recovery is manual `run resume`; a dead driver can hold a signer lane until resumed |
| DEC-24 | The execution lease grants **no** authority to cross a side-effect boundary; the per-side-effect nonce-lane claim + `claim_fencing_token` is the sole submission gate | A false-reaped double-drive still cannot double-submit |
| DEC-25 | Wallet lane releases at the side-effect ledger's **proven terminal** (Confirmed / NotSubmittedProven), not at the run's manual-block; held past terminal only when the ambiguity itself needs an operator; never timeout-released | A manual-blocked-but-proven side-effect frees the wallet; a still-ambiguous one holds it safely |
| DEC-26 | Per-signer write throughput is bounded by the terminal window (≈1/block at Receipt, ≈1/finality at Finalized); reads unbounded; scale via more signers | Honest scalability bound of serial-within-hold (DEC-1) |
| DEC-27 | `SideEffectVerify` is its **own validation family** (N-cardinality pairing invariant, evidence binding, post-rewrite single-producer check), not a singleton like other framework nodes | Zero per-author LOC, but real new certifier/lowering work |
| DEC-28 | Runs are **idempotent by default** (same certified graph → one run); a caller forces a distinct run of identical work via an explicit `run_id` or a distinguishing input that flows into the graph | The safe multi-agent default — independent identical requests converge on one run, never double-execute |
| DEC-29 | Executable identity is a **drive-time determinism guard, NOT part of run identity** (`binding.rs:113`/`history.rs:212` enforce it on drive/resume). Same work → one run regardless of build; a different-build launcher **attaches/reports** ("needs compatible build"), never duplicates | Reverses build-scoped identity: avoids cross-build **double-execution** (review #1) while keeping the determinism guard; rolling upgrade drains on matching build, cross-build takeover deferred |
| DEC-30 | v1 mid-submission reconciliation is **net-new EVM-verifier work (WS-D)** — chain-truth read + re-sign==anchor — not emergent from frontier driving; bare re-broadcast (today's `submit_or_recover_submission`) is insufficient and gated out | "At most one tx lands" holds *given* WS-D; without it, dropped-mempool + advanced-nonce is mishandled |
| DEC-31 | **Normative:** one Postgres deployment = one trust domain (all launchers co-authorized for its signers); **trust scope is a factor of the run-id preimage**. Cross-tenant isolation is a non-goal (would need the launching principal in the preimage + claim + evidence) | A trust *boundary*, not a note (review #2); co-authorized convergence is safe, distrusting tenants out of scope |
| DEC-32 | `Receipt`-level terminalization is **final-at-risk** — the op designer's deliberate per-side-effect choice (`Finalized` is the safe option). A post-receipt reorg can invalidate the output **and** wedge the next nonce; both are **recoverable** via the deferred stuck-tx / nonce-reclaim workflow, not a safety hole and not auto-healed | R7 #1 — accepted+recoverable risk, not a forced hold-to-finality; corrects last round's "self-heals" |
| DEC-33 | Ratification reconciles `docs/saga.md` ("every requested lane acquired together", `saga.md:161`) **down to single-lane** to match the code (`single_lane_claim_admission`, `resource_lanes.rs:94`); multi-lane stays deferred | review #6 — the saga contract overstated the single-lane implementation |
| DEC-34 | *(Optional v1 hardening, recommended.)* `RunAdmitted` records first-class `RunIdentityEvidence` (projection algorithm/version, semantic-projection digest, trust scope, admission-policy digest); attach re-derives the projection from the stored spec and compares, failing closed on mismatch | MFM's no-bare-hash hygiene (R7 #3); guards a projection-bug / schema-evolution wrong-attach. Practical risk needs a hash collision → recommended, not required, for v1 |
| DEC-35 | `NotSubmittedProven` is the terminal *outcome of recovery investigation* (receipt-recovery / stuck-tx cancel-or-bump / foreign-superseded / resubmit), not a single read; WS-D resolves clear cases, richer diagnosis is deferred | R7 #2 — "nonce advanced + receipt absent" is ambiguity; a transient RPC read triggers investigation, never terminalization; not always about finality |
| DEC-36 | The **resolved** admission policy — finality depth + registry-snapshot digest + resolver identity — is folded into the run-id preimage and recorded in `RunAdmitted` | R7 #4 — registry-resolved terminality is outcome-affecting, so a registry change must yield a different run, not silent divergence |
| DEC-37 | The framework `SideEffectVerify` node **invokes** the domain `SideEffectState::output_from_*` contract for the output value; it never constructs domain output in `kernel/runtime` | R7 #5 — keeps `kernel/runtime` domain-free (`architecture.md:80`); framework owns lifecycle, domain owns output |
| DEC-38 | v1 claim is **processes are safe to lose** (a crash never corrupts), NOT automatic at-least-once progress — progress after a driver loss needs manual `run resume`; automatic at-least-once is the deferred recovery effort | R7 #6 — "at-least-once execution" overstated automatic progress for v1 |

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
- **Typed contract + impl (resolves the placement question):** the admission/dispatcher *contract*
  lives in `kernel/store` (consumed by app/runtime); the Postgres SQL is its implementation in
  `crates/storages/stream-store-postgres`. App never reaches into the SQL.
- **Targets:** `kernel/store` (contract + lane-id/fingerprint derivation),
  `crates/storages/stream-store-postgres` (migration + impl).
- **Depends on:** nothing (behavior-preserving refactor for resource lanes).

### WS-B — Content-addressed run identity + execution claim + invoker drive loop

- **Default run-id derivation (semantic):** replace random `mfm_app::new_run_id()` defaults on
  CLI/REST launch with a derived run id over the canonical lowered graph + **resolved semantic
  inputs/config** + admission policy + trust scope, **excluding executables**, audit/provenance
  (`TypedExecutionSpecAudit`), the run id, and launch spelling; define remediation-linked
  (`forward_run_id`) handling (DEC-18/31, MED-3). Launch is **try-admit → attach-on-`CommitConflict`**;
  a launcher whose executables don't match the admitted run **attaches/reports** ("needs a
  compatible-build driver"), never duplicates and never hard-errors. **Initial launch verifies
  capability before `RunAdmitted`** (ingress failure, DEC-3); only resume/attach declines (review #5).
- **Execution lane class** (`NowaitSkip`, `lane_id` = derived run id) on the WS-A primitive:
  try-acquire active-driver coordination + heartbeat + lease expiry. Dedup identity is the run id;
  the claim avoids duplicate driver work and gives future recovery a stale-claim substrate.
- **Invoker drive loop** (library, in the `mfm run` path): acquire-or-attach the claim → fold →
  drive-until-blocked while heartbeating → hold + poll through short Receipt waits → release at
  terminal.
- **Runnable granularity:** surface the existing scheduler decision (`Run` / `Blocked` / `Completed`,
  `crates/kernel/runtime/src/frontier.rs`) to the drive loop; the observation feed may remain
  Started/Completed for watch/status.
- **Not in v1 (deferred to the AC/DC + saga recovery effort):** automatic resume of dead-driver runs
  + reconciliation-on-takeover (DEC-21/23); `due_at` + tenure-release-on-wait (DEC-6/DEC-8). v1
  delivers content-addressed dedup identity, manual `run resume`, and the execution-claim substrate
  that effort will build on; the observation feed stays status/watch only.
- **Targets:** `crates/app` (run-id derivation + drive loop), `bin/*` / `bin/rest-api` (derived
  default launch id; resume path unchanged), `kernel/runtime` (surface scheduler decision),
  `kernel/store` + `storages` (claim lane + expiry query).
- **Depends on:** WS-A.

### WS-C — Side-effect pair authority + verification framework state

- **Phase gate: ledger authority redesign [§6.4d].** Re-key the ledger from a certified pair
  identity (drop `attempt_id`/`node_id`, `side_effect_driver.rs:1096`), projection lookup by ledger
  key (`side_effect_lifecycle.rs:278`), pair-bound release authority (`resource_lanes.rs:75`),
  multi-node payload validation, **plus remediation-link retargeting to the pair and an atomic pair
  forward-fence** (`runtime/src/side_effects.rs:213`, `docs/saga.md`) (`kernel/store`,
  `kernel/runtime`). This is foundational: without pair-keyed authority, the verify node cannot
  continue the submit node's ledger or release the wallet lane.
- **`FrameworkNodeSpec::SideEffectVerify`** spec types + parse/json (`kernel/spec`); lowering inserts
  it after each side-effect node and records the **certified pair identity** (`kernel/program`). Its
  **own validation family** (`kernel/certify/framework_lifecycle.rs`): N-cardinality pairing
  invariant, evidence binding, post-rewrite single-producer check — not the singleton check the other
  framework nodes use (DEC-27).
- **`verification: Receipt | Finalized`** on `SideEffectContractSpec` (hash-defining); finality
  **depth pinned at run admission** as `RunAdmitted` evidence and verified per worker
  (`kernel/spec`, `kernel/certify`, run admission in `kernel/runtime` / `crates/app`). [§6.4b]
- **Terminal-evidence model [§6.4c]:** add `output_from_receipt`; runtime validates terminal
  evidence against the *configured* level instead of unconditional `is_confirmed()`
  (`kernel/program:1561`, `side_effect_lifecycle.rs:298`).
- **Verify-node runner** (`kernel/runtime`): drive `Submission* → Receipt → Confirmation → terminal`
  to the level; bind the verified output cell — **invoking** the domain `SideEffectState::output_from_*`
  contract for the output value (DEC-37), never constructing domain output in the kernel. Scheduler
  invariant once a run is driven: non-terminal side-effect ledger ⇒ runnable frontier node (future
  `due_at` for long waits).
- **Live `SideEffectVerifier`** contract (`crates/adapter-contracts`) + EVM impl
  (`crates/adapters/evm-contracts` + `crates/transports/evm`, relocating receipt/confirmation poll),
  behind a read capability.
- **Submission anchor [§6.4a]:** the deterministic signed-tx hash is recorded in the
  prepared-invocation artifact at the **adapter** layer (`crates/adapters/evm-contracts`), *not* in
  the state; `crates/states/evm-contracts` keeps only its semantic `IdempotencyInput` (no signing).
- **Depends on:** pair-authority phase gate above; WS-B for the v1 invoker drive loop.

### WS-D — Anchor reconstruction & reconciliation (hard gate, not emergent)

- **Replace bare re-broadcast with reconcile-before-resubmit (DEC-30).** Today's EVM recovery callback
  `submit_or_recover_submission` unconditionally re-broadcasts and returns `Observed`
  (`adapters/evm-contracts/src/lib.rs:1758-1774`). WS-D makes the live `SideEffectVerifier` read chain
  truth first and emit `Confirmed` / `NotSubmittedProven` / still-`Unknown`, driving the generic
  `SideEffectSubmissionDecision` (`side_effect_driver.rs:848-878`) instead of always-`Observed`. This
  is a **hard gate** of the verify-pair, not optional.
- **Re-sign == recorded-anchor assertion** in the submit-node recovery path (`kernel/runtime`);
  fail closed on mismatch before any re-broadcast.
- **Signer determinism remains a contract invariant** (`crates/signing`); the production ECDSA path
  already uses RFC 6979-style deterministic signing, so the load-bearing invariant is anchor
  reconstruction, not the signature (DEC-15).
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
Phase 2: WS-B  (run identity + claim + drive) ── dedup identity + active-driver coordination
Phase 3: WS-C + WS-D (pair authority + verification) ── productize side effects (manual-resume path)
Phase 4: WS-E  (perf / ops)                ── scale optimizations
```

**Automatic recovery is deferred (review HIGH-2 + the AC/DC + saga effort).** v1 has an explicit
manual resume path, not a background expired-claim sweep. A run has one intended active driver at a
time, but correctness does not depend on that uniqueness: false reaps/races cost duplicate driver
work, while per-run append CAS and side-effect fencing preserve safety. If a dead driver leaves a
side-effect ledger non-terminal, the signer lane can remain held until manual resume drives the run.

## 9. Migration & compatibility

- **`resource_lane_waiters` → `admission_lane`/`admission_waiter`.** A fresh schema is acceptable;
  resource-lane *holders* are event-derived (rebuilt from the stream), so only the operational
  waiter queue is replaced. No semantic data migration is required for holders.
- **Side-effect contract gains `verification`.** This changes the spec hash; existing certified
  specs are re-lowered/re-certified (no in-place migration of historical runs, per `docs/design.md`
  CLI/REST rules).
- **Public surfaces.** The `mfm run start` / REST launch default run id changes from random UUID to a
  **semantic** content address (certified graph + resolved config/inputs + admission policy + trust
  scope, **excluding executables**); callers may pass an explicit run id for a distinct run. Initial
  launch now fails before `RunAdmitted` if the invoker lacks the policy capability (DEC-3). The run
  path gains an execution claim for active-driver coordination. Existing `run resume`, `run replay`,
  and `run list --watch` remain; manual resume is the v1 recovery trigger. Automatic resume is
  deferred (§12); there is no separate worker-pool entrypoint.

## 10. Risks & mitigations

| Risk | Mitigation |
|---|---|
| Admission table mistaken for authority | Structural: mode∝class, liveness-only, no safety read branches on it (§6.2 inv. 2); lane-state projection kept separate |
| Run-id derivation accidentally includes volatile launch spelling or executables | Hash only the canonical graph + resolved semantic inputs/config + admission policy + trust scope; exclude run id, **executables**, audit/provenance, raw file path/bytes, and unresolved "latest" spelling (DEC-18/29) |
| One-ledger redesign reaches event schemas / store admission / lane release / producer evidence (wide blast radius) | Accepted as the chosen end state and a WS-C phase gate (DEC-14/17); pair-authority schema design in §6.4d + replay/kill-mid-flight tests; two-linked-ledger considered and rejected |
| Fungible workers terminalize finality at different depths | Depth resolved from a certified policy at admission (§6.4b), identical for all workers and replay; a worker lacking the capability declines the claim |
| Signer cannot reproduce a recorded anchor → wedged lane | Defined terminal disposition (DEC-20): reconcile by the recorded anchor; failure-with-lane-release or policy manual-block, never a silent wedge |
| Automatic takeover of a side-effecting run without reconciliation | Automatic resume is deferred to the AC/DC + saga effort (DEC-21/23); v1 uses explicit manual resume, and ambiguous ledgers keep the signer lane held until driven |
| Signer determinism regresses | Production signer is already deterministic; re-sign == recorded-anchor assertion catches mismatch before re-broadcast (DEC-15/20) |
| Herd on the claim table (future resume sweep / concurrent invokers) | Content-addressed run id gives one lane per runtime graph; `NowaitSkip` try-acquire losers pay one `UPDATE`; duplicate invokers attach/report rather than do driver work (DEC-18/22) |
| Postgres as the coordination/scaling ceiling | Per-run and per-lane locks parallelize disjoint work; materialized lane projection (WS-E) removes the O(history) fold |
| Wallet wedged by a dead driver before verify terminal | Accepted v1 availability tradeoff: manual `run resume` is the recovery trigger; the lane releases at the ledger's proven terminal (DEC-25), never on a timeout. Automatic recovery is deferred |
| Observation-feed frontier lag (`pg_snapshot_xmin`) | Affects status/watch freshness only; dispatch is invoker-driven, manual resume is keyed by run id, and future automatic resume reads the claim table directly — no dispatch coupling (DEC-22/23) |
| `ManualResolution` holds a wallet lane | Only when the ambiguity itself needs an operator (rare; most ambiguity auto-reconciles); blocks only same-signer effects; mitigate operationally (per-class signers, fast alerting) (DEC-25) |
| Same work on a different build double-executes if executables are in run identity (review #1) | Executables are a **drive guard, not identity** (DEC-29); semantic run id → identical work is one run; a different-build launcher attaches/reports via try-admit→attach, never duplicates |
| "Any worker drives any run" overstated — drive is executable-scoped and v1 is manual-resumable (review #3) | Doctrine scoped in §1/§6.1: driving requires matching executables; v1 invoker-driven + manual resume; full auto-recovery deferred |
| Manual resume of an ambiguous submission left unreconciled (bare re-broadcast) | WS-D reconcile-before-resubmit (chain truth → Confirmed/NotSubmittedProven, re-sign==anchor) is a hard gate (DEC-30), replacing today's always-`Observed` callback |
| Dedup convergence under mutual distrust drives a side effect under another launcher's signer | One deployment = one trust domain, trust scope in the run-id preimage (DEC-31, normative); cross-tenant isolation is a non-goal |
| Capability-lacking invoker strands an admitted run (review #5) | Initial launch verifies capability **before** `RunAdmitted` (ingress failure, DEC-3); only resume/attach declines cleanly |
| Receipt-level output invalidated by a post-receipt reorg (review #4) | `Receipt` is explicitly **final-at-risk** (DEC-32); outputs needing reorg-safety use `Finalized` |
| `docs/saga.md` multi-lane contract diverges from single-lane code (review #6) | Reconcile saga.md down to single-lane at ratification (DEC-33); multi-lane deferred |
| Receipt-reorg nonce wedge / invalid output (R7 #1) | Accepted, designer-chosen risk of `Receipt` (`Finalized` is the safe option); both recoverable via the deferred stuck-tx / nonce-reclaim workflow, not a safety hole (DEC-32) |
| `NotSubmittedProven` released on a transient/lagging RPC read (R7 #2) | It is an investigation *outcome*, not a single read (DEC-35); a transient read triggers investigation, never terminalization |
| Registry policy change silently diverges terminality (R7 #4) | Resolved admission policy + registry-snapshot digest in the run-id preimage and `RunAdmitted` (DEC-36); a change yields a different run |
| Attach trusts a bare `run_id` digest (R7 #3) | Optional `RunIdentityEvidence` re-derived and compared at attach (DEC-34); wrong-attach needs a hash collision, so recommended hardening |
| Verify node leaks domain output into the kernel (R7 #5) | Framework node **invokes** the domain `output_from_*` contract, never constructs output (DEC-37); `kernel/runtime` stays domain-free |

## 11. Testing strategy

- **Concurrency:** property/integration tests with N concurrent drivers per run asserting
  single-commit-wins and idempotent re-presentation.
- **Run identity:** same canonical graph + same resolved semantic inputs/config ⇒ same run id;
  different config/inputs or graph ⇒ different run id; run-id derivation excludes run id, executables,
  audit/provenance, and volatile launch spelling; **same work + different bound executables ⇒ the SAME
  run id** (executables not in identity, DEC-29) — a different-build launcher **attaches/reports**
  ("needs compatible build"), never a second run and never a hard `CommitConflict`; a same-build
  duplicate launch attaches via commit-key idempotency; an explicit run id or distinguishing input
  yields a distinct run (opt-out, DEC-28).
- **Admission:** FIFO fairness, lease-expiry reaping, `NowaitSkip` no-queue, mode∝class compile-fail
  fixtures.
- **Side-effect manual recovery:** kill-mid-submit and kill-mid-verify integration tests against a
  local chain (reth) asserting the signer lane remains held until manual resume, at-most-once
  landing, idempotent re-broadcast, and correct `Receipt/Confirmation/NotSubmittedProven`
  terminalization after resume. Resume must **reconcile via a chain-truth read**, not bare
  re-broadcast: a tx dropped from the mempool with an advanced nonce terminalizes `NotSubmittedProven`,
  not `Observed` (DEC-30).
- **Determinism:** assert re-sign reproduces the recorded anchor; a deliberately non-deterministic
  signer fixture must fail closed.
- **Replay:** verify-node evidence replays from recorded facts with no live IO.
- **Boundary:** cargo-metadata + compile-fail fixtures keeping the admission primitive
  domain-agnostic and the verifier read-only.

## 12. Deferred / open

- **Automatic resume + dead-driver takeover + reconciliation-on-takeover (DEC-21/23)** — part of the
  larger AC/DC + saga recovery effort, not this RFC. Includes **cross-build takeover** (resuming a run
  admitted under different executables, DEC-29), and the **stuck-tx / nonce-reclaim / receipt-recovery
  investigation** that resolves an ambiguous submission (`NotSubmittedProven` / `Confirmed`) and clears
  a `Receipt`-reorg nonce wedge (DEC-32/35).
- Pipelined nonces (DEC-1); per-attempt execution leases (DEC-7); feed sharding and cross-run
  priority (DEC-9); multi-lane admission (§3 Non-goals); `due_at` re-wake + tenure-release-on-wait
  (DEC-6/DEC-8, for long `Finalized` waits at scale).
- D5–D11 defaults to be rubber-stamped at ratification.
- Concrete shapes: the prepared-invocation submission anchor (adapter) and the state's semantic
  `IdempotencyInput` (now distinct).

## 13. Appendix — key code anchors

| Concern | Location |
|---|---|
| Per-run append lock; head `MAX(seq)` | `run_store/stream.rs:54`, `:25-52` |
| Commit-key idempotency; `expected_next_seq` | `run_store/append.rs:19-55`; `run_store/mod.rs:157` |
| Append-only authority triggers | `migrations/0001_run_store.sql:12-18,200-230` |
| Per-lane advisory lock + FIFO waiters | `run_store/resource_lanes.rs:32-45,134-310` |
| Observation/watch feed + NOTIFY | `run_store/observations.rs:104-268,379-461`; channel `mod.rs:89` |
| Observation surface (app/REST/CLI) | `crates/app/src/lib.rs`; `bin/rest-api/src/lib.rs`; `bin/cli/src/commands/run/list.rs` |
| Current random run-id default | `crates/app/src/lib.rs:454`; `bin/cli/src/commands/run/start.rs:99`; `bin/rest-api/src/lib.rs:716` |
| Scheduler decision (`Run` / `Blocked` / `Completed`) | `crates/kernel/runtime/src/frontier.rs:31` |
| Side-effect fencing | `kernel/store/src/v1/{projection.rs:36,412 , resource_lanes.rs:42,80 , mod.rs:2179-2220}` |
| Side-effect phases/events | `kernel/events/src/lib.rs:413-439`; `SideEffectPhase` in `store/v1` |
| `IdempotencyKey` / `idempotency_input` | `kernel/program/src/lib.rs:1506,1531` |
| `SideEffectContractSpec` / `ResourceClaimSpec` | `kernel/spec/src/lib.rs:1740`, `:1689-1731` |
| `FrameworkNodeSpec` + framework-node validation | `kernel/spec/src/lib.rs:1758`; `kernel/certify/src/framework_lifecycle.rs` |
| `SideEffectReplayVerifier` | `kernel/replay/src/lib.rs:470` |
| EVM exclusive nonce lane; nonce read | `states/evm-contracts/src/lib.rs:107`; `transports/evm/src/lib.rs:469` |
| EVM prepare (live nonce/fee/gas) + reconstruct | `adapters/evm-contracts/src/lib.rs:665-683,1145+` |
| Observation snapshot-frontier gate | `run_store/observations.rs:185-192` |
| Advisory-lock 32-bit truncation | `run_store/stream.rs:66-74` |
| Remediation link / forward fence | `runtime/src/side_effects.rs:213`; `docs/saga.md` |
| Side-effect terminal output (confirmation-only today) | `program/src/lib.rs:1561`; `side_effect_lifecycle.rs:313` |
| RunAdmitted records driver executables; drive-path binding check | `kernel/runtime/src/commit.rs:235-237`; `binding.rs:113`; `history.rs:212` |
| EVM recovery: bare re-broadcast (always `Observed`); generic submission decisions | `adapters/evm-contracts/src/lib.rs:1758-1774`; `runtime/src/side_effect_driver.rs:848-878` |
| Ledger key derivation (attempt-scoped today) | `kernel/runtime/src/side_effect_driver.rs:1091-1127` |
| Spec audit/provenance (excluded from run-id projection) | `kernel/spec/src/lib.rs` `TypedExecutionSpecAudit`; `certify` `forward_run_id` |
| saga.md multi-lane contract vs single-lane code | `docs/saga.md:159-169`; `run_store/resource_lanes.rs:94` |
| Capability bindings verified at admission (ingress) | `docs/design.md:216` |
