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
  };
}
