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

  commonRuntimeInputs = [
    pkgs.bash
    pkgs.coreutils
    pkgs.findutils
    pkgs.gnused
    pkgs.gnugrep
    pkgs.jq
    pkgs.nix
  ]
  ++ (conf.tooling.runtimePackages or [ ]);

  solcPackage = if pkgs ? solc then pkgs.solc else null;

  mkContractArtifactProgram =
    {
      name,
      sourceFile,
      contractName,
    }:
    pkgs.writeShellScriptBin name ''
      set -euo pipefail

      find_workspace_root() {
        local dir="''${MFM_WORKSPACE_ROOT:-$PWD}"
        while [ "$dir" != "/" ]; do
          if [ -f "$dir/${sourceFile}" ]; then
            printf '%s' "$dir"
            return 0
          fi
          dir="$(dirname "$dir")"
        done
        echo "ERROR: unable to locate workspace root containing ${sourceFile}" >&2
        exit 3
      }

      workspace_root="$(find_workspace_root)"
      source_rel="${sourceFile}"
      source_key="${sourceFile}:${contractName}"
      solc_bin=${lib.escapeShellArg (if solcPackage != null then "${solcPackage}/bin/solc" else "")}

      if [ -z "$solc_bin" ] || [ ! -x "$solc_bin" ]; then
        echo "ERROR: solc compiler is unavailable in runtime" >&2
        exit 3
      fi

      compile_json="$(
        cd "$workspace_root"
        "$solc_bin" --combined-json abi,bin "$source_rel"
      )"
      abi_json="$(
        printf '%s' "$compile_json" \
          | ${pkgs.jq}/bin/jq -ce --arg key "$source_key" '
            .contracts[$key].abi
            | if type == "string" then fromjson else . end
          '
      )"
      bytecode_hex="$(
        printf '%s' "$compile_json" \
          | ${pkgs.jq}/bin/jq -re --arg key "$source_key" '.contracts[$key].bin'
      )"

      ${pkgs.jq}/bin/jq -cn --argjson abi "$abi_json" --arg bytecode "0x$bytecode_hex" \
        '{artifact: {abi: $abi, bytecode: $bytecode}}'
    '';

  configurableCounterArtifactProgram = mkContractArtifactProgram {
    name = "mfm-contract-artifact-configurable-counter";
    sourceFile = "contracts/src/ConfigurableCounter.sol";
    contractName = "ConfigurableCounter";
  };

  mockErc20ArtifactProgram = mkContractArtifactProgram {
    name = "mfm-contract-artifact-mock-erc20";
    sourceFile = "contracts/src/MockERC20.sol";
    contractName = "MockERC20";
  };

  rustRuntimeInputs = commonRuntimeInputs ++ [
    configurableCounterArtifactProgram
    mockErc20ArtifactProgram
  ];
  postgresPackage = conf.modules.postgres.package or pkgs.postgresql_16;
  minioPackage = conf.modules.minio.package or pkgs.minio;
  minioClientPackage = conf.modules.minio.clientPackage or pkgs.minio-client;
  rethPackage = conf.modules.reth.package or pkgs.reth;

  minioRootUser = conf.modules.minio.rootUser or "minio";
  minioRootPassword = conf.modules.minio.rootPassword or "minio123456";

  sharedPassThroughEnv = [
    "HOME"
    project.envVar
    project.slotVar
    "CI_ARTIFACTS_DIR"
    "CI_MAX_WORKERS"
    "NIXFIED_CI_MAX_WORKERS"
    "NIX_CFLAGS_COMPILE"
    "NIX_LDFLAGS"
    "LIBRARY_PATH"
    "CPATH"
    "SDKROOT"
    "MACOSX_DEPLOYMENT_TARGET"
    "API_KEY"
    "LOG_LEVEL"
    "OUTPUT_MODE"
    "RUSTFLAGS"
    "RUSTDOCFLAGS"
    "CARGO_BUILD_RUSTFLAGS"
    "CARGO_ENCODED_RUSTFLAGS"
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
    "DATABASE_URL"
    "MFM_EVM_RPC_URL"
    "MFM_S3_ENDPOINT"
    "MFM_S3_REGION"
    "MFM_S3_BUCKET"
    "MFM_S3_PREFIX"
    "AWS_ACCESS_KEY_ID"
    "AWS_SECRET_ACCESS_KEY"
    "AWS_REGION"
    "AWS_DEFAULT_REGION"
    "AWS_EC2_METADATA_DISABLED"
  ];

  sharedCargoRustEnv =
    {
      RUSTC_WRAPPER = "sccache";
      CARGO_PROFILE_CI_DEBUG = "0";
    }
    // lib.optionalAttrs pkgs.stdenv.isDarwin {
      LIBRARY_PATH = "${pkgs.libiconv}/lib";
    };

  cargoFmtCheckCmd = "cargo fmt --all -- --check";
  cargoClippyCmd = "cargo clippy --workspace --lib --examples --tests --benches --all-features -- -D warnings";
  cargoNextestCiCmd = "cargo nextest run --cargo-profile ci";
  cargoNextestWorkspaceCiCmd = "${cargoNextestCiCmd} --workspace";

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

  ciEnvOffsetCase = builtins.concatStringsSep "\n" (
    map (
      envName: "    ${envName}) env_offset=${toString (conf.envs.${envName}.offset or 0)} ;;"
    ) envNames
  );

  ciServicePortPrelude = ''
        slot_var=${lib.escapeShellArg project.slotVar}
        env_var=${lib.escapeShellArg project.envVar}
        slot_default=${toString conf.slots.default}
        env_default=${lib.escapeShellArg "dev"}

        slot_value="''${!slot_var:-$slot_default}"
        env_value="''${!env_var:-$env_default}"

        if ! [[ "$slot_value" =~ ^[0-9]+$ ]]; then
          echo "ERROR: $slot_var must be an integer"
          exit 3
        fi

        case "$env_value" in
    ${ciEnvOffsetCase}
          *)
            echo "ERROR: unsupported $env_var '$env_value'"
            exit 3
            ;;
        esac

        POSTGRES_PORT=$(( ${toString conf.ports.postgres} + env_offset + (slot_value * ${toString conf.slots.stride}) ))
        MINIO_API_PORT=$(( ${
          toString (conf.ports.minioApi or conf.ports.minio)
        } + env_offset + (slot_value * ${toString conf.slots.stride}) ))
        MINIO_CONSOLE_PORT=$(( ${
          toString (conf.ports.minioConsole or conf.ports.minio_console)
        } + env_offset + (slot_value * ${toString conf.slots.stride}) ))
        RETH_HTTP_PORT=$(( ${toString conf.ports.rethHttp} + env_offset + (slot_value * ${toString conf.slots.stride}) ))
        RETH_WS_PORT=$(( ${toString conf.ports.rethWs} + env_offset + (slot_value * ${toString conf.slots.stride}) ))
        RETH_AUTH_PORT=$(( ${toString conf.ports.rethAuth} + env_offset + (slot_value * ${toString conf.slots.stride}) ))
  '';

  ciParityServiceEnv = ''
    ${ciServicePortPrelude}

    export MFM_WORKSPACE_ROOT="''${MFM_WORKSPACE_ROOT:-$(pwd -P)}"
    export MFM_PARITY_EVM_RETH_RUN_IDS_PATH="''${MFM_PARITY_EVM_RETH_RUN_IDS_PATH:-$artifacts_dir/parity-evm-reth-run-ids.json}"
    export MFM_PARITY_AAVE_V3_RUN_IDS_PATH="''${MFM_PARITY_AAVE_V3_RUN_IDS_PATH:-$artifacts_dir/parity-aave-v3-run-ids.json}"
    export MFM_PARITY_AAVE_V3_RETH_PROBE_PATH="''${MFM_PARITY_AAVE_V3_RETH_PROBE_PATH:-$artifacts_dir/parity-aave-v3-reth-probe.json}"
    export DATABASE_URL="''${DATABASE_URL:-postgresql://postgres:postgres@127.0.0.1:$POSTGRES_PORT/mfm_test}"
    export MFM_EVM_RPC_URL="''${MFM_EVM_RPC_URL:-http://127.0.0.1:$RETH_HTTP_PORT}"
    export MFM_S3_ENDPOINT="''${MFM_S3_ENDPOINT:-http://127.0.0.1:$MINIO_API_PORT}"
    export MFM_S3_REGION="''${MFM_S3_REGION:-us-east-1}"
    export MFM_S3_BUCKET="''${MFM_S3_BUCKET:-mfm-test}"
    export MFM_S3_PREFIX="''${MFM_S3_PREFIX:-mfm-artifacts}"
    export AWS_ACCESS_KEY_ID="''${AWS_ACCESS_KEY_ID:-${minioRootUser}}"
    export AWS_SECRET_ACCESS_KEY="''${AWS_SECRET_ACCESS_KEY:-${minioRootPassword}}"
    export AWS_REGION="''${AWS_REGION:-''${MFM_S3_REGION}}"
    export AWS_DEFAULT_REGION="''${AWS_DEFAULT_REGION:-''${MFM_S3_REGION}}"
    export AWS_EC2_METADATA_DISABLED="''${AWS_EC2_METADATA_DISABLED:-true}"
  '';
  ciServicesRuntimeInputs = commonRuntimeInputs ++ [
    postgresPackage
    minioPackage
    minioClientPackage
    rethPackage
  ];

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
          enable = false;
          portKeyRpc = conf.modules.helios.portKeyRpc or "heliosRpc";
          executionRpcPortKey = conf.modules.helios.executionRpcPortKey or "rethHttp";
        };
      };

      operations = {
        enable = true;
        validateEnv.enable = true;
        testIsolation = {
          enable = conf.isolation.enable or true;
          slots =
            conf.isolation.slots or [
              conf.slots.default
            ];
          envs =
            let
              configured = conf.isolation.envs or [ ];
            in
            if configured == [ ] then envNames else configured;
          logsDir = conf.isolation.logsDir or "/tmp/${project.id}-isolation";
          keepLogsOnSuccess = conf.isolation.keepLogsOnSuccess or false;
          keepLogsOnFailure = conf.isolation.keepLogsOnFailure or true;
          maxParallel = conf.isolation.maxParallel or 4;
          runApp = conf.isolation.run.app or "ci";
          runArgs = conf.isolation.run.args or [ "--summary" ];
          validateApp = conf.isolation.validate.app or "validate-env";
          runEnv = conf.isolation.runEnv or { };
        };
        ports.enable = true;
        checkPorts.enable = true;
        health.enable = true;
        ready.enable = true;
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
          env = sharedCargoRustEnv;
          command = ''
            set -euo pipefail

            echo "INFO: running formatting checks"
            ${cargoFmtCheckCmd}

            echo "INFO: running clippy"
            ${cargoClippyCmd}

            echo "INFO: running architecture verifier"
            cargo run -p mfm-architecture-verify --

            echo "OK: quality checks completed"
          '';
        };

        format = mkCommandTask {
          id = "task.format";
          appName = "format";
          summary = "Format Rust and Nix files";
          usage = [ "nix run .#format" ];
          runtimeInputs = rustRuntimeInputs;
          command = ''
            set -euo pipefail

            cargo fmt --all
            find . -name '*.nix' -print0 | xargs -0 nixfmt --
            echo "OK: formatted rust and nix files"
          '';
        };

        test = mkCommandTask {
          id = "task.test";
          appName = "test";
          summary = "Run workspace tests";
          description = "Runs the full workspace test suite using cargo-nextest.";
          usage = [ "nix run .#test" ];
          runtimeInputs = rustRuntimeInputs;
          env = sharedCargoRustEnv;
          command = ''
            set -euo pipefail

            echo "INFO: running workspace tests"
            ${cargoNextestWorkspaceCiCmd}
            echo "OK: tests completed"
          '';
        };

        ci = mkCommandTask {
          id = "task.ci";
          appName = "ci";
          kind = "workflow";
          summary = "Run CI workflows (use --mode <mode>)";
          description = "Dispatches to model-derived CI workflows.";
          usage = [
            "nix run .#ci -- --mode basic --summary"
            "nix run .#ci -- --mode audit --summary"
            "nix run .#ci -- --mode parity --summary"
            "nix run .#ci -- --mode full --summary"
            "nix run .#ci -- --mode mainnet --summary"
          ];
          runtimeInputs = rustRuntimeInputs;
          env = sharedCargoRustEnv;
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
              type = "string";
              description = "CI mode to run (resolved from workflow.ci.* in the compiled model).";
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

        ci-workflow-basic =
          mkCommandTask {
            id = "task.ci.workflow-basic";
            appName = "ci-workflow-basic";
            kind = "internal";
            summary = "Run workflow.ci.basic";
            description = "Internal workflow reference used to compose workflow.ci.full.";
            runtimeInputs = rustRuntimeInputs;
            workflowId = "workflow.ci.basic";
          }
          // {
            ui.app.expose = false;
          };

        ci-workflow-parity =
          mkCommandTask {
            id = "task.ci.workflow-parity";
            appName = "ci-workflow-parity";
            kind = "internal";
            summary = "Run workflow.ci.parity";
            description = "Internal workflow reference used to compose workflow.ci.full.";
            runtimeInputs = rustRuntimeInputs;
            workflowId = "workflow.ci.parity";
          }
          // {
            ui.app.expose = false;
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
            env = sharedCargoRustEnv;
            command = ''
              set -euo pipefail
              ${ciStepPreamble}

              log_file="$artifacts_dir/fmt.log"
              echo "INFO: running ci step=fmt"
              run_with_log "$log_file" ${cargoFmtCheckCmd}
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
            env = sharedCargoRustEnv;
            command = ''
              set -euo pipefail
              ${ciStepPreamble}

              log_file="$artifacts_dir/clippy.log"
              echo "INFO: running ci step=clippy"
              run_with_log "$log_file" ${cargoClippyCmd}
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
            env = sharedCargoRustEnv;
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
                grep -q "id = \"task.ci.services-start\";" "$ROOT/nixfied/project/module.nix"
                grep -q "id = \"task.ci.services-stop\";" "$ROOT/nixfied/project/module.nix"
                grep -q "id = \"task.ci.workflow-basic\";" "$ROOT/nixfied/project/module.nix"
                grep -q "id = \"task.ci.workflow-parity\";" "$ROOT/nixfied/project/module.nix"
                grep -q "id = \"workflow.ci.full\";" "$ROOT/nixfied/project/module.nix"
                grep -q "taskId = \"task.ci.workflow-basic\";" "$ROOT/nixfied/project/module.nix"
                grep -q "taskId = \"task.ci.workflow-parity\";" "$ROOT/nixfied/project/module.nix"

                if grep -R -n "[.]framework/" "$ROOT/nixfied/project" --include='*.nix' >/dev/null; then
                  echo "ERROR: project layer references framework-private paths"
                  exit 1
                fi
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
            env = sharedCargoRustEnv;
            command = ''
              set -euo pipefail
              ${ciStepPreamble}

              log_file="$artifacts_dir/tests.log"
              echo "INFO: running ci step=tests"
              run_with_log "$log_file" ${cargoNextestWorkspaceCiCmd}
              echo "OK: ci step passed step=tests log=$log_file"
            '';
          }
          // {
            ui.app.expose = false;
          };

        ci-services-start =
          mkCommandTask {
            id = "task.ci.services-start";
            appName = "ci-services-start";
            kind = "ci-step";
            summary = "Start local CI parity services";
            description = "Boots local postgres/minio/reth dependencies for deterministic parity checks.";
            tags = [
              "ci"
              "parity"
              "services"
            ];
            runtimeInputs = ciServicesRuntimeInputs;
            command = ''
              set -euo pipefail
              ${ciStepPreamble}
              ${ciParityServiceEnv}

              services_root="$artifacts_dir/services"
              mkdir -p "$services_root"

              postgres_root="$services_root/postgres"
              postgres_data="$postgres_root/data"
              postgres_log="$artifacts_dir/postgres-service.log"
              mkdir -p "$postgres_data"

              if ! ${postgresPackage}/bin/pg_isready -U postgres -h 127.0.0.1 -p "$POSTGRES_PORT" -q 2>/dev/null; then
                if [ ! -f "$postgres_data/PG_VERSION" ]; then
                  ${postgresPackage}/bin/initdb -D "$postgres_data" -U postgres --no-locale --encoding=UTF8 -A trust >/dev/null
                  cat > "$postgres_data/pg_hba.conf" <<'EOF'
              # TYPE  DATABASE        USER  ADDRESS       METHOD
              local   all             all                 trust
              host    all             all   127.0.0.1/32  trust
              host    all             all   ::1/128       trust
              EOF
                fi

                if [ -f "$postgres_data/postmaster.pid" ]; then
                  stale_pid="$(head -1 "$postgres_data/postmaster.pid" 2>/dev/null || true)"
                  if [ -n "$stale_pid" ] && ! kill -0 "$stale_pid" 2>/dev/null; then
                    rm -f "$postgres_data/postmaster.pid"
                  fi
                fi

                ${postgresPackage}/bin/pg_ctl -D "$postgres_data" -l "$postgres_log" -o "-p $POSTGRES_PORT -h 127.0.0.1" start
              fi

              for _ in $(seq 1 120); do
                if ${postgresPackage}/bin/pg_isready -U postgres -h 127.0.0.1 -p "$POSTGRES_PORT" -q 2>/dev/null; then
                  break
                fi
                sleep 0.25
              done

              if ! ${postgresPackage}/bin/pg_isready -U postgres -h 127.0.0.1 -p "$POSTGRES_PORT" -q 2>/dev/null; then
                echo "ERROR: postgres failed to become ready port=$POSTGRES_PORT"
                exit 1
              fi

              ${postgresPackage}/bin/createdb -h 127.0.0.1 -p "$POSTGRES_PORT" -U postgres mfm >/dev/null 2>&1 || true
              ${postgresPackage}/bin/createdb -h 127.0.0.1 -p "$POSTGRES_PORT" -U postgres mfm_test >/dev/null 2>&1 || true

              minio_root="$services_root/minio"
              minio_data="$minio_root/data"
              minio_log="$artifacts_dir/minio-service.log"
              mkdir -p "$minio_data"

              if ! ${pkgs.curl}/bin/curl -fsS --max-time 2 "http://127.0.0.1:$MINIO_API_PORT/minio/health/ready" >/dev/null 2>&1; then
                export MINIO_ROOT_USER="$AWS_ACCESS_KEY_ID"
                export MINIO_ROOT_PASSWORD="$AWS_SECRET_ACCESS_KEY"
                ${minioPackage}/bin/minio server "$minio_data" \
                  --address "127.0.0.1:$MINIO_API_PORT" \
                  --console-address "127.0.0.1:$MINIO_CONSOLE_PORT" \
                  >"$minio_log" 2>&1 &
                echo "$!" > "$minio_root/minio.pid"
              fi

              for _ in $(seq 1 120); do
                if ${pkgs.curl}/bin/curl -fsS --max-time 2 "http://127.0.0.1:$MINIO_API_PORT/minio/health/ready" >/dev/null 2>&1; then
                  break
                fi
                sleep 0.25
              done

              if ! ${pkgs.curl}/bin/curl -fsS --max-time 2 "http://127.0.0.1:$MINIO_API_PORT/minio/health/ready" >/dev/null 2>&1; then
                echo "ERROR: minio failed to become ready port=$MINIO_API_PORT"
                if [ -f "$minio_log" ]; then
                  tail -50 "$minio_log" >&2 || true
                fi
                exit 1
              fi

              ${minioClientPackage}/bin/mc alias set ci "http://127.0.0.1:$MINIO_API_PORT" "$AWS_ACCESS_KEY_ID" "$AWS_SECRET_ACCESS_KEY" >/dev/null
              ${minioClientPackage}/bin/mc mb --ignore-existing "ci/$MFM_S3_BUCKET" >/dev/null

              reth_root="$services_root/reth"
              reth_data="$reth_root/data"
              reth_run="$reth_root/run"
              reth_log="$artifacts_dir/reth-service.log"
              mkdir -p "$reth_data" "$reth_run"

              reth_jwt="$reth_root/jwt.hex"
              if [ ! -f "$reth_jwt" ]; then
                printf '%064x\n' 0 > "$reth_jwt"
              fi
              chmod 600 "$reth_jwt" 2>/dev/null || true

              if ! ${pkgs.curl}/bin/curl -fsS --max-time 2 \
                -H 'content-type: application/json' \
                --data '{"jsonrpc":"2.0","id":1,"method":"web3_clientVersion","params":[]}' \
                "http://127.0.0.1:$RETH_HTTP_PORT" \
                | ${pkgs.gnugrep}/bin/grep -q '"result"'; then
                ${rethPackage}/bin/reth node \
                  --dev \
                  --datadir "$reth_data" \
                  --ipcpath "$reth_run/reth.ipc" \
                  --http \
                  --http.addr 127.0.0.1 \
                  --http.port "$RETH_HTTP_PORT" \
                  --ws \
                  --ws.addr 127.0.0.1 \
                  --ws.port "$RETH_WS_PORT" \
                  --authrpc.addr 127.0.0.1 \
                  --authrpc.port "$RETH_AUTH_PORT" \
                  --authrpc.jwtsecret "$reth_jwt" \
                  >"$reth_log" 2>&1 &
                echo "$!" > "$reth_root/reth.pid"
              fi

              for _ in $(seq 1 160); do
                if ${pkgs.curl}/bin/curl -fsS --max-time 2 \
                  -H 'content-type: application/json' \
                  --data '{"jsonrpc":"2.0","id":1,"method":"web3_clientVersion","params":[]}' \
                  "http://127.0.0.1:$RETH_HTTP_PORT" \
                  | ${pkgs.gnugrep}/bin/grep -q '"result"'; then
                  break
                fi
                sleep 0.25
              done

              if ! ${pkgs.curl}/bin/curl -fsS --max-time 2 \
                -H 'content-type: application/json' \
                --data '{"jsonrpc":"2.0","id":1,"method":"web3_clientVersion","params":[]}' \
                "http://127.0.0.1:$RETH_HTTP_PORT" \
                | ${pkgs.gnugrep}/bin/grep -q '"result"'; then
                echo "ERROR: reth failed to become ready port=$RETH_HTTP_PORT"
                if [ -f "$reth_log" ]; then
                  tail -50 "$reth_log" >&2 || true
                fi
                exit 1
              fi

              echo "OK: ci services ready postgres=$POSTGRES_PORT minio=$MINIO_API_PORT reth=$RETH_HTTP_PORT"
            '';
          }
          // {
            ui.app.expose = false;
          };

        ci-services-stop =
          mkCommandTask {
            id = "task.ci.services-stop";
            appName = "ci-services-stop";
            kind = "ci-step";
            summary = "Stop local CI parity services";
            description = "Stops local postgres/minio/reth service processes started for CI.";
            tags = [
              "ci"
              "parity"
              "services"
            ];
            runtimeInputs = ciServicesRuntimeInputs;
            command = ''
              set -euo pipefail
              ${ciStepPreamble}

              services_root="$artifacts_dir/services"
              postgres_data="$services_root/postgres/data"
              minio_pid_file="$services_root/minio/minio.pid"
              reth_pid_file="$services_root/reth/reth.pid"

              if [ -f "$reth_pid_file" ]; then
                reth_pid="$(cat "$reth_pid_file" 2>/dev/null || true)"
                if [ -n "$reth_pid" ] && kill -0 "$reth_pid" 2>/dev/null; then
                  kill "$reth_pid" 2>/dev/null || true
                  for _ in $(seq 1 40); do
                    if ! kill -0 "$reth_pid" 2>/dev/null; then
                      break
                    fi
                    sleep 0.25
                  done
                  kill -KILL "$reth_pid" 2>/dev/null || true
                fi
                rm -f "$reth_pid_file"
              fi

              if [ -f "$minio_pid_file" ]; then
                minio_pid="$(cat "$minio_pid_file" 2>/dev/null || true)"
                if [ -n "$minio_pid" ] && kill -0 "$minio_pid" 2>/dev/null; then
                  kill "$minio_pid" 2>/dev/null || true
                  for _ in $(seq 1 40); do
                    if ! kill -0 "$minio_pid" 2>/dev/null; then
                      break
                    fi
                    sleep 0.25
                  done
                  kill -KILL "$minio_pid" 2>/dev/null || true
                fi
                rm -f "$minio_pid_file"
              fi

              if [ -f "$postgres_data/postmaster.pid" ]; then
                ${postgresPackage}/bin/pg_ctl -D "$postgres_data" stop -m fast >/dev/null 2>&1 || true
              fi

              echo "OK: ci services stopped"
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
            env = sharedCargoRustEnv;
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
            env = sharedCargoRustEnv;
            command = ''
              set -euo pipefail
              ${ciStepPreamble}

              log_file="$artifacts_dir/parity-compile.log"
              echo "INFO: running ci step=parity-compile"
              run_with_log "$log_file" ${cargoNextestCiCmd} --no-run -p mfm-integration-tests --features parity-tests -p mfm --features parity-tests
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
            env = sharedCargoRustEnv;
            command = ''
              set -euo pipefail
              ${ciStepPreamble}
              ${ciParityServiceEnv}

              log_file="$artifacts_dir/parity-rest-api-smoke.log"
              echo "INFO: running ci step=parity-rest-api-smoke"
              run_with_log "$log_file" ${cargoNextestCiCmd} --jobs 1 -p mfm-integration-tests --features parity-tests --test parity_event_store_postgres_contract --test parity_artifact_store_s3_contract --test parity_rest_api_postgres_s3_smoke
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
            env = sharedCargoRustEnv;
            command = ''
              set -euo pipefail
              ${ciStepPreamble}
              ${ciParityServiceEnv}

              key_log_file="$artifacts_dir/parity-keystore-reth-tx-sign-send.log"
              smoke_log_file="$artifacts_dir/parity-evm-helios-smoke.log"
              response_file="$artifacts_dir/parity-evm-helios-smoke.response.json"

              echo "INFO: running ci step=parity-evm-helios-smoke"
              run_with_log "$key_log_file" ${cargoNextestCiCmd} -p mfm --features parity-tests --test parity_keystore_reth_tx_send

              if [ -z "''${MFM_EVM_RPC_URL:-}" ]; then
                echo "SKIP: MFM_EVM_RPC_URL is unset; skipping helios RPC curl probe"
                exit 0
              fi

              run_with_log "$smoke_log_file" bash -euo pipefail -c '
                response_file="$1"
                rpc_url="$2"
                curl -fsS \
                  -H "content-type: application/json" \
                  --data "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"eth_chainId\",\"params\":[]}" \
                  "$rpc_url" \
                  | tee "$response_file"
              ' _ "$response_file" "$MFM_EVM_RPC_URL"

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
            env = sharedCargoRustEnv;
            command = ''
              set -euo pipefail
              ${ciStepPreamble}
              ${ciParityServiceEnv}

              log_file="$artifacts_dir/parity-evm-reth.log"
              echo "INFO: running ci step=parity-evm-reth"
              run_with_log "$log_file" ${cargoNextestCiCmd} --jobs 1 -p mfm-integration-tests --features parity-tests --test evm_rpc_pool_failover --test evm_rpc_getlogs_chunking --test parity_rest_api_evm_reth_pipeline --test parity_portfolio_tracker_reth_mock_erc20 --test parity_portfolio_tracker_reth_snapshot
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
            env = sharedCargoRustEnv;
            command = ''
              set -euo pipefail
              ${ciStepPreamble}
              ${ciParityServiceEnv}

              log_file="$artifacts_dir/parity-aave-v3-reth.log"
              echo "INFO: running ci step=parity-aave-v3-reth"
              run_with_log "$log_file" ${cargoNextestCiCmd} -p mfm-integration-tests --features parity-tests --test parity_aave_v3_reth_scenario
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
            env = sharedCargoRustEnv;
            command = ''
              set -euo pipefail
              ${ciStepPreamble}
              ${ciParityServiceEnv}

              log_file="$artifacts_dir/parity-postgres-state-events-audit.log"
              echo "INFO: running ci step=parity-postgres-state-events-audit"
              run_with_log "$log_file" ${cargoNextestCiCmd} -p mfm-integration-tests --features parity-tests --test parity_postgres_state_events_audit
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
            env = sharedCargoRustEnv;
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
                "workflow-mode-valid"
                "workflow-mode-invalid"
                "workflow-mode-dotted-shorthand"
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
              type = "string";
              description = "CI workflow mode used by the workflow-ci shard.";
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
              "workflow-mode-valid"
              "workflow-mode-invalid"
              "workflow-mode-dotted-shorthand"
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
            Usage: nix run .#framework::test [-- --mode <mode>] [--summary] [--summary-json <path>] [--shard <name>] [--list-shards]

            Shards:
              flake-check   Run nix flake check for the current project root.
              help          Validate generated help output.
              workflow-test Run the test app surface.
              workflow-ci   Run the ci app surface in selected mode.
              workflow-mode-valid             Validate a model-derived CI mode via --mode.
              workflow-mode-invalid           Validate unknown mode handling and expected-mode output.
              workflow-mode-dotted-shorthand Validate dotted shorthand flags are rejected.
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

            list_ci_modes() {
              if [ -n "''${CI_MODES_CACHE:-}" ]; then
                printf '%s\n' "$CI_MODES_CACHE"
                return 0
              fi

              CI_MODES_CACHE="$(
                nix run "path:$ROOT"#help \
                  | sed -n 's/^  workflow\.ci\.\([a-z0-9.-]\+\) - .*$/\1/p'
              )"
              if [ -z "$CI_MODES_CACHE" ]; then
                log_error "unable to derive workflow.ci modes from model-generated help output"
                return 1
              fi

              printf '%s\n' "$CI_MODES_CACHE"
            }

            ci_mode_choices() {
              local mode_name expected=""
              while IFS= read -r mode_name; do
                if [ -z "$mode_name" ]; then
                  continue
                fi
                if [ -z "$expected" ]; then
                  expected="$mode_name"
                else
                  expected="$expected|$mode_name"
                fi
              done < <(list_ci_modes)
              printf '%s' "$expected"
            }

            mode_exists() {
              local candidate="$1"
              local mode_name
              while IFS= read -r mode_name; do
                if [ "$candidate" = "$mode_name" ]; then
                  return 0
                fi
              done < <(list_ci_modes)
              return 1
            }

            first_ci_mode() {
              if mode_exists "mainnet"; then
                printf '%s\n' "mainnet"
                return 0
              fi
              if mode_exists "basic"; then
                printf '%s\n' "basic"
                return 0
              fi
              list_ci_modes | head -n 1
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
              local rc
              rc=$?
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

            shard_workflow_mode_valid() {
              local derived_mode
              derived_mode="$(first_ci_mode)"
              if [ -z "$derived_mode" ]; then
                log_error "no model-derived workflow.ci mode found"
                return 1
              fi
              nix run "path:$ROOT"#ci -- --mode "$derived_mode" --summary
            }

            shard_workflow_mode_invalid() {
              local output rc expected_mode
              expected_mode="$(first_ci_mode)"
              if [ -z "$expected_mode" ]; then
                log_error "no model-derived workflow.ci mode found"
                return 1
              fi

              set +e
              output="$(nix run "path:$ROOT"#ci -- --mode "__invalid_mode__" --summary 2>&1)"
              rc=$?
              set -e

              if [ "$rc" -eq 0 ]; then
                log_error "expected invalid mode command to fail"
                return 1
              fi

              printf '%s' "$output" | grep -F "unknown mode '__invalid_mode__'" >/dev/null || {
                log_error "invalid mode output missing unknown-mode marker"
                printf '%s\n' "$output" >&2
                return 1
              }
              printf '%s' "$output" | grep -F "expected:" >/dev/null || {
                log_error "invalid mode output missing expected-mode marker"
                printf '%s\n' "$output" >&2
                return 1
              }
              printf '%s' "$output" | grep -F "$expected_mode" >/dev/null || {
                log_error "invalid mode output missing derived mode '$expected_mode'"
                printf '%s\n' "$output" >&2
                return 1
              }
            }

            shard_workflow_mode_dotted_shorthand() {
              local derived_mode dotted_mode output rc
              derived_mode="$(first_ci_mode)"
              if [ -z "$derived_mode" ]; then
                log_error "no model-derived workflow.ci mode found"
                return 1
              fi
              dotted_mode="$derived_mode.probe"

              set +e
              output="$(nix run "path:$ROOT"#ci -- "--$dotted_mode" --summary 2>&1)"
              rc=$?
              set -e

              if [ "$rc" -eq 0 ]; then
                log_error "expected dotted shorthand '--$dotted_mode' to fail"
                return 1
              fi

              printf '%s' "$output" | grep -F "unknown option '--$dotted_mode'" >/dev/null || {
                log_error "dotted shorthand rejection output missing expected marker"
                printf '%s\n' "$output" >&2
                return 1
              }
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
                workflow-mode-valid)
                  run_shard "$shard_name" shard_workflow_mode_valid
                  ;;
                workflow-mode-invalid)
                  run_shard "$shard_name" shard_workflow_mode_invalid
                  ;;
                workflow-mode-dotted-shorthand)
                  run_shard "$shard_name" shard_workflow_mode_dotted_shorthand
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

            if ! mode_exists "$MODE"; then
              expected_modes="$(ci_mode_choices)"
              if [ -n "$expected_modes" ]; then
                log_error "unknown mode '$MODE' (expected: $expected_modes)"
              else
                log_error "unknown mode '$MODE'"
              fi
              exit 2
            fi

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
          preRun.tasks = [ ];
          postRun = {
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
            parallel = true;
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
          preRun.tasks = [ ];
          postRun = {
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
            parallel = true;
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
          preRun.tasks = [
            "task.ci.services-start"
            "task.ops.ready"
            "task.ops.health"
          ];
          postRun = {
            tasks = [ "task.ci.services-stop" ];
            alwaysRun = true;
          };
          artifacts = {
            root = "/tmp/ci-artifacts";
            keepOnSuccess = false;
            keepOnFailure = true;
            writeSummary = true;
          };
          execution = {
            parallel = true;
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
            basic = mkWorkflowUnit {
              taskId = "task.ci.workflow-basic";
            };

            parity = mkWorkflowUnit {
              taskId = "task.ci.workflow-parity";
              needs = [ "basic" ];
            };
          };
          stages = [ ];
          preRun.tasks = [ ];
          postRun = {
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
            parallel = true;
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
          preRun.tasks = [ ];
          postRun = {
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
            parallel = true;
            failFast = true;
            lockPolicy = "exclusive";
            emitRegistryEvents = true;
          };
        };
      };
    };
  };
}
