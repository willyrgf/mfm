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
    package = lib.mkOption {
      type = t.nullOr t.str;
      default = null;
    };
    portKeyRpc = lib.mkOption {
      type = t.str;
      default = "heliosRpc";
    };
    dataDirName = lib.mkOption {
      type = t.str;
      default = "helios";
    };
    network = lib.mkOption {
      type = t.str;
      default = "local";
    };
    executionRpcPortKey = lib.mkOption {
      type = t.str;
      default = "rethHttp";
    };
    executionRpcUrl = lib.mkOption {
      type = t.str;
      default = "";
    };
    consensusRpcUrl = lib.mkOption {
      type = t.str;
      default = "";
    };
    checkpoint = lib.mkOption {
      type = t.str;
      default = "";
    };
    extraArgs = lib.mkOption {
      type = t.listOf t.str;
      default = [ ];
    };
  };
}
