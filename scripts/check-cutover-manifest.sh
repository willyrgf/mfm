#!/usr/bin/env bash
set -euo pipefail

repository_root="$(git rev-parse --show-toplevel)"
manifest_path="$repository_root/SINGLE_TRUST_BOUNDARY_CUTOVER_MANIFEST.md"
pattern_file="$(mktemp)"
trap 'rm -f "$pattern_file"' EXIT

awk '
  /^```text[[:space:]]*$/ { inside = 1; next }
  inside && /^```[[:space:]]*$/ { inside = 0; next }
  inside && length($0) > 0 { print }
' "$manifest_path" >"$pattern_file"

if [[ ! -s "$pattern_file" ]]; then
  echo "cutover manifest contains no scanner patterns" >&2
  exit 2
fi

if rg -n -F -f "$pattern_file" "$repository_root" \
  -g '!SINGLE_TRUST_BOUNDARY_CUTOVER_MANIFEST.md' \
  -g '!RFC_REFACTOR_SINGLE_TRUST_BOUNDARY.md' \
  -g '!IMPL_PLAN_RFC_REFACTOR_SINGLE_TRUST_BOUNDARY.md' \
  -g '!FINISH_EVM_PORTF_REFACTOR.md' \
  -g '!REQUIRED_FIXES_FOR_RFC.md' \
  -g '!RFC_RUNTIME_STORE_JOURNAL_TYPED_PROOF.md' \
  -g '!RFC_FOLLOWUPS_FROM_REFACT_RUNTIME.md' \
  -g '!IMPL_PLAN_RFC_REFACTOR_SINGLE_TRUST_BOUNDARY.log'; then
  echo "cutover manifest scan found a superseded literal" >&2
  exit 1
fi

echo "cutover manifest scan passed"
