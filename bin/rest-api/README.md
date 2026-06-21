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
- `MFM_EVM_RPC_SOURCES_JSON`: runtime-only EVM source registry for EVM contract entry-point runs
- `MFM_EVM_CONTRACT_SOURCE_REF`: optional EVM source id for EVM contract entry-point runs
- `MFM_EVM_CONTRACT_SOURCE_POLICY_ID`: optional EVM source policy id for EVM contract entry-point runs
- `MFM_EVM_SIGNERS_JSON`: runtime-only signer provider registry for EVM contract entry-point runs

The REST API validates the PostgreSQL schema on startup and does not create or
alter tables. Apply the `mfm-stream-store-postgres` migrations before starting
the server:

```bash
export DATABASE_URL="postgresql://postgres:postgres@localhost:5432/mfm_test"
cargo sqlx migrate run --source crates/storages/stream-store-postgres/migrations
```

## API

All responses are JSON envelopes:

- success: `{"status":"success","data": ...}`
- error: `{"status":"error","error":{"code":"...","message":"..."}}`

Endpoints:

- `GET /v1/health`
- `GET /v1/ready`
- `POST /v1/runs/start`
- `POST /v1/runs/:run_id/resume`
- `POST /v1/runs/:run_id/manual-resolution`
- `GET /v1/runs/:run_id/status`
- `GET /v1/runs/:run_id/stream?from_seq=1&to_seq=<optional>`
- `POST /v1/runs/:run_id/replay`
- `GET /v1/runs/:run_id/public-output/:schema_id`

Probe semantics:

- `/v1/health`: liveness only (process is running)
- `/v1/ready`: typed run store and typed artifact store probes must succeed

## Start A Run

`POST /v1/runs/start` accepts only entry-point operation requests. The REST layer decodes HTTP
input and delegates registry resolution, planning, certification, launch preparation, run
admission, and verified public-output rendering to app assembly.

Request shape:

```json
{
  "op": "portfolio_snapshot",
  "op_version": 1,
  "config_format": "json",
  "config": {
    "...": "entry-point config"
  },
  "run_id": "run:sha256-jcs-v1:<optional-digest>",
  "framework_version": "mfm.rest_api.typed.v1",
  "source_revision": "git-or-build-id",
  "drive": "until_blocked"
}
```

Request notes:

- `op` is required and selects a public entry-point operation.
- `op_version` is optional. When omitted, the latest registered version for `op` is used.
- `config_format` is `toml` or `json`; it defaults to `toml`.
- `config` is required. With `config_format: "toml"`, it must be a string. With
  `config_format: "json"`, it may be a JSON object/array/value accepted by the selected op.
- `run_id` is optional; the server generates a typed digest run id when omitted.
- `drive` is `until_blocked`, `append_only`, or `once`; it defaults to `until_blocked`.

The response is `{"run": ..., "public_output": ...}` inside the standard success envelope.
`public_output` is present when the run completes during the selected drive mode and the op exposes
a public output schema id.

Portfolio snapshot:

```json
{
  "op": "portfolio_snapshot",
  "config_format": "json",
  "config": {
    "portfolio": { "...": "PortfolioConfig JSON" },
    "valuation_source_registry": { "sources": [] }
  }
}
```

EVM contract deploy:

```json
{
  "op": "evm_contract_deploy",
  "config_format": "json",
  "config": {
    "...": "DeployPhaseConfig JSON"
  },
  "drive": "until_blocked"
}
```

EVM contract configure:

```json
{
  "op": "evm_contract_configure",
  "config_format": "json",
  "config": {
    "config": {
      "...": "ConfigurePhaseConfig JSON"
    },
    "deployed": {
      "...": "DeployedContract JSON"
    }
  },
  "drive": "until_blocked"
}
```

EVM contract validate:

```json
{
  "op": "evm_contract_validate",
  "config_format": "json",
  "config": {
    "config": {
      "...": "ValidatePhaseConfig JSON"
    },
    "configured": {
      "...": "ConfiguredContract JSON"
    }
  },
  "drive": "until_blocked"
}
```

EVM contract lifecycle:

```json
{
  "op": "evm_contract_lifecycle",
  "config_format": "json",
  "config": {
    "...": "ContractLifecycleConfig JSON"
  },
  "drive": "until_blocked"
}
```

EVM configs carry semantic network intent, expected chain id, artifact refs, and non-secret signer
intent. Process-local RPC endpoints and keystore paths are resolved only through
`MFM_EVM_RPC_SOURCES_JSON` and `MFM_EVM_SIGNERS_JSON`.

Stable launch error codes:

- `InvalidJson`: the request envelope is not accepted by the route schema.
- `InvalidPublicOpName`: `op` is not a valid public entry-point name.
- `InvalidOpVersion`: `op_version` is zero.
- `EntryPointOpNotFound`: no op is registered for the supplied public name.
- `EntryPointOpVersionNotFound`: the requested explicit version is not registered.
- `AuthoredConfigDecodeFailed`: the supplied config cannot be decoded for the selected op.
- `AuthoredConfigFormatShapeMismatch`: `config_format` and `config` shape do not match.
- `PortfolioSnapshotConfigInvalid`: portfolio snapshot config validation failed.
- `EvmContractPlanFailed`: EVM contract entry-point planning failed.
- `EntryPointOpCertificationFailed`: the planned spec failed app-owned certification.
- `LaunchRunnerUnavailable`: the verified spec references a state descriptor without a production
  runner binding.

## Resume, Replay, And Public Output

Resume:

```bash
curl -s -X POST "http://127.0.0.1:3001/v1/runs/$RUN_ID/resume" \
  -H "content-type: application/json" \
  -d '{"drive":"once"}'
```

Record a signed manual resolution:

```bash
curl -s -X POST "http://127.0.0.1:3001/v1/runs/$RUN_ID/manual-resolution" \
  -H "content-type: application/json" \
  -d '{
    "kind": "manual_resolution_v1",
    "outcome": "confirm_remediated",
    "evidence_json": {
      "operator_note": "reviewed"
    },
    "authorization_proof": {
      "...": "canonical manual authorization proof JSON"
    },
    "drive": "until_blocked"
  }'
```

The manual-resolution route canonicalizes `evidence_json` and `authorization_proof`, then submits
the resulting bytes to `mfm-app`. Runtime derives the current prefix authority from the stored run
stream, verifies the proof against the certified manual policy, checks signatures and quorum,
stages the evidence and authorization artifacts, and appends only through the typed
manual-resolution commit boundary. The route does not accept request-supplied prefix facts, artifact
ids, hashes, verifier ids, authority ids, quorum values, keystore paths, signer configuration, or
signature secrets. Proof authoring is outside this REST ingress; submitted proof bytes are always
untrusted until runtime verifies them.

Optional fields:

- `evidence_media_type`: defaults to `application/json`.
- `note`: optional redaction-safe operator note recorded in `ManualResolutionRecorded`.
- `drive`: `append_only`, `once`, or `until_blocked`; defaults to `until_blocked`.

Replay verification:

```bash
curl -s -X POST "http://127.0.0.1:3001/v1/runs/$RUN_ID/replay"
```

Render typed public output:

```bash
curl -s "http://127.0.0.1:3001/v1/runs/$RUN_ID/public-output/$SCHEMA_ID"
```

The status and stream endpoints are typed inspection views over the authoritative typed run stream.
Stream range filters are applied only after the app service validates the full stored stream.
Resume, replay, and public-output rendering load the stored spec/certificate artifacts, verify them
against the production registry, compare their evidence to `RunAdmitted`, and rebuild stream
evidence before constructing runtime, replay, or render authority. Rendered public-output JSON is an
output/cache surface only.

Typed run responses expose semantic status through `run_mode`, not the old absent/started/completed
phase. `run_mode` is one of `forward`, `remediating`, `manual_blocked`, `completed`, `compensated`,
`manually_resolved`, or `failed_without_acdc_claim`. The nested `saga` object reports the certified
policy, derived per-forward-ledger obligations, linked remediation ledgers, manual-block reason and
manual authorization requirements when applicable, terminal resolution claim when present,
projected resource ledgers with declared claim/key/touched-set evidence, and active exclusive lane
holders referenced by the target run's persisted live side-effect ledgers. Status does not serialize
unrelated global lane holders or scheduler waiters that blocked before appending lane evidence.
`attempt_dispositions` reports committed attempt-level lifecycle status separately from `run_mode`;
each entry has `node_id`, `attempt_id`, `disposition` (`started`, `completed`, `failed`, or
`interrupted`), and status-specific fields such as `attempt_no`, `retryable`, or `output_cell_id`.
`scheduler_status` is read-only `observed` for `GET /v1/runs/:run_id/status`; start/resume responses
set it to `advanced`, `blocked`, or `public_output_projected` according to the app dispatch loop.
Manual authorization requirements include the required evidence schema, signing scheme, authority id,
allowed operator public identities or a safe summary, and quorum. They never expose signer runtime
sources, keystore paths, password paths, passwords, or other secrets.

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
