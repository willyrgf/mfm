# MFM CLI Documentation

## Overview

The `mfm_cli` is the command-line interface for the MFM toolkit. It provides a user-friendly and scriptable way to interact with MFM modules, starting with comprehensive keystore management. The CLI is built using the `clap` crate for robust argument parsing and command structure.

## Design Philosophy

- **User-Centric**: Commands are designed to be intuitive and easy to remember.
- **Scriptable**: Supports non-interactive modes, input from `stdin`, and configuration via environment variables, making it suitable for automation and scripting.
- **Secure by Default**: Integrates directly with the `mfm_core::keystore` to ensure all key operations adhere to the same high security standards.
- **AI-Friendly Output**: Provides machine-readable JSON output via a global `--output-format` flag, making it ideal for AI agents and automation, while preserving human-readable text output by default.
- **Minimalism**: Focuses on essential commands, avoiding feature bloat to maintain a clean and simple interface.

## Architecture

The CLI's structure is organized to separate concerns, making it maintainable and extensible.

```
mfm_cli/
├── src/
│   ├── main.rs         # Application entry point, argument parsing
│   └── cli/
│       ├── mod.rs      # Defines top-level commands (e.g., `keystore`)
│       ├── keystore/
│       │   ├── mod.rs  # Defines `keystore` subcommands (import, list, delete)
│       │   ├── import.rs # Logic for the `import` command
│       │   ├── list.rs   # Logic for the `list` command
│       │   └── delete.rs # Logic for the `delete` command
│       └── utils/
│           ├── mod.rs      # Utility module declarations
│           ├── keystore.rs # KeystoreManager for handling path and unlocking
│           ├── input.rs    # Handles user input (passwords, confirmations)
│           └── output.rs   # Formats command output (tables, JSON)
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

When `--output-format json` is used, all commands return structured JSON responses:

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
- `InvalidPrivateKey`: Private key format is invalid
- `InvalidMnemonic`: Mnemonic phrase is invalid
- `KeyNotFound`: Requested key does not exist
- `InvalidUuid`: Provided UUID format is invalid
- `AmbiguousLabel`: Multiple keys found with same label
- `MissingArgument`: Required argument not provided
- `OperationCancelled`: User cancelled the operation

## Command Reference

All keystore operations are available under the `keystore` subcommand.

### `keystore import`

Imports a private key or a mnemonic phrase into the keystore.

**Usage:**
```sh
mfm_cli keystore import [OPTIONS]
```

**Key Options:**
- `--import-type <TYPE>`: Required. `privatekey` or `mnemonic`.
- `--label <LABEL>`: A human-readable alias for the key. If omitted, a default label is generated.
- `--derivation-path <PATH>`: For mnemonics, the BIP32 derivation path. Defaults to `m/44'/60'/0'/0/0`.
- `--stdin`: Reads the key material (private key or mnemonic) from standard input instead of an interactive prompt.
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

## Configuration

The CLI's behavior can be modified using environment variables, which is ideal for CI/CD pipelines and automated scripts.

- **`MFM_OUTPUT_FORMAT`**: Sets the default output format for all commands. Valid values are `text` and `json`. Command-line `--output-format` flag takes precedence.
  ```sh
  export MFM_OUTPUT_FORMAT="json"
  mfm_cli keystore list  # Will output JSON
  ```

- **`MFM_KEYSTORE_PATH`**: Overrides the default keystore path (`~/.mfm/keystore`). If a `--keystore` flag is provided, it takes precedence.
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
