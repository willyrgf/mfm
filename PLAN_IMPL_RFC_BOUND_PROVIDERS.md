# Implementation Plan: Source-Bound Transport Providers

Status: engineer handoff plan for `RFC_BOUND_PROVIDERS.md`.

## Mandatory Rules

Read these before editing:

- `docs/code-quality.md`
- `docs/architecture.md`
- `docs/design.md`
- `docs/persisted-public-surfaces.md`
- `RFC_BOUND_PROVIDERS.md`

Optimize for fewer concepts, fewer code paths, fewer public types, fewer duplicated
responsibilities, and fewer places future changes must touch. LOC reduction is valuable, but do not
make code cryptic just to make it shorter.

This work is intentionally breaking:

- Do not maintain any backward compatibility.
- All breaking changes are allowed.
- Do not create fallbacks, compatibility constructors, feature flags, deprecated wrappers, aliases,
  hidden shims, or old/new dual paths.
- Delete old code instead of hiding it.
- Delete tests whose only purpose is preserving the old request-owned source-binding path.
- If removed code is needed later, recover it from git history.

Commit progressively. Prefer commits that leave the touched dependency cone compiling and tested.
Do not keep compatibility APIs just to make an artificial intermediate state easier. Suggested commit
subjects below are lower case.

Before each commit, run the Nix-managed gates required by `AGENTS.md`:

```bash
nix run .#check
nix run .#test
nix run .#test-db
```

Run `nix run .#ci` after the major migration or before final merge-readiness validation.

If architecture or design decisions are unclear, spawn an architect-agent before coding further and
pass along every rule in this section.

## Target Architecture

The live transport public shape is:

```text
runtime config
  -> raw router/client
  -> bind checked semantic source binding
  -> bound provider
  -> operation-only capability request
  -> provider-owned sealed pipeline
  -> private Verified*Call / Verified*Session
  -> raw operation IO
  -> call-finish response/evidence checks
```

Only the bound provider implements capability provider traits.

The raw router/client:

- owns runtime config route/source descriptors and low-level protocol mechanics
- exposes no-IO binding validation
- exposes `bind_*` constructors for bound providers
- does not implement live capability provider traits

The bound provider:

- owns the checked semantic binding privately
- implements capability provider traits
- owns a private sealed pipeline for init, call-prepare, operation IO, and call-finish checks
- mints private verified-call/session values
- converts failures to closed redacted provider diagnostics

Capability requests:

- carry operation parameters only
- do not contain source binding fields
- do not expose source binding accessors

Replay:

- uses recorded providers or direct replay verifiers over certified semantic binding and recorded
  evidence only
- never loads runtime config, source registries, HTTP clients, signer providers, or live source
  policy selection

## Forbidden Patterns

Do not add or keep:

- public `*Guard` types
- public transport `Middleware`, `Layer`, interceptor, provider decorator, `inner`, `unchecked`, or
  `skip_validation` APIs
- public live-transport `Preflight`, `PostInit`, `ValidationContext`, or `CallContext` types
- adapter/app-selected mandatory transport checks
- source-bearing operation request constructors
- source-binding request accessors used only for live transport binding
- adapter-local generic request authority helpers
- raw router/client capability provider trait impls
- app wiring that passes one unbound live provider as a capability backend
- replay helper APIs that require callers to manually validate source evidence
- compatibility aliases, deprecated wrappers, hidden fallbacks, feature flags, or old constructors
- `#[allow(dead_code)]` to retain removed APIs

The implementation is incomplete if the old request-owned source-binding path and the new
bound-provider path both remain callable for the same transport family.

## Implementation Order

Use vertical slices that delete the old path for each transport family. BTC is smaller and should go
first. EVM follows once the shape is proven.

### Commit 1: btc bound providers replace source-bound requests

Suggested subject: `btc bind jsonrpc providers to source`

Primary files:

- `crates/btc-capabilities/src/lib.rs`
- `crates/btc-capabilities/tests/capability_contract.rs`
- `crates/transports/btc-jsonrpc-http/src/lib.rs`
- `crates/transports/btc-jsonrpc-http/src/tests.rs`
- `crates/adapters/btc-jsonrpc/src/lib.rs`
- `crates/adapters/btc-jsonrpc/src/tests.rs`
- `crates/adapters/portfolio/src/lib.rs`
- `crates/adapters/portfolio/src/tests.rs`
- `crates/app/src/btc_collector.rs`
- `crates/app/src/lib.rs`
- BTC-related app tests

Capability work:

- Add `BtcSourceBinding` in `mfm-btc-capabilities`.
- Use a checked Bitcoin network tag in the binding; do not leave arbitrary unchecked strings inside
  the binding.
- Delete private `BtcRequestSource`.
- Make `BtcChainHeadRequest` operation-only:
  - constructor takes only `BtcHeadSelection`
  - no `network_id()`, `source_identity()`, or `bitcoin_network()` accessors
- Make `BtcBalanceReadRequest` operation-only:
  - constructor takes `address`, `block_height`, and `block_hash`
  - no source binding accessors
- Update `RedactedBtcSourceEvidence` construction to use `BtcSourceBinding`.
- Keep source evidence fields as redacted audit provenance.
- Update provider trait docs: successful responses already enforced provider binding and
  operation-specific invariants.
- Delete tests that assert source-bearing request constructors/accessors.

Transport work:

- Split the combined live provider shape into:
  - `BtcJsonRpcRouter`: route/source map and no-IO binding validation
  - `BtcJsonRpcSourceProvider`: bound provider implementing `BtcChainHeadReadProvider` and
    `BtcBalanceReadProvider`
- Delete the old combined provider path if it remains callable as a capability backend.
- Add private provider-owned sealed pipeline stages or equivalent functions:
  - `validate_source_binding(&BtcSourceBinding)` with no network IO
  - `prepare_call` that resolves the bound source and probes `getblockchaininfo`
  - call-finish checks for chain-head response identity
- Make source-probe IO private and closed to identity/status probes.
- Make raw operation helpers use `VerifiedBtcCall`.
- Preserve fail-closed behavior:
  - observed Bitcoin network mismatch fails before `getblockhash`, `getblockheader`, or
    `scantxoutset`
  - unsupported balance reads still verify source first when source authority matters
- Ensure diagnostics do not expose URLs, credentials, file paths, request bodies, response bodies,
  or provider messages.

BTC adapter and app work:

- Build `BtcSourceBinding` from certified `ObserveBtcChainHeadConfig`.
- Build operation-only `BtcChainHeadRequest` immediately before provider call.
- Replace any unbound provider capability with a binding factory/router supplied by app assembly.
- `validate_ingress`, if present, should use no-IO binding validation only.
- Add or update recorded provider support:
  - recorded provider is bound to `BtcSourceBinding`
  - recorded provider validates recorded evidence against binding plus operation request
  - no public replay helper should make callers validate source evidence manually
- Update `btc_collector` app wiring to construct a router/factory, not an unbound capability
  provider.
- Delete old BTC request builders and old replay helper APIs.

Portfolio BTC work:

- If operation-only BTC requests break portfolio, update the portfolio BTC read path in this commit.
- Bind BTC providers from `PortfolioNetworkReadIntent::Bitcoin` and
  `PortfolioBalanceReadIntent::BitcoinNativeBalance`.
- Delete portfolio BTC helpers whose only job is attaching `network_id`, `source_identity`, and
  `bitcoin_network` to every request.
- Do not commit a state where the BTC capability API has changed but portfolio still depends on the
  old BTC request constructors.

Focused verification:

```bash
cargo test -p mfm-btc-capabilities
cargo test -p mfm-transports-btc-jsonrpc-http
cargo test -p mfm-adapters-btc-jsonrpc
cargo test -p mfm-adapters-portfolio
cargo test -p mfm-app
```

### Commit 2: portfolio binds btc and evm providers per source intent

Suggested subject: `portfolio binds providers per source intent`

Primary files:

- `crates/adapters/portfolio/src/lib.rs`
- `crates/adapters/portfolio/src/tests.rs`
- `crates/adapters/portfolio/README.md`
- `crates/app/src/lib.rs`
- app portfolio tests

Work:

- Replace `PortfolioRuntimeValidator` plus raw provider fields with a transport factory shape that
  binds providers per certified source intent.
- The final factory should look like the RFC shape:
  - `bind_evm(network_id, expected_chain_id) -> Arc<dyn PortfolioEvmProvider>`
  - `bind_btc(network_id, source_identity, bitcoin_network) -> Arc<dyn PortfolioBtcProvider>`
- Do not keep a separate raw provider path for BTC after commit 1.
- Do not introduce a temporary public compatibility interface for old EVM source-bound requests.
  If EVM is not migrated in this commit, keep any remaining EVM work clearly local and remove it in
  the EVM vertical slice. Prefer combining this commit with Commit 3 if that avoids a temporary
  concept or a broken workspace.
- Bind providers in the runner/backend from semantic read intents.
- Cache bound providers within one runner invocation only if it materially reduces duplicate code;
  do not add a global app-level cache or cross-run authority concept.
- Replace BTC request construction with operation-only constructors.
- After EVM migration, replace EVM request construction with operation-only constructors.
- Delete portfolio helpers whose only job is attaching source binding to requests.
- Keep workflow/domain validation in portfolio states and adapters; move generic source binding to
  transport provider construction.

Focused verification:

```bash
cargo test -p mfm-adapters-portfolio
cargo test -p mfm-app
```

### Commit 3: evm bound providers replace source-bound requests

Suggested subject: `evm bind jsonrpc providers to network`

Primary files:

- `crates/evm-capabilities/src/lib.rs`
- `crates/evm-capabilities/tests/capability_contract.rs`
- `crates/transports/evm/src/lib.rs`
- `crates/transports/evm/src/tests.rs`
- `crates/adapters/evm-contracts/src/lib.rs`
- `crates/adapters/evm-contracts/src/tests.rs`
- `crates/app/src/evm_contracts.rs`
- `crates/app/src/lib.rs`
- EVM app tests

Capability work:

- Add `EvmNetworkBinding` in `mfm-evm-capabilities`.
- Delete private `EvmRequestSource`.
- Make every EVM request operation-only:
  - `EvmChainIdentityRequest`
  - `EvmBlockReadRequest`
  - `EvmBalanceReadRequest`
  - `EvmCallReadRequest`
  - `EvmCodeReadRequest`
  - `EvmLogsReadRequest`
  - `EvmNonceReadRequest`
  - `EvmFeeReadRequest`
  - `EvmGasEstimateRequest`
  - `EvmTransactionSubmitRequest`
  - `EvmReceiptReadRequest`
  - `EvmNonceOccupancyReadRequest`
- Remove constructors that take `(network_id, expected_chain_id, ...)`.
- Remove request `network_id()` and `expected_chain_id()` accessors.
- Update `RedactedEvmSourceEvidence` construction to use `EvmNetworkBinding`.
- Keep source refs and policy ids as audit provenance only.
- Update provider trait docs to refer to provider-bound source binding, not request-local source
  authority.
- Delete tests that assert old request source fields or constructors.

Transport work:

- Keep `EvmJsonRpcClient` as the raw router/client only.
- Remove every EVM capability provider trait impl from `EvmJsonRpcClient`.
- Add `EvmJsonRpcNetworkProvider` with private `EvmNetworkBinding`.
- Add `validate_network_binding(&EvmNetworkBinding)` with no network IO.
- Add `bind_network(EvmNetworkBinding) -> EvmJsonRpcNetworkProvider`.
- Add a private provider-owned sealed pipeline or equivalent staged functions:
  - `ensure_initialized` for optional local/warmup checks
  - `prepare_call` for route lookup, source candidate selection, `eth_chainId`, sync/feature checks,
    and evidence construction
  - operation IO helpers that require `VerifiedEvmCall`
  - call-finish checks for block/hash/log/receipt/transaction identity
- Keep `eth_chainId` probing as private closed source-probe IO, not general operation IO.
- Preserve current fallback semantics:
  - retry next source on transport/RPC failure where existing policy permits
  - fail closed on reachable wrong-chain source
- Add tests that prove each provider call probes source identity unless a future transport-specific
  cache RFC changes that rule.
- Ensure diagnostics remain redacted.

EVM contract adapter and app work:

- Change `EvmContractRuntimeFactory` signatures to bind with expected chain id:
  - `validate_runtime_for(network_id, expected_chain_id, signer_ref)`
  - `read_runtime_for(network_id, expected_chain_id)`
  - `runtime_for(network_id, expected_chain_id)`
- Delete `EvmRequestAuthority`.
- Replace all EVM request construction with operation-only constructors.
- Delete adapter-side source evidence revalidation after successful provider calls.
- Keep workflow-level checks:
  - prepared transaction evidence matches certified context
  - signed payload hash matches prepared transaction
  - receipt status satisfies lifecycle policy
  - replay evidence matches certified context
- Update `RuntimeConfigEvmContractRuntimeFactory` to construct `EvmNetworkBinding` and bind
  `EvmJsonRpcNetworkProvider`.
- Remove app wrappers that reload runtime config per provider call.

Focused verification:

```bash
cargo test -p mfm-evm-capabilities
cargo test -p mfm-transports-evm
cargo test -p mfm-adapters-evm-contracts
cargo test -p mfm-app
```

### Commit 4: replay and evidence cleanup

Suggested subject: `replay validates provider binding evidence`

Primary files:

- `crates/adapters/btc-jsonrpc/src/lib.rs`
- `crates/adapters/btc-jsonrpc/src/tests.rs`
- `crates/adapters/evm-contracts/src/lib.rs`
- `crates/adapters/evm-contracts/src/tests.rs`
- `crates/app/src/lib.rs`
- replay-related app tests

Work:

- Ensure recorded providers or replay verifiers validate evidence against certified semantic
  binding plus operation request.
- Delete public helper APIs that ask callers to validate recorded source evidence manually.
- Do not force direct EVM contract replay through a recorded provider if the direct verifier is
  smaller and already validates against certified context without runtime config.
- Ensure recorded provider pipelines are pure:
  - no runtime config
  - no route/source registries
  - no HTTP clients
  - no live source probes
  - no live policy fallback selection
- Ensure replay error diagnostics remain redacted and source refs/policy ids are audit provenance
  only.

Focused verification:

```bash
cargo test -p mfm-adapters-btc-jsonrpc
cargo test -p mfm-adapters-evm-contracts
cargo test -p mfm-app
```

### Commit 5: docs and contract tests enforce the new boundary

Suggested subject: `docs require bound transport providers`

Primary docs:

- `docs/architecture.md`
- `docs/design.md`
- `docs/evm-rpc-routing.md`
- `docs/btc-rpc-routing.md`
- `docs/persisted-public-surfaces.md`
- `crates/adapters/portfolio/README.md` if changed behavior is described there
- relevant crate rustdoc examples

Docs work:

- Replace old "source-bound requests" language with:
  - provider-bound semantic binding
  - operation-only requests
  - raw router/client not implementing capability traits
  - provider-owned sealed pipelines
  - private source-probe IO and token-gated operation IO
- Clarify source refs and policy ids remain audit provenance only.
- Clarify app ingress validation remains no-IO binding validation only.
- Clarify replay validates recorded evidence from certified binding and never consults runtime
  config.

Contract test work:

- Add focused exact scans in an appropriate architecture or app test for forbidden production
  surfaces:
  - `EvmRequestAuthority`
  - old source-bearing request constructors in adapter code
  - raw router/client capability impls
  - public `Middleware`, `Layer`, `inner`, `unchecked`, `skip_validation`
  - public guard-style APIs
- Keep scans precise enough not to ban domain config fields such as portfolio network ids or
  persisted redacted evidence fields.

Focused verification:

```bash
cargo test -p mfm-integration-tests --test cargo_metadata_contract
cargo test -p mfm-app --test transport_boundaries
cargo test --workspace
```

## Required Cleanup Scans

Run these during the migration and again before final validation. Adjust paths only when the code
has moved; do not weaken the intent of the scans.

```bash
rg -n "EvmRequestAuthority|EvmRequestSource|BtcRequestSource" crates
rg -n "impl .*Provider for EvmJsonRpcClient|impl_provider!\\(" crates/transports/evm
rg -n "BtcJsonRpcChainHeadProvider" crates
rg -n "with_middleware|Middleware|Layer|inner\\(|unchecked|skip_validation" crates
rg -n "Evm[A-Za-z]+Request::new\\([^\\n]*(network_id|expected_chain_id)" crates/adapters crates/app crates/transports
rg -n "Btc[A-Za-z]+Request::new\\([^\\n]*(network_id|source_identity|bitcoin_network)" crates/adapters crates/app crates/transports
```

Expected final state:

- scans for old helper types and public middleware escape hatches return no production hits
- old combined provider names are gone or no longer capability providers
- request constructor scans find no source-binding constructor usage
- any remaining `network_id`, `expected_chain_id`, `source_identity`, or `bitcoin_network` use is
  semantic config, binding construction, redacted evidence, replay verification, or diagnostics

## Verification Checklist

Use focused checks while developing:

```bash
cargo fmt --all -- --check
cargo check --workspace
cargo test -p mfm-btc-capabilities
cargo test -p mfm-transports-btc-jsonrpc-http
cargo test -p mfm-adapters-btc-jsonrpc
cargo test -p mfm-evm-capabilities
cargo test -p mfm-transports-evm
cargo test -p mfm-adapters-evm-contracts
cargo test -p mfm-adapters-portfolio
cargo test -p mfm-app
cargo test --workspace
```

Before each commit:

```bash
nix run .#check
nix run .#test
nix run .#test-db
```

Before final merge readiness:

```bash
nix run .#ci
```

If a parity test needs live Postgres, Reth, or Bitcoin Core, start the service manually and run the
focused Cargo test with explicit environment variables. Do not weaken the test or add a fallback.

## Final Acceptance

The work is complete only when:

- operation requests are source-free
- capability binding values live in capability crates
- raw routers/clients do not implement live capability provider traits
- bound providers own sealed pipelines and private verified-call/session values
- raw operation IO cannot be reached without a private verified token
- adapters bind providers from certified semantic source intent and issue operation-only requests
- app assembly no longer passes unbound live providers as capability backends
- replay validates recorded evidence without runtime config
- old constructors, old helpers, old tests, old docs, and old compatibility paths are deleted
- exact scans confirm old and new source-binding designs do not coexist as callable production paths
