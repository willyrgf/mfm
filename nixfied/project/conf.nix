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

  contractArtifactTool = pkgs.writeShellScriptBin "mfm-contract-artifact-configurable-counter" ''
        set -euo pipefail

        tmp="$(${pkgs.coreutils}/bin/mktemp -d)"
        cleanup() { ${pkgs.coreutils}/bin/rm -rf "$tmp"; }
        trap cleanup EXIT

        ${pkgs.coreutils}/bin/mkdir -p "$tmp/src"
        ${pkgs.coreutils}/bin/cp "${../../contracts/src/ConfigurableCounter.sol}" "$tmp/src/ConfigurableCounter.sol"

        cat > "$tmp/foundry.toml" <<'EOF'
    [profile.default]
    src = "src"
    out = "out"
    libs = ["lib"]
    optimizer = true
    optimizer_runs = 200
    solc = "${pkgs.solc}/bin/solc"
    EOF

        (
          cd "$tmp"
          ${pkgs.foundry}/bin/forge build --quiet > /dev/null
        )

        artifact="$tmp/out/ConfigurableCounter.sol/ConfigurableCounter.json"
        ${pkgs.jq}/bin/jq -c '{artifact:{abi:.abi,bytecode:{object:.bytecode.object}}}' "$artifact"
  '';

  contractArtifactMockErc20Tool = pkgs.writeShellScriptBin "mfm-contract-artifact-mock-erc20" ''
        set -euo pipefail

        tmp="$(${pkgs.coreutils}/bin/mktemp -d)"
        cleanup() { ${pkgs.coreutils}/bin/rm -rf "$tmp"; }
        trap cleanup EXIT

        ${pkgs.coreutils}/bin/mkdir -p "$tmp/src"
        ${pkgs.coreutils}/bin/cp "${../../contracts/src/MockERC20.sol}" "$tmp/src/MockERC20.sol"

        cat > "$tmp/foundry.toml" <<'EOF'
    [profile.default]
    src = "src"
    out = "out"
    libs = ["lib"]
    optimizer = true
    optimizer_runs = 200
    solc = "${pkgs.solc}/bin/solc"
    EOF

        (
          cd "$tmp"
          ${pkgs.foundry}/bin/forge build --quiet > /dev/null
        )

        artifact="$tmp/out/MockERC20.sol/MockERC20.json"
        ${pkgs.jq}/bin/jq -c '{artifact:{abi:.abi,bytecode:{object:.bytecode.object}}}' "$artifact"
  '';
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
    rethHttp = 8545;
    rethWs = 8546;
    rethAuth = 8551;
    heliosRpc = 8547;
  };

  # Base data directory for per-slot/per-env state
  directories = {
    base = "\${XDG_DATA_HOME:-$HOME/.local/share}/${project.id}";
  };

  # Process-first runtime registry root used by framework process::* commands.
  process = {
    registryRoot = "/tmp/nixfied-runtime/${project.id}";
  };

  # Discovery index policy used by `nix run .#check`.
  discovery = {
    strict = true;
    refreshArg = "--refresh-discovery";
    requiredDocs = [
      "README.md"
      "AGENTS.md"
      "docs/architecture.md"
      "docs/redesign.md"
    ];
    riskAreas = [
      {
        path = "nixfied/project/conf.nix";
        risk = "Project identity, environment names, and port contract.";
        required_checks = [
          "nix run .#check"
          "nix run .#ci -- --summary"
        ];
      }
      {
        path = "nixfied/project/ci.nix";
        risk = "CI pipeline behavior and release gates.";
        required_checks = [
          "nix run .#ci -- --summary"
        ];
      }
      {
        path = "nixfied/project/quality.nix";
        risk = "Quality checks and discovery drift enforcement.";
        required_checks = [
          "nix run .#check"
        ];
      }
      {
        path = "nixfied/.framework";
        risk = "Framework internals; avoid direct edits in installed repos.";
        required_checks = [
          "nix run .#help"
        ];
      }
      {
        path = "crates/core/src/keystore";
        risk = "Security-sensitive key handling, tamper detection, and persisted keystore compatibility.";
        required_checks = [
          "nix run .#check"
          "nix run .#test"
          "nix run .#ci -- --audit --summary"
        ];
      }
      {
        path = "crates/machine";
        risk = "Recovery, replay, and deterministic state-machine runtime semantics.";
        required_checks = [
          "nix run .#check"
          "nix run .#test"
          "nix run .#ci -- --parity --summary"
        ];
      }
    ];
  };

  tooling = rec {
    runtimePackages = [
      pkgs.coreutils
      pkgs.gnused
      pkgs.git
      pkgs.lsof
      pkgs.curl
      pkgs.jq
      pkgs.cargo-nextest
      pkgs.cargo-audit
      pkgs.foundry
      pkgs.reth
      pkgs.minio
      contractArtifactTool
      contractArtifactMockErc20Tool
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
        readiness = {
          type = "http";
          host = "127.0.0.1";
          port = "\${MINIO_PORT}";
          path = "/minio/health/ready";
          initialDelaySeconds = 2;
          periodSeconds = 5;
          timeoutSeconds = 5;
          failureThreshold = 12;
        };
        env = {
          MINIO_ROOT_USER = "minio";
          MINIO_ROOT_PASSWORD = "minio123456";
        };
      };
    };
  };

  ephemeral = {
    enable = true;
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
    reth = {
      enable = true;
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
      enable = true;
      # Keep the framework wrapper default package and source the real binary
      # from HELIOS_BIN (or PATH) where Helios workflows are executed.
      portKeyRpc = "heliosRpc";
      dataDirName = "helios";
      network = "local";
      executionRpcPortKey = "rethHttp";
      # Mainnet default for workflows that do not set HELIOS_EXECUTION_RPC_URL explicitly.
      executionRpcUrl = "https://eth.drpc.org";
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
