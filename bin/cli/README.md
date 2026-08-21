# MFM CLI

`mfm_cli` is the command-line rendering of the typed Application client surface. It owns bounded
argv/file/stdin parsing, optional client-side RunId generation, text/JSON rendering, and CLI-only
schema provisioning. It does not parse deployment TOML, resolve locator environments, construct
providers, plan domain Programs, or interpret run history.

## Commands

```text
mfm_cli [--output text|json] entry-point list

mfm_cli [--deployment <PATH>] [--output text|json] store init \
    --admin-store-locator-env <NAME>
mfm_cli [--deployment <PATH>] [--output text|json] binding list

mfm_cli [--deployment <PATH>] [--output text|json] config import <NAME> --from <PATH|->
mfm_cli [--deployment <PATH>] [--output text|json] config list [--cursor <C>] [--limit <N>]
mfm_cli [--deployment <PATH>] [--output text|json] config show <NAME>
mfm_cli [--deployment <PATH>] [--output text|json] config delete <NAME> --digest <DIGEST>

mfm_cli [--deployment <PATH>] [--output text|json] run start --config <NAME> \
    [--config-digest <DIGEST>] [--run-id <RUN_ID>]
mfm_cli [--deployment <PATH>] [--output text|json] run progress --run-id <RUN_ID>
mfm_cli [--deployment <PATH>] [--output text|json] run show --run-id <RUN_ID>
mfm_cli [--deployment <PATH>] [--output text|json] run list [--cursor <C>] [--limit <N>]
```

`entry-point list` is static and rejects `--deployment`. Every other command loads the override or
the conventional XDG/HOME `deployment.toml`. `store init` additionally resolves the checked admin
locator name and retains no administrative handle after provisioning.

`config import` reads at most 256 KiB plus one byte from a file or stdin. The stored document is a
complete tagged execution config; `run start` accepts no entry-point or inline document. Without
`--config-digest`, start selects the revision currently bound to the name. Supplying it asserts the
exact current digest.

Without `--run-id`, only the CLI obtains exactly 32 bytes from the OS cryptographic random source
and passes them to the frozen pure derivation helper. Entropy failure is
`run_id_generation_failed`; there is no time, PID, counter, environment, provider, or existence
fallback. Explicit RunIds bypass generation and support deterministic retries.

## Output

`--output text` is the default human presentation. Config summaries include `config_name`,
`config_digest`, and `entry_point`. Start prints those fields before the run fields. Terminal run
text includes `contract_ref`, `value_ref`, and exact canonical `value`.

`--output json` is the stable automation surface shared with REST. It preserves the documented
Application models: entry-point and binding lists, config summaries/documents/pages, start results,
mechanical run pages, and the full tagged RunView. Canonical config and terminal values occupy raw
JSON positions rather than quoted strings. Successful bodyless operations emit `{}` in JSON mode
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

The managed `rest-e2e` task builds both binaries explicitly and compares this JSON surface with the
Unix-socket REST rendering over the same stored configs and retained runs. Both renderers also match
the frozen start/progress recovery envelopes in `docs/contracts/client-surface/`.
