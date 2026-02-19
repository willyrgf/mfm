# Base project configuration
{
  pkgs ? null,
}:

rec {
  project = {
    name = "Nixfied Project";
    id = "nixfied-project";
    description = "Reusable Nix development framework";
    envVar = "PROJECT_ENV";
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
    stride = 1;
    default = 0;
  };

  # Port roles (keys become <KEY>_PORT in slot scripts)
  ports = {
    backend = 3000;
    frontend = 3100;
    http = 8080;
    https = 8443;
    postgres = 5432;
    minioApi = 9000;
    minioConsole = 9001;
    rethHttp = 8545;
    rethWs = 8546;
    rethAuth = 8551;
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

  tooling = {
    runtimePackages = [
      pkgs.coreutils
      pkgs.gnused
    ];
    devShellPackages = [ ];
    devShellHook = ''
      echo "Nix framework dev shell ready."
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
      "CLAUDE.md"
      "ARCHITECTURE.md"
      "REDESIGN.md"
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
    enable = false;
    excludePatterns = [
      ".git"
      "node_modules"
      ".next"
      "dist"
      ".turbo"
      ".cache"
      "*.log"
      "test-results"
      "coverage"
    ];
    extraDirs = [ ];
  };

  process = {
    registryRoot = "/tmp/nixfied-runtime/${project.id}";
  };

  modules = {
    postgres = {
      enable = false;
      database = "app";
      testDatabase = "app_test";
      extensions = [ ];
      package = if pkgs != null then pkgs.postgresql_16 else null;
      portKey = "postgres";
      dataDirName = "postgres";
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
    };
    nginx = {
      enable = false;
      portKeyHttp = "http";
      portKeyHttps = "https";
      dataDirName = "nginx";
    };
    minio = {
      enable = false;
      package = if pkgs != null then pkgs.minio else null;
      clientPackage = if pkgs != null then pkgs.minio-client else null;
      portKeyApi = "minioApi";
      portKeyConsole = "minioConsole";
      dataDirName = "minio";
      rootUser = "minioadmin";
      rootPassword = "minioadmin";
      browser = true;
    };
    reth = {
      enable = false;
      package = if pkgs != null then pkgs.reth else null;
      portKeyHttp = "rethHttp";
      portKeyWs = "rethWs";
      portKeyAuth = "rethAuth";
      dataDirName = "reth";
      network = "local";
      devMode = true;
      extraArgs = [ ];
    };
    helios = {
      enable = false;
      package = if pkgs != null then pkgs.callPackage ../.framework/helios/package.nix { } else null;
      portKeyRpc = "heliosRpc";
      dataDirName = "helios";
      network = "local";
      executionRpcPortKey = "rethHttp";
      executionRpcUrl = "";
      consensusRpcUrl = "";
      checkpoint = "";
      extraArgs = [ ];
    };
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
    names = [ ];
    sockets = { };
  };

  packages = { };
}
