{
  pkgs,
  system,
  projectRoot,
  projectModules,
  extraModules ? [ ],
  localOverrides ? [ ],
  frameworkSourceRevision ? import ./framework-revision.nix {
    sourcePath = ../../.;
    metadataPath = ../../VENDORED.txt;
  },
}:
let
  lib = pkgs.lib;
  mkShellApp = import ./mk-shell-app.nix { inherit pkgs; };

  modules = import ../../modules;

  canonical = import ./canonical.nix { inherit lib; };

  registry = import ../runtime/registry {
    inherit
      pkgs
      ;
  };

  compiler = import ../../compiler {
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

  runner = import ../runtime {
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
  featureIds = builtins.sort builtins.lessThan (builtins.attrNames compiled.model.features);

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
  featuresTable = builtins.concatStringsSep "\n" (
    map (
      featureId:
      let
        feature = compiled.model.features.${featureId};
        coverageFlag = if feature.coverageRequired or false then "required" else "optional";
      in
      "${feature.id}\t${feature.kind}\t${coverageFlag}\t${feature.summary}"
    ) featureIds
  );

  taskSchema = builtins.fromJSON (builtins.readFile ../../schemas/task-contract.json);
  workflowSchema = builtins.fromJSON (builtins.readFile ../../schemas/workflow-contract.json);
  modelSchema = builtins.fromJSON (builtins.readFile ../../schemas/model-export.json);

  schemaBundle = builtins.toJSON {
    task = taskSchema;
    workflow = workflowSchema;
    model = modelSchema;
  };

  schemaBundleFile = pkgs.writeText "nixfied-schema-bundle.json" "${schemaBundle}\n";

  schemaDir = pkgs.runCommand "nixfied-schemas" { } ''
    mkdir -p "$out"
    cp ${../../schemas/task-contract.json} "$out/task-contract.json"
    cp ${../../schemas/workflow-contract.json} "$out/workflow-contract.json"
    cp ${../../schemas/model-export.json} "$out/model-export.json"
  '';

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
        value = mkShellApp {
          appName = "task::${taskId}";
          binPrefix = "nixfied-introspect";
          body = ''
            cat ${taskFile}
          '';
        };
      }
    ) taskIds
  );

  introspectionApps = {
    model = mkShellApp {
      appName = "model";
      binPrefix = "nixfied-introspect";
      body = ''
              cat <<'NIXFIED_MODEL'
        ${modelCanonical}
        NIXFIED_MODEL
      '';
    };

    stateHash = mkShellApp {
      appName = "stateHash";
      binPrefix = "nixfied-introspect";
      body = ''
        echo ${lib.escapeShellArg compiled.stateHash}
      '';
    };

    tasks = mkShellApp {
      appName = "tasks";
      binPrefix = "nixfied-introspect";
      body = ''
              cat <<'NIXFIED_TASKS'
        ${tasksTable}
        NIXFIED_TASKS
      '';
    };

    services = mkShellApp {
      appName = "services";
      binPrefix = "nixfied-introspect";
      body = ''
              cat <<'NIXFIED_SERVICES'
        ${servicesTable}
        NIXFIED_SERVICES
      '';
    };

    schema = mkShellApp {
      appName = "schema";
      binPrefix = "nixfied-introspect";
      body = ''
        cat ${schemaBundleFile}
      '';
    };
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
    features = pkgs.writeText "nixfied-features.txt" "${featuresTable}\n";
    schema = schemaDir;
  }
  // taskPackages
  // compiled.resolved.packages;

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
  features = compiled.model.features;

  apps = apps;
  packages = packages;
  checks = checks;
  devShells = devShells;

  schema = {
    bundle = schemaBundle;
    directory = schemaDir;
  };
}
