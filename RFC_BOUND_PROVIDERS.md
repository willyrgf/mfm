# RFC: Source-Bound Transport Providers

Status: Draft

## Summary

MFM should standardize every live transport around **source-bound providers** backed by
provider-owned, non-bypassable sealed pipelines.

The current transport boundary still lets adapters and call sites participate in transport source
binding. Recent work removed public chain guards, but the replacement still leaves too much ceremony
outside transports: adapters construct source-bearing requests, helper functions repeat
`network_id` / `expected_chain_id` or Bitcoin source fields, and transport methods rely on each
operation remembering to call validation before raw protocol IO.

That does not scale. If a transport eventually needs ten mandatory checks before provider
initialization or every call, those checks must live behind one structural boundary. They cannot be
repeated across adapters, states, entry points, request factories, optional middleware wrappers, or
provider methods by convention.

This RFC proposes one standard for all transports:

- runtime config creates a raw transport router/client
- adapters derive a checked semantic source binding from certified config, context, or intent
- adapters bind the router once per certified source binding
- the returned bound provider implements capability provider traits
- provider initialization and every provider call go through a private provider-owned sealed pipeline
- the call-prepare phase mints a private verified-call/session token
- raw operation IO requires that private verified-call/session token
- call-finish checks validate response identity, operation invariants, and redaction before success
- capability requests carry operation parameters only
- no public guard, proof-like source object, or optional middleware wrapper is passed
  around as domain authority

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

### Many Checks Need Composition, Not Optional Wrappers

The first source-bound provider sketch used one `validate_call` gate. That is the right boundary,
but it is not necessarily the best implementation shape once a transport has many checks.

A mature transport may need checks across several phases:

- config and route graph validation
- no-IO binding validation before run admission
- provider construction and local initialization checks
- optional live warmup or health checks
- per-call source identity probes
- sync, height, finality, rate, feature, or method support checks
- source fallback and retry policy checks
- response identity checks against block/hash/receipt/log/address request fields
- redaction and diagnostic classification checks

Putting those checks in adapters does not scale. Putting them in a public wrapper middleware around
an otherwise unsafe provider also does not scale, because the wrapper can be skipped or the inner
provider can be used directly. If the wrapper cannot be skipped, then it is not really a separate
architecture boundary; it is part of the provider implementation.

The scalable shape is a provider-owned sealed pipeline behind the bound provider. The pipeline may
be implemented with private stages, functions, or traits, but it is not an optional public layer that
adapters assemble.

### Adapters Became Transport Request Factories

Adapters should bind certified workflow intent to capability providers. They should not own generic
transport request ceremony.

The current EVM adapter needs local helpers to repeatedly construct source-bearing requests.
Portfolio has similar BTC helpers that combine semantic portfolio fields into BTC provider request
binding. This is a symptom that source binding is in the wrong place.

The adapter should say:

```text
give me the EVM provider bound to this certified network
read this block / nonce / receipt / code
```

It should not say:

```text
for every operation, rebuild the same transport source binding and attach it to the request
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

Live initialization or warmup checks are also not enough by themselves. They can detect a bad
deployment earlier, warm pools, or populate local transport state, but they are stale by the first
real provider call. Therefore, app ingress validation must remain binding-only, and live source
proof belongs in the bound provider call path.

### Request-Owned Source Binding Does Not Scale

Putting source binding into every request is better than public guards, but it is still too noisy.
It means every request constructor and every adapter call site knows transport binding fields.

If all EVM requests carry `(network_id, expected_chain_id)`, and all BTC requests carry
`(network_id, source_identity, bitcoin_network)`, source binding is duplicated across many operation
values. That also encourages workflow adapters to grow generic request-binding helpers, which is
transport architecture leaking upward.

## Goals

- Guarantee mandatory live transport checks for every provider call by construction.
- Make source binding a provider property, not a per-operation request property.
- Provide a provider-owned sealed pipeline for many init, call-prepare, and call-finish
  validations.
- Keep required checks non-bypassable by adapters, states, app assembly, and tests.
- Keep capability-provider operation IO unreachable without a private verified-call/session token.
- Keep adapters free of generic transport-source request construction.
- Keep states and ops free of transport request/source binding.
- Keep app ingress validation offline and binding-only.
- Keep live initialization and warmup checks separate from per-call source proof.
- Keep replay evidence validation separate from live runtime config.
- Keep the solution simple and low LOC: per-transport-family types, no generic cross-chain framework.
- Preserve redaction boundaries for diagnostics and evidence.

## Non-Goals

- Do not reintroduce public `*Guard` types.
- Do not create a universal cross-chain validator, middleware, hook, interceptor, or policy
  framework.
- Do not move live source checks into app start/resume ingress.
- Do not treat live initialization or warmup checks as source proof for later calls.
- Do not make request factories or macros the primary safety boundary.
- Do not expose middleware as a public adapter/app extension point, wrapper stack, provider
  decorator, or optional runtime layer.
- Do not expose public `Preflight`, `PostInit`, `ValidationContext`, `CallContext`, `Middleware`,
  `Layer`, `inner`, `unchecked`, or `skip_validation` APIs for live transport checks.
- Do not let adapters or app assembly choose which mandatory checks run.
- Do not persist runtime source refs, URLs, credentials, provider bodies, or middleware state as
  semantic authority.

## Proposed Standard

Every live transport crate should expose two public layers:

1. **Router/client**: runtime-config-backed route and source resolution. It does not implement
   capability provider traits.
2. **Bound provider**: a provider bound to checked semantic source binding. It implements capability
   provider traits.

The bound provider may also have private sealed-pipeline types behind those layers. That pipeline is
an implementation technique, not a third public authority boundary.

Only the bound provider can satisfy adapter capability dependencies.

### Transport Router

The raw router owns runtime config and low-level protocol mechanics. It can validate route binding
without network IO, but it is not a live capability provider.

```rust
pub struct EvmJsonRpcClient {
    // route registry, source registry, HTTP client
}

impl EvmJsonRpcClient {
    pub fn validate_network_binding(
        &self,
        binding: &EvmNetworkBinding,
    ) -> TransportResult<()>;

    pub fn bind_network(
        &self,
        binding: EvmNetworkBinding,
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
        binding: &BtcSourceBinding,
    ) -> BtcResult<()>;

    pub fn bind_source(
        &self,
        binding: BtcSourceBinding,
    ) -> BtcResult<BtcJsonRpcSourceProvider>;
}
```

`validate_*_binding` is no-IO and suitable for app ingress. `bind_*` attaches checked semantic
binding to a provider. Neither operation proves that the live source will be correct later.

### Capability-Owned Binding Input

The binding input should live in the capability crate as a typed value. It is not a guard and it
does not prove anything by itself. It is the checked semantic input used to bind a provider and to
verify recorded evidence.

Use `Binding`, not `Authority`, for these public values. In MFM, authority is reserved for stronger
non-forgeable runtime/certification/store proof objects. These values are constructible semantic
inputs.

```rust
pub struct EvmNetworkBinding {
    network_id: EvmNetworkId,
    expected_chain_id: NonZeroU64,
}

pub struct BtcSourceBinding {
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

The source binding is already attached to the provider.

### Bound Provider

A bound provider has private source binding and implements capability traits.

```rust
pub struct EvmJsonRpcNetworkProvider {
    client: EvmJsonRpcClient,
    binding: EvmNetworkBinding,
}

impl EvmBlockReadProvider for EvmJsonRpcNetworkProvider {
    fn read_block<'a>(
        &'a self,
        request: &'a EvmBlockReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmBlockReadResponse> {
        Box::pin(async move {
            let verified = self.validate_call(EvmOperation::ReadBlock).await?;
            let raw = self.read_block_checked(&verified, request).await?;
            self.checks().after_block_read(&verified, request, raw).await
        })
    }
}
```

The same provider can implement many capability traits, but each implementation must enter through
the same required check path.

### Provider-Owned Sealed Pipeline

A transport crate may organize bound-provider construction and calls with private provider-owned
sealed pipelines. This is the only acceptable middleware shape.

A pipeline is not a third public layer. It is not a trait object registry, adapter-supplied stack,
decorator, `with_middleware` hook, or optional runtime extension point. It is private implementation
inside the live bound provider. The raw router/client and the pipeline do not implement capability
provider traits.

A sealed pipeline may use named phases:

```text
router construction
  -> static config/route/policy checks
bind provider
  -> no-IO semantic binding checks
provider initialization
  -> local init checks and optional live warmup checks
call prepare
  -> live source probe and mandatory operation checks
operation IO
  -> raw protocol calls requiring Verified*Call
call finish
  -> response identity, operation invariants, and diagnostic/redaction checks
```

The implementation can be a list of private stages, private traits, enums, modules, or direct
functions. The architecture constraint is not the internal shape; it is that the required pipeline
is owned by the bound provider and cannot be skipped.

The binding pipeline may run only no-IO checks: binding shape, route/source/signer reference
resolution, policy membership, and redacted launch/binding audit construction. It must not claim
live source proof.

The live call pipeline owns the mandatory provider-call sequence: resolve route/source candidates
from the provider's private binding, run the closed source probe needed to mint a private
`Verified*Call` or method-local `Verified*Session`, apply mandatory transport checks, run operation
IO only through token-gated helpers, validate operation-specific response invariants, attach
redacted evidence, and convert failures to closed provider diagnostics.

Allowed:

- private transport-family check stages such as `EvmPreCallCheck` or `BtcPostCallCheck`
- local composition to keep `validate_call` small and testable
- optional non-semantic observability or metrics hooks that cannot affect source proof
- transport-specific retry/fallback stages when they preserve fail-closed source mismatch semantics

Forbidden:

- a public middleware wrapper that implements capability provider traits around an unsafe inner
  provider
- public `Middleware`, `Layer`, provider decorator, `with_middleware`, `inner`, `unchecked`, or
  `skip_validation` APIs for production transports
- adapter-selected mandatory checks
- public middleware that can mint `Verified*Call`
- app ingress live checks being treated as provider-call proof
- shared cross-chain middleware traits that erase EVM/BTC source semantics
- middleware state or provider bodies persisted as replay authority

The default should be: config and binding checks are no-IO; live initialization checks may happen
lazily before the first provider call; live source identity checks happen per provider call unless a
transport-specific RFC defines safe cache semantics.

### Concrete Provider Execution Shape

The provider-owned pipeline should be visible in transport code as one private execution shape, not
as many hand-assembled call paths.

```rust
impl EvmJsonRpcNetworkProvider {
    async fn execute_checked<T>(
        &self,
        operation: EvmOperation,
        run: impl AsyncFnOnce(&VerifiedEvmCall<'_>) -> TransportResult<T>,
    ) -> TransportResult<T> {
        self.pipeline.ensure_initialized().await?;
        let verified = self.pipeline.prepare_call(self, operation).await?;
        let output = run(&verified).await?;
        self.pipeline.finish_call(&verified, operation, output).await
    }
}
```

Each capability method then supplies only the operation-specific IO and response checks:

```rust
impl EvmBlockReadProvider for EvmJsonRpcNetworkProvider {
    fn read_block<'a>(
        &'a self,
        request: &'a EvmBlockReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmBlockReadResponse> {
        Box::pin(async move {
            self.execute_checked(EvmOperation::ReadBlock, async |verified| {
                let raw = self.rpc_get_block(verified, request).await?;
                self.pipeline.finish_block_read(verified, request, raw).await
            })
            .await
            .map_err(capability_error_from_transport)
        })
    }
}
```

Mandatory checks such as EVM chain-id validation belong in `prepare_call`, before the verified token
exists for operation IO:

```rust
let observed_chain_id = self.client.probe_chain_id(source).await?;
if observed_chain_id != self.binding.expected_chain_id() {
    return Err(source_mismatch_diagnostic(...));
}
let verified = VerifiedEvmCall { source, evidence };
```

Non-authoritative observers such as metrics or tracing may be called from this private execution
shape, but they must not mint verified tokens, select mandatory checks, change provider authority,
or make provider success depend on telemetry persistence.

### Private Verified-Call Token

Raw operation IO must require a private verified token. Source-probe IO used to mint that token is a
separate private validation-pipeline path with a closed method set, such as EVM `eth_chainId` or BTC
`getblockchaininfo`; it is not a general RPC helper and must not be callable from adapters, states,
ops, app assembly, or provider operation implementations.

```rust
struct VerifiedEvmCall<'a> {
    source: &'a EvmRuntimeSource,
    evidence: RedactedEvmSourceEvidence,
}

impl EvmJsonRpcNetworkProvider {
    async fn validate_call(&self, operation: EvmOperation) -> TransportResult<VerifiedEvmCall<'_>> {
        // ensure provider initialization checks have run
        // route lookup
        // source policy selection
        // chain/source probe
        // sync/finality/rate/feature/method checks
        // redacted evidence construction
        // all future mandatory per-call checks
    }

    async fn rpc_call(
        &self,
        verified: &VerifiedEvmCall<'_>,
        method: &'static str,
        params: serde_json::Value,
    ) -> TransportResult<serde_json::Value> {
        // capability-provider operation JSON-RPC IO
    }
}
```

No provider method should be able to call capability-provider operation IO with just a route/source.
It must have a `Verified*Call` value, and only the bound provider's sealed pipeline can mint
one.

This rule does not require every reusable low-level protocol client method in a transport crate to
take a verified token. For example, validation itself may need a private pre-token chain probe such
as EVM `eth_chainId` or BTC `getblockchaininfo`. The hard rule is narrower: operation IO used to
satisfy capability provider traits must be behind the verified-call/session value.

### Many Validations Without Public Middleware

A transport with many mandatory checks still uses one structural gate. `validate_call` may delegate
to private transport-local helpers for route resolution, source selection, source identity probes,
sync/finality checks, operation allowlists, rate or retry policy, redacted diagnostic construction,
and any future mandatory call checks.

These helpers are not public traits, middleware layers, request factories, or adapter callbacks.
They must not be individually callable from adapters, states, app ingress, or replay. Every live
capability method enters through `validate_call`, and every raw operation IO helper requires the
private verified-call token returned by that gate.

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

The macro is only ergonomics. The boundary is still the bound provider, provider-owned sealed
pipeline, and private verified token.

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

This keeps workflow-specific semantic extraction in adapters while moving generic source binding
into transport-owned provider construction.

Binding should happen per certified source intent, not once globally at app startup. A portfolio run
can span multiple networks/sources, so a runner/backend may cache bound providers within one
invocation, but app assembly should not expose one unbound provider as the capability backend.

## Replay And Recorded Providers

Replay must not use live runtime config. The same standard still applies:

- replay assembly builds recorded providers or replay verifiers from certified/replayed semantic
  binding
- recorded providers implement the same capability traits as live bound providers when that reduces
  code paths
- recorded providers validate recorded evidence against their bound binding and operation request
- replay does not resolve source refs or policy ids against current runtime config

Recorded providers may use the same binding types as live bound providers. They should not expose
public guard-style verification helpers.

Recorded providers may use private recorded-evidence pipelines, but those pipelines must be pure
validation over certified binding, operation requests, and recorded evidence. They must not share
live pipeline types that contain runtime config, route/source registries, HTTP clients, source
probes, or live source policy selection.

Do not force every existing replay verifier through a recorded provider if that adds abstraction
without removing duplication. EVM contract lifecycle replay already validates retained evidence
against certified context without live runtime config; that can remain a direct replay verifier if it
is the smaller path.

## Why This Is Better

### Adding Checks Becomes Local

With bound providers, adding a mandatory validation is one transport edit:

```text
add a stage to the provider-owned sealed pipeline
```

It does not require touching every adapter call site, every operation request constructor, and every
provider method.

### Many Checks Stay Understandable

The sealed pipeline keeps `validate_call` from becoming one giant function while preserving one
non-bypassable provider boundary. A transport can test route checks, bind checks, init checks,
call-prepare checks, and call-finish checks separately without letting adapters assemble or skip
them.

### The Compiler Helps

Capability-provider operation helpers take a private verified token. New provider code cannot
accidentally call operation IO without first passing through validation, because it cannot produce
the token any other way.

### Adapters Stop Knowing Transport Ceremony

Adapters bind once and then issue operation requests. They still own workflow semantics, but they no
longer rebuild transport source binding for every operation.

### Provider Traits Become Honest

A successful provider response means:

```text
source binding was enforced
required init/call-prepare/call-finish checks ran
response evidence matches the bound provider binding
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
acceptable only after the bound provider and provider-owned sealed pipeline already enforce the
boundary.

### Public Middleware

Public middleware remains rejected. A wrapper stack can be bypassed by exposing the inner provider or
raw client, can validate one source and delegate to another, and tends to move transport authority
into app/adapter configuration.

MFM must not add public transport middleware, pre-init hooks, post-init hooks, call interceptors, or
validation context traits for provider authority. Those concepts create extra public API surface
without owning authority, and they duplicate app ingress binding validation, runtime resource-lane
preparation, transport provider validation, and replay evidence validation.

### Provider-Owned Pipelines

Provider-owned sealed pipelines are allowed because they are not optional middleware. The provider
has no unchecked capability implementation behind it; the router/client does not implement
capability traits; raw operation IO requires the private token minted by the pipeline; and replay
providers use separate recorded-evidence validation without live runtime config.

### Public Guard Objects

Public guards become general authority and leak transport concerns into states, adapters, replay, and
tests. This RFC explicitly avoids that shape.

### App-Ingress Live Validation

Live validation before `RunAdmitted` is stale by the first real provider call and changes source
mismatch from an attempt-time provider failure into a deployment admission condition. Keep app
ingress no-IO and binding-only.

### Generic Cross-Chain Middleware

EVM network/chain identity, BTC source/network identity, exact anchors, receipt checks, and log
checks are not one erased contract. Similarity between transport families is not enough reason to
add generic cross-chain middleware. Add shared code only when it removes real duplication without
erasing protocol semantics.

## Migration Plan

1. Add capability-owned binding values:
   - `EvmNetworkBinding`
   - `BtcSourceBinding`
2. Add bound providers in live transport crates:
   - `EvmJsonRpcNetworkProvider`
   - `BtcJsonRpcSourceProvider`
3. Add private provider-owned sealed pipelines or equivalent staged functions:
   - static config / route checks
   - no-IO binding checks
   - optional live initialization checks
   - call-prepare checks
   - call-finish response checks
4. Move existing live source checks into the call-prepare path.
5. Make capability-provider operation IO helpers require private `Verified*Call` values.
6. Move capability trait impls from raw routers/clients to bound providers.
7. Make operation requests source-free.
8. Update adapter runtime factories to return bound providers.
9. Remove adapter-local generic source request helpers.
10. Update recorded/replay providers or direct replay verifiers to bind from certified semantic
    binding and validate recorded evidence internally.
11. Update docs:
    - `docs/architecture.md`
    - `docs/design.md`
    - `docs/evm-rpc-routing.md`
    - `docs/btc-rpc-routing.md`
    - `docs/persisted-public-surfaces.md`
12. Add tests proving:
    - raw routers do not implement capability provider traits
    - mandatory checks cannot be skipped by adapters or app assembly
    - capability-provider operation IO helpers cannot be called without private verified tokens
    - bound providers reject source mismatches before operation-specific RPC
    - call-finish checks reject mismatched response identity
    - live initialization checks do not replace per-call source checks
    - adapters no longer construct source-bearing operation requests

## Acceptance Criteria

- No live raw transport router implements capability provider traits.
- No operation request contains source binding fields.
- No adapter-local generic transport request binding helper exists.
- Mandatory transport checks are owned by the bound provider and cannot be selected or skipped by
  adapters.
- No public `Middleware`, `Layer`, provider decorator, `inner`, `unchecked`, or `skip_validation`
  API exists for production transports.
- Provider-owned pipelines are private transport implementation details and cannot be supplied by
  adapters, states, ops, binaries, or app assembly.
- Source-probe IO is private, closed to identity/status probes, and cannot be used for operation IO.
- Capability-provider operation IO helpers require a private verified token.
- Every capability provider implementation in a live transport enters through the required check
  path.
- App ingress validation remains no-IO route/source/signer binding validation.
- Live initialization or warmup checks are not treated as provider-call proof.
- Replay providers or direct replay verifiers validate recorded evidence internally and do not
  consult runtime config.
- Exact scans for public guard-style APIs remain empty.

## Open Questions

- Should binding values live in capability crates immediately, or should the first migration keep
  them private in transport crates until request structs become operation-only? The end state should
  be capability-owned typed binding values.
- Should optional non-semantic hooks for metrics/tracing/retry ever be public per transport family,
  or should they remain private until a real reuse case appears? If public, they must not own
  provider authority or be required for safety.
- Should bound providers cache source probes for a short operation-local window, or should every
  provider call probe source identity independently? The default should be per-call validation unless
  a transport-specific RFC defines safe cache semantics.
- Should `EvmOperation` / `BtcOperation` be persisted in diagnostics as a closed enum, or should it
  remain a transport-local diagnostic field?
- Which existing replay paths should become recorded providers, and which should stay direct replay
  verifiers because that is the smaller code path?
