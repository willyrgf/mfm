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
    dataDirName = lib.mkOption {
      type = t.str;
      default = "reth";
    };
    network = lib.mkOption {
      type = t.str;
      default = "local";
    };
    devMode = lib.mkOption {
      type = t.bool;
      default = false;
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
    probes = probeLib.probeOptions;
  };
}
