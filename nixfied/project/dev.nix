{ project, ... }:

{
  commands = {
    dev = {
      description = "Start the dev workflow";
      env = {
        "${project.envVar}" = "dev";
      };
      useDeps = true;
      script = ''
        eval "$($SLOT_INFO)"

        run_hook POSTGRES_FULL_START

        export DATABASE_URL="postgresql://postgres:postgres@127.0.0.1:$POSTGRES_PORT/mfm"
        export MFM_REST_API_ADDR="127.0.0.1:$REST_API_PORT"

        start_service rest-api \
          --wait-http "http://127.0.0.1:$REST_API_PORT/v1/health" \
          -- \
          cargo run -p mfm-rest-api --bin mfm_rest_api

        wait
      '';
    };

    mfm_cli = {
      description = "Run mfm_cli (Cargo run)";
      env = { };
      useDeps = true;
      script = ''
        exec cargo run -p mfm --bin mfm_cli -- "$@"
      '';
    };

    mfm_rest_api = {
      description = "Run mfm_rest_api (Cargo run)";
      env = { };
      useDeps = true;
      script = ''
        exec cargo run -p mfm-rest-api --bin mfm_rest_api -- "$@"
      '';
    };
  };
}
