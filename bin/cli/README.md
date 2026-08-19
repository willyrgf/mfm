# MFM CLI

`mfm_cli` is the one current transport for driving a Portfolio snapshot against a live EVM route and
the durable PostgreSQL run history. It parses one JSON configuration file, renders one view format,
and owns no session, no cached state, and no output DTO.

## Commands

```
mfm_cli init     --config <PATH>
mfm_cli snapshot --config <PATH> --run-id <RUN_ID>
mfm_cli show     --config <PATH> --run-id <RUN_ID>
```

- `init` installs the run-history schema when the store is empty and verifies an existing
  installation. It is idempotent, never modifies an incompatible installation, and prints nothing on
  success.
- `snapshot` plans, admits, and progresses one Portfolio snapshot under the supplied RunId. Repeating
  it with the same RunId and configuration re-admits the retained run and renders the same bytes.
- `show` reads one retained run without progressing it.

`--run-id` is the complete explicit `run:sha256-jcs-v1:<64 hex>` identity. The CLI never derives,
defaults, or invents a RunId.

`show` builds the same fully bound composition as `snapshot`, so it needs `rpc_url_env` too: Program
association pre-resolves every implementation and Read callback before the fold reads one frame.

## Configuration

One JSON file. Every level rejects unknown fields. URLs are supplied only through environment
variable NAMES, never as values in the file.

```json
{
  "portfolio": { "portfolio_id": "portfolio-example", "quotes": ["usd"],
    "collections": [{ "correlation": "native-collection",
      "request": { "sources": [{ "source_id": "wallet.native", "chain_id": 1337,
        "address": "0x70997970c51812dc3a010c7d01b50e0d17dc79c8", "token": null }],
        "decimals": 18 } }] },
  "selector": { "target": "portfolio-example", "quote": "usd" },
  "evm": { "chain_id": 1337, "endpoint_id": "reth-dev", "rpc_url_env": "MFM_E2E_RPC_URL" },
  "store": { "database_url_env": "MFM_E2E_DATABASE_URL" }
}
```

`portfolio` and `selector` deserialize directly into the Portfolio domain's checked types, which own
all domain validation.

`endpoint_id` is the only route material: the same name derives the same route reference, the same
Program, and the same run identity in every process. Changing the URL VALUE behind `rpc_url_env`
never changes any of them; changing `endpoint_id` changes all of them.

**One EVM route per configuration file.** The CLI builds exactly one route from the `evm` block, so a
configuration whose collections use any chain id other than `evm.chain_id` cannot be planned and
fails before admission.

## Output

`snapshot` and `show` write the identical view rendering to stdout:

```
run_id=run:sha256-jcs-v1:<64 hex>
head_sequence=<u64>
head_digest=content:sha256-v1:<64 hex>
state=runnable|succeeded|failed
<exact canonical retained bytes, one line, only for succeeded and failed>
```

The same retained run therefore renders byte-identically across independent invocations.

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | `init` succeeded, or a view was rendered with `state=succeeded` |
| 1 | A view was rendered with `state=runnable` or `state=failed` |
| 2 | No view: usage, configuration, environment, store, provider, or Application failure |

## Redaction

Every failure prints exactly one reviewed line to stderr:

```
error: configuration is invalid
error: environment variable <NAME> is not set
error: run id is invalid
error: postgres store is unavailable
error: postgres store is incompatible
error: evm provider transport could not be constructed
error: configuration cannot be planned for the configured evm route
error: runtime operation failed: <reviewed runtime failure>
```

Environment variable NAMES appear. A database URL, an RPC URL, a credential, or an unreviewed
provider or driver detail never appears in stdout, stderr, or an exit code.
