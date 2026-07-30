# MFM REST API

`mfm-rest-api` is the HTTP adapter for the recoverability-v3 `mfm_app::Application` facade. The
router owns request decoding, bearer extraction, status mapping, and response envelopes only.
Storage, authentication policy, tenant derivation, run authority, planning, runtime, and replay
remain behind the application facade.

## Routes

The complete route set is:

| Method | Route | Authentication |
| --- | --- | --- |
| `GET` | `/v1/health` | none |
| `GET` | `/v1/ready` | none |
| `GET` | `/v1/entry-points` | none |
| `POST` | `/v1/runs` | `Admit` |
| `GET` | `/v1/runs/{run_id}` | `ReadPublic` |
| `POST` | `/v1/runs/{run_id}/drive` | `Drive` |
| `POST` | `/v1/runs/{run_id}/replay` | `Replay` |
| `GET` | `/v1/runs/{run_id}/trace` | `InspectTrace` |
| `GET` | `/v1/runs/{run_id}/audit` | `InspectAudit` |
| `POST` | `/v1/runs/{run_id}/exports` | `Export` |

There is no fact endpoint, `GET /v1/runs` list/watch route, start/resume route, status/stream
split, manual-resolution route, public-output route, or arbitrary object endpoint.

`GET /v1/health` is process liveness only. `GET /v1/ready` performs a bounded PostgreSQL
writable-lineage probe against the already qualified authoritative run store and independently
checks the fenced executor ledger; both are mandatory. Success uses the normal envelope with data
exactly `{ "ok": true }` and exposes no component checks. It never performs EVM, DNS, provider,
semantic-callback, or run work. Any probe failure returns the same `503 NotReady` error with
`The service is not ready`.

## Authentication and errors

Every protected request accepts exactly:

```http
Authorization: Bearer <opaque credential>
```

The credential is bounded to 64 KiB and passed to the app in a consuming zeroizing type. Tenant
scope is derived only by the injected access policy. Tenant headers, query parameters, request
fields, cookies, and sessions are not accepted.

JSON successes use:

```json
{"status":"success","data":{}}
```

Errors use:

```json
{"status":"error","error":{"code":"...","message":"..."}}
```

Methods not registered for a listed path return `405 MethodNotAllowed`; unknown paths return
`404 NotFound`. Every listed path explicitly rejects `HEAD`; on `GET` paths this prevents Axum's
implicit GET handling from invoking
authentication or application work. Per HTTP semantics the HEAD response retains the reviewed
status and headers, including `Content-Type: application/json` and an `Allow` header containing
only the route's real `GET` or `POST` method, but has an empty body.

Authentication failures are `401 AuthenticationRequired`; a valid credential lacking the exact
grant receives `403 GrantDenied`. An absent run and a run in another tenant both receive the same
`404 RunNotFound`. Export dependency denial or a wrong-tenant dependency receives
`403 SourceRunExportDenied`; if a separately authorized dependency named by verified append-only
history is then absent, the response is the fixed `500 ReplayVerificationFailed`.

## Requests and responses

Admission accepts the exact annex-validated request:

```json
{
  "version": "mfm.admit-run-request.v1",
  "entry_point_id": "mfm.portfolio/snapshot@1",
  "invocation_identity": "de305d54-75b4-431b-adb2-eb6b9e546014",
  "input": {"target": "portfolio-name"}
}
```

`newly_admitted`, `attached`, and `outcome_unknown` map to HTTP `201`, `200`, and `202`
respectively. Drive has an empty body. Replay requires exactly one `mode` query parameter:

```text
POST /v1/runs/{run_id}/replay?mode=verify
POST /v1/runs/{run_id}/replay?mode=reproduce
POST /v1/runs/{run_id}/replay?mode=compare_current
```

`verify` requires an empty body. `reproduce` and `compare_current` require the raw portable stream
as the request body and exactly one value for each header:

```http
Content-Type: application/vnd.mfm.run-export-stream.v2+json-seq
Mfm-Content-Digest: content:sha256-v1:<64 lowercase hex>
```

The content type must match byte-for-byte and cannot carry parameters. The adapter derives the
current stream schema identity, constructs an affine body reader without polling it, and passes
that reader and external digest to the app. Non-verify replay obtains a separate same-run
`Export` decision before polling the body and rejects wrong framing, digest,
run/store/tenant binding, semantic coordinate, or closure before any resolver. Replay never
generates or fetches a replacement export.

Malformed, noncanonical, missing, forbidden, or binding-mismatched caller streams return
`400 ReplayArtifactInvalid` with `The replay artifact is invalid.` Stream-reader I/O failures
return only the fixed `500 ExportStreamIoFailed` contract. Authenticated store/history integrity
failures remain `500 ReplayVerificationFailed`.
Candidate execution and comparison-integrity failures use that same 500 response. An unavailable
sealed current candidate returns `503 RuntimeCatalogUnavailable` with
`The exact admitted runtime catalog is unavailable`; a missing exact historical executable remains
a successful `reproduced` response whose result is `unavailable`.

Trace and audit use only `cursor` and `limit` query parameters, with a default of 100 and maximum
of 500.
Protected request bodies are read only after bearer extraction and reject duplicate JSON fields.
Ordinary JSON request bodies are bounded to 16 MiB. A non-verify replay body has no total-byte
validity ceiling: stream frames and decoded chunks are individually bounded and verification
advances in bounded cooperative steps. The protected `GET` routes, `drive`, and verify replay
reject non-empty bodies.
Transition traces render an authorized-but-absent cross-run source with the same redacted lineage
as a source denied by grant or tenant. A fresh source `AuthenticationRequired` decision fails the
page with `401`.
Each audit entry exposes exactly `authorization_ref`, `observation_ref`,
`authorization_journal_head`, `observation_journal_head`, `capability_binding_ref`,
`capability_operation_id`, `request_ref`, `status`, `result_ref`, `failure`,
`non_domain_failure`, `effect_key`,
`delivery_audit_ref`, and `delivery_audit_terminal`. The last field is `true` for a verified
terminal returned ensure, `false` for a verified pending returned ensure, and `null` for reads or
when no returned ensure is verified at the page head. It is presentation-only; the journal
persists only `delivery_audit_ref`.

Export accepts `{"kind":"semantic"}` or `{"kind":"audit"}`. It is the only non-envelope success:
the response body lazily streams the exact framed portable-export bytes, `Content-Type` is
`application/vnd.mfm.run-export-stream.v2+json-seq`, and `Mfm-Content-Digest` is SHA-256 over every
record separator, canonical frame byte, and line feed. A body-reader failure exposes only the
fixed `export stream unavailable` body error, never a private path or backend diagnostic.

## Deployment

The repository's standalone server does not possess a deployment-owned
`AuthoritativeWriterFence`, so `make_default_app_state` fails closed with
`AuthoritativeWriterFenceUnavailable` before binding a socket. `MFM_REST_API_ADDR` controls only
the bind address and cannot grant writer authority.

A deployment embeds this crate, constructs a qualified `mfm_app::Application` with its real access
policy, run-store writer fence, exact wallet deployment, and independent executor
writer-generation fence, wraps it in `AppState::new`, and passes that state to `make_app`.

The frozen application and wire contract is
[`docs/recoverability-app-surface-v3.md`](../../docs/recoverability-app-surface-v3.md).
