# MFM — REDESIGN_PLAN_v2 Critic (v2)

> Scope: Evaluate `REDESIGN_PLAN_v2.md` for quality and alignment with `REDESIGN.md`.
> Date: 2026-02-09
>
> Contract rule reminder: `REDESIGN.md` is the semantic source of truth. If the plan tightens or changes semantics,
> it must either (1) update `REDESIGN.md` explicitly, or (2) record a deviation (no silent drift).

---

## Executive Summary

`REDESIGN_PLAN_v2.md` is generally strong: it translates the redesign contract into PR-sized work units, ties work to
named acceptance tests (AT-01..AT-09), and correctly separates a fast lane (SQLite + FS) from an integration lane
(Postgres + MinIO/S3). It is broadly aligned with the core redesign goals: append-only events, transactional appends,
content-addressed artifacts, deterministic replay, and strict secret hygiene.

The main alignment problems are not in “what” the plan builds, but in “what semantics are declared where”:

* The plan introduces or strengthens several semantic rules (S1/S2/S3 + ID regex constraints) that are not fully
  reflected in `REDESIGN.md` and are not consistently tracked as deviations.
* D-01 (state transition atomicity) is correctly identified, but PR 00 must update more than `REDESIGN.md §6.4` to
  remove all contradictory contract language (notably Appendix C’s `EventRecorder` contract comment).
* `REDESIGN.md` describes nested machines as first-class (and includes `ChildRunSpawned`), but the plan never schedules
  implementing child-run spawning/linkage, nor marks it as explicitly deferred.

---

## High-Impact Misalignments / Gaps

### 1) D-01 is bigger than `REDESIGN.md §6.4`

`REDESIGN_PLAN_v2.md` S1 allows a single state attempt to span multiple transactional `EventStore::append(...)` calls.
This is a reasonable design for crash-safe fact recording, but it contradicts more than one place in `REDESIGN.md`:

* `REDESIGN.md §6.4` currently claims state transition event emission must be atomic.
* `REDESIGN.md Appendix C / recorder` says:
  “Domain events emitted within a state must be committed atomically with the state transition.”

Plan implication:
* PR 00 must update `REDESIGN.md` in all relevant locations (not only §6.4) so the contract and plan match end-to-end.

#### Alternative Resolution (Proposed): Deterministic Rewind Across Completed Transitions

If the desired semantics are “single atomic append per state transition” *and* strong determinism, another (larger)
contract change is to make **rewinds first-class**:

* On certain failures (typically “impure” / side-effecting states), the engine may **rewind the run’s effective
  checkpoint** to an earlier dependency boundary, then re-execute subsequent states.
* “Rewind” does **not** delete history. The stream stays append-only; earlier `StateCompleted` events remain, but become
  **superseded** for “current state” derivation after a rewind marker.

This idea is hinted at by the existing contract types:
* `meta::standard_tags::{IMPURE, APPLY_SIDE_EFFECT}` and `meta::SideEffectKind` exist, but have no defined rewind
  semantics today (`REDESIGN.md Appendix C`).
* `StateMeta.depends_on` / `DependencyStrategy` exist, but are currently described as authoring-time hints and not part
  of execution correctness (`REDESIGN.md Appendix C / meta`).

To make deterministic rewinds real, the redesign contract would need to declare (at minimum):

* **A kernel rewind marker** (new kernel event) that makes the rewind auditable and makes resume deterministic.
  For example:
  - `KernelEvent::RunRewound { to_state_id, to_snapshot_id, reason }`
  - optionally include an `epoch`/`generation` counter to disambiguate “current” vs “superseded” completions.
* **A deterministic rule for selecting the rewind target**, likely based on:
  - explicit plan edges (ancestor dependencies),
  - plus `StateMeta.side_effects` / `IMPURE` tags,
  - plus optional `depends_on` tag hints and `depends_on_strategy` (e.g., default to `LatestSuccessful` among eligible
    candidates).
  - if rewinds depend on plan metadata/tags, that metadata must be stable for a given run (e.g., plan artifact hashed
    and referenced by the manifest), otherwise “deterministic rewind” becomes version-dependent.
* **A deterministic projection algorithm** for “current run state”:
  - given an append-only stream containing rewinds, define exactly which `StateCompleted` snapshot is the active
    resume boundary, and which later state completions are ignored as superseded.
  - explicitly define whether the active `base_snapshot_id`/checkpoint pointer is allowed to move backward (it is, if
    “rewind” is a real semantic) and how that affects subsequent `StateEntered.base_snapshot_id` values.
  - update AT-01 language (“replay ignores nothing except orphan handling”) to include “superseded segments” semantics
    if this model is adopted.
  - clarify whether `StateEntered` is emitted at attempt start (requires multi-append) or can remain “end-of-attempt”
    metadata in a single-append-per-transition model.
* **Idempotency + replay interaction**:
  - rewinds imply re-execution; therefore, side-effecting states must be provably idempotent (via
    `StateMeta.idempotency` and/or external-system idempotency),
  - re-executed READ_ONLY_IO must be replay-safe: either facts are already recorded and deduped by `FactKey`, or the
    IO is deterministic given earlier recorded inputs (e.g., pinned block hash/height) such that refetching produces the
    same bytes,
  - and replay semantics must define whether facts recorded in superseded segments remain valid (recommended: facts are
    run-scoped and remain valid; everything else is audit-only unless explicitly projected).

One concrete policy variant (as described in discussion) is:
* Always persist a diagnostic snapshot on failure (`KernelEvent::StateFailed.failure_snapshot_id`), even if it will not
  be used as a resume boundary.
* If the failing state is tagged `IMPURE` (or otherwise classified as non-determinism-sensitive), **rewind to a
  dependency state**:
  - default: the latest successful ancestor state that is not `IMPURE`
  - optional: constrain/choose candidates using `StateMeta.depends_on` tags + `depends_on_strategy`
* Resume execution from the rewound snapshot in a new epoch/generation.

Decision note:
* If the repo adopts “deterministic rewinds” as the D-01 resolution, then the current plan’s S1 framing becomes a
  competing alternative. The plan and contract must be updated together (no silent drift).

Plan implication (if this alternative is chosen):
* `REDESIGN_PLAN_v2.md` must either (A) adopt this rewind model explicitly (new kernel event + tests), or (B) reject it
  and keep S1. It is not safe to leave “rewind across completed states” implicit because it changes how replay/resume
  derives the current checkpoint and plan progression.

### 2) Replay `fact_key` policy is stricter than the contract

`REDESIGN_PLAN_v2.md` S3 says that in Replay mode, IO that affects determinism MUST supply a `fact_key`; missing
`fact_key` yields a structured, non-retryable error by default.

`REDESIGN.md §8.1` currently says IO calls that influence deterministic behavior “SHOULD use `fact_key`”.

Plan implication:
* Either update `REDESIGN.md §8` to “MUST (in Replay mode)” (recommended if you want strong determinism guarantees),
  or add a deviation entry for this stricter policy (and keep `REDESIGN.md` as-is).

### 3) FactKey single-assignment (S2) is a new semantic, not a restatement

`REDESIGN_PLAN_v2.md` S2 defines per-run `FactKey` as single-assignment (“first write wins forever”), and makes this
the basis for deterministic retries/crash recovery.

`REDESIGN.md` treats facts as first-class but does not define overwrite/versioning rules.

Plan implication:
* Decide whether single-assignment is part of the redesign contract (likely yes for Milestone 1). If yes, add it to
  `REDESIGN.md` (probably in §7.3 and/or §8). If no, track it as a deviation.

### 4) Nested machines are in the contract, but missing from the plan

`REDESIGN.md §1.6` and §11 explicitly support nested machines, and Appendix C defines `ChildRunSpawned`.
`REDESIGN_PLAN_v2.md` focuses on flattened execution and does not schedule:

* a `spawn_child_run(...)` capability in the engine/sdk
* event emission for child-run linkage
* any acceptance tests covering nested runs

Plan implication:
* Either (A) add a PR for minimal nested-run spawning + `ChildRunSpawned` emission, or (B) explicitly defer it in the
  plan and note that Milestone 1 covers flattened composition only.

### 5) ID regex policy vs examples in `REDESIGN.md`

The plan adopts a strict segment regex for machine/step/state IDs and forbids dots inside segments. This is good for
validation and operability, but `REDESIGN.md Appendix C` includes examples like `pipeline[0].aave_tracker` that would
violate the plan’s policy.

Plan implication:
* Update `REDESIGN.md` examples and/or tighten the contract to match the enforced rules. Otherwise, authors will
  follow examples and fail validation.

### 6) Plan metadata mismatch: referenced critic file does not exist

`REDESIGN_PLAN_v2.md` claims it integrates `REDESIGN_CRITIC_v2.md`. Ensure this file exists (this document) and is
kept in sync with plan updates.

Plan implication:
* When `REDESIGN_PLAN_v2.md` changes materially, update `REDESIGN_CRITIC_v2.md` in the same PR to prevent drift.

---

## Non-PR Sections (Quality + Alignment)

### Goals + Acceptance Tests (AT-01..AT-09)

Strong and well aligned with `REDESIGN.md §15`:

* Append-only event streams (AT-01)
* Atomic append visibility + optimistic concurrency (AT-02, AT-03)
* Content-addressed artifacts + corruption detection (AT-04)
* Canonical JSON hashing (AT-05)
* Live→Replay determinism + crash/resume determinism + side-effect idempotency (AT-06..AT-08)
* Secrets never persisted (AT-09)

The explicit “Delivery Map” is a real strength: it prevents semantics from becoming “everyone’s responsibility” and
therefore nobody’s.

### Fast Lane vs Integration Lane

Aligned with `REDESIGN.md §12.3`. Good call that correctness must be proven in the fast lane and that integration
stores cannot be the only place semantics are tested.

### Deviations Discipline

The deviations section is good practice, but it is currently under-inclusive. At minimum, S2 and S3 represent
contract-tightening semantics that should either be added to `REDESIGN.md` or tracked explicitly as deviations.

---

## Per-PR Critique (Alignment to `REDESIGN.md`)

### PR 00 — Resolve D-01 (Align `REDESIGN.md` §6.4 With S1)

Good and necessary. Ensure the scope includes all conflicting language, especially:

* `REDESIGN.md Appendix C / recorder` contract comment
* any other “atomic transition emission” statements outside §6.4

If the chosen model is “attempt envelopes with multiple appends,” then the contract should clearly define what is
atomic (each append) and what is eventual (attempt completion marker).

### PR 01 — Create `crates/` + `bin/` (Move Only)

Well aligned with `REDESIGN.md §4`. Keep it mechanically move-only; avoid accidental behavior changes while fixing
paths in `Cargo.toml`/Nix/CI.

### PR 02 — Cargo Package Naming Policy (Hyphens)

Aligned with `REDESIGN.md §2.6`. The suggested split to reduce diff noise is pragmatic. Keeping the binary name stable
while renaming packages is the right way to avoid breaking external scripts.

### PR 03 — Remove `core -> machine` Dependency

Directly aligned with `REDESIGN.md §5.2` boundary rules. The “boundary guardrail” is a strong idea; implement it in a
way that is hard to accidentally bypass (for example, checking resolved dependency graphs, not only `Cargo.toml`).

### PR 04a — Add `crates/machine` (Foundational Types)

Aligned with `REDESIGN.md Appendix C`. Good to land ID validation early.

One key alignment requirement:
* The enforced ID constraints must match the contract text and examples, or the contract must be updated.

### PR 04b — Runtime Abstractions (Context, Events, IO, Recorder)

Mostly aligned with Appendix C.

Potential semantic tightening to track:
* Plan proposes validation that domain payloads are canonical-hashable and forbids floats. `REDESIGN.md` says “avoid
  floats,” but not “reject floats.” If this becomes enforcement, the contract should say so explicitly.

### PR 04c — Planning + Stores + Engine Traits

Aligned with Appendix C type/trait surface and the incremental migration strategy in `REDESIGN.md §16`.

The “no bridging in Milestone 1” choice is fine, but should be clearly reflected in migration guidance so no one
expects interoperability between legacy/new machines.

### PR 05 — Canonical JSON + SHA-256 Hash Helpers

Aligned with `REDESIGN.md §2.1` and §7.2. The proposed tests are good.

Addendum:
* Prefer importing known-good JCS (RFC 8785) test vectors to reduce the risk that the chosen crate’s behavior differs
  from the contract in edge cases.

### PR 06 — New derive macros (Deferred)

Deferring is reasonable. If `mfm-machine` depends on derive in the long run (per dependency graph), keep it optional
until ergonomics are proven; don’t force proc-macro adoption in the semantics milestone.

### PR 07 — Storage Implementations (Fast Lane)

Strong alignment with `REDESIGN.md §12.3` and AT-02/03/04. Good details on SQLite atomicity + WAL and on artifact
store atomic write + hash-on-read verification.

Potential scope risk:
* Combining event-store + artifact-store in one PR might be large. Consider splitting if reviewability suffers.

### PR 08 — Postgres `EventStore`

Aligned with `REDESIGN.md §12.3` (Postgres primary event store). Advisory locks + expected-seq semantics align with
AT-03.

Operational note:
* Scanning events for `run list/status` is acceptable early, but consider adding minimal indexes if it becomes slow.

### PR 09 — MinIO/S3 `ArtifactStore`

Aligned with `REDESIGN.md §12.3` and §12.4. Streaming hash-on-read and a size limit are good security/DoS controls.

### PR 10 — Engine Skeleton + Kernel Events

This is the semantic core and aligns with `REDESIGN.md §6` and §9 if D-01 is resolved first.

Risks / suggestions:
* This PR is large as described (start + resume + failpoints + recorder). Consider splitting to keep it reviewable.
* Ensure `RunCompleted` emission is explicitly included and tested; it is a kernel event in `REDESIGN.md §6.1`.

### PR 10b — Engine Policy Semantics (Attempts, Retry, Skip-Tags, Event Profile)

Aligned with `REDESIGN.md §8.3` and Appendix C `RunConfig`.

Skip-tags semantics are sensible and auditable (still emit `StateEntered` + `StateCompleted`). One thing to decide:
* Whether “skip” should also emit a stable domain event name/type that is part of the public CLI/API surface.

### PR 11 — Context Snapshotting (Full Snapshots, Staging, No Secrets)

Aligned with `REDESIGN.md §2.2` (full snapshots) and §6.4 (transactional context), and §14 (secret handling).

Important caveat:
* Namespace-based “secret.*” heuristics are not sufficient. The real enforcement must come from type design and tests.

### PR 12 — `LiveIo` + Fact Recording

Aligned with `REDESIGN.md §8` (facts) and the reproducibility model. The crash window test is especially important.

Semantic clarifications to reflect in the contract:
* The plan says “if IO may include secrets, it MUST NOT be recorded as a fact.” `REDESIGN.md §12.4` allows encrypted
  artifacts “if unavoidable.” Decide which policy is the contract for Milestone 1.

### PR 13 — `ReplayIo` + Missing Fact Semantics + Fact Index

Aligned with `REDESIGN.md §8.2` missing fact semantics and determinism goals.

But it tightens semantics:
* Missing `fact_key` in Replay mode becomes an error (S3). This must be reflected in `REDESIGN.md` or tracked as a
  deviation.

### PR 14 — `mfm-sdk` Planning + Operation/Pipeline

Strong alignment with `REDESIGN.md §5.3` (machine vs sdk ownership) and §10.7 ID assignment.

Again: the plan’s enforced ID constraints must match the contract and examples.

### PR 15 — Proof Op End-to-End (Includes Side-Effect Idempotency)

Highly aligned with `REDESIGN.md §15` testing requirements and §9.3 idempotency.

Suggestion:
* Consider exercising `now_millis`/`random_bytes` fact recording in at least one test, since Appendix C exposes these
  methods and they are common determinism footguns.

### PR 16 — New Thin CLI (Run Start/Resume/Inspect)

Aligned with `REDESIGN.md §13.1` and the repo’s “stable output format” contract. Good that it calls out JSON schema
stability tests and “no secrets in outputs.”

Scope caution:
* Consider landing “run start/resume/events/artifact get” first before adding list/status/plan inspect to keep the PR
  reviewable.

### PR 17 — Deprecate/Remove Legacy CLI

Aligned with `REDESIGN.md §16` migration. Good to require a migration guide and update Nix to run the new CLI.

### PR 18 — REST API Scaffold (Deferred)

Aligned with `REDESIGN.md §13.2` and correctly marked optional/deferred for the semantics milestone.

### PR 19 — CI Integration Lane (Postgres + MinIO)

Aligned with `REDESIGN.md §12.3`. Pinning versions and adding readiness checks is correct and prevents flaky CI.

### PR 20 — Delete Legacy Machine + Derive

Aligned with the migration plan. The prerequisites list is good and should prevent premature deletion.

---

## Recommended Plan Fixes (Most Important)

1. Resolve D-01 by:
   * adopt deterministic rewinds (new kernel rewind marker + projection semantics).
2. Expand PR 00 to update all conflicting contract language, including Appendix C `EventRecorder` semantics, not only
   `REDESIGN.md §6.4`.
3. Decide whether S2/S3 (FactKey single-assignment; Replay requires `fact_key`) are contract rules.
   If yes: update `REDESIGN.md` accordingly. If no: record deviations explicitly.
4. Resolve ID policy inconsistencies by updating `REDESIGN.md` examples (or relaxing the regex rule).
5. Either add nested-machine support to the plan (minimal `ChildRunSpawned` linkage), or explicitly defer it in the
   plan and note that Milestone 1 covers flattened composition only.
