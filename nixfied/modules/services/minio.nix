{ lib, ... }:
let
  t = lib.types;
  probeLib = import ./probes.nix { inherit lib; };
  sourceOptions = import ./source-options.nix { inherit lib; };
  sourceSpec = sourceOptions.mkSourceSpec {
    withClientPackage = true;
  };
in
{
  options.nixfied.services.minio = {
    enable = lib.mkOption {
      type = t.bool;
      default = false;
    };
    portKeyApi = lib.mkOption {
      type = t.str;
      default = "minioApi";
    };
    portKeyConsole = lib.mkOption {
      type = t.str;
      default = "minioConsole";
    };
    dataDirName = lib.mkOption {
      type = t.str;
      default = "minio";
    };
    rootUser = lib.mkOption {
      type = t.str;
      default = "minioadmin";
    };
    rootPassword = lib.mkOption {
      type = t.str;
      default = "minioadmin";
    };
    browser = lib.mkOption {
      type = t.bool;
      default = true;
    };
    sources = lib.mkOption {
      type = t.attrsOf sourceSpec;
      default = { };
    };
    sourceKeys = lib.mkOption {
      type = t.listOf t.str;
      default = [ ];
    };
    defaultSource = lib.mkOption {
      type = t.str;
      default = "";
    };
    probes = probeLib.probeOptions;
  };
}
