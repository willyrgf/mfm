{
  pkgs,
  canonical,
  compiledCore,
}:
let
  mkShellApp = import ./mk-shell-app.nix { inherit pkgs; };

  taskIds = builtins.sort builtins.lessThan (builtins.attrNames compiledCore.model.tasks);
  serviceIds = builtins.sort builtins.lessThan (builtins.attrNames compiledCore.model.serviceCatalog);
  featureIds = builtins.sort builtins.lessThan (builtins.attrNames compiledCore.model.features);

  modelCanonical = canonical.toCanonicalNix compiledCore.model;
  tasksTable = builtins.concatStringsSep "\n" (
    map (taskId: "${taskId}\t${compiledCore.model.tasks.${taskId}.summary}") taskIds
  );
  servicesTable = builtins.concatStringsSep "\n" (
    map (
      serviceId:
      let
        service = compiledCore.model.serviceCatalog.${serviceId};
      in
      "${service.id}\t${service.name}\t${if service.enable then "enabled" else "disabled"}"
    ) serviceIds
  );
  featuresTable = builtins.concatStringsSep "\n" (
    map (
      featureId:
      let
        feature = compiledCore.model.features.${featureId};
        coverageFlag = if feature.coverageRequired or false then "required" else "optional";
      in
      "${feature.id}\t${feature.kind}\t${coverageFlag}\t${feature.summary}"
    ) featureIds
  );
  helpText = builtins.concatStringsSep "\n" (compiledCore.model.views.help.lines or [ ]);
  docsText = builtins.concatStringsSep "\n" (compiledCore.model.views.docs.lines or [ ]);
  featureText = builtins.concatStringsSep "\n" (compiledCore.model.views.features.lines or [ ]);

  taskSchema = builtins.fromJSON (builtins.readFile ../../schemas/task-contract.json);
  workflowSchema = builtins.fromJSON (builtins.readFile ../../schemas/workflow-contract.json);
  modelSchema = builtins.fromJSON (builtins.readFile ../../schemas/model-export.json);

  schemaBundle = builtins.toJSON {
    task = taskSchema;
    workflow = workflowSchema;
    model = modelSchema;
  };

  schemaBundleFile = pkgs.writeText "nixfied-schema-bundle.json" "${schemaBundle}\n";
  helpFile = pkgs.writeText "nixfied-help.txt" "${helpText}\n";
  docsFile = pkgs.writeText "nixfied-docs.md" "${docsText}\n";
  featureFile = pkgs.writeText "nixfied-features.txt" "${featureText}\n";

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
          canonical.toCanonicalNix compiledCore.model.tasks.${taskId}
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
    help = mkShellApp {
      appName = "help";
      binPrefix = "nixfied-introspect";
      body = ''
        cat ${helpFile}
      '';
    };

    docs = mkShellApp {
      appName = "docs";
      binPrefix = "nixfied-introspect";
      body = ''
        cat ${docsFile}
      '';
    };

    features = mkShellApp {
      appName = "features";
      binPrefix = "nixfied-introspect";
      body = ''
        cat ${featureFile}
      '';
    };

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
        echo ${pkgs.lib.escapeShellArg compiledCore.stateHash}
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

  packages = {
    help = helpFile;
    docs = docsFile;
    model = pkgs.writeText "nixfied-model.nix" "${modelCanonical}\n";
    stateHash = pkgs.writeText "nixfied-state-hash.txt" "${compiledCore.stateHash}\n";
    tasks = pkgs.writeText "nixfied-tasks.txt" "${tasksTable}\n";
    services = pkgs.writeText "nixfied-services.txt" "${servicesTable}\n";
    features = pkgs.writeText "nixfied-features.txt" "${featuresTable}\n";
    schema = schemaDir;
  }
  // builtins.listToAttrs (
    map (taskId: {
      name = "task::${taskId}";
      value = pkgs.writeText "task-spec-${builtins.substring 0 10 (builtins.hashString "sha256" taskId)}.nix" "${
        canonical.toCanonicalNix compiledCore.model.tasks.${taskId}
      }\n";
    }) taskIds
  )
  // compiledCore.resolved.packages;

  checks = {
    model-hash-stable =
      assert canonical.hashCanonical compiledCore.model == compiledCore.stateHash;
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
      ++ compiledCore.resolved.tooling.devShellPackages;

      shellHook = compiledCore.resolved.tooling.devShellHook;
    };
  };
in
{
  apps = introspectionApps;
  inherit
    packages
    checks
    devShells
    schemaBundle
    schemaDir
    ;
  schema = {
    bundle = schemaBundle;
    directory = schemaDir;
  };
}
