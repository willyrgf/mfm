# Shared ID helpers
{
  pkgs,
  project ? { },
  loggingPrelude ? null,
}:

let
  projectId = (project.project or { }).id or "project";
  counterRoot = "/tmp/nixfied-runtime/${projectId}/id-counters";
  resolvedLoggingPrelude =
    if loggingPrelude != null && loggingPrelude != "" then
      loggingPrelude
    else
      (import ./helpers.nix {
        inherit pkgs project;
        hooks = { };
        summaryParser = "";
      }).loggingPrelude;

  mkPlanId = pkgs.writeShellScript "mk-plan-id" ''
    ${resolvedLoggingPrelude}

    set -euo pipefail

    VALUE=""
    case "''${1:-}" in
      --from-stdin)
        VALUE="$(cat)"
        ;;
      *)
        VALUE="''${1:-}"
        ;;
    esac

    if [ -z "$VALUE" ]; then
      log_error "mk-plan-id requires non-empty input"
      exit 1
    fi

    HASH="$(printf '%s' "$VALUE" | ${pkgs.coreutils}/bin/sha256sum | ${pkgs.gawk}/bin/awk '{print $1}')"
    echo "plan-''${HASH:0:16}"
  '';

  mkUniqueId = pkgs.writeShellScript "mk-unique-id" ''
    set -euo pipefail

    COUNTER_ROOT="${counterRoot}"
    LOCK_FILE="$COUNTER_ROOT/global.lock"
    COUNTER_FILE="$COUNTER_ROOT/global.counter"
    mkdir -p "$COUNTER_ROOT"

    NEXT=$(
      (
        exec 9>"$LOCK_FILE"
        ${pkgs.flock}/bin/flock -x 9
        CURRENT=0
        if [ -f "$COUNTER_FILE" ]; then
          CURRENT="$(cat "$COUNTER_FILE" 2>/dev/null || echo 0)"
        fi
        case "$CURRENT" in
          *[!0-9]*|"") CURRENT=0 ;;
        esac
        CURRENT=$((CURRENT + 1))
        printf '%s\n' "$CURRENT" > "$COUNTER_FILE"
        printf '%s\n' "$CURRENT"
      )
    )

    TS="$(${pkgs.coreutils}/bin/date -u +%Y%m%d-%H%M%S)"
    printf '%s-%06d\n' "$TS" "$NEXT"
  '';

  mkRunId = pkgs.writeShellScript "mk-run-id" ''
    ${resolvedLoggingPrelude}

    set -euo pipefail

    PLAN_ID="''${1:-}"
    if [ -z "$PLAN_ID" ]; then
      log_error "mk-run-id requires plan id input"
      exit 1
    fi

    case "$PLAN_ID" in
      *[!A-Za-z0-9._:-]*)
        log_error "plan id contains invalid characters value=$PLAN_ID"
        log_hint "use only [A-Za-z0-9._:-]"
        exit 1
        ;;
      *)
        ;;
    esac

    COUNTER_ROOT="${counterRoot}"
    SAFE_PLAN_ID="$(printf '%s' "$PLAN_ID" | ${pkgs.gnused}/bin/sed 's/[^A-Za-z0-9._-]/_/g')"
    LOCK_FILE="$COUNTER_ROOT/run-$SAFE_PLAN_ID.lock"
    COUNTER_FILE="$COUNTER_ROOT/run-$SAFE_PLAN_ID.counter"
    mkdir -p "$COUNTER_ROOT"

    NEXT=$(
      (
        exec 9>"$LOCK_FILE"
        ${pkgs.flock}/bin/flock -x 9
        CURRENT=0
        if [ -f "$COUNTER_FILE" ]; then
          CURRENT="$(cat "$COUNTER_FILE" 2>/dev/null || echo 0)"
        fi
        case "$CURRENT" in
          *[!0-9]*|"") CURRENT=0 ;;
        esac
        CURRENT=$((CURRENT + 1))
        printf '%s\n' "$CURRENT" > "$COUNTER_FILE"
        printf '%s\n' "$CURRENT"
      )
    )

    printf '%s-%04d\n' "$PLAN_ID" "$NEXT"
  '';

  resolveId = pkgs.writeShellScript "resolve-id" ''
    ${resolvedLoggingPrelude}

    set -euo pipefail
    VALUE="''${1:-}"
    PLAN_ID="''${2:-''${NIXFIED_PLAN_ID:-}}"

    if [ -n "$VALUE" ]; then
      case "$VALUE" in
        *[!A-Za-z0-9._:-]*)
          log_error "id contains invalid characters value=$VALUE"
          log_hint "use only [A-Za-z0-9._:-]"
          exit 1
          ;;
        *)
          echo "$VALUE"
          exit 0
          ;;
      esac
    fi

    if [ -n "$PLAN_ID" ]; then
      exec ${mkRunId} "$PLAN_ID"
    fi

    exec ${mkUniqueId}
  '';
in
{
  inherit
    mkPlanId
    mkUniqueId
    mkRunId
    resolveId
    ;
}
