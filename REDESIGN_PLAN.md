# MFM — `REDESIGN.md` Implementation Plan (Milestone 1)

> Source of truth: `REDESIGN.md` (v4, last updated 2026-02-09).
> Generated: 2026-02-10
>
> Goal: a complete, step-by-step TODO list to reach the `REDESIGN.md` Milestone 1 architecture in THIS repo.

---

## Summary

This plan migrates the current workspace (`mfm_core`, `mfm_machine`, `mfm_cli`) to the v4 redesign architecture:

- append-only, per-run event streams (kernel + domain events)
- transactional, atomic `EventStore::append([...])`
- attempt envelopes across one-or-more appends
- content-addressed artifacts (manifest, snapshots, fact payloads, outputs)
- canonical JSON hashing (RFC 8785/JCS semantics) + SHA-256
- explicit IO (live vs replay) with fact recording and structured missing-fact errors
- crash-safe resume semantics (orphan attempt handling)
- thin CLI to start/resume/inspect runs, while keeping existing keystore commands

Milestone 1 hard rule from `REDESIGN.md`: **secrets must not appear in persisted run surfaces**
(manifests, events, artifacts including fact payloads and snapshots, CLI outputs, and error details).

---

## Current Baseline (Repo Reality)

- Workspace crates today:
  - `mfm_core/`: keystore + config models (security-critical)
  - `mfm_machine/`: legacy state machine framework
  - `mfm_cli/`: CLI exposing keystore management
- CI today:
  - Rust CI runs `cargo +nightly fmt`, `cargo +nightly clippy --all-features`, `cargo nextest run --workspace`
  - Nix CI runs `nix flake check` and `nix build`

---

## Locked Decisions (Chosen Upfront)

These are implementation decisions that remove ambiguity for the rest of the work:

1. **CLI scope**: keep existing `keystore` subcommands in the same CLI binary while adding run start/resume/inspect.
2. **Repo layout**: adopt `crates/` + `bin/` early (start reorganizing up front).
3. **Storage backends for Milestone 1**: implement both a fast local lane and a parity lane:
   - **fast lane (default tests)**: in-memory `EventStore` + filesystem `ArtifactStore`
   - **parity lane (integration)**: PostgreSQL `EventStore` + MinIO/S3 `ArtifactStore`
   - parity tests MUST be opt-in so `cargo nextest run --workspace` remains service-free by default.
4. **Manifest ownership**: the caller (SDK/CLI/tests) computes `manifest_id` (canonical JSON hash) and stores the
   manifest as an artifact before calling `ExecutionEngine::start`. The engine validates the ID and references it
   from `RunStarted`.

---

## Workflow Rules (Git)

- Work directly on the `dev` branch for this milestone (no long-lived feature branches).
- Each PR-sized unit below MUST land as a **single commit**.
- Commit message MUST be prefixed with: `redesign: `
  - Example: `redesign: layout scaffold`
- Keep the repo green after each commit (fmt/clippy/nextest).

---

## Non-Goals (Milestone 1)

- REST API surface (deferred)
- ClickHouse projections/indexers (deferred)
- Parallel execution (deferred; executor is sequential)
- Engine-managed child runs / nested machines (deferred; Milestone 1 uses flattened composition only)
- Artifact GC/compaction (deferred)
- Encrypted secret-bearing artifacts for the run system (deferred; Milestone 1 is "no secrets persisted")

---

## Public Interfaces (Milestone 1)

Milestone 1 public API contract for `mfm-machine` is Appendix C.1 in `REDESIGN.md`.
Milestone 1 public API contract for `mfm-sdk` is Appendix C.2 in `REDESIGN.md`.
This plan follows that contract; any deviation MUST be recorded in [Deviations](#deviations).

### Kernel events (required for recovery)

- `RunStarted { op_id, manifest_id, initial_snapshot_id }`
- `StateEntered { state_id, attempt, base_snapshot_id }`
- `StateCompleted { state_id, context_snapshot_id }`
- `StateFailed { state_id, error, failure_snapshot_id? }`
- `RunCompleted { status, final_snapshot_id? }`

### Domain events (audit-only; never required for correctness)

Milestone 1 domain events use a generic envelope:

- `DomainEvent { name, payload, payload_ref? }`

Recommended standard payloads (helpers only; not required by the engine):

- `FactRecorded { key, payload_id, meta }`
- `ArtifactWritten { artifact_id, kind, meta }`
- `OpBoundary { op_path, phase }`

Rules:

- Domain events must never be required for plan progression or resume checkpoint selection (kernel events remain sufficient).
- Domain events must never contain secrets.
- Domain event emission is controlled by `RunConfig.event_profile` (Minimal/Normal/Verbose/Custom).

Determinism note (Milestone 1):
- If the runtime records fact payloads (replayable IO with `fact_key`, or `now_millis()` / `random_bytes()`),
  it MUST persist a `FactKey -> payload_id` binding as a `fact_recorded` domain event regardless of `EventProfile`,
  otherwise `ReplayIo` and orphan-attempt fact reuse cannot be correct.

### Store traits (signatures)

- `EventStore` (async):
  - `head_seq(run_id) -> Result<u64, StorageError>`
  - `append(run_id, expected_seq, events) -> Result<u64, StorageError>` (atomic; optimistic concurrency)
  - `read_range(run_id, from_seq, to_seq: Option<u64>) -> Result<Vec<EventEnvelope>, StorageError>`
- `ArtifactStore` (async):
  - `put(kind, bytes) -> Result<ArtifactId, StorageError>` (content-addressed)
  - `get(id) -> Result<Vec<u8>, StorageError>` (verify hash; corruption => `StorageError::Corruption`)
  - `exists(id) -> Result<bool, StorageError>`

### IDs and naming conventions

- `EventEnvelope.seq` is 1-indexed; empty run head is 0
- `OpPath = <machine_id>.<step_id>` (2 segments)
- `StateId = <machine_id>.<step_id>.<state_local_id>` (3 segments)
- each segment matches `^[a-z][a-z0-9_]{0,62}$`
- `ArtifactId` is SHA-256 lowercase hex, 64 chars
- `ContextKey` and `ErrorCode` are stable string newtypes (see Appendix C.1)

---

## Acceptance Tests (Milestone 1)

Each invariant must be verifiable by at least one named acceptance test:

- AT-01 AppendOnlyEventStream
- AT-02 AtomicAppendVisibility
- AT-03 ExpectedSeqConcurrency
- AT-04 ArtifactContentAddressing
- AT-05 CanonicalJsonHashing
- AT-06 LiveThenReplayDeterminism
- AT-07 CrashResumeDeterminism
- AT-08 SideEffectIdempotency
- AT-09 SecretsNeverPersisted
- AT-10 FactKeySingleAssignment
- AT-11 StateIdStabilityAndShape
- AT-12 FullSnapshotAsArtifact
- AT-13 ReplayMissingFactErrors
- AT-14 TimeAndRandomAsFacts

---

## CI Requirements (fast lane + parity lane)

Rust CI currently runs `cargo nextest run --workspace` with no services. For Milestone 1 we keep that as the default
**fast lane**, using local store implementations (in-memory `EventStore` + filesystem `ArtifactStore`).

Add an opt-in **parity lane** that runs against Postgres + MinIO:

1. Add a separate GitHub Actions job (or workflow) with `services:` for `postgres` and `minio`.
2. Gate parity integration tests behind a feature flag or env toggle so they do not run in the fast lane.
3. Make parity tests read connection details from env vars (with sane defaults matching CI services).

Both lanes MUST satisfy the same correctness contract/invariants.

---

## Step-by-Step TODO (PR-Sized)

Each PR is intended to be small, reviewable, and keep the repo green.

### PR 01 — Adopt `crates/` + `bin/` layout (scaffold only)

- Create `crates/` and `bin/cli/`
- Move existing crates into new locations (minimize code changes; fix paths only):
  - `mfm_core/` -> `crates/core/`
  - `mfm_machine/` -> `crates/machine-legacy/`
  - `mfm_machine_derive/` -> `crates/machine-derive-legacy/`
  - `mfm_cli/` -> `bin/cli/`
- Update root `Cargo.toml` workspace members
- Ensure all existing tests still pass

### PR 02 — Restore boundary rule: `core` must not depend on machine

- Remove dependency `core -> machine-legacy`
- Replace any `mfm_machine` types re-exported by core with:
  - local equivalents in core, or
  - types moved into the new v4 machine/sdk crates (later PRs)
- Ensure keystore remains unchanged behaviorally (security-sensitive)

### PR 03 — Introduce v4 crate skeletons (no behavior yet)

- Add:
  - `crates/machine/` (package `mfm-machine`)
  - `crates/machine-derive/` (package `mfm-machine-derive`)
  - `crates/sdk/` (package `mfm-sdk`)
  - `crates/storages/event-store-mem/` (package `mfm-event-store-mem`) (fast lane)
  - `crates/storages/artifact-store-fs/` (package `mfm-artifact-store-fs`) (fast lane)
  - `crates/storages/event-store-postgres/` (package `mfm-event-store-postgres`)
  - `crates/storages/artifact-store-s3/` (package `mfm-artifact-store-s3`)
  - `crates/ops/proof-op/` (package `mfm-op-proof`)
  - `crates/ops/keystore-op/` (package `mfm-op-keystore`)
- Add minimal `lib.rs` and `README.md` per crate describing boundaries
- Keep legacy machine in place for now

### PR 04 — `mfm-machine` public API contract (Appendix C.1)

Define the stable Milestone 1 API surface exactly as described in `REDESIGN.md` Appendix C.1 (types + traits only):

- `ids`: `RunId`, `OpId`, `OpPath`, `StateId`, `ArtifactId`, `FactKey`, `ContextKey`, `ErrorCode`
- `config`: `IoMode`, `EventProfile`, retry/backoff policy types, `RunConfig`, `RunManifest`, `BuildProvenance`
- `meta`: tags + side-effect classification + idempotency declarations
- `errors`: `ErrorInfo`-based structured errors (no secrets)
- `context`: `DynContext` + typed extension trait
- `events`: kernel events, generic `DomainEvent`, `EventEnvelope`, and recommended standard payload helpers
- `io`: `IoCall`/`IoResult` + `IoProvider`
- `recorder`: `EventRecorder`
- `state`: `State` trait + `StateOutcome`
- `plan`: `StateGraph` / `ExecutionPlan` + plan validation errors
- `stores`: `EventStore` + `ArtifactStore` traits + `ArtifactKind`
- `engine`: `ExecutionEngine::{start,resume}` contract types

Add unit tests for:

- ID validation (including `OpPath`/`StateId` segment regex invariants)
- stable error code `missing_fact_key`

Implementation notes:

- `BackoffPolicy` uses `Duration` in Appendix C.1; implement a stable serde representation (integer-based; no floats)
  so `RunManifest` remains canonical-JSON hashable.

### PR 05 — Canonical JSON + SHA-256 helpers (AT-05)

- Implement canonical JSON bytes for hashing (RFC 8785 / JCS semantics)
- Reject floats (fractional JSON numbers) and NaN/Inf in hashed structures
- Implement `ArtifactId` computation (SHA-256 lowercase hex)
- Add tests:
  - canonicalization vectors
  - float rejection
  - `ArtifactId` format

### PR 06 — ArtifactStore implementations (fast lane + parity lane) (AT-04)

- Implement filesystem backend in `mfm-artifact-store-fs` (fast lane; no services)
- Implement S3/MinIO backend in `mfm-artifact-store-s3` (parity lane)
- Both backends MUST:
  - be content-addressed (`put` computes `ArtifactId` from bytes)
  - verify hashes on `get` (corruption => `StorageError::Corruption`)
  - support `exists`
- Tests:
  - add a backend-agnostic ArtifactStore contract test harness (reused by all backends)
  - place the harness in a dependency-cycle-safe location (e.g. `mfm-machine` internal test-support module or a tiny helper crate
    that depends only on `mfm-machine`)
  - fast-lane contract tests (AT-04) using filesystem backend
  - parity integration contract tests for S3/MinIO backend (gated; see CI section)

### PR 07 — EventStore implementations (fast lane + parity lane) (AT-01..AT-03)

- Implement in-memory backend in `mfm-event-store-mem` (fast lane)
- Implement Postgres backend in `mfm-event-store-postgres` (parity lane):
  - schema for runs + events
  - append in a single transaction:
    - validate `expected_seq` vs current head
    - insert contiguous seq events
    - return new head seq
  - `read_range(run_id, from_seq, to_seq: Option<u64>)`
- Tests:
  - add a backend-agnostic EventStore contract test harness (reused by all backends)
  - place the harness in a dependency-cycle-safe location (e.g. `mfm-machine` internal test-support module or a tiny helper crate
    that depends only on `mfm-machine`)
  - fast-lane contract tests:
    - AT-01 AppendOnlyEventStream
    - AT-02 AtomicAppendVisibility
    - AT-03 ExpectedSeqConcurrency
  - parity integration contract tests for Postgres (gated; see CI section)

### PR 08 — Event emission + attempt envelope validation + event profiles

- Implement attempt-envelope validation helpers (kernel envelope + orphan detection):
  - `StateEntered ... (StateCompleted|StateFailed)` rules
  - orphan attempt handling rules (domain events allowed; must not advance checkpoint)
- Implement `EventRecorder` plumbing:
  - domain events are emitted only via `EventRecorder`
  - domain events may span multiple transactional appends (per-append atomicity preserved)
- Implement event profile behavior (`RunConfig.event_profile`):
  - `minimal`: kernel + determinism-critical runtime domain events (at least `fact_recorded` when facts are recorded)
  - `normal`: `minimal` + facts + artifacts + boundaries (recommended audit surface)
  - `verbose`: more domain details (no secrets)
- Add tests for:
  - envelope validation
  - event profile filtering
  - “domain events are never required for plan progression/checkpoint selection” invariants

### PR 09 — Context + full snapshots as artifacts

- Implement staged context:
  - base snapshot -> staged writes
  - commit on success only
  - discard on failure
- Snapshot = full snapshot stored as artifact (content-addressed)
- Add determinism tests (same logical state => same snapshot id) (AT-12)

### PR 10 — Executor + resume + orphan attempt semantics (AT-07 core)

- Implement run start plumbing (aligned with Appendix C.1 `StartRun`):
  - `StartRun` carries both `manifest` and precomputed `manifest_id`
  - caller (SDK/CLI/tests) stores the manifest as an artifact before starting the run
  - engine validates:
    - `artifacts.exists(&manifest_id)` is true (or returns a structured storage error)
    - `manifest_id == hash(canonical_json(manifest))` (defense-in-depth)
    - `manifest.op_id == plan.op_id` (fail fast on mismatch; engine uses this `op_id` in `RunStarted`)
    - `run_config == manifest.run_config` (fail fast on mismatch)
  - store the initial context snapshot as an artifact
  - append `RunStarted { op_id, manifest_id, initial_snapshot_id }`
- Implement sequential executor:
  - validate `RunConfig.execution_mode` is `Sequential`; reject `FanOutJoin { .. }` with a structured error
    (e.g., `RunError::InvalidPlan` + stable `ErrorCode` `unsupported_execution_mode`)
  - append `StateEntered` before handler
  - domain events may span multiple appends
  - on success: write snapshot artifact, append `StateCompleted`
  - on failure: append `StateFailed` (no checkpoint advance)
- Implement retry/backoff policy:
  - honor `RunConfig.retry_policy` and state `attempt` increments
  - only retry errors explicitly marked retryable via structured `ErrorInfo`
- Implement resume:
  - compute checkpoint as last `StateCompleted.context_snapshot_id`
  - handle orphan attempt (stream ends after `StateEntered`): retry from `base_snapshot_id`
- Add crash/resume tests with deterministic failpoints:
  - AT-07

### PR 11 — Live IO + fact recording (facts are single-assignment)

- Define `IoProvider` and `EventRecorder`
- Implement `LiveIo`:
  - replayable IO uses `fact_key`
  - facts are single-assignment (first durable `FactRecorded` wins)
  - time/random recorded as facts when used, with stable attempt-scoped keys derived from:
    - `(run_id, state_id, attempt, call_ordinal, kind)` (encoding must be stable)
  - implementation note: avoid `&mut dyn IoProvider` + `&mut dyn EventRecorder` borrowing conflicts by buffering
    `FactRecorded` domain events inside the concrete `LiveIo` value and letting the engine drain+append them via the
    recorder after `State::handle` returns (attempt envelope semantics preserved; domain events still appended by the engine)
  - ensure resume builds a run-scoped FactKey index by scanning existing `FactRecorded` domain events (including orphan attempts)
    so retries/replays reuse previously recorded facts (per `REDESIGN.md` §6.2 + §7.3)
- Tests:
  - fact dedupe + single-assignment (AT-10)
  - time/random fact recording (AT-14)

Implementation detail clarification (align with `REDESIGN.md` §8.3):
- Time/random fact keys are attempt-scoped and must be derived from:
  `(run_id, state_id, attempt, call_ordinal, kind)`, where `call_ordinal` increments per `(state_id, attempt)`.

### PR 12 — Replay IO semantics (AT-06 foundation)

- Implement `ReplayIo`:
  - deterministic IO missing `fact_key` => `MissingFactKey` (`missing_fact_key`)
  - missing fact => `MissingFact`
  - retryability controlled by `RunConfig.replay_missing_fact_retryable`
- Tests for stable error semantics (AT-13)

### PR 13 — SDK: operations and pipelines (flattened composition)

- Implement the `mfm-sdk` public API contract (Appendix C.2 in `REDESIGN.md`):
  - `Operation` + `OperationRegistry`
  - `Pipeline` + `PipelinePlanner`
  - `RunLauncher` (`start_pipeline`/`resume`)
- Define `Operation` trait:
  - `op_id`, `op_version`, `expand(...) -> StateGraph`
- Define pipeline builder:
  - enforce `OpPath` and `StateId` shapes
  - wrap single-op runs with `step_id = main`
- Implement namespacing + wiring validation:
  - default context key namespacing by `OpPath`
  - explicit exports/imports and planner validation that imports are satisfiable
- Add tests for ID enforcement and stable expansion
- Implement a thin run launcher helper in the SDK contract:
  - compute/store manifest artifact
  - call `ExecutionEngine::start` with `StartRun { manifest, manifest_id, plan, run_config, initial_context }`

### PR 14 — Proof op end-to-end (AT-06..AT-08)

- Implement `mfm-op-proof`:
  - READ_ONLY_IO state recording facts
  - APPLY_SIDE_EFFECT state with explicit idempotency key recorded in domain events
  - output artifact written and referenced
- Add end-to-end tests:
  - AT-06 LiveThenReplayDeterminism
  - AT-07 CrashResumeDeterminism
  - AT-08 SideEffectIdempotency

### PR 15 — Thin CLI: run start/resume/inspect + keep keystore

- Add `run` commands:
  - `run start`
  - `run resume`
  - `run events`
  - `run artifacts get`
  - `run status`
- Keep existing `keystore` subcommands and output contract
- Enforce boundary rule from `REDESIGN.md`:
  - CLI depends on `sdk` + `ops` (not directly on `core`)
  - implement keystore command behavior in `mfm-op-keystore` (which depends on `core`)
- Ensure stable `--output-format` across both
- Add CLI JSON schema stability tests for new surfaces

### PR 16 — AT-09 SecretsNeverPersisted guardrails

- Ensure secrets cannot reach persisted run surfaces:
  - no secret-bearing payload bytes in error strings/events/artifacts
  - redaction strategy for error details
- Add tests that attempt to leak secrets and assert persisted surfaces remain clean

### PR 17 — Cleanup: remove legacy machine crates

- Delete legacy machine and derive crates once v4 is fully in use
- Ensure all acceptance tests pass in CI

---

## Master Checklist

- [ ] PR 01 Layout scaffold (`crates/` + `bin/`)
- [ ] PR 02 Core boundary fix (core independent of machine)
- [ ] PR 03 v4 crate skeletons
- [ ] PR 04 `mfm-machine` API contract (Appendix C.1)
- [ ] PR 05 Canonical JSON + SHA-256 (AT-05)
- [ ] PR 06 ArtifactStore implementations (fast + parity) (AT-04)
- [ ] PR 07 EventStore implementations (fast + parity) (AT-01..AT-03)
- [ ] PR 08 Event emission + envelopes + profiles
- [ ] PR 09 Context snapshots as artifacts
- [ ] PR 10 Executor + resume + orphan handling (AT-07)
- [ ] PR 11 LiveIo + facts
- [ ] PR 12 ReplayIo semantics
- [ ] PR 13 SDK operations + pipeline
- [ ] PR 14 Proof op end-to-end (AT-06..AT-08)
- [ ] PR 15 CLI run commands + keep keystore
- [ ] PR 16 Secrets guardrails (AT-09)
- [ ] PR 17 Remove legacy machine crates

---

## Decision Log (Plan-Level)

- DL-01 Two-lane testing: keep service-free fast lane by default; parity lane is opt-in (matches `REDESIGN.md` §12.3).
- DL-02 Legacy coexistence: keep v1 machine as `crates/machine-legacy/` until Milestone 1 is complete, then delete.
- DL-03 Manifest ownership: caller stores manifest artifact + computes `manifest_id`; engine validates existence + hash.
- DL-04 LiveIo fact recording: buffer fact-related domain events in LiveIo and drain/append them via the engine.
- DL-05 Store correctness: use a shared backend-agnostic contract test harness for EventStore/ArtifactStore.
- DL-06 Duration serialization: keep `Duration` types per Appendix C.1, but serialize deterministically using an integer
  representation suitable for canonical JSON hashing (no floats).

## Risk Register (Plan-Level)

- R-01 Canonical JSON corner cases: mitigate with RFC 8785/JCS vectors + targeted fuzz/property tests.
- R-02 Secrets leakage across persisted surfaces: mitigate with explicit redaction + AT-09 scanning tests.
- R-03 Borrow/ownership complexity in IO + event recording: mitigate with the LiveIo buffering pattern (DL-04).
- R-04 Parity lane drift from fast lane: mitigate with shared contract tests (DL-05).
- R-05 SDK contract churn risk: mitigate by treating Appendix C.2 as authoritative and recording deviations explicitly.

---

## Deviations

None.
