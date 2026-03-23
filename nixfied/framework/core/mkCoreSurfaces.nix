{
  pkgs,
  canonical,
  compiledCore,
}:
let
  mkShellApp = import ./mk-shell-app.nix { inherit pkgs; };

  featureIds = builtins.sort builtins.lessThan (builtins.attrNames compiledCore.model.features);
  helpText = builtins.concatStringsSep "\n" (compiledCore.model.views.help.lines or [ ]);
  docsText = builtins.concatStringsSep "\n" (compiledCore.model.views.docs.lines or [ ]);
  featureText = builtins.concatStringsSep "\n" (compiledCore.model.views.features.lines or [ ]);
  featureRows = map (
    featureId:
    let
      feature = compiledCore.model.features.${featureId};
      coverageFlag = if feature.coverageRequired or false then "required" else "optional";
    in
    "${feature.id}\t${feature.kind}\t${coverageFlag}\t${feature.summary}"
  ) featureIds;
  featuresTable = builtins.concatStringsSep "\n" featureRows;

  introspectionGraphJson = builtins.toJSON compiledCore.introspectionGraph;
  introspectionQueryScript = ./introspection-query.py;

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
  introspectionGraphFile = pkgs.writeText "nixfied-introspection-graph.json" "${introspectionGraphJson}\n";

  schemaDir = pkgs.runCommand "nixfied-schemas" { } ''
    mkdir -p "$out"
    cp ${../../schemas/task-contract.json} "$out/task-contract.json"
    cp ${../../schemas/workflow-contract.json} "$out/workflow-contract.json"
    cp ${../../schemas/model-export.json} "$out/model-export.json"
  '';

  introspectionApps = {
    help = mkShellApp {
      appName = "help";
      binPrefix = "nixfied-introspect";
      body = ''
        cat ${helpFile}
      '';
    };

    introspect = mkShellApp {
      appName = "introspect";
      binPrefix = "nixfied-introspect";
      body = ''
        exec ${pkgs.python3}/bin/python3 ${introspectionQueryScript} ${introspectionGraphFile} "$@"
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

    stateHash = mkShellApp {
      appName = "stateHash";
      binPrefix = "nixfied-introspect";
      body = ''
        echo ${pkgs.lib.escapeShellArg compiledCore.stateHash}
      '';
    };

    schema = mkShellApp {
      appName = "schema";
      binPrefix = "nixfied-introspect";
      body = ''
        cat ${schemaBundleFile}
      '';
    };
  };

  packages = {
    help = helpFile;
    docs = docsFile;
    introspectionGraph = introspectionGraphFile;
    stateHash = pkgs.writeText "nixfied-state-hash.txt" "${compiledCore.stateHash}\n";
    features = pkgs.writeText "nixfied-features.txt" "${featuresTable}\n";
    schema = schemaDir;
  }
  // compiledCore.resolved.packages;

  checks = {
    model-hash-stable =
      assert canonical.hashCanonical compiledCore.model == compiledCore.stateHash;
      pkgs.runCommand "model-hash-stable" { } ''
        echo "OK: model hash is deterministic" > "$out"
      '';

    introspection-schema = pkgs.runCommand "schema-bundle" { } ''
      ${pkgs.jq}/bin/jq -e '.task and .workflow and .model' ${schemaBundleFile} > /dev/null
      ${pkgs.jq}/bin/jq -e '.schema.kind == "nixfied-introspection-graph"' ${introspectionGraphFile} > /dev/null
      echo "OK: schema bundle and introspection graph are valid" > "$out"
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
        pkgs.python3
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
