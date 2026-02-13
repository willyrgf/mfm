# Helios module aggregator
{
  pkgs,
  project,
  slots,
}:

let
  processRegistry = import ../lib/process-registry.nix { inherit pkgs project; };
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

  logs = pkgs.writeShellScript "helios-logs" ''
    set -euo pipefail
    SLOT_INFO_OUT="$(${slots.getSlotInfo})" || exit 1
    eval "$SLOT_INFO_OUT"
    exec ${processRegistry.serviceLogs} --service helios --slot "$SLOT" --env "$ENV" "$@"
  '';
  log = logs;

  events = pkgs.writeShellScript "helios-events" ''
    set -euo pipefail
    SLOT_INFO_OUT="$(${slots.getSlotInfo})" || exit 1
    eval "$SLOT_INFO_OUT"
    exec ${processRegistry.serviceEvents} --service helios --slot "$SLOT" --env "$ENV" "$@"
  '';

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
    extensions = {
      full-start = {
        script = lifecycle.fullStart;
        hook = "FULL_START";
        summary = "Init/check/start Helios";
        details = "Performs init + check-config + start for Helios.";
      };
      full-start-test = {
        script = lifecycle.fullStartTest;
        hook = "FULL_START_TEST";
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
      log = {
        script = log;
        hook = "LOG";
        summary = "Show Helios log";
        details = "Shows Helios runtime log for the current slot/environment.";
        usage = [ "nix run .#service::helios::log -- [--lines N] [--follow]" ];
      };
      logs = {
        script = logs;
        hook = "LOGS";
        summary = "Alias for service::helios::log";
        details = "Compatibility alias for service::helios::log.";
        usage = [ "nix run .#service::helios::logs -- [--lines N] [--follow]" ];
      };
      events = {
        script = events;
        hook = "EVENTS";
        summary = "Show Helios lifecycle events";
        details = "Shows Helios lifecycle events from the global process registry for the current slot/environment.";
        usage = [ "nix run .#service::helios::events -- [--limit N]" ];
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
