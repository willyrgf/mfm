# Shared shell logging runtime.
{ pkgs }:

let
  shellCommon = import ../../core/shell-common.nix { inherit pkgs; };
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

    # print_log_tail PATH [lines] [label]
    # - print trailing log lines when available; emit WARN when the log file is missing.
    print_log_tail() {
      local path="$1"
      local lines="''${2:-50}"
      local label="''${3:-}"
      local prefix=""

      if [ -z "$path" ]; then
        echo "usage: print_log_tail <path> [lines] [label]" >&2
        return 1
      fi

      if [ -n "$label" ]; then
        prefix="$label "
      fi

      if [ -f "$path" ]; then
        log_info "''${prefix}log tail path=$path lines=$lines"
        tail -n "$lines" "$path" >&2 || true
      else
        log_warn "''${prefix}log file missing path=$path"
      fi
    }

    _NIXFIED_LOG_LEVEL_RAW=""
    _NIXFIED_LOG_LEVEL_NUM="2"
    _nixfied_refresh_log_level

    ${shellCommon}
  '';
in
{
  inherit loggingPrelude;
}
