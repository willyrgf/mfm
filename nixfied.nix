{
  lib,
  nixfiedLib,
  pkgs,
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
    doc-tests = cargoLeaf {
      run = [ "cargo" "test" "--workspace" "--doc" ];
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
        "doc-tests"
      ];
    };
  };

  nixfied.surface.verbs = [ "ci" ];
}
