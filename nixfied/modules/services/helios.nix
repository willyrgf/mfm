{ lib, ... }:
let
  t = lib.types;
in
{
  options.nixfied.services.helios = {
    enable = lib.mkOption {
      type = t.bool;
      default = false;
    };
    portKeyRpc = lib.mkOption {
      type = t.str;
      default = "heliosRpc";
    };
    executionRpcPortKey = lib.mkOption {
      type = t.str;
      default = "rethHttp";
    };
  };
}
