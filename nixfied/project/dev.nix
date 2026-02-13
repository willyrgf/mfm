{ project, ... }:

{
  commands = {
    dev = {
      description = "Start the dev workflow";
      api = {
        version = 1;
        summary = "Start the dev workflow (Postgres + MinIO + Reth + REST API)";
        details = "Starts Postgres, MinIO (S3), Reth, and the REST API, then waits (local development).";
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
          --wait-http "http://127.0.0.1:$REST_API_PORT/v1/health" \
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

          Set `MFM_KEEP_SERVICES=1` to keep Helios/Postgres running after the command exits.
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
            name = "MFM_KEEP_SERVICES";
            description = "Set to 1/true/yes to keep Helios + Postgres running after this app exits (default: 0).";
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

        KEEP_SERVICES_RAW="''${MFM_KEEP_SERVICES:-0}"
        case "$KEEP_SERVICES_RAW" in
          1|true|TRUE|yes|YES)
            KEEP_SERVICES=1
            ;;
          0|false|FALSE|no|NO|"")
            KEEP_SERVICES=0
            ;;
          *)
            echo "ERROR: MFM_KEEP_SERVICES must be 0/1/true/false (got '$KEEP_SERVICES_RAW')" >&2
            exit 1
            ;;
        esac

        STARTED_POSTGRES=0
        STARTED_HELIOS=0

        # Start Postgres (required by portfolio snapshot). If already running, don't stop it.
        if run_hook POSTGRES_STATUS >/dev/null 2>&1; then
          run_hook POSTGRES_SETUP_DB >/dev/null 2>&1 || true
        else
          STARTED_POSTGRES=1
          PG_LOGFILE="$(artifact_path "postgres-mfm-portfolio-snapshot.log")"
          fixture_start_service postgres dev 60 1 "$PG_LOGFILE" "$KEEP_SERVICES"
        fi

        # Start Helios (mainnet-backed). If already running, don't stop it.
        if run_hook HELIOS_STATUS >/dev/null 2>&1; then
          run_hook HELIOS_HEALTH >/dev/null 2>&1
        else
          STARTED_HELIOS=1
          HELIOS_LOGFILE="$(artifact_path "helios-mfm-portfolio-snapshot.log")"
          fixture_start_service helios dev 300 1 "$HELIOS_LOGFILE" "$KEEP_SERVICES"
        fi

        export DATABASE_URL="postgresql://postgres:postgres@127.0.0.1:$POSTGRES_PORT/mfm"
        export MFM_EVM_RPC_URL="http://127.0.0.1:$HELIOSRPC_PORT"

        echo "INFO: waiting for Helios readiness (eth_blockNumber)..." >&2
        run_hook HELIOS_READY

        OUT_FILE="$(artifact_path "mfm-portfolio-snapshot.json")"
        cargo run -q -p mfm --bin mfm_cli -- \
          --output-format json \
          portfolio snapshot "$ADDRESS" \
          --chain-id 1 \
          >"$OUT_FILE"

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

        ART_FILE="$(artifact_path "mfm-portfolio-snapshot-artifact.json")"
        cargo run -q -p mfm --bin mfm_cli -- \
          --output-format json \
          run artifacts get "$ART_ID" \
          >"$ART_FILE"

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

        OUT_WITH_SNAPSHOT_FILE="$(artifact_path "mfm-portfolio-snapshot-with-content.json")"
        jq -s '.[0] as $base | .[1].data.value as $snapshot | $base | .data.result.snapshot = $snapshot' "$OUT_FILE" "$ART_FILE" >"$OUT_WITH_SNAPSHOT_FILE"

        if [ "$KEEP_SERVICES" = "1" ] && { [ "$STARTED_POSTGRES" = "1" ] || [ "$STARTED_HELIOS" = "1" ]; }; then
          echo "INFO: MFM_KEEP_SERVICES=1; leaving started services running for reuse." >&2
          if [ "$STARTED_HELIOS" = "1" ]; then
            echo "INFO: stop Helios with: MFM_ENV=$ENV NIX_ENV=$SLOT nix run .#service::helios::stop" >&2
          fi
          if [ "$STARTED_POSTGRES" = "1" ]; then
            echo "INFO: stop Postgres with: MFM_ENV=$ENV NIX_ENV=$SLOT nix run .#service::postgres::stop" >&2
          fi
        fi

        cat "$OUT_WITH_SNAPSHOT_FILE" >&3
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
