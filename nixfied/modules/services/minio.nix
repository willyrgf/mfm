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
    portKeyApi = lib.mkOption {
      type = t.str;
      default = "minioApi";
    };
    portKeyConsole = lib.mkOption {
      type = t.str;
      default = "minioConsole";
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
