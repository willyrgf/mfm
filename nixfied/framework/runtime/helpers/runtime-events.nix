# Runtime lifecycle event helpers built on the shared NDJSON registry.
{
  pkgs,
  project ? { },
  loggingPrelude ? null,
}:

let
  projectMeta = project.project or { };
  projectId = projectMeta.id or "project";
  commonRuntimeShell = import ../common-runtime.nix { inherit pkgs; };
  id = import ./id.nix {
    inherit pkgs project;
  };
  servicePolicy = import ./service-policy.nix { inherit pkgs; };
  registry = import ../registry/events.nix { inherit pkgs; };
  projectIdUpper =
    let
      replaced = pkgs.lib.replaceStrings [ "-" "." ] [ "_" "_" ] projectId;
    in
    pkgs.lib.strings.toUpper replaced;
  slotVar = projectMeta.slotVar or "NIX_ENV";
  envVar = projectMeta.envVar or "PROJECT_ENV";
  processCfg = project.process or { };
  registryRoot =
    if project ? state && project.state ? policy && project.state.policy ? registryRoot then
      project.state.policy.registryRoot
    else if project ? state && project.state ? registry && project.state.registry ? root then
      project.state.registry.root
    else if project ? state && project.state ? registryRoot then
      project.state.registryRoot
    else
      processCfg.registryRoot or "/tmp/nixfied-runtime/${projectId}/registry";
  baseDirExpr =
    if project ? state && project.state ? policy && project.state.policy ? runtimeBase then
      project.state.policy.runtimeBase
    else
      (project.directories.base or "\${XDG_DATA_HOME:-$HOME/.local/share}/${projectId}");
  ciCfg = project.ci or { };
  artifactsCfg = ciCfg.artifacts or { };
  artifactsRootExpr =
    if project ? state && project.state ? policy && project.state.policy ? artifactsRoot then
      project.state.policy.artifactsRoot
    else
      artifactsCfg.dir or "/tmp/ci-artifacts";
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
    ${commonRuntimeShell}

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

    runtime_index_segment() {
      local value="$1"
      if [ -z "$value" ]; then
        printf '%s' "__empty__"
        return 0
      fi
      value="''${value//\//_}"
      value="''${value//$'\n'/_}"
      value="''${value//$'\r'/_}"
      value="''${value//$'\t'/_}"
      printf '%s' "$value"
    }

    runtime_events_index_root() {
      printf '%s/runtime-events' "$REGISTRY_ROOT"
    }

    service_events_root_for() {
      local service_name="$1"
      printf '%s/services/%s' "$(runtime_events_index_root)" "$(runtime_index_segment "$service_name")"
    }

    service_events_index_file_for() {
      local service_name="$1"
      local slot_name="$2"
      local env_name="$3"
      printf '%s/%s/%s/events.tsv' \
        "$(service_events_root_for "$service_name")" \
        "$(runtime_index_segment "$slot_name")" \
        "$(runtime_index_segment "$env_name")"
    }

    service_status_file_for() {
      local service_name="$1"
      local slot_name="$2"
      local env_name="$3"
      printf '%s/%s/%s/status.env' \
        "$(service_events_root_for "$service_name")" \
        "$(runtime_index_segment "$slot_name")" \
        "$(runtime_index_segment "$env_name")"
    }

    slot_status_file_for() {
      local slot_name="$1"
      local env_name="$2"
      printf '%s/slots/%s/%s/status.env' \
        "$(runtime_events_index_root)" \
        "$(runtime_index_segment "$slot_name")" \
        "$(runtime_index_segment "$env_name")"
    }

    write_shell_vars_file() {
      local target_file="$1"
      shift
      local parent_dir
      local tmp

      parent_dir="$(dirname "$target_file")" || return 1
      mkdir -p "$parent_dir" || return 1
      tmp="$(mktemp "$target_file.tmp.XXXXXX")" || return 1
      while [ "$#" -gt 1 ]; do
        printf '%s=%q\n' "$1" "$2" >> "$tmp" || {
          rm -f "$tmp"
          return 1
        }
        shift 2
      done
      if ! mv "$tmp" "$target_file"; then
        rm -f "$tmp"
        return 1
      fi
    }

    append_index_line_locked() {
      local target_file="$1"
      local line="$2"
      local lock_file="$target_file.lock"
      local lock_fd
      local parent_dir

      parent_dir="$(dirname "$target_file")" || return 1
      mkdir -p "$parent_dir" || return 1
      lock_fd="$(registry_lock_acquire "$lock_file" "runtime-events-index:$target_file" "$REGISTRY_DEFAULT_LOCK_TIMEOUT_SECONDS")" || return 1
      if ! printf '%s\n' "$line" >> "$target_file"; then
        registry_lock_release "$lock_fd" "$lock_file"
        return 1
      fi
      registry_lock_release "$lock_fd" "$lock_file"
    }

    write_service_status_index() {
      local service_name="$1"
      local slot_name="$2"
      local env_name="$3"
      local registry_state="$4"
      local owner_run_id="$5"
      local owner_scope="$6"
      local ephemeral_root="$7"
      local wait_reason="$8"
      local log_path="$9"
      local status_file

      status_file="$(service_status_file_for "$service_name" "$slot_name" "$env_name")"
      write_shell_vars_file \
        "$status_file" \
        SERVICE_STATUS_STATE "$registry_state" \
        SERVICE_STATUS_OWNER_RUN_ID "$owner_run_id" \
        SERVICE_STATUS_OWNER_SCOPE "$owner_scope" \
        SERVICE_STATUS_EPHEMERAL_ROOT "$ephemeral_root" \
        SERVICE_STATUS_WAIT_REASON "$wait_reason" \
        SERVICE_STATUS_LOG_PATH "$log_path"
    }

    write_slot_owner_index() {
      local slot_name="$1"
      local env_name="$2"
      local owner_run_id="$3"
      local status_file

      status_file="$(slot_status_file_for "$slot_name" "$env_name")"
      write_shell_vars_file "$status_file" SLOT_STATUS_OWNER_RUN_ID "$owner_run_id"
    }

    load_shell_vars_file() {
      local source_file="$1"

      if [ ! -f "$source_file" ]; then
        return 1
      fi

      . "$source_file"
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

    DETAIL_JSON="$(
      printf '{'
      printf '"kind":'
      json_quote_string "$EVENT_KIND"
      printf ',"eventType":'
      json_quote_string "$EVENT_TYPE"
      printf ',"commandName":%s' "$(json_string_or_null "$EVENT_COMMAND")"
      printf ',"projectId":'
      json_quote_string "$PROJECT_ID"
      printf ',"service":%s' "$(json_string_or_null "$EVENT_SERVICE")"
      printf ',"slot":%s' "$(json_string_or_null "$EVENT_SLOT")"
      printf ',"env":%s' "$(json_string_or_null "$EVENT_ENV")"
      printf ',"profile":%s' "$(json_string_or_null "$EVENT_PROFILE")"
      printf ',"pid":%s' "$(json_number_or_null "$EVENT_PID")"
      printf ',"pgid":%s' "$(json_number_or_null "$EVENT_PGID")"
      printf ',"planId":%s' "$(json_string_or_null "$EVENT_PLAN_ID")"
      printf ',"unitId":%s' "$(json_string_or_null "$EVENT_UNIT_ID")"
      printf ',"attempt":%s' "$(json_number_or_null "$EVENT_ATTEMPT")"
      printf ',"ownerScope":%s' "$(json_string_or_null "$EVENT_OWNER_SCOPE")"
      printf ',"reusePolicy":%s' "$(json_string_or_null "$EVENT_REUSE_POLICY")"
      printf ',"discoveryScope":%s' "$(json_string_or_null "$EVENT_DISCOVERY_SCOPE")"
      printf ',"ephemeralRoot":%s' "$(json_string_or_null "$EVENT_EPHEMERAL_ROOT")"
      printf ',"readiness":{'
      printf '"healthOk":%s' "$(json_bool_or_null "$EVENT_READINESS_HEALTH_NORM")"
      printf ',"readyOk":%s' "$(json_bool_or_null "$EVENT_READINESS_READY_NORM")"
      printf ',"lastError":%s' "$(json_string_or_null "$EVENT_LAST_ERROR")"
      printf '}'
      printf ',"waitReason":%s' "$(json_string_or_null "$EVENT_WAIT_REASON")"
      printf ',"logPath":%s' "$(json_string_or_null "$EVENT_LOG_PATH")"
      printf '}'
    )"

    if ! registry_append_event "$REGISTRY_ROOT" "$EVENT_RUN_ID" "''${NIXFIED_ATTEMPT_ID:-}" "" "" "$EVENT_STATE" "$DETAIL_JSON"; then
      log_error "failed to record runtime event event_type=$EVENT_TYPE run_id=$EVENT_RUN_ID"
      exit 1
    fi

    if [ -n "$EVENT_SERVICE" ] && [ -n "''${REGISTRY_APPEND_LAST_SEQ:-}" ] && [ -n "''${REGISTRY_APPEND_LAST_EVENT_JSON:-}" ]; then
      append_index_line_locked \
        "$(service_events_index_file_for "$EVENT_SERVICE" "$EVENT_SLOT" "$EVENT_ENV")" \
        "''${REGISTRY_APPEND_LAST_SEQ}	''${REGISTRY_APPEND_LAST_EVENT_JSON}" || {
        log_error "failed to update service event index service=$EVENT_SERVICE"
        exit 1
      }
      write_service_status_index \
        "$EVENT_SERVICE" \
        "$EVENT_SLOT" \
        "$EVENT_ENV" \
        "$EVENT_STATE" \
        "$EVENT_RUN_ID" \
        "$EVENT_OWNER_SCOPE" \
        "$EVENT_EPHEMERAL_ROOT" \
        "$EVENT_WAIT_REASON" \
        "$EVENT_LOG_PATH" || {
        log_error "failed to update service status index service=$EVENT_SERVICE"
        exit 1
      }
    fi

    case "$EVENT_KIND:$EVENT_TYPE" in
      slotLifecycle:slot_acquired)
        write_slot_owner_index "$EVENT_SLOT" "$EVENT_ENV" "$EVENT_RUN_ID" || {
          log_error "failed to update slot owner index slot=$EVENT_SLOT env=$EVENT_ENV"
          exit 1
        }
        ;;
      slotLifecycle:slot_released)
        write_slot_owner_index "$EVENT_SLOT" "$EVENT_ENV" "" || {
          log_error "failed to clear slot owner index slot=$EVENT_SLOT env=$EVENT_ENV"
          exit 1
        }
        ;;
    esac

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

    collect_service_event_files() {
      local search_root

      search_root="$(service_events_root_for "$SERVICE")"
      if [ -n "$SLOT_FILTER" ]; then
        search_root="$search_root/$(runtime_index_segment "$SLOT_FILTER")"
      fi
      if [ -n "$ENV_FILTER" ]; then
        search_root="$search_root/$(runtime_index_segment "$ENV_FILTER")"
      fi

      if [ ! -d "$search_root" ]; then
        return 0
      fi

      ${pkgs.findutils}/bin/find "$search_root" -type f -name 'events.tsv' | ${pkgs.coreutils}/bin/sort
    }

    OUT="$(
      while IFS= read -r events_file; do
        [ -f "$events_file" ] || continue
        cat "$events_file"
      done < <(collect_service_event_files) | ${pkgs.coreutils}/bin/sort -t $'\t' -k1,1n | ${pkgs.coreutils}/bin/tail -n "$LIMIT" | ${pkgs.coreutils}/bin/cut -f2-
    )"

    if [ -z "$OUT" ]; then
      log_ok "no events found service=$SERVICE project_id=$PROJECT_ID"
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

    STATUS_FILE="$(service_status_file_for "$SERVICE" "$SLOT_FILTER" "$ENV_FILTER")"
    if ! load_shell_vars_file "$STATUS_FILE"; then
      log_error "no registry events found; cannot resolve log path service=$SERVICE"
      exit 1
    fi
    LOG_PATH="''${SERVICE_STATUS_LOG_PATH:-}"

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

    STATUS_FILE="$(service_status_file_for "$SERVICE" "$SLOT_FILTER" "$ENV_FILTER")"
    SLOT_STATUS_FILE="$(slot_status_file_for "$SLOT_FILTER" "$ENV_FILTER")"

    if ! load_shell_vars_file "$STATUS_FILE"; then
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
    STATE="''${SERVICE_STATUS_STATE:-unknown}"
    OWNER_RUN_ID="''${SERVICE_STATUS_OWNER_RUN_ID:-}"
    OWNER_SCOPE="''${SERVICE_STATUS_OWNER_SCOPE:-}"
    EPHEMERAL_ROOT="''${SERVICE_STATUS_EPHEMERAL_ROOT:-}"
    WAIT_REASON="''${SERVICE_STATUS_WAIT_REASON:-}"
    LOG_PATH="''${SERVICE_STATUS_LOG_PATH:-}"

    SLOT_OWNER=""
    if load_shell_vars_file "$SLOT_STATUS_FILE"; then
      SLOT_OWNER="''${SLOT_STATUS_OWNER_RUN_ID:-}"
    fi

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
