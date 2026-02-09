# MFM — REDESIGN.md Implementation Plan

> This document turns `REDESIGN.md` into an executable, PR-sized checklist.
> If this plan disagrees with `REDESIGN.md`, `REDESIGN.md` wins.
>
> Last updated: 2026-02-09

## Goals (Success Criteria)

- Implement the `REDESIGN.md` core invariants:
  - append-only per-run event streams
  - transactional state transitions at the event-store layer
  - content-addressed artifacts (manifest, context snapshots, fact payloads, outputs)
  - canonical JSON hashing for structured hashed data
  - explicit IO abstraction (live vs replay), missing facts are structured errors
  - kernel events are sufficient for recovery/resume correctness
  - no secrets in events/manifests/artifacts by default
- Restructure workspace to `crates/` + `bin/` and align Cargo naming policy (§2.6).
- Provide a thin CLI surface that can start/resume runs and inspect events/artifacts with stable
  `--output-format` / `MFM_OUTPUT_FORMAT` behavior.
- Add design-level tests from §15:
  - live-then-replay determinism
  - crash/resume determinism
  - optimistic concurrency for event appends

## Non-Goals (For Now)

- ClickHouse projections/indexers (explicitly deferred by `REDESIGN.md` §12.3/§16).
- Production hardening of REST API (auth, multi-tenant, quotas).
- A full suite of domain ops/collectors (we start with 1 minimal op crate to prove the model).

## Current Repo Reality (Baseline)

- Workspace members today: `mfm_cli`, `mfm_machine`, `mfm_machine_derive`, `mfm_core`.
- Current state machine is a recoverable, tag-based scheduler with `SafeContext` snapshots.
- `mfm_core` currently depends on `mfm_machine` only to re-export `SafeContext` (must be removed per
  redesign boundary rules: `core` must not depend on `machine`).

## Decision Defaults (Locked In Unless We Update This File)

- Workspace restructure happens early (create `crates/` + `bin/` first).
- Primary persistence backends:
  - Event store: PostgreSQL
  - Artifact store: MinIO/S3
- Keep a fast unit-test lane implementation (in-memory / filesystem) even if the default dev story
  is Postgres+MinIO, to keep `cargo nextest` fast and hermetic.
- Canonical JSON: RFC 8785 (JCS) semantics (likely `serde_jcs`).
- Hashing: SHA-256 (lowercase hex string in `ArtifactId`).
- Plan IDs follow §10.7: `StateId = <machine_id>.<step_id>.<state_local_id>` (stable, no random
  nonces).

## CI Parity Commands (Required Before Each PR Is “Done”)

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

# Phase 0 — Workspace and Naming

## PR 01 — Create `crates/` + `bin/` (Move Only)

**Intent:** Restructure without changing behavior.

### Steps

1. Add directories: `crates/`, `bin/`.
2. Move existing crates:
   - `mfm_core` → `crates/core`
   - `mfm_machine` → `crates/machine_legacy` (temporary)
   - `mfm_machine_derive` → `crates/machine-derive_legacy` (temporary)
   - `mfm_cli` → `bin/cli_legacy` (temporary)
3. Update root `Cargo.toml` workspace `members` paths.
4. Update all `path = "../..."` dependencies.
5. Update `flake.nix` to build the legacy CLI at its new path.

### Acceptance

- All CI parity commands pass.
- `nix run .#... -- --help` still works (using the legacy CLI app target).

## PR 02 — Cargo Package Naming Policy (Hyphens)

**Intent:** Align packages with `REDESIGN.md` §2.6 while legacy code still exists.

### Steps

- Rename packages:
  - `crates/machine_legacy` → package name `mfm-machine-legacy`
  - `crates/machine-derive_legacy` → package name `mfm-machine-derive-legacy`
  - `crates/core` → package name `mfm-core` (Rust import stays `mfm_core`)
  - `bin/cli_legacy` → package name `mfm-cli-legacy` (if desired) but keep binary name stable
    until replacement.
- Fix all workspace dependency references.

### Acceptance

- All CI parity commands pass.

## PR 03 — Remove `core -> machine` Dependency

**Intent:** Enforce boundary rule: `core` must not depend on `machine` (REDESIGN §5.2).

### Steps

1. Remove `mfm_machine` dependency from `crates/core/Cargo.toml`.
2. Remove `SafeContext` re-export from `crates/core/src/lib.rs`.
3. Update any call sites (likely few) to import context directly from legacy machine (temporary)
   or migrate them to the new machine later.

### Acceptance

- All CI parity commands pass.

---

# Phase 1 — New `mfm-machine` Public Contract (Appendix C)

## PR 04 — Add `crates/machine` (Types + Traits Only)

**Intent:** Introduce the redesign’s stable public surface without committing to implementation
details yet.

### Steps

1. Create `crates/machine` (package name `mfm-machine`).
2. Implement modules from Appendix C (public contract):
   - `ids`, `canonical` (marker), `config`, `meta`, `errors`, `context`, `events`, `io`,
     `recorder`, `state`, `plan`, `stores`, `engine`
3. Add minimal compile/serde tests.

### Acceptance

- All CI parity commands pass.
- No other crate is forced to use this new API yet.

## PR 05 — Canonical JSON + SHA-256 Hash Helpers (Internal)

**Intent:** Provide a single correct hashing pipeline for manifests/snapshots/events.

### Steps

1. Add internal helpers in `crates/machine` (not part of Appendix C contract):
   - `canonical_json_bytes(value) -> Vec<u8>`
   - `artifact_id_for_bytes(bytes) -> ArtifactId` (SHA-256, lowercase hex)
2. Add tests:
   - object key order does not change canonical bytes/hash
   - stable hash for equivalent structures

### Acceptance

- All CI parity commands pass.

## PR 06 — Add `crates/machine-derive` (New Macros)

**Intent:** Replace legacy metadata boilerplate with macros aligned to new `StateMeta`.

### Steps

1. Create `crates/machine-derive` (package name `mfm-machine-derive`).
2. Implement macro(s) aligned to Appendix C `StateMeta` fields.
3. Add `trybuild` compile-fail tests for invalid args.

### Acceptance

- All CI parity commands pass.

---

# Phase 2 — Storage Backends (Postgres + MinIO/S3)

## PR 07 — Storage Traits + “Fast Lane” Local Implementations

**Intent:** Unblock runtime tests without requiring services.

### Steps

1. Add `crates/storages/event-store` (package `mfm-event-store`):
   - `InMemoryEventStore` implementing `mfm_machine::stores::EventStore`
2. Add `crates/storages/artifact-store` (package `mfm-artifact-store`):
   - `FsArtifactStore` writing by content hash under a root dir
3. Tests:
   - optimistic concurrency behavior (expected seq)
   - artifact de-dupe by content hash

### Acceptance

- All CI parity commands pass without external services.

## PR 08 — Postgres `EventStore` Implementation

**Intent:** Implement the primary transactional event store.

### Postgres Schema (Locked In)

- Table `run_events`:
  - `run_id uuid not null`
  - `seq bigint not null`
  - `ts_millis bigint null`
  - `event_json jsonb not null` (serialized `EventEnvelope`)
  - primary key `(run_id, seq)`

### Append Semantics (Locked In)

- `append(run_id, expected_seq, events)` must be one SQL transaction:
  - check current head seq equals `expected_seq`
  - insert `N` events as seq `expected_seq+1..expected_seq+N`
  - commit
- Wrong `expected_seq` returns `StorageError::Concurrency`.

### Steps

1. Implement using `sqlx` + tokio.
2. Add integration tests gated behind env (e.g. `MFM_TEST_PG_URL`).

### Acceptance

- Fast lane still passes without Postgres.
- Integration lane passes when env is provided.

## PR 09 — MinIO/S3 `ArtifactStore` Implementation

**Intent:** Implement primary artifact storage.

### Steps

1. Implement using `aws-sdk-s3` (configure endpoint for MinIO).
2. Store objects keyed by `ArtifactId` (content hash).
3. Add integration tests gated behind env (e.g. `MFM_TEST_S3_ENDPOINT`, bucket, credentials).

### Acceptance

- Fast lane still passes without MinIO.
- Integration lane passes when env is provided.

---

# Phase 3 — Execution Engine + Kernel Events

## PR 10 — Engine Skeleton + Kernel Events Emission

**Intent:** Make `start` / `resume` real, with kernel events sufficient for recovery.

### Locked-In Engine Semantics

- Each state attempt is transactional with respect to:
  - context updates (staged; commit on success only)
  - event append (entered + domain + completed/failed in a single append)
- Artifacts referenced by events must be written before emitting the referencing event(s).

### Steps

1. Add `DefaultEngine` implementing `ExecutionEngine`.
2. Implement `start()`:
   - store manifest artifact
   - store initial snapshot artifact
   - append `RunStarted`
   - execute plan sequentially (dependency edges)
3. Implement `resume()`:
   - read kernel events, find last durable boundary, resume incomplete states deterministically
4. Add `EventRecorder` implementation that buffers domain events per state attempt.

### Acceptance

- Unit tests covering kernel event ordering and transactional semantics.

## PR 11 — Context Snapshotting (Full Snapshots)

**Intent:** Ensure full-snapshot artifacts are created and referenced by kernel events.

### Steps

1. Provide a `DynContext` implementation suitable for staging:
   - base context loaded from snapshot
   - staged writes tracked in-memory
   - on commit: materialize full `dump()` and store snapshot artifact
2. Tests:
   - staged writes discarded on `StateFailed`
   - committed snapshots reproduce full context

### Acceptance

- All CI parity commands pass.

---

# Phase 4 — IO Provider + Replay Determinism

## PR 12 — `LiveIo` + Fact Recording Domain Events

**Intent:** Make IO explicit; record facts as artifacts referenced by events.

### Steps

1. Implement `LiveIo`:
   - `call(IoCall)` uses a transport registry (start with a test transport)
   - when `fact_key` is present:
     - store response as `ArtifactKind::FactPayload`
     - emit domain event `FactRecorded { key, payload_id, meta }`
2. Add tests with a fake transport:
   - Live call records payload and emits event

### Acceptance

- All CI parity commands pass.

## PR 13 — `ReplayIo` + Missing Fact Semantics

**Intent:** Replay uses recorded facts/artifacts; missing facts return structured error.

### Steps

1. Implement `ReplayIo`:
   - `call(IoCall)` requires `fact_key` for replay-sensitive calls; otherwise treat as error or
     as “non-replayable” depending on policy (explicit in code).
   - lookup recorded fact from run’s domain events.
   - missing returns `IoError::MissingFact { key, info }`
2. Add determinism test:
   - live run records fact
   - replay run produces identical derived outputs without transport access

### Acceptance

- All CI parity commands pass.

---

# Phase 5 — `mfm-sdk` Planning + First Op

## PR 14 — Add `crates/sdk` (Operation + Pipeline)

**Intent:** Move planning ergonomics out of `machine` per boundary rules.

### Steps

1. Add `crates/sdk` (package `mfm-sdk`) containing:
   - `Operation::expand(...) -> StateGraph`
   - `Pipeline::then(...)` and `build() -> ExecutionPlan`
2. Implement ID assignment per §10.7:
   - `OpPath = <machine_id>.<step_id>`
   - `StateId = <machine_id>.<step_id>.<state_local_id>`
3. Add plan validation tests (cycles, duplicates, missing nodes).

### Acceptance

- All CI parity commands pass.

## PR 15 — Add `crates/ops/<minimal-op>` End-to-End

**Intent:** Prove the model with one runnable op crate.

### Steps

1. Add `crates/ops/hello-run` (or similar) that expands into:
   - `config` → `fetch_data` (records fact) → `report` (writes output artifact)
2. Add §15 tests:
   - live then replay produces identical output artifact IDs
   - crash after N events, resume, completes consistently

### Acceptance

- All CI parity commands pass (replay/crash tests run in fast lane).

---

# Phase 6 — Replace CLI With Thin Orchestrator

## PR 16 — New `bin/cli` (Run Start/Resume/Inspect)

**Intent:** Make binaries thin wrappers over sdk+machine+storages.

### Commands (Minimum Set)

- `run start --op <id> --input <json> [--io-mode live|replay]`
- `run resume --run-id <uuid>`
- `run events --run-id <uuid> [--from <seq>]`
- `artifact get --id <hash>`

### Output Contract

- Preserve global `--output-format` and `MFM_OUTPUT_FORMAT` semantics (current CLI contract).
- Stable JSON schema for success/errors; no secrets in outputs.

### Acceptance

- CLI e2e tests for the new commands.
- Legacy keystore commands remain available (temporarily) or are migrated to separate subcommands
  without touching event store.

## PR 17 — Deprecate/Remove Legacy CLI

- Remove `bin/cli_legacy` after new CLI is the default.
- Update `flake.nix` to build and `nix run` the new CLI.

---

# Phase 7 — REST API Scaffold

## PR 18 — `bin/rest-api` Minimal Surface

- Endpoints mirroring CLI surfaces:
  - start run, resume run, list/read events, get artifact
- No auth; explicitly non-production.

---

# Phase 8 — CI Integration Lane (Postgres + MinIO)

## PR 19 — GitHub Actions Services for Postgres + MinIO

- Add a CI job that runs integration tests with:
  - Postgres service container
  - MinIO service container
- Gate integration tests behind env vars so fast lane remains unchanged.

---

# Phase 9 — Remove Legacy State Machine

## PR 20 — Delete Legacy Machine + Derive

Prerequisites:
- No crate depends on `mfm-machine-legacy` or `mfm-machine-derive-legacy`.
- All ops/CLI use new `mfm-machine` + stores + sdk.

Steps:
- Remove legacy crates from workspace.
- Remove legacy docs/tests referencing them.

---

## Review Checklist (For Our Next Pass)

- Confirm crate naming and binary naming we want long-term (`mfm` vs `mfm_cli`).
- Confirm canonical JSON crate choice (JCS semantics + test coverage).
- Confirm Postgres schema migrations approach (`sqlx::migrate!` vs hand-rolled).
- Confirm how `ReplayIo` discovers facts (event scan vs index).
- Confirm how “crash simulation” is implemented in tests (controlled failpoint vs harness).

