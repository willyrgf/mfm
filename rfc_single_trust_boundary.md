# RFC: one trust boundary

Status: draft for review on `refact-runtime`

Supersedes: `execution_flow_analysis.md` and `explain_added_net_loc.md`. Both documents are
deleted by this RFC. Their verified factual findings are carried forward in the evidence index
(Appendix A) and the measurement summary (Appendix B); their *positions* are revised where §
"Supersession map" says so. Everything in this RFC that states a file:line fact was re-verified
against the tree during drafting, not inherited on trust.

---

## Summary

The system stations full verification at every internal seam. Each verification layer is locally
justified — the two superseded analyses proved that at length, verdict by verdict — but the layers
are each other's alibi: every one re-establishes at runtime a property an adjacent layer already
establishes, and the redundancy is then cited as why both are needed. The measurable results:

- **Θ(N²) execution.** One drive action performs four full genesis re-verifications of the run
  prefix. An EVM submission run (~40 states, ~120 batches) executes on the order of twenty
  thousand reduce/compile/compare cycles to perform roughly eleven RPC calls.
- **A product that does not run.** Both shipped binaries fail closed unconditionally. The only
  complete execution path in the repository is an integration test.
- **~165k lines** for two published entry points, with the largest single costs concentrated in
  re-verification machinery and a boot-time authority ceremony.

The design's crown jewel — an append-only, content-addressed, CAS-headed journal — exists
precisely so that a verified prefix cannot change out from under anyone. No component acts as if
it believes this. This RFC draws **one explicit trust boundary** (bytes entering the process trust
base: database cold loads, RPC/provider responses, imported exports, configuration streams),
keeps full hostile verification exactly there, and deletes the runtime shadows everywhere else.

The proposal is four phases: (1) one verified load per action over an incrementally folded
projection; (2) verification placed at real boundaries only; (3) a deployment authority the
binaries can actually construct, or deletion of the transports; (4) the mechanical consolidations
the superseded LOC analysis already costed. Phases 1–3 were each deepened by a dedicated
architect pass; their target designs, cutover scopes, and commit sequences are §4–§6.

---

## 1. Problem

### 1.1 The compensating verification stack

The execution path re-proves the same facts at four stacked seams. Every claim below is verified
at the cited line.

**Per drive action — four full loads.** Each load runs `qualify_recorded_history` plus `replay()`
(`crates/kernel/store/src/structured/semantic_open.rs:211`), which executes `replay_step` —
a full `reduce_event` + `compile_preview` + `ComparedReduction::compare` + `discharge` — for
**every batch since genesis**. No projection survives between loads.

| # | Site | Stated purpose | What already proves it |
| --- | --- | --- | --- |
| 1 | `crates/app/src/production_structured.rs:435` | tenant check — evidence used for one field compare (`:777`), then discarded | load #4 repeats the identical check |
| 2 | `crates/kernel/runtime/src/structured.rs:488` | select the next action | — (the one load-bearing load) |
| 3 | `runtime/structured.rs:1036` | re-base after committing the authorization | the commit already returned the committed record (`:1006–1022`); `classify_backend` already compared it to the candidate |
| 4 | `production_structured.rs:443` | fetch the new journal head | `drive_once` held the post-commit head and threw it away |

**Per append — the writer verifies itself twice.** `commit_event`
(`crates/kernel/store/src/structured/coordinator.rs:63`) reduces the event as intent (`:78`),
compiles, re-qualifies its own candidate as hostile input (`qualify_recorded_successor`, `:97`),
reduces the candidate again as recorded (`:121`), compares the two reductions (`:127`), and then
compares the backend's returned batch against the retained candidate (`classify_backend`,
`:144`).

**Per authorization — a third proof of durability.** After `commit_event` returns the committed
record, and after `classify_backend` proved the backend persisted exactly the candidate, the
runtime reloads the entire run (`:1036`) and re-checks the record field-by-field
(`:1075–1102`) — a third proof, via a third code path, of a fact established twice in the
preceding microseconds.

**Compile-time guarantees kept alongside their runtime shadows.** Grant typestates make a
wrong-grant call uncompilable; the backend keeps `debug_assert_eq!(call.grant(), …)` anyway
(`production_structured.rs:434`). The typed `OperationBuilder` makes ill-formed programs
unrepresentable; admission then runs the full 11,593-line hostile certifier over the program the
same process authored one call earlier.

**The cost, multiplied out.** Actions ≈ N, loads per action = 4, each load O(prefix). For a
~120-batch submission run: ≈ Σ 4·i ≈ **29,000 replay steps** through the 2,920-line reducer, to
execute ~40 states whose useful external work is ~11 RPC calls. It is worse than Θ(N²): every
load also re-executes every retained prior-run fact selection, recursively reloading and
re-reducing each producer prefix (`fact_scan.rs::qualify_and_reduce_for_scan`), so the true
shape is **Θ(N²·P)** with P the producer closure. The contract asks for none of this:
`docs/run-execution.md:173` requires "a reviewed disposition after at most one action" — one
action, not four re-verifications of it.

**The proof that it is unnecessary ships in the same file.** `verify_incremental_equivalence`
(`semantic_open.rs:273`) proves fold(state, batch) == full replay of the prefix — i.e. proves a
cached projection folded forward is exactly equivalent to what production recomputes from genesis
four times per action. It is compiled only under `cfg(test, feature = "test-support")`.

### 1.2 The product does not run

`connect_application` (`bin/cli/src/support/application.rs:28`) returns
`Err(AuthoritativeWriterFenceUnavailable)` unconditionally; `bin/rest-api/src/lib.rs:100` is the
same shape — and the standalone REST binary calls it before binding its listener, so **even
`/v1/health` is unreachable from the shipped binary** (the health route is live only for an
embedder calling `make_app`; both superseded analyses overstated the reachable surface). Every
`mfm run`/`mfm ops` subcommand dead-ends the same way. That is ~2,880 + 638 source lines of transport (plus 2,006 lines of their
tests), including 710 lines of atomic-pair file output for an export command that cannot execute.
There is no complete execution path in the repository. Failing closed is correct given the absent
authority — and the REST crate documents embedding as the intent (`lib.rs:95–99`) — but
shipping 6k lines of transport over an unconditional error, indefinitely, is not a design; it is
a diorama unless the embedder is named or the authority is supplied.

### 1.3 Boot-time authority ceremony vs per-write fencing

The three-party Begin/Finish bracket (`crates/app/src/application.rs:403–532`,
`crates/storages/evm-postgres/src/provider.rs`, `crates/live/evm/src/transport/inventory.rs`,
`tests/wallet-authority-provider`) costs ~7,500 lines to attest, once at boot, that transport,
signer generation, catalog, and wallet incarnation are simultaneously current. Independently,
**every actual mutation is fenced**: the wallet authority validates activation and revalidates
retained state inside each fenced write (`crates/storages/evm-postgres/src/authority.rs`), and
the run store CASes every append behind a fence validated at semantic open. The bracket's wire
protocol is declared twice — client and test server — and the two declarations **have already
diverged** (`ProviderMutation`: 3 variants at `provider.rs:1020` vs 5 at
`tests/wallet-authority-provider/src/lib.rs:70`; a client-only `Integrity` reply variant; one
bound read from `mfm_evm`, the other hardcoded).

### 1.4 Hostile certification at a friendly boundary

`mfm-certify` (11,634 src lines, one 11,593-line file, 416 rejection sites) is the choke point
where "an untrusted serialized program becomes executable authority." On the only production path,
the program it distrusts was authored by the adjacent statement, from compiled-in typed builders,
inside the same process (`production_structured.rs` `prepare_*` → `certifier.certify_document`).
The genuinely untrusted arrivals of a program — a persisted document loaded from the database, an
imported export verified offline — are real boundaries and keep full validation. Admission is
not one. (Expansion — the deterministic lowering of capability recipes into visible states — is a
real transformation and stays at admission regardless; see §5.)

### 1.5 Mechanical debt

Carried from the superseded LOC analysis, corrected estimates intact: the 11,593-line certify
file (≥5 separable concerns), ~108 duplicated test fixture structs and 76 `impl State` blocks
across four fixture corpora, the diverged provider wire pair, three hand-synchronized copies of
the commit-ambiguity protocol in `authority.rs`, and the 1,035-line normalizer/denormalizer codec
in `mfm-spec` that re-implements, imperatively and with seven hard-coded field names, bounds the
schema system cannot express for recursive types. Detail and costing: Appendix B and §7.

### 1.6 What is *not* a problem (carried-forward verdicts)

The superseded analyses were right about the bricks, and this RFC keeps them:

- The journal: append-only, content-addressed, five strict record families, derived append ids
  for idempotent retry, store-derived run ids for idempotent admission.
- One reducer as the sole interpreter; the scheduler as one `min_by` over occurrence paths.
- The crash-closure semantics (`close_parked_attempt`): the only observation a runtime can
  synthesize without lying, scoped to declared `EntryAbsorbing`.
- `SafeFailureDisposition` and the typed authoring surface; the affine
  authorize → invoke → observe bracket ("the result cannot escape recording").
- The purpose-sealed evidence newtypes and the 81 compile-fail tests.
- `mfm-authority-seal` (38 lines), the keystore (1:1 test ratio, correct), the zeroizing
  bounded JSON-RPC scanner in `transport/exact.rs` (its four properties are real; serde does not
  provide them), the export source-closure authorization.
- The nonce authority's placement outside the run journal (a nonce is a cross-run invariant),
  and its permanent operation-key idempotency.
- The 1,318-line runtime that replaced a 35,130-line crate — the proof this team can collapse
  layers when it decides to.

---

## 2. Design principle

Add to `docs/design.md`, and hold every phase to it:

> **The process trust base is one boundary.** Bytes are verified in full when they enter it —
> database loads not previously verified by this process, RPC and provider responses, imported
> exports, configuration revisions — and are not re-verified after. Components inside the
> boundary prove properties to each other by construction (types, seals, content addressing,
> CAS heads), never by repeated runtime re-derivation. A property enforced at compile time has
> no runtime shadow; a fact proven by an append's returned record is not re-proven by reloading.

Everything below is this principle applied four times.

---

## 3. Solution overview and phase order

| Phase | Delivers | Depends on |
| --- | --- | --- |
| 1 | one verified load per action; projection cache + fold-forward; Θ(N²) → Θ(N) | — |
| 2 | verification at real boundaries only; write-XOR-read decision; certification placement | 1 |
| 3 | deployment authority the binaries can construct (or transport deletion); one wire contract | independent of 1–2 |
| 4 | mechanical consolidation (certify split, fixtures, AccessKind, spec flattening) | 2 (certify), else independent |

Each phase is a coherent cutover: no compatibility paths, superseded code deleted in the same
commit sequence, `docs/design.md` / `docs/architecture.md` / `docs/run-execution.md` updated in
the same change as the code they describe.

---

## 4. Phase 1 — one verified load per action

### 4.1 Contract finding: genesis replay per load is not the contract

No sentence in `docs/design.md`, `docs/architecture.md`, or `docs/run-execution.md` requires
re-qualification and re-reduction from genesis on every load. The load-bearing language:

- `docs/design.md:226-229` — "Every Runtime drive loads a verified predecessor from one backend
  snapshot and compares its complete current projection. … No historical successor or head-only
  cache answer can become store authority." It demands one snapshot, projection comparison, and
  that no *head-query-free* cache answer become authority. It does not say "from genesis."
- `docs/design.md:224-225` — "Reducing every complete prefix from raw persisted batches must
  produce an equivalent verified run" — an equivalence over the reduction rule, not a
  prescription that each load recompute it.
- `docs/run-execution.md:150-153` — "**A fresh process** opens through the same deployment
  fence, loads the full prefix, re-qualifies and re-reduces it, and continues from the same
  cursor." Full-prefix re-verification is scoped to a fresh process. This is the strongest
  textual evidence that per-load genesis replay is an implementation artifact.
- `docs/design.md:568-569` — "the runtime mints invocation authority only after re-reading that
  exact committed authorization" — the one sentence that mandates load #3; §4.3(c) edits it
  deliberately.
- Caching is already a sanctioned pattern: the persisted-program verifier's "cache is keyed by
  certified content identity" (`design.md:121`, `:172-173`; in code
  `ProgramVerificationRegistry`, `qualification.rs:42-105`). The projection cache is the same
  pattern keyed by `(run_id, exact journal head)`.

**The tamper-evidence rationale is refuted by the contract itself.** `docs/design.md:549-563`
("Append-only authority and the absent witness") and `docs/known-gaps.md:12-30` accept that "a
rolled-back database is a valid earlier state of itself: heads chain, digests verify, and
reduction is consistent at the earlier revision." Per-load genesis replay provides **zero**
rollback protection — a rewound prefix replays clean. What it detects is in-place mutation of an
old batch between two loads inside one process lifetime; the incremental design narrows that
detection to cold loads and the suffix chain check, exactly the absent-witness posture the
contract already accepts for the strictly stronger rollback case. The invariant that must
survive without exception is different and is preserved structurally: "no external effect may
occur that was not durably journaled first" (`design.md:565-567`).

Verdict: a deliberate but narrow contract update — two `design.md` sentences and their
`architecture.md` echo — not a silent implementation change. Everything else (one snapshot,
head-first anchoring, full hostile qualification of never-before-verified bytes, fresh-process
full replay, no second reducer, no mutable cursor row) is preserved verbatim.

### 4.2 Target design: the run projection cache and the incremental fold

**The cache.** One store-private type, one instance per opened store
(new `structured/projection_cache.rs`, ~130 LOC):

```
pub(super) struct RunProjectionCache { entries: Mutex<BTreeMap<RunId, CacheEntry>> }
struct CacheEntry { run: VerifiedStructuredRun, canonical_bytes: usize, objects: usize, lru_tick: u64 }
```

- Created in `qualify_and_open_structured_store` beside the `backend`/`programs`/`physical`
  `Arc`s; a fourth field of `StructuredRunHistoryWriter` and `StructuredRunHistoryReader`. Every
  purpose reader wraps a clone of the same reader, so the writer, the Runtime adapter, the
  coordinator's retry/stale-head classifications, and all six purpose readers share one cache
  with no new wiring. Only `load_and_compare` learns the type; it is never public, never a
  purpose surface, never serialized.
- Concurrency: a `Mutex` around the map; lookups clone out (`Arc` bump + `ReducedRunState`
  clone — both cheap; `ReducedRunState` is already `Clone + Eq`), so the lock is never held
  across `.await`. Installs are monotonic per run (strictly greater `run_sequence` wins);
  install sites are exactly two: a fully verified load/fold, and `classify_backend` on
  `NewlyCommitted`/`ExistingSame` installing the `PreparedAppendProof.successor` it currently
  drops. `StaleHead`/`AcknowledgementUnknown` never install.
- Bounding: LRU by retained canonical bytes with one constant,
  `RUN_PROJECTION_CACHE_CANONICAL_BYTES = MAX_RUN_HISTORY_CANONICAL_BYTES`. Per-run cost is
  already bounded by the run-capacity contract; no new limit concept. Eviction affects cost
  only, never correctness.
- **Authority rule:** the cache is a pure accelerator with zero authority. No load returns
  cached state without first reading the backend's indexed head in a fresh snapshot; cached
  state is reusable only at the exact head (sequence and commit digest) that snapshot reports;
  any disagreement discards the entry and falls back to the full genesis path.

**The load protocol.** Reshape the one backend read loads use:
`StructuredHistoryBackend::load_snapshot(run_id, after: Option<&JournalHead>, limit)` returning
the current projection plus batches strictly after `after`. PostgreSQL implements it inside the
existing head-first repeatable-read transaction — head row first (the snapshot point), then
`WHERE run_sequence > $after`; a warm no-change load is **one indexed SELECT**. `load_and_compare`
becomes: (1) snapshot the cache entry; (2) suffix-load after the cached head with the remaining
byte/object budget; (3) run absent → drop entry, `RunNotFound`; (4) empty suffix + projection
equal (sequence, commit digest, tenant, attention) → reuse; (5) non-empty suffix → **fold**: per
batch, `qualify_recorded_successor` (full hostile-input qualification — its envelope check binds
the first suffix batch to the cached head's exact digest) then `replay_step`, then the scoped
fact-selection re-verification below, then the existing final projection compare; install;
(6) regression, chain mismatch, or any fold failure → discard the entry and run today's full
path byte-for-byte (cold start goes straight here). **Cold loads stay fully verified, including
the complete producer-closure re-execution.**

**Fact selections under the fold.** Factor the barrier loop of `qualify_and_reduce_for_scan`
into `verify_fact_barriers(...)` and scope it, under a fold, to barriers whose authorization or
`Returned` observation arrives in the suffix. Cost model: each retained selection is re-executed
once per process when its evidence first appears — not once per load with its full recursive
producer closure. `PrefixVerificationMemo` stays session-scoped and unchanged: its `Visiting`
state is cycle detection and its `verified_prefixes()` is the offline-closure completeness
witness; a global memo would silently satisfy closure checks with prefixes an export bundle
never supplied, and its historical-head keying would make a merged cache miss precisely where it
matters. The projection cache removes the reason the memo was ever hot.

**Commit returns its successor.** `RuntimeHistoryPort::commit_event` returns
`(RunId, StructuredAppendAttempt, Option<VerifiedRun>)` — the verified successor on
`NewlyCommitted`/`ExistingSame`, `None` otherwise. The store already holds it
(`PreparedAppendProof.successor`; today used once to mint the fact-scan capability and then
dropped). No new port method, no runtime-side reducer: the successor is the value the
coordinator's requalify-reduce-compare-discharge pipeline sealed, which
`verify_incremental_equivalence` proves equal to genesis replay. Within one drive action the
runtime chains verified state — load → (commit → successor) → invoke → (commit → successor) —
with zero reloads.

### 4.3 Load collapse: four → one

**(a) Tenant check from the runtime's own load — deletes load #1.** `Runtime::drive_once` gains
`expected_tenant: &TenantScopeId` and rejects
`verified.admission().tenant_scope_id != expected_tenant` with the same public `run_not_found`
the facade produces today, checked before any action executes. The run's admission is already
the canonical tenant binding (`run_id = derive_run_id(store, tenant, entry_op, invocation)`,
re-derived by qualification), so evidence and keying agree by construction. The facade's
`drive_once` becomes `runtime.drive_once(run_id, call.tenant_scope_id())` and nothing else.
(`load_public_authorized` survives for the read endpoints, where it is the read.)

**(b) Runtime returns the post-action head — deletes load #4.** `drive_once` returns
`(DriveOutcome, JournalHead)`: the committed successor's head for
`TransitionCommitted`/`AccessObserved`, the loaded verified head otherwise. `DriveOutcome`'s
variants keep their exact shape; `DriveResponse::from_runtime` already takes the head
separately, so the facade change is deleting the load. Wire shape unchanged.

**(c) Fold-forward replaces load #3 — and the honest accounting of the recheck block.**
`authorize()` on `NewlyCommitted` takes the commit-returned successor as `Authorized.verified`
and deletes the reload. All eight checks at `runtime/structured.rs:1075-1102` become
tautological under the new construction — the proof and the successor are minted together from
the same `PreparedAppendProof`, only after `classify_backend` byte-compared the backend's
committed batch to the sealed candidate. In particular the `predecessor_head == pre_fault_head`
check ("created by this worker on this exact predecessor") is now enforced *strictly more
strongly* upstream: the commit consumed the very `VerifiedStructuredRun` whose head is
`pre_fault_head`, and the CAS succeeded only against it. None of the eight survives as code;
what survives is the **invocation precondition, restated structurally**: a
`CommittedAccessAuthorization` can exist only if the backend positively acknowledged
`NewlyCommitted` for the exact worker-sealed batch, whose predecessor was this worker's own
verified head. The deleted reload proved *visibility to a second snapshot*, not durability — an
acknowledged-but-unflushed commit is visible to an immediate same-process re-read anyway, so the
reload never strengthened the durability claim against the only failure mode it could address.
`ExistingSame` and `AcknowledgementUnknown` continue to mint nothing and invoke nothing.

Result: one verified load per drive action (the selection load), zero reloads inside the access
bracket, zero facade loads.

### 4.4 Cutover

**Deletions:** both drive-path `load_public_authorized` calls
(`production_structured.rs:435-436`, `:442-445`); the post-authorization reload and the whole
recheck block (`runtime/structured.rs:1036-1044`, `:1075-1102`);
`CommittedAccessAuthorization.{predecessor_head, successor_head}` fields, constructor
parameters, and accessors (the successor carries its own heads). No compatibility paths: the
suffix-load signature replaces the full-only one; the four-load drive shape has no preserved
variant.

**Contract edits (exact):**
- `design.md:226-229` → "Every Runtime drive performs exactly one verified load. A load reads
  the indexed head and any batches after this process's last verified head from one backend
  snapshot; the head query is the snapshot point for a non-mutating load, and any later append
  is handled by the append path's exact-head compare-and-append. Batches this process has never
  verified are fully qualified and folded forward by the same reducer, retained fact selections
  are re-verified when their evidence first appears, and a run first seen by this process is
  replayed from genesis. A cached reduction is evidence only at the exact backend head it was
  verified against; no head-query-free cache answer can become store authority, and a suffix
  that does not chain onto the cached head discards the cache and replays from genesis. A
  committed append returns the verified successor produced by the same requalification and
  reduction that sealed it; that successor is the predecessor for the next action without a
  reload."
- `design.md:568-569` → "the runtime mints invocation authority only from the backend's direct
  positive acknowledgement of that exact committed authorization append; an ambiguous, stale,
  or idempotent acknowledgement mints nothing and no invocation occurs."
- `architecture.md:173-176` and `:306-310` updated to match (one-load drive; suffix-after-head
  loading); `run-execution.md:150-153` stays verbatim (already fresh-process-scoped);
  `known-gaps.md` needs no edit under the accept-and-replay regression policy.

**Tests:** the sixteen `structured_runtime_causal.rs` scenarios are preserved
assertion-for-assertion; ~31 direct
`drive_once` call sites take a mechanical two-token edit for the new signature.
`verify_incremental_equivalence` is re-pointed at the *production* fold function, becoming the
cache's permanent proof harness; add two targeted tests (fold across a foreign suffix containing
a fact barrier; regression discards the entry and re-verifies).

**LOC:** net ≈ +300 product lines (cache module + suffix loads + successor plumbing − reload −
recheck − facade loads). The deletion is small in lines because what Θ(N²·P) burned was *calls*,
not code; the win is the removed work and the two removed load sites per action.

**Commit sequence:**
1. `store: retain canonical byte and object totals on qualified history`
2. `store: return the verified successor from commit_event`
3. `store: load run snapshots as suffixes after a verified head`
4. `store: add the run projection cache and incremental fold to verified loads`
5. `runtime: check tenant scope and return the journal head from drive_once`
6. `docs: reconcile load and drive contracts`

### 4.5 Phase-1 material uncertainties

1. **The "complete current projection" reading.** `design.md:226-229` is ambiguous between "the
   comparison covers the complete projection value" (satisfied by the fold) and "the projection
   is recomputed completely." If the stricter reading was intended, this phase is a deliberate
   contract weakening and says so; approving the §4.4 sentence is approving the reading.
2. **Regression policy.** On observed head regression the design discards the entry and
   replays (accept-and-replay, keeping `known-gaps.md` true verbatim). The cache incidentally
   holds exactly the "memory of a later head" the absent-witness section says nothing holds; a
   fail-closed `InvalidHistory` alternative (precedent: `configuration_heads`) is one error path
   away if review prefers it — though a restart trivially bypasses it, which is why it was not
   chosen.
3. **Interior-batch mutation detection narrows to cold loads.** Confirm no operational runbook
   treats per-drive load as an integrity sweep; the semantic-open audit remains the deliberate
   full sweep at process start.
4. **Test-invariance literalism.** If "causal tests pass unchanged" must be byte-literal, the
   tenant check stays in the facade as a cache-served one-SELECT load instead of moving into
   `drive_once`; the design chose the structural collapse and the mechanical edit.
5. **`ConcurrentProgress` head semantics.** The reported head becomes this worker's verified
   head, not whatever newer head the deleted post-drive load happened to observe. Check
   `DriveResponseWire` consumers; wire shape is unchanged.
6. **Foreign-suffix fact-selection cost in multi-worker deployments** (each worker re-verifies
   each selection once). Measured by the existing `fact_scan_counters` instrumentation; any
   producer-cache follow-up is gated on production numbers, deliberately excluded now.
7. **PostgreSQL suffix-query numeric boundary** (`run_sequence > $after` under the text-bound
   numeric encoding). Wrong means a fold degrades to a safe full replay, never wrong evidence;
   extend the numeric-ordering test to the suffix form.

---

## 5. Phase 2 — verification at real boundaries only

**Trust-base definition (normative for this phase).** Bytes are *hostile* when they enter the
process from outside its own call chain — PostgreSQL rows (cold loads, idempotency lookups,
projection reads), RPC/provider responses, portable export bundles, offline replay closures,
configuration streams, caller request bodies. Bytes are *friendly* when produced by this process
inside the same call chain and carried in affine, workspace-sealed types (`ValidatedRunAppend`,
`CommittedAccessAuthorization`, `PendingSemanticStep`, `AuthorizedRunCall<'_, G>`).

### 5.1 Boundary map (result)

The full site-by-site map was produced during the architect pass; its conclusion: **every real
boundary already has exactly one owner** — store qualification/reduction for persisted bytes,
`verify_root` for program documents, the obligation checkers for release currency, the policy
for caller authority. The friendly seams, each with the adjacent mechanism that already proves
the same fact:

| Friendly seam | Site | Already proven by |
| --- | --- | --- |
| write-side requalify + second reduction + compare | `coordinator.rs:95-134` | the compiler derived those exact digests one call earlier; reducer determinism is a testable law, not a per-append property |
| positive-echo compare on `NewlyCommitted` | `coordinator.rs:329-345` | the backend echoes the value it took out of the sealed command it was moved (`ValidatedRunAppend` is store-constructible only) |
| runtime field-by-field recheck of own committed record | `runtime/structured.rs:1075-1102` | the proof was minted from the sealed candidate inside the byte-compared `NewlyCommitted` arm |
| grant runtime field + seven `debug_assert_eq!` | `application.rs:942/956`, `production_structured.rs:434,…` | `run_grant` marker consts + `AuthorizedRunCall<'_, G>` — wrong grants do not compile |
| hostile predicate pass at admission authoring | `production_structured.rs:620-623/711-714` → `certify_document` | typed `OperationBuilder` (illegal programs unrepresentable) **and** store ingress runs `verify_root` on the same document before any byte persists |
| postgres pre-DML coherence block | `storages/postgres/src/structured.rs:611-628` | the `ValidatedRunAppend` seal: only `coordinator::prepare_append` can construct it |

Real boundaries that **stay exactly as they are**: `qualify_admission_intent` (store ingress
choke point — same code that qualifies hostile cold loads; must remain total), the
configuration-stream resolution and semantics re-derivation against the sealed signer,
idempotent-retry byte-equality against the DB-retained attempt, obligation discharge
`RetainedAndCurrent` (cross-authority release currency, not self-history), `StaleHead`
re-load-and-compare, the export closure authorization, offline replay, the semantic-open sweep,
and the entire cold-load path.

### 5.2 Decision: verify-on-read

**Delete write-side re-verification (pipeline steps 7–9). Cold loads remain fully verified.**

The criterion: verification belongs where bytes enter the trust base, and only there.
Self-written history enters the trust base when read back from PostgreSQL. Cold loads therefore
remain fully verified *no matter what* — so the write/read XOR is only realizable by deleting
the write side; choosing verify-on-write would mean paying both forever. Cost agrees: the
write-side compare doubles reduction on every append for the run's whole life, while read-side
verification under the phase-1 cache amortizes to one cold verification per process/run plus a
fold per externally-appended batch. The write side's unique catch — drift between the reducer's
`Intent` and `Recorded` arms, or compiler round-trip infidelity — is a deterministic first-party
property, provable once by the equivalence law (`verify_incremental_equivalence`, promoted to
the production fold by phase 1), not a per-append runtime property.

**New write pipeline:** `reduce_event(Intent)` → `compile_preview` (the compiler now owns the
binding token that `ComparisonPassed` carries today) → `discharge(RetainedAndCurrent)` → seal →
append. The successor binds from the intent reduction plus a construction-only
`QualifiedHistory::append_authored(candidate)` (object-index extend + `qualify_batch` without
envelope re-derivation), replacing `qualify_recorded_successor`.

**The write path retains:** exact-head CAS predecessor binding (construction, not
verification); the run-capacity budget with the two-batch authorization reserve (rehosted onto
retained verified totals); obligation discharge; idempotent-retry byte-equality; proposal-ingress
canonical/schema validation inside the single intent reduction. It retains **no** envelope
requalification, **no** second reduction, **no** semantic self-compare, **no** positive-echo
compare.

### 5.3 Runtime shadows — dispositions

- **`runtime/structured.rs:1075-1102` — delete** (phase 1 already deletes the reload it
  depends on). The recheck cannot provide durability beyond the SQL commit ack — a database
  that loses the write after the microsecond read-back window passes it anyway, and the
  absent-witness doctrine declares that class undetectable. "No external effect ahead of its
  durable journal record" is carried by *invoke only on `NewlyCommitted` from a real commit*,
  which is untouched.
- **Grant `debug_assert`s — delete, plus their subject:** the runtime
  `grant: ApplicationAccessGrant` field and accessor on `AuthorizedRunCall`, and the redundant
  `grant` parameter of `authorize_run` (derive from `G::GRANT`). One source of truth: the
  marker const. None of the 16 app compile-fail tests references `grant()`.
- **Double reduction + compare — delete** (§5.2). `semantic_eq` and the
  `intent: Option<PendingSemanticStep>` parameter of `ComparedReduction::compare` go;
  `replay_step`'s recorded-path call becomes the only signature. The reducer's dual
  `Intent`/`Recorded` arms remain — proposals carry values, records carry refs — and their
  equivalence becomes a tested law.
- **`classify_backend` — split by value provenance.** `NewlyCommitted` becomes **payloadless**
  (the echo compare re-proved an in-process fact; the coordinator uses its retained candidate;
  the runtime-facing outcome payloads are already dead — every consumer matches `(_)`).
  `ExistingSame` is the **hostile half** — postgres reconstructs it from database rows — so the
  byte-equality against the retained candidate stays, and the coordinator becomes its *sole*
  owner (backends return the reconstruction unconditionally instead of pre-deciding
  `ExistingSame | AppendConflict`). `StaleHead` re-load-and-compare stays. The postgres
  pre-DML coherence block is deleted; the stored-frame byte bound stays (write-budget
  construction).

### 5.4 Certification placement

Expansion stays at admission; the hostile predicate set runs only where persisted program bytes
enter authority — and that owner **already exists**: `AdmissionVerificationRegistry::verify_root`,
invoked by store qualification for both admission intents and cold loads (memoized by exact
content identity), and by offline replay through the trust snapshot. Nothing new is built at
the boundaries; the change is to stop running the hostile pass a *second* time at authoring.

- `certify_document` → expansion-only `expand_document`: entry-signature match, the expansion
  transformation with its profile bounds (bounds stay — they make the transformation total and
  protect the process; they are not a hostile check), document assembly, closure hashing.
  Dropped from authoring: `validate_authored_program`, `validate_expanded_program`,
  `validate_policy_coverage`, the support-envelope subset check, `verify_certified_document`.
- **Why nothing is lost:** every admission still passes the full hostile pass exactly once
  before any byte persists — `qualify_admission_intent` → `programs.qualify` → `verify_root`
  on memo miss, mapped to `CandidateRejected`. A typed-builder bug is caught there, pre-append.
- **`CertifiedProgramDocument` persisted shape: unchanged.** No schema, content-identity, or
  export byte changes. Registry-build coverage certification stays full (assembly is a real
  ingress and runs once). Component objects consumed by offline replay: unchanged.
- The placement decision **is** the module boundary for the phase-4 file split:
  `expansion` / `validate` / `registry` / `process`. After the split, the rule is an import
  direction: `mfm-app` imports `expansion`, never `validate`.

### 5.5 Phase interaction with phase 1 (normative)

Phase 1 lands first, with the commit-returned successor produced by the still-complete
pipeline; its promotion of `verify_incremental_equivalence` to the production fold is the
standing guarantee. Phase 2 then removes the per-append compare, at which point the successor
is intent-derived (`append_authored`) and the equivalence law — running over the causal corpus,
the qualification corpus, and the three production programs — is the replacement for the
deleted check. The cache adopts the append-time successor and folds externally-appended batches
via `replay_step` (resolving the sibling-scope question both passes raised). Cold loads
recorded-verify everything regardless, so a long-lived writer's history is re-derived from
recorded bytes at every process start and by every other process.

### 5.6 Cutover

**Deletions and LOC:** runtime recheck (−28); dead outcome payloads (−15); grant
field/asserts (−30); `qualify_recorded_successor` → `append_authored` (−25);
steps 7–9 + capacity rehost + payloadless classify (−55); `semantic_eq` + compare param +
token move (−30); backend echo/coherence (−35); `expand_document` plumbing (+40); ~40 changed
doc lines. Net ≈ **−180 to −220 src LOC** and, more importantly, one reduction instead of
two-plus-requalification per append and one hostile certification instead of two per program
root.

**Contract edits (design.md, exact):** the reduction typestate block loses `compare` on the
write path (compiler owns the binding token); "Comparison alone owns the private token…" → the
compiler; the backend sentence becomes "a backend applies the sealed command mechanically — it
returns no substitutable positive batch"; "requalifies those exact bytes, and reduces the
recorded form…" → "Candidate authoring reduces an intent once and lets the compiler author and
bind bytes. Recorded-form verification of those exact bytes belongs to the read boundary…
intent/recorded reduction equivalence is a tested law of the one reducer, not a per-append
runtime check" (the adjacent "Reducing every complete prefix… must produce an equivalent
verified run" stays verbatim — it is the law the read boundary enforces); the invocation
sentence per §4.4. `run-execution.md`: normalization sentence → "a positive backend
acknowledgement carries no batch to substitute; an idempotent duplicate is compared
byte-for-byte against the store-retained candidate"; admission paragraph "certify it" →
"expands it into the certified document" (store re-certification sentence stays).
`architecture.md`: drop "comparison," from the affine-boundary list.

**Tests:** the 81 compile-fail tests untouched. `structured_runtime_causal.rs` loses the
`ObservationSubstitutedPositive` injection arm (unrepresentable once `NewlyCommitted` is
payloadless — the test asserted a check whose attack the type system now forbids); postgres
history tests take mechanical variant updates. New: the equivalence law wired into the causal
corpus; one certify test proving `expand_document` output ==
what `verify_root` reconstructs. Unaffected: `structured_history_qualification.rs` (the suite
that matters most now) and the memo-substitution rejection test.

**Commit sequence:**
1. `split certify structured into expansion, validate, registry, and process modules`
2. `author admission documents through expansion without the hostile predicate pass`
3. `delete the redundant runtime grant field and grant debug asserts`
4. `mint invocation authority from the committed authorization append`
5. `seal appends from the intent reduction`
6. `acknowledge newly committed appends without echoing the batch`
7. `exercise incremental reduction equivalence in the causal corpus` (may swap with 5 for
   bisectability)

### 5.7 Phase-2 material uncertainties

1. **Silent writer drift after deleting the write-side compare.** The reducer's `Intent` and
   `Recorded` arms could diverge on a legal input in a future edit; the writer's intent-derived
   successor would differ from what readers derive — undetected until the read boundary rejects
   something. Bytes stay authoritative and all readers agree with each other; the writer would
   select a wrong-but-legal action. Mitigation: the equivalence law over the full corpus;
   long-term, unify the arms behind one `ResolvedEvent` so drift is unrepresentable.
2. **The recheck as an operational canary.** Deleting the read-back removes a theoretically
   worthless but possibly-watched signal for immediate write loss. Needs ops sign-off; the
   crash-injection scenarios cover the recoverable classes.
3. **External consumers of `HistoryAppendOutcome` payloads.** Workspace-wide, every consumer
   matches `(_)`; confirm with the embedding deployment (if one exists — §6) before commit 6.
4. **Admission bounds under expansion-only authoring.** If some profile bound is checked only
   in `validate_expanded_program`, commit 2 must move that counter into `ExpansionContext` so
   authoring cannot blow up before `verify_root` runs. Resolve while doing the module split.
5. **The coverage-template fast path** (`certify_document` returns the cached coverage document
   for template-identical candidates) must survive in `expand_document`, or submission
   admissions get slower. Mechanical; easy to drop by accident.

---

## 6. Phase 3 — deployment authority: one admission round, and binaries that run

**Inventory correction.** The "~7,500-line handshake" decomposes as: client protocol
`provider.rs` 2,203 (of which ~800 is the **per-write** lease client — not boot-time, not up
for deletion), transport choreography `inventory.rs` 633, app-side bracket ~530, test server
3,796 (+692 fault proxy, bracket-unrelated), plus the integration loopback's qualification
handler. Boot today is **four provider rounds** plus N private-endpoint HTTP exchanges.
Additional divergence found beyond §1.3: the deployment proof byte bound exists in **three
inconsistent copies** (client 4,096 via `mfm_evm`; inventory 16,384; server 16,384) — masked
today only because proofs happen to be 32-byte HMACs.

### 6.1 Authority overlap: the residue

The bracket attests six properties. Compared against what per-write fencing already enforces
(every wallet mutation re-proves provider currentness, probes the exact SQL session, takes an
affine lease refused under revocation/supersession, and retains a signed per-mutation
attestation; the run store fences and CASes independently; the signer re-proves its binding per
call):

| Property | Verdict |
| --- | --- |
| P1 catalog currentness at boot | fail-fast only — a rotation one second after Finish leaves the worker equally stale until its first wallet write; differential guarantee at steady state is zero |
| P2 wallet-domain currentness | **redundant** — every reservation/activation/completion re-proves exactly this inside the fenced transaction, and the first real-world effect is reachable only after two fenced wallet writes |
| P3 signer/release-closure admission | **residue R1** — nothing re-proves it. Failure story: an operator restarts a worker from stale config retaining a signer release rotated away *because it was found defective*; all twelve local cross-checks pass (they check internal consistency only); a fresh store has no retained head for the strict-descendant rule to bind against; the first submission signs and broadcasts under the revoked release. Per-write wallet fencing never consults signer releases. The provider is the only memory outside both the database and the process — a DB row cannot hold this (absent witness: a restored database would re-admit a superseded closure). |
| P4 proven endpoint possession | **residue R2, qualified** — no per-call mechanism binds URL → route after boot. But the proof is boot-instant (DNS re-pointing after Finish defeats it identically), satisfiable only by MFM-aware proxies, and dominated continuously by TLS with certificate pinning. The load-bearing kernel: *the deployment authority, not the caller, decides which endpoint identity each route uses* — which does not require interactivity. |
| P5 cross-component sameness at one instant | the *instant* matters; the *lease/drain apparatus* exists only because assembly spans an external round-trip. Under one request/response, "one instant" is one server-side snapshot and revocation trivially linearizes before or after it. |
| P6 nonforgeability of the completed deployment | **keep the type-system half** (affine, non-serializable, compile-fail-tested — independent of the wire choreography); the commitment/reveal dance duplicates the challenge-bound Ed25519 assertion already on the reply. |

### 6.2 Target design (I): two boot rounds, one wire owner

1. `QualifyDomain` — unchanged, owned by `open_wallet_nonce_authority` (the wallet authority's
   own fail-fast and the source of the retained offline verifier anchoring every per-write
   lease).
2. `AdmitDeployment` — replaces catalog-qualify + Begin + endpoint exchange + Finish. Request:
   catalog descriptor + the existing `DeploymentAssemblyBinding` extended with **per-route
   endpoint commitments** (`sha256(normalized URL ‖ credential id)` — never the URL; secret-free
   by construction). Reply: registry head, fence head, one challenge-bound Ed25519 admission
   assertion over the whole request, evaluated against **one** provider state snapshot in one
   critical section (P1 fail-fast, P2 fail-fast, R1 admission, R2-kernel commitment match).
   The client verifies and constructs the still-opaque, still-affine
   `QualifiedEvmRoutingCatalog`; `PendingEvmRpcInventory::finish(admission)` remains the only
   door to a transport (P6 preserved with a single gate type).

DB-backed attestation rows were considered and **rejected**: the rows would live in the
database whose currentness is the question; a restored database would re-admit a rolled-back
closure. R1's value is precisely that the provider process is memory outside the database.

**Wire ownership:** new `provider_wire` module in `mfm-evm` (the layer `docs/design.md` already
names as owner of "canonical wallet-domain requests, responses, keys, proofs"): one
`ProviderRequest`/`ProviderReply` (server gains the ability to emit `Integrity` instead of
conflating it into `Rejected`), one five-variant `ProviderMutation`, the binding/context/proof
shapes, one protocol version, one bounds set. Sockets, channel IO, Ed25519 signing and
verification stay outside the domain layer. The three inconsistent bounds and both duplicated
declarations collapse to one each.

**Deltas:** `provider.rs` 2,203 → ~1,400; `inventory.rs` 633 → ~150 (the affine gate survives,
the challenge choreography goes); app bracket −90 (the twelve input cross-checks stay); test
server 3,796 → ~3,000; `mfm-evm` +~350. Net ≈ **−1,900 LOC**, four rounds → two, ~14 public
types deleted, every load-bearing residue retained.

### 6.3 Target design (II): ship

**Ground truth that frames the decision:** no external embedding consumer of `Application` can
exist against the published crates — `mfm-authority-seal` is `publish = false` and the sealed
session-admission broker has no out-of-workspace constructor, so an embedder exists only by
vendoring the workspace. Meanwhile the standalone REST binary exits before binding its
listener, and the only working shipped surface is `mfm keystore`. Deleting the binaries would
merely move the dead-authority problem one layer down: `Application`, the router, and the whole
EVM assembly would retain exactly zero possible non-test consumers.

**Recommendation: SHIP.** The one input that flips it to DELETE: an external embedder (a
private repo vendoring this workspace) actually exists — then that repo owns deployment
authority and these binaries duplicate its responsibility. This is a user decision; the
phase-3 (I) design is identical under both answers.

SHIP scope:
- Promote `tests/wallet-authority-provider` → `crates/deployment/wallet-authority-provider`
  (binary `mfm-wallet-authorityd`), under a new `deployment` layer row in the architecture
  taxonomy ("owns deployment secrets; never linked into application workers"). Fault injection
  (crash points, lease holds, the postgres proxy, loopback RPC) stays behind as a thin
  test-only wrapper. One hardening delta: the provider persists its captured SQL prefix digest
  in a provider-owned file instead of accepting it back through configuration (closes the
  "wallet prefix tampering" residual in `known-gaps.md`).
- New `crates/deployment/assembly` (`mfm-deployment`): implements the sealed credential broker
  over operator config; promotes the deployment-bootstrap logic that today exists only as test
  fixtures (release-history authoring, activation issuance, configuration seeding); assembles
  input → `AdmitDeployment` → `Application`. The genuinely new work: ~1,200–1,800 lines
  promoted or authored from what the integration test already proves.
- `connect_application` and `make_default_app_state` become real. Every `mfm run`/`mfm ops`
  command and every REST route becomes runnable against a single-operator deployment;
  `output_file.rs` regains its only caller.

The DELETE flip is recorded with exact scope (remove `bin/rest-api`; remove `commands/run`,
`commands/ops`, `support/{application,access,output_file}` from the CLI, keep keystore; ~3,900
src + ~900 test lines) so taking it later is a decision, not a re-analysis.

**The out-of-process proof survives** under both designs: the provider still runs as a
separately spawned binary holding the only signing key and probe credential; every persisted
mutation still carries a provider-signed attestation the worker cannot mint. The crash matrix
and ambiguity proxy in the integration suite are untouched.

### 6.4 Cutover

Contract edits: `design.md`'s "one affine provider bracket" paragraph is replaced by the
one-admission paragraph (drafted verbatim in the architect pass, including "Revocation closes
admission atomically; an admission linearizes entirely before or after that cutover");
`architecture.md` mirrors it, drops assembly leases from the drain sentence, and gains the
`mfm-evm` wire-contract bullet and the `deployment` layer row; the endpoint-authenticity
limitation is stated next to the absent-witness section ("transport-layer authenticity after
admission is a deployment responsibility"). For SHIP: the two "standalone bootstraps fail
closed" sentences in `design.md`/`architecture.md` are replaced by the deployment-crate
construction sentence, and `known-gaps.md`'s "EVM provider trust" / "retained-release trust"
rows narrow accordingly.

Tests: provider-protocol hostile-wire cases retarget the admit reply; the assembly-lease
TTL/race cases are replaced by per-field admission-rejection cases (stale fence, revoked at
admit, each policy field, each endpoint commitment); the transport-surface contract flips from
asserting fail-closed to asserting the deployment path; the crash/absorption matrix is
untouched.

**Commit sequence:**
1. `own the wallet provider wire contract in mfm-evm`
2. `replace the deployment assembly bracket with one admit round in design and architecture docs`
3. `collapse deployment assembly to one admit round`
4. `delete the rpc inventory challenge choreography`
5. `add a deployment layer and promote the wallet authority provider`
6. `add single-operator deployment assembly`
7. `wire the cli and rest binaries to deployment assembly`

(Under the DELETE flip, commits 5–7 become the three deletion commits with the scope above.)

### 6.5 Phase-3 material uncertainties

1. **Does an external embedding consumer exist?** The explicit user-decision input; nothing
   in-repo can answer it. Wrong-SHIP builds a second owner for deployment authority;
   wrong-DELETE strands the embedder's expected surface. The (I) redesign is unaffected either
   way.
2. **TLS posture of admitted RPC routes.** The endpoint-commitment replacement for the
   challenge choreography assumes TLS with certificate verification dominates a boot-instant
   HMAC possession proof. If plaintext deployment-owned proxies are intended, keep the per-route
   HMAC exchange as an *optional provider policy* riding inside the single admission (~300
   lines, not 1,300). One deployment question resolves it.
3. **`QualifyDomain` as a second round.** Kept because the wallet authority is constructed
   before the assembly input exists; folding to strictly one round inverts assembly order and
   widens the blast radius. Both shapes keep all residues; settle during commit 3.
4. **Server-emittable `integrity`.** Behavior-preserving at boot call sites, but per-write
   dispositions distinguish `Integrity` from `Rejected`; audit the five server `Rejected` sites
   against the client's per-operation arms while writing commit 1.
5. **LOC deltas are static-reading estimates** (±30%); direction and ordering do not change.
6. **Layer placement of the promoted provider** (new `deployment` row vs overloading
   `secret-provider`): a one-table-row question for the architecture doc owner, settled in
   commit 5.

---

## 7. Phase 4 — mechanical consolidation

Carried from the superseded LOC analysis (its corrected §3 estimates, re-scoped against phases
1–3):

| Workstream | Est. lines | Risk | Notes after phases 1–3 |
| --- | ---: | --- | --- |
| test architecture: one fixture crate, `state!` macro over 76 `impl State`, shared harness | −3,500…−6,500 | medium | unchanged; do after phase 1 so the harness pins the cache |
| flatten structured programs into definition tables (`mfm-spec` §2.7 of the old doc) | −1,400…−2,600 | high | **resets all run history**; schedule with phase 2's certify restructuring; policy-document `SchemaId` byte-identity must be proven first |
| sealed `AccessKind` across capabilities/program/certify/live-evm | −550…−850 | medium | unchanged; the runtime is already the model shape |
| one keyed-mutation protocol in `authority.rs`; one provider wire contract in `mfm-evm` | −250…−550 | medium | wire-contract half is absorbed by phase 3 |

The four spike gates from the superseded doc remain the entry condition for each workstream
(dev-dependency cycle, `ReviewsCompletion` typecheck, borrow across await in the write
transaction, derived policy `SchemaId` byte-identity).

---

## 8. Verification plan

- **Phase 1 invariant harness:** promote `verify_incremental_equivalence` from test-support to
  the permanent proof obligation of the cache: every cached/folded projection must equal full
  re-verification, extended to cover concurrent-writer head advancement. The sixteen causality
  tests in `store/tests/structured_runtime_causal.rs` must pass unchanged because they assert
  durable-history semantics, which the cache must not alter.
- **Phase 2:** tests that assert deleted shadows are deleted with them; tests that assert
  boundary verification (hostile persisted batches, malformed rows, tampered exports) must pass
  unchanged. The 81 compile-fail tests stay.
- **Phase 3:** add a focused out-of-process-authority proof for whatever form the target design
  keeps (see §6); no composed-system gate currently owns that evidence.
- Scope-driven gates per `docs/build-and-verification.md`; no broad gates merely because a
  commit exists; `.#ci` composes `.#check`/`.#test`/`.#test-db`.

---

## 9. Supersession map

Positions of the superseded documents that this RFC **overturns**:

| Old position | Where | Revision |
| --- | --- | --- |
| "~55% irreducible given the contract" | flow analysis §10 | Withdrawn. It conflated the product contract with the current implementation strategy. The contract (`run-execution.md:173`) demands one action per drive; snapshot-plus-fold satisfies every stated invariant. What is irreducible is the journal, the reducer, boundary verification, and the nonce authority — not four re-verifications per action. |
| Per-crate "Earned" verdicts as the unit of judgment | flow analysis §9 | Subordinated. Judging each layer against its own stated purpose can only ever acquit it; stacked compensation is visible only by asking, per runtime check, *which adjacent mechanism already proves this*. The per-file verdicts stand as descriptions, not as justification. |
| "Load #3 … should not be touched without an architect decision" | flow analysis §7.4 | Resolved — this RFC (with its phase-1 architect pass) is that decision: fold the returned batch forward; delete the reload. |
| "−8,400 lines is where reduction actually is" | LOC analysis §3 | Re-scoped to phase 4 only. It answered "which lines can go while keeping this architecture"; phases 1–3 answer "which architecture keeps these properties." |
| "LOC overstates semantic weight ~1.4x (rustfmt at 100 cols)" | LOC analysis §1.1 | Reinterpreted: a type like `Result<Vec<ExpandedStructuralPathDefinition>, StructuredStoreError>` failing to fit a line is evidence about the design, not about the measurement. |
| "A test pins property X" read as "X earns its code" | both, passim | Rejected as inference. Tests prove code does what it does, not that the property required the code; the crash-restart suite passes identically over a folded projection — the equivalence test guarantees it. |
| "It is a real property … and I found no cheaper construction that keeps it" (the provider bracket) | flow analysis §3.2 | Overturned. The phase-3 pass found the cheaper construction: of the six attested properties, four are fail-fast or choreography; the two load-bearing residues (signer/release-closure admission, authority-decided endpoint identity) survive in one admission round at ≈ −1,900 LOC (§6.1–6.2). |
| "reachable execution: health only" for the REST binary | flow analysis §1 | Corrected. The standalone binary fails closed before binding its listener; even `/v1/health` is unreachable from the shipped artifact. |

Facts of the superseded documents that this RFC **carries forward** as its evidence base:
the execution-flow map and phase inventory, the four-load table, the commit pipeline steps, the
fail-closed binaries, the crate inventory and keep-list (§1.6), all dup% measurements, the
per-file reduction estimates with their published corrections, and the branch accounting
(refactor ≈ −65k net; new EVM/Postgres authority capability ≈ +45k net; whole-tree 193,176 →
165,450 lines, 585 → 325 files).

---

## Material uncertainties

Full per-phase lists live in §4.5, §5.7, and §6.5. The items below are the **decision-level**
ones — each requires a reviewer or owner ruling before its phase begins; none is routine.

1. **The "complete current projection" contract reading** (§4.5.1). Phase 1 reads
   `design.md:226-229` as "the comparison covers the complete projection value"; if "recompute
   completely" was intended, phase 1 is a deliberate contract weakening and must be approved as
   one. Approving the §4.4 replacement sentence is the ruling.
2. **Cache regression policy: accept-and-replay vs fail-closed** (§4.5.2). The draft chooses
   accept-and-replay, keeping `known-gaps.md` true verbatim; fail-closed on observed head
   regression is one error path away if review prefers surfacing mid-process restores.
3. **Verify-on-read as the surviving side** (§5.2). The criterion is stated (cold loads must
   stay verified regardless, so only the write side is deletable); the residual risk is silent
   writer drift between the reducer's intent/recorded arms (§5.7.1), mitigated by the promoted
   equivalence law and ultimately closed by unifying the arms. Ops must also sign off on losing
   the read-back canary (§5.7.2).
4. **Does an external embedding consumer of `Application` exist?** (§6.5.1). The single input
   that flips SHIP → DELETE for the binaries. Nothing in-repo can answer it; the phase-3
   authority redesign is identical under both answers. **This question should be answered
   before phase 3 is scheduled.**
5. **TLS posture of admitted RPC routes** (§6.5.2). If any admitted route is deliberately
   non-TLS, the per-route HMAC exchange returns as an optional provider policy inside the
   single admission round instead of being deleted.
6. **Phase-2/phase-1 successor form** (§5.5). Resolved in this draft — the cache adopts the
   append-time successor; externally-appended batches fold via `replay_step`; the equivalence
   law is the standing guarantee — but it is a normative coupling between the two phases and
   must land in that order.

Per `CLAUDE.md`, items 1–5 are architecture/design-contract rulings; each phase's first commit
lands only after its ruling.

---

## Appendix A — verified evidence index

Every fact this RFC's argument rests on, with where it was verified:

| Fact | Site |
| --- | --- |
| drive contract: "at most one action" | `docs/run-execution.md:173` |
| pre-drive load, evidence discarded after tenant compare | `crates/app/src/production_structured.rs:435`, `:777` |
| post-drive load only for journal head | `production_structured.rs:443` |
| action-selection load | `crates/kernel/runtime/src/structured.rs:488` |
| post-authorize reload | `runtime/structured.rs:1036` |
| field-by-field recheck of own committed record | `runtime/structured.rs:1075–1102` |
| commit returns the committed record before the reload | `runtime/structured.rs:1006–1027` |
| genesis replay per load | `crates/kernel/store/src/structured/semantic_open.rs:211–234` |
| replay_step = reduce + compile + compare + discharge | `semantic_open.rs:236–270` |
| incremental ≡ full-replay proof, test-only | `semantic_open.rs:273` (`cfg(test, feature = "test-support")`) |
| double reduction + self-requalification + compare per append | `crates/kernel/store/src/structured/coordinator.rs:78`, `:97`, `:121`, `:127` |
| backend answer compared against candidate | `coordinator.rs:144` (`classify_backend`) |
| commits take `previous: Option<VerifiedStructuredRun>` (no reload in commit) | `coordinator.rs:63–65` |
| grant debug_asserts under grant typestate | `production_structured.rs:434` and siblings |
| CLI fails closed unconditionally | `bin/cli/src/support/application.rs:28–37` |
| REST fails closed unconditionally, before binding the listener (health unreachable) | `bin/rest-api/src/lib.rs:100`, `bin/rest-api/src/main.rs` |
| no out-of-workspace embedder can exist (sealed broker, unpublished seal crate) | `crates/kernel/authority-seal/Cargo.toml` (`publish = false`), `session.rs:350` (`test-support`-only constructor) |
| provider proof bound in three inconsistent copies (4096 / 16384 / 16384) | `provider.rs:42`, `inventory.rs:22`, test provider `lib.rs:2182` |
| no end-to-end executable path | CLI and REST both fail closed before acquiring writer authority |
| embedding is the documented intent for REST | `bin/rest-api/src/lib.rs:95–99` ("Deployments embed this library and inject a fully composed `AppState`") |
| provider wire divergence: 3 vs 5 `ProviderMutation` variants | `crates/storages/evm-postgres/src/provider.rs:1020` vs `tests/wallet-authority-provider/src/lib.rs:70` |
| per-mutation fenced revalidation in the wallet authority | `crates/storages/evm-postgres/src/authority.rs` (`reserve_after_qualification_inner` / `activate_candidate_inner` / `complete_inner`) |
| indexed-head-first snapshot protocol | `crates/storages/postgres/src/structured.rs` |

## Appendix B — measurement summary (carried from the superseded LOC analysis)

- Branch: added 147,491 / deleted 176,436 / net −28,945 against `dev`; whole tree 193,176 →
  165,450 `.rs` lines (−14.4%), 585 → 325 files (−44.4%).
- The runtime refactor proper ≈ −65k net (`crates/kernel/runtime` alone −32,711; a 1,318-line
  `runtime/src/structured.rs` replaced `runners.rs`/`framework.rs`/`commit*.rs`/… );
  the new EVM/Postgres deployment-authority capability ≈ +45k with no predecessor
  (`crates/storages/evm-postgres` +16,247/0 deleted; `tests/wallet-authority-provider` +4,516;
  `crates/domains/evm` +10,480).
- Duplication (exact 8-line windows, lower bounds): highest `authority.rs` 25.8%,
  `structured_runtime_causal.rs` 22.3%, `process_handle_tests.rs` 19.5%,
  `structured_certification.rs` 18.9%; cleanest large files `journal/structured.rs` 1.1%,
  `submission_process.rs` 1.0%, `values/lib.rs` 0.0%, `reducer.rs` 1.5%.
- Cross-file: 222 exactly duplicated lines between the two provider protocol files; ~108
  fixture structs and 76 `impl State` blocks across four uncoordinated test corpora.
- 39 files exceed +1,000 net lines; their per-file explanations, published corrections, and
  "leave it" verdicts (`wallet_authority.rs`, `reducer.rs`, `journal`, `values`,
  `submission_process.rs`, `transport/exact.rs`, `spec` types, postgres backend) are accepted
  by this RFC as written and not repeated here.
