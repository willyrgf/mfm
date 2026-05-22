#!/usr/bin/env bash
set -euo pipefail

ROOT="."
SUMMARY_FILE=""
RUN_SELF_TESTS=0

usage() {
  cat <<'EOF'
usage: check-manual-persisted-impls.sh [--root PATH] [--summary-file PATH] [--self-test]

Rejects manually authored persisted-surface trait implementations outside
framework allowlists. The source scanner tokenizes Rust syntax so aliases,
spacing, multiline impl headers, and macro bodies cannot bypass the check by
changing textual formatting.
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

CHECKER_SOURCE="$ROOT_ABS/nixfied/project/check-manual-persisted-impls.rs"
if [ ! -f "$CHECKER_SOURCE" ]; then
  echo "ERROR: missing parser source path=$CHECKER_SOURCE" >&2
  exit 2
fi

bool_json() {
  if [ "$1" -eq 0 ]; then
    printf 'true'
  else
    printf 'false'
  fi
}

read_report_value() {
  local report_file="$1"
  local key="$2"
  local report_key
  local value

  while IFS='=' read -r report_key value; do
    if [ "$report_key" = "$key" ]; then
      printf '%s' "$value"
      return 0
    fi
  done <"$report_file"

  echo "ERROR: parser report missing key=$key file=$report_file" >&2
  exit 2
}

write_contract_summary() {
  local summary_file="$1"
  local rust_file_count="$2"
  local manual_impl_count="$3"
  local provenance_forgery_count="$4"
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
      --argjson manual_value_config_output_impls_rejected "$(bool_json "$((manual_impl_count + provenance_forgery_count))")" \
      --argjson rust_file_count "$rust_file_count" \
      --argjson manual_impl_count "$manual_impl_count" \
      --argjson provenance_forgery_count "$provenance_forgery_count" \
      '.kind = "typed-kernel-contract-summary"
       | .version = 1
       | .payload.manual_value_config_output_impls_rejected = $manual_value_config_output_impls_rejected
       | .payload.persisted_source_boundary_rust_file_count = $rust_file_count
       | .payload.manual_persisted_trait_impl_violation_count = $manual_impl_count
       | .payload.derive_provenance_forgery_violation_count = $provenance_forgery_count' \
      "$summary_file" >"$summary_tmp"
  else
    jq -n \
      --argjson manual_value_config_output_impls_rejected "$(bool_json "$((manual_impl_count + provenance_forgery_count))")" \
      --argjson rust_file_count "$rust_file_count" \
      --argjson manual_impl_count "$manual_impl_count" \
      --argjson provenance_forgery_count "$provenance_forgery_count" \
      '{
        kind: "typed-kernel-contract-summary",
        version: 1,
        payload: {
          manual_value_config_output_impls_rejected: $manual_value_config_output_impls_rejected,
          persisted_source_boundary_rust_file_count: $rust_file_count,
          manual_persisted_trait_impl_violation_count: $manual_impl_count,
          derive_provenance_forgery_violation_count: $provenance_forgery_count
        }
      }' >"$summary_tmp"
  fi

  mv "$summary_tmp" "$summary_file"
  echo "INFO: typed_kernel_contract_summary=$summary_file"
}

CHECKER_BIN="$(mktemp "${TMPDIR:-/tmp}/mfm-manual-impl-checker.XXXXXX")"
REPORT_FILE="$(mktemp "${TMPDIR:-/tmp}/mfm-manual-impl-report.XXXXXX")"
trap 'rm -f "$CHECKER_BIN" "$REPORT_FILE"' EXIT

rustc --edition=2021 "$CHECKER_SOURCE" -o "$CHECKER_BIN"

CHECKER_ARGS=(
  --root "$ROOT_ABS"
  --report-file "$REPORT_FILE"
)
if [ "$RUN_SELF_TESTS" -eq 1 ]; then
  CHECKER_ARGS+=(--self-test)
fi

set +e
"$CHECKER_BIN" "${CHECKER_ARGS[@]}"
CHECKER_STATUS="$?"
set -e

if [ -s "$REPORT_FILE" ]; then
  rust_file_count="$(read_report_value "$REPORT_FILE" rust_file_count)"
  manual_impl_count="$(read_report_value "$REPORT_FILE" manual_impl_count)"
  provenance_forgery_count="$(read_report_value "$REPORT_FILE" provenance_forgery_count)"
  write_contract_summary "$SUMMARY_FILE" "$rust_file_count" "$manual_impl_count" "$provenance_forgery_count"
fi

exit "$CHECKER_STATUS"
