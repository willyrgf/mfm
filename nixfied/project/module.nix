{ lib, pkgs, ... }:
let
  conf = import ./conf.nix { inherit pkgs; };
  project = conf.project;

  envNames = builtins.attrNames conf.envs;
  envOffsets = lib.mapAttrs (_: value: value.offset or 0) conf.envs;

  thinWrapperFlake = import ../install/wrapper-flake.nix {
    frameworkInput = "github:willyrgf/nixfied";
  };

  vendoredWrapperFlake = import ../install/wrapper-flake.nix {
    vendorPath = "./nixfied";
  };

  commonRuntimeInputs =
    [
      pkgs.bash
      pkgs.coreutils
      pkgs.findutils
      pkgs.gnused
      pkgs.gnugrep
      pkgs.jq
      pkgs.nix
    ]
    ++ (conf.tooling.runtimePackages or [ ]);

  rustRuntimeInputs = commonRuntimeInputs;

  sharedPassThroughEnv = [
    "HOME"
    project.envVar
    project.slotVar
    "CI_ARTIFACTS_DIR"
    "CI_MAX_WORKERS"
    "NIXFIED_CI_MAX_WORKERS"
    "API_KEY"
    "LOG_LEVEL"
    "OUTPUT_MODE"
    "RUST_LOG"
    "MFM_LOG"
    "LOG_FORMAT"
    "MFM_LOG_FORMAT"
    "LOG_SPAN_EVENTS"
    "MFM_LOG_SPAN_EVENTS"
    "HELIOS_NETWORK"
    "HELIOS_EXECUTION_RPC_URL"
    "HELIOS_CONSENSUS_RPC_URL"
    "HELIOS_CHECKPOINT"
    "MFM_CI_ENABLE_PARITY"
    "MFM_CI_ENABLE_MAINNET"
  ];

  ciStepPreamble = ''
    artifacts_dir="''${CI_ARTIFACTS_DIR:-/tmp/ci-artifacts}"
    mkdir -p "$artifacts_dir"

    run_with_log() {
      local logfile="$1"
      shift
      local mode
      mode="$(printf '%s' "''${OUTPUT_MODE:-stdout}" | tr '[:upper:]' '[:lower:]')"

      case "$mode" in
        logs)
          "$@" >"$logfile" 2>&1
          ;;
        stdout|both|"")
          "$@" 2>&1 | tee "$logfile"
          ;;
        *)
          "$@" >"$logfile" 2>&1
          ;;
      esac
    }
  '';

  mkCommandTask =
    {
      id,
      appName,
      summary,
      description ? "",
      command ? "",
      kind ? "command",
      tags ? [ ],
      usage ? [ ],
      examples ? [ ],
      runtimeInputs ? commonRuntimeInputs,
      workflowId ? null,
      contractArgs ? [ ],
      allowUnknownArgs ? true,
      outputFormat ? "text",
      outputChannels ? "stdout",
      effects ? [ "writes-state" ],
      idempotent ? false,
      passThroughEnv ? sharedPassThroughEnv,
      env ? { },
    }:
    {
      inherit
        id
        kind
        summary
        description
        tags
        ;

      runner =
        if workflowId == null then
          {
            type = "shell";
            command = command;
          }
        else
          {
            type = "workflowRef";
            workflowId = workflowId;
          };

      contract = {
        version = 1;
        input = {
          args = {
            parser = "typed";
            allowUnknown = allowUnknownArgs;
            spec = contractArgs;
          };
          env = {
            schemaRef = "runtimePrimitives";
            extra = [ ];
          };
        };
        output = {
          format = outputFormat;
          channels = outputChannels;
          keys = [ ];
        };
        behavior = {
          idempotent = idempotent;
          effects = effects;
          timeoutSec = 0;
        };
        errors.codes = {
          generic = 1;
          usage = 2;
          precondition = 3;
        };
      };

      runtime = {
        slotEnv = "optional";
        workdir = "projectRoot";
        hermetic = true;
        runtimeInputs = runtimeInputs;
        passThroughEnv = passThroughEnv;
        inherit env;
        umask = "022";
        locale = "C.UTF-8";
        timezone = "UTC";
      };

      scheduling = {
        locks = [ ];
        maxAttempts = 1;
        retryBackoffSec = [ ];
        priority = 100;
      };

      deps = {
        needs = [ ];
        softNeeds = [ ];
      };

      produces = {
        artifacts = [ ];
        stateKeys = [ ];
      };

      ui.app = {
        expose = true;
        name = appName;
        category = "core";
        usage = usage;
        examples = examples;
      };
    };

  mkWorkflowUnit =
    {
      taskId,
      needs ? [ ],
      skipIfMissingEnv ? [ ],
    }:
    {
      inherit
        taskId
        needs
        skipIfMissingEnv
        ;
      locks = [ ];
      when = {
        envEquals = { };
        envPresent = [ ];
      };
    };
in
{
  imports = [
    ../modules/profiles/webapp.nix
  ];

  config = {
    nixfied = {
      identity = {
        projectId = project.id;
        projectName = project.name;
        description = project.description;
      };

      runtime = {
        slot = {
          var = project.slotVar;
          default = conf.slots.default;
          max = conf.slots.max;
          stride = conf.slots.stride;
        };

        env = {
          var = project.envVar;
          names = envNames;
          offsets = envOffsets;
          default = "dev";
        };

        logging = {
          levelDefault = conf.logging.level;
          outputDefault = conf.logging.output;
        };

        ports = conf.ports;
        directories.base = conf.directories.base;
      };

      state = {
        registryRoot = conf.process.registryRoot;
        artifactsRoot = "/tmp/ci-artifacts";
      };

      tooling = {
        runtimePackages = conf.tooling.runtimePackages;
        devShellPackages = conf.tooling.devShellPackages;
        devShellHook = conf.tooling.devShellHook;
      };

      services = {
        postgres = {
          enable = conf.modules.postgres.enable or false;
          database = conf.modules.postgres.database or "app";
          portKey = conf.modules.postgres.portKey or "postgres";
        };

        nginx = {
          enable = conf.modules.nginx.enable or false;
          portKeyHttp = conf.modules.nginx.portKeyHttp or "http";
          portKeyHttps = conf.modules.nginx.portKeyHttps or "https";
        };

        minio = {
          enable = conf.modules.minio.enable or false;
          portKeyApi = conf.modules.minio.portKeyApi or "minioApi";
          portKeyConsole = conf.modules.minio.portKeyConsole or "minioConsole";
        };

        reth = {
          enable = conf.modules.reth.enable or false;
          portKeyHttp = conf.modules.reth.portKeyHttp or "rethHttp";
          portKeyWs = conf.modules.reth.portKeyWs or "rethWs";
          portKeyAuth = conf.modules.reth.portKeyAuth or "rethAuth";
        };

        helios = {
          enable = conf.modules.helios.enable or false;
          portKeyRpc = conf.modules.helios.portKeyRpc or "heliosRpc";
          executionRpcPortKey = conf.modules.helios.executionRpcPortKey or "rethHttp";
        };
      };

      operations = {
        enable = true;
        validateEnv.enable = true;
        testIsolation.enable = true;
        ports.enable = true;
        checkPorts.enable = true;
      };

      tasks = {
        dev = mkCommandTask {
          id = "task.dev";
          appName = "dev";
          summary = "Start the MFM REST API in dev mode";
          description = ''
            Runs mfm_rest_api via cargo with developer-friendly defaults.
          '';
          tags = [
            "dev"
            "local"
          ];
          usage = [ "MFM_ENV=dev NIX_ENV=0 nix run .#dev" ];
          examples = [
            "DATABASE_URL=postgresql://postgres:postgres@127.0.0.1:5432/mfm nix run .#dev"
          ];
          runtimeInputs = rustRuntimeInputs;
          command = ''
            set -euo pipefail

            echo "INFO: starting dev workflow"

            if [ -z "''${DATABASE_URL:-}" ]; then
              export DATABASE_URL="postgresql://postgres:postgres@127.0.0.1:5432/mfm"
            fi

            if [ -z "''${MFM_EVM_RPC_URL:-}" ]; then
              export MFM_EVM_RPC_URL="http://127.0.0.1:8545"
            fi

            if [ -z "''${MFM_REST_API_ADDR:-}" ]; then
              export MFM_REST_API_ADDR="127.0.0.1:3001"
            fi

            echo "INFO: launching mfm_rest_api addr=$MFM_REST_API_ADDR"
            exec cargo run -p mfm-rest-api --bin mfm_rest_api -- "$@"
          '';
        };

        build = mkCommandTask {
          id = "task.build";
          appName = "build";
          summary = "Build release artifacts";
          description = "Builds the workspace in release mode with all features enabled.";
          usage = [ "nix run .#build" ];
          runtimeInputs = rustRuntimeInputs;
          command = ''
            set -euo pipefail

            echo "INFO: running release build"
            cargo build --release --all-features
            echo "OK: build completed"
          '';
        };

        check = mkCommandTask {
          id = "task.check";
          appName = "check";
          summary = "Run fmt + clippy + architecture verification";
          description = "Runs quality checks for the workspace.";
          usage = [ "nix run .#check" ];
          runtimeInputs = rustRuntimeInputs;
          command = ''
            set -euo pipefail

            echo "INFO: running formatting checks"
            cargo fmt --all -- --check

            echo "INFO: running clippy"
            cargo clippy --workspace --lib --examples --tests --benches --all-features -- -D warnings

            echo "INFO: running architecture verifier"
            cargo run -p mfm-architecture-verify --

            echo "OK: quality checks completed"
          '';
        };

        format = mkCommandTask {
          id = "task.format";
          appName = "format";
          summary = "Format Nix files";
          usage = [ "nix run .#format" ];
          command = ''
            set -euo pipefail

            find . -name '*.nix' -print0 | xargs -0 nixfmt --
            echo "OK: formatted nix files"
          '';
        };

        test = mkCommandTask {
          id = "task.test";
          appName = "test";
          summary = "Run workspace tests";
          description = "Runs the full workspace test suite using cargo-nextest.";
          usage = [ "nix run .#test" ];
          runtimeInputs = rustRuntimeInputs;
          env =
            {
              RUSTC_WRAPPER = "sccache";
              CARGO_PROFILE_CI_DEBUG = "0";
            }
            // lib.optionalAttrs pkgs.stdenv.isDarwin {
              LIBRARY_PATH = "${pkgs.libiconv}/lib";
            };
          command = ''
            set -euo pipefail

            echo "INFO: running workspace tests"
            cargo nextest run --workspace --cargo-profile ci
            echo "OK: tests completed"
          '';
        };

        ci = mkCommandTask {
          id = "task.ci";
          appName = "ci";
          kind = "workflow";
          summary = "Run CI workflows (basic|audit|parity|full|mainnet)";
          description = "Dispatches to model workflows and supports legacy CI mode flags.";
          usage = [
            "nix run .#ci -- --basic --summary"
            "nix run .#ci -- --audit --summary"
            "nix run .#ci -- --parity --summary"
            "nix run .#ci -- --full --summary"
            "nix run .#ci -- --mainnet --summary"
            "nix run .#ci -- --mode parity --summary"
          ];
          runtimeInputs = rustRuntimeInputs;
          workflowId = "workflow.ci.full";
          contractArgs = [
            {
              name = "summary";
              kind = "flag";
              long = "--summary";
              description = "Print compact workflow summary output.";
            }
            {
              name = "mode";
              kind = "option";
              long = "--mode";
              type = "enum";
              values = [
                "basic"
                "audit"
                "parity"
                "full"
                "mainnet"
              ];
              description = "CI mode to run.";
            }
            {
              name = "basic";
              kind = "flag";
              long = "--basic";
              description = "Alias for --mode basic.";
            }
            {
              name = "audit";
              kind = "flag";
              long = "--audit";
              description = "Alias for --mode audit.";
            }
            {
              name = "parity";
              kind = "flag";
              long = "--parity";
              description = "Alias for --mode parity.";
            }
            {
              name = "full";
              kind = "flag";
              long = "--full";
              description = "Alias for --mode full.";
            }
            {
              name = "mainnet";
              kind = "flag";
              long = "--mainnet";
              description = "Alias for --mode mainnet.";
            }
            {
              name = "bg";
              kind = "flag";
              long = "--bg";
              description = "Compatibility flag; currently runs foreground only.";
            }
            {
              name = "background";
              kind = "flag";
              long = "--background";
              description = "Compatibility alias for --bg.";
            }
          ];
        };

        ci-fmt =
          mkCommandTask {
            id = "task.ci.fmt";
            appName = "ci-fmt";
            kind = "ci-step";
            summary = "CI formatting step";
            tags = [
              "ci"
              "quality"
            ];
            runtimeInputs = rustRuntimeInputs;
            command = ''
              set -euo pipefail
              ${ciStepPreamble}

              log_file="$artifacts_dir/fmt.log"
              echo "INFO: running ci step=fmt"
              run_with_log "$log_file" cargo fmt --all -- --check
              echo "OK: ci step passed step=fmt log=$log_file"
            '';
          }
          // {
            ui.app.expose = false;
          };

        ci-clippy =
          mkCommandTask {
            id = "task.ci.clippy";
            appName = "ci-clippy";
            kind = "ci-step";
            summary = "CI clippy step";
            tags = [
              "ci"
              "quality"
            ];
            runtimeInputs = rustRuntimeInputs;
            command = ''
              set -euo pipefail
              ${ciStepPreamble}

              log_file="$artifacts_dir/clippy.log"
              echo "INFO: running ci step=clippy"
              run_with_log "$log_file" cargo clippy --workspace --lib --examples --tests --benches --all-features -- -D warnings
              echo "OK: ci step passed step=clippy log=$log_file"
            '';
          }
          // {
            ui.app.expose = false;
          };

        ci-architecture-verify =
          mkCommandTask {
            id = "task.ci.architecture-verify";
            appName = "ci-architecture-verify";
            kind = "ci-step";
            summary = "CI architecture verification step";
            tags = [
              "ci"
              "quality"
            ];
            runtimeInputs = rustRuntimeInputs;
            command = ''
              set -euo pipefail
              ${ciStepPreamble}

              log_file="$artifacts_dir/architecture-verify.log"
              echo "INFO: running ci step=architecture-verify"
              run_with_log "$log_file" cargo run -p mfm-architecture-verify --
              echo "OK: ci step passed step=architecture-verify log=$log_file"
            '';
          }
          // {
            ui.app.expose = false;
          };

        ci-shell-app-contracts =
          mkCommandTask {
            id = "task.ci.shell-app-contracts";
            appName = "ci-shell-app-contracts";
            kind = "ci-step";
            summary = "CI shell/model surface contract checks";
            tags = [
              "ci"
              "quality"
            ];
            runtimeInputs = rustRuntimeInputs;
            command = ''
              set -euo pipefail
              ${ciStepPreamble}

              ROOT="$(pwd -P)"
              export ROOT

              log_file="$artifacts_dir/shell-app-contracts.log"
              echo "INFO: running ci step=shell-app-contracts"
              run_with_log "$log_file" bash -euo pipefail -c '
                test -f "$ROOT/flake.nix"
                test -f "$ROOT/nixfied/schemas/task-contract.json"
                test -f "$ROOT/nixfied/schemas/workflow-contract.json"
                test -f "$ROOT/nixfied/schemas/model-export.json"

                jq -e "." "$ROOT/nixfied/schemas/task-contract.json" >/dev/null
                jq -e "." "$ROOT/nixfied/schemas/workflow-contract.json" >/dev/null
                jq -e "." "$ROOT/nixfied/schemas/model-export.json" >/dev/null

                grep -q "id = \"task.ci\";" "$ROOT/nixfied/project/module.nix"
                grep -q "id = \"workflow.ci.full\";" "$ROOT/nixfied/project/module.nix"
              '
              echo "OK: ci step passed step=shell-app-contracts log=$log_file"
            '';
          }
          // {
            ui.app.expose = false;
          };

        ci-tests =
          mkCommandTask {
            id = "task.ci.tests";
            appName = "ci-tests";
            kind = "ci-step";
            summary = "CI tests step";
            tags = [
              "ci"
              "tests"
            ];
            runtimeInputs = rustRuntimeInputs;
            env =
              {
                RUSTC_WRAPPER = "sccache";
                CARGO_PROFILE_CI_DEBUG = "0";
              }
              // lib.optionalAttrs pkgs.stdenv.isDarwin {
                LIBRARY_PATH = "${pkgs.libiconv}/lib";
              };
            command = ''
              set -euo pipefail
              ${ciStepPreamble}

              log_file="$artifacts_dir/tests.log"
              echo "INFO: running ci step=tests"
              run_with_log "$log_file" cargo nextest run --workspace --cargo-profile ci
              echo "OK: ci step passed step=tests log=$log_file"
            '';
          }
          // {
            ui.app.expose = false;
          };

        ci-audit =
          mkCommandTask {
            id = "task.ci.audit";
            appName = "ci-audit";
            kind = "ci-step";
            summary = "CI security audit step";
            tags = [
              "ci"
              "audit"
            ];
            runtimeInputs = rustRuntimeInputs;
            command = ''
              set -euo pipefail
              ${ciStepPreamble}

              log_file="$artifacts_dir/audit.log"
              echo "INFO: running ci step=audit"
              run_with_log "$log_file" cargo audit
              echo "OK: ci step passed step=audit log=$log_file"
            '';
          }
          // {
            ui.app.expose = false;
          };

        ci-parity-compile =
          mkCommandTask {
            id = "task.ci.parity-compile";
            appName = "ci-parity-compile";
            kind = "ci-step";
            summary = "CI parity precompile step";
            tags = [
              "ci"
              "parity"
            ];
            runtimeInputs = rustRuntimeInputs;
            command = ''
              set -euo pipefail
              ${ciStepPreamble}

              log_file="$artifacts_dir/parity-compile.log"
              echo "INFO: running ci step=parity-compile"
              run_with_log "$log_file" cargo nextest run --no-run --cargo-profile ci -p mfm-integration-tests --features parity-tests -p mfm --features parity-tests
              echo "OK: ci step passed step=parity-compile log=$log_file"
            '';
          }
          // {
            ui.app.expose = false;
          };

        ci-parity-rest-api-smoke =
          mkCommandTask {
            id = "task.ci.parity-rest-api-smoke";
            appName = "ci-parity-rest-api-smoke";
            kind = "ci-step";
            summary = "CI parity REST API smoke tests";
            tags = [
              "ci"
              "parity"
            ];
            runtimeInputs = rustRuntimeInputs;
            command = ''
              set -euo pipefail
              ${ciStepPreamble}

              log_file="$artifacts_dir/parity-rest-api-smoke.log"
              echo "INFO: running ci step=parity-rest-api-smoke"
              run_with_log "$log_file" cargo nextest run --cargo-profile ci --jobs 1 -p mfm-integration-tests --features parity-tests --test parity_event_store_postgres_contract --test parity_artifact_store_s3_contract --test parity_rest_api_postgres_s3_smoke
              echo "OK: ci step passed step=parity-rest-api-smoke log=$log_file"
            '';
          }
          // {
            ui.app.expose = false;
          };

        ci-parity-evm-helios-smoke =
          mkCommandTask {
            id = "task.ci.parity-evm-helios-smoke";
            appName = "ci-parity-evm-helios-smoke";
            kind = "ci-step";
            summary = "CI parity helios smoke tests";
            tags = [
              "ci"
              "parity"
            ];
            runtimeInputs = rustRuntimeInputs;
            command = ''
              set -euo pipefail
              ${ciStepPreamble}

              key_log_file="$artifacts_dir/parity-keystore-reth-tx-sign-send.log"
              smoke_log_file="$artifacts_dir/parity-evm-helios-smoke.log"
              response_file="$artifacts_dir/parity-evm-helios-smoke.response.json"

              echo "INFO: running ci step=parity-evm-helios-smoke"
              run_with_log "$key_log_file" cargo nextest run --cargo-profile ci -p mfm --features parity-tests --test parity_keystore_reth_tx_send

              if [ -z "''${MFM_EVM_RPC_URL:-}" ]; then
                echo "SKIP: MFM_EVM_RPC_URL is unset; skipping helios RPC curl probe"
                exit 0
              fi

              run_with_log "$smoke_log_file" bash -euo pipefail -c '
                curl -fsS \
                  -H "content-type: application/json" \
                  --data "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"eth_chainId\",\"params\":[]}" \
                  "$MFM_EVM_RPC_URL" \
                  | tee "$response_file"
              '

              jq -e '.result | strings' "$response_file" >/dev/null
              echo "OK: ci step passed step=parity-evm-helios-smoke log=$smoke_log_file"
            '';
          }
          // {
            ui.app.expose = false;
          };

        ci-parity-evm-reth =
          mkCommandTask {
            id = "task.ci.parity-evm-reth";
            appName = "ci-parity-evm-reth";
            kind = "ci-step";
            summary = "CI parity EVM + portfolio tracker tests";
            tags = [
              "ci"
              "parity"
            ];
            runtimeInputs = rustRuntimeInputs;
            command = ''
              set -euo pipefail
              ${ciStepPreamble}

              log_file="$artifacts_dir/parity-evm-reth.log"
              echo "INFO: running ci step=parity-evm-reth"
              run_with_log "$log_file" cargo nextest run --cargo-profile ci -p mfm-integration-tests --features parity-tests --test evm_rpc_pool_failover --test evm_rpc_getlogs_chunking --test parity_rest_api_evm_reth_pipeline --test parity_portfolio_tracker_reth_mock_erc20 --test parity_portfolio_tracker_reth_snapshot
              echo "OK: ci step passed step=parity-evm-reth log=$log_file"
            '';
          }
          // {
            ui.app.expose = false;
          };

        ci-parity-aave-v3-reth =
          mkCommandTask {
            id = "task.ci.parity-aave-v3-reth";
            appName = "ci-parity-aave-v3-reth";
            kind = "ci-step";
            summary = "CI parity Aave v3 scenario tests";
            tags = [
              "ci"
              "parity"
            ];
            runtimeInputs = rustRuntimeInputs;
            command = ''
              set -euo pipefail
              ${ciStepPreamble}

              log_file="$artifacts_dir/parity-aave-v3-reth.log"
              echo "INFO: running ci step=parity-aave-v3-reth"
              run_with_log "$log_file" cargo nextest run --cargo-profile ci -p mfm-integration-tests --features parity-tests --test parity_aave_v3_reth_scenario
              echo "OK: ci step passed step=parity-aave-v3-reth log=$log_file"
            '';
          }
          // {
            ui.app.expose = false;
          };

        ci-parity-postgres-state-events-audit =
          mkCommandTask {
            id = "task.ci.parity-postgres-state-events-audit";
            appName = "ci-parity-postgres-state-events-audit";
            kind = "ci-step";
            summary = "CI parity postgres state event audit";
            tags = [
              "ci"
              "parity"
            ];
            runtimeInputs = rustRuntimeInputs;
            command = ''
              set -euo pipefail
              ${ciStepPreamble}

              log_file="$artifacts_dir/parity-postgres-state-events-audit.log"
              echo "INFO: running ci step=parity-postgres-state-events-audit"
              run_with_log "$log_file" cargo nextest run --cargo-profile ci -p mfm-integration-tests --features parity-tests --test parity_postgres_state_events_audit
              echo "OK: ci step passed step=parity-postgres-state-events-audit log=$log_file"
            '';
          }
          // {
            ui.app.expose = false;
          };

        ci-mainnet-portfolio-snapshot-helios =
          mkCommandTask {
            id = "task.ci.mainnet-portfolio-snapshot-helios";
            appName = "ci-mainnet-portfolio-snapshot-helios";
            kind = "ci-step";
            summary = "CI mainnet portfolio snapshot validation";
            tags = [
              "ci"
              "mainnet"
            ];
            runtimeInputs = rustRuntimeInputs;
            command = ''
              set -euo pipefail
              ${ciStepPreamble}

              address="''${MFM_CI_MAINNET_ADDRESS:-0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045}"
              log_file="$artifacts_dir/mainnet-portfolio-snapshot.log"
              out_file="$artifacts_dir/mainnet-portfolio-snapshot.json"

              export HELIOS_NETWORK="''${HELIOS_NETWORK:-mainnet}"
              export HELIOS_EXECUTION_RPC_URL="''${HELIOS_EXECUTION_RPC_URL:-https://eth.drpc.org}"

              echo "INFO: running ci step=mainnet-portfolio-snapshot-helios address=$address"

              mode="$(printf '%s' "''${OUTPUT_MODE:-stdout}" | tr '[:upper:]' '[:lower:]')"
              case "$mode" in
                logs)
                  cargo run -q -p mfm --bin mfm_cli -- --output-format json portfolio snapshot "$address" --chain-id 1 >"$out_file" 2>"$log_file"
                  ;;
                stdout|both|"")
                  cargo run -q -p mfm --bin mfm_cli -- --output-format json portfolio snapshot "$address" --chain-id 1 > >(tee "$out_file") 2> >(tee "$log_file" >&2)
                  ;;
                *)
                  cargo run -q -p mfm --bin mfm_cli -- --output-format json portfolio snapshot "$address" --chain-id 1 >"$out_file" 2>"$log_file"
                  ;;
              esac

              jq -e '.status == "success"' "$out_file" >/dev/null
              jq -e '.data.result.phase == "completed"' "$out_file" >/dev/null
              echo "OK: ci step passed step=mainnet-portfolio-snapshot-helios log=$log_file"
            '';
          }
          // {
            ui.app.expose = false;
          };

        framework-test = mkCommandTask {
          id = "task.framework.test";
          appName = "framework::test";
          kind = "utility";
          summary = "Run framework validation shards";
          description = ''
            Runs deterministic framework validation shards.
          '';
          runtimeInputs = rustRuntimeInputs;
          usage = [
            "nix run .#framework::test"
            "nix run .#framework::test -- --summary"
            "nix run .#framework::test -- --mode parity --summary"
          ];
          examples = [
            "nix run .#framework::test -- --list-shards"
            "nix run .#framework::test -- --shard workflow-ci"
            "FRAMEWORK_ISOLATION=1 nix run .#framework::test -- --summary"
          ];
          contractArgs = [
            {
              name = "summary";
              kind = "flag";
              long = "--summary";
              description = "Print compact summary output.";
            }
            {
              name = "summary-json";
              kind = "option";
              long = "--summary-json";
              type = "string";
              description = "Write summary JSON to a file.";
            }
            {
              name = "shard";
              kind = "option";
              long = "--shard";
              type = "string";
              values = [
                "flake-check"
                "help"
                "workflow-test"
                "workflow-ci"
                "isolation"
              ];
              description = "Run one shard only.";
            }
            {
              name = "list-shards";
              kind = "flag";
              long = "--list-shards";
              description = "List available shards and exit.";
            }
            {
              name = "mode";
              kind = "option";
              long = "--mode";
              type = "enum";
              values = [
                "basic"
                "audit"
                "parity"
                "full"
                "mainnet"
              ];
              description = "CI workflow mode used by the workflow-ci shard.";
            }
            {
              name = "basic";
              kind = "flag";
              long = "--basic";
              description = "Alias for --mode basic.";
            }
            {
              name = "audit";
              kind = "flag";
              long = "--audit";
              description = "Alias for --mode audit.";
            }
            {
              name = "parity";
              kind = "flag";
              long = "--parity";
              description = "Alias for --mode parity.";
            }
            {
              name = "full";
              kind = "flag";
              long = "--full";
              description = "Alias for --mode full.";
            }
            {
              name = "mainnet";
              kind = "flag";
              long = "--mainnet";
              description = "Alias for --mode mainnet.";
            }
          ];
          command = ''
            set -euo pipefail

            ROOT="$(pwd -P)"
            MODE="full"
            SHARD=""
            LIST_SHARDS=0
            SUMMARY=0
            SUMMARY_JSON=""
            SHARDS=(
              "flake-check"
              "help"
              "workflow-test"
              "workflow-ci"
              "isolation"
            )
            EXECUTED=0
            STARTED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
            START_EPOCH="$(date +%s)"

            log_info() {
              printf 'INFO: %s\n' "$*"
            }

            log_error() {
              printf 'ERROR: %s\n' "$*" >&2
            }

            log_ok() {
              printf 'OK: %s\n' "$*"
            }

            log_skip() {
              printf 'SKIP: %s\n' "$*"
            }

            usage() {
              cat <<'USAGE'
            Usage: nix run .#framework::test [-- --mode <basic|audit|parity|full|mainnet>] [--summary] [--summary-json <path>] [--shard <name>] [--list-shards]

            Shards:
              flake-check   Run nix flake check for the current project root.
              help          Validate generated help output.
              workflow-test Run the test app surface.
              workflow-ci   Run the ci app surface in selected mode.
              isolation     Run isolation checks when FRAMEWORK_ISOLATION=1 or explicitly selected.
            USAGE
            }

            print_shards() {
              local shard_name
              for shard_name in "''${SHARDS[@]}"; do
                printf '%s\n' "$shard_name"
              done
            }

            shard_exists() {
              local candidate="$1"
              local shard_name
              for shard_name in "''${SHARDS[@]}"; do
                if [ "$candidate" = "$shard_name" ]; then
                  return 0
                fi
              done
              return 1
            }

            write_summary_json() {
              local rc="$1"
              local finished_at duration
              finished_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
              duration="$(( $(date +%s) - START_EPOCH ))"
              mkdir -p "$(dirname "$SUMMARY_JSON")"
              cat > "$SUMMARY_JSON" <<JSON
            {
              "mode": "$MODE",
              "shard": $(if [ -n "$SHARD" ]; then printf '"%s"' "$SHARD"; else printf 'null'; fi),
              "executed_shards": $EXECUTED,
              "exit_code": $rc,
              "duration_seconds": $duration,
              "started_at": "$STARTED_AT",
              "finished_at": "$finished_at"
            }
            JSON
              log_info "wrote summary json path=$SUMMARY_JSON"
            }

            run_shard() {
              local shard_name="$1"
              shift
              log_info "running shard=$shard_name"
              if "$@"; then
                EXECUTED="$((EXECUTED + 1))"
                log_ok "shard passed name=$shard_name"
                return 0
              fi
              local rc=$?
              log_error "shard failed name=$shard_name rc=$rc"
              return "$rc"
            }

            shard_flake_check() {
              nix flake check "path:$ROOT"
            }

            shard_help() {
              nix run "path:$ROOT"#help >/dev/null
            }

            shard_workflow_test() {
              nix run "path:$ROOT"#test
            }

            shard_workflow_ci() {
              nix run "path:$ROOT"#ci -- --mode "$MODE" --summary
            }

            shard_isolation() {
              if [ "''${FRAMEWORK_ISOLATION:-}" = "1" ] || [ "$SHARD" = "isolation" ]; then
                nix run "path:$ROOT"#test-isolation
                return 0
              fi
              log_skip "isolation shard disabled (set FRAMEWORK_ISOLATION=1 to enable)"
              return 0
            }

            run_named_shard() {
              local shard_name="$1"
              case "$shard_name" in
                flake-check)
                  run_shard "$shard_name" shard_flake_check
                  ;;
                help)
                  run_shard "$shard_name" shard_help
                  ;;
                workflow-test)
                  run_shard "$shard_name" shard_workflow_test
                  ;;
                workflow-ci)
                  run_shard "$shard_name" shard_workflow_ci
                  ;;
                isolation)
                  run_shard "$shard_name" shard_isolation
                  ;;
                *)
                  log_error "unknown shard '$shard_name'"
                  return 2
                  ;;
              esac
            }

            while [ "$#" -gt 0 ]; do
              case "$1" in
                --mode)
                  if [ "$#" -lt 2 ]; then
                    log_error "--mode requires a value"
                    exit 2
                  fi
                  MODE="$2"
                  shift 2
                  ;;
                --basic|--audit|--parity|--full|--mainnet)
                  MODE="''${1#--}"
                  shift
                  ;;
                --summary)
                  SUMMARY=1
                  shift
                  ;;
                --summary-json)
                  if [ "$#" -lt 2 ]; then
                    log_error "--summary-json requires a value"
                    exit 2
                  fi
                  SUMMARY_JSON="$2"
                  shift 2
                  ;;
                --shard)
                  if [ "$#" -lt 2 ]; then
                    log_error "--shard requires a value"
                    exit 2
                  fi
                  SHARD="$2"
                  shift 2
                  ;;
                --list-shards)
                  LIST_SHARDS=1
                  shift
                  ;;
                --help|-h)
                  usage
                  exit 0
                  ;;
                --)
                  shift
                  break
                  ;;
                *)
                  log_error "unknown option '$1'"
                  usage >&2
                  exit 2
                  ;;
              esac
            done

            if [ "$#" -gt 0 ]; then
              log_error "unexpected positional arguments: $*"
              exit 2
            fi

            case "$MODE" in
              basic|audit|parity|full|mainnet)
                ;;
              *)
                log_error "unknown mode '$MODE' (expected: basic|audit|parity|full|mainnet)"
                exit 2
                ;;
            esac

            if [ "$LIST_SHARDS" -eq 1 ]; then
              print_shards
              exit 0
            fi

            if [ -n "$SHARD" ] && ! shard_exists "$SHARD"; then
              log_error "unknown shard '$SHARD'"
              log_info "valid shards: $(print_shards | tr '\n' ' ')"
              exit 2
            fi

            cleanup() {
              local rc=$?
              if [ -n "$SUMMARY_JSON" ]; then
                write_summary_json "$rc"
              fi
              return "$rc"
            }
            trap cleanup EXIT

            if [ -n "$SHARD" ]; then
              run_named_shard "$SHARD"
            else
              for shard_name in "''${SHARDS[@]}"; do
                run_named_shard "$shard_name"
              done
            fi

            if [ "$SUMMARY" -eq 1 ]; then
              log_info "summary mode=$MODE executed_shards=$EXECUTED"
            fi

            log_ok "framework::test completed"
          '';
        };

        framework-install = mkCommandTask {
          id = "task.framework.install";
          appName = "framework::install";
          kind = "utility";
          summary = "Install thin wrapper flake";
          description = "Creates a thin wrapper flake by default, or vendored wrapper with --vendor.";
          runtimeInputs = [
            pkgs.coreutils
            pkgs.findutils
            pkgs.gnused
          ];
          usage = [
            "nix run .#framework::install"
            "nix run .#framework::install -- --vendor"
          ];
          command = ''
            set -euo pipefail

            source_root="${builtins.toString ../.}"
            target="."
            vendor=0

            while [ "$#" -gt 0 ]; do
              case "$1" in
                --vendor)
                  vendor=1
                  shift
                  ;;
                --target)
                  if [ "$#" -lt 2 ]; then
                    echo "ERROR: --target requires a value"
                    exit 2
                  fi
                  target="$2"
                  shift 2
                  ;;
                *)
                  echo "ERROR: unknown argument '$1'"
                  exit 2
                  ;;
              esac
            done

            mkdir -p "$target"

            if [ "$vendor" -eq 1 ]; then
              if [ -e "$target/nixfied" ]; then
                chmod -R u+w "$target/nixfied" 2>/dev/null || true
                rm -rf "$target/nixfied"
              fi
              mkdir -p "$target/nixfied"
              cp -R "$source_root/." "$target/nixfied"
              chmod -R u+w "$target/nixfied" 2>/dev/null || true
              rm -rf "$target/nixfied/.git"
              rm -f "$target/nixfied/result"
              cat > "$target/flake.nix" <<'NIXFIED_WRAPPER'
            ${vendoredWrapperFlake}
            NIXFIED_WRAPPER
              echo "OK: vendored wrapper flake generated at $target/flake.nix"
            else
              cat > "$target/flake.nix" <<'NIXFIED_WRAPPER'
            ${thinWrapperFlake}
            NIXFIED_WRAPPER
              echo "OK: thin wrapper flake generated at $target/flake.nix"
            fi
          '';
        };
      };

      workflows = {
        ci-basic = {
          id = "workflow.ci.basic";
          summary = "Basic CI workflow";
          description = "Runs quality and tests.";
          mode = "ci";
          maxWorkers = 4;
          units = {
            fmt = mkWorkflowUnit {
              taskId = "task.ci.fmt";
            };

            clippy = mkWorkflowUnit {
              taskId = "task.ci.clippy";
            };

            architecture = mkWorkflowUnit {
              taskId = "task.ci.architecture-verify";
            };

            shell-app-contracts = mkWorkflowUnit {
              taskId = "task.ci.shell-app-contracts";
            };

            tests = mkWorkflowUnit {
              taskId = "task.ci.tests";
              needs = [
                "fmt"
                "clippy"
                "architecture"
                "shell-app-contracts"
              ];
            };
          };
          stages = [ ];
          setup.tasks = [ ];
          teardown = {
            tasks = [ ];
            alwaysRun = true;
          };
          artifacts = {
            root = "/tmp/ci-artifacts";
            keepOnSuccess = false;
            keepOnFailure = true;
            writeSummary = true;
          };
          execution = {
            failFast = true;
            lockPolicy = "exclusive";
            emitRegistryEvents = true;
          };
        };

        ci-audit = {
          id = "workflow.ci.audit";
          summary = "Audit CI workflow";
          description = "Runs cargo-audit security checks.";
          mode = "ci";
          maxWorkers = 1;
          units = {
            audit = mkWorkflowUnit {
              taskId = "task.ci.audit";
            };
          };
          stages = [ ];
          setup.tasks = [ ];
          teardown = {
            tasks = [ ];
            alwaysRun = true;
          };
          artifacts = {
            root = "/tmp/ci-artifacts";
            keepOnSuccess = false;
            keepOnFailure = true;
            writeSummary = true;
          };
          execution = {
            failFast = true;
            lockPolicy = "exclusive";
            emitRegistryEvents = true;
          };
        };

        ci-parity = {
          id = "workflow.ci.parity";
          summary = "Parity CI workflow";
          description = "Runs parity compile/integration/audit steps.";
          mode = "ci";
          maxWorkers = 2;
          units = {
            parity-compile = mkWorkflowUnit {
              taskId = "task.ci.parity-compile";
            };

            parity-rest-api-smoke = mkWorkflowUnit {
              taskId = "task.ci.parity-rest-api-smoke";
              needs = [ "parity-compile" ];
            };

            parity-evm-helios-smoke = mkWorkflowUnit {
              taskId = "task.ci.parity-evm-helios-smoke";
              needs = [ "parity-compile" ];
            };

            parity-evm-reth = mkWorkflowUnit {
              taskId = "task.ci.parity-evm-reth";
              needs = [
                "parity-rest-api-smoke"
                "parity-evm-helios-smoke"
              ];
            };

            parity-aave-v3-reth = mkWorkflowUnit {
              taskId = "task.ci.parity-aave-v3-reth";
              needs = [ "parity-evm-reth" ];
            };

            parity-postgres-state-events-audit = mkWorkflowUnit {
              taskId = "task.ci.parity-postgres-state-events-audit";
              needs = [
                "parity-evm-reth"
                "parity-aave-v3-reth"
              ];
            };
          };
          stages = [ ];
          setup.tasks = [ ];
          teardown = {
            tasks = [ ];
            alwaysRun = true;
          };
          artifacts = {
            root = "/tmp/ci-artifacts";
            keepOnSuccess = false;
            keepOnFailure = true;
            writeSummary = true;
          };
          execution = {
            failFast = true;
            lockPolicy = "exclusive";
            emitRegistryEvents = true;
          };
        };

        ci-full = {
          id = "workflow.ci.full";
          summary = "Full CI workflow";
          description = "Runs basic checks/tests followed by the full parity stage sequence.";
          mode = "ci";
          maxWorkers = 2;
          units = {
            fmt = mkWorkflowUnit {
              taskId = "task.ci.fmt";
            };

            clippy = mkWorkflowUnit {
              taskId = "task.ci.clippy";
            };

            architecture = mkWorkflowUnit {
              taskId = "task.ci.architecture-verify";
            };

            shell-app-contracts = mkWorkflowUnit {
              taskId = "task.ci.shell-app-contracts";
            };

            tests = mkWorkflowUnit {
              taskId = "task.ci.tests";
              needs = [
                "fmt"
                "clippy"
                "architecture"
                "shell-app-contracts"
              ];
            };

            parity-compile = mkWorkflowUnit {
              taskId = "task.ci.parity-compile";
              needs = [ "tests" ];
            };

            parity-rest-api-smoke = mkWorkflowUnit {
              taskId = "task.ci.parity-rest-api-smoke";
              needs = [ "parity-compile" ];
            };

            parity-evm-helios-smoke = mkWorkflowUnit {
              taskId = "task.ci.parity-evm-helios-smoke";
              needs = [ "parity-compile" ];
            };

            parity-evm-reth = mkWorkflowUnit {
              taskId = "task.ci.parity-evm-reth";
              needs = [
                "parity-rest-api-smoke"
                "parity-evm-helios-smoke"
              ];
            };

            parity-aave-v3-reth = mkWorkflowUnit {
              taskId = "task.ci.parity-aave-v3-reth";
              needs = [ "parity-evm-reth" ];
            };

            parity-postgres-state-events-audit = mkWorkflowUnit {
              taskId = "task.ci.parity-postgres-state-events-audit";
              needs = [
                "parity-evm-reth"
                "parity-aave-v3-reth"
              ];
            };
          };
          stages = [ ];
          setup.tasks = [ ];
          teardown = {
            tasks = [ ];
            alwaysRun = true;
          };
          artifacts = {
            root = "/tmp/ci-artifacts";
            keepOnSuccess = false;
            keepOnFailure = true;
            writeSummary = true;
          };
          execution = {
            failFast = true;
            lockPolicy = "exclusive";
            emitRegistryEvents = true;
          };
        };

        ci-mainnet = {
          id = "workflow.ci.mainnet";
          summary = "Mainnet CI workflow";
          description = "Runs mainnet Helios-backed portfolio snapshot checks.";
          mode = "ci";
          maxWorkers = 1;
          units = {
            mainnet-portfolio-snapshot-helios = mkWorkflowUnit {
              taskId = "task.ci.mainnet-portfolio-snapshot-helios";
              skipIfMissingEnv = [ "MFM_CI_ENABLE_MAINNET" ];
            };
          };
          stages = [ ];
          setup.tasks = [ ];
          teardown = {
            tasks = [ ];
            alwaysRun = true;
          };
          artifacts = {
            root = "/tmp/ci-artifacts";
            keepOnSuccess = false;
            keepOnFailure = true;
            writeSummary = true;
          };
          execution = {
            failFast = true;
            lockPolicy = "exclusive";
            emitRegistryEvents = true;
          };
        };
      };
    };
  };
}
