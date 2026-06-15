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

## Phase 2: Event Accessors And Artifact Role Views

Status: completed.

Files changed:

- `crates/kernel/events/src/lib.rs`
- `crates/kernel/store/src/lib.rs`
- `crates/kernel/store/src/v1/artifact_refs.rs`
- `crates/kernel/runtime/src/side_effects.rs`
- `crates/kernel/runtime/src/history.rs`
- `crates/kernel/runtime/src/tests.rs`
- `IMPLEMENTATION_ACDC_SAGA_PRE_MERGE.md`

Validators deleted or replaced:

- Moved artifact requirement extraction from `mfm-store` into event-owned `KernelEventPayload::artifact_requirements`.
- Replaced store-local artifact requirement/source DTOs with re-exports of the event-owned view.
- Replaced store-local run id and spec hash match ladders with `KernelEventPayload::{run_id,spec_hash}`.
- Replaced runtime side-effect payload identity and ledger-purpose match ladders with `KernelEventPayload::side_effect_ref`.
- Replaced runtime local spec hash helper body with `KernelEventPayload::spec_hash`.

Tests added or updated:

- Added event accessor tests for run id, spec hash, and side-effect refs.
- Added event artifact requirement coverage for every artifact-bearing payload variant.
- Kept event schema golden tests unchanged to prove serialized event schema/hash output did not change.
- Updated a runtime synthetic ambiguity fixture to use the legal store-admitted terminal batch with same-commit non-retryable `StateAttemptFailed`.

Checks run:

- `cargo fmt --all -- --check`
- `cargo check -p mfm-events -p mfm-store -p mfm-runtime -p mfm-replay`
- `cargo test -p mfm-events`
- `cargo test -p mfm-store`
- `cargo test -p mfm-runtime`
- `cargo test -p mfm-replay`
- `cargo test -p mfm-integration-tests --test cargo_metadata_contract`
- `cargo test -p mfm-integration-tests --test architecture_namespace_contract`

## Phase 3: Raw, Lowered, Validated, Certified Spec Split

Status: completed.

Files changed:

- `Cargo.lock`
- `crates/kernel/spec/src/lib.rs`
- `crates/kernel/certify/Cargo.toml`
- `crates/kernel/certify/src/lib.rs`
- `crates/kernel/certify/tests/certify_authority_ui.rs`
- `crates/kernel/certify/tests/ui/fail/certified_spec_fields_private.rs`
- `crates/kernel/certify/tests/ui/fail/lowered_spec_is_read_only.rs`
- `crates/kernel/certify/tests/ui/fail/raw_typed_spec_not_certification_input.rs`
- `crates/kernel/certify/tests/ui/fail/validated_spec_fields_private.rs`
- `crates/app/src/lib.rs`
- `IMPLEMENTATION_ACDC_SAGA_PRE_MERGE.md`

Validators deleted or replaced:

- Added `spec::UntrustedTypedSpec` as the hostile parsed/constructed spec wrapper.
- Added `LoweredTypedSpec` for deterministic program-lowered specs that are not runtime authority.
- Added `ValidatedTypedExecutionSpec` with private fields and private constructor.
- Changed `certify_typed_spec` so plain `TypedExecutionSpec` no longer satisfies the public certification input type.
- Changed `CertifiedTypedSpec` to contain validated authority and removed the raw `CertifiedTypedSpec::spec()` shortcut.
- Changed parsed certified bundles to carry `UntrustedTypedSpec` until verifier-backed certification succeeds.

Tests added or updated:

- Added certifier compile-fail tests proving callers cannot pass raw typed specs to certification.
- Added certifier compile-fail tests proving callers cannot literal-construct validated or certified authority.
- Added certifier compile-fail test proving lowered specs expose read-only data and cannot be mutated into authority.
- Updated certifier mutation tests to wrap hostile raw specs explicitly before certification.
- Updated app invalid-bundle test to read through `validated_spec()`.

Checks run:

- `cargo check -p mfm-certify`
- `cargo test -p mfm-certify --test certify_authority_ui`
- `cargo check -p mfm-spec -p mfm-certify -p mfm-runtime -p mfm-app`
- `cargo fmt --all -- --check`
- `cargo test -p mfm-spec`
- `cargo test -p mfm-certify`
- `cargo test -p mfm-app`
- `cargo test -p mfm-runtime`
- `cargo test -p mfm-integration-tests --test cargo_metadata_contract`
- `cargo test -p mfm-integration-tests --test architecture_namespace_contract`
