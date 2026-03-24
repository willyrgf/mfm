# Reth module aggregator
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
  config = import ./config.nix { inherit pkgs project; };
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
      summary = "Initialize Reth runtime directories";
      details = "Creates Reth runtime directories and JWT auth material for the current slot/environment.";
    };
    preflight-start = {
      script = lifecycle.preflightStart;
      summary = "Validate Reth start preconditions";
      details = "Checks deterministic blockers before Reth startup for the current slot and environment.";
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
      summary = "Start Reth node";
      details = "Starts Reth with HTTP, WS, and auth RPC listeners for the current slot/environment.";
    };
    stop = {
      script = lifecycle.stop;
      summary = "Stop Reth node";
      details = "Stops the running Reth process for the current slot/environment.";
    };
    restart = {
      preOps = [
        "stop"
        "start"
      ];
      summary = "Restart Reth node";
      details = "Stops then starts Reth for the current slot/environment.";
    };
    status = {
      script = lifecycle.status;
      summary = "Show Reth status";
      details = "Prints process status and effective ports for Reth.";
    };
    health = {
      script = lifecycle.health;
      summary = "Run Reth health check";
      details = "Checks JSON-RPC responsiveness on the configured Reth HTTP port.";
    };
    check-config = {
      script = lifecycle.checkConfig;
      summary = "Validate Reth configuration";
      details = "Validates Reth binary availability and runtime configuration basics.";
    };
    full-start = {
      script = lifecycle.fullStartLeaf;
      hook = "FULL_START";
      preOps = [
        "init"
        "check-config"
        "preflight-start"
      ];
      summary = "Init/check/start Reth";
      details = "Performs init + check-config + start for Reth.";
    };
    full-start-test = {
      script = lifecycle.fullStartTestLeaf;
      hook = "FULL_START_TEST";
      preOps = [
        "init"
        "check-config"
        "preflight-start"
      ];
      summary = "Init/check/start Reth for test profile";
      details = "Performs init + check-config + start for Reth (test profile).";
    };
    ready = {
      script = lifecycle.ready;
      hook = "READY";
      summary = "Wait for Reth readiness";
      details = "Checks that Reth responds on the configured HTTP RPC port.";
    };
  };
in
serviceModule.mkServiceModule {
  service = "reth";
  summaryName = "Reth";
  summary = "Reth service management API";
  details = "Public service contract for managing Reth across dev/prod/test/ci.";
  artifacts = {
    httpPortVar = slots.portVarName config.portKeyHttp;
    wsPortVar = slots.portVarName config.portKeyWs;
    authPortVar = slots.portVarName config.portKeyAuth;
    serviceDir = slots.getServiceDir config.dataDirName;
    dataDir = slots.getServiceDir config.dataDirName;
    logFile = "${slots.getServiceDir config.dataDirName}/logs/reth.log";
    pidFile = "${slots.getServiceDir config.dataDirName}/run/reth.pid";
    network = config.network;
    devMode = config.devMode;
  };
  inherit
    config
    operations
    ;
  exported = {
    inherit (lifecycle)
      reth
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
