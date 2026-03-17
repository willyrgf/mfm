# Nginx module config defaults
{ pkgs, project }:

let
  serviceConfig = import ../../../core/service-config.nix { lib = pkgs.lib; };
  cfg = serviceConfig.getProjectServiceConfig {
    inherit project;
    name = "nginx";
  };
in
{
  package = cfg.package or null;
  portKeyHttp = cfg.portKeyHttp or "http";
  portKeyHttps = cfg.portKeyHttps or "https";
  dataDirName = cfg.dataDirName or "nginx";
  defaultSource = cfg.defaultSource or "";
  probePlans = cfg.resolved.probePlans or cfg.resolved.operationProbes or { };
  resolvedEndpoints = cfg.resolved.endpoints or { };
}
