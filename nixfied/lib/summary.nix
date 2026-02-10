# Summary parser - compact run summary output with JSON support
{
  pkgs,
  project ? { },
}:

let
  ciCfg = project.ci or { };
  failureSignals = ciCfg.failureSignals or [ ];
  defaultSignals = [ "HTTP_429" "ECONNRESET" "ENOTFOUND" ];
  allSignals = failureSignals ++ defaultSignals;
  signalPattern = pkgs.lib.concatStringsSep "|" allSignals;

  summaryParser = pkgs.writeShellScript "summary-parser" ''
    LOGFILE="$1"
    DURATION="$2"
    EXIT_CODE="$3"

    echo ""
    echo "────────────────────────────────────────────────────────────"
    echo "📊 Summary"
    echo "────────────────────────────────────────────────────────────"

    # Prefer summary.json if available
    SUMMARY_JSON=""
    if [ -n "''${CI_ARTIFACTS_DIR:-}" ] && [ -f "$CI_ARTIFACTS_DIR/summary.json" ]; then
      SUMMARY_JSON="$CI_ARTIFACTS_DIR/summary.json"
    elif [ -n "$LOGFILE" ]; then
      PARENT_DIR=$(dirname "$LOGFILE" 2>/dev/null || true)
      if [ -n "$PARENT_DIR" ] && [ -f "$PARENT_DIR/summary.json" ]; then
        SUMMARY_JSON="$PARENT_DIR/summary.json"
      fi
    fi

    if [ -n "$SUMMARY_JSON" ] && command -v ${pkgs.jq}/bin/jq >/dev/null 2>&1; then
      echo "📄 Source: $SUMMARY_JSON"
      ${pkgs.jq}/bin/jq -r '
        if .steps then
          .steps[] | "  \(if .status == "passed" then "✅" elif .status == "skipped" then "⏭️ " else "❌" end) \(.name) (\(.duration // "?")s)"
        else
          empty
        end
      ' "$SUMMARY_JSON" 2>/dev/null || true
    fi

    if [ -n "$DURATION" ]; then
      if [ "$DURATION" -lt 60 ] 2>/dev/null; then
        echo "⏱️  Total time: ''${DURATION}s"
      else
        MINS=$((DURATION / 60))
        SECS=$((DURATION % 60))
        echo "⏱️  Total time: ''${MINS}m ''${SECS}s"
      fi
    fi

    if [ "$EXIT_CODE" -ne 0 ] 2>/dev/null; then
      echo "❌ Exit code: $EXIT_CODE"

      # Check for known failure signals
      ${pkgs.lib.optionalString (allSignals != []) ''
        if [ -n "$LOGFILE" ] && [ -f "$LOGFILE" ]; then
          SIGNALS=$(grep -oE '${signalPattern}' "$LOGFILE" 2>/dev/null | sort -u || true)
          if [ -n "$SIGNALS" ]; then
            echo ""
            echo "⚠️  Detected failure signals:"
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
      echo "✅ Exit code: 0"
    fi

    # Show artifact pointers
    if [ -n "''${CI_ARTIFACTS_DIR:-}" ] && [ -d "''${CI_ARTIFACTS_DIR}" ]; then
      echo ""
      echo "📁 Artifacts: $CI_ARTIFACTS_DIR"
    fi

    echo "────────────────────────────────────────────────────────────"
  '';
in
{
  inherit summaryParser;
}
