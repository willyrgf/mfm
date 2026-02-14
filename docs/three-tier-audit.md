# Three-Tier Thin-Layer Alignment Audit

Date: 2026-02-14
Status: Post-refactor checkpoint; state extraction phases complete, verifier enforced in CI, final strictness/testing follow-ups pending

## 0. Architectural Model

```
bin/{cli,rest-api}   (THIN) parse input, select op/pipeline, render output
       |
       v
  crates/ops/*       (THIN) config validation + state graph wiring only
       |
       v
crates/ops/common/   (THICK) reusable single-responsibility states
  src/states/*               + shared pure utility modules
       |
       v
adapters/IO drivers  collectors, local_io, storages
```

Design rule: executable logic lives in reusable states; ops assemble state graphs; binaries stay transport-only.

## 1. Current Snapshot

| Metric | Current |
|---|---:|
| CLI command surfaces audited | 13 |
| REST routes audited | 9 |
| Registered runtime ops | 14 |
| Shared production `State` impls (`ops/common/src/states`) | 18 |
| Op-local production `State` impls (`ops/*/src/lib.rs`) | 2 |
| Total production `State` impls | 20 |
| Shared state ratio (production) | 90% |
| Target shared ratio | 80%+ |

Notes:
- Shared ratio now exceeds target.
- The two intentional op-local states are domain-specific output aggregators.

## 2. Tier Conformance

### 2.1 Tier 1: Binary Layer (Thin)

- REST API: aligned (wrapper-only handlers).
- CLI: aligned for thin-layer boundary.
  - `portfolio snapshot` token JSON parsing moved into app-layer helper.
  - `run pipeline deploy-configure-validate` spec input parsing moved into app-layer helper.
  - Remaining CLI JSON parsing for generic pipeline payloads is transport-level request decoding.

### 2.2 Tier 2: Ops Layer (Thin)

| Op | Thinness | Notes |
|---|---|---|
| `keystore_import` | Thin | Wires shared keystore admin state |
| `keystore_list` | Thin | Wires shared keystore admin state |
| `keystore_delete` | Thin | Wires shared keystore admin state |
| `keystore_tx_sign` | Thin | Wires shared `KeystoreTxSignState` |
| `keystore_tx_send_raw` | Thin | Wires shared `KeystoreTxSendRawState` |
| `evm_read` | Thin | Wires shared EVM read states |
| `evm_contract_from_nix` | Thin | Wires shared `NixArtifactToEvmContractState` |
| `evm_deploy` | Thin | Wires shared `EvmDeployState` |
| `evm_configure` | Thin | Wires shared `EvmConfigureState` |
| `evm_validate` | Thin | Wires shared `EvmValidateState` |
| `evm_deploy_configure_validate` | Thin | Composes child ops only |
| `nix_app` | Thin | Wires shared `NixExecState` |
| `portfolio_tracker` | Mixed | Keeps domain-specific `WriteSnapshotState` |
| `proof` | Mixed | Keeps domain-specific `WriteOutputState` |

Summary: 12/14 Thin, 2/14 Mixed, 0/14 Thick.

### 2.3 Tier 3: State Layer (Thick + Reusable)

Shared state modules now include:
- `evm.rs`
- `keystore_admin.rs`
- `keystore_tx.rs`
- `evm_write.rs`
- `io.rs`
- `side_effect.rs`
- `nix.rs`
- `meta.rs` (state metadata helpers)
- `evm_dcv.rs` (shared DCV config/validation helpers)

Domain-local production states intentionally retained:
- `portfolio_tracker::WriteSnapshotState`
- `proof::WriteOutputState`

## 3. Utility Consolidation (`B8`)

Canonical shared utility modules are in place:
- `crates/ops/common/src/hex.rs`
- `crates/ops/common/src/rlp.rs`
- `crates/ops/common/src/abi.rs`
- `crates/ops/common/src/evm_encoding.rs`
- `crates/ops/common/src/util_error.rs`

Current duplication ceilings enforced by `mfm-architecture-verify`:
- `normalize_hex_str` <= 3
- `hex_to_bytes` <= 3
- `bytes_to_hex_prefixed` <= 3
- `parse_abi` <= 3
- `encode_params` <= 2
- `parse_value_wei_to_hex` <= 2
- `trim_leading_zero_bytes` <= 1
- `u128_to_min_be` <= 1
- `rlp_encode_bytes` <= 1
- `rlp_encode_list` <= 1
- `usize_to_min_be` <= 1

Interpretation:
- RLP duplication is now single-source.
- Remaining duplication is mostly wrapper-layer compatibility around shared helpers.
- Final strict-mode target remains single-source for hex/ABI paths as well.

## 4. Guardrails and CI

`mfm-architecture-verify` is integrated into:
- `nix run .#check`
- CI basic mode (`nixfied/project/ci.nix` step: `architecture-verify`)

Checks enforced:
1. Expand boundary: forbids `.await` and ambient IO APIs inside op `expand()`.
2. Utility duplication ceilings: prevents regression above configured limits.
3. Shared-state ratio: now enforced at 80% minimum.

Bugfix included in verifier:
- State ratio counting now strips `#[cfg(test)]` items precisely, instead of truncating files at the first `#[cfg(test)]` occurrence.

## 5. Findings (Current)

### High

1. `B8` strict single-source utility goal is not fully complete.
- Impact: wrapper duplications remain for select hex/ABI helpers.
- Risk: minor maintenance drift risk compared to fully canonical call sites.

### Medium

2. Shared test harness duplication remains.
- `NoopRecorder` and `CountingTransport` are still duplicated in multiple test modules.
- `MapContext` still has duplicate implementations outside shared test support.

3. Full parity signoff remains a release gate item.
- `#check` and `#test` pass.
- `#ci -- --parity --summary` should be run for architecture checkpoint closure.

### Low

4. Two ops remain mixed by design (`portfolio_tracker`, `proof`) due domain-specific output assembly states.

## 6. Remaining Plan

1. Tighten utility duplication policy from ceiling mode to strict single-source mode (hex/ABI wrappers).
2. Consolidate duplicated test doubles into shared test support.
3. Run and archive parity CI summary (`nix run .#ci -- --parity --summary`).
4. Update architecture-facing docs (`ARCHITECTURE.md`, `ENFORCE_BINS_THIN_LAYER.md`) if needed to reflect new shared state modules.

## 7. Acceptance Criteria Status

1. No business workflow logic in binaries: **Met**.
2. Op `expand()` methods are config+graph only: **Met**.
3. Shared state ratio >= 80%: **Met** (90%).
4. Op-local states only for domain-specific aggregation/output: **Met**.
5. Reusable patterns extracted as shared states: **Met** for keystore-tx, evm-write, proof patterns, nix exec.
6. Side effects routed through IO abstraction: **Met**.
7. Architecture guard in CI: **Met**.
8. `check`/`test` parity gates passing: **Met** for `#check` and `#test`; full parity mode still pending in this checkpoint.
9. Utility single-source (strict): **Partially met** (RLP complete, hex/ABI wrappers remain).
10. Shared test harness for state tests: **Partially met**.
11. Utility test coverage for consolidated modules: **Partially met**.
12. Execute tests for every shared state: **Partially met**.
