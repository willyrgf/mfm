# Supervisor module aggregator
{
  pkgs,
  project,
  slots,
}:

let
  config = import ./config.nix { inherit pkgs project slots; };
  lifecycle = import ./lifecycle.nix {
    inherit pkgs project slots config;
  };
  statusMod = import ./status.nix { inherit pkgs slots config; };
  management = import ./management.nix { inherit pkgs slots config; };
in
{
  # Backward compat
  inherit (lifecycle) pc start stop startDaemon;
  inherit (config) generateConfig;
  inherit (statusMod) status isRunning logs;
  inherit (management) restart rotateLogs;
}
