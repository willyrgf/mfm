# Summary parser - compact run summary output with JSON support
{
  pkgs,
  project ? { },
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
    TIMING_TOTAL=""
    TIMING_SETUP=""
    TIMING_STEPS=""
    TIMING_TEARDOWN=""
    TIMING_ACCOUNTED=""
    TIMING_UNTRACKED=""
    if [ -n "''${CI_ARTIFACTS_DIR:-}" ] && [ -f "$CI_ARTIFACTS_DIR/summary.json" ]; then
      SUMMARY_JSON="$CI_ARTIFACTS_DIR/summary.json"
    elif [ -n "$LOGFILE" ]; then
      PARENT_DIR=$(dirname "$LOGFILE" 2>/dev/null || true)
      if [ -n "$PARENT_DIR" ] && [ -f "$PARENT_DIR/summary.json" ]; then
        SUMMARY_JSON="$PARENT_DIR/summary.json"
      fi
    fi

    if [ -n "$SUMMARY_JSON" ] && command -v ${pkgs.jq}/bin/jq >/dev/null 2>&1; then
      echo "Source: $SUMMARY_JSON"
      ${pkgs.jq}/bin/jq -r '
        if .steps then
          .steps[] | "  [\(if .status == "passed" then "PASS" elif .status == "skipped" then "SKIP" else "FAIL" end)] \(.name) (\(.duration // "?")s)"
        else
          empty
        end
      ' "$SUMMARY_JSON" 2>/dev/null || true

      TIMING_FIELDS=$(${pkgs.jq}/bin/jq -r '
        if (.timing and (.timing | type == "object")) then
          [
            (.timing.total_duration // ""),
            (.timing.setup_duration // ""),
            (.timing.steps_duration // ""),
            (.timing.teardown_duration // ""),
            (.timing.accounted_duration // ""),
            (.timing.untracked_duration // "")
          ] | @tsv
        else
          ""
        end
      ' "$SUMMARY_JSON" 2>/dev/null || true)
      if [ -n "$TIMING_FIELDS" ]; then
        IFS=$'\t' read -r TIMING_TOTAL TIMING_SETUP TIMING_STEPS TIMING_TEARDOWN TIMING_ACCOUNTED TIMING_UNTRACKED <<< "$TIMING_FIELDS"
        if _is_nonneg_int "$TIMING_TOTAL"; then
          DURATION="$TIMING_TOTAL"
        fi
      fi
    fi

    if _is_nonneg_int "$DURATION"; then
      echo "Total time: $(_format_duration "$DURATION")"
    fi

    if _is_nonneg_int "$TIMING_SETUP" \
      && _is_nonneg_int "$TIMING_STEPS" \
      && _is_nonneg_int "$TIMING_TEARDOWN" \
      && _is_nonneg_int "$TIMING_ACCOUNTED" \
      && _is_nonneg_int "$TIMING_UNTRACKED"; then
      echo "INFO: Time breakdown setup=''${TIMING_SETUP}s steps=''${TIMING_STEPS}s teardown=''${TIMING_TEARDOWN}s accounted=''${TIMING_ACCOUNTED}s untracked=''${TIMING_UNTRACKED}s"
    fi

    if [ "$EXIT_CODE" -ne 0 ] 2>/dev/null; then
      echo "ERROR: Exit code: $EXIT_CODE"

      # Check for known failure signals
      ${pkgs.lib.optionalString (allSignals != [ ]) ''
        if [ -n "$LOGFILE" ] && [ -f "$LOGFILE" ]; then
          SIGNALS=$(grep -oE '${signalPattern}' "$LOGFILE" 2>/dev/null | sort -u || true)
          if [ -n "$SIGNALS" ]; then
            echo ""
            echo "WARN: Detected failure signals:"
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
      echo "OK: Exit code: 0"
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
