# RFC: Transport Guard Boundary

Status: Draft

## Summary

Transport guards should be owned by transport-facing capability requests and enforced only by
transport/provider implementations when live external IO is about to happen.

The current codebase has drifted from that boundary. Guard construction and guard verification appear
in states, adapters, app/replay validation, and tests. That makes `EvmChainGuard` and `BtcChainGuard`
look like general domain authority types instead of live transport boundary contracts. It also creates
duplicated checks: a transport/provider verifies the guard, then adapter code often verifies the
returned evidence against the same guard again.

This RFC proposes a broad cleanup:

- Keep semantic workflow authority as plain config/context fields.
- Construct `*ChainGuard` only at adapter-to-transport call sites.
- Enforce `*ChainGuard` only inside live transport/provider implementations.
- Remove guard construction and transport request construction from state crates.
- Remove adapter-side live response guard re-verification.
- Keep replay evidence validation, but make it explicit and replay-only instead of using generic live
  guard verification helpers throughout the codebase.

## Problem

### Guard Semantics Are Currently Spread Across Layers

The design intent is that external provider identity is checked at the live transport boundary. In
practice, guard-related code appears in several layers:

- Capability crates define `EvmChainGuard`, `BtcChainGuard`, guarded requests, evidence, and
  `verify_guard` helpers.
- Transport crates resolve runtime routes and probe live provider identity.
- Adapter crates build guarded requests and also re-verify returned evidence.
- `mfm-states-btc` constructs `BtcChainGuard` and `BtcChainHeadRequest`.
- App/replay code constructs or validates EVM guards for diagnostic evidence.

This makes the boundary unclear. A reader cannot easily tell which code is semantic planning, which
code is request mapping, which code is live provider enforcement, and which code is replay evidence
validation.

### The Name "Guard" Is Doing Too Much

There are two concepts that have become conflated:

1. Workflow semantic authority.
   Examples: `network_id`, `expected_chain_id`, `source_identity`, `bitcoin_network`.

2. Transport guard.
   A checked request object passed to a live provider so the provider can reject a route/source
   mismatch before returning data or submitting side effects.

The first concept belongs in operation config, certified context, state config, facts, and replay
authority.

The second concept belongs at the transport boundary only.

### Adapter-Side Guard Verification Duplicates Provider Responsibility

Live adapters often do this after a successful provider call:

```rust
response.evidence.verify_guard(&guard)?;
```

or:

```rust
response.verify_request(&request)?;
```

When the provider contract already says that a guarded request is enforced before a successful
response is returned, these checks duplicate the provider's job. They also train downstream code to
treat provider evidence as untrusted even after crossing a trusted provider trait boundary.

Adapters should still validate domain-specific response facts that are not transport guard
enforcement. For example:

- transaction hash returned by `eth_sendRawTransaction`
- receipt transaction hash and receipt status
- block hash/height matching an exact request
- address and anchor consistency for exact balance reads

Those checks are not the same as "does this evidence match the transport guard?"

### BTC State Currently Crosses The Boundary

`mfm-states-btc` currently constructs a `BtcChainGuard` and a `BtcChainHeadRequest` from
`ObserveBtcChainHeadConfig`.

That is the wrong layer. State logic may validate pure semantic configuration and normalize already
obtained observations. It should not construct transport-boundary request objects.

The adapter runner should map state config plus input into a transport capability request immediately
before calling the provider.

### Replay Needs Evidence Validation, But Not Live Guard Enforcement

Replay has no live transport. It must verify stored evidence against certified workflow authority.
That validation is real and must remain.

However, replay validation should not look like live guard enforcement. A replay verifier should work
from certified semantic authority and recorded evidence, not from current runtime config and not from a
transport guard abstraction that appears to be a live request concern.

## Goals

- Make live transport/provider implementations the only live guard enforcement point.
- Keep state crates free of transport guard and transport request construction.
- Make adapter live flows trust successful provider responses with respect to guard enforcement.
- Preserve explicit replay evidence validation.
- Reduce repeated guard/evidence verification calls in adapters.
- Keep route/source runtime config out of states, ops, and replay.
- Preserve redaction rules for provider diagnostics and evidence.

## Non-Goals

- Do not remove semantic fields from configs, contexts, facts, or evidence.
- Do not remove source evidence from provider responses.
- Do not weaken source mismatch handling in transports.
- Do not add a cross-chain erased guard enum.
- Do not move runtime route resolution into adapters, states, ops, app, or replay.
- Do not silently trust recorded replay evidence without checking it against certified authority.

## Proposed Model

### Definitions

**Semantic authority**

Workflow-owned non-secret fields that describe the intended external source.

EVM examples:

- `network_id`
- `expected_chain_id`

Bitcoin examples:

- `network_id`
- `source_identity`
- `bitcoin_network`

Semantic authority may appear in certified config, context, state config, facts, public diagnostics,
and replay verifiers.

**Transport guard**

A checked request guard passed to a live transport/provider implementation.

EVM:

- `EvmChainGuard`

Bitcoin:

- `BtcChainGuard`

Transport guards should be created only as part of constructing a transport-facing capability request.
They should not be used as generic domain authority structs.

**Source evidence**

Redacted provider evidence returned by a transport/provider after live guard enforcement.

EVM examples:

- semantic `network_id`
- expected EVM chain id
- observed EVM chain id
- selected `source_ref`
- selected `policy_id`

Bitcoin examples:

- semantic `network_id`
- semantic `source_identity`
- expected Bitcoin network tag
- observed Bitcoin network tag
- source status

Source refs and policy ids are audit provenance only. They are not replay authority and must not be
resolved against current runtime config during replay.

### Layer Responsibilities

#### Ops

Ops assemble deterministic state graphs from semantic config.

Ops may contain semantic fields like `network_id`, `expected_chain_id`, `source_identity`, and
`bitcoin_network`.

Ops must not construct `EvmChainGuard`, `BtcChainGuard`, or transport request objects.

#### States

States own reusable domain semantics and deterministic normalization.

States may:

- validate semantic config fields
- validate state inputs against prior state/fact material
- normalize provider responses into state outputs
- build fact-index requests when fact-index read is itself the state capability

States must not:

- construct transport guards
- construct live EVM/BTC transport requests
- perform route/source binding checks
- verify live provider source evidence against transport guards

For BTC, this means `ObserveBtcChainHeadState` should no longer expose `request() ->
BtcChainHeadRequest`. It should expose semantic accessors or a pure semantic authority value that the
adapter can use to build a transport request.

#### Adapters

Adapters bind state intent to explicit capabilities.

Adapters may:

- load certified state config and input
- build transport capability requests from semantic authority immediately before provider calls
- call live providers
- map provider errors to runtime errors
- validate domain response invariants that are not guard enforcement
- record redacted provider evidence

Adapters should not:

- re-run `response.evidence.verify_guard(&guard)` after a successful live provider call
- use guard verification as a substitute for provider contract enforcement
- resolve runtime routes directly unless the adapter itself is the transport implementation

#### Transports / Providers

Transports/providers own live guard enforcement.

For every guarded live provider method:

1. Resolve the route/source from runtime config using the guard fields.
2. Probe source identity before using the source for the requested operation.
3. Compare observed identity with the guard.
4. Return `SourceMismatch` with a closed redacted diagnostic if the source does not match.
5. Construct source evidence only after the source has been verified.
6. Return successful responses whose source evidence matches the guard by construction.

EVM already largely follows this model with an internal `verified_source(&guard)` helper.

Bitcoin should follow the same shape with an internal helper such as `verified_transport(&guard)` or
`verified_source(&guard)` that performs route lookup, `getblockchaininfo`, network tag checking, and
evidence construction before operation-specific RPC behavior.

#### App Assembly

App assembly may validate that runtime config can bind semantic routes before run admission.

That validation should not be named `validate_guard` if it does not perform live guard enforcement.
Prefer names like:

- `validate_route_binding`
- `validate_runtime_binding`
- `validate_evm_route_binding`
- `validate_btc_source_binding`

Admission-time route binding checks should use only the fields they actually need. For example:

- EVM route binding needs `network_id`.
- BTC route binding needs `source_identity`.

Expected chain id and observed network tag checks belong to live provider calls, not no-IO route
binding validation.

#### Replay

Replay must not use live transports or runtime config.

Replay should verify recorded evidence against certified semantic authority using replay-specific
functions, for example:

```rust
verify_recorded_evm_source_evidence(authority, evidence)
verify_recorded_btc_source_evidence(authority, evidence)
verify_recorded_btc_chain_head_response(authority, selection, response)
```

Replay verifiers should not be named as generic live guard methods. They should make it obvious that
they operate on recorded evidence and certified authority.

## Proposed API Direction

### Keep Guarded Capability Requests

`EvmChainGuard` and `BtcChainGuard` can remain in transport-facing capability request structs because
the request is the live provider boundary contract.

Examples:

```rust
pub struct EvmBlockReadRequest {
    pub guard: EvmChainGuard,
    pub block: EvmBlockSelector,
}

pub struct BtcChainHeadRequest {
    pub guard: BtcChainGuard,
    pub selection: BtcHeadSelection,
}
```

The important rule is not "delete guards from capability requests." The rule is "do not let transport
guards become general workflow/state/replay authority types."

### Add Or Reuse Semantic Authority Types

To avoid passing loose strings everywhere while also avoiding transport guard leakage, introduce or
reuse pure semantic authority structs at the adapter/state boundary.

Possible BTC example:

```rust
pub struct BtcChainHeadAuthority {
    pub network: String,
    pub semantic_source_identity: String,
    pub bitcoin_network: String,
    pub selection: BtcHeadSelection,
}
```

This type would not be a transport guard. It would be a pure state/domain description. The adapter
would convert it into `BtcChainGuard` only when building `BtcChainHeadRequest`.

Possible EVM example:

```rust
pub struct EvmNetworkAuthority {
    pub network_id: String,
    pub expected_chain_id: u64,
}
```

If existing model/context structs already serve this role, avoid adding a new type.

### Move BTC Request Construction Out Of State

Current wrong shape:

```rust
impl ObserveBtcChainHeadConfig {
    pub fn request(&self) -> Result<BtcChainHeadRequest, BtcStateError> {
        ...
        let guard = BtcChainGuard::new(...)?;
        Ok(BtcChainHeadRequest { guard, selection })
    }
}
```

Preferred shape:

```rust
impl ObserveBtcChainHeadConfig {
    pub fn semantic_authority(&self) -> Result<BtcChainHeadAuthority, BtcStateError> {
        ...
    }
}
```

Then in `mfm-adapters-btc-jsonrpc`:

```rust
fn btc_chain_head_request(
    authority: &BtcChainHeadAuthority,
) -> Result<BtcChainHeadRequest, BtcJsonRpcAdapterError> {
    let guard = BtcChainGuard::new(
        BtcNetworkId::new(&authority.network)?,
        BtcSourceIdentity::new(&authority.semantic_source_identity)?,
        &authority.bitcoin_network,
    )?;
    Ok(BtcChainHeadRequest {
        guard,
        selection: authority.selection,
    })
}
```

The exact type and function names can be improved during implementation.

### Remove Live Adapter Guard Re-Verification

Current live adapter pattern:

```rust
let response = provider.read_block(&request).await?;
response.evidence.verify_guard(&guard)?;
```

Preferred live adapter pattern:

```rust
let response = provider.read_block(&request).await?;
```

The provider contract guarantees guard enforcement. If a provider implementation violates that
contract, the provider implementation is broken and should be fixed or tested directly.

Adapter code should still validate non-guard invariants:

```rust
if response.transaction_hash != expected_hash {
    return Err(...);
}
```

### Replace Generic Verify Methods With Replay-Specific Validators

Current generic helpers like:

```rust
evidence.verify_guard(&guard)
response.verify_request(&request)
```

invite use in live adapter code.

Preferred direction:

- Keep private/internal helpers in capability crates only if transports need them for construction.
- Move public validation intended for replay into replay/adapter replay modules.
- Use names that include `recorded`, `replay`, or `evidence`.

Examples:

```rust
verify_recorded_btc_chain_head_response(&authority, selection, response)
verify_recorded_evm_source_evidence(&authority, evidence)
```

These functions should accept semantic authority, not current runtime config and not a route binding.

## Migration Plan

### Phase 1: Clarify Naming And Documentation

- Document this boundary in `docs/architecture.md`, `docs/design.md`, `docs/evm-rpc-routing.md`, and
  `docs/btc-rpc-routing.md`.
- Replace language like "states/adapters derive guards" with "adapters build guarded transport
  requests immediately before provider calls."
- Clarify that route binding validation is not live guard enforcement.

### Phase 2: Fix BTC State Boundary

- Remove `BtcChainGuard` construction from `mfm-states-btc`.
- Remove `BtcChainHeadRequest` construction from `ObserveBtcChainHeadState`.
- Replace state request helpers with pure semantic authority/selection helpers.
- Move `BtcChainHeadRequest` construction into `mfm-adapters-btc-jsonrpc`.
- Keep state normalization pure and based on response plus semantic state input/config.

### Phase 3: Centralize BTC Transport Enforcement

- Add an internal BTC transport helper that mirrors EVM `verified_source`.
- Ensure every BTC provider method calls this helper before operation-specific behavior.
- Ensure successful BTC provider responses carry evidence that matches the guard by construction.
- Keep unsupported-operation handling after source verification when the operation depends on source
  authority.

### Phase 4: Remove Live Adapter Re-Verification

- Remove adapter-side calls to generic guard verification after successful provider calls.
- Keep non-guard invariant checks.
- Update tests that expected live adapters to catch mismatched provider evidence; those tests should
  move to provider/transport contract tests or replay evidence tests.

### Phase 5: Separate Replay Evidence Validators

- Rename or relocate public guard/evidence verification helpers so they are replay-specific.
- Ensure replay validators accept certified semantic authority and recorded evidence.
- Ensure replay never resolves `source_ref` or `policy_id` through current runtime config.
- Keep tests proving replay rejects mismatched recorded evidence.

### Phase 6: Rename Route Binding Preflight APIs

- Replace `validate_guard` APIs that perform no live IO with route-binding names.
- Use semantic route fields rather than full transport guards where possible.
- Keep live source identity checks inside provider methods.

## Testing Strategy

### Transport Tests

Transport tests should prove:

- route/source binding succeeds for configured sources
- missing route/source returns a redacted provider diagnostic
- observed chain/network mismatch returns `SourceMismatch`
- successful responses carry evidence matching the request guard by construction
- diagnostics do not leak URLs, auth headers, provider messages, request bodies, response bodies, or
  file paths

### Adapter Tests

Adapter tests should prove:

- adapters build the correct guarded transport request from certified semantic authority
- adapters map provider errors correctly
- adapters record provider evidence without re-validating guard enforcement
- adapters still reject non-guard domain mismatches such as transaction hash mismatch or exact anchor
  mismatch

### State Tests

State tests should prove:

- state config validation accepts/rejects semantic fields correctly
- state normalization does not require constructing transport guards
- checkpoint compatibility is checked against semantic state authority and fact subjects, not
  transport request guards

### Replay Tests

Replay tests should prove:

- recorded source evidence is checked against certified semantic authority
- mismatched observed chain/network evidence fails closed
- source refs and policy ids are treated as recorded audit provenance only
- replay does not open live transports or consult runtime config

## Acceptance Criteria

This boundary cleanup is complete when:

- `rg 'BtcChainGuard|EvmChainGuard' crates/states crates/ops` returns no state/op guard construction
  or live transport request construction.
- Live adapter code no longer calls generic guard verification after successful provider calls.
- Transport/provider implementations enforce guards before returning successful responses.
- Replay evidence checks are explicit, replay-named, and based on certified semantic authority.
- No-IO route binding APIs are not named `validate_guard`.
- Docs consistently describe transport guards as boundary-only live request contracts.

## Open Questions

1. Should `EvmChainGuard` and `BtcChainGuard` remain public types in capability crates, or should their
   constructors be narrowed once adapters are the only expected constructors?

2. Should replay evidence validators live in adapter crates, capability crates, or a dedicated replay
   module per provider family?

3. Should pure semantic authority structs be introduced for EVM/BTC, or should existing config/context
   structs be used directly to avoid another layer of types?

4. Should provider trait docs explicitly state that successful responses have already passed guard
   enforcement?

5. Should tests include a shared contract suite for provider implementations so adapters do not need
   defensive re-verification?

## Recommended Initial PR

Start with BTC because it has the clearest boundary violation.

1. Move `BtcChainGuard` and `BtcChainHeadRequest` construction out of `mfm-states-btc`.
2. Add a pure semantic authority/selection helper in `mfm-states-btc`.
3. Build `BtcChainHeadRequest` inside `mfm-adapters-btc-jsonrpc` immediately before calling the
   provider.
4. Add or clarify BTC provider tests that source mismatch is caught by the provider/transport.
5. Remove live adapter-side `verify_request` calls only where provider enforcement already covers the
   guard.
6. Keep replay evidence checks and rename them as replay-specific follow-up work if that makes the
   first PR too large.

This keeps the first implementation small while moving the architecture in the right direction.
