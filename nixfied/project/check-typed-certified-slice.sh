#!/usr/bin/env bash
set -euo pipefail

ROOT="."
SUMMARY_FILE=""
RUN_SELF_TESTS=0

REQUIRED_TRUE_KEYS=(
  typed_spec_hash_persisted
  run_started_v1_present
  seed_material_persisted
  side_effect_ledger_complete
  side_effect_invocation_started_before_submit
  side_effect_crash_cases_passed
  side_effect_no_duplicate_submit
  side_effect_submission_unknown_recovered
  side_effect_failed_semantics_covered
  side_effect_logical_key_conflicts_rejected
  managed_platform_outputs_committed
  public_output_before_run_completed
  resume_drift_rejected
  retention_projection_complete
)

REQUIRED_POSITIVE_KEYS=(
  cell_events_count
  resume_drift_fixture_count
)

REQUIRED_ZERO_KEYS=(
  replay_live_cap_requests_count
)

usage() {
  cat <<'EOF'
usage: check-typed-certified-slice.sh [--root PATH] [--summary-file PATH] [--self-test]

Runs the typed-certified-slice acceptance workflow and validates the RFC summary schema.
EOF
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --root)
      if [ "$#" -lt 2 ]; then
        echo "ERROR: --root requires a value" >&2
        exit 2
      fi
      ROOT="$2"
      shift 2
      ;;
    --summary-file)
      if [ "$#" -lt 2 ]; then
        echo "ERROR: --summary-file requires a value" >&2
        exit 2
      fi
      SUMMARY_FILE="$2"
      shift 2
      ;;
    --self-test)
      RUN_SELF_TESTS=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "ERROR: unknown argument '$1'" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if ! ROOT_ABS="$(cd "$ROOT" && pwd -P)"; then
  echo "ERROR: unable to resolve repository root path=$ROOT" >&2
  exit 2
fi

if [ -z "$SUMMARY_FILE" ]; then
  echo "ERROR: --summary-file is required" >&2
  exit 2
fi

validate_summary() {
  local summary_file="$1"
  local key
  local skipped_or_xfail_markers

  jq -e '.kind == "typed-certified-slice-summary" and .version == 1 and (.payload | type == "object")' "$summary_file" >/dev/null

  for key in "${REQUIRED_TRUE_KEYS[@]}"; do
    if ! jq -e --arg key "$key" '.payload[$key] == true' "$summary_file" >/dev/null; then
      echo "ERROR: typed-certified-slice key missing or false key=$key summary=$summary_file" >&2
      exit 1
    fi
  done

  for key in "${REQUIRED_POSITIVE_KEYS[@]}"; do
    if ! jq -e --arg key "$key" '(.payload[$key] | type == "number") and .payload[$key] > 0' "$summary_file" >/dev/null; then
      echo "ERROR: typed-certified-slice positive count missing or zero key=$key summary=$summary_file" >&2
      exit 1
    fi
  done

  for key in "${REQUIRED_ZERO_KEYS[@]}"; do
    if ! jq -e --arg key "$key" '(.payload[$key] | type == "number") and .payload[$key] == 0' "$summary_file" >/dev/null; then
      echo "ERROR: typed-certified-slice zero-count invariant failed key=$key summary=$summary_file" >&2
      exit 1
    fi
  done

  jq -e '(.payload.public_output_event_id | type == "string") and (.payload.public_output_event_id | length > 0)' "$summary_file" >/dev/null

  skipped_or_xfail_markers="$(
    jq -r '
      def marker_name:
        tostring
        | test("(^|[_-])(skip|skipped|xfail|expected[-_]?failure)([_-]|$)"; "i");
      def marker_string:
        test("(^|[^[:alnum:]])(skip|skipped|xfail|expected[-_ ]?failure)([^[:alnum:]]|$)|^(skip|skipped|xfail|expected[-_ ]?failure)(:|$)"; "i");

      (
        paths as $p
        | ($p | map(tostring)) as $path
        | select($path | any(marker_name))
        | "\($path | join("."))=<marker-key>"
      ),
      (
        paths(scalars) as $p
        | getpath($p) as $value
        | ($p | map(tostring)) as $path
        | select(($value | type) == "string" and ($value | marker_string))
        | "\($path | join("."))=\($value)"
      )
    ' "$summary_file"
  )"
  if [ -n "$skipped_or_xfail_markers" ]; then
    echo "ERROR: typed-certified-slice summary contains skipped/xfail markers summary=$summary_file" >&2
    printf '%s\n' "$skipped_or_xfail_markers" >&2
    exit 1
  fi
}

(
  cd "$ROOT_ABS"
  if [ "$RUN_SELF_TESTS" -eq 1 ]; then
    cargo test -p mfm-kernel-test-support typed_certified_slice_acceptance_summary_passes_required_contract --lib
  fi
  cargo run -q -p mfm-kernel-test-support --bin typed_certified_slice_summary -- --summary-file "$SUMMARY_FILE"
)

validate_summary "$SUMMARY_FILE"
echo "INFO: typed_certified_slice_summary=$SUMMARY_FILE"
echo "OK: typed-certified-slice summary passed"
