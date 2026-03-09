{ lib, ... }:
let
  t = lib.types;
  sourceSpec = t.submodule {
    options = {
      package = lib.mkOption {
        type = t.nullOr t.package;
        default = null;
      };
      clientPackage = lib.mkOption {
        type = t.nullOr t.package;
        default = null;
      };
    };
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
  };
}
