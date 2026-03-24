# Helios module aggregator
{
  pkgs,
  project,
  slots,
}:

let
  summary = import ../../helpers/summary.nix { inherit pkgs project; };
  helpers = import ../../helpers/helpers.nix {
    inherit pkgs project;
    inherit (summary) summaryParser;
  };
  loggingPrelude = helpers.loggingPrelude;
  serviceModule = import ../../helpers/service-module.nix { inherit pkgs project slots; };
  config = import ./config.nix {
    inherit
      pkgs
      project
      ;
  };
  lifecycle = import ./lifecycle.nix {
    inherit
      pkgs
      project
      slots
      config
      loggingPrelude
      ;
  };

  operations = {
    init = {
      script = lifecycle.init;
      summary = "Initialize Helios runtime directories";
      details = "Creates Helios runtime directories for the current slot/environment.";
    };
    preflight-start = {
      script = lifecycle.preflightStart;
      summary = "Validate Helios start preconditions";
      details = "Checks deterministic blockers before Helios startup for the current slot/environment.";
      exposeApp = false;
      exposeHook = false;
    };
    start = {
      script = lifecycle.startLeaf;
      preOps = [
        "init"
        "check-config"
        "preflight-start"
      ];
      summary = "Start Helios node";
      details = "Starts Helios RPC for the current slot/environment.";
    };
    stop = {
      script = lifecycle.stop;
      summary = "Stop Helios node";
      details = "Stops the running Helios process for the current slot/environment.";
    };
    restart = {
      preOps = [
        "stop"
        "start"
      ];
      summary = "Restart Helios node";
      details = "Stops then starts Helios for the current slot/environment.";
    };
    status = {
      script = lifecycle.status;
      summary = "Show Helios status";
      details = "Prints process status and effective RPC port for Helios.";
    };
    health = {
      script = lifecycle.health;
      summary = "Run Helios health check";
      details = "Checks JSON-RPC responsiveness on the configured Helios port.";
    };
    check-config = {
      script = lifecycle.checkConfig;
      summary = "Validate Helios configuration";
      details = "Validates Helios binary availability and required runtime configuration.";
    };
    full-start = {
      script = lifecycle.fullStartLeaf;
      hook = "FULL_START";
      preOps = [
        "init"
        "check-config"
        "preflight-start"
      ];
      summary = "Init/check/start Helios";
      details = "Performs init + check-config + start for Helios.";
    };
    full-start-test = {
      script = lifecycle.fullStartTestLeaf;
      hook = "FULL_START_TEST";
      preOps = [
        "init"
        "check-config"
        "preflight-start"
      ];
      summary = "Init/check/start Helios for test profile";
      details = "Performs init + check-config + start for Helios (test profile).";
    };
    ready = {
      script = lifecycle.ready;
      hook = "READY";
      summary = "Wait for Helios readiness";
      details = ''
        Waits until Helios can answer `eth_blockNumber` successfully.

        Note: framework fixtures intentionally skip Helios start/readiness checks
        for `network=local` when a beacon consensus endpoint is unavailable.

        Tunables:
        - `HELIOS_READY_TIMEOUT_SECS` (default: 300)
        - `HELIOS_READY_INTERVAL_SECS` (default: 1)
      '';
    };
  };
in
serviceModule.mkServiceModule {
  service = "helios";
  summaryName = "Helios";
  summary = "Helios service management API";
  details = "Public service contract for managing Helios across dev/prod/test/ci.";
  artifacts = {
    rpcPortVar = slots.portVarName config.portKeyRpc;
    executionPortVar = slots.portVarName config.executionRpcPortKey;
    serviceDir = slots.getServiceDir config.dataDirName;
    dataDir = slots.getServiceDir config.dataDirName;
    logFile = "${slots.getServiceDir config.dataDirName}/logs/helios.log";
    pidFile = "${slots.getServiceDir config.dataDirName}/run/helios.pid";
    network = config.network;
  };
  inherit
    config
    operations
    ;
  exported = {
    inherit (lifecycle)
      helios
      init
      start
      stop
      restart
      status
      health
      checkConfig
      ready
      fullStart
      fullStartTest
      ;
  };
}
