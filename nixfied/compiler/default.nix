{
  pkgs,
  canonical,
  modules,
  system,
  projectRoot,
  frameworkSourceRevision,
}:
let
  lib = pkgs.lib;
  contracts = import ../contracts {
    inherit
      lib
      canonical
      ;
  };

  idLib = import ./id.nix {
    inherit
      lib
      canonical
      ;
  };

  resolveModules = import ./resolve-modules.nix {
    inherit
      lib
      modules
      ;
  };

  compileStatePolicy = import ./compile-state-policy.nix { inherit lib; };
  normalizeRuntime = import ./normalize-runtime.nix { inherit lib; };
  compileServiceCatalog = import ./compile-service-catalog.nix { inherit lib; };
  compileServiceSets = import ./compile-service-sets.nix {
    inherit
      lib
      canonical
      ;
  };
  compileServiceSurfaceCatalog = import ./compile-service-surface-catalog.nix { inherit lib; };
  compileServices = import ./compile-services.nix { inherit lib; };

  compileTasks = import ./compile-tasks.nix {
    inherit
      lib
      canonical
      idLib
      ;
  };

  compileWorkflows = import ./compile-workflows.nix {
    inherit
      lib
      canonical
      idLib
      ;
  };

  compileFeatures = import ./compile-features.nix {
    inherit
      lib
      canonical
      ;
  };

  compileApps = import ./compile-apps.nix {
    inherit
      lib
      canonical
      ;
  };

  compileAppExecutionManifests = import ./compile-app-execution-manifests.nix {
    inherit
      lib
      canonical
      ;
  };

  compileIntrospectionGraph = import ./compile-introspection-graph.nix {
    inherit
      lib
      canonical
      ;
  };
  compileIntrospectionBundle = import ./compile-introspection-bundle.nix {
    inherit
      lib
      canonical
      ;
  };

  compileViews = import ./compile-views.nix { inherit lib; };
  compileSelectionIndex = import ./compile-selection-index.nix { inherit lib; };
  compileContractBundle = import ./compile-contract-bundle.nix {
    inherit
      lib
      canonical
      contracts
      ;
  };
  frameworkContractDefinitions = import ../framework/contracts/default-definitions.nix {
    contracts = contracts.types;
  };

  finalizeModel = import ./finalize-model.nix {
    inherit
      lib
      canonical
      ;
  };
in
rec {
  compileCore =
    {
      projectModules,
      extraModules ? [ ],
      localOverrides ? [ ],
    }:
    let
      resolvedModuleGraph = resolveModules {
        inherit
          pkgs
          system
          projectRoot
          frameworkSourceRevision
          projectModules
          extraModules
          localOverrides
          ;
      };

      legacyLocalDefault =
        let
          relativePath = "nixfied/local/default.nix";
          projectPath = builtins.unsafeDiscardStringContext "${builtins.toString projectRoot}/${relativePath}";
          templateContents = builtins.readFile ../local/default.nix;
          present = builtins.pathExists projectPath;
          contents = if present then builtins.readFile projectPath else "";
          customized = present && contents != templateContents;
          status =
            if !present then
              "missing"
            else if customized then
              "customized-inactive"
            else
              "template-inactive";
          message =
            if !present then
              "legacy local/default.nix is absent"
            else if customized then
              "legacy local/default.nix differs from the framework template and is not loaded by nixfied"
            else
              "legacy local/default.nix matches the framework template and is not loaded by nixfied";
        in
        {
          path = relativePath;
          inherit
            present
            customized
            status
            message
            ;
          active = false;
        };

      statePolicy = compileStatePolicy {
        inherit
          projectRoot
          ;
        resolved = resolvedModuleGraph.config;
      };

      runtime = normalizeRuntime {
        resolved = resolvedModuleGraph.config;
        inherit statePolicy;
      };

      serviceCatalog = compileServiceCatalog {
        resolved = resolvedModuleGraph.config;
      };

      serviceSets = compileServiceSets {
        inherit
          projectRoot
          serviceCatalog
          ;
        resolvedIdentity = resolvedModuleGraph.config.identity;
        baseStatePolicy = statePolicy;
        resolved = resolvedModuleGraph.config;
      };

      serviceSurfaceCatalog = compileServiceSurfaceCatalog {
        inherit serviceCatalog;
      };

      taskCompilation = compileTasks {
        resolved = resolvedModuleGraph.config;
        inherit runtime;
      };

      tasks = taskCompilation.tasks;

      workflows = compileWorkflows {
        resolved = resolvedModuleGraph.config;
        inherit tasks;
        inherit serviceSets;
        allTasks = taskCompilation.allTasks;
        declaredTaskIds = taskCompilation.declaredTaskIds;
        prunedTaskIds = taskCompilation.prunedTaskIds;
        pruneReasonsByTaskId = taskCompilation.pruneReasonsByTaskId;
      };

      contractBundle = compileContractBundle {
        resolved = resolvedModuleGraph.config;
        frameworkDefinitions = frameworkContractDefinitions;
      };

      apps = compileApps {
        resolved = resolvedModuleGraph.config;
        inherit
          serviceSets
          tasks
          workflows
          contractBundle
          ;
      };

      selectionIndex = compileSelectionIndex {
        inherit
          tasks
          workflows
          serviceCatalog
          ;
      };

      appExecutionManifests = compileAppExecutionManifests {
        resolvedIdentity = resolvedModuleGraph.config.identity;
        runtime = runtime;
        state = {
          policy = statePolicy;
          registry = {
            schemaVersion = 1;
          };
        };
        inherit
          serviceCatalog
          apps
          tasks
          workflows
          selectionIndex
          ;
      };

      features = compileFeatures {
        inherit
          projectRoot
          runtime
          apps
          tasks
          workflows
          ;
        services = serviceCatalog;
      };

      introspectionGraph = compileIntrospectionGraph {
        inherit
          projectRoot
          statePolicy
          runtime
          apps
          appExecutionManifests
          serviceSets
          tasks
          workflows
          serviceCatalog
          serviceSurfaceCatalog
          features
          selectionIndex
          ;
        resolved = resolvedModuleGraph.config;
        localOverridesActive = localOverrides != [ ];
        localOverrideCount = builtins.length localOverrides;
        inherit legacyLocalDefault;
      };

      introspectionBundle = compileIntrospectionBundle {
        inherit introspectionGraph;
      };

      views = compileViews {
        inherit projectRoot;
        resolved = resolvedModuleGraph.config;
        inherit
          statePolicy
          features
          runtime
          apps
          tasks
          workflows
          ;
        services = serviceCatalog;
      };

      finalized = finalizeModel {
        inherit
          system
          projectRoot
          statePolicy
          runtime
          serviceCatalog
          serviceSets
          apps
          tasks
          workflows
          features
          views
          ;
        resolved = resolvedModuleGraph.config;
      };
    in
    finalized
    // {
      resolved = resolvedModuleGraph.config;
      tasks = tasks;
      workflows = workflows;
      apps = apps;
      appExecutionManifests = appExecutionManifests;
      introspectionGraph = introspectionGraph;
      introspectionBundle = introspectionBundle;
      views = views;
      runtime = runtime;
      statePolicy = statePolicy;
      serviceCatalog = serviceCatalog;
      serviceSets = serviceSets;
      serviceSurfaceCatalog = serviceSurfaceCatalog;
      features = features;
      selectionIndex = selectionIndex;
      contractBundle = contractBundle;
      legacyLocalDefault = legacyLocalDefault;
    };

  compileServicesResolved =
    resolved:
    compileServices {
      inherit pkgs resolved;
    };

  compile =
    args:
    let
      core = compileCore args;
      services = compileServicesResolved core.resolved;
      runtimeHash = canonical.hashCanonical {
        schema = {
          kind = "nixfied-runtime";
          version = 1;
        };
        services = services;
      };
    in
    core
    // {
      inherit
        services
        runtimeHash
        ;
    };
}
