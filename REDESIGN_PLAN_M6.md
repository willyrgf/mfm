# MFM — `REDESIGN.md` Implementation Plan (Milestone 6)

> Source of truth: `REDESIGN.md` (v4, last updated 2026-02-09).
> Generated: 2026-02-10
>
> Goal: add projections/indexing and optional typed schema registry after the event model stabilizes
> under real ops and child runs.

---

## Summary

Milestone 6 adds:
- derived projections for run summaries / timelines
- optional typed schema registry stored as artifacts and referenced by manifests

---

## Locked Decisions

- Projections are derived-only: they never become correctness-critical for resume/replay.
- Projection ingestion consumes kernel events + selected domain events.
- Typed schemas, if introduced, are artifacts (content-addressed) referenced by manifest IDs.

---

## Deliverables (Step-by-Step)

### M6-01 — Projection store scaffold

Tasks:
- Add `crates/storages/indexer/` with:
  - schema migration tooling
  - ingestion pipeline interface

Acceptance:
- Opt-in parity lane build + smoke test.

### M6-02 — Minimal run summary projection

Tasks:
- Store per-run:
  - started/completed status
  - last seq
  - state timeline summary

Acceptance:
- Integration test validates summary matches the event stream.

### M6-03 — Optional typed schema registry

Tasks:
- Add schema artifacts, referenced from manifests.
- Add compatibility validation hooks in planner/runtime (non-blocking warnings first).

Acceptance:
- Tests ensure schemas are content-addressed and stable.

---

## CI / Done Definition

Milestone 6 is “done” when:
- projections provide useful queries without becoming a correctness dependency
- schema registry (if implemented) is artifact-based and reproducibility-friendly
