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

  serviceApps = lib.serviceApi.mkServiceAppsFromContract serviceApis;

  supervisorApps =
    if supervisor == null then
      { }
    else
      {
        up = mk {
          name = "up";
          summary = "Start all services";
          details = "Starts all supervisor-managed services for the current slot/env.";
          category = "supervisor";
          script = ''
            SLOT_ENV_OUT="$($REQUIRE_SLOT_ENV)" || exit 1
            eval "$SLOT_ENV_OUT"
            run_hook SUPERVISOR_START_DAEMON
          '';
        };
        down = mk {
          name = "down";
          summary = "Stop all services";
          details = "Stops all supervisor-managed services for the current slot/env.";
          category = "supervisor";
          script = ''
            SLOT_ENV_OUT="$($REQUIRE_SLOT_ENV)" || exit 1
            eval "$SLOT_ENV_OUT"
            run_hook SUPERVISOR_STOP
          '';
        };
        svc-status = mk {
          name = "svc-status";
          summary = "Show service status";
          details = "Shows the status of supervisor-managed services.";
          category = "supervisor";
          script = ''
            SLOT_ENV_OUT="$($REQUIRE_SLOT_ENV)" || exit 1
            eval "$SLOT_ENV_OUT"
            run_hook SUPERVISOR_STATUS
          '';
        };
        svc-health = mk {
          name = "svc-health";
          summary = "Check service health";
          details = "Checks readiness health for supervisor-managed services.";
          category = "supervisor";
          script = ''
            SLOT_ENV_OUT="$($REQUIRE_SLOT_ENV)" || exit 1
            eval "$SLOT_ENV_OUT"
            run_hook SUPERVISOR_HEALTH
          '';
        };
        svc-logs = mk {
          name = "svc-logs";
          summary = "Show service logs";
          details = "Streams logs for supervisor-managed services. Arguments are forwarded to the hook.";
          usage = [ "nix run .#svc-logs -- <args>" ];
          category = "supervisor";
          script = ''
            SLOT_ENV_OUT="$($REQUIRE_SLOT_ENV)" || exit 1
            eval "$SLOT_ENV_OUT"
            run_hook SUPERVISOR_LOGS "$@"
          '';
        };
        svc-restart = mk {
          name = "svc-restart";
          summary = "Restart a service";
          details = "Restarts a supervisor-managed service. Arguments are forwarded to the hook.";
          usage = [ "nix run .#svc-restart -- <args>" ];
          category = "supervisor";
          script = ''
            SLOT_ENV_OUT="$($REQUIRE_SLOT_ENV)" || exit 1
            eval "$SLOT_ENV_OUT"
            run_hook SUPERVISOR_RESTART "$@"
          '';
        };
      };

  portNames = builtins.attrNames (project.ports or { });

  utilityApps = {
    check-ports = mk {
      name = "check-ports";
      summary = "Scan configured ports for conflicts";
      details = "Scans the configured ports for the current slot/env and reports whether they are free or listening.";
      category = "utility";
      script = ''
        SLOT_INFO_OUT="$($SLOT_INFO)" || exit 1
        eval "$SLOT_INFO_OUT"
        LSOF="${pkgs.lsof}/bin/lsof"
        echo "Port status for slot ''${SLOT:-0}, env ''${ENV:-dev}:"
        echo ""
        ${pkgs.lib.concatMapStringsSep "\n" (
          portName:
          let
            varName =
              pkgs.lib.strings.toUpper (pkgs.lib.replaceStrings [ "-" "." ] [ "_" "_" ] portName) + "_PORT";
          in
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
        ) portNames}
      '';
    };
    ports = mk {
      name = "ports";
      summary = "Show port assignments";
      details = "Prints effective port assignments for the current slot/env.";
      category = "utility";
      script = ''
        SLOT_INFO_OUT="$($SLOT_INFO)" || exit 1
        eval "$SLOT_INFO_OUT"
        echo "Port assignments for slot ''${SLOT:-0}, env ''${ENV:-dev}:"
        echo ""
        ${pkgs.lib.concatMapStringsSep "\n" (
          portName:
          let
            varName =
              pkgs.lib.strings.toUpper (pkgs.lib.replaceStrings [ "-" "." ] [ "_" "_" ] portName) + "_PORT";
          in
          ''
            echo "  ${portName}: ''${${varName}:-n/a}"
          ''
        ) portNames}
      '';
    };
  };

  processApps = {
    "process::status" = mk {
      name = "process::status";
      summary = "Show process and service status";
      details = "Lists process/run/service entities tracked by the global process registry. Use --all to include completed and stopped entities.";
      usage = [
        "nix run .#process::status"
        "nix run .#process::status -- --all"
      ];
      category = "utility";
      script = ''
        exec ${toString lib.processStatus} "$@"
      '';
    };
    "process::slots" = mk {
      name = "process::slots";
      summary = "Show slot occupancy";
      details = "Shows slot ownership and contention metadata from the process registry.";
      usage = [
        "nix run .#process::slots"
        "nix run .#process::slots -- --all"
      ];
      category = "utility";
      script = ''
        exec ${toString lib.processSlots} "$@"
      '';
    };
    "process::runs" = mk {
      name = "process::runs";
      summary = "Show tracked runs";
      details = "Lists command runs tracked in the process registry. Use --all to include completed runs.";
      usage = [
        "nix run .#process::runs"
        "nix run .#process::runs -- --all"
      ];
      category = "utility";
      script = ''
        exec ${toString lib.processRuns} "$@"
      '';
    };
    "process::inspect" = mk {
      name = "process::inspect";
      summary = "Inspect a run/process/service";
      details = "Prints detailed registry events for a run id, service name, or event id.";
      usage = [ "nix run .#process::inspect -- <id>" ];
      category = "utility";
      script = ''
        exec ${toString lib.processInspect} "$@"
      '';
    };
    "process::gc" = mk {
      name = "process::gc";
      summary = "Reconcile orphaned process metadata";
      details = "Finds orphaned service metadata in the process registry and records orphaned state with --apply.";
      usage = [
        "nix run .#process::gc"
        "nix run .#process::gc -- --apply"
      ];
      category = "utility";
      script = ''
        exec ${toString lib.processGc} "$@"
      '';
    };
  };

  runtimeAliases = {
    "runtime::status" = mk {
      name = "runtime::status";
      summary = "Alias for process::status";
      details = "Compatibility alias for process::status.";
      usage = [ "nix run .#runtime::status -- [args]" ];
      category = "utility";
      script = ''
        echo "WARN: runtime::status is deprecated; use process::status"
        exec ${toString lib.processStatus} "$@"
      '';
    };
    "runtime::ps" = mk {
      name = "runtime::ps";
      summary = "Alias for process::status";
      details = "Compatibility alias for process::status.";
      usage = [ "nix run .#runtime::ps -- [args]" ];
      category = "utility";
      script = ''
        echo "WARN: runtime::ps is deprecated; use process::status"
        exec ${toString lib.processStatus} "$@"
      '';
    };
    "runtime::slots" = mk {
      name = "runtime::slots";
      summary = "Alias for process::slots";
      details = "Compatibility alias for process::slots.";
      usage = [ "nix run .#runtime::slots -- [args]" ];
      category = "utility";
      script = ''
        echo "WARN: runtime::slots is deprecated; use process::slots"
        exec ${toString lib.processSlots} "$@"
      '';
    };
    "runtime::runs" = mk {
      name = "runtime::runs";
      summary = "Alias for process::runs";
      details = "Compatibility alias for process::runs.";
      usage = [ "nix run .#runtime::runs -- [args]" ];
      category = "utility";
      script = ''
        echo "WARN: runtime::runs is deprecated; use process::runs"
        exec ${toString lib.processRuns} "$@"
      '';
    };
    "runtime::inspect" = mk {
      name = "runtime::inspect";
      summary = "Alias for process::inspect";
      details = "Compatibility alias for process::inspect.";
      usage = [ "nix run .#runtime::inspect -- <id>" ];
      category = "utility";
      script = ''
        echo "WARN: runtime::inspect is deprecated; use process::inspect"
        exec ${toString lib.processInspect} "$@"
      '';
    };
    "runtime::gc" = mk {
      name = "runtime::gc";
      summary = "Alias for process::gc";
      details = "Compatibility alias for process::gc.";
      usage = [ "nix run .#runtime::gc -- [args]" ];
      category = "utility";
      script = ''
        echo "WARN: runtime::gc is deprecated; use process::gc"
        exec ${toString lib.processGc} "$@"
      '';
    };
  };
in
serviceApps // supervisorApps // utilityApps // processApps // runtimeAliases
