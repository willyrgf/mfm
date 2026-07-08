# RFC: Capability Source Authority Boundary

Status: Draft

## Summary

MFM should have one external-source boundary concept: the capability request/response provider
contract.

The current `EvmChainGuard` and `BtcChainGuard` model has become a second public authority concept.
It appears in states, adapters, app/replay validation, transport route checks, and tests. That spreads
source-authority responsibility across the codebase and forces repeated evidence validation after
successful provider calls.

This RFC replaces that split with a smaller rule:

- Existing config/context/intent values carry workflow semantic authority.
- Adapters build external capability requests from that semantic authority.
- Capability provider backends enforce request source authority before returning success.
- Live backends enforce by resolving runtime routes and probing the external source.
- Replay/recorded backends enforce by checking recorded evidence against the same request.
- States, adapters, app replay, public-output code, and binaries do not re-verify provider source
  evidence after a successful provider response.

The public boundary is the capability request. Source authority should be private request data or a
private helper inside the capability/request implementation, not a public `*ChainGuard` type passed
around as domain authority.

## Problem

### Guards Became General Authority

`EvmChainGuard` and `BtcChainGuard` were meant to protect provider calls. In practice, guard
construction and guard verification now appear in several layers:

- capability crates expose public guard types, guarded request structs, evidence, and generic
  `verify_guard` / `verify_request` helpers
- transport crates enforce source identity
- adapters build guarded requests and often re-verify successful responses
- `mfm-states-btc` constructs `BtcChainGuard` and `BtcChainHeadRequest`
- app/replay code reconstructs EVM guards to validate diagnostic evidence
- docs describe replay as checking recorded evidence against request guards

That makes the guard look like semantic workflow authority. It is not. The semantic authority is
already present in certified config, context, and state intent fields such as:

- EVM `network_id` and `expected_chain_id`
- Bitcoin `network_id`, `semantic_source_identity`, and `bitcoin_network`

The guard abstraction adds another public type without owning a distinct responsibility.

### Provider Success Does Not Mean Enough Today

Some live adapters currently do this after provider success:

```rust
response.evidence.verify_guard(&guard)?;
```

or:

```rust
response.verify_request(&request)?;
```

If callers must repeat this after every successful provider call, the provider trait contract is too
weak. Every adapter then becomes a partial transport verifier, which duplicates code and leaves future
contributors unsure where source authority is enforced.

The provider contract should be stronger:

> A successful capability provider response has already enforced request source authority and carries
> matching source evidence by construction.

This applies to live and replay/recorded provider backends.

### BTC State Crosses The Boundary

`mfm-states-btc` currently imports `BtcChainGuard` and `BtcChainHeadRequest`, constructs the request
from `ObserveBtcChainHeadConfig`, and normalizes by verifying the response against that request.

That is the wrong layer. A state may validate semantic config, checkpoint compatibility, and
deterministic observation ordering. It should not construct external provider request objects or
validate provider source evidence.

The BTC adapter should build the request immediately before calling a provider. The provider, live or
recorded, should enforce the request.

### BTC Replay Helpers Are The Same Smell

`mfm-adapters-btc-jsonrpc` currently exposes replay helper functions such as:

```rust
verify_recorded_chain_head_evidence(...)
replay_chain_head_fact_from_evidence(...)
```

Those helpers keep source-evidence validation as a caller activity. The better shape is a recorded
provider backend that implements the same `BtcChainHeadReadProvider` trait as the live provider.

Then the call path is unified:

```text
state config/input
  -> adapter builds capability request
  -> provider.read(request)
  -> state normalizes successful response
```

Live and replay differ only in which provider backend app/replay assembly wires in.

## Goals

- Make the capability provider contract the only source-authority enforcement boundary.
- Remove public guard types as general domain/replay authority.
- Keep semantic authority in existing config/context/intent fields.
- Keep states free of external provider request construction and provider evidence validation.
- Make live adapters trust successful provider responses for source enforcement.
- Represent replay source checks as recorded provider backend behavior, not caller-side helper calls.
- Keep runtime route/source config out of states, ops, replay evidence, public outputs, and persisted
  semantic surfaces.
- Preserve redaction rules for source evidence and diagnostics.

## Non-Goals

- Do not remove semantic fields from configs, contexts, facts, or recorded evidence.
- Do not remove redacted source evidence from provider responses.
- Do not weaken source mismatch handling in live transports.
- Do not silently trust recorded replay evidence.
- Do not add a cross-chain erased guard enum or provider enum.
- Do not move runtime route resolution into states, ops, app replay, public-output rendering, or
  binaries.
- Do not add public `EvmNetworkAuthority` / `BtcChainHeadAuthority` types just to replace public
  guards.
- Do not preserve backwards-compatible guard/request constructors, adapter verification paths, replay
  helper APIs, or fallback compatibility shims.

## Compatibility And Deletion Policy

This is a breaking cleanup. MFM is pre-production, and correctness, fewer concepts, and fewer code
paths take priority over compatibility.

Old guard-based APIs and duplicate validation paths should be deleted, not hidden behind deprecation
wrappers, fallback branches, compatibility constructors, feature flags, or temporary shims. If removed
code is needed later, it can be recovered from git history.

Implementation PRs should prefer removing public surface and tests over preserving old behavior. A
change is incomplete if the old path still exists and can be called by states, adapters, app replay,
or tests.

Expected removals include:

- public `guard` fields on EVM/BTC capability request structs
- public `EvmChainGuard` / `BtcChainGuard` use from states, ops, adapters, app replay, and tests that
  are not provider-backend tests
- `ObserveBtcChainHeadConfig::request`
- `ObserveBtcChainHeadState::request`
- BTC state normalization APIs that accept `BtcChainHeadRequest`
- state-side `response.verify_request(...)`
- adapter-side `response.evidence.verify_guard(...)`
- adapter-side `response.verify_request(...)`
- public BTC replay helper functions that make callers validate recorded source evidence directly
- app replay code that reconstructs EVM guards to validate source diagnostics
- no-IO APIs named `validate_guard`
- tests whose only purpose is proving adapters/states catch mismatched provider source evidence after
  provider success

The replacement should be smaller:

- private request source authority plus request constructors/accessors
- live provider source enforcement
- recorded provider source enforcement
- state semantic validation and normalization only
- adapter request construction and workflow-level checks only

## Core Decision

### One Public Boundary: Capability Requests

External source reads and writes should expose capability request types, not public guard objects.

Current shape:

```rust
pub struct BtcChainHeadRequest {
    pub guard: BtcChainGuard,
    pub selection: BtcHeadSelection,
}
```

Preferred shape:

```rust
pub struct BtcChainHeadRequest {
    source: BtcRequestSource,
    selection: BtcHeadSelection,
}

impl BtcChainHeadRequest {
    pub fn new(
        network_id: BtcNetworkId,
        source_identity: BtcSourceIdentity,
        bitcoin_network: impl Into<String>,
        selection: BtcHeadSelection,
    ) -> Result<Self> {
        // validate source authority and selection shape
    }

    pub fn network_id(&self) -> &BtcNetworkId;
    pub fn source_identity(&self) -> &BtcSourceIdentity;
    pub fn bitcoin_network(&self) -> &str;
    pub const fn selection(&self) -> BtcHeadSelection;
}
```

`BtcRequestSource` can be private. EVM should follow the same pattern:

```rust
pub struct EvmBlockReadRequest {
    source: EvmRequestSource,
    block: EvmBlockSelector,
}
```

The exact private helper names do not matter. The important rule is that callers cannot treat
`*ChainGuard` as a reusable public authority object.

### One Provider Contract

Capability provider traits should document this invariant:

> A successful response has already enforced all request-local provider authority, including source
> identity, chain/network identity, and request-local selectors or anchors. Returned redacted source
> evidence matches the request by construction.

Provider-owned request-local checks include:

- source route/source identity
- observed EVM chain id or Bitcoin network tag
- requested block/hash/height identity when the request asks for an exact anchor
- requested transaction hash for receipt reads
- requested address/anchor for exact balance reads
- requested chain-head selection and finality fields

Adapters and states may still validate workflow-level invariants that are not provider source
authority. Examples:

- prepared transaction evidence matches certified lifecycle context
- a signed payload hash matches the prepared expected transaction hash before submit
- receipt `status` satisfies lifecycle success policy
- retained artifacts match certified context and schema authority
- normalized observation is not behind a loaded checkpoint

Those checks are not provider source-evidence validation.

### Live And Replay Use The Same Provider Trait

There should be no separate caller-side replay validation path for source evidence.

For a provider trait such as:

```rust
pub trait BtcChainHeadReadProvider: Send + Sync {
    fn read_chain_head<'a>(
        &'a self,
        request: &'a BtcChainHeadRequest,
    ) -> BtcCapabilityFuture<'a, BtcChainHeadResponse>;
}
```

both backends implement the same trait:

- `BtcJsonRpcChainHeadProvider`: live backend, resolves runtime route and probes RPC
- `RecordedBtcChainHeadProvider`: replay backend, loads recorded response/evidence and validates it
  against the request

The adapter runner should not know whether it is executing live or replay. App/replay assembly chooses
which backend is available for that execution mode.

The same model applies to EVM provider traits.

## Layer Responsibilities

### Ops

Ops assemble deterministic state graphs from semantic config.

Ops may contain semantic fields like `network_id`, `expected_chain_id`, `semantic_source_identity`,
and `bitcoin_network`.

Ops must not construct external capability requests, route bindings, source guards, live transports,
or recorded provider evidence.

### States

States own reusable domain semantics and deterministic normalization.

States may:

- validate semantic config fields
- validate state inputs against prior state/fact material
- expose semantic accessors or state-specific intent values
- normalize successful provider responses into state outputs
- build fact-index requests when the fact index itself is the state capability

States must not:

- construct external EVM/BTC provider requests
- construct `EvmChainGuard`, `BtcChainGuard`, or replacement public guard objects
- perform route/source binding checks
- verify provider source evidence
- know whether a provider response came from live IO or recorded replay evidence

For BTC, `ObserveBtcChainHeadState` should not expose `request() -> BtcChainHeadRequest`.
It should expose only semantic config/selection behavior needed by the adapter and use successful
provider responses as provider-validated input.

### Adapters

Adapters bind state intent to explicit capabilities.

Adapters may:

- load certified state config and inputs
- build capability requests from semantic config/context immediately before provider calls
- call live or recorded provider backends through the same trait
- map provider errors to runtime/replay errors
- validate workflow-level invariants that are not provider source authority
- record redacted provider evidence returned by successful providers

Adapters must not:

- re-run source evidence validation after successful provider calls
- use generic `verify_guard` / `verify_request` helpers as a substitute for provider contracts
- expose free replay helpers that make callers validate recorded source evidence directly
- resolve runtime routes unless the adapter is itself implementing a provider backend

### Live Providers / Transports

Live providers own live source enforcement.

For each live provider method:

1. Read source authority from the request.
2. Resolve runtime route/source using only the request fields needed for binding.
3. Probe source identity before operation-specific behavior.
4. Compare observed identity with the request authority.
5. Return `SourceMismatch` with closed redacted diagnostics on mismatch.
6. Construct source evidence only after successful verification.
7. Execute operation-specific behavior.
8. Return responses that match request-local provider authority by construction.

EVM already largely follows this model through internal `verified_source(&request.source)` behavior.

Bitcoin should mirror that shape with an internal helper such as `verified_source(&request)` or
`verified_transport(&request)` that performs route lookup, `getblockchaininfo`, Bitcoin network tag
checking, and evidence construction before operation-specific RPC behavior.

### Recorded Providers / Replay Backends

Recorded providers own replay source enforcement.

For each replay provider method:

1. Read source authority from the request.
2. Load the recorded response/evidence supplied by replay assembly.
3. Validate recorded source evidence against the request.
4. Validate request-local selectors or anchors against the recorded response.
5. Return a normal capability response on success.
6. Return a replay/provider mismatch error on failure.

Recorded providers must not:

- open live transports
- consult runtime config
- resolve `source_ref` or `policy_id` against current deployment config
- expose source validation as a caller-side helper

`source_ref` and `policy_id` remain audit provenance only.

### App Assembly

App assembly wires provider backends.

Live start/resume uses live provider backends. Replay uses recorded provider backends. Evidence-only
read paths do not construct live transports, signer providers, keystores, or live runtime config.

App code may validate persisted diagnostic artifact shape, digest, schema, and redaction policy. It
must not reconstruct EVM/BTC guards to validate source authority. That belongs to the provider backend
that produced or replays the capability response.

No-IO route binding preflight is allowed during live admission, but it is not source enforcement and
must not be named `validate_guard`.

Prefer names such as:

- `validate_route_binding`
- `validate_runtime_binding`
- `validate_evm_route_binding`
- `validate_btc_source_binding`

Route binding checks should use only fields needed for route lookup. For example:

- EVM route binding needs `network_id`.
- BTC route binding needs `source_identity`.

Expected chain id and observed network tag checks belong to provider calls, not no-IO preflight.

## BTC Implementation Smell Fix

BTC is the clearest first implementation target.

### Current Wrong Shape

`mfm-states-btc` currently owns external provider request construction:

```rust
impl ObserveBtcChainHeadConfig {
    pub fn request(&self) -> Result<BtcChainHeadRequest, BtcStateError> {
        let guard = BtcChainGuard::new(...)?;
        Ok(BtcChainHeadRequest { guard, selection })
    }
}
```

The state then normalizes by verifying the provider response against that request:

```rust
response.verify_request(request)?;
```

This gives state code provider-boundary responsibility.

### Preferred State Shape

`mfm-states-btc` should expose semantic behavior only:

```rust
impl ObserveBtcChainHeadConfig {
    pub fn selection(&self) -> Result<BtcHeadSelection, BtcStateError> {
        // validate head_kind / confirmation_depth and return selection
    }
}
```

State checkpoint validation should compare against semantic config and selection:

```rust
fn validate_loaded_checkpoint_for_config(
    config: &ObserveBtcChainHeadConfig,
    selection: BtcHeadSelection,
    loaded_checkpoint: &LoadedCollectorCheckpoint,
) -> Result<(), BtcStateError>
```

State normalization should not accept `BtcChainHeadRequest`:

```rust
pub fn normalize_chain_head_response(
    config: &ObserveBtcChainHeadConfig,
    response: &CapabilityChainHeadResponse,
    input: &ObserveBtcChainHeadInput,
) -> Result<BtcChainHeadObservation, BtcStateError>
```

The state may check that normalized fact subjects and checkpoints are compatible with semantic config,
but it should not validate response source evidence. Successful provider responses are already
provider-valid.

### Preferred Adapter Shape

`mfm-adapters-btc-jsonrpc` should build the capability request:

```rust
fn chain_head_request(
    config: &ObserveBtcChainHeadConfig,
) -> Result<BtcChainHeadRequest, BtcJsonRpcAdapterError> {
    let selection = config.selection()?;
    BtcChainHeadRequest::new(
        BtcNetworkId::new(&config.network)?,
        BtcSourceIdentity::new(&config.semantic_source_identity)?,
        &config.bitcoin_network,
        selection,
    )
}
```

The runner path becomes:

```rust
let request = chain_head_request(&config)?;
let response = provider.read_chain_head(&request).await?;
let fact = state.materialize_response(&input, &response)?;
```

There is no adapter-side `response.verify_request(&request)`.

### Preferred BTC Recorded Provider Shape

Replace free replay helpers with a provider backend:

```rust
pub struct RecordedBtcChainHeadProvider {
    // recorded response/evidence supplied by replay assembly
}

impl BtcChainHeadReadProvider for RecordedBtcChainHeadProvider {
    fn read_chain_head<'a>(
        &'a self,
        request: &'a BtcChainHeadRequest,
    ) -> BtcCapabilityFuture<'a, BtcChainHeadResponse> {
        // validate recorded evidence against request and return response
    }
}
```

This keeps replay source validation behind the same provider boundary as live source validation.

## API Direction

### Capability Crates

Capability crates should define:

- provider traits
- request/response types
- checked public identifier types
- redacted source evidence types
- redacted provider errors

Capability crates should not expose guard types as general authority. If internal request-source
helpers remain, keep them private or crate-private.

Public request fields should become private with constructors/accessors. This is intentionally
breaking; correctness and smaller authority surfaces take priority.

Generic public methods named like these should be removed or made private:

- `evidence.verify_guard(...)`
- `response.verify_request(...)`

If provider backends need common comparison logic, use private helpers inside the backend or
request/evidence implementation. Do not expose a generic validation method that adapters and states
can call after provider success.

### Transport Crates

Transport crates implement live providers.

They may expose no-IO route-binding preflight APIs, but those APIs must not be named as guard
validation and should not require full request source authority when route lookup needs only one
field.

### Adapter Crates

Adapter crates build requests and own workflow replay wiring.

When recorded evidence is workflow-specific, the recorded provider backend may live in the adapter
crate. That is preferable to adding a new replay transport crate just to hold a small backend.

The key constraint is that recorded evidence validation is hidden behind the provider trait, not
exposed as a caller-side helper.

## Migration Plan

### Phase 1: Document The Provider Contract

- Update capability trait docs to state that successful responses have passed source-authority and
  request-local validation.
- Update `docs/design.md`, `docs/architecture.md`, EVM/BTC routing docs, and persisted-surface docs
  to remove "request guard" replay language.
- Replace "adapters derive guards" with "adapters build capability requests from certified semantic
  fields."

### Phase 2: Fix BTC State Boundary

- Remove `BtcChainGuard` and `BtcChainHeadRequest` imports from `mfm-states-btc`.
- Remove `ObserveBtcChainHeadConfig::request`.
- Remove `ObserveBtcChainHeadState::request`.
- Add a semantic `selection()` helper or equivalent.
- Change checkpoint compatibility checks to use config plus selection.
- Change normalization to accept config/response/input rather than request/response/input.
- Update BTC state tests so they no longer construct provider requests.

### Phase 3: Move BTC Request Construction Into Adapter

- Add an adapter-local BTC request builder.
- Build `BtcChainHeadRequest` immediately before calling `BtcChainHeadReadProvider`.
- Remove adapter live response `verify_request` calls.
- Keep workflow-level validation and checkpoint semantics in state.

### Phase 4: Add BTC Recorded Provider Backend

- Delete public BTC replay helper functions and add a recorded provider implementation.
- Wire replay paths to call the same adapter/provider path using the recorded provider.
- Ensure recorded provider validation rejects mismatched source evidence, selection, finality, height,
  hash, or other request-local response fields.
- Ensure recorded provider never consults runtime config or opens live transports.

### Phase 5: Centralize BTC Live Enforcement

- Add internal BTC live `verified_source` / `verified_transport` helper.
- Ensure every BTC live provider method calls it before operation-specific behavior.
- Keep unsupported-operation handling after source verification when the operation depends on source
  authority.
- Add transport tests for missing route, source mismatch, successful evidence, redaction, and "no
  operation-specific RPC after source mismatch."

### Phase 6: Remove Live Adapter Re-Verification

- Delete adapter-side `verify_guard` / `verify_request` after successful provider calls in EVM, BTC,
  and portfolio adapters.
- Keep workflow-level invariant checks.
- Delete or rewrite tests expecting adapters to reject mismatched provider source evidence after
  provider success. Provider source mismatch tests belong to provider-backend tests.

### Phase 7: Delete Public Guard APIs

- Replace public request `guard` fields with private request source authority.
- Delete public `EvmChainGuard` / `BtcChainGuard` from state/adapter-facing APIs.
- Delete or privatize generic source-evidence verification helpers so states, adapters, app replay,
  and tests cannot call them.
- Rename no-IO `validate_guard` APIs to route/runtime binding names.

### Phase 8: Clean App Replay Diagnostics

- Stop reconstructing EVM guards in app replay diagnostic validation.
- Validate diagnostic artifact digest, schema, shape, and redaction only.
- Leave source-authority validation to recorded provider backends.
- Delete tests and helpers that assert app replay can validate source authority from reconstructed
  guards.

## Testing Strategy

### Live Provider Tests

Live provider tests should prove:

- route/source binding succeeds for configured sources
- missing route/source returns a redacted diagnostic
- observed chain/network mismatch returns `SourceMismatch`
- successful responses carry source evidence matching the request by construction
- request-local selectors and anchors are enforced by providers
- operation-specific RPC does not proceed after source mismatch
- diagnostics do not leak URLs, auth headers, provider messages, request bodies, response bodies,
  file paths, signer material, or signed raw transactions

### Recorded Provider Tests

Recorded provider tests should prove:

- recorded evidence matching the request returns a normal response
- mismatched source evidence fails closed
- mismatched selection/finality/anchor fields fail closed
- `source_ref` and `policy_id` are treated as recorded audit provenance only
- runtime config is not consulted
- live transports are not constructed

### Adapter Tests

Adapter tests should prove:

- adapters build the expected capability request from certified semantic fields
- adapters map provider errors correctly
- adapters do not re-validate provider source evidence after success
- adapters still enforce workflow-level invariants

### State Tests

State tests should prove:

- state config validation accepts/rejects semantic fields correctly
- state normalization does not require provider request construction
- checkpoint compatibility is checked against semantic state authority and selection, not a request
  guard
- normalized facts remain secret-free and route-free

## Acceptance Criteria

This cleanup is complete when:

- `rg 'BtcChainGuard|EvmChainGuard' crates/states crates/ops` returns no external-source guard usage.
- BTC chain-head state no longer constructs `BtcChainHeadRequest`.
- BTC replay source validation is implemented behind a `BtcChainHeadReadProvider` backend, not public
  helper functions.
- Live adapters no longer call `verify_guard` or `verify_request` after successful provider calls.
- Recorded/replay adapters do not expose caller-side source-evidence validation helpers.
- Live and recorded provider backends enforce source authority before returning successful responses.
- Capability request source fields are private and exposed through constructors/accessors.
- Old guard-based constructors, fields, helper methods, and tests are deleted rather than kept as
  compatibility surfaces.
- No-IO route binding APIs are not named `validate_guard`.
- App replay diagnostic validation does not reconstruct transport guards.
- Docs consistently describe source authority as provider-enforced request authority, not reusable
  workflow guard authority.

## Recommended Initial PR

Start with BTC because it has the clearest boundary violation and the smallest path to a real
simplification.

1. Move BTC chain-head request construction from `mfm-states-btc` to `mfm-adapters-btc-jsonrpc`.
2. Remove BTC state request helpers and request-based response verification.
3. Add semantic config/selection checkpoint validation in state.
4. Add a BTC adapter-local request builder.
5. Add a BTC recorded provider backend that implements `BtcChainHeadReadProvider`.
6. Add BTC live `verified_source` helper and provider contract tests.
7. Update BTC docs and tests.

After that PR, repeat the pattern for EVM/portfolio adapter re-verification and public guard API
privatization.
