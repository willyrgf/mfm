{
  adapters,
  pkgs,
  ...
}:
let
  rustToolchain = pkgs.rust-bin.stable."1.96.0".minimal.override {
    extensions = [
      "clippy"
      "rustfmt"
    ];
  };

  mfmRunner = pkgs.writeShellApplication {
    name = "mfm-nixfied-runner";
    runtimeInputs = [
      pkgs.coreutils
      pkgs.git
      pkgs.pkg-config
      rustToolchain
      pkgs.stdenv.cc
    ];
    text = ''
      command_name="''${1:?missing mfm task command}"
      shift

      state_dir=""
      while [[ $# -gt 0 ]]; do
        case "$1" in
          --state-dir)
            state_dir="''${2:?missing --state-dir value}"
            shift 2
            ;;
          *)
            echo "unknown argument for $command_name: $1" >&2
            exit 64
            ;;
        esac
      done

      if [[ -z "$state_dir" ]]; then
        echo "missing required --state-dir argument" >&2
        exit 64
      fi

      mkdir -p "$state_dir/cargo-target"
      export CARGO_TARGET_DIR="''${CARGO_TARGET_DIR:-$state_dir/cargo-target}"
      export RUST_BACKTRACE="''${RUST_BACKTRACE:-1}"

      case "$command_name" in
        fmt)
          exec cargo fmt --all -- --check
          ;;
        clippy)
          exec cargo clippy --workspace --lib --examples --tests --benches --all-features -- -D warnings
          ;;
        cargo-metadata-contract)
          exec cargo test -p mfm-integration-tests --test cargo_metadata_contract
          ;;
        architecture-namespace-contract)
          exec cargo test -p mfm-integration-tests --test architecture_namespace_contract
          ;;
        workspace-tests)
          exec cargo test --workspace
          ;;
        *)
          echo "unknown mfm task command: $command_name" >&2
          exit 64
          ;;
      esac
    '';
  };

  mfmTask =
    taskId: command:
    {
      operationId = "task.mfm.${taskId}.run";
      execId = "mfm-runner";
      args = [
        command
        "--state-dir"
        "\${stateDir}"
      ];
      logRefs = [ "task.mfm.${taskId}" ];
      summaryRefs = [ "summary" ];
    };
in
{
  # v2 currently requires at least one declared service. This placeholder is
  # unused by MFM workflows and is removed once the full CI services land.
  imports = [ adapters.synthetic ];

  nixfied.project.projectId = "mfm";
  nixfied.project.name = "MFM";
  nixfied.codebases.main.logicalRoot = ".";

  nixfied.closures.mfm-runner = {
    package = mfmRunner;
    executable = "bin/mfm-nixfied-runner";
    kind = "executable";
    requiresExecutable = true;
    operationBindings = [
      "task.mfm.fmt.run"
      "task.mfm.clippy.run"
      "task.mfm.cargo-metadata-contract.run"
      "task.mfm.architecture-namespace-contract.run"
      "task.mfm.workspace-tests.run"
    ];
    effects = [
      "process"
      "source-read"
      "file-write"
    ];
  };

  nixfied.execs.mfm-runner = {
    closureId = "mfm-runner";
    timeoutMs = 7200000;
  };

  nixfied.tasks = {
    mfm-fmt = mfmTask "fmt" "fmt";
    mfm-clippy = mfmTask "clippy" "clippy";
    mfm-cargo-metadata-contract = mfmTask "cargo-metadata-contract" "cargo-metadata-contract";
    mfm-architecture-namespace-contract =
      mfmTask "architecture-namespace-contract" "architecture-namespace-contract";
    mfm-workspace-tests = mfmTask "workspace-tests" "workspace-tests";
  };

  nixfied.workflows.check = {
    nodes = [
      {
        nodeId = "fmt";
        taskId = "mfm-fmt";
      }
      {
        nodeId = "clippy";
        taskId = "mfm-clippy";
        dependsOn = [ "fmt" ];
      }
      {
        nodeId = "cargo-metadata-contract";
        taskId = "mfm-cargo-metadata-contract";
        dependsOn = [ "clippy" ];
      }
      {
        nodeId = "architecture-namespace-contract";
        taskId = "mfm-architecture-namespace-contract";
        dependsOn = [ "cargo-metadata-contract" ];
      }
    ];
  };

  nixfied.workflows.test = {
    nodes = [
      {
        nodeId = "workspace-tests";
        taskId = "mfm-workspace-tests";
      }
    ];
  };

  nixfied.workflows.ci = {
    nodes = [
      {
        nodeId = "fmt";
        taskId = "mfm-fmt";
      }
      {
        nodeId = "clippy";
        taskId = "mfm-clippy";
        dependsOn = [ "fmt" ];
      }
      {
        nodeId = "cargo-metadata-contract";
        taskId = "mfm-cargo-metadata-contract";
        dependsOn = [ "clippy" ];
      }
      {
        nodeId = "architecture-namespace-contract";
        taskId = "mfm-architecture-namespace-contract";
        dependsOn = [ "cargo-metadata-contract" ];
      }
      {
        nodeId = "workspace-tests";
        taskId = "mfm-workspace-tests";
        dependsOn = [ "architecture-namespace-contract" ];
      }
    ];
  };
}
