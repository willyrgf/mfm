# Framework helpers - aggregator
# Re-exports the same flat attrset as the original lib.nix
{
  pkgs,
  project,
  hooks ? { },
}:

let
  summary = import ./summary.nix { inherit pkgs project; };
  helpers = import ./helpers.nix {
    inherit pkgs hooks;
    inherit (summary) summaryParser;
  };
  fixtures = import ./fixtures.nix {
    inherit pkgs project;
  };
  builders = import ./builders.nix {
    inherit pkgs project;
    fixtureLib = fixtures;
    inherit (helpers) loadEnv helpersScript hookExports;
  };
  appApi = import ./app-api.nix {
    inherit pkgs;
    inherit (builders) mkApp;
  };
  serviceApi = import ./service-api.nix {
    inherit pkgs appApi;
  };
  process = import ./process.nix { inherit pkgs; };
  portUtils = import ./port-utils.nix { inherit pkgs; };
  parallel = import ./parallel.nix { inherit pkgs; };
  runRegistry = import ./run-registry.nix { inherit pkgs project; };
in
{
  inherit (helpers) loadEnv helpersScript hookExports;
  inherit (summary) summaryParser;
  inherit (builders)
    withTiming
    mkAppScript
    mkApp
    mkAppWithDeps
    ;
  inherit fixtures;
  inherit appApi;
  inherit serviceApi;
  inherit (process) mkSignalHandler mkProcessManager;
  inherit (portUtils) mkPortCleanup mkPortConflictChecker;
  inherit (parallel) mkParallelRunner;
  inherit (runRegistry) runRegistryStart;
}
