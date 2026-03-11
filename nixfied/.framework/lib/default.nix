# Framework helpers - aggregator
# Re-exports the same flat attrset as the original lib.nix
{
  pkgs,
  project,
  hooks ? { },
  commandSurfaces ? null,
  featureInventory ? null,
}:

let
  shellContract = import ./shell-contract.nix { inherit pkgs; };
  baseLoggingPrelude =
    (import ./helpers.nix {
      inherit pkgs project;
      hooks = { };
      summaryParser = "";
    }).loggingPrelude;
  summary = import ./summary.nix {
    inherit pkgs project;
    loggingPrelude = baseLoggingPrelude;
  };
  helpers = import ./helpers.nix {
    inherit pkgs project hooks;
    inherit (summary) summaryParser;
  };
  fixtures = import ./fixtures.nix {
    inherit pkgs project;
  };
  builders = import ./builders.nix {
    inherit pkgs project;
    inherit shellContract;
    fixtureLib = fixtures;
    inherit (helpers)
      loadEnv
      loadEnvFile
      helpersScript
      hookExports
      ;
  };
  appApi = import ./app-api.nix {
    inherit pkgs;
    inherit shellContract;
    inherit (builders) mkApp;
  };
  serviceApi = import ./service-api.nix {
    inherit
      pkgs
      appApi
      shellContract
      ;
  };
  discovery = import ./discovery.nix {
    inherit pkgs project;
    inherit (helpers) loggingPrelude;
    inherit
      commandSurfaces
      featureInventory
      ;
  };
  process = import ./process.nix {
    inherit pkgs;
    inherit (helpers) loggingPrelude;
  };
  id = import ./id.nix {
    inherit pkgs project;
    inherit (helpers) loggingPrelude;
  };
  runtimeEvents = import ./runtime-events.nix {
    inherit pkgs project;
    inherit (helpers) loggingPrelude;
  };
  managedServiceLifecycle = import ./managed-service-lifecycle.nix { inherit pkgs; };
  slotEnvRuntime = import ./slot-env-runtime.nix { inherit pkgs; };
  servicePolicy = import ./service-policy.nix { inherit pkgs; };
  portUtils = import ./port-utils.nix {
    inherit pkgs;
    inherit (helpers) loggingPrelude;
  };
  parallel = import ./parallel.nix {
    inherit pkgs;
    inherit (helpers) loggingPrelude;
  };
  runRegistry = import ./run-registry.nix {
    inherit pkgs project;
    inherit (helpers) loggingPrelude;
  };
in
{
  inherit (helpers)
    loadEnv
    loadEnvFile
    loggingPrelude
    helpersScript
    hookExports
    ;
  inherit (summary) summaryParser;
  inherit (builders)
    withTiming
    mkAppScript
    mkApp
    mkAppWithDeps
    ;
  inherit fixtures;
  inherit shellContract;
  inherit appApi;
  inherit serviceApi;
  inherit discovery;
  inherit managedServiceLifecycle;
  inherit (process) mkSignalHandler mkProcessManager;
  inherit (id)
    mkPlanId
    mkUniqueId
    mkRunId
    resolveId
    ;
  inherit (runtimeEvents)
    emitEvent
    serviceEvents
    serviceLogs
    serviceStatus
    registryRoot
    ;
  inherit slotEnvRuntime;
  inherit servicePolicy;
  inherit (portUtils) mkPortCleanup mkPortConflictChecker;
  inherit (parallel) mkParallelRunner;
  inherit (runRegistry) runRegistryStart;
}
