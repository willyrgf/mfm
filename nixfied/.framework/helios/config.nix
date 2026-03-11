# Helios module config defaults
{ pkgs, project }:

let
  serviceConfig = import ../../lib/service-config.nix { lib = pkgs.lib; };
  cfg = serviceConfig.getProjectServiceConfig {
    inherit project;
    name = "helios";
  };
in
{
  package = cfg.package or (if pkgs != null then pkgs.callPackage ./package.nix { } else null);
  portKeyRpc = cfg.portKeyRpc or "heliosRpc";
  dataDirName = cfg.dataDirName or "helios";
  network = cfg.network or "local";
  executionRpcPortKey = cfg.executionRpcPortKey or "rethHttp";
  executionRpcUrl = cfg.executionRpcUrl or "";
  consensusRpcUrl = cfg.consensusRpcUrl or "";
  # Public default for mainnet consensus light-client data.
  #
  # NOTE: this is part of the weak-subjectivity trust model; pin a checkpoint
  # explicitly if you want deterministic control.
  defaultConsensusRpcUrl = cfg.defaultConsensusRpcUrl or "https://www.lightclientdata.org";
  checkpoint = cfg.checkpoint or "";
  extraArgs = cfg.extraArgs or [ ];
}
