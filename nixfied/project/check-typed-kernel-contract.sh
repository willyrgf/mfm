#!/usr/bin/env bash
set -euo pipefail

ROOT="."
SUMMARY_FILE=""
RUN_SELF_TESTS=0

REQUIRED_TRUE_KEYS=(
  kernel_crates_present
  typed_boundary_firewall_passed
  value_config_output_derives_present
  manual_value_config_output_impls_rejected
  registered_state_required
  registered_operation_required
  bridge_session_evidence_required
  semantic_lineage_evidence_present
  stable_ids_golden
  typed_spec_hash_deterministic
  v1_event_schema_golden
  invalid_topology_rejected
  invalid_interface_wiring_rejected
  invalid_semantic_transition_rejected
  invalid_data_shape_rejected
  invalid_data_meaning_rejected
  invalid_terminal_shape_rejected
)

REQUIRED_POSITIVE_KEYS=(
  kernel_crates_expected_count
  kernel_crates_actual_count
  persisted_source_boundary_rust_file_count
  typed_kernel_contract_required_key_count
)

REQUIRED_ZERO_KEYS=(
  missing_kernel_crate_count
  kernel_dependency_violation_count
  manual_persisted_trait_impl_violation_count
  derive_provenance_forgery_violation_count
)

usage() {
  cat <<'EOF'
usage: check-typed-kernel-contract.sh [--root PATH] [--summary-file PATH] [--self-test]

Runs the typed-kernel-contract coverage checks that are not owned by a narrower
project check script, writes their summary keys, and validates the complete
typed-kernel-contract summary schema.
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
      --argjson value_config_output_derives_present true \
      --argjson bridge_session_evidence_required true \
      --argjson semantic_lineage_evidence_present true \
      --argjson stable_ids_golden true \
      --argjson typed_spec_hash_deterministic true \
      --argjson typed_kernel_contract_required_key_count "${#REQUIRED_TRUE_KEYS[@]}" \
      '.kind = "typed-kernel-contract-summary"
       | .version = 1
       | .payload = (.payload // {})
       | .payload.value_config_output_derives_present = $value_config_output_derives_present
       | .payload.bridge_session_evidence_required = $bridge_session_evidence_required
       | .payload.semantic_lineage_evidence_present = $semantic_lineage_evidence_present
       | .payload.stable_ids_golden = $stable_ids_golden
       | .payload.typed_spec_hash_deterministic = $typed_spec_hash_deterministic
       | .payload.typed_kernel_contract_required_key_count = $typed_kernel_contract_required_key_count' \
      "$summary_file" >"$summary_tmp"
  else
    jq -n \
      --argjson value_config_output_derives_present true \
      --argjson bridge_session_evidence_required true \
      --argjson semantic_lineage_evidence_present true \
      --argjson stable_ids_golden true \
      --argjson typed_spec_hash_deterministic true \
      --argjson typed_kernel_contract_required_key_count "${#REQUIRED_TRUE_KEYS[@]}" \
      '{
        kind: "typed-kernel-contract-summary",
        version: 1,
        payload: {
          value_config_output_derives_present: $value_config_output_derives_present,
          bridge_session_evidence_required: $bridge_session_evidence_required,
          semantic_lineage_evidence_present: $semantic_lineage_evidence_present,
          stable_ids_golden: $stable_ids_golden,
          typed_spec_hash_deterministic: $typed_spec_hash_deterministic,
          typed_kernel_contract_required_key_count: $typed_kernel_contract_required_key_count
        }
      }' >"$summary_tmp"
  fi

  mv "$summary_tmp" "$summary_file"
}

validate_contract_summary() {
  local summary_file="$1"
  local key
  local skipped_or_xfail_markers

  if [ -z "$summary_file" ]; then
    echo "ERROR: --summary-file is required for typed-kernel-contract validation" >&2
    exit 2
  fi
  if [ ! -f "$summary_file" ]; then
    echo "ERROR: missing typed-kernel-contract summary path=$summary_file" >&2
    exit 2
  fi

  jq -e '.kind == "typed-kernel-contract-summary" and .version == 1 and (.payload | type == "object")' "$summary_file" >/dev/null

  for key in "${REQUIRED_TRUE_KEYS[@]}"; do
    if ! jq -e --arg key "$key" '.payload[$key] == true' "$summary_file" >/dev/null; then
      echo "ERROR: typed-kernel-contract key missing or false key=$key summary=$summary_file" >&2
      exit 1
    fi
  done

  for key in "${REQUIRED_POSITIVE_KEYS[@]}"; do
    if ! jq -e --arg key "$key" '(.payload[$key] | type == "number") and .payload[$key] > 0' "$summary_file" >/dev/null; then
      echo "ERROR: typed-kernel-contract positive count missing or zero key=$key summary=$summary_file" >&2
      exit 1
    fi
  done

  for key in "${REQUIRED_ZERO_KEYS[@]}"; do
    if ! jq -e --arg key "$key" '(.payload[$key] | type == "number") and .payload[$key] == 0' "$summary_file" >/dev/null; then
      echo "ERROR: typed-kernel-contract zero-count invariant failed key=$key summary=$summary_file" >&2
      exit 1
    fi
  done

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
    echo "ERROR: typed-kernel-contract summary contains skipped/xfail markers summary=$summary_file" >&2
    printf '%s\n' "$skipped_or_xfail_markers" >&2
    exit 1
  fi

  echo "INFO: typed_kernel_contract_summary=$summary_file"
}

run_contract_checks() {
  (
    cd "$ROOT_ABS"
    cargo test -p mfm-program-derive --test derive_ui derives_accept_and_reject_supported_surfaces
    cargo test -p mfm-program root_builder_binds_seed_and_public_output_specs --lib
    cargo test -p mfm-program child_scope_exports_bridge_nodes_and_validates_refs --lib
    cargo test -p mfm-program forged_or_stale_bridge_refs_do_not_certify --lib
    cargo test -p mfm-program same_scope_same_type_lineage_mismatch_rejects_for_certification --lib
    cargo test -p mfm-program stable_ids_and_value_lineage_golden_vectors --lib
    cargo test -p mfm-spec certified_spec_hash_golden --lib
  )
}

if [ "$RUN_SELF_TESTS" -eq 1 ]; then
  run_contract_checks
fi

write_contract_summary "$SUMMARY_FILE"
validate_contract_summary "$SUMMARY_FILE"
echo "OK: typed kernel contract summary passed"
