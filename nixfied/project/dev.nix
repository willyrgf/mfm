{ project, lib, ... }:

let
  # v2 shell-app contract inventory (project-level):
  # - dev: typed, outputs=text, wraps fixture orchestration + cargo run, failure map owner=project/dev.nix
  # - mfm_cli: passthrough, outputs=text, wraps cargo run, failure map owner=project/dev.nix
  # - mfm::portfolio::snapshot: json, outputs=json, wraps fixtures + cargo run, failure map owner=project/dev.nix
  # - mfm_rest_api: typed, outputs=text, wraps cargo run, failure map owner=project/dev.nix
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
  snapshotAppContract = {
    version = 2;
    name = "mfm::portfolio::snapshot";
    commandClass = "json";
    allowUnknownArgs = false;
    idempotent = false;
    failureCodes = failureCodesCargo;
    outputs = {
      mode = "json";
    };
    args = [
      {
        name = "help";
        kind = "flag";
        long = "--help";
        short = "-h";
        type = "bool";
        required = false;
      }
      {
        name = "address";
        kind = "positional";
        type = "string";
        required = false;
      }
    ];
    env = [
      {
        name = "HELIOS_NETWORK";
        type = "enum";
        values = [ "mainnet" ];
        required = false;
      }
      {
        name = "HELIOS_EXECUTION_RPC_URL";
        type = "string";
        required = false;
      }
      {
        name = "HELIOS_CONSENSUS_RPC_URL";
        type = "string";
        required = false;
      }
      {
        name = "HELIOS_CHECKPOINT";
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
    ];
  };
in
{
  commands = {
    dev = {
      description = "Start the dev workflow";
      api = lib.appApi.mkTypedCommandApi {
        name = "dev";
        summary = "Start the dev workflow (Postgres + MinIO + Reth + REST API)";
        details = ''
          Starts Postgres, MinIO (S3), Reth, and the REST API for local development.

          Startup is readiness-gated by fixture orchestration plus REST API probe:
          - `POSTGRES_READY`
          - `MINIO_READY`
          - `RETH_READY`
          - REST API `/v1/ready`

          Process-first observability:
          - `nix run .#process::status`
          - `nix run .#process::runs -- --all`
          - `nix run .#service::postgres::events -- --limit 50`
          - `nix run .#service::reth::log -- --lines 200`
        '';
        usage = [
          "nix run .#dev"
          "NIX_ENV=1 nix run .#dev"
        ];
        examples = [
          "nix run .#dev"
          "MFM_ENV=dev NIX_ENV=2 nix run .#dev"
        ];
        category = "core";
        failureCodes = failureCodesCargo;
        idempotent = false;
      };
      env = {
        "${project.envVar}" = "dev";
      };
      useDeps = true;
      fixtures = {
        services = [
          {
            name = "postgres";
            profile = "dev";
          }
          {
            name = "minio";
            profile = "dev";
            exports = [ "s3" ];
          }
          {
            name = "reth";
            profile = "dev";
          }
        ];
      };
      script = ''
        eval "$($SLOT_INFO)"

        export DATABASE_URL="postgresql://postgres:postgres@127.0.0.1:$POSTGRES_PORT/mfm"
        export MFM_REST_API_ADDR="127.0.0.1:$REST_API_PORT"
        export MFM_EVM_RPC_URL="http://127.0.0.1:$RETHHTTP_PORT"

        start_service rest-api \
          --wait-http "http://127.0.0.1:$REST_API_PORT/v1/ready" \
          -- \
          cargo run -p mfm-rest-api --bin mfm_rest_api

        wait
      '';
    };

    mfm_cli = {
      description = "Run mfm_cli (Cargo run)";
      api = lib.appApi.mkPassthroughCommandApi {
        name = "mfm_cli";
        summary = "Run mfm_cli";
        details = "Runs the CLI binary via `cargo run` inside the pinned Nix environment. Passthrough wrapper: arguments after `--` are forwarded unchanged.";
        usage = [
          "nix run .#mfm_cli -- --help"
          "nix run .#mfm_cli -- <args>"
        ];
        category = "core";
        failureCodes = failureCodesCargo;
      };
      env = { };
      useDeps = true;
      script = ''
        exec cargo run -p mfm --bin mfm_cli -- "$@"
      '';
    };

    "mfm::portfolio::snapshot" = {
      description = "Snapshot a wallet via Helios-backed mainnet RPC (starts Postgres + Helios)";
      api = lib.appApi.mkJsonCommandApi {
        name = "mfm::portfolio::snapshot";
        summary = "Snapshot an Ethereum wallet portfolio (Helios-backed)";
        details = ''
          Starts Postgres + Helios (local JSON-RPC), then runs:
            `mfm_cli portfolio snapshot <ADDRESS> --chain-id 1`

          Note: Helios mainnet configuration is expected to come from a root `.env` (Nixfied loads
          it automatically), e.g. `HELIOS_NETWORK=mainnet`, `HELIOS_EXECUTION_RPC_URL=...`,
          and optionally `HELIOS_CONSENSUS_RPC_URL=...` / `HELIOS_CHECKPOINT=...`.

          If `HELIOS_EXECUTION_RPC_URL` is unset, this app falls back to the internal default:
          `https://eth.drpc.org`.

          `MFM_KEEP_SERVICES` is deprecated and no longer accepted.
          For process-first reuse/ownership, set:
          - `SERVICE_REUSE_POLICY=same-slot|cross-run`
          - or `SERVICE_OWNER_SCOPE=persistent`
          - or `SERVICE_DISCOVERY_SCOPE=global`

          You can also start services explicitly:
          - `nix run .#service::postgres::start`
          - `nix run .#service::helios::start`
          and inspect reuse/ownership with:
          - `nix run .#process::status -- --all`
          - `nix run .#service::helios::events -- --limit 100`
        '';
        usage = [
          "nix run .#mfm::portfolio::snapshot -- <ADDRESS>"
        ];
        examples = [
          "nix run .#mfm::portfolio::snapshot -- 0x000000000000000000000000000000000000dead"
        ];
        category = "mfm";
        args = [
          {
            name = "address";
            description = "Ethereum address (0x...) to snapshot.";
          }
        ];
        env = [
          {
            name = "HELIOS_NETWORK";
            description = "Helios network (e.g. mainnet). Loaded from `.env` if present.";
          }
          {
            name = "HELIOS_EXECUTION_RPC_URL";
            description = "Upstream execution JSON-RPC URL for Helios (HTTP/WSS). Loaded from `.env` if present; defaults to https://eth.drpc.org when unset.";
          }
          {
            name = "HELIOS_CONSENSUS_RPC_URL";
            description = "Upstream consensus endpoint for Helios (Beacon API). Optional for mainnet: defaults to https://www.lightclientdata.org. Loaded from `.env` if present.";
          }
          {
            name = "HELIOS_CHECKPOINT";
            description = "Optional weak-subjectivity checkpoint (0x...). If unset for non-local networks, the Nixfied Helios service will derive one from the consensus endpoint at start time.";
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
        ];
        failureCodes = failureCodesCargo;
        idempotent = false;
        appContract = snapshotAppContract;
      };
      env = {
        "${project.envVar}" = "dev";
      };
      useDeps = true;
      script = ''
        set -euo pipefail

        # Reserve stdout for the final JSON output (for CI/scripting).
        exec 3>&1
        exec 1>&2

        sanitize_json_payload() {
          local raw_file="$1"
          local clean_file="$2"
          local label="$3"

          # Keep content from the first JSON object line onward. This isolates
          # machine-readable payloads from incidental log prelude lines.
          sed -n '/^[[:space:]]*{/,$p' "$raw_file" >"$clean_file"
          if [ ! -s "$clean_file" ] || ! jq -e . "$clean_file" >/dev/null; then
            echo "ERROR: $label is not valid JSON" >&2
            echo "RAW OUTPUT ($label):" >&2
            cat "$raw_file" >&2 || true
            exit 1
          fi
        }

        if [ "$#" -ne 1 ] || [ "''${1:-}" = "--help" ] || [ "''${1:-}" = "-h" ]; then
          echo "usage: nix run .#mfm::portfolio::snapshot -- <ADDRESS>" >&2
          exit 2
        fi

        ADDRESS="$1"

        eval "$($SLOT_INFO)"

        # Keep this app mainnet-only (chain-id 1) to avoid accidental local/reth wiring.
        if [ "''${HELIOS_NETWORK:-}" != "mainnet" ]; then
          echo "ERROR: HELIOS_NETWORK must be 'mainnet' for mfm::portfolio::snapshot (set it in .env)" >&2
          exit 1
        fi
        export HELIOS_EXECUTION_RPC_URL="''${HELIOS_EXECUTION_RPC_URL:-https://eth.drpc.org}"

        if [ "''${MFM_KEEP_SERVICES+x}" = "x" ]; then
          echo "ERROR: MFM_KEEP_SERVICES has been removed from mfm::portfolio::snapshot" >&2
          echo "Use process-first controls instead:" >&2
          echo "  SERVICE_REUSE_POLICY=never|same-root|same-slot|cross-run" >&2
          echo "  SERVICE_OWNER_SCOPE=ephemeral|persistent" >&2
          echo "  SERVICE_DISCOVERY_SCOPE=local|global" >&2
          echo "Or start services explicitly before running the snapshot command." >&2
          exit 1
        fi

        snapshot_keep_running_from_policy() {
          local owner_scope="''${SERVICE_OWNER_SCOPE:-}"
          local reuse_policy="''${SERVICE_REUSE_POLICY:-}"
          local discovery_scope="''${SERVICE_DISCOVERY_SCOPE:-}"

          case "$owner_scope" in
            persistent) echo "1"; return 0 ;;
            ephemeral) echo "0"; return 0 ;;
            "") ;;
            *)
              echo "ERROR: SERVICE_OWNER_SCOPE must be ephemeral|persistent (got '$owner_scope')" >&2
              return 1
              ;;
          esac

          case "$reuse_policy" in
            same-slot|cross-run) echo "1"; return 0 ;;
            never|same-root) echo "0"; return 0 ;;
            "") ;;
            *)
              echo "ERROR: SERVICE_REUSE_POLICY must be one of never|same-root|same-slot|cross-run (got '$reuse_policy')" >&2
              return 1
              ;;
          esac

          case "$discovery_scope" in
            global) echo "1"; return 0 ;;
            local) echo "0"; return 0 ;;
            "") ;;
            *)
              echo "ERROR: SERVICE_DISCOVERY_SCOPE must be local|global (got '$discovery_scope')" >&2
              return 1
              ;;
          esac

          echo "0"
          return 0
        }

        SNAPSHOT_KEEP_RUNNING="$(snapshot_keep_running_from_policy)"

        wait_helios_rpc_ready() {
          local timeout_secs="''${HELIOS_READY_TIMEOUT_SECS:-300}"
          local interval_secs="''${HELIOS_READY_INTERVAL_SECS:-1}"
          local start_ts
          local now_ts
          local attempt
          local resp
          local err_msg

          case "$timeout_secs" in
            *[!0-9]*|"")
              echo "ERROR: HELIOS_READY_TIMEOUT_SECS must be an integer seconds value (got '$timeout_secs')" >&2
              return 1
              ;;
          esac

          case "$interval_secs" in
            *[!0-9.]*|""|*.*.*|.*|*.)
              echo "ERROR: HELIOS_READY_INTERVAL_SECS must be a positive number (got '$interval_secs')" >&2
              return 1
              ;;
          esac

          start_ts=$(date +%s)
          attempt=0

          while true; do
            attempt=$((attempt + 1))
            resp="$(curl -fsS --max-time 2 \
              -H 'content-type: application/json' \
              --data '{"jsonrpc":"2.0","id":1,"method":"eth_blockNumber","params":[]}' \
              "http://127.0.0.1:$HELIOSRPC_PORT" 2>/dev/null || true)"

            if [ -n "$resp" ] && echo "$resp" | jq -e '.result | strings' >/dev/null 2>&1; then
              echo "OK: helios ready rpc_port=$HELIOSRPC_PORT mode=rpc_probe"
              return 0
            fi

            if [ $((attempt % 10)) -eq 0 ]; then
              err_msg=""
              if [ -n "$resp" ]; then
                err_msg="$(echo "$resp" | jq -r '.error.message // empty' 2>/dev/null || true)"
              fi
              if [ -n "$err_msg" ]; then
                echo "INFO: helios not ready yet: $err_msg" >&2
              fi
            fi

            now_ts=$(date +%s)
            if [ $((now_ts - start_ts)) -ge "$timeout_secs" ]; then
              echo "ERROR: helios not ready after $timeout_secs s (eth_blockNumber still failing) rpc_port=$HELIOSRPC_PORT" >&2
              echo "HINT: set HELIOS_CHECKPOINT and HELIOS_CONSENSUS_RPC_URL explicitly for mainnet." >&2
              return 1
            fi

            sleep "$interval_secs"
          done
        }

        # Start Postgres (required by portfolio snapshot). Prefer reusing a healthy
        # instance; if status metadata is stale and health fails, start it.
        if run_hook POSTGRES_HEALTH >/dev/null 2>&1; then
          run_hook POSTGRES_SETUP_DB >/dev/null 2>&1 || true
        else
          PG_LOGFILE="$(artifact_path "postgres-mfm-portfolio-snapshot.log")"
          fixture_start_service postgres dev 60 1 "$PG_LOGFILE" "$SNAPSHOT_KEEP_RUNNING"
        fi
        run_hook POSTGRES_READY

        # Start Helios (mainnet-backed). Prefer reusing a healthy instance; if
        # status metadata is stale and health fails, start it.
        if run_hook HELIOS_HEALTH >/dev/null 2>&1; then
          true
        else
          HELIOS_LOGFILE="$(artifact_path "helios-mfm-portfolio-snapshot.log")"
          fixture_start_service helios dev 300 1 "$HELIOS_LOGFILE" "$SNAPSHOT_KEEP_RUNNING"
        fi

        export DATABASE_URL="postgresql://postgres:postgres@127.0.0.1:$POSTGRES_PORT/mfm"
        export MFM_EVM_RPC_URL="http://127.0.0.1:$HELIOSRPC_PORT"

        echo "INFO: waiting for Helios readiness (eth_blockNumber)..." >&2
        HELIOS_STATUS_LINE="$(run_hook HELIOS_STATUS 2>/dev/null || true)"
        HELIOS_SCOPE="$(printf '%s\n' "$HELIOS_STATUS_LINE" | tr ' ' '\n' | awk -F= '$1=="scope" { print $2; exit }')"
        HELIOS_RUNNING="$(printf '%s\n' "$HELIOS_STATUS_LINE" | tr ' ' '\n' | awk -F= '$1=="running" { print $2; exit }')"

        if [ "$HELIOS_RUNNING" = "true" ] && [ "$HELIOS_SCOPE" = "global" ]; then
          echo "INFO: helios reuse detected scope=global; using rpc-only readiness probe" >&2
          wait_helios_rpc_ready
        else
          run_hook HELIOS_READY
        fi

        OUT_RAW_FILE="$(artifact_path "mfm-portfolio-snapshot.raw.out")"
        OUT_FILE="$(artifact_path "mfm-portfolio-snapshot.json")"
        cargo run -q -p mfm --bin mfm_cli -- \
          --output-format json \
          portfolio snapshot "$ADDRESS" \
          --chain-id 1 \
          >"$OUT_RAW_FILE"

        sanitize_json_payload "$OUT_RAW_FILE" "$OUT_FILE" "snapshot output"

        if ! jq -e '.status == "success"' "$OUT_FILE" >/dev/null; then
          echo "ERROR: snapshot command did not produce a success payload" >&2
          cat "$OUT_FILE" >&2 || true
          exit 1
        fi

        ART_ID="$(jq -r '.data.result.snapshot_artifact_id // empty' "$OUT_FILE")"
        if [ -z "$ART_ID" ] || [ "$ART_ID" = "null" ]; then
          echo "ERROR: missing snapshot_artifact_id in snapshot response" >&2
          cat "$OUT_FILE" >&2 || true
          exit 1
        fi

        ART_RAW_FILE="$(artifact_path "mfm-portfolio-snapshot-artifact.raw.out")"
        ART_FILE="$(artifact_path "mfm-portfolio-snapshot-artifact.json")"
        cargo run -q -p mfm --bin mfm_cli -- \
          --output-format json \
          run artifacts get "$ART_ID" \
          >"$ART_RAW_FILE"

        sanitize_json_payload "$ART_RAW_FILE" "$ART_FILE" "artifact output"

        if ! jq -e '.status == "success"' "$ART_FILE" >/dev/null; then
          echo "ERROR: failed to fetch snapshot artifact id=$ART_ID" >&2
          cat "$ART_FILE" >&2 || true
          exit 1
        fi

        if ! jq -e '.data.encoding == "json" and (.data.value | type == "object")' "$ART_FILE" >/dev/null; then
          echo "ERROR: snapshot artifact is not JSON id=$ART_ID" >&2
          cat "$ART_FILE" >&2 || true
          exit 1
        fi

        REPORT_FILE="$(artifact_path "mfm-portfolio-snapshot-report.json")"
        REPORT_TS="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
        jq -s \
          --arg address "$ADDRESS" \
          --arg generated_at "$REPORT_TS" \
          '
          .[0] as $base
          | .[1] as $artifact
          | .[1].data.value as $snapshot
          | $base
          | .data.result.snapshot = $snapshot
          | .data.report = {
              phase: "completed",
              generated_at: $generated_at,
              address: $address,
              snapshot_artifact_id: ($base.data.result.snapshot_artifact_id // null),
              artifact_encoding: ($artifact.data.encoding // null),
              sources: {
                snapshot_response: $base,
                artifact_response: $artifact
              }
            }
          ' "$OUT_FILE" "$ART_FILE" >"$REPORT_FILE"

        if ! jq -e '.data.report.phase == "completed" and (.data.result.snapshot | type == "object")' "$REPORT_FILE" >/dev/null; then
          echo "ERROR: report phase did not produce expected output" >&2
          cat "$REPORT_FILE" >&2 || true
          exit 1
        fi

        cat "$REPORT_FILE" >&3
      '';
    };

    mfm_rest_api = {
      description = "Run mfm_rest_api (Cargo run)";
      api = lib.appApi.mkTypedCommandApi {
        name = "mfm_rest_api";
        summary = "Run mfm_rest_api";
        details = "Runs the REST API server via `cargo run` inside the pinned Nix environment.";
        usage = [ "nix run .#mfm_rest_api" ];
        category = "core";
        failureCodes = failureCodesCargo;
      };
      env = { };
      useDeps = true;
      script = ''
        exec cargo run -p mfm-rest-api --bin mfm_rest_api -- "$@"
      '';
    };
  };
}
