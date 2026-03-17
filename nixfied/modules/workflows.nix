{ lib, ... }:
let
  t = lib.types;

  whenSpec = t.submodule {
    options = {
      envEquals = lib.mkOption {
        type = t.attrsOf (
          t.oneOf [
            t.str
            t.int
            t.bool
          ]
        );
        default = { };
      };
      envPresent = lib.mkOption {
        type = t.listOf t.str;
        default = [ ];
      };
    };
  };

  workflowUnit = t.submodule {
    options = {
      taskId = lib.mkOption { type = t.str; };
      needs = lib.mkOption {
        type = t.listOf t.str;
        default = [ ];
      };
      locks = lib.mkOption {
        type = t.listOf t.str;
        default = [ ];
      };
      when = lib.mkOption {
        type = whenSpec;
        default = { };
      };
      skipIfMissingEnv = lib.mkOption {
        type = t.listOf t.str;
        default = [ ];
      };
      serviceName = lib.mkOption {
        type = t.str;
        default = "";
      };
    };
  };
in
{
  options.nixfied.workflows = lib.mkOption {
    type = t.attrsOf (
      t.submodule (
        { name, ... }:
        {
          options = {
            id = lib.mkOption {
              type = t.str;
              default = name;
            };
            summary = lib.mkOption {
              type = t.str;
              default = name;
            };
            description = lib.mkOption {
              type = t.str;
              default = "";
            };
            mode = lib.mkOption {
              type = t.enum [
                "ci"
                "dev"
                "test"
                "build"
                "check"
                "format"
                "custom"
              ];
              default = "custom";
            };
            maxWorkers = lib.mkOption {
              type = t.int;
              default = 1;
            };

            units = lib.mkOption {
              type = t.attrsOf workflowUnit;
              default = { };
            };

            stages = lib.mkOption {
              type = t.listOf (t.listOf t.str);
              default = [ ];
            };

            preRun.tasks = lib.mkOption {
              type = t.listOf t.str;
              default = [ ];
            };

            logging = {
              levelDefault = lib.mkOption {
                type = t.nullOr (
                  t.enum [
                    "error"
                    "warn"
                    "info"
                    "debug"
                    "trace"
                  ]
                );
                default = null;
              };
              outputDefault = lib.mkOption {
                type = t.nullOr (
                  t.enum [
                    "stdout"
                    "logs"
                    "both"
                  ]
                );
                default = null;
              };
            };

            postRun = {
              tasks = lib.mkOption {
                type = t.listOf t.str;
                default = [ ];
              };
              alwaysRun = lib.mkOption {
                type = t.bool;
                default = true;
              };
            };

            artifacts = {
              root = lib.mkOption {
                type = t.str;
                default = "/tmp/ci-artifacts";
              };
              keepOnSuccess = lib.mkOption {
                type = t.bool;
                default = false;
              };
              keepOnFailure = lib.mkOption {
                type = t.bool;
                default = true;
              };
              writeSummary = lib.mkOption {
                type = t.bool;
                default = true;
              };
            };

            execution = {
              parallel = lib.mkOption {
                type = t.bool;
                default = false;
              };
              failFast = lib.mkOption {
                type = t.bool;
                default = true;
              };
              lockPolicy = lib.mkOption {
                type = t.enum [
                  "exclusive"
                  "shared-aware"
                ];
                default = "exclusive";
              };
              emitRegistryEvents = lib.mkOption {
                type = t.bool;
                default = true;
              };
              ephemeral = {
                enable = lib.mkOption {
                  type = t.nullOr t.bool;
                  default = null;
                };
              };
            };
          };
        }
      )
    );
    default = { };
  };
}
