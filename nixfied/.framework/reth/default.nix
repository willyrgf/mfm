# Reth module aggregator
{
  pkgs,
  project,
  slots,
}:

let
  processRegistry = import ../lib/process-registry.nix { inherit pkgs project; };
  observability = import ../lib/service-observability.nix {
    inherit
      pkgs
      slots
      processRegistry
      ;
  };
  config = import ./config.nix { inherit project; };
  lifecycle = import ./lifecycle.nix {
    inherit
      pkgs
      project
      slots
      config
      ;
  };

  logs = observability.mkLogScript "reth";
  log = logs;

  events = observability.mkEventsScript "reth";

  publicApi = {
    version = 1;
    service = "reth";
    summary = "Reth service management API";
    details = "Public service contract for managing Reth across dev/prod/test/ci.";
    profiles = [
      "dev"
      "prod"
      "test"
      "ci"
    ];
    artifacts = {
      httpPortVar = slots.portVarName config.portKeyHttp;
      wsPortVar = slots.portVarName config.portKeyWs;
      authPortVar = slots.portVarName config.portKeyAuth;
      dataDir = slots.getServiceDir config.dataDirName;
      network = config.network;
      devMode = config.devMode;
    };
    coreOps = {
      init = {
        script = lifecycle.init;
        summary = "Initialize Reth runtime directories";
        details = "Creates Reth runtime directories and JWT auth material for the current slot/environment.";
      };
      start = {
        script = lifecycle.start;
        summary = "Start Reth node";
        details = "Starts Reth with HTTP, WS, and auth RPC listeners for the current slot/environment.";
      };
      stop = {
        script = lifecycle.stop;
        summary = "Stop Reth node";
        details = "Stops the running Reth process for the current slot/environment.";
      };
      restart = {
        script = lifecycle.restart;
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
    };
    extensions = {
      full-start = {
        script = lifecycle.fullStart;
        hook = "FULL_START";
        summary = "Init/check/start Reth";
        details = "Performs init + check-config + start for Reth.";
      };
      full-start-test = {
        script = lifecycle.fullStartTest;
        hook = "FULL_START_TEST";
        summary = "Init/check/start Reth for test profile";
        details = "Performs init + check-config + start for Reth (test profile).";
      };
      ready = {
        script = lifecycle.ready;
        hook = "READY";
        summary = "Wait for Reth readiness";
        details = "Checks that Reth responds on the configured HTTP RPC port.";
      };
      log = {
        script = log;
        hook = "LOG";
        summary = "Show Reth log";
        details = "Shows Reth runtime log for the current slot/environment.";
        usage = [ "nix run .#service::reth::log -- [--lines N] [--follow]" ];
      };
      logs = {
        script = logs;
        hook = "LOGS";
        summary = "Alias for service::reth::log";
        details = "Compatibility alias for service::reth::log.";
        usage = [ "nix run .#service::reth::logs -- [--lines N] [--follow]" ];
      };
      events = {
        script = events;
        hook = "EVENTS";
        summary = "Show Reth lifecycle events";
        details = "Shows Reth lifecycle events from the global process registry for the current slot/environment.";
        usage = [ "nix run .#service::reth::events -- [--limit N]" ];
      };
    };
  };
in
{
  inherit config;

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

  inherit
    log
    logs
    events
    ;

  inherit publicApi;
}
