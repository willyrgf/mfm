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

mfm_cli [--deployment <PATH>] [--output text|json] postgres init \
    --admin-locator-env <NAME>
mfm_cli [--deployment <PATH>] [--output text|json] binding list

mfm_cli [--deployment <PATH>] [--output text|json] config import <NAME> --from <PATH|->
mfm_cli [--deployment <PATH>] [--output text|json] config list
mfm_cli [--deployment <PATH>] [--output text|json] config delete <NAME> --digest <DIGEST>

mfm_cli [--deployment <PATH>] [--output text|json] run start --config <NAME> \
    --config-digest <DIGEST> [--run-id <RUN_ID>]
mfm_cli [--deployment <PATH>] [--output text|json] run progress --run-id <RUN_ID>
mfm_cli [--deployment <PATH>] [--output text|json] run show --run-id <RUN_ID>
mfm_cli [--deployment <PATH>] [--output text|json] run list [--after <RUN_ID>] [--limit <N>]
```

`entry-point list` is static and rejects `--deployment`. Every other command loads the override or
the conventional XDG/HOME `deployment.toml`. `postgres init` additionally resolves the checked admin
locator name and retains no administrative handle after provisioning. The complete deployment
grammar, bounds, and example are documented by [`mfm-app`](../../crates/app/README.md).

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
`config_digest`, and `entry_point`. Start prints those fields before the run fields. Terminal run
text includes `contract_ref`, `value_ref`, and exact canonical `value`.

`--output json` is the stable automation surface shared with REST. It preserves the documented
Application models: generic item lists, config revision summaries, start results, mechanical run
pages, and the full tagged RunView. Terminal values occupy raw JSON positions rather than quoted
strings. Successful bodyless operations emit `{}` in JSON mode
and nothing in text mode.

Ordinary JSON errors are exactly `{"code":"...","message":"..."}`. An ambiguous run append adds
the shared `recovery` sum. Text mode prints `error: <message>` plus the same recovery RunId and
selected config summary when present. Locator values, URLs, credentials, config bodies, and provider
details are never rendered.

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | A non-run command succeeded, or a run view is `succeeded`. |
| 1 | A run view is `runnable` or durably `failed`. |
| 2 | No run view: usage, checked input, composition, entropy, request, or output failure. |

The old `init`, `snapshot`, and `show --config` grammar and combined configuration file do not
exist. Keystore administration and transaction submission remain outside this surface.

The managed `client-e2e` task builds both binaries explicitly, admits an exact historical run through
a deliberately unavailable live Read, deletes its config revision, resumes it through the Unix-socket
REST listener against Reth, validates the complete snapshot, and reloads the same durable RunView
through this JSON surface. It then reimports the same revision, starts a fresh CLI-generated run, and
requires an identical semantic result from the independent execution. Both renderers also match the
frozen start/progress recovery envelopes in `docs/contracts/client-surface/`.
