# Summary parser - compact run summary output with JSON support
{
  pkgs,
  project ? { },
  loggingPrelude ? "",
}:

let
  ciCfg = project.ci or { };
  failureSignals = ciCfg.failureSignals or [ ];
  defaultSignals = [
    "HTTP_429"
    "ECONNRESET"
    "ENOTFOUND"
  ];
  allSignals = failureSignals ++ defaultSignals;
  signalPattern = pkgs.lib.concatStringsSep "|" allSignals;

  summaryParser = pkgs.writeShellScript "summary-parser" ''
    ${loggingPrelude}

    LOGFILE="$1"
    DURATION="$2"
    EXIT_CODE="$3"

    echo ""
    echo "------------------------------------------------------------"
    echo "Summary"
    echo "------------------------------------------------------------"

    _is_nonneg_int() {
      case "''${1:-}" in
        ""|*[!0-9]*) return 1 ;;
        *) return 0 ;;
      esac
    }

    _format_duration() {
      local seconds="$1"
      if ! _is_nonneg_int "$seconds"; then
        echo "?"
        return 0
      fi
      if [ "$seconds" -lt 60 ]; then
        echo "''${seconds}s"
      else
        local mins=0
        local secs=0
        mins=$((seconds / 60))
        secs=$((seconds % 60))
        echo "''${mins}m ''${secs}s"
      fi
    }

    # Prefer summary.json if available
    SUMMARY_JSON=""
    SUMMARY_FIELDS_FILE=""
    SUMMARY_STEPS_FILE=""
    TIMING_TOTAL=""
    TIMING_SETUP=""
    TIMING_STEPS=""
    TIMING_TEARDOWN=""
    TIMING_ACCOUNTED=""
    TIMING_UNTRACKED=""
    PAR_MAX_WORKERS=""
    PAR_PEAK_WORKERS=""
    PAR_CANCELED_COUNT=""
    if [ -n "''${CI_ARTIFACTS_DIR:-}" ] && [ -f "$CI_ARTIFACTS_DIR/summary.json" ]; then
      SUMMARY_JSON="$CI_ARTIFACTS_DIR/summary.json"
    elif [ -n "$LOGFILE" ]; then
      PARENT_DIR=$(dirname "$LOGFILE" 2>/dev/null || true)
      if [ -n "$PARENT_DIR" ] && [ -f "$PARENT_DIR/summary.json" ]; then
        SUMMARY_JSON="$PARENT_DIR/summary.json"
      fi
    fi

    if [ -n "$SUMMARY_JSON" ]; then
      SUMMARY_DIR="$(dirname "$SUMMARY_JSON" 2>/dev/null || true)"
      SUMMARY_FIELDS_FILE="$SUMMARY_DIR/summary.fields"
      SUMMARY_STEPS_FILE="$SUMMARY_DIR/summary.steps.tsv"
    fi

    if [ -n "$SUMMARY_JSON" ] && [ -f "$SUMMARY_FIELDS_FILE" ]; then
      echo "Source: $SUMMARY_JSON"
      . "$SUMMARY_FIELDS_FILE"
      if [ -f "$SUMMARY_STEPS_FILE" ]; then
        while IFS=$'\t' read -r step_name step_status step_duration; do
          [ -n "$step_name" ] || continue
          case "$step_status" in
            passed) step_marker="PASS" ;;
            skipped) step_marker="SKIP" ;;
            failed|canceled) step_marker="FAIL" ;;
            *) step_marker="FAIL" ;;
          esac
          echo "  [$step_marker] $step_name (''${step_duration:-?}s)"
        done < "$SUMMARY_STEPS_FILE"
      fi

      TIMING_TOTAL="''${SUMMARY_TOTAL_DURATION:-}"
      TIMING_SETUP="''${SUMMARY_SETUP_DURATION:-}"
      TIMING_STEPS="''${SUMMARY_STEPS_DURATION:-}"
      TIMING_TEARDOWN="''${SUMMARY_TEARDOWN_DURATION:-}"
      TIMING_ACCOUNTED="''${SUMMARY_ACCOUNTED_DURATION:-}"
      TIMING_UNTRACKED="''${SUMMARY_UNTRACKED_DURATION:-}"
      if _is_nonneg_int "$TIMING_TOTAL"; then
        DURATION="$TIMING_TOTAL"
      fi

      PAR_MAX_WORKERS="''${SUMMARY_PARALLEL_MAX_WORKERS:-}"
      PAR_PEAK_WORKERS="''${SUMMARY_PARALLEL_PEAK_WORKERS:-}"
      PAR_CANCELED_COUNT="''${SUMMARY_PARALLEL_CANCELED_COUNT:-}"
    fi

    if _is_nonneg_int "$DURATION"; then
      echo "Total time: $(_format_duration "$DURATION")"
    fi

    if _is_nonneg_int "$TIMING_SETUP" \
      && _is_nonneg_int "$TIMING_STEPS" \
      && _is_nonneg_int "$TIMING_TEARDOWN" \
      && _is_nonneg_int "$TIMING_ACCOUNTED" \
      && _is_nonneg_int "$TIMING_UNTRACKED"; then
      log_info "Time breakdown setup=''${TIMING_SETUP}s steps=''${TIMING_STEPS}s teardown=''${TIMING_TEARDOWN}s accounted=''${TIMING_ACCOUNTED}s untracked=''${TIMING_UNTRACKED}s"
    fi

    if _is_nonneg_int "$PAR_MAX_WORKERS" && _is_nonneg_int "$PAR_PEAK_WORKERS" && _is_nonneg_int "$PAR_CANCELED_COUNT"; then
      log_info "Parallelism max_workers=$PAR_MAX_WORKERS peak_workers=$PAR_PEAK_WORKERS canceled_count=$PAR_CANCELED_COUNT"
    fi

    SKIPPED_COUNT=""
    if [ -n "$SUMMARY_FIELDS_FILE" ] && [ -f "$SUMMARY_FIELDS_FILE" ]; then
      SKIPPED_COUNT="''${SUMMARY_SKIPPED_COUNT:-}"
    fi
    if _is_nonneg_int "$SKIPPED_COUNT" && [ "$SKIPPED_COUNT" -gt 0 ]; then
      log_info "SKIP: $SKIPPED_COUNT task(s) skipped"
    fi

    if [ "$EXIT_CODE" -ne 0 ] 2>/dev/null; then
      log_error "Exit code: $EXIT_CODE" 2>&1

      # Check for known failure signals
      ${pkgs.lib.optionalString (allSignals != [ ]) ''
        if [ -n "$LOGFILE" ] && [ -f "$LOGFILE" ]; then
          SIGNALS=$(grep -oE '${signalPattern}' "$LOGFILE" 2>/dev/null | sort -u || true)
          if [ -n "$SIGNALS" ]; then
            echo ""
            log_warn "Detected failure signals:" 2>&1
            echo "$SIGNALS" | while read -r sig; do echo "   - $sig"; done
          fi
        fi
      ''}

      if [ -n "$LOGFILE" ] && [ -f "$LOGFILE" ]; then
        echo ""
        echo "Last 50 lines:"
        tail -50 "$LOGFILE" || true
      fi
    else
      log_ok "Exit code: 0"
    fi

    # Show artifact pointers
    if [ -n "''${CI_ARTIFACTS_DIR:-}" ] && [ -d "''${CI_ARTIFACTS_DIR}" ]; then
      echo ""
      echo "Artifacts: $CI_ARTIFACTS_DIR"
    fi

    echo "------------------------------------------------------------"
  '';
in
{
  inherit summaryParser;
}
