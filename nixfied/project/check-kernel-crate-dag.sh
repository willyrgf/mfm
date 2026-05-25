#!/usr/bin/env bash
set -euo pipefail

EXPECTED_KERNEL_MANIFESTS=(
  "crates/kernel/ids/Cargo.toml"
  "crates/kernel/canonical/Cargo.toml"
  "crates/kernel/values/Cargo.toml"
  "crates/kernel/effects/Cargo.toml"
  "crates/kernel/capabilities/Cargo.toml"
  "crates/kernel/program/Cargo.toml"
  "crates/kernel/program-derive/Cargo.toml"
  "crates/kernel/replay/Cargo.toml"
  "crates/kernel/runtime/Cargo.toml"
  "crates/kernel/spec/Cargo.toml"
  "crates/kernel/certify/Cargo.toml"
  "crates/kernel/events/Cargo.toml"
  "crates/kernel/store/Cargo.toml"
  "crates/kernel/test-support/Cargo.toml"
)

ROOT="."
METADATA_FILE=""
SUMMARY_FILE=""
RUN_SELF_TESTS=0

usage() {
  cat <<'EOF'
usage: check-kernel-crate-dag.sh [--root PATH] [--metadata PATH] [--summary-file PATH] [--self-test]

Verifies that typed kernel workspace crates are present and that every local
path dependency from crates/kernel/* points only at another crates/kernel/*
package.
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

write_contract_summary() {
  local summary_file="$1"
  local missing_count="$2"
  local violation_count="$3"
  local expected_count="$4"
  local actual_count="$5"
  local summary_dir
  local summary_tmp

  if [ -z "$summary_file" ]; then
    return 0
  fi

  summary_dir="$(dirname "$summary_file")"
  mkdir -p "$summary_dir"
  summary_tmp="$(mktemp "$summary_file.tmp.XXXXXX")"

  jq -n \
    --argjson kernel_crates_present "$(bool_json "$missing_count")" \
    --argjson crate_dag_passed "$(bool_json "$((missing_count + violation_count))")" \
    --argjson expected_count "$expected_count" \
    --argjson actual_count "$actual_count" \
    --argjson missing_count "$missing_count" \
    --argjson violation_count "$violation_count" \
    '{
      kind: "typed-kernel-contract-summary",
      version: 1,
      payload: {
        kernel_crates_present: $kernel_crates_present,
        crate_dag_passed: $crate_dag_passed,
        kernel_crates_expected_count: $expected_count,
        kernel_crates_actual_count: $actual_count,
        missing_kernel_crate_count: $missing_count,
        kernel_dependency_violation_count: $violation_count
      }
    }' >"$summary_tmp"

  mv "$summary_tmp" "$summary_file"
  echo "INFO: typed_kernel_contract_summary=$summary_file"
}

workspace_metadata_file() {
  if [ -n "$METADATA_FILE" ]; then
    printf '%s' "$METADATA_FILE"
    return 0
  fi

  local metadata_tmp
  metadata_tmp="$(mktemp "${TMPDIR:-/tmp}/mfm-kernel-crate-dag.metadata.XXXXXX")"
  (
    cd "$ROOT_ABS"
    cargo metadata --no-deps --format-version 1 >"$metadata_tmp"
  )
  printf '%s' "$metadata_tmp"
}

check_metadata() {
  local metadata_file="$1"
  local summary_file="$2"
  local expected_count="${#EXPECTED_KERNEL_MANIFESTS[@]}"
  local missing_count=0
  local violation_count=0
  local actual_count=0
  local name
  local manifest
  local manifest_rel
  local dependency_name
  local dependency_path
  local dependency_path_rel
  local expected_manifest
  local package_record
  local found_manifests_tmp

  found_manifests_tmp="$(mktemp "${TMPDIR:-/tmp}/mfm-kernel-crate-dag.found.XXXXXX")"

  while IFS=$'\t' read -r name manifest; do
    if [ -z "$name" ] || [ -z "$manifest" ]; then
      continue
    fi

    manifest_rel="$(normalize_repo_path "$manifest")"
    case "$manifest_rel" in
      crates/kernel/*/Cargo.toml)
        printf '%s\n' "$manifest_rel" >>"$found_manifests_tmp"
        ;;
    esac
  done < <(
    jq -r '
      .workspace_members as $members
      | .packages[]
      | select(.id as $id | ($members | index($id)))
      | [.name, .manifest_path]
      | @tsv
    ' "$metadata_file"
  )

  sort -u "$found_manifests_tmp" -o "$found_manifests_tmp"
  actual_count="$(wc -l <"$found_manifests_tmp" | tr -d '[:space:]')"

  for expected_manifest in "${EXPECTED_KERNEL_MANIFESTS[@]}"; do
    if ! grep -Fx "$expected_manifest" "$found_manifests_tmp" >/dev/null; then
      echo "ERROR: missing expected kernel crate manifest=$expected_manifest"
      missing_count=$((missing_count + 1))
    fi
  done

  while IFS=$'\t' read -r package_record manifest dependency_name dependency_path; do
    if [ -z "$package_record" ] || [ -z "$manifest" ] || [ -z "$dependency_path" ]; then
      continue
    fi

    manifest_rel="$(normalize_repo_path "$manifest")"
    case "$manifest_rel" in
      crates/kernel/*/Cargo.toml)
        ;;
      *)
        continue
        ;;
    esac

    dependency_path_rel="$(normalize_repo_path "$dependency_path")"
    case "$dependency_path_rel" in
      crates/kernel/*)
        ;;
      *)
        echo "ERROR: kernel crate dependency violation crate=$package_record manifest=$manifest_rel dependency=$dependency_name path=$dependency_path_rel"
        violation_count=$((violation_count + 1))
        ;;
    esac
  done < <(
    jq -r '
      .workspace_members as $members
      | .packages[]
      | select(.id as $id | ($members | index($id)))
      | . as $package
      | ($package.dependencies // [])[]
      | select(.path? != null and .path != "")
      | [$package.name, $package.manifest_path, .name, .path]
      | @tsv
    ' "$metadata_file"
  )

  write_contract_summary "$summary_file" "$missing_count" "$violation_count" "$expected_count" "$actual_count"

  if [ "$missing_count" -ne 0 ] || [ "$violation_count" -ne 0 ]; then
    echo "ERROR: kernel crate DAG check failed missing=$missing_count violations=$violation_count"
    rm -f "$found_manifests_tmp"
    return 1
  fi

  echo "OK: kernel crates present count=$actual_count expected=$expected_count"
  echo "OK: kernel crate local dependencies stay within crates/kernel"
  rm -f "$found_manifests_tmp"
}

make_dependency_fixture() {
  local base_metadata="$1"
  local dependency_name="$2"
  local dependency_rel_path="$3"
  local target_package="$4"
  local fixture_file="$5"

  jq \
    --arg dependency_name "$dependency_name" \
    --arg dependency_path "$ROOT_ABS/$dependency_rel_path" \
    --arg target_package "$target_package" \
    '
      .packages |= map(
        if .name == $target_package then
          .dependencies += [
            {
              name: $dependency_name,
              source: null,
              req: "*",
              kind: null,
              rename: null,
              optional: false,
              uses_default_features: true,
              features: [],
              target: null,
              registry: null,
              path: $dependency_path
            }
          ]
        else
          .
        end
      )
    ' "$base_metadata" >"$fixture_file"
}

run_self_tests() {
  local base_metadata
  local fixture
  local output
  local denied_name
  local denied_path
  local denied_label

  base_metadata="$(mktemp "${TMPDIR:-/tmp}/mfm-kernel-crate-dag.self-test.base.XXXXXX")"
  (
    cd "$ROOT_ABS"
    cargo metadata --no-deps --format-version 1 >"$base_metadata"
  )

  fixture="$(mktemp "${TMPDIR:-/tmp}/mfm-kernel-crate-dag.self-test.allow.XXXXXX")"
  make_dependency_fixture "$base_metadata" "mfm-ids" "crates/kernel/ids" "mfm-values" "$fixture"
  if ! check_metadata "$fixture" "" >/dev/null; then
    echo "ERROR: self-test failed: kernel-to-kernel fixture should pass" >&2
    return 1
  fi

  fixture="$(mktemp "${TMPDIR:-/tmp}/mfm-kernel-crate-dag.self-test.missing.XXXXXX")"
  jq '
    .packages |= map(select(.name != "mfm-events"))
    | .workspace_members = [.packages[].id]
  ' "$base_metadata" >"$fixture"
  if check_metadata "$fixture" "" >/dev/null 2>&1; then
    echo "ERROR: self-test failed: missing kernel crate fixture should fail" >&2
    return 1
  fi

  while IFS=$'\t' read -r denied_label denied_name denied_path; do
    fixture="$(mktemp "${TMPDIR:-/tmp}/mfm-kernel-crate-dag.self-test.denied.XXXXXX")"
    output="$(mktemp "${TMPDIR:-/tmp}/mfm-kernel-crate-dag.self-test.denied.out.XXXXXX")"
    make_dependency_fixture "$base_metadata" "$denied_name" "$denied_path" "mfm-ids" "$fixture"

    if check_metadata "$fixture" "" >"$output" 2>&1; then
      echo "ERROR: self-test failed: denied dependency unexpectedly passed label=$denied_label path=$denied_path" >&2
      cat "$output" >&2
      return 1
    fi

    if ! grep -F "path=$denied_path" "$output" >/dev/null; then
      echo "ERROR: self-test failed: denied dependency diagnostic missing path label=$denied_label path=$denied_path" >&2
      cat "$output" >&2
      return 1
    fi
  done <<'EOF'
old-machine	mfm-machine	crates/machine
old-sdk	mfm-sdk	crates/sdk
binary	mfm	bin/cli
domain	mfm-portfolio-model	crates/portfolio/model
old-op	mfm-op-proof	crates/ops/proof-op
old-state	mfm-state-common	crates/states/common
EOF

  echo "OK: kernel crate DAG denied dependency self-tests passed"
}

main() {
  local metadata_file
  local metadata_generated=0

  if [ "$RUN_SELF_TESTS" -eq 1 ]; then
    run_self_tests
  fi

  metadata_file="$(workspace_metadata_file)"
  if [ -z "$METADATA_FILE" ]; then
    metadata_generated=1
  fi

  if check_metadata "$metadata_file" "$SUMMARY_FILE"; then
    if [ "$metadata_generated" -eq 1 ]; then
      rm -f "$metadata_file"
    fi
    return 0
  fi

  if [ "$metadata_generated" -eq 1 ]; then
    rm -f "$metadata_file"
  fi
  return 1
}

main
