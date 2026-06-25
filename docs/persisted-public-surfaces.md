# Persisted And Public Surfaces

This inventory is the review checklist for data that is persisted by MFM or returned by CLI/REST.
Every entry records whether the surface may contain secrets and what authority role it has.

Authority roles:

- `strict authority`: durable data that strict resume, replay, status, rendering, or retention proof
  may trust only after validating canonical bytes, hashes, identities, and bindings.
- `observation`: derived facts for list/watch or operator display. They can guide users, but cannot
  mint execution, replay, retention, artifact, or lane authority.
- `rebuildable cache`: data that is recomputable from strict authority rows and must be treated as
  stale or corrupt until reverified against those rows.
- `operational coordination`: mutable store-owned rows used to coordinate admission or process
  behavior. They are not replay, resume, side-effect, lane ownership, or public-output authority.
- `operational telemetry`: runtime metadata used for paging, readiness, diagnostics, or process
  operation. It is not semantic run authority.
- `public output`: data intentionally returned to users by CLI/REST. It must not contain secrets and
  is not a substitute for strict authority unless the command explicitly verifies strict rows first.

## Postgres Storage

| Surface | Location | Secret Boundary | Authority Role | Notes |
| --- | --- | --- | --- | --- |
| Artifact bytes | `artifact_blobs.bytes` | No secrets allowed. Secret-like bytes must stay below artifacts. | strict authority | Trusted only with matching `artifact_admissions` evidence, digest, byte length, role, and proof-bearing read request. |
| Artifact evidence | `artifact_admissions`, `evidence_canonical_json`, `evidence_hash` | No secrets allowed. Producer and schema ids are non-secret identities. | strict authority | Binds artifact id, digest, byte length, media type, role, schema/semantic ids, and producer identity. |
| Commit artifact bindings | `commit_artifact_evidence`, `run_artifact_admissions` | No secrets allowed. | strict authority | Binds required/admitted artifact evidence to one commit and one run. |
| Commit authority bytes | `commits.idempotency_canonical_json`, `prepared_authority_canonical_json`, `commit_batch_canonical_json` | No secrets allowed. | strict authority | Strict load revalidates canonicalizer identity, hash-domain version, commit id, idempotency hash, prepared authority hash, and final batch hash. |
| Event canonical bytes | `run_events.payload_canonical_json`, `payload_hash`, event identity columns | No secrets allowed. | strict authority | Strict load reconstructs typed envelopes and cross-checks run id, seq, ordinal, commit key, event id, schema id, spec hash, logical key, payload hash, and canonical payload bytes. |
| Resource lane authority rows | `resource_lane_claim_events`, `resource_lane_release_events`, `resource_lane_transitions` | No secrets allowed. Lane keys and ledger keys are non-secret coordination identities. | strict authority | Strict load validates lane id derivation, source event bindings, transition hash chains, fencing token monotonicity, active-holder fold, and release legality. |
| Admission lane coordination rows | `admission_lane`, `admission_waiter` | No secrets allowed. Stores lane class, derived lane ids, execution run ids for execution claims, admission mode, optional operational holder token, waiter token, deterministic waiter ids, tickets, status, and lease timestamps. | operational coordination | Mutable Postgres-only admission state. In v1, resource lanes use single-lane `wait_fifo` waiters; execution claims use `nowait_skip` holder leases keyed by derived run id. Resource waiters may be inserted/refreshed by `AdmissionBlocked`, are skipped when admitted or expired, and never grant lane ownership or replace `ResourceLaneClaimed`/`ResourceLaneReleased`/`resource_lane_transitions` authority. Execution claim leases coordinate active drivers only; they do not grant semantic authority to append, replay, resume, or cross side-effect boundaries. Expired resource retries reuse the deterministic waiter id with a fresh lane-local ticket; expired execution claims require explicit token-matched reaping before another holder can acquire. |
| Run observations | Derived from `commits` and `run_events` | No secrets allowed. | observation | List/watch materializes rows from strict authority at read time. Corruption cannot affect strict status/resume/replay/public-output reads. |
| Cursor metadata | `run_observation_cursors` | No secrets. Contains internal append XIDs, sort keys, store epochs, and cursor versions that must not be exposed. | operational telemetry | Server-issued opaque tokens are epoch-bound and no-TTL in v1. Unknown, missing, or stale-format rows are invalid; epoch mismatch expires. |
| Store metadata | `store_metadata` | No secrets. Fields are internal non-secret metadata. | operational telemetry | Defines store epoch and schema contract version. Runtime credentials must not mutate this table. |
| SQLx metadata | `crates/storages/stream-store-postgres/.sqlx/*.json` | No secrets. | operational telemetry | Compile-time query metadata only; checked in and validated by Nix workflows. |

## CLI And REST

| Surface | Location | Secret Boundary | Authority Role | Notes |
| --- | --- | --- | --- | --- |
| Run list/watch output | `mfm_cli run list`, `GET /v1/runs` | No secrets. | public output, observation | Returns observed run rows and opaque cursors only. It may lag strict authority and cannot be used as replay/resume authority. |
| Run status output | `mfm_cli run status`, REST status surfaces | No secrets. | public output | Rendered from strict run-store authority. JSON output is stable public API, not durable authority. |
| Run stream output | `mfm_cli run stream` and equivalent API surfaces | No secrets. | public output | Exposes sanitized event envelopes for inspection; persisted `run_events` remain the authority. |
| Public-output rendering | CLI/REST public output commands/routes | No secrets. | public output | Reads strict authority rows and proof-bearing artifact requests before rendering. Rendered JSON is not resume or replay authority. |
| Manual-resolution evidence input/output | CLI manual resolution commands and REST request/response bodies | No secrets. | public output | Evidence bytes are artifacts only after digest/evidence verification and commit admission. CLI/REST error details must stay redacted. |
| Keystore list output | `mfm_cli keystore list` | Public addresses and labels only; no private keys, mnemonics, passwords, decrypted bytes, or raw transactions. | public output | Keystore secrets remain below CLI output and typed semantic surfaces. |
| REST health/readiness | `/v1/health`, `/v1/ready` | No secrets. | operational telemetry | Liveness/readiness only. |

## Review Rules

- Adding or renaming a persisted table, artifact/evidence row, event canonical byte field,
  observation fact, cursor field, CLI JSON field, or REST response field must update this inventory
  in the same change.
- Secret-bearing values must stay out of typed configs, events, artifacts, facts, public outputs,
  diagnostics, fixtures, and snapshots.
- List/watch output is an observation surface. Strict resume, replay, status, retention proof,
  artifact reads, public-output rendering, and side-effect recovery must load and verify strict
  authority rows.
- Cursor internals are storage-private. Public `change_id` and `next_cursor` values must remain
  opaque and must not expose commit ids, append XIDs, sort keys, cursor versions, or store epochs.
