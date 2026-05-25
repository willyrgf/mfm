#!/usr/bin/env bash
set -euo pipefail

ROOT="."
SUMMARY_FILE=""
RUN_SELF_TESTS=0

REQUIRED_TRUE_KEYS=(
  proof_implementation_conformance_passed
  portfolio_typed_port_passed
  evm_dcv_typed_port_passed
)

REQUIRED_POSITIVE_KEYS=(
  typed_port_gate_count
)

usage() {
  cat <<'EOF'
usage: check-typed-port-gates.sh [--root PATH] [--summary-file PATH] [--self-test]

Runs enabled typed workflow-port gates for proof, portfolio, and EVM DCV and
validates the emitted summary.
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

validate_summary() {
  local summary_file="$1"
  local key
  local markers

  jq -e '.kind == "typed-port-gates-summary" and .version == 1 and (.payload | type == "object")' "$summary_file" >/dev/null

  for key in "${REQUIRED_TRUE_KEYS[@]}"; do
    if ! jq -e --arg key "$key" '.payload[$key] == true' "$summary_file" >/dev/null; then
      echo "ERROR: typed-port gate key missing or false key=$key summary=$summary_file" >&2
      exit 1
    fi
  done

  for key in "${REQUIRED_POSITIVE_KEYS[@]}"; do
    if ! jq -e --arg key "$key" '(.payload[$key] | type == "number") and .payload[$key] > 0' "$summary_file" >/dev/null; then
      echo "ERROR: typed-port gate positive count missing or zero key=$key summary=$summary_file" >&2
      exit 1
    fi
  done

  markers="$(summary_contains_skip_or_xfail "$summary_file")"
  if [ -n "$markers" ]; then
    echo "ERROR: typed-port gate summary contains skipped/xfail markers summary=$summary_file" >&2
    printf '%s\n' "$markers" >&2
    exit 1
  fi
}

write_summary() {
  local summary_file="$1"
  local summary_dir
  local summary_tmp

  summary_dir="$(dirname "$summary_file")"
  mkdir -p "$summary_dir"
  summary_tmp="$(mktemp "$summary_file.tmp.XXXXXX")"

  jq -n \
    --argjson proof_implementation_conformance_passed true \
    --argjson portfolio_typed_port_passed true \
    --argjson evm_dcv_typed_port_passed true \
    --argjson typed_port_gate_count "${#REQUIRED_TRUE_KEYS[@]}" \
    '{
      kind: "typed-port-gates-summary",
      version: 1,
      payload: {
        proof_implementation_conformance_passed: $proof_implementation_conformance_passed,
        portfolio_typed_port_passed: $portfolio_typed_port_passed,
        evm_dcv_typed_port_passed: $evm_dcv_typed_port_passed,
        typed_port_gate_count: $typed_port_gate_count
      }
    }' >"$summary_tmp"

  mv "$summary_tmp" "$summary_file"
}

run_self_tests() {
  local summary_fixture

  summary_fixture="$(mktemp "${TMPDIR:-/tmp}/mfm-typed-port-gates.self-test.XXXXXX")"
  jq -n '{
    kind: "typed-port-gates-summary",
    version: 1,
    payload: {
      proof_implementation_conformance_passed: true,
      portfolio_typed_port_passed: true,
      evm_dcv_typed_port_passed: true,
      typed_port_gate_count: 3
    }
  }' >"$summary_fixture"
  validate_summary "$summary_fixture"

  jq '.payload.evm_dcv_typed_port_passed = false' "$summary_fixture" >"$summary_fixture.false"
  if ( validate_summary "$summary_fixture.false" ) >/dev/null 2>&1; then
    echo "ERROR: self-test failed: false typed-port key should fail" >&2
    exit 1
  fi

  jq '.payload.skip_reason = "skipped"' "$summary_fixture" >"$summary_fixture.skip"
  if ( validate_summary "$summary_fixture.skip" ) >/dev/null 2>&1; then
    echo "ERROR: self-test failed: skipped marker should fail" >&2
    exit 1
  fi

  rm -f "$summary_fixture" "$summary_fixture.false" "$summary_fixture.skip"
  echo "OK: typed-port gate summary self-tests passed"
}

if [ "$RUN_SELF_TESTS" -eq 1 ]; then
  run_self_tests
fi

(
  cd "$ROOT_ABS"
  cargo test -p mfm-transports-proof deterministic_proof_implementation_conforms --lib
  cargo test -p mfm-op-portfolio-tracker -p mfm-state-portfolio -p mfm-transports-portfolio --all-targets
  cargo test -p mfm-transports-evm-dcv --lib
)

write_summary "$SUMMARY_FILE"
validate_summary "$SUMMARY_FILE"
echo "INFO: typed_port_gates_summary=$SUMMARY_FILE"
echo "OK: typed workflow port gates passed"
