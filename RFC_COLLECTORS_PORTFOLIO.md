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
protocol IO (transports)
  -> family adapter binds observe/record runners
  -> family collector Operation (approved cycle shape)
  -> durable source-near Platform data facts
     (+ Control cursors only when progressive)
  -> portfolio report Operation (fact-index only)
  -> PortfolioSnapshot + PortfolioReport public outputs
```

Collectors produce reusable facts. Portfolio reporting consumes facts only.

`portfolio_snapshot` may remain a user-facing entry name, but its internal meaning moves from
"crawl every source now" to "assemble a portfolio view from certified observations", optionally
after running bounded collectors in a composed workflow.

This RFC is a breaking redesign. Dual live+facts paths, silent fallbacks, and crate-per-collector
scaffolding are rejected. Old live portfolio crawl code is deleted in the same design arc as the
fact-backed report lands. Git history is recovery.

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

- progressive collectors may need control cursors
- collectors may run periodically
- collectors may fan out by address, chain, token, price source, or protocol at planning time
- collectors may use different providers over time
- collectors may retry or lag behind the latest chain head

Reporting is deterministic and compositional:

- it should select a bounded fact frontier
- it should materialize fact response artifacts
- it should normalize facts into the existing portfolio `Observation` view
- it should produce stable public outputs
- replay should use recorded fact-query evidence, not live IO

Putting both lifecycles in one state graph creates a workflow that is hard to certify, hard to
resume, and hard to explain.

### Collector Surface Area Will Explode Without A Standard

Near-term collectors already include chain-head, Bitcoin UTXO, EVM native balance, and EVM token
balance. Later collectors add discovery, metadata, prices, and protocol positions. Without a
minimal standard:

- N nearly-identical op crates appear for thin topology glue
- checkpoint/control machinery is copied with wrong watermark semantics
- portfolio grows a second live truth path beside collectors
- a mega "collector framework" freezes tomorrow's discovery into today's balance reads

The expensive mistakes are dual IO truth paths and framework soup. The cheap mistake is pasting a
small `expand` again. Optimize for the expensive mistakes.

## Goals

- Move portfolio reporting to a fact-backed workflow with no live chain/source reads.
- Keep collectors independently useful and reusable across reports, audits, and future workflows.
- Keep `PortfolioSnapshot`, `PortfolioReport`, and `Observation` as report-facing DTOs only.
- Add durable source-near facts for holdings, prices, metadata, and coverage.
- Make missing, stale, incomplete, or unsupported source data explicit snapshot/report errors.
- Use recorded fact-query evidence for report replay.
- Keep chain/indexer/oracle IO in collectors, adapters, transports, and provider backends.
- Keep operation crates deterministic and states free of ambient IO.
- Standardize two approved collector cycle shapes without inventing a collector framework.
- Organize collectors by family monocrates so growth is O(families), not O(collectors).
- Prefer fewer concepts, fewer code paths, fewer public types, and fewer places future changes
  must touch.
- Support a phased path where configured symbols work before open-ended token discovery.
- Delete the live portfolio crawl path in the same design arc as fact-backed reporting.

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
- Do not create one op crate per collector cycle.
- Do not create a mega `CollectorFramework`, generic `CollectorPolicy`, or cross-domain checkpoint
  monomorphization.
- Do not keep a dual live+facts portfolio path "for safety."
- Do not force progressive control cursors onto configured snapshot re-reads for topology
  uniformity.

## Core Decision

### Facts Are Source-Near

Portfolio facts record bounded observations close to the source truth. The report operation then
converts selected facts into the existing portfolio model.

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

This keeps facts reusable for other views while preserving the public snapshot/report shape.

Holdings facts live in family state crates (`states/btc`, `states/evm`, later `states/price`).
They do **not** live in a `portfolio-facts` crate. Portfolio owns report query, selection,
normalization, and assembly only.

### Portfolio Reports Consume Facts Only

The report operation (public entry may remain `portfolio_snapshot`, internal meaning is fact-backed
assembly) must:

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

- `btc_chain_head_collector` (exists; progressive template)
- `bitcoin_address_utxo_collector` (snapshot re-read)
- `evm_account_balance_collector` (snapshot re-read)
- `evm_token_balance_collector` (snapshot re-read)

Later collectors:

- `evm_token_discovery_collector` (progressive)
- `token_metadata_collector`
- `price_collector` (usually snapshot re-read)
- protocol position collectors, such as Aave positions

### Delete The Live Portfolio Crawl

When the fact-backed report is real, delete the live crawl path. Do not leave both forever.

Delete:

- live `ObserveBatch` execution as the portfolio truth path
- portfolio adapter code whose only job is live chain/BTC reads into `Observation`
- implicit "zero means empty portfolio" without fact/coverage proof
- any dual-mode fallback that live-reads when facts are missing

Keep:

- `Observation`, `PortfolioSnapshot`, and `PortfolioReport` as report DTOs
- family collectors as the only source IO for holdings and related observations
- fail-closed report errors for missing, stale, incomplete, or unsupported facts

Composition of "run collectors then report" is orchestration (composed draft or multi-run), not a
permanent dual IO mode inside one graph.

### `portfolio_snapshot` Becomes Composition

For configured assets, a composed workflow can run a static set of bounded collectors and then run
the fact-backed report:

```text
configured wallet/symbol config
  -> static planning-time fanout of snapshot collectors
  -> portfolio report from facts
```

For discovered assets, collection and reporting are multi-run orchestration. Typed program topology
cannot depend on discovered runtime values inside the same certified run. If token discovery finds
new symbols, that creates a new planning boundary; it does not mutate the topology of the current
report run.

## Collector Standard (Minimal)

The shared platform is already the framework: certified ops, states, facts, fact-index read,
managed write, adapters, transports, and replay evidence. Collectors must not invent a second one.

### Approved Cycle Shapes

Exactly two shapes. Documented as a review standard, not as a crate.

#### 1. Progressive (cursor / watermark)

Use when work advances a cursor: chain head, discovery range, log scan.

```text
QueryControlCursor
  -> ObserveSource
  -> RecordDataFact
  -> RecordControlCursor
```

| Role | Effect | Capability | Audience of durable claim |
|---|---|---|---|
| QueryControlCursor | `ReadExternal` | Fact-index read | reads Control |
| ObserveSource | `ReadExternal` | domain source read | none yet |
| RecordDataFact | `ManagedPlatformWrite` | shared fact-record role | Platform |
| RecordControlCursor | `ManagedPlatformWrite` | shared fact-record role | Control |

Ordering invariants:

1. Cursor load is before observation so observe can reject regress / bound the batch start.
2. Data fact is recorded before cursor advance so a failed record never advances progress.
3. Cursor advance is derived only from certified observation/data-fact material plus prior cursor,
   never from ambient clocks or provider-only side channels.
4. Observe never writes facts; record never does live source IO.
5. Data facts are Platform; cursors are Control. Do not mix audiences.

Existing `btc_chain_head_collector` is the progressive template.

#### 2. Snapshot re-read (configured balances / prices)

Use when a bounded subject is re-observed fully each cycle: UTXO set, native balance, known ERC-20
set, price quote.

```text
ObserveSource@anchor
  -> RecordDataFact
```

Coverage and source status live on the data fact. Control cursors are **not** required for
configured full-subject re-reads. Report selection is "latest acceptable Platform fact at or before
frontier," not "resume collector cursor."

Near-term portfolio collectors use this shape:

- `bitcoin_address_utxo_collector`
- `evm_account_balance_collector`
- `evm_token_balance_collector`

Do not force snapshot collectors into the progressive 4-node shape for uniformity. Uniformity of the
wrong control model is still the wrong model.

### Configured Vs Progressive Semantics

| Mode | Observe | Control cursor | Coverage default |
|---|---|---|---|
| Snapshot re-read | Read fixed subjects from planning config at pinned/best head | not required | `configured_only` unless the source proves full wallet discovery |
| Progressive | Read a bounded batch from loaded watermark | high-watermark / scanned range | encodes scanned universe + truncation |

Configured multi-subject fanout is planning-time: the op expands one cycle subgraph per static
subject unit (address, token set unit). No runtime discovery fanout inside the certified run.

### Shared Vs Specialized

#### Must share (extract once; delete clones)

| Piece | Home |
|---|---|
| Managed fact-record runner | shared adapter kit (extract from private BTC-only runner) |
| Fact-index query runner (hydrate + selection evidence) | same shared kit |
| Fact-record capability role | `fact-capabilities` (not a BTC-bound record authority) |
| Coverage / source-status closed vocabulary | documented closed set; one pure home if string drift starts |
| Kernel fact/runtime machinery | existing kernel crates |

#### Specialized (monomorphic; do not abstract early)

| Piece | Home |
|---|---|
| Data fact subject/response + descriptors | family state crate (`btc`, `evm`, later `price`) |
| Progressive cursor fact types | same family that owns the watermark semantics |
| Observe config, validation, normalize, capability request | family state crate |
| Domain read capabilities | `btc-capabilities` / `evm-capabilities` |
| Live/replay providers | `transports/*` |
| Observe runners + protocol binding | family adapters |
| Topology `expand` (~thin wiring) | family op crate module |

#### Stay copy-paste until a third real duplicate hurts

- thin Operation `expand` graphs
- config → state-config mapping helpers
- `*_program_draft` builders
- certification registry lists in each op family crate

Copy-paste of topology glue is cheap. Copy-paste of fact schema semantics is not; those belong once
in the domain state crate.

### Explicit Non-Abstractions

1. No generic `CollectorState` / `ObserveSourceState<T>` mega-trait that erases request/response types.
2. No cross-family semantic soup types for holdings (`Address`, `Balance`, `Token` shared by BTC and
   EVM facts).
3. No unified chain capability spanning BTC JSON-RPC and EVM JSON-RPC.
4. No single soup `CollectorCheckpointFact` with optional `bitcoin_network` *and* `chain_id` *and*
   discovery fields.
5. No macro that emits whole collectors from a table.
6. No one-state "observe-and-record" that merges `ReadExternal` and `ManagedPlatformWrite`.
7. No dynamic topology from discovered subjects inside one certified run.
8. No ambient source routing in states or ops.
9. No shared collector framework crate that owns topology; ops remain the only expand owners.
10. No Platform fact-index reads inside collectors for "load previous portfolio balance." Collectors
    read sources and, when progressive, Control cursors. Reports own Platform fact selection.
11. No dual portfolio path that live-reads when facts are missing.
12. No progressive control cursor forced onto every snapshot re-read collector.

### Control Cursors: Pattern Yes, Shared Soup Type No

Progressive collectors get family-local Control cursor facts. BTC height/hash watermark stays BTC.
EVM progressive cursors, when needed, get EVM-local watermark semantics.

Do **not** add a shared `states/collector` checkpoint soup type in Phase 2. Share runners and
conventions first. Extract shared cursor *types* only when two progressive collectors share the same
watermark semantics (they almost will not).

If a progressive collector needs stronger identity, encode it in stable hash-defining
`collector_kind` + `partition` strings on its monomorphic cursor subject, not in a free-form map.

### Coverage Vocabulary (Near-Term)

Near-term configured collectors embed coverage on each data fact response. Minimum closed set:

- `complete_at_anchor` — source proved completeness for the claimed universe
- `configured_only` — explicit incomplete wallet universe; configured assets only
- `truncated` / `incomplete`
- `unsupported` / `failed` (or fail closed before fact write)

Coverage is produced in observe normalize, not as a fifth cycle role.

Dedicated coverage facts are Phase 4 discovery territory, when search proof is independent of one
holdings snapshot.

### Capability Matrix

| Need | Capability class |
|---|---|
| Source observation | family domain read caps (`btc-capabilities`, `evm-capabilities`, later price) |
| Progressive cursor load | shared fact-index read (Control-only) |
| Record data / cursor facts | shared managed fact-record role (`fact-capabilities`) |
| Portfolio report selection | Platform fact-index read with recorded query evidence (report track) |

### Adapter Duty Split

- Family adapter registers domain observe runners for that protocol.
- Shared fact-record and control-cursor query runners are reused across families.
- Adapter maps requests, binds providers, and records evidence.
- State `materialize_response` / normalize remains the sole normalize authority.
- Transports implement providers; they do not know collector topology.

### Fanout Rule

Static planning-time subject units only. Discovery that changes report topology requires a new
planning boundary and usually a new run.

## Repository Organization

Organize by durable family and layer monocrates, not by collector recipe name.

Collector name is a type, module, and schema name. It is not a crate name.

### Target Tree

```text
crates/
  btc-capabilities/
  evm-capabilities/
  fact-capabilities/

  portfolio/model/
  portfolio-config/

  states/
    btc/                 # chain-head + address UTXO facts/states
    evm/                 # NEW: native + token balance facts/states
    portfolio/           # report only: fact query -> Observation -> snapshot/report
    proof/               # REHOME from collectors/proof
    # later: price/ when price facts land

  ops/
    btc-collectors-op/   # RENAME/MERGE from btc-chain-head-collector-op
      chain_head_cycle.rs
      address_utxo_cycle.rs
    evm-collectors-op/   # NEW
      account_balance_cycle.rs
      token_balance_cycle.rs
    portfolio-tracker-op/  # fact-backed report; delete live crawl topology
    evm-contract-lifecycle-op/
    proof-op/

  adapters/
    btc-jsonrpc/         # all BTC collector observe runners + shared fact runners
    evm/                 # NEW when EVM collectors land (not hung on portfolio/contracts)
    portfolio/           # fact-index + pure report runners only
    fact/                # optional thin shared fact-record/query runners

  transports/
    btc-jsonrpc-http/
    evm/
    proof/

  app/                   # one register_* per adapter family; few public entry points
```

### Deletes And Merges

| Action | Target |
|---|---|
| MERGE | `ops/btc-chain-head-collector-op` → `ops/btc-collectors-op` |
| DELETE tree | `crates/collectors/` as a taxonomy layer |
| MOVE | `collectors/proof` → `states/proof` |
| DELETE path | live portfolio `ObserveBatch` crawl once fact-backed report exists |
| DELETE authority | BTC-only fact-record capability as the global record path; move to shared role |
| DO NOT ADD | one op crate per collector; `adapters/*-collector`; `states/portfolio-facts` for chain holdings |
| DO NOT ADD | `states/collector` soup checkpoint crate in Phase 2 |

### Placement Rules

1. If it plans a graph → family op crate module / `Operation` type.
2. If it is `State::handle`, fact types, or normalize → family state crate.
3. If it is provider authority → `*-capabilities`.
4. If it is HTTP/RPC → `transports/*`.
5. If it binds state intent to capability + records evidence → `adapters/{protocol}`.
6. If it is report DTO, wallet config, or valuation join → portfolio model/config/state/op.
7. If two families share identical runner mechanics → shared fact adapter kit, not copy-paste.
8. If two families only share a workflow slot → do not invent a generic cross-chain observer type.
9. Workflow names stop at ops; never appear in transport, capability, or adapter crate names.
10. New collector = new module + `Operation` + monomorphic states + adapter registration lines, not
    new crates at every layer.
11. Portfolio crates must not grow source-near holding fact types.
12. Collector ops must not depend on portfolio report DTOs.

### Near-Term Crate Budget

Near-term collectors: chain-head, address UTXO, EVM native balance, EVM token balance, plus
fact-backed report.

| Unit | Count |
|---|---:|
| Collector cycle Operation types | 4 |
| Collector op crates | 2 (`btc-collectors-op`, `evm-collectors-op`) |
| Report op crate | 1 (existing `portfolio-tracker-op`) |
| New state crates | 1 (`states/evm`; optional `adapters/fact`) |
| Extended state crates | 2 (`states/btc`, `states/portfolio`) |
| New transport crates | 0 |

Not chosen: one crate per collector (workspace and wiring tax). Not chosen: one all-chains
collectors monocrate (false coupling and recompile fan-in).

### App And Nix Wiring

Growth must stay O(families):

```text
app live start:
  register_btc_jsonrpc_runners(...)
  register_evm_runners(...)
  register_portfolio_runners(...)   # fact-index + pure only
  register_proof_runners(...)

certification:
  btc_collectors_op::register_*_descriptors
  evm_collectors_op::register_*_descriptors
  portfolio_tracker_op::register_*_descriptors
```

Rules:

1. One `register_*_runners` per adapter package, not per collector.
2. One certification register per op family crate, listing all cycles.
3. Public entry points stay few and durable. Internal collector cycles may certify and launch via
   tests or composed ops without each becoming a public entry point.
4. Nixfied gains workspace members and package deps only; no per-collector service slots.
5. Runtime config stays protocol sources (`btc`, `evm`), not collector recipe names.

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

Coverage is embedded in each snapshot fact for configured collectors. More complex discovery
workflows may also need dedicated coverage/control facts, for example:

- searched block range
- finality policy
- token standards covered
- protocol readers covered
- source-specific cursor or high watermark
- truncation or pagination status

Coverage claims are part of the semantic result. They must be replayable and content-addressed like
other facts.

## Fact Visibility

Portfolio holding, price, metadata, and coverage data facts default to public `Platform` audience.

The values observed by these collectors are public chain or public market data. MFM should preserve
that openness by making the durable fact layer publicly discoverable where the descriptor exposure
policy allows it.

Progressive control cursors are Control audience. They are not the public portfolio product.

Public does not mean every internal detail is returnable. Fact descriptors still classify fields
deliberately:

- returnable fields for ordinary public inspection
- query-only fields for filtering and ordering when values should not appear in public rows
- hidden fields for internal response material that should remain retained evidence only

The portfolio report public output remains the curated portfolio view. Public facts are the reusable
observation substrate behind that view.

## Report Query Semantics

The fact-backed report operation queries a bounded fact frontier.

For each configured wallet/symbol/quote requirement, the report selects:

- the latest fact at or before the report frontier
- facts matching the configured network, address/account, symbol, quote, and source policy
- facts whose coverage status satisfies the configured asset-universe policy
- facts whose freshness satisfies configured staleness policy

If no acceptable fact exists, the report emits a `PortfolioSnapshotError` instead of silently
returning zero.

Replay must use recorded fact-query evidence and retained response artifacts. It must not re-query
the live store frontier and accept newer facts.

## Phased Plan

### Phase 0: Collector Standard And Shared Fact Runners

- Treat this RFC as normative for collector shapes and repository organization.
- Extract shared managed fact-record and fact-index query runners from BTC-private adapter code.
- Move fact-record authority to the shared `fact-capabilities` role.
- Rebase `btc_chain_head_collector` onto the progressive standard without changing its product.
- Rename/merge `btc-chain-head-collector-op` into `btc-collectors-op` when the second BTC cycle
  lands, or earlier if the rename is cheaper alone.
- Rehome `collectors/proof` → `states/proof` and delete `crates/collectors/` when convenient in this
  arc.

### Phase 1: Fact-Backed Reporting Contract

- Implement fact-backed portfolio report semantics (entry may keep the `portfolio_snapshot` name).
- Define report-side fact query config and selection policy.
- Materialize selected fact response artifacts.
- Convert selected facts into existing `Observation` values.
- Emit explicit errors for missing, stale, incomplete, or unsupported facts.
- Record fact-query evidence for replay.
- Use test fixtures or manually recorded facts; production collectors are not required first.

### Phase 2: Configured Asset Collectors

- Add BTC UTXO snapshot collector (`Observe@anchor` → `RecordDataFact`) in `btc-collectors-op`.
- Add EVM native and ERC-20 balance collectors in `states/evm` + `evm-collectors-op`.
- Embed coverage on data facts; no control cursors for these snapshot re-reads.
- Add replay tests proving collectors do not call live providers during replay.
- Switch `portfolio_snapshot` meaning to report-from-facts.
- Delete the live portfolio crawl path and portfolio-only live balance adapter code.

### Phase 3: Price And Metadata Facts

- Add price snapshot facts and a price collector (usually snapshot re-read).
- Add token metadata facts when needed for ERC-20 display and decimals.
- Move valuation from fixed placeholders toward fact-selected prices.
- Keep all numeric prices as decimal strings or integer-scaled values.

### Phase 4: Discovery And Coverage

- Add token discovery collectors as progressive cycles with family-local cursors.
- Add explicit coverage/control facts for scanned ranges, token standards, finality, and source
  watermarks when search proof is independent of one snapshot.
- Extend portfolio config from configured symbols to explicit asset-universe policies.
- Use a new planning boundary when discovered assets change report topology.

## Closed Decisions

| Question | Decision |
|---|---|
| Where do holding facts live? | Family state crates (`btc`, `evm`). Not `portfolio-facts`. |
| One op crate per collector? | No. Family op crates with multiple `Operation` types. |
| Mega collector framework? | No. Kernel + two approved cycle shapes are the standard. |
| Always 4-node cycle? | No. Progressive uses 4 nodes; snapshot re-read uses 2. |
| Shared soup checkpoint type now? | No. Family-local progressive cursors; share runners first. |
| Report entry point? | Fact-backed assembly is the report meaning. Public name may stay `portfolio_snapshot`. Composition with collectors is orchestration. |
| Live portfolio crawl after facts? | Delete in the same design arc. No dual path. |
| `crates/collectors/` taxonomy? | Delete. Rehome proof under `states/proof`. |

## Open Questions

- What is the minimum fact-query capability change needed for report states to read Platform
  holding/price facts with recorded query evidence?
- How should freshness policy be represented in canonical portfolio config (global vs per-kind vs
  per-source)?
- Should Bitcoin UTXO entries be fully retained in the snapshot fact response, or should large UTXO
  sets use a separate content-addressed artifact referenced by the fact response?
- What is the first supported token discovery provider: local Reth/Erigon, hosted indexed data, or a
  provider-neutral log capability?
- Exact packaging of shared fact runners: `adapters/fact` vs adapter-internal shared kit. Local
  packaging choice, not an architecture fork.

## Verification Expectations

Implementation work should add focused tests for:

- report runs that consume only facts and fail if live chain providers are unavailable
- missing fact errors for configured symbols
- incomplete coverage errors
- stale price or balance fact errors
- replay from recorded fact-query evidence without live fact-index or chain reads
- progressive collector cursor non-regression and checkpoint progression
- snapshot collector replay without live source reads
- topology tests for both approved cycle shapes
- fact descriptor exposure, especially that public holding facts expose only intentional fields
- no-secret persisted/public surfaces for facts, artifacts, diagnostics, and outputs
- absence of dual portfolio live-read fallback after the crawl path is deleted

Final merge-readiness should continue to use the repository's normal Nixfied gates.
