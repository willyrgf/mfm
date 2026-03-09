# MinIO module config defaults
{ pkgs, project }:

let
  serviceConfig = import ../../lib/service-config.nix { lib = pkgs.lib; };
  cfg = serviceConfig.getProjectServiceConfig {
    inherit project;
    name = "minio";
  };
in
{
  package = cfg.package or null;
  clientPackage = cfg.clientPackage or null;
  rootUser = cfg.rootUser or "minioadmin";
  rootPassword = cfg.rootPassword or "minioadmin";
  portKeyApi = cfg.portKeyApi or "minioApi";
  portKeyConsole = cfg.portKeyConsole or "minioConsole";
  dataDirName = cfg.dataDirName or "minio";
  browser = cfg.browser or true;
}
