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
          "parity-evm-reth"
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
          with_cleanup "run_hook POSTGRES_STOP"
          # In local CI runs, recover from stale/foreign listeners on the slot test port.
          AUTO_STOP_CONFLICTING=1 run_hook POSTGRES_FULL_START_TEST

          export DATABASE_URL="postgresql://postgres:postgres@127.0.0.1:$POSTGRES_PORT/mfm_test"

          LOGFILE=$(artifact_path "parity-postgres.log")
          log_capture "$LOGFILE" -- cargo nextest run -p mfm-event-store-postgres --features parity-tests
        '';
      };

      parity-s3 = {
        description = "Parity: S3/MinIO artifact store";
        requires = [ "minio" ];
        run = ''
          eval "$($SLOT_INFO)"

          # Use the MinIO service module contract in CI parity (without supervisor).
          run_hook MINIO_INIT
          MINIO_LOGFILE=$(artifact_path "minio.log")
          # Hook and service app launch the same contract-generated operation script.
          MINIO_PID=$(start_service minio \
            --log "$MINIO_LOGFILE" \
            --wait-http "http://127.0.0.1:$MINIO_PORT/minio/health/ready" \
            --timeout 60 \
            -- \
            run_hook MINIO_START)
          with_cleanup "stop_service $MINIO_PID minio"

          export AWS_ACCESS_KEY_ID="minio"
          export AWS_SECRET_ACCESS_KEY="minio123456"
          export AWS_EC2_METADATA_DISABLED="true"

          export MFM_S3_ENDPOINT="http://127.0.0.1:$MINIO_PORT"
          export MFM_S3_REGION="us-east-1"
          export MFM_S3_BUCKET="mfm-test"
          export MFM_S3_PREFIX="mfm-artifacts"

          run_hook MINIO_BUCKET_CREATE "$MFM_S3_BUCKET"

          LOGFILE=$(artifact_path "parity-s3.log")
          log_capture "$LOGFILE" -- cargo nextest run -p mfm-artifact-store-s3 --features parity-tests
        '';
      };

      parity-evm-reth = {
        description = "Parity: EVM pipeline deploy/configure/validate on reth";
        requires = [
          "postgres"
          "minio"
        ];
        run = ''
          eval "$($SLOT_INFO)"

          # PostgreSQL parity store setup.
          with_cleanup "run_hook POSTGRES_STOP"
          AUTO_STOP_CONFLICTING=1 run_hook POSTGRES_FULL_START_TEST
          export DATABASE_URL="postgresql://postgres:postgres@127.0.0.1:$POSTGRES_PORT/mfm_test"

          # MinIO parity store setup.
          run_hook MINIO_INIT
          MINIO_LOGFILE=$(artifact_path "minio-evm.log")
          MINIO_PID=$(start_service minio \
            --log "$MINIO_LOGFILE" \
            --wait-http "http://127.0.0.1:$MINIO_PORT/minio/health/ready" \
            --timeout 60 \
            -- \
            run_hook MINIO_START)
          with_cleanup "stop_service $MINIO_PID minio"

          export AWS_ACCESS_KEY_ID="minio"
          export AWS_SECRET_ACCESS_KEY="minio123456"
          export AWS_EC2_METADATA_DISABLED="true"
          export MFM_S3_ENDPOINT="http://127.0.0.1:$MINIO_PORT"
          export MFM_S3_REGION="us-east-1"
          export MFM_S3_BUCKET="mfm-test"
          export MFM_S3_PREFIX="mfm-artifacts"
          run_hook MINIO_BUCKET_CREATE "$MFM_S3_BUCKET"

          # Start a real local reth dev node for deploy/configure/validate flow.
          RETH_LOGFILE=$(artifact_path "reth.log")
          RETH_DATADIR=$(artifact_path "reth-datadir")
          rm -rf "$RETH_DATADIR"
          RETH_PID=$(start_service reth \
            --log "$RETH_LOGFILE" \
            --wait-port "$RETH_RPC_PORT" \
            --timeout 60 \
            -- \
            reth node --dev --http --http.addr 127.0.0.1 --http.port "$RETH_RPC_PORT" --datadir "$RETH_DATADIR")
          with_cleanup "stop_service $RETH_PID reth"

          # Wait until JSON-RPC responds.
          for i in $(seq 1 60); do
            if curl -fsS \
              -H 'content-type: application/json' \
              --data '{"jsonrpc":"2.0","id":1,"method":"web3_clientVersion","params":[]}' \
              "http://127.0.0.1:$RETH_RPC_PORT" >/dev/null; then
              break
            fi
            if [ "$i" -eq 60 ]; then
              echo "reth JSON-RPC did not become ready in time" >&2
              exit 1
            fi
            sleep 1
          done

          export MFM_EVM_RPC_URL="http://127.0.0.1:$RETH_RPC_PORT"

          LOGFILE=$(artifact_path "parity-evm-reth.log")
          log_capture "$LOGFILE" -- cargo nextest run -p mfm-rest-api --features parity-tests --test parity_evm_reth_pipeline
        '';
      };
    };
  };
}
