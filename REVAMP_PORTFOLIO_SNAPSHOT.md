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

- new portfolio-domain shared state crate for the canonical portfolio model and workflow
- Aave-specific portfolio readers in the Aave shared state crate
- protocol-agnostic EVM primitives in `evm-runtime`
- thin planning / transport layers on top

### 1. New shared portfolio crate

Preferred new crate:

- `crates/states/portfolio/`

This crate should become the home for the new canonical `PortfolioSnapshot` model and the reusable portfolio runtime states.

Suggested contents:

- `crates/states/portfolio/src/lib.rs`
  - public module surface
- `crates/states/portfolio/src/model.rs`
  - `PortfolioSnapshot`
  - `PortfolioConfig`
  - `WalletConfig`
  - `SymbolConfig`
  - `Observation`
  - quote / valuation types
- `crates/states/portfolio/src/portfolio/mod.rs`
- `crates/states/portfolio/src/portfolio/states.rs`
  - portfolio-level aggregation states
- `crates/states/portfolio/src/portfolio/writers.rs`
  - portfolio-level writers
- `crates/states/portfolio/src/valuation.rs`
  - quote configuration
  - valuation result types
  - valuation normalization helpers
- `crates/states/portfolio/src/wallet/mod.rs`
- `crates/states/portfolio/src/wallet/states.rs`
  - wallet-level orchestration states
  - wallet-level readers / aggregation helpers
- `crates/states/portfolio/src/wallet/writers.rs`
  - wallet-level output shaping / writing helpers
- `crates/states/portfolio/src/symbol/mod.rs`
- `crates/states/portfolio/src/symbol/states.rs`
  - symbol-level orchestration states
  - dispatch balance readers
  - dispatch valuation readers
  - normalize symbol observations
- `crates/states/portfolio/src/symbol/readers.rs`
  - symbol-specific reader dispatch glue
- `crates/states/portfolio/src/symbol/writers.rs`
  - symbol-level output shaping / persistence helpers

Why:

- the portfolio model is now large enough to deserve its own reusable domain boundary
- this keeps the op thin, which matches the repo architecture
- the app, CLI, and API can all depend on one canonical portfolio library surface

### Wallet and symbol need separate state families

Wallet and symbol should not be treated as one flat pool of states.

They are distinct workflow layers:

- portfolio layer
  - owns aggregation across wallets
  - owns final snapshot / report writers
- wallet layer
  - owns wallet-scoped orchestration
  - owns wallet network association and wallet-scoped aggregation
  - can grow wallet-specific readers / writers over time
- symbol layer
  - owns symbol-scoped balance readers
  - owns symbol-scoped valuation readers
  - owns observation normalization for native / ERC-20 / protocol / debt / staked symbols

This split matters because both wallet and symbol will grow:

- wallets may eventually need their own metadata, pinning, filters, partial errors, and wallet-level summaries
- symbols will definitely need their own balance and valuation logic

So the implementation should reflect that growth early instead of collapsing everything into one generic `states.rs`.

### Concrete initial file skeleton

If we want a concrete starting skeleton, I would begin with this:

```text
crates/states/portfolio/
  src/
    lib.rs
    model.rs
    valuation.rs
    portfolio/
      mod.rs
      states.rs
      writers.rs
    wallet/
      mod.rs
      model.rs
      states.rs
      readers.rs
      writers.rs
    symbol/
      mod.rs
      model.rs
      states.rs
      readers.rs
      writers.rs
```

Suggested first-pass state inventory:

- `crates/states/portfolio/src/portfolio/states.rs`
  - `LoadPortfolioConfigState`
  - `PinPortfolioNetworksState`
  - `CollectPortfolioObservationsState`
- `crates/states/portfolio/src/portfolio/writers.rs`
  - `WritePortfolioSnapshotState`
  - `WritePortfolioReportState`

- `crates/states/portfolio/src/wallet/states.rs`
  - `LoadWalletSetState`
  - `PrepareWalletExecutionState`
  - `CollectWalletObservationsState`
- `crates/states/portfolio/src/wallet/readers.rs`
  - `ReadWalletNativeBalanceState`
  - `ReadWalletMetadataState`
- `crates/states/portfolio/src/wallet/writers.rs`
  - `WriteWalletObservationSetState`

- `crates/states/portfolio/src/symbol/states.rs`
  - `ResolveSymbolConfigState`
  - `ReadSymbolBalanceState`
  - `ReadSymbolValuationState`
  - `NormalizeSymbolObservationState`
- `crates/states/portfolio/src/symbol/readers.rs`
  - `DispatchSymbolBalanceReaderState`
  - `DispatchSymbolValuationReaderState`
- `crates/states/portfolio/src/symbol/writers.rs`
  - `WriteSymbolObservationState`

This is intentionally split so:

- portfolio states do cross-wallet orchestration and final aggregation
- wallet states do wallet-scoped orchestration
- symbol states do symbol-scoped reading and normalization

### Concrete Aave module skeleton

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
  - Aave raw read -> canonical portfolio observation

### Concrete `evm-runtime` expansion points

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

### 2. Keep Aave portfolio reads in the Aave shared state crate

Preferred location:

- `crates/states/aave-v3/`

Do not put Aave-specific portfolio logic in the generic portfolio crate.
The generic portfolio crate should understand symbol-driven workflows, but not Aave internals.

Suggested additions:

- `crates/states/aave-v3/src/portfolio/mod.rs`
- `crates/states/aave-v3/src/portfolio/states.rs`
  - `AaveMarketConfig`
  - `ReadAaveUserAccountSummaryState`
  - `ReadAaveReservePositionState`
  - `ReadAaveDebtPositionState`
  - `ReadAaveStakingPositionState`
- `crates/states/aave-v3/src/portfolio/normalize.rs`
  - normalization helpers from raw Aave reads into portfolio observations or intermediate Aave position types

Why:

- Aave is a protocol-specific domain
- the repo already uses `crates/states/aave-v3/` as the Aave shared-state boundary
- deploy/configure logic and read/portfolio logic can live in the same crate while remaining separated by module

Important note:

- avoid continuing to grow `crates/states/aave-v3/src/states.rs` into one giant mixed-purpose file
- prefer adding a dedicated module such as `portfolio.rs` or `reads.rs`

### 3. Put generic chain primitives in `evm-runtime`

Preferred location:

- `crates/evm-runtime/src/states/read.rs`
- possibly additional modules under `crates/evm-runtime/src/states/`

This is where protocol-agnostic pieces belong, for example:

- richer ABI-decoded `eth_call`
- multicall / batched read support
- generic oracle price reads
- generic event / logs reads when needed
- reusable helpers for deterministic pinned-block chain reads

Why:

- these are not portfolio-specific
- these are not Aave-specific
- other ops and state crates should be able to reuse them

Rule of thumb:

- if it works for many protocols, it belongs in `evm-runtime`
- if it only makes sense for portfolio assembly, it belongs in `crates/states/portfolio`
- if it only makes sense for Aave, it belongs in `crates/states/aave-v3`

### 4. Keep the current portfolio op thin

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

### 5. Keep app / CLI / REST transport-only

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

### 6. Docs to update alongside implementation

When the implementation starts landing, update:

- `docs/ops-and-states.md`
- `docs/architecture.md`
- `docs/redesign.md`
- `bin/cli/README.md`
- `bin/rest-api/README.md`

### Summary recommendation

If I had to choose one concrete direction now:

- add `crates/states/portfolio/` as the new shared home for the canonical portfolio model
- extend `crates/states/aave-v3/` with portfolio-reading modules
- extend `crates/evm-runtime/` only for protocol-agnostic read / valuation primitives
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
  - mirror the pattern in `crates/states/portfolio/`
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
  - pattern should be reused in both `crates/states/portfolio/` and Aave portfolio modules
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
  - portfolio equivalents should live near the corresponding portfolio model loaders
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
  - strong candidate to generalize for write-path code
- current responsibility:
  - normalize sender
  - attach calldata
  - send EVM transaction
- recommended destination:
  - `crates/evm-runtime/` write helpers
- reason:
  - generic EVM transaction submission pattern
  - not portfolio-specific, but broadly reusable

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
- keep Aave portfolio reads as new code in the Aave crate
- keep portfolio orchestration as new code in the portfolio crate

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

This effort should not preserve compatibility with the current portfolio snapshot request, response, or internal config model.

This means:

- no `PortfolioSnapshotV2`
- no requirement to preserve the current single-wallet request shape
- no requirement to preserve the current typed report shape
- no requirement to preserve old portfolio snapshot config compatibility

The goal is to define the new canonical `PortfolioSnapshot`.

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

## Proposed Domain Model

### Portfolio

A portfolio is a logical grouping of wallets and symbol configs.

Suggested responsibilities:

- stable `portfolio_id`
- list of wallets
- shared symbol registry / config map
- optional portfolio-level metadata

### Wallet

A wallet is an address on a network that owns or participates in positions.

Suggested fields:

- `wallet_id`
- `address`
- `network`
- `tracked_symbols`

### Symbol config

A symbol config describes how a portfolio entry should be:

- read for balance / quantity
- valued in quote units such as BTC or USD
- interpreted semantically

Suggested shape:

```json
{
  "symbol_id": "aave_v3.usdc.collateral.mainnet",
  "display_symbol": "USDC",
  "kind": "protocol_position",
  "role": "collateral",
  "network": "ethereum-mainnet",
  "protocol": "aave_v3",
  "balance_reader": {
    "kind": "aave_v3_reserve_position",
    "market_id": "aave-v3-mainnet",
    "reserve_symbol": "USDC"
  },
  "valuation": {
    "quotes": {
      "USD": {
        "kind": "oracle_price",
        "source": "chainlink",
        "pair": "USDC/USD"
      },
      "BTC": {
        "kind": "derived_cross_quote",
        "via": ["USDC/USD", "BTC/USD"]
      }
    }
  },
  "decimals": 6,
  "underlying_symbol_id": "usdc.wallet.mainnet",
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
- `network`
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

Suggested scalar aliases:

- `PortfolioId = String`
- `WalletId = String`
- `SymbolId = String`
- `NetworkId = String`
- `MarketId = String`
- `DecimalString = String`
- `AddressString = String`

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

pub enum AaveDebtKind {
    Variable,
    Stable,
}
```

### Canonical request / config types

```rust
pub struct PortfolioConfig {
    pub portfolio_id: String,
    pub quote_codes: Vec<QuoteCode>,
    pub networks: Vec<NetworkConfig>,
    pub wallets: Vec<WalletConfig>,
    pub symbol_configs: Vec<SymbolConfig>,
    pub metadata: std::collections::BTreeMap<String, serde_json::Value>,
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
    pub symbol_ids: Vec<String>,
    pub metadata: std::collections::BTreeMap<String, serde_json::Value>,
}

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
    AaveV3ReservePosition {
        market_id: String,
        reserve_id: String,
        use_as_collateral_required: Option<bool>,
    },
    AaveV3DebtPosition {
        market_id: String,
        reserve_id: String,
        debt_kind: AaveDebtKind,
    },
    AaveV3StakingPosition {
        market_id: String,
        staking_contract: String,
    },
}

pub struct SymbolValuationConfig {
    pub quotes: Vec<QuoteValuationConfig>,
}

pub struct QuoteValuationConfig {
    pub quote: QuoteCode,
    pub reader: ValuationReaderConfig,
}

#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ValuationReaderConfig {
    FixedQuote {
        value_dec: String,
    },
    OraclePrice {
        source_id: String,
        base_symbol: String,
        quote: QuoteCode,
    },
    DerivedCrossQuote {
        left_source_id: String,
        right_source_id: String,
        intermediate_quote: QuoteCode,
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
    pub value_dec: String,
    pub unit_price_dec: String,
    pub valuation_reader_kind: String,
    pub source_ref: String,
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
    }
  ],
  "wallets": [
    {
      "wallet_id": "wallet_treasury",
      "address": "0x000000000000000000000000000000000000dead",
      "network_id": "ethereum-mainnet",
      "symbol_ids": [
        "eth.native.mainnet",
        "usdc.wallet.mainnet",
        "aave_v3.usdc.collateral.mainnet",
        "aave_v3.usdc.variable_debt.mainnet"
      ],
      "metadata": {}
    }
  ],
  "symbol_configs": [
    {
      "symbol_id": "eth.native.mainnet",
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
            "reader": {
              "kind": "oracle_price",
              "source_id": "chainlink_eth_usd",
              "base_symbol": "ETH",
              "quote": "USD"
            }
          },
          {
            "quote": "BTC",
            "reader": {
              "kind": "oracle_price",
              "source_id": "chainlink_eth_btc",
              "base_symbol": "ETH",
              "quote": "BTC"
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
  "wallet_id": "wallet_treasury",
  "symbol_id": "aave_v3.usdc.variable_debt.mainnet",
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
      "value_dec": "10.000000",
      "unit_price_dec": "1.000000",
      "valuation_reader_kind": "oracle_price",
      "source_ref": "chainlink_usdc_usd"
    },
    {
      "quote": "BTC",
      "value_dec": "0.00010000",
      "unit_price_dec": "0.00001000",
      "valuation_reader_kind": "derived_cross_quote",
      "source_ref": "chainlink_usdc_usd x chainlink_btc_usd"
    }
  ],
  "source": {
    "balance_reader_kind": "aave_v3_debt_position",
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
- `underlying_symbol_id`, when present, must refer to an existing symbol
- all addresses are valid normalized EVM addresses when the reader requires them
- all decimals are integers and never floats

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

Every read should normalize into one observation shape, regardless of source.

Suggested normalized observation:

```json
{
  "wallet_id": "wallet_a",
  "symbol_id": "aave_v3.usdc.collateral.mainnet",
  "display_symbol": "USDC",
  "kind": "protocol_position",
  "role": "collateral",
  "network": "ethereum-mainnet",
  "protocol": "aave_v3",
  "raw_u256_dec": "1000000",
  "decimals": 6,
  "amount_dec": "1.000000",
  "value": {
    "USD": {
      "raw_dec": "1.000000",
      "price_reference": "USDC/USD",
      "pricing_source_kind": "oracle_price"
    },
    "BTC": {
      "raw_dec": "0.00001000",
      "price_reference": "USDC/USD x BTC/USD",
      "pricing_source_kind": "derived_cross_quote"
    }
  },
  "source": {
    "reader_kind": "aave_v3_reserve_position",
    "block_number": 12345678
  },
  "metadata": {
    "use_as_collateral": true
  }
}
```

This gives us one pipeline for:

- reading
- valuing
- sorting
- serialization
- aggregation
- reporting

## Proposed Snapshot Shape

The current snapshot artifact should evolve toward something like:

```json
{
  "portfolio_id": "portfolio_x",
  "generated_at_ms": 1234567890,
  "network_pins": {
    "ethereum-mainnet": {
      "chain_id": 1,
      "block_number": 12345678
    }
  },
  "wallets": [
    {
      "wallet_id": "wallet_a",
      "address": "0x...",
      "network": "ethereum-mainnet",
      "observations": [
        {
          "symbol_id": "eth.native.mainnet",
          "role": "native",
          "amount_dec": "1.000000000000000000",
          "value": {
            "USD": { "raw_dec": "3500.00" },
            "BTC": { "raw_dec": "0.05000000" }
          }
        },
        {
          "symbol_id": "usdc.wallet.mainnet",
          "role": "asset",
          "amount_dec": "100.000000",
          "value": {
            "USD": { "raw_dec": "100.000000" },
            "BTC": { "raw_dec": "0.00100000" }
          }
        },
        {
          "symbol_id": "aave_v3.usdc.collateral.mainnet",
          "role": "collateral",
          "amount_dec": "50.000000",
          "value": {
            "USD": { "raw_dec": "50.000000" },
            "BTC": { "raw_dec": "0.00050000" }
          }
        },
        {
          "symbol_id": "aave_v3.usdc.variable_debt.mainnet",
          "role": "debt",
          "amount_dec": "10.000000",
          "value": {
            "USD": { "raw_dec": "10.000000" },
            "BTC": { "raw_dec": "0.00010000" }
          }
        }
      ]
    }
  ],
  "symbol_configs": {
    "eth.native.mainnet": { "...": "..." },
    "usdc.wallet.mainnet": { "...": "..." },
    "aave_v3.usdc.collateral.mainnet": { "...": "..." },
    "aave_v3.usdc.variable_debt.mainnet": { "...": "..." }
  },
  "errors": []
}
```

## Why per-network pins matter

The user model implies that one portfolio can eventually include:

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
- network / block pins when prices come from chain data

For some pricing sources, the same per-network block pin may be enough.
For others, we may need explicit valuation-source pinning.

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
2. validate wallet + symbol config references
3. pin block per network
4. dispatch balance readers by `symbol_config.balance_reader.kind`
5. dispatch valuation readers by `symbol_config.valuation`
6. normalize each result into a common observation shape
7. aggregate observations per wallet
8. aggregate observations per portfolio
9. write artifact
10. write report

Reader dispatch examples:

- `native_balance`
- `erc20_balance`
- `aave_v3_reserve_position`
- `aave_v3_account_summary`
- `aave_v3_staked_position`
- `oracle_price`
- `derived_cross_quote`

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

This matters especially for Aave V3 positions where a portfolio may span many reserves.

## Proposed Phased Plan

### Phase 0: define the new canonical model

- define the new canonical `PortfolioSnapshot`
- remove compatibility requirements with the current request / response model
- define `portfolio`, `wallet`, `symbol_config`, `observation`, and valuation shapes

### Phase 1: implement base wallet + valuation support

- add `portfolio_id`
- add multi-wallet config
- add symbol registry / symbol config map
- keep network pinning explicit
- add quote support for at least `USD` and `BTC`
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
- Use `symbol_id` as the canonical machine identity, not raw `symbol`.
- Model native, wallet, collateral, debt, and staked positions with one normalized observation shape.
- Make valuation a core responsibility of symbol config, not a separate optional afterthought.
- Pin blocks per network, not globally.
- Treat Aave V3 as protocol-backed symbol configs, not as a separate portfolio system.
- Break and replace the current portfolio snapshot shape rather than carrying compatibility baggage.

## Open Questions

- Does one portfolio need to support multiple networks in the first version, or only one network with multiple wallets?
- What exactly counts as "staked assets"?
- Are Aave rewards in scope, or only principal positions?
- Do we need discovery, or is all tracking config-driven?
- Do we want best-effort snapshots, or strict fail-fast semantics?
- Which pricing sources are in scope first for valuation?
- Should quote support be configurable per portfolio, per wallet, or per symbol?
- How do we want to represent debt in quote summaries?
- Should debt be represented as:
  - positive amount with `role = debt`
  - negative amount
  - both

## Immediate Next Steps

1. Write the new canonical `PortfolioSnapshot` request and artifact schemas.
2. Define `symbol_config` so it owns both balance acquisition and valuation acquisition.
3. Add quote support requirements for at least `USD` and `BTC`.
4. Add Aave V3 shared read states in `crates/states/aave-v3`.
5. Keep the planner op thin and assemble native / ERC-20 / Aave readers through one normalized workflow.

## Next Time

- Write the exact canonical `PortfolioSnapshot` request and artifact schemas before changing code.
- Decide whether Aave V3 position reads should use generic ABI-decoded calls or dedicated Aave-specific states.
- Resolve the first pricing sources for `USD` and `BTC` valuation.
- Resolve the meaning of "staked assets" before implementing the Aave slice.
