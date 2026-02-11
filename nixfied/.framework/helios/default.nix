# Helios module aggregator
{
  pkgs,
  project,
  slots,
}:

let
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
      ;
  };

  publicApi = {
    version = 1;
    service = "helios";
    summary = "Helios service management API";
    details = "Public service contract for managing Helios across dev/prod/test/ci.";
    profiles = [
      "dev"
      "prod"
      "test"
      "ci"
    ];
    artifacts = {
      rpcPortVar = slots.portVarName config.portKeyRpc;
      executionPortVar = slots.portVarName config.executionRpcPortKey;
      dataDir = slots.getServiceDir config.dataDirName;
      network = config.network;
    };
    coreOps = {
      init = {
        script = lifecycle.init;
        summary = "Initialize Helios runtime directories";
        details = "Creates Helios runtime directories for the current slot/environment.";
      };
      start = {
        script = lifecycle.start;
        summary = "Start Helios node";
        details = "Starts Helios RPC for the current slot/environment.";
      };
      stop = {
        script = lifecycle.stop;
        summary = "Stop Helios node";
        details = "Stops the running Helios process for the current slot/environment.";
      };
      restart = {
        script = lifecycle.restart;
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
    };
  };
in
{
  inherit config;

  inherit (lifecycle)
    helios
    init
    start
    stop
    restart
    status
    health
    checkConfig
    ;

  inherit publicApi;
}
