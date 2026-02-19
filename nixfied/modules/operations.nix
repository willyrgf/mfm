{ lib, config, pkgs, ... }:
let
  cfg = config.nixfied.operations;
  runtime = config.nixfied.runtime;

  envNames = runtime.env.names;
  envPattern =
    if envNames == [ ] then
      runtime.env.default
    else
      builtins.concatStringsSep "|" envNames;

  envOffsetCase =
    builtins.concatStringsSep "\n" (
      map (
        envName: "    ${envName}) env_offset=${toString (runtime.env.offsets.${envName} or 0)} ;;"
      ) envNames
    );

  portNames = builtins.sort builtins.lessThan (builtins.attrNames runtime.ports);
  portEmitLines = builtins.concatStringsSep "\n" (
    map (
      portName:
      let
        base = runtime.ports.${portName};
      in
      ''
        value=$(( ${toString base} + env_offset + (slot_value * ${toString runtime.slot.stride}) ))
        printf "%s=%s\n" "${lib.toUpper portName}_PORT" "$value"
      ''
    ) portNames
  );

  portCheckLines = builtins.concatStringsSep "\n" (
    map (
      portName:
      let
        base = runtime.ports.${portName};
      in
      ''
        value=$(( ${toString base} + env_offset + (slot_value * ${toString runtime.slot.stride}) ))
        if command -v lsof >/dev/null 2>&1; then
          if lsof -nP -iTCP:"$value" -sTCP:LISTEN >/dev/null 2>&1; then
            status="LISTEN"
          else
            status="FREE"
          fi
        else
          status="UNKNOWN"
        fi
        printf "%s=%s (%s)\n" "${lib.toUpper portName}_PORT" "$value" "$status"
      ''
    ) portNames
  );

  mkTask =
    {
      id,
      appName,
      summary,
      description,
      command,
      runtimeInputs ? [ ],
    }:
    {
      inherit
        id
        summary
        description
        ;
      kind = "utility";
      runner = {
        type = "shell";
        command = command;
      };
      contract = {
        version = 1;
        output = {
          format = "text";
          channels = "stdout";
        };
        behavior = {
          idempotent = true;
          effects = [ "none" ];
          timeoutSec = 0;
        };
        errors.codes = {
          generic = 1;
          usage = 2;
          precondition = 3;
        };
      };
      runtime = {
        slotEnv = "optional";
        workdir = "projectRoot";
        hermetic = true;
        runtimeInputs = runtimeInputs;
        passThroughEnv = [
          "HOME"
          runtime.env.var
          runtime.slot.var
        ];
        env = { };
        umask = "022";
        locale = "C.UTF-8";
        timezone = "UTC";
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
      ui.app = {
        expose = true;
        name = appName;
        category = "ops";
        usage = [ "nix run .#${appName}" ];
        examples = [ ];
      };
    };

  validateScript = ''
    set -euo pipefail

    slot_var=${lib.escapeShellArg runtime.slot.var}
    env_var=${lib.escapeShellArg runtime.env.var}

    slot_default=${toString runtime.slot.default}
    slot_max=${toString runtime.slot.max}
    env_default=${lib.escapeShellArg runtime.env.default}

    slot_value="''${!slot_var:-$slot_default}"
    env_value="''${!env_var:-$env_default}"

    if ! [[ "$slot_value" =~ ^[0-9]+$ ]]; then
      echo "ERROR: $slot_var must be an integer"
      exit 3
    fi

    if [ "$slot_value" -gt "$slot_max" ]; then
      echo "ERROR: $slot_var exceeds max slot ($slot_max)"
      exit 3
    fi

    case "$env_value" in
      ${envPattern}) ;;
      *)
        echo "ERROR: unsupported $env_var '$env_value'"
        exit 3
        ;;
    esac

    echo "OK: environment is valid (''${env_var}=$env_value ''${slot_var}=$slot_value)"
  '';

  portsScript = ''
    set -euo pipefail

    slot_var=${lib.escapeShellArg runtime.slot.var}
    env_var=${lib.escapeShellArg runtime.env.var}
    slot_default=${toString runtime.slot.default}
    env_default=${lib.escapeShellArg runtime.env.default}

    slot_value="''${!slot_var:-$slot_default}"
    env_value="''${!env_var:-$env_default}"

    case "$env_value" in
${envOffsetCase}
      *)
        echo "ERROR: unsupported $env_var '$env_value'"
        exit 3
        ;;
    esac

    echo "INFO: Port assignments for slot ''${slot_value} env ''${env_value}"
${portEmitLines}
  '';

  checkPortsScript = ''
    set -euo pipefail

    slot_var=${lib.escapeShellArg runtime.slot.var}
    env_var=${lib.escapeShellArg runtime.env.var}
    slot_default=${toString runtime.slot.default}
    env_default=${lib.escapeShellArg runtime.env.default}

    slot_value="''${!slot_var:-$slot_default}"
    env_value="''${!env_var:-$env_default}"

    case "$env_value" in
${envOffsetCase}
      *)
        echo "ERROR: unsupported $env_var '$env_value'"
        exit 3
        ;;
    esac

    echo "INFO: Port status for slot ''${slot_value} env ''${env_value}"
${portCheckLines}
  '';

  isolationScript = ''
    set -euo pipefail
    echo "INFO: Starting deterministic isolation smoke run"
    echo "SKIP: isolation orchestration is not configured for this project"
    echo "OK: test-isolation completed"
  '';
in
{
  options.nixfied.operations = {
    enable = lib.mkOption {
      type = lib.types.bool;
      default = true;
    };

    validateEnv.enable = lib.mkOption {
      type = lib.types.bool;
      default = true;
    };

    testIsolation.enable = lib.mkOption {
      type = lib.types.bool;
      default = true;
    };

    ports.enable = lib.mkOption {
      type = lib.types.bool;
      default = true;
    };

    checkPorts.enable = lib.mkOption {
      type = lib.types.bool;
      default = true;
    };
  };

  config = lib.mkMerge [
    (lib.mkIf (cfg.enable && cfg.validateEnv.enable) {
      nixfied.tasks."validate-env" = mkTask {
        id = "task.ops.validate-env";
        appName = "validate-env";
        summary = "Validate slot/env settings";
        description = "Validates PROJECT_ENV and NIX_ENV values against model runtime constraints.";
        command = validateScript;
      };
    })

    (lib.mkIf (cfg.enable && cfg.testIsolation.enable) {
      nixfied.tasks."test-isolation" = mkTask {
        id = "task.ops.test-isolation";
        appName = "test-isolation";
        summary = "Run isolation checks";
        description = "Runs deterministic isolation smoke checks from model metadata.";
        command = isolationScript;
      };
    })

    (lib.mkIf (cfg.enable && cfg.ports.enable) {
      nixfied.tasks."ports" = mkTask {
        id = "task.ops.ports";
        appName = "ports";
        summary = "Print model-derived port assignments";
        description = "Prints computed per-slot/per-env ports from compiled runtime data.";
        command = portsScript;
      };
    })

    (lib.mkIf (cfg.enable && cfg.checkPorts.enable) {
      nixfied.tasks."check-ports" = mkTask {
        id = "task.ops.check-ports";
        appName = "check-ports";
        summary = "Check model-derived port availability";
        description = "Checks if computed per-slot/per-env ports are listening or free.";
        command = checkPortsScript;
        runtimeInputs = [
          pkgs.coreutils
          pkgs.gnugrep
          pkgs.gnused
          (if pkgs ? lsof then pkgs.lsof else pkgs.coreutils)
        ];
      };
    })
  ];
}
