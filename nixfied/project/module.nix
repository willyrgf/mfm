{
  lib,
  pkgs,
  frameworkSourceRevision,
  ...
}:
let
  conf = import ./conf.nix { inherit pkgs; };
  project = conf.project;
  ciRuntime = import ./ci-runtime.nix {
    inherit
      lib
      conf
      project
      ;
  };
  skipPolicy = import ../framework/runtime/helpers/skip-policy.nix { inherit pkgs; };
  servicePolicy = import ../framework/runtime/helpers/service-policy.nix { inherit pkgs; };

  envNames = builtins.attrNames conf.envs;
  envOffsets = lib.mapAttrs (_: value: value.offset or 0) conf.envs;

  coreRuntimeInputs = [
    pkgs.bash
    pkgs.coreutils
    pkgs.findutils
    pkgs.gnused
    pkgs.gnugrep
    pkgs.jq
    pkgs.nix
  ];

  commonRuntimeInputs = coreRuntimeInputs ++ (conf.tooling.runtimePackages or [ ]);

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
  leanRuntimeInputs = coreRuntimeInputs;
  configuredServices = conf.services or { };
  postgresService = configuredServices.postgres or { };
  nginxService = configuredServices.nginx or { };
  minioService = configuredServices.minio or { };
  rethService = configuredServices.reth or { };
  heliosService = configuredServices.helios or { };
  postgresSources = postgresService.sources or { };
  nginxSources = nginxService.sources or { };
  minioSources = minioService.sources or { };
  rethSources = rethService.sources or { };
  heliosSources = heliosService.sources or { };
  postgresLocalSource = postgresSources.local or { };
  minioLocalSource = minioSources.local or { };
  rethLocalSource = rethSources.local or { };
  postgresPackage = postgresLocalSource.package or pkgs.postgresql_16;
  minioPackage = minioLocalSource.package or pkgs.minio;
  minioClientPackage = minioLocalSource.clientPackage or pkgs.minio-client;
  rethPackage = rethLocalSource.package or pkgs.reth;
  postgresDatabase = postgresService.database or "mfm";
  postgresTestDatabase = postgresService.testDatabase or "${postgresDatabase}_test";
  minioRootUser = minioService.rootUser or "minio";
  minioRootPassword = minioService.rootPassword or "minio123456";
  heliosConfiguredNetwork = heliosService.network or "local";
  heliosConfiguredExecutionRpcUrl = heliosService.executionRpcUrl or "";
  heliosConfiguredConsensusRpcUrl = heliosService.consensusRpcUrl or "";
  heliosConfiguredDefaultConsensusRpcUrl =
    heliosService.defaultConsensusRpcUrl or heliosConfiguredConsensusRpcUrl;
  heliosConfiguredCheckpoint = heliosService.checkpoint or "";

  serviceSkipEnvVarName =
    serviceName:
    let
      safeServiceName = lib.toUpper (lib.replaceStrings [ "." "-" ] [ "_" "_" ] serviceName);
    in
    "SKIP_${safeServiceName}";

  normalizeSkipEnvValue =
    value: lib.toLower (builtins.replaceStrings [ " " "\n" "\r" "\t" ] [ "" "" "" "" ] value);

  isTruthySkipEnvValue =
    value:
    builtins.elem (normalizeSkipEnvValue value) [
      "1"
      "true"
      "yes"
      "on"
    ];

  excludedServices = builtins.sort builtins.lessThan (
    builtins.filter (
      serviceName: isTruthySkipEnvValue (builtins.getEnv (serviceSkipEnvVarName serviceName))
    ) (builtins.attrNames conf.services)
  );

  # Auto-compute SKIP_<SERVICE> env vars from service config (mirrors operations.nix:106-114)
  serviceSkipEnvVars = lib.unique (
    builtins.map serviceSkipEnvVarName (builtins.attrNames conf.services)
  );

  sharedPassThroughEnv = [
    project.envVar
    project.slotVar
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
    "RUST_LOG"
    "MFM_LOG"
    "LOG_FORMAT"
    "MFM_LOG_FORMAT"
    "LOG_SPAN_EVENTS"
    "MFM_LOG_SPAN_EVENTS"
    "HELIOS_NETWORK"
    "HELIOS_BIN"
    "HELIOS_EXECUTION_RPC_URL"
    "HELIOS_CONSENSUS_RPC_URL"
    "HELIOS_CHECKPOINT"
    "HELIOS_READY_TIMEOUT_SECS"
    "HELIOS_HEALTH_TIMEOUT_SECS"
    "HELIOS_READY_INTERVAL_SECS"
    "HELIOS_HEALTH_INTERVAL_SECS"
    "SERVICE_OWNER_SCOPE"
    "SERVICE_DISCOVERY_SCOPE"
    "MFM_SNAPSHOT_REQUEST_FILE"
    "MFM_SNAPSHOT_HANDOFF_FILE"
    "MFM_SNAPSHOT_RESULT_FILE"
    "MFM_CI_ENABLE_PARITY"
    "MFM_CI_ENABLE_MAINNET"
    "DATABASE_URL"
    "MFM_EVM_RPC_SOURCES_JSON"
    "MFM_EVM_RPC_PREFERRED_ORDER"
    "MFM_EVM_RPC_REQUIRE_GET_PROOF_IDS"
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
  ]
  ++ serviceSkipEnvVars;

  sharedCargoRustEnv = {
    RUSTC_WRAPPER = "sccache";
  }
  // lib.optionalAttrs pkgs.stdenv.isDarwin {
    LIBRARY_PATH = "${pkgs.libiconv}/lib";
    CC = "/usr/bin/clang";
    CXX = "/usr/bin/clang++";
  };

  # CI runs under ephemeral roots, so persistent sccache state can retain stale
  # temp paths across repeated runs.
  ciCargoRustEnv = (builtins.removeAttrs sharedCargoRustEnv [ "RUSTC_WRAPPER" ]) // {
    CARGO_BUILD_JOBS = "1";
  };
  ciArtifactsRoot = conf.process.artifactsRoot or "/tmp/nixfied-artifacts-${project.id}";
  ciShellAppContractsTimeoutSec = 300;
  sharedStatePolicy = {
    workspace = {
      mode = "literal";
      value = project.id;
    };
    roots = {
      runtimeBase = conf.directories.base;
      registryRoot = conf.process.registryRoot;
      artifactsRoot = ciArtifactsRoot;
    };
  };
  ephemeralRuntimeConfig =
    let
      runtimeEphemeral = conf.ephemeral or { };
    in
    if builtins.isAttrs runtimeEphemeral then
      {
        copyMode = runtimeEphemeral.copyMode or "nix-source";
        includeUntracked = runtimeEphemeral.includeUntracked or false;
        excludePatterns =
          runtimeEphemeral.excludePatterns or [
            ".git"
            "node_modules"
            ".next"
            "dist"
            ".turbo"
            ".cache"
            "target"
            "result"
            "result-*"
            "*.log"
            "test-results"
            "coverage"
          ];
        extraDirs = runtimeEphemeral.extraDirs or [ ];
        keepFailures = runtimeEphemeral.keepFailures or true;
        maxFailedRoots = runtimeEphemeral.maxFailedRoots or 8;
        maxFailedRootAgeHours = runtimeEphemeral.maxFailedRootAgeHours or 72;
        maxCopyBytes = runtimeEphemeral.maxCopyBytes or 0;
        minFreeBytesAfterCopy = runtimeEphemeral.minFreeBytesAfterCopy or 0;
        envFileMode = runtimeEphemeral.envFileMode or "disabled";
        envFilePath = runtimeEphemeral.envFilePath or ".env";
      }
    else
      {
        copyMode = "nix-source";
        includeUntracked = false;
        excludePatterns = [
          ".git"
          "node_modules"
          ".next"
          "dist"
          ".turbo"
          ".cache"
          "target"
          "result"
          "result-*"
          "*.log"
          "test-results"
          "coverage"
        ];
        extraDirs = [ ];
        keepFailures = true;
        maxFailedRoots = 8;
        maxFailedRootAgeHours = 72;
        maxCopyBytes = 0;
        minFreeBytesAfterCopy = 0;
        envFileMode = "disabled";
        envFilePath = ".env";
      };

  cargoFmtCheckCmd = "cargo fmt --all -- --check";
  cargoClippyCmd = "cargo clippy --workspace --lib --examples --tests --benches --all-features -- -D warnings";
  # Keep CI linting on the same Cargo profile as nextest to avoid profile drift
  # within .#ci. Clippy still uses its own driver, so reuse remains partial.
  cargoCiClippyCmd = "cargo clippy --profile ci --workspace --lib --examples --tests --benches --all-features -- -D warnings";
  cargoNextestCiCmd = "cargo nextest run --cargo-profile ci";
  cargoNextestWorkspaceCiCmd = "${cargoNextestCiCmd} --workspace";
  parityNextestArgs = "-p mfm-integration-tests --features parity-tests -p mfm --features parity-tests";
  parityNextestCmd = "${cargoNextestCiCmd} ${parityNextestArgs}";

  ciStepPreamble = ''
    artifacts_dir="''${CI_ARTIFACTS_DIR:-${ciArtifactsRoot}}"
    mkdir -p "$artifacts_dir"

    # Keep Cargo artifacts outside the workspace root so flake/model
    # evaluation does not trip over mutable target/ files.
    run_id_component="''${NIXFIED_ORCHESTRATOR_RUN_ID:-''${NIXFIED_RUN_ID:-''${NIX_ENV:-0}}}"
    run_id_component="$(printf '%s' "$run_id_component" | tr './:' '__')"
    cache_root="''${TMPDIR:-/tmp}/mfm-ci-cache/$run_id_component"
    # Share Cargo cache per CI run to avoid rebuilding identical crates/tests.
    export CARGO_HOME="''${CARGO_HOME:-$cache_root/cargo-home}"
    mkdir -p "$CARGO_HOME"
    export CARGO_TARGET_DIR="''${CARGO_TARGET_DIR:-$cache_root/cargo-target}"
    mkdir -p "$CARGO_TARGET_DIR"

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

  cargoWorkspaceTargetPreamble = ''
    # Task apps execute from the flake source under /nix/store, so Cargo outputs
    # must be redirected into a writable per-run location.
    export CARGO_TARGET_DIR="''${CARGO_TARGET_DIR:-''${CI_ARTIFACTS_DIR:-''${TMPDIR:-/tmp}/mfm-task-artifacts}/cargo-target}"
    mkdir -p "$CARGO_TARGET_DIR"
  '';

  ciServicePortPrelude = ciRuntime.servicePortPrelude;
  resolvePackagedMfmCliShell = ''
    resolve_packaged_mfm_cli_binary() {
      local packaged_mfm_cli_binary='${conf.packages."mfm-cli"}/bin/mfm_cli'
      if [ -x "$packaged_mfm_cli_binary" ]; then
        printf '%s' "$packaged_mfm_cli_binary"
        return 0
      fi

      echo "ERROR: packaged mfm_cli binary not present at packaged mfm-cli path=$packaged_mfm_cli_binary" >&2
      echo "INFO: ensure '#mfm-cli' is built in the current flake before running snapshot workflow" >&2
      return 1
    }
  '';

  ciParityServiceEnv = ''
    ${ciServicePortPrelude}

    export MFM_WORKSPACE_ROOT="''${MFM_WORKSPACE_ROOT:-$(pwd -P)}"
    export MFM_PARITY_EVM_RETH_RUN_IDS_PATH="''${MFM_PARITY_EVM_RETH_RUN_IDS_PATH:-$artifacts_dir/parity-evm-reth-run-ids.json}"
    export MFM_PARITY_AAVE_V3_RUN_IDS_PATH="''${MFM_PARITY_AAVE_V3_RUN_IDS_PATH:-$artifacts_dir/parity-aave-v3-run-ids.json}"
    export MFM_PARITY_AAVE_V3_RETH_PROBE_PATH="''${MFM_PARITY_AAVE_V3_RETH_PROBE_PATH:-$artifacts_dir/parity-aave-v3-reth-probe.json}"
    export PGDATABASE="''${PGDATABASE:-${postgresTestDatabase}}"
    export DATABASE_URL="''${DATABASE_URL:-postgresql://postgres:postgres@127.0.0.1:$POSTGRES_PORT/$PGDATABASE}"
    export MFM_EVM_RPC_URL="http://127.0.0.1:$RETH_HTTP_PORT"
    export MFM_EVM_RPC_SOURCES_JSON="[{\"id\":\"reth_ethereum_mainnet\",\"network_id\":\"ethereum-mainnet\",\"rpc_url\":\"http://127.0.0.1:$RETH_HTTP_PORT\",\"kind\":\"local\"},{\"id\":\"reth_local\",\"network_id\":\"reth-local\",\"rpc_url\":\"http://127.0.0.1:$RETH_HTTP_PORT\",\"kind\":\"local\"}]"
    export MFM_EVM_RPC_PREFERRED_ORDER="reth_ethereum_mainnet,reth_local"
    export MFM_EVM_RPC_REQUIRE_GET_PROOF_IDS=""
    export MFM_S3_ENDPOINT="''${MFM_S3_ENDPOINT:-http://127.0.0.1:$MINIO_API_PORT}"
    export MFM_S3_REGION="''${MFM_S3_REGION:-us-east-1}"
    export MFM_S3_BUCKET="''${MFM_S3_BUCKET:-mfm-test}"
    export MFM_S3_PREFIX="''${MFM_S3_PREFIX:-mfm-artifacts}"
    export AWS_ACCESS_KEY_ID="''${AWS_ACCESS_KEY_ID:-${minioRootUser}}"
    export AWS_SECRET_ACCESS_KEY="''${AWS_SECRET_ACCESS_KEY:-${minioRootPassword}}"
    export AWS_REGION="''${AWS_REGION:-''${MFM_S3_REGION}}"
    export AWS_DEFAULT_REGION="''${AWS_DEFAULT_REGION:-''${MFM_S3_REGION}}"
    export AWS_EC2_METADATA_DISABLED="''${AWS_EC2_METADATA_DISABLED:-true}"
    # Parity always uses the local Helios<->Reth pair; clear explicit mainnet
    # overrides that may be auto-loaded from `.env` for snapshot workflows.
    export HELIOS_NETWORK="local"
    export HELIOS_EXECUTION_RPC_URL=""
    export HELIOS_CONSENSUS_RPC_URL=""
    export HELIOS_CHECKPOINT=""
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
      ownerFile ? null,
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
      argParser ? "typed",
      allowUnknownArgs ? true,
      outputFormat ? "text",
      outputChannels ? "stdout",
      effects ? [ "writes-state" ],
      idempotent ? false,
      requirements ? {
        services = [ ];
      },
      passThroughEnv ? sharedPassThroughEnv,
      allowSensitivePassThrough ? true,
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

      requirements = requirements // {
        services = lib.unique (requirements.services or [ ]);
      };

      runner =
        if workflowId == null then
          {
            type = "shell";
            command = skipPolicy.skipPolicyFunctions + "\n" + command;
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
            parser = argParser;
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
        allowSensitivePassThrough = allowSensitivePassThrough;
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
    };

  mkTaskApp =
    {
      taskId,
      appId,
      category ? "core",
      usage ? [ "nix run .#${appId}" ],
      examples ? [ ],
      ownerFile ? null,
    }:
    {
      id = appId;
      kind = "taskRef";
      inherit
        taskId
        category
        usage
        examples
        ownerFile
        ;
    };

  mkWorkflowUnit =
    {
      taskId,
      needs ? [ ],
      skipIfMissingEnv ? [ ],
      requirements ? {
        services = [ ];
      },
    }:
    {
      inherit
        taskId
        needs
        skipIfMissingEnv
        ;
      locks = [ ];
      requirements = requirements // {
        services = lib.unique (requirements.services or [ ]);
      };
      when = {
        envEquals = { };
        envPresent = [ ];
      };
    };

  frameworkInstallPreset = import ../framework/presets/install.nix {
    inherit
      mkCommandTask
      mkTaskApp
      pkgs
      frameworkSourceRevision
      ;
  };
  frameworkTestPreset = import ../framework/presets/framework-test.nix {
    inherit
      lib
      pkgs
      conf
      mkCommandTask
      mkTaskApp
      ;
  };
  frameworkSelfhostPreset = import ../framework/presets/selfhost.nix {
    inherit
      mkCommandTask
      commonRuntimeInputs
      ;
  };
in
{
  imports = [
    ../modules/profiles/webapp.nix
    ../framework/presets/state-policies/project-shared.nix
    ../local/default.nix
  ];

  config = {
    nixfied = {
      identity = {
        projectId = project.id;
        projectName = project.name;
        description = project.description;
      };

      graph.excludedServices = excludedServices;

      runtime = {
        slot = {
          var = project.slotVar;
          default = conf.slots.default;
          max = conf.slots.max;
          stride = conf.slots.stride;
        };
        ephemeral = ephemeralRuntimeConfig;

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
      };

      state.policy = sharedStatePolicy;

      serviceSets = {
        ci-parity = {
          id = "service-set.ci-parity";
          summary = "CI parity services";
          description = "Managed postgres, minio, and reth instances used by parity CI workflows.";
          services.required = [
            "postgres"
            "minio"
            "reth"
          ];
          failureLogs = {
            capture = true;
            tailLines = 50;
          };
          ownerFile = "nixfied/project/module.nix";
        };
      };

      tooling = {
        runtimePackages = conf.tooling.runtimePackages;
        devShellPackages = conf.tooling.devShellPackages;
        devShellHook = conf.tooling.devShellHook;
      };

      packages = lib.removeAttrs conf.packages [ "helios" ];

      services = {
        postgres = {
          enable = postgresService.enable or false;
          database = postgresDatabase;
          testDatabase = postgresTestDatabase;
          portKey = postgresService.portKey or "postgres";
          dataDirName = postgresService.dataDirName or "postgres";
          extensions = postgresService.extensions or [ ];
          extraConfig = postgresService.extraConfig or "";
          envConfigs = postgresService.envConfigs or { };
          migrations = postgresService.migrations or { };
          sources = postgresSources;
          sourceKeys = postgresService.sourceKeys or builtins.attrNames postgresSources;
          defaultSource = postgresService.defaultSource or "local";
        };

        nginx = {
          enable = nginxService.enable or false;
          portKeyHttp = nginxService.portKeyHttp or "http";
          portKeyHttps = nginxService.portKeyHttps or "https";
          dataDirName = nginxService.dataDirName or "nginx";
          sources = nginxSources;
          sourceKeys = nginxService.sourceKeys or builtins.attrNames nginxSources;
          defaultSource = nginxService.defaultSource or "local";
        };

        minio = {
          enable = minioService.enable or false;
          portKeyApi = minioService.portKeyApi or "minioApi";
          portKeyConsole = minioService.portKeyConsole or "minioConsole";
          dataDirName = minioService.dataDirName or "minio";
          rootUser = minioRootUser;
          rootPassword = minioRootPassword;
          browser = minioService.browser or true;
          sources = minioSources;
          sourceKeys = minioService.sourceKeys or builtins.attrNames minioSources;
          defaultSource = minioService.defaultSource or "local";
        };

        reth = {
          enable = rethService.enable or false;
          portKeyHttp = rethService.portKeyHttp or "rethHttp";
          portKeyWs = rethService.portKeyWs or "rethWs";
          portKeyAuth = rethService.portKeyAuth or "rethAuth";
          dataDirName = rethService.dataDirName or "reth";
          network = rethService.network or "local";
          devMode = rethService.devMode or false;
          extraArgs = rethService.extraArgs or [ ];
          sources = rethSources;
          sourceKeys = rethService.sourceKeys or builtins.attrNames rethSources;
          defaultSource = rethService.defaultSource or "local";
        };

        helios =
          let
            heliosNetwork = heliosConfiguredNetwork;
            useRemoteHeliosDefaults = heliosNetwork != "local";
          in
          {
            enable = heliosService.enable or false;
            portKeyRpc = heliosService.portKeyRpc or "heliosRpc";
            executionRpcPortKey = heliosService.executionRpcPortKey or "rethHttp";
            dataDirName = heliosService.dataDirName or "helios";
            network = heliosNetwork;
            executionRpcUrl = if useRemoteHeliosDefaults then heliosConfiguredExecutionRpcUrl else "";
            consensusRpcUrl = if useRemoteHeliosDefaults then heliosConfiguredConsensusRpcUrl else "";
            defaultConsensusRpcUrl = heliosConfiguredDefaultConsensusRpcUrl;
            checkpoint = heliosConfiguredCheckpoint;
            extraArgs = heliosService.extraArgs or [ ];
            sources = heliosSources;
            sourceKeys = heliosService.sourceKeys or builtins.attrNames heliosSources;
            defaultSource = heliosService.defaultSource or "local";
            sourceKinds = heliosService.sourceKinds or { };
            readiness = heliosService.readiness or { };
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
            ${cargoWorkspaceTargetPreamble}

            echo "INFO: starting dev workflow"

            if [ -z "''${DATABASE_URL:-}" ]; then
              export DATABASE_URL="postgresql://postgres:postgres@127.0.0.1:5432/mfm"
            fi

            if [ -z "''${MFM_EVM_RPC_URL:-}" ]; then
              export MFM_EVM_RPC_URL="http://127.0.0.1:8545"
            fi
            if [ -z "''${MFM_EVM_RPC_SOURCES_JSON:-}" ]; then
              export MFM_EVM_RPC_SOURCES_JSON='[{"id":"reth_ethereum_mainnet","network_id":"ethereum-mainnet","rpc_url":"http://127.0.0.1:8545","kind":"local"},{"id":"reth_local","network_id":"reth-local","rpc_url":"http://127.0.0.1:8545","kind":"local"}]'
            fi
            if [ -z "''${MFM_EVM_RPC_PREFERRED_ORDER:-}" ]; then
              export MFM_EVM_RPC_PREFERRED_ORDER="reth_ethereum_mainnet,reth_local"
            fi

            if [ -z "''${MFM_REST_API_ADDR:-}" ]; then
              export MFM_REST_API_ADDR="127.0.0.1:3001"
            fi

            echo "INFO: launching mfm_rest_api addr=$MFM_REST_API_ADDR"
            exec cargo run -p mfm-rest-api --bin mfm_rest_api -- "$@"
          '';
        };

        mfm-portfolio-snapshot = mkCommandTask {
          id = "task.mfm.portfolio.snapshot";
          summary = "Snapshot a portfolio request with Helios-backed mainnet RPC";
          description = ''
            Starts/reuses Postgres + Helios, waits for Helios RPC health checks,
            then runs `mfm_cli --output-format json portfolio snapshot --request-file`.
          '';
          tags = [
            "mfm"
            "portfolio"
            "json"
          ];
          usage = [ "nix run .#mfm::portfolio::snapshot -- <REQUEST_FILE>" ];
          examples = [
            "MFM_ENV=dev HELIOS_NETWORK=mainnet SERVICE_OWNER_SCOPE=persistent SERVICE_DISCOVERY_SCOPE=global nix run .#mfm::portfolio::snapshot -- ./portfolio-request.json"
          ];
          runtimeInputs = leanRuntimeInputs;
          passThroughEnv = sharedPassThroughEnv ++ [
            "MFM_KEEP_SERVICES"
            "SERVICE_REUSE_POLICY"
          ];
          requirements = {
            services = [ "postgres" ];
          };
          argParser = "typed";
          allowUnknownArgs = false;
          outputFormat = "json";
          outputChannels = "stdout";
          contractArgs = [
            {
              name = "help";
              kind = "flag";
              long = "--help";
              short = "-h";
              description = "Show usage.";
            }
            {
              name = "request_file";
              kind = "positional";
              type = "string";
              required = false;
              description = "Path to the canonical portfolio snapshot request JSON file.";
            }
          ];
          command = ''
            set -euo pipefail
            ${resolvePackagedMfmCliShell}
            ${servicePolicy.policyRuntimeFunctions}

            log_error() {
              echo "ERROR: $*" >&2
            }

            if [ "''${1:-}" = "--help" ] || [ "''${1:-}" = "-h" ]; then
              echo "usage: nix run .#mfm::portfolio::snapshot -- <REQUEST_FILE>" >&2
              exit 0
            fi

            if [ "$#" -ne 1 ]; then
              echo "usage: nix run .#mfm::portfolio::snapshot -- <REQUEST_FILE>" >&2
              exit 2
            fi

            request_file="$1"
            if [ "''${request_file#/}" = "$request_file" ]; then
              request_file="$PWD/$request_file"
            fi
            snapshot_database="${postgresDatabase}"
            if [ ! -f "$request_file" ]; then
              echo "ERROR: request file does not exist: $request_file" >&2
              exit 1
            fi
            if [ ! -r "$request_file" ]; then
              echo "ERROR: request file is not readable: $request_file" >&2
              exit 1
            fi
            if [ -z "''${POSTGRES_PORT:-}" ]; then
              echo "ERROR: POSTGRES_PORT is not set for snapshot" >&2
              exit 1
            fi
            if [ "''${MFM_KEEP_SERVICES+x}" = "x" ]; then
              echo "ERROR: MFM_KEEP_SERVICES has been removed from mfm::portfolio::snapshot" >&2
              echo "Use SERVICE_OWNER_SCOPE=ephemeral|persistent and SERVICE_DISCOVERY_SCOPE=local|global instead." >&2
              exit 1
            fi
            if [ -n "''${SERVICE_REUSE_POLICY:-}" ]; then
              echo "ERROR: SERVICE_REUSE_POLICY is no longer accepted by mfm::portfolio::snapshot" >&2
              echo "Use SERVICE_OWNER_SCOPE=ephemeral|persistent and SERVICE_DISCOVERY_SCOPE=local|global instead." >&2
              exit 1
            fi
            if [ -z "''${SVC_POSTGRES_FULL_START:-}" ] || [ -z "''${SVC_POSTGRES_READY:-}" ] || [ -z "''${SVC_POSTGRES_STOP:-}" ]; then
              echo "ERROR: postgres lifecycle hooks are unavailable in the snapshot runtime" >&2
              exit 1
            fi
            if [ -z "''${SVC_POSTGRES_SETUP_DB:-}" ]; then
              echo "ERROR: postgres setup hook is unavailable in the snapshot runtime" >&2
              exit 1
            fi

            postgres_owned=0
            helios_owned=0
            result_file="$(mktemp "''${TMPDIR:-/tmp}/mfm-portfolio-snapshot.XXXXXX.json")"
            owner_scope="''${SERVICE_OWNER_SCOPE:-}"
            discovery_scope="''${SERVICE_DISCOVERY_SCOPE:-}"

            if [ -z "$owner_scope" ]; then
              case "$discovery_scope" in
                global)
                  owner_scope="persistent"
                  ;;
                local)
                  owner_scope="ephemeral"
                  ;;
                *)
                  owner_scope="ephemeral"
                  ;;
              esac
            fi

            if [ -z "$discovery_scope" ]; then
              case "$owner_scope" in
                persistent)
                  discovery_scope="global"
                  ;;
                *)
                  discovery_scope="local"
                  ;;
              esac
            fi

            nixfied_policy_validate_matrix "" "$owner_scope" "$discovery_scope" 0 1 || exit 1
            policy_mode="$(nixfied_policy_infer_reuse_policy "" "$owner_scope" "$discovery_scope" "same-root")"
            cleanup_required=1
            if [ "$owner_scope" = "persistent" ]; then
              cleanup_required=0
            fi

            cleanup() {
              local rc=$?
              if [ "$cleanup_required" = "1" ]; then
                if [ "$helios_owned" = "1" ] && [ -n "''${SVC_HELIOS_STOP:-}" ]; then
                  "$SVC_HELIOS_STOP" >/dev/null 2>&1 || true
                fi
                if [ "$postgres_owned" = "1" ]; then
                  "$SVC_POSTGRES_STOP" >/dev/null 2>&1 || true
                fi
              fi
              rm -f "$result_file" 2>/dev/null || true
              return "$rc"
            }
            trap cleanup EXIT
            trap 'exit 130' INT
            trap 'exit 143' TERM

            echo "INFO: snapshot service policy mode=$policy_mode owner=$owner_scope discovery=$discovery_scope cleanup=$cleanup_required" >&2

            if "$SVC_POSTGRES_READY" >/dev/null 2>&1; then
              echo "INFO: reusing postgres on port=$POSTGRES_PORT" >&2
            else
              postgres_owned=1
              echo "INFO: starting postgres on port=$POSTGRES_PORT" >&2
              "$SVC_POSTGRES_FULL_START" >&2
              "$SVC_POSTGRES_READY" >&2
            fi

            # Reused slots can have a live server without the task's configured database.
            echo "INFO: ensuring postgres database=$snapshot_database" >&2
            PGDATABASE="$snapshot_database" "$SVC_POSTGRES_SETUP_DB" >&2

            helios_available=0
            if ! is_service_skipped helios && [ -n "''${SVC_HELIOS_FULL_START:-}" ] && [ -n "''${SVC_HELIOS_READY:-}" ]; then
              helios_available=1
            fi

            if [ "$helios_available" = "1" ]; then
              if [ -z "''${HELIOSRPC_PORT:-}" ]; then
                echo "ERROR: HELIOSRPC_PORT is not set for snapshot" >&2
                exit 1
              fi
              export HELIOS_NETWORK="''${HELIOS_NETWORK:-mainnet}"
              if [ "$HELIOS_NETWORK" != "mainnet" ]; then
                echo "ERROR: HELIOS_NETWORK must be 'mainnet' for mfm::portfolio::snapshot" >&2
                exit 1
              fi
              export HELIOS_EXECUTION_RPC_URL="''${HELIOS_EXECUTION_RPC_URL:-${heliosConfiguredExecutionRpcUrl}}"
              export HELIOS_CONSENSUS_RPC_URL="''${HELIOS_CONSENSUS_RPC_URL:-${heliosConfiguredConsensusRpcUrl}}"
              export HELIOS_CHECKPOINT="''${HELIOS_CHECKPOINT:-${heliosConfiguredCheckpoint}}"

              if "$SVC_HELIOS_READY" >/dev/null 2>&1; then
                echo "INFO: reusing helios on port=$HELIOSRPC_PORT" >&2
              else
                helios_owned=1
                echo "INFO: starting helios on port=$HELIOSRPC_PORT network=$HELIOS_NETWORK" >&2
                "$SVC_HELIOS_FULL_START" >&2
                HELIOS_READY_TIMEOUT_SECS="''${HELIOS_READY_TIMEOUT_SECS:-300}" \
                  HELIOS_READY_INTERVAL_SECS="''${HELIOS_READY_INTERVAL_SECS:-1}" \
                  "$SVC_HELIOS_READY" >&2
              fi

              if [ -z "''${MFM_EVM_RPC_SOURCES_JSON:-}" ]; then
                export MFM_EVM_RPC_SOURCES_JSON="[{\"id\":\"helios_local\",\"network_id\":\"ethereum-mainnet\",\"rpc_url\":\"http://127.0.0.1:$HELIOSRPC_PORT\",\"kind\":\"local\"}]"
              fi
              if [ -z "''${MFM_EVM_RPC_PREFERRED_ORDER:-}" ]; then
                export MFM_EVM_RPC_PREFERRED_ORDER="helios_local"
              fi
            elif [ -z "''${MFM_EVM_RPC_SOURCES_JSON:-}" ]; then
              echo "ERROR: MFM_EVM_RPC_SOURCES_JSON is required when Helios is unavailable or skipped" >&2
              exit 1
            fi

            export DATABASE_URL="postgresql://postgres:postgres@127.0.0.1:$POSTGRES_PORT/$snapshot_database"
            mfm_cli_bin="$(resolve_packaged_mfm_cli_binary)" || exit 1

            echo "INFO: running packaged portfolio snapshot request_file=$request_file" >&2
            "$mfm_cli_bin" --output-format json portfolio snapshot --request-file "$request_file" >"$result_file"

            if ! ${pkgs.jq}/bin/jq -e '
              .status == "success"
              and .data.feature_id == "portfolio.snapshot"
              and (.data | has("result"))
            ' "$result_file" >/dev/null; then
              echo "ERROR: snapshot output did not match the expected JSON envelope" >&2
              cat "$result_file" >&2 || true
              exit 1
            fi

            cat "$result_file"
          '';
        };

        mfm_cli = mkCommandTask {
          id = "task.mfm_cli";
          summary = "Run the mfm_cli compatibility wrapper";
          description = ''
            Passthrough entrypoint that forwards all CLI arguments to mfm_cli.
          '';
          tags = [
            "cli"
            "compat"
          ];
          usage = [
            "nix run .#mfm_cli -- --help"
            "nix run .#mfm_cli -- keystore list"
          ];
          runtimeInputs = leanRuntimeInputs ++ [ conf.packages."mfm-cli" ];
          argParser = "passthrough";
          allowUnknownArgs = true;
          command = ''
            set -euo pipefail
            ${resolvePackagedMfmCliShell}

            mfm_cli_bin="$(resolve_packaged_mfm_cli_binary)" || exit 1

            exec "$mfm_cli_bin" "$@"
          '';
        };

        mfm_rest_api = mkCommandTask {
          id = "task.mfm_rest_api";
          summary = "Run the mfm_rest_api server";
          description = ''
            Typed entrypoint for launching the REST API process.
          '';
          tags = [
            "api"
            "server"
          ];
          usage = [ "nix run .#mfm_rest_api" ];
          runtimeInputs = rustRuntimeInputs;
          allowUnknownArgs = false;
          env = sharedCargoRustEnv;
          command = ''
            set -euo pipefail
            ${cargoWorkspaceTargetPreamble}

            exec cargo run -q -p mfm-rest-api --bin mfm_rest_api -- "$@"
          '';
        };

        build = mkCommandTask {
          id = "task.build";
          summary = "Build release artifacts";
          description = "Builds the workspace in release mode with all features enabled.";
          usage = [ "nix run .#build" ];
          runtimeInputs = rustRuntimeInputs;
          command = ''
            set -euo pipefail
            ${cargoWorkspaceTargetPreamble}

            echo "INFO: running release build"
            cargo build --release --all-features
            echo "OK: build completed"
          '';
        };

        check = mkCommandTask {
          id = "task.check";
          summary = "Run fmt + clippy";
          description = "Runs quality checks for the workspace.";
          usage = [ "nix run .#check" ];
          runtimeInputs = rustRuntimeInputs;
          env = sharedCargoRustEnv;
          command = ''
            set -euo pipefail
            ${cargoWorkspaceTargetPreamble}

            echo "INFO: running formatting checks"
            ${cargoFmtCheckCmd}

            echo "INFO: running clippy"
            ${cargoClippyCmd}

            echo "OK: quality checks completed"
          '';
        };

        publish-docs = mkCommandTask {
          id = "task.publish-docs";
          summary = "Plan or publish the docs.rs crate wave with exact crates.io observation";
          description = ''
            Runs the Rust Phase-1 publish-docs tool against `crates/docs/publish-wave.json`.
            The tool resolves local versions from `cargo metadata`, checks exact crates.io package
            versions, emits sanitized run artifacts under `.mfm/publish-docs/runs/`, and either
            plans or applies publish actions. By default this performs a real `cargo publish`;
            pass `--dry-run` to build a plan without uploading crates.
          '';
          tags = [
            "release"
            "docs"
          ];
          usage = [
            "nix run .#publish-docs -- --dry-run"
            "nix run .#publish-docs -- plan --json"
            "nix run .#publish-docs -- --from mfm-state-common"
            "nix run .#publish-docs -- --only mfm-docs"
            "nix run .#publish-docs"
          ];
          examples = [
            "nix run .#publish-docs -- --dry-run"
            "nix run .#publish-docs -- apply --json"
            "nix run .#publish-docs -- --from mfm-evm-runtime"
          ];
          runtimeInputs = rustRuntimeInputs ++ [ pkgs.git ];
          argParser = "passthrough";
          allowUnknownArgs = true;
          passThroughEnv = sharedPassThroughEnv ++ [
            "CARGO_HOME"
            "CARGO_REGISTRY_TOKEN"
            "CARGO_REGISTRIES_CRATES_IO_TOKEN"
            "MFM_OUTPUT_FORMAT"
          ];
          env = sharedCargoRustEnv;
          command = ''
            set -euo pipefail
            ${cargoWorkspaceTargetPreamble}
            exec cargo run -p mfm-publish-docs -- "$@"
          '';
        };

        format = mkCommandTask {
          id = "task.format";
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
          summary = "Run workspace tests";
          description = "Runs the full workspace test suite using cargo-nextest.";
          usage = [ "nix run .#test" ];
          runtimeInputs = rustRuntimeInputs;
          env = sharedCargoRustEnv;
          command = ''
            set -euo pipefail
            ${cargoWorkspaceTargetPreamble}

            echo "INFO: running workspace tests"
            ${cargoNextestWorkspaceCiCmd}
            echo "OK: tests completed"
          '';
        };

        ci = mkCommandTask {
          id = "task.ci";
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
          env = ciCargoRustEnv;
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

        ci-workflow-basic = mkCommandTask {
          id = "task.ci.workflow-basic";
          kind = "internal";
          summary = "Run workflow.ci.basic";
          description = "Internal workflow reference used to compose workflow.ci.full.";
          runtimeInputs = rustRuntimeInputs;
          workflowId = "workflow.ci.basic";
        };

        ci-workflow-parity = mkCommandTask {
          id = "task.ci.workflow-parity";
          kind = "internal";
          summary = "Run workflow.ci.parity";
          description = "Internal workflow reference used to compose workflow.ci.full.";
          runtimeInputs = rustRuntimeInputs;
          workflowId = "workflow.ci.parity";
        };

        ci-fmt = mkCommandTask {
          id = "task.ci.fmt";
          kind = "ci-step";
          summary = "CI formatting step";
          tags = [
            "ci"
            "quality"
          ];
          runtimeInputs = rustRuntimeInputs;
          env = ciCargoRustEnv;
          command = ''
            set -euo pipefail
            ${ciStepPreamble}

            log_file="$artifacts_dir/fmt.log"
            echo "INFO: running ci step=fmt"
            run_with_log "$log_file" ${cargoFmtCheckCmd}
            echo "OK: ci step passed step=fmt log=$log_file"
          '';
        };

        ci-clippy = mkCommandTask {
          id = "task.ci.clippy";
          kind = "ci-step";
          summary = "CI clippy step";
          tags = [
            "ci"
            "quality"
          ];
          runtimeInputs = rustRuntimeInputs;
          env = ciCargoRustEnv;
          command = ''
            set -euo pipefail
            ${ciStepPreamble}

            log_file="$artifacts_dir/clippy.log"
            echo "INFO: running ci step=clippy"
            run_with_log "$log_file" ${cargoCiClippyCmd}
            echo "OK: ci step passed step=clippy log=$log_file"
          '';
        };

        ci-shell-app-contracts = mkCommandTask {
          id = "task.ci.shell-app-contracts";
          kind = "ci-step";
          summary = "CI shell/model surface contract checks";
          tags = [
            "ci"
            "quality"
          ];
          runtimeInputs = rustRuntimeInputs;
          env = ciCargoRustEnv;
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

              build_output_with_timeout() {
                local ref="$1"
                local label="$2"
                local rc=0
                local output=""

                output="$(${pkgs.coreutils}/bin/timeout --signal=TERM --kill-after=10s ${toString ciShellAppContractsTimeoutSec} nix build --no-link --print-out-paths "$ref")" || rc="$?"
                if [ "$rc" -ne 0 ]; then
                  if [ "$rc" -eq 124 ]; then
                    echo "ERROR: $label build timed out after ${toString ciShellAppContractsTimeoutSec}s"
                  fi
                  exit "$rc"
                fi

                printf "%s" "$output"
              }

              INTROSPECTION_BUNDLE_PATH="$(build_output_with_timeout "path:$ROOT#introspectionBundle" "introspection bundle")"

              require_app() {
                local app_name="$1"
                if ! jq -e --arg node "app:$app_name" --arg id "$app_name" ".nodeViews[\$node].jsonByMode.default.resolved.kind == \"app\" and .nodeViews[\$node].jsonByMode.default.resolved.id == \$id" "$INTROSPECTION_BUNDLE_PATH" >/dev/null; then
                  echo "ERROR: missing required app surface app=$app_name"
                  exit 1
                fi
              }

              require_task() {
                local task_id="$1"
                if ! jq -e --arg node "task:$task_id" --arg id "$task_id" ".nodeViews[\$node].jsonByMode.default.resolved.kind == \"task\" and .nodeViews[\$node].jsonByMode.default.resolved.id == \$id" "$INTROSPECTION_BUNDLE_PATH" >/dev/null; then
                  echo "ERROR: missing required compiled task id=$task_id"
                  exit 1
                fi
              }

              require_workflow() {
                local workflow_id="$1"
                if ! jq -e --arg node "workflow:$workflow_id" --arg id "$workflow_id" ".nodeViews[\$node].jsonByMode.default.resolved.kind == \"workflow\" and .nodeViews[\$node].jsonByMode.default.resolved.id == \$id" "$INTROSPECTION_BUNDLE_PATH" >/dev/null; then
                  echo "ERROR: missing required compiled workflow id=$workflow_id"
                  exit 1
                fi
              }

              require_workflow_plan_task() {
                local workflow_id="$1"
                local task_id="$2"
                if ! jq -e --arg node "workflow:$workflow_id" --arg task "$task_id" ".nodeViews[\$node].jsonByMode.default.closure.summary.unitTaskIds | index(\$task) != null" "$INTROSPECTION_BUNDLE_PATH" >/dev/null; then
                  echo "ERROR: workflow plan missing task workflow=$workflow_id task=$task_id"
                  exit 1
                fi
              }

              require_app "mfm_cli"
              require_app "mfm::portfolio::snapshot"
              require_app "mfm_rest_api"
              require_app "dev"
              require_app "build"
              require_app "check"
              require_app "publish-docs"
              require_app "format"
              require_app "test"
              require_app "ci"
              require_app "svcset::ci-parity::start"
              require_app "svcset::ci-parity::stop"
              require_app "svcset::ci-parity::export"

              require_task "task.ci"
              require_task "task.ci.services-start"
              require_task "task.ci.workflow-basic"
              require_task "task.ci.workflow-parity"
              require_task "task.mfm_cli"
              require_task "task.mfm.portfolio.snapshot"
              require_task "task.mfm_rest_api"

              require_workflow "workflow.ci.full"
              require_workflow_plan_task "workflow.ci.full" "task.ci.workflow-basic"
              require_workflow_plan_task "workflow.ci.full" "task.ci.workflow-parity"

              if grep -R -n "[.]framework/" "$ROOT/nixfied/project" --include="*.nix" >/dev/null; then
                echo "ERROR: project layer references framework-private paths"
                exit 1
              fi
            '
            echo "OK: ci step passed step=shell-app-contracts log=$log_file"
          '';
        };

        ci-tests = mkCommandTask {
          id = "task.ci.tests";
          kind = "ci-step";
          summary = "CI tests step";
          tags = [
            "ci"
            "tests"
          ];
          runtimeInputs = rustRuntimeInputs;
          env = ciCargoRustEnv;
          command = ''
            set -euo pipefail
            ${ciStepPreamble}

            log_file="$artifacts_dir/tests.log"
            echo "INFO: running ci step=tests"
            run_with_log "$log_file" ${cargoNextestWorkspaceCiCmd}
            echo "OK: ci step passed step=tests log=$log_file"
          '';
        };

        ci-services-start = mkCommandTask {
          id = "task.ci.services-start";
          kind = "ci-step";
          summary = "Bootstrap local CI parity services";
          description = "Starts local postgres/minio/reth dependencies through public lifecycle hooks and prepares MinIO state for parity checks.";
          tags = [
            "ci"
            "parity"
            "services"
          ];
          runtimeInputs = ciServicesRuntimeInputs;
          requirements = {
            services = [
              "postgres"
              "minio"
              "reth"
            ];
          };
          command = ''
            set -euo pipefail
            ${ciStepPreamble}
            ${ciParityServiceEnv}

            echo "INFO: starting ci services env=$env_value slot=$slot_value postgres=$POSTGRES_PORT minio=$MINIO_API_PORT reth=$RETH_HTTP_PORT"

            has_service_hook() {
              local hook_var="$1"
              if [ -z "$hook_var" ]; then
                return 1
              fi
              [ -n "''${!hook_var:-}" ]
            }

            run_service_hook() {
              local hook_var="$1"
              shift || true

              local hook_cmd="''${!hook_var:-}"
              if [ -z "$hook_cmd" ]; then
                echo "ERROR: hook command is not available: $hook_var"
                return 1
              fi

              "$hook_cmd" "$@"
            }

            require_hook() {
              local hook_var="$1"
              if ! has_service_hook "$hook_var"; then
                local available_hooks
                available_hooks="$(env | grep '^SVC_' | cut -d= -f1 | tr '\n' ',')"
                echo "ERROR: required hook missing: $hook_var services=''${NIXFIED_SELECTED_SERVICES_CSV:-} hooks=''${available_hooks:-<none>}"
                exit 1
              fi
            }

            run_logged_hook() {
              local step_name="$1"
              local log_file="$2"
              shift 2

              echo "INFO: running ci bootstrap step=$step_name"
              if run_with_log "$log_file" "$@"; then
                echo "OK: ci bootstrap step passed step=$step_name log=$log_file"
              else
                local rc=$?
                echo "ERROR: ci bootstrap step failed step=$step_name log=$log_file rc=$rc" >&2
                exit "$rc"
              fi
            }

            require_hook "SVC_POSTGRES_FULL_START_TEST"
            require_hook "SVC_MINIO_FULL_START_TEST"
            require_hook "SVC_MINIO_BUCKET_ENSURE"
            require_hook "SVC_RETH_FULL_START_TEST"
            export MINIO_ROOT_USER="$AWS_ACCESS_KEY_ID"
            export MINIO_ROOT_PASSWORD="$AWS_SECRET_ACCESS_KEY"
            run_logged_hook "postgres-full-start-test" "$artifacts_dir/postgres-full-start.log" run_service_hook SVC_POSTGRES_FULL_START_TEST
            run_logged_hook "minio-full-start-test" "$artifacts_dir/minio-full-start.log" run_service_hook SVC_MINIO_FULL_START_TEST
            run_logged_hook "reth-full-start-test" "$artifacts_dir/reth-full-start.log" run_service_hook SVC_RETH_FULL_START_TEST

            run_logged_hook "minio-bucket-ensure" "$artifacts_dir/minio-bucket-ensure.log" run_service_hook SVC_MINIO_BUCKET_ENSURE "$MFM_S3_BUCKET"

            echo "OK: ci services ready postgres=$POSTGRES_PORT minio=$MINIO_API_PORT reth=$RETH_HTTP_PORT"
          '';
        };

        ci-audit = mkCommandTask {
          id = "task.ci.audit";
          kind = "ci-step";
          summary = "CI security audit step";
          tags = [
            "ci"
            "audit"
          ];
          runtimeInputs = rustRuntimeInputs;
          env = ciCargoRustEnv;
          command = ''
            set -euo pipefail
            ${ciStepPreamble}

            log_file="$artifacts_dir/audit.log"
            echo "INFO: running ci step=audit"
            run_with_log "$log_file" cargo audit
            echo "OK: ci step passed step=audit log=$log_file"
          '';
        };

        ci-parity-compile = mkCommandTask {
          id = "task.ci.parity-compile";
          kind = "ci-step";
          summary = "CI parity precompile step";
          tags = [
            "ci"
            "parity"
          ];
          runtimeInputs = rustRuntimeInputs;
          env = ciCargoRustEnv;
          command = ''
            set -euo pipefail
            ${ciStepPreamble}

            log_file="$artifacts_dir/parity-compile.log"
            echo "INFO: running ci step=parity-compile"
            run_with_log "$log_file" ${parityNextestCmd} --no-run
            echo "OK: ci step passed step=parity-compile log=$log_file"
          '';
        };

        ci-parity-rest-api-smoke = mkCommandTask {
          id = "task.ci.parity-rest-api-smoke";
          kind = "ci-step";
          summary = "CI parity REST API smoke tests";
          tags = [
            "ci"
            "parity"
          ];
          runtimeInputs = rustRuntimeInputs;
          env = ciCargoRustEnv;
          command = ''
            set -euo pipefail
            ${ciStepPreamble}
            ${ciParityServiceEnv}

            log_file="$artifacts_dir/parity-rest-api-smoke.log"
            echo "INFO: running ci step=parity-rest-api-smoke"
            run_with_log "$log_file" ${parityNextestCmd} --jobs 1 --test parity_event_store_postgres_contract --test parity_artifact_store_s3_contract --test parity_rest_api_postgres_s3_smoke
            echo "OK: ci step passed step=parity-rest-api-smoke log=$log_file"
          '';
        };

        ci-parity-evm-helios-smoke = mkCommandTask {
          id = "task.ci.parity-evm-helios-smoke";
          kind = "ci-step";
          summary = "CI parity helios smoke tests";
          tags = [
            "ci"
            "parity"
          ];
          runtimeInputs = rustRuntimeInputs;
          env = ciCargoRustEnv;
          requirements = {
            services = [ "helios" ];
          };
          command = ''
            set -euo pipefail
            ${ciStepPreamble}
            ${ciParityServiceEnv}

            has_service_hook() {
              local hook_var="$1"
              if [ -z "$hook_var" ]; then
                return 1
              fi
              [ -n "''${!hook_var:-}" ]
            }

            run_service_hook() {
              local hook_var="$1"
              shift || true

              local hook_cmd="''${!hook_var:-}"
              if [ -z "$hook_cmd" ]; then
                echo "ERROR: hook command is not available: $hook_var"
                return 1
              fi

              "$hook_cmd" "$@"
            }

            require_hook() {
              local hook_var="$1"
              if ! has_service_hook "$hook_var"; then
                local available_hooks
                available_hooks="$(env | grep '^SVC_' | cut -d= -f1 | tr '\n' ',')"
                echo "ERROR: required hook missing: $hook_var services=''${NIXFIED_SELECTED_SERVICES_CSV:-} hooks=''${available_hooks:-<none>}"
                exit 1
              fi
            }

            run_logged_hook() {
              local step_name="$1"
              local log_file="$2"
              shift 2

              echo "INFO: running ci bootstrap step=$step_name"
              if run_with_log "$log_file" "$@"; then
                echo "OK: ci bootstrap step passed step=$step_name log=$log_file"
              else
                local rc=$?
                echo "ERROR: ci bootstrap step failed step=$step_name log=$log_file rc=$rc" >&2
                exit "$rc"
              fi
            }

            smoke_log_file="$artifacts_dir/parity-evm-helios-smoke.log"
            response_file="$artifacts_dir/parity-evm-helios-smoke.response.json"

            echo "INFO: running ci step=parity-evm-helios-smoke"

            require_hook "SVC_HELIOS_FULL_START_TEST"
            require_hook "SVC_HELIOS_READY"
            export HELIOS_EXECUTION_RPC_URL="http://127.0.0.1:$RETH_HTTP_PORT"

            if run_service_hook SVC_HELIOS_READY >/dev/null 2>&1; then
              echo "INFO: reusing helios rpc_port=$HELIOS_RPC_PORT"
            else
              run_logged_hook "helios-full-start-test" "$artifacts_dir/helios-full-start.log" run_service_hook SVC_HELIOS_FULL_START_TEST
            fi

            helios_rpc_url="http://127.0.0.1:$HELIOS_RPC_PORT"
            run_with_log "$smoke_log_file" bash -euo pipefail -c '
              response_file="$1"
              rpc_url="$2"
              curl -fsS \
                -H "content-type: application/json" \
                --data "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"eth_chainId\",\"params\":[]}" \
                "$rpc_url" \
                | tee "$response_file"
            ' _ "$response_file" "$helios_rpc_url"

            jq -e '.result | strings' "$response_file" >/dev/null
            echo "OK: ci step passed step=parity-evm-helios-smoke log=$smoke_log_file"
          '';
        };

        ci-parity-evm-reth = mkCommandTask {
          id = "task.ci.parity-evm-reth";
          kind = "ci-step";
          summary = "CI parity EVM + portfolio tracker tests";
          tags = [
            "ci"
            "parity"
          ];
          runtimeInputs = rustRuntimeInputs;
          env = ciCargoRustEnv;
          command = ''
            set -euo pipefail
            ${ciStepPreamble}
            ${ciParityServiceEnv}

            log_file="$artifacts_dir/parity-evm-reth.log"
            echo "INFO: running ci step=parity-evm-reth"
            run_with_log "$log_file" ${parityNextestCmd} --jobs 1 --test parity_keystore_reth_tx_send --test evm_rpc_pool_failover --test evm_rpc_getlogs_chunking --test parity_rest_api_evm_reth_pipeline --test parity_portfolio_tracker_reth_mock_erc20 --test parity_portfolio_tracker_reth_snapshot
            echo "OK: ci step passed step=parity-evm-reth log=$log_file"
          '';
        };

        ci-parity-aave-v3-reth = mkCommandTask {
          id = "task.ci.parity-aave-v3-reth";
          kind = "ci-step";
          summary = "CI parity Aave v3 scenario tests";
          tags = [
            "ci"
            "parity"
          ];
          runtimeInputs = rustRuntimeInputs;
          env = ciCargoRustEnv;
          command = ''
            set -euo pipefail
            ${ciStepPreamble}
            ${ciParityServiceEnv}

            log_file="$artifacts_dir/parity-aave-v3-reth.log"
            echo "INFO: running ci step=parity-aave-v3-reth"
            run_with_log "$log_file" ${parityNextestCmd} --test parity_aave_v3_reth_scenario
            echo "OK: ci step passed step=parity-aave-v3-reth log=$log_file"
          '';
        };

        ci-parity-postgres-state-events-audit = mkCommandTask {
          id = "task.ci.parity-postgres-state-events-audit";
          kind = "ci-step";
          summary = "CI parity postgres state event audit";
          tags = [
            "ci"
            "parity"
          ];
          runtimeInputs = rustRuntimeInputs;
          env = ciCargoRustEnv;
          command = ''
            set -euo pipefail
            ${ciStepPreamble}
            ${ciParityServiceEnv}

            log_file="$artifacts_dir/parity-postgres-state-events-audit.log"
            echo "INFO: running ci step=parity-postgres-state-events-audit"
            run_with_log "$log_file" ${parityNextestCmd} --test parity_postgres_state_events_audit
            echo "OK: ci step passed step=parity-postgres-state-events-audit log=$log_file"
          '';
        };

        ci-mainnet-portfolio-snapshot-helios = mkCommandTask {
          id = "task.ci.mainnet-portfolio-snapshot-helios";
          kind = "ci-step";
          summary = "CI mainnet portfolio snapshot validation";
          tags = [
            "ci"
            "mainnet"
          ];
          runtimeInputs = leanRuntimeInputs ++ [ conf.packages."mfm-cli" ];
          requirements = {
            services = [ "helios" ];
          };
          command = ''
            set -euo pipefail
            ${ciStepPreamble}
            ${resolvePackagedMfmCliShell}

            address="''${MFM_CI_MAINNET_ADDRESS:-0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045}"
            log_file="$artifacts_dir/mainnet-portfolio-snapshot.log"
            out_file="$artifacts_dir/mainnet-portfolio-snapshot.json"
            request_file="$artifacts_dir/mainnet-portfolio-snapshot-request.json"

            export HELIOS_NETWORK="''${HELIOS_NETWORK:-mainnet}"
            export HELIOS_EXECUTION_RPC_URL="''${HELIOS_EXECUTION_RPC_URL:-${
              if heliosConfiguredExecutionRpcUrl != "" then
                heliosConfiguredExecutionRpcUrl
              else
                "https://ethereum-rpc.publicnode.com"
            }}"
            export HELIOS_CONSENSUS_RPC_URL="''${HELIOS_CONSENSUS_RPC_URL:-${
              if heliosConfiguredConsensusRpcUrl != "" then
                heliosConfiguredConsensusRpcUrl
              else
                "https://lodestar-mainnet.chainsafe.io"
            }}"

            echo "INFO: running ci step=mainnet-portfolio-snapshot-helios address=$address"

            cat >"$request_file" <<EOF
            {
              "portfolio": {
                "portfolio_id": "ci-mainnet-portfolio",
                "quote_codes": ["USD", "BTC"],
                "networks": [
                  {
                    "network_id": "ethereum-mainnet",
                    "chain_id": 1,
                    "metadata": {}
                  }
                ],
                "wallets": [
                  {
                    "wallet_id": "wallet_mainnet",
                    "address": "$address",
                    "network_id": "ethereum-mainnet",
                    "implementation": { "kind": "address_only" },
                    "symbol_ids": ["eth.native.ethereum-mainnet"],
                    "metadata": {}
                  }
                ],
                "symbol_configs": [
                  {
                    "symbol_id": "eth.native.ethereum-mainnet",
                    "display_symbol": "ETH",
                    "kind": "native_balance",
                    "role": "native",
                    "network_id": "ethereum-mainnet",
                    "protocol": null,
                    "balance_reader": { "kind": "native_balance" },
                    "valuation": {
                      "quotes": [
                        {
                          "quote": "USD",
                          "priced_symbol_id": "eth.native.ethereum-mainnet",
                          "reader": {
                            "kind": "direct_price",
                            "source": {
                              "source_id": "chainlink_eth_usd_mainnet",
                              "network_id": "ethereum-mainnet",
                              "base_symbol_id": "eth.native.ethereum-mainnet",
                              "quote": "USD"
                            }
                          }
                        },
                        {
                          "quote": "BTC",
                          "priced_symbol_id": "eth.native.ethereum-mainnet",
                          "reader": {
                            "kind": "derived_unit_price",
                            "numerator": {
                              "source_id": "chainlink_eth_usd_mainnet",
                              "network_id": "ethereum-mainnet",
                              "base_symbol_id": "eth.native.ethereum-mainnet",
                              "quote": "USD"
                            },
                            "denominator": {
                              "source_id": "chainlink_btc_usd_mainnet",
                              "network_id": "ethereum-mainnet",
                              "base_symbol_id": "btc.native.ethereum-mainnet",
                              "quote": "USD"
                            }
                          }
                        }
                      ]
                    },
                    "decimals": 18,
                    "underlying_symbol_id": null,
                    "metadata": {}
                  }
                ],
                "metadata": {}
              },
              "valuation_source_registry": {
                "sources": [
                  {
                    "source_id": "chainlink_eth_usd_mainnet",
                    "network_id": "ethereum-mainnet",
                    "base_symbol_id": "eth.native.ethereum-mainnet",
                    "quote": "USD",
                    "reader": {
                      "kind": "evm_oracle",
                      "oracle_kind": "chainlink_aggregator_v3",
                      "config": {
                        "contract_address": "0x5f4ec3df9cbd43714fe2740f5e3616155c5b8419"
                      }
                    },
                    "metadata": {}
                  },
                  {
                    "source_id": "chainlink_btc_usd_mainnet",
                    "network_id": "ethereum-mainnet",
                    "base_symbol_id": "btc.native.ethereum-mainnet",
                    "quote": "USD",
                    "reader": {
                      "kind": "evm_oracle",
                      "oracle_kind": "chainlink_aggregator_v3",
                      "config": {
                        "contract_address": "0xf4030086522a5beea4988f8ca5b36dbc97bee88c"
                      }
                    },
                    "metadata": {}
                  }
                ]
              }
            }
            EOF

            run_packaged_mfm_cli_snapshot() {
              local mfm_cli_bin=""
              mfm_cli_bin="$(resolve_packaged_mfm_cli_binary)" || return 1
              "$mfm_cli_bin" --output-format json portfolio snapshot --request-file "$request_file"
            }

            mode="$(printf '%s' "''${OUTPUT_MODE:-stdout}" | tr '[:upper:]' '[:lower:]')"
            case "$mode" in
              logs)
                run_packaged_mfm_cli_snapshot >"$out_file" 2>"$log_file"
                ;;
              stdout|both|"")
                run_packaged_mfm_cli_snapshot > >(tee "$out_file") 2> >(tee "$log_file" >&2)
                ;;
              *)
                run_packaged_mfm_cli_snapshot >"$out_file" 2>"$log_file"
                ;;
            esac

            jq -e '.status == "success"' "$out_file" >/dev/null
            jq -e '.data.result.phase == "completed"' "$out_file" >/dev/null
            echo "OK: ci step passed step=mainnet-portfolio-snapshot-helios log=$log_file"
          '';
        };

      }
      // frameworkInstallPreset.tasks
      // frameworkTestPreset.tasks
      // frameworkSelfhostPreset.tasks;

      apps = {
        dev = mkTaskApp {
          taskId = "task.dev";
          appId = "dev";
          usage = [ "MFM_ENV=dev NIX_ENV=0 nix run .#dev" ];
          examples = [
            "DATABASE_URL=postgresql://postgres:postgres@127.0.0.1:5432/mfm nix run .#dev"
          ];
          ownerFile = "nixfied/project/module.nix";
        };

        "mfm::portfolio::snapshot" = mkTaskApp {
          taskId = "task.mfm.portfolio.snapshot";
          appId = "mfm::portfolio::snapshot";
          usage = [ "nix run .#mfm::portfolio::snapshot -- <REQUEST_FILE>" ];
          examples = [
            "MFM_ENV=dev HELIOS_NETWORK=mainnet SERVICE_OWNER_SCOPE=persistent SERVICE_DISCOVERY_SCOPE=global nix run .#mfm::portfolio::snapshot -- ./portfolio-request.json"
          ];
          ownerFile = "nixfied/project/module.nix";
        };

        mfm_cli = mkTaskApp {
          taskId = "task.mfm_cli";
          appId = "mfm_cli";
          usage = [
            "nix run .#mfm_cli -- --help"
            "nix run .#mfm_cli -- keystore list"
          ];
          ownerFile = "nixfied/project/module.nix";
        };

        mfm_rest_api = mkTaskApp {
          taskId = "task.mfm_rest_api";
          appId = "mfm_rest_api";
          usage = [ "nix run .#mfm_rest_api" ];
          ownerFile = "nixfied/project/module.nix";
        };

        build = mkTaskApp {
          taskId = "task.build";
          appId = "build";
          usage = [ "nix run .#build" ];
          ownerFile = "nixfied/project/module.nix";
        };

        check = mkTaskApp {
          taskId = "task.check";
          appId = "check";
          usage = [ "nix run .#check" ];
          ownerFile = "nixfied/project/module.nix";
        };

        "publish-docs" = mkTaskApp {
          taskId = "task.publish-docs";
          appId = "publish-docs";
          usage = [
            "nix run .#publish-docs -- --dry-run"
            "nix run .#publish-docs -- plan --json"
            "nix run .#publish-docs -- --from mfm-state-common"
            "nix run .#publish-docs -- --only mfm-docs"
            "nix run .#publish-docs"
          ];
          examples = [
            "nix run .#publish-docs -- --dry-run"
            "nix run .#publish-docs -- apply --json"
            "nix run .#publish-docs -- --from mfm-evm-runtime"
          ];
          ownerFile = "nixfied/project/module.nix";
        };

        format = mkTaskApp {
          taskId = "task.format";
          appId = "format";
          usage = [ "nix run .#format" ];
          ownerFile = "nixfied/project/module.nix";
        };

        test = mkTaskApp {
          taskId = "task.test";
          appId = "test";
          usage = [ "nix run .#test" ];
          ownerFile = "nixfied/project/module.nix";
        };

        ci = mkTaskApp {
          taskId = "task.ci";
          appId = "ci";
          usage = [
            "nix run .#ci -- --mode basic --summary"
            "nix run .#ci -- --mode audit --summary"
            "nix run .#ci -- --mode parity --summary"
            "nix run .#ci -- --mode full --summary"
            "nix run .#ci -- --mode mainnet --summary"
          ];
          ownerFile = "nixfied/project/module.nix";
        };
      }
      // frameworkInstallPreset.apps
      // frameworkTestPreset.apps
      // frameworkSelfhostPreset.apps;

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

            shell-app-contracts = mkWorkflowUnit {
              taskId = "task.ci.shell-app-contracts";
            };

            tests = mkWorkflowUnit {
              taskId = "task.ci.tests";
              needs = [
                "fmt"
                "clippy"
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
            root = ciArtifactsRoot;
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
            root = ciArtifactsRoot;
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
              needs = [ "parity-rest-api-smoke" ];
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
          preRun = {
            tasks = [ "task.ci.services-start" ];
            serviceSets = [ ];
          };
          postRun = {
            tasks = [ ];
            serviceSets = [
              {
                serviceSetId = "service-set.ci-parity";
                operation = "stop";
              }
            ];
            alwaysRun = true;
          };
          artifacts = {
            root = ciArtifactsRoot;
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
            root = ciArtifactsRoot;
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
            root = ciArtifactsRoot;
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
      }
      // frameworkSelfhostPreset.workflows;
    };
  };
}
