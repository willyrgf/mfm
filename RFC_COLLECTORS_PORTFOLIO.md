# RFC: Fact-Backed Portfolio Collectors And Reports

Status: Draft

## Summary

`portfolio_snapshot` should not be a live crawler.

The current portfolio workflow attempts to pin chains, read wallet balances, value positions, and
assemble a report in one run. That works for narrow configured examples, but it is the wrong shape
for real portfolios. A portfolio view may require Bitcoin balances, EVM native balances, ERC-20
balances, token discovery, protocol positions, prices, metadata, freshness checks, and coverage
proofs. Pulling all of that on the fly makes the report workflow slow, incomplete, hard to replay,
and easy to misrepresent as complete.

The target architecture is:

```text
protocol IO (transports)
  -> family adapter binds observe/record runners
  -> family collector Operation (snapshot or progressive pattern)
  -> durable source-near Platform data facts
     (+ Control cursors only when progressive)
  -> portfolio report Operation (Platform fact-index only)
  -> PortfolioSnapshot + PortfolioReport public outputs
```

Collectors produce reusable facts. Portfolio reporting consumes facts only.

Public surfaces after cutover:

| Surface | Meaning |
|---|---|
| `portfolio_snapshot` | fact-backed report only |
| collector entry points / internal ops | source IO that records facts |
| CLI/docs "full refresh" | external multi-run orchestration (collect, then report), not a dual-IO certified op |

This RFC is a breaking redesign. Dual live+facts paths, silent fallbacks, and crate-per-collector
scaffolding are rejected. Live portfolio pin and observe code is deleted at the same public
cutover as the fact-backed report. Git history is recovery.

## Problem

### Live Portfolio Reads Do Not Scale To Real Portfolios

The example dual-mainnet portfolio contains one Bitcoin address and one Ethereum address. Even that
small shape exposes the problem:

- a Bitcoin address may have many UTXOs (near-term facts retain **balance total**, not full UTXO set)
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

- it should select facts against a hash-defining pin table
- it should materialize fact response artifacts
- it should normalize facts into the existing portfolio `Observation` view
- it should produce stable public outputs
- replay should use recorded fact-query evidence, not live IO

Putting both lifecycles in one state graph creates a workflow that is hard to certify, hard to
resume, and hard to explain.

### Live Portfolio Graph Is Dual IO Today

Current topology:

```text
ResolveSubjects
  -> PinViews          (live chain head / execution anchor)
  -> ResolveValuations
  -> ObserveBatch*     (live balances)
  -> MergeObservations
  -> AssembleSnapshot
  -> ProjectReport
```

Both `PinViews` and `ObserveBatch` are live chain IO. A facts-only report must delete **both**,
not only balance reads.

### Collector Surface Area Will Explode Without A Standard

Near-term collectors already include chain-head, Bitcoin balance, EVM native balance, and EVM token
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
- Use two collector control patterns without inventing a collector framework.
- Organize collectors by family monocrates so growth is O(families), not O(collectors).
- Prefer fewer concepts, fewer code paths, fewer public types, and fewer places future changes
  must touch.
- Support configured symbols before open-ended token discovery.
- Delete live portfolio pin and observe at one public cutover with the fact-backed report.

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
- Do not keep a dual live+facts portfolio path "for safety" or "for demos."
- Do not force progressive control cursors onto configured snapshot re-reads for topology
  uniformity.
- Do not use the app public-facts CLI/REST path as certified report authority.
- Do not require full UTXO set retention for near-term portfolio balance reporting.

---

## Foundational Contracts

These contracts are closed before implementation. They are not open questions.

### 1. Platform Fact-Index Read Capability

Today `FactIndexReadRequest` accepts only `FactAudience::Control`. Control reads with receipt,
trust-root, and selection evidence exist for progressive collector checkpoints. Platform reads
exist on the app public-facts surface (CLI/REST DTOs). That surface is **not** report authority.

**Decision:** extend the existing fact-index read capability so certified states may request
**Platform** audience reads with the same evidence class as Control reads today. Do not invent a
second query stack. Do not route report semantics through public-facts HTTP/CLI.

Normative sequence for each report fact selection:

```text
compile canonical fact query plan (from certified report config)
  -> FactIndexRead (Platform audience)
  -> authenticated receipt + trust root + selection evidence
  -> hydrate retained FactResponse artifact(s)
  -> validate selection cardinality and acceptability
  -> normalize into Observation / pin projection
```

Replay rules:

- replay uses recorded fact-query evidence and retained response artifacts only
- replay must not re-query the live index frontier and accept newer facts
- public-facts API and ambient store SQL are forbidden as semantic authority for report states

Who may call Platform fact-index in certified runs:

- portfolio report states (required)
- other certified readers only when they have the same evidence obligations
- collectors must not Platform-query prior portfolio holdings (see snapshot anchor rule)

### 2. Report Pin / Frontier Model

Two different concepts must not be conflated:

| Concept | Role |
|---|---|
| Store read frontier (projection generation, commit watermark) | Authenticates **which index snapshot** the query read; bound into the query receipt |
| Report pin table | Hash-defining **what chain time / as-of** the portfolio claims |

**Decision:** report run config carries an explicit, hash-defining **pin table**:

- one pin per configured network required by the portfolio (height and, when available, block hash)
- optional price as-of for later price-fact joins
- pins are certified planning material, not ambient process state

Selection rule:

- for each required holding, select the latest **acceptable** Platform fact whose anchor is
  consistent with that network's pin (at or before the pin, matching network/subject predicates)
- do **not** select "latest globally" across unpinned networks
- do **not** invent pins from live chain head during report
- `network_pins` in `PortfolioSnapshot` are projected from the certified pin table and/or selected
  fact anchors; they are not live-observed

Collectors own "best head at collect time." Reports only consume pins + facts.

Configured mode freshness (near-term):

- no wall-clock staleness checks until price/metadata phase
- acceptability is pin match + subject match + coverage allow-list + source status allow-list
- `generated_at_ms` on public output, if retained, must not become outcome-affecting ambient policy

### 3. Subject Projection (Portfolio Requirement → Fact Query)

Mapping is pure portfolio report logic. Source-near facts never embed `wallet_id` or `symbol_id`
unless those are true source identity (they are not).

Near-term projection table (`configured_symbols`):

| Portfolio requirement | Fact kind | Predicate fields | Coverage accept list |
|---|---|---|---|
| BTC native configured wallet+symbol | `bitcoin.address_balance_snapshot` | `network`, `bitcoin_network`, `semantic_source_identity`, `address` | `configured_only`, `complete_at_anchor` |
| EVM native configured account+symbol | `evm.address_native_balance_snapshot` | `network`, `chain_id`, `account` | `configured_only`, `complete_at_anchor` |
| EVM ERC-20 configured token set | `evm.address_token_balances_snapshot` | `network`, `chain_id`, `account`, reader/policy id, configured token set identity | `configured_only`, `complete_at_anchor` |

Projection inputs come from portfolio config (`network_id`, wallet address/account, symbol balance
reader, chain metadata). Fact subjects stay source-near.

### 4. Selection Policy And Fail-Closed Errors

For each configured requirement, the certified report plan fixes:

- fact kind
- subject predicates (from projection table)
- ordering (descriptor ordering name, e.g. latest by anchor height/block number)
- limit / cardinality (exactly one acceptable holding fact per requirement unless the fact kind is
  multi-entry by design, e.g. one token-balances fact containing many tokens)
- coverage allow-list
- source status allow-list
- pin consistency rule

These are hash-defining certified config or fixed policy digests. They are not adapter defaults.

If no acceptable fact exists, the report **fails closed**. It does not emit a silent zero holding.

Stable near-term `PortfolioSnapshotError.code` values:

| Code | When |
|---|---|
| `missing_fact` | no Platform fact matched subject + pin |
| `unacceptable_coverage` | fact exists but coverage not in allow-list |
| `unacceptable_source_status` | fact exists but source status not in allow-list |
| `pin_mismatch` | fact anchor inconsistent with certified pin |
| `ambiguous_facts` | selection cardinality violated |
| `unsupported_requirement` | portfolio requirement has no projection rule |

Soft errors inside snapshot payloads are allowed only when the certified report policy explicitly
permits partial success. Default for `configured_symbols` is hard fail on any required-symbol
failure.

### 5. Snapshot Collector Anchor Rule

Snapshot shape: `ObserveSource@anchor → RecordDataFact`.

**Decision (rule A):** the domain observe capability returns **pin material and balance material in
one observation response**. The anchor is certified observation material recorded into the data
fact. Collectors do not Platform-query prior portfolio balances. Collectors do not require a
separate pre-loaded pin config for Phase 2.

Rejected for Phase 2:

- silent independent "best head" per subject without recording the anchor on the fact
- Platform fact-index load of chain-head facts as a required collector step (rule C) unless a later
  progressive→snapshot dependency is introduced with full selection evidence

Orchestrators may still pin externally for multi-run coherence; the fact itself must carry the
observed anchor.

### 6. Public Entry And Composition Model

| Surface | Meaning after cutover |
|---|---|
| `portfolio_snapshot` | **report-only**: resolve subjects, query Platform facts, project report |
| collector ops | source IO only; record Platform data facts (+ Control cursors if progressive) |
| full refresh | **external** multi-run recipe: run required collectors, then run `portfolio_snapshot` |

A single certified draft that mixes collectors and report is optional later. It is **not** required
for cutover. Kernel multi-run orchestration is not a new authority layer; CLI/docs/app recipes are
non-authority glue.

Discovery that changes required symbols always creates a new planning boundary / new certified
run.

### 7. Cutover Gate (No Dual Public Path)

No merge may leave live `PinViews` or `ObserveBatch` as portfolio truth after public meaning flips.

**Public cutover (one meaning switch)** requires all of:

1. Platform fact-index read + evidence works for report states.
2. Dual-mainnet-shaped facts can be produced (configured collectors **or** a documented fixture /
   seed admission path for CI that uses the same `FactRecorded` + descriptor admission authority as
   production writers — not raw index row poking).
3. Fact-backed report graph replaces pin+observe.
4. Live portfolio pin/observe paths and portfolio-only live balance/head transports are deleted.
5. Public `portfolio_snapshot` means report-only.

Fixtures-only report work is allowed as **tests and private branches**. It must not land as a
half-migrated public entry that keeps live crawl "temporarily."

### 8. Coverage And Source Status Vocabulary

Typed closed enums (single pure owner; prefer a small shared pure module or the first family fact
crate with re-exports — not free-form strings). Near-term:

**CoverageStatus**

| Value | Meaning | May write Platform fact? | Report accept (`configured_symbols`)? |
|---|---|---|---|
| `complete_at_anchor` | source proved completeness for the claimed universe | yes | yes |
| `configured_only` | only configured assets; wallet discovery incomplete by design | yes | yes |
| `truncated` | source truncated or pagination incomplete | yes | no |
| `incomplete` | known incomplete observation | yes | no |

**SourceStatus**

| Value | Meaning | May write Platform fact? | Report accept? |
|---|---|---|---|
| `ok` | observation succeeded | yes | yes |
| `unsupported` | source cannot serve this subject | **no** — fail closed before write | n/a |
| `failed` | source error | **no** — fail closed before write | n/a |

Honesty bar: most JSON-RPC balance reads cannot claim full-wallet completeness. Configured
collectors default to `configured_only`.

### 9. Near-Term Bitcoin Holding Fact

**Decision:** Phase 2 uses `bitcoin.address_balance_snapshot` (total sats + anchor + coverage +
source status), matching current provider truth (`scantxoutset`-class total balance reads).

Full UTXO set retention, per-UTXO audit lists, and large-set artifact refs are deferred until an
analytics need forces them. Do not name the near-term fact `utxo_snapshot` if it does not retain
UTXOs.

### 10. Report Graph After Cutover

Delete from portfolio report topology and adapters:

- `PinViewsState` live head/anchor reads
- `ObserveBatchState` live balance reads
- `PortfolioReadCapability` / transport-factory paths used only for live pin or balance crawl

Keep / adapt pure steps:

- resolve subjects/symbols from certified config
- resolve valuations (placeholders allowed until price facts land; must be explicit)
- fact query plan + Platform fact-index read(s)
- normalize facts → `Observation`
- merge / assemble / project report DTOs

Target shape:

```text
ResolveSubjects
  -> ResolveValuations          (placeholders ok until price phase)
  -> QueryHoldingFacts*         (Platform fact-index; static fanout per requirement)
  -> MergeObservations
  -> AssembleSnapshot           (pins from certified pin table + fact anchors)
  -> ProjectReport
```

Exact state names may differ; the effect split and delete list are fixed.

### 11. Progressive Resume Invariants (Shape-Wide)

For progressive collectors:

1. Data fact records before control cursor advance.
2. Cursor advance derives only from certified observation/data-fact material + prior cursor.
3. Watermarks must not regress.
4. Observe is `ReadExternal` only; record is `ManagedPlatformWrite` only.
5. After data fact commits and cursor does not: resume may re-observe; a newer Platform fact may be
   written; cursor then advances from the new observation. Cycles are typically new runs; completed
   cells win within a run.
6. Snapshot re-reads may append multiple Platform facts for the same subject; report "latest
   acceptable" is totally ordered by descriptor ordering + certified selection policy.

### 12. Fixture And Test Admission

Facts used by report tests enter through `FactRecorded` + descriptor admission (or an explicit
test-only store authority that projects identically). Tests must not poke fact-index rows without
admission authority.

---

## Core Decision

### Facts Are Source-Near

Portfolio facts record bounded observations close to the source truth. The report operation then
converts selected facts into the existing portfolio model.

Examples:

- a Bitcoin balance snapshot fact records the address, anchor, total sats, and coverage
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

Holdings facts live in family state crates (`states/btc`, `states/evm`, later `states/price`).
They do **not** live in a `portfolio-facts` crate. Portfolio owns report query, selection,
normalization, and assembly only.

### Collectors Own Source IO

Near-term collectors:

- `btc_chain_head_collector` (exists; progressive template)
- `bitcoin_address_balance_collector` (snapshot re-read; total balance)
- `evm_account_balance_collector` (snapshot re-read)
- `evm_token_balance_collector` (snapshot re-read)

Later collectors:

- `evm_token_discovery_collector` (progressive)
- `token_metadata_collector`
- `price_collector` (usually snapshot re-read)
- protocol position collectors, such as Aave positions
- optional later: full UTXO snapshot if analytics require it

## Collector Patterns (Minimal)

The shared platform is already the framework: certified ops, states, facts, fact-index read,
managed write, adapters, transports, and replay evidence. Collectors must not invent a second one.

Two control patterns exist. They are review heuristics, not a framework crate, shape enum, or Phase
gate.

### Progressive (cursor / watermark)

Use when work advances a cursor: chain head, discovery range, log scan.

```text
QueryControlCursor
  -> ObserveSource
  -> RecordDataFact
  -> RecordControlCursor
```

| Role | Effect | Capability | Audience of durable claim |
|---|---|---|---|
| QueryControlCursor | `ReadExternal` | Fact-index read (Control) | reads Control |
| ObserveSource | `ReadExternal` | domain source read | none yet |
| RecordDataFact | `ManagedPlatformWrite` | shared fact-record role | Platform |
| RecordControlCursor | `ManagedPlatformWrite` | shared fact-record role | Control |

Existing `btc_chain_head_collector` is the progressive template.

### Snapshot re-read (configured balances / prices)

Use when a bounded subject is re-observed fully each cycle: balance total, known ERC-20 set, price
quote.

```text
ObserveSource@anchor   (pin + balances in one observation; rule A)
  -> RecordDataFact
```

Coverage and source status live on the data fact. Control cursors are not required.

Near-term portfolio collectors use this pattern:

- `bitcoin_address_balance_collector`
- `evm_account_balance_collector`
- `evm_token_balance_collector`

Do not force snapshot collectors into the progressive 4-node shape for uniformity.

### Configured Vs Progressive Semantics

| Mode | Observe | Control cursor | Coverage default |
|---|---|---|---|
| Snapshot re-read | Read fixed subjects; observation includes pin + balances | not required | `configured_only` unless the source proves full wallet discovery |
| Progressive | Read a bounded batch from loaded watermark | high-watermark / scanned range | encodes scanned universe + truncation |

Configured multi-subject fanout is planning-time: the op expands one cycle subgraph per static
subject unit. No runtime discovery fanout inside the certified run.

### Shared Vs Specialized

#### Must share (extract when a second consumer needs it)

| Piece | Home | When |
|---|---|---|
| Platform + Control fact-index query runner (hydrate + selection evidence) | shared adapter kit | **with report cutover** (Platform) and existing Control path |
| Managed fact-record runner | shared adapter kit | when second Platform fact writer lands (collectors), not as free-standing renames |
| Fact-record capability role | `fact-capabilities` | when deleting BTC-bound record authority for shared writers |
| Coverage / source status enums | single pure owner | with first holding fact types |
| Kernel fact/runtime machinery | existing kernel crates | already shared |

#### Specialized (monomorphic)

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

### Explicit Non-Abstractions

1. No generic `CollectorState` / `ObserveSourceState<T>` mega-trait.
2. No cross-family semantic soup types for holdings.
3. No unified chain capability spanning BTC and EVM JSON-RPC.
4. No single soup checkpoint type with optional `bitcoin_network` and `chain_id`.
5. No macro that emits whole collectors from a table.
6. No one-state observe-and-record merging effect classes.
7. No dynamic topology from discovered subjects inside one certified run.
8. No ambient source routing in states or ops.
9. No shared collector framework crate that owns topology.
10. No Platform fact-index reads inside collectors for prior portfolio balances.
11. No dual portfolio path that live-reads when facts are missing.
12. No progressive control cursor forced onto every snapshot collector.
13. No public-facts CLI/REST path as certified report authority.
14. No wall-clock staleness as uncertified ambient policy.

### Control Cursors

Progressive collectors get family-local Control cursor facts. Do not add a shared soup checkpoint
crate until two progressive collectors share the same watermark semantics.

### Capability Matrix

| Need | Capability class |
|---|---|
| Source observation | family domain read caps |
| Progressive cursor load | fact-index read (Control) |
| Record data / cursor facts | shared managed fact-record role |
| Portfolio report selection | fact-index read (Platform) + recorded evidence |

### Fanout Rule

Static planning-time subject units only. Discovery that changes report topology requires a new
planning boundary.

## Repository Organization

Organize by durable family and layer monocrates, not by collector recipe name. Collector name is a
type/module/schema name, not a crate name.

Placement rules are normative. The tree sketch is guidance for implementers, not a Phase gate.

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
10. New collector = new module + `Operation` + monomorphic states + adapter registration lines.
11. Portfolio crates must not grow source-near holding fact types.
12. Collector ops must not depend on portfolio report DTOs.

### Target Tree (Guidance)

```text
crates/
  btc-capabilities/
  evm-capabilities/
  fact-capabilities/          # Control + Platform fact-index; shared fact-record role

  portfolio/model/
  portfolio-config/

  states/
    btc/                      # chain-head + address balance facts/states
    evm/                      # native + token balance facts/states (when first EVM fact lands)
    portfolio/                # report only: fact query -> Observation -> snapshot/report

  ops/
    btc-collectors-op/        # family crate when second BTC cycle lands (rename from chain-head op)
    evm-collectors-op/        # when EVM collectors land
    portfolio-tracker-op/     # fact-backed report only after cutover

  adapters/
    btc-jsonrpc/
    evm/                      # when EVM collectors land
    portfolio/                # Platform fact-index + pure report runners only after cutover

  transports/
    btc-jsonrpc-http/
    evm/
```

Non-critical packaging (do when cheap; not cutover blockers):

- rename `btc-chain-head-collector-op` → `btc-collectors-op`
- rehome `collectors/proof` → `states/proof`; delete `crates/collectors/`
- extract shared runners to `adapters/fact` vs adapter-internal kit (local packaging)

### Deletes At Public Cutover

| Delete | Why |
|---|---|
| Live `PinViewsState` / head pin path in portfolio report | live chain IO |
| Live `ObserveBatchState` / balance crawl in portfolio report | live chain IO |
| Portfolio adapter transports used only for live pin or balance | dual truth |
| Silent zero holdings without acceptable facts | false completeness |
| Dual-mode "live if facts missing" | dual truth |

### App And Nix Wiring

Growth stays O(families): one `register_*_runners` per adapter package; one certification register
per op family crate; few public entry points; runtime config is protocol sources, not collector
recipes.

## Asset Universe Policy

Initial mode (hash-defining when present in config):

```toml
[portfolio.asset_universe]
mode = "configured_symbols"
```

Meaning:

- only symbols listed in wallet `symbol_ids` are required
- missing or unacceptable facts for configured symbols are errors
- unconfigured tokens or protocols are out of scope
- the report must not claim complete wallet discovery

Discovery mode config is deferred until discovery work starts. Do not invent runner defaults for
unfinalized discovery semantics.

## Recommended Fact Families

### `bitcoin.address_balance_snapshot` (near-term)

Subject:

- `network`
- `bitcoin_network`
- `semantic_source_identity`
- `address`
- optional `coverage_policy`

Response:

- anchor height
- anchor block hash (when available)
- total sats (decimal string or integer)
- coverage status
- source status

Matches current total-balance provider truth. Full UTXO lists are out of near-term scope.

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
- token balance entries for the configured or discovered set
- token metadata refs or inline non-secret metadata
- coverage status
- source status

Coverage policy must make configured-token vs discovery-backed reads explicit.

### `price.quote_snapshot` (later)

Subject:

- `priced_symbol_id`
- `quote`
- `source_id`

Response:

- unit price decimal string
- anchor or source timestamp
- freshness metadata
- source status
- source refs, when applicable

Prices remain decimal strings or integer-scaled values. No floats in hashed structures.

### Coverage On Facts

Near-term: embed coverage and source status on each snapshot fact (typed enums above).

Later discovery may add dedicated coverage/control facts (searched ranges, standards, watermarks)
when search proof is independent of one holdings snapshot.

## Fact Visibility

Holding, price, metadata, and coverage **data** facts default to public `Platform` audience.

Progressive control cursors are Control audience and are not the public portfolio product.

Descriptor field exposure remains deliberate: returnable, query-only, hidden.

The portfolio report public output remains the curated portfolio view. Public facts are the reusable
observation substrate behind that view.

## Phased Plan

Phases are product risk order. Packaging renames are not phases.

### Phase 1: Platform Fact Read + Fact-Backed Report Cutover

Single public-meaning arc (may be multiple tightly sequenced PRs; no dual public truth between
them):

1. Extend fact-index read for Platform audience with Control-parity evidence (receipt, trust root,
   selection evidence, response artifact hydration, replay verifier).
2. Implement report graph: resolve subjects → (optional placeholder valuations) → Platform fact
   queries → normalize → assemble → project.
3. Certified pin table + subject projection + selection policy + fail-closed error codes.
4. Fixture/seed admission path for dual-mainnet-shaped facts in tests/CI.
5. Delete live `PinViews`, `ObserveBatch`, and portfolio-only live pin/balance adapter paths.
6. Public `portfolio_snapshot` means report-only.

Valuation placeholders remain explicit until price facts land.

### Phase 2: Configured Asset Collectors

- Add `bitcoin.address_balance_snapshot` collector (snapshot pattern, rule A anchor).
- Add EVM native and configured ERC-20 balance collectors.
- Embed typed coverage/source status; no control cursors for these snapshot re-reads.
- Extract shared fact-record role/runners when the second writer needs them; delete BTC-bound
  record authority.
- Replay tests: collectors do not call live providers during replay.
- Document multi-run full-refresh recipe (collect then report).
- Family op packaging (`btc-collectors-op`, `evm-collectors-op`) when second cycles land.

### Phase 3: Price And Metadata Facts

- Price snapshot facts and collector.
- Token metadata when needed for display/decimals.
- Move valuation from placeholders to fact-selected prices.
- Define hash-defining freshness policy relative to pins/as-of (not ambient process time).

### Phase 4: Discovery

- Progressive discovery collectors with family-local cursors.
- Dedicated coverage/control facts when needed.
- Explicit discover asset-universe policy.
- New planning boundary when discovered assets change report topology.

## Closed Decisions

| Question | Decision |
|---|---|
| Where do holding facts live? | Family state crates. Not `portfolio-facts`. |
| Platform fact-index for report? | Extend existing fact-index read for Platform + evidence parity with Control. |
| Public-facts API as report authority? | No. |
| Report pin model? | Hash-defining pin table in report config; store frontier is query evidence only. |
| Subject join? | Pure portfolio projection table (above). |
| Fail-closed errors? | Stable codes: missing_fact, unacceptable_coverage, unacceptable_source_status, pin_mismatch, ambiguous_facts, unsupported_requirement. |
| Snapshot anchor? | Rule A: observe returns pin + balances in one response. |
| Public `portfolio_snapshot`? | Report-only after cutover. |
| Composition? | External multi-run recipe; not required dual-IO certified op. |
| Live PinViews + ObserveBatch? | Deleted at public cutover. No dual path. |
| Near-term BTC fact? | `bitcoin.address_balance_snapshot` (total + anchor + coverage). |
| One op crate per collector? | No. Family op crates. |
| Mega collector framework? | No. Two patterns as review heuristics. |
| Always 4-node cycle? | No. Snapshot is 2-node. |
| Shared soup checkpoint type now? | No. |
| Coverage vocabulary? | Typed enums + write/accept matrix (above). |
| Configured-mode freshness? | Pin/subject/coverage only until price phase. |
| Fixture admission? | FactRecorded + descriptor admission (or equivalent authority). |

## Open Questions

- How should freshness policy be represented in canonical portfolio config when price facts land
  (global vs per-kind vs per-source)? Encoding only; basis must remain pin/as-of / fact-carried
  time, not ambient process clock.
- What is the first supported token discovery provider: local Reth/Erigon, hosted indexed data, or a
  provider-neutral log capability?
- When (if ever) is full Bitcoin UTXO set retention required, and should large sets use a separate
  content-addressed artifact?
- Exact packaging of shared fact runners: `adapters/fact` vs adapter-internal kit.

## Verification Expectations

Critical path (Phase 1 cutover):

- report consumes only Platform facts with recorded query evidence
- report fails if live chain providers are unavailable
- missing / unacceptable coverage / pin mismatch errors for configured symbols
- replay without live fact-index or chain reads
- no live PinViews/ObserveBatch registration for portfolio report after cutover
- fixture facts admitted through real fact authority, not raw index poking
- no-secret persisted/public surfaces

When collectors land (Phase 2):

- snapshot collector replay without live source reads
- progressive cursor non-regression (existing + new progressive collectors)
- fact descriptor exposure for public holding facts
- multi-run collect-then-report recipe works for dual-mainnet shape

Final merge-readiness continues to use the repository's normal Nixfied gates.
