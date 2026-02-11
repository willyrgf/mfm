{ project, ... }:

{
  commands = {
    dev = {
      description = "Start the dev workflow";
      api = {
        version = 1;
        summary = "Start the dev workflow (Postgres + reth + REST API)";
        details = "Starts Postgres, local reth JSON-RPC, MinIO (S3), and the REST API, then waits (local development).";
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
        "${project.slotVar}" = "0";
      };
      useDeps = true;
      script = ''
        eval "$($SLOT_INFO)"

        run_hook POSTGRES_FULL_START

        export DATABASE_URL="postgresql://postgres:postgres@127.0.0.1:$POSTGRES_PORT/mfm"
        export MFM_REST_API_ADDR="127.0.0.1:$REST_API_PORT"
        export MFM_EVM_RPC_URL="http://127.0.0.1:$RETH_RPC_PORT"

        if ! command -v reth >/dev/null 2>&1; then
          echo "ERROR: reth binary not found in PATH. Install reth in the dev environment first." >&2
          exit 1
        fi

        RETH_STATE_DIR="''${XDG_DATA_HOME:-$HOME/.local/share}/mfm/reth-$SLOT-$ENV"
        mkdir -p "$RETH_STATE_DIR" "$LOG_DIR"
        start_service reth \
          --log "$LOG_DIR/reth.log" \
          --wait-port "$RETH_RPC_PORT" \
          --timeout 60 \
          -- \
          reth node \
            --dev \
            --datadir "$RETH_STATE_DIR" \
            --http \
            --http.addr "127.0.0.1" \
            --http.port "$RETH_RPC_PORT"

        # Use the MinIO service module contract in dev/test (without supervisor).
        run_hook MINIO_INIT
        start_service minio \
          --wait-http "http://127.0.0.1:$MINIO_PORT/minio/health/ready" \
          --timeout 60 \
          -- \
          "$MINIO_START"

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

    contracts_build = {
      description = "Build Solidity contracts via Foundry";
      api = {
        version = 1;
        summary = "Build contracts (forge build)";
        details = "Compiles contracts using Foundry (`forge build`) and writes artifacts to `contracts/out`.";
        usage = [
          "nix run .#contracts_build"
          "nix run .#contracts_build -- --sizes"
        ];
        category = "core";
      };
      env = { };
      useDeps = true;
      script = ''
        if ! command -v forge >/dev/null 2>&1; then
          echo "ERROR: forge binary not found in PATH." >&2
          exit 1
        fi
        if command -v solc >/dev/null 2>&1; then
          export FOUNDRY_SOLC="$(command -v solc)"
        fi
        exec forge build --offline "$@"
      '';
    };
  };
}
