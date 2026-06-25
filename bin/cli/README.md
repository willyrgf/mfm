# MFM CLI Documentation

## Overview

The `mfm_cli` is the command-line interface for the MFM toolkit. It provides a user-friendly and scriptable way to interact with MFM modules, including keystore management and an experimental `run` subcommand for starting/resuming/inspecting runs. The CLI is built using the `clap` crate for robust argument parsing and command structure.

Run the packaged CLI with `nix run .#mfm -- <ARGS>`, for example:
- `nix run .#mfm -- keystore list`
- `nix run .#mfm -- keystore tx-sign --to ...`
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
│   │   ├── keystore/      # `keystore` subcommands (import, list, delete, tx-sign)
│   │   └── run/           # `run` subcommands
│   ├── support/
│   │   ├── app_services.rs # Shared AppServices + error-adapter helpers
│   │   ├── input.rs       # Input prompts/password helpers
│   │   ├── keystore_manager.rs # Keystore path/unlock/create helpers
│   │   └── run_stores.rs  # Event/artifact store construction helpers
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
    "message": "Human-readable error message"
  }
}
```

**Common Error Codes:**
- `InvalidKeyMaterial`: Key material format is invalid
- `InvalidRecoveryPhrase`: Recovery phrase is invalid
- `KeyNotFound`: Requested key does not exist
- `InvalidUuid`: Provided UUID format is invalid
- `AmbiguousLabel`: Multiple keys found with same label
- `MissingArgument`: Required argument not provided
- `OperationCancelled`: User cancelled the operation

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

Signs an EIP-1559 transaction payload using a key already stored in the keystore and writes the signed raw transaction to a file.

Implementation note: this command is a direct CLI helper over the keystore and EVM libraries. It
does not start, resume, or certify a typed run.

The command output includes metadata only (`from`, `to`, `nonce`, `chain_id`, `tx_type`, `payload_hash`) and intentionally excludes local output paths, raw tx hex, and signature bytes.
The output file is created with restrictive permissions and must not already exist unless `--overwrite` is supplied. Symlink outputs and unsafe parent directories are rejected.

**Usage:**
```sh
mfm_cli keystore tx-sign [OPTIONS]
```

**Required options:**
- Key selector: `--id <UUID>` or `--by-label <LABEL>`
- `--to <ADDRESS>`
- `--value-wei <DEC_OR_0X_HEX>`
- `--chain-id <U64>`
- `--nonce <U64>`
- `--max-fee-per-gas <DEC_OR_0X_HEX>`
- `--max-priority-fee-per-gas <DEC_OR_0X_HEX>`
- `--gas-limit <U64>`
- `--out <PATH>`

**Optional options:**
- `--data <0xHEX>` (default: `0x`)
- `--keystore <PATH>`
- `--overwrite`: Replace an existing regular output file. Without this flag, `--out` must be a new path.

**Example:**
```sh
mfm_cli --output-format json keystore tx-sign \
  --by-label "my-main-wallet" \
  --to 0x1111111111111111111111111111111111111111 \
  --value-wei 1000000000000000 \
  --chain-id 31337 \
  --nonce 0 \
  --max-fee-per-gas 2000000000 \
  --max-priority-fee-per-gas 1000000000 \
  --gas-limit 21000 \
  --out /tmp/signed.tx
```

## Run Commands (Experimental)

Typed certified run dispatch and inspection are available under the `run` subcommand.

These commands use the certified PostgreSQL run store (requires `DATABASE_URL` or
`--database-url`).

The CLI validates the PostgreSQL schema on connect and does not create or alter
tables. Apply the `mfm-stream-store-postgres` migrations against a fresh or
explicitly reset local database before running typed run commands.
There is no downgrade migration for the current typed Postgres baseline; rolling
back to an older branch requires resetting the database or schema to that
branch's expected baseline. Old filesystem artifact roots are not read or
migrated by typed run commands.

Run ids use the typed identity format `run:<algorithm>:<digest>`. Old UUID dynamic run ids are not
accepted by the typed CLI run surface. Normal `run start` derives the typed run id from certified
run identity material: certified spec hash, store trust scope, and an optional distinct-run key
digest.

Old dynamic op launch, pipeline launch, context snapshots, and generic artifact reads have been
removed from the `run` subcommand. The CLI starts only through registered entry-point ops that app
assembly plans and certifies into typed execution specs, and it resumes/replays only from stored
typed run streams.

Old dynamic runs are not silently migrated into certified typed runs. Historical dynamic run data can
only be inspected through explicit operational tooling outside this typed run surface.

Keystore tx commands are direct CLI helpers over the keystore and EVM libraries. They do not submit
or resume certified typed runs, and they do not route through the removed legacy app bridge.

### `run start`

Starts a common workflow through a registered entry-point op. The CLI reads authored config,
passes the public op name, optional version, config format, and config bytes to app assembly, and
then starts the certified typed run prepared by the app layer. Config format defaults to TOML.

**Usage:**
```sh
mfm_cli run start --op <NAME> --config <PATH> [OPTIONS]
```

**Key Options:**
- `--op <NAME>`: Public entry-point operation name.
- `--config <PATH>`: Authored op config file.
- `--op-version <VERSION>`: Optional public op version. If omitted, the latest registered version is selected.
- `--config-format <toml|json>`: Authored config format. Defaults to `toml`.
- `--distinct-run-key <KEY>`: Forces a distinct run for otherwise identical certified work. The raw
  key is not persisted; only a domain-separated digest enters run identity material.
- `--framework-version <VALUE>`: Framework version evidence recorded in `RunAdmitted`.
- `--source-revision <VALUE>`: Source revision evidence recorded in `RunAdmitted` (or `MFM_SOURCE_REVISION`).
- `--drive <append-only|once|until-blocked>`: Scheduler drive policy after `RunAdmitted`.
- `--database-url <URL>`: PostgreSQL connection string (default: `$DATABASE_URL`)

Examples:

```sh
mfm_cli run start --op portfolio_snapshot --config portfolio.toml
mfm_cli run start --op evm_contract_lifecycle --config lifecycle.toml --op-version 1
```

Run start always resolves runner executable identities before `RunAdmitted`, because those identities
are replay authority. `--drive append-only` suppresses post-start execution only; it does not bypass
runner resolution. Specs that reference unported domain state descriptors fail with
`LaunchRunnerUnavailable` before any typed run event is written. The production CLI runner registry
contains the framework public-output renderer plus the portfolio and EVM contract domain runners
used by registered entry-point ops.

JSON and text output include `launch_outcome`. Fresh admissions report `admitted`. A duplicate start
for the same certified run identity reports `attached` without driving; if another process holds a
live execution claim it reports `already_driving`; if this process cannot match the admitted runner
executable evidence it reports `incompatible_executable`.

Stable launch errors include:

- `EntryPointOpNotFound`: no registered op matches `--op`.
- `EntryPointOpVersionNotFound`: `--op-version` selects no registered version for the public op.
- `AuthoredConfigReadFailed`: the authored config file could not be read.
- `AuthoredConfigDecodeFailed`: the authored config does not match the selected op schema.
- `EntryPointOpCertificationFailed`: app assembly could not certify the planned typed spec.
- `LaunchRunnerUnavailable`: the certified spec references a state descriptor with no production
  runner binding.

### `run resume`

Resumes a certified typed run by loading the spec and certificate artifacts bound by `RunAdmitted`,
verifying them against the production registry, rebuilding stream evidence, and driving the typed
scheduler according to `--drive`.

**Usage:**
```sh
mfm_cli run resume <RUN_ID> [OPTIONS]
```

It rejects non-typed run ids before storage access. `--drive append-only` validates and reports the
stored run without executing states.

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
- `--drive <append-only|once|until-blocked>`: Scheduler drive policy after the manual-resolution event.
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
`interrupted`), and status-specific fields such as `attempt_no`, `retryable`, or `output_cell_id`.
`scheduler_status` is read-only `observed` for `run status`; start/resume responses set it to
`advanced`, `blocked`, `public_output_projected`, `execution_claim_busy`,
`execution_claim_lost`, or `incompatible_executable` according to the app dispatch loop and
claim-coordination outcome. Manual
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

The CLI's behavior can be modified using environment variables, which is ideal for CI/CD pipelines and automated scripts.

- **`MFM_OUTPUT_FORMAT`**: Sets the default output format for all commands. Valid values are `text` and `json`. Command-line `--output-format` flag takes precedence.
  ```sh
  export MFM_OUTPUT_FORMAT="json"
  mfm_cli keystore list  # Will output JSON
  ```

- **`MFM_KEYSTORE_PATH`**: Default keystore path used by keystore CLI commands when `--keystore` is not provided. The CLI resolves this before launching the underlying op.
  ```sh
  export MFM_KEYSTORE_PATH="/etc/mfm/prod.keystore"
  mfm_cli keystore list
  ```

- **`MFM_KEYSTORE_PASSWORD`**: Provides the keystore password non-interactively. If this is set, the CLI will not prompt for a password.
  ```sh
  export MFM_KEYSTORE_PASSWORD="my-super-secret-password"
  mfm_cli keystore list
  ```

- **`MFM_INTEGRATION_TEST`**: When set to `1`, the CLI uses a faster, less secure KDF configuration for the keystore. **This should only be used for testing purposes.**

- **`DATABASE_URL`**: PostgreSQL connection string used by `run` commands (unless `--database-url` is provided).
  ```sh
  export DATABASE_URL="postgresql://postgres:postgres@localhost:5432/mfm_test"
  cargo sqlx migrate run --source crates/storages/stream-store-postgres/migrations
  mfm_cli run status "run:sha256-jcs-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
  ```

- **`MFM_EVM_RPC_SOURCES_JSON`**: Optional JSON source registry used by typed EVM RPC
  backends. It is runtime-only and never persisted. The registry contains endpoint-bearing
  `sources` and ordered fallback `policies`; workflow configs carry semantic network intent only.
  ```sh
  export MFM_EVM_RPC_SOURCES_JSON='{
    "sources": [
      {"id":"reth-local","expected_chain_id":31337,"rpc_url":"http://127.0.0.1:8545","authorization":null},
      {"id":"publicnode-ethereum-mainnet","expected_chain_id":1,"rpc_url":"https://ethereum-rpc.publicnode.com","authorization":null}
    ],
    "policies": [
      {"id":"reth-local","ordered_sources":["reth-local"]},
      {"id":"publicnode-ethereum-mainnet","ordered_sources":["publicnode-ethereum-mainnet"]}
    ]
  }'
  ```

- **`MFM_EVM_SIGNERS_JSON`**: Optional runtime signer registry consumed by typed EVM contract
  workflows. Signer provider entries resolve non-secret `signer_ref` values from config to
  process-local providers without exposing private keys in typed values or outputs.
  ```sh
  export MFM_EVM_SIGNERS_JSON='[
    {
      "signer_ref": "deployer",
      "entry_id": "<uuid>",
      "keystore_env": "MFM_KEYSTORE_PATH",
      "unlock_file_env": "MFM_KEYSTORE_PASSWORD_FILE"
    }
  ]'
  ```

- Typed EVM RPC source configuration requires `expected_chain_id` on every configured source and at
  least one policy with an ordered source list.
- Typed EVM contract requests use semantic `network_id` plus `expected_chain_id`; the app runtime
  uses `network_id` as the process-local EVM source and policy id when resolving
  `MFM_EVM_RPC_SOURCES_JSON`.
- Typed EVM contract requests use non-secret `signer_ref`; the app runtime resolves it against
  `MFM_EVM_SIGNERS_JSON`.
- Portfolio configs may also use `control_scope` when source-selection partitioning is part of the
  domain request identity.

- Typed EVM RPC note: per-request `rpc_url` override is not supported.
- Typed EVM RPC runbook: [`../../docs/evm-rpc-routing.md`](../../docs/evm-rpc-routing.md)

## Best Practices

- **For interactive use**, rely on the built-in prompts for passwords and confirmations.
- **For scripting and automation**, use a combination of environment variables (`MFM_KEYSTORE_PASSWORD`, `MFM_KEYSTORE_PATH`), the `--stdin` flag for input, and the `--yes` flag to bypass confirmations.
- **For AI agents and programmatic use**, use `--output-format json` to get structured, machine-readable responses with predictable error codes.
- **Secure your environment**: When using environment variables, ensure the security of your shell history and environment.
- **Backup your keystore file**: The CLI manages keys, but you are responsible for securely backing up the keystore file itself.

## AI/Machine Integration

The CLI is designed to be AI-friendly with consistent JSON output that makes it easy for AI agents to:

- Parse command results reliably using the standardized `{"status": "success", "data": {...}}` format
- Handle errors gracefully with structured error responses containing stable error codes
- Integrate with automation pipelines using environment variables and non-interactive modes
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
