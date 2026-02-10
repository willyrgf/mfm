{ project, ... }:

{
  commands = {
    ci = {
      description = "Run the CI pipeline";
      api = {
        version = 1;
        summary = "Run the CI pipeline";
        details = ''
          Runs the CI pipeline defined in `nixfied/project/ci.nix` (modes + steps).

          Supported options (from the framework CI runner):
          - `--basic|--audit|--parity`: select a mode
          - `--mode <name>`: select a mode by name
          - `--summary`: print a compact summary and write `summary.json` into the artifacts dir
          - `--bg`: run in background via the run registry
        '';
        usage = [
          "nix run .#ci -- --basic --summary"
          "nix run .#ci -- --audit --summary"
          "nix run .#ci -- --parity --summary"
          "nix run .#ci -- --mode basic --summary"
          "nix run .#ci -- --bg"
        ];
        examples = [
          "nix run .#ci -- --basic --summary"
          "CI_ARTIFACTS_DIR=/tmp/ci-artifacts nix run .#ci -- --parity --summary"
        ];
        args = [
          {
            name = "--basic|--audit|--parity";
            description = "Select CI mode (configured in nixfied/project/ci.nix).";
          }
          {
            name = "--mode <name>";
            description = "Select CI mode by name.";
          }
          {
            name = "--summary";
            description = "Print a compact summary and write artifacts/summary.json.";
          }
          {
            name = "--bg";
            description = "Run CI in background via the run registry.";
          }
        ];
        env = [
          {
            name = "CI_ARTIFACTS_DIR";
            description = "Override artifacts directory (default: /tmp/ci-artifacts).";
          }
        ];
        category = "core";
      };
      env = {
        "${project.envVar}" = "test";
      };
      useDeps = true;
      # Note: `nix run .#ci` is implemented by the framework CI runner (nixfied/ci.nix),
      # which reads `project.ci.*` below. This command exists primarily for `nix run .#help`.
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
    };
    useDeps = true;
    setup = "";
    teardown = "";
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
        ];
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
        requires = [ "postgres" ];
        run = ''
          eval "$($SLOT_INFO)"
          # parity-postgres starts a real daemon; ensure we stop it so slot reuse is re-entrant.
          with_cleanup "PGDATA=\"$POSTGRES_DIR\" run_hook POSTGRES_STOP"
          run_hook POSTGRES_FULL_START_TEST

          export DATABASE_URL="postgresql://postgres:postgres@127.0.0.1:$POSTGRES_PORT/mfm_test"

          LOGFILE=$(artifact_path "parity-postgres.log")
          log_capture "$LOGFILE" -- cargo nextest run -p mfm-event-store-postgres --features parity-tests
        '';
      };

      parity-s3 = {
        description = "Parity: S3/MinIO artifact store";
        run = ''
          eval "$($SLOT_INFO)"

          # Dev/test parity should not depend on the supervisor (production-only).
          # Start MinIO directly (no process-compose/TUI).
          mkdir -p "$MINIO_STATE_DIR"
          export MINIO_ROOT_USER="minio"
          export MINIO_ROOT_PASSWORD="minio123456"

          MINIO_LOGFILE=$(artifact_path "minio.log")
          MINIO_PID=$(start_service minio --log "$MINIO_LOGFILE" -- \
            minio server "$MINIO_STATE_DIR" \
              --address "127.0.0.1:$MINIO_PORT" \
              --console-address "127.0.0.1:$MINIO_CONSOLE_PORT")
          with_cleanup "stop_service $MINIO_PID minio"

          export AWS_ACCESS_KEY_ID="minio"
          export AWS_SECRET_ACCESS_KEY="minio123456"
          export AWS_EC2_METADATA_DISABLED="true"

          export MFM_S3_ENDPOINT="http://127.0.0.1:$MINIO_PORT"
          export MFM_S3_REGION="us-east-1"
          export MFM_S3_BUCKET="mfm-test"
          export MFM_S3_PREFIX="mfm-artifacts"

          wait_http "$MFM_S3_ENDPOINT/minio/health/ready" 60 1

          LOGFILE=$(artifact_path "parity-s3.log")
          log_capture "$LOGFILE" -- cargo nextest run -p mfm-artifact-store-s3 --features parity-tests
        '';
      };
    };
  };
}
