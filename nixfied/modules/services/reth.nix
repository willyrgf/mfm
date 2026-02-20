{ lib, ... }:
let
  t = lib.types;
in
{
  options.nixfied.services.reth = {
    enable = lib.mkOption {
      type = t.bool;
      default = false;
    };
    package = lib.mkOption {
      type = t.nullOr t.str;
      default = null;
    };
    portKeyHttp = lib.mkOption {
      type = t.str;
      default = "rethHttp";
    };
    portKeyWs = lib.mkOption {
      type = t.str;
      default = "rethWs";
    };
    portKeyAuth = lib.mkOption {
      type = t.str;
      default = "rethAuth";
    };
    network = lib.mkOption {
      type = t.str;
      default = "local";
    };
    devMode = lib.mkOption {
      type = t.bool;
      default = true;
    };
    extraArgs = lib.mkOption {
      type = t.listOf t.str;
      default = [ ];
    };
  };
}
