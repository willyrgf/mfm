# Recoverability Application Surface v1

Status: current application/replay surface summary

Exact schemas and hashes are generated in `contracts/recoverability/v1/annex.json`; this document
explains ownership and transport behavior without duplicating every shape member.

## Material uncertainties

none

## Current-only rule

There is one `v1` registry and no compatibility decoder. The structured Runtime cutover reset the
annex, corpus, PostgreSQL schema, fixtures, and projections in place. Pre-cutover histories,
exports, DTOs, and schema identities are rejected at every current boundary. Git history is the
archive.

## Canonical rules

- exact float-free JCS-style canonical JSON;
- UTF-16 object-key ordering and duplicate-key rejection;
- no implicit Unicode normalization;
- bounded unsigned JSON integers only where the schema permits them;
- `sha256-v1` over exact retained bytes for content identity; and
- `sha256-jcs-v1` over the registered domain envelope for semantic identity.

Every decoder requires the exact registered schema id, closed object fields, bounds, and invariant
clauses. A schema-valid value under another contract cannot substitute.

## Public contracts

The annex currently freezes strict contracts for:

- entry-point discovery and planning profile;
- admit request/response;
- drive response;
- public run view;
- replay mode;
- portable structured run export;
- journal head and core typed identities;
- retained value and component-object evidence contracts; and
- purpose-limited fact-selection query/request.

The public entry ids remain `mfm.portfolio/snapshot@1` and
`mfm.evm/submit-transaction@1`.

## Admission

`mfm.admit-run-request.v1` binds exactly:

```text
version = "mfm.admit-run-request.v1"
entry_point_id
invocation_identity
input
```

The input is the entry point's strict selector. The app selects the exact entry contract and
authenticates `Admit` against store scope, entry operation, configured target, and invocation
identity. The policy derives the tenant and stable authenticated principal from the credential.
Only then may the app resolve current configured-value history, certify the candidate, and derive
the logical run id.

The EVM selector additionally carries a bounded `caller_submission_token`. It is independent of
`invocation_identity`: the former identifies permanent caller intent inside the authenticated
tenant/principal namespace, while the latter identifies one run. The configured EVM value contains
transaction semantics but no tenant, principal, or caller token. The app constructs the final EVM
submission request only after authorization.

`mfm.admit-run-response.v1` returns run id, admission disposition, entry id, entry operation id,
invocation identity, and planning-profile reference. `newly_admitted`, `attached`, and
`outcome_unknown` distinguish durable append knowledge; none grants follow-up authority.

## Drive

One drive request names one run and carries a fresh credential. The response schema exposes:

```text
run_id
journal_head
kind = advanced | waiting | closed
reason = null | retryable_evidence_gap | operational_block | integrity_block
```

The app maps the richer internal one-action Runtime disposition into this reviewed transport
shape. It never drives to completion in a single request and never exposes provider or internal
error text.

## Public run view

`mfm.public-run-view.v1` contains run and tenant identity, invocation and entry operation identity,
physical and semantic heads, current status, and a nullable canonical terminal outcome. Current
status is exactly one of:

```text
actionable
waiting_reads
possible_entry
blocked_integrity
closed
```

The view is created from one callback-free verified prefix. The selected terminal outcome is the
exact nominal root value; it is not recomputed by application code.

## Trace and audit

Transition and access inspection are purpose-authorized separately. Each page is fixed to one
journal head, uses a bounded limit, preserves commit order, and returns an opaque next position.

Transition entries project exact occurrence/path/call identity, input, consumed observation,
nominal outcome, facts, and before/after semantic digests. Access entries project exact
authorization and observation linkage, semantic and physical binding, request digest, attempt
ordinal, and closed outcome. They contain no private session, credential, endpoint, provider text,
signature, or signed bytes.

## Replay

Replay verification loads and folds recorded history only. `reproduce` and `compare_current`
require the corresponding replay plus export grants and a current strict portable export. When no
qualified historical executable/current candidate is supplied, the response is the frozen
`unavailable` result. No replay mode calls a state callback, transport, provider, signer, scanner,
or wallet authority.

## Portable export

Export is two-phase and purpose-sealed. Phase one discovers every recursively referenced prior-run
source from identifier-only fact-selection metadata, authorizes each source under the same target,
tenant, principal, and `Export` purpose, and rejects cyclic or over-budget graphs. Phase two
serializes only after that closure succeeds. Missing, denied, wrong-target, wrong-tenant, stale,
cyclic, and otherwise inaccessible dependencies collapse to one redacted `SourceRunExportDenied`
error and emit zero bytes.

The one current export is a canonical JSON object, not a framed sequence:

```text
version = "mfm.structured-portable-run-export.v1"
kind = "semantic" | "audit"
store_scope_id
tenant_scope_id
run_id
journal_head
semantic_head
records
objects
```

Media type:

```text
application/vnd.mfm.structured-run-export.v1+json
```

The external `ContentRef` uses raw SHA-256 of the exact object bytes. Replay input checks size,
digest, strict annex shape, and exact store/tenant/run equality before use.

## Access policy

Each protected operation consumes a fresh `SecretCredential` and requests its exact purpose
grant:

- `Admit`
- `Drive`
- `ReadPublic`
- `Replay`
- `InspectTrace`
- `InspectAudit`
- `Export`

Replay verification requests only `Replay`. Replay reproduction and current comparison request
both a fresh `Replay` grant and a fresh same-run `Export` grant because they consume a
caller-held export. Policy returns a tenant identity and a stable authenticated principal derived
from the credential. Every additional same-run grant must return both values unchanged. The app
then loads the run callback-free and requires exact tenant equality. A prior grant, run id, page
cursor, content reference, or export is never bearer authorization.

## CLI and REST

The CLI retains:

- operation listing;
- run admit, drive, show, replay, trace, audit, and export; and
- keystore import, list, and delete.

REST retains:

- `GET /v1/health`
- `GET /v1/ready`
- `GET /v1/entry-points`
- `POST /v1/runs`
- `GET /v1/runs/:run_id`
- `POST /v1/runs/:run_id/drive`
- `POST /v1/runs/:run_id/replay`
- `GET /v1/runs/:run_id/trace`
- `GET /v1/runs/:run_id/audit`
- `POST /v1/runs/:run_id/exports`

There are no list/watch, start/resume, generic object, fact, status-stream, or manual-resolution
routes. Standalone bootstraps fail closed without deployment-owned writer/session qualification;
embedded deployments inject a complete `Application`/`AppState`.

## Errors and privacy

Public error JSON conforms to `mfm.public-error.v1`: a non-empty code of at most 128 UTF-8 bytes, a
non-empty reviewed message of at most 4096 UTF-8 bytes, and optional secret-free `runtime_fault`
attribution. Transport class is carried only by HTTP status or process exit status and is not a
JSON field. Runtime attribution freezes the phase, run, nullable verified pre-fault head, nullable
occurrence, and either the semantic process contract or store scope and epoch. It never names a
private implementation reference or arbitrary diagnostic. CLI JSON and REST use the exact
`mfm.error-response.v1` envelope; CLI text renders only the fixed code and message.

Private sources are classified at the app boundary. Canary tests must prove that credentials,
mnemonics, private keys, database URLs, endpoints, headers, provider messages/bodies, signatures,
signed transactions, target-session material, mutation permits, private implementation identity,
and diagnostics cannot reach responses, histories, exports, or logs.

## Conformance

`contracts/recoverability/generate.py` deterministically writes the annex and corpus. Canonical,
ids, application, REST, CLI, store, replay, and PostgreSQL tests consume the same current files.
The model check rejects drift, floats, duplicate keys, unknown contracts/domains, incorrect
artifact hashes, and old contract bytes.
