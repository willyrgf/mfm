{ project, ... }:

{
  commands = {
    dev = {
      description = "Start the dev workflow";
      api = {
        version = 1;
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
      api = {
        version = 1;
        summary = "Run mfm_cli";
        details = "Runs the CLI binary via `cargo run` inside the pinned Nix environment.";
        usage = [
          "nix run .#mfm_cli -- --help"
          "nix run .#mfm_cli -- <args>"
        ];
        category = "core";
      };
      env = { };
      useDeps = true;
      script = ''
        exec cargo run -p mfm --bin mfm_cli -- "$@"
      '';
    };

    "mfm::portfolio::snapshot" = {
      description = "Snapshot a wallet via Helios-backed mainnet RPC (starts Postgres + Helios)";
      api = {
        version = 1;
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
          To keep services running for reuse, start them explicitly:
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
            name = "<ADDRESS>";
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

        # Start Postgres (required by portfolio snapshot). Prefer reusing a healthy
        # instance; if status metadata is stale and health fails, start it.
        if run_hook POSTGRES_HEALTH >/dev/null 2>&1; then
          run_hook POSTGRES_SETUP_DB >/dev/null 2>&1 || true
        else
          PG_LOGFILE="$(artifact_path "postgres-mfm-portfolio-snapshot.log")"
          fixture_start_service postgres dev 60 1 "$PG_LOGFILE"
        fi
        run_hook POSTGRES_READY

        # Start Helios (mainnet-backed). Prefer reusing a healthy instance; if
        # status metadata is stale and health fails, start it.
        if run_hook HELIOS_HEALTH >/dev/null 2>&1; then
          true
        else
          HELIOS_LOGFILE="$(artifact_path "helios-mfm-portfolio-snapshot.log")"
          fixture_start_service helios dev 300 1 "$HELIOS_LOGFILE"
        fi

        export DATABASE_URL="postgresql://postgres:postgres@127.0.0.1:$POSTGRES_PORT/mfm"
        export MFM_EVM_RPC_URL="http://127.0.0.1:$HELIOSRPC_PORT"

        echo "INFO: waiting for Helios readiness (eth_blockNumber)..." >&2
        run_hook HELIOS_READY

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
      api = {
        version = 1;
        summary = "Run mfm_rest_api";
        details = "Runs the REST API server via `cargo run` inside the pinned Nix environment.";
        usage = [
          "nix run .#mfm_rest_api"
          "nix run .#mfm_rest_api -- <args>"
        ];
        category = "core";
      };
      env = { };
      useDeps = true;
      script = ''
        exec cargo run -p mfm-rest-api --bin mfm_rest_api -- "$@"
      '';
    };
  };
}
