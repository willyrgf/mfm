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

2) **Transactional transitions**
- A state transition is committed atomically (entered → domain events → completed/failed).

3) **Content addressing**
- Manifests, snapshots, facts, and outputs are stored as artifacts identified by hash.

4) **Canonical JSON for hashing**
- Structured data is hashed as **canonical JSON**.

5) **No ambient IO**
- States do not perform network/FS IO directly.
- States use an IO abstraction that supports live and replay.

6) **No secrets in events**
- Events must never contain private keys, mnemonics, passwords, or decrypted buffers.

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

- `RunStarted`
- `StateEntered`
- `StateCompleted`
- `StateFailed`
- `RunCompleted`

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

### Nested runs (for isolation / fan-out)
A state may spawn child runs (sub-machines). The parent machine does not need to know
the child’s internal states, but must record linkage events.

---

## Where code should live (map of responsibilities)

### `crates/machine/`
Owns the execution model:
- State trait + metadata (tags, IDs, dependencies)
- Context + snapshots (full snapshots)
- ExecutionPlan / StateGraph data structures
- Executor runtime (sequential default; opt-in fan-out/join patterns)
- Kernel event types (minimum for recovery)

Must NOT:
- contain chain-specific code
- contain concrete storage backends

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
- Optional index/projection stores (clickhouse)

Must NOT:
- contain business logic (no op workflows)

### `crates/collectors/*`
Owns data collection/normalization (RPC/HTTP):
- must operate through the IO abstraction
- should be usable in live or replay

### `crates/ops/*`
Owns domain workflows:
- defines ops (expand to graphs)
- composes collectors + storages + machine runtime
- contains op-specific states and tests

### `crates/sdk/` (optional but recommended)
Owns “glue” for binaries and integrations:
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

- Default execution is sequential.
- Ops that need parallelism should use **fan-out/join**:
  - spawn child runs per work item
  - join deterministically (stable ordering)
  - avoid concurrent writes to a shared context

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

Keep libraries free of printing/logging secrets.

---

## Read next

- **REDESIGN.md** (full contract)
- **AGENTS.md** (repo contribution rules / CI parity)

