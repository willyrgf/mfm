# MFM — Architecture (One‑Pager)

> This is the contributor-facing overview of the system.  
> The full design contract lives in **REDESIGN.md**.

---

## What MFM is

MFM is an **event-sourced execution engine** for on-chain/off-chain workflows where:

- Every workflow is a **state machine**
- Every run produces an **append-only event log**
- Every output is an **immutable, content-addressed artifact**
- Replay/recovery are first-class, not afterthoughts

---

## The core invariants

1) **Append-only**
- Runs append events; past events are never mutated.

2) **Per-append atomicity + attempt envelopes**
- Each `EventStore::append([...])` is atomic (all-or-nothing).
- A state attempt is delimited by `StateEntered … (StateCompleted|StateFailed)` and MAY span multiple appends.
- Only `StateCompleted { context_snapshot_id }` advances the run checkpoint; failed attempts discard staged writes.

3) **Content addressing**
- Manifests, snapshots, facts, and outputs are stored as artifacts identified by hash.

4) **Canonical JSON for hashing**
- Structured data is hashed as **canonical JSON**.
- Target semantics: RFC 8785 (JCS)-style canonicalization; no floats in hashed structures.

5) **No ambient IO**
- States do not perform network/FS IO directly.
- States use an IO abstraction that supports live and replay.

6) **No secrets persisted**
- Manifests, events, artifacts (including fact payloads and context snapshots), CLI/API outputs, and error details
  must never contain private keys, mnemonics, passwords, or decrypted buffers.

---

## The runtime model (mental picture)

A **Run** = (Manifest) + (Event Stream) + (Artifacts)

- **Manifest**: content-addressed description of inputs, configs, environment allowlist, modes.
- **Event Stream**: append-only log of state transitions + optional domain events.
- **Artifacts**: immutable blobs (snapshots, recorded facts payloads, outputs).

The executor can:
- start a run
- resume after crash
- replay deterministically (to the extent facts are recorded/available)

---

## Required kernel events (engine-level)

These events are always emitted to guarantee recovery/resume:

- `RunStarted { op_id, manifest_id, initial_snapshot_id }`
- `StateEntered { state_id, attempt, base_snapshot_id }`
- `StateCompleted { state_id, context_snapshot_id }`
- `StateFailed { state_id, error, failure_snapshot_id? }`
- `RunCompleted { status, final_snapshot_id? }`

Everything else is **operation-defined domain events** (facts, artifacts, boundaries, etc.).

---

## Replay behavior (in one sentence)

Replay doesn’t hard-fail on missing data:
- missing facts/IO return a structured error
- if tagged retryable → retry per run policy
- otherwise → deterministic failure

---

## Composition model

### Ops are expandable state graphs
An **Op** expands into a **StateGraph** (states + dependency edges) given:
- OpConfig (domain parameters)
- RunConfig (retry/replay policy, event profile, etc.)

### Flattening (pipelines)
A pipeline op can expand multiple ops into one plan:

- ops1 expands to N states
- ops2 expands to M states
- pipeline expands to K = N + M states

The engine executes K states in a single run with a shared context (namespaced).

### Nested machines
Nested/child runs are a supported concept, but **Milestone 1 uses flattened composition only**.
When introduced, parent-child linkage must be explicit in domain events.

At minimum:
- `ChildRunSpawned { child_run_id, child_manifest_id }`

---

## Where code should live (map of responsibilities)

Naming reminder:
- Paths/modules drop `mfm_` (e.g., `crates/machine/`, `crates/core/`).
- Cargo packages stay namespaced (`mfm-machine` → `mfm_machine`).

### `crates/machine/`
Owns the execution model:
- State trait + metadata (tags, IDs, dependencies)
- Context + snapshots (full snapshots)
- ExecutionPlan / StateGraph data structures
- Executor runtime (sequential; parallelism deferred for Milestone 1)
- Kernel event types (minimum for recovery)

Must NOT:
- contain chain-specific code
- contain concrete storage backends
- contain op registries / pipeline builders (belongs in `crates/sdk/`)

### `crates/machine-derive/`
Owns ergonomics at compile time:
- proc-macros for state metadata boilerplate
- compile-time validation with clear error messages

Must NOT:
- contain runtime behavior (no executor logic)
- depend on ops/collectors/storages

### `crates/core/`
Owns primitives and security-sensitive components:
- keystore + crypto utilities
- shared typed IDs/hashes helpers (if needed)
- config models (loaded as artifacts where appropriate)

Must NOT:
- depend on ops, collectors, or storages

### `crates/storages/*`
Owns persistence implementations:
- EventStore backends (e.g., postgres/local)
- ArtifactStore backends (e.g., minio/fs)
- Optional index/projection stores (deferred)

Must NOT:
- contain business logic (no op workflows)

### `crates/collectors/*`
Owns data collection/normalization (RPC/HTTP):
- must operate through the IO abstraction
- should be usable in live or replay

### `crates/ops/common/`
Owns reusable operation-level building blocks:
- reusable `State` implementations (for example shared fetch/validate/execute patterns)
- shared context/error/metadata helpers for ops

Must NOT:
- change `mfm-machine` runtime/planner semantics
- depend on binaries

### `crates/ops/*`
Owns domain workflows:
- defines ops (expand to graphs)
- composes collectors + storages + machine runtime
- reuses shared primitives from `crates/ops/common/` when possible
- contains op-specific states and tests

In practice:
- ops typically implement an `Operation` trait (recommended to live in `crates/sdk/`)

### `crates/sdk/` (optional but recommended)
Owns “glue” for binaries and integrations:
- `Operation` trait (or `Op` trait) + versioning conventions
- op registry
- pipeline builder convenience API
- run launcher / resume helpers (thin wrapper around machine + stores)

### `bin/cli/` and `bin/rest-api/`
Thin wrappers:
- parse requests
- start/resume runs via sdk
- render stable outputs

---

## Concurrency policy (simple + auditable)

- Parallel execution (including engine-managed child runs) is deferred.
- Any future parallelism MUST remain auditable + replayable (e.g., fan-out/join with deterministic ordering).

---

## Contributor checklist (high signal)

When adding a feature:
- Is it an op? Put it in `crates/ops/<name>/`.
- Does it fetch external data? Put it in `crates/collectors/` and route through IO.
- Is it persistence? Put it in `crates/storages/`.
- Is it keystore/crypto/primitives? Put it in `crates/core/`.
- Does it affect recovery/replay? Add tests:
  - crash/resume test
  - replay determinism test (live→replay)

Never print/log/persist secrets (including in errors, manifests, events, artifacts, and snapshots).

---

## Read next

- **REDESIGN.md** (full contract)
- **AGENTS.md** (repo contribution rules / CI parity)
