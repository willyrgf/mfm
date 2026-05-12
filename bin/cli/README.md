# MFM CLI Documentation

## Overview

The `mfm_cli` is the command-line interface for the MFM toolkit. It provides a user-friendly and scriptable way to interact with MFM modules, including keystore management and an experimental `run` subcommand for starting/resuming/inspecting runs. The CLI is built using the `clap` crate for robust argument parsing and command structure.

For Nixfied automation, prefer strict app wrappers when possible:
- `nix run .#mfm::keystore::list`
- `nix run .#mfm::keystore::tx-sign -- --to ...`
- `nix run .#mfm::run::status -- <RUN_ID>`

Build the packaged binary with `nix build .#mfm-cli` when you need the raw CLI under Nix.

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
- `--stdin`: Reads the key material (private key or mnemonic) from standard input instead of an interactive prompt.
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

Implementation note: this command is a thin wrapper over a run-backed op (`op_id = "keystore_tx_sign"`, `op_version = "v1"`). The CLI maps args to op input and calls `mfm_sdk::unstable::execute_single_op_report` to launch and decode the final report payload.

The command output includes metadata only (`from`, `to`, `nonce`, `chain_id`, `tx_type`, `payload_hash`, `out_path`) and intentionally excludes raw tx hex and signature bytes.

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

Run operations are available under the `run` subcommand.

These commands are intended for parity/integration testing and early workflows. They currently use:

- a PostgreSQL-backed stream store (requires `DATABASE_URL` or `--database-url`)
- a filesystem artifact store (defaults to `$MFM_ARTIFACT_ROOT` or `~/.mfm/run_artifacts`, or use `--artifact-root`)

Run commands and REST API run endpoints are backed by the same shared feature catalog/runtime layer (`mfm-app`) to keep both entrypoints behaviorally aligned.
The CLI applies its `proof`/`v1` defaults before calling the app layer; REST and generic feature
payloads must use the explicit tagged `run.start` envelope documented in `bin/rest-api/README.md`.

Keystore tx commands are also run-backed and use the same shared op registry; they intentionally keep domain execution out of `bin/cli`. They use ephemeral in-memory stream storage (no `DATABASE_URL` requirement) plus filesystem artifacts.

### `run start`

Starts a new run.

**Usage:**
```sh
mfm_cli run start [OPTIONS]
```

**Key Options:**
- `--op-id <ID>`: Public root operation id (default: `proof`)
- `--op-version <VERSION>`: Operation version (default: `v1`)
- `--op-config-json <JSON>`: Operation config JSON (must be canonical-json-hashable; no floats)
- `--database-url <URL>`: PostgreSQL connection string (default: `$DATABASE_URL`)
- `--artifact-root <PATH>`: Artifact store root directory (default: `$MFM_ARTIFACT_ROOT` or `~/.mfm/run_artifacts`)

`run start` accepts only public root ops from the built-in app bundle. Planner-internal semantic
ids such as `portfolio_prepare_execution_sources` and `portfolio_project_report` are registered for
recursive planning only and are rejected with `op_not_public`. Use `run pipeline start` when you
need an explicit custom pipeline payload.

**Examples:**
```sh
export DATABASE_URL="postgresql://postgres:postgres@localhost:5432/mfm_test"

# Start the built-in proof op
mfm_cli run start

# Start with an explicit op config JSON
mfm_cli run start --op-config-json '{"message":"hello"}'
```

Current built-in public root ops for `run start` are:

- `proof`
- `keystore_import`
- `keystore_list`
- `keystore_delete`
- `keystore_tx_sign`
- `evm_read`
- `evm_contract_from_nix`
- `evm_deploy`
- `evm_configure`
- `evm_validate`
- `evm_deploy_configure_validate_config_build`
- `evm_deploy_configure_validate_execute`
- `evm_deploy_configure_validate`
- `portfolio_config_build`
- `portfolio_execute`
- `portfolio_tracker`
- `nix_app`

### `run pipeline start`

Starts a run from a full pipeline JSON payload.

**Usage:**
```sh
mfm_cli run pipeline start --pipeline-json '<PIPELINE_JSON>' [OPTIONS]
```

**Key Options:**
- `--pipeline-json <JSON>`: Required. Full `mfm_sdk::pipeline::Pipeline` JSON.
- `--input-json <JSON>`: Optional pipeline input payload (default: `{}`).
- `--database-url <URL>`: PostgreSQL connection string (default: `$DATABASE_URL`).
- `--artifact-root <PATH>`: Artifact store root directory (default: `$MFM_ARTIFACT_ROOT` or `~/.mfm/run_artifacts`).

**Example:**
```sh
mfm_cli run pipeline start \
  --pipeline-json '{"machine_id":"proof","pipeline_version":"v1","steps":[{"step_id":"main","op_id":"proof","op_version":"v1","op_config":{}}]}'
```

### `run pipeline deploy-configure-validate`

Starts a standard 3-step pipeline:
1. `evm_deploy`
2. `evm_configure`
3. `evm_validate`

**Usage:**
```sh
mfm_cli run pipeline deploy-configure-validate --spec-json '<SPEC_JSON>' [OPTIONS]
mfm_cli run pipeline deploy-configure-validate --spec-file /path/to/spec.json [OPTIONS]
mfm_cli run pipeline deploy-configure-validate --spec-file /path/to/config.toml [OPTIONS]
```

`--spec-json` remains inline JSON-only. `--spec-file` accepts authored JSON or TOML and
canonicalizes it into the same typed pipeline spec before launch, preserving the existing default
values for `machine_id`, `pipeline_version`, and `input`. The generated pipeline still targets the
legacy `evm_deploy_configure_validate` root so existing CLI behavior stays stable while canonical
input is internally lowered through the new config-build and strict execute boundaries.

EVM write phases (`evm_deploy`, `evm_configure`, and contract-set deploys) require
`signing_key_env`. Node-managed unsigned transaction submission is rejected so the runtime can record
a durable signed transaction intent before broadcast.

### `run resume`

Resumes an existing run by id (executes any remaining states).

**Usage:**
```sh
mfm_cli run resume <RUN_ID> [OPTIONS]
```

**Examples:**
```sh
mfm_cli run resume "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"
```

### `run status`

Shows run status without executing states.

**Usage:**
```sh
mfm_cli run status <RUN_ID> [OPTIONS]
```

### `run stream`

Prints run stream records from the stream store.
For `run:*`, these records encode machine event payloads.

**Usage:**
```sh
mfm_cli run stream <RUN_ID> [OPTIONS]
```

**Key Options:**
- `--from-seq <N>`: First sequence number to read (default: 1)
- `--to-seq <N>`: Optional last sequence number to read (inclusive)

### `run artifacts get`

Fetches an artifact by id from the artifact store. The id must be a valid SHA-256 content address:
exactly 64 lowercase hexadecimal characters.

Note: this command does not require Postgres; it only uses the filesystem artifact store.

**Usage:**
```sh
mfm_cli run artifacts get <ARTIFACT_ID> [OPTIONS]
```

**Output Notes:**
- If the artifact bytes decode as JSON, the response uses `encoding: "json"` and includes a `value` field.
- Otherwise the response uses `encoding: "hex"` and includes a hex string field.

## Portfolio Commands (Experimental)

### `portfolio snapshot`

Starts the canonical portfolio snapshot flow from a full request object containing:
- `portfolio`
- `valuation_source_registry`

**Requirements:**
- `DATABASE_URL` (stream store)
- managed RPC bootstrap configuration (`MFM_EVM_RPC_SOURCES_JSON`)

**Usage:**
```sh
mfm_cli portfolio snapshot --request-file <REQUEST_FILE> [OPTIONS]
mfm_cli portfolio snapshot --request-json '<REQUEST_JSON>' [OPTIONS]
```

**Key Options:**
- `--request-file <PATH>`: Path to an authored request file in JSON or TOML
- `--request-json <JSON>`: Inline canonical request JSON payload

Both authored formats normalize immediately into the same canonical typed request shape. The
resulting canonical request must use the same in-place contract as the `portfolio.snapshot`
feature:

```json
{
  "portfolio": { "...": "canonical PortfolioConfig" },
  "valuation_source_registry": { "...": "canonical ValuationSourceRegistry" }
}
```

`portfolio.networks[*].control_scope` is optional and defaults to `shared`. Set it when one
portfolio flow must not share managed `rpc.control` source state with another flow on the same
network.

**Output Notes:**
- Result metadata includes run ids and snapshot ids.
- The response includes `snapshot_artifact_id` when the run completed.
- The response includes the canonical portfolio `report` when available.
- `report.schema_version` is currently `2`; the referenced snapshot artifact also carries `schema_version = 2`.
- `report.wallet_summaries[*].totals_by_quote[*]` and `report.totals_by_quote[*]` expose derived per-quote `assets_value_dec`, `collateral_value_dec`, `debt_value_dec`, `staked_value_dec`, and `net_value_dec`.

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
  mfm_cli run start
  ```

- **`MFM_ARTIFACT_ROOT`**: Filesystem artifact store root used by `run` commands (unless `--artifact-root` is provided).
  ```sh
  export MFM_ARTIFACT_ROOT="/tmp/mfm_artifacts"
  mfm_cli run artifacts get "<ARTIFACT_ID>"
  ```

- **`MFM_EVM_RPC_SOURCES_JSON`**: Optional JSON array of source objects used to bootstrap the canonical `rpc.control` source catalog. Runtime-only and never persisted.
  ```sh
  export MFM_EVM_RPC_SOURCES_JSON='[
    {"id":"helios_local","network_id":"ethereum-mainnet","rpc_url":"http://127.0.0.1:8545","kind":"local"},
    {"id":"drpc_public","network_id":"ethereum-mainnet","rpc_url":"https://eth.drpc.org","kind":"remote_public"}
  ]'
  ```

- **`MFM_EVM_RPC_PREFERRED_ORDER`**: Optional comma-separated source IDs that seed canonical control-plane ranking order.
  ```sh
  export MFM_EVM_RPC_PREFERRED_ORDER="helios_local,drpc_public"
  ```

- **`MFM_EVM_RPC_REQUIRE_GET_PROOF_IDS`**: Optional comma-separated source IDs that must pass `eth_getProof` during control-plane bootstrap/probe.
  ```sh
  export MFM_EVM_RPC_REQUIRE_GET_PROOF_IDS="helios_local"
  ```

- Managed `rpc.control` bootstrap requires `network_id` on every configured source.
- Canonical managed requests require an explicit `network_id`; `control_scope` defaults to
  `shared` unless an op config overrides it.

- Managed RPC note: per-request `rpc_url` override is not supported. Canonical routing enters through `rpc.control`.
- Managed RPC runbook: [`../../docs/evm-rpc-routing.md`](../../docs/evm-rpc-routing.md)

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
