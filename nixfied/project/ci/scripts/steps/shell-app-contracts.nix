{ pkgs }:
pkgs.writeText "mfm-ci-steps-shell-app-contracts.sh" ''
LOGFILE=$(artifact_path "shell-app-contracts.log")
set +e
(
  set -euo pipefail

  SYSTEM=$(nix eval --raw --impure --expr builtins.currentSystem)

  ci_contract=$(nix eval --json ".#apps.$SYSTEM.ci.meta.nixfied.api.appContract")
  check_contract=$(nix eval --json ".#apps.$SYSTEM.check.meta.nixfied.api.appContract")
  cli_contract=$(nix eval --json ".#apps.$SYSTEM.mfm_cli.meta.nixfied.api.appContract")
  rest_contract=$(nix eval --json ".#apps.$SYSTEM.mfm_rest_api.meta.nixfied.api.appContract")
  snapshot_contract=$(nix eval --json ".#apps.$SYSTEM.\"mfm::portfolio::snapshot\".meta.nixfied.api.appContract")
  process_status_contract=$(nix eval --json ".#apps.$SYSTEM.\"process::status\".meta.nixfied.api.appContract")
  run_start_contract=$(nix eval --json ".#apps.$SYSTEM.\"mfm::run::start\".meta.nixfied.api.appContract")

  echo "$ci_contract" | jq -e '.commandClass == "batch-runner" and .allowUnknownArgs == false' >/dev/null
  echo "$ci_contract" | jq -e '.args[] | select(.name == "mode") | .kind == "option" and .type == "enum" and (.values | index("basic") != null)' >/dev/null
  echo "$ci_contract" | jq -e '.args[] | select(.name == "mode") | ((["audit","basic","full","mainnet","parity"] - (.values // [])) | length) == 0' >/dev/null
  echo "$ci_contract" | jq -e '[.args[] | select((.name == "basic" or .name == "mode_basic") and .kind == "flag" and .type == "bool")] | length > 0' >/dev/null
  # CI app-contract env surface includes artifacts plus framework runtime primitives.
  # Legacy CI variables remain excluded.
  echo "$ci_contract" | jq -e '[(.env // [])[].name] | index("CI_ARTIFACTS_DIR") != null and index("CI_ARTIFACTS_BASE") != null and index("LOG_LEVEL") != null and index("OUTPUT_MODE") != null and index("CI_LOG_LEVEL") == null and index("CI_VERBOSE") == null and index("NIXFIED_VERBOSE") == null and index("NIXFIED_DEBUG") == null and index("NIXFIED_LOG_LEVEL") == null and index("NIXFIED_OUTPUT_MODE") == null' >/dev/null
  echo "$ci_contract" | jq -e '[.args[].long] | index("--verbose") == null and index("--debug") == null' >/dev/null
  echo "$check_contract" | jq -e '.commandClass == "typed" and .allowUnknownArgs == false and .outputs.mode == "text"' >/dev/null
  echo "$snapshot_contract" | jq -e '.commandClass == "json" and .allowUnknownArgs == false and .outputs.mode == "json"' >/dev/null
  echo "$run_start_contract" | jq -e '.commandClass == "json" and .allowUnknownArgs == false and .outputs.mode == "json"' >/dev/null
  echo "$cli_contract" | jq -e '.commandClass == "passthrough" and .allowUnknownArgs == true' >/dev/null
  echo "$rest_contract" | jq -e '.commandClass == "typed" and .allowUnknownArgs == false and .outputs.mode == "text"' >/dev/null
  echo "$process_status_contract" | jq -e '.commandClass == "passthrough" and .allowUnknownArgs == true and .outputs.mode == "text"' >/dev/null

  # Logging normalization contracts for CI setup/teardown scripts.
  grep -Fq 'normalize_ci_logging_env()' nixfied/project/ci/scripts/setup.nix
  grep -Fq 'reject_legacy_ci_env()' nixfied/project/ci/scripts/setup.nix
  grep -Fq 'deprecated CI env var(s)' nixfied/project/ci/scripts/setup.nix
  grep -Fq 'NIXFIED_LOG_LEVEL:-' nixfied/project/ci/scripts/setup.nix
  grep -Fq 'NIXFIED_OUTPUT_MODE:-' nixfied/project/ci/scripts/setup.nix
  grep -Fq 'LOG_LEVEL:-' nixfied/project/ci/scripts/setup.nix
  grep -Fq 'mode="both"' nixfied/project/ci/scripts/setup.nix
  grep -Fq 'validate_ci_output_mode()' nixfied/project/ci/scripts/setup.nix
  grep -Fq 'export LOG_LEVEL=' nixfied/project/ci/scripts/setup.nix
  grep -Fq 'export OUTPUT_MODE=' nixfied/project/ci/scripts/setup.nix
  grep -Fq 'export LOG_TEE=' nixfied/project/ci/scripts/setup.nix
  grep -Fq 'logs) export LOG_TEE="0"' nixfied/project/ci/scripts/setup.nix
  grep -Fq 'stdout|both) export LOG_TEE="1"' nixfied/project/ci/scripts/setup.nix
  grep -Fq 'OUTPUT_MODE must be one of stdout|logs|both' nixfied/project/ci/scripts/setup.nix
  grep -Fq '_ci_debug_enabled()' nixfied/project/ci/scripts/teardown.nix
  grep -Fq '_ci_output_mode()' nixfied/project/ci/scripts/teardown.nix
  grep -Fq "case \"''\${OUTPUT_MODE:-stdout}\" in" nixfied/.framework/lib/helpers.nix
  grep -Fq 'log_capture tee=' nixfied/.framework/lib/helpers.nix
  grep -Fq 'ci-diagnostics-service-' nixfied/project/ci/scripts/teardown.nix
  grep -Fq -- '-log.log' nixfied/project/ci/scripts/teardown.nix
  grep -Fq '`OUTPUT_MODE`' docs/architecture.md
  grep -Fq '`OUTPUT_MODE`' AGENTS.md
  grep -Fq 'compatibility aliases `NIXFIED_LOG_LEVEL` and `NIXFIED_OUTPUT_MODE` are supported.' nixfied/project/ci.nix
  grep -Fq "RUST_LOG='debug,mfm=trace'" nixfied/project/ci.nix
  if grep -Fq "LOG_LEVEL='debug,mfm=trace'" nixfied/project/ci.nix; then
    echo "ERROR: CI docs still include invalid composite LOG_LEVEL examples" >&2
    exit 1
  fi

  CHECK_UNKNOWN_LOG=$(mktemp)
  set +e
  nix run .#check -- --contract-probe-unknown >"$CHECK_UNKNOWN_LOG" 2>&1
  rc=$?
  set -e
  if [ "$rc" -eq 0 ]; then
    echo "ERROR: expected typed command to reject unknown args" >&2
    cat "$CHECK_UNKNOWN_LOG" >&2 || true
    exit 1
  fi
  if ! grep -Eq "unknown option token=--contract-probe-unknown|Unknown option: --contract-probe-unknown" "$CHECK_UNKNOWN_LOG"; then
    echo "ERROR: expected unknown option diagnostics for typed command" >&2
    cat "$CHECK_UNKNOWN_LOG" >&2 || true
    exit 1
  fi

  JSON_UNKNOWN_LOG=$(mktemp)
  set +e
  nix run .#mfm::run::start -- --contract-probe-unknown >"$JSON_UNKNOWN_LOG" 2>&1
  rc=$?
  set -e
  if [ "$rc" -eq 0 ]; then
    echo "ERROR: expected json-class command to reject unknown args" >&2
    cat "$JSON_UNKNOWN_LOG" >&2 || true
    exit 1
  fi
  if ! grep -Eq "unknown option token=--contract-probe-unknown|Unknown option: --contract-probe-unknown" "$JSON_UNKNOWN_LOG"; then
    echo "ERROR: expected unknown option diagnostics for json-class command" >&2
    cat "$JSON_UNKNOWN_LOG" >&2 || true
    exit 1
  fi

  CLI_UNKNOWN_LOG=$(mktemp)
  set +e
  nix run .#mfm_cli -- --contract-probe-unknown >"$CLI_UNKNOWN_LOG" 2>&1
  rc=$?
  set -e
  if [ "$rc" -eq 0 ]; then
    echo "ERROR: expected mfm_cli to fail downstream for unknown command args" >&2
    cat "$CLI_UNKNOWN_LOG" >&2 || true
    exit 1
  fi
  if grep -q "unknown option token=--contract-probe-unknown" "$CLI_UNKNOWN_LOG"; then
    echo "ERROR: mfm_cli unknown args were rejected by shell-app contract (expected passthrough)" >&2
    cat "$CLI_UNKNOWN_LOG" >&2 || true
    exit 1
  fi

  ARTIFACT_JSON=$(mktemp)
  nix run .#evm-contract-artifact-configurable-counter >"$ARTIFACT_JSON"
  jq -e '.artifact.abi and .artifact.bytecode.object' "$ARTIFACT_JSON" >/dev/null
) >"$LOGFILE" 2>&1
rc=$?
set -e
if [ "$rc" -ne 0 ]; then
  cat "$LOGFILE" >&2 || true
  exit "$rc"
fi
''
