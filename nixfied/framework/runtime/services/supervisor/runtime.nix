# Shared supervisor runtime setup and wrapped script builders
{
  pkgs,
  project,
  slots,
  config,
  loggingPrelude,
}:

let
  pc = pkgs.process-compose;
  serviceScripts = import ../../helpers/managed-service-lifecycle.nix { inherit pkgs; };
  slotEnvRuntime = import ../../helpers/slot-env-runtime.nix { inherit pkgs; };
  ports = project.ports or { };
  portNames = builtins.attrNames ports;

  slotPrelude = ''
    ${slotEnvRuntime.loadJsonFromCommand {
      outVar = "SLOT_INFO_JSON_OUT";
      command = toString slots.getSlotInfoJson;
      exportVars = false;
    }}
    ${slotEnvRuntime.readJsonField {
      targetVar = "RUN_DIR";
      jsonVar = "SLOT_INFO_JSON_OUT";
      jqExpr = ".directories.run";
    }}
  '';

  logDirPrelude = ''
    ${slotEnvRuntime.readJsonField {
      targetVar = "LOG_DIR";
      jsonVar = "SLOT_INFO_JSON_OUT";
      jqExpr = ".directories.log";
    }}
  '';

  portsPrelude = pkgs.lib.concatMapStringsSep "\n" (
    name:
    let
      portVar = slots.portVarName name;
    in
    ''
      ${slotEnvRuntime.readPortFromJson {
        targetVar = portVar;
        jsonVar = "SLOT_INFO_JSON_OUT";
        keyExpr = portVar;
      }}
    ''
  ) portNames;

  socketPrelude = ''
    SOCKET_HASH=$(printf '%s' "$RUN_DIR" | ${pkgs.coreutils}/bin/cksum | ${pkgs.coreutils}/bin/cut -d ' ' -f1)
    export PC_SOCKET_PATH="/tmp/nixfied-pc-$SOCKET_HASH.sock"
  '';

  helperPrelude = ''
    SUPERVISOR_PROCESS_JSON=""
    DAEMON_PID=""

    supervisor_pid_file() {
      printf '%s' "$RUN_DIR/supervisor.pid"
    }

    supervisor_exec_up() {
      exec ${pc}/bin/process-compose -f "$CONFIG_FILE" -t=false --keep-project up
    }

    supervisor_stop_server() {
      ${pc}/bin/process-compose down 2>/dev/null || true
    }

    supervisor_process_api_ready() {
      ${pc}/bin/process-compose process list -o json >/dev/null 2>&1
    }

    supervisor_fetch_process_json() {
      local proc_json=""
      local rc=0

      set +e
      proc_json="$(${pc}/bin/process-compose process list -o json 2>/dev/null)"
      rc="$?"
      set -e

      if [ "$rc" -ne 0 ] || [ -z "$proc_json" ]; then
        SUPERVISOR_PROCESS_JSON=""
        return 1
      fi

      SUPERVISOR_PROCESS_JSON="$proc_json"
      return 0
    }

    supervisor_spawn_daemon() {
      local log_file="$1"

      supervisor_exec_up > "$log_file" 2>&1 &
      DAEMON_PID=$!
    }

    supervisor_wait_process_api_ready() {
      local attempts="''${1:-40}"
      local interval="''${2:-0.25}"

      for _ in $(seq 1 "$attempts"); do
        if supervisor_process_api_ready; then
          return 0
        fi
        sleep "$interval"
      done

      return 1
    }

    supervisor_cleanup_orphans() {
      ${pkgs.lib.concatMapStringsSep "\n" (
        name:
        let
          portVar = slots.portVarName name;
        in
        ''
          PORT="''${${portVar}:-}"
          if [ -n "$PORT" ] && command -v lsof >/dev/null 2>&1; then
            ORPHANS=$(lsof -ti:"$PORT" 2>/dev/null || true)
            if [ -n "$ORPHANS" ]; then
              log_info "Cleaning orphan processes on port $PORT (${name}): $ORPHANS"
              echo "$ORPHANS" | xargs kill -TERM 2>/dev/null || true
            fi
          fi
        ''
      ) portNames}
    }

    supervisor_clear_state() {
      rm -f "$PC_SOCKET_PATH" "$(supervisor_pid_file)" 2>/dev/null || true
    }
  '';

  configPrelude = ''
    CONFIG_FILE=$(${config.generateConfig})
    export PC_CONFIG_FILES="$CONFIG_FILE"
    export PC_DISABLE_TUI=1
  '';

  mkSupervisorScript =
    {
      name,
      body,
      includeLogDir ? false,
      includePorts ? false,
      includeConfig ? false,
      useLoggingPrelude ? true,
    }:
    serviceScripts.mkWrappedScript {
      inherit name body;
      loggingPrelude = if useLoggingPrelude then loggingPrelude else "";
      runtimePrelude =
        slotPrelude
        + pkgs.lib.optionalString includeLogDir logDirPrelude
        + pkgs.lib.optionalString includePorts portsPrelude
        + socketPrelude
        + helperPrelude
        + pkgs.lib.optionalString includeConfig configPrelude;
    };
in
{
  inherit
    mkSupervisorScript
    slotPrelude
    logDirPrelude
    portsPrelude
    socketPrelude
    helperPrelude
    configPrelude
    ;
}
