{ lib, ... }:
let
  t = lib.types;
  probeLib = import ./probes.nix { inherit lib; };
  sourceOptions = import ./source-options.nix { inherit lib; };
  sourceSpec = sourceOptions.mkSourceSpec { };
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
    probes = probeLib.probeOptions;
  };
}
