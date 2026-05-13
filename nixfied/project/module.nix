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

  workspaceInventoryCheck = pkgs.writeShellScriptBin "mfm-verify-workspace-manifest" ''
    set -euo pipefail
    exec ${pkgs.python3}/bin/python3 ${./workspace-inventory-check.py} "$@"
  '';
  loggingRuntime = import ../framework/runtime/helpers/logging-runtime.nix { inherit pkgs; };
  discovery = import ../framework/runtime/helpers/discovery.nix {
    inherit pkgs;
    project = conf;
    loggingPrelude = loggingRuntime.loggingPrelude;
  };

  opensslBuildPackage = if pkgs.stdenv.isDarwin then pkgs.libressl else pkgs.openssl;
  opensslLibPackage = lib.getLib opensslBuildPackage;
  opensslDevPackage = lib.getDev opensslBuildPackage;

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
    workspaceInventoryCheck
    discovery.tool
  ];
  leanRuntimeInputs = coreRuntimeInputs;
  configuredServices = conf.services or { };
  postgresService = configuredServices.postgres or { };
  nginxService = configuredServices.nginx or { };
  minioService = configuredServices.minio or { };
  rethService = configuredServices.reth or { };
  postgresSources = postgresService.sources or { };
  nginxSources = nginxService.sources or { };
  minioSources = minioService.sources or { };
  rethSources = rethService.sources or { };
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

  serviceSkipEnvVarName =
    serviceName:
    let
      safeServiceName = lib.toUpper (lib.replaceStrings [ "." "-" ] [ "_" "_" ] serviceName);
    in
    "SKIP_${safeServiceName}";

  graphConfig = conf.graph or { };
  excludedServices = builtins.sort builtins.lessThan (
    lib.unique (graphConfig.excludedServices or [ ])
  );
  projectEnvToken = lib.toUpper (lib.replaceStrings [ "." "-" ] [ "_" "_" ] project.id);
  projectEphemeralRootEnvVar = "${projectEnvToken}_EPHEMERAL_ROOT";
  sccacheBinary = "${pkgs.sccache}/bin/sccache";

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
    "LOG_FORMAT"
    "LOG_SPAN_EVENTS"
    "SERVICE_OWNER_SCOPE"
    "SERVICE_DISCOVERY_SCOPE"
    "MFM_SNAPSHOT_REQUEST_FILE"
    "MFM_SNAPSHOT_HANDOFF_FILE"
    "MFM_SNAPSHOT_RESULT_FILE"
    "MFM_CI_ENABLE_PARITY"
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
    "SCCACHE_DIR"
    "SCCACHE_SERVER_UDS_PATH"
  ]
  ++ serviceSkipEnvVars;

  defaultSccacheDirExpr = "\${XDG_CACHE_HOME:-$HOME/.cache}/nixfied-runtime/${project.id}/sccache";
  sharedCargoRustEnv = {
    RUSTC_WRAPPER = sccacheBinary;
  }
  // lib.optionalAttrs pkgs.stdenv.isDarwin {
    LIBRARY_PATH = "${pkgs.libiconv}/lib";
    CC = "/usr/bin/clang";
    CXX = "/usr/bin/clang++";
  };

  # CI runs under ephemeral roots, but the ephemeral wrapper pins SCCACHE_DIR to
  # a stable host-side cache so local compiler reuse remains safe across runs.
  ciCargoRustEnv = sharedCargoRustEnv // {
    CARGO_BUILD_JOBS = "1";
    OPENSSL_DIR = "${opensslLibPackage}";
    OPENSSL_LIB_DIR = "${opensslLibPackage}/lib";
    OPENSSL_INCLUDE_DIR = "${opensslDevPackage}/include";
  };
  ciClippyCargoEnv = ciCargoRustEnv // {
    MFM_CI_CARGO_CACHE_SCOPE = "clippy-ci";
  };
  ciNextestCargoEnv = ciCargoRustEnv // {
    MFM_CI_CARGO_CACHE_SCOPE = "nextest-ci";
  };
  ciArtifactsRoot =
    conf.process.artifactsRoot or "\${XDG_CACHE_HOME:-$HOME/.cache}/nixfied-artifacts-${project.id}";
  ciShellAppContractsTimeoutSec = 300;
  parityNextestArchiveFileName = "parity-nextest.tar.zst";
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
  cargoWorkspaceInventoryCheckCmd = "mfm-verify-workspace-manifest --root . --metadata --expect-member-once crates/collectors/rpc-control";
  discoveryCheckCmd = "nixfied-discovery-index --verify --root .";
  # Keep CI linting on the same Cargo profile as nextest to avoid profile drift
  # within .#ci. Clippy still uses its own driver, so reuse remains partial.
  cargoCiClippyCmd = "cargo clippy --profile ci --workspace --lib --examples --tests --benches --all-features -- -D warnings";
  cargoNextestCiCmd = "cargo nextest run --cargo-profile ci";
  cargoNextestArchiveCiCmd = "cargo nextest archive --cargo-profile ci";
  cargoNextestWorkspaceCiCmd = "${cargoNextestCiCmd} --workspace";
  parityNextestArgs = "-p mfm-integration-tests --features parity-tests -p mfm --features parity-tests";
  parityNextestCmd = "${cargoNextestCiCmd} ${parityNextestArgs}";
  parityNextestArchiveCmd = "${cargoNextestArchiveCiCmd} ${parityNextestArgs}";
  mkNextestSelection =
    {
      binaryIds,
      jobs ? null,
      usesArchive ? true,
      workspaceRemap ? true,
    }:
    {
      inherit
        binaryIds
        jobs
        usesArchive
        workspaceRemap
        ;
    };
  mkNextestBinaryIdFilterArgs =
    selection:
    lib.concatMapStringsSep " " (
      binaryId: "-E ${lib.escapeShellArg "binary_id(=${binaryId})"}"
    ) selection.binaryIds;
  mkNextestJobArgs =
    selection: lib.optionalString (selection.jobs != null) "--jobs ${toString selection.jobs}";
  renderNextestContractCheck =
    taskId: selection:
    let
      expected = builtins.toJSON selection;
    in
    ''
      expected_nextest_json="$(mktemp "''${TMPDIR:-/tmp}/nextest-metadata.XXXXXX")"
      cat >"$expected_nextest_json" <<'"'"'JSON'"'"'
      ${expected}
      JSON
      if ! jq -e --arg node "task:${taskId}" --slurpfile expected "$expected_nextest_json" ".nodeViews[\$node].jsonByMode.default.resolution.data.ci.nextest == \$expected[0]" "$INTROSPECTION_BUNDLE_PATH" >/dev/null; then
        echo "ERROR: nextest task metadata mismatch task=${taskId}"
        rm -f "$expected_nextest_json"
        exit 1
      fi
      rm -f "$expected_nextest_json"
    '';
  parityRestApiSmokeNextest = mkNextestSelection {
    jobs = 1;
    binaryIds = [
      "mfm-integration-tests::parity_event_store_postgres_contract"
      "mfm-integration-tests::parity_artifact_store_s3_contract"
      "mfm-integration-tests::parity_rest_api_postgres_s3_smoke"
    ];
  };
  parityEvmRethNextest = mkNextestSelection {
    jobs = 1;
    binaryIds = [
      "mfm::parity_keystore_reth_tx_send"
      "mfm-integration-tests::evm_rpc_pool_failover"
      "mfm-integration-tests::evm_rpc_getlogs_chunking"
      "mfm-integration-tests::parity_rest_api_evm_reth_pipeline"
      "mfm-integration-tests::parity_portfolio_tracker_reth_mock_erc20"
      "mfm-integration-tests::parity_portfolio_tracker_reth_snapshot"
    ];
  };
  parityAaveV3RethNextest = mkNextestSelection {
    binaryIds = [
      "mfm-integration-tests::parity_aave_v3_reth_scenario"
    ];
  };
  parityPostgresStateEventsAuditNextest = mkNextestSelection {
    binaryIds = [
      "mfm-integration-tests::parity_postgres_state_events_audit"
    ];
  };

  cargoSccachePreamble = ''
    export SCCACHE_DIR="''${SCCACHE_DIR:-${defaultSccacheDirExpr}}"
    mkdir -p "$SCCACHE_DIR"

    # Keep the compiler cache on the host, but reset any inherited daemon so
    # the current runtime scope owns the socket/tmpdir selection.
    runtime_env="''${${project.envVar}:-dev}"
    runtime_slot="''${${project.slotVar}:-0}"
    runtime_tmpdir="''${TMPDIR:-/tmp}"
    sccache_tmpdir="/tmp/${project.id}-tmp-$runtime_env-$runtime_slot-p$$"
    rm -f "$sccache_tmpdir"
    ln -sfn "$runtime_tmpdir" "$sccache_tmpdir"
    export TMPDIR="$sccache_tmpdir"

    project_ephemeral_root_var="${projectEphemeralRootEnvVar}"
    project_ephemeral_root="''${!project_ephemeral_root_var:-}"
    ${sccacheBinary} --stop-server >/dev/null 2>&1 || true
    if [ -n "$project_ephemeral_root" ] && [ -z "''${SCCACHE_SERVER_UDS_PATH:-}" ]; then
      export SCCACHE_SERVER_UDS_PATH="$project_ephemeral_root/tmp/sccache.sock"
    elif [ -z "''${SCCACHE_SERVER_UDS_PATH:-}" ]; then
      export SCCACHE_SERVER_UDS_PATH="/tmp/${project.id}-sccache-$runtime_env-$runtime_slot.sock"
    fi

    if [ -n "''${SCCACHE_SERVER_UDS_PATH:-}" ]; then
      mkdir -p "$(dirname "$SCCACHE_SERVER_UDS_PATH")"
    fi
  '';

  ciStepPreamble = ''
    artifacts_dir="''${CI_ARTIFACTS_DIR:-${ciArtifactsRoot}}"
    mkdir -p "$artifacts_dir"
    ${cargoSccachePreamble}

    # Keep Cargo artifacts outside the workspace root so flake/model
    # evaluation does not trip over mutable target/ files.
    run_id_component="''${NIXFIED_ORCHESTRATOR_RUN_ID:-''${NIXFIED_RUN_ID:-''${NIX_ENV:-0}}}"
    run_id_component="$(printf '%s' "$run_id_component" | tr './:' '__')"
    task_id_component="''${NIXFIED_TASK_ID:-unknown-task}"
    task_id_component="$(printf '%s' "$task_id_component" | tr './:' '__')"
    cargo_cache_scope="''${MFM_CI_CARGO_CACHE_SCOPE:-$task_id_component}"
    cargo_cache_scope="$(printf '%s' "$cargo_cache_scope" | tr './:' '__')"
    cache_root="''${TMPDIR:-/tmp}/mfm-ci-cache/$run_id_component"
    cargo_artifacts_root="''${CI_ARTIFACTS_DIR:-$cache_root}"
    mkdir -p "$cargo_artifacts_root"
    # Nixfied keeps CI_ARTIFACTS_DIR stable across a managed workflow and its
    # nested workflowRef tasks, so compatible CI steps can opt into a shared
    # Cargo target scope without widening reuse across independent runs.
    export CARGO_HOME="''${CARGO_HOME:-$cache_root/cargo-home}"
    mkdir -p "$CARGO_HOME"
    export CARGO_TARGET_DIR="''${CARGO_TARGET_DIR:-$cargo_artifacts_root/cargo-target/$cargo_cache_scope}"
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
    ${cargoSccachePreamble}

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
    export MFM_EVM_RPC_SOURCES_JSON="[{\"id\":\"reth_ethereum_mainnet\",\"network_id\":\"ethereum-mainnet\",\"rpc_url\":\"http://127.0.0.1:$RETH_HTTP_PORT\",\"kind\":\"local\"},{\"id\":\"reth_local_mainnet\",\"network_id\":\"ethereum-mainnet\",\"rpc_url\":\"http://127.0.0.1:$RETH_HTTP_PORT\",\"kind\":\"local\"},{\"id\":\"reth_local\",\"network_id\":\"reth-local\",\"rpc_url\":\"http://127.0.0.1:$RETH_HTTP_PORT\",\"kind\":\"local\"}]"
    export MFM_EVM_RPC_PREFERRED_ORDER="reth_ethereum_mainnet,reth_local_mainnet,reth_local"
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
  '';
  parityNextestArchiveShell = ''
    parity_nextest_archive_file="''${MFM_CI_PARITY_NEXTEST_ARCHIVE_FILE:-$artifacts_dir/${parityNextestArchiveFileName}}"
    parity_nextest_extract_root="''${MFM_CI_PARITY_NEXTEST_EXTRACT_ROOT:-$artifacts_dir/parity-nextest-extract}"

    require_parity_nextest_archive() {
      if [ -f "$parity_nextest_archive_file" ]; then
        return 0
      fi

      echo "ERROR: missing parity nextest archive path=$parity_nextest_archive_file" >&2
      echo "INFO: run task.ci.parity-compile first to produce the shared parity archive" >&2
      return 1
    }

    parity_nextest_extract_dir_for_task() {
      local task_id="''${NIXFIED_TASK_ID:-unknown-task}"
      local task_component
      task_component="$(printf '%s' "$task_id" | tr './:' '__')"
      printf '%s/%s' "$parity_nextest_extract_root" "$task_component"
    }
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
      ci ? { },
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
      passThroughRuntimeEnv ? [ ],
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

      ci = {
        nextest = ci.nextest or null;
      };

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
        passThroughRuntimeEnv = passThroughRuntimeEnv;
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
      locks ? [ ],
      skipIfMissingEnv ? [ ],
      requirements ? {
        services = [ ];
      },
    }:
    {
      inherit
        taskId
        needs
        locks
        skipIfMissingEnv
        ;
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

      packages = conf.packages;

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
          portKeyP2p = rethService.portKeyP2p or "rethP2p";
          dataDirName = rethService.dataDirName or "reth";
          network = rethService.network or "local";
          devMode = rethService.devMode or false;
          extraArgs = rethService.extraArgs or [ ];
          sources = rethSources;
          sourceKeys = rethService.sourceKeys or builtins.attrNames rethSources;
          defaultSource = rethService.defaultSource or "local";
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
          env = sharedCargoRustEnv;
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
          summary = "Snapshot a portfolio request with configured mainnet RPC";
          description = ''
            Starts/reuses Postgres, configures an EVM RPC source if needed, then runs
            `mfm_cli --output-format json portfolio snapshot --request-file`.
          '';
          tags = [
            "mfm"
            "portfolio"
            "json"
          ];
          usage = [ "nix run .#mfm::portfolio::snapshot -- <REQUEST_FILE>" ];
          examples = [
            "MFM_ENV=dev SERVICE_OWNER_SCOPE=persistent SERVICE_DISCOVERY_SCOPE=global nix run .#mfm::portfolio::snapshot -- ./portfolio-request.json"
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
              description = "Path to the portfolio snapshot request file in JSON or TOML.";
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

            if [ -z "''${MFM_EVM_RPC_SOURCES_JSON:-}" ]; then
              export MFM_EVM_RPC_SOURCES_JSON='[{"id":"publicnode_ethereum_mainnet","network_id":"ethereum-mainnet","rpc_url":"https://ethereum-rpc.publicnode.com","kind":"remote_public"}]'
            fi
            if [ -z "''${MFM_EVM_RPC_PREFERRED_ORDER:-}" ]; then
              export MFM_EVM_RPC_PREFERRED_ORDER="publicnode_ethereum_mainnet"
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
          env = sharedCargoRustEnv;
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

            artifacts_dir="''${CI_ARTIFACTS_DIR:-${ciArtifactsRoot}}"
            mkdir -p "$artifacts_dir"
            workspace_log="$artifacts_dir/check-workspace-inventory.log"
            discovery_log="$artifacts_dir/check-discovery.log"
            fmt_log="$artifacts_dir/check-fmt.log"
            clippy_log="$artifacts_dir/check-clippy.log"

            run_with_log() {
              local logfile="$1"
              shift
              "$@" 2>&1 | tee "$logfile"
            }

            echo "INFO: validating Cargo workspace inventory"
            echo "INFO: command=${cargoWorkspaceInventoryCheckCmd} log=$workspace_log"
            run_with_log "$workspace_log" ${cargoWorkspaceInventoryCheckCmd}

            echo "INFO: verifying discovery artifacts"
            echo "INFO: command=${discoveryCheckCmd} log=$discovery_log"
            run_with_log "$discovery_log" ${discoveryCheckCmd}

            echo "INFO: running formatting checks"
            echo "INFO: command=${cargoFmtCheckCmd} log=$fmt_log"
            run_with_log "$fmt_log" ${cargoFmtCheckCmd}

            echo "INFO: running clippy"
            echo "INFO: command=${cargoClippyCmd} log=$clippy_log"
            run_with_log "$clippy_log" ${cargoClippyCmd}

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
            use `plan` to build a plan without uploading crates.
          '';
          tags = [
            "release"
            "docs"
          ];
          usage = [
            "nix run .#publish-docs -- plan"
            "nix run .#publish-docs -- plan --json"
            "nix run .#publish-docs -- --from mfm-state-common"
            "nix run .#publish-docs -- --only mfm-docs"
            "nix run .#publish-docs"
          ];
          examples = [
            "nix run .#publish-docs -- plan"
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
          env = ciClippyCargoEnv;
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

              build_output_with_skip_env_with_timeout() {
                local ref="$1"
                local label="$2"
                local rc=0
                local output=""

                output="$(
                  SKIP_POSTGRES=1 ${pkgs.coreutils}/bin/timeout --signal=TERM --kill-after=10s ${toString ciShellAppContractsTimeoutSec} \
                    nix build --impure --no-link --print-out-paths "$ref"
                )" || rc="$?"
                if [ "$rc" -ne 0 ]; then
                  if [ "$rc" -eq 124 ]; then
                    echo "ERROR: $label build timed out after ${toString ciShellAppContractsTimeoutSec}s"
                  fi
                  exit "$rc"
                fi

                printf "%s" "$output"
              }

              mfm-verify-workspace-manifest --root "$ROOT" --metadata --expect-member-once crates/collectors/rpc-control
              nixfied-discovery-index --verify --root "$ROOT"

              workspace_inventory_fixture="$(mktemp -d "''${TMPDIR:-/tmp}/workspace-inventory.XXXXXX")"
              mkdir -p "$workspace_inventory_fixture/a"
              cat >"$workspace_inventory_fixture/Cargo.toml" <<'"'"'EOF'"'"'
[workspace]
members = [
  "a",
  "a",
]
EOF
              if mfm-verify-workspace-manifest --manifest "$workspace_inventory_fixture/Cargo.toml" >"$workspace_inventory_fixture/out" 2>&1; then
                echo "ERROR: duplicate Cargo workspace member fixture unexpectedly passed"
                cat "$workspace_inventory_fixture/out"
                exit 1
              fi
              if ! grep -F "duplicate Cargo workspace members: a" "$workspace_inventory_fixture/out" >/dev/null; then
                echo "ERROR: duplicate Cargo workspace member fixture missing expected diagnostic"
                cat "$workspace_inventory_fixture/out"
                exit 1
              fi

              discovery_fixture="$(mktemp -d "''${TMPDIR:-/tmp}/discovery-index.XXXXXX")"
              mkdir -p "$discovery_fixture/docs"
              cat >"$discovery_fixture/docs/repo-index.json" <<'"'"'EOF'"'"'
{
  "schema_version": 1,
  "generated_by": "nixfied-discovery-index",
  "docs": [],
  "components": [],
  "risk_areas": [
    {
      "path": "missing/path",
      "risk": "fixture",
      "required_checks": []
    }
  ],
  "command_surfaces": [],
  "features": []
}
EOF
              cat >"$discovery_fixture/docs/repo-map.md" <<'"'"'EOF'"'"'
# Fixture
EOF
              if nixfied-discovery-index --verify --root "$discovery_fixture" >"$discovery_fixture/out" 2>&1; then
                echo "ERROR: nonexistent discovery path fixture unexpectedly passed"
                cat "$discovery_fixture/out"
                exit 1
              fi
              if ! grep -F "discovery index path does not exist field=risk_areas.path path=missing/path" "$discovery_fixture/out" >/dev/null; then
                echo "ERROR: nonexistent discovery path fixture missing expected diagnostic"
                cat "$discovery_fixture/out"
                exit 1
              fi

              INTROSPECTION_BUNDLE_PATH="$(build_output_with_timeout ".#introspectionBundle" "introspection bundle")"
              INTROSPECTION_BUNDLE_WITH_SKIP_POSTGRES_PATH="$(
                build_output_with_skip_env_with_timeout ".#introspectionBundle" "introspection bundle with SKIP_POSTGRES"
              )"

              if ! ${pkgs.diffutils}/bin/cmp -s "$INTROSPECTION_BUNDLE_PATH" "$INTROSPECTION_BUNDLE_WITH_SKIP_POSTGRES_PATH"; then
                echo "ERROR: introspection bundle changed when SKIP_POSTGRES was set during evaluation"
                exit 1
              fi

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

              require_absent_app() {
                local app_name="$1"
                if jq -e --arg node "app:$app_name" ".nodeViews | has(\$node)" "$INTROSPECTION_BUNDLE_PATH" >/dev/null; then
                  echo "ERROR: unexpected app surface app=$app_name"
                  exit 1
                fi
              }

              require_local_overrides_inactive() {
                if ! jq -e ".nodeViews[\"app:check\"].jsonByMode.default.diagnostics.localOverridesActive == false and .nodeViews[\"app:check\"].jsonByMode.default.diagnostics.legacyLocalDefault.active == false" "$INTROSPECTION_BUNDLE_PATH" >/dev/null; then
                  echo "ERROR: default compiled graph has active local overrides"
                  exit 1
                fi
              }

              require_local_overrides_inactive
              require_app "mfm::portfolio::snapshot"
              require_app "mfm_rest_api"
              require_app "dev"
              require_app "build"
              require_app "check"
              require_app "publish-docs"
              require_app "format"
              require_app "test"
              require_app "ci"
              require_app "framework::runtime-env-default"
              require_app "framework::runtime-env-opt-in"
              require_app "framework::runtime-env-invalid"
              require_app "framework::runtime-env-pass-through-blocked"
              require_app "svcset::ci-parity::start"
              require_app "svcset::ci-parity::stop"
              require_app "svcset::ci-parity::export"
              require_absent_app "aave-v3-origin-fetch"
              require_absent_app "aave-v3-origin-compile"
              require_absent_app "aave-v3-origin-deploy"

              require_task "task.ci"
              require_task "task.ci.services-start"
              require_task "task.ci.sccache-contracts"
              require_task "task.ci.workflow-basic"
              require_task "task.ci.workflow-parity"
              require_task "task.ci.parity-rest-api-smoke"
              require_task "task.ci.parity-evm-reth"
              require_task "task.ci.parity-aave-v3-reth"
              require_task "task.ci.parity-postgres-state-events-audit"
              require_task "task.framework.runtime-env-default"
              require_task "task.framework.runtime-env-opt-in"
              require_task "task.framework.runtime-env-invalid"
              require_task "task.framework.runtime-env-pass-through-blocked"
              require_task "task.mfm.portfolio.snapshot"
              require_task "task.mfm_rest_api"

              require_workflow "workflow.ci.basic"
              require_workflow "workflow.ci.full"
              require_workflow_plan_task "workflow.ci.basic" "task.ci.sccache-contracts"
              require_workflow_plan_task "workflow.ci.parity" "task.ci.parity-rest-api-smoke"
              require_workflow_plan_task "workflow.ci.parity" "task.ci.parity-evm-reth"
              require_workflow_plan_task "workflow.ci.parity" "task.ci.parity-aave-v3-reth"
              require_workflow_plan_task "workflow.ci.parity" "task.ci.parity-postgres-state-events-audit"
              require_workflow_plan_task "workflow.ci.full" "task.ci.workflow-basic"
              require_workflow_plan_task "workflow.ci.full" "task.ci.workflow-parity"

              ${renderNextestContractCheck "task.ci.parity-rest-api-smoke" parityRestApiSmokeNextest}
              ${renderNextestContractCheck "task.ci.parity-evm-reth" parityEvmRethNextest}
              ${renderNextestContractCheck "task.ci.parity-aave-v3-reth" parityAaveV3RethNextest}
              ${renderNextestContractCheck "task.ci.parity-postgres-state-events-audit" parityPostgresStateEventsAuditNextest}

              if grep -R -n "[.]framework/" "$ROOT/nixfied/project" --include="*.nix" >/dev/null; then
                echo "ERROR: project layer references framework-private paths"
                exit 1
              fi

              read_report_value() {
                local key="$1"
                local output_file="$2"
                local line=""
                line="$(grep -m1 "^$key=" "$output_file" || true)"
                if [ -z "$line" ]; then
                  echo "ERROR: missing runtime-env report key=$key file=$output_file"
                  cat "$output_file"
                  exit 1
                fi
                printf "%s" "''${line#*=}"
              }

              expect_report_value() {
                local output_file="$1"
                local key="$2"
                local expected="$3"
                local actual=""
                actual="$(read_report_value "$key" "$output_file")"
                if [ "$actual" != "$expected" ]; then
                  echo "ERROR: unexpected runtime-env value key=$key expected=$expected actual=$actual file=$output_file"
                  cat "$output_file"
                  exit 1
                fi
              }

              run_report_command() {
                local output_file="$1"
                shift
                if "$@" >"$output_file" 2>&1; then
                  :
                else
                  local rc=$?
                  echo "ERROR: runtime-env probe failed rc=$rc output=$output_file"
                  cat "$output_file"
                  exit "$rc"
                fi
              }

              run_report_command_expect_failure() {
                local output_file="$1"
                shift
                if "$@" >"$output_file" 2>&1; then
                  echo "ERROR: runtime-env probe unexpectedly passed output=$output_file"
                  cat "$output_file"
                  exit 1
                fi
              }

              runtime_env_fixture_root="$(mktemp -d "''${TMPDIR:-/tmp}/shell-app-contracts.runtime-env.XXXXXX")"
              trap "rm -rf \"$runtime_env_fixture_root\"" EXIT
              caller_home="$runtime_env_fixture_root/h"
              caller_tmp="$runtime_env_fixture_root/t"
              caller_xdg_data="$runtime_env_fixture_root/d"
              caller_xdg_state="$runtime_env_fixture_root/s"
              caller_xdg_cache="$runtime_env_fixture_root/c"
              mkdir -p "$caller_home" "$caller_tmp" "$caller_xdg_data" "$caller_xdg_state" "$caller_xdg_cache"

              default_report="$runtime_env_fixture_root/default.report"
              opt_in_report="$runtime_env_fixture_root/opt-in.report"
              fallback_report="$runtime_env_fixture_root/fallback.report"
              invalid_report="$runtime_env_fixture_root/invalid.report"
              blocked_report="$runtime_env_fixture_root/blocked.report"

              run_report_command \
                "$default_report" \
                env \
                HOME="$caller_home" \
                TMPDIR="$caller_tmp" \
                XDG_DATA_HOME="$caller_xdg_data" \
                XDG_STATE_HOME="$caller_xdg_state" \
                XDG_CACHE_HOME="$caller_xdg_cache" \
                nix run ".#framework::runtime-env-default"
              default_scope="$(read_report_value "NIXFIED_RUNTIME_DIR_SCOPE" "$default_report")"
              expect_report_value "$default_report" "HOME" "$default_scope/home"
              expect_report_value "$default_report" "TMPDIR" "$default_scope/tmp"
              expect_report_value "$default_report" "XDG_DATA_HOME" "$default_scope/xdg/data"
              expect_report_value "$default_report" "XDG_STATE_HOME" "$default_scope/xdg/state"
              expect_report_value "$default_report" "XDG_CACHE_HOME" "$default_scope/xdg/cache"

              run_report_command \
                "$opt_in_report" \
                env \
                HOME="$caller_home" \
                TMPDIR="$caller_tmp" \
                XDG_DATA_HOME="$caller_xdg_data" \
                XDG_STATE_HOME="$caller_xdg_state" \
                XDG_CACHE_HOME="$caller_xdg_cache" \
                nix run ".#framework::runtime-env-opt-in"
              expect_report_value "$opt_in_report" "HOME" "$caller_home"
              expect_report_value "$opt_in_report" "TMPDIR" "$caller_tmp"
              expect_report_value "$opt_in_report" "XDG_DATA_HOME" "$caller_xdg_data"
              expect_report_value "$opt_in_report" "XDG_STATE_HOME" "$caller_xdg_state"
              expect_report_value "$opt_in_report" "XDG_CACHE_HOME" "$caller_xdg_cache"

              run_report_command \
                "$fallback_report" \
                env \
                -u XDG_DATA_HOME \
                -u XDG_STATE_HOME \
                -u XDG_CACHE_HOME \
                HOME="$caller_home" \
                TMPDIR="$caller_tmp" \
                nix run ".#framework::runtime-env-opt-in"
              fallback_scope="$(read_report_value "NIXFIED_RUNTIME_DIR_SCOPE" "$fallback_report")"
              expect_report_value "$fallback_report" "HOME" "$caller_home"
              expect_report_value "$fallback_report" "TMPDIR" "$caller_tmp"
              expect_report_value "$fallback_report" "XDG_DATA_HOME" "$fallback_scope/xdg/data"
              expect_report_value "$fallback_report" "XDG_STATE_HOME" "$fallback_scope/xdg/state"
              expect_report_value "$fallback_report" "XDG_CACHE_HOME" "$fallback_scope/xdg/cache"

              run_report_command_expect_failure \
                "$invalid_report" \
                nix run ".#framework::runtime-env-invalid"
              if ! grep -F "runtime-owned runtime env passthrough blocked name=REGISTRY_ROOT" "$invalid_report" >/dev/null; then
                echo "ERROR: invalid runtime-env passthrough failure missing expected message"
                cat "$invalid_report"
                exit 1
              fi

              run_report_command_expect_failure \
                "$blocked_report" \
                nix run ".#framework::runtime-env-pass-through-blocked"
              if ! grep -F "runtime-owned passthrough env blocked name=TMPDIR" "$blocked_report" >/dev/null; then
                echo "ERROR: ordinary passThroughEnv failure missing expected message"
                cat "$blocked_report"
                exit 1
              fi
            '
            echo "OK: ci step passed step=shell-app-contracts log=$log_file"
          '';
        };

        ci-sccache-contracts = mkCommandTask {
          id = "task.ci.sccache-contracts";
          kind = "ci-step";
          summary = "CI sccache runtime contract checks";
          tags = [
            "ci"
            "quality"
          ];
          runtimeInputs = rustRuntimeInputs;
          env = ciCargoRustEnv;
          command = ''
            set -euo pipefail
            ${ciStepPreamble}

            log_file="$artifacts_dir/sccache-contracts.log"
            export NIXFIED_PROJECT_EPHEMERAL_ROOT_VAR="${projectEphemeralRootEnvVar}"
            echo "INFO: running ci step=sccache-contracts"
            run_with_log "$log_file" bash -euo pipefail -c '
              project_ephemeral_root_var="''${NIXFIED_PROJECT_EPHEMERAL_ROOT_VAR:-}"
              project_ephemeral_root=""

              if [ -z "$project_ephemeral_root_var" ]; then
                echo "ERROR: missing project ephemeral root env var name"
                exit 1
              fi

              eval "project_ephemeral_root=\''${$project_ephemeral_root_var:-}"
              if [ -z "$project_ephemeral_root" ]; then
                echo "ERROR: missing project ephemeral root path env=$project_ephemeral_root_var"
                exit 1
              fi

              if [ -z "''${CI_ARTIFACTS_DIR:-}" ]; then
                echo "ERROR: CI_ARTIFACTS_DIR is unset"
                exit 1
              fi

              expected_sccache_wrapper="${sccacheBinary}"

              if [ "''${RUSTC_WRAPPER:-}" != "$expected_sccache_wrapper" ]; then
                echo "ERROR: expected RUSTC_WRAPPER=$expected_sccache_wrapper got=''${RUSTC_WRAPPER:-<unset>}"
                exit 1
              fi

              if [ ! -x "$expected_sccache_wrapper" ]; then
                echo "ERROR: expected nix-provided sccache binary at $expected_sccache_wrapper"
                exit 1
              fi

              if [ -z "''${SCCACHE_DIR:-}" ]; then
                echo "ERROR: SCCACHE_DIR is unset"
                exit 1
              fi

              if [ -z "''${SCCACHE_SERVER_UDS_PATH:-}" ]; then
                echo "ERROR: SCCACHE_SERVER_UDS_PATH is unset"
                exit 1
              fi

              if [ ! -d "$SCCACHE_DIR" ]; then
                echo "ERROR: SCCACHE_DIR does not exist path=$SCCACHE_DIR"
                exit 1
              fi

              case "$SCCACHE_DIR" in
                "$project_ephemeral_root"|"$project_ephemeral_root"/*)
                  echo "ERROR: SCCACHE_DIR must live outside ephemeral root path=$SCCACHE_DIR ephemeral_root=$project_ephemeral_root"
                  exit 1
                  ;;
              esac

              case "$SCCACHE_SERVER_UDS_PATH" in
                "$project_ephemeral_root"|"$project_ephemeral_root"/*)
                  ;;
                *)
                  echo "ERROR: SCCACHE_SERVER_UDS_PATH must live under ephemeral root path=$SCCACHE_SERVER_UDS_PATH ephemeral_root=$project_ephemeral_root"
                  exit 1
                  ;;
              esac

              case "$SCCACHE_DIR" in
                "$CI_ARTIFACTS_DIR"|"$CI_ARTIFACTS_DIR"/*)
                  echo "ERROR: SCCACHE_DIR must live outside CI_ARTIFACTS_DIR path=$SCCACHE_DIR artifacts=$CI_ARTIFACTS_DIR"
                  exit 1
                  ;;
              esac

              echo "OK: sccache contract path=$SCCACHE_DIR socket=$SCCACHE_SERVER_UDS_PATH wrapper=$RUSTC_WRAPPER"
            '
            echo "OK: ci step passed step=sccache-contracts log=$log_file"
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
          env = ciNextestCargoEnv;
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

            postgres_test_database_ready() {
              psql -h 127.0.0.1 -p "$POSTGRES_PORT" -U postgres -d "$PGDATABASE" -Atqc 'select 1;' >/dev/null 2>&1
            }

            ensure_postgres_test_db() {
              if run_service_hook SVC_POSTGRES_READY >/dev/null 2>&1 && postgres_test_database_ready; then
                echo "OK: reusing postgres with ready test database port=$POSTGRES_PORT database=$PGDATABASE"
                return 0
              fi

              if run_service_hook SVC_POSTGRES_READY >/dev/null 2>&1; then
                echo "INFO: reusing postgres on port=$POSTGRES_PORT and ensuring database=$PGDATABASE"
                run_logged_hook "postgres-setup-db" "$artifacts_dir/postgres-setup-db.log" run_service_hook SVC_POSTGRES_SETUP_DB

                if postgres_test_database_ready; then
                  echo "OK: postgres test database ready via reuse port=$POSTGRES_PORT database=$PGDATABASE"
                  return 0
                fi

                echo "ERROR: postgres reuse path did not yield a ready test database port=$POSTGRES_PORT database=$PGDATABASE" >&2
                exit 1
              fi

              run_logged_hook "postgres-full-start-test" "$artifacts_dir/postgres-full-start.log" run_service_hook SVC_POSTGRES_FULL_START_TEST

              if ! postgres_test_database_ready; then
                echo "ERROR: postgres startup path did not yield a ready test database port=$POSTGRES_PORT database=$PGDATABASE" >&2
                exit 1
              fi
            }

            require_hook "SVC_POSTGRES_READY"
            require_hook "SVC_POSTGRES_SETUP_DB"
            require_hook "SVC_POSTGRES_FULL_START_TEST"
            require_hook "SVC_MINIO_FULL_START_TEST"
            require_hook "SVC_MINIO_BUCKET_ENSURE"
            require_hook "SVC_RETH_FULL_START_TEST"
            export MINIO_ROOT_USER="$AWS_ACCESS_KEY_ID"
            export MINIO_ROOT_PASSWORD="$AWS_SECRET_ACCESS_KEY"
            ensure_postgres_test_db
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
          env = ciNextestCargoEnv;
          command = ''
            set -euo pipefail
            ${ciStepPreamble}

            archive_file="''${MFM_CI_PARITY_NEXTEST_ARCHIVE_FILE:-$artifacts_dir/${parityNextestArchiveFileName}}"
            log_file="$artifacts_dir/parity-compile.log"
            echo "INFO: running ci step=parity-compile"
            run_with_log "$log_file" ${parityNextestArchiveCmd} --archive-file "$archive_file"
            echo "OK: ci step passed step=parity-compile archive=$archive_file log=$log_file"
          '';
        };

        ci-parity-rest-api-smoke = mkCommandTask {
          id = "task.ci.parity-rest-api-smoke";
          kind = "ci-step";
          summary = "CI parity REST API smoke tests";
          ci.nextest = parityRestApiSmokeNextest;
          tags = [
            "ci"
            "parity"
          ];
          runtimeInputs = rustRuntimeInputs;
          env = ciNextestCargoEnv;
          command = ''
            set -euo pipefail
            ${ciStepPreamble}
            ${ciParityServiceEnv}
            ${parityNextestArchiveShell}

            log_file="$artifacts_dir/parity-rest-api-smoke.log"
            extract_dir="$(parity_nextest_extract_dir_for_task)"
            mkdir -p "$extract_dir"
            require_parity_nextest_archive
            echo "INFO: running ci step=parity-rest-api-smoke archive=$parity_nextest_archive_file"
            run_with_log "$log_file" cargo nextest run --archive-file "$parity_nextest_archive_file" --extract-to "$extract_dir" --workspace-remap "$MFM_WORKSPACE_ROOT" ${mkNextestJobArgs parityRestApiSmokeNextest} ${mkNextestBinaryIdFilterArgs parityRestApiSmokeNextest}
            echo "OK: ci step passed step=parity-rest-api-smoke log=$log_file"
          '';
        };

        ci-parity-evm-reth = mkCommandTask {
          id = "task.ci.parity-evm-reth";
          kind = "ci-step";
          summary = "CI parity EVM + portfolio tracker tests";
          ci.nextest = parityEvmRethNextest;
          tags = [
            "ci"
            "parity"
          ];
          runtimeInputs = rustRuntimeInputs;
          env = ciNextestCargoEnv;
          command = ''
            set -euo pipefail
            ${ciStepPreamble}
            ${ciParityServiceEnv}
            ${parityNextestArchiveShell}

            log_file="$artifacts_dir/parity-evm-reth.log"
            extract_dir="$(parity_nextest_extract_dir_for_task)"
            mkdir -p "$extract_dir"
            require_parity_nextest_archive
            echo "INFO: running ci step=parity-evm-reth archive=$parity_nextest_archive_file"
            run_with_log "$log_file" cargo nextest run --archive-file "$parity_nextest_archive_file" --extract-to "$extract_dir" --workspace-remap "$MFM_WORKSPACE_ROOT" ${mkNextestJobArgs parityEvmRethNextest} ${mkNextestBinaryIdFilterArgs parityEvmRethNextest}
            echo "OK: ci step passed step=parity-evm-reth log=$log_file"
          '';
        };

        ci-parity-aave-v3-reth = mkCommandTask {
          id = "task.ci.parity-aave-v3-reth";
          kind = "ci-step";
          summary = "CI parity Aave v3 scenario tests";
          ci.nextest = parityAaveV3RethNextest;
          tags = [
            "ci"
            "parity"
          ];
          runtimeInputs = rustRuntimeInputs;
          env = ciNextestCargoEnv;
          command = ''
            set -euo pipefail
            ${ciStepPreamble}
            ${ciParityServiceEnv}
            ${parityNextestArchiveShell}

            log_file="$artifacts_dir/parity-aave-v3-reth.log"
            extract_dir="$(parity_nextest_extract_dir_for_task)"
            mkdir -p "$extract_dir"
            require_parity_nextest_archive
            echo "INFO: running ci step=parity-aave-v3-reth archive=$parity_nextest_archive_file"
            run_with_log "$log_file" cargo nextest run --archive-file "$parity_nextest_archive_file" --extract-to "$extract_dir" --workspace-remap "$MFM_WORKSPACE_ROOT" ${mkNextestJobArgs parityAaveV3RethNextest} ${mkNextestBinaryIdFilterArgs parityAaveV3RethNextest}
            echo "OK: ci step passed step=parity-aave-v3-reth log=$log_file"
          '';
        };

        ci-parity-postgres-state-events-audit = mkCommandTask {
          id = "task.ci.parity-postgres-state-events-audit";
          kind = "ci-step";
          summary = "CI parity postgres state event audit";
          ci.nextest = parityPostgresStateEventsAuditNextest;
          tags = [
            "ci"
            "parity"
          ];
          runtimeInputs = rustRuntimeInputs;
          env = ciNextestCargoEnv;
          command = ''
            set -euo pipefail
            ${ciStepPreamble}
            ${ciParityServiceEnv}
            ${parityNextestArchiveShell}

            log_file="$artifacts_dir/parity-postgres-state-events-audit.log"
            extract_dir="$(parity_nextest_extract_dir_for_task)"
            mkdir -p "$extract_dir"
            require_parity_nextest_archive
            echo "INFO: running ci step=parity-postgres-state-events-audit archive=$parity_nextest_archive_file"
            run_with_log "$log_file" cargo nextest run --archive-file "$parity_nextest_archive_file" --extract-to "$extract_dir" --workspace-remap "$MFM_WORKSPACE_ROOT" ${mkNextestJobArgs parityPostgresStateEventsAuditNextest} ${mkNextestBinaryIdFilterArgs parityPostgresStateEventsAuditNextest}
            echo "OK: ci step passed step=parity-postgres-state-events-audit log=$log_file"
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
            "MFM_ENV=dev SERVICE_OWNER_SCOPE=persistent SERVICE_DISCOVERY_SCOPE=global nix run .#mfm::portfolio::snapshot -- ./portfolio-request.json"
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
            "nix run .#publish-docs -- plan"
            "nix run .#publish-docs -- plan --json"
            "nix run .#publish-docs -- --from mfm-state-common"
            "nix run .#publish-docs -- --only mfm-docs"
            "nix run .#publish-docs"
          ];
          examples = [
            "nix run .#publish-docs -- plan"
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

            sccache-contracts = mkWorkflowUnit {
              taskId = "task.ci.sccache-contracts";
            };

            tests = mkWorkflowUnit {
              taskId = "task.ci.tests";
              needs = [
                "fmt"
                "clippy"
                "shell-app-contracts"
                "sccache-contracts"
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
            serviceSets = [
              {
                serviceSetId = "service-set.ci-parity";
                operation = "stop";
              }
            ];
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

      }
      // frameworkSelfhostPreset.workflows;
    };
  };
}
