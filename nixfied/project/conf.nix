# Base project configuration
{
  pkgs ? null,
}:

let
  fenix = pkgs.fenix or null;

  stableToolchain =
    if fenix != null && fenix ? stable && fenix.stable ? toolchain then
      fenix.stable.toolchain
    else
      null;

  # Prefer the "complete" nightly channel so rustfmt/clippy exist without rustup.
  nightlyToolchain =
    if fenix == null then
      null
    else if fenix ? complete && fenix.complete ? toolchain then
      fenix.complete.toolchain
    else if fenix ? latest && fenix.latest ? toolchain then
      fenix.latest.toolchain
    else if
      fenix ? latest
      && fenix ? combine
      && fenix.latest ? cargo
      && fenix.latest ? rustc
      && fenix.latest ? rustfmt
      && fenix.latest ? clippy
    then
      fenix.combine [
        fenix.latest.cargo
        fenix.latest.rustc
        fenix.latest.rustfmt
        fenix.latest.clippy
      ]
    else
      null;

  cargoNightly =
    if nightlyToolchain != null then
      pkgs.writeShellScriptBin "cargo-nightly" ''
        set -euo pipefail
        export PATH="${nightlyToolchain}/bin:$PATH"
        exec "${nightlyToolchain}/bin/cargo" "$@"
      ''
    else
      # Fallback when fenix isn't available: use the nixpkgs toolchain.
      # This keeps the command surface stable (`cargo-nightly`) for CI scripts.
      pkgs.writeShellScriptBin "cargo-nightly" ''
        set -euo pipefail
        exec cargo "$@"
      '';

  rustToolchainPackages =
    if stableToolchain != null then
      [ stableToolchain ]
    else
      [
        pkgs.cargo
        pkgs.rustc
        pkgs.rustfmt
        pkgs.clippy
      ];
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
    # Keep per-slot ports disjoint even when multiple port roles are adjacent
    # (e.g. MinIO data port + console port).
    stride = 100;
    # New Nixfied default-slot behavior: unset NIX_ENV resolves to this slot.
    default = 0;
  };

  # Port roles (keys become <KEY>_PORT in slot scripts)
  ports = {
    rest_api = 3001;
    postgres = 5432;
    minio = 9000;
    minio_console = 9001;
  };

  # Base data directory for per-slot/per-env state
  directories = {
    base = "\${XDG_DATA_HOME:-$HOME/.local/share}/${project.id}";
  };

  tooling = rec {
    runtimePackages = [
      pkgs.coreutils
      pkgs.gnused
      pkgs.git
      pkgs.lsof
      pkgs.cargo-nextest
      pkgs.cargo-audit
      pkgs.minio
    ]
    ++ rustToolchainPackages
    ++ [ cargoNightly ];
    devShellPackages = runtimePackages;
    devShellHook = ''
      echo "MFM dev shell ready. Use: nix run .#help"
    '';
  };

  install = {
    deps = "";
  };

  supervisor = {
    enable = true;
    services = {
      minio = {
        command = ''
          set -euo pipefail
          mkdir -p "$MINIO_STATE_DIR"
          exec ${pkgs.minio}/bin/minio server "$MINIO_STATE_DIR" \
            --address "127.0.0.1:$MINIO_PORT" \
            --console-address "127.0.0.1:$MINIO_CONSOLE_PORT"
        '';
        env = {
          MINIO_ROOT_USER = "minio";
          MINIO_ROOT_PASSWORD = "minio123456";
        };
      };
    };
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

  modules = {
    postgres = {
      enable = true;
      database = "mfm";
      testDatabase = "mfm_test";
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
      enable = true;
      portKeyApi = "minio";
      portKeyConsole = "minio_console";
      dataDirName = "minio";
      rootUser = "minio";
      rootPassword = "minio123456";
      browser = true;
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
    runApp = "ci";
    runArgs = [ "--summary" ];
    runEnv = { };
    preInstall = "";
    runCommand = "nix run path:.#ci -- --summary";
    validateCommand = "";
    cleanup = "";
  };

  services = {
    names = [ ];
    sockets = { };
  };

  packages = { };
}
