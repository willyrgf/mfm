{ lib, ... }:
let
  t = lib.types;
in
{
  options.nixfied.runtime = {
    slot = {
      var = lib.mkOption {
        type = t.str;
        default = "NIX_ENV";
      };
      default = lib.mkOption {
        type = t.int;
        default = 0;
      };
      max = lib.mkOption {
        type = t.int;
        default = 9;
      };
      stride = lib.mkOption {
        type = t.int;
        default = 1;
      };
    };

    env = {
      var = lib.mkOption {
        type = t.str;
        default = "PROJECT_ENV";
      };
      names = lib.mkOption {
        type = t.listOf t.str;
        default = [
          "dev"
          "test"
          "prod"
        ];
      };
      offsets = lib.mkOption {
        type = t.attrsOf t.int;
        default = {
          dev = 10;
          test = 20;
          prod = 0;
        };
      };
      default = lib.mkOption {
        type = t.str;
        default = "dev";
      };
    };

    logging = {
      levelDefault = lib.mkOption {
        type = t.enum [
          "error"
          "warn"
          "info"
          "debug"
          "trace"
        ];
        default = "info";
      };
      outputDefault = lib.mkOption {
        type = t.enum [
          "stdout"
          "logs"
          "both"
        ];
        default = "stdout";
      };
    };

    primitives = {
      version = lib.mkOption {
        type = t.int;
        default = 1;
      };
      defs = lib.mkOption {
        type = t.attrsOf (
          t.submodule {
            options = {
              type = lib.mkOption {
                type = t.enum [
                  "string"
                  "int"
                  "bool"
                  "enum"
                ];
                default = "string";
              };
              values = lib.mkOption {
                type = t.listOf t.str;
                default = [ ];
              };
              default = lib.mkOption {
                type = t.nullOr (
                  t.oneOf [
                    t.str
                    t.int
                    t.bool
                  ]
                );
                default = null;
              };
              aliases = lib.mkOption {
                type = t.listOf t.str;
                default = [ ];
              };
            };
          }
        );
        default = {
          LOG_LEVEL = {
            type = "enum";
            values = [
              "error"
              "warn"
              "info"
              "debug"
              "trace"
            ];
            default = "info";
            aliases = [ "NIXFIED_LOG_LEVEL" ];
          };
          OUTPUT_MODE = {
            type = "enum";
            values = [
              "stdout"
              "logs"
              "both"
            ];
            default = "stdout";
            aliases = [ "NIXFIED_OUTPUT_MODE" ];
          };
        };
      };
    };

    ports = lib.mkOption {
      type = t.attrsOf t.int;
      default = { };
      description = "Base port numbers keyed by logical role.";
    };

    directories = {
      base = lib.mkOption {
        type = t.str;
        default = "\${XDG_DATA_HOME:-$HOME/.local/share}/nixfied-project";
      };
    };
  };
}
