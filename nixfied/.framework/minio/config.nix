# MinIO module config defaults
{ project }:

let
  cfg = project.modules.minio or { };
in
{
  rootUser = cfg.rootUser or "minioadmin";
  rootPassword = cfg.rootPassword or "minioadmin";
  portKeyApi = cfg.portKeyApi or "minioApi";
  portKeyConsole = cfg.portKeyConsole or "minioConsole";
  dataDirName = cfg.dataDirName or "minio";
  browser = cfg.browser or true;
}
