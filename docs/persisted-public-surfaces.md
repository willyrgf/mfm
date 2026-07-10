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
- `audit provenance`: non-secret persisted context that explains which process-local resource served
  a recorded action. It must not mint replay, retry, consistency, public-output, or side-effect
  authority.
- `public output`: data intentionally returned to users by CLI/REST. It must not contain secrets and
  is not a substitute for strict authority unless the command explicitly verifies strict rows first.

## Postgres Storage

| Surface | Location | Secret Boundary | Authority Role | Notes |
| --- | --- | --- | --- | --- |
| Artifact bytes | `artifact_blobs.bytes` | No secrets allowed. Secret-like bytes must stay below artifacts. | strict authority | Trusted only with matching `artifact_admissions` evidence, digest, byte length, role, and proof-bearing read request. |
| Fact descriptor artifacts | `artifact_blobs.bytes` with `ArtifactRole::FactDescriptor` | No secrets allowed. Descriptors contain fact kind, schema ids, field definitions, ordering, operators, units/scales, and exposure policy only. | strict authority | Durable descriptor authority. Append, query, rebuild, and replay verify descriptor bytes by hash; public discovery exposes only descriptor-approved summaries through the facts service. |
| Fact response artifacts | `artifact_blobs.bytes` with `ArtifactRole::FactResponse` | No secrets allowed. Contains one typed response claim payload for v1. | strict authority | Binds a `FactRecorded` claim to retained response evidence. Response artifacts are not public output; CLI/REST may return only descriptor-approved `Returnable` field summaries. |
| Fact query evidence artifacts | `artifact_blobs.bytes` with `ArtifactRole::FactQueryEvidence` | No secrets allowed. May contain internal fact refs, returned summaries, receipt authentication, frontier, plan, and selection evidence. | strict authority | Private replay evidence referenced by `ArtifactReferenced`. Replay verifies the authenticated receipt and retained source fact authority; these artifacts are never projected into `fact_index` or returned through public fact APIs. |
| EVM prepared invocation artifacts | `artifact_blobs.bytes` for schema `mfm.evm.contract.adapter.prepared_invocation` | No secrets allowed. Contains unsigned transaction evidence, signing digest, and expected transaction hash only; raw signed transaction bytes and signatures remain transient. | strict authority | Resume reconstructs signing requests from this artifact and verifies the submit-time deterministic signed transaction hash against `expected_transaction_hash` before broadcast. |
| EVM not-submitted proof artifacts | `artifact_blobs.bytes` for schema `mfm.evm.contract.adapter.not_submitted_proof` | No secrets allowed. Contains public transaction hashes, signer address, nonce, optional occupying block number, and redacted chain evidence only. | strict authority | Emitted only when recovery observes a concrete non-anchor transaction occupying the prepared sender nonce. Mere nonce advancement or provider failure remains `SubmissionUnknown`. |
| Provider runtime source provenance and diagnostics | Typed BTC/EVM evidence fields in run events, facts, artifacts, snapshot errors, and closed provider diagnostics | No secrets allowed. May include provider family, stable diagnostic code, redaction-safe operation id, numeric status/error codes, semantic `network_id`, expected/observed EVM chain id, selected `source_ref`, `policy_id`, expected/observed Bitcoin network tag, Bitcoin head height/hash, and closed boolean/integer/id fields. Must not include RPC URLs, auth headers, file paths, request/response bodies, provider messages, signer material, or signed raw transactions. | audit provenance | Provider diagnostics use stable public error codes shaped as `{provider_family}_{diagnostic_code}` such as `bitcoin_rpc_http_status` or `evm_source_mismatch`. Source refs and policy ids are audit provenance only; replay verifies recorded evidence against the certified provider binding and operation request and must not resolve them against current runtime config. Retry, consistency, public-output, and side-effect authority must not depend on them. |
| Artifact evidence | `artifact_admissions`, `evidence_canonical_json`, `evidence_hash` | No secrets allowed. Producer and schema ids are non-secret identities. | strict authority | Binds artifact id, digest, byte length, media type, role, schema/semantic ids, and producer identity. |
| Commit artifact bindings | `commit_artifact_evidence`, `run_artifact_admissions` | No secrets allowed. | strict authority | Binds required/admitted artifact evidence to one commit and one run. |
| Commit authority bytes | `commits.idempotency_canonical_json`, `prepared_authority_canonical_json`, `commit_batch_canonical_json` | No secrets allowed. | strict authority | Strict load revalidates canonicalizer identity, hash-domain version, commit id, idempotency hash, prepared authority hash, and final batch hash. |
| Event canonical bytes | `run_events.payload_canonical_json`, `payload_hash`, event identity columns | No secrets allowed. | strict authority | Strict load reconstructs typed envelopes and cross-checks run id, seq, ordinal, commit key, event id, schema id, spec hash, logical key, payload hash, and canonical payload bytes. |
| `FactRecorded` normalized event fields | `run_events.payload_canonical_json` for `FactRecorded` | No secrets allowed. Subject material, response evidence, request evidence, observed time, and producer provenance must be non-secret. | strict authority | Carries explicit visibility, kind, descriptor hash, subject namespace/material hashes, `FactKey`, optional `observed_at`, request evidence, response artifact evidence, and capability/adapter provenance. It is the authoritative claim; fact projection rows copy or derive from it. |
| Fact descriptor catalog projection | `fact_descriptor_index` | No secrets allowed. Contains descriptor hashes, schema ids, fact kind, subject namespace hash, and descriptor artifact ids/evidence hashes. | rebuildable cache | Store-wide searchable descriptor catalog derived from descriptor artifacts. It is not semantic authority; rebuild and replay verify against retained descriptor artifacts. |
| Run fact descriptor admission projection | `run_fact_descriptor_admissions` | No secrets allowed. Contains run ids, descriptor hashes, descriptor artifact ids/evidence hashes, source run coordinates, and commit ids. | rebuildable cache | Per-run link proving which descriptor artifacts each `RunAdmitted` event admitted. Fact recording must reference a descriptor admitted by the producing run and present in the store-wide descriptor catalog. |
| Fact index projection | `fact_index` | No secrets allowed. Contains internal refs, source run/event coordinates, producer node id, recorded/observed times, visibility audience/scope, hashes, response artifact ids/evidence hashes, and adapter provenance. | rebuildable cache | Queryable projection for indexed `Platform` and `Control` facts only. Public APIs must filter to `Platform`/default and must not expose raw internal refs, artifact ids, hashes, run/event coordinates, or `Control` rows. |
| Fact index terms | `fact_index_terms` | No secrets allowed. Stores descriptor-derived scalar subject, result, and metadata terms only. | rebuildable cache | Generic query/order terms keyed by fact claim id and field id. Terms are recomputed from descriptors, retained subject material, response artifacts, and claim/store metadata. |
| Fact projection metadata | `fact_projection_metadata` | No secrets. Projection generation is store-private metadata. | operational telemetry | Names the current fact projection generation used in query frontiers and signed receipts. It is not semantic fact authority by itself. |
| Resource lane authority rows | `resource_lane_claim_events`, `resource_lane_release_events`, `resource_lane_transitions` | No secrets allowed. Lane keys and ledger keys are non-secret coordination identities. | strict authority | Strict load validates lane id derivation, source event bindings, transition hash chains, fencing token monotonicity, active-holder fold, and release legality. |
| Admission lane coordination rows | `admission_lane`, `admission_waiter` | No secrets allowed. Stores lane class, derived lane ids, execution holder run ids for execution claims, admission mode, optional operational holder token, waiter token, deterministic waiter ids, tickets, status, and lease timestamps. | operational coordination | Mutable Postgres-only coordination state. Execution claims are `nowait_skip` leases keyed by base work identity (`certified_spec_hash` + `store_scope_id`) with the holder `run_id` stored separately. Resource admission uses `wait_fifo` rows for one exclusive side-effect lane. These rows can block or wake attempts, but they never grant run authority or resource ownership; durable authority remains in `RunAdmitted` and `ResourceLaneClaimed`/`ResourceLaneReleased`/`resource_lane_transitions`. Expired resource retries get fresh lane-local tickets; expired execution claims require explicit holder-and-token-matched reaping before another holder can acquire. |
| Run observations | Derived from `commits` and `run_events` | No secrets allowed. | observation | List/watch materializes rows from strict authority at read time. Corruption cannot affect strict status/resume/replay/public-output reads. |
| Cursor metadata | `run_observation_cursors` | No secrets. Contains the internal store commit coordinate, store epoch, and cursor version; these must not be exposed. | operational telemetry | Server-issued opaque tokens are epoch-bound and no-TTL in v2. Unknown, missing, or stale-format rows are invalid; epoch mismatch expires. |
| Store metadata | `store_metadata` | No secrets. Fields are internal non-secret metadata. `store_scope_id` is non-secret deployment identity material. | operational telemetry | Defines store epoch, store-owned scope, and schema contract version. Runtime credentials must not mutate this table. |
| SQLx metadata | `crates/storages/stream-store-postgres/.sqlx/*.json` | No secrets. | operational telemetry | Compile-time query metadata only; checked in and validated by Nix workflows. |

## CLI And REST

| Surface | Location | Secret Boundary | Authority Role | Notes |
| --- | --- | --- | --- | --- |
| Run list/watch output | `mfm_cli run list`, `GET /v1/runs` | No secrets. | public output, observation | Returns observed run rows and opaque cursors only. It may lag strict authority and cannot be used as replay/resume authority. |
| Run status output | `mfm_cli run status`, REST status surfaces | No secrets. | public output | Rendered from strict run-store authority. JSON output is stable public API, not durable authority. |
| Run stream output | `mfm_cli run stream` and equivalent API surfaces | No secrets. | public output | Exposes sanitized event envelopes for inspection; persisted `run_events` remain the authority. |
| Public-output rendering | CLI/REST public output commands/routes | No secrets. | public output | Reads strict authority rows and proof-bearing artifact requests before rendering. Rendered JSON is not resume or replay authority. |
| CLI facts JSON output | `mfm_cli facts kinds/describe/explain/query/latest/history/top/show` | No secrets. Returns public kind/descriptor summaries, opaque public refs, recorded/observed times, and descriptor-approved `Returnable` field values only. | public output | Reads through evidence-only app fact services over `Platform`/default scope. Must not expose `Control` or `RunPrivate` facts, internal refs, artifact ids/evidence hashes, subject material or hashes, response artifacts, request/response hashes, raw run/event coordinates, or capability routing details. |
| REST facts responses | `/v1/facts/kinds`, `/v1/facts/kinds/:kind`, `/v1/facts/:kind`, `/v1/facts/:kind/latest`, `/v1/facts/ref/:public_ref` | No secrets. Returns public fact DTOs and stable redacted error envelopes only. | public output | Uses the same app/facts query boundary as CLI. Unknown refs and refs that resolve only to non-public facts must return the same redacted not-found class; descriptor discovery and ambiguity errors must not disclose non-public facts. |
| Manual-resolution evidence input/output | CLI manual resolution commands and REST request/response bodies | No secrets. | public output | Evidence bytes are artifacts only after digest/evidence verification and commit admission. CLI/REST error details must stay redacted. |
| Keystore list output | `mfm_cli keystore list` | Public addresses and labels only; no private keys, mnemonics, passwords, decrypted bytes, or raw transactions. | public output | Keystore secrets remain below CLI output and typed semantic surfaces. |
| REST health/readiness | `/v1/health`, `/v1/ready` | No secrets. | operational telemetry | Liveness/readiness only. |
| Live runtime config | Process-local runtime config file path, indirection paths, resolved RPC URLs/auth, signer paths, and signer unlock files | Secret-bearing; forbidden in CLI/REST output, run events, artifacts, fixtures, replay authority, and public-output rendering. | none | Evidence-only status, stream inspection, list/watch, replay, and public-output paths must not parse or validate live runtime config. Live start/resume may use it only as process-local capability wiring. |

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
  opaque and must not expose commit ids, store commit coordinates, cursor versions, or store epochs.
