#!/usr/bin/env bash
set -euo pipefail

ROOT="."
SUMMARY_FILE=""
RUN_SELF_TESTS=0
METADATA_FILE=""

OLD_DYNAMIC_PACKAGES=(
  "mfm-machine"
  "mfm-machine-derive"
  "mfm-machine-test-support"
  "mfm-sdk"
  "mfm-app-legacy"
  "mfm-control-plane-model"
  "mfm-op-evm-read"
  "mfm-op-evm-write"
  "mfm-op-keystore"
  "mfm-op-nix-app"
  "mfm-portfolio-plan"
  "mfm-state-common"
  "mfm-state-keystore"
  "mfm-state-aave-v3"
  "mfm-evm-runtime"
  "mfm-stream-store-mem"
  "mfm-artifact-store-s3"
  "mfm-artifact-store-secret"
  "mfm-control-plane-postgres"
  "mfm-transports-exec"
  "mfm-transports-local-evm"
  "mfm-transports-local-fs"
  "mfm-transports-local-keystore"
  "mfm-transports-rpc-control"
  "mfm-collectors-evm"
  "mfm-collectors-evm-jsonrpc-http"
  "mfm-collectors-exec"
  "mfm-collectors-local-evm"
  "mfm-collectors-local-keystore"
  "mfm-collectors-nix"
  "mfm-collectors-nix-exec"
  "mfm-collectors-rpc-control"
)

OLD_DYNAMIC_ROOTS=(
  "crates/machine"
  "crates/machine-derive"
  "crates/machine-test-support"
  "crates/sdk"
  "crates/app-legacy"
  "crates/control-plane/model"
  "crates/ops/evm-read-op"
  "crates/ops/evm-write-op"
  "crates/ops/keystore-op"
  "crates/ops/nix-app-op"
  "crates/portfolio/plan"
  "crates/states/common"
  "crates/states/keystore"
  "crates/states/aave-v3"
  "crates/evm-runtime"
  "crates/storages/stream-store-mem"
  "crates/storages/artifact-store-s3"
  "crates/storages/artifact-store-secret"
  "crates/storages/control-plane-postgres"
  "crates/transports/exec"
  "crates/transports/local-evm"
  "crates/transports/local-fs"
  "crates/transports/local-keystore"
  "crates/transports/rpc-control"
  "crates/collectors/evm"
  "crates/collectors/evm-jsonrpc-http"
  "crates/collectors/exec"
  "crates/collectors/local-evm"
  "crates/collectors/local-keystore"
  "crates/collectors/nix"
  "crates/collectors/nix-exec"
  "crates/collectors/rpc-control"
)

FORBIDDEN_SOURCE_REGEX='PlannedOp|PortKey|DynContext|StateGraph|DependencyEdge|IoProvider|ContextKey|mfm_machine|mfm_sdk|mfm_app_legacy|mfm-machine|mfm-sdk|mfm-app-legacy|context[_-]key|context[_-]dataflow|context[_-]snapshot|read_context|write_context'
TYPED_SUBMISSION_REGEX='CertifiedSpecEnvelope|TypedExecutionSpec|CertifiedTypedSpec|mfm_spec|mfm_certify|mfm_runtime|mfm_app|TypedRunEventStore|PostgresTypedRunEventStore|start_typed_run|run_start|start_run|RunStarted'

usage() {
  cat <<'EOF'
usage: check-typed-boundary-firewall.sh [--root PATH] [--metadata PATH] [--summary-file PATH] [--self-test]

Verifies that removed dynamic semantic APIs cannot re-enter active typed
source, workspace packages, or certified typed run submission authority.
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
    --metadata)
      if [ "$#" -lt 2 ]; then
        echo "ERROR: --metadata requires a value" >&2
        exit 2
      fi
      METADATA_FILE="$2"
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

normalize_repo_path() {
  local path="$1"
  local rel="$path"

  case "$path" in
    "$ROOT_ABS")
      rel="."
      ;;
    "$ROOT_ABS"/*)
      rel="${path#"$ROOT_ABS"/}"
      ;;
  esac

  rel="${rel#./}"
  rel="${rel%/}"
  printf '%s' "$rel"
}

bool_json() {
  if [ "$1" -eq 0 ]; then
    printf 'true'
  else
    printf 'false'
  fi
}

list_contains() {
  local needle="$1"
  shift
  local item

  for item in "$@"; do
    if [ "$item" = "$needle" ]; then
      return 0
    fi
  done
  return 1
}

path_under_old_root() {
  local rel="$1"
  local root

  for root in "${OLD_DYNAMIC_ROOTS[@]}"; do
    if [ "$rel" = "$root" ] || [[ "$rel" == "$root"/* ]]; then
      return 0
    fi
  done
  return 1
}

collect_boundary_files() {
  local scan_root="$1"
  local dir

  for dir in bin crates tests; do
    if [ -d "$scan_root/$dir" ]; then
      find "$scan_root/$dir" -type f \( -name '*.rs' -o -name 'Cargo.toml' \) \
        -not -path '*/target/*' \
        -not -path '*/.git/*'
    fi
  done
}

is_allowed_source_match() {
  local rel="$1"
  local line="$2"

  case "$rel:$line" in
    'bin/cli/src/commands/keystore/mod.rs:'*'"mfm_app_legacy",'*)
      return 0
      ;;
    'crates/app/tests/typed_transport_boundaries.rs:'*'"mfm-machine",'*)
      return 0
      ;;
    'crates/app/tests/typed_transport_boundaries.rs:'*'"mfm-sdk",'*)
      return 0
      ;;
    'crates/kernel/program/tests/ui/fail/context_key_state_input.rs:'*'struct ContextKey(String);'*)
      return 0
      ;;
    'crates/kernel/program/tests/ui/fail/context_key_state_input.rs:'*'let key = ContextKey("legacy.context.key".to_owned());'*)
      return 0
      ;;
    'crates/kernel/program/tests/ui/fail/context_key_state_input.rs:'*'<ContextKey as mfm_program::IntoStateInput'*)
      return 0
      ;;
    'crates/ops/portfolio-tracker-op/src/lib.rs:'*'.state_descriptor_name.contains("DynContext")'*)
      return 0
      ;;
    'crates/ops/proof-op/src/lib.rs:'*'.state_descriptor_name.contains("DynContext")'*)
      return 0
      ;;
  esac

  return 1
}

source_boundary_counts() {
  local scan_root="$1"
  local file
  local rel
  local match
  local match_rel
  local line
  local scanned_count=0
  local violation_count=0

  while IFS= read -r file; do
    scanned_count=$((scanned_count + 1))
    rel="$(normalize_repo_path "$file")"

    while IFS= read -r match; do
      [ -n "$match" ] || continue
      match_rel="${match#"$ROOT_ABS"/}"
      line="${match#*:}"
      line="${line#*:}"
      if printf '%s\n' "$line" | grep -Eq '^[[:space:]]*(//|#|\*)'; then
        continue
      fi
      if is_allowed_source_match "$rel" "$line"; then
        continue
      fi
      echo "ERROR: forbidden old semantic source token file=$match_rel" >&2
      violation_count=$((violation_count + 1))
    done < <(grep -HEn "$FORBIDDEN_SOURCE_REGEX" "$file" || true)
  done < <(collect_boundary_files "$scan_root")

  printf '%s\t%s\n' "$scanned_count" "$violation_count"
}

workspace_metadata_file() {
  if [ -n "$METADATA_FILE" ]; then
    printf '%s' "$METADATA_FILE"
    return 0
  fi

  local metadata_tmp
  metadata_tmp="$(mktemp "${TMPDIR:-/tmp}/mfm-typed-boundary.metadata.XXXXXX")"
  (
    cd "$ROOT_ABS"
    cargo metadata --no-deps --format-version 1 >"$metadata_tmp"
  )
  printf '%s' "$metadata_tmp"
}

old_dynamic_package_violation_count() {
  local metadata_file="$1"
  local count=0
  local package_name
  local package_record
  local manifest
  local dependency_name
  local dependency_path
  local dependency_path_rel

  while IFS= read -r package_name; do
    [ -n "$package_name" ] || continue
    if list_contains "$package_name" "${OLD_DYNAMIC_PACKAGES[@]}"; then
      echo "ERROR: old dynamic package remains in workspace package=$package_name" >&2
      count=$((count + 1))
    fi
  done < <(
    jq -r '
      .workspace_members as $members
      | .packages[]
      | select(.id as $id | ($members | index($id)))
      | .name
    ' "$metadata_file"
  )

  while IFS=$'\t' read -r package_record manifest dependency_name dependency_path; do
    [ -n "$package_record" ] || continue
    if list_contains "$dependency_name" "${OLD_DYNAMIC_PACKAGES[@]}"; then
      echo "ERROR: active package depends on old dynamic package package=$package_record dependency=$dependency_name manifest=$(normalize_repo_path "$manifest")" >&2
      count=$((count + 1))
    fi

    if [ -n "$dependency_path" ]; then
      dependency_path_rel="$(normalize_repo_path "$dependency_path")"
      if path_under_old_root "$dependency_path_rel"; then
        echo "ERROR: active package depends on old dynamic root package=$package_record dependency=$dependency_name path=$dependency_path_rel" >&2
        count=$((count + 1))
      fi
    fi
  done < <(
    jq -r '
      .workspace_members as $members
      | .packages[]
      | select(.id as $id | ($members | index($id)))
      | . as $package
      | ($package.dependencies // [])[]
      | select(.source == null)
      | [$package.name, $package.manifest_path, .name, (.path // "")]
      | @tsv
    ' "$metadata_file"
  )

  printf '%s\n' "$count"
}

old_dynamic_root_violation_count() {
  local scan_root="$1"
  local count=0
  local root

  for root in "${OLD_DYNAMIC_ROOTS[@]}"; do
    if [ -e "$scan_root/$root" ]; then
      echo "ERROR: old dynamic semantic root still exists path=$root" >&2
      count=$((count + 1))
    fi
  done

  printf '%s\n' "$count"
}

old_submission_escape_hatch_count() {
  local scan_root="$1"
  local count=0
  local root
  local file

  for root in "${OLD_DYNAMIC_ROOTS[@]}"; do
    if [ ! -d "$scan_root/$root" ]; then
      continue
    fi

    while IFS= read -r file; do
      if grep -En "$TYPED_SUBMISSION_REGEX" "$file" >/dev/null; then
        echo "ERROR: old dynamic root references certified typed submission authority file=$(normalize_repo_path "$file")" >&2
        count=$((count + 1))
      fi
    done < <(
      find "$scan_root/$root" -type f \( -name '*.rs' -o -name 'Cargo.toml' \) \
        -not -path '*/target/*' \
        -not -path '*/.git/*'
    )
  done

  printf '%s\n' "$count"
}

write_contract_summary() {
  local summary_file="$1"
  local scanned_file_count="$2"
  local source_violation_count="$3"
  local package_violation_count="$4"
  local root_violation_count="$5"
  local escape_hatch_count="$6"
  local total_violation_count
  local summary_dir
  local summary_tmp

  if [ -z "$summary_file" ]; then
    return 0
  fi

  total_violation_count=$((source_violation_count + package_violation_count + root_violation_count + escape_hatch_count))
  summary_dir="$(dirname "$summary_file")"
  mkdir -p "$summary_dir"
  summary_tmp="$(mktemp "$summary_file.tmp.XXXXXX")"

  if [ -f "$summary_file" ]; then
    jq \
      --argjson typed_boundary_firewall_passed "$(bool_json "$total_violation_count")" \
      --argjson typed_boundary_scanned_file_count "$scanned_file_count" \
      --argjson typed_boundary_forbidden_symbol_violation_count "$source_violation_count" \
      --argjson typed_boundary_old_dynamic_package_violation_count "$package_violation_count" \
      --argjson typed_boundary_old_dynamic_root_violation_count "$root_violation_count" \
      --argjson typed_boundary_old_submission_escape_hatch_count "$escape_hatch_count" \
      '.kind = "typed-kernel-contract-summary"
       | .version = 1
       | .payload = (.payload // {})
       | .payload.typed_boundary_firewall_passed = $typed_boundary_firewall_passed
       | .payload.typed_boundary_scanned_file_count = $typed_boundary_scanned_file_count
       | .payload.typed_boundary_forbidden_symbol_violation_count = $typed_boundary_forbidden_symbol_violation_count
       | .payload.typed_boundary_old_dynamic_package_violation_count = $typed_boundary_old_dynamic_package_violation_count
       | .payload.typed_boundary_old_dynamic_root_violation_count = $typed_boundary_old_dynamic_root_violation_count
       | .payload.typed_boundary_old_submission_escape_hatch_count = $typed_boundary_old_submission_escape_hatch_count' \
      "$summary_file" >"$summary_tmp"
  else
    jq -n \
      --argjson typed_boundary_firewall_passed "$(bool_json "$total_violation_count")" \
      --argjson typed_boundary_scanned_file_count "$scanned_file_count" \
      --argjson typed_boundary_forbidden_symbol_violation_count "$source_violation_count" \
      --argjson typed_boundary_old_dynamic_package_violation_count "$package_violation_count" \
      --argjson typed_boundary_old_dynamic_root_violation_count "$root_violation_count" \
      --argjson typed_boundary_old_submission_escape_hatch_count "$escape_hatch_count" \
      '{
        kind: "typed-kernel-contract-summary",
        version: 1,
        payload: {
          typed_boundary_firewall_passed: $typed_boundary_firewall_passed,
          typed_boundary_scanned_file_count: $typed_boundary_scanned_file_count,
          typed_boundary_forbidden_symbol_violation_count: $typed_boundary_forbidden_symbol_violation_count,
          typed_boundary_old_dynamic_package_violation_count: $typed_boundary_old_dynamic_package_violation_count,
          typed_boundary_old_dynamic_root_violation_count: $typed_boundary_old_dynamic_root_violation_count,
          typed_boundary_old_submission_escape_hatch_count: $typed_boundary_old_submission_escape_hatch_count
        }
      }' >"$summary_tmp"
  fi

  mv "$summary_tmp" "$summary_file"
  echo "INFO: typed_kernel_contract_summary=$summary_file"
}

run_self_tests() {
  local temp_root
  local counts
  local scanned
  local violations
  local metadata_fixture

  temp_root="$(mktemp -d "${TMPDIR:-/tmp}/mfm-typed-boundary.self-test.XXXXXX")"
  mkdir -p "$temp_root/crates/example/src"
  printf '%s\n' '//! PlannedOp in comments is not a semantic API.' >"$temp_root/crates/example/src/lib.rs"
  counts="$(source_boundary_counts "$temp_root" 2>/dev/null)"
  scanned="${counts%%$'\t'*}"
  violations="${counts##*$'\t'}"
  if [ "$scanned" -eq 0 ] || [ "$violations" -ne 0 ]; then
    echo "ERROR: self-test failed: comment-only forbidden symbol should pass" >&2
    exit 1
  fi

  printf '%s\n' 'pub struct PlannedOp;' >"$temp_root/crates/example/src/lib.rs"
  counts="$(source_boundary_counts "$temp_root" 2>/dev/null)"
  violations="${counts##*$'\t'}"
  if [ "$violations" -eq 0 ]; then
    echo "ERROR: self-test failed: forbidden source token should fail" >&2
    exit 1
  fi

  mkdir -p "$temp_root/crates/machine/src"
  printf '%s\n' 'use mfm_spec::v1::CertifiedSpecEnvelope;' >"$temp_root/crates/machine/src/lib.rs"
  if [ "$(old_dynamic_root_violation_count "$temp_root" 2>/dev/null)" -eq 0 ]; then
    echo "ERROR: self-test failed: old dynamic root should fail" >&2
    exit 1
  fi
  if [ "$(old_submission_escape_hatch_count "$temp_root" 2>/dev/null)" -eq 0 ]; then
    echo "ERROR: self-test failed: old dynamic typed submission reference should fail" >&2
    exit 1
  fi

  metadata_fixture="$(mktemp "${TMPDIR:-/tmp}/mfm-typed-boundary.self-test.metadata.XXXXXX")"
  jq -n '{
    packages: [
      {
        id: "path+file:///tmp/example#mfm-example@0.1.0",
        name: "mfm-example",
        manifest_path: "/tmp/example/Cargo.toml",
        dependencies: [
          {
            name: "mfm-machine",
            source: null,
            path: "/tmp/example/crates/machine"
          }
        ]
      }
    ],
    workspace_members: ["path+file:///tmp/example#mfm-example@0.1.0"]
  }' >"$metadata_fixture"
  if [ "$(old_dynamic_package_violation_count "$metadata_fixture" 2>/dev/null)" -eq 0 ]; then
    echo "ERROR: self-test failed: old dynamic package dependency should fail" >&2
    exit 1
  fi

  rm -rf "$temp_root"
  rm -f "$metadata_fixture"
  echo "OK: typed boundary firewall self-tests passed"
}

main() {
  local metadata_file
  local metadata_generated=0
  local counts
  local scanned_file_count
  local source_violation_count
  local package_violation_count
  local root_violation_count
  local escape_hatch_count
  local total_violation_count

  if [ "$RUN_SELF_TESTS" -eq 1 ]; then
    run_self_tests
  fi

  counts="$(source_boundary_counts "$ROOT_ABS")"
  scanned_file_count="${counts%%$'\t'*}"
  source_violation_count="${counts##*$'\t'}"
  if [ "$scanned_file_count" -eq 0 ]; then
    echo "ERROR: typed boundary firewall scanned zero source files" >&2
    source_violation_count=$((source_violation_count + 1))
  fi

  metadata_file="$(workspace_metadata_file)"
  if [ -z "$METADATA_FILE" ]; then
    metadata_generated=1
  fi

  package_violation_count="$(old_dynamic_package_violation_count "$metadata_file")"
  root_violation_count="$(old_dynamic_root_violation_count "$ROOT_ABS")"
  escape_hatch_count="$(old_submission_escape_hatch_count "$ROOT_ABS")"
  total_violation_count=$((source_violation_count + package_violation_count + root_violation_count + escape_hatch_count))

  write_contract_summary \
    "$SUMMARY_FILE" \
    "$scanned_file_count" \
    "$source_violation_count" \
    "$package_violation_count" \
    "$root_violation_count" \
    "$escape_hatch_count"

  if [ "$metadata_generated" -eq 1 ]; then
    rm -f "$metadata_file"
  fi

  if [ "$total_violation_count" -ne 0 ]; then
    echo "ERROR: typed boundary firewall failed violations=$total_violation_count" >&2
    exit 1
  fi

  echo "OK: typed boundary firewall passed scanned_files=$scanned_file_count"
}

main
