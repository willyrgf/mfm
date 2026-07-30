# MFM CLI

The `mfm` command is a thin local transport over `mfm-app`. It parses input, invokes one facade
operation, and renders either human-readable text or exact public JSON. It does not construct store
authorities, inspect journal objects, or retain authentication sessions.

Run it inside the pinned development environment:

```sh
nix develop -c cargo run -p mfm -- --help
```

## Output and authentication

`--output-format text|json` selects presentation; `MFM_OUTPUT_FORMAT` supplies the default. Run
successes in JSON mode are the corresponding public DTO directly. Errors use the shared redacted
envelope:

```json
{"status":"error","error":{"code":"...","message":"..."}}
```

Every `run` command requires the global option:

```text
--access-token-file PATH
```

The file is bounded to 64 KiB. The CLI removes at most one final LF or CRLF and transfers the
remaining opaque bytes in a consuming, zeroizing credential. There is no raw token argument,
tenant option, tenant environment variable, cookie, or session.

## Recoverability-v1 commands

Entry-point discovery itself requires no credential:

```sh
mfm ops list [--database-url URL] [--runtime-config PATH]
```

The command still bootstraps the same fully qualified production `Application` used by run
operations—database, runtime configuration, executable self-attestation, support admission,
registry sealing, access policy, and deployment writer-fence provider—and then reads its cached
entry-point slice. It is not a static or no-store catalog.

The exact run command tree is:

```text
mfm run admit ENTRY_POINT_ID --invocation-identity UUID --target TARGET
mfm run drive RUN_ID
mfm run show RUN_ID
mfm run replay RUN_ID --mode verify
mfm run replay RUN_ID --mode reproduce|compare-current \
  --portable-export PATH --portable-export-ref-file PATH
mfm run trace RUN_ID [--cursor CURSOR] [--limit 1..500]
mfm run audit RUN_ID [--cursor CURSOR] [--limit 1..500]
mfm run export RUN_ID --kind semantic|audit --output PATH --ref-output PATH
```

The qualified catalog contains `mfm.portfolio/snapshot@1` and
`mfm.evm/submit-transaction@1`; both use the same generic `run admit ... --target TARGET`
transport shape. There is no transaction-specific CLI submission or signing path.

There is no CLI health/readiness alias. Process readiness is the deployment REST adapter's bounded
PostgreSQL writable-lineage probe; the CLI does not substitute an EVM, DNS, provider, or semantic
callback check.

Place `--access-token-file` before or after the subcommand as allowed by clap. Admission requires a
caller-generated lower-case, hyphenated UUIDv4; the CLI never invents one. Non-verify replay opens
one caller-held semantic export as an affine reader without a total-byte cap and reads a strictly
canonical current-stream `ContentRef` sidecar bounded to 4,096 bytes; both flags are forbidden for
verify. Export preflights both distinct new paths before calling the app, rejects target or parent
symlinks and path aliases, then streams the response into secure mode-0600 same-directory
temporary files. It flushes and synchronizes both files, publishes each without overwrite, and
synchronizes both parent directories. Copy failure, cancellation, or a race on either target
removes temporary files and rolls back any first published target. It emits no success payload
that could be mistaken for the exported stream.

Missing or forbidden replay-artifact flags and malformed, noncanonical, legacy-schema, or
mismatched artifact input return `ReplayArtifactInvalid`. A sidecar limit violation returns
`ReplayArtifactTooLarge`; stream read/write and durability failures return the fixed
`ExportWriteFailed` contract without exposing a path or backend diagnostic.
Candidate recorded-history, execution, and comparison-integrity failures return the single
`ReplayVerificationFailed` contract. An unavailable sealed current candidate returns
`RuntimeCatalogUnavailable`; a missing exact historical executable remains a successful
`unavailable` reproduction result.

JSON audit output is the same exact public page DTO as REST. Its entry exposes the reviewed
authorization, observation, capability binding and operation, request/result/failure,
`non_domain_failure`, effect, and sole `delivery_audit_ref`; `delivery_audit_terminal` is the
presentation-only verified pending/terminal annotation.

Removed surfaces do not have aliases: there is no `facts`, `setup`, run list/watch, `start`,
`resume`, `status`, `stream`, `manual-resolution`, `public-output`, arbitrary object reader, raw
store authority, or tenant selector.

## Standalone authority qualification

The repository binary has no deployment-owned authoritative-writer fence. Consequently, `ops
list` and, after reading the required credential, every `run` operation fail the shared production
bootstrap with `AuthoritativeWriterFenceUnavailable`.

This is intentional. A deployment uses a separately qualified process transport around a fully
composed `mfm_app::Application`, supplying its access policy, run-store writer fence, exact wallet
deployment, and independent executor writer-generation fence. The repository CLI exposes no
authority-bearing constructor, and no fence can be enabled by a CLI flag.

## Retained operational commands

`mfm keystore import|list|delete` remains separate from the run facade. It manages local keystore
entries only. There is no standalone signing command; transaction signing is available solely
inside the qualified generation-guarded wallet executor.

Keystore secret inputs are captured through protected prompts, stdin, or explicitly selected
files. Do not place private keys, mnemonics, passphrases, bearer credentials, or provider
authorization values in command arguments, logs, configured values, or runtime configuration.

The exact application/transport contract is
[`docs/recoverability-app-surface-v1.md`](../../docs/recoverability-app-surface-v1.md).
