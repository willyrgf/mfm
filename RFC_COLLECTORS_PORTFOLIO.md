# RFC: Fact-Backed Portfolio Collectors And Reports

Status: Draft

## Summary

`portfolio_snapshot` should not be a live crawler.

The current portfolio workflow attempts to pin chains, read wallet balances, value positions, and
assemble a report in one run. That works for narrow configured examples, but it is the wrong shape
for real portfolios. A portfolio view may require Bitcoin UTXO sets, EVM native balances, ERC-20
balances, token discovery, protocol positions, prices, metadata, freshness checks, and coverage
proofs. Pulling all of that on the fly makes the report workflow slow, incomplete, hard to replay,
and easy to misrepresent as complete.

The target architecture is:

```text
chain, indexer, oracle, or source IO
  -> bounded collector operation
  -> durable typed facts with anchors, coverage, and source evidence
  -> fact-backed portfolio report operation
  -> PortfolioSnapshot + PortfolioReport public outputs
```

Collectors produce reusable facts. Portfolio reporting consumes facts only.

`portfolio_snapshot` can remain the user-facing entry point, but its internal meaning should move
from "crawl every source now" to "assemble a portfolio view from certified observations", optionally
after running bounded collectors in a composed workflow.

## Problem

### Live Portfolio Reads Do Not Scale To Real Portfolios

The example dual-mainnet portfolio contains one Bitcoin address and one Ethereum address. Even that
small shape exposes the problem:

- a Bitcoin address may have many UTXOs
- an EVM address may hold many ERC-20 tokens
- token discovery requires logs, an indexer, or another source beyond simple JSON-RPC balance reads
- valuation needs price observations with their own anchors and freshness policy
- completeness depends on what the source actually scanned, not only on whether a balance read
  returned zero

An on-demand report run cannot honestly infer a complete multi-chain portfolio unless it also proves
the data universe it searched.

### Zero Is Not The Same As Complete

If a report cannot discover all tokens, prices, or protocol positions, a zero total is misleading.
The workflow needs to distinguish:

- complete at anchor
- configured assets only
- discovery coverage incomplete
- missing facts
- stale facts
- source unsupported
- source failed

Coverage is a first-class output of collection. It is not a UI detail.

### Reporting And Collection Have Different Lifecycles

Collection is source-facing and operational:

- it may need checkpoints
- it may run periodically
- it may fan out by address, chain, token, price source, or protocol
- it may use different providers over time
- it may retry or lag behind the latest chain head

Reporting is deterministic and compositional:

- it should select a bounded fact frontier
- it should materialize fact response artifacts
- it should normalize facts into the existing portfolio `Observation` view
- it should produce stable public outputs
- replay should use recorded fact-query evidence, not live IO

Putting both lifecycles in one state graph creates a workflow that is hard to certify, hard to
resume, and hard to explain.

## Goals

- Move portfolio reporting to a fact-backed workflow with no live chain/source reads.
- Keep collectors independently useful and reusable across reports, audits, and future workflows.
- Preserve the existing `PortfolioSnapshot`, `PortfolioReport`, and `Observation` public output
  concepts as report-facing DTOs.
- Add durable source-near facts for holdings, prices, metadata, and coverage.
- Make missing, stale, incomplete, or unsupported source data explicit snapshot/report errors.
- Use recorded fact-query evidence for report replay.
- Keep chain/indexer/oracle IO in collectors, adapters, transports, and provider backends.
- Keep operation crates deterministic and states free of ambient IO.
- Support a phased path where configured symbols work before open-ended token discovery.

## Non-Goals

- Do not build a general blockchain indexer inside `portfolio_snapshot`.
- Do not make the report workflow scan chain history or discover token universes directly.
- Do not treat a provider's empty result as complete unless the selected coverage policy proves it.
- Do not store secrets, RPC URLs, credentials, raw provider diagnostics, or private source routing in
  facts, artifacts, public outputs, or errors.
- Do not make `Observation` the primary fact format. `Observation` is the normalized report-facing
  view.
- Do not require one fact per UTXO, token transfer, or tiny source detail unless a later analytics
  use case needs it.
- Do not weaken replay semantics by querying the live fact index during replay without recorded
  query evidence.

## Core Decision

### Facts Are Source-Near

Portfolio facts should record bounded observations close to the source truth. The report operation
then converts selected facts into the existing portfolio model.

Examples:

- a Bitcoin UTXO snapshot fact records the address, anchor, UTXO set, total sats, and coverage
- an EVM native balance fact records the account, block, raw wei, decimals, and coverage
- an EVM token balances fact records the account, block, selected token balances, token metadata
  refs, and coverage
- a price snapshot fact records the priced symbol, quote, source id, anchor, price decimal, and
  freshness material

The report-facing `Observation` stays a projection:

```text
holding facts + price facts + symbol config + valuation config
  -> Observation
```

This keeps facts reusable for other views while preserving the current public snapshot/report shape.

### Portfolio Reports Consume Facts Only

Add a fact-backed report operation, tentatively named `portfolio_snapshot_from_facts`.

The operation should:

- resolve configured subjects and symbols
- build deterministic fact query plans for the required holdings, prices, metadata, and coverage
- execute fact-index reads through explicit capabilities
- record fact-query evidence and trust-root material
- materialize selected fact response artifacts
- normalize selected facts into `Observation` values
- assemble `PortfolioSnapshot`
- project `PortfolioReport`

The report workflow must not call chain, indexer, oracle, or price-source providers directly.

### Collectors Own Source IO

Collectors are bounded typed workflows that produce facts.

Near-term collectors:

- `bitcoin_address_utxo_collector`
- `evm_account_balance_collector`
- `evm_token_balance_collector`

Later collectors:

- `evm_token_discovery_collector`
- `token_metadata_collector`
- `price_collector`
- protocol position collectors, such as Aave positions

Collectors should follow the existing `btc_chain_head_collector` shape:

- query prior control checkpoint facts when needed
- read source data through explicit capabilities
- normalize source responses into typed state outputs
- record data facts with `ManagedPlatformWrite`
- record checkpoint/control facts where the collector has progress state
- support replay from recorded evidence and retained artifacts

### `portfolio_snapshot` Becomes Composition

The public `portfolio_snapshot` entry point can remain, but its internal shape should change.

For configured assets, a composed snapshot can run a static set of bounded collectors and then run
the fact-backed report:

```text
configured wallet/symbol config
  -> bounded BTC/EVM/price collectors
  -> portfolio_snapshot_from_facts
```

For discovered assets, collection and reporting should probably be multi-run orchestration. Typed
program topology cannot depend on discovered runtime values inside the same certified run. If token
discovery finds new symbols, that should create a new planning boundary, not mutate the topology of
the current report run.

## Asset Universe Policy

The portfolio config needs an explicit asset-universe policy before the system can claim
completeness.

Initial mode:

```toml
[portfolio.asset_universe]
mode = "configured_symbols"
```

Meaning:

- only symbols listed in wallet `symbol_ids` are required
- missing facts for configured symbols are errors
- unconfigured tokens or protocols are out of scope
- the report must not claim complete wallet discovery

Future mode:

```toml
[portfolio.asset_universe]
mode = "discover"
token_standards = ["erc20"]
max_staleness_ms = 300000
```

Meaning:

- discovery collectors define the searched token/protocol universe
- coverage facts must prove what was searched
- discovered holdings may require a new planning boundary before reporting

The exact config surface is intentionally not finalized in this RFC. The design requirement is that
asset-universe semantics are explicit and hash-defining, not implicit runner behavior.

## Recommended Fact Families

### `bitcoin.address_utxo_snapshot`

Subject:

- `network`
- `bitcoin_network`
- `semantic_source_identity`
- `address`
- optional `coverage_policy`

Response:

- anchor height
- anchor block hash
- total sats
- UTXO entries
- coverage status
- source status
- observed source watermark, when available

The response may include UTXO entries as a bounded list because the portfolio report needs the total
and may need per-UTXO audit details. If a source truncates or cannot prove completeness, the fact
must say so.

### `evm.address_native_balance_snapshot`

Subject:

- `network`
- `chain_id`
- `account`

Response:

- block number
- optional block hash, when the provider can prove it
- raw wei decimal string
- decimals
- coverage status
- source status

### `evm.address_token_balances_snapshot`

Subject:

- `network`
- `chain_id`
- `account`
- reader or discovery policy id

Response:

- block number
- optional block hash, when the provider can prove it
- token balance entries
- token metadata refs or inline non-secret metadata
- coverage status
- source status
- discovered universe watermark, when applicable

This fact can represent configured-token reads or discovery-backed reads. The coverage policy must
make the difference explicit.

### `price.quote_snapshot`

Subject:

- `priced_symbol_id`
- `quote`
- `source_id`

Response:

- unit price decimal string
- anchor or source timestamp
- freshness metadata
- source status
- source refs, when the price comes from an on-chain oracle or derived source

Prices should remain decimal strings or integer-scaled values. Hashed structured data must not
contain floats.

### Coverage Facts

Coverage can be embedded in each snapshot fact for simple cases. More complex discovery workflows may
also need dedicated coverage/checkpoint facts, for example:

- searched block range
- finality policy
- token standards covered
- protocol readers covered
- source-specific cursor or high watermark
- truncation or pagination status

Coverage facts are part of the semantic result. They must be replayable and content-addressed like
other facts.

## Fact Visibility

Portfolio holding, price, metadata, and coverage facts should default to public `Platform` audience.

The values observed by these collectors are public chain or public market data. MFM should preserve
that openness by making the durable fact layer publicly discoverable where the descriptor exposure
policy allows it.

Public does not mean every internal detail is returnable. Fact descriptors should still classify
fields deliberately:

- returnable fields for ordinary public inspection
- query-only fields for filtering and ordering when values should not appear in public rows
- hidden fields for internal response material that should remain retained evidence only

The portfolio report public output remains the curated portfolio view. Public facts are the reusable
observation substrate behind that view.

## Report Query Semantics

The fact-backed report operation should query a bounded fact frontier.

For each configured wallet/symbol/quote requirement, the report should select:

- the latest fact at or before the report frontier
- facts matching the configured network, address/account, symbol, quote, and source policy
- facts whose coverage status satisfies the configured asset-universe policy
- facts whose freshness satisfies configured staleness policy

If no acceptable fact exists, the report emits a `PortfolioSnapshotError` instead of silently
returning zero.

Replay must use recorded fact-query evidence and retained response artifacts. It must not re-query
the live store frontier and accept newer facts.

## Crate Placement

Expected placement follows `docs/architecture.md`:

- fact value types and state contracts: `crates/states/*`
- collector topology: `crates/ops/*-collector-op`
- report topology: `crates/ops/portfolio-tracker-op` or a new portfolio report op crate
- provider request/response contracts: domain capability crates
- live provider implementations: `crates/transports/*`
- runner bindings and fact materialization: `crates/adapters/*`
- app/Nix wiring: `crates/app`, `nixfied.nix`, and flake surfaces only

The exact crate split can be decided during implementation. The boundary rule is fixed: operations
plan, states own deterministic semantics, adapters bind, transports perform live/replay provider
behavior, and app assembly wires services.

## Phased Plan

### Phase 1: Fact-Backed Reporting Contract

- Add `portfolio_snapshot_from_facts` or equivalent.
- Define report-side fact query config and selection policy.
- Materialize selected fact response artifacts.
- Convert selected facts into existing `Observation` values.
- Emit explicit errors for missing, stale, incomplete, or unsupported facts.
- Record fact-query evidence for replay.

This phase can use test fixtures or manually recorded facts. It should not require production
collectors to exist first.

### Phase 2: Configured Asset Collectors

- Add BTC UTXO snapshot collector for configured Bitcoin addresses.
- Add EVM native balance collector for configured EVM accounts.
- Add EVM ERC-20 balance collector for configured token contracts.
- Record data facts and control checkpoint facts.
- Add replay tests proving collectors do not call live providers during replay.

### Phase 3: Price And Metadata Facts

- Add price snapshot facts and a price collector.
- Add token metadata facts when needed for ERC-20 display and decimals.
- Move valuation from fixed placeholders toward fact-selected prices.
- Keep all numeric prices as decimal strings or integer-scaled values.

### Phase 4: Discovery And Coverage

- Add token discovery collectors.
- Add explicit coverage facts for scanned ranges, token standards, finality, and source watermarks.
- Extend portfolio config from configured symbols to explicit asset-universe policies.
- Use a new planning boundary when discovered assets change report topology.

## Open Questions

- Should holding facts live in a new `crates/states/portfolio-facts` crate, or in existing
  family-specific state crates such as BTC/EVM state crates?
- Should the first report operation be a new public entry point or an internal operation called by
  `portfolio_snapshot`?
- What is the minimum fact-query capability change needed for report states to read `Platform`
  portfolio facts with recorded query evidence?
- How should freshness policy be represented in canonical portfolio config?
- Should Bitcoin UTXO entries be fully retained in the snapshot fact response, or should large UTXO
  sets use a separate content-addressed artifact referenced by the fact response?
- What is the first supported token discovery provider: local Reth/Erigon, hosted indexed data, or a
  provider-neutral log capability?

## Verification Expectations

Implementation work should add focused tests for:

- report runs that consume only facts and fail if live chain providers are unavailable
- missing fact errors for configured symbols
- incomplete coverage errors
- stale price or balance fact errors
- replay from recorded fact-query evidence without live fact-index or chain reads
- collector checkpoint progression
- collector replay without live source reads
- fact descriptor exposure, especially that public holding facts expose only intentional fields
- no-secret persisted/public surfaces for facts, artifacts, diagnostics, and outputs

Final merge-readiness should continue to use the repository's normal Nixfied gates.
