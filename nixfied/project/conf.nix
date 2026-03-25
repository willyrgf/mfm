# Base project configuration
{
  pkgs ? null,
}:

let
  fenix = if pkgs == null then null else pkgs.fenix or null;

  stableToolchain =
    if fenix != null && fenix ? stable && fenix.stable ? toolchain then
      fenix.stable.toolchain
    else
      null;

  rustToolchainPackages =
    if stableToolchain != null then
      [ stableToolchain ]
    else if pkgs == null then
      [ ]
    else
      [
        pkgs.cargo
        pkgs.rustc
        pkgs.rustfmt
        pkgs.clippy
      ];

  aaveOriginTools = if pkgs == null then null else import ./aave-origin-tools.nix { inherit pkgs; };
  postgresLocalPackage = if pkgs != null then pkgs.postgresql_16 else null;
  nginxLocalPackage = if pkgs != null then pkgs.nginx else null;
  minioLocalPackage = if pkgs != null then pkgs.minio else null;
  minioLocalClientPackage = if pkgs != null then pkgs.minio-client else null;
  rethLocalPackage = if pkgs != null then pkgs.reth else null;
  heliosLocalPackage =
    if pkgs != null then
      pkgs.callPackage ../framework/runtime/services/helios/package.nix { }
    else
      null;
in
rec {
  project = {
    name = "MFM";
    id = "mfm";
    description = "WIP toolkit for on-chain operations";
    envVar = "MFM_ENV";
    slotVar = "NIX_ENV";
  };

  # Environment definitions and port offsets
  envs = {
    prod = {
      offset = 0;
    };
    dev = {
      offset = 10;
    };
    test = {
      offset = 20;
    };
  };

  # Slot behavior for port calculations
  slots = {
    max = 9;
    # Keep per-slot ports disjoint even when multiple port roles are adjacent.
    stride = 100;
    default = 0;
  };

  # Port roles (keys become <KEY>_PORT in slot scripts)
  ports = {
    rest_api = 3001;
    backend = 3000;
    frontend = 3100;
    http = 8080;
    https = 8443;

    postgres = 5432;

    minio = 9000;
    minio_console = 9001;
    minioApi = 9000;
    minioConsole = 9001;

    rethHttp = 8545;
    rethWs = 8546;
    rethAuth = 8551;
    rethP2p = 30303;
    heliosRpc = 8547;
  };

  # Base data directory for per-slot/per-env state
  directories = {
    base = "\${XDG_DATA_HOME:-$HOME/.local/share}/${project.id}";
  };

  logging = {
    level = "info";
    output = "stdout";
  };

  tooling = rec {
    runtimePackages =
      (
        if pkgs == null then
          [ ]
        else
          [
            pkgs.bash
            pkgs.coreutils
            pkgs.findutils
            pkgs.gnugrep
            pkgs.gnused
            pkgs.git
            pkgs.lsof
            pkgs.curl
            pkgs.jq
            pkgs.nix
            pkgs.nixfmt
            pkgs.cargo-nextest
            pkgs.cargo-audit
            pkgs.sccache
            pkgs.stdenv.cc
            pkgs.libiconv
            pkgs.pkg-config
            (if pkgs.stdenv.isDarwin then pkgs.libressl else pkgs.openssl)
            pkgs.foundry
            pkgs.reth
            pkgs.minio
            aaveOriginTools.aaveV3OriginFetchTool
            aaveOriginTools.aaveV3OriginCompileTool
            aaveOriginTools.aaveV3OriginDeployTool
          ]
      )
      ++ rustToolchainPackages;

    devShellPackages = runtimePackages;

    devShellHook = ''
      echo "MFM dev shell ready. Use: nix run .#help"
    '';

    envFile = {
      enable = true;
      strict = false;
      allow = [ ];
    };
  };

  discovery = {
    enable = true;
    strict = true;
    refreshArg = "--refresh-discovery";
    requiredDocs = [
      "README.md"
      "AGENTS.md"
      "docs/architecture.md"
      "docs/DETAILED.md"
      "docs/design.md"
      "docs/ops-and-states.md"
      "docs/UPGRADE.md"
    ];
  };

  install = {
    deps = "";
  };

  supervisor = {
    enable = true;
    services = { };
  };

  ephemeral = {
    enable = true;
    copyMode = "git-files";
    includeUntracked = true;
    excludePatterns = [
      ".git"
      ".direnv"
      "node_modules"
      ".next"
      "dist"
      ".turbo"
      ".cache"
      "target"
      "result"
      "*.log"
      "test-results"
      "coverage"
    ];
    extraDirs = [ ];
  };

  process = {
    registryRoot = "\${XDG_CACHE_HOME:-$HOME/.cache}/nixfied-runtime/${project.id}";
    artifactsRoot = "\${XDG_CACHE_HOME:-$HOME/.cache}/nixfied-artifacts-${project.id}";
    workspaceId = project.id;
  };

  # Isolation test runner configuration (nix run .#test-isolation)
  isolation = {
    enable = true;
    slots = [
      5
      7
      8
      9
    ];
    envs = [ ];
    validationInterval = 10;
    maxRuntime = 300;
    startupWait = 30;
    logsDir = "/tmp/${project.id}-isolation";
    keepLogsOnSuccess = false;
    keepLogsOnFailure = true;
    maxParallel = 12;
    useDeps = false;
    run = {
      kind = "runApp";
      app = "ci";
      args = [ "--summary" ];
    };
    validate = {
      kind = "runApp";
      app = "validate-env";
    };
    runEnv = { };
    setupActions = [ ];
    cleanupActions = [ ];
  };

  services = {
    postgres = {
      enable = true;
      database = "mfm";
      testDatabase = "mfm_test";
      portKey = "postgres";
      dataDirName = "postgres";
      extensions = [ ];
      extraConfig = "";
      envConfigs = {
        dev = { };
        prod = { };
        test = { };
      };
      migrations = {
        dir = "migrations";
        command = "";
        sourceDatabase = null;
      };
      sources.local = {
        package = postgresLocalPackage;
      };
      defaultSource = "local";
    };

    nginx = {
      enable = false;
      portKeyHttp = "http";
      portKeyHttps = "https";
      dataDirName = "nginx";
      sources.local = {
        package = nginxLocalPackage;
      };
      defaultSource = "local";
    };

    minio = {
      enable = true;
      portKeyApi = "minio";
      portKeyConsole = "minio_console";
      dataDirName = "minio";
      rootUser = "minio";
      rootPassword = "minio123456";
      browser = true;
      sources.local = {
        package = minioLocalPackage;
        clientPackage = minioLocalClientPackage;
      };
      defaultSource = "local";
    };

    reth = {
      enable = true;
      portKeyHttp = "rethHttp";
      portKeyWs = "rethWs";
      portKeyAuth = "rethAuth";
      portKeyP2p = "rethP2p";
      dataDirName = "reth";
      network = "local";
      devMode = true;
      extraArgs = [ ];
      sources.local = {
        package = rethLocalPackage;
      };
      defaultSource = "local";
    };

    helios = {
      enable = true;
      portKeyRpc = "heliosRpc";
      executionRpcPortKey = "rethHttp";
      dataDirName = "helios";
      network = "local";
      # Mainnet default for workflows that do not set HELIOS_EXECUTION_RPC_URL explicitly.
      executionRpcUrl = "https://ethereum-rpc.publicnode.com";
      # Mainnet default consensus endpoint used by Helios snapshot workflows.
      consensusRpcUrl = "https://lodestar-mainnet.chainsafe.io";
      defaultConsensusRpcUrl = "https://lodestar-mainnet.chainsafe.io";
      checkpoint = "";
      extraArgs = [ ];
      sources.local = {
        package = heliosLocalPackage;
      };
      defaultSource = "local";
      sourceKinds.local = "real";
      readiness.profile = "strict";
    };
  };

  packages =
    if pkgs == null then
      { }
    else
      {
        helios = (((services.helios.sources or { }).local or { }).package or null);
        "mfm-cli" = pkgs.rustPlatform.buildRustPackage rec {
          pname = "mfm-cli";
          version = "0.1.29";

          src = ../..;
          cargoLock.lockFile = ../../Cargo.lock;

          cargoBuildFlags = [
            "-p"
            "mfm"
            "--bin"
            "mfm_cli"
          ];

          cargoTestFlags = cargoBuildFlags;
          doCheck = false;

          nativeBuildInputs = [ pkgs.pkg-config ];
          buildInputs = if pkgs.stdenv.isDarwin then [ pkgs.libiconv ] else [ ];

          meta = with pkgs.lib; {
            description = "Packaged MFM CLI binary";
            mainProgram = "mfm_cli";
            license = licenses.mit;
            platforms = platforms.unix;
          };
        };
      };
}
