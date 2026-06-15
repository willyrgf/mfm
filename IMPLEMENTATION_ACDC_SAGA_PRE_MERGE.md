# ACDC/SAGA Pre-Merge Implementation Log

## Baseline

Validation inventory command:

```bash
rg 'fn valid|validate_|Validated|Certified|Verified|Authority|Projection|History|Ledger' crates/kernel -n
```

Count by kernel crate at start of implementation:

| Crate | Matches |
|---|---:|
| `crates/kernel/canonical` | 22 |
| `crates/kernel/capabilities` | 193 |
| `crates/kernel/certify` | 292 |
| `crates/kernel/effects` | 0 |
| `crates/kernel/events` | 94 |
| `crates/kernel/ids` | 10 |
| `crates/kernel/manual-auth` | 18 |
| `crates/kernel/program` | 33 |
| `crates/kernel/program-derive` | 5 |
| `crates/kernel/replay` | 122 |
| `crates/kernel/runtime` | 654 |
| `crates/kernel/spec` | 34 |
| `crates/kernel/store` | 520 |
| `crates/kernel/values` | 4 |

## Phase 1: Store Side-Effect Ambiguity Pairing

Status: completed.

Files changed:

- `crates/kernel/store/src/lib.rs`
- `crates/kernel/store/tests/commit_contract.rs`

Validators deleted or replaced:

- Replaced `validate_side_effect_attempt_failure_pairs` with `validate_terminal_side_effect_evidence_pairs`.
- Added a store-owned `TerminalSideEffectEvidencePair` helper so `SideEffectFailed` and `SideEffectAmbiguous` share the same terminal evidence pairing rule.
- Extended committed-batch construction to validate terminal side-effect pairing as well as append staging.

Tests added or updated:

- Added rejection for `SideEffectFailed` without matching same-commit `StateAttemptFailed`.
- Added acceptance for `SideEffectFailed` with matching same-commit `StateAttemptFailed`.
- Added rejection for failed terminal evidence with mismatched node/attempt.
- Added rejection for `SideEffectAmbiguous` without matching same-commit `StateAttemptFailed`.
- Added acceptance for `SideEffectAmbiguous` with matching non-retryable same-commit `StateAttemptFailed`.
- Added rejection for `SideEffectAmbiguous` paired with retryable attempt failure.
- Added rejection for ambiguous terminal evidence with mismatched node/attempt.
- Updated the resource-lane ambiguity fixture to commit the required same-commit non-retryable attempt failure.

Checks run:

- `cargo test -p mfm-store --test commit_contract side_effect_phase_order_and_fencing_are_enforced`
- `cargo fmt --all -- --check`
- `cargo check -p mfm-store`
- `cargo test -p mfm-store`
- `cargo test -p mfm-integration-tests --test cargo_metadata_contract`
- `cargo test -p mfm-integration-tests --test architecture_namespace_contract`
