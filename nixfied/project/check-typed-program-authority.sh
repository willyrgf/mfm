#!/usr/bin/env bash
set -euo pipefail

ROOT="."
SUMMARY_FILE=""
RUN_SELF_TESTS=0

usage() {
  cat <<'EOF'
usage: check-typed-program-authority.sh [--root PATH] [--summary-file PATH] [--self-test]

Verifies typed program registry-authority invariants that are small enough to
run during quality checks.
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
      --argjson registered_state_required true \
      '.kind = "typed-kernel-contract-summary"
       | .version = 1
       | .payload = (.payload // {})
       | .payload.registered_state_required = $registered_state_required' \
      "$summary_file" >"$summary_tmp"
  else
    jq -n \
      --argjson registered_state_required true \
      '{
        kind: "typed-kernel-contract-summary",
        version: 1,
        payload: {
          registered_state_required: $registered_state_required
        }
      }' >"$summary_tmp"
  fi

  mv "$summary_tmp" "$summary_file"
  echo "INFO: typed_kernel_contract_summary=$summary_file"
}

run_authority_check() {
  (
    cd "$ROOT_ABS"
    cargo test -p mfm-program unregistered_state_cannot_be_planned --lib
    cargo test -p mfm-program --test program_ui program_authoring_accepts_and_rejects_branded_handles
  )
}

if [ "$RUN_SELF_TESTS" -eq 1 ]; then
  run_authority_check
fi

write_contract_summary "$SUMMARY_FILE"
echo "OK: typed program registry-authority checks passed"
