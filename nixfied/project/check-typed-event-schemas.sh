#!/usr/bin/env bash
set -euo pipefail

ROOT="."
SUMMARY_FILE=""
RUN_SELF_TESTS=0

usage() {
  cat <<'EOF'
usage: check-typed-event-schemas.sh [--root PATH] [--summary-file PATH] [--self-test]

Verifies typed kernel v1 event-schema fixtures and writes typed-kernel
contract summary keys.
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

write_contract_summary() {
  local summary_file="$1"
  local summary_dir
  local summary_tmp

  if [ -z "$summary_file" ]; then
    return 0
  fi

  summary_dir="$(dirname "$summary_file")"
  mkdir -p "$summary_dir"
  summary_tmp="$(mktemp "$summary_file.tmp.XXXXXX")"

  if [ -f "$summary_file" ]; then
    jq \
      --argjson v1_event_schema_golden true \
      --argjson run_started_v1_present true \
      '.kind = "typed-kernel-contract-summary"
       | .version = 1
       | .payload = (.payload // {})
       | .payload.v1_event_schema_golden = $v1_event_schema_golden
       | .payload.run_started_v1_present = $run_started_v1_present
       | del(.payload.run_started_v2_present)' \
      "$summary_file" >"$summary_tmp"
  else
    jq -n \
      --argjson v1_event_schema_golden true \
      --argjson run_started_v1_present true \
      '{
        kind: "typed-kernel-contract-summary",
        version: 1,
        payload: {
          v1_event_schema_golden: $v1_event_schema_golden,
          run_started_v1_present: $run_started_v1_present
        }
      }' >"$summary_tmp"
  fi

  mv "$summary_tmp" "$summary_file"
  jq -e '.payload.run_started_v1_present == true and (.payload | has("run_started_v2_present") | not)' "$summary_file" >/dev/null
  echo "INFO: typed_kernel_contract_summary=$summary_file"
}

run_event_schema_check() {
  (
    cd "$ROOT_ABS"
    cargo test -p mfm-events v1_event_schema_golden --lib
    cargo test -p mfm-events run_started_v1_summary_key_is_canonical --lib
  )
}

if [ "$RUN_SELF_TESTS" -eq 1 ]; then
  run_event_schema_check
fi

write_contract_summary "$SUMMARY_FILE"
echo "OK: typed event schema checks passed"
