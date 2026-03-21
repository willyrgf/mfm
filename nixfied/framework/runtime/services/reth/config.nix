# Reth module config defaults
{ pkgs, project }:

let
  serviceConfig = import ../../../core/service-config.nix {
    lib = pkgs.lib;
    inherit pkgs;
  };
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
  portKeyP2p = cfg.portKeyP2p or "rethP2p";
  dataDirName = cfg.dataDirName or "reth";
  network = cfg.network or "local";
  devMode = cfg.devMode or false;
  extraArgs = cfg.extraArgs or [ ];
  defaultSource = cfg.defaultSource or "";
  probePlans = cfg.resolved.probePlans or cfg.resolved.operationProbes or { };
  resolvedEndpoints = cfg.resolved.endpoints or { };
}
