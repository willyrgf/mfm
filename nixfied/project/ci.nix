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
        "${project.slotVar}" = "0";
      };
      useDeps = true;
      # Note: `nix run .#ci` is implemented by the framework CI runner (nixfied/.framework/ci.nix),
      # which reads `project.ci.*` below. This command exists primarily for `nix run .#help`.
      script = "";
    };
  };

  ci = {
    enable = true;
    defaultMode = "basic";
    env = {
      "${project.envVar}" = "test";
      "${project.slotVar}" = "0";
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
          "parity-reth"
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
        requires = [ "minio" ];
        run = ''
          eval "$($SLOT_INFO)"

          # Use the MinIO service module contract in CI parity (without supervisor).
          run_hook MINIO_INIT
          MINIO_LOGFILE=$(artifact_path "minio.log")
          MINIO_PID=$(start_service minio \
            --log "$MINIO_LOGFILE" \
            --wait-http "http://127.0.0.1:$MINIO_PORT/minio/health/ready" \
            --timeout 60 \
            -- \
            "$MINIO_START")
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

      parity-reth = {
        description = "Parity: local reth JSON-RPC lane";
        requires = [ "postgres" ];
        run = ''
          eval "$($SLOT_INFO)"
          run_hook POSTGRES_FULL_START_TEST

          if ! command -v reth >/dev/null 2>&1; then
            fail "reth binary not available"
          fi

          export DATABASE_URL="postgresql://postgres:postgres@127.0.0.1:$POSTGRES_PORT/mfm_test"
          export MFM_EVM_RPC_URL="http://127.0.0.1:$RETH_RPC_PORT"

          if command -v solc >/dev/null 2>&1; then
            export FOUNDRY_SOLC="$(command -v solc)"
          fi

          if [ -f foundry.toml ]; then
            if ! command -v forge >/dev/null 2>&1; then
              fail "forge binary not available"
            fi

            FORGE_LOGFILE=$(artifact_path "forge-build.log")
            if ! log_capture "$FORGE_LOGFILE" -- forge build --offline; then
              echo "WARN: forge build failed in parity lane; continuing without blocking reth RPC parity checks" >&2
            fi
          fi

          RETH_DATA_DIR=$(mktemp -d "/tmp/mfm-reth-$SLOT-$ENV-XXXXXX")
          with_cleanup "rm -rf \"$RETH_DATA_DIR\""

          RETH_LOGFILE=$(artifact_path "reth.log")
          RETH_PID=$(start_service reth \
            --log "$RETH_LOGFILE" \
            --wait-port "$RETH_RPC_PORT" \
            --timeout 60 \
            -- \
            reth node \
              --dev \
              --datadir "$RETH_DATA_DIR" \
              --http \
              --http.addr "127.0.0.1" \
              --http.port "$RETH_RPC_PORT")
          with_cleanup "stop_service $RETH_PID reth"

          rpc_call() {
            local method="$1"
            local params="$2"
            curl -sS -H "content-type: application/json" \
              --data "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"$method\",\"params\":$params}" \
              "$MFM_EVM_RPC_URL"
          }

          CHAIN_ID_HEX=$(rpc_call "eth_chainId" "[]" | sed -n 's/.*"result"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')
          if [ -z "$CHAIN_ID_HEX" ]; then
            fail "failed to read eth_chainId from reth"
          fi
          CHAIN_ID_DEC=$((16#''${CHAIN_ID_HEX#0x}))

          FROM_ADDR=$(rpc_call "eth_accounts" "[]" | sed -n 's/.*"result"[[:space:]]*:[[:space:]]*\[[[:space:]]*"\([^"]*\)".*/\1/p')
          if [ -z "$FROM_ADDR" ]; then
            fail "failed to read first eth_accounts address from reth"
          fi

          if [ ! -f foundry.toml ]; then
            fail "foundry.toml not found; cannot build deploy/configure/validate artifact"
          fi

          ABI_JSON=$(forge inspect --json "contracts/src/ConfigurableCounter.sol:ConfigurableCounter" abi)
          BYTECODE_HEX=$(forge inspect --json "contracts/src/ConfigurableCounter.sol:ConfigurableCounter" bytecode)
          if [ -z "$ABI_JSON" ] || [ -z "$BYTECODE_HEX" ]; then
            fail "failed to inspect contract artifact from forge"
          fi
          case "$BYTECODE_HEX" in
            0x*) ;;
            *) BYTECODE_HEX="0x$BYTECODE_HEX" ;;
          esac

          SPEC_FILE=$(mktemp "/tmp/mfm-dcv-spec-$SLOT-$ENV-XXXXXX.json")
          with_cleanup "rm -f \"$SPEC_FILE\""
          {
            printf '{\n'
            printf '  "machine_id": "evm_deploy_configure_validate",\n'
            printf '  "pipeline_version": "v1",\n'
            printf '  "input": {},\n'
            printf '  "deploy": {\n'
            printf '    "artifact": {\n'
            printf '      "abi": %s,\n' "$ABI_JSON"
            printf '      "bytecode": "%s"\n' "$BYTECODE_HEX"
            printf '    },\n'
            printf '    "from": "%s",\n' "$FROM_ADDR"
            printf '    "constructor_args": [1]\n'
            printf '  },\n'
            printf '  "configure": {\n'
            printf '    "artifact": {\n'
            printf '      "abi": %s,\n' "$ABI_JSON"
            printf '      "bytecode": "%s"\n' "$BYTECODE_HEX"
            printf '    },\n'
            printf '    "from": "%s",\n' "$FROM_ADDR"
            printf '    "calls": [\n'
            printf '      {\n'
            printf '        "function": "setValue",\n'
            printf '        "args": [42]\n'
            printf '      }\n'
            printf '    ]\n'
            printf '  },\n'
            printf '  "validate": {\n'
            printf '    "artifact": {\n'
            printf '      "abi": %s,\n' "$ABI_JSON"
            printf '      "bytecode": "%s"\n' "$BYTECODE_HEX"
            printf '    },\n'
            printf '    "expected_chain_id": %s,\n' "$CHAIN_ID_DEC"
            printf '    "read_assertions": [\n'
            printf '      {\n'
            printf '        "function": "getValue",\n'
            printf '        "args": [],\n'
            printf '        "expected": 42\n'
            printf '      }\n'
            printf '    ],\n'
            printf '    "event_assertions": [\n'
            printf '      {\n'
            printf '        "event": "ValueSet",\n'
            printf '        "min_count": 2\n'
            printf '      }\n'
            printf '    ]\n'
            printf '  }\n'
            printf '}\n'
          } > "$SPEC_FILE"

          LOGFILE=$(artifact_path "parity-reth.log")
          log_capture "$LOGFILE" -- \
            cargo run -p mfm --bin mfm_cli -- --output-format json run pipeline deploy-configure-validate \
              --spec-file "$SPEC_FILE"
        '';
      };
    };
  };
}
