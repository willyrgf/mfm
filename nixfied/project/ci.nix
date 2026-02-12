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
          "parity-rest-api-smoke"
          "parity-evm-reth"
          "parity-portfolio-tracker-reth"
          "parity-evm-helios-smoke"
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
        env = {
          AUTO_STOP_CONFLICTING = "1";
        };
        fixtures = {
          services = [
            {
              name = "postgres";
              profile = "test";
            }
          ];
        };
        run = ''
          eval "$($SLOT_INFO)"

          export DATABASE_URL="postgresql://postgres:postgres@127.0.0.1:$POSTGRES_PORT/mfm_test"

          LOGFILE=$(artifact_path "parity-postgres.log")
          log_capture "$LOGFILE" -- cargo nextest run -p mfm-integration-tests --features parity-tests --test parity_event_store_postgres_contract
        '';
      };

      parity-s3 = {
        description = "Parity: S3/MinIO artifact store";
        fixtures = {
          services = [
            {
              name = "minio";
              profile = "test";
              exports = [ "s3" ];
              bucket = "mfm-test";
              prefix = "mfm-artifacts";
              region = "us-east-1";
              bootstrap = [ "mfm-test" ];
              logName = "minio.log";
            }
          ];
        };
        run = ''
          export MFM_S3_ENDPOINT="$MINIO_ENDPOINT"
          export MFM_S3_REGION="$MINIO_REGION"
          export MFM_S3_BUCKET="$MINIO_BUCKET"
          export MFM_S3_PREFIX="$MINIO_PREFIX"

          LOGFILE=$(artifact_path "parity-s3.log")
          log_capture "$LOGFILE" -- cargo nextest run -p mfm-integration-tests --features parity-tests --test parity_artifact_store_s3_contract
        '';
      };

      parity-rest-api-smoke = {
        description = "Parity: REST API smoke on Postgres + S3/MinIO";
        env = {
          AUTO_STOP_CONFLICTING = "1";
        };
        fixtures = {
          services = [
            {
              name = "postgres";
              profile = "test";
            }
            {
              name = "minio";
              profile = "test";
              exports = [ "s3" ];
              bucket = "mfm-test";
              prefix = "mfm-artifacts";
              region = "us-east-1";
              bootstrap = [ "mfm-test" ];
              logName = "minio-rest-api-smoke.log";
            }
          ];
        };
        run = ''
          eval "$($SLOT_INFO)"

          export DATABASE_URL="postgresql://postgres:postgres@127.0.0.1:$POSTGRES_PORT/mfm_test"

          export MFM_S3_ENDPOINT="$MINIO_ENDPOINT"
          export MFM_S3_REGION="$MINIO_REGION"
          export MFM_S3_BUCKET="$MINIO_BUCKET"
          export MFM_S3_PREFIX="$MINIO_PREFIX"

          LOGFILE=$(artifact_path "parity-rest-api-smoke.log")
          log_capture "$LOGFILE" -- cargo nextest run -p mfm-integration-tests --features parity-tests --test parity_rest_api_postgres_s3_smoke
        '';
      };

      parity-evm-reth = {
        description = "Parity: EVM pipeline deploy/configure/validate on reth";
        env = {
          AUTO_STOP_CONFLICTING = "1";
        };
        fixtures = {
          services = [
            {
              name = "postgres";
              profile = "test";
            }
            {
              name = "minio";
              profile = "test";
              exports = [ "s3" ];
              bucket = "mfm-test";
              prefix = "mfm-artifacts";
              region = "us-east-1";
              bootstrap = [ "mfm-test" ];
              logName = "minio-evm.log";
            }
            {
              name = "reth";
              profile = "test";
              logName = "reth.log";
            }
          ];
        };
        run = ''
          eval "$($SLOT_INFO)"

          export DATABASE_URL="postgresql://postgres:postgres@127.0.0.1:$POSTGRES_PORT/mfm_test"

          export MFM_S3_ENDPOINT="$MINIO_ENDPOINT"
          export MFM_S3_REGION="$MINIO_REGION"
          export MFM_S3_BUCKET="$MINIO_BUCKET"
          export MFM_S3_PREFIX="$MINIO_PREFIX"

          export MFM_EVM_RPC_URL="http://127.0.0.1:$RETHHTTP_PORT"

          LOGFILE=$(artifact_path "parity-evm-reth.log")
          log_capture "$LOGFILE" -- cargo nextest run -p mfm-integration-tests --features parity-tests --test parity_rest_api_evm_reth_pipeline
        '';
      };

      parity-portfolio-tracker-reth = {
        description = "Parity: portfolio_tracker snapshot against reth (MockERC20)";
        env = {
          AUTO_STOP_CONFLICTING = "1";
        };
        fixtures = {
          services = [
            {
              name = "postgres";
              profile = "test";
            }
            {
              name = "minio";
              profile = "test";
              exports = [ "s3" ];
              bucket = "mfm-test";
              prefix = "mfm-artifacts";
              region = "us-east-1";
              bootstrap = [ "mfm-test" ];
              logName = "minio-portfolio-tracker.log";
            }
            {
              name = "reth";
              profile = "test";
              logName = "reth-portfolio-tracker.log";
            }
          ];
        };
        run = ''
          eval "$($SLOT_INFO)"

          export DATABASE_URL="postgresql://postgres:postgres@127.0.0.1:$POSTGRES_PORT/mfm_test"

          export MFM_S3_ENDPOINT="$MINIO_ENDPOINT"
          export MFM_S3_REGION="$MINIO_REGION"
          export MFM_S3_BUCKET="$MINIO_BUCKET"
          export MFM_S3_PREFIX="$MINIO_PREFIX"

          export MFM_EVM_RPC_URL="http://127.0.0.1:$RETHHTTP_PORT"

          LOGFILE=$(artifact_path "parity-portfolio-tracker-reth.log")
          log_capture "$LOGFILE" -- cargo nextest run -p mfm-integration-tests --features parity-tests --test parity_portfolio_tracker_reth_mock_erc20
        '';
      };

      parity-evm-helios-smoke = {
        description = "Parity: Helios lifecycle and RPC smoke over reth execution";
        fixtures = {
          services = [
            {
              name = "reth";
              profile = "test";
              logName = "reth-helios.log";
            }
            {
              name = "helios";
              profile = "test";
              logName = "helios.log";
            }
          ];
        };
        run = ''
          eval "$($SLOT_INFO)"

          LOGFILE=$(artifact_path "parity-evm-helios-smoke.log")
          curl -fsS \
            -H 'content-type: application/json' \
            --data '{"jsonrpc":"2.0","id":1,"method":"eth_chainId","params":[]}' \
            "http://127.0.0.1:$HELIOSRPC_PORT" \
            | tee "$LOGFILE" \
            | jq -e '.result | strings' >/dev/null
        '';
      };
    };
  };
}
