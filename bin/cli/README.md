# MFM CLI

`mfm_cli` is the command-line rendering of the typed Application client surface. It owns bounded
argv/file/stdin parsing, text/JSON rendering, and CLI-only schema provisioning. It does not parse
deployment TOML, resolve locator environments, construct providers, plan domain Programs, or
interpret run history.

The CLI adds no authentication or authorization layer. Process execution, environment access, and
database credentials belong to the enclosing deployment.

## Commands

```text
mfm_cli [--output text|json] entry-point list
mfm_cli [--output text|json] inspect list

mfm_cli [--deployment <PATH>] [--output text|json] postgres init \
    --admin-locator-env <NAME>
mfm_cli [--deployment <PATH>] [--output text|json] binding list

mfm_cli [--deployment <PATH>] [--output text|json] config import <NAME> --from <PATH|->
mfm_cli [--deployment <PATH>] [--output text|json] config publish-enrichment <NAME> --run-id <RUN_ID>
mfm_cli [--deployment <PATH>] [--output text|json] config list
mfm_cli [--deployment <PATH>] [--output text|json] config delete <NAME> --digest <DIGEST>

mfm_cli [--deployment <PATH>] [--output text|json] run start --config <NAME> \
    --config-digest <DIGEST> [--run-id <RUN_ID>]
mfm_cli [--deployment <PATH>] [--output text|json] run progress --run-id <RUN_ID>
mfm_cli [--deployment <PATH>] [--output text|json] run show --run-id <RUN_ID>
mfm_cli [--deployment <PATH>] [--output text|json] run list [--after <RUN_ID>] [--limit <N>]
```

`entry-point list` and `inspect list` are static and reject `--deployment`. Every other command loads
the override or the conventional XDG/HOME `deployment.toml`. `postgres init` additionally resolves
the checked admin locator name and retains no administrative handle after provisioning. The complete
deployment grammar, bounds, and example are documented by
[`mfm-app`](../../crates/app/README.md).

`inspect list` is the crude developer inventory of every entry point, reusable Operation, and
Pure/Read State admitted by the compiled product composition. It does not expand an Operation or
load a configuration. IDs and kinds are stable machine fields; descriptions are non-semantic
human-facing prose. Component inspection is intentionally CLI-only.

`config import` reads at most 256 KiB plus one byte from a file or stdin. It creates one immutable
name/digest revision or leaves identical retained bytes unchanged. Different digests under one name
remain independent. `config delete` idempotently removes only the exact name/digest pair. The stored
document is a complete tagged execution config; `run start` accepts no entry-point or inline
document and always requires one exact retained digest.

Without `--run-id`, the CLI uses the shared Application client primitive that obtains exactly 32
bytes from the OS cryptographic random source and applies the frozen derivation. Entropy failure is
`run_id_generation_failed`; there is no time, PID, counter, environment, provider, or existence
fallback. Explicit RunIds bypass generation and support deterministic retries.

## Output

`--output text` is the default human presentation. Listed config revisions include `config_name`,
`config_digest`, and `entry_point`. Start prints those fields before the run fields. Successful run
text includes `contract_ref`, `value_ref`, and exact canonical `value`. Failed runs include
`value_ref` and the canonical `report`; runnable/pending views expose position, reason or EffectId.

`--output json` is the stable automation surface shared with REST. It preserves the documented
Application models: generic item lists, config revision summaries, start results, mechanical run
pages, and the full tagged RunView. Terminal values occupy raw JSON positions rather than quoted
strings. Successful bodyless operations emit `{}` in JSON mode
and nothing in text mode.

Component inspection JSON is an `ItemList` whose items contain `kind`, `id`, and `description`.
Text renders the same three fields for each component.

Ordinary JSON errors are exactly `{"code":"...","message":"..."}`. The Application-owned client
error serializer adds `recovery` plus `last_observed` for an ambiguous append, or the tagged
`invocation` detail for an execution-stopped call or stopped pending-Effect recovery. A null
last observation means no qualified head is known. Text prints the same recovery identity and
invocation details after `error: <message>`. These errors do not claim a durable terminal state. Locator
values, URLs, credentials, config bodies, and provider details are never rendered.

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | A non-run command succeeded, or a run view is `succeeded`. |
| 1 | A run view is `runnable`, `effect_pending` or durably `failed`. |
| 2 | A call failed: usage, input, composition, entropy, request, stopped invocation or output failure; any last observed view is historical. |

The old `init`, `snapshot`, and `show --config` grammar and combined configuration file do not
exist. Keystore administration and transaction submission remain outside this surface.

The managed `client-e2e` task builds both binaries explicitly, admits an exact historical run by interrupting
an in-flight live Read, deletes its config revision, resumes it through the Unix-socket
REST listener against Reth, validates the complete snapshot, and reloads the same durable RunView
through this JSON surface. It then reimports the same revision, starts a fresh CLI-generated run, and
requires an identical semantic result from the independent execution. Both renderers also match the
frozen start/progress recovery envelopes in `docs/contracts/client-surface/`.

Candidate discovery is selected by a `mfm.portfolio/enrich@1` configuration and uses ordinary
`run start/show/progress`. `config publish-enrichment` explicitly publishes a successful result as
a snapshot revision. Repetition returns the same revision without discovery. Start of an admitted
RunId with the same selection works after config deletion; a different revision conflicts.

Size-limit invocation failures use `size_limit_exceeded` and include
`invocation.size_limit` with `resource`, `actual`, and `limit` (bytes, or frames for
`frame_count`). The last observation remains historical; oversized inline reports do not
append a terminal conclusion or discard pending Effect authority. Capacity arithmetic overflow
uses `capacity_arithmetic_overflow` without fabricated measurements. CLI uses exit 2; REST uses
422 for these stopped invocations. Response payloads retain the shared full inline report; request
body limits do not impose a response-size limit.

Pending Effect views include `latest_failure`: null before the first audited failure, otherwise
`error`, `state_context` (qualified value objects) and `decision` (`{"kind":"retry"}` or
`{"kind":"stop","reason":"requested"}`, with other reviewed stop codes). Every acknowledged
pending operational outcome advances the durable head. Retry preserves the command/EffectId and
spends recovery allowance; Stop ends the invocation while explicit progress may resume it.
The `pending_failures` size resource identifies exhausted admitted audit capacity; it rejects
further provider entry and retains unresolved command authority. The removed `nonrecoverable`
stop code is rejected with the superseded wire contracts. Cancellation can interrupt a physical
attempt before its result is recorded; the audit covers acknowledged qualified failures.
