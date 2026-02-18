# Process registry - global process/run visibility + diagnostics
{
  pkgs,
  project ? { },
  loggingPrelude ? null,
}:

let
  projectMeta = project.project or { };
  projectId = projectMeta.id or "project";
  id = import ./id.nix {
    inherit pkgs project;
  };
  servicePolicy = import ./service-policy.nix { inherit pkgs; };
  projectIdUpper =
    let
      replaced = pkgs.lib.replaceStrings [ "-" "." ] [ "_" "_" ] projectId;
    in
    pkgs.lib.strings.toUpper replaced;
  slotVar = projectMeta.slotVar or "NIX_ENV";
  envVar = projectMeta.envVar or "PROJECT_ENV";
  processCfg = project.process or { };
  registryRoot = processCfg.registryRoot or "/tmp/nixfied-runtime/${projectId}";
  baseDirExpr = (project.directories.base or "\${XDG_DATA_HOME:-$HOME/.local/share}/${projectId}");
  ciCfg = project.ci or { };
  artifactsCfg = ciCfg.artifacts or { };
  artifactsRootExpr = artifactsCfg.dir or "/tmp/ci-artifacts";
  ephemeralPrefix = "/tmp/${projectId}-ephemeral-";
  resolvedLoggingPrelude =
    if loggingPrelude != null && loggingPrelude != "" then
      loggingPrelude
    else
      (import ./helpers.nix {
        inherit pkgs project;
        hooks = { };
        summaryParser = "";
      }).loggingPrelude;

  sharedPrelude = ''
    ${resolvedLoggingPrelude}

    set -euo pipefail

    REGISTRY_ROOT="${registryRoot}"
    PROJECT_ID="${projectId}"
    BASE_DIR_DEFAULT="${baseDirExpr}"
    CI_ARTIFACTS_BASE_DEFAULT="${artifactsRootExpr}"
    EPHEMERAL_PREFIX="${ephemeralPrefix}"
    SLOT_VAR="${slotVar}"
    ENV_VAR="${envVar}"
    EPHEMERAL_FLAG_VAR="${projectIdUpper}_EPHEMERAL"
    EPHEMERAL_ROOT_VAR="${projectIdUpper}_EPHEMERAL_ROOT"

    LOCK_DIR="$REGISTRY_ROOT/locks"
    EVENTS_FILE="$REGISTRY_ROOT/events.jsonl"
    SNAPSHOT_FILE="$REGISTRY_ROOT/snapshot.json"

    mkdir -p "$LOCK_DIR"
    touch "$EVENTS_FILE"

    normalize_bool() {
      case "''${1:-}" in
        1|true|TRUE|yes|YES|on|ON) echo "true" ;;
        0|false|FALSE|no|NO|off|OFF) echo "false" ;;
        *) echo "null" ;;
      esac
    }

    ${servicePolicy.policyRuntimeFunctions}

    infer_owner_scope_from_reuse_policy() {
      nixfied_policy_owner_scope_from_reuse "''${SERVICE_REUSE_POLICY:-}"
    }

    infer_discovery_scope_from_reuse_policy() {
      nixfied_policy_discovery_scope_from_reuse "''${SERVICE_REUSE_POLICY:-}"
    }

    infer_owner_scope() {
      local from_reuse=""

      if [ -n "''${SERVICE_OWNER_SCOPE:-}" ]; then
        echo "$SERVICE_OWNER_SCOPE"
        return 0
      fi

      # Precedence: explicit scope env > scope inferred from explicit reuse policy > execution context.
      from_reuse="$(infer_owner_scope_from_reuse_policy)"
      if [ -n "$from_reuse" ]; then
        echo "$from_reuse"
        return 0
      fi

      if [ "''${!EPHEMERAL_FLAG_VAR:-0}" = "1" ]; then
        echo "ephemeral"
      else
        echo "persistent"
      fi
    }

    infer_discovery_scope() {
      local from_reuse=""

      if [ -n "''${SERVICE_DISCOVERY_SCOPE:-}" ]; then
        echo "$SERVICE_DISCOVERY_SCOPE"
        return 0
      fi

      # Keep discovery aligned with explicit reuse policy unless callers set SERVICE_DISCOVERY_SCOPE.
      from_reuse="$(infer_discovery_scope_from_reuse_policy)"
      if [ -n "$from_reuse" ]; then
        echo "$from_reuse"
        return 0
      fi

      if [ "''${!EPHEMERAL_FLAG_VAR:-0}" = "1" ]; then
        echo "local"
      else
        echo "global"
      fi
    }

    infer_reuse_policy() {
      local owner_scope="$1"
      local discovery_scope="''${2:-}"
      nixfied_policy_infer_reuse_policy "''${SERVICE_REUSE_POLICY:-}" "$owner_scope" "$discovery_scope" "same-slot"
    }

    validate_policy_matrix() {
      local reuse="$1"
      local owner="$2"
      local discovery="$3"
      nixfied_policy_validate_matrix "$reuse" "$owner" "$discovery" 0 0
    }

    is_numeric_pid() {
      case "''${1:-}" in
        ""|*[!0-9]*) return 1 ;;
        *) return 0 ;;
      esac
    }

    is_active_run_state() {
      case "''${1:-}" in
        starting|running|ready|degraded|waiting|busy) return 0 ;;
        *) return 1 ;;
      esac
    }

    read_kv_field() {
      local line="''${1:-}"
      local key="''${2:-}"
      local token=""

      for token in $line; do
        if [ "''${token%%=*}" = "$key" ]; then
          printf '%s\n' "''${token#*=}"
          return 0
        fi
      done

      printf '\n'
    }

    rewrite_kv_field() {
      local line="''${1:-}"
      local key="''${2:-}"
      local value="''${3:-}"
      local token=""
      local out=""

      for token in $line; do
        if [ "''${token%%=*}" = "$key" ]; then
          token="$key=$value"
        fi

        if [ -z "$out" ]; then
          out="$token"
        else
          out="$out $token"
        fi
      done

      printf '%s\n' "$out"
    }

    append_event_json() {
      local payload="$1"
      local lock_file="$LOCK_DIR/events.lock"
      local tmp_snapshot="$SNAPSHOT_FILE.tmp.$$"

      (
        exec 9>"$lock_file"
        ${pkgs.flock}/bin/flock -x 9

        local next_seq=1
        local payload_with_seq="$payload"
        if [ -s "$EVENTS_FILE" ]; then
          next_seq="$(${pkgs.jq}/bin/jq -sr '
            if length == 0 then
              1
            else
              ((.[-1].seq // length) + 1)
            end
          ' "$EVENTS_FILE")"
        fi
        payload_with_seq="$(printf '%s\n' "$payload" | ${pkgs.jq}/bin/jq --arg seq "$next_seq" '.seq = ($seq | tonumber)')"
        echo "$payload_with_seq" >> "$EVENTS_FILE"

        ${pkgs.jq}/bin/jq -s --arg generated_at "$(${pkgs.coreutils}/bin/date -u +%Y-%m-%dT%H:%M:%SZ)" '
          {
            schema_version: 2,
            generated_at: $generated_at,
            totals: { events: length },
            events: .
          }
        ' "$EVENTS_FILE" > "$tmp_snapshot"
        mv "$tmp_snapshot" "$SNAPSHOT_FILE"
      )
    }
  '';

  emitEvent = pkgs.writeShellScript "process-registry-emit-event" ''
    ${sharedPrelude}

    EVENT_TYPE=""
    EVENT_STATE=""
    EVENT_SERVICE=""
    EVENT_RUN_ID=""
    EVENT_COMMAND=""
    EVENT_SLOT=""
    EVENT_ENV=""
    EVENT_PROFILE=""
    EVENT_PID=""
    EVENT_PGID=""
    EVENT_PLAN_ID=""
    EVENT_UNIT_ID=""
    EVENT_ATTEMPT=""
    EVENT_OWNER_SCOPE=""
    EVENT_REUSE_POLICY=""
    EVENT_DISCOVERY_SCOPE=""
    EVENT_EPHEMERAL_ROOT=""
    EVENT_WAIT_REASON=""
    EVENT_LOG_PATH=""
    EVENT_LAST_ERROR=""
    EVENT_READINESS_HEALTH=""
    EVENT_READINESS_READY=""

    while [ "$#" -gt 0 ]; do
      case "$1" in
        --event-type) EVENT_TYPE="$2"; shift 2 ;;
        --state) EVENT_STATE="$2"; shift 2 ;;
        --service) EVENT_SERVICE="$2"; shift 2 ;;
        --run-id) EVENT_RUN_ID="$2"; shift 2 ;;
        --command) EVENT_COMMAND="$2"; shift 2 ;;
        --slot) EVENT_SLOT="$2"; shift 2 ;;
        --env) EVENT_ENV="$2"; shift 2 ;;
        --profile) EVENT_PROFILE="$2"; shift 2 ;;
        --pid) EVENT_PID="$2"; shift 2 ;;
        --pgid) EVENT_PGID="$2"; shift 2 ;;
        --plan-id) EVENT_PLAN_ID="$2"; shift 2 ;;
        --unit-id) EVENT_UNIT_ID="$2"; shift 2 ;;
        --attempt) EVENT_ATTEMPT="$2"; shift 2 ;;
        --owner-scope) EVENT_OWNER_SCOPE="$2"; shift 2 ;;
        --reuse-policy) EVENT_REUSE_POLICY="$2"; shift 2 ;;
        --discovery-scope) EVENT_DISCOVERY_SCOPE="$2"; shift 2 ;;
        --ephemeral-root) EVENT_EPHEMERAL_ROOT="$2"; shift 2 ;;
        --wait-reason) EVENT_WAIT_REASON="$2"; shift 2 ;;
        --log-path) EVENT_LOG_PATH="$2"; shift 2 ;;
        --last-error) EVENT_LAST_ERROR="$2"; shift 2 ;;
        --readiness-health) EVENT_READINESS_HEALTH="$2"; shift 2 ;;
        --readiness-ready) EVENT_READINESS_READY="$2"; shift 2 ;;
        *)
          log_error "unknown argument: $1"
          exit 1
          ;;
      esac
    done

    if [ -z "$EVENT_TYPE" ]; then
      echo "Usage: process-registry-emit-event --event-type <type> [--service <name>] [--state <state>] [--run-id <id>] [--slot <slot>] [--env <env>] [--wait-reason <reason>] [--log-path <path>]" >&2
      exit 1
    fi

    if [ -z "$EVENT_PLAN_ID" ]; then
      EVENT_PLAN_ID="''${NIXFIED_PLAN_ID:-}"
    fi

    if [ -z "$EVENT_UNIT_ID" ]; then
      EVENT_UNIT_ID="''${NIXFIED_UNIT_ID:-}"
    fi

    if [ -z "$EVENT_ATTEMPT" ]; then
      EVENT_ATTEMPT="''${NIXFIED_UNIT_ATTEMPT:-}"
    fi
    if [ -z "$EVENT_ATTEMPT" ]; then
      EVENT_ATTEMPT="1"
    fi
    case "$EVENT_ATTEMPT" in
      *[!0-9]*|"")
        log_error "--attempt must be a positive integer (got '$EVENT_ATTEMPT')"
        exit 1
        ;;
      0)
        log_error "--attempt must be >= 1 (got '$EVENT_ATTEMPT')"
        exit 1
        ;;
      *)
        ;;
    esac

    if [ -z "$EVENT_RUN_ID" ]; then
      EVENT_RUN_ID="$(${id.resolveId} "''${RUN_ID:-}" "$EVENT_PLAN_ID")"
      export RUN_ID="$EVENT_RUN_ID"
    fi

    if [ -z "$EVENT_COMMAND" ]; then
      EVENT_COMMAND="''${COMMAND_NAME:-unknown}"
    fi

    if [ -z "$EVENT_SLOT" ]; then
      EVENT_SLOT="''${SLOT:-''${!SLOT_VAR:-}}"
    fi

    if [ -z "$EVENT_ENV" ]; then
      EVENT_ENV="''${ENV:-''${!ENV_VAR:-}}"
    fi

    if [ -z "$EVENT_PID" ]; then
      EVENT_PID="$$"
    fi

    if [ -z "$EVENT_PGID" ]; then
      EVENT_PGID="$(${pkgs.procps}/bin/ps -o pgid= -p "$EVENT_PID" 2>/dev/null | tr -d ' ' || true)"
    fi

    if [ -z "$EVENT_OWNER_SCOPE" ]; then
      EVENT_OWNER_SCOPE="$(infer_owner_scope)"
    fi
    if [ -z "$EVENT_DISCOVERY_SCOPE" ]; then
      EVENT_DISCOVERY_SCOPE="$(infer_discovery_scope)"
    fi
    if [ -z "$EVENT_REUSE_POLICY" ]; then
      EVENT_REUSE_POLICY="$(infer_reuse_policy "$EVENT_OWNER_SCOPE")"
    fi

    validate_policy_matrix "$EVENT_REUSE_POLICY" "$EVENT_OWNER_SCOPE" "$EVENT_DISCOVERY_SCOPE"

    if [ -z "$EVENT_EPHEMERAL_ROOT" ]; then
      EVENT_EPHEMERAL_ROOT="''${!EPHEMERAL_ROOT_VAR:-}"
    fi

    if [ -z "$EVENT_STATE" ]; then
      case "$EVENT_TYPE" in
        run_started) EVENT_STATE="running" ;;
        run_finished) EVENT_STATE="passed" ;;
        slot_acquired) EVENT_STATE="busy" ;;
        slot_released) EVENT_STATE="released" ;;
        service_starting) EVENT_STATE="starting" ;;
        service_ready) EVENT_STATE="ready" ;;
        service_stopped) EVENT_STATE="stopped" ;;
        service_orphaned) EVENT_STATE="orphaned" ;;
        readiness_progress) EVENT_STATE="waiting" ;;
        *) EVENT_STATE="unknown" ;;
      esac
    fi

    EVENT_ID="$(${id.mkUniqueId})"
    EVENT_TS="$(${pkgs.coreutils}/bin/date -u +%Y-%m-%dT%H:%M:%SZ)"
    EVENT_READINESS_HEALTH_NORM="$(normalize_bool "$EVENT_READINESS_HEALTH")"
    EVENT_READINESS_READY_NORM="$(normalize_bool "$EVENT_READINESS_READY")"

    PAYLOAD=$(${pkgs.jq}/bin/jq -cn \
      --argjson schema_version 2 \
      --arg event_id "$EVENT_ID" \
      --arg event_type "$EVENT_TYPE" \
      --arg timestamp "$EVENT_TS" \
      --arg run_id "$EVENT_RUN_ID" \
      --arg plan_id "$EVENT_PLAN_ID" \
      --arg unit_id "$EVENT_UNIT_ID" \
      --arg attempt "$EVENT_ATTEMPT" \
      --arg command_name "$EVENT_COMMAND" \
      --arg project_id "$PROJECT_ID" \
      --arg service "$EVENT_SERVICE" \
      --arg slot "$EVENT_SLOT" \
      --arg env "$EVENT_ENV" \
      --arg profile "$EVENT_PROFILE" \
      --arg pid "$EVENT_PID" \
      --arg pgid "$EVENT_PGID" \
      --arg state "$EVENT_STATE" \
      --arg owner_scope "$EVENT_OWNER_SCOPE" \
      --arg reuse_policy "$EVENT_REUSE_POLICY" \
      --arg discovery_scope "$EVENT_DISCOVERY_SCOPE" \
      --arg ephemeral_root "$EVENT_EPHEMERAL_ROOT" \
      --arg wait_reason "$EVENT_WAIT_REASON" \
      --arg log_path "$EVENT_LOG_PATH" \
      --arg last_error "$EVENT_LAST_ERROR" \
      --arg readiness_health "$EVENT_READINESS_HEALTH_NORM" \
      --arg readiness_ready "$EVENT_READINESS_READY_NORM" \
      '
      {
        schema_version: $schema_version,
        event_id: $event_id,
        event_type: $event_type,
        timestamp: $timestamp,
        run_id: (if $run_id == "" then null else $run_id end),
        plan_id: (if $plan_id == "" then null else $plan_id end),
        unit_id: (if $unit_id == "" then null else $unit_id end),
        attempt: (if $attempt == "" then null else (try ($attempt | tonumber) catch null) end),
        command_name: (if $command_name == "" then null else $command_name end),
        project_id: $project_id,
        service: (if $service == "" then null else $service end),
        slot: (if $slot == "" then null else $slot end),
        env: (if $env == "" then null else $env end),
        profile: (if $profile == "" then null else $profile end),
        pid: (if $pid == "" then null else (try ($pid | tonumber) catch $pid) end),
        pgid: (if $pgid == "" then null else (try ($pgid | tonumber) catch $pgid) end),
        state: (if $state == "" then "unknown" else $state end),
        owner_scope: (if $owner_scope == "" then null else $owner_scope end),
        reuse_policy: (if $reuse_policy == "" then null else $reuse_policy end),
        discovery_scope: (if $discovery_scope == "" then null else $discovery_scope end),
        ephemeral_root: (if $ephemeral_root == "" then null else $ephemeral_root end),
        readiness: {
          health_ok: (if $readiness_health == "null" then null else ($readiness_health == "true") end),
          ready_ok: (if $readiness_ready == "null" then null else ($readiness_ready == "true") end),
          last_error: (if $last_error == "" then null else $last_error end)
        },
        wait_reason: (if $wait_reason == "" then null else $wait_reason end),
        log_path: (if $log_path == "" then null else $log_path end)
      }
      ')

    append_event_json "$PAYLOAD"
    log_ok "process event recorded event_type=$EVENT_TYPE run_id=$EVENT_RUN_ID service=''${EVENT_SERVICE:-none} state=$EVENT_STATE"
  '';

  processStatus = pkgs.writeShellScript "process-status" ''
    ${sharedPrelude}

    SHOW_ALL=false
    while [ "$#" -gt 0 ]; do
      case "$1" in
        --all)
          SHOW_ALL=true
          shift
          ;;
        *)
          log_error "unknown argument: $1"
          exit 1
          ;;
      esac
    done

    if [ ! -s "$EVENTS_FILE" ]; then
      log_ok "no process events project_id=$PROJECT_ID registry_root=$REGISTRY_ROOT"
      exit 0
    fi

    OUT=$(${pkgs.jq}/bin/jq -sr --arg all "$SHOW_ALL" '
      def is_active:
        .state == "starting"
        or .state == "running"
        or .state == "ready"
        or .state == "degraded"
        or .state == "waiting"
        or .state == "busy";

      def service_key:
        (.service // "") + "|" + ((.slot // "") | tostring) + "|" + (.env // "");

      def run_key:
        (.run_id // "");

      def latest_by(f):
        sort_by(f, (.seq // 0), (.timestamp // ""))
        | group_by(f)
        | map(last);

      . as $events
      | ([$events[] | select(.service != null and .service != "")] | latest_by(service_key)) as $services
      | ([$events[] | select(.run_id != null and .run_id != "" and (.event_type == "run_started" or .event_type == "run_finished"))] | latest_by(run_key)) as $runs
      | [
          ($services[]? | select(($all == "true") or is_active)
            | "type=service service=\(.service) slot=\((.slot // "unknown")) env=\((.env // "unknown")) state=\((.state // "unknown")) run_id=\((.run_id // "unknown")) pid=\((.pid // "unknown")) owner_scope=\((.owner_scope // "unknown")) command=\((.command_name // "unknown"))"),
          ($runs[]? | select(($all == "true") or is_active)
            | "type=run run_id=\((.run_id // "unknown")) state=\((.state // "unknown")) command=\((.command_name // "unknown")) slot=\((.slot // "unknown")) env=\((.env // "unknown")) pid=\((.pid // "unknown"))")
        ]
      | .[]
    ' "$EVENTS_FILE")

    if [ -n "$OUT" ]; then
      RECONCILED_OUT=""
      while IFS= read -r LINE; do
        if [ -z "$LINE" ]; then
          continue
        fi

        ENTITY_TYPE="$(read_kv_field "$LINE" "type")"
        if [ "$ENTITY_TYPE" = "run" ]; then
          STATE="$(read_kv_field "$LINE" "state")"
          PID="$(read_kv_field "$LINE" "pid")"
          if is_active_run_state "$STATE" && is_numeric_pid "$PID" && ! kill -0 "$PID" 2>/dev/null; then
            if [ "$SHOW_ALL" = "true" ]; then
              LINE="$(rewrite_kv_field "$LINE" "state" "failed")"
            else
              continue
            fi
          fi
        fi

        if [ -z "$RECONCILED_OUT" ]; then
          RECONCILED_OUT="$LINE"
        else
          RECONCILED_OUT="$RECONCILED_OUT"$'\n'"$LINE"
        fi
      done <<< "$OUT"
      OUT="$RECONCILED_OUT"
    fi

    if [ -z "$OUT" ]; then
      if [ "$SHOW_ALL" = "true" ]; then
        log_ok "no process entities found project_id=$PROJECT_ID"
      else
        log_ok "no active process entities found project_id=$PROJECT_ID"
      fi
      exit 0
    fi

    echo "$OUT"
  '';

  processRuns = pkgs.writeShellScript "process-runs" ''
    ${sharedPrelude}

    SHOW_ALL=false
    while [ "$#" -gt 0 ]; do
      case "$1" in
        --all)
          SHOW_ALL=true
          shift
          ;;
        *)
          log_error "unknown argument: $1"
          exit 1
          ;;
      esac
    done

    if [ ! -s "$EVENTS_FILE" ]; then
      log_ok "no runs found project_id=$PROJECT_ID"
      exit 0
    fi

    OUT=$(${pkgs.jq}/bin/jq -sr --arg all "$SHOW_ALL" '
      def is_active:
        .state == "starting"
        or .state == "running"
        or .state == "ready"
        or .state == "degraded"
        or .state == "waiting"
        or .state == "busy";

      def run_key:
        (.run_id // "");

      def latest_by(f):
        sort_by(f, (.seq // 0), (.timestamp // ""))
        | group_by(f)
        | map(last);

      ([ .[] | select(.run_id != null and .run_id != "" and (.event_type == "run_started" or .event_type == "run_finished")) ]
        | latest_by(run_key)
        | .[]
        | select(($all == "true") or is_active)
        | "run_id=\((.run_id // "unknown")) state=\((.state // "unknown")) command=\((.command_name // "unknown")) slot=\((.slot // "unknown")) env=\((.env // "unknown")) pid=\((.pid // "unknown")) started_at=\((.timestamp // "unknown"))")
    ' "$EVENTS_FILE")

    if [ -n "$OUT" ]; then
      RECONCILED_OUT=""
      while IFS= read -r LINE; do
        if [ -z "$LINE" ]; then
          continue
        fi

        STATE="$(read_kv_field "$LINE" "state")"
        PID="$(read_kv_field "$LINE" "pid")"
        if is_active_run_state "$STATE" && is_numeric_pid "$PID" && ! kill -0 "$PID" 2>/dev/null; then
          if [ "$SHOW_ALL" = "true" ]; then
            LINE="$(rewrite_kv_field "$LINE" "state" "failed")"
          else
            continue
          fi
        fi

        if [ -z "$RECONCILED_OUT" ]; then
          RECONCILED_OUT="$LINE"
        else
          RECONCILED_OUT="$RECONCILED_OUT"$'\n'"$LINE"
        fi
      done <<< "$OUT"
      OUT="$RECONCILED_OUT"
    fi

    if [ -z "$OUT" ]; then
      if [ "$SHOW_ALL" = "true" ]; then
        log_ok "no run entities found project_id=$PROJECT_ID"
      else
        log_ok "no active runs found project_id=$PROJECT_ID"
      fi
      exit 0
    fi

    echo "$OUT"
  '';

  processSlots = pkgs.writeShellScript "process-slots" ''
    ${sharedPrelude}

    SHOW_ALL=false
    while [ "$#" -gt 0 ]; do
      case "$1" in
        --all)
          SHOW_ALL=true
          shift
          ;;
        *)
          log_error "unknown argument: $1"
          exit 1
          ;;
      esac
    done

    if [ ! -s "$EVENTS_FILE" ]; then
      log_ok "no slot events found project_id=$PROJECT_ID"
      exit 0
    fi

    OUT=$(${pkgs.jq}/bin/jq -sr --arg all "$SHOW_ALL" '
      def slot_key:
        ((.slot // "") | tostring) + "|" + (.env // "");

      def latest_by(f):
        sort_by(f, (.seq // 0), (.timestamp // ""))
        | group_by(f)
        | map(last);

      def is_released:
        .event_type == "slot_released"
        or .state == "released"
        or .state == "stopped"
        or .state == "failed"
        or .state == "passed";

      ([ .[] | select(.event_type == "slot_acquired" or .event_type == "slot_released") ] | latest_by(slot_key)) as $slot_events
      | (if ($slot_events | length) > 0 then
           $slot_events
         else
           ([ .[] | select(.service != null and .slot != null) ] | latest_by(slot_key))
         end)
      | .[]
      | (if is_released then "false" else "true" end) as $busy
      | select(($all == "true") or ($busy == "true"))
      | "slot=\((.slot // "unknown")) env=\((.env // "unknown")) busy=\($busy) owner_run_id=\((.run_id // "unknown")) owner_scope=\((.owner_scope // "unknown")) state=\((.state // "unknown")) since=\((.timestamp // "unknown")) command=\((.command_name // "unknown"))"
    ' "$EVENTS_FILE")

    if [ -z "$OUT" ]; then
      if [ "$SHOW_ALL" = "true" ]; then
        log_ok "no slot ownership entities found project_id=$PROJECT_ID"
      else
        log_ok "no busy slots found project_id=$PROJECT_ID"
      fi
      exit 0
    fi

    echo "$OUT"
  '';

  processInspect = pkgs.writeShellScript "process-inspect" ''
    ${sharedPrelude}

    TARGET=""
    while [ "$#" -gt 0 ]; do
      case "$1" in
        --id)
          TARGET="$2"
          shift 2
          ;;
        --)
          shift
          break
          ;;
        -*)
          log_error "unknown argument: $1"
          exit 1
          ;;
        *)
          if [ -z "$TARGET" ]; then
            TARGET="$1"
          else
            log_error "unexpected extra argument: $1"
            exit 1
          fi
          shift
          ;;
      esac
    done

    if [ -z "$TARGET" ]; then
      echo "Usage: process-inspect -- <id>" >&2
      echo "   or: process-inspect --id <id>" >&2
      exit 1
    fi

    if [ ! -s "$EVENTS_FILE" ]; then
      log_error "no process events found project_id=$PROJECT_ID"
      exit 1
    fi

    MATCH_COUNT=$(${pkgs.jq}/bin/jq -sr --arg id "$TARGET" '
      def matches:
        ((.run_id // "") == $id)
        or ((.event_id // "") == $id)
        or ((.service // "") == $id);
      [ .[] | select(matches) ] | length
    ' "$EVENTS_FILE")

    if [ "$MATCH_COUNT" = "0" ]; then
      log_error "no events found for id=$TARGET"
      exit 1
    fi

    ${pkgs.jq}/bin/jq -s --arg id "$TARGET" '
      def matches:
        ((.run_id // "") == $id)
        or ((.event_id // "") == $id)
        or ((.service // "") == $id);
      [ .[] | select(matches) ] as $matched
      | {
          inspect_id: $id,
          matched_events: ($matched | length),
          latest: ($matched | sort_by((.seq // 0), (.timestamp // "")) | last),
          events: ($matched | sort_by((.seq // 0), (.timestamp // "")))
        }
    ' "$EVENTS_FILE"
  '';

  processStop = pkgs.writeShellScript "process-stop" ''
    ${sharedPrelude}

    RUN_ID=""
    SCOPE="run"
    SCOPE_SET=false
    STOP_ALL=false
    DRY_RUN=false
    FORCE=false
    TIMEOUT_SECONDS="5"

    while [ "$#" -gt 0 ]; do
      case "$1" in
        --run-id)
          RUN_ID="$2"
          shift 2
          ;;
        --scope)
          SCOPE="$2"
          SCOPE_SET=true
          shift 2
          ;;
        --all)
          STOP_ALL=true
          shift
          ;;
        --dry-run)
          DRY_RUN=true
          shift
          ;;
        --force)
          FORCE=true
          shift
          ;;
        --timeout)
          TIMEOUT_SECONDS="$2"
          shift 2
          ;;
        *)
          log_error "unknown argument: $1"
          exit 1
          ;;
      esac
    done

    if [ "$STOP_ALL" = "true" ] && [ -n "$RUN_ID" ]; then
      log_error "--all cannot be combined with --run-id"
      exit 1
    fi

    if [ "$STOP_ALL" = "true" ] && [ "$SCOPE_SET" = "true" ]; then
      log_error "--all cannot be combined with --scope"
      exit 1
    fi

    if [ "$STOP_ALL" != "true" ] && [ -z "$RUN_ID" ]; then
      echo "Usage: process-stop (--run-id <id> [--scope run|slot-env] | --all) [--dry-run] [--force] [--timeout <seconds>]" >&2
      exit 1
    fi

    if [ "$STOP_ALL" != "true" ]; then
      case "$SCOPE" in
        run|slot-env) ;;
        *)
          log_error "--scope must be one of run|slot-env (got '$SCOPE')"
          exit 1
          ;;
      esac
    fi

    case "$TIMEOUT_SECONDS" in
      *[!0-9]*|"")
        log_error "--timeout must be a positive integer (got '$TIMEOUT_SECONDS')"
        exit 1
        ;;
    esac

    if [ ! -s "$EVENTS_FILE" ]; then
      log_error "no process events found project_id=$PROJECT_ID"
      exit 1
    fi

    PLAN_JSON=$(${pkgs.jq}/bin/jq -sr --arg run_id "$RUN_ID" --arg scope "$SCOPE" --arg stop_all "$STOP_ALL" '
      def is_active:
        .state == "starting"
        or .state == "running"
        or .state == "ready"
        or .state == "degraded"
        or .state == "waiting"
        or .state == "busy";

      def service_key:
        (.service // "") + "|" + ((.slot // "") | tostring) + "|" + (.env // "");

      def run_key:
        (.run_id // "");

      def latest_by(f):
        sort_by(f, (.seq // 0), (.timestamp // ""))
        | group_by(f)
        | map(last);

      def target_run:
        [ .[] | select((.run_id // "") == $run_id) ]
        | sort_by((.seq // 0), (.timestamp // ""))
        | last;

      if ($stop_all == "true") then
        {
          error: null,
          mode: "all",
          target_slot: "",
          target_env: "",
          service_rows:
            (
              [ .[] | select(.service != null and .service != "") ]
              | latest_by(service_key)
              | map(select(is_active and (.pid != null)))
              | map([
                  "service",
                  (.service // ""),
                  (.run_id // ""),
                  ((.slot // "") | tostring),
                  (.env // ""),
                  (.pid | tostring),
                  ((.pgid // "") | tostring),
                  (.command_name // ""),
                  (.log_path // "")
                ] | @tsv)
            ),
          run_rows:
            (
              [
                .[]
                | select(.run_id != null and .run_id != "")
                | select(.event_type == "run_started" or .event_type == "run_finished")
              ]
              | latest_by(run_key)
              | map(select(is_active and (.pid != null)))
              | map([
                  "run",
                  (.run_id // ""),
                  (.run_id // ""),
                  ((.slot // "") | tostring),
                  (.env // ""),
                  (.pid | tostring),
                  ((.pgid // "") | tostring),
                  (.command_name // ""),
                  ""
                ] | @tsv)
            )
        }
      else
        (target_run) as $target
        | if ($target == null) then
            { error: "missing_run" }
          else
            ($target.slot // "") as $slot
            | ($target.env // "") as $env
            | if ($scope == "slot-env" and ($slot == "" or $env == "")) then
                { error: "missing_slot_env" }
              else
                {
                  error: null,
                  mode: "run-id",
                  target_slot: $slot,
                  target_env: $env,
                  service_rows:
                    (
                      [ .[] | select(.service != null and .service != "") ]
                      | latest_by(service_key)
                      | map(select(is_active and (.pid != null)))
                      | map(
                          select(
                            if $scope == "run" then
                              ((.run_id // "") == $run_id)
                            else
                              (((.slot // "") | tostring) == $slot and (.env // "") == $env)
                            end
                          )
                        )
                      | map([
                          "service",
                          (.service // ""),
                          (.run_id // ""),
                          ((.slot // "") | tostring),
                          (.env // ""),
                          (.pid | tostring),
                          ((.pgid // "") | tostring),
                          (.command_name // ""),
                          (.log_path // "")
                        ] | @tsv)
                    ),
                  run_rows:
                    (
                      [
                        .[]
                        | select(.run_id != null and .run_id != "")
                        | select(.event_type == "run_started" or .event_type == "run_finished")
                      ]
                      | latest_by(run_key)
                      | map(select(is_active and (.pid != null)))
                      | map(
                          select(
                            if $scope == "run" then
                              ((.run_id // "") == $run_id)
                            else
                              (((.slot // "") | tostring) == $slot and (.env // "") == $env)
                            end
                          )
                        )
                      | map([
                          "run",
                          (.run_id // ""),
                          (.run_id // ""),
                          ((.slot // "") | tostring),
                          (.env // ""),
                          (.pid | tostring),
                          ((.pgid // "") | tostring),
                          (.command_name // ""),
                          ""
                        ] | @tsv)
                    )
                }
              end
          end
      end
    ' "$EVENTS_FILE")

    PLAN_ERROR=$(echo "$PLAN_JSON" | ${pkgs.jq}/bin/jq -r '.error // ""')
    case "$PLAN_ERROR" in
      "") ;;
      missing_run)
        log_error "run id not found run_id=$RUN_ID"
        exit 1
        ;;
      missing_slot_env)
        log_error "run id missing slot/env metadata run_id=$RUN_ID; cannot use --scope slot-env"
        exit 1
        ;;
      *)
        if [ "$STOP_ALL" = "true" ]; then
          log_error "failed to build stop plan mode=all error=$PLAN_ERROR"
        else
          log_error "failed to build stop plan run_id=$RUN_ID error=$PLAN_ERROR"
        fi
        exit 1
        ;;
    esac

    TARGET_MODE=$(echo "$PLAN_JSON" | ${pkgs.jq}/bin/jq -r '.mode // ""')
    TARGET_SLOT=$(echo "$PLAN_JSON" | ${pkgs.jq}/bin/jq -r '.target_slot // ""')
    TARGET_ENV=$(echo "$PLAN_JSON" | ${pkgs.jq}/bin/jq -r '.target_env // ""')
    TARGET_ROWS=$(echo "$PLAN_JSON" | ${pkgs.jq}/bin/jq -r '(.service_rows + .run_rows)[]?')

    if [ "$TARGET_MODE" = "all" ]; then
      STOP_CONTEXT="mode=all"
    else
      STOP_CONTEXT="mode=run-id run_id=$RUN_ID scope=$SCOPE slot=''${TARGET_SLOT:-unknown} env=''${TARGET_ENV:-unknown}"
    fi

    if [ -z "$TARGET_ROWS" ]; then
      log_ok "no active entities matched $STOP_CONTEXT"
      exit 0
    fi

    SELF_PID="$$"
    SELF_PGID="$(${pkgs.procps}/bin/ps -o pgid= -p "$SELF_PID" 2>/dev/null | tr -d ' ' || true)"

    STOPPED=0
    FAILED=0
    SKIPPED=0
    SEEN=""

    while IFS=$'\t' read -r KIND IDENTIFIER RUN_REF SLOT ENV PID PGID COMMAND LOG_PATH; do
      [ -n "$KIND" ] || continue

      KEY="$KIND|$IDENTIFIER|$RUN_REF|$SLOT|$ENV"
      case "$SEEN" in
        *"|$KEY|"*)
          continue
          ;;
      esac
      SEEN="$SEEN|$KEY|"

      if ! is_numeric_pid "$PID"; then
        SKIPPED=$((SKIPPED + 1))
        log_warn "skipping entity with invalid pid kind=$KIND id=$IDENTIFIER pid=''${PID:-unknown}"
        continue
      fi

      if [ "$PID" = "$SELF_PID" ]; then
        SKIPPED=$((SKIPPED + 1))
        log_warn "refusing to stop current process kind=$KIND id=$IDENTIFIER pid=$PID"
        continue
      fi

      TARGET="$PID"
      TARGET_DESC="pid=$PID"
      if is_numeric_pid "$PGID" && [ "$PGID" -gt 1 ] && [ "$PGID" != "$SELF_PGID" ]; then
        TARGET="-$PGID"
        TARGET_DESC="pgid=$PGID"
      fi

      if ! kill -0 "$PID" 2>/dev/null; then
        SKIPPED=$((SKIPPED + 1))
        log_info "entity already stopped kind=$KIND id=$IDENTIFIER pid=$PID"
        continue
      fi

      if [ "$DRY_RUN" = "true" ]; then
        log_info "dry-run would stop kind=$KIND id=$IDENTIFIER run_id=''${RUN_REF:-unknown} slot=''${SLOT:-unknown} env=''${ENV:-unknown} target=$TARGET_DESC"
        continue
      fi

      log_info "stopping kind=$KIND id=$IDENTIFIER run_id=''${RUN_REF:-unknown} slot=''${SLOT:-unknown} env=''${ENV:-unknown} target=$TARGET_DESC"
      kill -TERM -- "$TARGET" 2>/dev/null || true

      ELAPSED=0
      while kill -0 "$PID" 2>/dev/null && [ "$ELAPSED" -lt "$TIMEOUT_SECONDS" ]; do
        ${pkgs.coreutils}/bin/sleep 1
        ELAPSED=$((ELAPSED + 1))
      done

      if kill -0 "$PID" 2>/dev/null; then
        if [ "$FORCE" = "true" ]; then
          log_warn "escalating to SIGKILL kind=$KIND id=$IDENTIFIER target=$TARGET_DESC"
          kill -KILL -- "$TARGET" 2>/dev/null || true
          ELAPSED=0
          while kill -0 "$PID" 2>/dev/null && [ "$ELAPSED" -lt "2" ]; do
            ${pkgs.coreutils}/bin/sleep 1
            ELAPSED=$((ELAPSED + 1))
          done
        fi
      fi

      if kill -0 "$PID" 2>/dev/null; then
        FAILED=$((FAILED + 1))
        log_error "failed to stop entity kind=$KIND id=$IDENTIFIER pid=$PID timeout=$TIMEOUT_SECONDS force=$FORCE"
        continue
      fi

      STOPPED=$((STOPPED + 1))
      log_ok "stopped kind=$KIND id=$IDENTIFIER pid=$PID"

      if [ "$KIND" = "service" ]; then
        ${emitEvent} \
          --event-type service_stopped \
          --state stopped \
          --service "$IDENTIFIER" \
          --run-id "$RUN_REF" \
          --slot "$SLOT" \
          --env "$ENV" \
          --pid "$PID" \
          --command "$COMMAND" \
          --log-path "$LOG_PATH" \
          --wait-reason "process_stop" >/dev/null 2>&1 || true
      else
        ${emitEvent} \
          --event-type run_finished \
          --state stopped \
          --run-id "$RUN_REF" \
          --command "$COMMAND" \
          --slot "$SLOT" \
          --env "$ENV" \
          --pid "$PID" \
          --wait-reason "process_stop" \
          --last-error "stopped by process::stop" >/dev/null 2>&1 || true
      fi
    done <<< "$TARGET_ROWS"

    if [ "$DRY_RUN" = "true" ]; then
      log_info "process stop dry-run complete $STOP_CONTEXT"
      exit 0
    fi

    if [ "$FAILED" -gt 0 ]; then
      log_error "process stop completed with failures $STOP_CONTEXT stopped=$STOPPED skipped=$SKIPPED failed=$FAILED"
      exit 1
    fi

    log_ok "process stop complete $STOP_CONTEXT stopped=$STOPPED skipped=$SKIPPED"
  '';

  processGc = pkgs.writeShellScript "process-gc" ''
    ${sharedPrelude}

    APPLY=false
    while [ "$#" -gt 0 ]; do
      case "$1" in
        --apply)
          APPLY=true
          shift
          ;;
        *)
          log_error "unknown argument: $1"
          exit 1
          ;;
      esac
    done

    if [ ! -s "$EVENTS_FILE" ]; then
      log_ok "no process events found project_id=$PROJECT_ID"
      exit 0
    fi

    CANDIDATES=$(${pkgs.jq}/bin/jq -sr '
      def is_active:
        .state == "starting"
        or .state == "running"
        or .state == "ready"
        or .state == "degraded"
        or .state == "waiting"
        or .state == "busy";

      def service_key:
        (.service // "") + "|" + ((.slot // "") | tostring) + "|" + (.env // "");

      def latest_by(f):
        sort_by(f, (.seq // 0), (.timestamp // ""))
        | group_by(f)
        | map(last);

      ([ .[] | select(.service != null and .service != "") ] | latest_by(service_key))
      | .[]
      | select(is_active and (.pid != null))
      | [.service, (.slot // ""), (.env // ""), (.run_id // ""), (.pid | tostring), (.log_path // "")]
      | @tsv
    ' "$EVENTS_FILE")

    FOUND=0
    APPLIED=0
    RUN_FOUND=0
    RUN_APPLIED=0

    if [ -n "$CANDIDATES" ]; then
      while IFS=$'\t' read -r SERVICE SLOT ENV RUN_ID PID LOG_PATH; do
        if [ -z "$PID" ]; then
          continue
        fi

        if kill -0 "$PID" 2>/dev/null; then
          continue
        fi

        FOUND=$((FOUND + 1))
        log_warn "orphan detected service=$SERVICE slot=''${SLOT:-unknown} env=''${ENV:-unknown} pid=$PID run_id=''${RUN_ID:-unknown}"

        if [ "$APPLY" = "true" ]; then
          ${emitEvent} \
            --event-type service_orphaned \
            --state orphaned \
            --service "$SERVICE" \
            --slot "$SLOT" \
            --env "$ENV" \
            --run-id "$RUN_ID" \
            --pid "$PID" \
            --log-path "$LOG_PATH" \
            --wait-reason "gc_detected_dead_pid" >/dev/null 2>&1 || true
          APPLIED=$((APPLIED + 1))
          log_ok "orphan marked service=$SERVICE slot=''${SLOT:-unknown} env=''${ENV:-unknown} pid=$PID"
        fi
      done <<< "$CANDIDATES"
    fi

    RUN_CANDIDATES=$(${pkgs.jq}/bin/jq -sr '
      def is_active:
        .state == "starting"
        or .state == "running"
        or .state == "waiting"
        or .state == "busy";

      def run_key:
        (.run_id // "");

      def latest_by(f):
        sort_by(f, (.seq // 0), (.timestamp // ""))
        | group_by(f)
        | map(last);

      ([ .[] | select(.run_id != null and .run_id != "" and (.event_type == "run_started" or .event_type == "run_finished")) ] | latest_by(run_key))
      | .[]
      | select(is_active and (.pid != null))
      | [.run_id, (.pid | tostring), (.command_name // "unknown"), ((.slot // "unknown") | tostring), (.env // "unknown")]
      | @tsv
    ' "$EVENTS_FILE")

    while IFS=$'\t' read -r RUN_ID PID COMMAND SLOT ENV; do
      if [ -z "$RUN_ID" ] || [ -z "$PID" ]; then
        continue
      fi

      if kill -0 "$PID" 2>/dev/null; then
        continue
      fi

      RUN_FOUND=$((RUN_FOUND + 1))
      log_warn "stale run detected run_id=$RUN_ID pid=$PID command=''${COMMAND:-unknown} slot=''${SLOT:-unknown} env=''${ENV:-unknown}"

      if [ "$APPLY" = "true" ]; then
        ${emitEvent} \
          --event-type run_finished \
          --state failed \
          --run-id "$RUN_ID" \
          --command "$COMMAND" \
          --slot "$SLOT" \
          --env "$ENV" \
          --pid "$PID" \
          --wait-reason "gc_detected_dead_run_pid" \
          --last-error "run process no longer exists" >/dev/null 2>&1 || true
        RUN_APPLIED=$((RUN_APPLIED + 1))
        log_ok "stale run marked failed run_id=$RUN_ID"
      fi
    done <<< "$RUN_CANDIDATES"

    if [ "$FOUND" -eq 0 ] && [ "$RUN_FOUND" -eq 0 ]; then
      log_ok "no orphaned active processes found project_id=$PROJECT_ID"
      exit 0
    fi

    if [ "$APPLY" = "true" ]; then
      log_ok "process gc complete orphaned_marked=$APPLIED stale_runs_marked=$RUN_APPLIED"
    else
      log_info "process gc dry-run complete orphans_found=$FOUND stale_runs_found=$RUN_FOUND (re-run with --apply to reconcile)"
    fi
  '';

  serviceEvents = pkgs.writeShellScript "process-service-events" ''
    ${sharedPrelude}

    SERVICE=""
    SLOT_FILTER=""
    ENV_FILTER=""
    LIMIT="200"

    while [ "$#" -gt 0 ]; do
      case "$1" in
        --service) SERVICE="$2"; shift 2 ;;
        --slot) SLOT_FILTER="$2"; shift 2 ;;
        --env) ENV_FILTER="$2"; shift 2 ;;
        --limit) LIMIT="$2"; shift 2 ;;
        *)
          log_error "unknown argument: $1"
          exit 1
          ;;
      esac
    done

    if [ -z "$SERVICE" ]; then
      echo "Usage: process-service-events --service <name> [--slot <slot>] [--env <env>] [--limit <n>]" >&2
      exit 1
    fi

    if [ ! -s "$EVENTS_FILE" ]; then
      log_ok "no events found service=$SERVICE project_id=$PROJECT_ID"
      exit 0
    fi

    OUT=$(${pkgs.jq}/bin/jq -src \
      --arg service "$SERVICE" \
      --arg slot "$SLOT_FILTER" \
      --arg env "$ENV_FILTER" \
      --arg limit "$LIMIT" '
      [ .[]
        | select((.service // "") == $service)
        | select(($slot == "") or (((.slot // "") | tostring) == $slot))
        | select(($env == "") or ((.env // "") == $env))
      ]
      | sort_by((.seq // 0), (.timestamp // ""))
      | if (($limit | tonumber?) // 0) > 0 then
          .[-(($limit | tonumber?) // 0):]
        else
          .
        end
      | .[]
    ' "$EVENTS_FILE")

    if [ -z "$OUT" ]; then
      log_ok "no events matched service=$SERVICE slot=''${SLOT_FILTER:-any} env=''${ENV_FILTER:-any}"
      exit 0
    fi

    echo "$OUT"
  '';

  serviceLogs = pkgs.writeShellScript "process-service-logs" ''
    ${sharedPrelude}

    SERVICE=""
    SLOT_FILTER=""
    ENV_FILTER=""
    FOLLOW=false
    LINES="200"

    while [ "$#" -gt 0 ]; do
      case "$1" in
        --service) SERVICE="$2"; shift 2 ;;
        --slot) SLOT_FILTER="$2"; shift 2 ;;
        --env) ENV_FILTER="$2"; shift 2 ;;
        --lines) LINES="$2"; shift 2 ;;
        --follow|-f) FOLLOW=true; shift ;;
        *)
          log_error "unknown argument: $1"
          exit 1
          ;;
      esac
    done

    if [ -z "$SERVICE" ]; then
      echo "Usage: process-service-logs --service <name> [--slot <slot>] [--env <env>] [--lines <n>] [--follow]" >&2
      exit 1
    fi

    case "$LINES" in
      *[!0-9]*|"")
        log_error "--lines must be a positive integer (got '$LINES')"
        exit 1
        ;;
      *)
        ;;
    esac

    if [ -z "$SLOT_FILTER" ]; then
      SLOT_FILTER="''${SLOT:-''${!SLOT_VAR:-}}"
    fi
    if [ -z "$ENV_FILTER" ]; then
      ENV_FILTER="''${ENV:-''${!ENV_VAR:-}}"
    fi

    if [ ! -s "$EVENTS_FILE" ]; then
      log_error "no process events found; cannot resolve log path service=$SERVICE"
      exit 1
    fi

    LOG_PATH=$(${pkgs.jq}/bin/jq -sr \
      --arg service "$SERVICE" \
      --arg slot "$SLOT_FILTER" \
      --arg env "$ENV_FILTER" '
      [ .[]
        | select((.service // "") == $service)
        | select(($slot == "") or (((.slot // "") | tostring) == $slot))
        | select(($env == "") or ((.env // "") == $env))
        | select((.log_path // "") != "")
      ]
      | sort_by((.seq // 0), (.timestamp // ""))
      | (last | .log_path) // ""
    ' "$EVENTS_FILE")

    if [ -z "$LOG_PATH" ]; then
      log_error "no log path recorded for service=$SERVICE slot=''${SLOT_FILTER:-any} env=''${ENV_FILTER:-any}"
      log_hint "start the service once so it emits lifecycle events with log_path."
      exit 1
    fi

    case "$LOG_PATH" in
      /*) ;;
      *)
        log_error "log path must be absolute path=$LOG_PATH service=$SERVICE"
        exit 1
        ;;
    esac
    if echo "$LOG_PATH" | ${pkgs.gnugrep}/bin/grep -Eq '(^|/)[.]{1,2}(/|$)'; then
      log_error "log path contains unsafe traversal segments path=$LOG_PATH service=$SERVICE"
      exit 1
    fi

    LOG_PATH_CANON="$(${pkgs.coreutils}/bin/realpath "$LOG_PATH" 2>/dev/null || true)"
    if [ -z "$LOG_PATH_CANON" ]; then
      log_error "failed to resolve canonical log path path=$LOG_PATH service=$SERVICE"
      exit 1
    fi

    ALLOWED_ROOTS=()
    add_allowed_root() {
      local root_path="$1"
      local root_canon
      if [ -z "$root_path" ] || [ ! -d "$root_path" ]; then
        return 0
      fi
      root_canon="$(${pkgs.coreutils}/bin/realpath "$root_path" 2>/dev/null || true)"
      if [ -n "$root_canon" ]; then
        ALLOWED_ROOTS+=("$root_canon")
      fi
      return 0
    }

    is_under_root() {
      local candidate="$1"
      local root="$2"
      case "$candidate" in
        "$root"|"$root"/*) return 0 ;;
        *) return 1 ;;
      esac
    }

    add_allowed_root "''${BASE_DIR:-$BASE_DIR_DEFAULT}"
    add_allowed_root "$CI_ARTIFACTS_BASE_DEFAULT"
    add_allowed_root "''${CI_ARTIFACTS_BASE:-}"
    add_allowed_root "''${CI_ARTIFACTS_DIR:-}"

    case "$LOG_PATH_CANON" in
      "$EPHEMERAL_PREFIX"*)
        eph_suffix="''${LOG_PATH_CANON#$EPHEMERAL_PREFIX}"
        eph_id="''${eph_suffix%%/*}"
        if [ -n "$eph_id" ] && [ "$eph_id" != "$eph_suffix" -o "$LOG_PATH_CANON" = "$EPHEMERAL_PREFIX$eph_id" ]; then
          add_allowed_root "$EPHEMERAL_PREFIX$eph_id"
        fi
        ;;
      *)
        ;;
    esac

    ALLOWED=0
    for root in "''${ALLOWED_ROOTS[@]}"; do
      if is_under_root "$LOG_PATH_CANON" "$root"; then
        ALLOWED=1
        break
      fi
    done
    if [ "$ALLOWED" -ne 1 ]; then
      log_error "log path is outside allowed roots path=$LOG_PATH_CANON service=$SERVICE"
      log_hint "allowed roots are BASE_DIR/CI_ARTIFACTS and project ephemeral prefixes only."
      exit 1
    fi

    if [ ! -f "$LOG_PATH" ]; then
      log_error "log file not found path=$LOG_PATH_CANON service=$SERVICE"
      exit 1
    fi

    if [ "$FOLLOW" = "true" ]; then
      exec ${pkgs.coreutils}/bin/tail -n "$LINES" -f "$LOG_PATH_CANON"
    fi
    exec ${pkgs.coreutils}/bin/tail -n "$LINES" "$LOG_PATH_CANON"
  '';

  serviceStatus = pkgs.writeShellScript "process-service-status" ''
    ${sharedPrelude}

    SERVICE=""
    SLOT_FILTER=""
    ENV_FILTER=""

    emit_var() {
      local key="$1"
      local value="$2"
      printf '%s=%q\n' "$key" "$value"
    }

    while [ "$#" -gt 0 ]; do
      case "$1" in
        --service) SERVICE="$2"; shift 2 ;;
        --slot) SLOT_FILTER="$2"; shift 2 ;;
        --env) ENV_FILTER="$2"; shift 2 ;;
        *)
          log_error "unknown argument: $1"
          exit 1
          ;;
      esac
    done

    if [ -z "$SERVICE" ]; then
      echo "Usage: process-service-status --service <name> [--slot <slot>] [--env <env>]" >&2
      exit 1
    fi

    if [ -z "$SLOT_FILTER" ]; then
      SLOT_FILTER="''${SLOT:-''${!SLOT_VAR:-}}"
    fi
    if [ -z "$ENV_FILTER" ]; then
      ENV_FILTER="''${ENV:-''${!ENV_VAR:-}}"
    fi

    DISCOVERY_SCOPE="$(infer_discovery_scope)"

    if [ "$DISCOVERY_SCOPE" = "local" ]; then
      emit_var "REGISTRY_FOUND" "0"
      emit_var "REGISTRY_RUNNING" "false"
      emit_var "REGISTRY_SCOPE" "local"
      emit_var "REGISTRY_STATE" "local_only"
      emit_var "OWNER_RUN_ID" ""
      emit_var "OWNER_SCOPE" ""
      emit_var "EPHEMERAL_ROOT" ""
      emit_var "WAIT_REASON" ""
      emit_var "LOG_PATH" ""
      emit_var "SLOT_OWNER" ""
      exit 0
    fi

    if [ ! -s "$EVENTS_FILE" ]; then
      emit_var "REGISTRY_FOUND" "0"
      emit_var "REGISTRY_RUNNING" "false"
      emit_var "REGISTRY_SCOPE" "global"
      emit_var "REGISTRY_STATE" "unknown"
      emit_var "OWNER_RUN_ID" ""
      emit_var "OWNER_SCOPE" ""
      emit_var "EPHEMERAL_ROOT" ""
      emit_var "WAIT_REASON" ""
      emit_var "LOG_PATH" ""
      emit_var "SLOT_OWNER" ""
      exit 0
    fi

    MATCH=$(${pkgs.jq}/bin/jq -src \
      --arg service "$SERVICE" \
      --arg slot "$SLOT_FILTER" \
      --arg env "$ENV_FILTER" '
      [ .[]
        | select((.service // "") == $service)
        | select(($slot == "") or (((.slot // "") | tostring) == $slot))
        | select(($env == "") or ((.env // "") == $env))
      ]
      | sort_by((.seq // 0), (.timestamp // ""))
      | (last // {})
    ' "$EVENTS_FILE")

    if [ "$MATCH" = "{}" ]; then
      emit_var "REGISTRY_FOUND" "0"
      emit_var "REGISTRY_RUNNING" "false"
      emit_var "REGISTRY_SCOPE" "global"
      emit_var "REGISTRY_STATE" "unknown"
      emit_var "OWNER_RUN_ID" ""
      emit_var "OWNER_SCOPE" ""
      emit_var "EPHEMERAL_ROOT" ""
      emit_var "WAIT_REASON" ""
      emit_var "LOG_PATH" ""
      emit_var "SLOT_OWNER" ""
      exit 0
    fi

    STATE=$(echo "$MATCH" | ${pkgs.jq}/bin/jq -r '.state // "unknown"')
    OWNER_RUN_ID=$(echo "$MATCH" | ${pkgs.jq}/bin/jq -r '.run_id // ""')
    OWNER_SCOPE=$(echo "$MATCH" | ${pkgs.jq}/bin/jq -r '.owner_scope // ""')
    EPHEMERAL_ROOT=$(echo "$MATCH" | ${pkgs.jq}/bin/jq -r '.ephemeral_root // ""')
    WAIT_REASON=$(echo "$MATCH" | ${pkgs.jq}/bin/jq -r '.wait_reason // ""')
    LOG_PATH=$(echo "$MATCH" | ${pkgs.jq}/bin/jq -r '.log_path // ""')

    SLOT_OWNER=$(${pkgs.jq}/bin/jq -sr \
      --arg slot "$SLOT_FILTER" \
      --arg env "$ENV_FILTER" '
      [ .[]
        | select(.event_type == "slot_acquired" or .event_type == "slot_released")
        | select(($slot == "") or (((.slot // "") | tostring) == $slot))
        | select(($env == "") or ((.env // "") == $env))
      ]
      | sort_by((.seq // 0), (.timestamp // ""))
      | (last | .run_id) // ""
    ' "$EVENTS_FILE")

    REGISTRY_RUNNING="false"
    case "$STATE" in
      starting|running|ready|degraded|waiting|busy)
        REGISTRY_RUNNING="true"
        ;;
    esac

    emit_var "REGISTRY_FOUND" "1"
    emit_var "REGISTRY_RUNNING" "$REGISTRY_RUNNING"
    emit_var "REGISTRY_SCOPE" "global"
    emit_var "REGISTRY_STATE" "$STATE"
    emit_var "OWNER_RUN_ID" "$OWNER_RUN_ID"
    emit_var "OWNER_SCOPE" "$OWNER_SCOPE"
    emit_var "EPHEMERAL_ROOT" "$EPHEMERAL_ROOT"
    emit_var "WAIT_REASON" "$WAIT_REASON"
    emit_var "LOG_PATH" "$LOG_PATH"
    emit_var "SLOT_OWNER" "$SLOT_OWNER"
  '';
in
{
  inherit
    registryRoot
    emitEvent
    processStatus
    processSlots
    processRuns
    processInspect
    processStop
    processGc
    serviceEvents
    serviceLogs
    serviceStatus
    ;
}
