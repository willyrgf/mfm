{
  pkgs,
  exitCodes ? import ./exit-codes.nix,
  runtimeDefaults ? import ./runtime-defaults.nix,
}:
''
  NIXFIED_EXIT_GENERIC=${toString exitCodes.generic}
  NIXFIED_EXIT_USAGE=${toString exitCodes.usage}
  NIXFIED_EXIT_PRECONDITION=${toString exitCodes.precondition}
  NIXFIED_EXIT_UNAVAILABLE=${toString exitCodes.unavailable}
  NIXFIED_EXIT_TIMEOUT=${toString exitCodes.timeout}
  NIXFIED_EXIT_CANCELED=${toString exitCodes.canceled}

  NIXFIED_POLL_INTERVAL_FAST=${builtins.toJSON runtimeDefaults.intervals.pollFastSeconds}
  NIXFIED_POLL_INTERVAL_DEFAULT=${builtins.toJSON runtimeDefaults.intervals.pollDefaultSeconds}
  NIXFIED_POLL_INTERVAL_SLOW=${builtins.toJSON runtimeDefaults.intervals.pollSlowSeconds}
  NIXFIED_RETRY_INTERVAL_DEFAULT=${builtins.toJSON runtimeDefaults.intervals.retryDefaultSeconds}
  NIXFIED_LOCALHOST_IP=${builtins.toJSON runtimeDefaults.hosts.loopbackIp}
  NIXFIED_LOCALHOST_NAME=${builtins.toJSON runtimeDefaults.hosts.localhost}

  _nixfied_shell_error() {
    if command -v log_error >/dev/null 2>&1; then
      log_error "$*"
    else
      printf '%s\n' "ERROR: $*" >&2
    fi
  }

  nixfied_exit_generic() {
    if [ "$#" -gt 0 ]; then
      _nixfied_shell_error "$*"
    fi
    exit "$NIXFIED_EXIT_GENERIC"
  }

  nixfied_exit_usage() {
    if [ "$#" -gt 0 ]; then
      _nixfied_shell_error "$*"
    fi
    exit "$NIXFIED_EXIT_USAGE"
  }

  nixfied_exit_usage_with_usage() {
    local usage_fn="$1"
    shift || true
    if [ "$#" -gt 0 ]; then
      _nixfied_shell_error "$*"
    fi
    if [ -n "$usage_fn" ] && command -v "$usage_fn" >/dev/null 2>&1; then
      "$usage_fn" >&2
    fi
    exit "$NIXFIED_EXIT_USAGE"
  }

  nixfied_exit_precondition() {
    if [ "$#" -gt 0 ]; then
      _nixfied_shell_error "$*"
    fi
    exit "$NIXFIED_EXIT_PRECONDITION"
  }

  nixfied_exit_unavailable() {
    if [ "$#" -gt 0 ]; then
      _nixfied_shell_error "$*"
    fi
    exit "$NIXFIED_EXIT_UNAVAILABLE"
  }

  nixfied_exit_timeout() {
    if [ "$#" -gt 0 ]; then
      _nixfied_shell_error "$*"
    fi
    exit "$NIXFIED_EXIT_TIMEOUT"
  }

  nixfied_require_next_arg() {
    local flag="$1"
    local requirement="$2"
    shift 2 || true
    if [ "$#" -lt 2 ]; then
      nixfied_exit_usage "$flag requires $requirement"
    fi
    printf '%s\n' "$2"
  }

  nixfied_require_next_arg_with_usage() {
    local usage_fn="$1"
    local flag="$2"
    local requirement="$3"
    shift 3 || true
    if [ "$#" -lt 2 ]; then
      nixfied_exit_usage_with_usage "$usage_fn" "$flag requires $requirement"
    fi
    printf '%s\n' "$2"
  }

  nixfied_unknown_arg() {
    local arg="$1"
    nixfied_exit_usage "unknown argument '$arg'"
  }

  nixfied_unknown_arg_with_usage() {
    local usage_fn="$1"
    local arg="$2"
    nixfied_exit_usage_with_usage "$usage_fn" "unknown argument '$arg'"
  }

  nixfied_unexpected_positional_args() {
    if [ "$#" -gt 0 ]; then
      nixfied_exit_usage "unexpected positional arguments: $*"
    fi
  }

  nixfied_unexpected_positional_args_with_usage() {
    local usage_fn="$1"
    shift || true
    if [ "$#" -gt 0 ]; then
      nixfied_exit_usage_with_usage "$usage_fn" "unexpected positional arguments: $*"
    fi
  }
''
