{ runtime }:
{
  "runtime.ephemeral.source-materialization" = {
    kind = "runtime";
    summary = "Ephemeral source materialization defaults";
    surfaces = [
      {
        kind = "workflow-ephemeral";
        name = "ephemeral";
      }
    ];
    ownerFiles = [
      "nixfied/project/conf.nix"
      "nixfied/project/runtime.nix"
      "nixfied/framework/runtime/ephemeral.nix"
    ];
    modelPaths = [ "runtime.ephemeral.copyMode" ];
    status = "stable";
    defaults = {
      copyMode = runtime.ephemeral.copyMode;
    };
    coverageRequired = true;
    docs = [ ];
  };

  "runtime.ephemeral.include-untracked" = {
    kind = "runtime";
    summary = "Ephemeral worktree copy policy";
    surfaces = [
      {
        kind = "workflow-ephemeral";
        name = "ephemeral";
      }
    ];
    ownerFiles = [
      "nixfied/project/conf.nix"
      "nixfied/project/runtime.nix"
      "nixfied/framework/runtime/ephemeral.nix"
    ];
    modelPaths = [ "runtime.ephemeral.includeUntracked" ];
    status = "stable";
    defaults = {
      includeUntracked = runtime.ephemeral.includeUntracked;
    };
    coverageRequired = true;
    docs = [ ];
  };

  "runtime.ephemeral.env-file-loading" = {
    kind = "runtime";
    summary = "Ephemeral host env file loading policy";
    surfaces = [
      {
        kind = "workflow-ephemeral";
        name = "ephemeral";
      }
    ];
    ownerFiles = [
      "nixfied/project/conf.nix"
      "nixfied/project/runtime.nix"
      "nixfied/framework/runtime/ephemeral.nix"
    ];
    modelPaths = [
      "runtime.ephemeral.envFileMode"
      "runtime.ephemeral.envFilePath"
    ];
    status = "stable";
    defaults = {
      envFileMode = runtime.ephemeral.envFileMode;
      envFilePath = runtime.ephemeral.envFilePath;
    };
    coverageRequired = true;
    docs = [ ];
  };

  "runtime.registry.isolation" = {
    kind = "runtime";
    summary = "Run-scoped registry isolation";
    surfaces = [
      {
        kind = "dispatcher";
        name = "run-task/run-workflow";
      }
    ];
    ownerFiles = [
      "nixfied/framework/runtime/orchestrator.nix"
      "nixfied/framework/runtime/env-sandbox.nix"
    ];
    modelPaths = [ "state.registry.root" ];
    status = "stable";
    defaults = {
      registryRoot = null;
    };
    coverageRequired = true;
    docs = [ ];
  };

  "runtime.output.prefix-contract" = {
    kind = "runtime";
    summary = "Stable ASCII log prefix contract";
    surfaces = [
      {
        kind = "cli";
        name = "all-user-facing-output";
      }
    ];
    ownerFiles = [
      "nixfied/framework/runtime/helpers/helpers.nix"
      "nixfied/project/tasks.nix"
    ];
    modelPaths = [ ];
    status = "stable";
    defaults = {
      prefixes = [
        "INFO:"
        "WARN:"
        "ERROR:"
        "OK:"
        "SKIP:"
      ];
    };
    coverageRequired = true;
    docs = [ ];
  };
}
