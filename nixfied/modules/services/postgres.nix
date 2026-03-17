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
  envConfigSpec = t.submodule {
    options.extraConfig = lib.mkOption {
      type = t.lines;
      default = "";
    };
  };
in
{
  options.nixfied.services.postgres = {
    enable = lib.mkOption {
      type = t.bool;
      default = false;
    };
    database = lib.mkOption {
      type = t.str;
      default = "app";
    };
    testDatabase = lib.mkOption {
      type = t.str;
      default = "app_test";
    };
    portKey = lib.mkOption {
      type = t.str;
      default = "postgres";
    };
    dataDirName = lib.mkOption {
      type = t.str;
      default = "postgres";
    };
    extensions = lib.mkOption {
      type = t.listOf t.str;
      default = [ ];
    };
    extraConfig = lib.mkOption {
      type = t.lines;
      default = "";
    };
    envConfigs = lib.mkOption {
      type = t.attrsOf envConfigSpec;
      default = { };
    };
    migrations = {
      dir = lib.mkOption {
        type = t.str;
        default = "migrations";
      };
      command = lib.mkOption {
        type = t.lines;
        default = "";
      };
      sourceDatabase = lib.mkOption {
        type = t.nullOr t.str;
        default = null;
      };
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
