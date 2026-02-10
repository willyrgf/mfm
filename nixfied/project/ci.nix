{ project, ... }:

{
  commands = {
    ci = {
      description = "Run the CI pipeline";
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

          run_hook SUPERVISOR_START_DAEMON
          with_cleanup "run_hook SUPERVISOR_STOP"

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
