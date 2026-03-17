{ lib }:
let
  t = lib.types;

  probeWaitSpec = t.submodule {
    options = {
      enabled = lib.mkOption {
        type = t.bool;
        default = false;
      };
      timeoutSeconds = lib.mkOption {
        type = t.ints.positive;
        default = 300;
      };
      intervalSeconds = lib.mkOption {
        type = t.ints.positive;
        default = 1;
      };
    };
  };

  probeStepSpec = t.submodule {
    options = {
      kind = lib.mkOption {
        type = t.enum [
          "tcp"
          "http"
          "jsonrpc"
          "postgres-pg-isready"
          "postgres-query"
          "helios-ready"
          "exec"
        ];
      };
      endpoint = lib.mkOption {
        type = t.nullOr t.str;
        default = null;
      };
      executionEndpoint = lib.mkOption {
        type = t.nullOr t.str;
        default = null;
      };
      label = lib.mkOption {
        type = t.nullOr t.str;
        default = null;
      };
      host = lib.mkOption {
        type = t.str;
        default = "127.0.0.1";
      };
      path = lib.mkOption {
        type = t.str;
        default = "/";
      };
      method = lib.mkOption {
        type = t.str;
        default = "";
      };
      database = lib.mkOption {
        type = t.nullOr t.str;
        default = null;
      };
      query = lib.mkOption {
        type = t.lines;
        default = "";
      };
      command = lib.mkOption {
        type = t.lines;
        default = "";
      };
      sourceKinds = lib.mkOption {
        type = t.attrsOf (
          t.enum [
            "real"
            "shim"
            "mock"
            "unknown"
          ]
        );
        default = { };
      };
      readinessProfile = lib.mkOption {
        type = t.str;
        default = "fast";
      };
      requireNotSyncing = lib.mkOption {
        type = t.bool;
        default = false;
      };
      disallowSourceKinds = lib.mkOption {
        type = t.listOf (
          t.enum [
            "real"
            "shim"
            "mock"
            "unknown"
          ]
        );
        default = [ ];
      };
    };
  };

  probeOverrideSpec = t.submodule {
    options = {
      strategy = lib.mkOption {
        type = t.enum [
          "replace"
          "prepend"
          "append"
        ];
        default = "replace";
      };
      steps = lib.mkOption {
        type = t.listOf probeStepSpec;
        default = [ ];
      };
      wait = lib.mkOption {
        type = t.nullOr probeWaitSpec;
        default = null;
      };
    };
  };
in
{
  probeOptions = {
    health = lib.mkOption {
      type = t.nullOr probeOverrideSpec;
      default = null;
    };
    ready = lib.mkOption {
      type = t.nullOr probeOverrideSpec;
      default = null;
    };
  };
}
