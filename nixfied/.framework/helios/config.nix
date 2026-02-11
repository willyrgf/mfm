# Helios module config defaults
{ pkgs, project }:

let
  cfg = project.modules.helios or { };
in
{
  package = cfg.package or (if pkgs != null then pkgs.callPackage ./package.nix { } else null);
  portKeyRpc = cfg.portKeyRpc or "heliosRpc";
  dataDirName = cfg.dataDirName or "helios";
  network = cfg.network or "local";
  executionRpcPortKey = cfg.executionRpcPortKey or "rethHttp";
  executionRpcUrl = cfg.executionRpcUrl or "";
  consensusRpcUrl = cfg.consensusRpcUrl or "";
  checkpoint = cfg.checkpoint or "";
  extraArgs = cfg.extraArgs or [ ];
}
