# MFM — REDESIGN.md Implementation Plan (v3.0)

> This document turns `REDESIGN.md` into an executable, PR-sized checklist.
> Contract rule: `REDESIGN.md` wins on semantics, but **any known plan ↔ contract deviation must be recorded**
> in §Deviations (no silent drift).
>
> Version: v3.0 (applies recommendations from `REDESIGN_CRITIC_v2.md`)
> Last updated: 2026-02-09
>
> Note: The intended contract edits are captured in `REDESIGN_v3.md`. PR 00 applies those edits to `REDESIGN.md`.

---

## Goals (Success Criteria)

Implement the `REDESIGN.md` core invariants with *explicit, testable semantics*:

* Append-only per-run event streams
* Transactional appends at the event-store layer (each append is atomic; readers never observe partial appends)
* Content-addressed artifacts (manifest, context snapshots, fact payloads, outputs)
* Canonical JSON hashing for structured hashed data
* Explicit IO abstraction (live vs replay); missing facts are structured errors
* Kernel events sufficient for recovery/resume correctness
* **Secrets are excluded from everything persisted** (events, manifests, artifacts, context snapshots)
* Thin CLI surface to start/resume runs and inspect events/artifacts with stable `--output-format`

### Acceptance Tests (Named)

Each invariant must be verifiable by at least one named acceptance test. PRs MUST reference these IDs in their
acceptance criteria.

* AT-01 AppendOnlyEventStream: event streams are append-only; no mutation/deletion; replay ignores nothing except
  explicitly-defined orphan handling.
* AT-02 AtomicAppendVisibility: a multi-event append is atomic; readers never observe a partially-written append.
* AT-03 ExpectedSeqConcurrency: concurrent/competing appenders to the same run fail with `StorageError::Concurrency`.
* AT-04 ArtifactContentAddressing: `put(bytes) -> id`; `get(id) -> bytes`; tamper/corruption is detected on read.
* AT-05 CanonicalJsonHashing: canonical bytes+hash are stable across equivalent structures; edge cases are covered.
* AT-06 LiveThenReplayDeterminism: live run records all determinism-relevant facts; replay produces identical outputs
  for replayable paths.
* AT-07 CrashResumeDeterminism: killing mid-run and resuming yields the same replayable outputs as the uninterrupted run.
* AT-08 SideEffectIdempotency: side effects are not applied twice across retries/crash-resume.
* AT-09 SecretsNeverPersisted: secrets are excluded from persisted surfaces (events, manifests, artifacts, snapshots,
  and CLI/API outputs including error details).

### Delivery Map (Which PR Makes Which Test Possible)

| Acceptance Test | Primary PR(s) |
|---|---|
| AT-01 AppendOnlyEventStream | PR10 (+ PR07/PR08 stores) |
| AT-02 AtomicAppendVisibility | PR07, PR08 |
| AT-03 ExpectedSeqConcurrency | PR07, PR08 |
| AT-04 ArtifactContentAddressing | PR07, PR09 |
| AT-05 CanonicalJsonHashing | PR05, PR11 |
| AT-06 LiveThenReplayDeterminism | PR12, PR13, PR15 |
| AT-07 CrashResumeDeterminism | PR10, PR15 |
| AT-08 SideEffectIdempotency | PR15 |
| AT-09 SecretsNeverPersisted | PR11, PR12, PR16 |

---

## Non-Goals (Milestone 1: Semantics Proof)

* REST API surface (PR 18 is optional/deferred; not on the critical path)
* ClickHouse projections/indexers
* Performance baselines beyond rough sanity checks (see §Testing Strategy)
* Garbage collection / compaction for artifacts/snapshots/events
* Manual operator intervention (DLQ, force-complete, compensations, circuit breakers)
* A full suite of domain ops/collectors (start with one “proof” op crate)
* Postgres/MinIO are **integration-lane only** until fast-lane semantics are proven stable
* Nested machines / child-run spawning + linkage (explicitly deferred; Milestone 1 covers flattened composition only)
* Deterministic rewinds across completed transitions (kernel rewind marker + projection semantics; deferred)

## Non-Goals (Later / Optional)

* Production hardening of REST API (auth, multi-tenant, quotas)

---

## Current Repo Reality (Baseline)

* Workspace members today: `mfm_cli`, `mfm_machine`, `mfm_machine_derive`, `mfm_core`.
* Current machine uses `SafeContext` snapshots; keystore is mature and used by CLI.
* `mfm_core` currently depends on `mfm_machine` only to re-export `SafeContext` (must be removed).

---

# Core Semantic Clarifications (v3)

These clarifications remove ambiguity that leads to crash-unsafe or non-replayable behavior.

## S1 — Event durability boundaries (replace “single append per attempt”)

A state attempt is defined by a **kernel envelope** in the event stream:

* `KernelEvent::StateEntered { state_id, attempt, base_snapshot_id }` (attempt start)
* zero or more domain events (facts, artifact refs, boundaries, etc.)
* exactly one terminal kernel event: `StateCompleted` (success) or `StateFailed` (failure)

### Rules

1. The engine MUST append `StateEntered` at the start of **every** attempt (including PURE states),
   before calling the handler.
2. Event-store appends are transactional: each `EventStore::append([...])` is atomic and never partially visible
   to readers (AT-02).
3. A single state attempt MAY span multiple appends. This is required to avoid “IO happened but wasn’t recorded”
   windows for fact recording (AT-06/AT-07).
4. The terminal kernel event is the commit marker for context snapshot advancement:

   * only `StateCompleted { next_snapshot_id }` advances the run’s `base_snapshot_id`
   * `StateFailed` discards staged context and leaves the run at `base_snapshot_id`

### Crash/Resume Semantics (Orphan Handling)

* If the event stream ends with `StateEntered` and no terminal event, the attempt is **in-flight**.
  On resume, the engine retries from `base_snapshot_id` (discarding staged context).
* Domain events emitted during an in-flight attempt remain in the append-only stream. On resume:

  * **Facts are run-scoped**: `FactRecorded { key, payload_id }` remains valid for dedupe and replay even if the
    attempt never reaches a terminal event.
  * Other domain events emitted during an in-flight attempt MUST NOT advance plan progression or snapshot selection.
    They are treated as orphaned for recovery semantics, but remain inspectable/auditable in the raw stream.

**Note:** `REDESIGN.md` currently states stricter “atomic state transition emission” semantics (notably §6.2, §6.4,
and Appendix C `EventRecorder`). This plan adopts S1 (multi-append attempt envelopes + orphan handling) for Milestone 1
to eliminate “IO happened but wasn’t recorded” windows. PR 00 updates `REDESIGN.md` accordingly (no silent drift).

**Decision (Milestone 1):** Deterministic rewinds across completed transitions are explicitly deferred. If adopted later,
they MUST add a kernel rewind marker and deterministic projection semantics (see `REDESIGN_CRITIC_v2.md`).

## S2 — Facts are single-assignment and deduped by `FactKey`

Within a run:

* A `FactKey` is **single-assignment**: the first durable `FactRecorded { key, payload_id }`
  binds the key permanently to that payload.
* `LiveIo.call()` with a `fact_key` MUST:

  1. check whether the key already exists in the run’s fact index
  2. if exists → return the recorded payload (no transport call)
  3. else → perform IO, store payload artifact, append `FactRecorded`, return result

This makes retries and crash recovery deterministic for read-only IO.

### Clarifications

* Scope: facts are per-run. Nested runs have independent fact indexes (no implicit cross-run sharing).
* Invalidation/versioning is out-of-scope for Milestone 1: to “fix” a fact, start a new run or bump the key
  namespace (e.g., `collector_v2/...`).
* Concurrency: Milestone 1 executes states sequentially. If/when opt-in parallelism is implemented, FactKey
  acquisition MUST be made atomic (event-store enforced) to avoid races.

## S3 — Replay policy for missing `fact_key`

Default rule (explicit, not “TBD”):

* In **Replay** mode, any `IoCall` that affects deterministic behavior MUST supply `fact_key`.
* If `fact_key` is absent in Replay mode → return a structured `IoError` with a stable code
  (e.g. `missing_fact_key`) and `retryable = false` by default.

(If you later add an explicit “non-deterministic allowed” policy, it must be opt-in and auditable.)

### Clarifications

* Missing-fact retryability MUST be configurable (opt-in). Default remains `retryable = false`:

  * run-level: `RunConfig.replay_missing_fact_retryable = true`, OR
  * per-call: `IoCall.missing_fact_retryable = true`
* Distinguish:

  * `fact_key` absent (Replay mode) → `IoError` code `missing_fact_key`
  * `fact_key` present but not recorded → `IoError::MissingFact { key, info }`
  * `fact_key` recorded but payload is missing/corrupt → `StorageError::{NotFound,Corruption}` surfaced as
    storage failures (not “missing fact key”)

## S4 — Secrets: excluded from all persisted surfaces

**Hard rule:** secrets must never appear in:

* manifests
* events
* artifacts (including fact payloads and context snapshots)
* context snapshots / dumps
* error strings/details

Secrets exist only in-memory, typically behind:

* keystore (software)
* MetaMask / Trezor / hardware wallets
* external signing providers

### Enforcement (Primary Strategy)

* Architectural rule: secret material MUST NOT enter any serializable/persisted type. Secrets flow only through
  in-memory keystore/signing/transport layers; persisted data contains only non-secret references (key IDs,
  wallet type, intent hash, provenance).
* Type-level guardrails: represent secrets using non-serializable, non-`Debug`-leaking wrappers (e.g.,
  `secrecy::SecretString`, `zeroize::Zeroizing<Vec<u8>>`) and avoid `Serialize` impls for secret-carrying types.
* Error hygiene: transport/providers MUST scrub URLs/headers and never include raw credential-bearing material in
  `ErrorInfo.message` / `ErrorInfo.details`.

**Reproducibility note:**

* Side-effect execution that requires credentials is only retryable/reproducible in environments
  where the same credentials are available at runtime.
* Persisted data may include **non-secret configuration/provenance** describing *how* a signature
  was produced (wallet type, key reference ID, chain ID, RPC URL reference, intent hash),
  but never the secret material itself.

## S5 — Side effects require idempotency strategy

Any `APPLY_SIDE_EFFECT` state must declare an idempotency key (e.g. “intent hash”) and:

* write a durable domain event capturing the idempotency key (not secrets)
* apply the side effect using an external idempotency mechanism keyed by the same idempotency key
* ensure retries/resume do not re-apply the effect if it already happened (AT-08)

### Clarifications

* Storage: the idempotency key MUST be persisted in a domain event (system of record). Context may cache it,
  but context alone is insufficient for dedupe.
* Validation: `Idempotency::None` may exist as a type-level variant, but `APPLY_SIDE_EFFECT` states MUST be rejected
  at plan-build time if no key is provided.
* Non-blockchain example: HTTP POST MUST include an `Idempotency-Key` header derived from a canonical hash of the
  request intent.

---

# Decision Defaults (Milestone 1)

These defaults are “locked” for the first milestone unless this file is updated first.

* Execution model: sequential per run (parallel execution is deferred for Milestone 1).
* Sequencing: `EventEnvelope.seq` is **1-indexed** per run. An empty run has `head_seq = 0`. `expected_seq` is the
  current head sequence number.
* Backends/lane split:

  * Fast lane (correctness/proof): SQLite event store (file-backed) + filesystem artifact store
  * Integration lane (target backends): Postgres event store + MinIO/S3 artifact store
* Canonical JSON: RFC 8785 (JCS) semantics via a pinned implementation (e.g., `serde_jcs` + version pinned in
  `Cargo.lock`). Hashed structures reject NaN/Inf and MUST NOT contain floats (use integer-scaled or decimal strings).
* Hashing: SHA-256. `ArtifactId` is a **64-char lowercase hex** digest; parsing/validation MUST reject wrong length,
  uppercase, and non-hex (AT-04).
* Plan IDs (§10.7) must be valid and enforceable:

  * IDs are dot-separated segments: `<machine_id>.<step_id>.<state_local_id>`
  * Each segment MUST match: `^[a-z][a-z0-9_]{0,62}$` (lowercase ASCII, digits, underscore; max 63 chars)
  * Dots are reserved separators and therefore forbidden inside segments
* Crash simulation mechanism (for AT-07/AT-08): use explicit test failpoints (`fail` crate or equivalent), decided
  before PR 10 so the engine can be instrumented for deterministic crash tests.

---

# Deviations (Plan vs Contract)

These are the only allowed “escape hatches.” If a mismatch is discovered, either update this section (with rationale)
or update the contract and remove the deviation.

* D-01 State transition atomicity language: `REDESIGN.md` still implies “atomic state transition emission” (notably
  §1.1, §6.2, §6.4, and Appendix C `EventRecorder`), but S1 allows multi-append attempt envelopes (needed for
  crash-safe fact recording). Resolve by updating `REDESIGN.md` to match S1 before implementation begins.
* D-02 FactKey single-assignment: S2 defines per-run `FactKey` as single-assignment (first durable write wins), but
  `REDESIGN.md` does not define overwrite/versioning rules. Resolve by updating `REDESIGN.md` (§7.3/§8 + Appendix C).
* D-03 Replay requires `fact_key`: S3 requires `fact_key` for replayable IO in Replay mode (missing is a structured,
  non-retryable error by default), but `REDESIGN.md` §8.1 says such IO “SHOULD” use `fact_key`. Resolve by updating
  `REDESIGN.md` §8 + Appendix C.
* D-04 ID regex vs examples: the plan enforces a strict segment regex and forbids dots inside segments; `REDESIGN.md`
  Appendix C includes examples like `pipeline[0].aave_tracker` that violate this. Resolve by updating contract
  examples/constraints (Appendix C + §10.7).
* D-05 Nested machines milestone scope: `REDESIGN.md` treats nested machines as supported (and includes
  `ChildRunSpawned`), but Milestone 1 plan defers implementing engine-level child-run spawning/linkage. Resolve by
  updating `REDESIGN.md` to mark nested-machine execution as deferred for Milestone 1.
* D-06 Secrets-in-artifacts exception: `REDESIGN.md` suggests encrypted artifacts “if unavoidable” (§12.4/§14.2), but
  this plan’s Milestone 1 rule is “no secrets persisted” (including in fact payloads and context snapshots). Resolve
  by updating `REDESIGN.md` to make the Milestone 1 policy explicit and defer encrypted-secret artifacts.

---

# PR Dependency Graph (Critical Path)

This is the dependency DAG. PR numbers may be executed in parallel unless an edge exists.

```text
PR00 -> PR01 -> PR02 -> PR03

PR04a -> PR04b -> PR04c -> PR05
PR04c -> PR07 -> PR10 -> PR10b -> PR11 -> PR12 -> PR13 -> PR14 -> PR15 -> PR16 -> PR17 -> PR20

PR08, PR09 depend on PR04c + PR05 (types + hashing) and SHOULD start only after PR10/PR11 prove fast-lane semantics,
to avoid schema churn while the engine/recording model is still moving.
PR19 depends on PR08 + PR09 and can run in parallel with PR14–PR17.

PR18 (REST) is deferred until after PR20.
```

---

# CI Parity Commands (Required Before Each PR Is “Done”)

```bash
cargo +nightly fmt --all
cargo +nightly clippy --workspace --lib --examples --tests --benches --all-features
cargo nextest run --workspace
cargo audit

# optional parity lane
nix flake check
nix build
```

---

# Testing Strategy (Fast Lane vs Integration Lane)

Milestone 1 correctness is proven in the **fast lane** (durable + hermetic). Integration lane validates the target
backends but MUST NOT be the only place where correctness is exercised.

## Test Matrix

| Test Category | Fast Lane (SQLite + FS) | Integration Lane (Postgres + MinIO) |
|---|---|---|
| Unit tests (types, parsing, validation) | Yes | N/A |
| AT-01 Append-only + orphan handling | Yes | Yes |
| AT-02 Atomic append visibility | Yes | Yes |
| AT-03 Expected-seq concurrency | Yes (concurrent append test) | Yes |
| AT-04 Artifact integrity (hash-on-read) | Yes | Yes |
| AT-05 Canonical JSON hashing | Yes | N/A |
| AT-06 Live → Replay determinism | Yes | Optional |
| AT-07 Crash/resume determinism | Yes | Optional |
| AT-08 Side-effect idempotency | Yes | Optional |
| AT-09 Secrets never persisted | Yes | Optional |
| CLI e2e (json/text outputs) | Yes | Optional |

## Crash Simulation (Defined Up Front)

Use explicit failpoints (e.g., the `fail` crate) rather than “random panics”:

* Failpoints are compiled only for tests (or behind a `failpoints` feature).
* Engine and stores expose stable failpoint names so crash tests can target precise windows
  (e.g., “after storing artifact, before appending FactRecorded”).

## Performance (Not a Milestone 1 Goal)

No hard performance targets yet. Add a small baseline benchmark only if it helps prevent obvious regressions
(event append throughput, artifact put/get on local FS).

## Property / Fuzz Testing (Optional, High Value)

If time allows:

* property tests: canonical JSON stability and ArtifactId correctness (`put/get/hash`)
* fuzzing: any parser/decoder that consumes persisted JSON

---

# Phase -1 — Contract Alignment (Blocking)

## PR 00 — Resolve Contract Drift (D-01..D-06)

**Intent:** Make the plan auditable by removing known plan↔contract mismatches before code lands (no silent drift).

### Steps

1. Apply the edits in `REDESIGN_v3.md` to `REDESIGN.md` (or reproduce them by hand), covering at minimum:

   * per-append atomicity (no partial visibility)
   * attempt envelope (`StateEntered … domain … terminal`)
   * orphan handling on crash/resume
   * facts are run-scoped and remain valid across in-flight attempts
2. Ensure **all** conflicting “atomic transition emission” language is updated, including:

   * `REDESIGN.md` §1.1 and §6.2
   * `REDESIGN.md` Appendix C `recorder::EventRecorder` contract comment
3. Decide and encode the remaining semantic tighteners in `REDESIGN.md` (not just in this plan):

   * S2 FactKey single-assignment (first durable write wins)
   * S3 Replay requires `fact_key` for replayable IO
   * ID constraints + examples (`OpPath`, plan IDs) aligned with the enforced regex
   * Milestone 1 stance on nested machines (explicitly deferred, or schedule implementation)
   * Milestone 1 secrets-in-artifacts policy (no secrets persisted; encrypted-secret artifacts deferred)

### Acceptance

* D-01..D-06 are removed (or updated with new references) because plan and contract agree.

---

# Phase 0 — Workspace and Naming

## PR 01 — Create `crates/` + `bin/` (Move Only)

**Intent:** Restructure without changing behavior.

### Steps

1. Add directories: `crates/`, `bin/`.
2. Move existing crates:

   * `mfm_core` → `crates/core`
   * `mfm_machine` → `crates/machine-legacy` (temporary)
   * `mfm_machine_derive` → `crates/machine-derive-legacy` (temporary)
   * `mfm_cli` → `bin/cli-legacy` (temporary)
3. Update root `Cargo.toml` workspace `members` paths.
4. Update all `path = "../..."` dependencies (workspace + crates + bins).
5. Update `flake.nix` to build the legacy CLI at its new path and keep `nix run .#mfm_cli` working.
6. Update any references in CI/docs/scripts that assume old paths (e.g., `.github/workflows/nix.yml`).

### Acceptance

* All CI parity commands pass.
* `nix build .#mfm_cli` succeeds and `nix run .#mfm_cli -- --help` still works (legacy CLI).

## PR 02 — Cargo Package Naming Policy (Hyphens)

**Intent:** Align packages with `REDESIGN.md` §2.6 while legacy code still exists.

### Steps

* Rename packages (`package.name`), while keeping the legacy CLI binary name stable (`[[bin]] name = "mfm_cli"`):

  * `crates/machine-legacy` → `mfm-machine-legacy`
  * `crates/machine-derive-legacy` → `mfm-machine-derive-legacy`
  * `crates/core` → `mfm-core`
  * `bin/cli-legacy` → `mfm-cli-legacy` (optional), but keep binary name stable until replacement.
* If the diff is too noisy, split this into two PRs: rename `mfm-core` first, then legacy machine + legacy CLI.
* Ensure `flake.nix` / `.github/workflows/nix.yml` continue to work with `nix run .#mfm_cli` until PR 16.

### Acceptance

* All CI parity commands pass.
* `nix run .#mfm_cli -- --help` still works.

## PR 03 — Remove `core -> machine` Dependency

**Intent:** Enforce boundary rule: `core` must not depend on `machine` (REDESIGN §5.2).

### Steps

1. Remove legacy machine dependency from `crates/core/Cargo.toml`.
2. Remove `SafeContext` re-export from `crates/core/src/lib.rs`.
3. Update call sites that imported `SafeContext` via `mfm-core` to import it from legacy machine directly
   (temporary) or new machine later.
4. Add a boundary guardrail so `mfm-core` cannot silently regain a dependency on machine crates
   (CI script or equivalent).

### Acceptance

* All CI parity commands pass.
* Boundary guardrail passes (core has no dependency on machine crates).

---

# Phase 1 — New `mfm-machine` Public Contract (Appendix C)

## PR 04a — Add `crates/machine` (Foundational Types)

**Intent:** Land the redesign’s public contract in reviewable chunks.

### Scope

* `ids`, `canonical` (marker), `config`, `meta`, `errors`

### Steps

1. Create `crates/machine` (package `mfm-machine`).
2. Implement the scope modules.
3. Add validation + negative tests:

   * `ArtifactId` format (64-char lowercase hex)
   * Plan ID segment regex from §Decision Defaults (`^[a-z][a-z0-9_]{0,62}$`)
4. Add minimal serde roundtrip tests for key types.

### Acceptance

* All CI parity commands pass.
* ID format validation tests pass.

## PR 04b — `mfm-machine` Runtime Abstractions (Context, Events, IO, Recorder)

**Intent:** Define runtime interfaces without committing to the engine implementation.

### Scope

* `context`, `events`, `io`, `recorder`

### Steps

1. Implement the scope modules + docs.
2. Decide enforcement point for `DomainEvent.payload` invariants:

   * `EventRecorder` MUST validate payload is canonical-hashable under our policy (no floats, no forbidden values).
3. Tests:

   * `DynState` clone semantics (`Arc<dyn State>` is cloneable)
   * negative tests for invalid payload values per canonical policy

### Acceptance

* All CI parity commands pass.

## PR 04c — Planning + Stores + Engine Traits (Public Contract Complete)

**Intent:** Finish Appendix C type/trait surface so implementations can start.

### Scope

* `state`, `plan`, `stores`, `engine`

### Steps

1. Implement the scope modules.
2. Add plan validation errors for invalid IDs and obvious structural issues.
3. Coexistence rule: the new `mfm-machine` coexists with legacy machine without any bridging in Milestone 1.

### Acceptance

* All CI parity commands pass.
* No other crate is forced to use this new API yet.

## PR 05 — Canonical JSON + SHA-256 Hash Helpers (Public)

**Intent:** Provide a single correct hashing pipeline for manifests/snapshots/fact payloads/outputs.

### Steps

1. Add public helpers (either in `mfm-machine` or a tiny shared crate if needed by storages/sdk/ops):

   * `canonical_json_bytes(value) -> Vec<u8>`
   * `artifact_id_for_bytes(bytes) -> ArtifactId` (SHA-256 lowercase hex)
2. Tests:

   * object key order does not change canonical bytes/hash
   * stable hash for equivalent structures
   * Unicode escaping normalization (RFC 8785)
   * import RFC 8785/JCS test vectors (or equivalent) to lock in edge-case behavior
   * reject NaN/Inf and reject floats in hashed structures (policy)
   * string escaping rules (RFC 8785)

### Acceptance

* All CI parity commands pass.
* Canonical JSON + hashing helpers satisfy AT-05.

## PR 06 — (Optional / Deferred) `crates/machine-derive` (Macros)

**Intent:** Add macros aligned to new `StateMeta` once ergonomics are proven.

**Default:** Defer until after PR 15 proves an op end-to-end with the new API.

### Acceptance (when done)

* All CI parity commands pass.
* `trybuild` tests for invalid macro usage.

---

# Phase 2 — Storage Backends (Fast Lane + Postgres + MinIO/S3)

## PR 07 — Storage Implementations (Fast Lane)

**Intent:** Unblock runtime tests without external services *and* support crash/resume determinism.

### Steps

1. Add `crates/storages/event-store` (package `mfm-event-store`):

   * `SqliteEventStore` implementing `mfm_machine::stores::EventStore` (file-backed)
   * SQLite schema (explicit):

     * table `run_events(run_id text not null, seq integer not null, ts_millis integer null, event_json text not null)`
     * primary key `(run_id, seq)`
   * Concurrency model notes (explicit):

     * use WAL mode + `busy_timeout`
     * `append(...)` runs in a single transaction (`BEGIN IMMEDIATE`) so head-seq check + inserts are atomic
   * Optional `InMemoryEventStore` only for micro unit tests (plan validation, pure helpers). Never used for AT-07.
2. Add `crates/storages/artifact-store` (package `mfm-artifact-store`):

   * `FsArtifactStore` writing bytes under content-hash paths
   * directory layout (explicit): `<root>/<kind>/<prefix2>/<artifact_id>` (prevents giant directories)
   * writes are atomic (`tmpfile -> fsync -> rename`) where feasible
   * **Hash-on-read verification**: `get()` recomputes hash and returns `StorageError::Corruption` on mismatch
3. Tests:

   * optimistic concurrency behavior (`expected_seq`) including a concurrent race test (two appenders)
   * atomic append visibility: a multi-event append is all-or-nothing to readers (AT-02)
   * crash/resume support via re-opening SQLite store file
   * artifact de-dupe by content hash
   * artifact corruption detection (tamper file → `Corruption`)

### Acceptance

* All CI parity commands pass without external services.
* Fast lane satisfies: AT-02 (atomic append visibility), AT-03 (expected-seq concurrency), AT-04 (artifact integrity).
* Crash/resume tests (AT-07) run in fast lane using SQLite.

## PR 08 — Postgres `EventStore` Implementation

**Intent:** Implement the primary transactional event store (integration lane), after fast-lane semantics are stable.

### Postgres Schema (Integration Lane Target)

* Table `run_events`:

  * `run_id uuid not null`
  * `seq bigint not null`
  * `ts_millis bigint null`
  * `event_json jsonb not null` (serialized `EventEnvelope`)
  * primary key `(run_id, seq)`

### Append Semantics (Integration Lane Target)

* `append(run_id, expected_seq, events)` is one SQL transaction:

  * acquire a per-run lock (recommended: `pg_advisory_xact_lock` keyed by `run_id`)
  * verify head seq equals `expected_seq` (treat empty run as head = 0)
  * insert `N` events as seq `expected_seq+1..expected_seq+N`
  * commit
* Wrong `expected_seq` → `StorageError::Concurrency`.

### Steps

1. Implement using `sqlx` + tokio.
2. Decide migration story (recommended: `sqlx` migrations) and wire it for integration tests.
3. Define error mapping:

   * JSON deserialization failure on read → `StorageError::Corruption`
   * `ts_millis` is always non-negative and must fit in `i64` (reject out-of-range)
   * tradeoff: `event_json` is schema-less; invariants are enforced at write-time + read-time validation (no DB constraints)
4. For Milestone 1, run listing/status may scan `RunStarted` + terminal kernel events from `run_events` (no separate
   `runs` table). If this becomes painful, add a follow-up schema PR with a `runs` table + supporting indexes.
5. Add integration tests gated behind env (e.g. `MFM_TEST_PG_URL`).

### Acceptance

* Fast lane still passes without Postgres.
* Integration lane passes when env is provided.

## PR 09 — MinIO/S3 `ArtifactStore` Implementation

**Intent:** Implement the primary artifact store (integration lane), after fast-lane semantics are stable.

### Steps

1. Implement using `aws-sdk-s3` (configure endpoint for MinIO).
2. Store objects under key `<kind>/<artifact_id>` (content hash), to support lifecycle/debugging.
3. **Hash-on-read verification** must be streaming (hash while downloading; do not buffer arbitrarily large objects).
4. Define a Milestone 1 artifact size limit and reject larger payloads; multipart upload is deferred.
5. Integration tests gated behind env (endpoint/bucket/credentials). For tests/dev, create the bucket if missing.

### Acceptance

* Fast lane still passes without MinIO.
* Integration lane passes when env is provided.

---

# Phase 3 — Execution Engine + Kernel Events

## PR 10 — Engine Skeleton (Durable Boundaries) + Kernel Events Emission

**Intent:** Make `start` / `resume` real with crash-safe semantics (S1).

### Steps

1. Add `DefaultEngine` implementing `ExecutionEngine`.
2. Define how the engine obtains an `IoProvider`:

   * PR 10 uses a test-only `NoOpIo` (panics on calls) so lifecycle tests don’t depend on PR 12/13.
   * PR 12/13 will provide real `LiveIo` / `ReplayIo` implementations and wiring.
3. Implement `start()`:

   * store manifest artifact (kind `Manifest`)
   * store initial context snapshot artifact (kind `ContextSnapshot`)
   * append `RunStarted`
   * execute plan sequentially (dependency edges)
4. Implement state attempt lifecycle using S1:

   * append `StateEntered` at attempt start (always, even PURE)
   * allow domain events (especially facts) to append incrementally (each append atomic)
   * append `StateCompleted` or `StateFailed` as terminal event
5. Add explicit failpoints around critical windows needed by AT-07/AT-08 crash tests.
6. Implement `resume()`:

   * read kernel events, find last durable boundary, detect in-flight attempts, retry deterministically
   * orphan handling per S1: in-flight attempt does not advance plan progression or snapshot selection
7. Add an `EventRecorder` implementation that appends domain events durably as they are emitted (profile filtered in PR 10b).

### Acceptance

* Unit tests covering:

  * kernel boundary ordering (`Entered` precedes any domain events for the attempt)
  * in-flight attempt resume behavior
  * “no terminal event” retry behavior
  * orphan handling does not advance plan progression/snapshot selection
  * note: IO replay correctness is proven in PR 13/PR 15; PR 10 resume tests focus on no-IO paths via `NoOpIo`

## PR 10b — Engine Policy Semantics (Attempts, Retry, Skip-Tags, Event Profile)

**Intent:** Implement run-level policy semantics that exist in the contract.

### Steps

1. Attempt counters:

   * `StateEntered.attempt` increments per retry
2. Retry policy:

   * retry only retryable errors (`StateError.info.retryable == true`, and relevant `IoError`)
   * apply backoff policy in live mode; replay mode MUST NOT sleep
   * enforce `max_attempts`
3. Skip-tags:

   * define skip semantics precisely:

     * still emit `StateEntered`
     * do not run the handler; do not apply context writes
     * emit a domain event `StateSkipped { state_id, attempt, reason }` (subject to event profile)
     * emit `StateCompleted` referencing the unchanged `base_snapshot_id`
4. Event profile gating:

   * kernel events always emitted
   * domain events emitted based on profile (`Minimal` vs `Normal` vs `Verbose`) via `EventRecorder` filtering

### Acceptance

* Tests for:

  * attempt increments
  * retry stops at max_attempts
  * skip-tags prevents side-effect states from executing
  * event profile gates domain events but not kernel events

## PR 11 — Context Snapshotting (Full Snapshots, Staging, No Secrets)

**Intent:** Full snapshot artifacts with transactional context semantics and secret exclusion (S4).

### Steps

1. Provide a `DynContext` implementation suitable for staging:

   * base context loaded from snapshot
   * staged writes tracked in-memory
   * on commit: materialize full `dump()` and store snapshot artifact
   * note: snapshot size/count can grow; compaction/GC is deferred (see §Explicitly Deferred)
2. Enforce secret exclusion:

   * architectural rule: handlers MUST NOT write secret material into context (S4)
   * optional guardrail: reject reserved key namespaces (e.g. `secret.*`) but do not treat this as sufficient
   * ensure `dump()` is canonical and deterministic (insertion order does not affect bytes/hash)
3. Tests:

   * staged writes discarded on `StateFailed`
   * committed snapshots include full derived context state
   * snapshot determinism: same logical context state → same canonical bytes → same `ArtifactId` (AT-05)
   * attempts to persist secrets fail fast with structured `ContextError` where enforceable (AT-09)

### Acceptance

* All CI parity commands pass.

---

# Phase 4 — IO Provider + Replay Determinism

## PR 12 — `LiveIo` + Fact Recording (Crash-safe, Deduped)

**Intent:** Make IO explicit; record facts durably as they happen (S2).

### Steps

1. Implement `LiveIo`:

   * `call(IoCall)` uses a transport registry (start with a deterministic `TestTransport` keyed by a stable request
     fingerprint so tests can be hermetic)
   * when `fact_key` is present:

     * consult fact index; if exists → return recorded payload
     * else:

       * store response as `ArtifactKind::FactPayload` (must not contain secrets; if the IO result may include
         secrets, it MUST NOT be recorded as a fact; treat it as non-replayable and require `fact_key = None`)
       * append domain event `FactRecorded { key, payload_id, meta }` immediately (durable)
   * clarify source of truth: the event stream is the system of record; any `IoResult.recorded_payload_id` is a
     convenience only and is not durable until `FactRecorded` is appended (orphaned artifacts are possible on crash)
2. Tests with fake transport:

   * live call records payload and emits event
   * retries do not re-call transport when `FactKey` already recorded
   * crash window test (failpoint): crash after storing payload, before appending `FactRecorded` → artifact orphaned,
     and resume either re-calls transport or reuses fact only if recorded (behavior is explicit)

### Acceptance

* All CI parity commands pass.

## PR 13 — `ReplayIo` + Missing Fact Semantics + Fact Index

**Intent:** Replay uses recorded facts/artifacts; missing facts are structured errors (S3).

### Steps

1. Implement a per-run `FactIndex` built from events:

   * on run start/resume, scan events once and build `FactKey -> ArtifactId`
   * facts recorded during in-flight attempts are included (S1: facts are run-scoped)
   * (keep it simple; no separate DB tables required initially)
2. Implement `ReplayIo`:

   * if `fact_key` absent → `IoError` (non-retryable by default)
   * if key missing → `IoError::MissingFact { key, info }`
   * if key present → load payload from `ArtifactStore` and return
3. Determinism test:

   * live run records facts (including any non-deterministic inputs like time/randomness, if used) and produces output artifact
   * replay run produces identical derived output artifact IDs **for replayable paths**

### Acceptance

* All CI parity commands pass.

---

# Phase 5 — `mfm-sdk` Planning + First Op

## PR 14 — Add `crates/sdk` (Operation + Pipeline)

**Intent:** Move planning ergonomics out of `machine` per boundary rules.

### Steps

1. Add `crates/sdk` (package `mfm-sdk`) containing:

   * `Operation::{op_id(), op_version(), expand(...)} -> StateGraph`
   * `Pipeline::then(...)` and `build() -> ExecutionPlan`
2. Wire versioning into manifests/events:

   * `RunManifest` must include `op_id + op_version` and the SDK should populate it automatically.
3. Implement stable ID assignment per §10.7 (validated):

   * `OpPath = <machine_id>.<step_id>`
   * `StateId = <machine_id>.<step_id>.<state_local_id>`
   * enforce the segment regex from §Decision Defaults; reject dots inside segments
4. Add plan validation tests:

   * cycles, duplicates, missing nodes
   * cross-op `StateId` collision detection
   * duplicate step IDs within a machine
   * invalid IDs are rejected with structured `InvalidPlan` errors

### Acceptance

* All CI parity commands pass.

## PR 15 — Add `crates/ops/<proof-op>` End-to-End (Includes Side-Effect Idempotency)

**Intent:** Prove the architecture with one runnable op that exercises the “dragons.”

### Op shape

A single op expands into a small DAG (must include at least one fork/join), e.g.:

* `config` (PURE)
* `fetch_a` (READ_ONLY_IO; records fact)
* `fetch_b` (READ_ONLY_IO; records fact)
* `join` (PURE; combines `fetch_a` + `fetch_b`)
* `execute` (APPLY_SIDE_EFFECT; idempotent)
* `report` (writes `ArtifactKind::Output`)

Edges:

* `config → fetch_a → join → execute → report`
* `config → fetch_b → join`

### Key semantics

* `execute` must use an idempotency key (intent hash) and record it as a domain event.
* For replay determinism tests, `RunConfig.skip_tags` can skip `APPLY_SIDE_EFFECT` so the
  replayable portion remains deterministic.
* Side effects are **not expected to be replayable** without credentials; they are only
  retryable in environments where credentials exist at runtime.
* The proof op MUST expose a single “output ArtifactId” (the `report` output), which is what determinism/crash tests compare.

### Tests (design-level)

1. **Live → Replay determinism (replayable path):**

   * Run live with `skip_tags = [APPLY_SIDE_EFFECT]` (records facts, produces report)
   * Run replay with same inputs
   * Assert identical output `ArtifactId` (the `report` output)
2. **Crash/resume determinism (durable store):**

   * Use deterministic failpoints (see §Testing Strategy) to crash after N appended events during the run
   * Resume using SQLite event store
   * Assert run completes and produces same output `ArtifactId` (replayable path)
3. **Side-effect idempotency/dedupe:**

   * Use a test “external sink” representing the side-effect target (e.g., sqlite table with UNIQUE(idempotency_key))
   * Crash after side effect applied but before `StateCompleted`
   * Resume; assert side effect not applied twice (dedupe via idempotency key), e.g., `count(*) == 1`
4. **Non-determinism footguns are recorded when used:**

   * Exercise `IoProvider::now_millis()` and/or `IoProvider::random_bytes()` in at least one replayable path
   * Assert live records the fact(s) and replay produces identical derived output IDs

### Acceptance

* All CI parity commands pass (fast lane includes crash tests using SQLite).
* Fast lane satisfies: AT-06 (live→replay determinism), AT-07 (crash/resume determinism), AT-08 (side-effect idempotency).

---

# Phase 6 — Replace CLI With Thin Orchestrator

## PR 16 — New `bin/cli` (Run Start/Resume/Inspect)

**Intent:** Make binaries thin wrappers over sdk+machine+storages.

### Commands (Minimum Set + Operability)

* `run start --op <id> --input <json> [--io-mode live|replay]`
* `run resume --run-id <uuid>`
* `run status --run-id <uuid>`
* `run list [--op <id>] [--status <status>]` (may be implemented by scanning `RunStarted`/terminal events initially)
* `run events --run-id <uuid> [--from <seq>]`
* `plan inspect --op <id> --input <json>` (render/validate plan without running)
* `artifact get --id <hash>`
* (optional) `artifact list --run-id <uuid>` (or `run artifacts`)

Scope note (keep PR reviewable): land `run start`, `run resume`, `run events`, and `artifact get` first; then add
`run list/status` and `plan inspect` if it stays small.

### Store Configuration (Required)

The CLI must be able to connect to event + artifact stores.

* Flags (preferred): `--event-store <url>` and `--artifact-store <url>`
* Env fallbacks: `MFM_EVENT_STORE_URL`, `MFM_ARTIFACT_STORE_URL`
* Fast-lane defaults may be provided (local SQLite file + local artifact directory), but must be explicit and
  not silently write secrets.

### Output Contract

* Preserve global `--output-format` and `MFM_OUTPUT_FORMAT` semantics.
* Stable JSON schema for success/errors.
* **No secrets in outputs** (including error details).

### Acceptance

* CLI e2e tests for new commands.
* JSON schema stability tests (goldens/snapshots) for at least:

  * success output for `run start`, `run resume`, `run events`, `artifact get`
  * structured error outputs (missing run, missing artifact, missing fact)
* Legacy keystore commands remain available (temporarily) or are isolated as separate subcommands
  that do not touch event/artifact stores (no implicit store initialization, no run/event emission).

## PR 17 — Deprecate/Remove Legacy CLI

* Provide a short migration guide mapping legacy commands to new commands.
* Deprecate legacy commands for a defined period (even if just one release window).
* Remove `bin/cli-legacy` after new CLI is the default.
* Update `flake.nix` to build and `nix run` the new CLI.

---

# Phase 7 — REST API Scaffold (Deferred / Optional)

## PR 18 — `bin/rest-api` Minimal Surface

* Endpoints mirroring CLI surfaces:

  * start run, resume run, list/read events, get artifact
* No auth; explicitly non-production.
* Deferred until after PR 20 (avoid duplicating surfaces while core semantics are still moving).

---

# Phase 8 — CI Integration Lane (Postgres + MinIO)

## PR 19 — GitHub Actions Services for Postgres + MinIO

* Add an integration job that runs tests with service containers:

  * Postgres (pin an explicit major/minor tag, e.g. `postgres:16-alpine`)
  * MinIO (pin an explicit `RELEASE.*` tag)
* Add explicit readiness/health checks before running tests:

  * Postgres: `pg_isready` loop
  * MinIO: HTTP health endpoint + ensure the bucket exists
* Gate integration tests behind env vars so fast lane remains unchanged.
* This can land as soon as PR 08 + PR 09 exist; it should not be blocked by PR 18.

---

# Phase 9 — Remove Legacy State Machine

## PR 20 — Delete Legacy Machine + Derive

Prerequisites:

* No crate depends on `mfm-machine-legacy` or `mfm-machine-derive-legacy`.
* All ops/CLI use new `mfm-machine` + stores + sdk.
* Docs updated (README/ARCHITECTURE) to remove legacy references.
* If legacy proc-macro ergonomics are required, PR 06 is done or manual impls exist.
* `flake.nix` builds, `nix run` works, and CI passes without legacy crates.

Steps:

* Remove legacy crates from workspace.
* Remove legacy docs/tests referencing them.

Acceptance:

* All CI parity commands pass.
* All acceptance tests AT-01..AT-09 pass in fast lane.

---

## Explicitly Deferred (Architectural Concerns)

These are important, but explicitly *not* Milestone 1 deliverables (called out so they don’t get lost):

* Observability story (structured tracing/metrics)
* Error recovery beyond retry (dead-letter handling, manual override, compensations)
* Context growth controls (size limits, eviction, snapshot compaction)
* Artifact garbage collection (unreferenced content cleanup)
* Nested machines / child-run spawning + linkage (engine-level)
* Deterministic rewinds across completed transitions (kernel rewind marker + projection semantics)
* Encrypted secret-bearing artifacts (Milestone 1 is “no secrets persisted”)

---

## Review Checklist (v3.0)

* [ ] Workspace/binary naming is finalized (`mfm` vs `mfm_cli`)
* [ ] PR dependency graph is respected (no PR merged before its prerequisites)
* [ ] D-01..D-06 deviations resolved (contract updated, or plan adjusted) before implementation proceeds
* [ ] Canonical JSON implementation validated with edge cases (numbers, ordering, forbidden values)
* [ ] No floats in hashed structures policy is enforced (or explicitly changed with tests)
* [ ] `ArtifactId` validation enforced (64-char lowercase hex; reject invalid)
* [ ] `seq` numbering convention is implemented and tested (1-indexed; head=0)
* [ ] ID regex + examples are aligned (no contract examples that violate the enforced segment regex)
* [ ] Event durability boundaries follow S1; no “IO happened but not recorded” window
* [ ] Orphaned events from in-flight attempts are handled correctly on resume (facts included; non-fact domain events do not advance)
* [ ] `FactKey` is single-assignment and deduped (S2)
* [ ] Non-deterministic inputs (time/randomness) are recorded as facts when used
* [ ] Replay default requires `fact_key` (S3)
* [ ] Missing-fact retryability is explicit and configurable (default non-retryable)
* [ ] Secrets excluded from all persisted surfaces (S4), with tests
* [ ] Error messages/details are scrubbed (no raw URLs/headers/keys)
* [ ] Artifact stores verify hash-on-read and surface corruption errors
* [ ] Fast lane uses durable store (SQLite) for crash/resume tests
* [ ] SQLite concurrency behavior is tested (concurrent append race)
* [ ] Side-effect idempotency/dedupe has an explicit test
* [ ] Crash simulation mechanism is implemented (failpoints) and used by AT-07/AT-08 tests
* [ ] Snapshot determinism is tested (same logical state → same canonical bytes/hash)
