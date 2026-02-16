# Keep teardown diagnostics best-effort so step failures remain the primary CI exit code.
CI_DIAG_EVENTS_LIMIT="${CI_DIAG_EVENTS_LIMIT:-200}"

_ci_truthy() {
  case "${1:-}" in
    1|true|TRUE|yes|YES|on|ON) return 0 ;;
    *) return 1 ;;
  esac
}

ci_debug() {
  if _ci_truthy "${CI_VERBOSE:-0}" || _ci_truthy "${NIXFIED_VERBOSE:-0}" || _ci_truthy "${NIXFIED_DEBUG:-0}"; then
    echo "DEBUG: $*" >&2
    return 0
  fi
  case "${NIXFIED_LOG_LEVEL:-}" in
    debug|DEBUG|trace|TRACE)
      echo "DEBUG: $*" >&2
      ;;
  esac
}

capture_diag() {
  local name="$1"
  local kind="$2"
  shift 2
  local outfile=""
  outfile=$(artifact_path "$name")
  ci_debug "collecting ci diagnostic name=$name kind=$kind outfile=$outfile"
  set +e
  "$@" >"$outfile" 2>&1
  local rc=$?
  set -e

  # Service status diagnostics return rc=1 when service is simply not running.
  # Treat that as expected and avoid warning noise in successful runs.
  if [ "$rc" -ne 0 ] && [ "$kind" = "service-status" ]; then
    if grep -Eq '(^|[[:space:]])running=false([[:space:]]|$)' "$outfile"; then
      ci_debug "diagnostic status indicates service not running name=$name rc=$rc (expected)"
      return 0
    fi
  fi

  if [ "$rc" -ne 0 ]; then
    echo "WARN: diagnostic command failed name=$name rc=$rc" >&2
  fi
}

capture_service_diag() {
  local service="$1"
  local token=""
  local status_hook=""
  local events_hook=""

  token=$(echo "$service" | tr '[:lower:]' '[:upper:]' | tr '.:/-' '_')
  status_hook="SVC_${token}_STATUS"
  events_hook="SVC_${token}_EVENTS"

  if has_hook "$status_hook"; then
    capture_diag "ci-diagnostics-service-${service}-status.log" "service-status" run_hook "$status_hook"
  fi

  if has_hook "$events_hook"; then
    capture_diag "ci-diagnostics-service-${service}-events.log" "service-events" run_hook "$events_hook" --limit "$CI_DIAG_EVENTS_LIMIT"
  fi
}

capture_diag "ci-diagnostics-process-status.log" "process" nix run .#process::status -- --all
capture_diag "ci-diagnostics-process-runs.log" "process" nix run .#process::runs -- --all
capture_diag "ci-diagnostics-process-slots.log" "process" nix run .#process::slots -- --all
capture_diag "ci-diagnostics-process-gc.log" "process" nix run .#process::gc

for service in postgres minio reth helios nginx; do
  capture_service_diag "$service"
done
