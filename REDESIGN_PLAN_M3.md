# MFM — `REDESIGN.md` Implementation Plan (Milestone 3)

> Source of truth: `REDESIGN.md` (v4, last updated 2026-02-09).
> Generated: 2026-02-10
>
> Goal: REST API parity with the CLI for starting/resuming/inspecting runs, with stable JSON contracts and
> Milestone 1 secret rules preserved.

---

## Summary

Milestone 3 introduces `bin/rest-api/`:
- start run
- resume run
- fetch events
- fetch artifacts (or signed URLs)
- observe progress via polling (SSE/websocket deferred)

The REST API must be a thin wrapper over `mfm-sdk` + `mfm-machine` + storages, matching CLI semantics.

---

## Locked Decisions

- API is versioned under `/v1`.
- Responses are JSON-only (no text mode).
- Error shape matches the CLI convention:
  - `{"status":"error","error":{"code":"...","message":"..."}}`
- No secrets in outputs or error details (same hard rule).

---

## Deliverables (Step-by-Step)

### M3-01 — Scaffold `bin/rest-api`

Tasks:
- Add `bin/rest-api/Cargo.toml` and minimal server entrypoint.
- Add routing, JSON serialization, and graceful shutdown.

Acceptance:
- `cargo build -p mfm-rest-api`

### M3-02 — Implement run control endpoints

Endpoints:
- `POST /v1/runs/start`
- `POST /v1/runs/{run_id}/resume`
- `GET /v1/runs/{run_id}/status`

Acceptance:
- Integration tests: start + resume + status for the `proof` op against fast-lane stores.

### M3-03 — Implement inspection endpoints

Endpoints:
- `GET /v1/runs/{run_id}/events?from_seq=&to_seq=`
- `GET /v1/artifacts/{artifact_id}`

Acceptance:
- Integration test: fetch the event stream and final snapshot artifact after a completed run.

### M3-04 — Parity lane (opt-in) backends

Tasks:
- Support Postgres event store + S3/MinIO artifact store via config.
- Keep service-free fast-lane tests as default.

Acceptance:
- Opt-in integration test suite passes with local services.

---

## CI / Done Definition

Milestone 3 is “done” when:
- REST API supports the same run-control surfaces as CLI
- contracts are tested (schema stability + smoke tests)
- no secret-bearing data appears in responses or errors

