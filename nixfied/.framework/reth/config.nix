# Reth module config defaults
{ project }:

let
  cfg = project.modules.reth or { };
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
}
