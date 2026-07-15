# MFM REST API Documentation

Experimental REST API (Axum) for certified typed runs.

The REST API is a typed assembly surface only. It can start, resume, inspect, replay, and render
certified typed runs through `mfm-app`, `mfm-runtime`, `mfm-store`, and typed artifact storage. It
accepts only certified typed run authority.

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
- `DATABASE_URL`: Postgres URL for the certified run store (required)
- `MFM_REST_ROLE`: process role (`live` or `read`; default `live`)
- `MFM_RUNTIME_CONFIG_FILE`: optional runtime config file path for live capability-backed runs

The REST API validates the PostgreSQL schema on startup and does not create or
alter tables. Apply the `mfm-storage-postgres` migrations before starting
the server:

```bash
export DATABASE_URL="postgresql://postgres:postgres@localhost:5432/mfm_test"
cargo sqlx migrate run --source crates/storages/postgres/migrations
```

Use a fresh or explicitly reset database for this typed Postgres baseline. There
is no downgrade migration; rollback to another branch requires resetting the
database or schema to that branch's expected baseline. Filesystem artifact roots
outside the typed run store are not read or migrated by the REST API.

### Process roles

| Role | Store connector | Serves |
| --- | --- | --- |
| `live` (default) | live store connect | start/resume, status/stream/replay, public fact queries |
| `read` | evidence-only read store connect | status, stream, list, replay, public-output, public fact queries |

Only live start/resume requires `MFM_REST_ROLE=live`. Public fact queries are deterministic evidence
reads and are available from either role.

Public fact queries return descriptor-filtered Platform facts with deterministic evidence metadata;
they do not depend on process-local signing material.

REST startup does not load or validate `MFM_RUNTIME_CONFIG_FILE`; malformed or missing runtime
config is reported only when a live start/resume request needs the affected capability family.

## API

All responses are JSON envelopes:

- success: `{"status":"success","data": ...}`
- error: `{"status":"error","error":{"code":"...","message":"..."}}`

Endpoints:

- `GET /v1/health`
- `GET /v1/ready`
- `GET /v1/facts/kinds`
- `GET /v1/facts/kinds/:kind`
- `GET /v1/facts/:kind?shape=<shape>&order=<ordering>&field=<field>&limit=<n>`
- `GET /v1/facts/:kind/latest?shape=<shape>&order=<ordering>&field=<field>`
- `GET /v1/facts/ref/:public_ref`
- `GET /v1/runs?cursor=<opaque>&limit=<n>&wait_ms=<n>`
- `POST /v1/runs/start`
- `POST /v1/runs/:run_id/resume`
- `POST /v1/runs/:run_id/manual-resolution`
- `GET /v1/runs/:run_id/status`
- `GET /v1/runs/:run_id/stream?from_seq=1&to_seq=<optional>`
- `POST /v1/runs/:run_id/replay`
- `GET /v1/runs/:run_id/public-output/:schema_id`

Probe semantics:

- `/v1/health`: liveness only (process is running)
- `/v1/ready`: run store probe must succeed

## Public Facts

The `/v1/facts/...` routes expose descriptor-scoped public Platform facts only. REST decodes path
and query parameters into `mfm-app` public fact DTOs; descriptor resolution, field validation,
query compilation, public ref resolution, and non-public filtering are owned by the app/facts
services.

Routes:

- `GET /v1/facts/kinds`: list public fact kinds.
- `GET /v1/facts/kinds/:kind`: describe public descriptors for one kind.
- `GET /v1/facts/:kind`: query public fact history for one kind.
- `GET /v1/facts/:kind/latest`: query one latest fact for one kind. This is `limit = 1` plus the
  explicit `order` parameter; there is no mutable latest row.
- `GET /v1/facts/ref/:public_ref`: resolve an opaque public fact reference.

Query parameters for `GET /v1/facts/:kind` and `GET /v1/facts/:kind/latest`:

- `shape`: optional descriptor shape selector, matching a descriptor schema id or compatibility
  group.
- `order`: required descriptor ordering policy name.
- `field`: required and repeatable returnable field id.
- `subject`: repeatable subject predicate, such as `network=bitcoin-mainnet` or `subject.network.eq=bitcoin-mainnet`.
- `result`: repeatable result predicate, such as `amount_sat.gt=1000`.
- `where`: repeatable fully qualified predicate, such as `subject.bitcoin_network.eq=main`.
- `limit`: optional non-zero limit for `GET /v1/facts/:kind`; ignored by `/latest`, which always
  uses one result.

Predicate operators use suffixes: `.eq`, `.lt`, `.lte`, `.gt`, and `.gte`. Values are inferred as
booleans, signed or unsigned integers, decimal strings, or plain strings. Explicit typed values can
use prefixes: `string:`, `bool:`, `i64:`, `u64:`, `timestamp:`, `decimal:`, or `digest:`.

Examples:

```bash
curl -s "http://127.0.0.1:3001/v1/facts/kinds"

curl -s "http://127.0.0.1:3001/v1/facts/kinds/wallet.balance"

curl -s "http://127.0.0.1:3001/v1/facts/wallet.balance/latest?shape=mfm.wallet.balance.v1&order=result:block_number:desc&field=result.amount_sat&subject=chain%3Dbitcoin&subject=asset_ref%3Dbtc"

curl -s "http://127.0.0.1:3001/v1/facts/weather.observation?shape=mfm.weather.observation.v1&order=metadata:observed_at:desc&field=result.temperature_celsius_milli&subject=country%3DIE&result=temperature_celsius_milli.lt%3D0&limit=20"

curl -s "http://127.0.0.1:3001/v1/facts/ref/pfr_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
```

Query response shape:

```json
{
  "facts": [
    {
      "public_ref": "pfr_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      "fact_kind": "wallet.balance",
      "descriptor": {
        "descriptor_schema_id": "schema:mfm.wallet.balance.v1:...",
        "subject_schema_id": "schema:mfm.wallet.balance.subject.v1:...",
        "response_schema_id": "schema:mfm.wallet.balance.response.v1:..."
      },
      "recorded_at": "2026-07-02T00:00:00.000000Z",
      "observed_at": "2026-07-02T00:00:00.000000Z",
      "fields": [
        {
          "field_id": "result.amount_sat",
          "path": "result.amount_sat",
          "source": "result",
          "value_type": "unsigned_integer",
          "value": { "type": "unsigned_integer", "value": 1000 },
          "unit": "sat",
          "scale": null
        }
      ]
    }
  ],
  "next_cursor": null
}
```

Privacy and error contract:

- Public REST routes query only `audience = Platform` with the default public scope.
- `Control` and `RunPrivate` facts are absent from kind discovery, descriptor descriptions,
  queries, and exact-ref lookup.
- Public outputs never include internal fact refs, descriptor hashes, fact keys, subject material,
  subject hashes, response artifacts, request/response hashes, artifact ids, artifact evidence
  hashes, raw run ids, event ids, source sequences, event ordinals, adapter routing, or capability
  routing details.
- Unknown public refs and refs that resolve only to non-public facts return the same redacted
  `FactNotFound` class.
- Malformed query parameters return `InvalidQuery`; malformed predicates return
  `FactPredicateInvalid`; missing `order` returns `FactOrderingMissing`; missing `field` returns
  `FactReturnFieldMissing`; zero limits return `FactQueryLimitInvalid`.

## List Runs

`GET /v1/runs` returns one observation-only page. Without `cursor`, it lists the latest observed
run rows bounded by one sealed frontier. With `cursor`, it returns changes after that cursor and a
fresh `next_cursor`. A long-poll timeout is a successful empty page.

Run observation cursors are opaque server-issued tokens. In v1 they are epoch-bound and do not have
wall-clock TTL or garbage collection: a cursor remains valid while its durable token row, cursor
version, cursor key id, and store epoch still match the live store. Unknown, missing, retired-key,
or stale-format cursors return `InvalidCursor`. Store epoch mismatches return `CursorExpired`; clients
recover by listing again without a cursor.

Response shape:

```json
{
  "next_cursor": "opaque",
  "runs": [
    {
      "run_id": "run:sha256-jcs-v1:...",
      "head_seq": 3,
      "observed_status": "started",
      "started_at": "2026-01-01T00:00:00.000000Z",
      "updated_at": "2026-01-01T00:00:01.000000Z",
      "completed_at": null
    }
  ]
}
```

## Start A Run

`POST /v1/runs/start` accepts one exact entry-point id and a strict JSON request. The request uses
the same catalog-reference contract as the CLI; the REST layer delegates exact resolution,
planning, certification, admission, and verified rendering to app assembly.

Request shape:

```json
{
  "entry_point": "mfm.evm.contract/deploy@1",
  "request": {
    "context": {
      "name": "acme/contract-context",
      "digest": "content:sha256-jcs-v1:..."
    },
    "deploy_action": {
      "name": "acme/deploy-action",
      "digest": "content:sha256-jcs-v1:..."
    }
  },
  "invocation_key": "optional-key"
}
```

Request notes:

- `entry_point` is required and must be one exact id, including namespace and version.
- `request` is required, must be a JSON object, and is rejected when it contains unknown fields.
- Requests contain exact catalog references (`name` and `digest`); name-only selection and latest
  resolution do not exist.
- The exact public ids are returned by the CLI `ops list` command; BTC/EVM balance collection and
  the portfolio snapshot objective are internal during the anchored receipt cutover.
- Normal start derives the typed run id from certified run identity material: certified spec hash,
  store scope, and a required invocation key digest.
- `invocation_key` is optional at the API boundary. Supplying it makes retries target the same run.
  When omitted, the app mints a fresh opaque invocation key before deriving `run_id`. The raw key is
  not persisted; only a domain-separated digest enters run identity material.
- `run_id` is not a start field.

The response is `{"outcome": "...", "run": ..., "active_run_id": "...", "public_output": ...}`
inside the standard success envelope. Fresh admissions report `admitted`. Duplicate starts for the
same certified run identity report `attached` without driving. If another process holds the
execution lane for the same base work identity, start reports `already_active` with
`active_run_id` and omits `run`.
`public_output` is present when the run completes while driving and the op exposes a public output
schema id.

EVM contract deploy:

```json
{
  "entry_point": "mfm.evm.contract/deploy@1",
  "request": {
    "context": {"name": "acme/context", "digest": "content:sha256-jcs-v1:..."},
    "deploy_action": {"name": "acme/deploy", "digest": "content:sha256-jcs-v1:..."}
  }
}
```

EVM contract configure:

```json
{
  "entry_point": "mfm.evm.contract/configure@1",
  "request": {
    "context": {"name": "acme/context", "digest": "content:sha256-jcs-v1:..."},
    "import_deployed": {"name": "acme/import-deployed", "digest": "content:sha256-jcs-v1:..."},
    "configure_action": {"name": "acme/configure", "digest": "content:sha256-jcs-v1:..."}
  }
}
```

EVM contract validate:

```json
{
  "entry_point": "mfm.evm.contract/validate@1",
  "request": {
    "context": {"name": "acme/context", "digest": "content:sha256-jcs-v1:..."},
    "import_configured": {"name": "acme/import-configured", "digest": "content:sha256-jcs-v1:..."},
    "validate_action": {"name": "acme/validate", "digest": "content:sha256-jcs-v1:..."}
  }
}
```

EVM contract lifecycle:

```json
{
  "entry_point": "mfm.evm.contract/lifecycle@1",
  "request": {
    "context": {"name": "acme/context", "digest": "content:sha256-jcs-v1:..."},
    "deploy_action": {"name": "acme/deploy", "digest": "content:sha256-jcs-v1:..."},
    "configure_action": {"name": "acme/configure", "digest": "content:sha256-jcs-v1:..."},
    "validate_action": {"name": "acme/validate", "digest": "content:sha256-jcs-v1:..."}
  }
}
```

EVM contract entry configs carry certified lifecycle context, action specs, import specs, artifact
evidence refs, and non-secret signer intent. The runtime resolves the certified context network id
and action signer ref through `MFM_RUNTIME_CONFIG_FILE`. Process-local RPC endpoints, auth headers,
keystore paths, unlock files, and private material never belong in the entry-point config.
Source-run EVM imports use `kind: "from_mfm_run"` with both `source` and required retained
`evidence`; identifiers, public output JSON, projection rows, and raw lifecycle payloads are not
accepted as import authority.
The full lifecycle contract is documented in
[`../../docs/evm-contract-lifecycle.md`](../../docs/evm-contract-lifecycle.md).

Stable launch error codes:

- `InvalidJson`: the request envelope is not accepted by the route schema.
- `EntryPointNotFound`: the exact entry-point id is not registered.
- `EntryPointRequestInvalid`: the request does not match the selected strict schema.
- `CatalogStoreUnavailable`: catalog authority is unavailable during new-run preparation.
- `CatalogValueNotFound`: an exact referenced catalog value is missing.
- `CatalogValueTypeInvalid`: a catalog row does not decode as the referenced type.
- `CatalogValueCanonicalMismatch`: a catalog row fails canonical byte/digest verification.
- `CatalogValueValidationFailed`: a catalog value fails semantic validation.
- `EvmContractPlanFailed`: EVM contract entry-point planning failed.
- `EntryPointCertificationFailed`: the planned spec failed app-owned certification.
- `LaunchRunnerUnavailable`: the verified spec references a state descriptor without a production
  runner binding.

## Resume, Replay, And Public Output

Resume:

```bash
curl -s -X POST "http://127.0.0.1:3001/v1/runs/$RUN_ID/resume"
```

Manual resume is the v1 recovery trigger for a run left with an open execution claim,
side-effect uncertainty, or a resumable frontier. Automatic dead-driver takeover and background
worker-pool dispatch are deferred. Receipt-level side-effect terminalization is final-at-risk: a
later reorg can invalidate the published receipt-derived output, so operations that need reorg
safety must use certified `Finalized(depth)` verification.

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
    }
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
against the compiled certification registry, compare their evidence to `RunAdmitted`, and rebuild
stream evidence before constructing runtime, replay, or render authority. Rendered public-output JSON
is an output/cache surface only.

Typed run responses expose semantic status through `run_mode`. `run_mode` is one of `forward`,
`remediating`, `manual_blocked`, `completed`, `compensated`,
`manually_resolved`, or `failed_without_acdc_claim`. The nested `saga` object reports the certified
policy, derived per-forward-ledger obligations, linked remediation ledgers, manual-block reason and
manual authorization requirements when applicable, terminal resolution claim when present,
projected resource ledgers with declared claim/touched-set evidence and resource-key digests, and
active exclusive lane holders referenced by the target run's persisted live side-effect ledgers.
Status does not serialize raw resource keys, unrelated global lane holders, or scheduler waiters
that blocked before appending lane evidence.
`attempt_dispositions` reports committed attempt-level lifecycle status separately from `run_mode`;
each entry has `node_id`, `attempt_id`, `disposition` (`started`, `completed`, `failed`, or
`interrupted`), and status-specific fields such as `attempt_no`, `retryable`, `error_code`, or
`output_cell_id`. Failed attempt event references also expose the redaction-safe `error_code`.
`scheduler_status` is read-only `observed` for `GET /v1/runs/:run_id/status`; start/resume responses
set it to `advanced`, `blocked`, `public_output_projected`, `execution_claim_busy`, or
`execution_claim_lost` according to the app dispatch loop and claim-coordination outcome.
Manual authorization requirements include the required evidence schema, signing scheme, authority id,
allowed operator public identities or a safe summary, and quorum. They never expose signer runtime
sources, keystore paths, password paths, passwords, or other secrets.

Docs:

- Design contract: [`../../docs/design.md`](../../docs/design.md)
- Architecture overview: [`../../docs/architecture.md`](../../docs/architecture.md)
- CLI docs (parallel surface): [`../cli/README.md`](../cli/README.md)
