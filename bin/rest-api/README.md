# MFM REST API

Axum transport over an injected `mfm_app::Application`.

## Routes

```text
GET  /v1/health
GET  /v1/ready
GET  /v1/entry-points
POST /v1/runs
GET  /v1/runs/:run_id
POST /v1/runs/:run_id/drive
POST /v1/runs/:run_id/replay?mode=verify|reproduce|compare_current
GET  /v1/runs/:run_id/trace
GET  /v1/runs/:run_id/audit
POST /v1/runs/:run_id/exports
```

There are no run list/watch, start/resume, status-stream, fact, generic-object, public-output,
manual-resolution, or arbitrary mutation routes.

`health` is process liveness. `ready` performs the bounded writable-lineage probe on the already
qualified structured RunHistory store and never performs EVM/provider/callback work.

## JSON and errors

Ordinary success uses the current strict application response inside the API success envelope.
Unknown fields, duplicate keys, malformed typed ids, invalid cursors, and noncanonical replay input
are rejected. Reproduction and current-comparison replay also reject any media type other than the
portable-export media contract below. Error responses use reviewed HTTP status plus the frozen
`mfm.error-response.v1` JSON envelope. The nested error has a non-empty code bounded to 128 UTF-8
bytes, a non-empty reviewed message bounded to 4096 UTF-8 bytes, and optional secret-free
`runtime_fault` attribution. Transport class is not a JSON field. Runtime attribution may name
only phase, run, nullable verified pre-fault head, nullable occurrence, and a semantic process
contract or store scope and epoch; private implementation identity and diagnostics are excluded.

Every protected route requires exactly one credential header and obtains a fresh purpose-specific
application authorization. The policy derives both tenant and stable authenticated principal from
that credential. Trace, audit, replay, and export grants are distinct. Credentials, headers,
endpoints, provider bodies, and private diagnostics are never logged or returned.

`POST /v1/runs` authorizes the selector's exact configured target before configuration lookup. The
EVM selector contains `target` and a bounded `caller_submission_token`; it contains no tenant or
principal. EVM deployment configuration likewise contains no tenant, principal, or caller token.
Only after authorization does the app combine its credential-derived identity, the caller token,
and configured transaction semantics. `invocation_identity` remains independent and scopes the
run id, not permanent EVM submission progress.

## Pagination

Trace and audit accept a bounded `limit` plus an opaque cursor. The cursor binds the run,
inspection purpose, fixed journal head, next index, and integrity value. It conveys no authority.
Pages remain stable while newer history is appended because every page is fixed to its initial
head.

## Replay and export

`verify` requires an empty body. `reproduce` and `compare_current` require the exact portable export
body and headers:

```http
Content-Type: application/vnd.mfm.structured-run-export.v1+json
Mfm-Content-Digest: content:sha256-v1:<64 lowercase hex>
```

The body is a single strict canonical object. Digest, size, schema, store, tenant, and run are
validated before replay. Reproduction/comparison never use live callbacks and return the frozen
unavailable result when no candidate is supplied.

Export accepts `{"kind":"semantic"}` or `{"kind":"audit"}`. It is the only non-envelope success:
the response streams the exact canonical object with the media type above and a raw-byte
`Mfm-Content-Digest`. Stream failures expose only the fixed reviewed body error.

## Deployment

The standalone server has no deployment-owned `AuthoritativeWriterFence` or sealed wallet
target/session provider, so `make_default_app_state` fails closed before binding a socket.
`MFM_REST_API_ADDR` controls only the bind address.

Deployments embed this crate, construct a qualified `mfm_app::Application`, wrap it in
`AppState::new`, and pass it to `make_app`. See
[the application surface contract](../../docs/recoverability-app-surface-v1.md).
