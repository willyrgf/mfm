# Reth module config defaults
{ pkgs, project }:

let
  serviceConfig = import ../../lib/service-config.nix { lib = pkgs.lib; };
  cfg = serviceConfig.getProjectServiceConfig {
    inherit project;
    name = "reth";
  };
in
{
  package = cfg.package or null;
  portKeyHttp = cfg.portKeyHttp or "rethHttp";
  portKeyWs = cfg.portKeyWs or "rethWs";
  portKeyAuth = cfg.portKeyAuth or "rethAuth";
  dataDirName = cfg.dataDirName or "reth";
  network = cfg.network or "local";
  devMode = cfg.devMode or false;
  extraArgs = cfg.extraArgs or [ ];
}
