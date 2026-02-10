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
  builders = import ./builders.nix {
    inherit pkgs project;
    inherit (helpers) loadEnv helpersScript hookExports;
  };
  process = import ./process.nix { inherit pkgs; };
  portUtils = import ./port-utils.nix { inherit pkgs; };
  parallel = import ./parallel.nix { inherit pkgs; };
  runRegistry = import ./run-registry.nix { inherit pkgs project; };
in
{
  inherit (helpers) loadEnv helpersScript hookExports;
  inherit (summary) summaryParser;
  inherit (builders) withTiming mkAppScript mkApp mkAppWithDeps;
  inherit (process) mkSignalHandler mkProcessManager;
  inherit (portUtils) mkPortCleanup mkPortConflictChecker;
  inherit (parallel) mkParallelRunner;
  inherit (runRegistry) runRegistryStart;
}
