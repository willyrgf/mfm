{
  pkgs,
  system,
  projectRoot,
  projectModules,
  extraModules ? [ ],
  localOverrides ? [ ],
  frameworkSourceRevision ? "unknown",
}:
let
  lib = pkgs.lib;

  modules = import ../modules;

  canonical = import ./canonical.nix { inherit lib; };

  registry = import ../registry {
    inherit
      pkgs
      canonical
      ;
  };

  compiler = import ../compiler {
    inherit
      pkgs
      canonical
      modules
      system
      projectRoot
      frameworkSourceRevision
      ;
  };

  compiled = compiler.compile {
    inherit
      projectModules
      extraModules
      localOverrides
      ;
  };

  runner = import ../runner {
    inherit
      pkgs
      projectRoot
      registry
      ;
  };

  baseApps = runner.mkApps {
    model = compiled.model;
  };

  taskIds = builtins.sort builtins.lessThan (builtins.attrNames compiled.model.tasks);
  serviceIds = builtins.sort builtins.lessThan (builtins.attrNames compiled.model.services);

  modelCanonical = canonical.toCanonicalNix compiled.model;
  tasksTable = builtins.concatStringsSep "\n" (
    map (taskId: "${taskId}\t${compiled.model.tasks.${taskId}.summary}") taskIds
  );
  servicesTable = builtins.concatStringsSep "\n" (
    map (
      serviceId:
      let
        service = compiled.model.services.${serviceId};
      in
      "${service.id}\t${service.name}\t${if service.enable then "enabled" else "disabled"}"
    ) serviceIds
  );

  taskSchema = builtins.fromJSON (builtins.readFile ../schemas/task-contract.json);
  workflowSchema = builtins.fromJSON (builtins.readFile ../schemas/workflow-contract.json);
  modelSchema = builtins.fromJSON (builtins.readFile ../schemas/model-export.json);

  schemaBundle = builtins.toJSON {
    task = taskSchema;
    workflow = workflowSchema;
    model = modelSchema;
  };

  schemaBundleFile = pkgs.writeText "nixfied-schema-bundle.json" "${schemaBundle}\n";

  schemaDir = pkgs.runCommand "nixfied-schemas" { } ''
    mkdir -p "$out"
    cp ${../schemas/task-contract.json} "$out/task-contract.json"
    cp ${../schemas/workflow-contract.json} "$out/workflow-contract.json"
    cp ${../schemas/model-export.json} "$out/model-export.json"
  '';

  mkApp =
    appName: body:
    let
      suffix = builtins.substring 0 10 (builtins.hashString "sha256" appName);
      binName = "nixfied-introspect-${suffix}";
      script = pkgs.writeShellScriptBin binName ''
        set -euo pipefail
        ${body}
      '';
    in
    {
      type = "app";
      program = "${script}/bin/${binName}";
    };

  taskApps = builtins.listToAttrs (
    map (
      taskId:
      let
        taskFile = pkgs.writeText "task-${builtins.substring 0 10 (builtins.hashString "sha256" taskId)}.nix" "${
          canonical.toCanonicalNix compiled.model.tasks.${taskId}
        }\n";
      in
      {
        name = "task::${taskId}";
        value = mkApp "task::${taskId}" ''
          cat ${taskFile}
        '';
      }
    ) taskIds
  );

  introspectionApps = {
    model = mkApp "model" ''
            cat <<'NIXFIED_MODEL'
      ${modelCanonical}
      NIXFIED_MODEL
    '';

    stateHash = mkApp "stateHash" ''
      echo ${lib.escapeShellArg compiled.stateHash}
    '';

    tasks = mkApp "tasks" ''
            cat <<'NIXFIED_TASKS'
      ${tasksTable}
      NIXFIED_TASKS
    '';

    services = mkApp "services" ''
            cat <<'NIXFIED_SERVICES'
      ${servicesTable}
      NIXFIED_SERVICES
    '';

    schema = mkApp "schema" ''
      cat ${schemaBundleFile}
    '';
  }
  // taskApps;

  apps =
    baseApps
    // introspectionApps
    // {
      default = if baseApps ? help then baseApps.help else baseApps.default;
    };

  taskPackages = builtins.listToAttrs (
    map (taskId: {
      name = "task::${taskId}";
      value = pkgs.writeText "task-spec-${builtins.substring 0 10 (builtins.hashString "sha256" taskId)}.nix" "${
        canonical.toCanonicalNix compiled.model.tasks.${taskId}
      }\n";
    }) taskIds
  );

  packages = {
    default = pkgs.runCommand "nixfied-default" { } ''
      mkdir -p "$out/bin"
      ln -s ${apps.default.program} "$out/bin/default"
    '';

    model = pkgs.writeText "nixfied-model.nix" "${modelCanonical}\n";
    stateHash = pkgs.writeText "nixfied-state-hash.txt" "${compiled.stateHash}\n";
    tasks = pkgs.writeText "nixfied-tasks.txt" "${tasksTable}\n";
    services = pkgs.writeText "nixfied-services.txt" "${servicesTable}\n";
    schema = schemaDir;
  }
  // taskPackages;

  checks = {
    model-hash-stable =
      assert canonical.hashCanonical compiled.model == compiled.stateHash;
      pkgs.runCommand "model-hash-stable" { } ''
        echo "OK: model hash is deterministic" > "$out"
      '';

    introspection-schema = pkgs.runCommand "schema-bundle" { } ''
      ${pkgs.jq}/bin/jq -e '.task and .workflow and .model' ${schemaBundleFile} > /dev/null
      echo "OK: schema bundle is valid" > "$out"
    '';
  };

  devShells = {
    default = pkgs.mkShell {
      packages = [
        pkgs.coreutils
        pkgs.jq
        pkgs.nixfmt-rfc-style
        pkgs.gnugrep
        pkgs.gnused
      ]
      ++ compiled.resolved.tooling.devShellPackages;

      shellHook = compiled.resolved.tooling.devShellHook;
    };
  };
in
{
  model = compiled.model;
  stateHash = compiled.stateHash;
  tasks = compiled.model.tasks;
  services = compiled.model.services;
  workflows = compiled.model.workflows;

  apps = apps;
  packages = packages;
  checks = checks;
  devShells = devShells;

  schema = {
    bundle = schemaBundle;
    directory = schemaDir;
  };
}
