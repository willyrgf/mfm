# Isolation test runner apps (test-isolation + validate-env)
{
  pkgs,
  project,
  lib,
  slots,
}:

let
  isolation = project.isolation or { };
  enabled = isolation.enable or false;

  envVar = project.project.envVar or "PROJECT_ENV";
  slotVar = project.project.slotVar or "NIX_ENV";

  slotList =
    let
      configured = isolation.slots or [ ];
      maxSlot = slots.slotMax or 0;
    in
    if configured != [ ] then
      configured
    else
      builtins.genList (n: n) (maxSlot + 1);
  envList =
    let
      configured = isolation.envs or [ ];
      defaults = builtins.attrNames (project.envs or { });
    in
    if configured != [ ] then
      configured
    else
      defaults;

  slotListStr = pkgs.lib.concatMapStringsSep " " (s: toString s) slotList;
  envListStr = pkgs.lib.concatMapStringsSep " " pkgs.lib.escapeShellArg envList;

  validationInterval = isolation.validationInterval or 10;
  maxRuntime = isolation.maxRuntime or 300;
  startupWait = isolation.startupWait or 30;
  logsDir = isolation.logsDir or "/tmp/nixfied-isolation";
  keepLogsOnSuccess = isolation.keepLogsOnSuccess or false;
  keepLogsOnFailure = isolation.keepLogsOnFailure or true;
  useDeps = isolation.useDeps or false;

  runApp = isolation.runApp or "ci";
  runArgs = isolation.runArgs or [ "--summary" ];
  runArgsStr = pkgs.lib.escapeShellArgs runArgs;
  runCommand = isolation.runCommand or "";
  defaultRunCommand =
    if runArgsStr == "" then
      "nix run .#${runApp}"
    else
      "nix run .#${runApp} -- ${runArgsStr}";
  effectiveRunCommand = if runCommand != "" then runCommand else defaultRunCommand;

  runEnv = isolation.runEnv or { };
  runEnvExports = pkgs.lib.concatMapStringsSep "\n" (key: "export ${key}=${toString runEnv.${key}}") (
    builtins.attrNames runEnv
  );

  preInstall = isolation.preInstall or "";
  cleanupScript = isolation.cleanup or "";
  cleanupSlotScript = if cleanupScript != "" then cleanupScript else ":";

  validateCommand = isolation.validateCommand or "";
  effectiveValidateCommand = if validateCommand != "" then validateCommand else "nix run .#validate-env";

  portNames = builtins.attrNames (project.ports or { });
  portPairs = map (name: "${name}:${slots.portVarName name}") portNames;
  portPairsStr = pkgs.lib.concatMapStringsSep " " pkgs.lib.escapeShellArg portPairs;

  serviceNames = slots.serviceNames or [ ];
  serviceSockets = slots.serviceSockets or { };
  socketServiceNames = builtins.attrNames serviceSockets;
  serviceDirVars =
    builtins.concatLists (map (name: [
      "${slots.normalizeName name}_DIR"
      "${slots.normalizeName name}_LOG_DIR"
      "${slots.normalizeName name}_RUN_DIR"
      "${slots.normalizeName name}_CONFIG_DIR"
      "${slots.normalizeName name}_STATE_DIR"
      "${slots.normalizeName name}_SOCKET_DIR"
    ]) serviceNames);
  serviceDirVarsStr = pkgs.lib.concatMapStringsSep " " pkgs.lib.escapeShellArg serviceDirVars;
  socketChecks =
    map (name: let
      socketVar = "${slots.normalizeName name}_SOCKET";
      portVar = if builtins.hasAttr name (project.ports or { }) then slots.portVarName name else "";
    in "${name}:${socketVar}:${portVar}") socketServiceNames;
  socketChecksStr = pkgs.lib.concatMapStringsSep " " pkgs.lib.escapeShellArg socketChecks;

  dirVars = isolation.dirVars or [
    "LOG_DIR"
    "RUN_DIR"
    "CONFIG_DIR"
    "STATE_DIR"
  ];
  dirVarsStr = pkgs.lib.concatMapStringsSep " " pkgs.lib.escapeShellArg dirVars;

  validateEnvScript = ''
    set -euo pipefail

    ERRORS=0
    WARNINGS=0

    if [ -z "''${SLOT_INFO:-}" ] || [ ! -x "$SLOT_INFO" ]; then
      echo "ERROR: SLOT_INFO not available"
      exit 1
    fi

    eval "$("$SLOT_INFO")"

    PORT_PAIRS=(${portPairsStr})
    DIR_VARS=(${dirVarsStr})
    SERVICE_DIR_VARS=(${serviceDirVarsStr})
    SOCKET_CHECKS=(${socketChecksStr})

    LSOF="${pkgs.lsof}/bin/lsof"

    check_port() {
      local name="$1"
      local var="$2"
      local port="$3"

      if [ -z "$port" ]; then
        echo "ERROR: missing $var for $name"
        ERRORS=$((ERRORS + 1))
        return 0
      fi

      if ! echo "$port" | grep -qE '^[0-9]+$'; then
        echo "ERROR: invalid $var=$port"
        ERRORS=$((ERRORS + 1))
        return 0
      fi

      local listen_count
      listen_count=$("$LSOF" -nP -iTCP:"$port" -sTCP:LISTEN 2>/dev/null | awk 'NR>1 {c++} END {print c+0}')
      if [ "$listen_count" -eq 0 ]; then
        echo "WARN: $name not listening on $port"
        WARNINGS=$((WARNINGS + 1))
      elif [ "$listen_count" -gt 1 ]; then
        echo "WARN: $name has $listen_count listeners on $port"
        WARNINGS=$((WARNINGS + 1))
      fi
    }

    for pair in "''${PORT_PAIRS[@]}"; do
      name="''${pair%%:*}"
      var="''${pair#*:}"
      port="''${!var:-}"
      check_port "$name" "$var" "$port"
    done

    for var in "''${DIR_VARS[@]}" "''${SERVICE_DIR_VARS[@]}"; do
      path="''${!var:-}"
      if [ -z "$path" ]; then
        echo "WARN: $var not set"
        WARNINGS=$((WARNINGS + 1))
        continue
      fi
      if [ ! -d "$path" ]; then
        echo "WARN: $var missing: $path"
        WARNINGS=$((WARNINGS + 1))
      fi
    done

    check_socket() {
      local name="$1"
      local socket_var="$2"
      local port_var="$3"
      local socket="''${!socket_var:-}"
      if [ -z "$socket" ]; then
        return 0
      fi

      local require_socket=false
      if [ -n "$port_var" ]; then
        local port="''${!port_var:-}"
        if [ -n "$port" ] && echo "$port" | grep -qE '^[0-9]+$'; then
          local listen_count
          listen_count=$("$LSOF" -nP -iTCP:"$port" -sTCP:LISTEN 2>/dev/null | awk 'NR>1 {c++} END {print c+0}')
          if [ "$listen_count" -gt 0 ]; then
            require_socket=true
          fi
        fi
      fi

      if [ ! -S "$socket" ]; then
        if [ "$require_socket" = "true" ]; then
          echo "WARN: $name socket missing at $socket while port is listening"
        else
          echo "WARN: $name socket missing at $socket"
        fi
        WARNINGS=$((WARNINGS + 1))
      fi
    }

    for entry in "''${SOCKET_CHECKS[@]}"; do
      name="''${entry%%:*}"
      rest="''${entry#*:}"
      socket_var="''${rest%%:*}"
      port_var="''${rest#*:}"
      check_socket "$name" "$socket_var" "$port_var"
    done

    if [ "$ERRORS" -gt 0 ]; then
      exit 1
    fi
    exit 0
  '';

  testIsolationScript = ''
    set -euo pipefail

    ISOLATION_ENABLED=${if enabled then "true" else "false"}
    PREINSTALL_ENABLED=${if preInstall != "" then "true" else "false"}

    if [ "$ISOLATION_ENABLED" != "true" ]; then
      echo "Isolation runner disabled. Set project.isolation.enable = true to run it."
      exit 1
    fi

    if ! command -v nix >/dev/null 2>&1; then
      echo "nix is required in PATH" >&2
      exit 1
    fi

    if [ -z "''${SLOT_INFO:-}" ] || [ ! -x "$SLOT_INFO" ]; then
      echo "SLOT_INFO is required for isolation testing." >&2
      exit 1
    fi

    SLOTS=(${slotListStr})
    ENVS=(${envListStr})
    VALIDATION_INTERVAL=${toString validationInterval}
    MAX_RUNTIME=${toString maxRuntime}
    STARTUP_WAIT=${toString startupWait}
    LOG_DIR=${pkgs.lib.escapeShellArg logsDir}
    KEEP_LOGS_ON_SUCCESS=${if keepLogsOnSuccess then "true" else "false"}
    KEEP_LOGS_ON_FAILURE=${if keepLogsOnFailure then "true" else "false"}
    DIR_VARS=(${dirVarsStr})
    SERVICE_DIR_VARS=(${serviceDirVarsStr})

    declare -A PIDS
    declare -A OUTPUTS
    declare -A EXIT_CODES

    ERRORS=0
    WARNINGS=0

    KEEP_LOGS_OVERRIDE=false
    for arg in "$@"; do
      case "$arg" in
        --keep-logs) KEEP_LOGS_OVERRIDE=true ;;
        --help|-h)
          echo "Usage: nix run .#test-isolation [--keep-logs]"
          echo ""
          echo "Runs concurrent CI (or custom) commands across slot/env combinations and validates isolation."
          exit 0
          ;;
      esac
    done

    if [ "''${#SLOTS[@]}" -eq 0 ] || [ "''${#ENVS[@]}" -eq 0 ]; then
      echo "No slots or envs configured for isolation testing." >&2
      exit 1
    fi

    cleanup_slot() {
    ${cleanupSlotScript}
    }

    cleanup() {
      echo ""
      echo "==> Cleanup"

      for key in "''${!PIDS[@]}"; do
        pid=''${PIDS[$key]}
        if kill -0 "$pid" 2>/dev/null; then
          echo "Stopping $key (PID: $pid)..."
          kill "$pid" 2>/dev/null || true
          wait "$pid" 2>/dev/null || true
        fi
      done

      for slot in "''${SLOTS[@]}"; do
        for env in "''${ENVS[@]}"; do
          (
            export ${slotVar}="$slot"
            export ${envVar}="$env"
            cleanup_slot || true
          )
        done
      done

      local keep_logs=false
      if [ "$KEEP_LOGS_OVERRIDE" = "true" ]; then
        keep_logs=true
      elif [ "$KEEP_LOGS_ON_FAILURE" = "true" ]; then
        for key in "''${!PIDS[@]}"; do
          code=''${EXIT_CODES[$key]:-}
          if [ -z "$code" ] || [ "$code" -ne 0 ]; then
            keep_logs=true
            break
          fi
        done
      fi

      if [ "$KEEP_LOGS_ON_SUCCESS" = "true" ]; then
        keep_logs=true
      fi

      if [ "$keep_logs" = "true" ]; then
        echo "Logs preserved in $LOG_DIR"
        for output in "''${OUTPUTS[@]}"; do
          [ -f "$output" ] && echo "  $output"
        done
      else
        for output in "''${OUTPUTS[@]}"; do
          rm -f "$output" 2>/dev/null || true
        done
      fi
    }

    trap cleanup EXIT INT TERM

    mkdir -p "$LOG_DIR"

    echo "==> Isolation matrix"
    PORT_PAIRS=(${portPairsStr})
    for slot in "''${SLOTS[@]}"; do
      for env in "''${ENVS[@]}"; do
        INFO=$(${slotVar}=$slot ${envVar}=$env "$SLOT_INFO")
        PORTS=""
        for pair in "''${PORT_PAIRS[@]}"; do
          name="''${pair%%:*}"
          var="''${pair#*:}"
          value=$(echo "$INFO" | grep "^$var=" | cut -d= -f2 || true)
          PORTS="$PORTS $name:$value"
        done
        printf "  slot=%s env=%s%s\n" "$slot" "$env" "$PORTS"
      done
    done
    echo ""

    if [ "$PREINSTALL_ENABLED" = "true" ]; then
      echo "==> Pre-install"
    ${preInstall}
      echo ""
    fi

    ensure_dirs() {
      local var
      for var in "''${DIR_VARS[@]}" "''${SERVICE_DIR_VARS[@]}"; do
        local path="''${!var:-}"
        if [ -n "$path" ]; then
          mkdir -p "$path"
          chmod 700 "$path" 2>/dev/null || true
        fi
      done
    }

    run_cmd() {
    ${effectiveRunCommand}
    }

    validate_cmd() {
    ${effectiveValidateCommand}
    }

    echo "==> Starting runs"
    for slot in "''${SLOTS[@]}"; do
      for env in "''${ENVS[@]}"; do
        KEY="slot-$slot-$env"
        OUTPUT="$LOG_DIR/$KEY-$$.log"
        OUTPUTS[$KEY]="$OUTPUT"

        (
          export ${slotVar}="$slot"
          export ${envVar}="$env"
          eval "$("$SLOT_INFO")"
          if [ -z "''${CI_ARTIFACTS_DIR:-}" ]; then
            CI_ARTIFACTS_DIR="$STATE_DIR/ci-artifacts"
            export CI_ARTIFACTS_DIR
          fi
          mkdir -p "$CI_ARTIFACTS_DIR"
          ensure_dirs
    ${runEnvExports}
          run_cmd
        ) > "$OUTPUT" 2>&1 &
        PIDS[$KEY]=$!
        echo "  started $KEY (pid ''${PIDS[$KEY]})"
      done
    done

    echo ""
    echo "Waiting ${toString startupWait}s before validation..."
    sleep ${toString startupWait}
    echo ""

    for key in "''${!PIDS[@]}"; do
      pid=''${PIDS[$key]}
      if ! kill -0 "$pid" 2>/dev/null; then
        set +e
        wait "$pid" 2>/dev/null
        EXIT_CODES[$key]=$?
        set -e
      fi
    done

    echo "==> Validation loop"
    START_TIME=$(date +%s)
    while true; do
      CURRENT_TIME=$(date +%s)
      ELAPSED=$((CURRENT_TIME - START_TIME))

      if [ "$ELAPSED" -gt "$MAX_RUNTIME" ]; then
        echo "Max runtime reached ($MAX_RUNTIME s). Stopping."
        break
      fi

      for slot in "''${SLOTS[@]}"; do
        for env in "''${ENVS[@]}"; do
          KEY="slot-$slot-$env"
          pid=''${PIDS[$KEY]}
          if kill -0 "$pid" 2>/dev/null; then
            set +e
            VALIDATE_OUTPUT=$(${slotVar}=$slot ${envVar}=$env validate_cmd 2>&1)
            VALIDATE_RC=$?
            set -e

            ERROR_COUNT=$(echo "$VALIDATE_OUTPUT" | grep -c "^ERROR:" || true)
            WARN_COUNT=$(echo "$VALIDATE_OUTPUT" | grep -c "^WARN:" || true)

            if [ "$ERROR_COUNT" -gt 0 ]; then
              echo "ERROR: validation failed for $KEY"
              echo "$VALIDATE_OUTPUT" | sed 's/^/  /'
              ERRORS=$((ERRORS + ERROR_COUNT))
            elif [ "$VALIDATE_RC" -ne 0 ]; then
              echo "ERROR: validation exited $VALIDATE_RC for $KEY"
              echo "$VALIDATE_OUTPUT" | sed 's/^/  /'
              ERRORS=$((ERRORS + 1))
            fi

            if [ "$WARN_COUNT" -gt 0 ]; then
              echo "WARN: $KEY reported $WARN_COUNT warning(s)"
              WARNINGS=$((WARNINGS + WARN_COUNT))
            fi
          fi
        done
      done

      ALL_DONE=true
      for key in "''${!PIDS[@]}"; do
        pid=''${PIDS[$key]}
        if kill -0 "$pid" 2>/dev/null; then
          ALL_DONE=false
        else
          if [ -z "''${EXIT_CODES[$key]:-}" ]; then
            set +e
            wait "$pid" 2>/dev/null
            EXIT_CODES[$key]=$?
            set -e
          fi
        fi
      done

      if [ "$ALL_DONE" = true ]; then
        echo "All runs completed."
        break
      fi

      sleep "$VALIDATION_INTERVAL"
    done

    echo ""
    echo "==> Run summary"
    CI_FAILURES=0
    CI_UNKNOWN=0
    for key in "''${!PIDS[@]}"; do
      output=''${OUTPUTS[$key]}
      exit_code=''${EXIT_CODES[$key]:-999}
      if [ "$exit_code" -eq 999 ]; then
        echo "ERROR: $key unknown (killed or timed out)"
        CI_UNKNOWN=$((CI_UNKNOWN + 1))
      elif [ "$exit_code" -eq 0 ]; then
        echo "OK: $key passed"
      else
        echo "ERROR: $key failed (exit code: $exit_code)"
        CI_FAILURES=$((CI_FAILURES + 1))
        if [ -f "$output" ]; then
          echo "  last 10 lines:"
          tail -10 "$output" | sed 's/^/    /'
        fi
      fi
    done

    echo ""
    echo "==> Final summary"
    echo "validation errors: $ERRORS"
    echo "validation warnings: $WARNINGS"
    echo "run failures: $CI_FAILURES"
    echo "run unknown: $CI_UNKNOWN"

    if [ "$ERRORS" -eq 0 ] && [ "$CI_FAILURES" -eq 0 ] && [ "$CI_UNKNOWN" -eq 0 ]; then
      echo "ALL TESTS PASSED"
      exit 0
    fi

    echo "TESTS FAILED"
    exit 1
  '';
in
{
  validate-env = lib.mkApp {
    name = "validate-env";
    script = validateEnvScript;
    env = { };
    useDeps = false;
    description = "Validate slot/env listeners and directories";
  };

  test-isolation = lib.mkApp {
    name = "test-isolation";
    script = testIsolationScript;
    env = { };
    useDeps = useDeps;
    description = "Run concurrent isolation checks across slots/envs";
  };
}
