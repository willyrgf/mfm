# MFM — REDESIGN.md Implementation Plan (v4.0)

> This document turns `REDESIGN.md` into an executable, PR-sized checklist.
> Contract rule: `REDESIGN.md` wins on semantics, but **any known plan ↔ contract deviation must be recorded**
> in §Deviations (no silent drift).
>
> Version: v4.0 (aligned to `REDESIGN_V4.md`; removes v3 drift items by integrating them into the contract)
> Last updated: 2026-02-09

---

## Goals (Success Criteria)

Implement the `REDESIGN.md` core invariants with explicit, testable semantics:

- Append-only per-run event streams
- Transactional appends at the event-store layer (each append is atomic; readers never observe partial appends)
- Attempt envelopes across one-or-more appends (`StateEntered … domain … terminal`)
- Content-addressed artifacts (manifest, context snapshots, fact payloads, outputs)
- Canonical JSON hashing for structured hashed data (RFC 8785 semantics)
- Explicit IO abstraction (live vs replay)
  - Replay requires `fact_key` for deterministic IO
  - Missing fact_key is a structured error (`missing_fact_key`)
  - Missing facts are structured errors (`MissingFact`)
- Kernel events sufficient for recovery/resume correctness
- **Secrets excluded from everything persisted**
- Thin CLI surfaces to start/resume runs and inspect events/artifacts with stable `--output-format`

### Acceptance Tests (Named)

Each invariant must be verifiable by at least one named acceptance test. PRs MUST reference these IDs.

- AT-01 AppendOnlyEventStream
- AT-02 AtomicAppendVisibility
- AT-03 ExpectedSeqConcurrency
- AT-04 ArtifactContentAddressing
- AT-05 CanonicalJsonHashing
- AT-06 LiveThenReplayDeterminism
- AT-07 CrashResumeDeterminism
- AT-08 SideEffectIdempotency
- AT-09 SecretsNeverPersisted

### Delivery Map

| Acceptance Test | Primary PR(s) |
|---|---|
| AT-01 | PR10 (+ PR07/PR08 stores) |
| AT-02 | PR07, PR08 |
| AT-03 | PR07, PR08 |
| AT-04 | PR07, PR09 |
| AT-05 | PR05, PR11 |
| AT-06 | PR12, PR13, PR15 |
| AT-07 | PR10, PR15 |
| AT-08 | PR15 |
| AT-09 | PR11, PR12, PR16 |

---

## Non-Goals (Milestone 1: Semantics Proof)

- REST API surface (optional/deferred)
- ClickHouse projections/indexers
- Artifact GC / compaction
- Manual operator interventions (DLQ, force-complete, compensations)
- Full suite of domain ops/collectors (start with one proof op)
- Postgres/MinIO are integration-lane only until fast-lane semantics are proven stable
- Nested machines / child-run spawning + linkage (deferred)
- Deterministic rewinds across completed transitions (deferred)
- Encrypted secret-bearing artifacts (deferred; Milestone 1 is “no secrets persisted”)

---

## Core Semantic Clarifications (v4)

### S1 — Attempt envelopes with per-append atomicity

A state attempt is a kernel envelope:

- `StateEntered { state_id, attempt, base_snapshot_id }`
- zero or more domain events
- exactly one terminal kernel event: `StateCompleted { … }` or `StateFailed { … }`

Rules:
1. Engine MUST append `StateEntered` before calling the handler.
2. Each `EventStore::append([...])` is atomic and never partially visible.
3. A single attempt MAY span multiple appends (required for crash-safe fact recording).
4. Only `StateCompleted { context_snapshot_id }` advances the checkpoint.

Orphan handling:
- If the stream ends with `StateEntered` and no terminal event, attempt is in-flight.
  Resume retries from `base_snapshot_id`.
- Facts recorded during in-flight attempts remain valid (run-scoped).
- Non-fact domain events are audit-only and MUST NOT advance progression/checkpoints.

### S2 — Facts are single-assignment (FactKey first durable wins)

Within a run:
- The first durable `FactRecorded { key, payload_id }` binds the key permanently.

LiveIo behavior when `fact_key` is present:
1. Check fact index for key
2. If exists → return recorded payload (no transport)
3. Else → perform IO, store payload artifact, append `FactRecorded` durably

### S3 — Replay requires `fact_key` for deterministic IO

In Replay mode:
- If `fact_key` is absent → return `IoError::MissingFactKey` with stable code `missing_fact_key` (non-retryable).
- If `fact_key` present but missing → return `IoError::MissingFact { key, … }`

Missing-fact retryability:
- Controlled by `RunConfig.replay_missing_fact_retryable` (default false).

### S4 — Secrets excluded from all persisted surfaces

Hard rule:
- secrets must never appear in manifests, events, artifacts (including fact payloads and snapshots), or error details.

### S5 — Side effects require idempotency

Any APPLY_SIDE_EFFECT state must:
- declare an idempotency key
- emit a durable domain event recording the key
- use an external idempotency mechanism keyed by that value
- ensure crash/resume does not re-apply

### S6 — Time/random must be recorded when used

`IoProvider::now_millis()` and `random_bytes()` are nondeterministic:
- LiveIo MUST record them as facts with deterministic, attempt-scoped keys.
- ReplayIo MUST replay them or return MissingFact.
- Proof op tests MUST exercise at least one of these in the replayable path.

---

## Decision Defaults (Milestone 1)

- Sequential per run
- `EventEnvelope.seq` is 1-indexed; empty run head is 0
- Fast lane: SQLite event store (file-backed) + filesystem artifact store
- Integration lane: Postgres + MinIO
- Canonical JSON: RFC 8785 (JCS); reject floats, NaN/Inf
- Hash: SHA-256; `ArtifactId` is 64-char lowercase hex
- IDs:
  - `OpPath = <machine_id>.<step_id>`
  - `StateId = <machine_id>.<step_id>.<state_local_id>`
  - segment regex `^[a-z][a-z0-9_]{0,62}$`
  - single-op runs use `step_id = main`
- Crash simulation: explicit failpoints (`fail` crate or equivalent), chosen before PR10.

---

## Deviations (Plan vs Contract)

None (this v4 plan is aligned to `REDESIGN_V4.md`).

---

## PR Dependency Graph (Critical Path)

```text
PR00 -> PR01 -> PR02 -> PR03

PR04a -> PR04b -> PR04c -> PR05
PR04c -> PR07 -> PR10 -> PR10b -> PR11 -> PR12 -> PR13 -> PR14 -> PR15 -> PR16 -> PR17 -> PR20

PR08, PR09 depend on PR04c + PR05 and SHOULD start only after PR10/PR11 prove fast-lane semantics.
PR19 depends on PR08 + PR09 and can run in parallel with PR14–PR17.

PR18 (REST) is deferred until after PR20.
````

---

## Phase -1 — Contract Alignment (Blocking)

### PR 00 — Apply `REDESIGN_V4.md` to `REDESIGN.md`

Intent:

* Replace/merge `REDESIGN.md` content so the repo’s contract matches v4 semantics:

  * attempt envelopes + orphan handling
  * FactKey single-assignment
  * Replay requires fact_key
  * ID regex + examples
  * Milestone 1 scope deferrals
  * Milestone 1 “no secrets persisted” policy

Acceptance:

* No drift items remain; plan Deviations stays “None”.

---

## Phase 1 — New `mfm-machine` Public Contract (Appendix C)

### PR 04a — Foundational Types

Scope:

* `ids`, `canonical`, `config`, `meta`, `errors`

v4-specific requirements:

* Add `RunConfig.replay_missing_fact_retryable: bool` (default false).
* Add `IoError::MissingFactKey` variant and ensure stable code `missing_fact_key`.

Tests:

* `ArtifactId` format: 64-char lowercase hex
* ID segment regex validation

### PR 04b — Runtime Abstractions

Scope:

* `context`, `events`, `io`, `recorder`

v4-specific requirements:

* KernelEvent fields must match v4 (`context_snapshot_id` required in `StateCompleted`).
* Recorder docs reflect multi-append attempts.

### PR 04c — Planning + Stores + Engine Traits

Scope:

* `state`, `plan`, `stores`, `engine`

---

## Phase 2 — Hashing Helpers

### PR 05 — Canonical JSON + SHA-256 helpers

* canonical bytes per RFC 8785 semantics
* reject floats / NaN / Inf in hashed structures
* include JCS/RFC test vectors where possible

---

## Phase 3 — Storage Backends

### PR 07 — Fast-lane stores (SQLite + FS)

v4-specific notes:

* `EventEnvelope.seq` is assigned by the engine; store MUST validate:

  * first seq == expected_seq+1
  * contiguous increments
* Artifact store MUST hash-on-read and return `Corruption` on mismatch

Tests:

* AT-02, AT-03, AT-04 supported in fast lane

### PR 08 — Postgres EventStore (integration lane)

### PR 09 — MinIO/S3 ArtifactStore (integration lane)

---

## Phase 4 — Execution Engine + Kernel Events

### PR 10 — Engine skeleton (attempt envelopes) + kernel emission

* Implements S1, orphan handling, resume logic
* Failpoints added for crash tests

### PR 10b — Policy semantics

* retry policy (no sleep in replay mode)
* skip-tags semantics
* event profile gating for domain events

---

## Phase 5 — Context Snapshotting

### PR 11 — Full snapshots + staging + secret exclusion

* staged writes commit only on StateCompleted
* failure discards staged writes
* snapshot determinism tests
* AT-09 coverage begins here

---

## Phase 6 — IO Providers

### PR 12 — LiveIo + fact recording + dedupe

* FactKey single-assignment enforced by scanning/building fact index
* Time/random recorded as facts in deterministic, attempt-scoped way (S6)

### PR 13 — ReplayIo + MissingFactKey/MissingFact semantics

* missing fact_key → `IoError::MissingFactKey` (`missing_fact_key`, non-retryable)
* missing fact → `IoError::MissingFact` with retryable based on `RunConfig.replay_missing_fact_retryable`

---

## Phase 7 — SDK + Proof Op

### PR 14 — SDK planning (Operation + Pipeline)

* enforce ID shape and default `step_id = main` for single-op runs

### PR 15 — Proof op end-to-end

Must include:

* fork/join DAG shape
* read-only IO facts
* idempotent side-effect state (tested)
* report output artifact

Tests:

1. Live → Replay determinism (skip side effects)
2. Crash/resume determinism
3. Side-effect idempotency across crash/resume
4. now/random fact recording exercised in replayable path (S6)

---

## Phase 8 — CLI replacement

### PR 16 — Thin CLI for start/resume/inspect

* stable `--output-format`
* no secrets in outputs/errors

---

## Phase 9 — Cleanup

### PR 20 — Delete legacy machine + derive

* all AT-01..AT-09 passing in fast lane

