{ lib, ... }:
let
  t = lib.types;
in
{
  options.nixfied.apps = lib.mkOption {
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
                "taskRef"
                "workflowRef"
                "serviceSetRef"
                "machineOutput"
              ];
              default = "taskRef";
            };

            taskId = lib.mkOption {
              type = t.str;
              default = "";
            };

            workflowId = lib.mkOption {
              type = t.str;
              default = "";
            };

            serviceSetId = lib.mkOption {
              type = t.str;
              default = "";
            };

            operation = lib.mkOption {
              type = t.str;
              default = "";
            };

            targetAppId = lib.mkOption {
              type = t.str;
              default = "";
            };

            targetArgs = lib.mkOption {
              type = t.listOf t.str;
              default = [ ];
            };

            setupAppIds = lib.mkOption {
              type = t.listOf t.str;
              default = [ ];
            };

            teardownAppIds = lib.mkOption {
              type = t.listOf t.str;
              default = [ ];
            };

            validation = {
              contractRef = lib.mkOption {
                type = t.str;
                default = "";
              };
            };

            summary = lib.mkOption {
              type = t.str;
              default = "";
            };

            description = lib.mkOption {
              type = t.str;
              default = "";
            };

            category = lib.mkOption {
              type = t.str;
              default = "core";
            };

            usage = lib.mkOption {
              type = t.listOf t.str;
              default = [ ];
            };

            examples = lib.mkOption {
              type = t.listOf t.str;
              default = [ ];
            };

            ownerFile = lib.mkOption {
              type = t.nullOr t.str;
              default = null;
            };
          };
        }
      )
    );
    default = { };
  };
}
