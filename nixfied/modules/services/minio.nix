{ lib, ... }:
let
  t = lib.types;
in
{
  options.nixfied.services.minio = {
    enable = lib.mkOption {
      type = t.bool;
      default = false;
    };
    package = lib.mkOption {
      type = t.nullOr t.str;
      default = null;
    };
    clientPackage = lib.mkOption {
      type = t.nullOr t.str;
      default = null;
    };
    portKeyApi = lib.mkOption {
      type = t.str;
      default = "minioApi";
    };
    portKeyConsole = lib.mkOption {
      type = t.str;
      default = "minioConsole";
    };
    rootUser = lib.mkOption {
      type = t.str;
      default = "minio";
    };
    rootPassword = lib.mkOption {
      type = t.str;
      default = "minio123456";
    };
    browser = lib.mkOption {
      type = t.bool;
      default = true;
    };
  };
}
