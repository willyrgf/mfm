# implementation plan for `docs/RFC_STATE_TRANS_CTX.md`

This is a planning-only document. It intentionally describes a full breaking implementation of
`docs/RFC_STATE_TRANS_CTX.md`; it does not preserve old persisted specs, old public JSON contracts,
old raw typestate seeds, compatibility wrappers, fallback entry points, or schema shims.

The EVM lifecycle cutover is blocked until the kernel/program/certifier/runtime/replay commits in
phase 1 make context membership certified authority. A domain-only EVM refactor is non-compliant
with the RFC.

At the public cutover, old public configure and validate shapes are rejected, not translated:

- `{ "config": ConfigurePhaseConfig, "deployed": DeployedContract }`
- `{ "config": ValidatePhaseConfig, "configured": ConfiguredContract }`

Rendered public JSON, public fact refs/DTOs, projection rows, raw typed payloads, and copied
`context_ref` fields are never import authority. Imports must use certified source-run evidence, a
verified export bundle, or an explicit external adoption policy with replay-verifiable evidence.

## validations that must stay

The implementation deletes duplicate context-membership equality checks, but these validations
remain mandatory:

- transport guards for observed chain id and optional chain fingerprint
- evidence, artifact digest, and retained artifact verification
- schema, semantic type, canonical JSON, and no-float checks
- address syntax and normalization
- signer reference and expected public signer address validation
- transaction, receipt, confirmation, idempotency, and nonce-lane validation
- secret redaction on typed configs, events, artifacts, facts, public output, and errors
- replay checks for `FactRecorded`, `FactQueryEvidence`, receipts, confirmations, imports, outputs,
  and public render evidence
- public-output rendering checks that join certified context data without making rendered JSON
  authority

## phase 1: certified context authority

This phase establishes the reusable context contract before any EVM lifecycle cutover. It must be
complete before commits that remove the old EVM local network equality checks.

Each implementation commit should compile and pass its focused tests. Where a change would otherwise
break multiple ownership layers at once, the plan uses additive, unregistered staging commits
followed by one explicit cutover commit. Those staging commits are not compatibility shims: they do
not translate old shapes, do not register fallback public entry points, and do not make public JSON
or raw seeds authoritative. They only keep the review stack buildable until the cutover removes the
old public contract.

### commit 1: `spec: add certified transition context specs`

**goal**

Add the hash-defining persisted spec vocabulary for certified transition contexts, context-bound
cells, input context constraints, and producer/stage constraints.

**files/crates likely touched**

- `crates/kernel/ids`
- `crates/kernel/spec`
- `crates/kernel/spec/src/v1/tests.rs`
- `crates/kernel/spec/README.md`
- `docs/design.md`
- `docs/architecture.md`

**exact behavioral change**

- Add checked identity/value types such as `ContextRef`, `ContextDescriptorId`,
  `ContextResourceKind`, and `ContextStage`.
- Add a `contexts` table to `mfm_spec::v1::TypedExecutionSpec`.
- Add `CertifiedContextSpec` entries that carry:
  - context ref
  - context descriptor id
  - schema id
  - semantic type id
  - canonicalizer id
  - canonical context JSON
  - canonical context digest and byte length
- Add `NodeContextSpec` to `NodeSpec`, with `NoContext` for ordinary nodes and
  `Required { context_ref }` for context-scoped nodes.
- Add `CellContextSpec` to `CellSpec`, including no-context or context-bound resource metadata:
  `context_ref`, resource kind, stage, and the certified producer anchor.
- Add `InputContextSpec` to `InputBindingCellSpec`, so consuming nodes certify the context,
  resource kind, and stage expected from each context-bound input cell.
- Add producer/stage constraint data to the spec shape. The certifier will enforce it in a later
  commit, but the persisted spec must already have explicit fields for resource kind, stage,
  producer descriptor id, and whether seed producers are allowed.
- Include contexts and all context constraints in canonical spec JSON and spec hash derivation.
- Update spec parsing to fail closed on missing, malformed, duplicate, unknown, or non-canonical
  context fields.
- Bump lowering/spec/certifier identity material as needed so old persisted specs cannot verify as
  the new contract.

**validations/tests to add or update**

- Spec round-trip tests for `contexts`, node context, cell context, and input context constraints.
- Canonical JSON/hash tests proving context bytes and constraints are hash-defining.
- Negative tests for duplicate context refs, missing table entries, unknown context refs, context
  bytes that do not match their digest, and non-canonical persisted JSON.
- No-float tests for context canonical JSON.
- Tests proving old persisted specs without the context table are rejected rather than normalized.

**deletion/removal work included**

- Remove any spec tests or fixtures that assume `TypedExecutionSpec` is complete without an
  explicit context table.
- Remove wording from spec docs that implies schema id alone is enough to authorize a lifecycle
  resource.

**dependencies on previous commits**

- None.

**rollback/review risk notes**

- This is a persisted spec contract break. Reviewers should check all canonical JSON field names,
  ordering, and digest material because old specs are intentionally not migrated.

### commit 2: `program: add state context authoring`

**goal**

Make context membership part of typed program authoring and state contracts, not a user config field
or raw input seed.

**files/crates likely touched**

- `crates/kernel/program`
- `crates/kernel/values`
- `crates/kernel/program-derive`
- `crates/states/portfolio`
- `crates/collectors/proof`
- `crates/states/btc`
- `crates/ops/btc-chain-head-collector-op`
- `crates/kernel/facts` for state fact descriptor references that appear in descriptor evidence
- `crates/states/evm-contracts` only for mechanical `NoContext` declarations before the EVM cutover
- `crates/kernel/program/tests`
- `crates/kernel/program/tests/ui`
- `crates/kernel/program/README.md`
- `crates/kernel/program-derive/README.md`

**exact behavioral change**

- Add `StateSpec::Context`.
- Add framework-owned `NoContext` for states that do not need semantic context.
- Add a `MfmContext` marker contract over typed, canonical, non-secret context values.
- Add `CertifiedContext<C>` as the state-facing typed invocation authority. It contains the
  context ref, descriptor ids, schema ids, and typed context value materialized from the certified
  spec. It is not constructible by domain crates from public JSON.
- Record the context contract in state descriptors and state registration evidence, but do not yet
  change executable state trait method signatures. Runtime invocation signature changes land with
  runtime materialization in commit 4 so the workspace stays buildable without temporary adapters.
- Add typed authoring APIs for declaring one context value and binding nodes to it. The API should
  make the common shape explicit, for example:
  - declare `EvmContractContext` once at the root or operation planning boundary
  - run root transitions in that context
  - run non-root transitions with input handles already bound to the same context
- Add typed context-bound output and input contracts. Use typed traits/descriptor registration, not
  JSON-path predicates, to describe:
  - resource kind
  - stage
  - whether the resource is a lifecycle stage resource or a terminal report
  - whether a state may produce or consume that resource
- Add a typed context-bound value extractor contract for runtime output admission. The extractor
  must decode the registered Rust value type and read typed `context_ref`/stage/resource metadata;
  runtime must not validate payload context with ad hoc JSON paths.
- Update every existing state implementation, including BTC collector states, to declare
  `type Context = NoContext` unless it is converted later in a domain cutover.
- Update derive-generated state input and operation output helpers as needed so context metadata
  is carried on handles but cannot be forged from raw ids.

**validations/tests to add or update**

- Trybuild tests proving handles cannot be forged with a context ref.
- Trybuild tests proving a raw context key cannot be used as a state input substitute.
- Unit tests for `NoContext` state invocation and context-scoped state authoring.
- Tests that authored context refs are stable content addresses of canonical context bytes.
- Tests that context values with floats or secret-marked descriptors are rejected by the typed
  descriptor path.
- Compile updates for all existing `StateSpec` impls.

**deletion/removal work included**

- Delete or rewrite program tests that model context as a string key inside normal state input.
- Remove any helper API that would let operations attach context metadata to a handle without going
  through the typed context declaration path.

**dependencies on previous commits**

- Depends on commit 1 for persisted context spec fields.

**rollback/review risk notes**

- This is a source break for `StateSpec` implementers, but executable state method signatures are
  intentionally deferred to commit 4. Reviewers should focus on preserving crate boundaries: states
  depend on program/capability contracts only, not runtime/store/app.

### commit 3: `certify: enforce context graph constraints`

**goal**

Make the certified spec reject invalid context membership, producer/stage mismatches, and weak
raw-seed lifecycle resources before runtime can invoke a state.

**files/crates likely touched**

- `crates/kernel/certify`
- `crates/kernel/certify/src/node_contract.rs`
- `crates/kernel/certify/src/framework_lifecycle.rs`
- `crates/kernel/certify/tests.rs`
- `crates/kernel/certify/tests/ui`
- `crates/kernel/facts` only if context-bound fact descriptor/query constraints need shared types
- `crates/kernel/spec/src/v1/tests.rs`
- `docs/design.md`

**exact behavioral change**

- Validate the certified context table:
  - every context ref is content-derived from descriptor id, schema version, canonicalizer id, and
    canonical context bytes
  - every context value decodes through the registered typed context descriptor
  - no context contains floats or secret-bearing fields
- Validate node context constraints:
  - `StateSpec::Context = NoContext` nodes must be `NoContext`
  - context-scoped nodes must reference an existing context of the registered context type
- Validate output/cell context constraints:
  - a context-bound output cell must use the node's required context
  - output resource kind/stage must match the state descriptor's typed output contract
  - no seed can masquerade as a context-bound lifecycle stage unless an explicit import state
    descriptor and import policy authorizes it
- Validate input context constraints:
  - each context-bound input cell must reference a certified cell whose context, resource kind,
    stage, schema, semantic type, and lineage match the consuming state's descriptor contract
  - consuming a context-bound resource under a different node context is a certification failure
- Validate producer/stage constraints:
  - only approved producer descriptor ids may create a resource kind/stage
  - stage transitions must follow the typed state descriptor contract
  - terminal reports may be context-bound without becoming stage producers
- Ensure same-value bridges preserve context constraints. Cross-context adoption must be modeled as
  an import state, not a bridge.
- Include context descriptor identities and context-bound value extractor identities in registry
  digests and certificates.
- Preserve existing fact descriptor allow-list authority and extend it for context-bound facts:
  `StateSpec::emitted_fact_descriptors` and node `fact_descriptor_allowlist` remain necessary, but a
  context-bound fact also needs certified node context, descriptor, resource kind/stage, and
  query/selection constraints.

**validations/tests to add or update**

- Certification-fail test for configure consuming a deployed resource from another context.
- Certification-fail test for validate consuming a configured resource from another context.
- Certification-fail test for an unapproved producer creating a lifecycle stage resource.
- Certification-fail test for a seed with the right schema attempting to satisfy a context-bound
  lifecycle input.
- Certification-fail test for a context-bound output cell whose context differs from the node
  context.
- Certification-fail test for a same-value bridge that attempts cross-context resource movement.
- Certification-fail test for a context-bound fact/query result used under the wrong node context.
- Positive tests for `NoContext` existing workflows.

**deletion/removal work included**

- Remove certifier assumptions that schema id plus semantic type id is sufficient for resource
  authority when a cell is declared context-bound.

**dependencies on previous commits**

- Depends on commits 1 and 2.

**rollback/review risk notes**

- Keep the certifier domain-free. It may validate generic context/resource/stage contracts and
  registered typed descriptors, but it must not depend on EVM crates.

### commit 4: `runtime: materialize certified contexts and validate outputs`

**goal**

Enforce certified context authority before runner invocation and at output admission.

**files/crates likely touched**

- `crates/kernel/runtime`
- `crates/kernel/runtime/src/invocation.rs`
- `crates/kernel/runtime/src/history.rs`
- `crates/kernel/runtime/src/commit.rs`
- `crates/kernel/runtime/src/runner_kit.rs`
- `crates/kernel/runtime/src/binding.rs`
- `crates/kernel/runtime/src/spec_authority.rs`
- `crates/kernel/program`
- `crates/states/portfolio`
- `crates/collectors/proof`
- `crates/states/btc`
- `crates/ops/btc-chain-head-collector-op`
- `crates/adapters/btc-jsonrpc`
- `crates/btc-capabilities`
- `crates/fact-capabilities`
- `crates/kernel/facts`
- `crates/states/evm-contracts` for mechanical `NoContext` invocation updates
- adapters that call state execution traits, for mechanical `NoContext` invocation updates
- `crates/kernel/events`
- `crates/kernel/store` only if event/store artifact requirement helpers need context-aware
  expectation fields
- `crates/kernel/runtime/tests.rs`
- `crates/kernel/runtime/tests/ui`
- `crates/kernel/runtime/README.md`
- `docs/design.md`

**exact behavioral change**

- `CertifiedRuntimeSpec` indexes certified contexts and exposes typed materialization handles.
- `BoundRuntimeContext` binds the certified context-value extractor registry needed for output
  admission.
- `PreparedRunnerInvocation`, `PreInvocationRunCtx`, `ErasedRunCtx`, and `RunnerIngressContext`
  expose the certified context for the current node.
- Change every executable state trait to receive the certified invocation context:
  - `PureState`
  - `ReadState`
  - `ManagedWriteState`
  - `SideEffectState`
- Update framework lifecycle runners and all existing state/adapter callers to pass
  `CertifiedContext<NoContext>` for ordinary states.
- Input materialization validates:
  - input binding context constraint matches the certified cell constraint
  - input cell context matches the consuming node context
  - producer/stage/resource metadata matches the certified input binding
- Runner output admission validates:
  - `CellProduced` and artifact evidence match the certified output cell as they do today
  - if the output cell is context-bound, the typed output artifact decodes through the registered
    context-bound value extractor
  - the payload `context_ref`, resource kind, and stage match the certified output cell
  - a runner cannot admit a copied payload from another context by changing event metadata
- Attempt-start and attempt-output preconditions remain store-owned and domain-agnostic.
- Runtime does not read mutable runtime config, public output, projection rows, or live routing
  registries to materialize context.
- Runtime records context-bound `FactRecorded` claims and `FactQueryEvidence` only through certified
  descriptor/query authority. Existing `NoContext` collector workflows keep their typed fact
  recording and authenticated query evidence, but those facts are not promoted to transition context
  unless certified context constraints say so.
- Output validation failures inside a valid started attempt follow the existing redacted
  `InvalidRunnerOutput` attempt-failure path when no side-effect boundary has been crossed.

**validations/tests to add or update**

- Runtime test that rejects committed output whose payload context ref disagrees with the certified
  output cell.
- Runtime test that rejects input materialization when a context-bound input cell is wired under a
  different node context.
- Runtime test that rejects a context-bound output artifact with correct schema but wrong stage.
- Runtime test that `NoContext` nodes still materialize and run.
- Runner-kit test that typed state-output artifacts are context-validated through the registered
  extractor, not JSON field paths.
- Runtime admission test that missing context descriptors/extractors block launch before
  `RunAdmitted`.
- Runtime/runner-kit tests proving context-bound fact recording and fact-query evidence cannot bypass
  descriptor/query authority.
- BTC collector smoke test proving `NoContext` fact recording/query evidence still runs and replays.

**deletion/removal work included**

- Remove runtime tests or helpers that infer authority from raw cell ids, projections, or rendered
  output when certified context/cell constraints are available.

**dependencies on previous commits**

- Depends on commits 1 through 3.

**rollback/review risk notes**

- This is the main runtime safety boundary. Reviewers should inspect the exact order:
  `StateAttemptStarted`, materialization, runner invocation, output validation, commit planning.
  No runner-controlled payload should bypass context validation.

### commit 5: `replay: enforce context-bound evidence`

**goal**

Make replay and verified-history authority fail closed on context-bound mismatches and on
identifier-only imports.

**files/crates likely touched**

- `crates/kernel/replay`
- `crates/kernel/facts`
- `crates/fact-capabilities`
- `crates/kernel/store`
- `crates/kernel/runtime/src/history.rs`
- `crates/kernel/runtime/src/commit.rs`
- `crates/app/src/lib.rs`
- `crates/app/src/tests.rs`
- `crates/adapters/btc-jsonrpc/src/tests.rs`
- `tests/integration/tests/collector_workflow_happy_path.rs`
- `crates/kernel/replay/tests`
- `docs/design.md`

**exact behavioral change**

- `ReplayReadAuthority` carries certified context table authority from the verified runtime spec.
- Replay stream validation verifies context-bound cell events against certified context/cell/output
  constraints.
- Replay `FactRecorded`, `FactQueryEvidence`, and side-effect evidence lookup includes expected node
  context and rejects recorded evidence that decodes to another context when the evidence type is
  context-bound.
- Replay verifies authenticated fact-query receipts, returned refs, retained descriptor/response
  artifacts, selection evidence, and `Control`/`Platform` visibility boundaries without consulting the
  live fact index or collector providers.
- Add generic import evidence verification contracts that require retained proof material:
  source spec hash, source certificate/export certificate, committed stream or export bundle ref,
  source cell/output id, source schema/semantic ids, source producer descriptor id, source stage,
  source context ref, source value digest, source terminal event ref, and import policy digest.
- Reject import replay when only `source_run_id`, `source_cell_or_output_id`, or rendered JSON is
  supplied.
- Replay never queries live stores, current runtime config, current public-output renderers, live
  transports, or signer providers for import or context authority.

**validations/tests to add or update**

- Replay test that mismatched output context refs fail closed.
- Replay test that mismatched `FactRecorded`, `FactQueryEvidence`, receipt, confirmation, or import
  evidence context refs fail closed.
- Replay test that collector checkpoint query evidence replays from retained receipt/source fact
  authority and never queries the live fact index.
- Replay test that source-run import with identifiers only is rejected.
- Replay test that public-output JSON is rejected as import authority.
- Positive replay test for `NoContext` existing workflows.

**deletion/removal work included**

- Remove replay fixtures that treat rendered public-output JSON or projection rows as semantic
  authority.

**dependencies on previous commits**

- Depends on commits 1 through 4.

**rollback/review risk notes**

- Reviewers should verify replay remains evidence-only. Import verification must not create a
  backdoor into app services or live stores.

## phase 2: EVM lifecycle model and action cutover

This phase introduces the EVM domain model that uses the certified context primitives.

### commit 6: `evm-capabilities: add code read capability`

**goal**

Add the replayable EVM code-read authority required by the RFC's default external address adoption
policy.

**files/crates likely touched**

- `crates/evm-capabilities`
- `crates/evm-capabilities/tests/capability_contract.rs`
- `crates/transports/evm`
- `crates/transports/evm/src/tests.rs`
- `crates/adapters/evm-contracts`

**exact behavioral change**

- Add `EvmCodeReadCapability`, `EvmCodeReadProvider`, `EvmCodeReadRequest`, and
  `EvmCodeReadResponse`.
- Request includes `EvmChainGuard`, address, and block anchor.
- Response includes redacted source evidence and code bytes or code hash evidence.
- EVM transport implements `eth_getCode` and verifies the same guard model used by chain/call/log
  capabilities.
- Replay evidence for code reads is typed and artifact-backed like other read facts.

**validations/tests to add or update**

- Capability descriptor golden tests for `mfm.evm.code.read`.
- Transport tests for code read success, empty code, code hash, guard mismatch, and redaction.
- Replay fact tests for code-read request/response hash matching.

**deletion/removal work included**

- None.

**dependencies on previous commits**

- Depends on phase 1 only if code-read evidence is made context-bound immediately. Otherwise it can
  be implemented after commit 1, but it must land before external adoption defaults are exposed.

**rollback/review risk notes**

- This is a new reusable capability, not workflow topology. Keep it out of EVM lifecycle operation
  crates.

### commit 7: `evm-contract-model: add context-bound lifecycle values`

**goal**

Define pure EVM context, resource, report, import, evidence, and provenance model types.

**files/crates likely touched**

- `crates/evm-contract-model`
- `crates/evm-contract-model/tests/lifecycle_model.rs`
- `docs/architecture.md`

**exact behavioral change**

- Add `EvmNetworkContext`:
  - `network_id`
  - `expected_chain_id`
  - optional `chain_fingerprint`
  - optional `finality_or_observation_policy`
- Add `EvmContractContext`:
  - lifecycle key
  - `EvmNetworkContext`
  - required `contract_profile`
- Add digest-oriented `ContractProfile` fields for artifact/interface/code-shape identity.
- Add context-bound resources:
  - `ContractInstance<Deployed>` or concrete `DeployedContractInstance`
  - `ContractInstance<Configured>` or concrete `ConfiguredContractInstance`
- Add `ValidationReport` as a context-bound terminal value with `context_ref`, configured instance
  identity, assertion results, and evidence refs.
- Add closed `ConfigurationClaim` enum:
  - `MfmConfigured`
  - `ImportedMfmConfigured`
  - `ExternalObservedConfigured`
  - `ExternalClaimedConfigured`
- Add import specs/evidence model types:
  - `ImportFromMfmRun`
  - `ImportFromMfmRunEvidence`
  - `AdoptExternalAddress`
  - `ExternalAdoptionEvidencePolicy`
  - `ExternalAdoptionEvidence`
- Add typed context-bound value extractor implementations for deployed, configured, and validation
  report values.
- Public render helper types may expose selected context fields for audit, but model authority is
  `context_ref`.

**validations/tests to add or update**

- Context canonicalization and no-float tests.
- Context ref derivation tests for equal/different contexts.
- Context-bound value tests proving payload stage and `context_ref` are extracted through typed
  traits.
- Configuration claim tests proving external adoption cannot claim MFM configuration unless the
  enum variant proves it.
- Tests for address normalization and evidence ref validation.

**deletion/removal work included**

- Remove model tests that assert:
  - `ConfiguredContractRef` copies network fields from nested deployed values
  - `ValidationReport.expected_chain_id` is copied from validate config
- Schedule deletion of old `DeployedContract`, `ConfiguredContract`, `ConfiguredContractRef`, and
  `ExistingConfiguredContractRef` schemas in the cleanup commit once downstream crates are cut over.

**dependencies on previous commits**

- Depends on commits 1 through 5 for context ids and typed context-bound value contracts.

**rollback/review risk notes**

- This crate remains pure model/config logic. It must not import runtime, store, app, transports, or
  signer provider implementations.

### commit 8: `evm-contract-config: add context actions for cutover`

**goal**

Add the new context object plus phase-local action/import configs without switching registered EVM
entry points yet.

**files/crates likely touched**

- `crates/evm-contract-config`
- `crates/evm-contract-config/tests/phase_config.rs`
- `crates/evm-contract-model`
- `docs/architecture.md`

**exact behavioral change**

- Add `DeployAction`, `ConfigureAction`, and `ValidateAction`.
- New action types do not contain network authority.
- Move shared artifact/interface/code profile identity into `EvmContractContext.contract_profile`.
- Keep action-local fields:
  - deploy: signer, constructor args, value, transaction policy, receipt policy
  - configure: signer, calls, confirmation reads/events, transaction policy, receipt policy
  - validate: read assertions, event assertions
- Add entry-point config types:
  - full lifecycle: `context`, `deploy`, `configure`, `validate`
  - configure-only: `context`, `import_deployed`, `configure`
  - validate-only: `context`, `import_configured`, `validate`
  - deploy-only: `context`, `deploy`
- Add import spec config validation for source-run imports and external adoption policy.
- Deny unknown fields on the new config types so old `{ config, deployed }` and
  `{ config, configured }` shapes will fail once the public cutover selects the new schemas.
- Do not register the new public entry shapes in app/CLI/REST in this commit. Keeping old currently
  registered planners untouched here is a buildability step, not compatibility for the final
  implementation.

**validations/tests to add or update**

- Config tests that action configs reject `network` fields.
- Context config tests for required `expected_chain_id`, profile identity, canonical no-float data,
  and denied unknown fields.
- New-type decode tests proving old configure/validate shapes fail when decoded as the new schemas.
- Import policy tests for exact context-ref default and explicit cross-context acceptance.
- Receipt, transaction, signer, assertion, address, and quantity validation tests retained.

**deletion/removal work included**

- None in production code. Deletion of old phase config schemas and docs happens in the public
  cutover and final cleanup commits after states, ops, adapters, and tests can move together.

**dependencies on previous commits**

- Depends on commit 7.

**rollback/review risk notes**

- This is an additive staging commit. Reviewers should verify it does not expose a second public
  entry path and does not translate old JSON into imports.

### commit 9: `states: add context-bound evm lifecycle imports`

**goal**

Add context-bound EVM lifecycle state contracts and fail-closed import admission behavior.

**files/crates likely touched**

- `crates/states/evm-contracts`
- `crates/states/evm-contracts/src/tests.rs`
- `crates/evm-contract-model`
- `crates/evm-contract-config`

**exact behavioral change**

- Add the context-bound state descriptors that will replace the current registered lifecycle states
  at cutover:
  - deploy
  - configure
  - validate
  - `ImportDeployedContractState`
  - `ImportConfiguredContractState`
- These states use `StateSpec::Context = EvmContractContext`, `DeployAction`, `ConfigureAction`,
  and `ValidateAction`.
- Keep them out of public entry-point registration until the operation/adapter/public cutover. They
  are not compatibility wrappers and they do not translate old raw typestate seeds.
- Deploy root transition:
  - receives certified `EvmContractContext` and `DeployAction`
  - produces `ContractInstance<Deployed>` bound to the certified context
- Configure transition:
  - consumes `ContractInstance<Deployed>` bound by certified input constraints
  - receives certified context and `ConfigureAction`
  - produces `ContractInstance<Configured>`
- Validate transition:
  - consumes `ContractInstance<Configured>`
  - receives certified context and `ValidateAction`
  - produces context-bound `ValidationReport`
- Import transitions:
  - consume import specs/evidence policies
  - produce deployed/configured stage handles under the certified context
  - never accept raw typestate seeds as continuation authority
- Import admission must verify certified policy and evidence before returning any context-bound
  resource:
  - source-run imports require verified source spec/certificate, committed stream or certified
    export bundle evidence, source cell/output id, source value digest, producer descriptor,
    source stage, source context ref, terminal event ref, and import policy digest
  - external address adoption requires the certified evidence policy, chain guard evidence,
    address normalization, code existence/code hash evidence when required, block anchor when
    required, and configured read/event assertion evidence unless the certified policy allows
    `ExternalClaimedConfigured`
  - public JSON, projection rows, raw payloads, and identifiers alone are rejected before any
    lifecycle resource is produced
- State intent/report construction uses certified context for network id, expected chain id, profile,
  and guard material.
- State output construction copies `context_ref` from certified context and stage-specific evidence,
  not from phase configs or nested typestates.

**validations/tests to add or update**

- State tests for deploy/configure/validate intent construction from certified context.
- State tests for import claim variants and honest external provenance.
- Import admission tests proving source-run and external imports reject identifier-only input,
  public JSON, projection rows, raw payloads, missing retained proof, missing certified import
  policy evidence, mismatched context, wrong stage, and unapproved producer before producing a
  context-bound resource.
- State tests that `ExternalClaimedConfigured` renders/records no fake MFM configure calls,
  receipts, or confirmations.
- State tests for nonce idempotency retaining stable canonical intent digests.
- Tests preserving address, signer, transaction, receipt, confirmation, schema, canonical, and
  redaction validation.

**deletion/removal work included**

- None in the old registered lifecycle path yet. Deletion of old state-local matching happens when
  the operation/adapter cutover stops using the old states, then final cleanup removes the old code.

**dependencies on previous commits**

- Depends on commits 1 through 8.

**rollback/review risk notes**

- This is an additive staging commit. Reviewers should verify import admission is explicit and
  fail-closed, and that no public entry point can use these states until the certified operation and
  adapter cutover is ready.

## phase 3: operation topology and public entry-point cutover

### commit 10: `ops: add evm import topologies`

**goal**

Add the certified import-plus-transition topologies that will replace standalone configure/validate
raw typestate seeds at public cutover.

**files/crates likely touched**

- `crates/ops/evm-contract-lifecycle-op`
- `crates/ops/evm-contract-lifecycle-op/tests/lifecycle_planning.rs`
- `crates/ops/evm-contract-lifecycle-op/tests/ui`
- `crates/app/src/entry_points.rs`
- `crates/app/src/entry_points.rs` tests

**exact behavioral change**

- Add new operation descriptors/topology helpers for:
  - full lifecycle
  - deploy-only
  - configure-only
  - validate-only
- Full lifecycle topology:
  - declare one `EvmContractContext`
  - deploy in that context
  - configure the deployed handle
  - validate the configured handle
- Configure-only topology:
  - declare one `EvmContractContext`
  - run `ImportDeployedContractState`
  - configure the imported deployed handle
- Validate-only topology:
  - declare one `EvmContractContext`
  - run `ImportConfiguredContractState`
  - validate the imported configured handle
- Deploy-only topology:
  - declare one `EvmContractContext`
  - deploy in that context
- Bind public outputs to context-bound values.
- New planner functions accept only context/action/import config types.
- New configure/validate plans produce no `DEPLOY_SEED_KEY`, `CONFIGURED_SEED_KEY`, or raw
  continuation seed material.
- Do not switch the production app registry to these new planners until adapter runners and public
  boundary tests are ready. This avoids a half-cutover state without adding compatibility
  translation.

**validations/tests to add or update**

- Planning tests proving the new configure-only and validate-only plans contain import nodes, no
  raw seed material, and context-bound downstream handles.
- Certification tests proving imported resources can be consumed only under the certified context.
- Compile-fail tests for using a deployed handle where configured is required and vice versa,
  updated for context-bound handles.
- Tests proving full lifecycle no longer performs phase-to-phase network equality checks because
  phases do not carry network.

**deletion/removal work included**

- None in the currently registered public path. Deletion happens in the public cutover/final cleanup
  commits once runners and public tests are ready.

**dependencies on previous commits**

- Depends on commits 1 through 9.

**rollback/review risk notes**

- This is an additive topology commit. Reviewers should confirm the new configure/validate
  standalone starts are imports, not renamed raw seeds, and that the old registered entry points are
  not translated into these imports.

### commit 11: `adapters: execute evm from certified context`

**goal**

Update EVM runners, prepared invocation evidence, resource lanes, and transport requests to use
certified context authority and context-bound inputs.

**files/crates likely touched**

- `crates/adapters/evm-contracts`
- `crates/adapters/evm-contracts/src/tests.rs`
- `crates/evm-capabilities`
- `crates/runtime-config` only if factory routing APIs need context-aware lookup input
- `crates/app/src/evm_contracts.rs`
- `crates/app/src/evm_contracts/tests.rs`

**exact behavioral change**

- Load `EvmContractContext` from `ErasedRunCtx`/`PreInvocationRunCtx`, not from phase config.
- Runtime factory APIs route by certified semantic network context and signer ref:
  - process-local source selection still maps semantic network id to routes
  - route selection is not typestate membership authority
- Prepared invocation evidence includes:
  - `context_ref`
  - `evm_network_context_ref`
  - resource stage
  - network id and expected chain id from certified context
  - signer ref and expected signer address from action
- EVM request guards are built from certified `EvmNetworkContext`.
- Configure adapters take contract address from a certified context-bound deployed input.
- Validate adapters take contract address and configuration claim from a certified context-bound
  configured input.
- Nonce resource lanes use `evm_network_context_ref` plus expected signer address, not the full
  contract context alone and not a bare chain id/free-form network string.
- External adoption import execution uses code-read capability according to certified evidence
  policy.
- Source-run import execution reads only verified source-run/export evidence supplied through the
  certified import boundary. It rejects identifiers, public JSON, projections, raw payloads, missing
  retained artifacts, mismatched source context, wrong stage, and missing import policy before
  returning a context-bound lifecycle resource.
- Transport guards remain unchanged in purpose: they prove observed live route/chain behavior, not
  typestate membership.

**validations/tests to add or update**

- Adapter tests proving route selection uses certified context, not action config.
- Adapter tests proving prepared invocation, receipt, confirmation, validation read evidence, report
  state-output evidence, and import evidence reject mismatched `context_ref`.
- Adapter tests proving transport guard still rejects observed chain mismatch.
- Adapter import-admission tests proving source-run and external adoption imports fail closed before
  output when proof material or certified policy is missing or mismatched.
- Nonce-lane tests proving lane key includes `evm_network_context_ref` plus signer and does not use
  only full contract context or bare chain id.
- Redaction tests proving no RPC URL, auth header, keystore path, password path, private key,
  mnemonic, raw signed transaction, or signature scalar is persisted or printed.

**deletion/removal work included**

- Delete these patterns from the new adapter paths:
  - `runtime_for(config.network().network_id())`
  - `read_runtime_for(config.network().network_id())`
  - `validate_runtime_for(config.network().network_id(), ...)`
  - side-effect plan network forwarding from phase config
- Leave old adapter paths untouched only until the public cutover removes them from registration;
  no new path may call them.

**dependencies on previous commits**

- Depends on commits 1 through 10.

**rollback/review risk notes**

- Do not remove transport guard checks. They remain the live/replay source-observation invariant.

### commit 12: `replay: verify evm imports and lifecycle evidence`

**goal**

Make EVM replay verify context-bound import, side-effect, validation read evidence, report
state-output evidence, and output evidence from certified spec plus retained evidence only. Keep
generic fact replay as `FactRecorded` and `FactQueryEvidence` verification rather than implying EVM
validation always emits a fact.

**files/crates likely touched**

- `crates/adapters/evm-contracts`
- `crates/kernel/replay`
- `crates/app/src/lib.rs`
- `crates/app/src/evm_contracts.rs`
- `crates/app/src/evm_contracts/tests.rs`

**exact behavioral change**

- EVM side-effect replay verifier checks:
  - prepared invocation context ref
  - submission/receipt/confirmation context refs
  - stage identity
  - resource lane key context material
  - request hashes reconstructed from certified context and context-bound inputs
- EVM validation replay checks validation read request/response context refs, terminal
  `ValidationReport` state-output evidence, and transport guard evidence.
- Generic fact replay remains responsible for actual `FactRecorded` claims and pinned
  `FactQueryEvidence` artifacts.
- Import-from-MFM-run verifies source authority through committed run stream plus verified retained
  artifacts, or a certified export bundle. It does not trust rendered public JSON or projection
  rows.
- External adoption replay verifies code existence/code hash/block/read/event evidence according to
  the certified import policy.
- Public output rendering may show `context_ref`, `network_id`, `expected_chain_id`, observed chain
  id, and contract address, but later imports cannot use rendered JSON as authority.

**validations/tests to add or update**

- Import-from-MFM-run accepts exact matching source context and rejects mismatched source context by
  default.
- Import-from-MFM-run accepts explicit certified cross-context policy only when retained evidence
  proves the accepted relation.
- Identifier-only source imports fail replay.
- Public JSON import attempts fail.
- External address adoption replay fails if code-read evidence is absent when the policy requires
  code existence.
- Replay fails on mismatched prepared invocation, receipt, confirmation, validation read evidence,
  report state-output evidence, import, or output context refs.
- Replay continues to reject live capability construction.

**deletion/removal work included**

- Delete any EVM replay diagnostic path that reconstructs guard authority by decoding old phase
  configs rather than certified context.
- Delete tests that rely on old config artifacts as network membership authority.

**dependencies on previous commits**

- Depends on commits 1 through 11.

**rollback/review risk notes**

- Source-run imports are replay-proof only if retained proof material is sufficient. If a portable
  export bundle format is missing, implement the minimal certified export/import bundle in this
  commit rather than falling back to identifiers or public JSON.

### commit 13: `public: expose context-based evm entry points`

**goal**

Cut over CLI, REST, app registry, and public docs/tests to the new breaking EVM entry shapes.

**files/crates likely touched**

- `crates/app/src/entry_points.rs`
- `crates/app/src/entry_point.rs` tests as needed
- `crates/app/src/evm_contracts.rs`
- `crates/app/src/public_facts`
- `bin/cli/src/commands/run/start.rs`
- `bin/cli/src/commands/facts.rs`
- `bin/cli/tests`
- `bin/cli/README.md`
- `bin/rest-api/src/lib.rs`
- `bin/rest-api/src/tests.rs`
- `bin/rest-api/README.md`
- `docs/persisted-public-surfaces.md`
- `README.md` only if examples reference EVM lifecycle shapes

**exact behavioral change**

- Switch the production app registry from the old EVM planners/runners to the context/action/import
  planners and certified-context adapter runners added in commits 9 through 11.
- Registered EVM public op names may stay durable if desired, but their version/schema is breaking.
  If keeping names, bump op version and schema ids so old callers cannot be silently accepted.
- Public inputs become:
  - deploy: `{ "context": EvmContractContext, "deploy": DeployAction }`
  - configure: `{ "context": EvmContractContext, "import_deployed": ImportDeployedSpec,
    "configure": ConfigureAction }`
  - validate: `{ "context": EvmContractContext, "import_configured": ImportConfiguredSpec,
    "validate": ValidateAction }`
  - lifecycle: `{ "context": EvmContractContext, "deploy": DeployAction,
    "configure": ConfigureAction, "validate": ValidateAction }`
- CLI/REST decode errors for old shapes remain stable redacted errors; they do not translate old
  shapes to imports.
- Output docs state that public output is render/cache material only.
- Public fact CLI/REST output stays a public query surface only. `Platform` facts may be returned
  through DTOs and opaque `PublicFactRefId` values; `Control` facts stay hidden; neither public fact
  refs nor public fact JSON can satisfy source-run imports or lifecycle continuation.
- Remove old EVM public planner registrations in this commit. There is no period after this commit
  where both old and new EVM public entry shapes are accepted.

**validations/tests to add or update**

- CLI JSON/TOML examples for all new EVM shapes.
- REST request examples for all new EVM shapes.
- CLI and REST tests proving old `{ config, deployed }` and `{ config, configured }` shapes are
  rejected.
- App entry-point tests proving configure/validate plans use import nodes and no seed material.
- Public-output tests proving network data is rendered from certified context, not copied
  typestate fields.
- Public fact tests proving `Control` facts remain hidden and public fact refs cannot be used as
  import authority.
- Error-code tests for malformed legacy shapes.
- End-to-end app tests proving public configure/validate entry points produce import nodes and no
  raw seed material.

**deletion/removal work included**

- Delete CLI/REST docs and fixtures for old `DeployPhaseConfig`, `ConfigurePhaseConfig`,
  `ValidatePhaseConfig`, `DeployedContract`, and `ConfiguredContract` entry JSON.
- Delete tests that expect configure/validate entry points to stage raw seed material.
- Delete or update any examples that imply public fact refs, fact DTOs, or fact projection rows are
  lifecycle import authority.
- Delete old public planner registrations and old public entry config types:
  - `ContractConfigureEntryPointConfig { config, deployed }`
  - `ContractValidateEntryPointConfig { config, configured }`
- Delete raw public seed plumbing from the entry-point path:
  - `configure_contract_program_draft(config, deployed)`
  - `validate_contract_program_draft(config, configured)`
  - `DEPLOY_SEED_KEY`
  - `CONFIGURED_SEED_KEY`

**dependencies on previous commits**

- Depends on commits 1 through 12.

**rollback/review risk notes**

- This is the public contract break. Reviewers should verify old callers fail closed, no old EVM
  public entry shape remains registered, and no old shape is migrated implicitly.

## phase 4: final cleanup and regression gates

### commit 14: `cleanup: delete legacy evm network matching surface`

**goal**

Remove obsolete fields, helper functions, tests, docs, and compatibility-shaped code promised for
deletion by the RFC.

**files/crates likely touched**

- `crates/evm-contract-model`
- `crates/evm-contract-config`
- `crates/states/evm-contracts`
- `crates/ops/evm-contract-lifecycle-op`
- `crates/adapters/evm-contracts`
- `crates/app`
- `bin/cli`
- `bin/rest-api`
- `docs`
- cargo metadata/architecture tests

**exact behavioral change**

- The final codebase has one EVM lifecycle context authority and no old public continuation shape.
- Old persisted specs and public JSON contracts are not preserved.
- EVM lifecycle resources are context-bound values with certified producer/stage constraints.
- Standalone configure and validate are import flows followed by ordinary transitions.

**validations/tests to add or update**

- Source scan or architecture test proving these names/patterns are gone from production code:
  - `ensure_phase_networks_match`
  - `ensure_networks_match`
  - `ensure_network_matches`
  - `ContractConfigureEntryPointConfig`
  - `ContractValidateEntryPointConfig`
  - `DEPLOY_SEED_KEY`
  - `CONFIGURED_SEED_KEY`
  - `DeployedContract.network_id`
  - `DeployedContract.expected_chain_id`
  - `ConfiguredContract.deployed`
  - `ConfiguredContractRef.network_id`
  - `ConfiguredContractRef.expected_chain_id`
  - `ValidationReport.expected_chain_id`
  - adapter `runtime_for(config.network().network_id())`
- Cargo metadata checks continue to enforce architecture boundaries.
- Final focused suites:
  - `cargo fmt --all -- --check`
  - `cargo test -p mfm-spec`
  - `cargo test -p mfm-program`
  - `cargo test -p mfm-certify`
  - `cargo test -p mfm-runtime`
  - `cargo test -p mfm-replay`
  - `cargo test -p mfm-facts`
  - `cargo test -p mfm-fact-capabilities`
  - `cargo test -p mfm-btc-capabilities`
  - `cargo test -p mfm-states-btc`
  - `cargo test -p mfm-op-btc-chain-head-collector`
  - `cargo test -p mfm-adapters-btc-jsonrpc`
  - `cargo test -p mfm-evm-capabilities`
  - `cargo test -p mfm-state-evm-contracts`
  - `cargo test -p mfm-op-evm-contract-lifecycle`
  - `cargo test -p mfm-adapters-evm-contracts`
  - `cargo test -p mfm-app evm_contracts`
  - `cargo test -p mfm-integration-tests --test collector_workflow_happy_path`
  - CLI/REST focused tests for entry-point boundaries
- Final merge-readiness gates:
  - `nix run .#check`
  - `nix run .#test`
  - `nix run .#test-db`
  - `nix run .#ci`

**deletion/removal work included**

- Delete per-phase `network` fields from deploy/configure/validate configs.
- Delete lifecycle configs that repeat network in each phase.
- Delete public standalone configure config shaped as `{ config, deployed }`.
- Delete public standalone validate config shaped as `{ config, configured }`.
- Delete raw standalone typestate seeds for lifecycle continuation.
- Delete authoritative `network_id` and `expected_chain_id` fields on lifecycle typestates.
- Delete `ConfiguredContract` nesting full `DeployedContract` as identity.
- Delete copied network fields from configured refs and validation reports.
- Delete repeated operation/state/adapter network-matching helpers.
- Delete adapter runtime lookup by loose phase config network.
- Delete imports from rendered public JSON, projection rows, or raw typestate payloads.
- Delete imports from public fact refs, fact DTOs, or fact projection rows.
- Delete language implying transport guards prove typestate/context membership.
- Delete tests that only prove copied network fields agree in old shapes.

**dependencies on previous commits**

- Depends on commits 1 through 13.

**rollback/review risk notes**

- This commit should be mostly deletion plus regression gates. Any surviving old helper should be
  treated as a blocker unless it proves a validation that the RFC says must stay, such as transport
  guards, evidence verification, nonce lanes, schema/canonical/no-float checks, secret redaction, or
  replay checks.

## intentionally deferred implementation risks

These are not permission to ship a non-compliant implementation. They are review risks to track
while implementing the required commits.

- Portable export bundle UX can be minimal for the first cutover, but import-from-MFM-run must still
  verify either committed source-run stream/artifact authority or a certified export/import bundle.
  It must never fall back to identifiers or public JSON.
- Optional `chain_fingerprint` enforcement depends on capability evidence being available. If a
  context specifies a fingerprint, transport/replay evidence must verify it; contexts without a
  fingerprint still rely on the required chain-id guard.
- Cross-context adoption beyond exact context-ref equality should remain unavailable unless a typed,
  certified compatibility relation with replay-verifiable evidence is implemented.
- A richer context-bound value derive may be useful later. The first implementation should prefer a
  small typed extractor contract over generic JSON-path predicates.

## final review checklist

- The implementation is a breaking change with no compatibility shim.
- EVM cutover commits depend on certified context primitives.
- Certification rejects wrong-context consumers, unapproved producers, wrong stages, and raw-seed
  masquerading.
- Runtime rejects mismatched context-bound inputs before runner invocation.
- Runtime rejects output payload context mismatches before admitting cells.
- Import admission rejects missing policy, missing proof, wrong context, wrong stage, unapproved
  producer, identifier-only input, public JSON, projections, and raw payloads before producing
  context-bound resources.
- Import admission and replay are evidence-backed, not identifier-only.
- Public JSON, public fact refs, and public fact DTOs are never import authority.
- Context-bound fact/query evidence, when introduced, is certified through descriptor/query
  constraints and replayed from retained `FactRecorded`/`FactQueryEvidence` authority.
- Old local defensive network equality checks are removed only after certified invariants exist.
- Tests prove fail-closed behavior at certification, runtime admission, import admission, replay,
  and public entry boundaries.
