# MFM REST API

`mfm_rest_api` is the local REST rendering of the typed Application client surface. It composes the
complete Application before binding and listens only on a caller-selected Unix socket:

```text
mfm_rest_api serve [--deployment <PATH>] --unix-socket <PATH>
```

Deployment selection is the same strict override/XDG/HOME bootstrap used by the CLI. Composition
failures occur before bind and reach only local stderr.

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
| GET | `/healthz` | empty | `{"status":"ok"}` |
| GET | `/v1/entry-points` | empty | shared `EntryPointList` |
| GET | `/v1/bindings` | empty | shared `BindingList` |
| PUT | `/v1/configs/{name}` | raw config JSON | shared `ImportOutcome`; 201 created or 200 unchanged |
| GET | `/v1/configs?cursor=&limit=` | empty | shared `ConfigPage` |
| GET | `/v1/configs/{name}` | empty | shared `StoredConfigView` |
| DELETE | `/v1/configs/{name}` | empty plus `MFM-Config-Digest` | 204 |
| GET | `/v1/runs?cursor=&limit=` | empty | shared mechanical `RunPage` |
| POST | `/v1/runs/{run_id}/start` | strict selection JSON | shared `StartRunResult` |
| POST | `/v1/runs/{run_id}/progress` | `{}` | shared `RunView` |
| GET | `/v1/runs/{run_id}` | empty | shared `RunView` |

GET routes retain HTTP HEAD semantics: they perform the same Application admission and return the
same status and headers with no body. All other routed methods receive the normalized 405 envelope.
There is no schema-provisioning, deployment-view, inline-config start, server-generated RunId,
keystore, transaction, replay, trace, audit, or effect route.

Start accepts exactly one exhaustive selection:

```json
{"config":{"kind":"current","name":"daily"}}
{"config":{"kind":"exact","name":"daily","digest":"content:sha256-jcs-v1:<64 hex>"}}
```

The path RunId is always caller-owned. Conditional DELETE requires one bare checked digest header;
successful PUT responses set `Location: /v1/configs/{name}`. Mutating JSON routes require
`application/json`, optionally with only `charset=UTF-8`; DELETE is bodyless and uses its digest
header.

## Bounds and failures

Config request bodies are limited to 256 KiB and checked again by `ConfigDocument`. Run action
bodies are limited to 4 KiB. Bodyless routes accept exactly zero bytes. List query strings are
limited to 1 KiB, accept only one `cursor` and one `limit`, and use the shared 1–200 page bound with
default 50.

Ordinary routed errors are exactly `{"code":"...","message":"..."}` with the shared stable
Application code/message mapping. Ambiguous run appends add the shared tagged `recovery` object.
REST-local errors cover invalid body/query/media/path/header, fallback 404/405, and body size. HTTP
parser failures before Axum routing are outside that JSON contract. A durably failed run remains a
successful HTTP request with status 200 and tagged `state.kind:"failed"`.

The CLI-only asymmetries are intentional: `store init` retains schema authority outside the daemon,
and only the CLI may generate a RunId. HTTP status represents request success, while CLI exit 1 may
represent a runnable or durably failed run.

`nix run .#run -- --task rest-e2e` builds the CLI and REST binaries explicitly, starts this real
listener against local Reth and PostgreSQL, and compares discovery, config, Current/Exact start,
run-head, full RunView, and shared-error JSON. The two renderers also match the same frozen
start/progress indeterminate recovery fixtures under `docs/contracts/client-surface/`.
