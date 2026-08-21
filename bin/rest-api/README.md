# MFM REST API

`mfm_rest_api` is the local REST rendering of the typed Application client surface. It composes the
complete Application before binding and listens only on a caller-selected Unix socket:

```text
mfm_rest_api serve [--deployment <PATH>] --unix-socket <PATH>
```

Deployment selection is the same strict override/XDG/HOME bootstrap used by the CLI. Composition
failures occur before bind and reach only local stderr. The complete deployment grammar, bounds, and
example are documented by [`mfm-app`](../../crates/app/README.md).

## Local socket

The socket parent must already exist and the socket leaf must be absent. Normal graceful shutdown
removes the socket. There is no TCP listener.

The server performs no authentication or authorization. It does not inspect peer credentials,
filesystem ownership, `Host`, or `Origin`, and it has no token, cookie, session, or CORS policy.
Access isolation, socket cleanup after a crash, and filesystem permissions belong to the enclosing
deployment environment.

## Routes

| Method | Path | Request | Success body |
| --- | --- | --- | --- |
| GET | `/healthz` | none | `{"status":"ok"}` |
| GET | `/v1/entry-points` | none | shared `ItemList` |
| GET | `/v1/bindings` | none | shared `ItemList` |
| PUT | `/v1/configs/{name}` | raw config JSON | shared `ImportOutcome`; 201 created or 200 unchanged |
| GET | `/v1/configs` | none | complete retained-revision `ItemList` |
| DELETE | `/v1/configs/{name}/revisions/{digest}` | none | empty; 204 whether present or absent |
| GET | `/v1/runs?after=&limit=` | none | shared mechanical `RunPage` |
| POST | `/v1/runs/{run_id}/start` | strict selection JSON | shared `StartRunResult` |
| POST | `/v1/runs/{run_id}/progress` | `{}` | shared `RunView` |
| GET | `/v1/runs/{run_id}` | none | shared `RunView` |

GET routes retain HTTP HEAD semantics: they perform the same Application admission and return the
same status and headers with no body. All other routed methods receive the normalized 405 envelope.
There is no schema-provisioning, deployment-view, inline-config start, server-generated RunId,
keystore, transaction, replay, trace, audit, or effect route.

Start accepts exactly one revision selection:

```json
{"config":{"name":"daily","digest":"content:sha256-jcs-v1:<64 hex>"}}
```

The path RunId is always caller-owned. Mutating routes use Axum's JSON media-type admission,
accepting `application/json` and application media types with a `+json` suffix.

## Bounds and failures

Config request bodies are limited to 256 KiB and checked again by `ConfigDocument`. Run action
bodies are limited to 4 KiB. Config listing accepts no query fields and returns every retained
revision as one unpaginated aggregate; each document remains independently bounded. Typed run-list
extraction accepts only one `after` RunId and one `limit`, rejects unknown
or duplicate fields, and uses the shared 1–200 page bound with default 50.

Ordinary routed errors are exactly `{"code":"...","message":"..."}` with the shared stable
Application code/message mapping. Ambiguous run appends add the shared tagged `recovery` object.
REST-local errors cover invalid body/query/media/path/header, fallback 404/405, and body size. HTTP
parser failures before Axum routing are outside that JSON contract. A durably failed run remains a
successful HTTP request with status 200 and tagged `state.kind:"failed"`.

The CLI-only asymmetries are intentional: `postgres init` retains schema authority outside the daemon,
and only the CLI may generate a RunId. HTTP status represents request success, while CLI exit 1 may
represent a runnable or durably failed run.

`nix run .#run -- --task rest-e2e` builds the CLI and REST binaries explicitly, starts this real
listener against local Reth and PostgreSQL, and compares discovery, config, exact start/delete,
run-head, full RunView, and shared-error JSON. The two renderers also match the same frozen
start/progress indeterminate recovery fixtures under `docs/contracts/client-surface/`.
