# MFM REST API Documentation

Experimental REST API (Axum) for certified typed runs.

The REST API is a typed assembly surface only. It can start, resume, inspect, replay, and render
certified typed runs through `mfm-app`, `mfm-runtime`, `mfm-store`, and typed artifact storage. It
does not accept old dynamic DAGs, `PlannedOp`, context snapshots, generic feature execution, or
legacy artifact ids as semantic run authority.

## Running (Nixfied)

```bash
# Local dev workflow (starts Postgres + REST API)
nix run .#dev

# Run the server only (requires DATABASE_URL)
nix run .#mfm_rest_api
```

## Configuration

Environment variables:

- `MFM_REST_API_ADDR`: bind address (default: `127.0.0.1:3001`)
- `DATABASE_URL`: Postgres URL for the certified typed run-event store (required)
- `MFM_TYPED_ARTIFACT_ROOT`: typed artifact root (default: `~/.mfm/typed_run_artifacts`)
- `MFM_SOURCE_REVISION`: optional source revision evidence for typed run starts

## API

All responses are JSON envelopes:

- success: `{"status":"success","data": ...}`
- error: `{"status":"error","error":{"code":"...","message":"..."}}`

Endpoints:

- `GET /v1/health`
- `GET /v1/ready`
- `POST /v1/portfolio/snapshot`
- `POST /v1/runs/start`
- `POST /v1/runs/:run_id/resume`
- `GET /v1/runs/:run_id/status`
- `GET /v1/runs/:run_id/stream?from_seq=1&to_seq=<optional>`
- `POST /v1/runs/:run_id/replay`
- `GET /v1/runs/:run_id/public-output/:schema_id`

Probe semantics:

- `/v1/health`: liveness only (process is running)
- `/v1/ready`: typed run store and typed artifact store probes must succeed

## Start A Portfolio Snapshot

`POST /v1/portfolio/snapshot` accepts the same documented portfolio snapshot request JSON as
`mfm_cli portfolio snapshot`, compiles it into a certified typed portfolio spec, persists the
required typed config artifacts, and starts the certified run:

```json
{
  "kind": "portfolio_snapshot_start_v1",
  "request": {
    "portfolio": { "...": "PortfolioConfig" },
    "valuation_source_registry": { "sources": [] }
  },
  "run_id": "run:sha256-jcs-v1:<optional-digest>",
  "framework_version": "mfm.rest_api.portfolio.typed.v1",
  "source_revision": "git-or-build-id",
  "drive": "until_blocked"
}
```

The response is `{"run": ..., "public_output": ...}` inside the standard success envelope.
`public_output` is present when the run completes during the selected drive mode. This route never
submits old dynamic `portfolio_tracker`, `portfolio_execute`, or `portfolio_config_build` ops.

## Start A Typed Run

`POST /v1/runs/start` accepts only a certified typed spec bundle request:

```json
{
  "kind": "typed_run_start_v1",
  "bundle": {
    "kind": "certified_typed_spec_bundle_v1",
    "spec": {
      "...": "TypedExecutionSpec JSON"
    },
    "certificate": {
      "...": "CertifiedSpecCertificate JSON"
    }
  },
  "run_id": "run:sha256-jcs-v1:<optional-digest>",
  "seeds": [
    {
      "seed_id": "seed:sha256-jcs-v1:<digest>",
      "json": {
        "...": "canonical seed value"
      }
    }
  ],
  "framework_version": "mfm.rest_api.typed.v1",
  "source_revision": "git-or-build-id",
  "drive": "until_blocked"
}
```

Request notes:

- `kind` must be `typed_run_start_v1`.
- `bundle.kind` must be `certified_typed_spec_bundle_v1`.
- The bundle is parsed as untrusted transport data. The server verifies the contained spec and
  certificate against the production certification registry before `RunStarted`.
- Hash-only spec envelopes, raw typed spec JSON, certificate bytes, and summaries are not runtime
  authority.
- `run_id` is optional; the server generates a typed digest run id when omitted.
- `seeds[*].json` is canonicalized and persisted as JSON seed material.
- `drive` is `until_blocked` or `append_only`; it defaults to `until_blocked`.
- Specs that reference unported domain descriptors fail before `RunStarted` with
  `TypedRunnerUnavailable`.

Stable start error codes:

- `InvalidJson`: the request envelope is not accepted by the route schema.
- `TypedBundleInvalid`: the bundle is malformed, has the wrong kind, is missing fields, contains
  unknown fields, or cannot be canonicalized.
- `TypedCertificationFailed`: certificate/spec hash, registry digest, descriptor identity/digest,
  lowering/canonicalizer identity, public-output schema evidence, or certifier validation failed.
- `TypedRunnerUnavailable`: the verified spec references a state descriptor without a production
  runner binding.

## Resume, Replay, And Public Output

Resume:

```bash
curl -s -X POST "http://127.0.0.1:3001/v1/runs/$RUN_ID/resume" \
  -H "content-type: application/json" \
  -d '{"drive":"append_only"}'
```

Replay verification:

```bash
curl -s -X POST "http://127.0.0.1:3001/v1/runs/$RUN_ID/replay"
```

Render typed public output:

```bash
curl -s "http://127.0.0.1:3001/v1/runs/$RUN_ID/public-output/$SCHEMA_ID"
```

The status and stream endpoints are typed inspection views over the authoritative typed run stream.
The public-output endpoint and append-only resume validate the stored stream against the persisted
certified spec before returning semantic output.

## Removed Dynamic Surfaces

These old REST surfaces are intentionally not part of the typed API:

- `GET /v1/features`
- `POST /v1/features/:feature_id/execute`
- `GET /v1/artifacts/:artifact_id`
- dynamic single-op or pipeline run-start payloads
- context snapshot reads as public output

Old dynamic runs are not silently migrated into certified typed runs.

Docs:

- Design contract: [`../../docs/design.md`](../../docs/design.md)
- Architecture overview: [`../../docs/architecture.md`](../../docs/architecture.md)
- CLI docs (parallel surface): [`../cli/README.md`](../cli/README.md)
