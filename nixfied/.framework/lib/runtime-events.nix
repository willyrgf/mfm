# Runtime lifecycle event helpers built on the shared NDJSON registry.
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
  registry = import ../../registry/events.nix { inherit pkgs; };
  projectIdUpper =
    let
      replaced = pkgs.lib.replaceStrings [ "-" "." ] [ "_" "_" ] projectId;
    in
    pkgs.lib.strings.toUpper replaced;
  slotVar = projectMeta.slotVar or "NIX_ENV";
  envVar = projectMeta.envVar or "PROJECT_ENV";
  processCfg = project.process or { };
  registryRoot =
    if project ? state && project.state ? registry && project.state.registry ? root then
      project.state.registry.root
    else if project ? state && project.state ? registryRoot then
      project.state.registryRoot
    else
      processCfg.registryRoot or "/tmp/nixfied-runtime/${projectId}/registry";
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
  registryShell = registry.mkShellLib { };

  sharedPrelude = ''
    ${resolvedLoggingPrelude}

    set -euo pipefail

    REGISTRY_ROOT_DEFAULT="${registryRoot}"
    REGISTRY_ROOT="''${REGISTRY_ROOT:-$REGISTRY_ROOT_DEFAULT}"
    PROJECT_ID="${projectId}"
    BASE_DIR_DEFAULT="${baseDirExpr}"
    CI_ARTIFACTS_BASE_DEFAULT="${artifactsRootExpr}"
    EPHEMERAL_PREFIX="${ephemeralPrefix}"
    SLOT_VAR="${slotVar}"
    ENV_VAR="${envVar}"
    EPHEMERAL_FLAG_VAR="${projectIdUpper}_EPHEMERAL"
    EPHEMERAL_ROOT_VAR="${projectIdUpper}_EPHEMERAL_ROOT"

    ${registryShell}

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
  '';

  emitEvent = pkgs.writeShellScript "runtime-events-emit-event" ''
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
      echo "Usage: runtime-events-emit-event --event-type <type> [--service <name>] [--state <state>] [--run-id <id>] [--slot <slot>] [--env <env>] [--wait-reason <reason>] [--log-path <path>]" >&2
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

    EVENT_READINESS_HEALTH_NORM="$(normalize_bool "$EVENT_READINESS_HEALTH")"
    EVENT_READINESS_READY_NORM="$(normalize_bool "$EVENT_READINESS_READY")"
    EVENT_KIND="slotLifecycle"
    if [ -n "$EVENT_SERVICE" ]; then
      EVENT_KIND="serviceLifecycle"
    fi

    DETAIL_JSON=$(${pkgs.jq}/bin/jq -cnS \
      --arg kind "$EVENT_KIND" \
      --arg eventType "$EVENT_TYPE" \
      --arg commandName "$EVENT_COMMAND" \
      --arg projectId "$PROJECT_ID" \
      --arg service "$EVENT_SERVICE" \
      --arg slot "$EVENT_SLOT" \
      --arg env "$EVENT_ENV" \
      --arg profile "$EVENT_PROFILE" \
      --arg pid "$EVENT_PID" \
      --arg pgid "$EVENT_PGID" \
      --arg planId "$EVENT_PLAN_ID" \
      --arg unitId "$EVENT_UNIT_ID" \
      --arg attempt "$EVENT_ATTEMPT" \
      --arg ownerScope "$EVENT_OWNER_SCOPE" \
      --arg reusePolicy "$EVENT_REUSE_POLICY" \
      --arg discoveryScope "$EVENT_DISCOVERY_SCOPE" \
      --arg ephemeralRoot "$EVENT_EPHEMERAL_ROOT" \
      --arg waitReason "$EVENT_WAIT_REASON" \
      --arg logPath "$EVENT_LOG_PATH" \
      --arg lastError "$EVENT_LAST_ERROR" \
      --arg readinessHealth "$EVENT_READINESS_HEALTH_NORM" \
      --arg readinessReady "$EVENT_READINESS_READY_NORM" \
      '
      {
        kind: $kind,
        eventType: $eventType,
        commandName: (if $commandName == "" then null else $commandName end),
        projectId: $projectId,
        service: (if $service == "" then null else $service end),
        slot: (if $slot == "" then null else $slot end),
        env: (if $env == "" then null else $env end),
        profile: (if $profile == "" then null else $profile end),
        pid: (if $pid == "" then null else (try ($pid | tonumber) catch $pid) end),
        pgid: (if $pgid == "" then null else (try ($pgid | tonumber) catch $pgid) end),
        planId: (if $planId == "" then null else $planId end),
        unitId: (if $unitId == "" then null else $unitId end),
        attempt: (if $attempt == "" then null else (try ($attempt | tonumber) catch null) end),
        ownerScope: (if $ownerScope == "" then null else $ownerScope end),
        reusePolicy: (if $reusePolicy == "" then null else $reusePolicy end),
        discoveryScope: (if $discoveryScope == "" then null else $discoveryScope end),
        ephemeralRoot: (if $ephemeralRoot == "" then null else $ephemeralRoot end),
        readiness: {
          healthOk: (if $readinessHealth == "null" then null else ($readinessHealth == "true") end),
          readyOk: (if $readinessReady == "null" then null else ($readinessReady == "true") end),
          lastError: (if $lastError == "" then null else $lastError end)
        },
        waitReason: (if $waitReason == "" then null else $waitReason end),
        logPath: (if $logPath == "" then null else $logPath end)
      }
      ')

    if ! registry_append_event "$REGISTRY_ROOT" "$EVENT_RUN_ID" "" "" "$EVENT_STATE" "$DETAIL_JSON"; then
      log_error "failed to record runtime event event_type=$EVENT_TYPE run_id=$EVENT_RUN_ID"
      exit 1
    fi

    log_ok "runtime event recorded event_type=$EVENT_TYPE run_id=$EVENT_RUN_ID service=''${EVENT_SERVICE:-none} state=$EVENT_STATE"
  '';

  serviceEvents = pkgs.writeShellScript "service-events" ''
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
      echo "Usage: service-events --service <name> [--slot <slot>] [--env <env>] [--limit <n>]" >&2
      exit 1
    fi

    case "$LIMIT" in
      *[!0-9]*|"")
        log_error "--limit must be a positive integer (got '$LIMIT')"
        exit 1
        ;;
      *)
        ;;
    esac

    EVENTS_FILE="$(registry_events_snapshot "$REGISTRY_ROOT" 2>/dev/null || true)"
    if [ -z "$EVENTS_FILE" ] || [ ! -f "$EVENTS_FILE" ]; then
      log_ok "no events found service=$SERVICE project_id=$PROJECT_ID"
      exit 0
    fi
    trap 'registry_snapshot_cleanup "''${EVENTS_FILE:-}"' EXIT INT TERM

    OUT=$(${pkgs.jq}/bin/jq -c -s \
      --arg service "$SERVICE" \
      --arg slot "$SLOT_FILTER" \
      --arg env "$ENV_FILTER" \
      --arg limit "$LIMIT" '
      [ .[]
        | select((.detail.kind // "") == "serviceLifecycle")
        | select((.detail.service // "") == $service)
        | select(($slot == "") or (((.detail.slot // "") | tostring) == $slot))
        | select(($env == "") or ((.detail.env // "") == $env))
      ]
      | sort_by((.seq // 0), (.ts // ""))
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

  serviceLogs = pkgs.writeShellScript "service-logs" ''
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
      echo "Usage: service-logs --service <name> [--slot <slot>] [--env <env>] [--lines <n>] [--follow]" >&2
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

    EVENTS_FILE="$(registry_events_snapshot "$REGISTRY_ROOT" 2>/dev/null || true)"
    if [ -z "$EVENTS_FILE" ] || [ ! -f "$EVENTS_FILE" ]; then
      log_error "no registry events found; cannot resolve log path service=$SERVICE"
      exit 1
    fi
    trap 'registry_snapshot_cleanup "''${EVENTS_FILE:-}"' EXIT INT TERM

    LOG_PATH=$(${pkgs.jq}/bin/jq -r -s \
      --arg service "$SERVICE" \
      --arg slot "$SLOT_FILTER" \
      --arg env "$ENV_FILTER" '
      [ .[]
        | select((.detail.kind // "") == "serviceLifecycle")
        | select((.detail.service // "") == $service)
        | select(($slot == "") or (((.detail.slot // "") | tostring) == $slot))
        | select(($env == "") or ((.detail.env // "") == $env))
        | select((.detail.logPath // "") != "")
      ]
      | sort_by((.seq // 0), (.ts // ""))
      | (last | .detail.logPath) // ""
    ' "$EVENTS_FILE")

    if [ -z "$LOG_PATH" ]; then
      log_error "no log path recorded for service=$SERVICE slot=''${SLOT_FILTER:-any} env=''${ENV_FILTER:-any}"
      log_hint "start the service once so it emits lifecycle events with logPath."
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

    if [ ! -f "$LOG_PATH_CANON" ]; then
      log_error "log file not found path=$LOG_PATH_CANON service=$SERVICE"
      exit 1
    fi

    if [ "$FOLLOW" = "true" ]; then
      exec ${pkgs.coreutils}/bin/tail -n "$LINES" -f "$LOG_PATH_CANON"
    fi
    exec ${pkgs.coreutils}/bin/tail -n "$LINES" "$LOG_PATH_CANON"
  '';

  serviceStatus = pkgs.writeShellScript "service-status" ''
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
      echo "Usage: service-status --service <name> [--slot <slot>] [--env <env>]" >&2
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

    EVENTS_FILE="$(registry_events_snapshot "$REGISTRY_ROOT" 2>/dev/null || true)"
    if [ -z "$EVENTS_FILE" ] || [ ! -f "$EVENTS_FILE" ]; then
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
    trap 'registry_snapshot_cleanup "''${EVENTS_FILE:-}"' EXIT INT TERM

    MATCH=$(${pkgs.jq}/bin/jq -c -s \
      --arg service "$SERVICE" \
      --arg slot "$SLOT_FILTER" \
      --arg env "$ENV_FILTER" '
      [ .[]
        | select((.detail.kind // "") == "serviceLifecycle")
        | select((.detail.service // "") == $service)
        | select(($slot == "") or (((.detail.slot // "") | tostring) == $slot))
        | select(($env == "") or ((.detail.env // "") == $env))
      ]
      | sort_by((.seq // 0), (.ts // ""))
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
    OWNER_RUN_ID=$(echo "$MATCH" | ${pkgs.jq}/bin/jq -r '.runId // ""')
    OWNER_SCOPE=$(echo "$MATCH" | ${pkgs.jq}/bin/jq -r '.detail.ownerScope // ""')
    EPHEMERAL_ROOT=$(echo "$MATCH" | ${pkgs.jq}/bin/jq -r '.detail.ephemeralRoot // ""')
    WAIT_REASON=$(echo "$MATCH" | ${pkgs.jq}/bin/jq -r '.detail.waitReason // ""')
    LOG_PATH=$(echo "$MATCH" | ${pkgs.jq}/bin/jq -r '.detail.logPath // ""')

    SLOT_OWNER=$(${pkgs.jq}/bin/jq -r -s \
      --arg slot "$SLOT_FILTER" \
      --arg env "$ENV_FILTER" '
      [ .[]
        | select((.detail.kind // "") == "slotLifecycle")
        | select((.detail.eventType // "") == "slot_acquired" or (.detail.eventType // "") == "slot_released")
        | select(($slot == "") or (((.detail.slot // "") | tostring) == $slot))
        | select(($env == "") or ((.detail.env // "") == $env))
      ]
      | sort_by((.seq // 0), (.ts // ""))
      | (last | .runId) // ""
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
    serviceEvents
    serviceLogs
    serviceStatus
    ;
}
