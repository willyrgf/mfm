# Framework-level app generators
# Auto-generates user-facing apps from service contracts and enabled modules.
{
  pkgs,
  project,
  lib,
  serviceApis ? { },
  supervisor ? null,
  slots,
}:

let
  mk =
    {
      name,
      summary,
      details,
      usage ? [ "nix run .#${name}" ],
      script,
      category ? "module",
    }:
    lib.appApi.mkNixfiedApp {
      inherit name script;
      env = { };
      useDeps = false;
      api = {
        version = 1;
        summary = summary;
        details = details;
        usage = usage;
        category = category;
      };
    };

  mkSupervisorHookApp =
    {
      name,
      summary,
      details,
      hook,
      usage ? [ "nix run .#${name}" ],
      passArgs ? false,
    }:
    mk {
      inherit
        name
        summary
        details
        usage
        ;
      category = "supervisor";
      script = ''
        SLOT_ENV_OUT="$($REQUIRE_SLOT_ENV)" || exit 1
        eval "$SLOT_ENV_OUT"
        run_hook ${hook}${if passArgs then " \"$@\"" else ""}
      '';
    };

  mkProcessApp =
    {
      name,
      summary,
      details,
      usage,
      tool,
    }:
    mk {
      inherit
        name
        summary
        details
        usage
        ;
      category = "utility";
      script = ''
        exec ${toString tool} "$@"
      '';
    };

  mkRuntimeAliasApp =
    {
      name,
      target,
      tool,
      usage,
    }:
    mk {
      inherit name usage;
      summary = "Alias for ${target}";
      details = "Compatibility alias for ${target}.";
      category = "utility";
      script = ''
        echo "WARN: ${name} is deprecated; use ${target}" >&2
        exec ${toString tool} "$@"
      '';
    };

  mkAppsFromSpecs = mkFromSpec: specs:
    builtins.listToAttrs (
      map (spec: {
        name = spec.name;
        value = mkFromSpec spec;
      }) specs
    );

  serviceApps = lib.serviceApi.mkServiceAppsFromContract serviceApis;

  supervisorSpecs = [
    {
      name = "up";
      summary = "Start all services";
      details = "Starts all supervisor-managed services for the current slot/env.";
      hook = "SUPERVISOR_START_DAEMON";
    }
    {
      name = "down";
      summary = "Stop all services";
      details = "Stops all supervisor-managed services for the current slot/env.";
      hook = "SUPERVISOR_STOP";
    }
    {
      name = "svc-status";
      summary = "Show service status";
      details = "Shows the status of supervisor-managed services.";
      hook = "SUPERVISOR_STATUS";
    }
    {
      name = "svc-health";
      summary = "Check service health";
      details = "Checks readiness health for supervisor-managed services.";
      hook = "SUPERVISOR_HEALTH";
    }
    {
      name = "svc-logs";
      summary = "Show service logs";
      details = "Streams logs for supervisor-managed services. Arguments are forwarded to the hook.";
      usage = [ "nix run .#svc-logs -- <args>" ];
      hook = "SUPERVISOR_LOGS";
      passArgs = true;
    }
    {
      name = "svc-restart";
      summary = "Restart a service";
      details = "Restarts a supervisor-managed service. Arguments are forwarded to the hook.";
      usage = [ "nix run .#svc-restart -- <args>" ];
      hook = "SUPERVISOR_RESTART";
      passArgs = true;
    }
  ];

  supervisorApps =
    if supervisor == null then
      { }
    else
      mkAppsFromSpecs mkSupervisorHookApp supervisorSpecs;

  portNames = builtins.attrNames (project.ports or { });
  slotInfoEvalBlock = ''
    SLOT_INFO_OUT="$($SLOT_INFO)" || exit 1
    eval "$SLOT_INFO_OUT"
  '';
  portVarNameFor =
    portName:
    pkgs.lib.strings.toUpper (pkgs.lib.replaceStrings [ "-" "." ] [ "_" "_" ] portName) + "_PORT";
  mkPortScriptLines =
    render:
    pkgs.lib.concatMapStringsSep "\n" (
      portName:
      let
        varName = portVarNameFor portName;
      in
      render {
        inherit
          portName
          varName
          ;
      }
    ) portNames;

  utilityApps = {
    check-ports = mk {
      name = "check-ports";
      summary = "Scan configured ports for conflicts";
      details = "Scans the configured ports for the current slot/env and reports whether they are free or listening.";
      category = "utility";
      script = ''
        ${slotInfoEvalBlock}
        LSOF="${pkgs.lsof}/bin/lsof"
        echo "Port status for slot ''${SLOT:-0}, env ''${ENV:-dev}:"
        echo ""
        ${mkPortScriptLines (
          {
            portName,
            varName,
          }:
          ''
            PORT_VAL="''${${varName}:-}"
            if [ -n "$PORT_VAL" ]; then
              if "$LSOF" -iTCP:"$PORT_VAL" -sTCP:LISTEN -n -P >/dev/null 2>&1; then
                PIDS=$("$LSOF" -iTCP:"$PORT_VAL" -sTCP:LISTEN -n -P -t 2>/dev/null | tr '\n' ',' | sed 's/,$//')
                echo "  ${portName} ($PORT_VAL): IN USE (PIDs: $PIDS)"
              else
                echo "  ${portName} ($PORT_VAL): free"
              fi
            fi
          ''
        )}
      '';
    };
    ports = mk {
      name = "ports";
      summary = "Show port assignments";
      details = "Prints effective port assignments for the current slot/env.";
      category = "utility";
      script = ''
        ${slotInfoEvalBlock}
        echo "Port assignments for slot ''${SLOT:-0}, env ''${ENV:-dev}:"
        echo ""
        ${mkPortScriptLines (
          {
            portName,
            varName,
          }:
          ''
            echo "  ${portName}: ''${${varName}:-n/a}"
          ''
        )}
      '';
    };
  };

  processSpecs = [
    {
      name = "process::status";
      summary = "Show process and service status";
      details = "Lists process/run/service entities tracked by the global process registry. Use --all to include completed and stopped entities.";
      usage = [
        "nix run .#process::status"
        "nix run .#process::status -- --all"
      ];
      tool = lib.processStatus;
    }
    {
      name = "process::slots";
      summary = "Show slot occupancy";
      details = "Shows slot ownership and contention metadata from the process registry.";
      usage = [
        "nix run .#process::slots"
        "nix run .#process::slots -- --all"
      ];
      tool = lib.processSlots;
    }
    {
      name = "process::runs";
      summary = "Show tracked runs";
      details = "Lists command runs tracked in the process registry. Use --all to include completed runs.";
      usage = [
        "nix run .#process::runs"
        "nix run .#process::runs -- --all"
      ];
      tool = lib.processRuns;
    }
    {
      name = "process::inspect";
      summary = "Inspect a run/process/service";
      details = "Prints detailed registry events for a run id, service name, or event id.";
      usage = [ "nix run .#process::inspect -- <id>" ];
      tool = lib.processInspect;
    }
    {
      name = "process::gc";
      summary = "Reconcile orphaned process metadata";
      details = "Finds orphaned service metadata in the process registry and records orphaned state with --apply.";
      usage = [
        "nix run .#process::gc"
        "nix run .#process::gc -- --apply"
      ];
      tool = lib.processGc;
    }
  ];
  processApps = mkAppsFromSpecs mkProcessApp processSpecs;
  processToolByAppName = builtins.listToAttrs (
    map (spec: {
      name = spec.name;
      value = spec.tool;
    }) processSpecs
  );

  mkRuntimeAliasSpec =
    {
      name,
      target,
      usage,
    }:
    {
      inherit
        name
        target
        usage
        ;
      tool = processToolByAppName.${target};
    };

  runtimeAliasDefs = [
    {
      name = "runtime::status";
      target = "process::status";
      usageTail = "[args]";
    }
    {
      name = "runtime::ps";
      target = "process::status";
      usageTail = "[args]";
    }
    {
      name = "runtime::slots";
      target = "process::slots";
      usageTail = "[args]";
    }
    {
      name = "runtime::runs";
      target = "process::runs";
      usageTail = "[args]";
    }
    {
      name = "runtime::inspect";
      target = "process::inspect";
      usageTail = "<id>";
    }
    {
      name = "runtime::gc";
      target = "process::gc";
      usageTail = "[args]";
    }
  ];
  runtimeAliasSpecs = map (
    def:
    mkRuntimeAliasSpec {
      inherit (def)
        name
        target
        ;
      usage = [ "nix run .#${def.name} -- ${def.usageTail}" ];
    }
  ) runtimeAliasDefs;
  runtimeAliases = mkAppsFromSpecs mkRuntimeAliasApp runtimeAliasSpecs;
in
serviceApps // supervisorApps // utilityApps // processApps // runtimeAliases
