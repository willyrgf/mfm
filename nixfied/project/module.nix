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
  postgresPackage = conf.modules.postgres.package or pkgs.postgresql_16;
  minioPackage = conf.modules.minio.package or pkgs.minio;
  minioClientPackage = conf.modules.minio.clientPackage or pkgs.minio-client;
  rethPackage = conf.modules.reth.package or pkgs.reth;

  minioRootUser = conf.modules.minio.rootUser or "minio";
  minioRootPassword = conf.modules.minio.rootPassword or "minio123456";
  configuredServices = conf.services or { };
  postgresService = configuredServices.postgres or { };
  nginxService = configuredServices.nginx or { };
  minioService = configuredServices.minio or { };
  rethService = configuredServices.reth or { };
  heliosService = configuredServices.helios or { };
  mergeLocalSourceDefaults =
    localDefaults: serviceSources: lib.recursiveUpdate { local = localDefaults; } serviceSources;

  # The upgraded framework resolves service packages from the selected source.
  postgresSources = mergeLocalSourceDefaults (lib.optionalAttrs
    ((conf.modules.postgres.package or null) != null)
    {
      package = conf.modules.postgres.package;
    }
  ) (postgresService.sources or { });
  nginxSources = mergeLocalSourceDefaults { } (nginxService.sources or { });
  minioSources = mergeLocalSourceDefaults (
    (lib.optionalAttrs ((conf.modules.minio.package or null) != null) {
      package = conf.modules.minio.package;
    })
    // (lib.optionalAttrs ((conf.modules.minio.clientPackage or null) != null) {
      clientPackage = conf.modules.minio.clientPackage;
    })
  ) (minioService.sources or { });
  rethSources = mergeLocalSourceDefaults (lib.optionalAttrs
    ((conf.modules.reth.package or null) != null)
    {
      package = conf.modules.reth.package;
    }
  ) (rethService.sources or { });
  heliosSources = mergeLocalSourceDefaults (lib.optionalAttrs
    ((conf.modules.helios.package or null) != null)
    {
      package = conf.modules.helios.package;
    }
  ) (heliosService.sources or { });

  # Auto-compute SKIP_<SERVICE> env vars from service config (mirrors operations.nix:106-114)
  serviceSkipEnvVars = lib.unique (
    builtins.map (
      serviceName:
      let
        safeServiceName = lib.toUpper (lib.replaceStrings [ "." "-" ] [ "_" "_" ] serviceName);
      in
      "SKIP_${safeServiceName}"
    ) (builtins.attrNames conf.services)
  );

  # Reusable shell helpers mirroring the framework's skip-service primitives
  # (executor-runtime.nix:350-378, operations.nix:342-384)
  serviceSkipShellHelpers = ''
    is_truthy_skip_value() {
      local raw_value="$1"
      local normalized_value
      normalized_value="$(printf '%s' "$raw_value" | ${pkgs.coreutils}/bin/tr '[:upper:]' '[:lower:]' | ${pkgs.coreutils}/bin/tr -d '[:space:]')"
      case "$normalized_value" in
        1|true|yes|on) return 0 ;;
        *) return 1 ;;
      esac
    }

    is_service_skipped() {
      local service_name="$1"
      local safe_service_name
      local env_name
      local env_value
      safe_service_name="$(printf '%s' "$service_name" | ${pkgs.coreutils}/bin/tr '[:lower:]' '[:upper:]' | ${pkgs.coreutils}/bin/tr -cs 'A-Z0-9_' '_')"
      env_name="SKIP_$safe_service_name"
      env_value="''${!env_name:-}"
      is_truthy_skip_value "$env_value"
    }
  '';

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
    "SERVICE_REUSE_POLICY"
    "SERVICE_OWNER_SCOPE"
    "SERVICE_DISCOVERY_SCOPE"
    "MFM_KEEP_SERVICES"
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
  ] ++ serviceSkipEnvVars;

  sharedCargoRustEnv = {
    RUSTC_WRAPPER = "sccache";
    CARGO_PROFILE_CI_DEBUG = "0";
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
  ciArtifactsRoot = conf.process.artifactsRoot or "/tmp/ci-artifacts/${project.id}";
  ciShellAppContractsTimeoutSec = 300;
  ephemeralRuntimeConfig =
    let
      runtimeEphemeral = conf.ephemeral or { };
    in
    if builtins.isAttrs runtimeEphemeral then
      {
        copyMode = runtimeEphemeral.copyMode or "nix-source";
        includeUntracked = runtimeEphemeral.includeUntracked or false;
        excludePatterns = runtimeEphemeral.excludePatterns or [
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
  frameworkProject = conf.project // {
    services = conf.services;
    logging = conf.logging;
  };
  frameworkPortVarNames = {
    postgres = "POSTGRES_PORT";
    rethHttp = "RETH_HTTP_PORT";
    heliosRpc = "HELIOS_RPC_PORT";
  };
  frameworkServiceDirExprs = {
    postgres = "$NIXFIED_RUNTIME_DIR_SCOPE/services/postgres/data";
    helios = "$NIXFIED_RUNTIME_DIR_SCOPE/services/helios";
  };
  frameworkServiceSlots = {
    getSlotInfoJson = "$SLOT_INFO_JSON";
    portVarName =
      portKey:
      frameworkPortVarNames.${portKey}
        or (throw "unsupported framework service port key for snapshot task: ${portKey}");
    getServiceDir =
      dataDirName:
      frameworkServiceDirExprs.${dataDirName}
        or (throw "unsupported framework service data dir for snapshot task: ${dataDirName}");
  };
  frameworkPostgresService = import ../framework/runtime/services/postgres/default.nix {
    inherit pkgs;
    project = frameworkProject;
    slots = frameworkServiceSlots;
  };
  frameworkHeliosService = import ../framework/runtime/services/helios/default.nix {
    inherit pkgs;
    project = frameworkProject;
    slots = frameworkServiceSlots;
  };
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
    export DATABASE_URL="''${DATABASE_URL:-postgresql://postgres:postgres@127.0.0.1:$POSTGRES_PORT/mfm_test}"
    export MFM_EVM_RPC_URL="''${MFM_EVM_RPC_URL:-http://127.0.0.1:$RETH_HTTP_PORT}"
    export MFM_EVM_RPC_SOURCES_JSON="''${MFM_EVM_RPC_SOURCES_JSON:-[{\"id\":\"reth_ethereum_mainnet\",\"network_id\":\"ethereum-mainnet\",\"rpc_url\":\"http://127.0.0.1:$RETH_HTTP_PORT\",\"kind\":\"local\"},{\"id\":\"reth_local\",\"network_id\":\"reth-local\",\"rpc_url\":\"http://127.0.0.1:$RETH_HTTP_PORT\",\"kind\":\"local\"}]}"
    export MFM_EVM_RPC_PREFERRED_ORDER="''${MFM_EVM_RPC_PREFERRED_ORDER:-reth_ethereum_mainnet,reth_local}"
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
    pkgs.python3
  ];
  ciHeliosShimScript = pkgs.writeText "mfm-ci-helios-shim.py" ''
    import json
    import sys
    import urllib.error
    import urllib.request
    from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

    PORT = int(sys.argv[1])
    UPSTREAM = sys.argv[2]


    def stub(payload):
        method = payload.get("method")
        req_id = payload.get("id")
        if method in {"eth_chainId", "net_version"}:
            value = "0x1" if method == "eth_chainId" else "1"
            return {"jsonrpc": "2.0", "id": req_id, "result": value}
        if method == "eth_blockNumber":
            return {"jsonrpc": "2.0", "id": req_id, "result": "0x1"}
        if method == "eth_syncing":
            return {"jsonrpc": "2.0", "id": req_id, "result": False}
        return None


    class Handler(BaseHTTPRequestHandler):
        server_version = "helios-ci-shim/1.0"

        def do_POST(self):
            status = 200
            ctype = "application/json"
            try:
                length = int(self.headers.get("content-length", "0"))
                body = self.rfile.read(length)
                payload = json.loads(body.decode("utf-8"))

                if isinstance(payload, list):
                    out = []
                    for item in payload:
                        stubbed = stub(item)
                        if stubbed is not None:
                            out.append(stubbed)
                            continue
                        req = urllib.request.Request(
                            UPSTREAM,
                            data=json.dumps(item).encode("utf-8"),
                            headers={"content-type": "application/json"},
                            method="POST",
                        )
                        with urllib.request.urlopen(req, timeout=15) as resp:
                            out.append(json.loads(resp.read().decode("utf-8")))
                    raw = json.dumps(out).encode("utf-8")
                else:
                    stubbed = stub(payload)
                    if stubbed is not None:
                        raw = json.dumps(stubbed).encode("utf-8")
                    else:
                        req = urllib.request.Request(
                            UPSTREAM,
                            data=body,
                            headers={"content-type": "application/json"},
                            method="POST",
                        )
                        with urllib.request.urlopen(req, timeout=15) as resp:
                            raw = resp.read()
                            status = getattr(resp, "status", 200)
                            ctype = resp.headers.get("content-type", "application/json")
            except urllib.error.HTTPError as exc:
                raw = exc.read() or b'{"jsonrpc":"2.0","error":{"code":-32000,"message":"upstream http error"}}'
                status = exc.code
                ctype = exc.headers.get("content-type", "application/json")
            except Exception as exc:  # noqa: BLE001
                raw = (
                    '{"jsonrpc":"2.0","error":{"code":-32000,"message":"ci helios shim proxy error: %s"}}'
                    % str(exc).replace('"', "'")
                ).encode("utf-8")
                status = 502

            self.send_response(status)
            self.send_header("content-type", ctype)
            self.send_header("content-length", str(len(raw)))
            self.end_headers()
            self.wfile.write(raw)

        def log_message(self, *_args):
            return


    def main():
        server = ThreadingHTTPServer(("127.0.0.1", PORT), Handler)
        server.serve_forever()


    if __name__ == "__main__":
        main()
  '';

  mkCommandTask =
    {
      id,
      appName,
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

      ui.app = {
        expose = true;
        name = appName;
        inherit ownerFile;
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

  frameworkInstallPreset = import ../framework/presets/install.nix {
    inherit
      mkCommandTask
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
        directories.base = conf.directories.base;
      };

      state = {
        workspaceId = conf.process.workspaceId or project.id;
        registryRoot = conf.process.registryRoot;
        artifactsRoot = ciArtifactsRoot;
      };

      tooling = {
        runtimePackages = conf.tooling.runtimePackages;
        devShellPackages = conf.tooling.devShellPackages;
        devShellHook = conf.tooling.devShellHook;
      };

      packages = lib.removeAttrs conf.packages [ "helios" ];

      services = {
        postgres = {
          enable = postgresService.enable or (conf.modules.postgres.enable or false);
          database = postgresService.database or (conf.modules.postgres.database or "app");
          testDatabase = postgresService.testDatabase or (conf.modules.postgres.testDatabase or "app_test");
          portKey = postgresService.portKey or (conf.modules.postgres.portKey or "postgres");
          dataDirName = postgresService.dataDirName or (conf.modules.postgres.dataDirName or "postgres");
          sources = postgresSources;
          sourceKeys = postgresService.sourceKeys or builtins.attrNames postgresSources;
          defaultSource = postgresService.defaultSource or "local";
        };

        nginx = {
          enable = nginxService.enable or (conf.modules.nginx.enable or false);
          portKeyHttp = nginxService.portKeyHttp or (conf.modules.nginx.portKeyHttp or "http");
          portKeyHttps = nginxService.portKeyHttps or (conf.modules.nginx.portKeyHttps or "https");
          sources = nginxSources;
          sourceKeys = nginxService.sourceKeys or builtins.attrNames nginxSources;
          defaultSource = nginxService.defaultSource or "local";
        };

        minio = {
          enable = minioService.enable or (conf.modules.minio.enable or false);
          portKeyApi = minioService.portKeyApi or (conf.modules.minio.portKeyApi or "minioApi");
          portKeyConsole =
            minioService.portKeyConsole or (conf.modules.minio.portKeyConsole or "minioConsole");
          sources = minioSources;
          sourceKeys = minioService.sourceKeys or builtins.attrNames minioSources;
          defaultSource = minioService.defaultSource or "local";
        };

        reth = {
          enable = rethService.enable or (conf.modules.reth.enable or false);
          portKeyHttp = rethService.portKeyHttp or (conf.modules.reth.portKeyHttp or "rethHttp");
          portKeyWs = rethService.portKeyWs or (conf.modules.reth.portKeyWs or "rethWs");
          portKeyAuth = rethService.portKeyAuth or (conf.modules.reth.portKeyAuth or "rethAuth");
          sources = rethSources;
          sourceKeys = rethService.sourceKeys or builtins.attrNames rethSources;
          defaultSource = rethService.defaultSource or "local";
        };

        helios = {
          enable = heliosService.enable or (conf.modules.helios.enable or false);
          portKeyRpc = heliosService.portKeyRpc or (conf.modules.helios.portKeyRpc or "heliosRpc");
          executionRpcPortKey =
            heliosService.executionRpcPortKey or (conf.modules.helios.executionRpcPortKey or "rethHttp");
          dataDirName = heliosService.dataDirName or (conf.modules.helios.dataDirName or "helios");
          network = heliosService.network or (conf.modules.helios.network or "local");
          executionRpcUrl = heliosService.executionRpcUrl or (conf.modules.helios.executionRpcUrl or "");
          consensusRpcUrl = heliosService.consensusRpcUrl or (conf.modules.helios.consensusRpcUrl or "");
          defaultConsensusRpcUrl =
            heliosService.defaultConsensusRpcUrl or (conf.modules.helios.defaultConsensusRpcUrl or "");
          checkpoint = heliosService.checkpoint or (conf.modules.helios.checkpoint or "");
          extraArgs = heliosService.extraArgs or (conf.modules.helios.extraArgs or [ ]);
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

        mfm-portfolio-services-start =
          mkCommandTask {
            id = "task.mfm.portfolio.services-start";
            appName = "mfm-portfolio-services-start";
            kind = "internal";
            summary = "Start or reuse portfolio snapshot services";
            description = "Internal task that starts or reuses Postgres and Helios for the portfolio snapshot app.";
            runtimeInputs = commonRuntimeInputs ++ [
              postgresPackage
              pkgs.curl
              pkgs.lsof
            ];
            allowUnknownArgs = false;
            contractArgs = [
              {
                name = "handoff-file";
                kind = "option";
                long = "--handoff-file";
                type = "string";
                required = true;
                description = "Path to a non-secret handoff JSON file.";
              }
            ];
            command = ''
              set -euo pipefail
              ${serviceSkipShellHelpers}
              if [ -w /dev/tty ]; then
                exec >/dev/tty 2>&1
              fi

              handoff_file=""
              while [ "$#" -gt 0 ]; do
                case "$1" in
                  --handoff-file)
                    shift
                    handoff_file="''${1:-}"
                    ;;
                  --help|-h)
                    echo "usage: run-task task.mfm.portfolio.services-start --handoff-file <PATH>" >&2
                    exit 0
                    ;;
                  *)
                    echo "ERROR: unknown argument '$1'" >&2
                    exit 2
                    ;;
                esac
                shift
              done

              if [ -z "$handoff_file" ]; then
                echo "ERROR: --handoff-file is required" >&2
                exit 2
              fi

              if [ -z "''${POSTGRES_PORT:-}" ]; then
                echo "ERROR: POSTGRES_PORT is not set for snapshot service startup" >&2
                exit 1
              fi

              if ! is_service_skipped helios; then
                if [ -z "''${HELIOSRPC_PORT:-}" ]; then
                  echo "ERROR: HELIOSRPC_PORT is not set for snapshot service startup" >&2
                  exit 1
                fi
              fi

              if [ -z "''${NIXFIED_RUNTIME_DIR_SCOPE:-}" ]; then
                :
              fi

              if [ -z "''${NIXFIED_SERVICE_POSTGRES_DATA_DIR:-}" ] || [ -z "''${NIXFIED_SERVICE_POSTGRES_STATE_DIR:-}" ] || [ -z "''${NIXFIED_SERVICE_POSTGRES_LOG_DIR:-}" ]; then
                echo "ERROR: postgres runtime directories are unavailable in snapshot runtime" >&2
                exit 1
              fi

              if ! is_service_skipped helios; then
                if [ -z "''${NIXFIED_SERVICE_HELIOS_DATA_DIR:-}" ] || [ -z "''${NIXFIED_SERVICE_HELIOS_STATE_DIR:-}" ] || [ -z "''${NIXFIED_SERVICE_HELIOS_LOG_DIR:-}" ]; then
                  echo "ERROR: helios runtime directories are unavailable in snapshot runtime" >&2
                  exit 1
                fi
              fi

              mkdir -p "$(dirname "$handoff_file")"
              runtime_root="$(cd "$(dirname "$handoff_file")/../.." && pwd -P)"
              export NIXFIED_RUNTIME_DIR_SCOPE="$runtime_root"
              export NIXFIED_SERVICE_ROOT="$runtime_root/services"
              if ! is_service_skipped helios; then
                export HELIOS_RPC_PORT="$HELIOSRPC_PORT"
              fi
              mkdir -p "$NIXFIED_SERVICE_ROOT"
              mkdir -p "$runtime_root/run"

              slot_info_json_path="$runtime_root/slot-info.json"
              slot_info_json_script="$runtime_root/slot-info-json.sh"
              ${pkgs.jq}/bin/jq -n \
                --arg slot "''${NIX_ENV:-0}" \
                --arg env "''${MFM_ENV:-dev}" \
                --arg run_dir "$runtime_root/run" \
                --arg postgres_port "$POSTGRES_PORT" \
                --arg helios_rpc_port "''${HELIOS_RPC_PORT:-}" \
                --arg reth_http_port "''${RETH_HTTP_PORT:-}" \
                '{
                  slot: $slot,
                  env: $env,
                  directories: {
                    run: $run_dir
                  },
                  ports: (
                    {
                      POSTGRES_PORT: $postgres_port
                    }
                    + (if $helios_rpc_port == "" then {} else { HELIOS_RPC_PORT: $helios_rpc_port } end)
                    + (if $reth_http_port == "" then {} else { RETH_HTTP_PORT: $reth_http_port } end)
                  )
                }' > "$slot_info_json_path"
              cat > "$slot_info_json_script" <<EOF
              #!/bin/sh
              cat "$slot_info_json_path"
              EOF
              chmod +x "$slot_info_json_script"
              export SLOT_INFO_JSON="$slot_info_json_script"

              resolve_owner_scope() {
                if [ -n "''${SERVICE_OWNER_SCOPE:-}" ]; then
                  printf '%s' "$SERVICE_OWNER_SCOPE"
                  return 0
                fi

                case "''${SERVICE_REUSE_POLICY:-}" in
                  same-slot|cross-run)
                    printf '%s' "persistent"
                    return 0
                    ;;
                  same-root)
                    printf '%s' "ephemeral"
                    return 0
                    ;;
                esac

                case "''${SERVICE_DISCOVERY_SCOPE:-}" in
                  global)
                    printf '%s' "persistent"
                    ;;
                  *)
                    printf '%s' "ephemeral"
                    ;;
                esac
              }

              resolve_discovery_scope() {
                local owner_scope="$1"
                if [ -n "''${SERVICE_DISCOVERY_SCOPE:-}" ]; then
                  printf '%s' "$SERVICE_DISCOVERY_SCOPE"
                  return 0
                fi

                case "''${SERVICE_REUSE_POLICY:-}" in
                  same-slot|cross-run)
                    printf '%s' "global"
                    return 0
                    ;;
                  same-root)
                    printf '%s' "local"
                    return 0
                    ;;
                esac

                case "$owner_scope" in
                  persistent)
                    printf '%s' "global"
                    ;;
                  *)
                    printf '%s' "local"
                    ;;
                esac
              }

              resolve_reuse_policy() {
                local owner_scope="$1"
                local discovery_scope="$2"
                if [ -n "''${SERVICE_REUSE_POLICY:-}" ]; then
                  printf '%s' "$SERVICE_REUSE_POLICY"
                  return 0
                fi

                if [ "$owner_scope" = "persistent" ] || [ "$discovery_scope" = "global" ]; then
                  printf '%s' "same-slot"
                else
                  printf '%s' "same-root"
                fi
              }

              validate_policy_matrix() {
                local reuse="$1"
                local owner="$2"
                local discovery="$3"

                case "$reuse" in
                  never|same-root|same-slot|cross-run) ;;
                  *)
                    echo "ERROR: SERVICE_REUSE_POLICY must be one of never|same-root|same-slot|cross-run (got '$reuse')" >&2
                    return 1
                    ;;
                esac

                case "$owner" in
                  ephemeral|persistent) ;;
                  *)
                    echo "ERROR: SERVICE_OWNER_SCOPE must be ephemeral|persistent (got '$owner')" >&2
                    return 1
                    ;;
                esac

                case "$discovery" in
                  local|global) ;;
                  *)
                    echo "ERROR: SERVICE_DISCOVERY_SCOPE must be local|global (got '$discovery')" >&2
                    return 1
                    ;;
                esac

                if [ "$reuse" = "cross-run" ] && { [ "$owner" != "persistent" ] || [ "$discovery" != "global" ]; }; then
                  echo "ERROR: cross-run reuse requires SERVICE_OWNER_SCOPE=persistent and SERVICE_DISCOVERY_SCOPE=global" >&2
                  return 1
                fi

                if [ "$reuse" = "same-root" ] && { [ "$owner" != "ephemeral" ] || [ "$discovery" != "local" ]; }; then
                  echo "ERROR: same-root reuse requires SERVICE_OWNER_SCOPE=ephemeral and SERVICE_DISCOVERY_SCOPE=local" >&2
                  return 1
                fi

                if [ "$owner" = "persistent" ] && [ "$discovery" != "global" ]; then
                  echo "ERROR: persistent owner scope requires SERVICE_DISCOVERY_SCOPE=global" >&2
                  return 1
                fi

                if [ "$owner" = "ephemeral" ] && [ "$discovery" != "local" ]; then
                  echo "ERROR: ephemeral owner scope requires SERVICE_DISCOVERY_SCOPE=local" >&2
                  return 1
                fi
              }

              owner_scope="$(resolve_owner_scope)"
              discovery_scope="$(resolve_discovery_scope "$owner_scope")"
              reuse_policy="$(resolve_reuse_policy "$owner_scope" "$discovery_scope")"
              validate_policy_matrix "$reuse_policy" "$owner_scope" "$discovery_scope"

              cleanup_required=1
              if [ "$owner_scope" = "persistent" ]; then
                cleanup_required=0
              fi

              echo "INFO: snapshot service policy reuse=$reuse_policy owner=$owner_scope discovery=$discovery_scope"

              service_scope_root="$runtime_root"
              if [ "$owner_scope" = "persistent" ] || [ "$discovery_scope" = "global" ]; then
                service_env_name="''${MFM_ENV:-''${ENV:-dev}}"
                service_slot="''${NIX_ENV:-''${SLOT:-0}}"
                service_runtime_base="''${MFM_SNAPSHOT_RUNTIME_DIR_BASE:-}"
                if [ -z "$service_runtime_base" ]; then
                  service_user_name="$(id -un 2>/dev/null || true)"
                  if [ -n "$service_user_name" ]; then
                    service_user_home="$(eval printf '%s' "~$service_user_name")"
                    if [ -n "$service_user_home" ] && [ "$service_user_home" != "~$service_user_name" ]; then
                      service_runtime_base="$service_user_home/.local/share/${project.id}"
                    fi
                  fi
                fi
                if [ -n "$service_runtime_base" ]; then
                  service_scope_root="$service_runtime_base/$service_env_name/slot-$service_slot"
                fi
              fi
              service_root="$service_scope_root/services"

              export NIXFIED_RUNTIME_DIR_SCOPE="$service_scope_root"
              export NIXFIED_SERVICE_ROOT="$service_root"
              export NIXFIED_SERVICE_POSTGRES_DATA_DIR="$service_root/postgres/data"
              export NIXFIED_SERVICE_POSTGRES_STATE_DIR="$service_root/postgres/state"
              export NIXFIED_SERVICE_POSTGRES_LOG_DIR="$service_root/postgres/log"
              if ! is_service_skipped helios; then
                export NIXFIED_SERVICE_HELIOS_DATA_DIR="$service_root/helios/data"
                export NIXFIED_SERVICE_HELIOS_STATE_DIR="$service_root/helios/state"
                export NIXFIED_SERVICE_HELIOS_LOG_DIR="$service_root/helios/log"
              fi

              mkdir -p "$NIXFIED_SERVICE_ROOT"
              mkdir -p "$NIXFIED_SERVICE_POSTGRES_DATA_DIR" "$NIXFIED_SERVICE_POSTGRES_STATE_DIR" "$NIXFIED_SERVICE_POSTGRES_LOG_DIR"
              if ! is_service_skipped helios; then
                mkdir -p "$NIXFIED_SERVICE_HELIOS_DATA_DIR" "$NIXFIED_SERVICE_HELIOS_STATE_DIR" "$NIXFIED_SERVICE_HELIOS_LOG_DIR"
              fi

              echo "INFO: snapshot service scope root=$service_scope_root service_root=$service_root"

              postgres_data="$NIXFIED_SERVICE_POSTGRES_DATA_DIR"
              postgres_log="$postgres_data/postgres.log"
              postgres_owned=0

              postgres_is_running() {
                local pid=""
                if [ ! -f "$postgres_data/postmaster.pid" ]; then
                  return 1
                fi
                pid="$(head -1 "$postgres_data/postmaster.pid" 2>/dev/null || true)"
                [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null
              }

              postgres_port_ready() {
                ${postgresPackage}/bin/pg_isready -U postgres -h 127.0.0.1 -p "$POSTGRES_PORT" -q 2>/dev/null
              }

              if postgres_is_running; then
                if [ "$reuse_policy" = "never" ]; then
                  echo "ERROR: postgres already managed for this slot and reuse policy is 'never'" >&2
                  exit 1
                fi
                echo "INFO: reusing framework-managed postgres port=$POSTGRES_PORT data=$postgres_data"
              elif postgres_port_ready; then
                if [ "$reuse_policy" = "never" ]; then
                  echo "ERROR: postgres already listening on port $POSTGRES_PORT and reuse policy is 'never'" >&2
                  exit 1
                fi
                echo "INFO: reusing healthy postgres on slot port=$POSTGRES_PORT outside expected data dir=$postgres_data"
              else
                postgres_owned=1
                echo "INFO: starting framework-managed postgres port=$POSTGRES_PORT data=$postgres_data"
              fi

              if [ "$postgres_owned" = "1" ]; then
                if ! ${frameworkPostgresService.fullStart}; then
                  echo "ERROR: postgres full-start failed port=$POSTGRES_PORT data=$postgres_data" >&2
                  if [ -f "$postgres_log" ]; then
                    tail -50 "$postgres_log" >&2 || true
                  fi
                  exit 1
                fi
              fi

              if ! ${frameworkPostgresService.ready}; then
                echo "ERROR: postgres failed framework readiness checks port=$POSTGRES_PORT" >&2
                if [ -f "$postgres_log" ]; then
                  tail -50 "$postgres_log" >&2 || true
                fi
                exit 1
              fi
              if [ "$postgres_owned" != "1" ]; then
                if ! ${frameworkPostgresService.setupDb}; then
                  echo "ERROR: postgres setup-db failed on reused instance port=$POSTGRES_PORT" >&2
                  exit 1
                fi
              fi
              echo "OK: postgres readiness validated service=postgres source=local"

              helios_owned=0
              helios_pid_file=""
              helios_log=""

              if is_service_skipped helios; then
                echo "INFO: helios skipped (SKIP_HELIOS=1)"
              else
                HELIOS_NETWORK="''${HELIOS_NETWORK:-mainnet}"
                if [ "$HELIOS_NETWORK" != "mainnet" ]; then
                  echo "ERROR: HELIOS_NETWORK must be 'mainnet' for mfm::portfolio::snapshot" >&2
                  exit 1
                fi

                export HELIOS_RPC_PORT="$HELIOSRPC_PORT"
                export HELIOS_EXECUTION_RPC_URL="''${HELIOS_EXECUTION_RPC_URL:-${
                  conf.modules.helios.executionRpcUrl or "https://ethereum-rpc.publicnode.com"
                }}"
                export HELIOS_CONSENSUS_RPC_URL="''${HELIOS_CONSENSUS_RPC_URL:-}"
                export HELIOS_CHECKPOINT="''${HELIOS_CHECKPOINT:-}"

                if [ -z "$HELIOS_EXECUTION_RPC_URL" ]; then
                  echo "ERROR: HELIOS_EXECUTION_RPC_URL is required for snapshot startup" >&2
                  exit 1
                fi

                helios_root="$service_root/helios"
                helios_log="$helios_root/logs/helios.log"
                helios_pid_file="$helios_root/run/helios.pid"
                helios_health_timeout_seconds="''${HELIOS_HEALTH_TIMEOUT_SECS:-''${HELIOS_READY_TIMEOUT_SECS:-300}}"
                helios_health_interval_seconds="''${HELIOS_HEALTH_INTERVAL_SECS:-''${HELIOS_READY_INTERVAL_SECS:-1}}"

                helios_is_running() {
                  local pid=""
                  if [ ! -f "$helios_pid_file" ]; then
                    return 1
                  fi
                  pid="$(cat "$helios_pid_file" 2>/dev/null || true)"
                  [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null
                }

                helios_port_ready() {
                  ${frameworkHeliosService.health} >/dev/null 2>&1
                }

                wait_for_helios_health() {
                  local timeout="$helios_health_timeout_seconds"
                  local interval="$helios_health_interval_seconds"
                  local start_ts
                  local now_ts
                  local attempt=0

                  case "$timeout" in
                    *[!0-9]*|"")
                      echo "ERROR: HELIOS_HEALTH_TIMEOUT_SECS must be an integer seconds value (got '$timeout')" >&2
                      return 1
                      ;;
                  esac

                  case "$interval" in
                    *[!0-9]*|"")
                      echo "ERROR: HELIOS_HEALTH_INTERVAL_SECS must be an integer seconds value (got '$interval')" >&2
                      return 1
                      ;;
                  esac

                  start_ts="$(${pkgs.coreutils}/bin/date +%s)"
                  while true; do
                    attempt=$((attempt + 1))

                    if ${frameworkHeliosService.health} >/dev/null 2>&1; then
                      return 0
                    fi

                    if [ $((attempt % 10)) -eq 0 ]; then
                      log_info "helios health checks not ready yet attempt=$attempt"
                    fi

                    now_ts="$(${pkgs.coreutils}/bin/date +%s)"
                    if [ $((now_ts - start_ts)) -ge "$timeout" ]; then
                      return 1
                    fi

                    ${pkgs.coreutils}/bin/sleep "$interval"
                  done
                }

                if helios_is_running; then
                  if [ "$reuse_policy" = "never" ]; then
                    echo "ERROR: helios already managed for this slot and reuse policy is 'never'" >&2
                    exit 1
                  fi
                  echo "INFO: reusing framework-managed helios port=$HELIOS_RPC_PORT root=$helios_root"
                elif helios_port_ready; then
                  if [ "$reuse_policy" = "never" ]; then
                    echo "ERROR: helios already serving on port $HELIOS_RPC_PORT and reuse policy is 'never'" >&2
                    exit 1
                  fi
                  echo "INFO: reusing healthy helios on slot port=$HELIOS_RPC_PORT outside expected pid file=$helios_pid_file"
                else
                  helios_owned=1
                  echo "INFO: starting framework-managed helios port=$HELIOS_RPC_PORT root=$helios_root execution_rpc=$HELIOS_EXECUTION_RPC_URL"
                fi

                if [ "$helios_owned" = "1" ]; then
                  if ! ${frameworkHeliosService.fullStart}; then
                    echo "ERROR: helios full-start failed port=$HELIOS_RPC_PORT execution_rpc=$HELIOS_EXECUTION_RPC_URL" >&2
                    if [ -f "$helios_log" ]; then
                      tail -50 "$helios_log" >&2 || true
                    fi
                    exit 1
                  fi
                fi

                if ! wait_for_helios_health; then
                  echo "ERROR: helios failed snapshot health checks after $helios_health_timeout_seconds s port=$HELIOS_RPC_PORT execution_rpc=$HELIOS_EXECUTION_RPC_URL" >&2
                  if [ -f "$helios_log" ]; then
                    tail -50 "$helios_log" >&2 || true
                  fi
                  exit 1
                fi
              fi

              ${pkgs.jq}/bin/jq -n \
                --arg reuse_policy "$reuse_policy" \
                --arg owner_scope "$owner_scope" \
                --arg discovery_scope "$discovery_scope" \
                --arg cleanup_required "$cleanup_required" \
                --arg service_scope_root "$service_scope_root" \
                --arg service_root "$service_root" \
                --arg postgres_owned "$postgres_owned" \
                --arg helios_owned "$helios_owned" \
                --arg postgres_data "$postgres_data" \
                --arg postgres_log "$postgres_log" \
                --arg helios_pid_file "$helios_pid_file" \
                --arg helios_log "$helios_log" \
                '{
                  reuse_policy: $reuse_policy,
                  owner_scope: $owner_scope,
                  discovery_scope: $discovery_scope,
                  cleanup_required: ($cleanup_required == "1"),
                  service_scope_root: $service_scope_root,
                  service_root: $service_root,
                  postgres_owned: ($postgres_owned == "1"),
                  helios_owned: ($helios_owned == "1"),
                  postgres_data: $postgres_data,
                  postgres_log: $postgres_log,
                  helios_pid_file: $helios_pid_file,
                  helios_log: $helios_log
                }' > "$handoff_file"

              echo "OK: snapshot services ready postgres=$POSTGRES_PORT helios=''${HELIOS_RPC_PORT:-skipped} handoff=$handoff_file"
            '';
          }
          // {
            ui.app.expose = false;
          };

        mfm-portfolio-services-stop =
          mkCommandTask {
            id = "task.mfm.portfolio.services-stop";
            appName = "mfm-portfolio-services-stop";
            kind = "internal";
            summary = "Stop owned portfolio snapshot services";
            description = "Internal task that stops only Postgres and Helios processes owned by the current snapshot invocation.";
            runtimeInputs = commonRuntimeInputs ++ [ postgresPackage ];
            allowUnknownArgs = false;
            contractArgs = [
              {
                name = "handoff-file";
                kind = "option";
                long = "--handoff-file";
                type = "string";
                required = true;
                description = "Path to a non-secret handoff JSON file.";
              }
            ];
            command = ''
              set -euo pipefail

              handoff_file=""
              while [ "$#" -gt 0 ]; do
                case "$1" in
                  --handoff-file)
                    shift
                    handoff_file="''${1:-}"
                    ;;
                  --help|-h)
                    echo "usage: run-task task.mfm.portfolio.services-stop --handoff-file <PATH>" >&2
                    exit 0
                    ;;
                  *)
                    echo "ERROR: unknown argument '$1'" >&2
                    exit 2
                    ;;
                esac
                shift
              done

              if [ -z "$handoff_file" ]; then
                echo "ERROR: --handoff-file is required" >&2
                exit 2
              fi

              if [ ! -f "$handoff_file" ]; then
                echo "SKIP: snapshot handoff file is missing path=$handoff_file"
                exit 0
              fi

              cleanup_required="$(${pkgs.jq}/bin/jq -r 'if .cleanup_required then "1" else "0" end' "$handoff_file")"
              postgres_owned="$(${pkgs.jq}/bin/jq -r 'if .postgres_owned then "1" else "0" end' "$handoff_file")"
              helios_owned="$(${pkgs.jq}/bin/jq -r 'if .helios_owned then "1" else "0" end' "$handoff_file")"
              postgres_data="$(${pkgs.jq}/bin/jq -r '.postgres_data // empty' "$handoff_file")"
              postgres_log="$(${pkgs.jq}/bin/jq -r '.postgres_log // empty' "$handoff_file")"
              helios_pid_file="$(${pkgs.jq}/bin/jq -r '.helios_pid_file // empty' "$handoff_file")"
              helios_log="$(${pkgs.jq}/bin/jq -r '.helios_log // empty' "$handoff_file")"

              if [ "$cleanup_required" != "1" ]; then
                echo "SKIP: snapshot services preserved by policy handoff=$handoff_file"
                exit 0
              fi

              if [ "$helios_owned" = "1" ] && [ -n "$helios_pid_file" ] && [ -f "$helios_pid_file" ]; then
                helios_pid="$(cat "$helios_pid_file" 2>/dev/null || true)"
                if [ -n "$helios_pid" ] && kill -0 "$helios_pid" 2>/dev/null; then
                  echo "INFO: stopping owned helios pid=$helios_pid"
                  kill "$helios_pid" 2>/dev/null || true
                  for _ in $(seq 1 40); do
                    if ! kill -0 "$helios_pid" 2>/dev/null; then
                      break
                    fi
                    sleep 0.25
                  done
                  kill -KILL "$helios_pid" 2>/dev/null || true
                fi
                rm -f "$helios_pid_file"
              fi

              if [ "$postgres_owned" = "1" ] && [ -n "$postgres_data" ] && [ -d "$postgres_data" ]; then
                echo "INFO: stopping owned postgres data=$postgres_data"
                ${postgresPackage}/bin/pg_ctl -D "$postgres_data" stop -m fast >/dev/null 2>&1 || true
              fi

              echo "OK: snapshot services stopped handoff=$handoff_file"
            '';
          }
          // {
            ui.app.expose = false;
          };

        mfm-portfolio-snapshot-preflight =
          mkCommandTask {
            id = "task.mfm.portfolio.snapshot.preflight";
            appName = "mfm-portfolio-snapshot-preflight";
            kind = "internal";
            summary = "Validate snapshot workflow inputs";
            description = "Internal task that validates env and temp paths for the portfolio snapshot workflow.";
            runtimeInputs = leanRuntimeInputs;
            allowUnknownArgs = false;
            command = ''
              set -euo pipefail
              ${serviceSkipShellHelpers}

              if [ "$#" -ne 0 ]; then
                echo "ERROR: task.mfm.portfolio.snapshot.preflight does not accept arguments" >&2
                exit 2
              fi

              if [ -z "''${MFM_SNAPSHOT_REQUEST_FILE:-}" ]; then
                echo "ERROR: MFM_SNAPSHOT_REQUEST_FILE is required for the snapshot workflow" >&2
                exit 1
              fi

              if [ ! -f "$MFM_SNAPSHOT_REQUEST_FILE" ]; then
                echo "ERROR: MFM_SNAPSHOT_REQUEST_FILE does not exist: $MFM_SNAPSHOT_REQUEST_FILE" >&2
                exit 1
              fi

              if [ ! -r "$MFM_SNAPSHOT_REQUEST_FILE" ]; then
                echo "ERROR: MFM_SNAPSHOT_REQUEST_FILE is not readable: $MFM_SNAPSHOT_REQUEST_FILE" >&2
                exit 1
              fi

              if [ -z "''${MFM_SNAPSHOT_HANDOFF_FILE:-}" ]; then
                echo "ERROR: MFM_SNAPSHOT_HANDOFF_FILE is required for the snapshot workflow" >&2
                exit 1
              fi

              if [ -z "''${MFM_SNAPSHOT_RESULT_FILE:-}" ]; then
                echo "ERROR: MFM_SNAPSHOT_RESULT_FILE is required for the snapshot workflow" >&2
                exit 1
              fi

              if [ -z "''${POSTGRES_PORT:-}" ]; then
                echo "ERROR: POSTGRES_PORT is not set for snapshot" >&2
                exit 1
              fi

              if ! is_service_skipped helios; then
                if [ -z "''${HELIOSRPC_PORT:-}" ]; then
                  echo "ERROR: HELIOSRPC_PORT is not set for snapshot" >&2
                  exit 1
                fi
              fi

              if [ -z "''${NIXFIED_EXECUTOR_SELF:-}" ]; then
                echo "ERROR: NIXFIED_EXECUTOR_SELF is not available in the snapshot runtime" >&2
                exit 1
              fi

              if [ "''${MFM_KEEP_SERVICES+x}" = "x" ]; then
                echo "ERROR: MFM_KEEP_SERVICES has been removed from mfm::portfolio::snapshot" >&2
                echo "Use process-first controls instead:" >&2
                echo "  SERVICE_REUSE_POLICY=never|same-root|same-slot|cross-run" >&2
                echo "  SERVICE_OWNER_SCOPE=ephemeral|persistent" >&2
                echo "  SERVICE_DISCOVERY_SCOPE=local|global" >&2
                exit 1
              fi

              if ! is_service_skipped helios; then
                export HELIOS_NETWORK="''${HELIOS_NETWORK:-mainnet}"
                if [ "$HELIOS_NETWORK" != "mainnet" ]; then
                  echo "ERROR: HELIOS_NETWORK must be 'mainnet' for mfm::portfolio::snapshot" >&2
                  exit 1
                fi
              fi

              mkdir -p "$(dirname "$MFM_SNAPSHOT_HANDOFF_FILE")"
              mkdir -p "$(dirname "$MFM_SNAPSHOT_RESULT_FILE")"

              echo "OK: snapshot preflight request_file=$MFM_SNAPSHOT_REQUEST_FILE"
            '';
          }
          // {
            ui.app.expose = false;
          };

        mfm-portfolio-snapshot-services-start =
          mkCommandTask {
            id = "task.mfm.portfolio.snapshot.services-start";
            appName = "mfm-portfolio-snapshot-services-start";
            kind = "internal";
            summary = "Start snapshot workflow services";
            description = "Internal task that delegates to the portfolio snapshot service bootstrap task.";
            runtimeInputs = leanRuntimeInputs;
            allowUnknownArgs = false;
            command = ''
              set -euo pipefail

              if [ "$#" -ne 0 ]; then
                echo "ERROR: task.mfm.portfolio.snapshot.services-start does not accept arguments" >&2
                exit 2
              fi

              if [ -z "''${MFM_SNAPSHOT_HANDOFF_FILE:-}" ]; then
                echo "ERROR: MFM_SNAPSHOT_HANDOFF_FILE is required for snapshot service startup" >&2
                exit 1
              fi

              if [ -z "''${NIXFIED_EXECUTOR_SELF:-}" ]; then
                echo "ERROR: NIXFIED_EXECUTOR_SELF is required for snapshot service startup" >&2
                exit 1
              fi

              NIXFIED_CALLER_PWD="$PWD" "$NIXFIED_EXECUTOR_SELF" run-task task.mfm.portfolio.services-start --handoff-file "$MFM_SNAPSHOT_HANDOFF_FILE"
            '';
          }
          // {
            ui.app.expose = false;
          };

        mfm-portfolio-snapshot-exec =
          mkCommandTask {
            id = "task.mfm.portfolio.snapshot.exec";
            appName = "mfm-portfolio-snapshot-exec";
            kind = "internal";
            summary = "Run the packaged snapshot CLI";
            description = "Internal task that runs the packaged mfm_cli binary and writes the raw JSON result file.";
            runtimeInputs = leanRuntimeInputs ++ [ conf.packages."mfm-cli" ];
            allowUnknownArgs = false;
            command = ''
              set -euo pipefail
              ${serviceSkipShellHelpers}
              if [ -w /dev/tty ]; then
                exec >/dev/tty 2>&1
              fi
              ${resolvePackagedMfmCliShell}

              if [ "$#" -ne 0 ]; then
                echo "ERROR: task.mfm.portfolio.snapshot.exec does not accept arguments" >&2
                exit 2
              fi

              if [ -z "''${MFM_SNAPSHOT_REQUEST_FILE:-}" ]; then
                echo "ERROR: MFM_SNAPSHOT_REQUEST_FILE is required for snapshot execution" >&2
                exit 1
              fi

              if [ -z "''${MFM_SNAPSHOT_RESULT_FILE:-}" ]; then
                echo "ERROR: MFM_SNAPSHOT_RESULT_FILE is required for snapshot execution" >&2
                exit 1
              fi

              if [ -z "''${POSTGRES_PORT:-}" ]; then
                echo "ERROR: POSTGRES_PORT is not set for snapshot execution" >&2
                exit 1
              fi

              if is_service_skipped helios && [ -z "''${MFM_EVM_RPC_SOURCES_JSON:-}" ]; then
                echo "ERROR: MFM_EVM_RPC_SOURCES_JSON is required when SKIP_HELIOS=1" >&2
                exit 1
              fi
              if [ -z "''${MFM_EVM_RPC_SOURCES_JSON:-}" ] && [ -z "''${HELIOSRPC_PORT:-}" ]; then
                echo "ERROR: either HELIOSRPC_PORT or MFM_EVM_RPC_SOURCES_JSON is required for snapshot execution" >&2
                exit 1
              fi

              echo "INFO: snapshot exec preparing packaged mfm_cli request_file=$MFM_SNAPSHOT_REQUEST_FILE result_file=$MFM_SNAPSHOT_RESULT_FILE"
              mfm_cli_bin="$(resolve_packaged_mfm_cli_binary)" || exit 1

              export DATABASE_URL="postgresql://postgres:postgres@127.0.0.1:$POSTGRES_PORT/${
                conf.modules.postgres.database or "mfm"
              }"
              if [ -z "''${MFM_EVM_RPC_SOURCES_JSON:-}" ]; then
                if [ -z "''${HELIOSRPC_PORT:-}" ]; then
                  echo "ERROR: HELIOSRPC_PORT is required when MFM_EVM_RPC_SOURCES_JSON is not set" >&2
                  exit 1
                fi
                export HELIOS_RPC_PORT="$HELIOSRPC_PORT"
                export MFM_EVM_RPC_SOURCES_JSON="[{\"id\":\"helios_local\",\"network_id\":\"ethereum-mainnet\",\"rpc_url\":\"http://127.0.0.1:$HELIOS_RPC_PORT\",\"kind\":\"local\"}]"
                export MFM_EVM_RPC_PREFERRED_ORDER="helios_local"
              fi

              echo "INFO: launching packaged mfm_cli portfolio snapshot request_file=$MFM_SNAPSHOT_REQUEST_FILE rpc_sources=helios_local"
              "$mfm_cli_bin" --output-format json portfolio snapshot --request-file "$MFM_SNAPSHOT_REQUEST_FILE" >"$MFM_SNAPSHOT_RESULT_FILE"
            '';
          }
          // {
            ui.app.expose = false;
          };

        mfm-portfolio-snapshot-services-stop =
          mkCommandTask {
            id = "task.mfm.portfolio.snapshot.services-stop";
            appName = "mfm-portfolio-snapshot-services-stop";
            kind = "internal";
            summary = "Stop snapshot workflow services";
            description = "Internal task that delegates to the portfolio snapshot service teardown task.";
            runtimeInputs = leanRuntimeInputs;
            allowUnknownArgs = false;
            command = ''
              set -euo pipefail
              if [ -w /dev/tty ]; then
                exec >/dev/tty 2>&1
              fi

              if [ "$#" -ne 0 ]; then
                echo "ERROR: task.mfm.portfolio.snapshot.services-stop does not accept arguments" >&2
                exit 2
              fi

              if [ -z "''${MFM_SNAPSHOT_HANDOFF_FILE:-}" ]; then
                echo "ERROR: MFM_SNAPSHOT_HANDOFF_FILE is required for snapshot service teardown" >&2
                exit 1
              fi

              if [ -z "''${NIXFIED_EXECUTOR_SELF:-}" ]; then
                echo "ERROR: NIXFIED_EXECUTOR_SELF is required for snapshot service teardown" >&2
                exit 1
              fi

              NIXFIED_CALLER_PWD="$PWD" "$NIXFIED_EXECUTOR_SELF" run-task task.mfm.portfolio.services-stop --handoff-file "$MFM_SNAPSHOT_HANDOFF_FILE"
            '';
          }
          // {
            ui.app.expose = false;
          };

        mfm-portfolio-snapshot = mkCommandTask {
          id = "task.mfm.portfolio.snapshot";
          appName = "mfm-portfolio-snapshot-internal";
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
            "MFM_ENV=dev HELIOS_NETWORK=mainnet SERVICE_REUSE_POLICY=same-slot nix run .#mfm::portfolio::snapshot -- ./portfolio-request.json"
          ];
          runtimeInputs = leanRuntimeInputs;
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
            ${serviceSkipShellHelpers}

            # Reserve stdout for the final JSON payload and route progress logs to
            # the controlling terminal when available, because nested nixfied task
            # stderr is otherwise swallowed by the app launcher.
            exec 3>&1
            if [ -w /dev/tty ]; then
              exec 4>/dev/tty
            else
              exec 4>&2
            fi
            exec 1>&4
            exec 2>&4

            if [ "''${1:-}" = "--help" ] || [ "''${1:-}" = "-h" ]; then
              echo "usage: nix run .#mfm::portfolio::snapshot -- <REQUEST_FILE>" >&2
              exit 0
            fi

            if [ "$#" -ne 1 ]; then
              echo "usage: nix run .#mfm::portfolio::snapshot -- <REQUEST_FILE>" >&2
              exit 2
            fi

            if [ -z "''${POSTGRES_PORT:-}" ]; then
              echo "ERROR: POSTGRES_PORT is not set for snapshot" >&2
              exit 1
            fi

            if ! is_service_skipped helios; then
              if [ -z "''${HELIOSRPC_PORT:-}" ]; then
                echo "ERROR: HELIOSRPC_PORT is not set for managed snapshot services" >&2
                exit 1
              fi
            fi

            if [ -z "''${NIXFIED_EXECUTOR_SELF:-}" ]; then
              echo "ERROR: NIXFIED_EXECUTOR_SELF is not available in the snapshot runtime" >&2
              exit 1
            fi

            if ! is_service_skipped helios; then
              export HELIOS_NETWORK="''${HELIOS_NETWORK:-mainnet}"
              if [ "$HELIOS_NETWORK" != "mainnet" ]; then
                echo "ERROR: HELIOS_NETWORK must be 'mainnet' for mfm::portfolio::snapshot" >&2
                exit 1
              fi
            fi

            REQUEST_FILE="$1"
            if [ "''${REQUEST_FILE#/}" = "$REQUEST_FILE" ]; then
              REQUEST_FILE="$PWD/$REQUEST_FILE"
            fi
            session_dir="$(mktemp -d "''${TMPDIR:-/tmp}/mfm-portfolio-snapshot.XXXXXX")"
            handoff_file="$session_dir/services.json"
            out_file="$session_dir/result.json"
            snapshot_runtime_dir_base="''${MFM_SNAPSHOT_RUNTIME_DIR_BASE:-}"
            if [ -z "$snapshot_runtime_dir_base" ]; then
              snapshot_user_name="$(id -un 2>/dev/null || true)"
              if [ -n "$snapshot_user_name" ]; then
                snapshot_user_home="$(eval printf '%s' "~$snapshot_user_name")"
                if [ -n "$snapshot_user_home" ] && [ "$snapshot_user_home" != "~$snapshot_user_name" ]; then
                  snapshot_runtime_dir_base="$snapshot_user_home/.local/share/${project.id}"
                fi
              fi
            fi
            export MFM_SNAPSHOT_REQUEST_FILE="$REQUEST_FILE"
            export MFM_SNAPSHOT_HANDOFF_FILE="$handoff_file"
            export MFM_SNAPSHOT_RESULT_FILE="$out_file"
            export MFM_SNAPSHOT_RUNTIME_DIR_BASE="$snapshot_runtime_dir_base"

            cleanup() {
              local rc=$?
              if [ -n "''${NIXFIED_EXECUTOR_SELF:-}" ] && [ -n "''${MFM_SNAPSHOT_HANDOFF_FILE:-}" ]; then
                NIXFIED_CALLER_PWD="$PWD" "$NIXFIED_EXECUTOR_SELF" run-task task.mfm.portfolio.snapshot.services-stop >/dev/null 2>&1 || true
              fi
              rm -rf "$session_dir" 2>/dev/null || true
              return "$rc"
            }
            trap cleanup EXIT
            trap 'exit 130' INT
            trap 'exit 143' TERM

            echo "INFO: snapshot workflow bootstrap request_file=$REQUEST_FILE postgres_port=$POSTGRES_PORT heliosrpc_port=$HELIOSRPC_PORT reuse=''${SERVICE_REUSE_POLICY:-default}"
            echo "INFO: snapshot workflow session_dir=$session_dir handoff_file=$handoff_file result_file=$out_file"
            echo "INFO: snapshot workflow phases: preflight -> services-start/reuse -> snapshot.exec -> services-stop"
            NIXFIED_CALLER_PWD="$PWD" "$NIXFIED_EXECUTOR_SELF" run-workflow workflow.mfm.portfolio.snapshot

            echo "INFO: snapshot workflow finished; validating JSON envelope"
            if ! ${pkgs.jq}/bin/jq -e '
              .status == "success"
              and .data.feature_id == "portfolio.snapshot"
              and (.data | has("result"))
            ' "$out_file" >/dev/null; then
              echo "ERROR: snapshot output did not match the expected JSON envelope" >&2
              cat "$out_file" >&2 || true
              exit 1
            fi

            cat "$out_file" >&3
          '';
        };

        mfm_cli = mkCommandTask {
          id = "task.mfm_cli";
          appName = "mfm_cli";
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
          appName = "mfm_rest_api";
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
          appName = "build";
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
          appName = "check";
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
          appName = "publish-docs";
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
            ${cargoWorkspaceTargetPreamble}

            echo "INFO: running workspace tests"
            ${cargoNextestWorkspaceCiCmd}
            echo "OK: tests completed"
          '';
        };

        ci = mkCommandTask {
          id = "task.ci";
          appName = "ci-internal";
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
            env = ciCargoRustEnv;
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
            env = ciCargoRustEnv;
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
                test -f "$ROOT/nixfied/project/model-introspection.nix"

                jq -e "." "$ROOT/nixfied/schemas/task-contract.json" >/dev/null
                jq -e "." "$ROOT/nixfied/schemas/workflow-contract.json" >/dev/null
                jq -e "." "$ROOT/nixfied/schemas/model-export.json" >/dev/null

                model_rc=0
                ${pkgs.coreutils}/bin/timeout --signal=TERM --kill-after=10s ${toString ciShellAppContractsTimeoutSec} \
                  nix run --impure "path:$ROOT"#model >/dev/null || model_rc="$?"
                if [ "$model_rc" -ne 0 ]; then
                  if [ "$model_rc" -eq 124 ]; then
                    echo "ERROR: model export timed out after ${toString ciShellAppContractsTimeoutSec}s"
                  fi
                  exit "$model_rc"
                fi

                MODEL_INFO_JSON="$(nix eval --impure --json --file "$ROOT/nixfied/project/model-introspection.nix")"
                SYSTEM="$(jq -r ".system" <<<"$MODEL_INFO_JSON")"
                APPS_JSON="$(nix eval --json "path:$ROOT#apps.$SYSTEM")"

                require_app() {
                  local app_name="$1"
                  if ! jq -e --arg name "$app_name" "has(\$name) and .[\$name].type == \"app\"" <<<"$APPS_JSON" >/dev/null; then
                    echo "ERROR: missing required app surface app=$app_name system=$SYSTEM"
                    exit 1
                  fi
                }

                require_task() {
                  local task_id="$1"
                  if ! jq -e --arg id "$task_id" ".taskIds | index(\$id) != null" <<<"$MODEL_INFO_JSON" >/dev/null; then
                    echo "ERROR: missing required compiled task id=$task_id"
                    exit 1
                  fi
                }

                require_workflow() {
                  local workflow_id="$1"
                  if ! jq -e --arg id "$workflow_id" ".workflowIds | index(\$id) != null" <<<"$MODEL_INFO_JSON" >/dev/null; then
                    echo "ERROR: missing required compiled workflow id=$workflow_id"
                    exit 1
                  fi
                }

                require_workflow_plan_task() {
                  local workflow_id="$1"
                  local task_id="$2"
                  if ! jq -e --arg workflow "$workflow_id" --arg task "$task_id" "(.workflowPlanTaskIds[\$workflow] // []) | index(\$task) != null" <<<"$MODEL_INFO_JSON" >/dev/null; then
                    echo "ERROR: workflow plan missing task workflow=$workflow_id task=$task_id"
                    exit 1
                  fi
                }

                require_app "mfm_cli"
                require_app "mfm::portfolio::snapshot"
                require_app "mfm_rest_api"

                require_task "task.ci"
                require_task "task.ci.services-start"
                require_task "task.ci.services-stop"
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
            env = ciCargoRustEnv;
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
            description = "Boots local postgres/minio/reth/helios dependencies for deterministic parity checks.";
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

              echo "INFO: starting ci services env=$env_value slot=$slot_value postgres=$POSTGRES_PORT minio=$MINIO_API_PORT reth=$RETH_HTTP_PORT helios=$HELIOS_RPC_PORT"
              skip_helios=0
              if [ -n "''${SKIP_HELIOS+x}" ]; then
                skip_helios=1
                echo "SKIP: starting/parity checks for helios are disabled (SKIP_HELIOS=1)"
              fi

              services_root="$artifacts_dir/services"
              mkdir -p "$services_root"

              run_ready_task() {
                local service="$1"
                local source_key="$2"
                ${project.envVar}="$env_value" ${project.slotVar}="$slot_value" \
                  nix run --impure .#ready -- --service "$service" --source "$source_key"
              }

              wait_for_ready_task() {
                local service="$1"
                local timeout_secs="$2"
                local interval_secs="$3"
                local source_key="''${4:-local}"
                local ready_log="$artifacts_dir/''${service}-ready.log"
                local start_ts
                local now_ts

                case "$timeout_secs" in
                  *[!0-9]*|"")
                    echo "ERROR: timeout for service '$service' must be integer seconds (got '$timeout_secs')" >&2
                    return 1
                    ;;
                esac

                case "$interval_secs" in
                  *[!0-9.]*|""|*.*.*|.*|*.)
                    echo "ERROR: interval for service '$service' must be a positive number (got '$interval_secs')" >&2
                    return 1
                    ;;
                esac

                start_ts=$(date +%s)

                while true; do
                  if run_ready_task "$service" "$source_key" >"$ready_log" 2>&1; then
                    return 0
                  fi

                  now_ts=$(date +%s)
                  if [ $((now_ts - start_ts)) -ge "$timeout_secs" ]; then
                    echo "ERROR: service '$service' failed readiness checks after $timeout_secs s" >&2
                    if [ -f "$ready_log" ]; then
                      tail -50 "$ready_log" >&2 || true
                    fi
                    return 1
                  fi

                  sleep "$interval_secs"
                done
              }

              postgres_root="$services_root/postgres"
              postgres_data="$postgres_root/data"
              postgres_run="$postgres_root/run"
              postgres_log="$artifacts_dir/postgres-service.log"
              mkdir -p "$postgres_data" "$postgres_run"

              if ! ${postgresPackage}/bin/pg_isready -U postgres -h 127.0.0.1 -p "$POSTGRES_PORT" -q 2>/dev/null; then
                if [ ! -f "$postgres_data/PG_VERSION" ]; then
                  if ! ${postgresPackage}/bin/initdb -D "$postgres_data" -U postgres --no-locale --encoding=UTF8 -A trust >/dev/null; then
                    echo "ERROR: postgres initdb failed port=$POSTGRES_PORT data=$postgres_data"
                    if [ -f "$postgres_log" ]; then
                      tail -50 "$postgres_log" >&2 || true
                    fi
                    exit 1
                  fi
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

                if ! ${postgresPackage}/bin/pg_ctl -D "$postgres_data" -l "$postgres_log" -o "-p $POSTGRES_PORT -h 127.0.0.1 -k $postgres_run" start; then
                  echo "ERROR: postgres failed to start port=$POSTGRES_PORT data=$postgres_data"
                  if [ -f "$postgres_log" ]; then
                    tail -50 "$postgres_log" >&2 || true
                  fi
                  if command -v lsof >/dev/null 2>&1; then
                    lsof -nP -iTCP:"$POSTGRES_PORT" -sTCP:LISTEN >&2 || true
                  fi
                  exit 1
                fi
              fi

              for _ in $(seq 1 120); do
                if ${postgresPackage}/bin/pg_isready -U postgres -h 127.0.0.1 -p "$POSTGRES_PORT" -q 2>/dev/null; then
                  break
                fi
                sleep 0.25
              done

              if ! ${postgresPackage}/bin/pg_isready -U postgres -h 127.0.0.1 -p "$POSTGRES_PORT" -q 2>/dev/null; then
                echo "ERROR: postgres failed to become ready port=$POSTGRES_PORT"
                if [ -f "$postgres_log" ]; then
                  tail -50 "$postgres_log" >&2 || true
                fi
                if command -v lsof >/dev/null 2>&1; then
                  lsof -nP -iTCP:"$POSTGRES_PORT" -sTCP:LISTEN >&2 || true
                fi
                exit 1
              fi

              ${postgresPackage}/bin/createdb -h 127.0.0.1 -p "$POSTGRES_PORT" -U postgres mfm >/dev/null 2>&1 || true
              ${postgresPackage}/bin/createdb -h 127.0.0.1 -p "$POSTGRES_PORT" -U postgres mfm_test >/dev/null 2>&1 || true

              if ! wait_for_ready_task "postgres" "30" "1" "local"; then
                echo "ERROR: postgres failed framework readiness checks port=$POSTGRES_PORT" >&2
                if [ -f "$postgres_log" ]; then
                  tail -50 "$postgres_log" >&2 || true
                fi
                if command -v lsof >/dev/null 2>&1; then
                  lsof -nP -iTCP:"$POSTGRES_PORT" -sTCP:LISTEN >&2 || true
                fi
                exit 1
              fi

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

              if ! wait_for_ready_task "minio" "45" "1" "local"; then
                echo "ERROR: minio failed to become ready port=$MINIO_API_PORT" >&2
                if [ -f "$minio_log" ]; then
                  tail -50 "$minio_log" >&2 || true
                fi
                if command -v lsof >/dev/null 2>&1; then
                  lsof -nP -iTCP:"$MINIO_API_PORT" -sTCP:LISTEN >&2 || true
                  lsof -nP -iTCP:"$MINIO_CONSOLE_PORT" -sTCP:LISTEN >&2 || true
                fi
                exit 1
              fi

              if ! ${minioClientPackage}/bin/mc alias set ci "http://127.0.0.1:$MINIO_API_PORT" "$AWS_ACCESS_KEY_ID" "$AWS_SECRET_ACCESS_KEY" >/dev/null; then
                echo "ERROR: failed to configure minio alias endpoint=http://127.0.0.1:$MINIO_API_PORT"
                if [ -f "$minio_log" ]; then
                  tail -50 "$minio_log" >&2 || true
                fi
                if command -v lsof >/dev/null 2>&1; then
                  lsof -nP -iTCP:"$MINIO_API_PORT" -sTCP:LISTEN >&2 || true
                fi
                exit 1
              fi

              if ! ${minioClientPackage}/bin/mc mb --ignore-existing "ci/$MFM_S3_BUCKET" >/dev/null; then
                echo "ERROR: failed to ensure minio bucket bucket=$MFM_S3_BUCKET"
                if [ -f "$minio_log" ]; then
                  tail -50 "$minio_log" >&2 || true
                fi
                if command -v lsof >/dev/null 2>&1; then
                  lsof -nP -iTCP:"$MINIO_API_PORT" -sTCP:LISTEN >&2 || true
                fi
                exit 1
              fi

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
                "http://127.0.0.1:$RETH_HTTP_PORT" 2>/dev/null \
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

              if ! wait_for_ready_task "reth" "60" "1" "local"; then
                echo "ERROR: reth failed to become ready port=$RETH_HTTP_PORT" >&2
                if [ -f "$reth_log" ]; then
                  tail -50 "$reth_log" >&2 || true
                fi
                if command -v lsof >/dev/null 2>&1; then
                  lsof -nP -iTCP:"$RETH_HTTP_PORT" -sTCP:LISTEN >&2 || true
                  lsof -nP -iTCP:"$RETH_WS_PORT" -sTCP:LISTEN >&2 || true
                  lsof -nP -iTCP:"$RETH_AUTH_PORT" -sTCP:LISTEN >&2 || true
                fi
                exit 1
              fi

              if [ "$skip_helios" = "0" ]; then
                helios_root="$services_root/helios"
                helios_data="$helios_root/data"
                helios_log="$artifacts_dir/helios-service.log"
                helios_pid_file="$helios_root/helios.pid"
                mkdir -p "$helios_data"

                HELIOS_EXECUTION_RPC_URL_VALUE="http://127.0.0.1:$RETH_HTTP_PORT"
                export HELIOS_EXECUTION_RPC_URL="$HELIOS_EXECUTION_RPC_URL_VALUE"

                if ! ${pkgs.curl}/bin/curl -fsS --max-time 2 \
                  -H 'content-type: application/json' \
                  --data '{"jsonrpc":"2.0","id":1,"method":"eth_blockNumber","params":[]}' \
                  "http://127.0.0.1:$HELIOS_RPC_PORT" 2>/dev/null \
                  | ${pkgs.gnugrep}/bin/grep -q '"result"'; then
                  if [ -f "$helios_pid_file" ]; then
                    stale_pid="$(cat "$helios_pid_file" 2>/dev/null || true)"
                    if [ -n "$stale_pid" ] && ! kill -0 "$stale_pid" 2>/dev/null; then
                      rm -f "$helios_pid_file"
                    fi
                  fi

                  # Clear a stale listener on the Helios RPC port (for example from
                  # a previous failed CI run that did not own this pid file).
                  if command -v lsof >/dev/null 2>&1; then
                    stale_listener_pid="$(lsof -t -nP -iTCP:"$HELIOS_RPC_PORT" -sTCP:LISTEN 2>/dev/null | head -n1 || true)"
                    if [ -n "$stale_listener_pid" ]; then
                      kill "$stale_listener_pid" 2>/dev/null || true
                      for _ in $(seq 1 40); do
                        if ! kill -0 "$stale_listener_pid" 2>/dev/null; then
                          break
                        fi
                        sleep 0.25
                      done
                      kill -KILL "$stale_listener_pid" 2>/dev/null || true
                    fi
                  fi

                  if [ -x "${pkgs.util-linux}/bin/setsid" ]; then
                    "${pkgs.util-linux}/bin/setsid" \
                      ${pkgs.python3}/bin/python3 \
                      ${ciHeliosShimScript} \
                      "$HELIOS_RPC_PORT" \
                      "$HELIOS_EXECUTION_RPC_URL_VALUE" \
                      </dev/null >"$helios_log" 2>&1 &
                  elif command -v setsid >/dev/null 2>&1; then
                    setsid \
                      ${pkgs.python3}/bin/python3 \
                      ${ciHeliosShimScript} \
                      "$HELIOS_RPC_PORT" \
                      "$HELIOS_EXECUTION_RPC_URL_VALUE" \
                      </dev/null >"$helios_log" 2>&1 &
                  elif command -v nohup >/dev/null 2>&1; then
                    nohup \
                      ${pkgs.python3}/bin/python3 \
                      ${ciHeliosShimScript} \
                      "$HELIOS_RPC_PORT" \
                      "$HELIOS_EXECUTION_RPC_URL_VALUE" \
                      </dev/null >"$helios_log" 2>&1 &
                  else
                    ${pkgs.python3}/bin/python3 ${ciHeliosShimScript} "$HELIOS_RPC_PORT" "$HELIOS_EXECUTION_RPC_URL_VALUE" </dev/null >"$helios_log" 2>&1 &
                  fi
                  echo "$!" > "$helios_pid_file"
                fi

                if ! wait_for_ready_task "helios" "120" "1" "local"; then
                  echo "ERROR: helios failed to become ready port=$HELIOS_RPC_PORT execution_rpc=$HELIOS_EXECUTION_RPC_URL_VALUE" >&2
                  if [ -f "$helios_log" ]; then
                    tail -50 "$helios_log" >&2 || true
                  fi
                  if command -v lsof >/dev/null 2>&1; then
                    lsof -nP -iTCP:"$HELIOS_RPC_PORT" -sTCP:LISTEN >&2 || true
                  fi
                  exit 1
                fi
              fi

              health_log="$artifacts_dir/services-health.log"
              if [ "$skip_helios" = "1" ]; then
                for service in postgres minio reth; do
                  if ! ${project.envVar}="$env_value" ${project.slotVar}="$slot_value" \
                    nix run --impure .#health -- --service "$service" >"$health_log" 2>&1; then
                    echo "ERROR: framework health checks failed for ci service=$service" >&2
                    if [ -f "$health_log" ]; then
                      tail -50 "$health_log" >&2 || true
                    fi
                    exit 1
                  fi
                done
              else
                if ! ${project.envVar}="$env_value" ${project.slotVar}="$slot_value" \
                  nix run --impure .#health -- --service all >"$health_log" 2>&1; then
                  echo "ERROR: framework health checks failed for ci services" >&2
                  if [ -f "$health_log" ]; then
                    tail -50 "$health_log" >&2 || true
                  fi
                  exit 1
                fi
              fi

              if [ "$skip_helios" = "1" ]; then
                echo "OK: ci services ready postgres=$POSTGRES_PORT minio=$MINIO_API_PORT reth=$RETH_HTTP_PORT helios=skipped"
              else
                echo "OK: ci services ready postgres=$POSTGRES_PORT minio=$MINIO_API_PORT reth=$RETH_HTTP_PORT helios=$HELIOS_RPC_PORT"
              fi
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
            description = "Stops local postgres/minio/reth/helios service processes started for CI.";
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
              helios_pid_file="$services_root/helios/helios.pid"

              if [ -z "''${SKIP_HELIOS+x}" ] && [ -f "$helios_pid_file" ]; then
                helios_pid="$(cat "$helios_pid_file" 2>/dev/null || true)"
                if [ -n "$helios_pid" ] && kill -0 "$helios_pid" 2>/dev/null; then
                  kill "$helios_pid" 2>/dev/null || true
                  for _ in $(seq 1 40); do
                    if ! kill -0 "$helios_pid" 2>/dev/null; then
                      break
                    fi
                    sleep 0.25
                  done
                  kill -KILL "$helios_pid" 2>/dev/null || true
                fi
                rm -f "$helios_pid_file"
              fi

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
            env = ciCargoRustEnv;
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
            env = ciCargoRustEnv;
            command = ''
              set -euo pipefail
              ${ciStepPreamble}

              log_file="$artifacts_dir/parity-compile.log"
              echo "INFO: running ci step=parity-compile"
              run_with_log "$log_file" ${parityNextestCmd} --no-run
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
            env = ciCargoRustEnv;
            command = ''
              set -euo pipefail
              ${ciStepPreamble}
              ${ciParityServiceEnv}

              key_log_file="$artifacts_dir/parity-keystore-reth-tx-sign-send.log"
              smoke_log_file="$artifacts_dir/parity-evm-helios-smoke.log"
              response_file="$artifacts_dir/parity-evm-helios-smoke.response.json"

              echo "INFO: running ci step=parity-evm-helios-smoke"
              if [ -n "''${SKIP_HELIOS+x}" ]; then
                echo "SKIP: parity-evm-helios-smoke skipped by SKIP_HELIOS=1"
                exit 0
              fi

              run_with_log "$key_log_file" ${parityNextestCmd} --test parity_keystore_reth_tx_send

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
            env = ciCargoRustEnv;
            command = ''
              set -euo pipefail
              ${ciStepPreamble}
              ${ciParityServiceEnv}

              log_file="$artifacts_dir/parity-evm-reth.log"
              echo "INFO: running ci step=parity-evm-reth"
              run_with_log "$log_file" ${parityNextestCmd} --jobs 1 --test evm_rpc_pool_failover --test evm_rpc_getlogs_chunking --test parity_rest_api_evm_reth_pipeline --test parity_portfolio_tracker_reth_mock_erc20 --test parity_portfolio_tracker_reth_snapshot
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
            runtimeInputs = leanRuntimeInputs ++ [ conf.packages."mfm-cli" ];
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
                conf.modules.helios.executionRpcUrl or "https://ethereum-rpc.publicnode.com"
              }}"
              export HELIOS_CONSENSUS_RPC_URL="''${HELIOS_CONSENSUS_RPC_URL:-${
                conf.modules.helios.consensusRpcUrl or "https://lodestar-mainnet.chainsafe.io"
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
          }
          // {
            ui.app.expose = false;
          };

      }
      // frameworkInstallPreset.tasks
      // frameworkTestPreset.tasks
      // frameworkSelfhostPreset.tasks;

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
          ];
          postRun = {
            tasks = [ "task.ci.services-stop" ];
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

        mfm-portfolio-snapshot = {
          id = "workflow.mfm.portfolio.snapshot";
          summary = "Internal portfolio snapshot workflow";
          description = "Bootstraps Postgres and Helios, runs the packaged snapshot CLI, and tears down owned services.";
          mode = "custom";
          maxWorkers = 1;
          units = {
            services-start = mkWorkflowUnit {
              taskId = "task.mfm.portfolio.snapshot.services-start";
            };

            snapshot-exec = mkWorkflowUnit {
              taskId = "task.mfm.portfolio.snapshot.exec";
              needs = [ "services-start" ];
            };
          };
          stages = [ ];
          preRun.tasks = [ "task.mfm.portfolio.snapshot.preflight" ];
          postRun = {
            tasks = [ "task.mfm.portfolio.snapshot.services-stop" ];
            alwaysRun = true;
          };
          artifacts = {
            root = ciArtifactsRoot;
            keepOnSuccess = false;
            keepOnFailure = true;
            writeSummary = true;
          };
          execution = {
            parallel = false;
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
