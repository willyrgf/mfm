# Revamp Portfolio Snapshot

## Purpose

This note captures:

- what the repo supports today for portfolio snapshots
- the gap between the current wallet-balance slice and a real multi-asset / protocol-aware portfolio model
- a proposed direction for a revamp, including Aave V3 collateral / debt / staked assets
- the new canonical direction for `PortfolioSnapshot`, without preserving old portfolio snapshot code or config compatibility

This is intended as a design note, not a final spec.

## Current Repo Anchors

- `crates/ops/portfolio-tracker-op/src/lib.rs`
- `crates/evm-runtime/src/states/read.rs`
- `crates/app/src/lib.rs`
- `bin/cli/src/commands/portfolio/snapshot.rs`
- `bin/cli/README.md`
- `docs/ops-and-states.md`
- `crates/states/aave-v3/src/states.rs`
- `crates/states/aave-v3/src/manifest.rs`

## Suggested Repo Placement

### Recommended layout

The cleanest placement is:

- new portfolio-domain shared state crate for orchestration and aggregation
- new wallet-domain shared state crate for wallet implementations and signing concerns
- new symbol-domain shared state crate for balance/value implementations and symbol behavior
- Aave-specific portfolio readers in the Aave shared state crate
- existing keystore crates reused as wallet implementation building blocks
- protocol-agnostic EVM primitives in `evm-runtime`
- thin planning / transport layers on top

### 1. Portfolio orchestration crate

Preferred new crate:

- `crates/states/portfolio/`

This crate should become the home for:

- the top-level `PortfolioConfig`
- the top-level `PortfolioSnapshot`
- the top-level `PortfolioReport`
- portfolio orchestration and aggregation states

Suggested contents:

- `crates/states/portfolio/src/lib.rs`
  - public module surface
- `crates/states/portfolio/src/model.rs`
  - `PortfolioSnapshot`
  - `PortfolioConfig`
- `crates/states/portfolio/src/states.rs`
  - portfolio-level aggregation states
- `crates/states/portfolio/src/writers.rs`
  - portfolio-level writers

Why:

- portfolio should orchestrate wallets and symbols, not own their internal implementations
- wallet and symbol should be reusable state domains in `crates/states/`, not portfolio-private modules
- this keeps the op thin, which matches the repo architecture
- the app, CLI, and API can all depend on one canonical portfolio orchestration surface

### 2. Wallet shared-state crate

Preferred new crate:

- `crates/states/wallet/`

This crate should become the first-class wallet domain.

Wallet should be treated as its own shared-state boundary because it will carry:

- wallet identity and address resolution
- signer implementations
- signer capability discovery
- transaction building / signing / submission preparation
- wallet metadata
- wallet capability discovery
- wallet-scoped orchestration
- wallet-level readers / writers

Suggested contents:

- `crates/states/wallet/src/lib.rs`
- `crates/states/wallet/src/model.rs`
  - `WalletConfig`
  - `WalletSnapshot`
  - `WalletCapabilities`
  - `ResolvedWallet`
  - `WalletImplementationConfig`
  - `WalletSignerConfig`
- `crates/states/wallet/src/states.rs`
  - wallet-level orchestration states
  - wallet execution preparation states
- `crates/states/wallet/src/readers.rs`
  - wallet metadata readers
  - wallet capability readers
- `crates/states/wallet/src/signers.rs`
  - signer resolution
  - signing request preparation
  - payload signing helpers
- `crates/states/wallet/src/submit.rs`
  - wallet-scoped submission helpers
- `crates/states/wallet/src/writers.rs`
  - wallet observation writers
- `crates/states/wallet/src/implementations/address_only.rs`
- `crates/states/wallet/src/implementations/keystore.rs`
- `crates/states/wallet/src/implementations/node_managed.rs`
- `crates/states/wallet/src/implementations/external.rs`

Important note:

- `crates/states/wallet/` should not reimplement keystore internals
- it should compose:
  - `crates/states/keystore/`
  - `crates/states/keystore-submit/`
  - any future signer/backends
- protocol crates should not decide directly between keystore, node-managed, or external signer flows
- portfolio snapshots may start mostly read-only, but wallet should still be designed as a repo-level execution domain

### 3. Symbol shared-state crate

Preferred new crate:

- `crates/states/symbol/`

This crate should become the first-class symbol domain.

Symbol should be treated as its own shared-state boundary because it will carry:

- symbol identity and metadata
- symbol implementation selection
- balance reader selection
- valuation reader selection
- symbol normalization rules
- symbol implementation dispatch
- underlying-asset / protocol-position relationships

Suggested contents:

- `crates/states/symbol/src/lib.rs`
- `crates/states/symbol/src/model.rs`
  - `SymbolConfig`
  - `ResolvedSymbolBalanceReader`
  - `ResolvedSymbolValuationReader`
  - `Observation`
  - `ObservationQuantity`
  - `ObservationValue`
  - `BalanceReaderConfig`
  - `SymbolValuationConfig`
  - `ValuationReaderConfig`
- `crates/states/symbol/src/states.rs`
  - symbol-level orchestration states
  - dispatch balance readers
  - dispatch valuation readers
  - normalize symbol observations
- `crates/states/symbol/src/readers.rs`
  - generic symbol reader dispatch
- `crates/states/symbol/src/registry.rs`
  - symbol registry loading
  - symbol lookup helpers
- `crates/states/symbol/src/normalize.rs`
  - canonical observation normalization
- `crates/states/symbol/src/writers.rs`
  - symbol observation writers
- `crates/states/symbol/src/implementations/mod.rs`
- `crates/states/symbol/src/implementations/native.rs`
- `crates/states/symbol/src/implementations/erc20.rs`

Important note:

- `crates/states/symbol/` should own generic symbol behavior
- protocol-specific symbol implementations should stay in their protocol crates
- portfolio should not need to understand how native, ERC-20, debt, or staked symbol implementations differ internally

Examples:

- native and ERC-20 symbol implementations can live in `crates/states/symbol/`
- Aave-specific symbol implementations should live in `crates/states/aave-v3/`

### Wallet and symbol need separate state families and separate crates

Wallet and symbol should not be treated as one flat pool of states.
They also should not be hidden as only submodules of the portfolio crate.

They are distinct workflow layers:

- portfolio layer
  - owns aggregation across wallets
  - owns final snapshot / report writers
- wallet layer
  - owns wallet-scoped orchestration
  - owns wallet network association and wallet-scoped aggregation
  - owns wallet implementation and signing concerns
- symbol layer
  - owns symbol-scoped balance readers
  - owns symbol-scoped valuation readers
  - owns observation normalization and implementation dispatch for native / ERC-20 / protocol / debt / staked symbols

And they are also distinct reusable repo-level domains:

- wallet should serve portfolio snapshots, deploy/configure flows, and future transaction-oriented workflows
- symbol should serve portfolio snapshots, protocol adapters, valuation pipelines, and later symbol discovery / registry workflows

This split matters because both wallet and symbol will grow:

- wallets may eventually need their own metadata, pinning, filters, partial errors, wallet-level summaries, and multiple signer implementations
- symbols will definitely need their own balance logic, valuation logic, protocol adapters, and implementation routing

So the implementation should reflect that growth early instead of:

- collapsing everything into one generic `states.rs`
- burying wallet and symbol logic inside portfolio-only modules

### Concrete initial file skeleton

If we want a concrete starting skeleton, I would begin with this:

```text
crates/states/portfolio/
  src/
    lib.rs
    model.rs
    states.rs
    writers.rs

crates/states/wallet/
  src/
    lib.rs
    model.rs
    states.rs
    readers.rs
    signers.rs
    submit.rs
    writers.rs
    implementations/
      address_only.rs
      keystore.rs
      node_managed.rs
      external.rs

crates/states/symbol/
  src/
    lib.rs
    model.rs
    states.rs
    readers.rs
    registry.rs
    normalize.rs
    writers.rs
    implementations/
      mod.rs
      native.rs
      erc20.rs
```

Suggested first-pass state inventory:

- `crates/states/portfolio/src/states.rs`
  - `LoadPortfolioConfigState`
  - `PinPortfolioNetworksState`
  - `CollectPortfolioWalletSnapshotsState`
  - `CollectPortfolioObservationsState`
  - `CollectPortfolioTotalsState`
- `crates/states/portfolio/src/writers.rs`
  - `WritePortfolioSnapshotState`
  - `WritePortfolioReportState`

- `crates/states/wallet/src/states.rs`
  - `LoadWalletSetState`
  - `ResolveWalletAddressState`
  - `ResolveWalletCapabilitiesState`
  - `PrepareWalletExecutionState`
  - `CollectWalletObservationsState`
- `crates/states/wallet/src/readers.rs`
  - `ReadWalletNativeBalanceState`
  - `ReadWalletMetadataState`
  - `ResolveWalletImplementationState`
- `crates/states/wallet/src/signers.rs`
  - `ResolveWalletSignerState`
  - `BuildWalletSigningRequestState`
  - `SignWalletPayloadState`
- `crates/states/wallet/src/submit.rs`
  - `SubmitWalletTransactionState`
- `crates/states/wallet/src/writers.rs`
  - `WriteWalletObservationSetState`
  - `WriteWalletCapabilityReportState`

- `crates/states/symbol/src/states.rs`
  - `LoadSymbolRegistryState`
  - `ResolveSymbolConfigState`
  - `ResolveSymbolImplementationState`
  - `ReadSymbolBalanceState`
  - `ReadSymbolValuationState`
  - `NormalizeSymbolObservationState`
- `crates/states/symbol/src/readers.rs`
  - `DispatchSymbolBalanceReaderState`
  - `DispatchSymbolValuationReaderState`
- `crates/states/symbol/src/registry.rs`
  - `LookupSymbolConfigState`
- `crates/states/symbol/src/normalize.rs`
  - `ResolveUnderlyingSymbolState`
- `crates/states/symbol/src/writers.rs`
  - `WriteSymbolObservationState`

This is intentionally split so:

- portfolio states do cross-wallet orchestration and final aggregation
- wallet states do wallet-scoped orchestration, implementation/signing resolution, and later transaction execution
- symbol states do symbol-scoped reading, valuation, normalization, and implementation dispatch

### 4. Keep Aave portfolio reads in the Aave shared state crate

For the Aave side, I would start with:

```text
crates/states/aave-v3/
  src/
    lib.rs
    manifest.rs
    states.rs
    portfolio/
      mod.rs
      model.rs
      states.rs
      readers.rs
      normalize.rs
```

Suggested first-pass Aave portfolio state inventory:

- `crates/states/aave-v3/src/portfolio/states.rs`
  - `LoadAaveMarketConfigState`
  - `ReadAaveUserAccountSummaryState`
  - `ReadAaveReservePositionState`
  - `ReadAaveDebtPositionState`
  - `ReadAaveStakingPositionState`
  - `CollectAavePortfolioObservationsState`
- `crates/states/aave-v3/src/portfolio/readers.rs`
  - ABI-driven helpers for Aave calls
  - reserve lookup helpers
  - account summary lookup helpers
- `crates/states/aave-v3/src/portfolio/normalize.rs`
  - Aave raw read -> canonical symbol/portfolio observation

This crate should implement Aave-specific symbol behavior, not generic symbol infrastructure.

### 5. Concrete `evm-runtime` expansion points

I would keep generic reusable expansions here:

```text
crates/evm-runtime/
  src/
    states/
      read.rs
      call.rs
      price.rs
      batch.rs
```

Suggested first-pass generic additions:

- `call.rs`
  - richer ABI-decoded call state
  - generic calldata encode / decode helpers
- `price.rs`
  - generic oracle / quote read states
- `batch.rs`
  - multicall / batched read states

This keeps the protocol-independent building blocks reusable across portfolio, Aave, and any later protocol integrations.

### 6. Keep the current portfolio op thin

Current location:

- `crates/ops/portfolio-tracker-op/`

This crate should become a planner / assembly layer only.

It should do:

- config parsing
- graph expansion
- wiring generic portfolio states with protocol-specific states

It should not own:

- balance reader business logic
- valuation logic
- Aave-specific read logic
- large artifact assembly logic

If we keep the existing crate name, that is fine for now.
If we later want to rename it to match the new canonical model more closely, that can be done separately.

### 7. Keep app / CLI / REST transport-only

Keep thin surfaces in:

- `crates/app/src/lib.rs`
- `bin/cli/src/commands/portfolio/snapshot.rs`
- `bin/rest-api/`

These layers should only do:

- request parsing
- environment/config loading
- start/resume execution
- final response rendering

They should not implement:

- symbol resolution rules
- protocol-specific balance reads
- valuation rules
- portfolio aggregation logic

### 8. Docs to update alongside implementation

When the implementation starts landing, update:

- `docs/ops-and-states.md`
- `docs/architecture.md`
- `docs/redesign.md`
- `bin/cli/README.md`
- `bin/rest-api/README.md`

### Summary recommendation

If I had to choose one concrete direction now:

- add `crates/states/portfolio/` for top-level portfolio orchestration and aggregation
- add `crates/states/wallet/` as a first-class wallet state domain
- add `crates/states/symbol/` as a first-class symbol state domain
- extend `crates/states/aave-v3/` with Aave-specific symbol implementations and portfolio-reading modules
- extend `crates/evm-runtime/` only for protocol-agnostic read / valuation primitives
- keep existing keystore crates as wallet implementation building blocks
- keep `crates/ops/portfolio-tracker-op/`, `crates/app/`, and `bin/*` thin

## What Exists Today

### Portfolio snapshot today

`portfolio_tracker` is currently a wallet-balance snapshot op, not a protocol-position op.

Current flow:

1. Validate `eth_chainId` matches the requested `chain_id`
2. Pin `eth_blockNumber`
3. Read native ETH balance for one wallet
4. Read one ERC-20 `balanceOf(wallet)` per configured token
5. Write a content-addressed snapshot artifact
6. Write a small typed report

Relevant code:

- `crates/ops/portfolio-tracker-op/src/lib.rs`
- `docs/ops-and-states.md`

### Current inputs

Current feature inputs are:

- `address`
- optional `chain_id`
- `tokens`

The CLI exposes `tokens` through `--tokens-json`.
The app also merges `MFM_PORTFOLIO_TOKENS_JSON` into the request.

Important limitations:

- single wallet
- single chain
- explicit token allowlist only
- no token discovery
- no protocol position discovery

### Current snapshot artifact shape

Today the output artifact is roughly:

```json
{
  "wallet_address": "0x...",
  "chain_id": 1,
  "block_number": 100,
  "generated_at_ms": 1234567890,
  "native": { "...": "..." },
  "tokens": [{ "...": "..." }],
  "errors": []
}
```

Important note:

- the artifact contains `tokens`
- the typed report / app response currently only surfaces `snapshot_artifact_id`, `chain_id`, `block_number`, and `native_balance`

So even multi-token wallet balances are only fully visible in the artifact today.

### Current reusable read primitives

Today we already have reusable low-level read states for:

- `ReadU64HexState`
- `NativeBalanceState`
- `TokenBalanceState`
- `EthCallState`

This is enough for:

- native balance
- ERC-20 `balanceOf`
- simple RPC reads

This is not yet enough for rich Aave portfolio reads, because the generic `EthCallState` only decodes:

- raw hex
- `u64`
- `u256` hex

Most Aave reads need richer ABI-decoded outputs such as:

- tuples / structs
- booleans
- addresses
- protocol-specific reserve data

### Aave V3 today

The repo has an Aave V3 shared state crate, but it is for deploy / configure / adaptation flows, not portfolio reading.

What exists:

- deploy manifest loading
- contract deployment
- receipt waiting
- deploy manifest writing
- Origin deploy output adaptation
- runtime configuration
- config report writing

What does not exist:

- Aave user position reader
- Aave reserve reader for portfolio use
- staking / rewards reader
- Aave portfolio aggregation state

## What From Current Aave V3 Can Be Generalized

The current Aave V3 crate is mostly deploy / configure oriented, so most of it should not be copied directly into the new portfolio system.

The useful approach is:

- generalize a few helpers / patterns
- keep protocol-specific logic in Aave
- do not pull deployment-specific states into the portfolio flow

### Detailed audit map

This is the most concrete way to think about the current Aave crate:

- current item
- classification
- recommended destination
- reason

#### A. State-by-state audit

##### `LoadCompileManifestState`

- classification:
  - good candidate to generalize as a pattern
- current responsibility:
  - read JSON from context
  - decode typed value
  - validate
  - persist normalized typed JSON back into context
- recommended destination:
  - do not move this exact state
  - mirror the pattern across:
    - `crates/states/portfolio/`
    - `crates/states/wallet/`
    - `crates/states/symbol/`
- concrete analogs:
  - `LoadPortfolioConfigState`
  - `LoadWalletSetState`
  - `LoadSymbolRegistryState`
- reason:
  - the behavior pattern is generic
  - the payload type is Aave deploy-specific

##### `LoadDeployManifestState`

- classification:
  - good candidate to generalize as a pattern
- current responsibility:
  - load and validate a typed deploy artifact
- recommended destination:
  - pattern should be reused in:
    - `crates/states/portfolio/`
    - `crates/states/symbol/`
    - Aave portfolio modules
- concrete analogs:
  - `LoadAaveMarketConfigState`
  - `LoadValuationSourceConfigState`
- reason:
  - same pattern as above
  - current state is still tightly coupled to deploy manifests

##### `WaitForReceiptState`

- classification:
  - good candidate to generalize as reusable EVM helper logic
- current responsibility:
  - wait for tx receipt
  - enforce success
  - normalize receipt payloads
- recommended destination:
  - if extracted, move to `crates/evm-runtime/`
- concrete analog:
  - `EvmWaitForReceiptsState`
- reason:
  - receipt waiting is a generic EVM concern
  - portfolio itself may not need it immediately, but other ops will

##### `WaitForConfigReceiptState`

- classification:
  - same as `WaitForReceiptState`
- recommended destination:
  - same generic EVM receipt-waiting layer
- reason:
  - it is structurally the same pattern with a different payload type

##### `CollectDeployOutputsState`

- classification:
  - good pattern, not direct reusable code
- current responsibility:
  - read intermediate receipts
  - normalize to one final typed artifact
  - validate before writing
- recommended destination:
  - do not move this code
  - reuse the pattern in:
    - `CollectWalletObservationsState`
    - `CollectPortfolioObservationsState`
    - `CollectAavePortfolioObservationsState`
- reason:
  - the structure is excellent
  - the concrete output type is deploy-specific

##### `AdaptOriginDeployOutputState`

- classification:
  - good pattern, not direct reusable code
- current responsibility:
  - adapt external payload into internal canonical shape
- recommended destination:
  - keep current Aave version where it is
  - reuse the adapter pattern if portfolio imports external configs / valuations / discovery payloads
- reason:
  - adapter pattern is broadly useful
  - current payload is deploy-tool-specific

##### `CollectConfigOutputsState`

- classification:
  - good pattern, not direct reusable code
- current responsibility:
  - read normalized intermediate data
  - construct final typed report payload
  - persist it
- recommended destination:
  - use as a template for portfolio reporting states
- concrete analogs:
  - `CollectWalletObservationsState`
  - `WritePortfolioReportState`
- reason:
  - it is a clean report assembly pattern
  - the payload is configure-specific

##### `DeployContractState`

- classification:
  - not useful for portfolio reuse
- current responsibility:
  - mutation / deployment
- reason:
  - portfolio work is read-oriented
  - this does not help with balance or valuation flows

##### `ConfigureRuntimeCallState`

- classification:
  - not useful for portfolio reuse
- current responsibility:
  - mutation / post-deploy transaction submission
- reason:
  - also mutation-oriented
  - not relevant to portfolio reading or valuation

##### `WriteDeployManifestState`

- classification:
  - low reuse value for portfolio
- current responsibility:
  - write deploy artifact to context export key
- reason:
  - the writing part is generic, but this exact state is too specialized to justify reuse
  - better to write dedicated portfolio writers

##### `WriteConfigReportState`

- classification:
  - low reuse value for portfolio
- current responsibility:
  - write configure report to export key
- reason:
  - same as above
  - the exact code is too shape-specific

#### B. Helper-function audit

##### `read_compile_manifest`

- classification:
  - good pattern, not direct reusable code
- current responsibility:
  - context read + typed decode wrapper
- recommended destination:
  - equivalents should live near the corresponding:
    - portfolio model loaders
    - wallet model loaders
    - symbol model loaders
- reason:
  - tiny helper
  - useful style, not worth lifting as-is

##### `read_deploy_manifest_loaded`

- classification:
  - same as `read_compile_manifest`
- reason:
  - same loader-wrapper pattern

##### `encode_call_data`

- classification:
  - strong candidate to generalize
- current responsibility:
  - parse ABI from artifact
  - resolve function call
  - encode calldata
- recommended destination:
  - `crates/evm-runtime/`
- reason:
  - ABI-driven calldata encoding is generic
  - protocol readers should not each reinvent this

##### `send_contract_transaction`

- classification:
  - split candidate, not reusable as-is
- current responsibility:
  - normalize sender
  - attach calldata
  - send EVM transaction
- recommended destination:
  - split the responsibility across:
    - `crates/states/wallet/` for sender / signer resolution
    - `crates/evm-runtime/` for raw EVM transaction submission helpers
    - protocol crates for protocol-specific call intent
- reason:
  - the current helper bundles wallet concerns, transport concerns, and Aave call intent
  - the new architecture should keep those boundaries separate

#### C. Manifest / validation audit

##### `validate_deploy_runtime_config` / `validate_configure_runtime_config`

- classification:
  - good template pattern
- recommended destination:
  - keep current versions in Aave
  - mirror the style in portfolio config validation
- reason:
  - typed config validation with exact failure cases is the right pattern
  - current functions validate Aave deploy/configure types, not portfolio types

##### `decode_compile_manifest` / `decode_deploy_manifest` / `decode_origin_deploy_output`

- classification:
  - good template pattern
- recommended destination:
  - keep in Aave
  - create portfolio-specific `decode_*` functions in the new portfolio crate
- reason:
  - decode + validate pairing is valuable
  - payload types are Aave-only

##### `validate_compile_manifest` / `validate_origin_deploy_output` / `validate_deploy_manifest`

- classification:
  - good template pattern
- recommended destination:
  - keep in Aave
  - use the same approach for:
    - `validate_portfolio_config`
    - `validate_wallet_config`
    - `validate_symbol_config`
    - `validate_aave_market_config`
- reason:
  - explicit invariants and exact error codes are exactly what we want in portfolio code too

##### `contract_from_manifest` / `origin_contract_from_output`

- classification:
  - good candidate to generalize as a local pattern
- recommended destination:
  - do not create one global helper immediately
  - replicate the same style for:
    - `wallet_from_config`
    - `symbol_from_registry`
    - `market_from_registry`
    - `reserve_from_market`
- reason:
  - stable-id lookup with typed missing-entity errors is broadly useful
  - the concrete entity types should stay local to each domain

##### `ContractArtifactJson`

- classification:
  - medium-value generalization candidate
- current responsibility:
  - Aave-local wrapper around ABI/bytecode payloads with parse support
- recommended destination:
  - avoid introducing more protocol-local artifact wrappers if shared EVM types can be used directly
- reason:
  - if portfolio/Aave readers need ABI artifacts, prefer one shared artifact representation in generic EVM layers

#### D. Intermediate payload audit

##### `PendingDeployment`

- classification:
  - not useful for portfolio reuse
- reason:
  - deploy-flow internal state only

##### `DeploymentReceipt`

- classification:
  - not useful for portfolio reuse
- reason:
  - deploy-flow internal state only

##### `PendingRuntimeCall`

- classification:
  - not useful for portfolio reuse
- reason:
  - configure-flow internal state only

### Practical conclusion from the audit

The current Aave crate gives us:

- several good architectural patterns
- a few helper shapes worth moving into shared EVM/runtime layers
- almost no direct code that should be transplanted into portfolio logic unchanged

So the right move is:

- reuse patterns aggressively
- extract only truly generic helper logic
- split wallet-related execution concerns out of protocol helpers instead of generalizing mixed helpers unchanged
- keep Aave portfolio reads as new code in the Aave crate
- keep portfolio orchestration as new code in the portfolio crate
- keep wallet implementation logic as new code in the wallet crate
- keep symbol implementation / dispatch logic as new code in the symbol crate

### Boundary correction implied by first-class wallet and symbol crates

The current Aave deploy/configure states make sense for today, but they also show a boundary we should tighten in the new architecture.

Right now a protocol state can still end up owning too much of:

- wallet selection
- signer resolution
- calldata preparation
- transaction submission
- protocol-specific intent

The new split should be:

- wallet crate:
  - wallet implementation selection
  - signer capability discovery
  - signer resolution
  - wallet-scoped signing / submission preparation
- `evm-runtime`:
  - generic ABI encode / decode
  - generic EVM call / receipt / submit helpers
- symbol crate:
  - generic symbol implementation routing
  - canonical observation normalization
- Aave crate:
  - Aave market / reserve / user-position logic
  - Aave-specific symbol implementations
  - Aave raw reads -> canonical symbol observations

That gives us a cleaner long-term rule:

- protocol crates should describe protocol intent and protocol-specific decoding
- wallet crates should describe who is acting and how they can sign / submit
- symbol crates should describe how a symbol is observed and valued
- portfolio crates should aggregate the results

## Problem Statement

The current portfolio model is too narrow.

It assumes:

- one wallet
- one chain
- balances only
- wallet assets and protocol positions are different categories with different implied workflows

That model breaks down once we want:

- multiple wallets
- multiple different assets
- Aave V3 collateral
- Aave V3 debt
- staked assets
- one unified portfolio view

## Proposed Direction

### Explicit break decision

This effort should not preserve compatibility with the current portfolio snapshot request, response, artifact, or internal config model.

This means:

- no `PortfolioSnapshotV2`
- no compatibility shim for the current single-wallet request shape
- no compatibility shim for the current typed report shape
- no compatibility shim for the current artifact shape
- no requirement to preserve old portfolio snapshot config compatibility
- the existing `portfolio snapshot` surface is replaced in place

This repository is currently on a dev branch, so this in-place break is acceptable and preferred.
The goal is to avoid spending design energy on compatibility baggage while the canonical portfolio model is still being defined.

The goal is to define the new canonical `PortfolioSnapshot` and update the CLI, app, REST, Nix tasks, CI expectations, and docs in the same change whenever the implementation lands.

### Core idea

A portfolio should be modeled as:

`portfolio_x -> {wallet_a, wallet_b, ..., wallet_z} -> {symbol_a, symbol_b, ..., symbol_n} -> symbol_config`

The key idea is that `symbol` is a higher-order entity with configuration, rather than a thin display label.

This allows:

- native balances
- ERC-20 wallet balances
- Aave collateral
- Aave debt
- staked assets

to all be handled by the same workflow shape.

### Important refinement: use stable symbol identity, not just display symbol

A raw display symbol like `USDC` is not enough as an identity key.

Examples that are all different portfolio entries:

- wallet USDC
- Aave V3 supplied USDC
- Aave V3 collateral-enabled USDC
- Aave V3 variable debt USDC
- Aave V3 stable debt USDC

Those may share a human display symbol, but they are not the same portfolio symbol entry.

So the model should use:

- `symbol_id`: stable machine identity
- `display_symbol`: optional human label

Example `symbol_id`s:

- `eth.native.mainnet`
- `usdc.wallet.mainnet`
- `aave_v3.usdc.collateral.mainnet`
- `aave_v3.usdc.variable_debt.mainnet`
- `aave_v3.aave.staked.mainnet`

### Multi-network is first-class in the first implementation

Multi-network support is not an optional future extension.
It is part of the base canonical model.

That means:

- `PortfolioConfig.networks` is required from day one
- every wallet is attached to exactly one `network_id`
- every symbol is attached to exactly one `network_id`
- every valuation source reference includes an explicit `network_id`
- `PortfolioSnapshot.network_pins` records one pinned block per referenced network

This is the only clean way to make the type system and integration points correct across:

- multiple wallets on one network
- multiple wallets across multiple networks
- balance reads on one network with valuation reads on another network
- future protocol integrations that depend on network-local contracts

## Proposed Domain Model

### Portfolio

A portfolio is a logical grouping of wallets and symbol configs.

Suggested responsibilities:

- stable `portfolio_id`
- list of wallets
- shared symbol registry
- shared network registry
- optional portfolio-level metadata

Important canonical encoding rule:

- the registries are logical maps keyed by ID
- the serialized JSON form uses sorted arrays for deterministic hashing and stable diffs

### Wallet

A wallet is an address on a network that owns or participates in positions.

Suggested fields:

- `wallet_id`
- `address`
- `network_id`
- `symbol_ids`

### Valuation source registry and price resolution

Valuation sources must be first-class config, not hidden runtime wiring.

The canonical contract must keep these identities separate:

- `PriceSourceRef.source_id`
  - logical valuation source identity
- `NetworkConfig.rpc_source_id`
  - transport routing hint for EVM JSON-RPC reads on a network

A valuation `source_id` must not be overloaded as an RPC transport selector.
It resolves through a typed valuation source registry loaded alongside the portfolio config.

This means runtime "price discovery" is intentionally narrow in the base design:

- snapshot execution is config-driven
- runtime resolves explicit `source_id` refs from symbol valuation routes
- runtime deduplicates direct price reads across symbols that share the same source
- runtime does not heuristically choose an oracle from a raw symbol string or display symbol

If a future discovery feature is added, it must run before snapshot execution and materialize:

- explicit `PriceSourceRef` entries in symbol configs
- explicit `ValuationSourceConfig` entries in the valuation source registry

Suggested first-pass registry shape:

```json
{
  "sources": [
    {
      "source_id": "chainlink_eth_usd",
      "network_id": "ethereum-mainnet",
      "base_symbol_id": "eth.native.ethereum-mainnet",
      "quote": "USD",
      "reader": {
        "kind": "evm_oracle",
        "oracle_kind": "chainlink_aggregator_v3",
        "config": {
          "contract_address": "0x0000000000000000000000000000000000000000"
        }
      }
    }
  ]
}
```

Validation rules:

- every `PriceSourceRef.source_id` must resolve to exactly one valuation source entry
- the registry entry must match the referring `PriceSourceRef` on:
  - `network_id`
  - `base_symbol_id`
  - `quote`
- unknown or mismatched source refs are configuration errors
- unsupported valuation source reader kinds must raise structured errors
  - no implicit fallback
  - no hidden oracle selection

This contract lets the revamp proceed before live oracle integrations are finished:

- `fixed_unit_price` remains sufficient for tests and tightly scoped manual configs
- live source families can land incrementally behind `ValuationSourceReaderConfig.kind`
- source-kind scaffolding may return `unsupported_valuation_source_kind` until implemented

### Symbol config

A symbol config describes how a portfolio entry should be:

- read for balance / quantity
- valued in quote units such as BTC or USD
- interpreted semantically

Suggested shape:

```json
{
  "symbol_id": "aave_v3.usdc.collateral.ethereum-mainnet",
  "display_symbol": "USDC",
  "kind": "protocol_position",
  "role": "collateral",
  "network_id": "ethereum-mainnet",
  "protocol": "aave_v3",
  "balance_reader": {
    "kind": "protocol_position",
    "protocol": "aave_v3",
    "reader": "reserve_position",
    "config": {
      "market_id": "aave-v3-mainnet",
      "reserve_id": "usdc",
      "use_as_collateral_required": true
    }
  },
  "valuation": {
    "quotes": [
      {
        "quote": "USD",
        "priced_symbol_id": "usdc.wallet.ethereum-mainnet",
        "reader": {
          "kind": "direct_price",
          "source": {
            "source_id": "chainlink_usdc_usd",
            "network_id": "ethereum-mainnet",
            "base_symbol_id": "usdc.wallet.ethereum-mainnet",
            "quote": "USD"
          }
        }
      },
      {
        "quote": "BTC",
        "priced_symbol_id": "usdc.wallet.ethereum-mainnet",
        "reader": {
          "kind": "derived_unit_price",
          "numerator": {
            "source_id": "chainlink_usdc_usd",
            "network_id": "ethereum-mainnet",
            "base_symbol_id": "usdc.wallet.ethereum-mainnet",
            "quote": "USD"
          },
          "denominator": {
            "source_id": "chainlink_btc_usd",
            "network_id": "ethereum-mainnet",
            "base_symbol_id": "btc.wallet.ethereum-mainnet",
            "quote": "USD"
          }
        }
      }
    ]
  },
  "decimals": 6,
  "underlying_symbol_id": "usdc.wallet.ethereum-mainnet",
  "metadata": {
    "notes": "position derived from Aave reserve state"
  }
}
```

Suggested top-level config fields:

- `symbol_id`
- `display_symbol`
- `kind`
- `role`
- `network_id`
- `protocol`
- `balance_reader`
- `valuation`
- `decimals`
- `underlying_symbol_id`
- `metadata`

Suggested `kind` values:

- `native_balance`
- `erc20_balance`
- `protocol_position`
- `staked_position`

Suggested `role` values:

- `asset`
- `native`
- `collateral`
- `debt`
- `staked`

This keeps native, wallet, collateral, debt, and staked positions on the same conceptual level.

### Symbol characteristics drive both quantity and value

The characteristic of a symbol is what determines:

- how we obtain the balance / quantity for that symbol
- how we obtain the value of that quantity in quote units such as BTC or USD

Examples:

- native ETH:
  - balance from `eth_getBalance`
  - value from ETH/USD and ETH/BTC pricing sources
- wallet USDC:
  - balance from ERC-20 `balanceOf`
  - value from USDC/USD and derived BTC quote
- Aave V3 collateral USDC:
  - balance from Aave reserve position reads
  - value from the same underlying pricing rules as USDC, plus protocol metadata
- Aave V3 debt USDC:
  - balance from Aave debt position reads
  - value from the same quote sources, but represented as debt exposure

This is the main reason symbol config has to be a first-class entity, not just display metadata.

### Quote support is mandatory

The new `PortfolioSnapshot` should support value output in explicit quote units.

Initial quote targets should include at least:

- `USD`
- `BTC`

This means the snapshot pipeline should not stop at raw balances.
It must also produce value observations for configured quote units.

`quote_codes` is portfolio-level configuration.
Every `SymbolConfig` must provide exactly one valuation route for every configured quote code.

### Valuation semantics must stay simple and composable

The valuation model should be based on explicit unit-price routes.

Core semantics:

- every valuation computes a unit price first
- `value_dec = amount_dec * unit_price_dec`
- the unit price applies to `priced_symbol_id`, not to a display symbol string
- protocol positions usually price through `underlying_symbol_id`
- every direct price source is explicitly identified by `source_id` and `network_id`
- every derived price route is explicit arithmetic over direct price sources

Canonical route semantics:

- `fixed_unit_price`
  - use a fixed decimal-string unit price
  - mostly useful for tests or tightly scoped manual configs
- `direct_price`
  - read one unit price from one source
  - example: `ETH/USD`, `USDC/USD`, `ETH/BTC`
- `derived_unit_price`
  - compute `numerator / denominator`
  - both inputs must share the same quote unit
  - example: `USDC/BTC = (USDC/USD) / (BTC/USD)`

Replay semantics:

- each direct price source carries `network_id`
- each `ObservationValue` records the concrete source refs that were used
- each source ref resolves against a pinned network block from `network_pins`

Registry resolution semantics:

- `PriceSourceRef.source_id` resolves through the valuation source registry, not the network transport registry
- the resolved source config determines the concrete reader implementation family
- the network used for the read comes from the resolved source config plus the pinned `network_id`
- `rpc_source_id` remains a transport concern of `NetworkConfig`, never a valuation-source concern

This keeps valuation:

- deterministic
- multi-network aware
- composable
- reusable across wallet balances, protocol positions, and staked positions

## Canonical Type Definitions

This section turns the model into an exact starting schema.

These are design-level canonical types for the new implementation.
They are not final Rust code yet, but they are close enough to drive the crate/module implementation.

### Type conventions

- no floats in config, snapshot, or report payloads
- all decimal quantities and values are decimal strings
- IDs are stable strings
- collections should be sorted deterministically before hashing / persistence
- debt quantities should be stored as positive quantities with `role = debt`
  - netting should happen only in derived summaries
- `metadata` and protocol config blobs must still be canonical JSON objects
  - no floats
  - no secrets
  - deterministic key ordering before persistence

Suggested scalar aliases:

- `PortfolioId = String`
- `WalletId = String`
- `SymbolId = String`
- `NetworkId = String`
- `MarketId = String`
- `DecimalString = String`
- `AddressString = String`

### Recommended type ownership by crate

- `crates/states/portfolio/src/model.rs`
  - `PortfolioConfig`
  - `NetworkConfig`
  - `PortfolioSnapshot`
  - `NetworkPin`
  - `PortfolioReport`
  - `WalletReport`
  - `PortfolioQuoteTotal`
- `crates/states/wallet/src/model.rs`
  - `WalletConfig`
  - `WalletSnapshot`
  - `WalletCapabilities`
  - `ResolvedWallet`
  - `WalletImplementationConfig`
  - `WalletSignerConfig`
- `crates/states/symbol/src/model.rs`
  - `QuoteCode`
  - `SymbolKind`
  - `SymbolRole`
  - `SymbolConfig`
  - `ValuationSourceRegistry`
  - `ValuationSourceConfig`
  - `BalanceReaderConfig`
  - `SymbolValuationConfig`
  - `QuoteValuationConfig`
  - `PriceSourceRef`
  - `ValuationSourceReaderConfig`
  - `ValuationReaderConfig`
  - `ResolvedSymbolBalanceReader`
  - `ResolvedSymbolValuationReader`
  - `Observation`
  - `ObservationQuantity`
  - `ObservationValue`
  - `ObservationValueSourceRef`
  - `ObservationSource`
  - `PortfolioSnapshotError`

### Core enums

```rust
pub enum QuoteCode {
    USD,
    BTC,
}

pub enum SymbolKind {
    NativeBalance,
    Erc20Balance,
    ProtocolPosition,
    StakedPosition,
}

pub enum SymbolRole {
    Native,
    Asset,
    Collateral,
    Debt,
    Staked,
}
```

### Canonical request / config types

`PortfolioConfig` remains the canonical portfolio-owned config surface.
Valuation source resolution uses a sibling registry surface that is loaded by a dedicated config state and validated alongside the portfolio config.

```rust
pub struct PortfolioConfig {
    pub portfolio_id: String,
    pub quote_codes: Vec<QuoteCode>,
    pub networks: Vec<NetworkConfig>,
    pub wallets: Vec<WalletConfig>,
    pub symbol_configs: Vec<SymbolConfig>,
    pub metadata: std::collections::BTreeMap<String, serde_json::Value>,
}

pub struct ValuationSourceRegistry {
    pub sources: Vec<ValuationSourceConfig>,
}

pub struct NetworkConfig {
    pub network_id: String,
    pub chain_id: u64,
    pub rpc_source_id: Option<String>,
    pub metadata: std::collections::BTreeMap<String, serde_json::Value>,
}

pub struct WalletConfig {
    pub wallet_id: String,
    pub address: String,
    pub network_id: String,
    pub implementation: WalletImplementationConfig,
    pub symbol_ids: Vec<String>,
    pub metadata: std::collections::BTreeMap<String, serde_json::Value>,
}
```

### Canonical wallet implementation types

```rust
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WalletImplementationConfig {
    AddressOnly {},
    KeystoreEntry {
        entry_id: String,
    },
    NodeManagedAccount {
        account_index: usize,
    },
    ExternalSigner {
        signer_id: String,
    },
}

pub struct WalletSignerConfig {
    pub signer_kind: String,
    pub signer_ref: String,
}

pub struct WalletCapabilities {
    pub can_resolve_address: bool,
    pub can_sign: bool,
    pub can_submit: bool,
}

pub struct ResolvedWallet {
    pub wallet_id: String,
    pub address: String,
    pub network_id: String,
    pub implementation_kind: String,
    pub capabilities: WalletCapabilities,
    pub signer: Option<WalletSignerConfig>,
}
```

### Canonical symbol config type

```rust
pub struct SymbolConfig {
    pub symbol_id: String,
    pub display_symbol: Option<String>,
    pub kind: SymbolKind,
    pub role: SymbolRole,
    pub network_id: String,
    pub protocol: Option<String>,
    pub balance_reader: BalanceReaderConfig,
    pub valuation: SymbolValuationConfig,
    pub decimals: Option<u8>,
    pub underlying_symbol_id: Option<String>,
    pub metadata: std::collections::BTreeMap<String, serde_json::Value>,
}
```

### Canonical reader config types

```rust
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BalanceReaderConfig {
    NativeBalance {},
    Erc20Balance {
        token_address: String,
    },
    ProtocolPosition {
        protocol: String,
        reader: String,
        config: std::collections::BTreeMap<String, serde_json::Value>,
    },
}

pub struct SymbolValuationConfig {
    pub quotes: Vec<QuoteValuationConfig>,
}

pub struct QuoteValuationConfig {
    pub quote: QuoteCode,
    pub priced_symbol_id: String,
    pub reader: ValuationReaderConfig,
}

pub struct PriceSourceRef {
    pub source_id: String,
    pub network_id: String,
    pub base_symbol_id: String,
    pub quote: QuoteCode,
}

pub struct ValuationSourceConfig {
    pub source_id: String,
    pub network_id: String,
    pub base_symbol_id: String,
    pub quote: QuoteCode,
    pub reader: ValuationSourceReaderConfig,
    pub metadata: std::collections::BTreeMap<String, serde_json::Value>,
}

pub struct ResolvedSymbolBalanceReader {
    pub kind: String,
    pub implementation_ref: String,
}

pub struct ResolvedSymbolValuationReader {
    pub quote: QuoteCode,
    pub kind: String,
    pub implementation_ref: String,
}

#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ValuationSourceReaderConfig {
    EvmOracle {
        oracle_kind: String,
        config: std::collections::BTreeMap<String, serde_json::Value>,
    },
}

#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ValuationReaderConfig {
    FixedUnitPrice {
        unit_price_dec: String,
    },
    DirectPrice {
        source: PriceSourceRef,
    },
    DerivedUnitPrice {
        numerator: PriceSourceRef,
        denominator: PriceSourceRef,
    },
}
```

### Canonical snapshot / artifact types

```rust
pub struct PortfolioSnapshot {
    pub portfolio_id: String,
    pub generated_at_ms: u64,
    pub network_pins: Vec<NetworkPin>,
    pub wallets: Vec<WalletSnapshot>,
    pub symbol_configs: Vec<SymbolConfig>,
    pub errors: Vec<PortfolioSnapshotError>,
}

pub struct NetworkPin {
    pub network_id: String,
    pub chain_id: u64,
    pub block_number: u64,
}

pub struct WalletSnapshot {
    pub wallet_id: String,
    pub address: String,
    pub network_id: String,
    pub observations: Vec<Observation>,
}
```

### Canonical observation / value types

```rust
pub struct Observation {
    pub wallet_id: String,
    pub symbol_id: String,
    pub display_symbol: Option<String>,
    pub kind: SymbolKind,
    pub role: SymbolRole,
    pub network_id: String,
    pub protocol: Option<String>,
    pub quantity: ObservationQuantity,
    pub values: Vec<ObservationValue>,
    pub source: ObservationSource,
    pub metadata: std::collections::BTreeMap<String, serde_json::Value>,
}

pub struct ObservationQuantity {
    pub raw_dec: String,
    pub decimals: u8,
    pub amount_dec: String,
}

pub struct ObservationValue {
    pub quote: QuoteCode,
    pub priced_symbol_id: String,
    pub value_dec: String,
    pub unit_price_dec: String,
    pub valuation_reader_kind: String,
    pub source_refs: Vec<ObservationValueSourceRef>,
}

pub struct ObservationValueSourceRef {
    pub source_id: String,
    pub network_id: String,
    pub block_number: u64,
}

pub struct ObservationSource {
    pub balance_reader_kind: String,
    pub network_id: String,
    pub block_number: u64,
}

pub struct PortfolioSnapshotError {
    pub code: String,
    pub message: String,
    pub wallet_id: Option<String>,
    pub symbol_id: Option<String>,
    pub network_id: Option<String>,
    pub reader_kind: Option<String>,
}
```

### Canonical report types

The snapshot artifact is the source of truth.
The report should be a derived, easier-to-consume summary.

```rust
pub struct PortfolioReport {
    pub portfolio_id: String,
    pub generated_at_ms: u64,
    pub network_pins: Vec<NetworkPin>,
    pub wallet_summaries: Vec<WalletReport>,
    pub totals_by_quote: Vec<PortfolioQuoteTotal>,
    pub error_count: u64,
}

pub struct WalletReport {
    pub wallet_id: String,
    pub network_id: String,
    pub totals_by_quote: Vec<PortfolioQuoteTotal>,
}

pub struct PortfolioQuoteTotal {
    pub quote: QuoteCode,
    pub assets_value_dec: String,
    pub collateral_value_dec: String,
    pub debt_value_dec: String,
    pub staked_value_dec: String,
    pub net_value_dec: String,
}
```

### Exact recommended JSON shape for `PortfolioConfig`

```json
{
  "portfolio_id": "portfolio_main",
  "quote_codes": ["USD", "BTC"],
  "networks": [
    {
      "network_id": "ethereum-mainnet",
      "chain_id": 1,
      "rpc_source_id": "mainnet_primary",
      "metadata": {}
    },
    {
      "network_id": "arbitrum-mainnet",
      "chain_id": 42161,
      "rpc_source_id": "arbitrum_primary",
      "metadata": {}
    }
  ],
  "wallets": [
    {
      "wallet_id": "wallet_treasury_eth",
      "address": "0x000000000000000000000000000000000000dead",
      "implementation": {
        "kind": "address_only"
      },
      "network_id": "ethereum-mainnet",
      "symbol_ids": [
        "eth.native.ethereum-mainnet",
        "usdc.wallet.ethereum-mainnet"
      ],
      "metadata": {}
    },
    {
      "wallet_id": "wallet_ops_arb",
      "address": "0x000000000000000000000000000000000000beef",
      "implementation": {
        "kind": "address_only"
      },
      "network_id": "arbitrum-mainnet",
      "symbol_ids": [
        "eth.native.arbitrum-mainnet"
      ],
      "metadata": {}
    }
  ],
  "symbol_configs": [
    {
      "symbol_id": "eth.native.ethereum-mainnet",
      "display_symbol": "ETH",
      "kind": "native_balance",
      "role": "native",
      "network_id": "ethereum-mainnet",
      "protocol": null,
      "balance_reader": {
        "kind": "native_balance"
      },
      "valuation": {
        "quotes": [
          {
            "quote": "USD",
            "priced_symbol_id": "eth.native.ethereum-mainnet",
            "reader": {
              "kind": "direct_price",
              "source": {
                "source_id": "chainlink_eth_usd",
                "network_id": "ethereum-mainnet",
                "base_symbol_id": "eth.native.ethereum-mainnet",
                "quote": "USD"
              }
            }
          },
          {
            "quote": "BTC",
            "priced_symbol_id": "eth.native.ethereum-mainnet",
            "reader": {
              "kind": "direct_price",
              "source": {
                "source_id": "chainlink_eth_btc",
                "network_id": "ethereum-mainnet",
                "base_symbol_id": "eth.native.ethereum-mainnet",
                "quote": "BTC"
              }
            }
          }
        ]
      },
      "decimals": 18,
      "underlying_symbol_id": null,
      "metadata": {}
    },
    {
      "symbol_id": "usdc.wallet.ethereum-mainnet",
      "display_symbol": "USDC",
      "kind": "erc20_balance",
      "role": "asset",
      "network_id": "ethereum-mainnet",
      "protocol": null,
      "balance_reader": {
        "kind": "erc20_balance",
        "token_address": "0x0000000000000000000000000000000000000001"
      },
      "valuation": {
        "quotes": [
          {
            "quote": "USD",
            "priced_symbol_id": "usdc.wallet.ethereum-mainnet",
            "reader": {
              "kind": "direct_price",
              "source": {
                "source_id": "chainlink_usdc_usd",
                "network_id": "ethereum-mainnet",
                "base_symbol_id": "usdc.wallet.ethereum-mainnet",
                "quote": "USD"
              }
            }
          },
          {
            "quote": "BTC",
            "priced_symbol_id": "usdc.wallet.ethereum-mainnet",
            "reader": {
              "kind": "derived_unit_price",
              "numerator": {
                "source_id": "chainlink_usdc_usd",
                "network_id": "ethereum-mainnet",
                "base_symbol_id": "usdc.wallet.ethereum-mainnet",
                "quote": "USD"
              },
              "denominator": {
                "source_id": "chainlink_btc_usd",
                "network_id": "ethereum-mainnet",
                "base_symbol_id": "btc.wallet.ethereum-mainnet",
                "quote": "USD"
              }
            }
          }
        ]
      },
      "decimals": 6,
      "underlying_symbol_id": null,
      "metadata": {}
    },
    {
      "symbol_id": "eth.native.arbitrum-mainnet",
      "display_symbol": "ETH",
      "kind": "native_balance",
      "role": "native",
      "network_id": "arbitrum-mainnet",
      "protocol": null,
      "balance_reader": {
        "kind": "native_balance"
      },
      "valuation": {
        "quotes": [
          {
            "quote": "USD",
            "priced_symbol_id": "eth.native.arbitrum-mainnet",
            "reader": {
              "kind": "direct_price",
              "source": {
                "source_id": "chainlink_eth_usd",
                "network_id": "arbitrum-mainnet",
                "base_symbol_id": "eth.native.arbitrum-mainnet",
                "quote": "USD"
              }
            }
          },
          {
            "quote": "BTC",
            "priced_symbol_id": "eth.native.arbitrum-mainnet",
            "reader": {
              "kind": "direct_price",
              "source": {
                "source_id": "chainlink_eth_btc",
                "network_id": "arbitrum-mainnet",
                "base_symbol_id": "eth.native.arbitrum-mainnet",
                "quote": "BTC"
              }
            }
          }
        ]
      },
      "decimals": 18,
      "underlying_symbol_id": null,
      "metadata": {}
    }
  ],
  "metadata": {}
}
```

### Exact recommended JSON shape for `Observation`

```json
{
  "wallet_id": "wallet_treasury_eth",
  "symbol_id": "aave_v3.usdc.variable_debt.ethereum-mainnet",
  "display_symbol": "USDC",
  "kind": "protocol_position",
  "role": "debt",
  "network_id": "ethereum-mainnet",
  "protocol": "aave_v3",
  "quantity": {
    "raw_dec": "10000000",
    "decimals": 6,
    "amount_dec": "10.000000"
  },
  "values": [
    {
      "quote": "USD",
      "priced_symbol_id": "usdc.wallet.ethereum-mainnet",
      "value_dec": "10.000000",
      "unit_price_dec": "1.000000",
      "valuation_reader_kind": "direct_price",
      "source_refs": [
        {
          "source_id": "chainlink_usdc_usd",
          "network_id": "ethereum-mainnet",
          "block_number": 12345678
        }
      ]
    },
    {
      "quote": "BTC",
      "priced_symbol_id": "usdc.wallet.ethereum-mainnet",
      "value_dec": "0.00010000",
      "unit_price_dec": "0.00001000",
      "valuation_reader_kind": "derived_unit_price",
      "source_refs": [
        {
          "source_id": "chainlink_usdc_usd",
          "network_id": "ethereum-mainnet",
          "block_number": 12345678
        },
        {
          "source_id": "chainlink_btc_usd",
          "network_id": "ethereum-mainnet",
          "block_number": 12345678
        }
      ]
    }
  ],
  "source": {
    "balance_reader_kind": "protocol_position:aave_v3:debt_position",
    "network_id": "ethereum-mainnet",
    "block_number": 12345678
  },
  "metadata": {
    "debt_kind": "variable"
  }
}
```

### Validation rules for the canonical types

At minimum, validation should enforce:

- `portfolio_id`, `wallet_id`, `symbol_id`, and `network_id` are non-empty
- `wallet_id`s are unique
- `symbol_id`s are unique
- `network_id`s are unique
- every `wallet.network_id` exists in `networks`
- every `wallet.symbol_id` exists in `symbol_configs`
- every `symbol.network_id` exists in `networks`
- `quote_codes` are unique
- `symbol.valuation.quotes[].quote` are unique per symbol
- every `symbol.valuation.quotes[].priced_symbol_id` exists in `symbol_configs`
- every symbol defines exactly one valuation route for each portfolio-level `quote_code`
- `underlying_symbol_id`, when present, must refer to an existing symbol
- all addresses are valid normalized EVM addresses when the reader requires them
- all decimals are integers and never floats
- all protocol reader `config` objects are canonical JSON objects with no floats
- all `metadata` objects are canonical JSON objects with no floats
- direct price sources reference configured `network_id`s
- derived price sources use the same quote unit in numerator and denominator
- no metadata or protocol config may contain secrets

### Deterministic ordering rules

For stable hashing and replay, sort these before persistence:

- `networks` by `network_id`
- `wallets` by `wallet_id`
- `wallet.symbol_ids` lexicographically
- `symbol_configs` by `symbol_id`
- `quote_codes` by quote code
- `observations` by `(wallet_id, symbol_id)`
- `values` by quote code
- `errors` by `(network_id, wallet_id, symbol_id, code)`

## Proposed Observation Model

Every read should normalize into the exact `Observation` shape defined above.
There is no separate alternate observation envelope.

This gives us one pipeline for:

- reading
- valuing
- sorting
- serialization
- aggregation
- reporting

## Proposed Snapshot Shape

The canonical snapshot artifact should use the exact `PortfolioSnapshot` shape defined above.
An aligned JSON example is:

```json
{
  "portfolio_id": "portfolio_x",
  "generated_at_ms": 1234567890,
  "network_pins": [
    {
      "network_id": "arbitrum-mainnet",
      "chain_id": 42161,
      "block_number": 230000000
    },
    {
      "network_id": "ethereum-mainnet",
      "chain_id": 1,
      "block_number": 12345678
    }
  ],
  "wallets": [
    {
      "wallet_id": "wallet_a",
      "address": "0x...",
      "network_id": "ethereum-mainnet",
      "observations": [
        {
          "symbol_id": "eth.native.ethereum-mainnet",
          "display_symbol": "ETH",
          "kind": "native_balance",
          "role": "native",
          "network_id": "ethereum-mainnet",
          "protocol": null,
          "quantity": {
            "raw_dec": "1000000000000000000",
            "decimals": 18,
            "amount_dec": "1.000000000000000000"
          },
          "values": [
            {
              "quote": "USD",
              "priced_symbol_id": "eth.native.ethereum-mainnet",
              "value_dec": "3500.00000000",
              "unit_price_dec": "3500.00000000",
              "valuation_reader_kind": "direct_price",
              "source_refs": [
                {
                  "source_id": "chainlink_eth_usd",
                  "network_id": "ethereum-mainnet",
                  "block_number": 12345678
                }
              ]
            }
          ],
          "source": {
            "balance_reader_kind": "native_balance",
            "network_id": "ethereum-mainnet",
            "block_number": 12345678
          },
          "metadata": {}
        }
      ]
    }
  ],
  "symbol_configs": [
    { "...": "..." }
  ],
  "errors": []
}
```

## Why per-network pins matter

The user model requires that one portfolio can include, from the first implementation:

- multiple wallets
- multiple networks

That means a single global:

- `chain_id`
- `block_number`

is no longer sufficient.

We need pinned block context per network, for example:

- Ethereum mainnet pin
- Arbitrum pin
- Base pin

This also lets multiple wallets on the same network share one deterministic block pin.

### Valuation pins also need determinism

If value is part of the snapshot, its sourcing must also be deterministic.

That means the snapshot should record enough information to replay valuation, for example:

- quote units requested
- pricing source kind
- pricing source identifiers
- network / block pins for every direct source reference used in valuation

For the first implementation, valuation sources should stay within the pinned network model.
If a later source family cannot be expressed this way, it should not be added until its replay contract is fully specified.

## Aave V3 Requirements

### What we need beyond wallet balances

For Aave V3 portfolio support, the snapshot system needs to capture at least:

- supplied assets
- collateral-enabled assets
- borrowed assets
- debt mode where relevant
- staked assets or rewards positions, depending on scope

### Why aToken / debt token `balanceOf` is not enough

Reading token balances alone is useful, but incomplete.

It may tell us:

- aToken amount
- debt token amount

It does not necessarily tell us everything we need for a portfolio model, such as:

- whether supply is enabled as collateral
- richer account-level protocol state
- market-specific protocol metadata
- health-factor-style summary fields

So we likely need Aave-specific read states, not just more `TokenBalanceState`s.

### Suggested Aave-specific shared states

These should live in `crates/states/aave-v3` as reusable state-layer building blocks.

In the canonical portfolio model, these are selected through:

- `balance_reader.kind = "protocol_position"`
- `balance_reader.protocol = "aave_v3"`
- `balance_reader.reader = <aave reader name>`
- `balance_reader.config = <aave-specific canonical JSON object>`

Possible additions:

- `LoadAaveMarketConfigState`
- `PinAaveNetworkBlockState` or reuse common block pinning
- `ReadAaveUserAccountSummaryState`
- `ReadAaveReservePositionState`
- `ReadAaveCollateralFlagsState`
- `ReadAaveDebtPositionState`
- `ReadAaveStakingPositionState`
- `CollectAavePortfolioObservationsState`

The planner op should stay thin and assemble these states, consistent with the repo architecture.

### Aave market config we will need

Per network / market, we likely need config for:

- `market_id`
- `chain_id`
- `pool`
- reserve list
- reserve token addresses
- aToken addresses
- debt token addresses
- staking / incentives contract addresses where relevant

This config should be explicit and replayable, not discovered through ambient IO in ad hoc ways.

### Aave valuation implications

Aave positions still need to value in quote units like BTC and USD.

That means:

- supplied positions should resolve to underlying exposure
- collateral positions should value against the underlying asset
- debt positions should value against the borrowed asset
- staking / rewards positions should have explicit quote handling

The valuation workflow should not treat Aave positions as special one-off report logic.
They should use the same symbol-driven valuation path as every other symbol.

In other words:

- the portfolio crate owns the canonical observation and valuation model
- the Aave crate owns the typed meaning of `protocol = "aave_v3"` plus the concrete `reader` names and config validation
- the op wires the two together without hard-coding Aave-specific reporting logic into the generic portfolio model

## Workflow Direction

### Current workflow

Current `portfolio_tracker` is effectively:

1. pin chain
2. pin block
3. read native balance
4. read ERC-20 balances
5. write snapshot
6. write report

### Proposed workflow

A more general workflow should look like:

1. load portfolio config
2. load valuation source registry
3. validate wallet, symbol, network, valuation route, and valuation source refs
4. compute the required network set from wallets plus valuation source refs
5. pin block per network
6. dispatch balance readers by `symbol_config.balance_reader.kind`
7. dispatch protocol readers by `(protocol, reader)` where `kind = protocol_position`
8. resolve direct `source_id` refs through the valuation source registry
9. execute each unique direct price read once per pinned source network
10. derive composite unit prices from direct price reads
11. normalize each result into the canonical observation shape
12. aggregate observations per wallet
13. aggregate observations per portfolio
14. write artifact
15. write report

Reader dispatch examples:

- `native_balance`
- `erc20_balance`
- `protocol_position` with `protocol = "aave_v3"` and `reader = "reserve_position"`
- `protocol_position` with `protocol = "aave_v3"` and `reader = "debt_position"`
- `protocol_position` with `protocol = "aave_v3"` and `reader = "staked_position"`
- `direct_price`
- `derived_unit_price`

## Typed Report Direction

The new `PortfolioSnapshot` report should be redesigned from scratch.

We do not need to preserve the current report shape.

Report direction:

- report portfolio metadata
- report network pins
- report wallets
- report normalized observations
- report values in requested quote units
- optionally include summaries by role:
  - assets
  - collateral
  - debt
  - staked
- optionally include summaries by quote:
  - total USD
  - total BTC
  - per-wallet USD
  - per-wallet BTC

## Error Handling Direction

The snapshot artifact already has `errors: []`, but the current implementation behaves like fail-fast.

For a real portfolio system, we should decide explicitly between:

- fail-fast snapshot
- best-effort snapshot with partial errors

Best-effort is likely more practical for:

- many wallets
- many tokens
- many protocol reads

If we choose best-effort, each error should be attached to:

- wallet
- symbol
- network
- reader kind
- exact error code

## Scaling Concerns

The current state graph chains reads sequentially.

That is acceptable for:

- one wallet
- a few tokens

It will become expensive for:

- many wallets
- many symbols
- Aave reserve-by-reserve reads

So the revamp should keep an eye on:

- batching
- multicall-style reads
- avoiding one-state-per-call explosion
- avoiding one-price-read-per-symbol explosion
- deduplicating direct source reads across symbols and wallets that share the same valuation source

This matters especially for Aave V3 positions where a portfolio may span many reserves.

## Proposed Phased Plan

### Phase 0: define the new canonical model

- freeze the exact canonical `PortfolioSnapshot` schema in this document
- keep the break decision in place instead of introducing a compatibility layer
- define `portfolio`, `wallet`, `symbol_config`, `observation`, and valuation shapes as one coherent model

### Phase 1: implement the base multi-network wallet + valuation slice

- add `portfolio_id`
- add multi-network config and per-network pins
- add multi-wallet config
- add symbol registry / symbol config map
- add valuation source registry / source-id resolution
- add quote support for at least `USD` and `BTC`
- require one valuation route per symbol for every portfolio-level quote
- validate every `PriceSourceRef` against the valuation source registry
- ship fixed-price and source-registry plumbing before any live oracle family is required
- normalize all reads and values into a common observation shape

### Phase 2: add Aave V3 portfolio reads

- add Aave market config
- add Aave-specific shared read states
- add collateral and debt observations
- add Aave valuation support through the same symbol-driven path
- write Aave observations into the artifact

### Phase 3: add staked assets

- define exactly what "staked" means in scope
- add reader states for those contracts / positions
- normalize them into the same observation shape
- add quote valuation for those positions

### Phase 4: aggregation and risk summaries

- net exposure
- quote totals by wallet / portfolio
- health metrics / aggregate risk summaries where protocol data supports them

## Recommended Decisions

- Keep `portfolio_tracker` thin; put reusable runtime behavior in shared states.
- Replace the current portfolio snapshot surface in place on the dev branch.
- Use `symbol_id` as the canonical machine identity, not raw `symbol`.
- Model native, wallet, collateral, debt, and staked positions with one normalized observation shape.
- Keep multi-network semantics in the base model and first implementation.
- Make valuation a core responsibility of symbol config, not a separate optional afterthought.
- Use explicit unit-price routes with `direct_price` and `derived_unit_price`.
- Make valuation-source resolution config-driven, not heuristic discovery.
- Keep valuation `source_id` separate from network `rpc_source_id`.
- Configure quotes at the portfolio level and require every symbol to satisfy them.
- Pin blocks per network, not globally.
- Treat Aave V3 as protocol-backed symbol configs, not as a separate portfolio system.
- Represent debt as a positive quantity with `role = debt`; netting happens only in derived summaries.
- Allow the base runtime to return structured `unsupported_valuation_source_kind` errors until live source families are implemented.

## Open Questions

- What exactly counts as "staked assets"?
- Are Aave rewards in scope, or only principal positions?
- Do we need a future preflight discovery tool that materializes explicit portfolio and valuation-source config, or is hand-authored config sufficient?
- Do we want best-effort snapshots, or strict fail-fast semantics?
- Which concrete `ValuationSourceReaderConfig.kind` ships first for live `USD` and `BTC` valuation?
- Do we want health-factor-style protocol summaries in the base Aave slice, or only raw observations first?

## Immediate Next Steps

1. Mirror the canonical schema and valuation source registry contract in code without adding a compatibility layer for the old surface.
2. Add validators for multi-network refs, portfolio-level quote coverage, valuation source routes, and registry/source-ref agreement.
3. Implement the base multi-network wallet/native/ERC-20/valuation slice with `fixed_unit_price`, registry plumbing, and structured unsupported-source errors.
4. Add the first reusable live valuation source state family under `crates/evm-runtime/src/states/price.rs`.
5. Add Aave V3 shared read states in `crates/states/aave-v3` using the `protocol_position` reader envelope.
6. Keep the planner op thin and assemble native / ERC-20 / protocol readers through one normalized workflow.

## Next Time

- Start mapping the exact canonical schema in this document into `crates/states/portfolio`, `crates/states/wallet`, and `crates/states/symbol`.
- Decide whether Aave V3 position reads should use generic ABI-decoded calls or dedicated Aave-specific states.
- Resolve the first concrete live valuation source reader kind and initial registry entries for `USD` and `BTC` valuation.
- Resolve the meaning of "staked assets" before implementing the Aave slice.

## Appendix A: Concrete Implementation Plan

This appendix turns the design direction above into a concrete repo rollout plan.

The implementation strategy is:

- land one commit-oriented milestone at a time
- keep runtime behavior small and testable at each step
- delay Aave-specific reads until the base multi-network portfolio flow is stable
- update CLI / app / REST / Nix surfaces in the same milestone that switches the runtime contract
- commit the work directly on the current branch instead of planning separate review batches

### Milestone 1: freeze the schema in code

Goal:

- mirror the canonical model in code without changing the runtime execution path yet

Primary files and crates:

- `Cargo.toml`
- `crates/states/portfolio/`
- `crates/states/wallet/`
- `crates/states/symbol/`

Tasks:

- add workspace members for:
  - `crates/states/portfolio`
  - `crates/states/wallet`
  - `crates/states/symbol`
- add model-only crates first:
  - serde types
  - rustdoc
  - validation helpers
  - deterministic sorting / normalization helpers
- keep executable runtime logic out of these crates for this milestone
- add schema-focused tests for:
  - valid canonical config decoding
  - invalid ref detection
  - no-float validation on metadata and protocol config blobs
  - deterministic ordering normalization

Acceptance criteria:

- workspace builds with the new crates added
- canonical model types exist in code
- validation rules from this document are covered by tests
- no app / CLI / runtime behavior changes yet

### Milestone 2: add the base reusable runtime slice

Goal:

- implement the minimum reusable runtime needed for the first real portfolio run

Scope for this milestone:

- wallet implementations:
  - `address_only` only
- balance readers:
  - `native_balance`
  - `erc20_balance`
- valuation readers:
  - `direct_price`
  - `derived_unit_price`
- networks:
  - multi-network from the start

Primary files and crates:

- `crates/evm-runtime/src/states/read.rs`
- optionally new modules under `crates/evm-runtime/src/states/`
- `crates/states/portfolio/src/*`
- `crates/states/wallet/src/*`
- `crates/states/symbol/src/*`

Tasks:

- add reusable network-pinning states for per-network block pins
- add reusable direct price read states in `evm-runtime`
- keep `derived_unit_price` pure when possible
- implement wallet resolution states for `address_only`
- implement symbol dispatch and normalization states for:
  - native balances
  - ERC-20 balances
  - valuation application
- write canonical `Observation` and `PortfolioSnapshot` assembly states

Acceptance criteria:

- a single run can read multiple wallets across multiple networks
- all observations use the canonical shape
- all values use portfolio-level quote requirements
- valuation source refs are recorded deterministically against pinned blocks

### Milestone 3: replace the planner op in place

Goal:

- switch `portfolio_tracker` from the old single-wallet slice to the new canonical flow

Primary files and crates:

- `crates/ops/portfolio-tracker-op/src/lib.rs`
- `crates/ops/portfolio-tracker-op/src/tests/portfolio_tracker_op_tests.rs`

Tasks:

- replace the current op config parsing with canonical `PortfolioConfig`
- replace the linear:
  - chain id
  - block
  - native
  - token
  - report
  flow with the canonical multi-network planner
- keep the op thin:
  - config validation
  - graph wiring
  - no domain execution logic in the op
- keep snapshot and report writers either:
  - in shared portfolio state crates
  - or as narrow op-local output exceptions only if still justified

Acceptance criteria:

- `portfolio_tracker` runs the new canonical flow
- old request / response / artifact shape is gone
- deterministic ordering is covered by tests
- replay / resume coverage exists for the new portfolio flow

### Milestone 4: switch app, CLI, REST, and Nix surfaces

Goal:

- move all transport and workflow entrypoints to the new in-place contract

Primary files:

- `crates/app/src/lib.rs`
- `bin/cli/src/commands/portfolio/snapshot.rs`
- `bin/cli/README.md`
- `bin/rest-api/README.md`
- `docs/ops-and-states.md`
- `docs/architecture.md`
- `nixfied/project/module.nix`

Tasks:

- replace `PortfolioSnapshotRequest` and `PortfolioSnapshotResponse`
- update feature schemas in `crates/app`
- update CLI args:
  - stop assuming `<ADDRESS>` plus `--chain-id` plus `--tokens-json`
  - accept canonical config input instead
- update REST examples and feature docs
- update Nix task wiring and CI assumptions to use the new request shape

Acceptance criteria:

- CLI, app, REST, and Nix all invoke the same canonical contract
- docs reflect the breaking change in place
- no compatibility shim for the old surface remains

### Milestone 5: add Aave V3 `protocol_position`

Goal:

- add the first protocol-backed symbol implementation on top of the stable base flow

Primary files and crates:

- `crates/states/aave-v3/src/portfolio/*`
- `crates/states/aave-v3/src/lib.rs`
- `crates/states/aave-v3/src/states.rs`
- portfolio op tests and integration tests

Tasks:

- define canonical Aave market config types
- implement Aave `protocol_position` readers for:
  - reserve positions
  - debt positions
  - optional staking positions depending on scoped meaning
- normalize Aave reads into generic `Observation`s
- route valuation through `underlying_symbol_id`
- add protocol-specific validation for Aave config blobs

Acceptance criteria:

- Aave observations appear in the same canonical artifact shape as wallet balances
- no Aave-specific report-only branch exists outside the Aave crate
- Aave valuation uses the same generic quote path as other symbols

### Milestone 6: summaries and hardening

Goal:

- add derived reporting, operational hardening, and final parity coverage

Primary files:

- `crates/states/portfolio/src/*`
- `crates/app/src/lib.rs`
- integration tests
- docs

Tasks:

- add wallet and portfolio quote totals
- add net exposure summaries
- decide and implement fail-fast vs best-effort behavior
- add integration tests for:
  - multi-network runs
  - replay determinism
  - crash / resume
  - valuation determinism
  - CLI / REST contract behavior
- update docs and inventories fully

Acceptance criteria:

- summary reporting is stable and tested
- replay / resume guarantees are explicitly covered for the new portfolio flow
- docs and code inventory are in sync

### Recommended execution order

The recommended order is:

1. Milestone 1
2. Milestone 2
3. Milestone 3
4. Milestone 4
5. Milestone 5
6. Milestone 6

This order matters because:

- Milestone 1 freezes the schema before runtime churn starts
- Milestone 2 proves the base model without protocol complexity
- Milestone 3 switches the runtime core once the base pieces exist
- Milestone 4 moves public entrypoints only after the runtime is ready
- Milestone 5 adds Aave on top of a stable generic path
- Milestone 6 is where aggregation and operational hardening belong

### First implementation cut

To keep momentum and review size under control, the first fully runnable implementation should be:

- multi-network
- multi-wallet
- address-only wallets
- native balances
- ERC-20 balances
- direct USD pricing
- derived BTC pricing
- canonical artifact and report

Explicitly not required for the first runnable cut:

- keystore-backed wallet execution
- node-managed or external signer execution
- Aave reads
- staked assets
- risk summaries
- discovery

That first cut is enough to validate:

- the canonical schema
- multi-network semantics
- valuation semantics
- thin-op boundaries
- transport integration
