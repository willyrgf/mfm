{ lib, ... }:
let
  t = lib.types;
  exitCodes = import ../framework/core/exit-codes.nix;
  serviceConfigLib = import ../framework/core/service-config.nix { inherit lib; };
  serviceRequirementType = t.enum serviceConfigLib.supportedServiceNames;
  runtimeWorkdirType = t.enum [
    "projectRoot"
    "stateRoot"
    "custom"
  ];

  argSpec = t.submodule {
    options = {
      name = lib.mkOption { type = t.str; };
      kind = lib.mkOption {
        type = t.enum [
          "flag"
          "option"
          "positional"
        ];
      };
      type = lib.mkOption {
        type = t.enum [
          "string"
          "int"
          "bool"
          "enum"
          "pathAbs"
          "pathRel"
          "json"
          "durationSec"
          "port"
        ];
        default = "string";
      };
      long = lib.mkOption {
        type = t.nullOr t.str;
        default = null;
      };
      short = lib.mkOption {
        type = t.nullOr t.str;
        default = null;
      };
      required = lib.mkOption {
        type = t.bool;
        default = false;
      };
      values = lib.mkOption {
        type = t.listOf t.str;
        default = [ ];
      };
      min = lib.mkOption {
        type = t.nullOr t.int;
        default = null;
      };
      max = lib.mkOption {
        type = t.nullOr t.int;
        default = null;
      };
      description = lib.mkOption {
        type = t.str;
        default = "";
      };
    };
  };

  envSpec = t.submodule {
    options = {
      name = lib.mkOption { type = t.str; };
      type = lib.mkOption {
        type = t.enum [
          "string"
          "int"
          "bool"
          "enum"
          "pathAbs"
          "pathRel"
          "json"
          "durationSec"
          "port"
        ];
        default = "string";
      };
      required = lib.mkOption {
        type = t.bool;
        default = false;
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
      sensitive = lib.mkOption {
        type = t.bool;
        default = false;
      };
      description = lib.mkOption {
        type = t.str;
        default = "";
      };
    };
  };

  hookSpec = t.submodule {
    options = {
      command = lib.mkOption { type = t.lines; };
      runtimeInputs = lib.mkOption {
        type = t.listOf t.package;
        default = [ ];
      };
      passThroughEnv = lib.mkOption {
        type = t.listOf t.str;
        default = [ ];
      };
      env = lib.mkOption {
        type = t.attrsOf (
          t.oneOf [
            t.str
            t.int
            t.bool
          ]
        );
        default = { };
      };
      workdir = lib.mkOption {
        type = t.nullOr runtimeWorkdirType;
        default = null;
      };
      customWorkdir = lib.mkOption {
        type = t.nullOr t.str;
        default = null;
      };
    };
  };

in
{
  options.nixfied.tasks = lib.mkOption {
    type = t.attrsOf (
      t.submodule (
        { name, ... }:
        {
          options = {
            id = lib.mkOption {
              type = t.str;
              default = name;
            };
            kind = lib.mkOption {
              type = t.enum [
                "command"
                "service-op"
                "supervisor-op"
                "utility"
                "ci-step"
                "workflow"
                "internal"
              ];
              default = "command";
            };
            summary = lib.mkOption {
              type = t.str;
              default = name;
            };
            description = lib.mkOption {
              type = t.str;
              default = "";
            };
            tags = lib.mkOption {
              type = t.listOf t.str;
              default = [ ];
            };

            requirements = {
              services = lib.mkOption {
                type = t.listOf serviceRequirementType;
                default = [ ];
                description = "Hard service capability requirements used for graph exclusion and runtime skip.";
              };
            };

            runner = {
              type = lib.mkOption {
                type = t.enum [
                  "shell"
                  "derivation"
                  "workflowRef"
                ];
                default = "shell";
              };
              command = lib.mkOption {
                type = t.lines;
                default = "";
              };
              package = lib.mkOption {
                type = t.nullOr t.package;
                default = null;
              };
              workflowId = lib.mkOption {
                type = t.nullOr t.str;
                default = null;
              };
            };

            contract = {
              version = lib.mkOption {
                type = t.int;
                default = 1;
              };
              input = {
                args = {
                  parser = lib.mkOption {
                    type = t.enum [
                      "typed"
                      "passthrough"
                      "json"
                    ];
                    default = "typed";
                  };
                  allowUnknown = lib.mkOption {
                    type = t.bool;
                    default = false;
                  };
                  spec = lib.mkOption {
                    type = t.listOf argSpec;
                    default = [ ];
                  };
                };
                env = {
                  schemaRef = lib.mkOption {
                    type = t.str;
                    default = "runtimePrimitives";
                  };
                  extra = lib.mkOption {
                    type = t.listOf envSpec;
                    default = [ ];
                  };
                };
              };
              output = {
                format = lib.mkOption {
                  type = t.enum [
                    "text"
                    "kv"
                    "json"
                    "ndjson"
                  ];
                  default = "text";
                };
                channels = lib.mkOption {
                  type = t.enum [
                    "stdout"
                    "logs"
                    "both"
                  ];
                  default = "stdout";
                };
                keys = lib.mkOption {
                  type = t.listOf t.str;
                  default = [ ];
                };
              };
              behavior = {
                idempotent = lib.mkOption {
                  type = t.bool;
                  default = true;
                };
                effects = lib.mkOption {
                  type = t.listOf (
                    t.enum [
                      "none"
                      "writes-state"
                      "starts-daemon"
                      "network"
                      "reads-secrets"
                    ]
                  );
                  default = [ "none" ];
                };
                timeoutSec = lib.mkOption {
                  type = t.int;
                  default = 0;
                };
              };
              errors = {
                codes = lib.mkOption {
                  type = t.attrsOf t.int;
                  default = builtins.removeAttrs exitCodes [ "canceled" ];
                };
              };
            };

            runtime = {
              slotEnv = lib.mkOption {
                type = t.enum [
                  "required"
                  "optional"
                  "disabled"
                ];
                default = "optional";
              };
              workdir = lib.mkOption {
                type = runtimeWorkdirType;
                default = "projectRoot";
              };
              customWorkdir = lib.mkOption {
                type = t.nullOr t.str;
                default = null;
              };
              hermetic = lib.mkOption {
                type = t.bool;
                default = true;
              };
              runtimeInputs = lib.mkOption {
                type = t.listOf t.package;
                default = [ ];
              };
              passThroughEnv = lib.mkOption {
                type = t.listOf t.str;
                default = [ ];
              };
              allowSensitivePassThrough = lib.mkOption {
                type = t.bool;
                default = false;
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
              env = lib.mkOption {
                type = t.attrsOf (
                  t.oneOf [
                    t.str
                    t.int
                    t.bool
                  ]
                );
                default = { };
              };
              umask = lib.mkOption {
                type = t.str;
                default = "022";
              };
              locale = lib.mkOption {
                type = t.str;
                default = "C.UTF-8";
              };
              timezone = lib.mkOption {
                type = t.str;
                default = "UTC";
              };
              preHooks = lib.mkOption {
                type = t.attrsOf hookSpec;
                default = { };
              };
              postHooks = lib.mkOption {
                type = t.attrsOf hookSpec;
                default = { };
              };
            };

            scheduling = {
              locks = lib.mkOption {
                type = t.listOf t.str;
                default = [ ];
              };
              maxAttempts = lib.mkOption {
                type = t.int;
                default = 1;
              };
              retryBackoffSec = lib.mkOption {
                type = t.listOf t.int;
                default = [ ];
              };
              priority = lib.mkOption {
                type = t.int;
                default = 100;
              };
            };

            deps = {
              needs = lib.mkOption {
                type = t.listOf t.str;
                default = [ ];
              };
              softNeeds = lib.mkOption {
                type = t.listOf t.str;
                default = [ ];
              };
            };

            produces = {
              artifacts = lib.mkOption {
                type = t.listOf t.str;
                default = [ ];
              };
              stateKeys = lib.mkOption {
                type = t.listOf t.str;
                default = [ ];
              };
            };
          };
        }
      )
    );
    default = { };
  };
}
