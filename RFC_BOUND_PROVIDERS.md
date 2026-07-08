# RFC: Source-Bound Transport Providers

Status: Draft

## Summary

MFM should standardize every live transport around **source-bound providers**.

The current transport boundary still lets adapters and call sites participate in transport source
authority. Recent work removed public chain guards, but the replacement still leaves too much
ceremony outside transports: adapters construct source-bearing requests, helper functions repeat
`network_id` / `expected_chain_id` or Bitcoin source fields, and transport methods rely on each
operation remembering to call validation before raw protocol IO.

That does not scale. If a transport eventually needs ten mandatory checks before every call, those
checks must live behind one structural boundary. They cannot be repeated across adapters, states,
entry points, request factories, or provider methods by convention.

This RFC proposes one standard for all transports:

- runtime config creates a raw transport router/client
- adapters bind that router once to certified semantic source authority
- the returned bound provider implements capability provider traits
- every provider method goes through one private `validate_call` gate
- raw protocol IO requires a private verified-call/session token
- capability requests carry operation parameters only
- no public guard or public transport-authority object is passed around as domain authority

## Problem

### Validation Is Still A Convention

EVM and BTC transports currently perform the right live checks in principle:

- EVM resolves route/policy/source and probes `eth_chainId`
- BTC resolves source identity and probes `getblockchaininfo`
- both return redacted evidence on successful provider calls

The weakness is the shape. A provider method can still be written in a way that forgets the common
validation step before issuing protocol IO. The raw protocol helper has not been made structurally
unreachable without a verified source token.

That means the design relies on discipline:

```text
remember to validate source
remember to build matching evidence
remember to perform operation-specific checks
remember to keep diagnostics redacted
```

Discipline is not a boundary. The next transport operation, next adapter, or next entry point can
drift.

### Adapters Became Transport Request Factories

Adapters should bind certified workflow intent to capability providers. They should not own generic
transport request ceremony.

The current EVM adapter needs local helpers to repeatedly construct source-bearing requests. Portfolio
has similar BTC helpers that combine semantic portfolio fields into BTC provider request authority.
This is a symptom that source binding is in the wrong place.

The adapter should say:

```text
give me the EVM provider bound to this certified network
read this block / nonce / receipt / code
```

It should not say:

```text
for every operation, rebuild the same transport source authority and attach it to the request
```

### Initialization Checks Are Not Live Source Proof

Offline app ingress validation is still useful. It should confirm that process-local runtime config
can resolve a route/source/signer before `RunAdmitted`.

But offline validation cannot prove the live source for a later call. It cannot prove:

- the endpoint still serves the expected chain
- a policy fallback selected the same source family
- the source is synced enough for the operation
- a response body matches the requested block/hash/log/receipt
- future transport-specific invariants were checked

Therefore, app ingress validation must remain binding-only. Live source proof belongs in the bound
provider call path.

### Request-Owned Source Authority Does Not Scale

Putting source authority into every request is better than public guards, but it is still too noisy.
It means every request constructor and every adapter call site knows transport binding fields.

If all EVM requests carry `(network_id, expected_chain_id)`, and all BTC requests carry
`(network_id, source_identity, bitcoin_network)`, source authority is duplicated across many
operation values. That also encourages workflow adapters to grow generic request-authority helpers,
which is transport architecture leaking upward.

## Goals

- Guarantee mandatory live transport checks for every provider call by construction.
- Make source binding a provider property, not a per-operation request property.
- Keep raw protocol IO unreachable without a private verified-call/session token.
- Keep adapters free of generic transport-source request construction.
- Keep states and ops free of transport request/source authority.
- Keep app ingress validation offline and binding-only.
- Keep replay evidence validation separate from live runtime config.
- Keep the solution simple and low LOC: per-transport-family types, no generic cross-chain framework.
- Preserve redaction boundaries for diagnostics and evidence.

## Non-Goals

- Do not reintroduce public `*Guard` types.
- Do not create a universal cross-chain validator abstraction.
- Do not move live source checks into app start/resume ingress.
- Do not make request factories or macros the primary safety boundary.
- Do not let middleware be optional around an otherwise unsafe provider.
- Do not persist runtime source refs, URLs, credentials, or provider bodies as semantic authority.

## Proposed Standard

Every live transport crate should expose two layers:

1. **Router/client**: runtime-config-backed route and source resolution. It does not implement
   capability provider traits.
2. **Bound provider**: a provider bound to semantic source authority. It implements capability
   provider traits.

Only the bound provider can satisfy adapter capability dependencies.

### Transport Router

The raw router owns runtime config and low-level protocol mechanics. It can validate route binding
without network IO, but it is not a live capability provider.

```rust
pub struct EvmJsonRpcClient {
    // route registry, source registry, HTTP client
}

impl EvmJsonRpcClient {
    pub fn validate_route_binding(
        &self,
        authority: &EvmNetworkAuthority,
    ) -> TransportResult<()>;

    pub fn bind_network(
        &self,
        authority: EvmNetworkAuthority,
    ) -> TransportResult<EvmJsonRpcNetworkProvider>;
}
```

BTC mirrors this with source binding:

```rust
pub struct BtcJsonRpcRouter {
    // source identity -> transport
}

impl BtcJsonRpcRouter {
    pub fn validate_source_binding(
        &self,
        authority: &BtcSourceAuthority,
    ) -> BtcResult<()>;

    pub fn bind_source(
        &self,
        authority: BtcSourceAuthority,
    ) -> BtcResult<BtcJsonRpcSourceProvider>;
}
```

### Capability-Owned Bind Authority

The bind authority should live in the capability crate as a typed value. It is not a guard and it
does not prove anything by itself. It is the checked semantic input used to bind a provider.

```rust
pub struct EvmNetworkAuthority {
    network_id: EvmNetworkId,
    expected_chain_id: NonZeroU64,
}

pub struct BtcSourceAuthority {
    network_id: BtcNetworkId,
    source_identity: BtcSourceIdentity,
    bitcoin_network: BitcoinNetworkTag,
}
```

These types may expose accessors for evidence construction and replay checks. They should not expose
runtime route details and should not be accepted as proof that a live call used the right source.

### Operation-Only Requests

Capability requests should carry operation parameters only.

```rust
pub struct EvmBlockReadRequest {
    block: EvmBlockSelector,
}

pub struct EvmReceiptReadRequest {
    transaction_hash: B256,
}

pub struct BtcChainHeadRequest {
    selection: BtcHeadSelection,
}

pub struct BtcBalanceReadRequest {
    address: BtcAddress,
    block_height: u64,
    block_hash: BtcBlockHash,
}
```

The source authority is already attached to the provider.

### Bound Provider

A bound provider has private source authority and implements capability traits.

```rust
pub struct EvmJsonRpcNetworkProvider {
    client: EvmJsonRpcClient,
    authority: EvmNetworkAuthority,
}

impl EvmBlockReadProvider for EvmJsonRpcNetworkProvider {
    fn read_block<'a>(
        &'a self,
        request: &'a EvmBlockReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmBlockReadResponse> {
        Box::pin(async move {
            let verified = self.validate_call(EvmOperation::ReadBlock).await?;
            self.read_block_checked(verified, request).await
        })
    }
}
```

The same provider can implement many capability traits, but each implementation must enter through
the same validation gate.

### Private Verified-Call Token

The transport crate should make raw protocol IO require a private verified token.

```rust
struct VerifiedEvmCall<'a> {
    source: &'a EvmRuntimeSource,
    evidence: RedactedEvmSourceEvidence,
}

impl EvmJsonRpcNetworkProvider {
    async fn validate_call(&self, operation: EvmOperation) -> TransportResult<VerifiedEvmCall<'_>> {
        // route lookup
        // source policy selection
        // chain/source probe
        // sync/finality/rate policy checks
        // redacted evidence construction
        // all future mandatory per-call checks
    }

    async fn rpc_call(
        &self,
        verified: &VerifiedEvmCall<'_>,
        method: &'static str,
        params: serde_json::Value,
    ) -> TransportResult<serde_json::Value> {
        // raw JSON-RPC IO
    }
}
```

No provider method should be able to call raw protocol IO with just a route/source. It must have a
`Verified*Call` value, and only `validate_call` can mint one.

BTC follows the same pattern:

```rust
struct VerifiedBtcCall {
    transport: Arc<dyn BtcJsonRpcTransport>,
    info: BlockchainInfo,
    evidence: RedactedBtcSourceEvidence,
}
```

### Local Macro Is Allowed, But Not The Boundary

A transport crate may use a local macro to reduce provider implementation boilerplate:

```rust
impl_checked_provider!(
    EvmBlockReadProvider,
    read_block,
    EvmBlockReadRequest,
    EvmBlockReadResponse,
    EvmOperation::ReadBlock,
    read_block_checked
);
```

The macro is only ergonomics. The boundary is still the private verified token and raw IO requiring
that token.

## Adapter Shape

Adapters should receive bound providers from app/runtime factories.

For EVM contract lifecycle, the factory should bind with expected chain id:

```rust
fn read_runtime_for(
    &self,
    network_id: &str,
    expected_chain_id: u64,
) -> mfm_runtime::Result<EvmContractReadRuntime>;

fn runtime_for(
    &self,
    network_id: &str,
    expected_chain_id: u64,
) -> mfm_runtime::Result<EvmContractRuntime>;
```

The returned runtime contains provider trait objects backed by `EvmJsonRpcNetworkProvider`.

For portfolio, replace the split of raw providers plus `PortfolioRuntimeValidator` with a factory
that returns bound providers:

```rust
trait PortfolioTransportFactory: Send + Sync {
    fn bind_evm(
        &self,
        network_id: &str,
        expected_chain_id: u64,
    ) -> mfm_runtime::Result<Arc<dyn PortfolioEvmProvider>>;

    fn bind_btc(
        &self,
        network_id: &str,
        source_identity: &str,
        bitcoin_network: &str,
    ) -> mfm_runtime::Result<Arc<dyn PortfolioBtcProvider>>;
}
```

This keeps workflow-specific semantic extraction in adapters while moving generic source binding into
transport-owned provider construction.

## Replay And Recorded Providers

Replay must not use live runtime config. The same standard still applies:

- replay assembly builds recorded providers from certified/replayed semantic authority
- recorded providers implement the same capability traits as live bound providers
- recorded providers validate recorded evidence against their bound authority and operation request
- replay does not resolve source refs or policy ids against current runtime config

Recorded providers may use the same authority types as live bound providers. They should not expose
public guard-style verification helpers.

## Why This Is Better

### Adding Checks Becomes Local

With bound providers, adding a mandatory validation is one edit:

```rust
validate_call(...)
```

It does not require touching every adapter call site, every operation request constructor, and every
provider method.

### The Compiler Helps

Raw protocol helpers take a private verified token. New transport code cannot accidentally call
protocol IO without first passing through validation, because it cannot produce the token any other
way.

### Adapters Stop Knowing Transport Ceremony

Adapters bind once and then issue operation requests. They still own workflow semantics, but they no
longer rebuild transport source authority for every operation.

### Provider Traits Become Honest

A successful provider response means:

```text
source authority was enforced
response evidence matches the bound provider authority
operation-specific response checks passed
diagnostics are redacted
```

Consumers should not re-verify this after every successful call.

## Why Not Other Options

### Request Factories

Factories reduce call-site typing but do not guarantee live validation. They still leave adapters
constructing transport-source-bearing operation requests.

### Macros

Macros reduce provider implementation boilerplate, but they do not define authority. A macro is
acceptable only after raw IO already requires a private verified token.

### Middleware

Middleware can be bypassed by calling the inner provider or raw client. It also cannot prove that the
inner provider used the same source after middleware validation.

### Public Guard Objects

Public guards become general authority and leak transport concerns into states, adapters, replay, and
tests. This RFC explicitly avoids that shape.

## Migration Plan

1. Add capability-owned authority values:
   - `EvmNetworkAuthority`
   - `BtcSourceAuthority`
2. Add bound providers in live transport crates:
   - `EvmJsonRpcNetworkProvider`
   - `BtcJsonRpcSourceProvider`
3. Move existing live source checks into private `validate_call` methods.
4. Make raw protocol helpers require private `Verified*Call` values.
5. Move capability trait impls from raw routers/clients to bound providers.
6. Make operation requests source-free.
7. Update adapter runtime factories to return bound providers.
8. Remove adapter-local generic source request helpers.
9. Update recorded/replay providers to bind from certified authority and validate recorded evidence
   internally.
10. Update docs:
    - `docs/architecture.md`
    - `docs/design.md`
    - `docs/evm-rpc-routing.md`
    - `docs/btc-rpc-routing.md`
    - `docs/persisted-public-surfaces.md`
11. Add tests proving:
    - raw routers do not implement capability provider traits
    - raw protocol IO helpers cannot be called without private verified tokens
    - bound providers reject source mismatches before operation-specific RPC
    - adapters no longer construct source-bearing operation requests

## Acceptance Criteria

- No live raw transport router implements capability provider traits.
- No operation request contains source authority fields.
- No adapter-local generic transport request authority helper exists.
- Every raw protocol IO helper requires a private verified token.
- Every capability provider implementation in a live transport enters through `validate_call`.
- App ingress validation remains no-IO route/source/signer binding validation.
- Replay providers validate recorded evidence internally and do not consult runtime config.
- Exact scans for public guard-style APIs remain empty.

## Open Questions

- Should bound providers cache source probes for a short operation-local window, or should every
  provider call probe source identity independently? The default should be per-call validation unless
  a transport-specific RFC defines safe cache semantics.
- Should `EvmOperation` / `BtcOperation` be persisted in diagnostics as a closed enum, or should it
  remain a transport-local diagnostic field?
- Should authority values live in capability crates immediately, or should the first migration keep
  them private in transport crates until request structs become operation-only? The end state should
  be capability-owned typed authority values.

