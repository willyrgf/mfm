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
    modelPaths = [ "state.policy.registryRoot" ];
    status = "stable";
    defaults = {
      registryRoot = null;
    };
    coverageRequired = true;
    docs = [ ];
  };

  "runtime.service-hooks" = {
    kind = "runtime";
    summary = "Generated service hook env vars and service operation apps";
    surfaces = [
      {
        kind = "dispatcher";
        name = "run-task/run-workflow";
      }
      {
        kind = "app";
        name = "svc::<service>::<op>";
      }
    ];
    ownerFiles = [
      "nixfied/framework/core/mkServiceRuntimeSurfaces.nix"
      "nixfied/framework/runtime/helpers/service-api.nix"
      "nixfied/framework/runtime/env-sandbox.nix"
    ];
    modelPaths = [ "services" ];
    status = "stable";
    defaults = {
      hookPrefix = "SVC_";
      appPrefix = "svc::";
    };
    coverageRequired = true;
    docs = [ ];
  };

  "runtime.service-set-surfaces" = {
    kind = "runtime";
    summary = "Grouped service-set lifecycle, export, and workflow adapter surfaces";
    surfaces = [
      {
        kind = "app";
        name = "svcset::<service-set>::<operation>";
      }
      {
        kind = "workflow-phase";
        name = "preRun.serviceSets/postRun.serviceSets";
      }
    ];
    ownerFiles = [
      "nixfied/modules/service-sets.nix"
      "nixfied/framework/core/mkServiceSetPrograms.nix"
      "nixfied/framework/core/materializeExecution.nix"
      "nixfied/compiler/compile-service-sets.nix"
      "nixfied/compiler/compile-workflows.nix"
    ];
    modelPaths = [ "serviceSets" ];
    status = "stable";
    defaults = {
      appPrefix = "svcset::";
      operations = [
        "start"
        "stop"
        "status"
        "health"
        "ready"
        "export"
      ];
    };
    coverageRequired = true;
    docs = [ ];
  };

  "runtime.app-execution-manifests" = {
    kind = "runtime";
    summary = "App-scoped execution manifests for selected launchers and machine-output wrappers";
    surfaces = [
      {
        kind = "app";
        name = "selected-app";
      }
      {
        kind = "execution";
        name = "app-manifest";
      }
    ];
    ownerFiles = [
      "nixfied/modules/apps.nix"
      "nixfied/compiler/compile-app-execution-manifests.nix"
      "nixfied/framework/core/materializeExecution.nix"
      "nixfied/framework/core/mkMachineOutputPrograms.nix"
    ];
    modelPaths = [ "apps" ];
    status = "stable";
    defaults = {
      manifestScope = "app";
      wrapperKinds = [
        "taskRef"
        "workflowRef"
        "machineOutput"
      ];
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
