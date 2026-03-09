{ lib, ... }:
let
  t = lib.types;
  sourceSpec = t.submodule {
    options = {
      package = lib.mkOption {
        type = t.nullOr t.package;
        default = null;
      };
    };
  };
in
{
  options.nixfied.services.nginx = {
    enable = lib.mkOption {
      type = t.bool;
      default = false;
    };
    portKeyHttp = lib.mkOption {
      type = t.str;
      default = "http";
    };
    portKeyHttps = lib.mkOption {
      type = t.str;
      default = "https";
    };
    dataDirName = lib.mkOption {
      type = t.str;
      default = "nginx";
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
