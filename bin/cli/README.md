# MFM CLI

Transport-only command line interface over `mfm_app::Application` plus local keystore services.

## Commands

```text
mfm ops list

mfm run admit <entry-point-id> --invocation-identity <uuid> --target <target> [--caller-submission-token <token>]
mfm run drive <run-id>
mfm run show <run-id>
mfm run replay <run-id> --mode verify
mfm run trace <run-id> [--cursor <cursor>] [--limit <n>]
mfm run audit <run-id> [--cursor <cursor>] [--limit <n>]
mfm run export <run-id> --kind semantic|audit --output <new-path> --ref-output <new-sidecar-path>

mfm keystore import ...
mfm keystore list ...
mfm keystore delete ...
```

Use `mfm <command> --help` for exact flags. Run requests are bounded one-action calls; the CLI does
not loop a run to completion. `ops list` returns the application registry's exact published entry
contracts rather than a hard-coded list.

## Input and output

Run commands construct or decode the current strict application DTOs. JSON mode prints one
canonical reviewed response to stdout. Text mode renders only the corresponding reviewed fields.
Errors exit non-zero. Their code is non-empty and bounded to 128 UTF-8 bytes; their reviewed
message is non-empty and bounded to 4096 UTF-8 bytes. JSON uses the frozen
`mfm.error-response.v1` envelope and may include secret-free `runtime_fault` attribution. Text
prints only code and message. Private implementation identity, diagnostics, credentials,
endpoints, provider text, and database details are never printed.

Admission accepts the current public selectors for `mfm.portfolio/snapshot@1` and
`mfm.evm/submit-transaction@1`. EVM admission requires `--caller-submission-token`; portfolio
admission rejects it. The token is bounded ordinary-caller idempotency input and is scoped by the
credential-derived tenant and stable principal. It is not configured deployment identity. The
invocation identity is a separate caller-generated canonical UUIDv4, so retries may use the same
submission token in distinct runs while converging on the same permanent EVM intent. The returned
run id is the deterministic typed run identity for store, tenant, operation, and invocation.

Replay is recorded-history verification only. Export writes the current bounded frame stream only to
a safely created new regular file and a content-reference sidecar; old export bytes are rejected.

## Credentials

Protected application calls read credentials through the reviewed CLI input path; do not place
credentials in command arguments or logs. Each app call re-authenticates and re-authorizes its
exact purpose. Admission authorization includes the selected configured target. Tenant and
authenticated principal are derived from the credential, never from CLI fields or configured
values. A run id, cursor, export, or content reference is not bearer authority.

Keystore commands are separate from the run facade. Secret inputs use protected prompts, stdin, or
explicit files and are zeroized by the lower service. There is no standalone signing command.

## Standalone authority

The repository binary has no deployment-owned authoritative RunHistory fence or sealed wallet
target/session provider. Therefore `ops list` and every `run` operation fail closed with
`AuthoritativeWriterFenceUnavailable`. No CLI flag can mint that authority.

A deployment embeds or wraps the transport around a fully composed `mfm_app::Application`.
Keystore commands remain locally usable because they do not construct run or wallet authority.

See [the application surface contract](../../docs/recoverability-app-surface-v1.md).
