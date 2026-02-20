{
  pkgs,
  canonical,
  modules,
  system,
  projectRoot,
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

  compileViews = import ./compile-views.nix { inherit lib; };

  finalizeModel = import ./finalize-model.nix {
    inherit
      lib
      canonical
      ;
  };
in
{
  compile =
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
          projectModules
          extraModules
          localOverrides
          ;
      };

      runtime = normalizeRuntime {
        resolved = resolvedModuleGraph.config;
      };

      services = compileServices {
        resolved = resolvedModuleGraph.config;
      };

      tasks = compileTasks {
        resolved = resolvedModuleGraph.config;
        inherit runtime;
      };

      workflows = compileWorkflows {
        resolved = resolvedModuleGraph.config;
        inherit tasks;
      };

      views = compileViews {
        inherit projectRoot;
        resolved = resolvedModuleGraph.config;
        inherit
          runtime
          services
          tasks
          workflows
          ;
      };

      finalized = finalizeModel {
        inherit
          system
          projectRoot
          runtime
          services
          tasks
          workflows
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
      services = services;
    };
}
