# Implementation Plan: RFC Deletion-Gate Cleanup

Status: execution plan

This plan turns the deletion-gate audit in `RFC_REFACTORS_CLEANUPS.md` into an implementation
sequence. The goal is not to add more abstractions. The goal is to delete old displaced surfaces and
make the new abstractions the only maintained authority where they are worth keeping.

## Non-Negotiable Rules

1. No backward compatibility is required.
2. Breaking API changes are allowed.
3. Do not deprecate old APIs. Delete them.
4. Do not keep compatibility shims, aliases, compatibility wrappers, or fallback paths.
5. Public or production-crate implementations used only by tests must be deleted, made private to the
   owning test module, or replaced by a smaller approved corruption/contract fixture.
6. Test-only infrastructure may remain only when it deletes larger old fixture machinery in the same
   commit series. If it only adds another layer, delete it too.
7. Approved fixture classes are limited to store admission/codec corruption, replay corruption,
   artifact-store contract, and deliberately forged persisted-stream contract tests.
8. A phase does not count as cleanup when it only adds baselines, declarations, wrappers, helper
   crates, compile-fail tests, or generic APIs.
9. Each commit must have one deletion target and one validation story.
10. If there is doubt about whether a retained helper is authority-bearing or just stale test support,
   stop and ask an architect-agent to review that exact helper before keeping it.

Commit subjects must be lower case.

## Baseline Before Starting

Run and record:

```bash
git status --short --branch
git diff --shortstat origin/dev..HEAD
git diff --numstat origin/dev..HEAD | sort -nr -k1,1 | head -40
cargo check --workspace --all-targets
cargo fmt --all -- --check
```

The branch should start from the deletion-gate audit state:

- `RFC_REFACTORS_CLEANUPS.md` contains the deletion-gate audit.
- Current branch diff from `origin/dev` is net positive.
- The work below should reduce that net positive number by deleting displaced code.

## Commit 1: `docs: add deletion gate implementation plan`

Scope:

- Add this file.

Validation:

```bash
git diff --check -- PLAN_IMPL_RFC_DELETION_GATE.md
```

Deletion credit:

- None. This is planning only.

## Commit 2: `app: remove raw stream presentation entrypoint`

Deletion target:

- Delete or make non-public `typed_run_stream_response_from_events` in `crates/app/src/lib.rs`.
- Delete tests that call it directly as a raw event-slice formatter.

Replacement:

- `RunServices::run_stream` and any remaining stream rendering must go through a verified read
  context or a private formatter that can only be called after verification.
- If a small formatter remains, it must take a verified context/view or be private to a module where
  the caller already proved the stream.

Do not keep:

- Public helpers accepting raw `&[KernelEventEnvelope]`.
- Public helpers that accept a caller-provided stream head without proving it came from the verified
  stream.

Validation:

```bash
cargo test -p mfm-app run_stream
cargo test -p mfm-integration-tests --test rest_api_run_control
rg -n "pub fn typed_run_stream_response_from_events|typed_run_stream_response_from_events\\(" crates bin tests
```

Exit gate:

- No public app API accepts raw event slices for stream/status/public-output rendering.

## Commit 3: `app cli: fold projection status behind verified service`

Deletion target:

- Delete public or externally reachable `run_status_with_projection` unless it can be made private
  and constrained to an approved store-owned cross-run resource-lane exception.
- Remove CLI direct projection preloading if the service can load the required status context itself.

Replacement:

- One status entrypoint on app services.
- If resource-lane projection data is still needed, expose it as a private typed input created by the
  store layer, not as a second public status path.

Do not keep:

- A separate CLI status read path that bypasses service construction.
- Public app status helpers accepting arbitrary projection snapshots.

Validation:

```bash
cargo test -p mfm --test json_output_integration run_status
cargo test -p mfm-integration-tests --test rest_api_run_control
rg -n "run_status_with_projection|projection_snapshot\\(" bin/cli crates/app/src bin/rest-api
```

Exit gate:

- CLI, REST, and app tests use the same status service entrypoint except for documented private
  store-owned resource-lane internals.

## Commit 4: `store: remove public sync in memory execution surface`

Deletion target:

- Remove public `InMemoryTypedRunStore` construction and direct use outside approved store/replay
  corruption or commit-contract internals.
- Remove runtime/app/integration scheduler fixtures that wrap sync stores into async execution.

Replacement:

- Use `AsyncInMemoryTypedRunStore` for scheduler, app, CLI, REST, and integration execution tests.
- If the async store needs a mutable in-memory core internally, make that core private to `mfm-store`.
- If a forged-stream/corruption fixture needs direct batch construction, keep it only in an approved
  store/replay test-support module, not as a public production API.

Do not keep:

- Public sync store APIs for runtime/app execution.
- Runtime/integration helper types that implement async store traits by borrowing
  `InMemoryTypedRunStore`.
- Sync fixture wrappers in `crates/kernel/runtime/src/tests.rs` or `tests/integration/src/test_support.rs`
  unless they are explicitly corruption-only and cannot drive scheduler execution.

Validation:

```bash
cargo test -p mfm-runtime
cargo test -p mfm-app
cargo test -p mfm-integration-tests --test proof_transport_conformance
rg -n "InMemoryTypedRunStore" crates/kernel/runtime crates/app tests/integration bin
```

Exit gate:

- `InMemoryTypedRunStore` does not appear in runtime/app/integration execution paths.
- Remaining references are limited to `mfm-store` internals/tests or explicitly named corruption
  fixtures.

## Commit 5: `store tests: eliminate public forged batch helper`

Deletion target:

- Delete public `build_committed_batch` from `mfm_store::v1`.
- Delete any public hash/fingerprint helper retained only so tests can forge streams.

Replacement:

- Move forged-stream construction into the smallest possible test-only location.
- Prefer scenario-owned typed corruption builders that produce named invalid streams.
- If low-level store contract tests need direct envelope rewriting, keep helpers inside
  `crates/kernel/store/tests/commit_contract.rs` or a private test module, not exported from
  `mfm-store`.

Do not keep:

- Public APIs whose only callers are corruption tests.
- Copies of the same forged-batch logic in runtime, app, replay, and integration tests.

Validation:

```bash
cargo test -p mfm-store --test commit_contract
cargo test -p mfm-runtime
cargo test -p mfm-app
cargo test -p mfm-integration-tests --test proof_transport_conformance
rg -n "build_committed_batch|commit_fingerprint\\(|payload_hash\\(" crates bin tests
```

Exit gate:

- No public production crate API exists only to help tests forge committed envelopes.

## Commit 6: `events runtime replay: finish artifact role contract deletion`

Deletion target:

- Delete duplicate role policy/classification matches outside the store admission boundary.
- Delete local role parse/string helpers that do not delegate to `ArtifactRole::parse` or
  `ArtifactRole::as_str`.

Replacement:

- Runtime staged artifact requirements consume the events-owned role contract or an events-owned
  requirement lens.
- Runtime same-commit role extraction uses role contract metadata where possible.
- Replay artifact authorization uses the same role policy table for role/schema/producer decisions.
- Fixture role groups use events-owned role classifications or scenario data.

Keep:

- Store same-commit admission checks that are genuinely store authority.

Do not keep:

- Handwritten role class lists in runtime, replay, app, or integration fixtures.
- Independent string tag parsers in storage crates.

Validation:

```bash
cargo test -p mfm-events
cargo test -p mfm-store --test commit_contract artifact_role
cargo test -p mfm-runtime artifact_role
cargo test -p mfm-replay artifact_role
cargo test -p mfm-integration-tests --test typed_portfolio_snapshot_local
rg -n "ArtifactRole::|parse_artifact_role|artifact_role_str|TypedSpecCertificate|PreparedInvocation|RetentionManifest" crates tests
```

Exit gate:

- Role policy has one source of truth in `mfm-events`.
- Duplicate role classification matches outside approved store admission code are gone.

## Commit 7: `tests: make scenario data replace proof corruption fixtures`

Deletion target:

- Delete handwritten proof corruption mutation dispatch by string.
- Delete local proof replay equivalence vectors if they can move into scenario data.
- Delete replaced replay-artifact negative case match helpers.

Replacement:

- Scenario descriptors use typed mutation enums, not string labels.
- Scenario data owns expected summaries, stream-shape hashes, or public-output hashes.
- Integration tests iterate typed scenarios and assert scenario-owned expectations.

Do not keep:

- Scenario labels that simply route back to test-local handwritten mutation code.
- Parallel old/new proof corruption fixture systems.

Validation:

```bash
cargo test -p mfm-integration-tests --test proof_transport_conformance
cargo test -p mfm-integration-tests --test cargo_metadata_contract
rg -n "unknown proof replay corruption mutation|proof_replay_corruption_equivalence_golden|corruption_cases" tests/integration tests/kernel-scenario-data
```

Exit gate:

- Proof corruption fixtures are scenario-owned, typed, and replace the old local dispatch machinery.

## Commit 8: `tests: consolidate manual resolution public scenario`

Deletion target:

- Delete route-shape-only CLI/REST tests that duplicate but do not execute the manual-resolution
  scenario.
- Delete app-only manual-resolution fixture plumbing that cannot be reused.

Replacement:

- One logical manual-resolution scenario is shared across app, CLI, and REST contract tests.
- The scenario proves manual block, manual resolution recording, `ResolveSagaTerminal` completion,
  framework attempt ordering, public status, stream visibility, and non-exposure of authorization
  proof bytes or signer material.

Do not keep:

- Independent app/CLI/REST fixtures for the same manual-resolution story.
- Tests that only assert static route shape when a production-path scenario can assert behavior.

Validation:

```bash
cargo test -p mfm-app manual_resolution
cargo test -p mfm --test json_output_integration manual
cargo test -p mfm-integration-tests --test rest_api_run_control manual
rg -n "ResolveSagaTerminal|manual resolution|manual-resolution" crates/app bin/cli bin/rest-api tests
```

Exit gate:

- The same scenario drives all public manual-resolution surfaces.

## Commit 9: `security: replace no secret sentinels with production scenarios`

Deletion target:

- Delete source-scan-only or string-sentinel-only tests that are superseded by typed production-path
  tests.
- Delete duplicate no-secret fixture helpers that do not exercise production entrypoints.

Replacement:

- Scenario goldens for secret-bearing runner inputs.
- Production entrypoint checks across runtime, store, replay, public output, CLI, and REST.
- Side-effect signed/raw payload paths prove raw signed payloads, signatures, mnemonics, passwords,
  and private key material never reach persisted/public surfaces.

Do not keep:

- Source scans as the primary proof.
- Secret-shaped string sentinel checks without production entrypoint coverage.

Validation:

```bash
cargo test -p mfm-events secret
cargo test -p mfm-runtime secret
cargo test -p mfm-app secret
cargo test -p mfm-integration-tests --test rest_api_run_control secret
rg -n "mnemonic|private key|raw signed|signature|password|secret" crates bin tests docs
```

Exit gate:

- Source scans are defense-in-depth only.
- Typed production-path tests are the authority for no-secret guarantees.

## Commit 10: `runner kit: delete duplicate valid runner builders`

Deletion target:

- Delete adapter/test helper builders that construct valid `CellProduced`, `FactRecorded`, staged
  artifacts, retained refs, public-output payloads, and side-effect payloads by hand.
- Delete local runner output helpers in proof, portfolio, EVM, app tests, runtime tests, and
  integration support when they model normal valid runner output.

Replacement:

- Valid runner output goes through `RunnerArtifactBuilder`, `RunnerPayloadBuilder`, and
  `RunnerOutputBuilder`.
- Invalid-output tests use explicitly named negative fixtures.

Keep only if justified:

- Adapter-local executable identity wrappers that preserve explicit descriptor/factory identity.
  These must be documented as retained authority ceremony and not counted as cleanup.

Do not keep:

- Local `registered_descriptor` or `register_runner` wrappers if `RunnerRegistrationBuilder` can
  preserve the same executable identity explicitly.
- Test-side valid output builders that duplicate runner kit behavior.

Validation:

```bash
cargo test -p mfm-runtime runner
cargo test -p mfm-app runner
cargo test -p mfm-integration-tests --test proof_transport_conformance
cargo test -p mfm-integration-tests --test typed_portfolio_snapshot_local
rg -n "CellProduced\\(|FactRecorded\\(|staged_artifact|retention_ref|registered_descriptor|register_runner\\(" crates tests
```

Exit gate:

- Normal valid runner output construction has one path through the runner kit.
- Remaining manual builders are explicitly negative fixtures.

## Commit 11: `side effects: mint claim authority in runtime`

Deletion target:

- Delete adapter-owned durable side-effect authority fields from `SideEffectIntentPlan`.
- Delete proof/EVM code that hardcodes or derives ledger key, claim owner, fencing token, and epoch
  as durable protocol authority.

Replacement:

- Runtime/store mints side-effect ledger key, claim owner, fencing token, and epoch authority.
- Adapters supply domain intent, idempotency input, prepared invocation material, submission, receipt,
  confirmation, and output mapping only.
- The driver consumes runtime-minted verified side-effect views.

Do not keep:

- `RunnerSideEffectBinding` or `SideEffectClaimAuthority` as adapter-returned durable authority.
- Adapter callbacks that decide claim owner/fencing token/epoch.

Validation:

```bash
cargo test -p mfm-runtime side_effect_driver
cargo test -p mfm-store --test commit_contract side_effect
cargo test -p mfm-integration-tests --test proof_transport_conformance
rg -n "SideEffectClaimAuthority|RunnerSideEffectBinding|fencing_token|invocation_epoch|ledger_key" crates/adapters crates/transports crates/kernel/runtime
```

Exit gate:

- Durable side-effect claim authority is minted by runtime/store, not adapters.

## Commit 12: `side effects: cover recovery and delete old side effect fixtures`

Deletion target:

- Delete side-effect test fixtures that duplicate the generic driver for valid behavior.
- Delete unsupported generic driver branches or implement them if they are required by current
  lifecycle semantics.

Replacement:

- Proof and EVM recovery/unknown-submission/not-submitted/ambiguity/failure/output-after-confirmation
  goldens.
- EVM runner-output and executable-identity goldens.
- Valid side-effect tests use the driver; invalid side-effect tests use named negative fixtures.

Do not keep:

- Generic driver support that rejects a lifecycle phase runtime classifies as recoverable.
- Parallel valid side-effect fixture builders in runtime/app/integration support.

Validation:

```bash
cargo test -p mfm-runtime side_effect
cargo test -p mfm-store --test commit_contract side_effect
cargo test -p mfm-integration-tests --test proof_transport_conformance
cargo test -p mfm-integration-tests --test parity_evm_contract_lifecycle_reth -- --ignored
rg -n "side_effect_.*builder|SideEffectInvocationPrepared\\(|unsupported.*Prepared|not submitted|ambiguity" crates tests
```

Exit gate:

- Side-effect driver behavior is golden-covered across proof and EVM.
- Old valid side-effect fixture implementations are deleted.

## Commit 13: `events store: promote fact recorded declarations to production`

Deletion target:

- Delete check-only `FactRecorded` declaration scaffolding that duplicates production behavior.
- Delete handwritten `FactRecorded` descriptor, artifact requirement, store codec, and projection
  transition pieces after the declaration becomes authoritative.

Replacement:

- `FactRecorded` schema descriptor, artifact lenses/requirements, store payload codec, and projection
  transition inputs are generated or derived from one declaration.
- Tests compare byte-stable output against existing goldens.

Do not keep:

- A production handwritten path and a test-only declaration path for the same `FactRecorded`
  contract.

Validation:

```bash
cargo test -p mfm-events fact_recorded
cargo test -p mfm-store --test commit_contract fact_recorded
cargo test -p mfm-replay fact_recorded
cargo test -p mfm-runtime fact_recorded
rg -n "FACT_RECORDED_.*DECLARATION|FactRecordedProjectionTransitionDeclaration|fact_recorded.*handwritten|FactRecorded" crates/kernel/events crates/kernel/store crates/kernel/replay crates/kernel/runtime
```

Exit gate:

- `FactRecorded` has one production declaration source.

## Commit 14: `events store: promote state attempt started declarations`

Deletion target:

- Delete check-only `StateAttemptStarted` event family, codec, and projection declaration scaffolding
  if it is not promoted.
- If promoted, delete the parallel handwritten schema/codec/projection inputs it replaces.

Replacement:

- `StateAttemptStarted` descriptor, store codec inputs, and projection transition inputs are generated
  or derived from one declaration.

Do not keep:

- Check-only declaration code with no production replacement.

Validation:

```bash
cargo test -p mfm-events state_attempt_started
cargo test -p mfm-store --test commit_contract state_attempt_started
cargo test -p mfm-runtime state_attempt_started
rg -n "STATE_ATTEMPT_STARTED_.*DECLARATION|StateAttemptStartedProjectionTransitionDeclaration|StateAttemptStarted" crates/kernel/events crates/kernel/store crates/kernel/runtime
```

Exit gate:

- `StateAttemptStarted` declaration code is either production authority or gone.

## Commit 15: `store postgres: delete non authoritative projection persistence`

Deletion target:

- Delete Postgres projection tables and projection writers if stream-authoritative reads are now the
  intended model.
- Delete projection-table repair/repopulate code.

Replacement:

- Reads rebuild from `typed_run_events`.
- If a cache is needed for performance, this commit must stop and produce a separate projection
  persistence RFC instead of silently keeping non-authoritative projection persistence.

No compatibility:

- Existing database compatibility is not required.
- Initial migration and schema may be rewritten.
- Do not add compatibility migrations to preserve old projection tables.

Validation:

```bash
cargo test -p mfm-store --test commit_contract
cargo test -p mfm-integration-tests --test rest_api_run_control
cargo test -p mfm-integration-tests --test parity_rest_api_postgres_typed_smoke -- --ignored
rg -n "projection_.*table|typed_run_.*projection|clear_projection|repopulate|projection rows" crates/storages/stream-store-postgres crates/kernel/store crates/app
```

Exit gate:

- Projection persistence is either deleted or explicitly moved to a separate RFC before any code
  remains.

## Commit 16: `docs: update rfc deletion gate ledger`

Scope:

- Update `RFC_REFACTORS_CLEANUPS.md` with the actual deletion results.
- Update the LOC ledger with before/after numbers by phase.
- Mark each phase as one of:
  - deletion-complete
  - production path closed but fixture gate open
  - baseline/hardening only
  - deferred

Do not keep:

- Stale "closed" wording for phases that still retain old surfaces.
- Claimed LOC reduction that includes docs-only or baseline-only additions.

Validation:

```bash
git diff --check -- RFC_REFACTORS_CLEANUPS.md PLAN_IMPL_RFC_DELETION_GATE.md
```

Exit gate:

- The RFC accounting matches the actual code state.

## Final Validation Gate

Run after all commits:

```bash
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo test --workspace
cargo test -p mfm-integration-tests --test cargo_metadata_contract
cargo test -p mfm-integration-tests --test architecture_namespace_contract
cargo test -p mfm-integration-tests --test proof_transport_conformance
cargo test -p mfm-integration-tests --test rest_api_run_control
nix run .#ci
```

If `nix run .#ci` fails because a service dependency is unavailable locally, collect the failing
check name and logs before stopping.

## Known Break Points

- Deleting raw stream helpers will break app tests that construct stream responses from event slices.
  Migrate those tests to `RunServices::run_stream` or a verified read context in the same commit.
- Removing sync store convenience use will break many runtime/app/integration fixtures. Migrate
  ordinary tests before adding strict source guards.
- Removing `build_committed_batch` will break forged-stream tests across runtime, app, replay, store,
  and integration. Move only approved corruption cases to named negative fixture support.
- Replacing role-policy matches can break replay and commit-contract negative tests. Move expected
  policy data to `ArtifactRoleContract` before deleting local matches.
- Deleting adapter-minted side-effect authority too early will break recovery and ambiguity behavior.
  Runtime/store authority must land before adapter cleanup.
- Protocol generation must be per event family. Do not delete broad store codec/projection matches
  without family-specific descriptor, canonical JSON, spec-hash, role, runtime, replay, and projection
  goldens.

## Source Scans That Must Be Clean Or Explained

Run after the commit series:

```bash
rg -n "TypedRunEventStore|InMemoryTypedRunStore|build_committed_batch|typed_run_stream_response_from_events|run_status_with_projection" crates bin tests
rg -n "SideEffectClaimAuthority|RunnerSideEffectBinding|unsupported.*Prepared|not_submitted|unknown_submission" crates bin tests
rg -n "EventFamilyDeclaration|EventCodecDeclaration|ProjectionTransitionDeclaration|check-only|handwritten" crates/kernel/events crates/kernel/store
rg -n "parse_artifact_role|artifact_role_str|ArtifactRole::" crates tests
```

Allowed explanations must name an approved low-level contract or negative fixture. Otherwise delete
the match.

## Architect-Agent Escalation Points

Spawn an architect-agent before keeping any of these:

- A public API used only by tests.
- A sync store helper outside `mfm-store` internals.
- A forged-stream helper outside a named corruption fixture.
- A check-only declaration that does not replace production code in the same phase.
- An adapter-owned side-effect authority field.
- A projection persistence writer/table after stream-authoritative reads are available.

When spawning, pass these rules exactly:

- No backward compatibility is required.
- Breaking changes are allowed.
- Test-only and unused implementations should be removed.
- The phase counts as cleanup only when old displaced code is deleted.
- Retained exceptions must be named as low-level corruption/contract fixtures or authority-bearing
  production paths.
