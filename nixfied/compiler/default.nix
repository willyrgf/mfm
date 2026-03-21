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

  normalizeRuntime = import ./normalize-runtime.nix { inherit lib; };
  compileServiceCatalog = import ./compile-service-catalog.nix { inherit lib; };
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

  compileViews = import ./compile-views.nix { inherit lib; };
  compileSelectionIndex = import ./compile-selection-index.nix { inherit lib; };

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

      runtime = normalizeRuntime {
        resolved = resolvedModuleGraph.config;
      };

      serviceCatalog = compileServiceCatalog {
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
        allTasks = taskCompilation.allTasks;
        declaredTaskIds = taskCompilation.declaredTaskIds;
        prunedTaskIds = taskCompilation.prunedTaskIds;
        pruneReasonsByTaskId = taskCompilation.pruneReasonsByTaskId;
      };

      selectionIndex = compileSelectionIndex {
        inherit
          tasks
          workflows
          serviceCatalog
          ;
      };

      features = compileFeatures {
        inherit
          projectRoot
          runtime
          tasks
          workflows
          ;
        services = serviceCatalog;
      };

      views = compileViews {
        inherit projectRoot;
        resolved = resolvedModuleGraph.config;
        inherit
          features
          runtime
          tasks
          workflows
          ;
        services = serviceCatalog;
      };

      finalized = finalizeModel {
        inherit
          system
          projectRoot
          runtime
          serviceCatalog
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
      views = views;
      runtime = runtime;
      serviceCatalog = serviceCatalog;
      serviceSurfaceCatalog = serviceSurfaceCatalog;
      features = features;
      selectionIndex = selectionIndex;
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
