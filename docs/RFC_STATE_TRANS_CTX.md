# RFC: State Transition Contexts

Status: proposed

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

## Proposed Architecture

### Core Concept

Introduce a reusable architectural concept:

```text
CertifiedTransitionContext<C>
```

`C` is canonical, hash-defining domain context. A context is semantic execution identity, not
process-local routing.

Examples:

- EVM chain/network context.
- EVM contract lifecycle context.
- Portfolio valuation context.
- Protocol market context.
- Account or signer lane context when it is part of domain identity.

A context has a stable digest:

```text
ContextRef = digest(canonical C)
```

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
State(resource_bound_to_context, action_without_context)
```

For root transitions that create the first resource, pass the context explicitly:

```text
RootTransition(context, action) -> resource_bound_to_context
```

For external starts, import first:

```text
Import(context, external_claim) -> resource_bound_to_context
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

`contract_profile` identifies the artifact, ABI/interface, bytecode class, or expected code shape
used by deploy, configure, and validate phases. The exact fields can evolve by domain need, but the
profile belongs to context when all lifecycle phases depend on the same contract identity.

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
  configure_provenance
  configure_evidence
  configured_block_number
```

The deployed/configured values should not duplicate `network_id` and `expected_chain_id` as
authoritative fields. Public renderers can include network information by joining the output with
the certified context. The context, not copied typestate fields, is authority.

`Configured` should not nest the full `Deployed` value as its identity boundary. It should reference
the configured contract identity and provenance under the same context. If detailed deploy evidence
is useful for audit output, it should be referenced or joined through lineage/evidence, not copied as
a second source of truth.

### Transitions

Full lifecycle:

```text
Deploy(EvmContractContext, DeployAction)
  -> ContractInstance<Deployed>

Configure(ContractInstance<Deployed>, ConfigureAction)
  -> ContractInstance<Configured>

Validate(ContractInstance<Configured>, ValidateAction)
  -> ValidationReport
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
  -> ValidationReport
```

There is no public semantic operation shaped as:

```text
ConfigureAction + DeployedContract
ValidateAction + ConfiguredContract
```

The import node produces the same stage handle that deploy/configure would have produced. Downstream
phases consume only stage handles.

## Import Semantics

Imports are the explicit trust boundary for external lifecycle material.

Two import families are expected.

### Import From MFM Run

```text
ImportFromMfmRun
  source_run_id
  source_spec_hash
  source_cell_or_output_id
  source_value_digest
  source_context_ref
  required_stage
```

Admission verifies the source run, certified spec, stream evidence, value digest, stage, and context
reference. The imported resource is valid only if the source material was produced for the same
certified context or for a context explicitly accepted by the import policy.

This import proves MFM provenance.

### Adopt External Address

```text
AdoptExternalAddress
  address
  provenance_label
  optional_code_hash
  optional_block_anchor
  optional_initial_read_assertions
  optional_initial_event_assertions
```

This import does not prove historical deployment origin. It declares that MFM is adopting an
external address under a certified context, optionally after live/read verification. The public
output must be honest about that provenance.

This import proves only what its evidence proves:

- address syntax and normalization
- observed code hash, if requested
- observed assertions, if requested
- observed chain identity through transport/capability evidence
- block anchoring, if requested

It does not prove that MFM deployed or configured the contract unless the import source is a verified
MFM run or another explicitly certified provenance mechanism.

## Certified Spec Requirements

The certified spec must make context binding hash-defining.

At minimum, certified lifecycle specs need to represent:

- context definitions and context refs
- which nodes execute under which context
- which cells/resources are bound to which context
- which states are authorized producers for each stage
- which imports are authorized producers for externally admitted stages
- input binding trees from context-bound resources to consuming states
- lineage from import/deploy/configure outputs to later configure/validate inputs

The certifier should reject any graph where a transition consumes a context-bound resource under a
different context than the transition requires.

The certifier should also reject unapproved producers. For EVM contract lifecycle:

- `ContractInstance<Deployed>` can be produced by deploy or import-deployed states.
- `ContractInstance<Configured>` can be produced by configure or import-configured states.
- validate consumes configured instances but does not mint lifecycle instances.

This avoids a weak model where any seed with the right schema can masquerade as a lifecycle stage.

## Layer Responsibilities

### Domain Model

Owns:

- `EvmNetworkContext`
- `EvmContractContext`
- action types
- context-bound contract instance types
- import specs
- provenance and evidence refs

Does not own:

- runtime source routing
- RPC clients
- signer material resolution
- workflow topology

### Operation Layer

Owns:

- lifecycle graph topology
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
- intent construction under a certified context
- output projection from typed evidence
- replay-checkable request/response contracts

States should receive an already-certified context or a context-bound resource. They should not
receive a phase config network and a typestate network that can disagree.

### Adapter Layer

Owns:

- runner binding from certified state intent to capabilities
- prepared invocation evidence
- side-effect request/receipt/confirmation evidence
- validation read evidence

Adapters route by certified context. They use addresses from context-bound resources. They do not
combine a network from phase config with an address from an independent typestate seed.

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

Store owns:

- append-only event authority
- artifact evidence admission
- atomic commit behavior
- projection rebuild from stream authority

Replay owns:

- reconstructing expected requests from certified context and recorded inputs
- verifying facts, receipts, confirmations, imports, and outputs from recorded evidence only
- never consulting live routing or mutable environment config

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
- any language implying an EVM transport guard proves typestate/context membership

Replace with:

- one certified `EvmContractContext`
- networkless action configs
- context-bound contract instances
- explicit import states
- certified producer/context constraints
- runtime/admission verification of context-bound inputs and imported material

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

## Resource Lane Implications

Side-effect resource lanes should be keyed by certified context, not by bare chain id or free-form
network string.

For EVM signer nonce mutation lanes, the key should include:

```text
context_ref
expected_signer_address
```

or a deliberate narrower/wider lane definition if the architecture chooses one. The important point
is that the lane key should derive from certified execution context rather than repeated,
call-site-local strings.

## Expected Benefits

- Wrong-network configure/validate edges become structurally invalid.
- Standalone workflows become explicit and auditable.
- Imports become honest trust boundaries.
- Transport guards have one clear responsibility.
- States no longer duplicate network matching checks as their primary invariant.
- Typestates stop carrying copied context authority.
- Replay can verify context and provenance from certified spec plus recorded evidence.
- The same pattern can be reused by other context-scoped domains.

## Risks And Open Questions

### Import Evidence Strength

Address-only adoption is weak. It may be acceptable for some operational workflows, but public
output must state that it is adoption, not proof of deployment origin.

Open question: what minimum evidence should MFM require for `AdoptExternalAddress` by default?

Possible answer:

- require observed chain identity
- require code exists at address
- optionally require code hash when available
- optionally require configured read assertions before producing `Configured`

### Context Granularity

`EvmNetworkContext` may be enough for some workflows. EVM contract lifecycle likely needs
`EvmContractContext`, because artifact/interface identity often scopes deploy, configure, and
validate semantics too.

Open question: should `EvmContractContext` always include contract profile, or should profile remain
action-local for deploy-only workflows?

The default should be conservative: if multiple phases depend on the same artifact/interface
identity, put it in context.

### Kernel Support

This RFC uses generic terms like `CertifiedTransitionContext` and `ContextBoundResource`.

Open question: should these be first-class kernel/program primitives or domain-level conventions
validated by state descriptors and certifier extensions?

The preferred direction is a minimal kernel primitive for context refs and context-bound cell
constraints, with domain crates defining the concrete context/resource types.

## Cutover Plan

This is not a compatibility migration.

1. Add the context-bound model and certified context constraints.
2. Replace EVM contract lifecycle configs with context plus action configs.
3. Replace lifecycle typestates with context-bound contract instances.
4. Replace standalone configure/validate with import-based entry points.
5. Update adapters to route from certified context.
6. Update replay to recompute requests from certified context and context-bound inputs.
7. Delete old config/typestate pair entry points and old network matching helpers.
8. Update CLI/REST/docs/tests to expose only the new shapes.

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
- adapter route selection uses certified context, not phase action config
- transport guard still rejects observed chain mismatch
- replay verifies context refs and fails closed on mismatched import, fact, receipt, or output
  evidence
- public output renders network data from context, not copied typestate fields
- resource lane keys include certified context identity

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

