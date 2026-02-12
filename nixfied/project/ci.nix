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
      mainnet = {
        steps = [ "mainnet-portfolio-snapshot-helios" ];
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

      mainnet-portfolio-snapshot-helios = {
        description = "Mainnet: mfm::portfolio::snapshot (Helios) and assert Vitalik has ETH";
        run = ''
          eval "$($SLOT_INFO)"

          export HELIOS_NETWORK="mainnet"
          export HELIOS_EXECUTION_RPC_URL="https://eth.llamarpc.com"
          # Intentionally rely on the Nixfied Helios service default consensus endpoint.

          # Mainnet Helios can take a while to sync; gate on eth_blockNumber.
          export HELIOS_READY_TIMEOUT_SECS="''${HELIOS_READY_TIMEOUT_SECS:-900}"

          export MFM_ARTIFACT_ROOT="$CI_ARTIFACTS_DIR/mfm-mainnet-artifacts"
          mkdir -p "$MFM_ARTIFACT_ROOT"

          ADDRESS="0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045"

          LOGFILE=$(artifact_path "mainnet-portfolio-snapshot.log")
          OUTFILE=$(artifact_path "mainnet-portfolio-snapshot.json")

          set +e
          nix run .#mfm::portfolio::snapshot -- "$ADDRESS" >"$OUTFILE" 2>"$LOGFILE"
          rc=$?
          set -e

          if [ $rc -ne 0 ]; then
            echo "ERROR: mfm::portfolio::snapshot failed rc=$rc" >&2
            tail -200 "$LOGFILE" >&2 || true
            if [ -s "$OUTFILE" ]; then
              echo "STDOUT:" >&2
              cat "$OUTFILE" >&2 || true
            fi
            exit $rc
          fi

          jq -e '.status == "success"' "$OUTFILE" >/dev/null
          jq -e '.data.feature_id == "portfolio.snapshot"' "$OUTFILE" >/dev/null
          jq -e '.data.result.phase == "completed"' "$OUTFILE" >/dev/null

          ART_ID=$(jq -r '.data.result.snapshot_artifact_id // empty' "$OUTFILE")
          if [ -z "$ART_ID" ] || [ "$ART_ID" = "null" ]; then
            echo "ERROR: missing snapshot_artifact_id" >&2
            cat "$OUTFILE" >&2
            exit 1
          fi

          SNAPSHOT_FILE="$MFM_ARTIFACT_ROOT/''${ART_ID:0:2}/$ART_ID"
          if [ ! -f "$SNAPSHOT_FILE" ]; then
            echo "ERROR: snapshot artifact not found at $SNAPSHOT_FILE" >&2
            exit 1
          fi

          BAL_WEI=$(jq -r '.native.raw_u256_dec // empty' "$SNAPSHOT_FILE")
          if [ -z "$BAL_WEI" ] || [ "$BAL_WEI" = "null" ] || [ "$BAL_WEI" = "0" ]; then
            echo "ERROR: expected non-zero ETH balance; got native.raw_u256_dec=$BAL_WEI" >&2
            cat "$SNAPSHOT_FILE" >&2
            exit 1
          fi

          echo "OK: mainnet snapshot non-zero ETH balance wei=$BAL_WEI artifact_id=$ART_ID"
        '';
      };
    };
  };
}
