# Shell runtime helpers - sourced by all app scripts
{
  pkgs,
  project ? { },
  hooks ? { },
  summaryParser,
}:

let
  envLoader = import ./env-loader.nix {
    inherit pkgs project;
    loggingPrelude = loggingPrelude;
  };
  servicePolicy = import ./service-policy.nix { inherit pkgs; };
  hookEnv = hooks.env or { };
  hookExports = pkgs.lib.concatMapStringsSep "\n" (key: ''
    # Always pin framework hook paths for deterministic app behavior.
    # User shell/.env hook overrides can route commands to stale scripts.
    export ${key}="${toString hookEnv.${key}}"
  '') (pkgs.lib.sort (a: b: a < b) (builtins.attrNames hookEnv));

  loadEnv = envLoader.loadEnv;
  loadEnvFile = envLoader.loadEnvFile;
  loggingPrelude = ''
    _nixfied_level_num() {
      case "''${1:-info}" in
        error) printf '%s\n' "0" ;;
        warn) printf '%s\n' "1" ;;
        info) printf '%s\n' "2" ;;
        debug) printf '%s\n' "3" ;;
        trace) printf '%s\n' "4" ;;
        *) printf '%s\n' "2" ;;
      esac
    }

    _nixfied_refresh_log_level() {
      local current_level="info"
      if [ -n "''${LOG_LEVEL:-}" ]; then
        current_level="$LOG_LEVEL"
      elif [ -n "''${NIXFIED_LOG_LEVEL:-}" ]; then
        current_level="$NIXFIED_LOG_LEVEL"
      fi
      if [ "''${_NIXFIED_LOG_LEVEL_RAW:-}" != "$current_level" ]; then
        _NIXFIED_LOG_LEVEL_RAW="$current_level"
        _NIXFIED_LOG_LEVEL_NUM="$(_nixfied_level_num "$current_level")"
      fi
    }

    _nixfied_emit_to_terminal() {
      local to_stderr="$1"
      local message="$2"
      if [ "$to_stderr" = "1" ]; then
        printf '%s\n' "$message" >&2
      else
        printf '%s\n' "$message"
      fi
    }

    _nixfied_emit() {
      local level_num="$1"
      local message="$2"
      local to_stderr="$3"
      local output_mode=""
      local log_file="''${NIXFIED_LOG_FILE:-}"

      _nixfied_refresh_log_level
      if [ -n "''${OUTPUT_MODE:-}" ]; then
        output_mode="$OUTPUT_MODE"
      elif [ -n "''${NIXFIED_OUTPUT_MODE:-}" ]; then
        output_mode="$NIXFIED_OUTPUT_MODE"
      else
        output_mode="stdout"
      fi

      if [ "$level_num" -gt "''${_NIXFIED_LOG_LEVEL_NUM:-2}" ]; then
        return 0
      fi

      case "$output_mode" in
        stdout|"")
          _nixfied_emit_to_terminal "$to_stderr" "$message"
          ;;
        logs)
          if [ -n "$log_file" ]; then
            printf '%s\n' "$message" >>"$log_file"
          fi
          if [ "$to_stderr" = "1" ] || [ -z "$log_file" ]; then
            _nixfied_emit_to_terminal "$to_stderr" "$message"
          fi
          ;;
        both)
          _nixfied_emit_to_terminal "$to_stderr" "$message"
          if [ -n "$log_file" ]; then
            printf '%s\n' "$message" >>"$log_file"
          fi
          ;;
        *)
          _nixfied_emit_to_terminal "$to_stderr" "$message"
          ;;
      esac
    }

    log_error() {
      _nixfied_emit 0 "ERROR: $*" "1"
    }

    log_detail() {
      _nixfied_emit 0 "DETAIL: $*" "1"
    }

    log_hint() {
      _nixfied_emit 0 "HINT: $*" "1"
    }

    log_warn() {
      _nixfied_emit 1 "WARN: $*" "1"
    }

    log_info() {
      _nixfied_emit 2 "INFO: $*" "0"
    }

    log_ok() {
      _nixfied_emit 2 "OK: $*" "0"
    }

    log_skip() {
      _nixfied_emit 2 "SKIP: $*" "0"
    }

    log_stop() {
      _nixfied_emit 2 "STOP: $*" "0"
    }

    log_timing() {
      _nixfied_emit 2 "TIMING: $*" "0"
    }

    log_run() {
      _nixfied_emit 2 "RUN: $*" "0"
    }

    log_output() {
      _nixfied_emit 2 "OUTPUT: $*" "0"
    }

    log_step() {
      local step="$1"
      local total="$2"
      shift 2 || true
      _nixfied_emit 2 "Step $step/$total: $*" "0"
    }

    log_debug() {
      _nixfied_emit 3 "DEBUG: $*" "1"
    }

    _NIXFIED_LOG_LEVEL_RAW=""
    _NIXFIED_LOG_LEVEL_NUM="2"
    _nixfied_refresh_log_level
  '';

  helpersScript = pkgs.writeShellScript "framework-helpers" ''
    ${loggingPrelude}

    # require_env VAR [message]
    # - fail if VAR is unset/empty; prints message to stderr.
    require_env() {
      local var="$1"
      local msg="''${2:-Missing required env var: $var}"
      if [ -z "''${!var:-}" ]; then
        log_error "$msg"
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
        log_skip "$reason"
        return 1
      fi
      return 0
    }

    is_uint() {
      case "''${1:-}" in
        *[!0-9]*|"")
          return 1
          ;;
        *)
          return 0
          ;;
      esac
    }

    require_positive_int() {
      local name="$1"
      local value="$2"
      if ! is_uint "$value" || [ "$value" -le 0 ]; then
        log_error "$name must be a positive integer (got '$value')"
        return 1
      fi
      return 0
    }

    require_positive_number() {
      local name="$1"
      local value="$2"
      local whole=""
      local frac=""

      case "$value" in
        *[!0-9.]*|""|*.*.*|.*|*.)
          log_error "$name must be a positive number (got '$value')"
          return 1
          ;;
      esac

      if [ "''${value#*.}" = "$value" ]; then
        if ! is_uint "$value" || [ "$value" -le 0 ]; then
          log_error "$name must be a positive number (got '$value')"
          return 1
        fi
        return 0
      fi

      whole="''${value%%.*}"
      frac="''${value#*.}"
      if ! is_uint "$whole" || ! is_uint "$frac"; then
        log_error "$name must be a positive number (got '$value')"
        return 1
      fi
      if [ "$whole" -eq 0 ] && [ -z "''${frac//0/}" ]; then
        log_error "$name must be a positive number (got '$value')"
        return 1
      fi
      return 0
    }

    require_port() {
      local name="$1"
      local value="$2"
      if ! is_uint "$value" || [ "$value" -lt 1 ] || [ "$value" -gt 65535 ]; then
        log_error "$name must be a valid TCP port (1-65535, got '$value')"
        return 1
      fi
      return 0
    }

    # wait_until TIMEOUT INTERVAL CHECK_FN
    # - run CHECK_FN until success or timeout.
    wait_until() {
      local timeout="$1"
      local interval="$2"
      local check_fn="$3"
      local start
      start=$(date +%s)

      if [ -z "$check_fn" ]; then
        echo "usage: wait_until <timeout> <interval> <check_fn>" >&2
        return 1
      fi
      require_positive_int "timeout" "$timeout" || return 1
      require_positive_number "interval" "$interval" || return 1

      while true; do
        if "$check_fn"; then
          return 0
        fi
        if [ $(( $(date +%s) - start )) -ge "$timeout" ]; then
          return 1
        fi
        sleep "$interval"
      done
    }

    # wait_http URL [timeout] [interval]
    # - poll HTTP(S) endpoint until it responds 2xx/3xx or timeout.
    wait_http() {
      local url="$1"
      local timeout="''${2:-30}"
      local interval="''${3:-1}"

      if [ -z "$url" ]; then
        echo "usage: wait_http <url> [timeout] [interval]" >&2
        return 1
      fi

      _wait_http_probe() {
        ${pkgs.curl}/bin/curl -sSf "$url" >/dev/null 2>&1
      }

      wait_until "$timeout" "$interval" _wait_http_probe
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
      case "$name" in
        */*|*\\*|.|..)
          log_error "artifact name must be a single file name (got '$name')"
          return 1
          ;;
      esac
      local dir
      dir=$(artifact_dir)
      mkdir -p "$dir"
      echo "$dir/$name"
    }

    # print_log_tail PATH [lines]
    # - print trailing log lines when available; emit WARN when log file is missing.
    print_log_tail() {
      local path="$1"
      local lines="''${2:-50}"
      if [ -z "$path" ]; then
        echo "usage: print_log_tail <path> [lines]" >&2
        return 1
      fi
      if [ -f "$path" ]; then
        log_info "log tail path=$path lines=$lines"
        tail -n "$lines" "$path" >&2 || true
      else
        log_warn "fixture log file missing path=$path"
      fi
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
        log_error "Hook not available: $var"
        return 1
      fi
      "$cmd" "$@"
    }

    # has_hook ENV_VAR
    # - check whether a hook variable is exported and non-empty.
    has_hook() {
      local var="$1"
      if [ -z "$var" ]; then
        return 1
      fi
      [ -n "''${!var:-}" ]
    }

    # wait_hook_ok ENV_VAR [timeout] [interval]
    # - poll a hook command until it succeeds (stdout/stderr suppressed).
    wait_hook_ok() {
      local var="$1"
      local timeout="''${2:-60}"
      local interval="''${3:-1}"

      if ! has_hook "$var"; then
        log_error "Hook not available: $var"
        return 1
      fi

      _wait_hook_probe() {
        run_hook "$var" >/dev/null 2>&1
      }

      wait_until "$timeout" "$interval" _wait_hook_probe
    }

    _service_token() {
      local service="$1"
      echo "$service" | tr '[:lower:]' '[:upper:]' | tr '.:/-' '_'
    }

    _service_hook_name() {
      local service="$1"
      local op="$2"
      local token
      token=$(_service_token "$service")
      echo "SVC_''${token}_''${op}"
    }

    # fixture_start_service SERVICE [profile] [timeout] [interval] [logfile] [keep_running]
    # - start service lifecycle from hook contract and register cleanup unless keep_running=1.
    #
    # Lifecycle policy invariants:
    # - start hooks may return before the service is externally ready.
    # - READY/HEALTH hooks must be safe to poll and deterministic when not ready.
    # - profile-specific readiness hooks (for example *_READY_TEST) take precedence when present.
    # - fixtures must poll READY/HEALTH with timeout loops after start (never one-shot checks).
    # - failure diagnostics must not introduce secondary errors (for example tailing missing logs).
    fixture_start_service() {
      local service="$1"
      local profile="''${2:-default}"
      local timeout="''${3:-60}"
      local interval="''${4:-1}"
      local logfile="''${5:-}"
      local keep_running_raw="''${6:-0}"
      local keep_running="0"

      if [ -z "$service" ]; then
        echo "usage: fixture_start_service <service> [profile] [timeout] [interval] [logfile] [keep_running]" >&2
        return 1
      fi

      case "$keep_running_raw" in
        1|true|TRUE|yes|YES)
          keep_running="1"
          ;;
        0|false|FALSE|no|NO|"")
          keep_running="0"
          ;;
        *)
          log_error "fixture_start_service keep_running must be 0/1/true/false (got '$keep_running_raw')"
          return 1
          ;;
      esac
      require_positive_int "timeout" "$timeout" || return 1
      require_positive_number "interval" "$interval" || return 1

      local start_hook=""
      local init_hook=""
      local check_hook=""
      local stop_hook=""
      local health_hook=""
      local ready_hook=""
      local profile_ready_hook=""
      local wait_hook=""
      local status_hook=""
      local start_cmd=""
      local op=""
      local hook=""
      local profile_token=""
      local -a start_ops=()

      case "$profile" in
        test)
          start_ops=(FULL_START_TEST FULL_START START)
          ;;
        *)
          start_ops=(FULL_START START)
          ;;
      esac

      for op in "''${start_ops[@]}"; do
        hook=$(_service_hook_name "$service" "$op")
        if has_hook "$hook"; then
          start_hook="$hook"
          break
        fi
      done

      if [ -z "$start_hook" ]; then
        log_error "fixture_start_service could not resolve start hook service=$service profile=$profile"
        return 1
      fi

      stop_hook=$(_service_hook_name "$service" "STOP")
      health_hook=$(_service_hook_name "$service" "HEALTH")
      ready_hook=$(_service_hook_name "$service" "READY")
      status_hook=$(_service_hook_name "$service" "STATUS")
      init_hook=$(_service_hook_name "$service" "INIT")
      check_hook=$(_service_hook_name "$service" "CHECK_CONFIG")
      start_cmd="''${!start_hook}"

      if [ -n "$profile" ] && [ "$profile" != "default" ]; then
        profile_token=$(echo "$profile" | tr '[:lower:]' '[:upper:]' | tr '.:/-' '_')
        profile_ready_hook=$(_service_hook_name "$service" "READY_''${profile_token}")
      fi

      if has_hook "$init_hook"; then
        run_hook "$init_hook"
      fi
      if has_hook "$check_hook"; then
        run_hook "$check_hook"
      fi

      local pid=""
      if [ -n "$logfile" ]; then
        if ! NIXFIED_START_SERVICE_MANAGED_CLEANUP=1 start_service_into pid "$service" --log "$logfile" -- "$start_cmd"; then
          log_error "fixture service start failed service=$service hook=$start_hook"
          return 1
        fi
      else
        if ! NIXFIED_START_SERVICE_MANAGED_CLEANUP=1 start_service_into pid "$service" -- "$start_cmd"; then
          log_error "fixture service start failed service=$service hook=$start_hook"
          return 1
        fi
      fi
      if [ -z "$pid" ]; then
        log_error "fixture service start returned empty pid service=$service hook=$start_hook"
        return 1
      fi

      # Clean wrapper process and module-native process state unless persistence is requested.
      if [ "$keep_running" = "1" ]; then
        log_info "fixture service keep_running enabled service=$service pid=$pid"
      else
        with_cleanup stop_service "$pid" "$service"
        if has_hook "$stop_hook"; then
          with_cleanup run_hook "$stop_hook"
        fi
      fi

      if [ -n "$profile_ready_hook" ] && has_hook "$profile_ready_hook"; then
        wait_hook="$profile_ready_hook"
      elif has_hook "$ready_hook"; then
        wait_hook="$ready_hook"
      elif has_hook "$health_hook"; then
        wait_hook="$health_hook"
      fi

      if [ -n "$wait_hook" ]; then
        local start_ts
        start_ts=$(date +%s)

        while true; do
          if run_hook "$wait_hook" >/dev/null 2>&1; then
            break
          fi

          # If the wrapper process died, fail fast unless STATUS indicates service is up.
          if ! kill -0 "$pid" 2>/dev/null; then
            local status_ok=0
            if has_hook "$status_hook"; then
              if run_hook "$status_hook" >/dev/null 2>&1; then
                status_ok=1
              fi
            fi

            if [ "$status_ok" -ne 1 ]; then
              log_error "fixture service start exited early service=$service pid=$pid hook=$start_hook"
              if [ -n "$logfile" ]; then
                print_log_tail "$logfile" 200
              fi
              if has_hook "$stop_hook"; then
                run_hook "$stop_hook" >/dev/null 2>&1 || true
              fi
              stop_service "$pid" "$service" >/dev/null 2>&1 || true
              return 1
            fi
          fi

          if [ $(( $(date +%s) - start_ts )) -ge "$timeout" ]; then
            log_error "fixture readiness check failed service=$service hook=$wait_hook timeout=''${timeout}s"
            if [ -n "$logfile" ]; then
              print_log_tail "$logfile" 200
            fi
            if has_hook "$stop_hook"; then
              run_hook "$stop_hook" >/dev/null 2>&1 || true
            fi
            stop_service "$pid" "$service" >/dev/null 2>&1 || true
            return 1
          fi

          sleep "$interval"
        done
      fi

      log_ok "fixture service ready service=$service profile=$profile"
      return 0
    }

    _cleanup_initialized=false
    _cleanup_actions=()

    # with_cleanup CMD [arg...]
    # - register cleanup command + args to run on EXIT/INT/TERM (LIFO order).
    #   Commands run in a subshell that inherits helper function definitions.
    with_cleanup() {
      if [ "$#" -lt 1 ]; then
        echo "usage: with_cleanup <command> [arg...]" >&2
        return 1
      fi
      local script
      script="$(mktemp "''${TMPDIR:-/tmp}/nixfied-cleanup.XXXXXX")" || {
        log_error "failed to create cleanup script file"
        return 1
      }
      chmod 700 "$script" 2>/dev/null || true
      {
        printf '#!/usr/bin/env bash\n'
        printf 'set -euo pipefail\n'
        printf ' %q' "$@"
        printf '\n'
      } >"$script"
      _cleanup_actions+=("$script")
      if [ "$_cleanup_initialized" = false ]; then
        _cleanup_initialized=true
        trap _run_cleanups EXIT INT TERM
      fi
    }

    _run_cleanups() {
      local i=$(( ''${#_cleanup_actions[@]} - 1 ))
      while [ $i -ge 0 ]; do
        (
          # shellcheck source=/dev/null
          source "''${_cleanup_actions[$i]}"
        ) || true
        rm -f "''${_cleanup_actions[$i]}" >/dev/null 2>&1 || true
        i=$((i - 1))
      done
    }

    # wait_port PORT [timeout] [interval]
    # - wait for a TCP port to listen (lsof or nc).
    wait_port() {
      local port="$1"
      local timeout="''${2:-30}"
      local interval="''${3:-1}"

      if [ -z "$port" ]; then
        echo "usage: wait_port <port> [timeout] [interval]" >&2
        return 1
      fi
      require_port "port" "$port" || return 1

      _wait_port_probe() {
        if command -v lsof >/dev/null 2>&1; then
          if lsof -iTCP:"$port" -sTCP:LISTEN -n -P >/dev/null 2>&1; then
            return 0
          fi
        elif command -v nc >/dev/null 2>&1; then
          if nc -z localhost "$port" >/dev/null 2>&1; then
            return 0
          fi
        fi
        return 1
      }

      wait_until "$timeout" "$interval" _wait_port_probe
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
        log_stop "$name (PID $pid)"
        kill -TERM "$pid" 2>/dev/null || true
        wait "$pid" 2>/dev/null || true
      fi
    }

    _start_service_policy_any_set() {
      [ -n "''${SERVICE_OWNER_SCOPE:-}" ] || [ -n "''${SERVICE_REUSE_POLICY:-}" ] || [ -n "''${SERVICE_DISCOVERY_SCOPE:-}" ]
    }

    _start_service_is_truthy() {
      case "''${1:-}" in
        1|true|TRUE|yes|YES|on|ON)
          return 0
          ;;
        *)
          return 1
          ;;
      esac
    }

    ${servicePolicy.policyRuntimeFunctions}

    _start_service_infer_owner_scope_from_reuse_policy() {
      nixfied_policy_owner_scope_from_reuse "''${SERVICE_REUSE_POLICY:-}"
    }

    _start_service_infer_discovery_scope_from_reuse_policy() {
      nixfied_policy_discovery_scope_from_reuse "''${SERVICE_REUSE_POLICY:-}"
    }

    _start_service_infer_owner_scope() {
      local from_reuse=""
      if [ -n "''${SERVICE_OWNER_SCOPE:-}" ]; then
        echo "$SERVICE_OWNER_SCOPE"
        return 0
      fi

      from_reuse="$(_start_service_infer_owner_scope_from_reuse_policy)"
      if [ -n "$from_reuse" ]; then
        echo "$from_reuse"
        return 0
      fi

      case "''${SERVICE_DISCOVERY_SCOPE:-}" in
        global)
          echo "persistent"
          ;;
        local)
          echo "ephemeral"
          ;;
        *)
          echo ""
          ;;
      esac
    }

    _start_service_infer_discovery_scope() {
      local owner_scope="''${1:-}"
      local from_reuse=""
      if [ -n "''${SERVICE_DISCOVERY_SCOPE:-}" ]; then
        echo "$SERVICE_DISCOVERY_SCOPE"
        return 0
      fi

      from_reuse="$(_start_service_infer_discovery_scope_from_reuse_policy)"
      if [ -n "$from_reuse" ]; then
        echo "$from_reuse"
        return 0
      fi

      case "$owner_scope" in
        persistent)
          echo "global"
          ;;
        ephemeral)
          echo "local"
          ;;
        *)
          echo ""
          ;;
      esac
    }

    _start_service_infer_reuse_policy() {
      local owner_scope="''${1:-}"
      local discovery_scope="''${2:-}"
      nixfied_policy_infer_reuse_policy "''${SERVICE_REUSE_POLICY:-}" "$owner_scope" "$discovery_scope" ""
    }

    _start_service_validate_policy_matrix() {
      local reuse="$1"
      local owner="$2"
      local discovery="$3"
      nixfied_policy_validate_matrix "$reuse" "$owner" "$discovery" 1 1
    }

    # start_service_should_register_cleanup [explicit_mode]
    # - return 1 to register stop cleanup and 0 to preserve process after command exit.
    start_service_should_register_cleanup() {
      local explicit_mode="''${1:-auto}"
      local owner_scope=""
      local discovery_scope=""
      local reuse_policy=""

      case "$explicit_mode" in
        cleanup)
          echo "1"
          return 0
          ;;
        keep-running)
          echo "0"
          return 0
          ;;
        auto)
          ;;
        *)
          log_error "invalid start_service cleanup mode '$explicit_mode' (expected auto|cleanup|keep-running)"
          return 1
          ;;
      esac

      if _start_service_is_truthy "''${NIXFIED_START_SERVICE_MANAGED_CLEANUP:-0}"; then
        echo "0"
        return 0
      fi

      if ! _start_service_policy_any_set; then
        echo "1"
        return 0
      fi

      owner_scope="$(_start_service_infer_owner_scope)"
      discovery_scope="$(_start_service_infer_discovery_scope "$owner_scope")"
      reuse_policy="$(_start_service_infer_reuse_policy "$owner_scope" "$discovery_scope")"

      _start_service_validate_policy_matrix "$reuse_policy" "$owner_scope" "$discovery_scope" || return 1

      case "$reuse_policy" in
        same-slot|cross-run)
          echo "0"
          ;;
        never|same-root)
          echo "1"
          ;;
        "")
          if [ "$owner_scope" = "persistent" ] || [ "$discovery_scope" = "global" ]; then
            echo "0"
          else
            echo "1"
          fi
          ;;
        *)
          log_error "unresolved start_service reuse policy '$reuse_policy'"
          return 1
          ;;
      esac
      return 0
    }

    # start_service NAME [opts] -- <command...>
    # - run a background service with optional logging and readiness checks.
    start_service() {
      local name="$1"
      shift || true

      if [ -z "$name" ]; then
        echo "usage: start_service <name> [--log <file>] [--cwd <dir>] [--wait-http <url>] [--wait-port <port>] [--timeout <s>] [--interval <s>] [--keep-running|--cleanup] -- <command...>" >&2
        return 1
      fi

      local log=""
      local cwd=""
      local wait_http_url=""
      local wait_port_num=""
      local timeout="30"
      local interval="1"
      local cleanup_mode="auto"
      local register_cleanup="1"

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
          --keep-running)
            if [ "$cleanup_mode" = "cleanup" ]; then
              log_error "start_service flags --keep-running and --cleanup are mutually exclusive"
              return 1
            fi
            cleanup_mode="keep-running"
            shift
            ;;
          --cleanup)
            if [ "$cleanup_mode" = "keep-running" ]; then
              log_error "start_service flags --keep-running and --cleanup are mutually exclusive"
              return 1
            fi
            cleanup_mode="cleanup"
            shift
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
      require_positive_int "--timeout" "$timeout" || return 1
      require_positive_number "--interval" "$interval" || return 1
      if [ -n "$wait_port_num" ]; then
        require_port "--wait-port" "$wait_port_num" || return 1
      fi
      register_cleanup="$(start_service_should_register_cleanup "$cleanup_mode")" || return 1
      if [ "$register_cleanup" != "0" ] && [ "$register_cleanup" != "1" ]; then
        log_error "start_service cleanup decision must be 0 or 1 (got '$register_cleanup')"
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

      if [ -n "$wait_http_url" ]; then
        if ! wait_http "$wait_http_url" "$timeout" "$interval"; then
          log_error "$name failed readiness check (http)"
          stop_service "$pid" "$name" >/dev/null 2>&1 || true
          return 1
        fi
      fi

      if [ -n "$wait_port_num" ]; then
        if ! wait_port "$wait_port_num" "$timeout" "$interval"; then
          log_error "$name failed readiness check (port)"
          stop_service "$pid" "$name" >/dev/null 2>&1 || true
          return 1
        fi
      fi

      # Avoid registering cleanup in command substitution subshells (they exit immediately).
      # App wrappers that execute command bodies in a dedicated subshell set
      # NIXFIED_CLEANUP_OWNER_BASHPID to the app-body shell BASHPID.
      if [ "$register_cleanup" = "1" ] && { { [ -n "''${NIXFIED_CLEANUP_OWNER_BASHPID:-}" ] && [ -n "''${BASHPID:-}" ] && [ "''${NIXFIED_CLEANUP_OWNER_BASHPID}" = "''${BASHPID}" ]; } || { [ -n "''${BASHPID:-}" ] && [ "''${BASHPID}" = "$$" ]; }; }; then
        with_cleanup stop_service "$pid" "$name"
      fi

      echo "$pid"
    }

    # start_service_into PID_VAR NAME [opts] -- <command...>
    # - start a service and assign PID to PID_VAR in the current shell.
    start_service_into() {
      local pid_var="$1"
      shift || true

      if [ -z "$pid_var" ]; then
        echo "usage: start_service_into <pid_var> <name> [opts] -- <command...>" >&2
        return 1
      fi

      # Avoid command substitution so start_service runs in this shell context.
      local pid_file=""
      pid_file="$(mktemp "''${TMPDIR:-/tmp}/nixfied-start-service.XXXXXX")" || {
        echo "start_service_into: failed to allocate pid capture file" >&2
        return 1
      }

      local _pid=""
      if ! start_service "$@" >"$pid_file"; then
        rm -f "$pid_file" >/dev/null 2>&1 || true
        echo "start_service_into: failed to start service" >&2
        return 1
      fi

      local line=""
      while IFS= read -r line; do
        if [ -n "$line" ]; then
          _pid="$line"
        fi
      done <"$pid_file"
      rm -f "$pid_file" >/dev/null 2>&1 || true

      if [ -z "$_pid" ]; then
        echo "start_service_into: failed to start service" >&2
        return 1
      fi
      printf -v "$pid_var" '%s' "$_pid"
      return 0
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
      if ! start_service_into pid "$name" "''${args[@]}"; then
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
  inherit
    loadEnv
    loadEnvFile
    loggingPrelude
    helpersScript
    hookExports
    ;
}
