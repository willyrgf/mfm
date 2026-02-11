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
in
serviceApps // supervisorApps // utilityApps
