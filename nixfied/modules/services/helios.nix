{ lib, ... }:
let
  t = lib.types;
  sourceKindType = t.enum [
    "real"
    "shim"
    "mock"
  ];
  disallowedKindType = t.enum [
    "real"
    "shim"
    "mock"
    "unknown"
  ];
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
    sourceKeys = lib.mkOption {
      type = t.listOf t.str;
      default = [ ];
    };
    defaultSource = lib.mkOption {
      type = t.str;
      default = "";
    };

    sourceKinds = lib.mkOption {
      type = t.attrsOf sourceKindType;
      default = { };
    };

    readiness = {
      profile = lib.mkOption {
        type = t.enum [
          "fast"
          "strict"
        ];
        default = "fast";
      };

      requireNotSyncing = lib.mkOption {
        type = t.bool;
        default = false;
      };

      disallowSourceKinds = lib.mkOption {
        type = t.listOf disallowedKindType;
        default = [ ];
      };
    };
  };
}
