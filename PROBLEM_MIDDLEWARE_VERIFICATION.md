# Middleware And Verification Problem

Date: 2026-05-28
Branch reviewed: `rfc-state-ops`

Status: architecture/problem review; documentation only; no implementation changes in this note.

This note reviews the remaining persistence, middleware, lifecycle, and verification ownership
problems in the typed runtime branch. It should be read together with:

- `RFC_STATE_OPS_PROBLEM.md`
- `RFC_TYPED_CORE_PROPOSAL_1.md`
- `PROBLEM_IMPLEMENTATION_TYPED_CORE.md`
- `DECISION_TYPED_CORE_CORRECTIVE_ARCHITECTURE.md`

## Executive Summary

The current branch is materially stronger than the earlier typed-core review captured in
`PROBLEM_IMPLEMENTATION_TYPED_CORE.md`: public generic start now accepts a certified bundle and
`mfm-app` calls `mfm_certify::verify_certified_bundle_with_trusted_registry` before constructing a
`CertifiedRuntimeSpec`. `CertifiedRuntimeSpec::new` now requires `CertifiedTypedSpec`, not a
hash-only envelope. That closes the biggest previous runtime-authority bypass.

The remaining problem is different: persistence and lifecycle authority are still split across app
helpers, runtime scheduler methods, erased transport runners, artifact-store methods, typed-store
methods, test fixtures, and conformance helpers. The code has the shape of mandatory runtime
middleware, but not all durable bytes and framework lifecycle transitions pass through one
state-execution wrapper.

The central leak is:

```text
state execution returns event payload evidence
transport/app code persists bytes and framework artifacts by convention
runtime validates runner output and appends typed events
store validates event/evidence preconditions
```

The target remains:

```text
state = executable runtime unit
  user state
  sealed framework state

all run-mutating work executes through mandatory runtime middleware
only store commits make persisted bytes authority
artifact bytes may be staged before commit, but staged/orphan bytes are never authority
```

The branch already contains the precedent needed to avoid inventing a new public runtime-transition
API. `NodeSpec` has `framework: Option<FrameworkNodeSpec>`, and current certified specs already
contain framework nodes for same-value bridges and public-output rendering. The problem is that
run start, run completion, retention projection, public-output receipt persistence, and some
bootstrap/config staging are still special paths rather than sealed framework state executions.

## Current Code Evidence Map

### Certified Authority Boundary Is Mostly Corrected

Current evidence no longer supports the old claim that generic app/CLI/REST start wraps raw spec
JSON directly into runtime authority:

- `crates/kernel/runtime/src/lib.rs:235` defines `CertifiedRuntimeSpec::new` over
  `CertifiedTypedSpec`.
- `crates/app/src/lib.rs:1542` defines `verify_certified_bundle_run_start_request`.
- `crates/app/src/lib.rs:1550` calls
  `mfm_certify::verify_certified_bundle_with_trusted_registry`.
- `crates/app/src/lib.rs:1565` defines
  `build_certified_typed_run_start_request` for already certifier-backed authority.
- `bin/cli/src/commands/run/start.rs:111` uses
  `verify_certified_bundle_run_start_request`.
- `bin/rest-api/src/lib.rs:583` uses `verify_certified_bundle_run_start_request`.
- `crates/app/src/lib.rs:1184` loads stored spec and certificate artifacts, verifies the bundle,
  and only then returns `CertifiedTypedSpec`.
- `crates/app/src/lib.rs:1215` constructs runtime authority from that verified value.
- `crates/app/src/lib.rs:2545` and `crates/app/src/lib.rs:2604` test invalid bundle rejection
  before `RunStarted`.

This note does not reopen the previous P0 as current evidence. The remaining findings assume the
certified bundle boundary is the intended baseline.

### Framework-State Precedents Already Exist

The current branch has concrete framework-state machinery:

- `crates/kernel/spec/src/lib.rs:883` stores `NodeSpec.framework`.
- `crates/kernel/spec/src/lib.rs:954` defines `FrameworkNodeSpec::{Bridge, PublicOutputRender}`.
- `crates/kernel/spec/src/lib.rs:1860` parses only those framework node kinds.
- `crates/kernel/certify/src/lib.rs:1046` lowers bridge nodes with framework config refs and
  framework descriptors.
- `crates/kernel/certify/src/lib.rs:1151` lowers a public-output render node into the certified
  node list.
- `crates/kernel/certify/src/lib.rs:1866` validates built-in framework state descriptors against
  framework authority.
- `crates/kernel/certify/src/lib.rs:2445` validates framework node metadata.
- `crates/kernel/runtime/src/lib.rs:781` resolves `PublicOutputRender` as a built-in runtime
  runner rather than a transport-registered domain runner.
- `crates/kernel/runtime/src/lib.rs:884` executes the framework public-output runner.
- `crates/kernel/runtime/src/lib.rs:5595` tests that runtime specs require the framework
  public-output render node.
- `crates/kernel/certify/src/lib.rs:4527` tests that forged framework descriptors are rejected.

These anchors are important. They show that framework states are already part of the certified
state-program model; the missing work is generalizing lifecycle work into that model where it fits.

### Current State-Execution Middleware Shape

The runtime already has a strong middleware core around normal node execution:

- `crates/kernel/runtime/src/lib.rs:1833` starts `run_node_attempt`.
- `crates/kernel/runtime/src/lib.rs:1861` appends `StateAttemptStarted`.
- `crates/kernel/runtime/src/lib.rs:1919` calls `validate_runner_output`.
- `crates/kernel/runtime/src/lib.rs:1929` binds staged retention refs.
- `crates/kernel/runtime/src/lib.rs:1936` records required artifact evidence.
- `crates/kernel/runtime/src/lib.rs:1949` appends the terminal typed commit.
- `crates/kernel/runtime/src/lib.rs:1953` repeats the same structure for async stores.
- `crates/kernel/runtime/src/lib.rs:4402` validates runner output against certified node, attempt,
  side-effect, public-output, and scheduler-owned payload rules.

This is the right structural center. The leak is that runners persist bytes before returning
`ErasedRunnerOutput`, while the middleware only receives evidence and payloads.

### Current Persistence Surfaces

Searches for `persist_`, `put_artifact`, `put_verified_artifact`,
`record_artifact_evidence`, `append_typed_run_commit`, `required_artifacts`, and
`staged_retention_refs` identify these persistence-shaped surfaces:

- `crates/storages/artifact-store-fs/src/lib.rs:166` writes bytes and derives artifact evidence.
- `crates/storages/artifact-store-fs/src/lib.rs:189` writes bytes after verifying supplied
  evidence.
- `crates/kernel/store/src/lib.rs:1432` and `crates/kernel/store/src/lib.rs:1435` define the
  typed store evidence and commit APIs.
- `crates/kernel/store/src/lib.rs:730` puts `required_artifacts` on `TypedCommitRequest`.
- `crates/kernel/store/src/lib.rs:1722` validates commit preconditions.
- `crates/kernel/store/src/lib.rs:1813` stages a typed commit after payload/evidence validation.
- `crates/kernel/store/src/lib.rs:1976` records artifact evidence in the in-memory store.
- `crates/storages/stream-store-postgres/src/typed.rs:266` records artifact evidence in the
  durable store.
- `crates/storages/stream-store-postgres/src/typed.rs:287` appends a typed commit in the durable
  store.
- `crates/app/src/lib.rs:619` and `crates/app/src/lib.rs:647` expose app-level spec/config
  artifact persistence helpers.
- `crates/app/src/lib.rs:314` forwards proof transport runner writes to
  `FsTypedArtifactStore::put_verified_artifact`; this is a production sink adapter, not merely a
  test helper.
- `crates/app/src/lib.rs:1255`, `crates/app/src/lib.rs:1283`, and
  `crates/app/src/lib.rs:1353` persist spec, certificate, and seed artifacts for launch.
- `crates/app/src/lib.rs:1322` loads config artifact evidence for launch and verifies it against
  certified `ConfigRef` metadata.
- `crates/app/src/lib.rs:1670` persists framework public-output receipt artifacts after events
  already exist.
- `crates/app/src/lib.rs:870` and `crates/app/src/lib.rs:1158` build/persist retention manifest
  artifacts and call scheduler projection methods.
- `crates/kernel/runtime/src/lib.rs:1569` and `crates/kernel/runtime/src/lib.rs:1649` append
  `RunStarted`.
- `crates/kernel/runtime/src/lib.rs:2111` and `crates/kernel/runtime/src/lib.rs:2150` append
  `RunCompleted`.
- `crates/kernel/runtime/src/lib.rs:2172` and `crates/kernel/runtime/src/lib.rs:2238` append
  retention manifest projection events.
- `crates/kernel/runtime/src/lib.rs:1053` rebuilds public-output receipt artifact bytes.
- `crates/kernel/runtime/src/lib.rs:2332` builds retention manifest artifact bytes.
- `crates/transports/proof/src/lib.rs:707`,
  `crates/transports/portfolio/src/lib.rs:713`, and
  `crates/transports/evm-dcv/src/lib.rs:1969` define transport-local `persist_artifact` helpers.
- `crates/transports/evm-dcv/src/lib.rs:362` persists side-effect intent artifacts.
- `bin/cli/src/commands/portfolio/snapshot.rs:165`,
  `bin/rest-api/src/lib.rs:793`, and `tests/integration/tests/support/mod.rs:242` persist typed
  config artifacts outside the runtime.
- `tests/integration/tests/rest_api_run_control.rs:489` and `:516` persist program and framework
  config artifacts directly in REST control fixtures.
- `tests/integration/tests/parity_rest_api_evm_reth_pipeline.rs:60` persists EVM DCV config
  artifacts for the parity pipeline.
- `crates/app/src/lib.rs:3074` and `:3101` contain app test helpers that manually persist draft and
  framework config artifacts.
- `crates/transports/proof/src/lib.rs:1352`, `crates/transports/proof/src/lib.rs:1400`, and
  `crates/transports/proof/src/lib.rs:1467` manually persist conformance spec/config/public-output
  artifacts.
- `crates/kernel/test-support/src/lib.rs:2088` and `:2236` contain test-support side-effect
  persistence helpers.

Search false positives or non-runtime cases:

- `crates/core/src/keystore/tests.rs:550` is a keystore audit-record persistence test name, not a
  typed runtime persistence helper.
- `bin/cli/tests/json_output_integration.rs:22` and `:28` verify CLI JSON test responses, not
  runtime authority.
- `bin/cli/src/support/keystore.rs` validation helpers are keystore/CLI input checks, not typed
  runtime middleware.

## Persistence Leak Classification

| Current surface | Evidence | Classification | Target owner | Notes |
| --- | --- | --- | --- | --- |
| `FsTypedArtifactStore::put_artifact` / `put_verified_artifact` | `crates/storages/artifact-store-fs/src/lib.rs:166`, `:189` | Not a leak by itself | Artifact store | This is the permanent byte/evidence integrity boundary. The leak is who calls it during state execution. |
| `TypedRunEventStore::record_artifact_evidence` | `crates/kernel/store/src/lib.rs:1432`, `crates/storages/stream-store-postgres/src/typed.rs:266` | Not a leak by itself, but currently broadly reachable | Store/middleware | Store-owned evidence staging is valid. Follow-up must decide whether evidence recording and commit append should become one atomic store API for run-mutating paths. |
| `TypedRunEventStore::append_typed_run_commit` | `crates/kernel/store/src/lib.rs:1435`, `crates/storages/stream-store-postgres/src/typed.rs:287` | Not a leak by itself | Store | This is the append-only authority boundary. Callers outside runtime/test fixtures should be rare. |
| REST `InMemoryAsyncTypedRunStore` forwarding impl | `bin/rest-api/src/lib.rs:154`, `:166` | Not a persistence leak by itself | Store adapter | This adapts the store trait for tests/dev. It is acceptable if production routing still reaches runtime/app authority first. |
| `TypedAppServices::persist_certified_spec` | `crates/app/src/lib.rs:619` | App persistence leak | `BootstrapRun` framework state materialization | Public app facade can persist spec bytes without directly starting a run. That may be byte staging today, but durable launch authority must come from the sealed bootstrap framework batch. |
| `TypedAppServices::persist_config_artifact` | `crates/app/src/lib.rs:647` | App/input staging leak | Certification input materialization or bootstrap middleware | Verifies config evidence against `ConfigRef`; needed, but owner should be a runtime/certification input materializer rather than ad hoc app API. |
| App `FsProofArtifactSink` and transport artifact sink adapters | `crates/app/src/lib.rs:314`, `crates/transports/proof/src/lib.rs:55`, `crates/transports/portfolio/src/lib.rs:64`, `crates/transports/evm-dcv/src/lib.rs:69` | Production persistence capability leak | Sealed per-attempt staged artifact writer + runtime middleware | App gives runner code a direct artifact-store write channel. Production runners should not receive any trait that can call `put_verified_artifact` directly. |
| `persist_certified_spec_artifact` | `crates/app/src/lib.rs:1255` | Bootstrap persistence leak | `BootstrapRun` framework state materialization | Persists bytes before `RunStarted`; acceptable only as non-authoritative CAS staging until the sealed first bootstrap batch binds it. |
| `persist_certified_spec_certificate_artifact` | `crates/app/src/lib.rs:1283` | Bootstrap persistence leak | `BootstrapRun` framework state materialization | Same as spec artifact; must remain certifier-backed and bound by the sealed first bootstrap batch. |
| `load_config_artifacts_for_spec` | `crates/app/src/lib.rs:1322` | App-owned launch materialization | Bootstrap/input materialization boundary | Not a write, but it decides which pre-existing config artifacts become launch evidence. Future owner should be the same boundary that owns config byte staging. |
| `persist_seed_inputs_for_spec` | `crates/app/src/lib.rs:1353` | Bootstrap persistence leak | Bootstrap framework state/middleware | Seed materialization is a real launch concern. Current app helper owns digest and producer evidence checks. |
| CLI/REST portfolio config persistence | `bin/cli/src/commands/portfolio/snapshot.rs:165`, `bin/rest-api/src/lib.rs:793` | Transport-local artifact staging | Input materialization boundary | Domain route config bytes are persisted before `build_certified_typed_run_start_request`. This is not runtime authority until commit, but it is a hidden lifecycle path. |
| Integration support config persistence | `tests/integration/tests/support/mod.rs:242`, `tests/integration/tests/rest_api_run_control.rs:489`, `:516`, `tests/integration/tests/parity_rest_api_evm_reth_pipeline.rs:60` | Test fixture bypass | Test support only, or same production helper | Tests may need corruption fixtures, but positive fixtures should exercise the same production path. Framework config persistence in fixtures is especially easy to mistake for production lifecycle logic. |
| App unit-test config persistence helpers | `crates/app/src/lib.rs:3074`, `:3101` | Test fixture bypass | Test support only | Keep for targeted tests only; do not let fixture helper shape drive production app APIs. |
| Kernel test-support side-effect helpers | `crates/kernel/test-support/src/lib.rs:2088`, `:2236` | Test fixture bypass | Test support only | These helpers are useful for runtime/store tests but must not become production lifecycle patterns. |
| Operation `*_framework_config_artifacts` helpers | `crates/ops/portfolio-tracker-op/src/lib.rs:378`, `crates/ops/evm-deploy-configure-validate-op/src/lib.rs:252` | Build/stage leak under another name | Certification/lowering or bootstrap materialization | These build framework config artifacts introduced by certification. Useful evidence, but framework config byte materialization should be owned by framework lowering/materialization. |
| `persist_framework_public_output_receipts` | `crates/app/src/lib.rs:1670` | Framework output persistence leak | Public-output framework state middleware | Public-output render is already a framework state, but receipt bytes are repaired/persisted later by app code. |
| `build_public_output_receipt_artifact` | `crates/kernel/runtime/src/lib.rs:1053` | Artifact builder, not persistence by itself | Framework middleware | Correct location may remain runtime, but callers should be middleware/framework-state code, not app repair paths. |
| Retention projection app path | `crates/app/src/lib.rs:870`, `crates/app/src/lib.rs:1158` | Lifecycle persistence leak | Retention projection framework state | App decides when to project, persists manifest bytes, then calls scheduler. |
| `build_retention_manifest_artifact` | `crates/kernel/runtime/src/lib.rs:2332` | Artifact builder, not persistence by itself | Retention projection framework state/middleware | Must be fed from authoritative stream projection. |
| `SerialTypedScheduler::start_run` / async | `crates/kernel/runtime/src/lib.rs:1569`, `:1649` | Lifecycle commit outside node execution | `BootstrapRun` framework state first batch | It validates evidence, records artifacts, appends `RunStarted` and initial retention refs. Target shape is a sealed first `TypedCommitRequest` that also records the bootstrap framework attempt/receipt after `RunStarted`. |
| `SerialTypedScheduler::complete_run` / async | `crates/kernel/runtime/src/lib.rs:2111`, `:2150` | Lifecycle commit outside framework state | `CompleteRun` framework state output | Completion is derived from public-output projection and should not require app/user code. Target shape is normal framework attempt output that middleware converts into the existing `RunCompleted` event in the same commit. |
| `append_retention_manifest_projection` / async | `crates/kernel/runtime/src/lib.rs:2172`, `:2238` | Lifecycle commit outside framework state | `ProjectRetentionManifest` framework state | It validates manifest bytes/evidence and appends `RetentionManifestProjected` plus retention refs. |
| Normal `run_node_attempt` wrapper | `crates/kernel/runtime/src/lib.rs:1833`, `:1953` | Good middleware skeleton, incomplete byte ownership | Runtime state middleware | It validates output, records evidence, binds retention refs, and appends commits. It should receive staged bytes, not just evidence. |
| `bind_staged_retention_refs` | `crates/kernel/runtime/src/lib.rs:2503` | Good middleware precedent | Runtime state middleware | Normal runner retention refs are already converted into scheduler-owned `RetentionRefsAppended` payloads by middleware. The leak is bootstrap/manifest retention refs and direct artifact writes, not this binding pattern. |
| Transport `persist_artifact` helpers | `crates/transports/proof/src/lib.rs:707`, `crates/transports/portfolio/src/lib.rs:713`, `crates/transports/evm-dcv/src/lib.rs:1969` | State-execution persistence leak | Sealed staged artifact writes + runtime middleware | Runners persist fact, output, side-effect, receipt, confirmation, and diagnostic artifacts directly. Production runners should instead return staged artifacts/handles whose authority is bound by middleware. |
| `persist_side_effect_intent` | `crates/transports/evm-dcv/src/lib.rs:362` | Side-effect artifact persistence leak | Side-effect state middleware | Side-effect intent bytes must be durable before ledger events, but the framework should own the managed write. |
| Proof conformance artifact persistence | `crates/transports/proof/src/lib.rs:1352`, `:1400`, `:1467` | Conformance-only hidden lifecycle path | Shared app-agnostic runtime/framework helpers or fixture-only scaffolding | Conformance may test lower-level runtime/transport contracts, but it must converge on the same sealed lifecycle primitives and not teach a second production model. |

## Verification Ownership Classification

| Current verification surface | Evidence | Future owner | Temporary leak or permanent defense | Notes |
| --- | --- | --- | --- | --- |
| Bundle parsing helpers | `crates/app/src/lib.rs:1468`, `:1480` | Transport boundary | Permanent input parsing defense | They correctly return untrusted bytes only. |
| `verify_certified_bundle_run_start_request` | `crates/app/src/lib.rs:1542` | App assembly calling certifier verifier | Permanent serialized-boundary defense | Current location is acceptable as assembly if it stays thin; the authority is minted by `mfm-certify`. |
| `mfm_certify::verify_certified_bundle*` | `crates/kernel/certify/src/lib.rs:652`, `:666` | Certifier | Permanent persisted-data defense | Persisted spec/certificate bytes are hostile forever. |
| `mfm_certify::validate_typed_spec` and children | `crates/kernel/certify/src/lib.rs:1468` through `:3055` | Certifier | Both compile-time backstop and permanent persisted defense | Rust-authored workflows should make many authoring failures unrepresentable, but the certifier must keep defending stored/imported specs. |
| `CertifiedRuntimeSpec::validate_runtime_contract` | `crates/kernel/runtime/src/lib.rs:384` | Runtime construction boundary | Permanent defense in depth | Runtime-only executable shape checks remain valid even after certification. |
| `validate_public_output_render_contract` | `crates/kernel/runtime/src/lib.rs:511` | Runtime construction boundary | Permanent framework-state defense | Needed because runtime has a built-in renderer and must prove the certified node matches it. |
| `validate_launch_artifacts` | `crates/app/src/lib.rs:1618` | Bootstrap middleware/framework state | Permanent defense, current owner wrong | Verifies persisted spec/certificate/seed/config bytes before start. It should not be an ad hoc app helper. |
| `validate_spec_artifact_evidence` / `validate_certificate_artifact_evidence` / `validate_run_started_matches_spec` | `crates/app/src/lib.rs:2038`, `:2062`, `:2084` | Runtime/certifier read boundary | Permanent persisted-history defense | These rebuild trust from `RunStarted`; app should call a boundary, not own all matching rules. |
| `load_certified_spec_for_run` | `crates/app/src/lib.rs:1184` | Runtime/certifier read/resume boundary | Permanent persisted-history defense | Correctly verifies stored bundle today; should become a reusable authority loader. |
| `load_config_artifacts_for_spec` | `crates/app/src/lib.rs:1322` | Bootstrap/input materialization boundary | Permanent launch-input defense, current owner too ad hoc | It verifies pre-existing config artifact evidence before launch. This remains necessary, but should share the owner that persists/materializes config bytes. |
| `validate_run_stream` and historical validators | `crates/kernel/runtime/src/lib.rs:1491`, `:3248` | Runtime resume/read boundary | Permanent persisted-history defense | Required against corrupt or hostile streams. |
| `validate_seed_cells` | `crates/kernel/runtime/src/lib.rs:3162` | Bootstrap/runtime boundary | Permanent persisted-history defense | Defends `RunStarted` seed evidence. |
| `validate_spec_artifact`, `validate_certificate_artifact`, `validate_config_artifacts` | `crates/kernel/runtime/src/lib.rs:4288`, `:4315`, `:4344` | Bootstrap runtime boundary | Permanent defense | These are correctly in runtime, but currently called by scheduler start methods outside normal node execution. |
| `validate_runner_output` | `crates/kernel/runtime/src/lib.rs:4402` | Mandatory state middleware | Permanent runner-defense | Some checks may shrink with typed framework outputs, but erased runner output must remain defended. |
| `validate_runner_side_effect_payload` / `validate_side_effect_resume_output` | `crates/kernel/runtime/src/lib.rs:4653`, `:4722` | Side-effect runtime middleware | Permanent side-effect defense | External effects and recovery cannot be proven only by types. |
| `validate_public_output` / `validate_public_output_render_node` | `crates/kernel/runtime/src/lib.rs:4829`, `:4873` | Framework-state middleware | Permanent framework output defense | These remain as the runtime counterpart to public-output certification. |
| `verify_public_output_read_authority` | `crates/app/src/lib.rs:1751` | Runtime public-output read boundary | Permanent persisted-history defense | Current non-forgeable `PublicOutputReadAuthority` is the right shape; owner should move out of app if possible. |
| `verify_public_output_cell_evidence` and cache evidence checks | `crates/app/src/lib.rs:1884`, `crates/app/src/lib.rs:1941` | Public-output read boundary + artifact store | Permanent artifact defense | Rendered JSON is a cache; evidence must be checked on every read. |
| `replay_authority_for_run` | `crates/app/src/lib.rs:1219` | Replay boundary | Permanent replay defense | Current app helper reconstructs retained artifact evidence; should be a runtime/replay authority constructor. |
| `ReplayBroker::from_run_stream` and replay contract validators | `crates/kernel/replay/src/lib.rs:386`, `:412`, `:706`, `:1200`, `:1369`, `:1471`, `:1588`, `:1677` | Replay boundary | Permanent persisted-history/external-evidence defense | Replay correctly distrusts streams and retained evidence, but `ReplayAuthority::new` / `from_certified_spec` at `crates/kernel/replay/src/lib.rs:157` and `:174` are public and do not themselves prove certifier authority. Production replay authority should require `CertifiedTypedSpec` or a replay-specific sealed wrapper minted from it. |
| `verify_deterministic_proof_replay` | `crates/transports/proof/src/lib.rs:1034` | Domain replay verifier | Permanent external evidence defense | Replay verifiers remain domain-specific. |
| `verify_evm_dcv_replay` and fact replay validators | `crates/transports/evm-dcv/src/lib.rs:993`, `:1252`, `:1323`, `:1484` | Domain replay verifier | Permanent external evidence defense | Validates EVM facts, side-effect receipts, and typed replay inputs. |
| Transport config/input loaders that validate without `validate_` names | `crates/transports/proof/src/lib.rs:729`, `:608`; `crates/transports/portfolio/src/lib.rs:501`, `:526`; `crates/transports/evm-dcv/src/lib.rs:1978`, `:2003`, `:2041` | Runtime state middleware or domain runner boundary | Mixed | These check config refs, input-cell shape, and artifact evidence while running states. Some are permanent hostile-data checks; some should move left if middleware materializes typed inputs and config values before invoking runners. |
| Store payload/precondition/evidence validators | `crates/kernel/store/src/lib.rs:1722`, `:1813`, `:2133`, `:2161`, `:2278` | Store | Permanent append-only defense | Store must defend atomicity and event/projection consistency. |
| Artifact-store evidence validators | `crates/storages/artifact-store-fs/src/lib.rs:254` and following | Artifact store | Permanent byte/evidence defense | Artifact stores must reject corrupt metadata and byte mismatches. |
| Identity/canonical/value primitive validators | `crates/kernel/ids/src/lib.rs:684`, `crates/kernel/canonical/src/lib.rs:602`, `crates/kernel/values/src/lib.rs:1384` | Primitive constructors/canonicalization layer | Permanent persisted-data defense | These are foundational value defenses, not middleware leaks. |
| Program input-binding validators | `crates/kernel/program/src/lib.rs:4540`, `:4573` | Typed authoring/lowering boundary | Mostly compile-time/authoring defense | These protect builder-produced input binding shape before certification. |
| Domain config validators | `crates/portfolio/model/src/portfolio.rs:594`, `crates/portfolio/model/src/aave.rs:416`, `crates/states/portfolio/src/lib.rs:1485`, `crates/states/evm-dcv/src/lib.rs:985` | Domain input/config layer | Permanent hostile-input defense | These validate user/external domain data, not runtime authority. |
| `validate_phase_alignment` | `crates/ops/evm-deploy-configure-validate-op/src/lib.rs:353` | Type system or certifier where expressible; op config validation for remaining config facts | Mixed | Matching network/control-scope across phases is partly authoring semantics and partly user config validation. |
| Proc-macro lifetime/schema validators | `crates/kernel/program-derive/src/lib.rs:1475`, `:1501` | Type/derive layer | Permanent compile-time defense | These are correctly left of certification. |
| REST sequence validation | `bin/rest-api/src/lib.rs:839` | REST request boundary | Permanent input defense | Query range validation is transport input validation, not runtime authority. |
| Runtime test-only `CertifiedRuntimeSpec::from_verified_envelope` | `crates/kernel/runtime/src/lib.rs:241` | Test fixture only | Test-only authority bypass | This is `#[cfg(test)]`; keep it confined to negative/runtime unit fixtures and do not mirror it in app, transport, or conformance production helpers. |
| Dependency-boundary and parity helper validators | `tests/integration/tests/cargo_metadata_contract.rs:92`, `tests/integration/tests/parity_rest_api_evm_reth_pipeline.rs:505`, `:517`, `:534` | Test evidence | Test-only | These verify CI/parity fixtures and should not be treated as runtime owners. |
| CLI/keystore validation helpers | `bin/cli/src/support/keystore.rs:395`, `:624`, `:752`, `:792` | CLI/keystore boundary | Not part of typed runtime middleware | Keep separate; security-sensitive but not a state-runtime ownership leak. |

## Temporary Architecture Leak vs Permanent Persisted-Data Defense

Not every `verify_*` or `validate_*` function is debt. The distinction is the owner.

Temporary architecture leaks:

- App code persists spec/certificate/seed/config artifacts and public-output receipts as mandatory
  runtime work.
- App code passes `FsTypedArtifactStore` through sink traits so transport runners can persist
  state-output, fact, side-effect, receipt, and confirmation artifacts directly.
- App code schedules retention projection and writes retention manifest bytes.
- Scheduler methods append `RunStarted`, `RunCompleted`, and retention projection events outside
  the `NodeSpec`/framework-node execution path.
- Framework public-output rendering is a certified framework state, but its receipt bytes are
  reconstructed and written later by app code.
- Transport runners validate certified config refs and input artifact evidence because config/input
  materialization is still split between runtime context, artifact store, and domain runner code.
- Public replay authority constructors can be called with hash-only spec envelopes; app currently
  supplies verified authority, but the replay type boundary does not encode that precondition.
- Tests and conformance fixtures manually persist framework config/spec/public-output artifacts in
  ways that can hide bypasses from production paths.

Permanent persisted-data defenses:

- Certified bundle verification at any serialized spec/certificate boundary.
- Artifact byte/digest/metadata verification.
- Store append preconditions, event ordering, idempotency, and projection rebuild.
- Runtime validation of historical run streams before resume, replay, public-output reads, or
  retention projection.
- Runtime validation of erased runner output before commit.
- Replay verification of facts, side-effect evidence, and retained artifacts.
- Domain validation of hostile user input and external-system observations.
- Config, seed, and input artifact byte checks at serialized/runtime boundaries, even if typed
  materialization moves earlier.

The expected result of the follow-up work is not fewer invariants. It is fewer arbitrary locations
where invariants are enforced.

## Framework States And Lifecycle Work

The current branch proves that framework states can be represented as certified `NodeSpec` values:

- Bridge nodes are lowered with `FrameworkNodeSpec::Bridge`, same-value lineage, deterministic
  input/output cells, and built-in framework descriptors.
- Public-output render nodes are lowered with `FrameworkNodeSpec::PublicOutputRender`, required
  public cells, a receipt output cell, managed platform-write effect evidence, and a built-in
  runtime runner.

That precedent should be the preferred unifying model for lifecycle work.

The existing `NodeSpec` shape matters when deciding whether lifecycle work fits:

- every certified node has a `config_ref`, `input_bindings`, an `output_cell`, effect/capability
  metadata, and deterministic predecessor evidence;
- `FrameworkNodeSpec` is closed today to `Bridge` and `PublicOutputRender`, so new lifecycle kinds
  would be sealed by extending that enum and its parser/certifier/runtime validators;
- normal node execution has an attempt-start/attempt-terminal envelope and a terminal cell;
- `RetentionRefsAppended` is already middleware-derived for normal runner output through
  `bind_staged_retention_refs`, so the model does not require user runners to mint retention
  events directly.

That means lifecycle work should be represented as sealed framework states first. When an event
shape such as `RunStarted` or `RunCompleted` has no node id today, middleware should derive the
current kernel event from a certified framework node output rather than adding a new public
"runtime transition" authoring surface.

### Public-Output Rendering

Public-output rendering is already a framework state:

- Certification inserts the render node in `lower_public_output_render_node`.
- Runtime resolves it as a built-in `FrameworkPublicOutputRunner`.
- `render_public_output` returns `CellProduced`, `PublicOutputProduced`, and
  `StateAttemptCompleted`, plus required receipt artifact evidence.

The remaining leak is byte persistence. The framework runner returns receipt artifact evidence, but
`persist_framework_public_output_receipts` later rebuilds and persists receipt bytes from the
stream. The definition to carry forward is:

- the sealed public-output framework runner is the conceptual producer of the receipt bytes;
- runner output should include staged receipt bytes or a sealed staged artifact handle, not only
  evidence;
- runtime middleware persists and verifies those bytes before recording artifact evidence and
  appending the terminal commit;
- deterministic rebuild from `PublicOutputProduced` remains a pre-commit validation check, not the
  normal post-commit app repair path.

### Run Completion

`RunCompleted` is currently scheduler-derived:

- `drive_once` detects public-output completion evidence.
- `complete_run` appends `RunCompleted` with a public-output logical-key precondition.

`RunCompleted` should remain a persisted kernel event because store projection currently uses it
to mark the run terminal. The unified definition is:

- the certifier lowers a sealed `CompleteRun` framework node after public-output rendering;
- that node uses the normal attempt lifecycle and produces a small framework completion receipt
  cell;
- runtime middleware recognizes that sealed framework output and appends the existing
  `RunCompleted { outcome: Completed(PublicOutputCompletionEvidence) }` payload in the same
  atomic commit as the completion node terminal output;
- user/domain runners still cannot emit `RunCompleted` directly.

This keeps the event schema stable while removing the ad hoc scheduler `complete_run` path.

### Retention Projection

Retention projection is a strong framework-state candidate:

- It depends on authoritative stream/projection evidence, not user state data.
- It builds a retention manifest artifact from the stream.
- It appends `RetentionManifestProjected` and `RetentionRefsAppended`.

The current `NodeSpec` input model is cell-oriented. Retention projection needs access to the
authoritative run stream/projection and prior retention state. That does not require a new public
runtime-transition API, but it may require a sealed framework state input kind or framework
capability that only runtime can provide.

Retention projection is not equivalent to a user state consuming normal cells. The source data is
the append-only stream plus the store projection rebuilt from it. If represented as a framework
state, its "input" should be runtime-owned projection authority, not a user-supplied context value.
That argues for a sealed framework input/capability inside `FrameworkNodeSpec`, not a new public
state authoring facility.

The definition to carry forward is:

- `ProjectRetentionManifest` is a sealed `FrameworkNodeSpec`;
- `NodeSpec` supplies identity, descriptor binding, output/receipt shape, and ordinary dependency
  triggers such as the public-output receipt cell;
- the actual manifest data source is a sealed runtime-projection input, not an
  `InputBindingNodeSpec`;
- middleware builds/persists the manifest artifact and derives `RetentionManifestProjected` plus
  `RetentionRefsAppended` from the sealed framework output.

This avoids exposing the run stream/projection as a public user-state input while still making the
lifecycle work part of the certified framework-state program.

### Bootstrap Persistence And `RunStarted`

Bootstrap persistence is the hard case. `RunStarted` currently:

- must be first in the run stream;
- binds spec hash, spec artifact id, certificate artifact id/digest, spec/certificate media,
  spec/lowering versions, descriptor identities, runner executables, adapter executables,
  canonicalizer identity, framework/source revisions, public-output schema id, and seed cells;
- is required before normal scheduler execution can validate historical stream authority;
- is the root evidence used by resume, replay, public-output reads, and retention projection.

The current flow is:

1. App verifies or receives `CertifiedTypedSpec`.
2. App persists spec and certificate artifacts.
3. App loads config artifacts and persists seed artifacts.
4. Runtime scheduler validates `RunStartEvidence`.
5. Scheduler records artifact evidence in the store.
6. Scheduler appends `RunStarted` plus initial `RetentionRefsAppended`.

This is not a normal node attempt. A normal node attempt assumes a run stream already exists and
uses `RuntimeRunView::from_stream`, which requires exactly one matching `RunStarted`.

## The Hard `RunStarted` Bootstrap Problem

`RunStarted` can be made compatible with the framework-state principle, but the current scheduler
cannot simply treat it as another ordinary node without special handling.

The cyclic pressure is real:

- The certified spec would need to include the bootstrap framework node if bootstrap is part of
  certified semantics.
- The bootstrap framework node would persist or bind the certified spec artifact.
- The certified spec artifact is content addressed by canonical bytes that include any bootstrap
  node included in the spec.
- Normal attempts need `RunStarted` before they can execute.
- `RunStarted` itself is the event that creates the run stream authority.

Definition to carry forward:

1. `BootstrapRun` is a sealed framework node in the certified spec.
2. It is hash-defining, but it must not contain post-hash launch evidence such as the spec artifact
   id, certificate artifact id, certificate digest, run id, executable identities, or concrete seed
   artifact evidence.
3. The first commit is still a normal `TypedCommitRequest` that produces normal
   `KernelEventEnvelope`s; it is only sealed by bootstrap-specific payload order and preconditions.
4. The first payload remains `RunStarted`, preserving the authority-root invariant. The same batch
   then records the bootstrap framework node attempt and terminal receipt.
5. It is still part of the framework-state model. The implementation may use a dedicated internal
   first-batch executor, but that executor is not a new public runtime transition API and must not
   be copied for completion, retention, public-output, replay, or diagnostics.

The spec-hash concern is manageable if the bootstrap node describes the bootstrap contract but does
not embed the content-addressed spec artifact id, spec digest, certificate artifact id, or
certificate digest. Those values are derived after canonical spec/certificate bytes exist and are
bound by the `RunStarted` payload. This is the same separation already used elsewhere: certified
spec semantics are hashed in the spec, while artifact identities bind persisted bytes at run time.

Sealed bootstrap framework batch:

```text
TypedCommitRequest with RequiredRunState::Absent
  RunStarted
  StateAttemptStarted(BootstrapRun)
  CellProduced(bootstrap receipt cell)
  StateAttemptCompleted(BootstrapRun)
  RetentionRefsAppended(reason = RunStarted)
```

This preserves the main rule: user and framework states are the executable runtime model. Bootstrap
differs only in first-batch scheduling because it creates the authority root needed by normal node
attempts.

## Hidden Lifecycle Paths To Audit

The following paths bypass or partially bypass the proposed unified middleware model and must be
included in follow-up work:

- Generic CLI run start:
  - `bin/cli/src/commands/run/start.rs:111` now verifies certified bundles correctly.
  - Still relies on app helper persistence for spec/certificate/seed artifacts.

- Portfolio CLI and REST starts:
  - `bin/cli/src/commands/portfolio/snapshot.rs:165` persists config artifacts.
  - `bin/rest-api/src/lib.rs:793` persists config artifacts.
  - `bin/rest-api/src/lib.rs:534` starts from already certified portfolio authority.

- Resume, replay, public-output reads:
  - `crates/app/src/lib.rs:767`, `:928`, and `:981` load/verify stored authority before use.
  - These are read/resume boundaries, not state executions, but their verification should be
    reusable runtime/replay authority code rather than app-local matching logic.
  - `crates/app/src/lib.rs:1712` builds typed run status from a stream. That is read-only
    inspection if the result is never used to mint execution/replay/public-output authority.

- Retention projection:
  - `crates/app/src/lib.rs:870` and `:1158` decide when to project and persist manifest bytes.
  - Runtime validates and appends through `append_retention_manifest_projection`.

- App-managed artifact sink adapters:
  - `crates/app/src/lib.rs:294` installs a proof artifact sink backed by `FsTypedArtifactStore`.
  - `crates/app/src/lib.rs:314` implements the sink by calling `put_verified_artifact`.
  - This is the production capability path that lets transport runners persist bytes during state
    execution. It is more important than the transport-local helper names because any equivalent
    sink adapter would preserve the same leak.

- Transport runners:
  - Proof, portfolio, and EVM DCV runners write artifacts directly through transport-local sinks
    before returning `ErasedRunnerOutput`.
  - Side-effect intent/receipt/confirmation artifact persistence is included in this class.
  - `load_config`, `load_input_cell`, `load_artifact_value`, `ensure_config`, and
    `ensure_struct_input_digest` are also part of the audit because they materialize and validate
    persisted runtime inputs inside transport code even when their names do not include
    `verify_` or `validate_`.

- Replay authority:
  - Production app code currently reaches `ReplayBroker::from_run_stream` after
    `load_certified_spec_for_run`.
  - `ReplayAuthority::from_certified_spec` and `ReplayBroker::from_run_stream` are public enough
    that tests/conformance can construct replay authority directly from stream/spec evidence.
  - Replay should require a certifier-backed authority type or replay-specific wrapper minted from
    one. App-level verification remains the assembly step, not the only authority boundary.

- Proof conformance:
  - `crates/transports/proof/src/lib.rs:1196` runs scheduler directly.
  - It manually persists conformance spec/config/public-output receipt artifacts.
  - It builds replay authority directly from stream retention evidence.

- Integration tests:
  - Test support manually persists config/framework config artifacts.
  - EVM parity support manually persists DCV config artifacts.
  - Some tests directly append corrupt commits to assert resume/read rejection. Those direct store
    paths should remain clearly test-only.
  - `crates/kernel/runtime/src/lib.rs:241` provides a `#[cfg(test)]`
    `CertifiedRuntimeSpec::from_verified_envelope` constructor. Its use is acceptable only for
    runtime unit fixtures that deliberately bypass certificate verification.

- Store implementations:
  - In-memory and Postgres stores both expose `record_artifact_evidence` and
    `append_typed_run_commit`.
  - Production runtime paths need atomic evidence admission and commit append so separately
    committed artifact evidence staging does not remain a production authority surface.

## Strengthened Acceptance Criteria

The architecture is corrected when these are all true:

- `rg "persist_artifact" crates/transports` finds no state-execution artifact writes from domain
  runners.
- Production artifact sink traits/adapters do not expose direct artifact-store persistence to
  runners. If a sink abstraction remains, it is a staged-write capability whose bytes become
  authoritative only through runtime middleware and a typed commit.
- `ErasedRunnerOutput` or its replacement carries typed staged bytes/managed writes in addition to
  evidence, and runtime middleware persists those bytes before append.
- App and transport code do not directly persist mandatory runtime artifacts such as public-output
  receipts, retention manifests, state output cells, facts, side-effect evidence, or seed material
  except through sealed bootstrap, typed input materialization, or staged-artifact middleware.
- Transport runners no longer own generic config/input artifact materialization unless the
  remaining checks are domain validation over already materialized typed runtime inputs.
- Public-output receipt bytes are persisted by the public-output framework state middleware, not
  repaired later by app code.
- Retention projection is represented as a sealed framework state with a sealed
  runtime-projection input, not app scheduling plus scheduler append.
- `RunCompleted` is derived from a sealed `CompleteRun` framework state output with no
  user/transport authority.
- `RunStarted` is represented by a certified `BootstrapRun` framework node executed through the
  sealed first bootstrap batch described above.
- User/domain runners cannot emit scheduler-owned lifecycle payloads. Framework lifecycle effects
  are returned as sealed framework outputs or allowed only for certified framework node kinds.
- Production typed store append admits artifact evidence and appends the referencing commit
  atomically. Low-level separate evidence staging is confined to tests, migrations, or corruption
  fixtures.
- `record_artifact_evidence` and `append_typed_run_commit` are called by runtime/store middleware
  and explicit test fixtures only; app, transport, CLI, REST, and conformance positive paths use
  higher-level authority APIs.
- Store commit validation still proves required artifacts exist, payloads match run/spec identity,
  terminal attempt/cell pairs are atomic, retention manifest pairs are atomic, and side-effect
  failure pairs are atomic.
- Orphaned artifact bytes or store artifact evidence never become resume, replay, public-output, or
  retention authority without a committed typed event.
- Public-output read authority is minted only after verified certified spec/certificate authority,
  validated run stream, rebuilt projection, and artifact evidence checks.
- Replay authority is minted only from verified certified authority plus retained evidence from the
  authoritative stream; production replay constructors encode that precondition by type.
- Run status and raw stream inspection helpers remain read-only views and are not accepted as
  runtime, replay, public-output, or retention authority.
- Positive CLI/REST/app/conformance paths use the same middleware/authority path; direct store
  mutation remains confined to negative/corruption tests.

## Follow-Up Audit Checklist

Use this checklist before implementation work:

1. Run the mandatory searches:

   ```text
   rg "persist_"
   rg "verify_"
   rg "validate_"
   rg "put_artifact"
   rg "put_verified_artifact"
   rg "record_artifact_evidence"
   rg "append_typed_run_commit"
   rg "RunStarted"
   rg "RunCompleted"
   rg "RetentionRefsAppended"
   rg "RetentionManifestProjected"
   rg "PublicOutputProduced"
   rg "FrameworkNodeSpec"
   rg "ErasedRunnerOutput"
   rg "required_artifacts"
   rg "staged_retention_refs"
   ```

   Then run the secondary ownership searches:

   ```text
   rg "ArtifactSink|put_verified_artifact"
   rg "ReplayAuthority::|ReplayBroker::from_run_stream"
   rg "from_verified_envelope"
   rg "load_config|load_input_cell|load_artifact_value"
   rg "ensure_config|ensure_struct_input_digest"
   rg "ensure_.*replay|verify_.*replay"
   ```

2. For every result, classify it as one of:

   ```text
   artifact store
   typed store
   certifier
   runtime state middleware
   sealed framework state
   sealed bootstrap framework batch
   runtime read/resume/replay boundary
   domain input validation
   transport/app leak
   test-only corruption fixture
   ```

3. For every `ErasedRunnerOutput`, answer:
   - Which bytes did the runner create?
   - Where were those bytes persisted?
   - Which evidence references them?
   - Which event payload binds them?
   - Which middleware verifies bytes/evidence before commit?
   - Can the runner produce the same authority without direct artifact-store access?

4. For every `build_*artifact` helper, answer:
   - Is it only a deterministic byte/evidence builder?
   - Who persists the bytes?
   - Who records the evidence in the typed store?
   - Who appends the event that makes the artifact authoritative?

5. For every lifecycle event, answer:
   - Is it produced by a user state, framework state, scheduler method, store helper, app helper,
     or test fixture?
   - Is the producer sealed?
   - Does certification know the lifecycle work exists?
   - Is the event bound to a certified node, a framework output, or the sealed bootstrap first
     batch?

6. For every `validate_*` or `verify_*`, assign the earliest sound owner:
   - type system;
   - derive/proc macro;
   - certifier;
   - runtime construction;
   - runtime state middleware;
   - sealed framework state;
   - runtime read/resume/replay boundary;
   - store;
   - artifact store;
   - domain input validation;
   - test fixture only.

7. For every app/CLI/REST/conformance positive path, confirm:
   - it cannot construct runtime authority from parsed spec JSON;
   - it cannot append `RunStarted` without certifier-backed authority;
   - it cannot bypass launch artifact verification;
   - it cannot rely on app-local receipt/retention repair after the fact.

8. For every artifact sink trait or adapter, confirm:
   - whether it writes bytes immediately or only stages bytes for middleware;
   - whether runners can choose arbitrary descriptors/evidence;
   - whether middleware revalidates the bytes/evidence before commit;
   - whether orphaned writes are ever treated as authority.

9. For every replay/read authority constructor, confirm:
   - whether the input authority was minted by the certifier or reconstructed from verified
     persisted spec/certificate artifacts;
   - whether raw stream inspection can be accidentally reused as authority;
   - whether lower-level constructors are `cfg(test)`, crate-private, or clearly documented
     low-level APIs.

10. For every transport config/input loader, confirm:
   - whether it is validating hostile domain data that must remain runner-local;
   - whether it is compensating for missing runtime typed input materialization;
   - whether evidence and byte checks happen before the runner receives the value;
   - whether the same checks run during replay.

11. For every test-only direct store mutation, confirm:
   - the test is explicitly negative/corruption/replay coverage;
   - no production helper copies that pattern;
   - the fixture name makes the bypass obvious.

## Resolved Architecture Definitions

The follow-up investigation narrows the earlier open questions into definitions. These are not
implementation designs; they are the architectural direction the implementation should preserve.

1. `BootstrapRun` can be part of the certified spec without a circular hash relationship.
   The node is hash-defining and declares the bootstrap contract, but it must not contain
   post-hash launch evidence. The dependency remains one-way:
   `TypedExecutionSpec including BootstrapRun -> spec_hash -> spec/certificate artifacts ->
   RunStarted evidence`.

2. Bootstrap uses normal event envelopes and normal attempt lifecycle payloads inside a sealed
   first bootstrap batch.
   `RunStarted` remains ordinal 0. The same `TypedCommitRequest` can then include
   `StateAttemptStarted(BootstrapRun)`, a real bootstrap receipt cell, `StateAttemptCompleted`, and
   initial retention refs. This is unification, not a separate event-envelope model.

3. Production store append should atomically admit artifact evidence and append the commit.
   Artifact bytes may be staged in the artifact store before commit, but typed-store artifact
   evidence should become authority only through the commit that references it. Separate
   `record_artifact_evidence` remains useful for tests, migrations, and corruption fixtures, not
   production lifecycle paths.

4. `ProjectRetentionManifest` is a sealed framework node with a sealed runtime-projection input.
   `NodeSpec` should provide identity, descriptor binding, receipt/output shape, and ordinary
   readiness dependencies such as the public-output receipt cell. The manifest data itself comes
   from the authoritative stream/projection, not from `InputBindingNodeSpec` or a public runner
   capability.

5. `RunCompleted` remains a kernel event, but it is derived from a sealed `CompleteRun` framework
   node output.
   The completion node uses normal attempt lifecycle events and produces a framework receipt cell.
   Middleware appends the existing `RunCompleted` payload in the same commit as the completion
   terminal output. User/domain runners still cannot emit `RunCompleted`.

6. Public-output receipt bytes are produced by the sealed public-output framework runner as staged
   artifact bytes or a sealed staged handle.
   Middleware persists and verifies those bytes before recording artifact evidence and appending
   the terminal commit. Deterministic rebuild from `PublicOutputProduced` is a validation check, not
   app-side post-commit repair.

7. App read-boundary authority should move down into kernel-owned read helpers without creating a
   new authoring API.
   App may still load concrete filesystem bytes and shape CLI/REST DTOs, but invariant-bearing
   helpers such as persisted certified-spec loading, replay authority construction, and
   public-output read authority minting should live in runtime/replay boundaries.

8. Conformance helpers are lower-level runtime/transport harnesses, not production app paths.
   They should not depend on `mfm-app`, because that would invert crate layering. They may bypass
   app to test lower-level contracts, but duplicated spec/config/receipt/run-start materialization
   should converge on the same app-agnostic runtime/framework primitives as production.

9. Artifact-store writes are managed platform writes even when they describe external facts or
   side effects.
   The external side effect is the RPC/read/transaction. Persisting fact, output, intent,
   submission, receipt, confirmation, and diagnostic bytes is platform persistence and should be
   mediated by runtime middleware with distinct staged binding kinds.

10. Separately persisted artifact bytes are acceptable non-authoritative CAS staging; separately
    recorded typed-store artifact evidence is not acceptable as production staging.
    If a commit fails, no new run-store evidence should remain. Store implementations may
    materialize evidence tables internally, but the authority relationship must be commit-derived.

11. Replay should require certifier-backed authority, not a hash-only spec envelope plus app
    convention.
    App verification remains the assembly step, but `ReplayAuthority::from_certified_spec` /
    `ReplayBroker::from_run_stream` should accept `CertifiedTypedSpec` or a replay-specific sealed
    wrapper minted from one.

12. Config and ordinary input artifacts should be materialized into typed runtime values before
    runner invocation through a framework-owned typed runner adapter.
    Runtime core should remain domain-free. The materializer verifies bytes/evidence against
    certified config refs, cell terminal evidence, schema/semantic ids, roles, producers, digest,
    and byte length, then invokes the domain runner with typed config/input values. Domain runners
    keep hostile external data validation, side-effect protocol checks, and replay-specific checks.

13. Production runners should not own `ArtifactSink` or artifact-store write traits.
    If a write interface remains, it is a sealed per-attempt staging interface that cannot call
    `put_verified_artifact` directly and cannot make orphaned bytes/evidence authoritative.
    Read-only verified artifact loading is a separate capability.

14. Typed run status and raw stream endpoints are certification-agnostic inspection views.
    They report the store-owned history and projection state, but they must not mint runtime,
    replay, render, resume, or retention authority. DTO types such as `TypedRunResponse` and
    `TypedRunStreamResponse` should remain impossible to pass where `CertifiedRuntimeSpec`,
    public-output read authority, or replay authority is required.

## Residual Implementation Details

These details remain to be designed, but they should not reopen the architecture above:

- the exact bootstrap receipt cell schema and whether it represents launch evidence, seed
  materialization, or a compact run-authority receipt;
- the exact production store API shape for atomic evidence admission and commit append;
- whether staged artifacts are returned inline in runner output or through a sealed streaming
  per-attempt stager for large artifacts;
- the exact `ProjectRetentionManifestNodeSpec` fields: public-output schema id, manifest version,
  media identity, trigger policy, and receipt/output shape;
- the crate/layering shape for certifier-backed replay authority and typed config/input
  materialization without introducing domain dependencies into `mfm-runtime`;
- whether prepared invocation artifacts contain operationally sensitive material that needs a
  stricter retention/confidentiality rule.
