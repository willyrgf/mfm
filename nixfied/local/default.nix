{ pkgs, ... }:
let
  aaveTools = import ../project/aave-origin-tools.nix { inherit pkgs; };

  mkBinaryTask =
    {
      id,
      package,
      binary,
      summary,
      description,
      effects ? [ "none" ],
    }:
    {
      inherit
        id
        summary
        description
        ;
      kind = "utility";
      tags = [
        "aave"
        "local"
      ];
      requirements.services = [ ];
      runner = {
        type = "shell";
        command = ''
          set -euo pipefail
          exec ${package}/bin/${binary} "$@"
        '';
      };
      contract = {
        version = 1;
        input = {
          args = {
            parser = "passthrough";
            allowUnknown = true;
            spec = [ ];
          };
          env = {
            schemaRef = "runtimePrimitives";
            extra = [ ];
          };
        };
        output = {
          format = "text";
          channels = "stdout";
          keys = [ ];
        };
        behavior = {
          idempotent = false;
          inherit effects;
          timeoutSec = 0;
        };
        errors.codes = {
          generic = 1;
          usage = 2;
          precondition = 3;
        };
      };
      runtime = {
        slotEnv = "disabled";
        workdir = "projectRoot";
        hermetic = true;
        runtimeInputs = [ package ];
        passThroughEnv = [ ];
        allowSensitivePassThrough = false;
        env = { };
        umask = "022";
        locale = "C.UTF-8";
        timezone = "UTC";
        preHooks = { };
        postHooks = { };
      };
      scheduling = {
        locks = [ ];
        maxAttempts = 1;
        retryBackoffSec = [ ];
        priority = 100;
      };
      deps = {
        needs = [ ];
        softNeeds = [ ];
      };
      produces = {
        artifacts = [ ];
        stateKeys = [ ];
      };
    };

  mkTaskApp =
    {
      taskId,
      appId,
      usage ? [ "nix run .#${appId} -- --help" ],
      examples ? usage,
    }:
    {
      id = appId;
      kind = "taskRef";
      inherit
        taskId
        usage
        examples
        ;
      category = "local";
      ownerFile = "nixfied/local/default.nix";
    };
in
{
  config = {
    nixfied.tasks = {
      aave-v3-origin-fetch = mkBinaryTask {
        id = "task.aave.origin.fetch";
        package = aaveTools.aaveV3OriginFetchTool;
        binary = "mfm-aave-v3-origin-fetch";
        summary = "Fetch Aave V3 origin metadata";
        description = "Downloads and normalizes upstream Aave V3 origin inputs.";
      };

      aave-v3-origin-compile = mkBinaryTask {
        id = "task.aave.origin.compile";
        package = aaveTools.aaveV3OriginCompileTool;
        binary = "mfm-aave-v3-origin-compile";
        summary = "Compile Aave V3 origin metadata";
        description = "Compiles fetched Aave V3 origin inputs into MFM-ready outputs.";
      };

      aave-v3-origin-deploy = mkBinaryTask {
        id = "task.aave.origin.deploy";
        package = aaveTools.aaveV3OriginDeployTool;
        binary = "mfm-aave-v3-origin-deploy";
        summary = "Deploy Aave V3 origin outputs";
        description = "Publishes compiled Aave V3 origin outputs to the configured destination.";
        effects = [
          "writes-state"
          "network"
        ];
      };
    };

    nixfied.apps = {
      aave-v3-origin-fetch = mkTaskApp {
        taskId = "task.aave.origin.fetch";
        appId = "aave-v3-origin-fetch";
      };

      aave-v3-origin-compile = mkTaskApp {
        taskId = "task.aave.origin.compile";
        appId = "aave-v3-origin-compile";
      };

      aave-v3-origin-deploy = mkTaskApp {
        taskId = "task.aave.origin.deploy";
        appId = "aave-v3-origin-deploy";
      };
    };
  };
}
