# MFM CLI Documentation

## Overview

The `mfm_cli` is the command-line interface for the MFM toolkit. It provides a user-friendly and scriptable way to interact with MFM modules, including keystore management, public fact discovery/query, and an experimental `run` subcommand for starting/resuming/inspecting runs. The CLI is built using the `clap` crate for robust argument parsing and command structure.

Run the packaged CLI with `nix run .#mfm -- <ARGS>`, for example:
- `nix run .#mfm -- keystore list`
- `nix run .#mfm -- keystore tx-sign --signer-ref deployer --from 0x... --to 0x...`
- `nix run .#mfm -- facts kinds`
- `nix run .#mfm -- ops list`
- `nix run .#mfm -- run status <RUN_ID>`

## Design Philosophy

- **User-Centric**: Commands are designed to be intuitive and easy to remember.
- **Scriptable**: Supports non-interactive modes, input from `stdin`, and configuration via environment variables, making it suitable for automation and scripting.
- **Secure by Default**: Uses the security-hardened keystore implementation from `crates/core` so key operations share the same security invariants.
- **AI-Friendly Output**: Provides machine-readable JSON output via a global `--output-format` flag, making it ideal for AI agents and automation, while preserving human-readable text output by default.
- **Minimalism**: Focuses on essential commands, avoiding feature bloat to maintain a clean and simple interface.

## Architecture

The CLI's structure is organized to separate concerns, making it maintainable and extensible.

```
mfm_cli/
├── src/
│   ├── main.rs            # Application entry point
│   ├── commands/
│   │   ├── mod.rs         # Top-level clap CLI + dispatch
│   │   ├── result.rs      # Shared command result/error types
│   │   ├── facts.rs       # `facts` public discovery/query subcommands
│   │   ├── keystore/      # `keystore` subcommands (import, list, delete, tx-sign)
│   │   ├── ops.rs         # `ops` public entry-point discovery
│   │   └── run/           # `run` subcommands
│   ├── support/
│   │   ├── keystore.rs    # Keystore path/unlock/create helpers
│   │   └── run_store.rs   # Event/artifact store construction helpers
│   └── presentation/
│       └── output.rs      # Text/JSON output models and rendering
└── tests/
    ├── cli_e2e_tests.rs # End-to-end tests for command workflows
    └── ...              # Other unit and integration tests
```

## Global Options

The CLI supports global options that apply to all commands:

- **`--output-format <FORMAT>`**: Specifies the output format for command results.
  - `text` (default): Human-readable text output
  - `json`: Machine-readable JSON output with standardized structure
- **Environment Variable**: `MFM_OUTPUT_FORMAT` can be set to change the default output format. Command-line flags take precedence over environment variables.

**Examples:**
```sh
# Use JSON output for a single command
mfm_cli --output-format json keystore list

# Set JSON as default for the session
export MFM_OUTPUT_FORMAT=json
mfm_cli keystore list

# Override environment variable with command-line flag
MFM_OUTPUT_FORMAT=json mfm_cli --output-format text keystore list
```

## JSON Output Format

When `--output-format json` or `MFM_OUTPUT_FORMAT=json` is used, commands and CLI argument parser
failures return structured JSON responses. Parser failures use the stable error code
`CliParseError` and preserve clap's exit code.

**Success Response:**
```json
{
  "status": "success",
  "data": { /* command-specific data */ }
}
```

**Error Response:**
```json
{
  "status": "error",
  "error": {
    "code": "ErrorCode",
    "message": "Human-readable error message",
    "diagnostics": [ /* optional closed provider diagnostics */ ]
  }
}
```

CLI errors serialize the shared app `PublicError` payload. `diagnostics` is omitted when empty;
when present it contains only closed redaction-safe provider-family diagnostics. Text mode renders
`<code>: <message>` and may add command-specific remediation without changing the JSON payload.

**Common Error Codes:**
- `InvalidKeyMaterial`: Key material format is invalid
- `InvalidRecoveryPhrase`: Recovery phrase is invalid
- `KeyNotFound`: Requested key does not exist
- `InvalidUuid`: Provided UUID format is invalid
- `AmbiguousLabel`: Multiple keys found with same label
- `MissingArgument`: Required argument not provided
- `OperationCancelled`: User cancelled the operation
- `MissingDatabaseUrl`: `DATABASE_URL` was not set and `--database-url` was not supplied
- `FactNotFound`: A fact kind/ref was not found or is not available through the public fact service
- `FactDescriptorAmbiguous`: A public fact kind resolves to more than one descriptor; pass `--shape`
- `FactQueryInvalid`: Fact query input was rejected by the app/facts query service
- `FactPredicateInvalid`: A CLI fact predicate flag could not be decoded
- `FactQueryLimitInvalid`: `--limit` must be greater than zero

## Facts Commands

Public fact discovery and read-only queries are available under the `facts` subcommand.

Facts commands use only evidence-backed app read services over the certified PostgreSQL run store
and retained fact descriptor artifacts. They do not construct live transports, signer providers,
keystores, runtime source config, or workflow runners. Public queries always use the app service's
Platform audience and default public scope; the CLI does not expose flags for `Control` or
`RunPrivate` facts.

All fact query commands require `DATABASE_URL` or `--database-url`.

### `facts kinds`

Lists public fact kinds that have Platform/default indexed facts.

```sh
mfm_cli facts kinds [--database-url <URL>]
```

JSON output:

```json
{
  "status": "success",
  "data": {
    "kinds": [
      { "fact_kind": "wallet.balance", "descriptor_count": 1 }
    ]
  }
}
```

### `facts describe`

Describes public descriptors for one fact kind, including public-safe fields and ordering policies.

```sh
mfm_cli facts describe <KIND> [--database-url <URL>]
```

### `facts explain`

Returns the same public descriptor information in a query-oriented envelope.

```sh
mfm_cli facts explain <KIND> [--database-url <URL>]
```

### `facts query`

Runs a descriptor-scoped public fact query.

```sh
mfm_cli facts query \
  --kind wallet.balance \
  --shape mfm.wallet.balance.v1 \
  --order result.amount_sat.desc \
  --subject asset_ref=btc \
  --result amount_sat.gt=100000000 \
  --field subject.asset_ref \
  --field result.amount_sat \
  --limit 20
```

`--kind`, `--order`, and at least one `--field` are required. `--shape` is optional only when the
kind resolves to exactly one public descriptor. The app service owns descriptor resolution, field
validation, exposure checks, operator checks, ordering normalization, and query compilation.
Query execution returns deterministic unsigned fact-query evidence and uses no process-local
cryptographic material.

Predicate flags:

- `--subject field=value`: maps to `subject.field equal value`
- `--result field.op=value`: maps to `result.field <operator> value`
- `--where full.field.op=value`: uses the full descriptor field id as supplied

Supported operator suffixes are `.eq`, `.lt`, `.lte`, `.gt`, and `.gte`; omitted suffix means
equality. Unprefixed scalar values are decoded as bool, signed/unsigned integer, decimal string, or
string. Explicit scalar prefixes are available when needed: `string:`, `bool:`, `i64:`, `u64:`,
`timestamp:`, `decimal:`, and `digest:`.

### `facts latest`, `facts history`, and `facts top`

Kind-first helpers over the same public query service.

```sh
mfm_cli facts latest wallet.balance \
  --shape mfm.wallet.balance.v1 \
  --order result.block_number.desc \
  --subject account_ref=addr:bc1q... \
  --field result.amount_sat

mfm_cli facts history weather.observation \
  --shape mfm.weather.observation.v1 \
  --order metadata.observed_at.desc \
  --subject country=IE \
  --result temperature_celsius_milli.lt=0 \
  --field result.temperature_celsius_milli \
  --limit 50

mfm_cli facts top wallet.balance \
  --shape mfm.wallet.balance.v1 \
  --order result.amount_sat.desc \
  --subject asset_ref=btc \
  --field result.amount_sat \
  --limit 20
```

`latest` always sends `limit = 1` to the app query service and still requires an explicit ordering
policy. `history` and `top` default to `--limit 50`.

### `facts show`

Resolves one opaque public fact reference returned by a facts query.

```sh
mfm_cli facts show <PUBLIC_REF> [--database-url <URL>]
```

Unknown refs and refs for non-public facts return the same redacted `FactNotFound` error class.

### Facts Output And Privacy Contract

Facts JSON uses the standard CLI success/error envelope. Query and show results contain only app
public DTOs: `public_ref`, `fact_kind`, a public descriptor reference, `recorded_at`,
`observed_at`, and descriptor-approved returned fields.

Text output is concise and follows the same privacy boundary. Facts commands must not print or
serialize internal refs, artifact ids, artifact evidence hashes, descriptor hashes, subject
material, subject hashes, response hashes, raw run/event coordinates, response artifacts,
capability routing details, RPC URLs, authorization headers, signer material, or keystore paths.

## Command Reference

All keystore operations are available under the `keystore` subcommand.

### `keystore import`

Imports a private key or a mnemonic-derived key into the keystore. Mnemonic phrases are one-time
inputs: MFM stores only the selected derived private key plus non-secret derivation metadata, so
users must keep their own seed backup outside MFM.
Do not put plaintext private-key file paths or key material in config; import into the keystore and
reference the entry by id or label.

**Usage:**
```sh
mfm_cli keystore import [OPTIONS]
```

**Key Options:**
- `--import-type <TYPE>`: Required. `privatekey` or `mnemonic`.
- `--label <LABEL>`: A human-readable alias for the key. If omitted, a default label is generated.
- `--derivation-path <PATH>`: For mnemonics, the BIP32 derivation path. Defaults to `m/44'/60'/0'/0/0`.
- `--stdin`: Reads the key material (private key or mnemonic) from standard input instead of an interactive hidden prompt. Use this only with controlled pipes or files; stdin input must contain exactly one line of secret material.
- `--passphrase-prompt`: For mnemonic imports, prompts for the optional BIP-39 passphrase inside the local keystore transport. Cannot be combined with `--stdin`.
- `--passphrase-file <PATH>`: For mnemonic imports, reads the optional BIP-39 passphrase from a local UTF-8 file or FIFO. Trailing `\n` and `\r\n` line endings are stripped; other bytes are used as-is.
- `--keystore <PATH>`: Specifies a custom path to the keystore file.

**Examples:**

- **Import a private key interactively:**
  ```sh
  mfm_cli keystore import --import-type privatekey --label "my-main-wallet"
  ```

- **Import a mnemonic from `stdin` with a custom derivation path:**
  ```sh
  echo "word1 word2 ..." | mfm_cli keystore import --import-type mnemonic --label "my-hd-wallet" --derivation-path "m/44'/60'/0'/0/1" --stdin
  ```

- **Import a mnemonic from `stdin` with a BIP-39 passphrase file:**
  ```sh
  mfm_cli keystore import --import-type mnemonic --label "my-hd-wallet" --passphrase-file ./bip39-extra.txt --stdin < mnemonic.txt
  ```

### `keystore list`

Lists all keys stored in the keystore, displaying their metadata.

**Usage:**
```sh
mfm_cli keystore list [OPTIONS]
```

**Key Options:**
- `--show-addresses`: Include the derived Ethereum address in the output.
- `--filter-label <REGEX>`: Filter keys by a regular expression matching the label.
- `--sort-by <FIELD>`: Sort keys by `label`, `created` (default), or `type`.
- `--keystore <PATH>`: Specifies a custom path to the keystore file.

**Examples:**

- **List all keys with addresses:**
  ```sh
  mfm_cli keystore list --show-addresses
  ```

- **List all keys with "wallet" in their label, sorted by label, in JSON format:**
  ```sh
  mfm_cli --output-format json keystore list --filter-label "wallet" --sort-by label
  ```

### `keystore delete`

Deletes a key from the keystore.

**Usage:**
```sh
mfm_cli keystore delete [OPTIONS] (<ID> | --by-label <LABEL>)
```

**Key Options:**
- `<ID>`: The UUID of the key to delete.
- `--by-label <LABEL>`: Deletes a key by its unique label. Fails if multiple keys share the same label.
- `--yes`: Skips the confirmation prompt, useful for scripting.
- `--keystore <PATH>`: Specifies a custom path to the keystore file.

**Examples:**

- **Delete a key by its ID with confirmation:**
  ```sh
  mfm_cli keystore delete "a1b2c3d4-..."
  ```

- **Delete a key by its label without confirmation:**
  ```sh
  mfm_cli keystore delete --by-label "my-main-wallet" --yes
  ```

### `keystore tx-sign`

Signs one checked EIP-1559 transaction through the canonical app/library signing service and writes
the signed raw transaction hex to an explicit bearer-output file. The command resolves exactly one
`[signers.<REF>]` runtime binding and its referenced `[keystores.<REF>]` profile. It has no direct
keystore/key selector, label lookup, private-key access, custom encoder, legacy transaction mode, or
provider fallback.

The unsigned envelope uses Alloy as its only signing-hash and encoding implementation. The app
signing service parses canonical unsigned decimal or lowercase `0x`-prefixed quantities as `U256`,
constructs the checked envelope, and rejects values outside Alloy's exact EIP-1559 field
representation. The CLI binary only maps arguments into that raw app request, writes the returned
bearer to the explicit protected file, and renders redacted metadata. The keystore provider enforces
the deterministic RFC 6979 recoverable low-s profile and expected sender, and blocking
file/unlock/KDF/key work runs outside async runtime workers.

Command output contains metadata only: `from`, `to`, canonical decimal-string `nonce` and
`chain_id`, `signing_digest`, and `transaction_hash`. The two hashes are intentionally distinct.
Output excludes local paths, raw transaction hex, signature bytes, keystore entry ids, and runtime
configuration. The `--out` file is the only bearer boundary, is created with mode 0600, and must not
already exist unless `--overwrite` is supplied. Symlink outputs and unsafe parent directories are
rejected. This command does not start, resume, submit, or certify a typed run.

**Usage:**
```sh
mfm_cli keystore tx-sign [OPTIONS]
```

**Required options:**
- `--signer-ref <REF>`: Exact signer binding under `[signers]`
- `--from <ADDRESS>`: Expected sender; signing fails if the bound key differs
- `--to <ADDRESS>`
- `--value-wei <DEC_OR_0X_HEX>`
- `--chain-id <DEC_OR_0X_HEX>`
- `--nonce <DEC_OR_0X_HEX>`
- `--max-fee-per-gas <DEC_OR_0X_HEX>`
- `--max-priority-fee-per-gas <DEC_OR_0X_HEX>`
- `--gas-limit <DEC_OR_0X_HEX>`
- `--out <PATH>`

**Optional options:**
- `--data <0xHEX>` (default: `0x`)
- `--runtime-config <PATH>` (default: `MFM_RUNTIME_CONFIG_FILE`)
- `--overwrite`: Replace an existing regular output file. Without this flag, `--out` must be a new path.

**Example:**
```sh
mfm_cli --output-format json keystore tx-sign \
  --signer-ref deployer \
  --from 0x2222222222222222222222222222222222222222 \
  --to 0x1111111111111111111111111111111111111111 \
  --value-wei 1000000000000000 \
  --chain-id 31337 \
  --nonce 0 \
  --max-fee-per-gas 2000000000 \
  --max-priority-fee-per-gas 1000000000 \
  --gas-limit 21000 \
  --out /tmp/signed.tx
```

## Public Entry-Point Commands

### `ops list`

Lists the exact public entry-point ids compiled into the binary. It does not connect to PostgreSQL
or load runtime configuration.

```sh
mfm_cli ops list
mfm_cli --output-format json ops list
```

JSON output returns plain string ids under `entry_points`. The production surface contains exactly
one entry point, `mfm.portfolio/snapshot@1`; that exact versioned id is the complete discovery
surface.

### `setup import`, `setup list`, and `setup export`

Setup owns the current target-keyed configuration before admission:

```sh
nix run .#mfm -- setup import ./organization.toml
nix run .#mfm -- setup list
nix run .#mfm -- setup export acme/primary --output ./portfolio.json
```

For local development, the `.#mfm` app starts Nixfied-managed PostgreSQL in slot
9, applies the typed store migrations, sets `DATABASE_URL`, delegates all
arguments to the packaged CLI, and stops PostgreSQL afterward without removing
its data. Separate invocations share setup and run state under the Nixfied state
root for `mfm/dev/9`. When supplying an external `DATABASE_URL` or
`--database-url`, use `mfm_cli`, `cargo run -p mfm -- <ARGS>`, or the raw binary
produced by `nix build .#mfm`.

The import document is strict TOML with a closed `configs` list. Each configuration derives its
target from its intrinsic domain id; a portfolio config with `portfolio_id = "acme/primary"`
publishes target `acme/primary`. Import is atomic and reports `created`, `updated`, or `unchanged`
for every target. Documents larger than 4 MiB fail with `SetupFileTooLarge` without being read in
full. `setup list` returns only current targets in stable order. `setup export` atomically publishes
one target's verified canonical JSON to a new path and never overwrites an existing file; a failed
write does not leave a partial final file.

There is no setup name, catalog digest selector, revision/history lookup, cursor, or delete
command. Importing a replacement configuration changes only that target's current row; already
admitted runs retain their concrete certified configuration and are unaffected.

## Run Commands (Experimental)

Typed certified run dispatch and inspection are available under the `run` subcommand.

These commands use the certified PostgreSQL run store (requires `DATABASE_URL` or
`--database-url`).

The CLI validates the PostgreSQL schema on connect and does not create or alter
tables. Apply the `mfm-storage-postgres` migrations against a fresh or
explicitly reset local database before running typed run commands.
There is no downgrade migration for the current typed Postgres baseline; rolling
back to another branch requires resetting the database or schema to that
branch's expected baseline. Filesystem artifact roots outside the typed run
store are not read or migrated by typed run commands.

Run ids use the typed identity format `run:<algorithm>:<digest>`. Normal `run start` derives the
typed run id from certified run identity material: certified spec hash, store scope, and a
required invocation key digest. The raw CLI `--invocation-key` is optional; when omitted, the app
mints a fresh opaque key before deriving the digest.

The CLI starts only through registered entry-point ops that app assembly plans and certifies into
typed execution specs, and it resumes/replays only from stored typed run streams.

`keystore tx-sign` is a thin client of the app-assembled canonical signing service. It does not
submit or resume certified typed runs and is not an alternate transaction implementation.

### `run start`

Starts a run from an exact entry-point id and a target. The app resolves the target's current
configuration and validates its schema, canonical bytes/digest, semantics, and embedded domain id
before planning; the resulting typed spec is then certified and admitted.

**Usage:**
```sh
mfm_cli run start <ENTRY_POINT> <TARGET> [OPTIONS]
```

**Key Options:**
- `<ENTRY_POINT>`: Exact public entry-point id, including namespace and version.
- `<TARGET>`: Stable configuration target such as `acme/primary`.
- `--invocation-key <KEY>`: Uses caller-provided invocation identity for retry-stable starts. When
  omitted, the app mints a fresh opaque invocation key. The raw key is not persisted; only a
  domain-separated digest enters run identity material.
- `--database-url <URL>`: PostgreSQL connection string (default: `$DATABASE_URL`)
- `--runtime-config <PATH>`: Runtime config file for live capabilities (default:
  `$MFM_RUNTIME_CONFIG_FILE`). Read-only commands do not use this option.

The repository includes a complete strict-import fixture at
`examples/setup/organization.toml`; copy it to a local setup file before importing.
`examples/setup/portfolio-erc20.toml` is the runnable token-only counterpart; pair it with an
Ethereum runtime route and start it by its derived target after importing.

`ops list` is the authoritative public surface. The setup fixture publishes portfolio config only;
setup changes target configuration without changing the entry-point registry.

For example:

```sh
mfm_cli run start mfm.portfolio/snapshot@1 acme/primary
```

The target selects only its current `PortfolioConfig`. Old `--entry-point` and `--request` flags,
JSON request files, catalog `{name,digest}` objects, old entry-point ids, unversioned ids, and
latest-like forms are rejected. The target cannot select collector policies, child configs, native
decimals, runtime routes, or a collect/reuse/report-only mode.

Run start always resolves runner executable identities before `RunAdmitted`, because those identities
are replay authority. Specs that reference unported domain state descriptors fail with
`LaunchRunnerUnavailable` before any typed run event is written. The production CLI runner registry
contains the portfolio EVM runner family and the reusable EVM transaction/validation bindings.

JSON output exposes `outcome`; text output renders `launch_outcome`. Fresh admissions report `admitted`. A duplicate start
for the same certified run identity reports `attached` without driving. If another process holds the
execution lane for the same base work identity, start reports `already_active` with
`active_run_id` and no `run` body.

Stable launch errors include:

- `EntryPointNotFound`: the exact entry-point id is not registered.
- `ConfiguredTargetInvalid`: the target is not a valid portfolio id.
- `ConfiguredValueNotFound`: the target has no current configuration.
- `ConfiguredValueSchemaInvalid`: the target's current row has the wrong schema.
- `ConfiguredValueTypeInvalid`: the target's current row does not decode as a portfolio config.
- `ConfiguredValueCanonicalMismatch`: a current row fails canonical byte/digest verification.
- `ConfiguredValueValidationFailed`: a current row fails semantic portfolio validation.
- `ConfiguredTargetMismatch`: the config's embedded portfolio id differs from the selected target.
- `PortfolioSnapshotPlanFailed`: the concrete portfolio cannot expand into the snapshot objective.
- `EntryPointCertificationFailed`: app assembly could not certify the planned typed spec.
- `LaunchRunnerUnavailable`: the certified spec references a state descriptor with no production
  runner binding.
- `RuntimeConfigRequired`: one or more certified live-source routes are absent. The neutral shared
  message identifies the configured target and required provider families; structured diagnostics
  retain only semantic network bindings. Text output appends `; pass --runtime-config`.
- `RuntimeConfigInvalid`: a supplied runtime config is unreadable, malformed, or semantically
  invalid. It is not downgraded to a missing-route error.

### `run resume`

Resumes a certified typed run by loading the spec and certificate artifacts bound by `RunAdmitted`,
verifying them against the compiled certification registry, rebuilding stream evidence, and driving
the typed scheduler until it blocks or the run completes.

**Usage:**
```sh
mfm_cli run resume <RUN_ID> [OPTIONS]
```

It rejects non-typed run ids before storage access.

Key live options:

- `--database-url <URL>`: PostgreSQL connection string (default: `$DATABASE_URL`)
- `--runtime-config <PATH>`: Runtime config file for live capabilities (default:
  `$MFM_RUNTIME_CONFIG_FILE`)

Manual `run resume <RUN_ID>` is the v1 recovery trigger for a run left with an open execution claim,
side-effect uncertainty, or a resumable frontier. Automatic dead-driver takeover and background
worker-pool dispatch are deferred. Receipt-level side-effect terminalization is final-at-risk: a
later reorg can invalidate the published receipt-derived output, so operations that need reorg
safety must use certified `Finalized(depth)` verification.

### `run manual-resolution`

Records a signed manual resolution for a certified typed run whose current stream prefix derives
`manual_blocked`. The command reads the evidence artifact bytes exactly as supplied, canonicalizes
the authorization proof JSON, and submits both through `mfm-app` so runtime can derive prefix
authority, verify signatures/quorum, stage artifacts, and append the typed manual-resolution commit.
It does not load keystores, signer registries, password files, or other signer runtime sources.

**Usage:**
```sh
mfm_cli run manual-resolution <RUN_ID> \
  --outcome <confirm-remediated|fail-without-acdc-claim> \
  --evidence <PATH> \
  --authorization-proof <PATH> \
  [OPTIONS]
```

**Key Options:**
- `--outcome <confirm-remediated|fail-without-acdc-claim>`: Operator-selected resolution outcome.
- `--evidence <PATH>`: Evidence artifact bytes covered by the signed proof claim.
- `--authorization-proof <PATH>`: Canonical or canonicalizable manual authorization proof JSON.
- `--evidence-media-type <TYPE>`: Evidence media type recorded with the artifact (default: `application/json`).
- `--note <TEXT>`: Optional redaction-safe operator note.
- `--database-url <URL>`: PostgreSQL connection string (default: `$DATABASE_URL`)

Stable manual-resolution errors include:

- `ManualResolutionEvidenceReadFailed`: the evidence file could not be read.
- `ManualResolutionProofReadFailed`: the authorization proof file could not be read.
- `ManualResolutionProofInvalid`: the authorization proof JSON could not be canonicalized.
- `ManualResolutionEvidenceMediaTypeInvalid`: the evidence media type is invalid.
- `ManualResolutionNoteInvalid`: the optional note is not accepted by the event text contract.
- `LaunchRuntimeError`: runtime rejected the prefix or proof binding.
- `RunStoreRejected`: the prepared commit was stale or violated typed store admission.

### `run list`

Lists observation-only run rows or reads changes from a previous cursor. Strict per-run status and
stream authority remain available through `run status` and `run stream`; list/watch output is bounded
by the sealed observation frontier and may lag a just-written commit until the frontier advances.
Run observation cursors are opaque server-issued tokens. They are epoch-bound in v1 and have no
wall-clock TTL or cursor garbage collection. Unknown, missing, retired-key, or stale-format cursors
fail as `InvalidCursor`; store epoch mismatches fail as `CursorExpired`, and the recovery path is to
run a fresh list without `--cursor`.

**Usage:**
```sh
mfm_cli run list [OPTIONS]
```

**Key Options:**
- `--cursor <OPAQUE>`: Cursor returned by a previous page.
- `--limit <N>`: Maximum run rows to return (default: 50).
- `--wait-ms <N>`: Long-poll wait in milliseconds.
- `--watch`: Read changes from `--cursor`; requires a cursor from a previous list page.
- `--database-url <URL>`: PostgreSQL connection string (default: `$DATABASE_URL`).

### `run status`

Shows certified typed run status without executing states. JSON output uses semantic saga status:
`run_mode` is one of `forward`, `remediating`, `manual_blocked`, `completed`, `compensated`,
`manually_resolved`, or `failed_without_acdc_claim`. The `saga` object reports the certified policy,
derived obligations per forward ledger, linked remediation ledgers, manual-block reason and manual
authorization requirements when applicable, terminal resolution claim when present, projected
resource ledgers with declared claim/touched-set evidence and resource-key digests, and active
exclusive lane holders referenced by the target run's persisted live side-effect ledgers. It does
not serialize raw resource keys, unrelated global lane holders, or scheduler waiters that blocked
before appending lane evidence.
`attempt_dispositions` reports committed attempt-level lifecycle status separately from `run_mode`;
each entry has `node_id`, `attempt_id`, `disposition` (`started`, `completed`, `failed`, or
`interrupted`), and status-specific fields such as `attempt_no`, `retryable`, `error_code`, or
`output_cell_id`. Failed portfolio SelectHoldings / assembly attempts surface domain codes in the
failed attempt disposition and the stream event reference (`error_code`), not soft snapshot fields.
Product cutover domain codes are the closed set
`missing_fact`, `receipt_mismatch`, `unsupported_requirement`, and `ambiguous_facts`.
`inconsistent_network_anchors` is a residual hard-fail guard (not a product
soft path). Generic
runtime classes such as `runner_output_invalid` remain for non-domain runner failures. `scheduler_status` is read-only `observed` for `run status`;
start/resume responses set it to `observed` when an already-terminal run needs no scheduler
dispatch, otherwise `advanced`, `blocked`, `public_output_projected`, `execution_claim_busy`, or
`execution_claim_lost` according to the app dispatch loop and claim-coordination outcome. Manual
authorization requirements include the required evidence schema, signing scheme, authority id,
allowed operator public identities or a safe summary, and quorum. They never expose signer runtime
sources, keystore paths, password paths, passwords, or other secrets.

**Usage:**
```sh
mfm_cli run status <RUN_ID> [OPTIONS]
```

### `run stream`

Prints store-owned references for certified typed run events. Sequence range filters are applied
only after the app service validates the full stored stream.

**Usage:**
```sh
mfm_cli run stream <RUN_ID> [OPTIONS]
```

**Key Options:**
- `--from-seq <N>`: First sequence number to read (default: 1)
- `--to-seq <N>`: Optional last sequence number to read (inclusive)

### `run public-output`

Renders a typed public output by schema id through sealed read authority minted from verified stored
spec/certificate artifacts, rebuilt stream evidence, store-owned public-output projection evidence,
and typed artifact bytes. Rendered JSON is not resume, replay, or render authority.

**Usage:**
```sh
mfm_cli run public-output <RUN_ID> --schema-id <SCHEMA_ID> [OPTIONS]
```

### `run replay`

Verifies replay authority for a certified typed run by loading the stored certified spec and
certificate artifacts, comparing them to `RunAdmitted`, verifying them against the production
registry, loading retained artifact evidence, and then constructing the typed replay broker. The
command rejects missing retained evidence, executable identity drift, and live-capability fallback.
Domain replay execution is available only after the corresponding typed runner/replay adapter is
registered by a domain port.

**Usage:**
```sh
mfm_cli run replay <RUN_ID> [OPTIONS]
```

## Configuration

The CLI's process-level configuration is intentionally narrow.

- **`MFM_OUTPUT_FORMAT`**: Sets the default output format for all commands. Valid values are `text` and `json`. Command-line `--output-format` flag takes precedence.
  ```sh
  export MFM_OUTPUT_FORMAT="json"
  mfm_cli keystore list  # Will output JSON
  ```

- **`DATABASE_URL`**: PostgreSQL connection string used by `run` commands (unless `--database-url` is provided).
  ```sh
  export DATABASE_URL="postgresql://postgres:postgres@localhost:5432/mfm_test"
  cargo sqlx migrate run --source crates/storages/postgres/migrations
  mfm_cli run status "run:sha256-jcs-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
  ```

- **`MFM_RUNTIME_CONFIG_FILE`**: Runtime-only TOML or JSON config file used by live capability
  drivers. Live `run start` and `run resume` may also pass `--runtime-config <PATH>`, which takes
  precedence over this environment variable. Read-only run commands do not load runtime config.

  ```toml
  [evm.routes.reth-dev]
  source_ref = "reth-local"
  rpc_url = "http://127.0.0.1:8545"

  [keystores.default]
  keystore_path = "/run/mfm/deployer.keystore"
  unlock_file = "/run/mfm/deployer.password"

  [signers.deployer]
  provider = "keystore"
  keystore_ref = "default"
  entry_id = "<uuid>"
  ```

- Keystore administration commands (`import`, `list`, and `delete`) use either `--keystore <PATH>`,
  which prompts locally for credentials, or a runtime-config keystore profile selected by
  `--runtime-config <PATH>` or `MFM_RUNTIME_CONFIG_FILE`. `--keystore-ref <REF>` defaults to
  `default` for those administration commands. `tx-sign` instead requires `--signer-ref` and loads
  that exact signer-to-keystore binding; it does not accept direct keystore or key selectors.
- Live BTC/EVM provider failures are reported with redacted diagnostic codes such as
  `bitcoin_rpc_http_status`, `bitcoin_rpc_json_error`, `evm_rpc_http_status`, or
  `evm_source_mismatch`. Diagnostics may include closed operation ids and numeric status/error
  codes, but never RPC URLs, authorization headers, provider messages, request/response bodies, or
  runtime config paths.
- Bitcoin portfolio configs use non-secret `source_identity` to select the semantic runtime route.

- Typed EVM RPC note: per-request `rpc_url` override is not supported.
- Typed EVM RPC runbook: [`../../docs/evm-rpc-routing.md`](../../docs/evm-rpc-routing.md)

## Best Practices

- **For interactive keystore administration**, pass `--keystore <PATH>` and rely on the built-in
  prompts for passwords and confirmations.
- **For scripting and automation**, use runtime-config keystore profiles with unlock files, the
  `--stdin` flag for import material, and `--yes` to bypass confirmations where supported. For
  transaction signing, configure one exact `[signers]` binding and pass its `--signer-ref` plus the
  expected `--from` address.
- **For AI agents and programmatic use**, use `--output-format json` to get structured, machine-readable responses with predictable error codes.
- **Secure your environment**: When using environment variables, ensure the security of your shell history and environment.
- **Backup your keystore file**: The CLI manages keys, but you are responsible for securely backing up the keystore file itself.

## AI/Machine Integration

The CLI is designed to be AI-friendly with consistent JSON output that makes it easy for AI agents to:

- Parse command results reliably using the standardized `{"status": "success", "data": {...}}` format
- Handle errors gracefully with structured error responses containing stable error codes
- Integrate with automation pipelines using runtime config and non-interactive modes
- Process keystore operations programmatically without human intervention

**Example AI workflow:**
```sh
# Set JSON output for the session
export MFM_OUTPUT_FORMAT=json

# Import a key programmatically
echo "1234567890abcdef..." | mfm_cli keystore import --import-type privatekey --stdin --label "ai-generated-key"

# List keys and parse the JSON response
mfm_cli keystore list | jq '.data[] | select(.label == "ai-generated-key")'
```
