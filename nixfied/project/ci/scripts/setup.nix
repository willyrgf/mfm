{ pkgs }:
pkgs.writeText "mfm-ci-setup.sh" ''
_ci_truthy() {
  case "''${1:-}" in
    1|true|TRUE|yes|YES|on|ON) return 0 ;;
    *) return 1 ;;
  esac
}

normalize_ci_logging_env() {
  local resolved_ci_log_level=""
  local resolved_log_level=""

  if [ -n "''${CI_LOG_LEVEL:-}" ]; then
    resolved_ci_log_level="$CI_LOG_LEVEL"
  elif _ci_truthy "''${CI_VERBOSE:-0}" || _ci_truthy "''${NIXFIED_VERBOSE:-0}" || _ci_truthy "''${NIXFIED_DEBUG:-0}"; then
    resolved_ci_log_level="debug"
  elif [ -n "''${NIXFIED_LOG_LEVEL:-}" ]; then
    resolved_ci_log_level="$NIXFIED_LOG_LEVEL"
  elif [ -n "''${LOG_LEVEL:-}" ]; then
    resolved_ci_log_level="$LOG_LEVEL"
  fi

  if [ -n "$resolved_ci_log_level" ]; then
    export CI_LOG_LEVEL="$resolved_ci_log_level"
  fi

  if [ -n "''${LOG_LEVEL:-}" ]; then
    resolved_log_level="$LOG_LEVEL"
  elif [ -n "''${MFM_LOG:-}" ]; then
    resolved_log_level="$MFM_LOG"
  elif [ -n "''${RUST_LOG:-}" ]; then
    resolved_log_level="$RUST_LOG"
  elif [ -n "''${CI_LOG_LEVEL:-}" ]; then
    resolved_log_level="$CI_LOG_LEVEL"
  elif [ -n "''${NIXFIED_LOG_LEVEL:-}" ]; then
    resolved_log_level="$NIXFIED_LOG_LEVEL"
  fi

  if [ -n "$resolved_log_level" ]; then
    export LOG_LEVEL="$resolved_log_level"
    export MFM_LOG="''${MFM_LOG:-$resolved_log_level}"
    export RUST_LOG="''${RUST_LOG:-$resolved_log_level}"
  fi

  if [ -n "''${CI_LOG_LEVEL:-}" ] && [ -z "''${NIXFIED_LOG_LEVEL:-}" ]; then
    export NIXFIED_LOG_LEVEL="$CI_LOG_LEVEL"
  fi

  if [ -n "''${MFM_LOG_FORMAT:-}" ] && [ -z "''${LOG_FORMAT:-}" ]; then
    export LOG_FORMAT="$MFM_LOG_FORMAT"
  fi
  if [ -n "''${LOG_FORMAT:-}" ] && [ -z "''${MFM_LOG_FORMAT:-}" ]; then
    export MFM_LOG_FORMAT="$LOG_FORMAT"
  fi

  if [ -n "''${MFM_LOG_SPAN_EVENTS:-}" ] && [ -z "''${LOG_SPAN_EVENTS:-}" ]; then
    export LOG_SPAN_EVENTS="$MFM_LOG_SPAN_EVENTS"
  fi
  if [ -n "''${LOG_SPAN_EVENTS:-}" ] && [ -z "''${MFM_LOG_SPAN_EVENTS:-}" ]; then
    export MFM_LOG_SPAN_EVENTS="$LOG_SPAN_EVENTS"
  fi
}

init_ci_cargo_paths() {
  local default_target_dir=""
  local default_cargo_home=""

  if [ -z "''${CARGO_TARGET_DIR:-}" ]; then
    if [ -n "''${MFM_EPHEMERAL_ROOT:-}" ]; then
      default_target_dir="$MFM_EPHEMERAL_ROOT/build/cargo-target"
    else
      default_target_dir="$(pwd)/target"
    fi
    export CARGO_TARGET_DIR="$default_target_dir"
  fi

  if [ -z "''${CARGO_HOME:-}" ]; then
    if [ -n "''${MFM_EPHEMERAL_ROOT:-}" ]; then
      default_cargo_home="$MFM_EPHEMERAL_ROOT/build/cargo-home"
    else
      default_cargo_home="$HOME/.cargo"
    fi
    export CARGO_HOME="$default_cargo_home"
  fi

  mkdir -p "$CARGO_TARGET_DIR" "$CARGO_HOME"
  echo "INFO: ci cargo paths target_dir=$CARGO_TARGET_DIR cargo_home=$CARGO_HOME" >&2
}

eval "$($SLOT_INFO)"
normalize_ci_logging_env
init_ci_cargo_paths

kill_conflicting_listener() {
  local port="$1"
  local label="$2"
  local expected_token="''${3:-}"
  local service_name="''${4:-}"
  local pids=""
  local filtered_pids=""
  local remaining=""
  local cmd=""
  local pid=""
  local status_hook=""
  local status_line=""
  local status_running=""
  local status_pid=""
  local status_owner_run_id=""
  local status_slot_owner=""
  local status_owner_scope=""
  local effective_owner=""

  if [ -z "$port" ] || ! echo "$port" | grep -Eq '^[0-9]+$'; then
    return 0
  fi

  if ! command -v lsof >/dev/null 2>&1; then
    echo "WARN: lsof not available; skipping conflict cleanup label=$label port=$port" >&2
    return 0
  fi

  pids="$({
    lsof -tiTCP:"$port" -sTCP:LISTEN -n -P 2>/dev/null || true
    lsof -tiUDP:"$port" -n -P 2>/dev/null || true
  } | sort -u)"
  if [ -z "$pids" ]; then
    return 0
  fi

  if [ -n "$expected_token" ]; then
    while IFS= read -r pid; do
      [ -z "$pid" ] && continue
      cmd=$(ps -o command= -p "$pid" 2>/dev/null || true)
      if echo "$cmd" | grep -F "$expected_token" >/dev/null 2>&1; then
        filtered_pids="$filtered_pids
$pid"
      fi
    done <<<"$pids"
  else
    filtered_pids="$pids"
  fi

  filtered_pids="$(echo "$filtered_pids" | sed '/^$/d' || true)"
  if [ -z "$filtered_pids" ]; then
    return 0
  fi

  if [ -n "$service_name" ] && command -v has_hook >/dev/null 2>&1 && command -v run_hook >/dev/null 2>&1; then
    status_hook="SVC_$(echo "$service_name" | tr '[:lower:]' '[:upper:]' | tr '.:/-' '_')_STATUS"
    if has_hook "$status_hook"; then
      status_line="$(run_hook "$status_hook" 2>/dev/null | tail -n 1 || true)"
      status_running="$(echo "$status_line" | sed -n 's/.* running=\([^ ]*\).*/\1/p')"
      status_pid="$(echo "$status_line" | sed -n 's/.* pid=\([^ ]*\).*/\1/p')"
      status_owner_run_id="$(echo "$status_line" | sed -n 's/.* owner_run_id=\([^ ]*\).*/\1/p')"
      status_slot_owner="$(echo "$status_line" | sed -n 's/.* slot_owner=\([^ ]*\).*/\1/p')"
      status_owner_scope="$(echo "$status_line" | sed -n 's/.* owner_scope=\([^ ]*\).*/\1/p')"

      effective_owner="$status_owner_run_id"
      case "$effective_owner" in
        ""|unknown|none) effective_owner="$status_slot_owner" ;;
      esac
      case "$effective_owner" in
        ""|unknown|none) effective_owner="" ;;
      esac

      if [ "$status_running" = "true" ] \
        && [ "$status_owner_scope" = "persistent" ] \
        && [ -n "$status_pid" ] \
        && [ -n "$effective_owner" ] \
        && [ "$effective_owner" != "''${RUN_ID:-}" ] \
        && printf '%s\n' "$filtered_pids" | grep -Fx "$status_pid" >/dev/null 2>&1; then
        echo "INFO: preserving active service listener label=$label port=$port service=$service_name owner_run_id=$effective_owner pid=$status_pid" >&2
        return 0
      fi
    fi
  fi

  echo "INFO: stopping conflicting listener label=$label port=$port pids=$(echo "$filtered_pids" | tr '\n' ' ')" >&2
  echo "$filtered_pids" | xargs kill -TERM 2>/dev/null || true

  for _ in $(seq 1 10); do
    remaining="$({
      lsof -tiTCP:"$port" -sTCP:LISTEN -n -P 2>/dev/null || true
      lsof -tiUDP:"$port" -n -P 2>/dev/null || true
    } | sort -u)"
    if [ -z "$remaining" ]; then
      break
    fi
    sleep 0.2
  done

  remaining="$({
    lsof -tiTCP:"$port" -sTCP:LISTEN -n -P 2>/dev/null || true
    lsof -tiUDP:"$port" -n -P 2>/dev/null || true
  } | sort -u)"
  if [ -n "$expected_token" ]; then
    filtered_pids=""
    while IFS= read -r pid; do
      [ -z "$pid" ] && continue
      cmd=$(ps -o command= -p "$pid" 2>/dev/null || true)
      if echo "$cmd" | grep -F "$expected_token" >/dev/null 2>&1; then
        filtered_pids="$filtered_pids
$pid"
      fi
    done <<<"$remaining"
    remaining="$(echo "$filtered_pids" | sed '/^$/d' || true)"
  fi

  if [ -n "$remaining" ]; then
    echo "WARN: force-killing listener label=$label port=$port pids=$(echo "$remaining" | tr '\n' ' ')" >&2
    echo "$remaining" | xargs kill -KILL 2>/dev/null || true
  fi
}

# Parity flows rely on MinIO fixture bootstrap. Clean stale listeners from
# previous slot-shared runs before steps execute.
kill_conflicting_listener "''${MINIO_PORT:-}" "minio-api" "minio" "minio"
kill_conflicting_listener "''${MINIO_CONSOLE_PORT:-}" "minio-console" "minio" "minio"
kill_conflicting_listener "''${RETHHTTP_PORT:-}" "reth-http" "reth" "reth"
kill_conflicting_listener "''${RETHWS_PORT:-}" "reth-ws" "reth" "reth"
kill_conflicting_listener "''${RETHAUTH_PORT:-}" "reth-auth" "reth" "reth"
kill_conflicting_listener "30303" "reth-p2p" "reth" "reth"
''
