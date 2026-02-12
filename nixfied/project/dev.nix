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
          `HELIOS_CONSENSUS_RPC_URL=...`.
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
            description = "Upstream execution JSON-RPC URL for Helios (HTTP/WSS). Loaded from `.env` if present.";
          }
          {
            name = "HELIOS_CONSENSUS_RPC_URL";
            description = "Upstream consensus endpoint for Helios (required when HELIOS_NETWORK != local). Loaded from `.env` if present.";
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
        if [ -z "''${HELIOS_EXECUTION_RPC_URL:-}" ]; then
          echo "ERROR: HELIOS_EXECUTION_RPC_URL is required (set it in .env)" >&2
          exit 1
        fi
        if [ -z "''${HELIOS_CONSENSUS_RPC_URL:-}" ]; then
          echo "ERROR: HELIOS_CONSENSUS_RPC_URL is required for mainnet (set it in .env)" >&2
          exit 1
        fi

        # Start Postgres (required by portfolio snapshot). If already running, don't stop it.
        if run_hook POSTGRES_STATUS >/dev/null 2>&1; then
          run_hook POSTGRES_SETUP_DB >/dev/null 2>&1 || true
        else
          PG_LOGFILE="$(artifact_path "postgres-mfm-portfolio-snapshot.log")"
          fixture_start_service postgres dev 60 1 "$PG_LOGFILE"
        fi

        # Start Helios (mainnet-backed). If already running, don't stop it.
        if run_hook HELIOS_STATUS >/dev/null 2>&1; then
          run_hook HELIOS_HEALTH >/dev/null 2>&1
        else
          HELIOS_LOGFILE="$(artifact_path "helios-mfm-portfolio-snapshot.log")"
          fixture_start_service helios dev 300 1 "$HELIOS_LOGFILE"
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

        cat "$OUT_FILE" >&3
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
