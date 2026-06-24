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
2. **Execution claim (dedup + recovery-ownership substrate)** — the invoker drives its own run; an
   execution claim coordinates concurrent invocations (no duplicate driver of the same run) and, via
   its lease, leaves a dead driver's run *reclaimable*. **Automatic resume is out of scope here** — it
   belongs to the larger AC/DC + saga recovery effort (§6.5, §12). The observation feed stays
   status/watch only.
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

- **No execution claim, and no safe takeover/resume.** Execution is invoker-driven (the process that
  starts a run drives it) — fine — but there is no durable execution claim to dedup concurrent
  invocations of the same run (this RFC adds it), and no way for another process to safely take over
  and resume a dead driver's run (deferred to the AC/DC + saga recovery effort). The observation feed
  is read/observe only.
- **No external-effect reconciliation.** A worker that submits a transaction and dies leaves an
  ambiguous nonce; nothing reads chain truth to resolve landed-vs-never-sent.
- **Verification is not productized.** Receipt/finality checking is embedded ad hoc inside the
  side-effect runner rather than configurable per state and reusable across domains.

## 3. Goals and non-goals

### Goals

- A correctness model that depends only on the Postgres transaction boundary, never on process
  identity or lifetime.
- Many disposable workers (CLI, daemon, REST host, agent session) coordinating only through
  Postgres, with **at-least-once execution, at-most-once MFM-authored mutation per deterministic
  anchor, and evidence-backed terminalization/compensation** — not a full AC/DC / exactly-once claim
  (see `docs/saga.md`).
- One reusable coordination primitive for all exclusive-admission needs.
- Uniform, configurable side-effect verification (receipt vs finality) across all domains.
- External-effect recovery that requires no bespoke machinery beyond ordinary dispatch.

### Non-goals

- Pipelined nonces (multiple in-flight transactions per account). Deferred; serial-within-hold is
  sufficient and far simpler (DEC-1).
- Cross-run priority/fairness beyond observation order, and feed sharding. Deferred (DEC-9).
- Multi-lane admission (one claim spanning several lanes, with all-or-nothing acquisition, deadlock
  ordering, and starvation policy). v1 is single-lane only — what the code and `docs/saga.md` already
  provide.
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
- **Execution claim (`NowaitSkip`, lane_id = run id)** — uses only the lane row's lease; writes no
  waiters; the lease *is* the holder, harmless because safety = the store's per-run lock +
  `expected_next_seq`. Try-acquire is the **dedup** gate: a second invoker of the same run finds it
  held and attaches instead of starting a duplicate (§6.3). Crucially, the execution lease confers
  **no** authority to cross a side-effect boundary — that gate is solely the per-side-effect
  nonce-lane claim + `claim_fencing_token`. So even two processes that both believe they hold the
  execution lease (after a false reap) cannot double-submit: only the claim-holder may submit, and
  nonce serialization + the claim-generation fence make at most one mutation land. Execution lease =
  dispatch liveness; side-effect claim = mutation safety.

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
so distinct lanes can falsely contend at scale; the new primitive should key its lock on a 64-bit
value or the full lane id (safety is unaffected — it holds via PK + CAS).

**Scope (v1) — documents the existing single-lane scope, not a new constraint.** The primitive ships
exactly two lane classes: single-lane `WaitFifo` (resource lanes; one `(namespace, key)` per claim —
all the code and `docs/saga.md` already provide) and `NowaitSkip` (execution). Multi-lane admission
stays deferred (§3 Non-goals). The point is only that the RFC's prose must not *imply* multi-lane
atomicity the design never had.

### 6.3 Execution: invoker-drives, claim, and automatic resume

MFM is **invoker-driven**, not pool-dispatched: `mfm run --op xyz` starts the state-machine runner
*in that process* and drives the run to completion or to a block. There is no pool of idle workers
polling a feed for work; the observation feed (§5) is **status/watch only** and never a dispatch
surface.

The execution claim is a `NowaitSkip` admission lane keyed on the content-addressed run id, doing
two jobs — neither of which is discovery:

- **Dedup.** A second `mfm run --op xyz` (same op+config → same run id) finds the claim held by a
  live driver and does not start a duplicate; it attaches to / reports the existing run.
- **Recovery ownership.** The driver heartbeats the claim while running (renewing `lease_expires_at`
  forward). If it dies, the lease expires and the run becomes resumable by another process — without
  ever transferring *semantic* ownership (safety is still the store, §5).

The driver holds the run through blocks rather than handing it off:

```
acquire_execution_claim(run) or attach-to-existing       // dedup
while frontier(fold(run)) is Runnable(t):
  renew_claim(); commit(t)                                // heartbeat + advance
  // a short Receipt-level side-effect wait (~1 block) is polled in-line while heartbeating
release_claim(run)                                        // terminal
```

**Dead-driver recovery is deferred (to the AC/DC + saga effort).** The execution claim's lease +
expiry is the *substrate* for recovery — a dead driver's lease lapses, so its run is no longer locked
and becomes reclaimable — but the mechanism that *automatically* finds and resumes such runs (and,
for side-effecting runs, reconciles an ambiguous in-flight submission) is part of the larger,
separate AC/DC + saga recovery effort, not this RFC. v1 delivers the claim (dedup + the reclaimable
lease); automatic resume is future work (§12). No manual `--resume` is added here either. When it is
built, it should read the **claim table** for expired claims (a fresh indexed read), not the
snapshot-frontier-gated feed.

**Why no feed-driven discovery, and no `due_at` in v1 (review HIGH-2).** The observation feed is
gated behind `pg_snapshot_xmin` (`observations.rs:185-192`), so a pool that discovered work through
it would inherit a cluster-wide liveness lag. That lag is real for the *watch* feed but irrelevant
here: MFM does not dispatch through it — the invoker drives, and resume reads the claim table.
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
run admission** to a concrete depth, recorded in `RunAdmitted` launch evidence, and read from the
run stream by every worker and by replay. (Pure admission-pinning from local config would only move
the process dependence to *which worker admitted* — resolving from certified authority removes it.)
A worker that lacks the capability to satisfy the admitted policy **declines the execution claim**
(the run waits for a capable worker); it does **not** fail the run. A run no deployed worker can
satisfy is a capacity/deployment stall surfaced operationally, never a semantic failure. *(Resolves
the review's depth-model inconsistency: this is the single committed model.)*

**(c) Terminal evidence by level — enables receipt-level output.** Today terminal output is built
only from confirmation (`output_from_confirmation`, `program/src/lib.rs:1561`) and runtime requires
`is_confirmed()` before output (`side_effect_lifecycle.rs:298`). Generalize to a typed
**terminal-evidence** model: the `SideEffectState` builds output from the terminal evidence at its
level — `output_from_receipt(.. Receipt)` for `Receipt`, `output_from_confirmation(.. Confirmation)`
for `Finalized` (the `Receipt` and `Confirmation` associated types already exist,
`program/src/lib.rs:1534-1537`). Runtime validates terminal evidence against the *configured* level,
not unconditionally `is_confirmed()`.

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

### 6.5 Reconciliation & recovery (emergent — no bespoke machinery)

The verify-pair is designed so recovery *falls out of ordinary driving* rather than needing bespoke
machinery. **The automatic resumption that triggers cross-process recovery is deferred to the AC/DC +
saga effort (§6.3, §12);** this section specifies how recovery *behaves* once a run is resumed (by
that future effort) or re-driven within a live driver — it is not a v1 sweep. Because the verify node
is an ordinary frontier node:

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
`Receipt`-level reorg can briefly gap the next nonce on that signer; it self-heals via
reconciliation.) (DEC-26)

### 6.6 Determinism, capabilities, replay

- **Deterministic signing required (DEC-15).** The signer contract must produce deterministic
  signatures (RFC 6979 for ECDSA secp256k1). Enforced, not assumed: the submission anchor (the
  prepared-invocation expected hash, §6.4a) is recorded before the boundary; recovery
  **reconstructs it from the committed prepared invocation and asserts the re-signed hash equals
  it** — a determinism-violating signer fails closed instead of double-submitting. **Disposition on mismatch (review Med-6):** the re-sign check gates
  *resubmission*, not reconciliation — the verify node still reconciles by the *recorded* anchor
  against chain truth. On-chain → confirm + release the lane; provably not on-chain and
  unresubmittable → a **defined terminal**: `NotSubmittedProven` → failure-with-lane-release (default
  `FailWithoutAcdcClaim`), or a `ManualResolution`-policy manual block that holds the lane pending
  operator action. Never a silent ledger/lane wedge.
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
| DEC-2 | Reconcile on the deterministic **submission anchor** (prepared-invocation signed-tx hash, adapter-computed — distinct from the state's semantic idempotency input), never the nonce; no exclusive-ownership assumption | Respects the state/signer boundary; foreign-occupied nonce = safe detectable terminal; exclusivity = availability, not safety |
| DEC-3 | Finality: semantic class + **certified policy descriptor** resolved against the registry at admission to a depth recorded in `RunAdmitted` | Depth is outcome-affecting → from certified authority, not worker config; a worker lacking the capability **declines the claim**, never fails the run |
| DEC-4 | Admission leases + heartbeat; expiry routes to recovery, never silent release | Superseded for resource-lane *holders* by §6.4–6.5 (ledger-lifetime); applies to execution tenure |
| DEC-5 | Recovery = ordinary fungible dispatch of a run in a recovering state | Needs only a read capability; no dedicated recovery worker class |
| DEC-6 | `due_at` re-wake for undriven parked runs — **DEFERRED** (v1 holds+heartbeats through short Receipt waits) | Only needed for long `Finalized` waits at scale |
| DEC-7 | Per-run execution lease (not per-attempt) | Simplest; runs are the parallelism unit; per-attempt deferred |
| DEC-8 | Release execution tenure on long external waits — **DEFERRED** (v1 holds+heartbeats) | Pairs with `due_at`; only for long `Finalized` waits |
| DEC-9 | Single feed / oldest-first now; shard by `hash(run_id)` later | Defer scale complexity until needed |
| DEC-10 | Materialize the lane-state projection (rebuildable cache, not authority) | Removes the O(history) `resource_lane:%` fold |
| DEC-11 | Size pools to worker count; pooler-safe (xact-scoped locks + lease tables) | No session advisory locks → transaction-mode poolers OK |
| DEC-12 | Unify resource lanes + execution claims into one admission-lane primitive | One table-pair, one `admit(mode)`; primitive owns order+lease, caller owns authority |
| DEC-13 | Side-effect verification is a paired framework state (submit + verify) | Uniform/configurable verification; recovery-for-free; collapses DEC-2/DEC-4 machinery |
| DEC-14 | One ledger across the verify pair (**committed**) — re-key by certified pair identity, projection-by-ledger-key, pair-bound release, multi-node payload validation | Simplest end state; decouples attempt vs ledger lifecycle. Two-linked-ledger rejected (less effort, more complex end state) |
| DEC-15 | Deterministic signing (RFC 6979) **and** recovery reconstructs the anchor from the committed prepared invocation, asserting re-sign == it | The load-bearing determinism is artifact reconstruction (nonce/fee/gas are live reads), not the signature; fails closed |
| DEC-16 | Side-effect terminal output via a typed terminal-evidence model (`output_from_receipt` / `output_from_confirmation`), validated against the configured level | Enables `Receipt`-level terminalization; output is confirmation-only today |
| DEC-17 | One-ledger's first-class certified `SideEffectPair` authority (pair identity, event attribution, store admission, pair-bound release, producer evidence) is accepted as the chosen path | Node/attempt shaping reaches the event schemas (`events.rs:2389`); blast radius accepted, not a reason to switch |
| DEC-19 | Lowering re-points the original output cell's producer to the verify node; submit binds no downstream cell | Downstream never observes "submitted" before "verified"; one producer per cell |
| DEC-20 | Re-sign mismatch gates resubmission only; reconcile by the recorded anchor to a defined terminal (failure-with-lane-release or policy manual-block) | Never a silent ledger/lane wedge |
| DEC-21 | Resume of a side-effecting run requires reconciliation → deferred to the AC/DC + saga recovery effort | In v1 each run has a single driver (its invoker); side effects handled by the verify-pair |
| DEC-22 | **Invoker-driven** execution (not pool-dispatched): the starting process drives its run; the execution claim = dedup + recovery ownership; the observation feed is status/watch only | Corrects a worker-pool misframing; no feed-driven dispatch → no snapshot-frontier liveness coupling |
| DEC-23 | Dead-driver **automatic resume** (background sweep over expired claims) — **DEFERRED** to the AC/DC + saga recovery effort | v1 delivers the claim's reclaimable lease as the substrate; no manual `--resume` added either |
| DEC-24 | The execution lease grants **no** authority to cross a side-effect boundary; the per-side-effect nonce-lane claim + `claim_fencing_token` is the sole submission gate | A false-reaped double-drive still cannot double-submit |
| DEC-25 | Wallet lane releases at the side-effect ledger's **proven terminal** (Confirmed / NotSubmittedProven), not at the run's manual-block; held past terminal only when the ambiguity itself needs an operator; never timeout-released | A manual-blocked-but-proven side-effect frees the wallet; a still-ambiguous one holds it safely |
| DEC-26 | Per-signer write throughput is bounded by the terminal window (≈1/block at Receipt, ≈1/finality at Finalized); reads unbounded; scale via more signers | Honest scalability bound of serial-within-hold (DEC-1) |
| DEC-27 | `SideEffectVerify` is its **own validation family** (N-cardinality pairing invariant, evidence binding, post-rewrite single-producer check), not a singleton like other framework nodes | Zero-LOC for authors, real new certifier/lowering work |

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

### WS-B — Execution claim + invoker drive loop

- **Execution lane class** (`NowaitSkip`, `lane_id` = content-addressed run id) on the WS-A
  primitive: try-acquire dedup + heartbeat + lease expiry.
- **Invoker drive loop** (library, in the `mfm run` path): acquire-or-attach the claim → fold →
  drive-until-blocked while heartbeating → hold + poll through short Receipt waits → release at
  terminal.
- **Runnable granularity:** extend the frontier read so the driver gets Runnable/Blocked/Terminal
  (today the feed reports only Started/Completed).
- **Not in v1 (deferred to the AC/DC + saga recovery effort):** automatic resume of dead-driver runs
  + reconciliation-on-takeover (DEC-21/23); `due_at` + tenure-release-on-wait (DEC-6/DEC-8). v1
  delivers the execution claim (dedup) + the reclaimable lease that effort will build on; the
  observation feed stays status/watch only.
- **Targets:** `crates/app` (drive loop), `bin/*` (the `mfm run` runner exists), `kernel/runtime`
  (frontier granularity), `kernel/store` + `storages` (claim lane + expiry query).
- **Depends on:** WS-A.

### WS-C — Side-effect verification as a paired framework state

- **`FrameworkNodeSpec::SideEffectVerify`** spec types + parse/json (`kernel/spec`); lowering inserts
  it after each side-effect node and records the **certified pair identity** (`kernel/program`). Its
  **own validation family** (`kernel/certify/framework_lifecycle.rs`): N-cardinality pairing
  invariant, evidence binding, post-rewrite single-producer check — not the singleton check the other
  framework nodes use (DEC-27).
- **`verification: Receipt | Finalized`** on `SideEffectContractSpec` (hash-defining); finality
  **depth pinned at run admission** as `RunAdmitted` evidence and verified per worker
  (`kernel/spec`, `kernel/certify`, run admission in `kernel/runtime` / `crates/app`). [§6.4b]
- **Ledger authority redesign [§6.4d]:** re-key the ledger from a certified pair identity (drop
  `attempt_id`/`node_id`, `side_effect_driver.rs:1096`), projection lookup by ledger key
  (`side_effect_lifecycle.rs:278`), pair-bound release authority (`resource_lanes.rs:75`),
  multi-node payload validation, **plus remediation-link retargeting to the pair and an atomic pair
  forward-fence** (`runtime/src/side_effects.rs:213`, `docs/saga.md`) (`kernel/store`,
  `kernel/runtime`).
- **Terminal-evidence model [§6.4c]:** add `output_from_receipt`; runtime validates terminal
  evidence against the *configured* level instead of unconditional `is_confirmed()`
  (`kernel/program:1561`, `side_effect_lifecycle.rs:298`).
- **Verify-node runner** (`kernel/runtime`): drive `Submission* → Receipt → Confirmation → terminal`
  to the level; bind the verified output cell. Scheduler invariant: non-terminal side-effect ledger
  ⇒ runnable / `due_at` frontier node.
- **Live `SideEffectVerifier`** contract (`crates/adapter-contracts`) + EVM impl
  (`crates/adapters/evm-contracts` + `crates/transports/evm`, relocating receipt/confirmation poll),
  behind a read capability.
- **Submission anchor [§6.4a]:** the deterministic signed-tx hash is recorded in the
  prepared-invocation artifact at the **adapter** layer (`crates/adapters/evm-contracts`), *not* in
  the state; `crates/states/evm-contracts` keeps only its semantic `IdempotencyInput` (no signing).
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
Phase 2: WS-B  (execution claim + drive)   ── dedup + invoker drive loop (no resume; that's deferred)
Phase 3: WS-C + WS-D (verification + determinism) ── productize side effects (live-driver path)
Phase 4: WS-E  (perf / ops)                ── scale optimizations
```

**Recovery is deferred (review HIGH-1 + the AC/DC + saga effort).** The only multi-process touch of a
*running* side-effecting run is resume of a dead driver, which is unsafe without reconciliation and is
therefore scoped into the larger AC/DC + saga recovery effort, not this RFC. In v1 each run is driven
by its invoker (single driver), including its side effects via the verify-pair; a dead driver's run is
left reclaimable for that future effort.

## 9. Migration & compatibility

- **`resource_lane_waiters` → `admission_lane`/`admission_waiter`.** A fresh schema is acceptable;
  resource-lane *holders* are event-derived (rebuilt from the stream), so only the operational
  waiter queue is replaced. No semantic data migration is required for holders.
- **Side-effect contract gains `verification`.** This changes the spec hash; existing certified
  specs are re-lowered/re-certified (no in-place migration of historical runs, per `docs/design.md`
  CLI/REST rules).
- **Public surfaces.** The `mfm run` path gains an execution claim (dedup); existing
  `run start / replay / list --watch` are unchanged. Automatic resume is deferred (§12) and no new
  manual `run --resume` is added; there is no separate worker-pool entrypoint.

## 10. Risks & mitigations

| Risk | Mitigation |
|---|---|
| Admission table mistaken for authority | Structural: mode∝class, liveness-only, no safety read branches on it (§6.2 inv. 2); lane-state projection kept separate |
| One-ledger redesign reaches event schemas / store admission / producer evidence (wide blast radius) | Accepted as the chosen end state (DEC-14/17); pair-authority schema design in §6.4d + replay/kill-mid-flight tests; two-linked-ledger considered and rejected |
| Fungible workers terminalize finality at different depths | Depth resolved from a certified policy at admission (§6.4b), identical for all workers and replay; a worker lacking the capability declines the claim |
| Signer cannot reproduce a recorded anchor → wedged lane | Defined terminal disposition (DEC-20): reconcile by the recorded anchor; failure-with-lane-release or policy manual-block, never a silent wedge |
| Resuming a side-effecting run without reconciliation | Resume is deferred to the AC/DC + saga effort (DEC-21/23); in v1 each run has a single driver, so the case does not arise |
| Non-deterministic signer slips in | Enforced by the re-sign == anchor assertion (DEC-15): fails closed, never double-submits |
| Herd on the claim table (resume sweep / concurrent invokers) | `NowaitSkip` try-acquire: losers pay one `UPDATE`; a duplicate invoker attaches rather than starts; no feed scanning (DEC-22) |
| Postgres as the coordination/scaling ceiling | Per-run and per-lane locks parallelize disjoint work; materialized lane projection (WS-E) removes the O(history) fold |
| Wallet wedged by a stuck/slow verify (live driver) | Scheduler invariant (§6.5) keeps a runnable node the live driver re-drives; the lane releases at the ledger's proven terminal (DEC-25), never on a timeout. Dead-driver resume is the deferred saga effort |
| Observation-feed frontier lag (`pg_snapshot_xmin`) | Affects status/watch freshness only; dispatch is invoker-driven and resume reads the claim table directly — no dispatch coupling (DEC-22/23) |
| `ManualResolution` holds a wallet lane | Only when the ambiguity itself needs an operator (rare; most ambiguity auto-reconciles); blocks only same-signer effects; mitigate operationally (per-class signers, fast alerting) (DEC-25) |

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

- **Automatic resume + dead-driver takeover + reconciliation-on-takeover (DEC-21/23)** — part of the
  larger AC/DC + saga recovery effort, not this RFC.
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
| Observation surface (app/REST/CLI) | `app/src/lib.rs:1157`; `bin/rest-api/src/lib.rs:502`; `bin/cli/src/commands/run/list.rs` |
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
| Side-effect terminal output (confirmation-only today) | `program/src/lib.rs:1561`; `side_effect_lifecycle.rs:298` |
