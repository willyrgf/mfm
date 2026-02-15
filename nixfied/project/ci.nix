{ project, lib, ... }:

let
  # v2 shell-app contract inventory (project-level):
  # - ci: typed, outputs=text, wraps CI shell runner + cargo tools, failure map owner=project/ci.nix
  failureCodesScript = {
    generic = 1;
    usage = 2;
    precondition = 3;
    unavailable = 4;
    timeout = 5;
  };
  failureCodesCargo = failureCodesScript // {
    cargoFailure = 101;
  };
  ciModeValues = [
    "basic"
    "audit"
    "parity"
    "mainnet"
  ];
  ciAppContract = {
    version = 2;
    name = "ci";
    allowUnknownArgs = false;
    idempotent = false;
    failureCodes = failureCodesCargo;
    outputs = {
      mode = "text";
    };
    args = [
      {
        name = "basic";
        kind = "flag";
        long = "--basic";
        type = "bool";
        required = false;
      }
      {
        name = "audit";
        kind = "flag";
        long = "--audit";
        type = "bool";
        required = false;
      }
      {
        name = "parity";
        kind = "flag";
        long = "--parity";
        type = "bool";
        required = false;
      }
      {
        name = "mainnet";
        kind = "flag";
        long = "--mainnet";
        type = "bool";
        required = false;
      }
      {
        name = "mode";
        kind = "option";
        long = "--mode";
        type = "enum";
        values = ciModeValues;
        required = false;
      }
      {
        name = "summary";
        kind = "flag";
        long = "--summary";
        type = "bool";
        required = false;
      }
      {
        name = "background";
        kind = "flag";
        long = "--background";
        type = "bool";
        required = false;
      }
      {
        name = "bg";
        kind = "flag";
        long = "--bg";
        type = "bool";
        required = false;
      }
      {
        name = "verbose";
        kind = "flag";
        long = "--verbose";
        type = "bool";
        required = false;
      }
      {
        name = "debug";
        kind = "flag";
        long = "--debug";
        type = "bool";
        required = false;
      }
    ];
    env = [
      {
        name = "CI_ARTIFACTS_DIR";
        type = "string";
        required = false;
      }
      {
        name = "CI_ARTIFACTS_BASE";
        type = "string";
        required = false;
      }
      {
        name = "SERVICE_REUSE_POLICY";
        type = "enum";
        values = [
          "never"
          "same-root"
          "same-slot"
          "cross-run"
        ];
        required = false;
      }
      {
        name = "SERVICE_OWNER_SCOPE";
        type = "enum";
        values = [
          "ephemeral"
          "persistent"
        ];
        required = false;
      }
      {
        name = "SERVICE_DISCOVERY_SCOPE";
        type = "enum";
        values = [
          "local"
          "global"
        ];
        required = false;
      }
      {
        name = "CI_VERBOSE";
        type = "bool";
        required = false;
      }
      {
        name = "NIXFIED_LOG_LEVEL";
        type = "string";
        required = false;
      }
    ];
  };
in
{
  commands = {
    ci = {
      description = "Run the CI pipeline";
      api = lib.appApi.mkApi {
        name = "ci";
        summary = "Run the CI pipeline";
        details = ''
          Runs the CI pipeline defined in `nixfied/project/ci.nix` (modes + steps).

          Supported options (from the framework CI runner):
          - `--basic|--audit|--parity|--mainnet`: select a mode
          - `--mode <name>` or `--mode=<name>`: select a mode by name
          - `--summary`: print a compact summary and write `summary.json` into the artifacts dir
          - `--bg|--background`: run in background via the run registry
          - `--verbose|--debug`: enable debug logs (including teardown diagnostic collection details)

          Readiness-first behavior:
          - parity/mainnet workflows gate on service `*_READY` hooks
          - parity fixtures with `profile = "test"` use profile-specific readiness when available (Postgres uses `POSTGRES_READY_TEST`)
          - fixture logs are persisted under CI artifacts for debugging

          Process-first diagnostics:
          - `nix run .#process::status -- --all`
          - `nix run .#process::runs -- --all`
          - `nix run .#process::inspect -- <run-id>`
          - `nix run .#service::postgres::events -- --limit 100`
        '';
        usage = [
          "nix run .#ci -- --basic --summary"
          "nix run .#ci -- --audit --summary"
          "nix run .#ci -- --parity --summary"
          "nix run .#ci -- --mode basic --summary"
          "nix run .#ci -- --mode=basic --summary"
          "nix run .#ci -- --bg"
          "nix run .#ci -- --background"
        ];
        examples = [
          "nix run .#ci -- --basic --summary"
          "CI_ARTIFACTS_DIR=/tmp/ci-artifacts nix run .#ci -- --parity --summary"
          "nix run .#process::status -- --all"
        ];
        args = [
          {
            name = "--basic";
            description = "Select basic mode.";
          }
          {
            name = "--audit";
            description = "Select audit mode.";
          }
          {
            name = "--parity";
            description = "Select parity mode.";
          }
          {
            name = "--mainnet";
            description = "Select mainnet mode.";
          }
          {
            name = "--mode";
            description = "Select CI mode by name (value: <name>).";
          }
          {
            name = "--summary";
            description = "Print a compact summary and write artifacts/summary.json.";
          }
          {
            name = "--bg";
            description = "Run CI in background via the run registry.";
          }
          {
            name = "--background";
            description = "Alias for --bg.";
          }
          {
            name = "--verbose";
            description = "Enable debug logging for framework and CI teardown diagnostics.";
          }
          {
            name = "--debug";
            description = "Alias for --verbose debug logging.";
          }
        ];
        env = [
          {
            name = "CI_ARTIFACTS_DIR";
            description = "Override artifacts directory (default: /tmp/ci-artifacts).";
          }
          {
            name = "CI_ARTIFACTS_BASE";
            description = "Override artifacts root directory (must be absolute path).";
          }
          {
            name = "SERVICE_REUSE_POLICY";
            description = "Optional process-first policy override (never|same-root|same-slot|cross-run).";
          }
          {
            name = "SERVICE_OWNER_SCOPE";
            description = "Optional process-first policy override (ephemeral|persistent).";
          }
          {
            name = "SERVICE_DISCOVERY_SCOPE";
            description = "Optional process-first policy override (local|global).";
          }
          {
            name = "CI_VERBOSE";
            description = "Set to 1/true to enable debug logs (equivalent to --verbose).";
          }
          {
            name = "NIXFIED_LOG_LEVEL";
            description = "Set to debug to enable framework debug logs.";
          }
        ];
        category = "core";
        allowUnknownArgs = false;
        failureCodes = failureCodesCargo;
        idempotent = false;
        appContract = ciAppContract;
      };
      env = {
        "${project.envVar}" = "test";
      };
      useDeps = true;
      # Note: `nix run .#ci` is implemented by the framework CI runner (nixfied/.framework/ci.nix),
      # which reads `project.ci.*` below. This command exists for `nix run .#help`, and
      # `commands.ci.api` is the canonical CI docs source mirrored into app metadata.
      script = "";
    };
  };

  ci = {
    enable = true;
    defaultMode = "basic";
    env = {
      "${project.envVar}" = "test";
      CARGO_TERM_COLOR = "always";
      RUST_BACKTRACE = "1";
      SERVICE_OWNER_SCOPE = "persistent";
      SERVICE_DISCOVERY_SCOPE = "global";
      SERVICE_REUSE_POLICY = "same-slot";
    };
    useDeps = true;
    setup = ''
      eval "$($SLOT_INFO)"

      kill_conflicting_listener() {
        local port="$1"
        local label="$2"
        local expected_token="''${3:-}"
        local pids=""
        local filtered_pids=""
        local remaining=""
        local cmd=""
        local pid=""

        if [ -z "$port" ] || ! echo "$port" | grep -Eq '^[0-9]+$'; then
          return 0
        fi

        if ! command -v lsof >/dev/null 2>&1; then
          echo "WARN: lsof not available; skipping conflict cleanup label=$label port=$port" >&2
          return 0
        fi

        pids="$(
          {
            lsof -tiTCP:"$port" -sTCP:LISTEN -n -P 2>/dev/null || true
            lsof -tiUDP:"$port" -n -P 2>/dev/null || true
          } | sort -u
        )"
        if [ -z "$pids" ]; then
          return 0
        fi

        if [ -n "$expected_token" ]; then
          while IFS= read -r pid; do
            [ -z "$pid" ] && continue
            cmd=$(ps -o command= -p "$pid" 2>/dev/null || true)
            if echo "$cmd" | grep -F "$expected_token" >/dev/null 2>&1; then
              filtered_pids="$filtered_pids
$pid"
            fi
          done <<<"$pids"
        else
          filtered_pids="$pids"
        fi

        filtered_pids="$(echo "$filtered_pids" | sed '/^$/d' || true)"
        if [ -z "$filtered_pids" ]; then
          return 0
        fi

        echo "INFO: stopping conflicting listener label=$label port=$port pids=$(echo "$filtered_pids" | tr '\n' ' ')" >&2
        echo "$filtered_pids" | xargs kill -TERM 2>/dev/null || true

        for _ in $(seq 1 10); do
          remaining="$(
            {
              lsof -tiTCP:"$port" -sTCP:LISTEN -n -P 2>/dev/null || true
              lsof -tiUDP:"$port" -n -P 2>/dev/null || true
            } | sort -u
          )"
          if [ -z "$remaining" ]; then
            break
          fi
          sleep 0.2
        done

        remaining="$(
          {
            lsof -tiTCP:"$port" -sTCP:LISTEN -n -P 2>/dev/null || true
            lsof -tiUDP:"$port" -n -P 2>/dev/null || true
          } | sort -u
        )"
        if [ -n "$expected_token" ]; then
          filtered_pids=""
          while IFS= read -r pid; do
            [ -z "$pid" ] && continue
            cmd=$(ps -o command= -p "$pid" 2>/dev/null || true)
            if echo "$cmd" | grep -F "$expected_token" >/dev/null 2>&1; then
              filtered_pids="$filtered_pids
$pid"
            fi
          done <<<"$remaining"
          remaining="$(echo "$filtered_pids" | sed '/^$/d' || true)"
        fi

        if [ -n "$remaining" ]; then
          echo "WARN: force-killing listener label=$label port=$port pids=$(echo "$remaining" | tr '\n' ' ')" >&2
          echo "$remaining" | xargs kill -KILL 2>/dev/null || true
        fi
      }

      # Parity flows rely on MinIO fixture bootstrap. Clean stale listeners from
      # previous slot-shared runs before steps execute.
      kill_conflicting_listener "''${MINIO_PORT:-}" "minio-api" "minio"
      kill_conflicting_listener "''${MINIO_CONSOLE_PORT:-}" "minio-console" "minio"
      kill_conflicting_listener "''${RETHHTTP_PORT:-}" "reth-http" "reth"
      kill_conflicting_listener "''${RETHWS_PORT:-}" "reth-ws" "reth"
      kill_conflicting_listener "''${RETHAUTH_PORT:-}" "reth-auth" "reth"
      kill_conflicting_listener "30303" "reth-p2p" "reth"
    '';
    teardown = ''
      # Keep teardown diagnostics best-effort so step failures remain the primary CI exit code.
      CI_DIAG_EVENTS_LIMIT="''${CI_DIAG_EVENTS_LIMIT:-200}"

      _ci_truthy() {
        case "''${1:-}" in
          1|true|TRUE|yes|YES|on|ON) return 0 ;;
          *) return 1 ;;
        esac
      }

      ci_debug() {
        if _ci_truthy "''${CI_VERBOSE:-0}" || _ci_truthy "''${NIXFIED_VERBOSE:-0}" || _ci_truthy "''${NIXFIED_DEBUG:-0}"; then
          echo "DEBUG: $*" >&2
          return 0
        fi
        case "''${NIXFIED_LOG_LEVEL:-}" in
          debug|DEBUG|trace|TRACE)
            echo "DEBUG: $*" >&2
            ;;
        esac
      }

      capture_diag() {
        local name="$1"
        local kind="$2"
        shift 2
        local outfile=""
        outfile=$(artifact_path "$name")
        ci_debug "collecting ci diagnostic name=$name kind=$kind outfile=$outfile"
        set +e
        "$@" >"$outfile" 2>&1
        local rc=$?
        set -e

        # Service status diagnostics return rc=1 when service is simply not running.
        # Treat that as expected and avoid warning noise in successful runs.
        if [ "$rc" -ne 0 ] && [ "$kind" = "service-status" ]; then
          if grep -Eq '(^|[[:space:]])running=false([[:space:]]|$)' "$outfile"; then
            ci_debug "diagnostic status indicates service not running name=$name rc=$rc (expected)"
            return 0
          fi
        fi

        if [ "$rc" -ne 0 ]; then
          echo "WARN: diagnostic command failed name=$name rc=$rc" >&2
        fi
      }

      capture_service_diag() {
        local service="$1"
        local token=""
        local status_hook=""
        local events_hook=""

        token=$(echo "$service" | tr '[:lower:]' '[:upper:]' | tr '.:/-' '_')
        status_hook="''${token}_STATUS"
        events_hook="''${token}_EVENTS"

        if has_hook "$status_hook"; then
          capture_diag "ci-diagnostics-service-''${service}-status.log" "service-status" run_hook "$status_hook"
        fi

        if has_hook "$events_hook"; then
          capture_diag "ci-diagnostics-service-''${service}-events.log" "service-events" run_hook "$events_hook" --limit "$CI_DIAG_EVENTS_LIMIT"
        fi
      }

      capture_diag "ci-diagnostics-process-status.log" "process" nix run .#process::status -- --all
      capture_diag "ci-diagnostics-process-runs.log" "process" nix run .#process::runs -- --all
      capture_diag "ci-diagnostics-process-slots.log" "process" nix run .#process::slots -- --all
      capture_diag "ci-diagnostics-process-gc.log" "process" nix run .#process::gc

      for service in postgres minio reth helios nginx; do
        capture_service_diag "$service"
      done
    '';
    failureSignals = [ ];
    runsRoot = "/tmp/${project.id}-runs";
    useEphemeral = true;
    artifacts = {
      dir = "/tmp/ci-artifacts";
      keepOnFailure = true;
      keepOnSuccess = false;
    };
    modes = {
      basic = {
        steps = [
          "fmt"
          "clippy"
          "architecture-verify"
          "shell-app-contracts"
          "build"
          "tests"
        ];
      };
      audit = {
        steps = [ "audit" ];
      };
      parity = {
        steps = [
          "parity-postgres"
          "parity-s3"
          "parity-rest-api-smoke"
          "parity-evm-reth"
          "parity-portfolio-tracker-reth"
          "parity-keystore-reth-tx-sign-send"
          "parity-evm-helios-smoke"
        ];
      };
      mainnet = {
        steps = [ "mainnet-portfolio-snapshot-helios" ];
      };
    };
    steps = {
      tests = {
        description = "Tests";
        run = ''
          LOGFILE=$(artifact_path "tests.log")
          log_capture "$LOGFILE" -- cargo nextest run --workspace
        '';
      };
      fmt = {
        description = "Formatting (nightly)";
        run = ''
          LOGFILE=$(artifact_path "fmt.log")
          log_capture "$LOGFILE" -- cargo-nightly fmt --all -- --check
        '';
      };
      clippy = {
        description = "Clippy (nightly)";
        run = ''
          LOGFILE=$(artifact_path "clippy.log")
          log_capture "$LOGFILE" -- cargo-nightly clippy --workspace --lib --examples --tests --benches --all-features
        '';
      };
      architecture-verify = {
        description = "Architecture verifier";
        run = ''
          LOGFILE=$(artifact_path "architecture-verify.log")
          log_capture "$LOGFILE" -- cargo run -p mfm-architecture-verify --
        '';
      };
      shell-app-contracts = {
        description = "Shell app contract checks (strict typed + passthrough)";
        run = ''
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
            svc_logs_contract=$(nix eval --json ".#apps.$SYSTEM.\"svc-logs\".meta.nixfied.api.appContract")

            echo "$ci_contract" | jq -e '.allowUnknownArgs == false' >/dev/null
            echo "$ci_contract" | jq -e '.args[] | select(.name == "mode") | .kind == "option" and .type == "enum" and (.values | index("basic") != null)' >/dev/null
            echo "$ci_contract" | jq -e '.args[] | select(.name == "mode") | (.values | sort) == ["audit","basic","mainnet","parity"]' >/dev/null
            echo "$ci_contract" | jq -e '.args[] | select(.name == "basic") | .kind == "flag" and .type == "bool"' >/dev/null
            echo "$check_contract" | jq -e '.allowUnknownArgs == false' >/dev/null
            echo "$snapshot_contract" | jq -e '.allowUnknownArgs == false and .outputs.mode == "json"' >/dev/null
            echo "$cli_contract" | jq -e '.allowUnknownArgs == true' >/dev/null
            echo "$rest_contract" | jq -e '.allowUnknownArgs == true' >/dev/null
            echo "$svc_logs_contract" | jq -e '.allowUnknownArgs == true' >/dev/null

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
        '';
      };
      build = {
        description = "Build (release, all features)";
        run = ''
          LOGFILE=$(artifact_path "build.log")
          log_capture "$LOGFILE" -- cargo build --release --all-features
        '';
      };
      audit = {
        description = "Security audit (cargo-audit)";
        run = ''
          LOGFILE=$(artifact_path "audit.log")
          log_capture "$LOGFILE" -- cargo audit
        '';
      };

      parity-postgres = {
        description = "Parity: Postgres event store";
        env = {
          AUTO_STOP_CONFLICTING = "1";
        };
        fixtures = {
          services = [
            {
              name = "postgres";
              profile = "test";
            }
          ];
          artifacts = {
            logs = true;
            prefix = "parity-postgres";
          };
        };
        run = ''
          eval "$($SLOT_INFO)"
          # Fixture `postgres` with profile `test` already waits on POSTGRES_READY_TEST.

          export DATABASE_URL="postgresql://postgres:postgres@127.0.0.1:$POSTGRES_PORT/mfm_test"

          LOGFILE=$(artifact_path "parity-postgres.log")
          log_capture "$LOGFILE" -- cargo nextest run -p mfm-integration-tests --features parity-tests --test parity_event_store_postgres_contract
        '';
      };

      parity-s3 = {
        description = "Parity: S3/MinIO artifact store";
        fixtures = {
          services = [
            {
              name = "minio";
              profile = "test";
              exports = [ "s3" ];
              bucket = "mfm-test";
              prefix = "mfm-artifacts";
              region = "us-east-1";
              bootstrap = [ "mfm-test" ];
              logName = "minio.log";
            }
          ];
          artifacts = {
            logs = true;
            prefix = "parity-s3";
          };
        };
        run = ''
          export MFM_S3_ENDPOINT="$MINIO_ENDPOINT"
          export MFM_S3_REGION="$MINIO_REGION"
          export MFM_S3_BUCKET="$MINIO_BUCKET"
          export MFM_S3_PREFIX="$MINIO_PREFIX"

          LOGFILE=$(artifact_path "parity-s3.log")
          log_capture "$LOGFILE" -- cargo nextest run -p mfm-integration-tests --features parity-tests --test parity_artifact_store_s3_contract
        '';
      };

      parity-rest-api-smoke = {
        description = "Parity: REST API smoke on Postgres + S3/MinIO";
        env = {
          AUTO_STOP_CONFLICTING = "1";
        };
        fixtures = {
          services = [
            {
              name = "postgres";
              profile = "test";
            }
            {
              name = "minio";
              profile = "test";
              exports = [ "s3" ];
              bucket = "mfm-test";
              prefix = "mfm-artifacts";
              region = "us-east-1";
              bootstrap = [ "mfm-test" ];
              logName = "minio-rest-api-smoke.log";
            }
          ];
          artifacts = {
            logs = true;
            prefix = "parity-rest-api-smoke";
          };
        };
        run = ''
          eval "$($SLOT_INFO)"

          export DATABASE_URL="postgresql://postgres:postgres@127.0.0.1:$POSTGRES_PORT/mfm_test"

          export MFM_S3_ENDPOINT="$MINIO_ENDPOINT"
          export MFM_S3_REGION="$MINIO_REGION"
          export MFM_S3_BUCKET="$MINIO_BUCKET"
          export MFM_S3_PREFIX="$MINIO_PREFIX"

          LOGFILE=$(artifact_path "parity-rest-api-smoke.log")
          log_capture "$LOGFILE" -- cargo nextest run -p mfm-integration-tests --features parity-tests --test parity_rest_api_postgres_s3_smoke
        '';
      };

      parity-evm-reth = {
        description = "Parity: EVM pipeline deploy/configure/validate on reth";
        env = {
          AUTO_STOP_CONFLICTING = "1";
        };
        fixtures = {
          services = [
            {
              name = "postgres";
              profile = "test";
            }
            {
              name = "minio";
              profile = "test";
              exports = [ "s3" ];
              bucket = "mfm-test";
              prefix = "mfm-artifacts";
              region = "us-east-1";
              bootstrap = [ "mfm-test" ];
              logName = "minio-evm.log";
            }
            {
              name = "reth";
              profile = "test";
              logName = "reth.log";
            }
          ];
          artifacts = {
            logs = true;
            prefix = "parity-evm-reth";
          };
        };
        run = ''
          eval "$($SLOT_INFO)"

          export DATABASE_URL="postgresql://postgres:postgres@127.0.0.1:$POSTGRES_PORT/mfm_test"

          export MFM_S3_ENDPOINT="$MINIO_ENDPOINT"
          export MFM_S3_REGION="$MINIO_REGION"
          export MFM_S3_BUCKET="$MINIO_BUCKET"
          export MFM_S3_PREFIX="$MINIO_PREFIX"

          export MFM_EVM_RPC_URL="http://127.0.0.1:$RETHHTTP_PORT"

          LOGFILE=$(artifact_path "parity-evm-reth.log")
          log_capture "$LOGFILE" -- cargo nextest run -p mfm-integration-tests --features parity-tests --test parity_rest_api_evm_reth_pipeline
        '';
      };

      parity-portfolio-tracker-reth = {
        description = "Parity: portfolio_tracker snapshot against reth (MockERC20)";
        env = {
          AUTO_STOP_CONFLICTING = "1";
        };
        fixtures = {
          services = [
            {
              name = "postgres";
              profile = "test";
            }
            {
              name = "minio";
              profile = "test";
              exports = [ "s3" ];
              bucket = "mfm-test";
              prefix = "mfm-artifacts";
              region = "us-east-1";
              bootstrap = [ "mfm-test" ];
              logName = "minio-portfolio-tracker.log";
            }
            {
              name = "reth";
              profile = "test";
              logName = "reth-portfolio-tracker.log";
            }
          ];
          artifacts = {
            logs = true;
            prefix = "parity-portfolio-tracker-reth";
          };
        };
        run = ''
          eval "$($SLOT_INFO)"

          export DATABASE_URL="postgresql://postgres:postgres@127.0.0.1:$POSTGRES_PORT/mfm_test"

          export MFM_S3_ENDPOINT="$MINIO_ENDPOINT"
          export MFM_S3_REGION="$MINIO_REGION"
          export MFM_S3_BUCKET="$MINIO_BUCKET"
          export MFM_S3_PREFIX="$MINIO_PREFIX"

          export MFM_EVM_RPC_URL="http://127.0.0.1:$RETHHTTP_PORT"

          LOGFILE=$(artifact_path "parity-portfolio-tracker-reth.log")
          log_capture "$LOGFILE" -- cargo nextest run -p mfm-integration-tests --features parity-tests --test parity_portfolio_tracker_reth_mock_erc20
        '';
      };

      parity-keystore-reth-tx-sign-send = {
        description = "Parity: keystore CLI import/sign/send raw tx on reth";
        env = {
          AUTO_STOP_CONFLICTING = "1";
        };
        fixtures = {
          services = [
            {
              name = "reth";
              profile = "test";
              logName = "reth-keystore-cli.log";
            }
          ];
          artifacts = {
            logs = true;
            prefix = "parity-keystore-reth-tx-sign-send";
          };
        };
        run = ''
          eval "$($SLOT_INFO)"

          export MFM_EVM_RPC_URL="http://127.0.0.1:$RETHHTTP_PORT"

          LOGFILE=$(artifact_path "parity-keystore-reth-tx-sign-send.log")
          log_capture "$LOGFILE" -- cargo nextest run -p mfm --features parity-tests --test parity_keystore_reth_tx_send
        '';
      };

      parity-evm-helios-smoke = {
        description = "Parity: Helios lifecycle and RPC smoke over reth execution";
        fixtures = {
          services = [
            {
              name = "reth";
              profile = "test";
              logName = "reth-helios.log";
            }
          ];
          artifacts = {
            logs = true;
            prefix = "parity-evm-helios-smoke";
          };
        };
        run = ''
          eval "$($SLOT_INFO)"

          export HELIOS_NETWORK="local"
          export HELIOS_EXECUTION_RPC_URL="http://127.0.0.1:$RETHHTTP_PORT"
          export HELIOS_CONSENSUS_RPC_URL="http://127.0.0.1:$RETHHTTP_PORT"
          export HELIOS_READY_TIMEOUT_SECS="''${HELIOS_READY_TIMEOUT_SECS:-120}"
          HELIOS_SERVICE_LOG=$(artifact_path "parity-evm-helios-service.log")
          fixture_start_service helios test "''${HELIOS_FIXTURE_TIMEOUT_SECS:-120}" 1 "$HELIOS_SERVICE_LOG"

          LOGFILE=$(artifact_path "parity-evm-helios-smoke.log")
          curl -fsS \
            -H 'content-type: application/json' \
            --data '{"jsonrpc":"2.0","id":1,"method":"eth_chainId","params":[]}' \
            "http://127.0.0.1:$HELIOSRPC_PORT" \
            | tee "$LOGFILE" \
            | jq -e '.result | strings' >/dev/null
        '';
      };

      mainnet-portfolio-snapshot-helios = {
        description = "Mainnet: mfm::portfolio::snapshot (Helios) and assert Vitalik has ETH";
        run = ''
          eval "$($SLOT_INFO)"

          export HELIOS_NETWORK="mainnet"
          # Execution RPC must support `eth_getProof` for explicit block numbers (not just `latest`).
          export HELIOS_EXECUTION_RPC_URL="''${HELIOS_EXECUTION_RPC_URL:-https://eth.drpc.org}"

          # Prefer a stable, up-to-date consensus endpoint for CI (faster and less flaky than relying
          # on the default if it is temporarily unavailable).
          export HELIOS_CONSENSUS_RPC_URL="''${HELIOS_CONSENSUS_RPC_URL:-https://lodestar-mainnet.chainsafe.io}"

          # Mainnet Helios can take a while to sync; gate on eth_blockNumber.
          export HELIOS_READY_TIMEOUT_SECS="''${HELIOS_READY_TIMEOUT_SECS:-900}"

          export MFM_ARTIFACT_ROOT="$CI_ARTIFACTS_DIR/mfm-mainnet-artifacts"
          mkdir -p "$MFM_ARTIFACT_ROOT"

          ADDRESS="0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045"

          LOGFILE=$(artifact_path "mainnet-portfolio-snapshot.log")
          RAW_OUTFILE=$(artifact_path "mainnet-portfolio-snapshot.raw.out")
          OUTFILE=$(artifact_path "mainnet-portfolio-snapshot.json")

          set +e
          nix run .#mfm::portfolio::snapshot -- "$ADDRESS" >"$RAW_OUTFILE" 2>"$LOGFILE"
          rc=$?
          set -e

          if [ $rc -ne 0 ]; then
            echo "ERROR: mfm::portfolio::snapshot failed rc=$rc" >&2

            HELIOS_EVENTS_FILE=$(artifact_path "mainnet-helios-events.log")
            HELIOS_SERVICE_LOG_FILE=$(artifact_path "mainnet-helios-service-log.log")
            PROCESS_INSPECT_FILE=$(artifact_path "mainnet-process-inspect.log")

            if has_hook HELIOS_EVENTS; then
              run_hook HELIOS_EVENTS -- --limit 200 >"$HELIOS_EVENTS_FILE" 2>&1 || true
              echo "INFO: helios events (tail 200) path=$HELIOS_EVENTS_FILE" >&2
              tail -200 "$HELIOS_EVENTS_FILE" >&2 || true
            fi

            if has_hook HELIOS_LOG; then
              run_hook HELIOS_LOG -- --lines 200 >"$HELIOS_SERVICE_LOG_FILE" 2>&1 || true
              echo "INFO: helios service log (tail 200) path=$HELIOS_SERVICE_LOG_FILE" >&2
              tail -200 "$HELIOS_SERVICE_LOG_FILE" >&2 || true
            fi

            if [ -n "''${RUN_ID:-}" ]; then
              nix run .#process::inspect -- "$RUN_ID" >"$PROCESS_INSPECT_FILE" 2>&1 || true
              echo "INFO: process inspect path=$PROCESS_INSPECT_FILE run_id=$RUN_ID" >&2
              tail -200 "$PROCESS_INSPECT_FILE" >&2 || true
            fi

            tail -200 "$LOGFILE" >&2 || true
            if [ -s "$RAW_OUTFILE" ]; then
              echo "STDOUT:" >&2
              cat "$RAW_OUTFILE" >&2 || true
            fi
            exit $rc
          fi

          # The app contract is final JSON on stdout, but Rust/tracing notices may still appear
          # ahead of the payload in some environments. Keep only the JSON document.
          sed -n '/^{/,$p' "$RAW_OUTFILE" >"$OUTFILE"
          if ! jq -e . "$OUTFILE" >/dev/null; then
            echo "ERROR: snapshot output is not valid JSON" >&2
            echo "RAW STDOUT:" >&2
            cat "$RAW_OUTFILE" >&2 || true
            echo "STDERR LOG:" >&2
            tail -200 "$LOGFILE" >&2 || true
            exit 1
          fi

          jq -e '.status == "success"' "$OUTFILE" >/dev/null
          jq -e '.data.feature_id == "portfolio.snapshot"' "$OUTFILE" >/dev/null
          jq -e '.data.result.phase == "completed"' "$OUTFILE" >/dev/null
          jq -e '.data.result.snapshot | type == "object"' "$OUTFILE" >/dev/null
          jq -e '.data.result.snapshot.chain_id == .data.result.chain_id' "$OUTFILE" >/dev/null
          jq -e '.data.result.snapshot.block_number == .data.result.block_number' "$OUTFILE" >/dev/null

          ART_ID=$(jq -r '.data.result.snapshot_artifact_id // empty' "$OUTFILE")
          if [ -z "$ART_ID" ] || [ "$ART_ID" = "null" ]; then
            echo "ERROR: missing snapshot_artifact_id" >&2
            cat "$OUTFILE" >&2
            exit 1
          fi

          SNAPSHOT_FILE="$MFM_ARTIFACT_ROOT/''${ART_ID:0:2}/$ART_ID"
          if [ ! -f "$SNAPSHOT_FILE" ]; then
            echo "ERROR: snapshot artifact not found at $SNAPSHOT_FILE" >&2
            exit 1
          fi

          INLINE_BAL_WEI=$(jq -r '.data.result.snapshot.native.raw_u256_dec // empty' "$OUTFILE")
          BAL_WEI=$(jq -r '.native.raw_u256_dec // empty' "$SNAPSHOT_FILE")
          MIN_BAL_WEI="32000000000000000000"

          if [ -z "$INLINE_BAL_WEI" ] || [ "$INLINE_BAL_WEI" = "null" ] || ! echo "$INLINE_BAL_WEI" | grep -Eq '^[0-9]+$'; then
            echo "ERROR: invalid embedded ETH balance; got data.result.snapshot.native.raw_u256_dec=$INLINE_BAL_WEI" >&2
            cat "$OUTFILE" >&2
            exit 1
          fi

          if [ -z "$BAL_WEI" ] || [ "$BAL_WEI" = "null" ] || ! echo "$BAL_WEI" | grep -Eq '^[0-9]+$'; then
            echo "ERROR: invalid ETH balance; got native.raw_u256_dec=$BAL_WEI" >&2
            cat "$SNAPSHOT_FILE" >&2
            exit 1
          fi

          if [ "$INLINE_BAL_WEI" != "$BAL_WEI" ]; then
            echo "ERROR: embedded snapshot balance mismatch inline=$INLINE_BAL_WEI artifact=$BAL_WEI" >&2
            exit 1
          fi

          # Compare large decimal integers without relying on 64-bit shell arithmetic.
          if [ "''${#BAL_WEI}" -lt "''${#MIN_BAL_WEI}" ] || { [ "''${#BAL_WEI}" -eq "''${#MIN_BAL_WEI}" ] && [ "$BAL_WEI" \< "$MIN_BAL_WEI" ]; }; then
            echo "ERROR: expected at least 32 ETH (wei >= $MIN_BAL_WEI); got native.raw_u256_dec=$BAL_WEI" >&2
            cat "$SNAPSHOT_FILE" >&2
            exit 1
          fi

          echo "OK: mainnet snapshot ETH balance >= 32 ETH wei=$BAL_WEI artifact_id=$ART_ID"
        '';
      };
    };
  };
}
