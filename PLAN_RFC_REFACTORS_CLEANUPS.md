# Implementation Plan: RFC Refactors, Cleanups, And Larger Abstraction Bets

Status: implementation planning, ready for phase execution

Source RFC: `RFC_REFACTORS_CLEANUPS.md`

Architectural basis: `docs/design.md`, `docs/architecture.md`, `docs/code-quality.md`

Planning date: 2026-06-19

## Planning Assumptions

- Breaking changes are allowed when they improve correctness, clarity, or maintainability.
- Breaking changes must still be deliberate: update docs, tests, public contract fixtures, and
  persisted-format expectations in the same commit series.
- This plan does not authorize hacks, compatibility shims, weakened tests, dynamic untyped
  authority surfaces, or storing secret material in any persisted/public surface.
- Commit subjects below are written in the repository's lower-case style and should remain one
  logical change each.
- Nixfied wrappers are not the default verification gate. Use focused Cargo checks unless the
  change is specifically about Nixfied behavior.

## Phase Order

1. Phase 0: baseline tests and drift guards.
2. Phase 2: artifact role contract, first production cleanup.
3. Phase 3: scenario and golden infrastructure.
4. Phase 3b: persisted/public no-secret provenance hardening.
5. Phase 4: shared committed-run history view and app read paths.
6. Phase 5: async-primary service and driver collapse.
7. Phase 6: runtime runner kit.
8. Phase 7: generic side-effect driver.
9. Phase 8: check-only kernel protocol declarations, then gated production replacement.

Phase 1 is already complete. Projection persistence remains deferred to a separate measurement RFC.

## Cross-Cutting Gates

Every phase must preserve or deliberately version:

- event names, schema ids, descriptor ids, role tags, config artifact identities, public-output
  schema ids, certified spec hashes, canonical JSON bytes, and payload hashes
- append-only `run:*` streams as semantic authority
- all-or-nothing store commits
- content addressing for manifests, facts, snapshots, artifacts, specs, outputs, and certificates
- no floats in hash-defining structured data
- no ambient IO in state logic
- replay evidence-only behavior after app/runtime ingress
- public output as render/cache material, not execution/replay/resume authority
- projections as derived or verified caches, not independent semantic authority
- no secrets in typed configs, event payloads, artifacts, facts, public outputs, errors, fixtures,
  logs, CLI JSON, REST JSON, or replay/public-output responses

Source scans may be used as defense-in-depth, but typed tests and production-path fixtures are the
authority.

## Phase 0: Baselines

Goal: freeze the behavior that later table-driven or generated code must reproduce.

Entry criteria:

- Current post-FSM tree builds enough for focused baseline tests.
- No production API replacement is included in this phase.

Commits:

1. `add artifact role baseline goldens`
   - Touch likely: `crates/kernel/events/src/lib.rs`,
     `crates/kernel/store/tests/commit_contract.rs`,
     `crates/storages/artifact-store-fs/src/lib.rs` or tests.
   - Add compact table tests for every current role: spec, certificate, config, seed, state output,
     fact, side-effect phase evidence, manual resolution, public output, diagnostic, and retention
     manifest.
   - Freeze role tag, schema policy, semantic policy, producer policy, staging class, retention
     class, and same-commit policy as current behavior.
   - Add store codec roundtrip drift test: `ArtifactRole -> tag -> ArtifactRole`.
   - Add filesystem artifact-store metadata producer-policy matrix.

2. `add event schema and descriptor drift baselines`
   - Extend existing `v1_event_schema_golden`,
     `event_schema_descriptors_have_no_opaque_external_shapes`,
     `all_event_schema_descriptors_are_unique`, and
     `artifact_requirement_accessor_covers_artifact_bearing_variants`.
   - Add compact golden rows for `(event name, schema id, artifact requirement sources)`.
   - Add `ArtifactRole` schema descriptor tag baseline.

3. `add store projection differential baselines`
   - Add a reusable differential harness comparing store-maintained projections with
     `ProjectionSnapshot::rebuild_from_run_stream`.
   - Cover committed run view fields, public-output rendered/source artifact projections,
     retention refs, resource lanes, and side-effect phase tags.

4. `add runtime lifecycle baseline scenarios`
   - Add deterministic tuple summaries for ordinary attempts, open attempts, terminalized
     failures, framework attempts, resource-lane blocking, and side-effect recovery.
   - Ensure fixtures are secret-free and hash surfaces are float-free.

5. `add public output and replay acceptance baselines`
   - Add app-level tests proving `typed_public_output`, `verify_replay_for_run`, `run_status`, and
     `run_stream` reject the same tampered full stream.
   - Add REST/CLI coverage proving stream range filtering and presentation happen only after full
     authoritative stream validation.

Exit gate:

```bash
cargo fmt --all -- --check
cargo check --workspace
cargo test -p mfm-events
cargo test -p mfm-store --test commit_contract
cargo test -p mfm-artifact-store-fs
cargo test -p mfm-runtime
cargo test -p mfm-app
cargo test -p mfm-integration-tests --test cargo_metadata_contract
cargo test -p mfm-integration-tests --test architecture_namespace_contract
```

## Phase 2: Artifact Role Contract

Goal: make artifact roles one typed contract table instead of repeated string, schema, producer,
staging, replay, and retention matches.

Entry criteria:

- Phase 0 artifact-role and descriptor baselines are merged.
- Any intentional role-model break is documented before code changes.

Target API:

```rust
pub struct ArtifactRoleContract {
    pub role: ArtifactRole,
    pub tag: &'static str,
    pub schema: ArtifactSchemaPolicy,
    pub semantic: ArtifactSemanticPolicy,
    pub producer: ArtifactProducerScope,
    pub staging: ArtifactStagingClass,
    pub retention: ArtifactRetentionClass,
    pub same_commit: ArtifactSameCommitPolicy,
}
```

Add at least:

- `ArtifactRole::ALL`
- `ArtifactRole::as_str()`
- `ArtifactRole::parse()`
- `ArtifactRole::contract()`
- explicit evidence-field policies such as `Any`, `Absent`, and `Exact(T)` for artifact
  requirements, replacing ambiguous `Option<T>` where `None` currently means both unchecked and
  must-be-absent

Prefer adding an explicit role for resource touched-set evidence, such as
`resource_touched_set_evidence`, because breaking changes are allowed and schema-only exceptions
are a future drift source.

Commits:

1. `mfm-events add artifact role contract table`
   - Add contract structs/enums beside `ArtifactRole`.
   - Add full matrix tests.
   - Keep the current enum order and tags unless deliberately versioning a role.

2. `mfm-events derive artifact role descriptors from contracts`
   - Generate event schema role tag lists from `ArtifactRole::ALL`.
   - Route event artifact requirement builders through contract-aware helpers.
   - Preserve existing descriptor and requirement goldens, or update them only for explicit
     versioned changes.

3. `mfm-store use artifact role contracts for codecs`
   - Replace store-local role parse/string matches with wrappers around `mfm-events`.
   - Keep persisted JSON tag bytes stable unless deliberately versioned.

4. `mfm-store enforce artifact evidence field policies`
   - Convert requirement matching to explicit `Any/Absent/Exact` semantics.
   - Enforce schema, semantic, producer, and role policy for admitted artifacts.
   - Keep commit admission, same-commit legality, and context-specific policy in `mfm-store`.

5. `mfm-runtime derive staged artifact bindings from role contracts`
   - Replace role classification matches in runtime artifact/staging helpers with contract-backed
     staging and phase lookups.
   - Do not let the contract table mint commit authority.

6. `mfm-replay authorize artifacts through role contracts`
   - Replace replay role/schema branches with contract-backed authorization.
   - Keep replay evidence-only and keep manual-proof verification separate.

7. `mfm-app validate launch and public output artifacts by contract`
   - Use role contracts for spec, certificate, config, seed, public-output cell, and rendered
     artifact checks.
   - Keep `PublicOutputReadAuthority` app-minted and render-only.

8. `mfm-artifact-store-fs share artifact role metadata contract`
   - Remove duplicate role parse/string functions.
   - Use contract producer/schema/semantic policies for metadata validation.

9. `mfm-stream-store-postgres align typed artifact role persistence`
   - Keep `typed_artifacts.artifact_role` storage shape unless adding a generic producer
     exclusivity check.
   - Do not encode the whole Rust role matrix as SQL checks.

Exit gate:

```bash
cargo fmt --all -- --check
cargo check -p mfm-events -p mfm-store -p mfm-runtime -p mfm-replay -p mfm-app -p mfm-artifact-store-fs
cargo test -p mfm-events artifact_role_contract
cargo test -p mfm-events v1_event_schema_golden
cargo test -p mfm-store --test commit_contract artifact_role_contract
cargo test -p mfm-runtime artifact_role_contract
cargo test -p mfm-replay artifact_role_contract
cargo test -p mfm-artifact-store-fs artifact_role_contract
cargo test -p mfm-app verified_run_history
cargo test -p mfm-integration-tests --test proof_transport_conformance
cargo test -p mfm-integration-tests --test cargo_metadata_contract
cargo test -p mfm-integration-tests --test architecture_namespace_contract
```

With Postgres available:

```bash
DATABASE_URL=... cargo test -p mfm-stream-store-postgres
```

## Phase 3: Scenario And Golden Infrastructure

Goal: reduce fixture ceremony without hiding authority assertions or negative cases.

Entry criteria:

- Phase 2 is merged, or Phase 3 avoids role-contract assumptions.
- Phase 0 proof/replay/public baselines are available.

Commits:

1. `tests: add kernel scenario data package`
   - Add `tests/kernel-scenario-data` as `mfm-kernel-scenario-data`.
   - Add it to workspace members.
   - Keep it data-only: scenario descriptors, expected public JSON fragments, corruption case
     names, and golden labels.
   - Initial dependency allowlist: `serde`, `serde_json`, `mfm-ids`, `mfm-canonical`,
     `mfm-events`, `mfm-spec`.
   - Explicitly disallow `mfm-store`, `mfm-runtime`, `mfm-replay`, `mfm-app`, ops, adapters,
     transports, storages, binaries, `tokio`, `reqwest`, `tempfile`, signing, and crypto crates.

2. `tests: enforce scenario data dependency allowlist`
   - Extend `cargo_metadata_contract` to reject stale or forbidden `mfm-kernel-scenario-data`
     dependencies.
   - Ensure no production crate depends on scenario crates.

3. `tests: convert proof replay corruption fixtures`
   - First target: `tests/integration/tests/proof_transport_conformance.rs`.
   - Convert retention/replay corruption helpers into table-driven cases.
   - Keep stream mutation execution crate-local in `mfm-integration-tests`.

4. `tests: add equivalence goldens for converted proof cases`
   - Before deleting old helper plumbing, compare old/new rejection class and stream shape.
   - Preserve `ReplayErrorKind::InvalidRunStream` and all wrong-role/wrong-producer/missing-artifact
     expectations.

5. `tests: convert narrow replay artifact negative cases`
   - Target replay artifact corruption helpers in `tests/integration/src/test_support.rs`.
   - Preserve named cases for wrong role, wrong producer, missing committed artifact reference,
     tampered spec/certificate, and replay-with-live-capability.

6. `app cli rest: add manual resolution ingress`
   - Required before true public `ResolveSagaTerminal` fixture coverage.
   - Add app service method, CLI command, and REST route only through typed manual-resolution
     authority.
   - Update `bin/cli/README.md` and `bin/rest-api/README.md`.
   - Keep manual proof construction signed, prefix-bound, and secret-free.

7. `tests: add public resolve saga terminal fixture`
   - Drive a real run to `manual_blocked`, record signed manual resolution, resume to
     `manually_resolved`.
   - Assert public JSON `run_mode`, manual fields, completed framework attempt disposition, and
     stream ordering.
   - Assert terminal authority is `ResolveSagaTerminal`, not ordinary `CompleteRun`.
   - Assert no authorization proof bytes, signer material, or secret diagnostics appear in public
     JSON.

Exit gate:

```bash
cargo fmt --all -- --check
cargo test -p mfm-integration-tests --test cargo_metadata_contract
cargo test -p mfm-integration-tests --test architecture_namespace_contract
cargo test -p mfm-integration-tests --test proof_transport_conformance
cargo test -p mfm-runtime manual_resolution
cargo test -p mfm-store --test commit_contract saga_terminal
cargo test -p mfm-replay manual_resolution
cargo test -p mfm-app
cargo test -p mfm --test json_output_integration
cargo test -p mfm-integration-tests --test rest_api_run_control
```

## Phase 3b: Persisted/Public No-Secret Provenance Hardening

Goal: make the no-secret invariant reviewable across persisted and public surfaces.

Entry criteria:

- Phase 2 role table is merged.
- Minimal Phase 3 scenario/golden support exists.

Primary gap:

- Public error boundaries still risk forwarding lower-level `to_string()` messages into CLI/REST
  responses.
- `MfmErrorInfo.safe_message` is structurally a `String`; make safe public diagnostics explicit.

Commits:

1. `docs: inventory persisted and public surfaces`
   - Add `docs/persisted-public-surfaces.md`.
   - Link it from `docs/design.md` and `docs/architecture.md`.
   - Table columns: surface, owner crate, persisted/public exposure, allowed data classes,
     forbidden classes, provenance authority, validation path, tests.
   - Explicitly classify CLI `keystore tx-sign --out` raw transaction output as intentional local
     bearer material outside typed-run persisted/public surfaces.

2. `app cli rest centralize public safe errors`
   - Add a small public-safe error/message boundary used by `AppError`, REST `ApiError`, and CLI
     `CommandError`.
   - Stop forwarding backend `to_string()` directly for store, artifact, runtime, IO, transport,
     and signer/provider errors.
   - Preserve stable error codes where useful; breaking message changes are allowed.

3. `mfm-events harden persisted error info`
   - Add constructors/validation for `MfmErrorInfo.safe_message`, `public_details`, and diagnostic
     artifact refs.
   - Reject secret-shaped public diagnostics in runtime/store commit paths.
   - Keep diagnostic artifacts redacted and content-addressed.

4. `tests add production path redaction goldens`
   - Cover CLI JSON/text, REST JSON, run events, projections, artifact metadata, facts, public
     outputs, replay responses, and diagnostic artifacts with sentinel secrets in failing inputs or
     provider errors.
   - Assert sentinels never appear in stdout/stderr, REST bodies, events, artifacts, projections,
     public output, or replay output.

5. `evm contracts harden side effect provenance tests`
   - Prepared invocation artifacts may contain unsigned evidence only: signer refs, addresses,
     hashes, nonce/gas fields, and data digests.
   - Assert no raw tx, signature, provider kind, endpoint, RPC URL, authorization, keystore path,
     password, mnemonic, or private key crosses into typed events/artifacts/replay.

Defense-in-depth scans:

```bash
rg -n --hidden -g '*.rs' -g '*.md' -g '*.toml' -g '*.json' \
  '(private[_-]?key|mnemonic|password|passphrase|seed phrase|api[_-]?key|authorization|bearer |raw[_-]?transaction|signed[_-]?payload|keystore[_-]?path|rpc[_-]?url)' \
  crates bin tests docs .github

rg -n 'error\.to_string\(\)|format!\(".*\{error\}' \
  crates/app/src bin/rest-api/src bin/cli/src crates/adapters crates/transports -g '*.rs'

rg -n 'println!|eprintln!|tracing::|debug!|info!|warn!|error!|instrument\(' \
  crates bin -g '*.rs'

rg -n 'raw_transaction|raw_tx|signed_payload|TransientRawTransaction|SignedEvmPayload' \
  crates bin tests -g '*.rs'
```

Exit gate:

```bash
cargo fmt --all -- --check
cargo check --workspace
cargo test -p mfm-values
cargo test -p mfm-program-derive --test derive_ui
cargo test -p mfm-events
cargo test -p mfm-store --test commit_contract
cargo test -p mfm-runtime
cargo test -p mfm-app
cargo test -p mfm-artifact-store-fs
cargo test -p mfm-signing --test signing_ui
cargo test -p mfm-signers-keystore
cargo test -p mfm-evm-signing --test evm_signing_ui
cargo test -p mfm-evm-capabilities
cargo test -p mfm-adapters-evm-contracts
cargo test -p mfm --test json_output_integration
cargo test -p mfm --test parity_keystore_reth_tx_send
cargo test -p mfm-integration-tests --test architecture_namespace_contract
cargo test -p mfm-integration-tests --test rest_api_run_control
cargo test -p mfm-integration-tests --test proof_transport_conformance
```

## Phase 4: Shared Committed-Run History View

Goal: promote the existing runtime fold into a sealed shared verified view consumed by runtime,
replay, and app through explicit wrappers.

Entry criteria:

- Phase 3 scenario support covers run-view/read-path corruption cases.
- Phase 3b redaction boundaries are in place for public read paths.

Target API:

```rust
pub struct VerifiedRunHistoryView {
    // private committed stream, verified artifacts, and runtime fold indexes
}

impl VerifiedRunHistoryView {
    pub fn from_committed_stream(
        runtime_spec: &CertifiedRuntimeSpec,
        committed: store::CommittedRunStream,
        artifacts: store::VerifiedRunArtifactStore,
    ) -> Result<Self>;

    pub fn run_id(&self) -> &RunId;
    pub fn spec_hash(&self) -> &SpecHash;
    pub fn events(&self) -> &[store::KernelEventEnvelope];
    pub fn committed_stream(&self) -> &store::CommittedRunStream;
    pub fn artifact_store(&self) -> &store::VerifiedRunArtifactStore;
    pub fn projection_snapshot(&self) -> &store::ProjectionSnapshot;
    pub fn run_admitted(&self) -> &events::RunAdmitted;
    pub fn head_seq(&self) -> store::StreamSeq;
}
```

`VerifiedRunHistory` stores the view and exposes `view(&self)`. No public raw-event constructor or
public fields. `CommittedRunStream` stays store-owned.

Commits:

1. `test: baseline shared run view read paths`
   - Add differential tests comparing runtime view, replay broker projection, app status,
     run stream, replay response, and public-output response for the same committed streams.
   - Include corruption fixtures for missing retention projection, post-completion events, and
     tampered public-output payloads.

2. `runtime: add sealed verified run history view`
   - Promote retained parts of `RuntimeRunView` from `crates/kernel/runtime/src/history.rs`.
   - Store the view inside `VerifiedRunHistory`.
   - Add rustdoc and compatibility accessors.

3. `runtime: route history loaders through verified view`
   - Update `VerifiedRunHistory::from_store`, `from_async_store`, and `from_committed_stream`.
   - Keep scheduler-only stream fold crate-private if artifact authority is not available during
     drive context loading.

4. `replay: build read authority from verified run history view`
   - Add `ReplayReadAuthority::from_verified_run_history_view`.
   - Reuse view projections/events where appropriate.
   - Keep replay-specific verifier identity, artifact authorization, manual proof checks, and
     no-live-capability enforcement in `mfm-replay`.

5. `app: unify status stream replay and output reads`
   - Add one private helper that loads stream, verifies certified bundle, builds
     `CommittedRunStream`, loads `VerifiedRunArtifactStore`, and returns runtime spec plus view.
   - Route `run_status`, `run_stream`, `verify_replay_for_run`, and `typed_public_output` through
     this helper.
   - Keep `PublicOutputReadAuthority` app-minted.

6. `cli rest: preserve validated stream filtering`
   - Ensure range filtering and presentation happen only after app-level full stream validation.
   - Do not expose raw store streams as trusted input.

Compile-fail tests:

- `raw_verified_run_history_view_constructor.rs`
- `verified_run_history_view_fields_private.rs`
- update replay/app constructor-removal UI fixtures as needed

Exit gate:

```bash
cargo fmt --all -- --check
cargo check --workspace
cargo test -p mfm-runtime --test runtime_authority_ui
cargo test -p mfm-runtime --lib
cargo test -p mfm-replay --test replay_authority_ui
cargo test -p mfm-replay
cargo test -p mfm-app --test public_output_authority_ui
cargo test -p mfm-app
cargo test -p mfm-store --test commit_contract
cargo test -p mfm
cargo test -p mfm-rest-api
cargo test -p mfm-integration-tests --test rest_api_run_control
cargo test -p mfm-integration-tests --test proof_transport_conformance
cargo test -p mfm-integration-tests --test cargo_metadata_contract
cargo test -p mfm-integration-tests --test architecture_namespace_contract
```

## Phase 5: Async-Primary Service And Driver Collapse

Goal: collapse duplicate sync/async app service and runtime IO choreography after shared read
authority exists.

Entry criteria:

- Phase 4 shared view is merged.
- Runtime/app sync-vs-async parity baselines exist.

Compatibility stance:

- Async is the production execution path.
- Keep sync compatibility only as a narrow wrapper or low-level store-test boundary.
- CLI and REST converge on async app services.

Commits:

1. `test: add sync async runtime parity baselines`
   - Compare `SchedulerStatus`, emitted event payloads, staged artifacts, projections, replay
     result, public status, and public JSON.
   - Cover ordinary attempts, framework attempts, append-only resume, public-output projection,
     open-attempt recovery, stale expected seq, resource-lane blocked side effects, and redacted
     failure terminalization.

2. `mfm-store add async in-memory typed store`
   - Implement `AsyncTypedRunEventStore` for a test/local async in-memory adapter.
   - Keep locking explicit; do not hold mutable locks across awaits.

3. `mfm-app extract verified read ingress helpers`
   - Reuse Phase 4 helper for full stream load, bundle verification, runtime spec construction,
     retained artifact verification, shared view construction, and response derivation.

4. `mfm-app make async services primary`
   - Collapse `RunServices` and `AsyncRunServices` into one async facade or keep
     `AsyncRunServices` as the public facade while deprecating sync.
   - Remove duplicated launch/resume/status/stream/replay/public-output logic.

5. `cli rest use unified run services`
   - Move CLI status/stream/public-output/replay and REST handlers through the unified app helpers.
   - Keep HTTP status mapping outside CLI JSON contracts.
   - Avoid rebuilding production service wiring inside request paths.

6. `runtime collapse driver io choreography`
   - Route sync compatibility through the async-primary core or delete sync entry points once
     callers are migrated.
   - Do not change lifecycle semantics, attempt-start ordering, expected-seq concurrency,
     resource-lane behavior, or side-effect recovery.

Exit gate:

```bash
cargo fmt --all -- --check
cargo check -p mfm-store -p mfm-runtime -p mfm-app -p mfm -p mfm-rest-api -p mfm-integration-tests
cargo test -p mfm-runtime
cargo test -p mfm-app
cargo test -p mfm
cargo test -p mfm-rest-api
cargo test -p mfm-store --test commit_contract side_effect
cargo test -p mfm-integration-tests --test rest_api_run_control
cargo test -p mfm-integration-tests --test cargo_metadata_contract
cargo test -p mfm-integration-tests --test architecture_namespace_contract
```

With Postgres available:

```bash
DATABASE_URL=... cargo test -p mfm --test status_contract_postgres
DATABASE_URL=... cargo test -p mfm-integration-tests --test parity_rest_api_postgres_typed_smoke
```

## Phase 6: Runtime Runner Kit

Goal: reduce repeated runner artifact/payload ceremony without moving domain behavior into runtime
or app.

Entry criteria:

- Phase 5 leaves one ordinary execution path.
- Runner output goldens exist.

Crate placement:

- Start in `crates/kernel/runtime/src/runner_kit.rs`, re-exported from `mfm-runtime`.
- Do not add `mfm-artifact-capabilities` to runtime.
- Keep config/input artifact loading adapter-local for the first slice.

Target API:

- `RunnerJsonArtifact`
- `RunnerArtifactBuilder`
- `RunnerPayloadBuilder`
- `RunnerOutputBuilder`
- later `RunnerRegistrationBuilder`
- `RunnerGolden` for deterministic test summaries

Builders produce existing `StagedArtifact`, `StagedRetentionRefs`, and `RunnerEventPayload` only.
No loose `serde_json::Value` runner execution surface.

Commits:

1. `test: add runner output goldens`
   - Normalize proof read/side-effect/assemble, portfolio read/pure outputs, EVM validate/output
     and side-effect phase payloads, plus executable identity summaries.

2. `runtime: add runner kit artifact and payload builders`
   - Add canonical JSON artifact helpers, schema-less prepared-invocation helpers, retention refs,
     `CellProduced`, `FactRecorded`, and side-effect evidence payload helpers.
   - Keep constructors contextual on `ErasedRunCtx`.

3. `proof: use runner kit for deterministic proof outputs`
   - Port proof first.
   - Preserve conformance stream/event/artifact/retention summaries.

4. `portfolio: use runner kit for read and pure outputs`
   - Port portfolio `read_output`, state output, artifact creation, retention, and
     `CellProduced`.
   - Keep artifact reads and input decoding local.

5. `evm contracts: use runner kit for validate and mutation payloads`
   - Port EVM validation read output, mutation final output, and side-effect artifact/payload
     construction.
   - Keep phase switching, provider logic, signing, and raw transaction bytes inside the adapter.

6. `runtime: add runner registration helpers`
   - Add only after executable identity goldens are in place.
   - Preserve factory id and `ExecutableIdentity` bytes, or version deliberately.

Exit gate:

```bash
cargo fmt --all -- --check
cargo check -p mfm-runtime -p mfm-transports-proof -p mfm-adapters-portfolio -p mfm-adapters-evm-contracts -p mfm-app
cargo test -p mfm-runtime
cargo test -p mfm-transports-proof
cargo test -p mfm-adapters-portfolio
cargo test -p mfm-adapters-evm-contracts
cargo test -p mfm-app
cargo test -p mfm-integration-tests --test proof_transport_conformance
cargo test -p mfm-integration-tests --test typed_portfolio_snapshot_local
cargo test -p mfm-integration-tests --test cargo_metadata_contract
cargo test -p mfm-integration-tests --test architecture_namespace_contract
```

## Phase 7: Generic Side-Effect Driver

Goal: centralize side-effect protocol ceremony while keeping runtime/store as authority and
adapters/transports as live IO owners.

Entry criteria:

- Phase 6 runner-kit helpers are available.
- Side-effect ambiguity, recovery, not-submitted, and no-secret goldens exist.

Crate placement:

- Start in `mfm-runtime` as generic side-effect protocol support.
- Do not create an adapter-category helper crate; current metadata rules would make
  `mfm-transports-proof` dependency placement awkward.

Target API:

- `SideEffectAttemptView`, private constructor from verified runtime context or `ErasedRunCtx`
- `SideEffectProtocolAction`
- `SideEffectClaimAuthority`
- `SideEffectEvidenceBuilder`
- `SideEffectDriverCallbacks`
- `SideEffectDriver::drive(...) -> ErasedRunnerOutput`

The driver proposes runner output only. Runtime still derives preconditions and commit purpose;
store still admits transitions.

Commits:

1. `test: add side effect driver baselines`
   - Add proof and EVM timelines: intent, claim, prepared, started, submission observed,
     submission unknown, not-submitted, receipt, confirmation, ambiguity, failure, and output after
     confirmation.

2. `runtime: expose verified side effect attempt view`
   - Wrap `SideEffectLifecycle::projection_for_attempt` and `SideEffectLedgerState`.
   - Constructor must require verified runtime context, not raw projection snapshots.

3. `runtime: add side effect evidence builder`
   - Build phase payloads and staged artifacts through runner kit.
   - Include ledger key, purpose, epoch, claim owner/generation, fencing token, replay verifier id,
     and resource evidence explicitly.
   - Do not create commits or attempt failures.

4. `runtime: add side effect driver callbacks`
   - Define callbacks for intent/idempotency, prepare, reconstruct prepared invocation, submit or
     recover submission, read receipt, build confirmation, and map confirmation to output.
   - Keep artifact reads and live providers in adapters.

5. `proof: port deterministic proof side effect to driver`
   - Preserve event/artifact sequence, replay verifier behavior, and no duplicate submission on
     resume.

6. `evm contracts: port deploy side effect driver`
   - Preserve executable identity and replay verifier id.
   - Keep raw signed transaction bytes transient inside EVM adapter.
   - Cover resume from started, unknown, submission, receipt, and confirmation phases.

7. `evm contracts: port configure side effect driver`
   - Port after deploy parity.
   - Preserve full lifecycle public output.

8. `replay: centralize side effect frame collection`
   - Optional final slice.
   - Share event-frame collection while keeping domain verifier callbacks in proof/EVM.
   - Replay must remain evidence-only.

Exit gate:

```bash
cargo fmt --all -- --check
cargo check --workspace
cargo test -p mfm-store --test commit_contract side_effect
cargo test -p mfm-runtime side_effect
cargo test -p mfm-transports-proof
cargo test -p mfm-adapters-evm-contracts
cargo test -p mfm-app
cargo test -p mfm-integration-tests --test proof_transport_conformance
cargo test -p mfm-integration-tests --test cargo_metadata_contract
cargo test -p mfm-integration-tests --test architecture_namespace_contract
```

Live services, only when manually started:

```bash
cargo test -p mfm-integration-tests --test parity_evm_contract_lifecycle_reth
cargo test -p mfm-integration-tests --test parity_portfolio_tracker_reth_snapshot
```

## Phase 8: Kernel Protocol Declarations

Goal: prove declarative kernel protocol definitions can reproduce existing behavior before
deleting handwritten descriptors, codecs, lenses, or projection inputs.

Entry criteria:

- Phase 2 role contract is merged.
- Phase 3 scenario/golden support exists.
- Phase 4/5 read path and service convergence are not actively changing the same surfaces.

Recommendation:

- Start with a normal Rust declaration table plus tests.
- Do not start with a proc macro or `build.rs`.
- Do not start with `ProgramPackage`, `WorkflowDescriptor`, or `StateSpec` derives.
- First event family: `FactRecorded`, because it exercises schema descriptors, payload codec,
  artifact requirements, and projection behavior without side-effect/resource-lane complexity.

Commits:

1. `tests: add fact recorded protocol baselines`
   - Freeze schema descriptor canonical JSON/hash, payload canonical JSON/hash, store roundtrip,
     artifact requirement output, and projection delta from a minimal stream.

2. `mfm-events add check only event family declaration`
   - Add private/test-only `EventFamilyDeclaration` for `FactRecorded`.
   - Include variant tag, schema name, Rust type path, fields, and artifact lens metadata.
   - Compare generated descriptor with handwritten descriptor byte-for-byte.

3. `mfm-events add check only artifact requirement generation`
   - Generate expected `FactRecorded` requirements from the declaration.
   - Compare with current `event_artifact_requirements`.

4. `mfm-store add check only event codec declaration view`
   - Use declaration to produce expected payload field/tag shape.
   - Compare with `payload_canonical_json`, payload hash, and `payload_from_json_value`.
   - Keep production codec as source of truth.

5. `mfm-store add check only projection transition descriptor`
   - Describe expected `FactRecorded` projection delta.
   - Compare with `ProjectionSnapshot::rebuild_from_run_stream`.
   - Do not generate projection application yet.

6. `mfm-events expand check only declaration to second family`
   - Next candidates: `StateAttemptStarted` for lifecycle coverage or `CellProduced` for artifact
     and output-cell coverage.
   - Avoid side-effect/resource-lane families until the model proves simpler invariants.

7. `mfm-events replace first handwritten descriptor through declaration`
   - Only after at least two families have byte-identical descriptor, codec, artifact, projection,
     runtime, and replay checks.
   - Treat any hash or canonical JSON change as a versioned persisted-format migration.

8. `mfm-program add package declaration baselines`
   - Only after event-family declaration replacement is proven.
   - Capture descriptor maps, registry summaries, certified spec hashes, config artifact sets, and
     public-output schema goldens.

9. `mfm-program add handwritten program package wrappers`
   - Start proof, then portfolio, then EVM.
   - Centralize descriptor registration and config artifact selection without changing graph
     expansion semantics.

10. `mfm-program-derive add state declaration derive`
    - Only after descriptor-id and trybuild diagnostic goldens exist.
    - Derive descriptors, not adapter execution authority.
    - Adapter crates still bind runners, executable identities, and capability implementations
      explicitly.

Exit gate:

```bash
cargo fmt --all -- --check
cargo check -p mfm-events -p mfm-spec -p mfm-store -p mfm-certify -p mfm-program -p mfm-program-derive -p mfm-runtime -p mfm-replay
cargo test -p mfm-events
cargo test -p mfm-spec
cargo test -p mfm-store --test commit_contract
cargo test -p mfm-certify
cargo test -p mfm-program
cargo test -p mfm-program-derive
cargo test -p mfm-runtime
cargo test -p mfm-replay
cargo test -p mfm-integration-tests --test cargo_metadata_contract
cargo test -p mfm-integration-tests --test architecture_namespace_contract
```

## Deferred: Projection Persistence Boundary

Do not implement projection persistence changes under this plan. Prepare a separate measurement RFC
with this checklist:

- inventory projection tables as authority indexes, read caches, or concurrency/admission aids
- measure stream-only rebuild latency by run size and event count
- measure Postgres query count, bytes read, and lock behavior for append/status/resume
- compare memory-store and Postgres projection parity
- poison/delete projection rows and confirm status/admission remain stream-authoritative
- benchmark global resource-lane rebuild cost across many runs
- evaluate ephemeral projections, compact verified snapshots, retained read models, and dedicated
  resource-lane admission tables
- define migration/repair behavior, backfill, rebuild, schema validation, and SQLx metadata updates
- preserve that projections never authorize resume, replay, public output, retention, or status
  without event-stream authority

## Final Workspace Gate

After each major phase, run focused checks first. Before considering the full RFC implementation
track ready for integration, run:

```bash
cargo fmt --all -- --check
cargo check --workspace
cargo test --workspace
cargo test -p mfm-integration-tests --test cargo_metadata_contract
cargo test -p mfm-integration-tests --test architecture_namespace_contract
```

Run live-service parity manually with explicit environment variables when the touched phase affects
Postgres, Reth, EVM, REST, or other external-service behavior.
