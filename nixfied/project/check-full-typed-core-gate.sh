#!/usr/bin/env bash
set -euo pipefail

ROOT="."
ARTIFACTS_DIR="${CI_ARTIFACTS_DIR:-}"
REGISTRY_ROOT_VALUE="${REGISTRY_ROOT:-}"
RUN_ID="${NIXFIED_RUN_ID:-}"
ATTEMPT_ID="${NIXFIED_ATTEMPT_ID:-}"
SUMMARY_FILE=""
RUN_SELF_TESTS=0

KERNEL_TRUE_KEYS=(
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

KERNEL_POSITIVE_KEYS=(
  kernel_crates_expected_count
  kernel_crates_actual_count
  typed_boundary_scanned_file_count
  persisted_source_boundary_rust_file_count
  typed_kernel_contract_required_key_count
)

KERNEL_ZERO_KEYS=(
  missing_kernel_crate_count
  kernel_dependency_violation_count
  typed_boundary_forbidden_symbol_violation_count
  typed_boundary_old_dynamic_package_violation_count
  typed_boundary_old_dynamic_root_violation_count
  typed_boundary_old_submission_escape_hatch_count
  manual_persisted_trait_impl_violation_count
  derive_provenance_forgery_violation_count
)

SLICE_TRUE_KEYS=(
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

SLICE_POSITIVE_KEYS=(
  cell_events_count
  resume_drift_fixture_count
)

SLICE_ZERO_KEYS=(
  replay_live_cap_requests_count
)

PORT_TRUE_KEYS=(
  proof_implementation_conformance_passed
  portfolio_typed_port_passed
  evm_dcv_typed_port_passed
)

PORT_POSITIVE_KEYS=(
  typed_port_gate_count
)

REQUIRED_FULL_TASKS=(
  task.ci.workflow-basic
  task.ci.typed-certified-slice
  task.ci.typed-port-gates
  task.ci.workflow-parity
)

usage() {
  cat <<'EOF'
usage: check-full-typed-core-gate.sh [--root PATH] [--artifacts-dir PATH] [--registry-root PATH]
                                     [--run-id ID] [--attempt-id ID] [--summary-file PATH]
                                     [--self-test]

Validates that full CI executed all typed-core gate summaries and mandatory
workflow-port tasks without missing, false, skipped, or xfail required gates.
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
    --artifacts-dir)
      if [ "$#" -lt 2 ]; then
        echo "ERROR: --artifacts-dir requires a value" >&2
        exit 2
      fi
      ARTIFACTS_DIR="$2"
      shift 2
      ;;
    --registry-root)
      if [ "$#" -lt 2 ]; then
        echo "ERROR: --registry-root requires a value" >&2
        exit 2
      fi
      REGISTRY_ROOT_VALUE="$2"
      shift 2
      ;;
    --run-id)
      if [ "$#" -lt 2 ]; then
        echo "ERROR: --run-id requires a value" >&2
        exit 2
      fi
      RUN_ID="$2"
      shift 2
      ;;
    --attempt-id)
      if [ "$#" -lt 2 ]; then
        echo "ERROR: --attempt-id requires a value" >&2
        exit 2
      fi
      ATTEMPT_ID="$2"
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

summary_contains_skip_or_xfail() {
  local summary_file="$1"

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
}

validate_summary_shape() {
  local summary_file="$1"
  local expected_kind="$2"

  if [ ! -f "$summary_file" ]; then
    echo "ERROR: required summary missing path=$summary_file" >&2
    exit 1
  fi

  jq -e --arg expected_kind "$expected_kind" \
    '.kind == $expected_kind and .version == 1 and (.payload | type == "object")' \
    "$summary_file" >/dev/null
}

validate_true_keys() {
  local summary_file="$1"
  shift
  local key

  for key in "$@"; do
    if ! jq -e --arg key "$key" '.payload[$key] == true' "$summary_file" >/dev/null; then
      echo "ERROR: required summary key missing or false key=$key summary=$summary_file" >&2
      exit 1
    fi
  done
}

validate_positive_keys() {
  local summary_file="$1"
  shift
  local key

  for key in "$@"; do
    if ! jq -e --arg key "$key" '(.payload[$key] | type == "number") and .payload[$key] > 0' "$summary_file" >/dev/null; then
      echo "ERROR: required positive summary count missing or zero key=$key summary=$summary_file" >&2
      exit 1
    fi
  done
}

validate_zero_keys() {
  local summary_file="$1"
  shift
  local key

  for key in "$@"; do
    if ! jq -e --arg key "$key" '(.payload[$key] | type == "number") and .payload[$key] == 0' "$summary_file" >/dev/null; then
      echo "ERROR: required zero summary count failed key=$key summary=$summary_file" >&2
      exit 1
    fi
  done
}

validate_no_markers() {
  local summary_file="$1"
  local markers

  markers="$(summary_contains_skip_or_xfail "$summary_file")"
  if [ -n "$markers" ]; then
    echo "ERROR: required summary contains skipped/xfail markers summary=$summary_file" >&2
    printf '%s\n' "$markers" >&2
    exit 1
  fi
}

validate_kernel_summary() {
  local summary_file="$1"

  validate_summary_shape "$summary_file" "typed-kernel-contract-summary"
  validate_true_keys "$summary_file" "${KERNEL_TRUE_KEYS[@]}"
  validate_positive_keys "$summary_file" "${KERNEL_POSITIVE_KEYS[@]}"
  validate_zero_keys "$summary_file" "${KERNEL_ZERO_KEYS[@]}"
  validate_no_markers "$summary_file"
}

validate_slice_summary() {
  local summary_file="$1"

  validate_summary_shape "$summary_file" "typed-certified-slice-summary"
  validate_true_keys "$summary_file" "${SLICE_TRUE_KEYS[@]}"
  validate_positive_keys "$summary_file" "${SLICE_POSITIVE_KEYS[@]}"
  validate_zero_keys "$summary_file" "${SLICE_ZERO_KEYS[@]}"
  jq -e '(.payload.public_output_event_id | type == "string") and (.payload.public_output_event_id | length > 0)' "$summary_file" >/dev/null
  validate_no_markers "$summary_file"
}

validate_port_summary() {
  local summary_file="$1"

  validate_summary_shape "$summary_file" "typed-port-gates-summary"
  validate_true_keys "$summary_file" "${PORT_TRUE_KEYS[@]}"
  validate_positive_keys "$summary_file" "${PORT_POSITIVE_KEYS[@]}"
  validate_no_markers "$summary_file"
}

event_index_file() {
  local registry_root="$1"
  printf '%s/events.index.tsv' "$registry_root"
}

task_latest_state() {
  local index_file="$1"
  local task_id="$2"
  local run_id="$3"
  local attempt_id="$4"
  local seq
  local ts_epoch
  local ts
  local event_run
  local event_attempt
  local workflow
  local task
  local state
  local reason
  local exit_code
  local found=0
  local latest_state=""
  local latest_reason=""

  while IFS=$'\t' read -r seq ts_epoch ts event_run event_attempt workflow task state reason exit_code; do
    if [ "$event_run" != "$run_id" ] || [ "$task" != "$task_id" ]; then
      continue
    fi
    if [ -n "$attempt_id" ] && [ "$event_attempt" != "$attempt_id" ]; then
      continue
    fi
    found=1
    latest_state="$state"
    latest_reason="$reason"
  done <"$index_file"

  if [ "$found" -eq 1 ]; then
    printf '%s\t%s' "$latest_state" "$latest_reason"
  fi
}

resolve_current_attempt_id() {
  local index_file="$1"
  local run_id="$2"
  local current_task_id="$3"
  local seq
  local ts_epoch
  local ts
  local event_run
  local event_attempt
  local workflow
  local task
  local state
  local reason
  local exit_code
  local found=0
  local latest_attempt=""

  while IFS=$'\t' read -r seq ts_epoch ts event_run event_attempt workflow task state reason exit_code; do
    if [ "$event_run" != "$run_id" ] || [ "$task" != "$current_task_id" ]; then
      continue
    fi
    found=1
    latest_attempt="$event_attempt"
  done <"$index_file"

  if [ "$found" -eq 1 ]; then
    printf '%s' "$latest_attempt"
  fi
}

validate_required_task_events() {
  local registry_root="$1"
  local run_id="$2"
  local attempt_id="$3"
  local index_file
  local task_id
  local record
  local state
  local reason
  local required_count=0

  if [ -z "$registry_root" ] || [ -z "$run_id" ] || [ -z "$attempt_id" ]; then
    echo "ERROR: registry root, run id, and attempt id are required for full typed-core task validation" >&2
    exit 2
  fi

  index_file="$(event_index_file "$registry_root")"
  if [ ! -f "$index_file" ]; then
    echo "ERROR: registry event index missing path=$index_file" >&2
    exit 1
  fi

  for task_id in "${REQUIRED_FULL_TASKS[@]}"; do
    required_count=$((required_count + 1))
    record="$(task_latest_state "$index_file" "$task_id" "$run_id" "$attempt_id")"
    if [ -z "$record" ]; then
      echo "ERROR: required full CI task missing task=$task_id run=$run_id attempt=${attempt_id:-<any>}" >&2
      exit 1
    fi
    state="${record%%$'\t'*}"
    reason="${record#*$'\t'}"
    if [ "$state" != "passed" ]; then
      echo "ERROR: required full CI task did not pass task=$task_id state=$state reason=${reason:-<none>}" >&2
      exit 1
    fi
  done

  FULL_GATE_REQUIRED_TASK_COUNT="$required_count"
}

write_summary() {
  local summary_file="$1"
  local required_summary_count="$2"
  local required_task_count="$3"
  local summary_dir
  local summary_tmp

  if [ -z "$summary_file" ]; then
    return 0
  fi

  summary_dir="$(dirname "$summary_file")"
  mkdir -p "$summary_dir"
  summary_tmp="$(mktemp "$summary_file.tmp.XXXXXX")"

  jq -n \
    --argjson full_typed_core_gate_passed true \
    --argjson required_summary_count "$required_summary_count" \
    --argjson required_task_count "$required_task_count" \
    '{
      kind: "full-typed-core-gate-summary",
      version: 1,
      payload: {
        full_typed_core_gate_passed: $full_typed_core_gate_passed,
        required_gate_summary_count: $required_summary_count,
        required_workflow_task_count: $required_task_count
      }
    }' >"$summary_tmp"

  mv "$summary_tmp" "$summary_file"
}

write_valid_fixture_summaries() {
  local dir="$1"

  jq -n '{
    kind: "typed-kernel-contract-summary",
    version: 1,
    payload: {
      kernel_crates_present: true,
      typed_boundary_firewall_passed: true,
      value_config_output_derives_present: true,
      manual_value_config_output_impls_rejected: true,
      registered_state_required: true,
      registered_operation_required: true,
      bridge_session_evidence_required: true,
      semantic_lineage_evidence_present: true,
      stable_ids_golden: true,
      typed_spec_hash_deterministic: true,
      v1_event_schema_golden: true,
      invalid_topology_rejected: true,
      invalid_interface_wiring_rejected: true,
      invalid_semantic_transition_rejected: true,
      invalid_data_shape_rejected: true,
      invalid_data_meaning_rejected: true,
      invalid_terminal_shape_rejected: true,
      kernel_crates_expected_count: 14,
      kernel_crates_actual_count: 14,
      typed_boundary_scanned_file_count: 1,
      persisted_source_boundary_rust_file_count: 1,
      typed_kernel_contract_required_key_count: 17,
      missing_kernel_crate_count: 0,
      kernel_dependency_violation_count: 0,
      typed_boundary_forbidden_symbol_violation_count: 0,
      typed_boundary_old_dynamic_package_violation_count: 0,
      typed_boundary_old_dynamic_root_violation_count: 0,
      typed_boundary_old_submission_escape_hatch_count: 0,
      manual_persisted_trait_impl_violation_count: 0,
      derive_provenance_forgery_violation_count: 0
    }
  }' >"$dir/typed-kernel-contract.summary.json"

  jq -n '{
    kind: "typed-certified-slice-summary",
    version: 1,
    payload: {
      typed_spec_hash_persisted: true,
      run_started_v1_present: true,
      seed_material_persisted: true,
      side_effect_ledger_complete: true,
      side_effect_invocation_started_before_submit: true,
      side_effect_crash_cases_passed: true,
      side_effect_no_duplicate_submit: true,
      side_effect_submission_unknown_recovered: true,
      side_effect_failed_semantics_covered: true,
      side_effect_logical_key_conflicts_rejected: true,
      managed_platform_outputs_committed: true,
      public_output_before_run_completed: true,
      resume_drift_rejected: true,
      retention_projection_complete: true,
      cell_events_count: 1,
      resume_drift_fixture_count: 1,
      replay_live_cap_requests_count: 0,
      public_output_event_id: "event:fixture"
    }
  }' >"$dir/typed-certified-slice.summary.json"

  jq -n '{
    kind: "typed-port-gates-summary",
    version: 1,
    payload: {
      proof_implementation_conformance_passed: true,
      portfolio_typed_port_passed: true,
      evm_dcv_typed_port_passed: true,
      typed_port_gate_count: 3
    }
  }' >"$dir/typed-port-gates.summary.json"
}

write_valid_fixture_events() {
  local registry_root="$1"
  local run_id="$2"
  local attempt_id="$3"
  local index_file="$registry_root/events.index.tsv"
  local seq=1
  local task_id

  mkdir -p "$registry_root"
  : >"$index_file"
  for task_id in "${REQUIRED_FULL_TASKS[@]}"; do
    printf '%s\t0\t1970-01-01T00:00:00Z\t%s\t%s\tworkflow.fixture\t%s\tpassed\t\t0\n' \
      "$seq" "$run_id" "$attempt_id" "$task_id" >>"$index_file"
    seq=$((seq + 1))
  done
}

run_self_tests() {
  local temp_root
  local artifacts
  local registry
  local run_id="run-self-test"
  local attempt_id="attempt-self-test"

  temp_root="$(mktemp -d "${TMPDIR:-/tmp}/mfm-full-typed-core-gate.self-test.XXXXXX")"
  artifacts="$temp_root/artifacts"
  registry="$temp_root/registry"
  mkdir -p "$artifacts" "$registry"
  write_valid_fixture_summaries "$artifacts"
  write_valid_fixture_events "$registry" "$run_id" "$attempt_id"

  validate_kernel_summary "$artifacts/typed-kernel-contract.summary.json"
  validate_slice_summary "$artifacts/typed-certified-slice.summary.json"
  validate_port_summary "$artifacts/typed-port-gates.summary.json"
  validate_required_task_events "$registry" "$run_id" "$attempt_id"

  if ( validate_required_task_events "$registry" "$run_id" "attempt-stale" ) >/dev/null 2>&1; then
    echo "ERROR: self-test failed: stale attempt should not satisfy required task events" >&2
    exit 1
  fi

  printf '%s\t0\t1970-01-01T00:00:00Z\t%s\t%s\tworkflow.fixture\t%s\trunning\t\t0\n' \
    999 "$run_id" "$attempt_id" "task.ci.full-typed-core-gate" >>"$registry/events.index.tsv"
  if [ "$(resolve_current_attempt_id "$registry/events.index.tsv" "$run_id" "task.ci.full-typed-core-gate")" != "$attempt_id" ]; then
    echo "ERROR: self-test failed: current attempt resolution returned unexpected attempt id" >&2
    exit 1
  fi

  jq '.payload.typed_boundary_firewall_passed = false' \
    "$artifacts/typed-kernel-contract.summary.json" >"$artifacts/typed-kernel-contract.summary.false.json"
  if ( validate_kernel_summary "$artifacts/typed-kernel-contract.summary.false.json" ) >/dev/null 2>&1; then
    echo "ERROR: self-test failed: false kernel summary key should fail" >&2
    exit 1
  fi

  jq '.payload.skip_reason = "skipped"' \
    "$artifacts/typed-port-gates.summary.json" >"$artifacts/typed-port-gates.summary.skip.json"
  if ( validate_port_summary "$artifacts/typed-port-gates.summary.skip.json" ) >/dev/null 2>&1; then
    echo "ERROR: self-test failed: skipped marker should fail" >&2
    exit 1
  fi

  while IFS=$'\t' read -r seq ts_epoch ts event_run event_attempt workflow task state reason exit_code; do
    if [ "$task" = "task.ci.typed-certified-slice" ]; then
      state="canceled"
      reason="service-skipped"
    fi
    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
      "$seq" "$ts_epoch" "$ts" "$event_run" "$event_attempt" "$workflow" "$task" "$state" "$reason" "$exit_code"
  done <"$registry/events.index.tsv" >"$registry/events.index.skip.tsv"
  mv "$registry/events.index.skip.tsv" "$registry/events.index.tsv"
  if ( validate_required_task_events "$registry" "$run_id" "$attempt_id" ) >/dev/null 2>&1; then
    echo "ERROR: self-test failed: skipped required task should fail" >&2
    exit 1
  fi

  rm -rf "$temp_root"
  echo "OK: full typed-core gate self-tests passed"
}

if [ "$RUN_SELF_TESTS" -eq 1 ]; then
  run_self_tests
fi

if [ -z "$ARTIFACTS_DIR" ]; then
  echo "ERROR: --artifacts-dir or CI_ARTIFACTS_DIR is required" >&2
  exit 2
fi

if [ -z "$SUMMARY_FILE" ]; then
  SUMMARY_FILE="$ARTIFACTS_DIR/full-typed-core-gate.summary.json"
fi

KERNEL_SUMMARY="$ARTIFACTS_DIR/typed-kernel-contract.summary.json"
SLICE_SUMMARY="$ARTIFACTS_DIR/typed-certified-slice.summary.json"
PORT_SUMMARY="$ARTIFACTS_DIR/typed-port-gates.summary.json"
FULL_GATE_REQUIRED_TASK_COUNT=0
REGISTRY_INDEX_FILE="$(event_index_file "$REGISTRY_ROOT_VALUE")"

if [ -z "$ATTEMPT_ID" ]; then
  if [ ! -f "$REGISTRY_INDEX_FILE" ]; then
    echo "ERROR: registry event index missing path=$REGISTRY_INDEX_FILE" >&2
    exit 1
  fi
  CURRENT_TASK_ID="${NIXFIED_TASK_ID:-task.ci.full-typed-core-gate}"
  ATTEMPT_ID="$(resolve_current_attempt_id "$REGISTRY_INDEX_FILE" "$RUN_ID" "$CURRENT_TASK_ID")"
fi

if [ -z "$ATTEMPT_ID" ]; then
  echo "ERROR: unable to resolve current attempt id run=$RUN_ID task=${CURRENT_TASK_ID:-task.ci.full-typed-core-gate}" >&2
  exit 2
fi

validate_kernel_summary "$KERNEL_SUMMARY"
validate_slice_summary "$SLICE_SUMMARY"
validate_port_summary "$PORT_SUMMARY"
validate_required_task_events "$REGISTRY_ROOT_VALUE" "$RUN_ID" "$ATTEMPT_ID"
write_summary "$SUMMARY_FILE" 3 "$FULL_GATE_REQUIRED_TASK_COUNT"

echo "INFO: full_typed_core_gate_summary=$SUMMARY_FILE"
echo "OK: full typed-core gate passed"
