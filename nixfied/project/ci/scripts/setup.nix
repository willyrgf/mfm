{ pkgs }:
pkgs.writeText "mfm-ci-setup.sh" ''
  eval "$($SLOT_INFO)"

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
