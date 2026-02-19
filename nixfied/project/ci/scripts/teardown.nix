{ pkgs }:
pkgs.writeText "mfm-ci-teardown.sh" ''
  # Keep teardown diagnostics best-effort so step failures remain the primary CI exit code.
  CI_DIAG_EVENTS_LIMIT="''${CI_DIAG_EVENTS_LIMIT:-200}"
  CI_DIAG_LOG_LINES="''${CI_DIAG_LOG_LINES:-200}"
  CI_PROCESS_STOP_TIMEOUT_SECS="''${CI_PROCESS_STOP_TIMEOUT_SECS:-20}"

  _ci_output_mode() {
    local mode_lower=""
    mode_lower="$(printf '%s' "''${OUTPUT_MODE:-stdout}" | tr '[:upper:]' '[:lower:]')"
    case "$mode_lower" in
      stdout|logs|both)
        printf '%s\n' "$mode_lower"
        ;;
      *)
        # Setup validates OUTPUT_MODE, but teardown remains tolerant.
        printf '%s\n' "stdout"
        ;;
    esac
  }

  _ci_debug_enabled() {
    local level_lower=""
    level_lower="$(printf '%s' "''${LOG_LEVEL:-}" | tr '[:upper:]' '[:lower:]')"
    case "$level_lower" in
      *debug*|*trace*) return 0 ;;
      *) return 1 ;;
    esac
  }

  ci_debug() {
    if _ci_debug_enabled; then
      echo "DEBUG: $*" >&2
      return 0
    fi
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
    local logs_hook=""

    token=$(echo "$service" | tr '[:lower:]' '[:upper:]' | tr '.:/-' '_')
    status_hook="SVC_''${token}_STATUS"
    events_hook="SVC_''${token}_EVENTS"
    logs_hook="SVC_''${token}_LOG"

    if has_hook "$status_hook"; then
      capture_diag "ci-diagnostics-service-''${service}-status.log" "service-status" run_hook "$status_hook"
    fi

    if has_hook "$events_hook"; then
      capture_diag "ci-diagnostics-service-''${service}-events.log" "service-events" run_hook "$events_hook" --limit "$CI_DIAG_EVENTS_LIMIT"
    fi

    # OUTPUT_MODE routes process logs (stdout/log-file/both). Service log hooks
    # are file-backed diagnostics and stay enabled for debug/trace regardless.
    if _ci_debug_enabled && has_hook "$logs_hook"; then
      ci_debug "capturing service log tail service=$service output_mode=$(_ci_output_mode)"
      capture_diag "ci-diagnostics-service-''${service}-log.log" "service-log" run_hook "$logs_hook" -- --lines "$CI_DIAG_LOG_LINES"
    fi
  }

  capture_diag "ci-diagnostics-process-status.log" "process" nix run .#process::status -- --all
  capture_diag "ci-diagnostics-process-runs.log" "process" nix run .#process::runs -- --all
  capture_diag "ci-diagnostics-process-slots.log" "process" nix run .#process::slots -- --all
  if [ -z "''${SCCACHE_DIR:-}" ] && [ -n "''${MFM_EPHEMERAL_ROOT:-}" ]; then
    export SCCACHE_DIR="$MFM_EPHEMERAL_ROOT/build/sccache"
  fi
  if command -v sccache >/dev/null 2>&1; then
    sccache --show-stats >&2 || true
    capture_diag "ci-diagnostics-sccache-stats.log" "tool" sccache --show-stats
  fi
  # CI parity modes can intentionally keep fixture services running across steps
  # (same-slot reuse). Stop the run-scoped processes before ephemeral root cleanup.
  if [ -n "''${RUN_ID:-}" ]; then
    capture_diag "ci-diagnostics-process-stop.log" "process" \
      nix run .#process::stop -- --run-id "$RUN_ID" --scope slot-env --timeout "$CI_PROCESS_STOP_TIMEOUT_SECS" --force
  else
    echo "WARN: RUN_ID is unset; skipping process::stop run-scoped cleanup" >&2
  fi
  capture_diag "ci-diagnostics-process-gc.log" "process" nix run .#process::gc -- --apply

  for service in postgres minio reth helios nginx; do
    capture_service_diag "$service"
  done
''
