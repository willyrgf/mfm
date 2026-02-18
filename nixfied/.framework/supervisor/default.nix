# Supervisor module aggregator
{
  pkgs,
  project,
  slots,
}:

let
  summary = import ../lib/summary.nix { inherit pkgs project; };
  helpers = import ../lib/helpers.nix {
    inherit pkgs project;
    inherit (summary) summaryParser;
  };
  loggingPrelude = helpers.loggingPrelude;
  config = import ./config.nix { inherit pkgs project slots; };
  lifecycle = import ./lifecycle.nix {
    inherit
      pkgs
      project
      slots
      config
      loggingPrelude
      ;
  };
  statusMod = import ./status.nix {
    inherit
      pkgs
      slots
      config
      loggingPrelude
      ;
  };
  management = import ./management.nix {
    inherit
      pkgs
      slots
      config
      loggingPrelude
      ;
  };
in
{
  # Backward compat
  inherit (lifecycle)
    pc
    start
    stop
    startDaemon
    ;
  inherit (config) generateConfig;
  inherit (statusMod)
    status
    isRunning
    health
    logs
    ;
  inherit (management) restart rotateLogs;
}
