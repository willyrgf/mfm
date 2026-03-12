# Supervisor module aggregator
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
  config = import ./config.nix { inherit pkgs project slots; };
  runtime = import ./runtime.nix {
    inherit
      pkgs
      project
      slots
      config
      loggingPrelude
      ;
  };
  lifecycle = import ./lifecycle.nix {
    inherit
      pkgs
      project
      slots
      config
      runtime
      loggingPrelude
      ;
  };
  statusMod = import ./status.nix {
    inherit
      pkgs
      slots
      config
      runtime
      loggingPrelude
      ;
  };
  management = import ./management.nix {
    inherit
      pkgs
      slots
      config
      runtime
      loggingPrelude
      ;
  };
in
{
  inherit (lifecycle)
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
