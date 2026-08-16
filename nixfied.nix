{
  lib,
  nixfiedLib,
  pkgs,
  adapters,
  ...
}:
let
  rustToolchain = import ./nix/rust-toolchain.nix { inherit pkgs; };
  cargoTools = [
    "rust-toolchain"
    pkgs.bash
    pkgs.coreutils
    pkgs.gawk
    pkgs.git
    pkgs.ripgrep
    pkgs.pkg-config
    "cc"
  ]
  ++ lib.optionals pkgs.stdenv.hostPlatform.isLinux [
    pkgs.bubblewrap
    "ldd"
  ]
  ++ lib.optionals pkgs.stdenv.hostPlatform.isDarwin [ pkgs.libiconv ];
  ccEnvSuffix = lib.replaceStrings [ "-" ] [ "_" ] pkgs.stdenv.hostPlatform.config;
  cargoEnv = {
    CARGO_TARGET_DIR = "target/verification";
    CARGO_INCREMENTAL = "0";
    CARGO_PROFILE_DEV_DEBUG = "1";
    CARGO_PROFILE_TEST_DEBUG = "1";
    CARGO_PROFILE_DEV_SPLIT_DEBUGINFO = "off";
    CARGO_PROFILE_TEST_SPLIT_DEBUGINFO = "off";
    CARGO_BUILD_JOBS = "2";
    RUST_BACKTRACE = "1";
    TMPDIR = "\${stateDir}";
  }
  // lib.optionalAttrs pkgs.stdenv.hostPlatform.isDarwin {
    "NIX_LDFLAGS_${ccEnvSuffix}" = "-L${pkgs.libiconv}/lib";
    CPATH = "${pkgs.libiconv}/include";
  };
  cargoLeaf =
    { run
    , env ? { }
    }:
    {
      invocation = {
        tools = cargoTools;
        run = [
          "bash"
          "-c"
          ''
            set -euo pipefail
            export CARGO_TARGET_DIR="$(pwd -P)/$CARGO_TARGET_DIR"
            exec "$@"
          ''
          "mfm-cargo"
        ] ++ run;
        env = cargoEnv // env;
        timeoutMs = 7200000;
      };
    };
in
{
  imports = [ adapters.postgres ];

  nixfied.project.projectId = "mfm";
  nixfied.project.name = "MFM";
  nixfied.codebases.main.logicalRoot = ".";

  nixfied.slotPolicy = {
    min = 0;
    default = 0;
    max = 9;
  };

  nixfied.closures.rust-toolchain = {
    package = rustToolchain;
    executable = "bin/cargo";
    effects = [ "process" "source-read" "file-write" ];
  };
  nixfied.closures.cc = {
    package = pkgs.stdenv.cc;
    executable = "bin/cc";
    effects = [ "process" ];
  };
  nixfied.closures.ldd = lib.mkIf pkgs.stdenv.hostPlatform.isLinux {
    package = pkgs.glibc.bin;
    executable = "bin/ldd";
    effects = [ "process" ];
  };

  nixfied.tasks = {
    fmt = cargoLeaf {
      run = [ "cargo" "fmt" "--all" "--" "--check" ];
    };
    clippy = cargoLeaf {
      run = [
        "cargo" "clippy" "--workspace" "--all-targets" "--all-features" "--" "-D" "warnings"
      ];
    };
    cargo-check = cargoLeaf {
      run = [ "cargo" "check" "--workspace" "--all-targets" ];
    };
    cargo-test = cargoLeaf {
      run = [ "cargo" "test" "--workspace" "--all-targets" ];
    };
    postgres-test = (cargoLeaf {
      run = [ "cargo" "test" "-p" "mfm-storage-postgres" "--features" "test-support" "--all-targets" ];
      env = {
        DATABASE_URL = "postgresql://postgres@\${host:postgres}:\${port:postgres}/postgres";
      };
    }) // {
      requires = [ "postgres" ];
    };
    test-db = {
      kind = "composite";
      steps = nixfiedLib.seq [ "postgres-test" ];
    };
    doc-tests = cargoLeaf {
      run = [ "cargo" "test" "--workspace" "--doc" ];
    };
    capacity-app = cargoLeaf {
      run = [
        "cargo" "test" "-p" "mfm-app" "maximum_portfolio_program_records_capacity_envelope" "--"
        "--nocapture"
      ];
    };
    capacity-runtime = cargoLeaf {
      run = [
        "cargo" "test" "-p" "mfm-runtime" "pure_session_advances_through_runtime_and_store" "--"
        "--nocapture"
      ];
    };
    capacity-store = cargoLeaf {
      run = [
        "cargo" "test" "-p" "mfm-store"
        "configuration_capacity_accepts_each_exact_bound_and_rejects_plus_one" "--" "--nocapture"
      ];
    };
    capacity-envelope = {
      kind = "composite";
      steps = nixfiedLib.seq [
        "capacity-app"
        "capacity-runtime"
        "capacity-store"
      ];
    };
    negative-scan = cargoLeaf {
      run = [ "bash" "scripts/check-cutover-manifest.sh" ];
    };
    ci = {
      kind = "composite";
      steps = nixfiedLib.seq [
        "fmt"
        "negative-scan"
        "clippy"
        "cargo-check"
        "cargo-test"
        "test-db"
        "doc-tests"
        "capacity-envelope"
      ];
    };
  };

  nixfied.surface.verbs = [ "ci" ];
}
