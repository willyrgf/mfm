# MFM — `REDESIGN.md` Implementation Plan (Milestone 5)

> Source of truth: `REDESIGN.md` (v4, last updated 2026-02-09).
> Generated: 2026-02-10
>
> Goal: implement nested/parallel execution via child runs (auditable + replayable), without interleaving
> concurrent state execution in a single event stream.

---

## Summary

Milestone 5 introduces child runs:
- a parent state can spawn child runs (each child has its own manifest + event stream + artifacts)
- parent records explicit linkage events so audit/replay can follow the structure
- replay mode does not spawn; it validates and consumes recorded linkage and artifacts

---

## Locked Decisions

- Parallelism is expressed as multiple child runs + a deterministic join, not parallel state execution within one run.
- Parent/child linkage is represented as domain events (kernel event set stays minimal).
- Join ordering is deterministic: sort by `child_run_id` bytes before merging results.

---

## Deliverables (Step-by-Step)

### M5-01 — Parent/child linkage model

Tasks:
- Add domain event payloads (if not already present) for:
  - `ChildRunSpawned { parent_run_id, child_run_id, child_manifest_id }`
  - `ChildRunCompleted { child_run_id, status, final_snapshot_id? }`
- Ensure events never embed large blobs; only IDs and small metadata.

Acceptance:
- Unit tests: event payloads serialize canonically and contain no secrets.

### M5-02 — Engine-managed child run execution

Tasks:
- Add an SDK helper to spawn and await child runs from a state handler (live mode only).
- Record linkage events.
- In replay mode, forbid spawning and instead read recorded outcomes.

Acceptance:
- Integration test:
  - spawn 2 child `proof` runs
  - join results into parent context
  - live→replay determinism across the whole tree

### M5-03 — Crash/resume semantics for child runs

Tasks:
- Ensure parent resume handles:
  - “spawned but not completed”
  - “completed but parent didn’t record join”
in a deterministic and auditable way.

Acceptance:
- Crash/resume tests for each scenario.

---

## CI / Done Definition

Milestone 5 is “done” when:
- child runs provide practical parallelism while preserving auditability and replay semantics
- deterministic join behavior is tested and stable

