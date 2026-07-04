# RFC: State Transition Contexts

Status: implemented

## Summary

MFM should model network, chain, market, account, and other domain-scoping identities as certified
state-transition context, not as repeated fields copied through phase configs and typestate values.

The current EVM contract lifecycle shape allows independently authored phase configs and lifecycle
typestates to be paired at standalone entry points:

```text
ConfigurePhaseConfig + DeployedContract
ValidatePhaseConfig + ConfiguredContract
```

Those pairings are rejected today by repeated `network_id` and `expected_chain_id` comparisons in
operation and state code. That is a symptom of the real problem: the certified program contract does
not represent that the phase and the typestate belong to the same semantic context.

This RFC proposes a reusable model:

```text
CertifiedTransitionContext<C>
ContextBoundResource<Stage, C>
Import<C, Stage>
Transition<FromStage, ToStage, C>
```

For EVM contract lifecycle, this means:

```text
EvmContractContext
  -> deploy/import deployed
  -> configure/import configured
  -> validate
```

Standalone configure and validate flows are no longer modeled as "phase config plus raw typestate
seed." They are modeled as explicit import/admission flows into a certified context, followed by
ordinary lifecycle transitions.

This is a breaking design. No compatibility shim, fallback old entry point, or duplicate legacy
shape should be preserved.

The collector/facts work adds an adjacent authority surface that this RFC must account for:
`FactRecorded` claims, fact descriptor allow-lists, `FactQueryEvidence`, store-authenticated query
receipts, and public fact DTOs. Those facts can be replay and continuation evidence, such as a
collector checkpoint, but they are not automatically certified transition context. If a fact or fact
query is used inside a context-scoped workflow, the context binding must be explicit in certified
node/cell/query authority, not inferred from public fact output or projection rows.

## Problem

MFM currently treats EVM contract lifecycle phase configs and lifecycle typestates as independently
composable values:

- `DeployPhaseConfig`
- `ConfigurePhaseConfig`
- `ValidatePhaseConfig`
- `DeployedContract`
- `ConfiguredContract`

The full lifecycle path validates that deploy, configure, and validate configs target the same
network. Standalone configure and validate paths accept an external typestate seed plus a phase
config:

```text
ConfigurePhaseConfig + DeployedContract
ValidatePhaseConfig + ConfiguredContract
```

Those pairs can be constructed even when their semantic network contexts do not match. Current code
defends the boundary with local checks that compare:

```text
network_id
expected_chain_id
```

These checks appear in operation planning, state intent preparation, state output projection, and
validation read/report paths. That repetition is the architectural smell. The invariant is important
enough that many layers re-check it, but no layer owns it as a first-class typed program contract.

## Why Transport Guards Do Not Solve This

EVM transport guards prove a different invariant.

Transport guards answer:

```text
Did this live RPC/source route observe the chain id requested by the certified execution context?
```

They do not answer:

```text
Was this typestate seed produced for the same semantic context as the phase consuming it?
```

A configure phase can build live transport requests from its configure config while taking
`contract_address` from a separately supplied deployed typestate. The RPC source may correctly
observe the requested chain id, while the typestate address came from a different semantic network
or from unproven external material.

Transport routing is runtime capability selection. Typestate/context membership is certified
program semantics. They must remain separate responsibilities.

## Root Cause

The root cause is not the absence of a helper type for network equality.

The root cause is that the current lifecycle model has two independent authorities for the same
concept:

1. The phase config says which network the phase targets.
2. The typestate says which network produced the lifecycle artifact.

The type system then permits a consumer to combine those authorities and asks local code to reject
bad pairings after construction.

That is backward. A lifecycle transition should not receive two network authorities. It should
execute inside one certified transition context, and every consumed resource should already be bound
to that context.

## Goals

- Make context membership a certified program invariant.
- Remove repeated `network_id` and `expected_chain_id` matching as the primary safety mechanism.
- Delete public standalone entry points that pair a phase config with a raw typestate seed.
- Make standalone configure/validate flows explicit import flows.
- Keep transport guards narrowly responsible for live source and observed-chain validation.
- Provide a reusable pattern for future context-scoped domains, not only EVM contracts.
- Prefer clear breaking changes over compatibility layers.

## Non-Goals

- Preserve old EVM contract lifecycle schemas.
- Preserve old standalone configure/validate entry point shapes.
- Add a compatibility wrapper around `DeployedContract` or `ConfiguredContract`.
- Encode dynamic network ids as Rust phantom types.
- Add generic JSON-path predicates to the certifier as a substitute for domain modeling.
- Treat address-only external imports as proof of historical deployment origin.

## Implementation Gate

This RFC is implemented only when context binding is enforced by kernel, program, certifier, runtime,
admission, and replay authority.

A domain-only implementation is non-compliant. It is not enough to add `EvmContractContext`, store a
`context_ref` inside lifecycle payloads, rename phase configs to actions, or replace the old network
comparisons with new local helper checks. Those changes can be useful pieces of the cutover, but they
do not satisfy the RFC unless the certified spec can express the context invariant and the runtime can
enforce it before state invocation and output admission.

The following must be impossible or rejected by certified/kernel authority before the EVM lifecycle
cutover lands:

- consuming a context-bound resource under a different node context
- accepting a lifecycle resource from an unapproved producer or stage
- admitting an output whose payload context disagrees with the certified cell context
- importing external or source-run material without certified import policy and replay-verifiable
  evidence
- using rendered public JSON, projection rows, raw seeds, or loose payloads as lifecycle continuation
  authority

If kernel/program/certifier/runtime support is missing, the implementation must add that support
first. It must not land as an EVM-only convention guarded by state, adapter, or operation equality
checks.

## Proposed Architecture

### Core Concept

Introduce a reusable architectural concept:

```text
CertifiedTransitionContext<C>
```

`C` is canonical, hash-defining domain context. A context is semantic execution identity, not
process-local routing.

Context bytes are certified typed data. The context descriptor includes the semantic type identity,
schema/version identity, and canonicalization identity used to hash the context. Hashed context data
follows the repository-wide canonical JSON rule and must not contain floats.

Examples:

- EVM chain/network context.
- EVM contract lifecycle context.
- Portfolio valuation context.
- Protocol market context.
- Account or signer lane context when it is part of domain identity.
- Recurring collector source/checkpoint context when checkpoint identity is part of workflow
  continuation.

Current BTC collector checkpoints are encoded as `Control` facts with typed subject/query evidence.
That is valid fact-query replay authority, but it is not the same thing as
`CertifiedTransitionContext`. A future collector context cutover should either model that source and
checkpoint partition as certified context, or explicitly keep it as fact-query authority outside the
context-bound transition model.

A context has a stable digest:

```text
ContextRef =
  digest(domain_separator, context_descriptor_id, schema_version, canonicalizer_id, canonical C)
```

`ContextRef` is a content address, not a standalone capability. It becomes authority only through a
certified spec context table and certified cell/input constraints that bind a node, resource, or
output to that context reference.

Context-bound resources carry or are certified with that context reference:

```text
ContextBoundResource<Stage, C>
  context_ref
  stage
  domain identity
  provenance/evidence
```

Transitions consume context-bound resources and produce context-bound resources:

```text
Transition<FromStage, ToStage, C>
```

External material can enter a context only through explicit imports:

```text
Import<C, Stage>
```

Imports are ordinary certified state transitions with typed config, evidence, replay rules, and
public outputs. They are not raw seed shortcuts.

### Generic Rule

A state that consumes a context-bound resource must not also accept an independent context authority
that can disagree with that resource.

Bad shape:

```text
State(config_with_context, resource_with_context)
```

Good shape:

```text
State(certified_context<C>, resource_bound_to_context, action_without_context)
```

The certified context is materialized by runtime from the certified spec. It is not resolved from a
mutable runtime config and is not separately authored by the caller. The resource's `context_ref`,
the node's required context, and the materialized context value must all identify the same certified
context before the runner is invoked.

The same rule applies when a state consumes facts or fact-query results as workflow continuation
material. A fact subject, internal fact ref, `PublicFactRefId`, or returned projection row can carry
domain-scoping fields, but those fields do not become certified context by themselves. Context-bound
fact consumption must be represented by certified query/selection policy and retained
`FactQueryEvidence`, and any context-bearing fact payload must be validated against the consuming
node's certified context before it can drive a context-bound transition.

The intended API shape is a separate certified invocation binding, not another user config field or
input cell:

```text
StateSpec
  Context = C | NoContext
  Config = action_without_context
  Input = context_bound_resource | unit

CertifiedStateInvocation
  context: CertifiedContext<C>
  config: StateSpec::Config
  input: StateSpec::Input
```

Existing states that do not need semantic context use `NoContext`. Context-bound lifecycle states
declare `Context = EvmContractContext`. Program authors bind a node to a context handle/ref during
planning; certification records that binding in the spec context table and node constraints. Runtime
then passes the certified context through the runner invocation envelope. The context is not loaded
from mutable config and is not materialized by deserializing a public output field.

For root transitions that create the first resource, pass the context explicitly:

```text
RootTransition(certified_context<C>, action) -> resource_bound_to_context
```

For external starts, import first:

```text
Import(certified_context<C>, external_claim) -> resource_bound_to_context
```

Then continue with ordinary transitions.

## EVM Contract Lifecycle Model

### Context Types

EVM contract lifecycle should be scoped by a single context:

```text
EvmNetworkContext
  network_id
  expected_chain_id
  optional chain_fingerprint
  optional finality_or_observation_policy

EvmContractContext
  lifecycle_key
  network: EvmNetworkContext
  contract_profile
```

`network_id` remains semantic domain data. It is not a runtime `source_id` and does not need to
equal any process-local source or policy name.

`expected_chain_id` remains a concrete chain identity expected from live or replayed chain
observations.

`chain_fingerprint` is optional but recommended where chain id alone is too weak. Examples include
genesis hash or another non-secret identity value available through the relevant capability.

`finality_or_observation_policy`, when present, is shared observation identity for reads and replay:
for example a block tag, anchoring rule, or chain-observation stability rule that all phases must
share. It must not be a second transaction-finality authority. Mutating transaction finality,
receipt confirmation, and side-effect terminal policy belong in the action `receipt_policy` and the
certified side-effect contract. If one policy affects both shared observation and transaction
completion, operation lowering must author it once and derive the certified context/action material
from that single source.

`contract_profile` identifies the artifact, ABI/interface, bytecode class, or expected code shape
used by deploy, configure, and validate phases. For the EVM contract lifecycle, it is always present
in `EvmContractContext`. A minimal profile can contain only a profile id and declared interface
identity, but shared artifact/interface identity must not be repeated independently across phase
actions.

Profile fields should be digest-oriented and non-secret. Expected examples:

- profile id or lifecycle kind
- artifact digest or artifact ref, when deploy/configure/validate share one artifact
- ABI/interface digest, when read/write selectors are part of the lifecycle contract
- creation bytecode digest, deployed code hash, or code-shape policy when enforced
- selector/event compatibility policy for configure and validate assertions

Action-local fields remain the concrete work for that phase: constructor args, call values,
transaction policy, receipt policy, and assertion lists. Artifact bytes and ABI material should be
referenced by digest or typed artifact ref rather than copied into multiple actions.

### Action Types

Phase configs become phase-local actions. They do not carry network.

```text
DeployAction
  signer
  constructor_args
  value_wei
  transaction_policy
  receipt_policy

ConfigureAction
  signer
  calls
  confirmation_read_assertions
  confirmation_event_assertions
  transaction_policy
  receipt_policy

ValidateAction
  read_assertions
  event_assertions
```

The action says what to do. The context says where the lifecycle exists.

### Resource Types

Lifecycle typestates become context-bound resources, not independent authority containers.

```text
ContractInstance<Deployed>
  context_ref
  address
  deploy_provenance
  deploy_evidence
  deployed_block_number

ContractInstance<Configured>
  context_ref
  address
  configured_from
  configuration_claim
  configure_or_import_provenance
  configure_or_import_evidence
  configured_block_number
  asserted_configuration_snapshot
```

The deployed/configured values should not duplicate `network_id` and `expected_chain_id` as
authoritative fields. Public renderers can include network information by joining the output with
the certified context. The context, not copied typestate fields, is authority.

`Configured` should not nest the full `Deployed` value as its identity boundary. It should reference
the configured contract identity and provenance under the same context. If detailed deploy evidence
is useful for audit output, it should be referenced or joined through lineage/evidence, not copied as
a second source of truth.

`configuration_claim` describes why the instance is considered configured. For a normal configure
transition it points at the configure action, calls, confirmations, and recorded evidence. For an
imported configured instance it points at the import claim and the admission evidence. It must be
honest: an adopted external address cannot claim MFM configuration unless the import source proves
that provenance.

The claim should be a closed enum so renderers and replay cannot infer meaning from loose labels:

```text
ConfigurationClaim
  MfmConfigured
    configure_node
    configure_action_digest
    call_evidence_refs
    confirmation_evidence_refs

  ImportedMfmConfigured
    source_run_id
    source_spec_hash
    source_cell_or_output_id
    source_value_digest
    source_context_ref

  ExternalObservedConfigured
    provenance_label
    evidence_policy_digest
    external_adoption_evidence_digest

  ExternalClaimedConfigured
    provenance_label
    evidence_policy_digest
```

`ExternalClaimedConfigured` is allowed only when certified import policy permits an unverified
configured claim. Validation and public output must not synthesize successful configure call,
configuration read, or configuration event results for that variant. If the public schema requires
configuration result arrays, they should be empty and accompanied by provenance/status that marks
the claim as externally adopted and unverified.

`asserted_configuration_snapshot` is optional typed evidence for configured-state semantics, such as
read assertion results or event assertion anchors. If an externally adopted configured instance does
not require those checks, public output must render the configuration evidence as imported or
unverified rather than as successful MFM configure results.

### Transitions

Full lifecycle:

```text
Deploy(EvmContractContext, DeployAction)
  -> ContractInstance<Deployed>

Configure(ContractInstance<Deployed>, ConfigureAction)
  -> ContractInstance<Configured>

Validate(ContractInstance<Configured>, ValidateAction)
  -> ValidationReport(context_ref)
```

Configure-only:

```text
ImportDeployed(EvmContractContext, ImportDeployedSpec)
  -> ContractInstance<Deployed>

Configure(ContractInstance<Deployed>, ConfigureAction)
  -> ContractInstance<Configured>
```

Validate-only:

```text
ImportConfigured(EvmContractContext, ImportConfiguredSpec)
  -> ContractInstance<Configured>

Validate(ContractInstance<Configured>, ValidateAction)
  -> ValidationReport(context_ref)
```

There is no public semantic operation shaped as:

```text
ConfigureAction + DeployedContract
ValidateAction + ConfiguredContract
```

The import node produces the same stage handle that deploy/configure would have produced. Downstream
phases consume only stage handles.

`ValidationReport` is a context-bound terminal typed value, not a lifecycle stage producer. It should
carry `context_ref`, the configured instance identity, assertion result evidence, and recorded
capability evidence refs. Public renderers can turn it into JSON, but rendered public JSON is not
authority for later imports or replay.

Validation reports are state-output evidence unless an implementation deliberately adds a separate
typed validation fact descriptor. References to validation evidence in this RFC mean validation read
evidence plus the terminal report/output evidence, not an implicit `FactRecorded` event.

### Public Entry Shapes

The public EVM lifecycle inputs should have one context object and phase-local action objects.

Full lifecycle:

```text
context: EvmContractContext
deploy: DeployAction
configure: ConfigureAction
validate: ValidateAction
```

Configure-only:

```text
context: EvmContractContext
import_deployed: ImportDeployedSpec
configure: ConfigureAction
```

Validate-only:

```text
context: EvmContractContext
import_configured: ImportConfiguredSpec
validate: ValidateAction
```

Old public shapes that pair `{ config, deployed }` or `{ config, configured }` are rejected. The
cutover should use new entry-point schema names or versions so callers cannot accidentally send an
old shape that is interpreted as a weak import.

## Import Semantics

Imports are the explicit trust boundary for external lifecycle material.

Two import families are expected.

### Import From MFM Run

The public import config is a single `from_mfm_run` envelope with both required parts:

```text
ImportDeployedSpec::FromMfmRun { source: ImportFromMfmRun, evidence: ImportFromMfmRunEvidence }
ImportConfiguredSpec::FromMfmRun { source: ImportFromMfmRun, evidence: ImportFromMfmRunEvidence }
```

Omitting `evidence` is invalid. There is no identifier-only source-run import shape.

```text
ImportFromMfmRun
  source_run_id
  source_spec_hash
  source_cell_or_output_id
  source_value_digest
  source_context_ref
  required_stage
  accepted_context_policy
```

Admission verifies the certified import policy, retained source typed-spec certificate artifact,
retained source run stream or certified export bundle artifact, source value artifact, value digest,
schema and semantic ids, lifecycle stage, context ref, and context descriptor. For certified export
bundles, the bundle carries canonical source typed-spec JSON; replay verifies that spec and
certificate against the compiled certification registry before accepting any terminal event. The
imported resource is valid only if the source material was produced for the same certified context
or for a context explicitly accepted by the import policy.

This import proves MFM provenance.

The source must be verified through authoritative typed evidence, not rendered public JSON. Valid
source authority is either:

- a `CommittedRunStream` plus `VerifiedRunArtifactStore` from a trusted MFM store boundary
- an explicitly certified export/import bundle whose retained typed-spec certificate, embedded
  canonical source spec, and retained artifacts verify against the compiled registry

`source_cell_or_output_id` means a typed cell or typed terminal output binding in that authority
surface. It does not mean a CLI/REST JSON field, cached public-output document, or projection row.

The import state records an import evidence bundle with enough retained proof material for replay:

```text
ImportFromMfmRunEvidence
  source_spec_hash
  source_spec_certificate_ref
  source_run_stream_ref_or_export_bundle_ref
  source_cell_or_output_id
  source_cell_schema_id
  source_cell_semantic_type_id
  source_producer_descriptor_id
  source_stage
  source_context_ref
  source_context_descriptor_id
  source_value_digest
  source_value_artifact_ref_or_inline_canonical_value
  source_terminal_cell_or_output_event_ref
  import_policy_digest
```

Replay verifies the imported resource from that recorded evidence plus the certified import config.
Missing retained proof, mismatched digest, wrong type, wrong stage, wrong context, public JSON,
projection rows, raw payloads, and identifiers alone all fail closed before any lifecycle resource is
produced. Replay does not query a live store, current runtime config, or current public-output
renderer.

`accepted_context_policy` is hash-defining certified config. The default policy is exact context
ref equality. Any cross-context adoption must name explicit accepted context refs or a typed,
certified compatibility relation that replay can verify from stored evidence only.

### Adopt External Address

```text
AdoptExternalAddress
  address
  provenance_label
  evidence_policy
  optional_expected_code_hash
  optional_block_anchor
  optional_initial_read_assertions
  optional_initial_event_assertions
```

This import does not prove historical deployment origin. It declares that MFM is adopting an
external address under a certified context, with verification requirements set by certified
`evidence_policy`. The public output must be honest about that provenance.

`evidence_policy` is hash-defining certified config. It declares which observations are required
for admission, including chain identity, code existence, code hash, block anchoring, initial reads,
and initial events. Defaults may be applied only by schema or operation lowering before
certification, where missing fields are materialized into `evidence_policy`. Runtime, admission, and
replay must never infer missing policy fields from current defaults.

Code existence and code hash require an EVM code-read capability with replayable evidence, such as:

```text
EvmCodeReadCapability
  get_code(address, block_anchor, guard) -> code_bytes_or_hash_evidence
```

If the current EVM capability surface does not expose code reads, that capability must be added
before the default external-adoption policy can be implemented.

This import proves only what its evidence proves:

- address syntax and normalization
- required evidence policy was satisfied
- observed code hash, if requested
- observed assertions, if requested
- observed chain identity through transport/capability evidence
- block anchoring, if requested

It does not prove that MFM deployed or configured the contract unless the import source is a verified
MFM run or another explicitly certified provenance mechanism.

For the lowered default external-adoption policy, MFM should require observed chain identity and code
existence at the adopted address. Producing a `Configured` stage from external adoption additionally
requires either configured-state assertion evidence or a certified policy that permits
`ExternalClaimedConfigured`. Public output only renders that typed authority; it is not semantic
permission.

## Certified Spec Requirements

The certified spec must make context binding hash-defining.

At minimum, certified lifecycle specs need to represent:

- a context table mapping `ContextRef` to certified context descriptors and canonical context bytes
- node required-context constraints
- output/cell context constraints, including resource stage and semantic resource kind
- input binding trees from context-bound resources to consuming states
- producer/stage constraints for each context-bound resource type
- import policy material for externally admitted stages
- lineage from import/deploy/configure outputs to later configure/validate inputs
- side-effect resource lane derivation inputs when the state crosses an external mutation boundary
- context-bound fact/query constraints when facts participate in a context-scoped workflow

The certifier should reject any graph where a transition consumes a context-bound resource under a
different context than the transition requires.

The certifier should also reject unapproved producers. For EVM contract lifecycle:

- `ContractInstance<Deployed>` can be produced by deploy or import-deployed states.
- `ContractInstance<Configured>` can be produced by configure or import-configured states.
- validate consumes configured instances but does not mint lifecycle instances.

This avoids a weak model where any seed with the right schema can masquerade as a lifecycle stage.

Fact descriptor allow-lists certify which fact shapes a node may emit, but they are not a substitute
for context membership. If a fact claim, fact response artifact, or fact-query result is
context-bound, the certified spec must say which node context, fact descriptor, resource kind/stage,
and query/selection policy are allowed. Replay must then verify the retained fact authority and
authenticated query receipt against those constraints.

A `context_ref` field inside a resource payload is not enough. The certified output cell descriptor
must say that the cell is bound to a specific context ref and stage. Runtime must validate committed
state outputs against that descriptor before it admits the output as a typed cell value. A consumer
must bind inputs through the certified input tree, so a matching payload copied from elsewhere cannot
be substituted for the certified producer output.

For a state invocation, runtime materializes the certified context value from the spec and provides
it to the runner together with the typed input values. Adapters and states must not resolve
`context_ref` through runtime config, public output, projection tables, or live routing registries.

The store remains domain-agnostic append authority. It should not infer EVM context semantics, but
it must only admit commits prepared by runtime/app authority that has already validated output
context constraints, artifact evidence, and side-effect ledger transitions.

## Layer Responsibilities

### Domain Model

Owns:

- `EvmNetworkContext`
- `EvmContractContext`
- action types
- context-bound contract instance types
- import specs
- evidence policy and configuration claim enums
- provenance and evidence refs

Does not own:

- runtime source routing
- RPC clients
- signer material resolution
- workflow topology

### Operation Layer

Owns:

- lifecycle graph topology
- certified context declarations
- full lifecycle entry points
- configure-only import-plus-configure topology
- validate-only import-plus-validate topology
- public output bindings

Does not own:

- runtime IO
- signer resolution
- transport guard behavior
- ad hoc context checks that should be certified graph constraints

### State Layer

Owns:

- deterministic deploy/configure/validate/import semantics
- intent construction from certified context plus typed inputs
- output projection from typed evidence
- replay-checkable request/response contracts

States should receive an already-certified context materialized by runtime and, for non-root
transitions, a context-bound resource whose certified input constraint points at the same context.
They should not receive a phase config network and a typestate network that can disagree.

### Adapter Layer

Owns:

- runner binding from certified state intent to capabilities
- prepared invocation evidence
- side-effect request/receipt/confirmation evidence
- validation read evidence

Adapters build capability requests from certified context and context-bound resources. The certified
context supplies semantic network identity and guard expectations. Certified capability/source policy
supplies any outcome-affecting route constraints. Endpoints, credentials, and fallback mechanics
remain runtime-local transport data. Adapters must not treat a `context_ref` as a process-local route
id and must not combine a network from phase config with an address from an independent typestate
seed.

### Transport Layer

Owns:

- process-local source refs
- policies and fallback
- endpoint and authorization redaction
- live chain identity reads
- RPC calls, logs, transactions, receipts

Transport guards prove observed live network behavior. They do not prove typestate membership in a
semantic context.

### Runtime, Store, Replay

Runtime owns:

- materializing certified contexts and context-bound inputs
- enforcing certified producer/input/context requirements before runner invocation
- validating committed outputs against certified cell/context expectations
- preparing import evidence and side-effect evidence under certified node context
- staging context-bound `FactRecorded` claims and `FactQueryEvidence` only through certified
  descriptor/query authority

Store owns:

- append-only event authority
- artifact evidence admission
- atomic commit behavior
- projection rebuild from stream authority
- fact descriptor, fact response, and fact query evidence artifacts as strict authority
- fact indexes, terms, descriptor catalogs, and public fact refs as rebuildable/query surfaces, not
  lifecycle import authority

Replay owns:

- reconstructing expected requests from certified context and recorded inputs
- verifying `FactRecorded`, `FactQueryEvidence`, receipts, confirmations, imports, and outputs from
  recorded evidence only
- verifying authenticated fact-query receipts, retained returned refs, and `Control`/`Platform`
  visibility boundaries without consulting the live fact index
- verifying source-run imports from recorded import evidence or certified export bundles
- never consulting live routing, mutable environment config, current source registries, or public
  output renderers

## Deletions Required

This design intentionally deletes old concepts rather than wrapping them.

Delete:

- per-phase `network` fields from deploy/configure/validate configs
- lifecycle configs that repeat network in each phase
- public standalone configure config shaped as `{ config, deployed }`
- public standalone validate config shaped as `{ config, configured }`
- raw standalone typestate seeds for lifecycle continuation
- authoritative `network_id` and `expected_chain_id` fields on lifecycle typestates
- `ConfiguredContract` nesting the full `DeployedContract` as its identity boundary
- repeated `ensure_phase_networks_match`
- repeated `ensure_network_matches_deployed`
- repeated `ensure_network_matches_configured`
- adapter runtime lookup by loose `config.network().network_id()`
- imports from rendered public JSON, projection rows, or raw typestate payloads
- any language implying an EVM transport guard proves typestate/context membership

Replace with:

- one certified `EvmContractContext`
- networkless action configs
- context-bound contract instances
- explicit import states
- certified producer/context constraints
- certified context materialization for state and adapter execution
- runtime/admission verification of context-bound inputs and imported material
- replay-verifiable import evidence bundles

## Validation Code This Removes

This RFC is expected to make the implementation smaller, not only safer. The new certified context
invariant should delete repeated local checks whose only job is to defend against pairing two
independent network authorities after construction.

The following current EVM lifecycle code patterns should disappear after the context-bound model is
implemented.

### Operation Planning Checks

Delete phase-to-phase network equality checks:

```text
validate_contract_lifecycle_config
ensure_phase_networks_match
ensure_networks_match
network_mismatch_error
```

Full lifecycle planning no longer needs to compare deploy/configure/validate phase configs, because
there is one `EvmContractContext` and phase actions do not carry network.

Delete standalone seed pairing checks:

```text
ConfigurePhaseConfig.network == DeployedContract.network
ValidatePhaseConfig.network == ConfiguredContract.deployed.network
```

Configure-only and validate-only planning should not admit raw typestate seeds. They should build an
import node under a certified context, then consume the imported stage handle. Any mismatch is a
certification/admission failure on the import edge, not an operation-local string comparison.

Delete EVM lifecycle seed plumbing that exists only to smuggle raw continuation typestates into a
new run:

```text
ContractConfigureEntryPointConfig { config, deployed }
ContractValidateEntryPointConfig { config, configured }
configure_contract_program_draft(config, deployed)
validate_contract_program_draft(config, configured)
seed bytes for DEPLOY_SEED_KEY / CONFIGURED_SEED_KEY continuation
```

The replacement entry points are context-plus-import specs. The import state owns provenance,
evidence, and replay behavior.

### State Checks

Delete state-local network matching used to guard configure and validate inputs:

```text
ensure_network_matches
ConfigureContractState::prepare_intent input/config network check
ConfigureContractState::output_from_receipt input/config network check
ConfigureContractState::output_from_confirmation input/config network check
ValidateContractState::read_request input/config network check
ValidateContractState::report_from_response input/config network check
```

Those checks are redundant once runtime invokes the state with:

```text
CertifiedContext<EvmContractContext>
ContractInstance<Stage> bound to the same context
networkless action config
```

State code should use the materialized certified context to build intents and reports. It should not
compare a phase config network against a typestate network, because neither side exists as an
independent authority.

Delete intent/report construction that copies network authority from phase config into lifecycle
typestates or report refs:

```text
transaction_intent_from_deploy_config copying network_id / expected_chain_id from phase config
transaction_intent_from_configure_call copying network_id / expected_chain_id from phase config
ConfiguredContractRef::from_configured copying network_id / expected_chain_id from nested deployed
ValidationReport.expected_chain_id copied from validate config
```

The intent still carries the guard expectations needed by the capability call, but those values come
from certified context. Public reports render network data by joining to context.

### Model Invariant Fields

Delete copied context authority from lifecycle values:

```text
DeployedContract.network_id
DeployedContract.expected_chain_id
ConfiguredContract.deployed as the configured identity boundary
ConfiguredContractRef.network_id
ConfiguredContractRef.expected_chain_id
ValidationReport.expected_chain_id as an independently copied field
```

Replace those fields with:

```text
context_ref
stage identity
contract address
provenance/evidence refs
configuration claim
```

Any network or chain fields shown to users are rendered from certified context and recorded
capability evidence, not copied through every typestate.

### Adapter Routing Checks

Delete adapter routing that selects process-local runtime by loose phase config network:

```text
runtime_for(config.network().network_id())
read_runtime_for(config.network().network_id())
validate_runtime_for(config.network().network_id(), ...)
side-effect plan network_id() forwarding phase config network
```

Adapters should receive certified context and certified capability/source policy. They build EVM
guards and runtime capability requests from those certified inputs. They should not recover a
network from phase config while taking the address from an independently supplied typestate.

Delete prepared/replay checks that merely re-derive phase-config network equality. Replace them with
context-bound evidence validation:

```text
prepared invocation context_ref matches certified node context
receipt/submission/confirmation evidence context_ref matches certified node context
validation read evidence and report state-output evidence context_ref match certified node context
input resource context_ref matches certified node context
```

### Test Cases To Remove Or Rewrite

Remove tests whose only purpose is to prove repeated copied network fields agree across old shapes:

```text
full lifecycle rejects deploy/configure network mismatch
full lifecycle rejects deploy/validate network mismatch
configure-only rejects config/deployed network mismatch
validate-only rejects config/configured network mismatch
configured ref copies network fields from configured.deployed
validation report copies expected_chain_id from validate config
```

Replace them with tests at the certified boundary:

```text
certifier rejects a configure edge from a deployed instance in another context
certifier rejects a validate edge from a configured instance in another context
runtime rejects committed output whose payload context_ref disagrees with the certified cell
import admission rejects mismatched source context unless certified import policy allows it
public JSON cannot be used as import authority
transport guard still rejects observed chain mismatch
```

## Validations That Stay

This RFC removes duplicated context-membership checks. It does not remove validations that prove
different invariants.

Keep:

- schema, semantic type, no-float, and canonical JSON validation
- address syntax and normalization
- signer reference and expected signer address validation
- transaction, receipt, confirmation, idempotency, and nonce-lane validation
- EVM transport guard checks for observed chain id and optional chain fingerprint
- evidence and artifact digest verification
- prepared invocation public-surface and secret-leak checks
- replay checks that `FactRecorded`, `FactQueryEvidence`, receipts, confirmations, imports, and
  outputs match certified context and recorded evidence
- public output rendering checks that join context data without making rendered JSON authority

The cleanup target is local defensive equality code caused by duplicate authorities. The new
invariant is enforced once at certification, runtime input materialization, output admission, import
admission, and replay evidence verification.

## Why Not Rust Phantom Network Types

Rust phantom types are the wrong abstraction for semantic networks in MFM.

MFM networks are authored, canonicalized, certified, persisted data. They are not a closed set of
compile-time Rust types. A type like `DeployedContract<Mainnet>` would only work for networks known
at compile time and would fail to model user-defined semantic networks, local dev chains, and
certified context values loaded from persisted specs.

The invariant must live in certified typed program data and runtime authority, not in compile-time
type names.

## Why Not Generic Certifier JSON Predicates

Generic JSON-path predicates would create a second weak type system:

```text
node.config.network.network_id == input.deployed.network_id
```

That repeats the current problem in a different layer. The certifier would know string paths, but it
would still not understand lifecycle context, producer authority, import semantics, or stage
transitions.

The certifier should validate typed domain concepts:

```text
this configured contract instance is bound to this certified EVM contract context
```

not generic JSON equality expressions.

## Public Output Semantics

Public output may include network and chain information for usability, but it should render that
data from the certified context.

Do not rely on copied `network_id` and `expected_chain_id` fields inside lifecycle resources as
authority. If a report needs to show:

```text
network_id
expected_chain_id
observed_chain_id
contract_address
```

then:

- `network_id` and `expected_chain_id` come from the certified context
- `observed_chain_id` comes from recorded capability evidence
- `contract_address` comes from the context-bound resource

This keeps audit output useful without making copied fields a second authority surface.

Rendered output may include both the `context_ref` and selected context fields for auditability.
Those rendered fields are not import authority. A later run that wants to continue from a previous
run must import from typed run evidence or a certified export bundle, not from copied public JSON.

Public fact APIs are the same kind of public surface. `Platform` fact DTOs and opaque
`PublicFactRefId` values can help users find recorded facts, but they are not source-run evidence,
artifact authority, or lifecycle continuation authority. `Control` facts may be used by internal
collector/runtime workflows through authenticated query evidence; they must stay hidden from public
fact APIs and must still be replayed from retained `FactQueryEvidence`, not from live projections.

## Resource Lane Implications

Side-effect resource lanes should be derived from the certified side-effect contract, not by bare
chain id or free-form network string.

For EVM signer nonce mutation lanes, the key should include:

```text
evm_network_context_ref
expected_signer_address
```

The network context ref, not the full contract context ref, is the default nonce lane partition. Two
different contract lifecycles on the same chain and signer still share the same external account
nonce. A full `EvmContractContext` may appear in evidence, but using it as the only nonce lane key
would allow unsafe concurrency across contracts unless a stronger nonce manager is specified.

## Expected Benefits

- Wrong-network configure/validate edges become structurally invalid.
- Standalone workflows become explicit and auditable.
- Imports become honest trust boundaries.
- Transport guards have one clear responsibility.
- States no longer duplicate network matching checks as their primary invariant.
- Typestates stop carrying copied context authority.
- Replay can verify context and provenance from certified spec plus recorded evidence.
- The same pattern can be reused by other context-scoped domains.

## Risks And Resolved Decisions

### Import Evidence Strength

Address-only adoption is weak. It may be acceptable for some operational workflows, but public
output must state that it is adoption, not proof of deployment origin.

Default policy:

- require observed chain identity
- require code exists at address
- require code hash when the import config declares one
- require block anchor when the import config declares one
- require configured read/event assertions before producing `Configured` unless certified import
  policy permits `ExternalClaimedConfigured`

### Context Granularity

`EvmNetworkContext` may be enough for some workflows. EVM contract lifecycle likely needs
`EvmContractContext`, because artifact/interface identity often scopes deploy, configure, and
validate semantics too.

Decision: EVM contract lifecycle uses `EvmContractContext` with a `contract_profile`. A minimal
profile is allowed for simple deploy-only workflows, but when multiple phases depend on the same
artifact/interface identity, that identity belongs to context and is referenced by action-local
work.

### Kernel Support

This RFC uses generic terms like `CertifiedTransitionContext` and `ContextBoundResource`.

Decision: implement minimal kernel/program support for context refs, certified context tables,
context-bound cell constraints, input context constraints, and producer/stage constraints. Domain
crates define the concrete context and resource types. A domain-only convention is not sufficient
for this RFC, because it would leave the core invariant outside the certified runtime contract.

The EVM lifecycle cutover is implemented on those certified constraints, not on local helper checks
that imitate the new model.

## Cutover Plan

This is not a compatibility migration.

The implemented order keeps the EVM schema and entry-point break after kernel/program/certifier/
runtime support can enforce context-bound inputs, outputs, producer/stage constraints, and import
evidence.

1. Add the context-bound model and certified context constraints.
2. Add EVM context, action, resource, import, report, and evidence model types.
3. Add the EVM code-read capability and replay evidence needed by external adoption.
4. Replace EVM contract lifecycle configs with context plus action configs.
5. Replace lifecycle typestates with context-bound contract instances.
6. Replace standalone configure/validate with import-based entry points.
7. Update adapters to build requests from certified context and context-bound inputs.
8. Update prepared invocation, fact, receipt, confirmation, and validation evidence to include the
   relevant context refs and stage identities.
9. Update replay to recompute requests from certified context and verify context-bound evidence.
10. Delete old config/typestate pair entry points and old network matching helpers.
11. Update CLI/REST/docs/tests to expose only the new shapes and reject the old shapes.

Old persisted specs and old public JSON contracts are not preserved by this RFC. If old behavior is
needed for analysis, recover it from git history rather than carrying it forward in production code.

## Test Strategy

Tests should prove the wrong shape is impossible or rejected at the certified boundary.

Required tests:

- compile-fail or certification-fail test for configure consuming a deployed instance from a
  different context
- compile-fail or certification-fail test for validate consuming a configured instance from a
  different context
- import-from-MFM-run accepts matching source context and rejects mismatched source context
- external address adoption records honest provenance and does not claim MFM deployment
- external address adoption records and replays code-existence/code-hash evidence according to
  certified evidence policy
- adapter route selection uses certified context, not phase action config
- transport guard still rejects observed chain mismatch
- replay verifies context refs and fails closed on mismatched import, fact, receipt, or output
  evidence
- public output renders network data from context, not copied typestate fields
- public output is rejected as import authority
- old `{ config, deployed }` and `{ config, configured }` public input shapes are rejected
- prepared invocation, receipt, confirmation, and validation evidence are rejected when their
  context refs do not match the certified node context
- EVM nonce resource lane keys use certified network context plus signer, not full contract context
  alone

## Decision

Adopt context-scoped state transitions as the lifecycle modeling rule.

For EVM contract lifecycle, delete independently composable phase-config plus typestate pairings.
Represent network and contract identity once as certified context. Represent deployed/configured
contracts as resources bound to that context. Represent standalone starts as explicit imports into
that context.

The invariant should no longer be:

```text
compare phase.network_id with typestate.network_id at every call site
```

It should be:

```text
the certified graph cannot express a transition that consumes a resource outside its context
```
