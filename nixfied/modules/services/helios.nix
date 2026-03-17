{ lib, ... }:
let
  t = lib.types;
  probeLib = import ./probes.nix { inherit lib; };
  sourceSpec = t.submodule {
    options = {
      package = lib.mkOption {
        type = t.nullOr t.package;
        default = null;
      };
    };
  };
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
    dataDirName = lib.mkOption {
      type = t.str;
      default = "helios";
    };
    network = lib.mkOption {
      type = t.str;
      default = "local";
    };
    executionRpcUrl = lib.mkOption {
      type = t.str;
      default = "";
    };
    consensusRpcUrl = lib.mkOption {
      type = t.str;
      default = "";
    };
    defaultConsensusRpcUrl = lib.mkOption {
      type = t.str;
      default = "https://www.lightclientdata.org";
    };
    checkpoint = lib.mkOption {
      type = t.str;
      default = "";
    };
    extraArgs = lib.mkOption {
      type = t.listOf t.str;
      default = [ ];
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

    probes = probeLib.probeOptions;
  };
}
