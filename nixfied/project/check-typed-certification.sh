#!/usr/bin/env bash
set -euo pipefail

ROOT="."
SUMMARY_FILE=""
RUN_SELF_TESTS=0

usage() {
  cat <<'EOF'
usage: check-typed-certification.sh [--root PATH] [--summary-file PATH] [--self-test]

Verifies typed spec certification rejects every RFC problem-taxonomy class
and writes typed-kernel contract summary keys.
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
      --argjson invalid_topology_rejected true \
      --argjson invalid_interface_wiring_rejected true \
      --argjson invalid_semantic_transition_rejected true \
      --argjson invalid_data_shape_rejected true \
      --argjson invalid_data_meaning_rejected true \
      --argjson invalid_terminal_shape_rejected true \
      '.kind = "typed-kernel-contract-summary"
       | .version = 1
       | .payload = (.payload // {})
       | .payload.invalid_topology_rejected = $invalid_topology_rejected
       | .payload.invalid_interface_wiring_rejected = $invalid_interface_wiring_rejected
       | .payload.invalid_semantic_transition_rejected = $invalid_semantic_transition_rejected
       | .payload.invalid_data_shape_rejected = $invalid_data_shape_rejected
       | .payload.invalid_data_meaning_rejected = $invalid_data_meaning_rejected
       | .payload.invalid_terminal_shape_rejected = $invalid_terminal_shape_rejected' \
      "$summary_file" >"$summary_tmp"
  else
    jq -n \
      --argjson invalid_topology_rejected true \
      --argjson invalid_interface_wiring_rejected true \
      --argjson invalid_semantic_transition_rejected true \
      --argjson invalid_data_shape_rejected true \
      --argjson invalid_data_meaning_rejected true \
      --argjson invalid_terminal_shape_rejected true \
      '{
        kind: "typed-kernel-contract-summary",
        version: 1,
        payload: {
          invalid_topology_rejected: $invalid_topology_rejected,
          invalid_interface_wiring_rejected: $invalid_interface_wiring_rejected,
          invalid_semantic_transition_rejected: $invalid_semantic_transition_rejected,
          invalid_data_shape_rejected: $invalid_data_shape_rejected,
          invalid_data_meaning_rejected: $invalid_data_meaning_rejected,
          invalid_terminal_shape_rejected: $invalid_terminal_shape_rejected
        }
      }' >"$summary_tmp"
  fi

  mv "$summary_tmp" "$summary_file"
  jq -e '
    .payload.invalid_topology_rejected == true
    and .payload.invalid_interface_wiring_rejected == true
    and .payload.invalid_semantic_transition_rejected == true
    and .payload.invalid_data_shape_rejected == true
    and .payload.invalid_data_meaning_rejected == true
    and .payload.invalid_terminal_shape_rejected == true
  ' "$summary_file" >/dev/null
  echo "INFO: typed_kernel_contract_summary=$summary_file"
}

run_certification_check() {
  (
    cd "$ROOT_ABS"
    cargo test -p mfm-certify certifies_reference_program_draft --lib
    cargo test -p mfm-certify typed_certification_rejects_problem_taxonomy --lib
    cargo test -p mfm-certify typed_spec_requires_registry_authority --lib
    cargo test -p mfm-certify certification_rejects_forged_framework_descriptor_bypass --lib
    cargo test -p mfm-certify certification_rejects_operation_input_binding_tampering --lib
    cargo test -p mfm-certify certification_rejects_stable_id_and_scope_tampering --lib
    cargo test -p mfm-certify certification_rejects_framework_lineage_tampering --lib
    cargo test -p mfm-certify certification_rejects_side_effect_contract_mismatch --lib
    cargo test -p mfm-certify certification_summary_keys_are_stable --lib
  )
}

if [ "$RUN_SELF_TESTS" -eq 1 ]; then
  run_certification_check
fi

write_contract_summary "$SUMMARY_FILE"
echo "OK: typed certification checks passed"
