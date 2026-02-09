# MFM — `REDESIGN_FINAL.md` Implementation Plan (Milestone 1)

> Source of truth: `REDESIGN_FINAL.md` (v4, last updated 2026-02-09).
> Generated: 2026-02-10
>
> Goal: a complete, step-by-step TODO list to reach the `REDESIGN_FINAL.md` Milestone 1 architecture in THIS repo.

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

Milestone 1 hard rule from `REDESIGN_FINAL.md`: **secrets must not appear in persisted run surfaces**
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
3. **Storage backends for Milestone 1**: go straight to **PostgreSQL EventStore** + **MinIO (S3) ArtifactStore**.

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

### Kernel events (required for recovery)

- `RunStarted { op_id, manifest_id, initial_snapshot_id }`
- `StateEntered { state_id, attempt, base_snapshot_id }`
- `StateCompleted { state_id, context_snapshot_id }`
- `StateFailed { state_id, error, failure_snapshot_id? }`
- `RunCompleted { status, final_snapshot_id? }`

### Domain events (audit-only; never required for correctness)

Examples:

- `FactRecorded { key, payload_id, meta }`
- `ArtifactWritten { artifact_id, kind, meta }`
- `OpBoundary { op_path, phase }`

Rule: domain events must never contain secrets.

### IDs and naming conventions

- `EventEnvelope.seq` is 1-indexed; empty run head is 0
- `OpPath = <machine_id>.<step_id>` (2 segments)
- `StateId = <machine_id>.<step_id>.<state_local_id>` (3 segments)
- each segment matches `^[a-z][a-z0-9_]{0,62}$`
- `ArtifactId` is SHA-256 lowercase hex, 64 chars

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

---

## CI Requirements (because Postgres + MinIO are mandatory)

Rust CI currently runs `cargo nextest run --workspace` with no services. For Milestone 1 tests that require
Postgres/MinIO, we will make CI decision-complete by doing BOTH:

1. Add GitHub Actions `services:` for `postgres` and `minio` to `.github/workflows/checks.yml`.
2. Make integration tests read connection details from env vars (with sane defaults matching CI services).

This keeps unit tests fast and makes integration-lane tests reliable in CI without manual setup.

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
  - `crates/sdk/` (package `mfm-sdk`)
  - `crates/storages/event-store-postgres/` (package `mfm-event-store-postgres`)
  - `crates/storages/artifact-store-s3/` (package `mfm-artifact-store-s3`)
  - `crates/ops/proof-op/` (package `mfm-op-proof`)
- Add minimal `lib.rs` and `README.md` per crate describing boundaries
- Keep legacy machine in place for now

### PR 04 — Canonical JSON + SHA-256 helpers (AT-05)

- Implement canonical JSON bytes for hashing (RFC 8785 / JCS semantics)
- Reject floats (fractional JSON numbers) and NaN/Inf in hashed structures
- Define `ArtifactId` newtype (sha256 hex)
- Add tests:
  - canonicalization vectors
  - float rejection
  - ArtifactId format

### PR 05 — Core v4 types, IDs, and errors

- Define and validate:
  - `RunId`, `OpId`, `OpPath`, `StateId`, `FactKey`
- Define `RunConfig` including `replay_missing_fact_retryable: bool` (default false)
- Define structured errors:
  - `IoError::MissingFactKey` with stable code `missing_fact_key`
  - `IoError::MissingFact { key, ... }`
  - `StorageError`, `StateError` with explicit retryability semantics
- Add unit tests for validation + error code stability

### PR 06 — ArtifactStore trait + MinIO/S3 implementation (AT-04 partial)

- Add `ArtifactStore` trait to v4 machine:
  - `put(kind, bytes) -> ArtifactId` (content-addressed)
  - `get(id) -> bytes` (verify hash; corruption => error)
- Implement S3/MinIO backend in `mfm-artifact-store-s3`
- Add integration tests using MinIO service

### PR 07 — EventStore trait + Postgres implementation (AT-01..AT-03 foundation)

- Add `EventStore` trait to v4 machine:
  - `head_seq(run_id)`
  - `append(run_id, expected_seq, events)` (atomic; optimistic concurrency)
  - `read_range(run_id, from_seq, to_seq)`
- Implement Postgres backend in `mfm-event-store-postgres`:
  - schema for runs + events
  - append in a single transaction:
    - validate expected head
    - insert contiguous seq events
- Add integration tests:
  - AT-02 atomic visibility
  - AT-03 expected seq concurrency

### PR 08 — Kernel + domain events and attempt envelope validation

- Define kernel and domain event payloads
- Define `EventEnvelope` shape and (de)serialization
- Add attempt-envelope validation helpers:
  - `StateEntered ... terminal` rules
- Add tests for event stability (no secrets)

### PR 09 — Context + full snapshots as artifacts

- Implement staged context:
  - base snapshot -> staged writes
  - commit on success only
  - discard on failure
- Snapshot = full snapshot stored as artifact (content-addressed)
- Add determinism tests (same logical state => same snapshot id)

### PR 10 — Executor + resume + orphan attempt semantics (AT-07 core)

- Implement sequential executor:
  - append `StateEntered` before handler
  - domain events may span multiple appends
  - on success: write snapshot artifact, append `StateCompleted`
  - on failure: append `StateFailed` (no checkpoint advance)
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
  - time/random recorded as facts when used
- Tests:
  - fact dedupe
  - time/random fact recording

### PR 12 — Replay IO semantics (AT-06 foundation)

- Implement `ReplayIo`:
  - deterministic IO missing `fact_key` => `MissingFactKey` (`missing_fact_key`)
  - missing fact => `MissingFact`
  - retryability controlled by `RunConfig.replay_missing_fact_retryable`
- Tests for stable error semantics

### PR 13 — SDK: operations and pipelines (flattened composition)

- Define `Operation` trait:
  - `op_id`, `op_version`, `expand(...) -> StateGraph`
- Define pipeline builder:
  - enforce `OpPath` and `StateId` shapes
  - wrap single-op runs with `step_id = main`
- Add tests for ID enforcement and stable expansion

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
- [ ] PR 04 Canonical JSON + SHA-256 (AT-05)
- [ ] PR 05 IDs + errors + RunConfig
- [ ] PR 06 ArtifactStore + MinIO (AT-04)
- [ ] PR 07 EventStore + Postgres (AT-01..AT-03)
- [ ] PR 08 Kernel/domain events + envelopes
- [ ] PR 09 Context snapshots as artifacts
- [ ] PR 10 Executor + resume + orphan handling (AT-07)
- [ ] PR 11 LiveIo + facts
- [ ] PR 12 ReplayIo semantics
- [ ] PR 13 SDK operations + pipeline
- [ ] PR 14 Proof op end-to-end (AT-06..AT-08)
- [ ] PR 15 CLI run commands + keep keystore
- [ ] PR 16 Secrets guardrails (AT-09)
- [ ] PR 17 Remove legacy machine crates

