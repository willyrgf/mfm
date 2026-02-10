# Shell runtime helpers - sourced by all app scripts
{
  pkgs,
  hooks ? { },
  summaryParser,
}:

let
  hookEnv = hooks.env or { };
  hookExports = pkgs.lib.concatMapStringsSep "\n" (key: ''
    if [ -z "''${${key}:-}" ]; then
      export ${key}="${toString hookEnv.${key}}"
    fi
  '') (builtins.attrNames hookEnv);

  # Script to load .env if it exists (does not override existing env vars)
  loadEnv = pkgs.writeShellScript "load-env" ''
    if [ -f ".env" ]; then
      while IFS='=' read -r key value || [ -n "$key" ]; do
        case "$key" in
          \#*|"") continue ;;
        esac
        value=$(echo "$value" | sed -e 's/^"//' -e 's/"$//' -e "s/^'//" -e "s/'$//")
        if [ -z "''${!key:-}" ]; then
          export "$key=$value"
        fi
      done < .env
    fi
  '';

  helpersScript = pkgs.writeShellScript "framework-helpers" ''
    # require_env VAR [message]
    # - fail if VAR is unset/empty; prints message to stderr.
    require_env() {
      local var="$1"
      local msg="''${2:-Missing required env var: $var}"
      if [ -z "''${!var:-}" ]; then
        echo "ERROR: $msg" >&2
        return 1
      fi
      return 0
    }

    # skip_if_missing VAR [reason]
    # - return 1 if VAR is missing so callers can skip work.
    skip_if_missing() {
      local var="$1"
      local reason="''${2:-Missing required env var: $var}"
      if [ -z "''${!var:-}" ]; then
        echo "SKIP: $reason"
        return 1
      fi
      return 0
    }

    # wait_http URL [timeout] [interval]
    # - poll HTTP(S) endpoint until it responds 2xx/3xx or timeout.
    wait_http() {
      local url="$1"
      local timeout="''${2:-30}"
      local interval="''${3:-1}"
      local start
      start=$(date +%s)

      if [ -z "$url" ]; then
        echo "usage: wait_http <url> [timeout] [interval]" >&2
        return 1
      fi

      while true; do
        if ${pkgs.curl}/bin/curl -sSf "$url" >/dev/null 2>&1; then
          return 0
        fi
        if [ $(( $(date +%s) - start )) -ge "$timeout" ]; then
          return 1
        fi
        sleep "$interval"
      done
    }

    # log_capture LOGFILE -- <command...>
    # - capture stdout/stderr to logfile (set LOG_TEE=1 to also stream to stdout).
    log_capture() {
      local logfile="$1"
      shift || true
      if [ "''${1:-}" = "--" ]; then
        shift
      fi
      if [ -z "$logfile" ] || [ "$#" -eq 0 ]; then
        echo "usage: log_capture <logfile> -- <command...>" >&2
        return 1
      fi
      if [ "''${LOG_TEE:-0}" = "1" ]; then
        "$@" 2>&1 | tee "$logfile"
      else
        "$@" > "$logfile" 2>&1
      fi
    }

    # summary_parse LOGFILE DURATION EXIT_CODE
    # - print a compact run summary (used by CI --summary mode).
    summary_parse() {
      local logfile="$1"
      local duration="$2"
      local exit_code="$3"
      ${summaryParser} "$logfile" "$duration" "$exit_code"
    }

    # artifact_dir
    # - return the current CI artifacts dir (CI_ARTIFACTS_DIR or /tmp/ci-artifacts).
    artifact_dir() {
      if [ -n "''${CI_ARTIFACTS_DIR:-}" ]; then
        echo "$CI_ARTIFACTS_DIR"
        return 0
      fi
      echo "/tmp/ci-artifacts"
      return 0
    }

    # artifact_path NAME
    # - create artifacts dir if needed and echo a full path for NAME.
    artifact_path() {
      local name="$1"
      if [ -z "$name" ]; then
        echo "usage: artifact_path <name>" >&2
        return 1
      fi
      local dir
      dir=$(artifact_dir)
      mkdir -p "$dir"
      echo "$dir/$name"
    }

    # run_hook ENV_VAR [args...]
    # - execute the command stored in ENV_VAR.
    run_hook() {
      local var="$1"
      shift || true
      if [ -z "$var" ]; then
        echo "usage: run_hook <ENV_VAR> [args...]" >&2
        return 1
      fi
      local cmd="''${!var:-}"
      if [ -z "$cmd" ]; then
        echo "ERROR: Hook not available: $var" >&2
        return 1
      fi
      "$cmd" "$@"
    }

    _cleanup_initialized=false
    _cleanup_actions=()

    # with_cleanup CMD
    # - register cleanup command to run on EXIT/INT/TERM (LIFO order).
    with_cleanup() {
      local cmd="$1"
      if [ -z "$cmd" ]; then
        echo "usage: with_cleanup <command>" >&2
        return 1
      fi
      _cleanup_actions+=("$cmd")
      if [ "$_cleanup_initialized" = false ]; then
        _cleanup_initialized=true
        trap _run_cleanups EXIT INT TERM
      fi
    }

    _run_cleanups() {
      local i=$(( ''${#_cleanup_actions[@]} - 1 ))
      while [ $i -ge 0 ]; do
        eval "''${_cleanup_actions[$i]}" || true
        i=$((i - 1))
      done
    }

    # wait_port PORT [timeout] [interval]
    # - wait for a TCP port to listen (lsof or nc).
    wait_port() {
      local port="$1"
      local timeout="''${2:-30}"
      local interval="''${3:-1}"
      local start
      start=$(date +%s)

      if [ -z "$port" ]; then
        echo "usage: wait_port <port> [timeout] [interval]" >&2
        return 1
      fi

      while true; do
        if command -v lsof >/dev/null 2>&1; then
          if lsof -iTCP:"$port" -sTCP:LISTEN -n -P >/dev/null 2>&1; then
            return 0
          fi
        elif command -v nc >/dev/null 2>&1; then
          if nc -z localhost "$port" >/dev/null 2>&1; then
            return 0
          fi
        fi
        if [ $(( $(date +%s) - start )) -ge "$timeout" ]; then
          return 1
        fi
        sleep "$interval"
      done
    }

    # stop_service PID [name]
    # - terminate a background service and wait for it to exit.
    stop_service() {
      local pid="$1"
      local name="''${2:-service}"

      if [ -z "$pid" ]; then
        echo "usage: stop_service <pid> [name]" >&2
        return 1
      fi

      if kill -0 "$pid" 2>/dev/null; then
        echo "STOP: $name (PID $pid)"
        kill -TERM "$pid" 2>/dev/null || true
        wait "$pid" 2>/dev/null || true
      fi
    }

    # start_service NAME [opts] -- <command...>
    # - run a background service with optional logging and readiness checks.
    start_service() {
      local name="$1"
      shift || true

      if [ -z "$name" ]; then
        echo "usage: start_service <name> [--log <file>] [--cwd <dir>] [--wait-http <url>] [--wait-port <port>] [--timeout <s>] [--interval <s>] -- <command...>" >&2
        return 1
      fi

      local log=""
      local cwd=""
      local wait_http_url=""
      local wait_port_num=""
      local timeout="30"
      local interval="1"

      while [ "''$#" -gt 0 ]; do
        case "''$1" in
          --log)
            log="$2"
            shift 2
            ;;
          --cwd)
            cwd="$2"
            shift 2
            ;;
          --wait-http)
            wait_http_url="$2"
            shift 2
            ;;
          --wait-port)
            wait_port_num="$2"
            shift 2
            ;;
          --timeout)
            timeout="$2"
            shift 2
            ;;
          --interval)
            interval="$2"
            shift 2
            ;;
          --)
            shift
            break
            ;;
          *)
            break
            ;;
        esac
      done

      if [ "''$#" -eq 0 ]; then
        echo "start_service: missing command" >&2
        return 1
      fi

      local in_subshell="false"
      if [ "''${BASH_SUBSHELL:-0}" -gt 0 ]; then
        in_subshell="true"
      fi

      if [ "$in_subshell" = "true" ] && command -v nohup >/dev/null 2>&1; then
        if [ -n "$cwd" ]; then
          if [ -n "$log" ]; then
            (cd "$cwd" && nohup "$@" > "$log" 2>&1) &
          else
            (cd "$cwd" && nohup "$@" >/dev/null 2>&1) &
          fi
        else
          if [ -n "$log" ]; then
            nohup "$@" > "$log" 2>&1 &
          else
            nohup "$@" >/dev/null 2>&1 &
          fi
        fi
      else
        if [ -n "$cwd" ]; then
          if [ -n "$log" ]; then
            (cd "$cwd" && "$@" > "$log" 2>&1) &
          else
            (cd "$cwd" && "$@") &
          fi
        else
          if [ -n "$log" ]; then
            ("$@" > "$log" 2>&1) &
          else
            ("$@") &
          fi
        fi
      fi

      local pid=$!
      # Avoid registering cleanup in command substitution subshells (they exit immediately).
      if [ -n "''${BASHPID:-}" ] && [ "''${BASHPID}" = "$$" ]; then
        with_cleanup "stop_service $pid \"$name\""
      fi

      if [ -n "$wait_http_url" ]; then
        if ! wait_http "$wait_http_url" "$timeout" "$interval"; then
          echo "ERROR: $name failed readiness check (http)" >&2
          return 1
        fi
      fi

      if [ -n "$wait_port_num" ]; then
        if ! wait_port "$wait_port_num" "$timeout" "$interval"; then
          echo "ERROR: $name failed readiness check (port)" >&2
          return 1
        fi
      fi

      echo "$pid"
    }

    # with_service NAME [start opts] -- <start command...> --run <command...>
    # - start service, then run command (cleanup handled automatically).
    with_service() {
      local name="$1"
      shift || true

      if [ -z "$name" ]; then
        echo "usage: with_service <name> [start options] -- <start command...> --run <command...>" >&2
        return 1
      fi

      local args=()
      local run_cmd=()
      local state="start"

      while [ "''$#" -gt 0 ]; do
        case "''$1" in
          --run)
            state="run"
            shift
            ;;
          *)
            if [ "$state" = "start" ]; then
              args+=("''$1")
            else
              run_cmd+=("''$1")
            fi
            shift
            ;;
        esac
      done

      if [ "''${#run_cmd[@]}" -eq 0 ]; then
        echo "with_service: missing --run <command...>" >&2
        return 1
      fi

      local pid=""
      local rc=0
      pid=$(start_service "$name" "''${args[@]}")
      if [ -z "$pid" ]; then
        echo "with_service: failed to start $name" >&2
        return 1
      fi
      set +e
      "''${run_cmd[@]}"
      rc=$?
      set -e
      stop_service "$pid" "$name"
      return "$rc"
    }
  '';
in
{
  inherit loadEnv helpersScript hookExports;
}
