#!/usr/bin/env bash
set -euo pipefail

repository_root="$(git rev-parse --show-toplevel)"
manifest_path="$repository_root/RUNTIME_STORE_JOURNAL_CUTOVER_MANIFEST.md"
canary_path=""
canary_root=""

cleanup_canary() {
  if [[ -n "$canary_path" ]] && [[ -e "$canary_path" || -L "$canary_path" ]]; then
    rm -f -- "$canary_path"
  fi
  if [[ -n "$canary_root" && -d "$canary_root" ]]; then
    rm -rf -- "$canary_root"
  fi
  canary_path=""
  canary_root=""
}
trap cleanup_canary EXIT

manifest_rules() {
  awk '
    /^```cutover-rules$/ { inside = 1; next }
    inside && /^```$/ { exit }
    inside && NF { print }
  ' "$manifest_path"
}

ledger_digest() {
  awk '
    /^```text$/ { inside = 1; next }
    inside && /^```$/ { exit }
    inside { print }
  ' "$manifest_path" | sha256sum | cut -d ' ' -f 1
}

ruleset_digest() {
  manifest_rules | awk -F '\t' '$1 != "ruleset" { print }' | sha256sum | cut -d ' ' -f 1
}

manifest_coverage() {
  awk '
    /^```cutover-coverage$/ { inside = 1; next }
    inside && /^```$/ { exit }
    inside && NF { print }
  ' "$manifest_path"
}

coverage_digest() {
  manifest_coverage | sha256sum | cut -d ' ' -f 1
}

ledger_entry_count() {
  awk '
    /^```text$/ { inside = 1; next }
    inside && /^```$/ { exit }
    inside && NF { count += 1 }
    END { print count + 0 }
  ' "$manifest_path"
}

scan_regex() {
  local scope="$1"
  local pattern="$2"
  local -a targets=()
  local -a globs=()
  local -a modes=(-P -U)
  case "$scope" in
    cargo)
      targets=("$repository_root/Cargo.toml" "$repository_root/Cargo.lock" "$repository_root/crates" "$repository_root/bin")
      globs=(-g 'Cargo.toml')
      ;;
    production)
      targets=("$repository_root/crates" "$repository_root/bin")
      globs=(-g '**/src/**/*.rs')
      ;;
    schema)
      targets=("$repository_root/crates" "$repository_root/bin" "$repository_root/.github" "$repository_root/.config" "$repository_root/nix" "$repository_root/flake.nix" "$repository_root/nixfied.nix" "$repository_root/scripts")
      globs=(-g '**/src/**/*.rs' -g '**/migrations/*.sql' -g 'build.rs' -g '*.nix' -g '*.yml' -g '*.yaml' -g '*.toml' -g '*.sh' -g '!check-cutover-manifest.sh')
      ;;
    sql)
      targets=("$repository_root/crates")
      globs=(-g '**/migrations/*.sql' -g 'build.rs')
      ;;
    task)
      targets=("$repository_root/flake.nix" "$repository_root/nixfied.nix" "$repository_root/.github" "$repository_root/.config" "$repository_root/nix" "$repository_root/scripts")
      globs=(-g '*.nix' -g '*.yml' -g '*.yaml' -g '*.toml' -g '*.sh' -g '!check-cutover-manifest.sh')
      ;;
    docs)
      targets=("$repository_root/README.md" "$repository_root/AGENTS.md" "$repository_root/docs" "$repository_root/crates" "$repository_root/bin")
      globs=(-g '*.md' -g '!code-quality.md')
      modes=(-i -P -U)
      ;;
    *)
      targets=("$repository_root/$scope")
      ;;
  esac

  local status=0
  rg -n --hidden "${modes[@]}" "$pattern" "${targets[@]}" "${globs[@]}" || status=$?
  if [[ "$status" -eq 0 ]]; then
    return 0
  fi
  if [[ "$status" -gt 1 ]]; then
    echo "cutover scan error: scope=$scope pattern=$pattern" >&2
    return 0
  fi
  return 1
}

scan_current_tree() {
  local failures=0
  local kind scope pattern
  local count=0
  local expected_count=""
  while IFS=$'\t' read -r kind scope pattern; do
    [[ -n "$kind" && -n "$pattern" ]] || continue
    count=$((count + 1))
    case "$kind" in
      path)
        if [[ -e "$repository_root/$pattern" || -L "$repository_root/$pattern" ]]; then
          echo "$pattern" >&2
          echo "cutover scan failed: deleted path" >&2
          failures=1
        fi
        ;;
      regex)
        if scan_regex "$scope" "$pattern"; then
          echo "cutover scan failed: scope=$scope pattern=$pattern" >&2
          failures=1
        fi
        ;;
      ledger)
        if [[ "$pattern" != "$(ledger_digest)" ]]; then
          echo "cutover scan failed: complete deletion ledger fingerprint changed" >&2
          failures=1
        fi
        ;;
      ruleset)
        if [[ "$pattern" != "$(ruleset_digest)" ]]; then
          echo "cutover scan failed: machine rule contents changed without review" >&2
          failures=1
        fi
        ;;
      coverage)
        if [[ "$pattern" != "$(coverage_digest)" ]]; then
          echo "cutover scan failed: ledger coverage map changed without review" >&2
          failures=1
        fi
        ;;
      inventory)
        expected_count="$pattern"
        ;;
      *)
        echo "cutover scan failed: unknown manifest rule kind '$kind'" >&2
        failures=1
        ;;
    esac
  done < <(manifest_rules)
  if [[ -z "$expected_count" || "$count" -ne "$expected_count" ]]; then
    echo "cutover scan failed: machine rule inventory is not exact" >&2
    failures=1
  fi
  local coverage_count=0
  local ordinal rule_id rule_number
  while IFS=$'\t' read -r ordinal rule_id; do
    [[ -n "$ordinal" && -n "$rule_id" ]] || continue
    coverage_count=$((coverage_count + 1))
    if [[ "$ordinal" != "$coverage_count" || ! "$rule_id" =~ ^R[0-9]{3}$ ]]; then
      echo "cutover scan failed: malformed ledger coverage row $coverage_count" >&2
      failures=1
      continue
    fi
    rule_number=$((10#${rule_id#R}))
    if [[ "$rule_number" -lt 1 || "$rule_number" -gt "$count" ]]; then
      echo "cutover scan failed: coverage references missing rule $rule_id" >&2
      failures=1
    fi
  done < <(manifest_coverage)
  if [[ "$coverage_count" -ne "$(ledger_entry_count)" ]]; then
    echo "cutover scan failed: deletion ledger is not fully mapped to machine rules" >&2
    failures=1
  fi
  [[ "$failures" -eq 0 ]]
}

run_canary() {
  local label="$1"
  cleanup_canary
  canary_root="$(mktemp -d "$repository_root/crates/.cutover-scan-canary.XXXXXX")"
  case "$label" in
    path)
      canary_path="$repository_root/docs/btc-rpc-routing.md"
      ln -s "$canary_root/missing-target" "$canary_path"
      ;;
    package)
      printf '%s\n' '[package]' 'name = "mfm-facts"' >"$canary_root/Cargo.toml"
      ;;
    dependency)
      printf '%s\n' '[dependencies]' 'mfm-replay = { path = "../replay" }' >"$canary_root/Cargo.toml"
      ;;
    package-compact)
      printf '%s\n' '[package]' 'name="mfm-facts"' >"$canary_root/Cargo.toml"
      ;;
    package-single-quoted)
      printf '%s\n' '[package]' "name = 'mfm-replay'" >"$canary_root/Cargo.toml"
      ;;
    dependency-quoted)
      printf '%s\n' '[dependencies]' '"mfm-replay" = { path = "../replay" }' >"$canary_root/Cargo.toml"
      ;;
    dependency-renamed)
      printf '%s\n' '[dependencies]' 'old_replay = { package = "mfm-replay", path = "../replay" }' >"$canary_root/Cargo.toml"
      ;;
    dependency-dotted)
      printf '%s\n' '[dependencies.mfm-replay]' 'path = "../replay"' >"$canary_root/Cargo.toml"
      ;;
    dependency-dotted-quoted)
      printf '%s\n' '[dependencies."mfm-facts"]' 'path = "../facts"' >"$canary_root/Cargo.toml"
      ;;
    dependency-dotted-dev)
      printf '%s\n' '[dev-dependencies.mfm-replay]' 'path = "../replay"' >"$canary_root/Cargo.toml"
      ;;
    dependency-dotted-target)
      printf '%s\n' '[target.'"'"'cfg(unix)'"'"'.dependencies.mfm-replay]' 'path = "../replay"' >"$canary_root/Cargo.toml"
      ;;
    feature-double-quoted)
      printf '%s\n' '[features]' '"test-support" = []' >"$canary_root/Cargo.toml"
      ;;
    feature-single-quoted)
      printf '%s\n' '[features]' "'parity-tests'=[]" >"$canary_root/Cargo.toml"
      ;;
    export)
      mkdir -p "$canary_root/src"
      printf '%s\n' 'pub struct SuspendedRun;' >"$canary_root/src/lib.rs"
      ;;
    private)
      mkdir -p "$canary_root/src"
      printf '%s\n' 'struct SuspendedRun;' >"$canary_root/src/lib.rs"
      ;;
    hot-value)
      mkdir -p "$canary_root/src"
      printf '%s\n' 'pub struct HotValue;' >"$canary_root/src/lib.rs"
      ;;
    execution-key)
      mkdir -p "$canary_root/src"
      printf '%s\n' 'pub struct StateExecutionKey;' >"$canary_root/src/lib.rs"
      ;;
    reexport-direct)
      mkdir -p "$canary_root/src"
      printf '%s\n' 'pub use old::RunSession;' >"$canary_root/src/lib.rs"
      ;;
    reexport-braced)
      mkdir -p "$canary_root/src"
      printf '%s\n' 'pub use old::{RunFrame, SuspendedRun};' >"$canary_root/src/lib.rs"
      ;;
    reexport-multiline)
      mkdir -p "$canary_root/src"
      printf '%s\n' 'pub use old::{' '    EvmAdapterError,' '    RunFrame,' '};' >"$canary_root/src/lib.rs"
      ;;
    conflict)
      mkdir -p "$canary_root/src"
      printf '%s\n' 'pub enum RuntimeError {' '    Conflict,' '}' >"$canary_root/src/lib.rs"
      ;;
    function)
      mkdir -p "$canary_root/src"
      printf '%s\n' 'fn application_catalog() {}' 'pub fn canonical_value() {}' >"$canary_root/src/lib.rs"
      ;;
    limit-field)
      mkdir -p "$canary_root/src"
      printf '%s\n' 'struct Limits { max_active_sessions: usize }' >"$canary_root/src/lib.rs"
      ;;
    capability)
      mkdir -p "$canary_root/src"
      printf '%s\n' 'impl ReadCapabilityContract for EvmCapability<3> {}' >"$canary_root/src/lib.rs"
      ;;
    wire)
      mkdir -p "$canary_root/src"
      printf '%s\n' 'const TAG: &str = "state_prepared";' >"$canary_root/src/lib.rs"
      ;;
    sql)
      mkdir -p "$canary_root/migrations"
      printf '%s\n' 'CREATE TABLE run_reservations (object_count BIGINT);' >"$canary_root/migrations/canary.sql"
      ;;
    sql-lowercase)
      mkdir -p "$canary_root/migrations"
      printf '%s\n' 'create table run_reservations (id bigint);' >"$canary_root/migrations/canary.sql"
      ;;
    sql-uppercase-object)
      mkdir -p "$canary_root/migrations"
      printf '%s\n' 'CREATE TABLE RUN_RESERVATIONS (id bigint);' >"$canary_root/migrations/canary.sql"
      ;;
    sql-uppercase-column)
      mkdir -p "$canary_root/migrations"
      printf '%s\n' 'CREATE TABLE unrelated (OBJECT_COUNT bigint);' >"$canary_root/migrations/canary.sql"
      ;;
    docs)
      canary_path="$canary_root/canary.md"
      printf '%s\n' 'Portable replay is supported.' >"$canary_path"
      ;;
    docs-journal)
      canary_path="$canary_root/canary.md"
      printf '%s\n' 'Journal writes state_prepared records.' >"$canary_path"
      ;;
    docs-app)
      canary_path="$canary_root/canary.md"
      printf '%s\n' 'Application owns a suspended run map and derives status from frames.' >"$canary_path"
      ;;
    effect-identities)
      mkdir -p "$canary_root/src"
      printf '%s\n' 'pub enum EffectKindKind {}' 'pub enum EffectVersionKind {}' >"$canary_root/src/lib.rs"
      ;;
    store-readiness)
      mkdir -p "$canary_root/src"
      printf '%s\n' 'pub trait Store { fn check_ready(&self); }' >"$canary_root/src/lib.rs"
      ;;
    address-fields)
      mkdir -p "$canary_root/src"
      printf '%s\n' 'struct StateDeclaration { address: u16, entry_address: u16 }' >"$canary_root/src/lib.rs"
      ;;
    occurrence-field)
      mkdir -p "$canary_root/src"
      printf '%s\n' 'struct StateRecord { occurrence: u16 }' >"$canary_root/src/lib.rs"
      ;;
    reservation-instruction)
      mkdir -p "$canary_root/src"
      printf '%s\n' 'pub enum ReservationInstruction { Open, Replace, Consume }' >"$canary_root/src/lib.rs"
      ;;
    replay-reducer)
      mkdir -p "$canary_root/src"
      printf '%s\n' 'struct ReplayReducer;' >"$canary_root/src/lib.rs"
      ;;
    wallet-domain)
      mkdir -p "$canary_root/src"
      printf '%s\n' 'const wallet_nonce_effect_domain: &str = "wallet";' >"$canary_root/src/lib.rs"
      ;;
    submission-status)
      mkdir -p "$canary_root/src"
      printf '%s\n' 'pub enum ReadCapabilityFamily { SubmissionStatus }' >"$canary_root/src/lib.rs"
      ;;
    async-ownership)
      mkdir -p "$canary_root/src"
      printf '%s\n' 'struct AcknowledgementLease;' 'struct RuntimeCancellationToken;' 'struct RuntimeTimeoutPolicy;' 'fn detached_completion_finalizer() {}' >"$canary_root/src/lib.rs"
      ;;
    schema-kind-variants)
      mkdir -p "$canary_root/src"
      printf '%s\n' 'pub enum SchemaKind { StateInput, OperationOutput, PublicOutput }' >"$canary_root/src/lib.rs"
      ;;
    terminal-field)
      mkdir -p "$canary_root/src"
      printf '%s\n' 'struct StateDeclaration { terminal: bool }' >"$canary_root/src/lib.rs"
      ;;
    completion-cell)
      mkdir -p "$canary_root/src"
      printf '%s\n' 'struct CompletionCell;' >"$canary_root/src/lib.rs"
      ;;
    finalization-cell)
      mkdir -p "$canary_root/src"
      printf '%s\n' 'struct FinalizationCell;' >"$canary_root/src/lib.rs"
      ;;
    pending-owner-permit)
      mkdir -p "$canary_root/src"
      printf '%s\n' 'struct PendingOwnerPermit;' >"$canary_root/src/lib.rs"
      ;;
    resolver-token)
      mkdir -p "$canary_root/src"
      printf '%s\n' 'struct ResolverToken;' >"$canary_root/src/lib.rs"
      ;;
    access-binding)
      mkdir -p "$canary_root/src"
      printf '%s\n' 'pub struct AccessBindingV2;' >"$canary_root/src/lib.rs"
      ;;
    run-admitted)
      mkdir -p "$canary_root/src"
      printf '%s\n' 'pub struct RunAdmitted;' >"$canary_root/src/lib.rs"
      ;;
    postgres-wallet-nonce)
      mkdir -p "$canary_root/src"
      printf '%s\n' 'pub struct PostgresWalletNonceStore;' >"$canary_root/src/lib.rs"
      ;;
    enum-whitespace)
      mkdir -p "$canary_root/src"
      printf '%s\n' 'pub' 'enum AppendResult {' '    Existing,' '}' >"$canary_root/src/lib.rs"
      ;;
    capability-owned-direct)
      cleanup_canary
      canary_root="$(mktemp -d "$repository_root/crates/kernel/capabilities/src/.cutover-scan-canary.XXXXXX")"
      printf '%s\n' 'pub enum ProposedStateOutcome {}' >"$canary_root/canary.rs"
      ;;
    capability-owned-reexport)
      cleanup_canary
      canary_root="$(mktemp -d "$repository_root/crates/kernel/capabilities/src/.cutover-scan-canary.XXXXXX")"
      printf '%s\n' 'pub use outcomes::{' '    ProposedStateOutcome,' '    Other,' '};' >"$canary_root/canary.rs"
      ;;
    store-owned-reexport)
      cleanup_canary
      canary_root="$(mktemp -d "$repository_root/crates/kernel/store/src/.cutover-scan-canary.XXXXXX")"
      printf '%s\n' 'pub use postgres::StoreOpenError;' >"$canary_root/canary.rs"
      ;;
    live-owned-alias)
      cleanup_canary
      canary_root="$(mktemp -d "$repository_root/crates/live/evm/src/.cutover-scan-canary.XXXXXX")"
      printf '%s\n' 'pub type EvmPhysicalTarget = ();' >"$canary_root/canary.rs"
      ;;
    live-owned-reexport)
      cleanup_canary
      canary_root="$(mktemp -d "$repository_root/crates/live/evm/src/.cutover-scan-canary.XXXXXX")"
      printf '%s\n' 'pub use mfm_evm::EvmPhysicalTarget;' >"$canary_root/canary.rs"
      ;;
    acknowledgement-field)
      mkdir -p "$canary_root/src"
      printf '%s\n' 'struct Pending { acknowledgement_lease: u64 }' >"$canary_root/src/lib.rs"
      ;;
    completion-field)
      mkdir -p "$canary_root/src"
      printf '%s\n' 'struct Pending { completion_cell: u64 }' >"$canary_root/src/lib.rs"
      ;;
    finalization-field)
      mkdir -p "$canary_root/src"
      printf '%s\n' 'struct Pending { finalization_cell: u64 }' >"$canary_root/src/lib.rs"
      ;;
    pending-owner-limit-field)
      mkdir -p "$canary_root/src"
      printf '%s\n' 'struct Pending { pending_owner_limit: u64 }' >"$canary_root/src/lib.rs"
      ;;
    pending-owner-permit-field)
      mkdir -p "$canary_root/src"
      printf '%s\n' 'struct Pending { pending_owner_permit: u64 }' >"$canary_root/src/lib.rs"
      ;;
    resolver-field)
      mkdir -p "$canary_root/src"
      printf '%s\n' 'struct Pending { resolver_token: u64 }' >"$canary_root/src/lib.rs"
      ;;
    cancellation-field)
      mkdir -p "$canary_root/src"
      printf '%s\n' 'struct Pending { cancellation_token: u64 }' >"$canary_root/src/lib.rs"
      ;;
    timeout-field)
      mkdir -p "$canary_root/src"
      printf '%s\n' 'struct Pending { timeout_policy: u64 }' >"$canary_root/src/lib.rs"
      ;;
    read-replacement)
      mkdir -p "$canary_root/src"
      printf '%s\n' 'struct ReadAttempt { preparation_ordinal: u16, replaces: u16 }' >"$canary_root/src/lib.rs"
      ;;
    state-input-derive)
      mkdir -p "$canary_root/src"
      printf '%s\n' '#[proc_macro_derive(StateInput)]' 'pub fn derive_state_input() {}' >"$canary_root/src/lib.rs"
      ;;
    operation-output-derive)
      mkdir -p "$canary_root/src"
      printf '%s\n' '#[proc_macro_derive(OperationOutput)]' 'pub fn derive_operation_output() {}' >"$canary_root/src/lib.rs"
      ;;
    public-outputs-derive)
      mkdir -p "$canary_root/src"
      printf '%s\n' '#[proc_macro_derive(PublicOutputs)]' 'pub fn derive_public_outputs() {}' >"$canary_root/src/lib.rs"
      ;;
    *)
      echo "unknown cutover scanner canary: $label" >&2
      exit 2
      ;;
  esac
  if scan_current_tree >/dev/null 2>&1; then
    echo "cutover scanner self-test failed: $label canary was accepted" >&2
    exit 2
  fi
}

if [[ ! -s "$manifest_path" ]] || [[ -z "$(manifest_rules)" ]]; then
  echo "cutover manifest is missing or has no complete machine rule inventory" >&2
  exit 2
fi

# Refuse to run destructive canary cleanup until the real deleted paths are known absent.
if ! scan_current_tree; then
  exit 1
fi

for canary in path package dependency package-compact package-single-quoted dependency-quoted dependency-renamed dependency-dotted dependency-dotted-quoted dependency-dotted-dev dependency-dotted-target feature-double-quoted feature-single-quoted export private hot-value execution-key reexport-direct reexport-braced reexport-multiline conflict function limit-field capability wire sql sql-lowercase sql-uppercase-object sql-uppercase-column docs docs-journal docs-app effect-identities store-readiness address-fields occurrence-field reservation-instruction replay-reducer wallet-domain submission-status async-ownership schema-kind-variants terminal-field completion-cell finalization-cell pending-owner-permit resolver-token access-binding run-admitted postgres-wallet-nonce enum-whitespace capability-owned-direct capability-owned-reexport store-owned-reexport live-owned-alias live-owned-reexport acknowledgement-field completion-field finalization-field pending-owner-limit-field pending-owner-permit-field resolver-field cancellation-field timeout-field read-replacement state-input-derive operation-output-derive public-outputs-derive; do
  run_canary "$canary"
done
cleanup_canary

if ! scan_current_tree; then
  exit 1
fi

echo "cutover manifest scan and scoped canary tests passed"
