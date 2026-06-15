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

## Phase 4: Descriptor Ref Migration And Spec Hash/Golden Updates

Status: completed.

Files changed:

- `crates/kernel/spec/src/lib.rs`
- `crates/kernel/certify/src/lib.rs`
- `crates/kernel/runtime/src/tests.rs`
- `IMPLEMENTATION_ACDC_SAGA_PRE_MERGE.md`

Validators deleted or replaced:

- Added spec-owned `DescriptorFamily` and `DescriptorRef`.
- Added spec-owned descriptor reference/fingerprint APIs on `DescriptorIdentity`.
- Changed persisted node JSON to store `descriptor_ref` instead of repeated `descriptor_id`, `state_kind`, `state_version`, `effect_kind`, and `capability_bindings`.
- Changed persisted operation lineage and public-output renderer surfaces to use descriptor refs.
- Changed persisted spec parsing to verify every descriptor ref against the descriptor table before rebuilding in-memory DTO fields.
- Replaced certifier descriptor evidence hashing with `DescriptorIdentity::descriptor_ref`.
- Replaced certifier registry digest descriptor payloads with descriptor-ref payloads.
- Deleted certifier-local full descriptor identity JSON helpers used for descriptor fingerprinting.

Tests added or updated:

- Added spec parser regression for descriptor ref digest/family mismatches.
- Updated public-output digest, spec hash, and certificate hash goldens for the descriptor-ref shape.
- Updated a runtime malformed lifecycle fixture to remove stale descriptor table entries before appending replacement lifecycle nodes.

Checks run:

- `cargo check -p mfm-spec`
- `cargo test -p mfm-spec`
- `cargo check -p mfm-spec -p mfm-certify`
- `cargo test -p mfm-certify`
- `cargo check -p mfm-runtime -p mfm-app`
- `cargo test -p mfm-runtime`
- `cargo test -p mfm-app`
- `cargo fmt --all -- --check`
- `cargo check -p mfm-spec -p mfm-certify -p mfm-runtime -p mfm-app`
- `cargo test -p mfm-integration-tests --test cargo_metadata_contract`
- `cargo test -p mfm-integration-tests --test architecture_namespace_contract`

## Phase 5: Certified Descriptor Set And Framework Lifecycle Authority

Status: completed.

Files changed:

- `crates/kernel/certify/src/lib.rs`
- `crates/kernel/certify/tests/ui/fail/certified_spec_fields_private.stderr`
- `crates/kernel/runtime/src/spec_authority.rs`
- `crates/kernel/runtime/src/tests.rs`
- `IMPLEMENTATION_ACDC_SAGA_PRE_MERGE.md`

Validators deleted or replaced:

- Added `CertifiedDescriptorSet` with private fields and certifier-only construction from validated spec authority.
- Added `CertifiedFrameworkLifecycle` and `CertifiedFrameworkNodeRole` with private fields and certifier-only construction.
- Changed `CertifiedTypedSpec` to carry certified descriptor and framework lifecycle views.
- Changed `CertifiedTypedSpec::into_parts` to transfer envelope, certificate, descriptor set, and lifecycle authority as one runtime construction object.
- Changed production `CertifiedRuntimeSpec::new` to consume descriptor/lifecycle authority from `CertifiedTypedSpec`.
- Deleted runtime-side duplicate static validation for descriptor/node contracts, public-output render shape, lifecycle node roles, lifecycle receipt consumers, lifecycle config refs, framework output contracts, and lifecycle tail finality.
- Kept runtime checks focused on hash verification, duplicate index construction, missing/cyclic predecessor detection, runner/capability availability, stream/history agreement, and state advancement.

Tests added or updated:

- Added certifier test proving certification mints descriptor set and lifecycle role views.
- Updated certifier compile-fail fixture for the expanded private `CertifiedTypedSpec` fields.
- Removed runtime tests that asserted certifier-owned static lifecycle rejection paths.

Checks run:

- `cargo test -p mfm-certify certification_mints_descriptor_set_and_lifecycle_views`
- `cargo test -p mfm-certify --test certify_authority_ui`
- `cargo test -p mfm-runtime certified_runtime_spec_accepts_certifier_authority`
- `cargo fmt --all -- --check`
- `cargo check -p mfm-certify -p mfm-runtime`
- `cargo test -p mfm-certify`
- `cargo test -p mfm-runtime`
- `cargo test -p mfm-integration-tests --test cargo_metadata_contract`
- `cargo test -p mfm-integration-tests --test architecture_namespace_contract`

## Phase 6: Program Authoring Builders For Manual And Resource Policy

Status: completed.

Files changed:

- `crates/kernel/program/src/lib.rs`
- `crates/kernel/program/src/tests.rs`
- `crates/kernel/program/tests/ui/fail/remediation_handle_as_forward_input.rs`
- `crates/kernel/certify/src/lib.rs`
- `crates/ops/evm-contract-lifecycle-op/src/lib.rs`
- `crates/ops/proof-op/src/lib.rs`
- `tests/integration/src/test_support.rs`
- `tests/integration/tests/architecture_namespace_contract.rs`
- `IMPLEMENTATION_ACDC_SAGA_PRE_MERGE.md`

Validators deleted or replaced:

- Replaced program-authoring `ResourceClaimSpec` inputs with `ResourceClaim` constructors:
  `manual_only`, `exclusive`, and `exact_touched_set`.
- Added `ManualResolutionPolicyDraft`, `ManualAuthorizationDraft`, `OperatorAuthoritySnapshotDraft`,
  `NonEmptyUniqueOperators`, and `ThresholdQuorum`.
- Made empty manual authorities, duplicate operator ids, zero quorum, and quorum greater than
  authority size fail at program-authoring construction.
- Kept registry membership, signing scheme support, and verifier/operator authority checks in
  certification.
- Updated certifier lowering to consume the validated manual policy draft instead of reading raw
  public fields.

Tests added or updated:

- Added program unit tests for manual authority builder invariants.
- Added program unit tests for resource claim constructors.
- Updated program compile-fail fixture to use `ResourceClaim`.
- Updated ops and integration authoring fixtures to use `ResourceClaim`.
- Updated namespace guard allowlist counts for the manual policy builder terminology move.

Checks run:

- `cargo check -p mfm-program -p mfm-certify -p mfm-op-proof -p mfm-op-evm-contract-lifecycle -p mfm-integration-tests`
- `cargo test -p mfm-program`
- `cargo test -p mfm-certify certification_rejects_unsupported_manual_signing_scheme_and_quorum`
- `cargo test -p mfm-certify`
- `cargo test -p mfm-op-proof -p mfm-op-evm-contract-lifecycle`
- `cargo fmt --all -- --check`
- `cargo test -p mfm-integration-tests --test cargo_metadata_contract`
- `cargo test -p mfm-integration-tests --test architecture_namespace_contract`

## Phase 7: Store-Owned Committed Run Stream Authority

Status: completed.

Files changed:

- `crates/kernel/store/src/lib.rs`
- `crates/kernel/store/tests/commit_contract.rs`
- `crates/kernel/runtime/src/history.rs`
- `crates/kernel/runtime/src/lib.rs`
- `crates/kernel/runtime/src/tests.rs`
- `IMPLEMENTATION_ACDC_SAGA_PRE_MERGE.md`

Validators deleted or replaced:

- Added `CommittedRunStream` as the store-owned authority for one run stream's ordering, atomic
  commit grouping, projection rebuild, next sequence, and artifact role requirements.
- Added `CommittedRunStreamCommit` read views so callers can inspect commit grouping without
  reconstructing it from loose event vectors.
- Added `VerifiedProjectionSnapshot` as the internal projection authority carried by
  `CommittedRunStream`.
- Replaced runtime-local next-sequence reconstruction with `CommittedRunStream::next_seq`.
- Replaced runtime-local stream validation/projection rebuild entry paths with
  `CommittedRunStream::from_events` and `RuntimeRunView::from_committed_stream`.
- Kept runtime validation focused on certified-spec agreement and historical runtime semantics
  after store validation has established stream shape and projection authority.

Tests added or updated:

- Added store contract tests proving `CommittedRunStream` exposes run id, committed events,
  commit grouping, projection state, next sequence, side-effect views, and artifact role
  requirements.
- Added store contract test proving `CommittedRunStream` rejects persisted events for a different
  requested run id.
- Updated runtime test helpers to derive synthetic next sequence through `CommittedRunStream`
  instead of runtime-local reconstruction.

Checks run:

- `cargo fmt --all -- --check`
- `cargo check -p mfm-store -p mfm-runtime -p mfm-replay`
- `cargo test -p mfm-store committed_run_stream`
- `cargo test -p mfm-store`
- `cargo test -p mfm-runtime`
- `cargo test -p mfm-replay`
- `cargo test -p mfm-integration-tests --test cargo_metadata_contract`
- `cargo test -p mfm-integration-tests --test architecture_namespace_contract`

## Phase 8: Purpose-Specific Prepared Commits

Status: completed.

Files changed:

- `crates/kernel/store/src/lib.rs`
- `crates/kernel/store/tests/commit_contract.rs`
- `crates/kernel/runtime/src/commit.rs`
- `crates/kernel/runtime/src/manual_resolution.rs`
- `crates/kernel/runtime/src/scheduler.rs`
- `IMPLEMENTATION_ACDC_SAGA_PRE_MERGE.md`

Validators deleted or replaced:

- Added `NonEmptyPayloadBatch`, `CommitArtifactEvidenceSet`, `PreparedCommit<Purpose>`, and
  `PreparedCommitPlan`.
- Added sealed purpose markers for run start, state-attempt start, attempt terminal,
  side-effect terminal, side-effect progress, retention, manual resolution, and saga terminal
  commits.
- Added plan-based sync and async store append entrypoints. Runtime production mutation now calls
  `append_prepared_commit_plan` instead of submitting raw `PreparedTypedCommit` values.
- Added purpose constructors that reject empty batches, mixed-run payloads, mixed-spec payloads,
  wrong-purpose payload shapes, missing non-retention artifact evidence, and mismatched required
  artifact evidence sets before store admission.
- Added runtime required-artifact augmentation from verified run views so commits can require
  previously committed artifact evidence without re-admitting it.
- Left low-level `TypedCommitRequest` and `PreparedTypedCommit` available as storage/test DTOs for
  later deletion phases; production scheduler paths no longer start from them at the mutation
  boundary.

Tests added or updated:

- Added store contract tests for `NonEmptyPayloadBatch` empty-batch rejection.
- Added store contract tests for valid run-start purpose authority and invalid empty, mixed-run,
  mixed-spec, missing-artifact, and wrong-purpose construction attempts.
- Added store contract test proving runner output plans classify terminal attempt commits.
- Updated runtime launch, attempt-start, runner-output, and manual-resolution planners to mint
  purpose-specific prepared commits/plans.

Checks run:

- `cargo fmt --all -- --check`
- `cargo check -p mfm-store -p mfm-runtime -p mfm-replay -p mfm-stream-store-postgres`
- `cargo test -p mfm-store`
- `cargo test -p mfm-runtime`
- `cargo test -p mfm-replay`
- `cargo test -p mfm-integration-tests --test cargo_metadata_contract`
- `cargo test -p mfm-integration-tests --test architecture_namespace_contract`

## Phase 9: Run-Start Saga Policy Ref and SagaAdmitToken

Status: completed.

Files changed:

- `crates/kernel/spec/src/lib.rs`
- `crates/kernel/events/src/lib.rs`
- `crates/kernel/store/src/lib.rs`
- `crates/kernel/store/src/v1/admission.rs`
- `crates/kernel/store/src/v1/projection.rs`
- `crates/kernel/store/tests/commit_contract.rs`
- `crates/kernel/runtime/src/commit.rs`
- `crates/kernel/runtime/src/history.rs`
- `crates/kernel/runtime/src/manual_resolution.rs`
- `crates/kernel/runtime/src/tests.rs`
- `crates/kernel/replay/src/lib.rs`
- `crates/storages/stream-store-postgres/src/typed.rs`
- `crates/storages/artifact-store-fs/tests/contract.rs`
- `IMPLEMENTATION_ACDC_SAGA_PRE_MERGE.md`

Validators deleted or replaced:

- Added canonical `SagaPolicySpec::saga_policy_digest` as the reusable policy identity.
- Added `RunStarted::saga_policy_digest` and projected the digest as run-start authority.
- Added `SagaAdmitToken`, binding run id, spec hash, saga policy digest, and certified saga policy.
- Replaced raw `CommitPreconditions::saga_policy` with `CommitPreconditions::saga_admit_token`.
- Replaced manual-resolution and saga-terminal admission checks that trusted a raw policy
  precondition with token checks against the run-start digest projection.
- Replaced Postgres projection snapshot loading for saga policy authority with an event-stream
  rebuild path so rebuilt projections retain the run-start policy digest.
- Replaced runtime manual and terminal saga commits with minted `SagaAdmitToken` authority from
  certified runtime spec state.

Tests added or updated:

- Added store contract coverage proving a saga admit token with a different policy digest than the
  run-start digest is rejected.
- Updated store, events, runtime, replay, artifact-store, and Postgres fixtures to include
  run-start saga policy digests.
- Updated Postgres parity fixture to start saga runs with the manual saga policy used by later
  manual-resolution and terminal preconditions.

Checks run:

- `cargo fmt --all -- --check`
- `cargo check -p mfm-events -p mfm-spec -p mfm-store -p mfm-runtime -p mfm-replay -p mfm-stream-store-postgres -p mfm-artifact-store-fs`
- `cargo test -p mfm-store`
- `cargo test -p mfm-events -p mfm-runtime -p mfm-replay -p mfm-artifact-store-fs`
- `cargo test -p mfm-spec`
- `cargo test -p mfm-stream-store-postgres typed_saga_projection_tables_persist_and_rebuild_from_events`
- `cargo test -p mfm-stream-store-postgres --features parity-tests typed_saga_projection_tables_persist_and_rebuild_from_events` (compiled, then stopped because `DATABASE_URL` is unset for parity tests)
- `cargo test -p mfm-integration-tests --test cargo_metadata_contract`
- `cargo test -p mfm-integration-tests --test architecture_namespace_contract`
