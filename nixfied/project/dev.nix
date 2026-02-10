{ project, ... }:

{
  commands = {
    dev = {
      description = "Start the dev workflow";
      api = {
        version = 1;
        summary = "Start the dev workflow (Postgres + REST API)";
        details = "Starts Postgres, MinIO (S3), and the REST API, then waits (local development).";
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
      script = ''
        eval "$($SLOT_INFO)"

        run_hook POSTGRES_FULL_START

        export DATABASE_URL="postgresql://postgres:postgres@127.0.0.1:$POSTGRES_PORT/mfm"
        export MFM_REST_API_ADDR="127.0.0.1:$REST_API_PORT"

        # Dev/test should not depend on the supervisor (production-only).
        # Start MinIO directly for local development.
        mkdir -p "$MINIO_STATE_DIR" "$LOG_DIR"
        export MINIO_ROOT_USER="minio"
        export MINIO_ROOT_PASSWORD="minio123456"
        start_service minio \
          --log "$LOG_DIR/minio.log" \
          --wait-http "http://127.0.0.1:$MINIO_PORT/minio/health/ready" \
          --timeout 60 \
          -- \
          minio server "$MINIO_STATE_DIR" \
            --address "127.0.0.1:$MINIO_PORT" \
            --console-address "127.0.0.1:$MINIO_CONSOLE_PORT"

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
  };
}
