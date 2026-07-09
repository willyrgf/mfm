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
  -> durable source-near Platform data facts (each with an explicit anchor)
     (+ Control cursors only when progressive)
  -> portfolio report Operation (Platform fact-index only)
  -> PortfolioSnapshot + PortfolioReport public outputs
     (network_pins projected from selected fact anchors)
```

Collectors produce reusable facts. Portfolio reporting consumes facts only.

**Time / as-of lives on the fact.** Every holding (and later price) fact carries an explicit anchor
(chain height/hash, oracle timestamp, or equivalent). The report does not maintain a separate pin
table for near-term configured mode. Public `network_pins` are projected from the anchors of the
facts the report selected.

Public surfaces after cutover:

| Surface | Meaning |
|---|---|
| `portfolio_snapshot` | fact-backed report only |
| collector entry points / internal ops | source IO that records facts |
| CLI/docs "full refresh" | external multi-run orchestration (collect, then report), not a dual-IO certified op |

This RFC is a breaking redesign on a breaking-change branch. Dual live+facts paths, silent
fallbacks, and crate-per-collector scaffolding are rejected. Live portfolio pin and observe code is
deleted when the fact-backed report lands. Delivery is **one PR** with progressive **commits**; git
history is recovery.

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

- it should select facts by subject, coverage, and certified "latest acceptable" policy
- it should materialize fact response artifacts
- it should normalize facts into the existing portfolio `Observation` view (including anchors)
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
not only balance reads. Live pins are replaced by **anchors on selected facts**, not by a second
live pin path and not by a parallel pin-table concept unless a later as-of filter is added.

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
- Add durable source-near facts for holdings, prices, metadata, and coverage; each data fact carries
  an explicit time/chain anchor.
- Make missing, stale, incomplete, or unsupported source data explicit report **run** failures.
- Use recorded fact-query evidence for report replay.
- Keep chain/indexer/oracle IO in collectors, adapters, transports, and provider backends.
- Keep operation crates deterministic and states free of ambient IO.
- Use two collector control patterns without inventing a collector framework.
- Organize collectors by family monocrates so growth is O(families), not O(collectors).
- Prefer fewer concepts: anchors on facts, not a separate report pin authority for near-term mode.
- Support configured symbols before open-ended token discovery.
- Delete live portfolio pin and observe with the fact-backed report on this branch.
- Deliver as one PR with progressive commits; do not leave dual public truth between commits that
  land on the branch tip intended for merge.

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
- Do not introduce a separate report pin table for near-term configured mode when fact anchors
  already carry as-of.
- Do not ship soft-success snapshots with empty/zeroed required holdings under `configured_symbols`.

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
compile canonical fact query plan (from certified report config + subject projection)
  -> FactIndexRead (Platform audience)
  -> authenticated receipt + trust root + selection evidence
  -> hydrate retained FactResponse artifact(s)
  -> validate selection cardinality and acceptability
  -> normalize into Observation (including fact anchor)
```

Replay rules:

- replay uses recorded fact-query evidence and retained response artifacts only
- replay must not re-query the live index frontier and accept newer facts
- public-facts API and ambient store SQL are forbidden as semantic authority for report states

Hydration is mandatory for report holding facts: receipt alone is not enough when the balance
payload lives in the retained response artifact.

Who may call Platform fact-index in certified runs:

- portfolio report states (required)
- other certified readers only when they have the same evidence obligations
- collectors must not Platform-query prior portfolio holdings (review policy; observe is source IO)

Phase 1 also extracts a shared fact-index query path (audience-agnostic request, hydrate, evidence)
so portfolio does not copy the BTC Control runner. Packaging may stay in an existing adapter tree
temporarily; the shared functions must exist on the branch before report cutover commits.

### 2. Anchors On Facts (As-Of Model)

Two different concepts must not be conflated:

| Concept | Role |
|---|---|
| Store read frontier (projection generation, commit watermark) | Authenticates **which index snapshot** the query read; bound into the query receipt |
| Fact anchor | Source-near **chain/oracle as-of** recorded on every data fact response |

**Decision (fewer concepts):** near-term configured reports do **not** use a separate hash-defining
report pin table. Time relation lives on the fact.

#### Anchor requirements (every Platform holding / price data fact)

- Anchor material is part of the fact **response** (content-addressed with the claim).
- Chain holdings: height/block number and block hash when the provider can prove it; height alone is
  allowed only when hash is unavailable and the fact kind documents that.
- Price facts (later): source timestamp or on-chain oracle anchor, never process wall clock alone.
- Observe normalize fails closed before write if anchor material is missing for a kind that requires
  it.

#### Report selection (near-term `configured_symbols`)

Named selection policy (hash-defining fixed digest in report states):

```text
policy_id = mfm.portfolio.holding.latest-acceptable.v1

for each required portfolio holding:
  query Platform facts matching subject predicates (projection table)
  filter coverage ∈ allow-list AND source_status ∈ allow-list
  order by:
    result.anchor_height_or_block.desc
    then store_commit_order.desc
    then fact_claim_id.desc
  accept first row (limit 1 after ordered acceptable candidates)

if zero acceptable candidates -> run fails missing_fact
if plan/receipt violates cardinality after policy -> run fails ambiguous_facts
```

Acceptability filters are applied in the ordered candidate scan (or as query predicates that preserve
the same total order). Do **not** take raw latest-by-height with `limit=1` and then reject coverage,
or an older acceptable fact will be missed.

#### Public `network_pins` projection (single rule)

- Project **only** from selected holding facts' anchors.
- One pin per network that appears in selected required holdings.
- If multiple selected holdings share a network but disagree on anchor height/hash → run fails
  `inconsistent_network_anchors`.
- If all selected holdings on a network share the same anchor → that becomes the network pin.
- Do **not** invent pins from live chain head.
- Do **not** maintain a parallel pin table that can desync from selected facts.

Observations also carry the per-holding anchor from the selected fact (existing
`ExecutionAnchor`-shaped material on the report DTO path).

#### Multi-run full refresh (no pin handoff)

```text
collectors write Platform facts with observed anchors
  -> portfolio_snapshot selects latest acceptable facts
  -> network_pins projected from those anchors
```

There is no collect→pin_table→report glue. Collectors and report share the store's fact index only.

#### Optional later: as-of upper bound

A future certified report config field may filter `anchor ≤ as_of` per network or globally. That is
an **optional filter on fact anchors**, not a second pin authority. Not in near-term cutover.

#### Configured mode freshness (near-term)

- no wall-clock staleness checks until price/metadata phase
- acceptability is subject match + coverage allow-list + source status allow-list + selection policy
- `generated_at_ms` on public output, if retained, must not become outcome-affecting ambient policy

### 3. Subject Projection (Portfolio Requirement → Fact Query)

Mapping is pure portfolio report logic. Source-near facts never embed `wallet_id` or `symbol_id`
unless those are true source identity (they are not).

**Near-term cutover scope (dual-mainnet natives):**

| Portfolio requirement | Fact kind | Predicate fields | Coverage accept list |
|---|---|---|---|
| BTC native configured wallet+symbol | `bitcoin.address_balance_snapshot` | `network`, `bitcoin_network`, `semantic_source_identity`, `address` | `configured_only`, `complete_at_anchor` |
| EVM native configured account+symbol | `evm.address_native_balance_snapshot` | `network`, `chain_id`, `account` | `configured_only`, `complete_at_anchor` |

EVM ERC-20 is **out of cutover scope**. When it lands later:

| Portfolio requirement | Fact kind | Predicate fields | Coverage accept list |
|---|---|---|---|
| EVM ERC-20 configured token set | `evm.address_token_balances_snapshot` | `network`, `chain_id`, `account`, reader/policy id, `token_set_identity` | `configured_only`, `complete_at_anchor` |

`token_set_identity` (later): JCS-SHA256 of canonical sorted unique token addresses (plus chain_id /
network as required). Writer and reader call the same pure function; ownership is the family subject
builder + portfolio projection helper sharing that pure function.

Projection inputs come from portfolio config. Fact subjects stay source-near. Report joins back to
`wallet_id` / `symbol_id` only when building `Observation` DTOs.

### 4. Selection Policy And Fail-Closed Errors

For each configured requirement, the certified report plan fixes:

- fact kind
- subject predicates (from projection table)
- selection policy id (`mfm.portfolio.holding.latest-acceptable.v1`)
- coverage allow-list
- source status allow-list

These are hash-defining certified config or fixed policy digests. They are not adapter defaults.

**v1 `configured_symbols` is hard-fail only.** Any required-symbol selection or normalization
failure fails the **state/run**. Do not emit a successful public snapshot with empty wallets or
zeroed required holdings. Soft `PortfolioSnapshot.errors` partial success is **out of scope** until
a real partial-success product requirement exists.

Stable near-term run failure codes (map to CLI/public error surface):

| Code | When |
|---|---|
| `missing_fact` | no acceptable Platform fact for a required subject |
| `unacceptable_coverage` | only candidates failed coverage allow-list (if distinguished before empty set) |
| `unacceptable_source_status` | only candidates failed source status allow-list |
| `ambiguous_facts` | selection cardinality violated after policy |
| `inconsistent_network_anchors` | selected holdings on one network disagree on anchor |
| `unsupported_requirement` | portfolio requirement has no projection rule |

Prefer collapsing filter-empty outcomes to `missing_fact` when simpler; keep distinct codes only when
the failure path is unambiguous. Do not use a `pin_mismatch` code without a separate pin table.

Valuation placeholders: fixed unit prices (including intentional dual-mainnet zeros) are not
selection failures. View-dependent valuation readers (`DirectPrice`) are config-rejected until price
facts land.

### 5. Snapshot Collector Observe Anchor (Phase 2 writers)

Snapshot pattern: `ObserveSource → RecordDataFact`.

**Decision:** the collector's observe path produces **one observation DTO** that includes anchor +
balances + coverage + source status. That material is what becomes the Platform data fact.

How the observe state obtains the tip may be adapter multi-cap composition (e.g. head read + balance
at tip) or a combined capability. That is Phase 2 packaging. The fact always records the joint
observation anchor.

Note on current BTC API: `BtcBalanceReadRequest` is pin-in today; Phase 2 observe may resolve tip
inside the same observe state, then read balance at that tip, then emit one observation. Report never
supplies pins.

### 6. Public Entry And Composition Model

| Surface | Meaning after cutover |
|---|---|
| `portfolio_snapshot` | **report-only**: resolve subjects, query Platform facts, project report |
| collector ops | source IO only; record Platform data facts (+ Control cursors if progressive) |
| full refresh | **external** multi-run recipe: run required collectors, then run `portfolio_snapshot` |

A single certified draft that mixes collectors and report is optional later. Not required for
cutover. Kernel multi-run orchestration is not a new authority layer; CLI/docs/app recipes are
non-authority glue.

Discovery that changes required symbols always creates a new planning boundary / new certified run.

### 7. Cutover Gate And Branch Delivery

This branch is a **breaking-change branch**. Work lands as **one PR** to `dev` after the tip is
coherent. Divide work by **commit**, not by PR.

**No dual public truth on the branch tip intended for merge.** Intermediate commits may break
portfolio temporarily while the branch is WIP; the merge tip must not keep live pin/observe as a
fallback beside facts.

**Merge-tip cutover requires all of:**

1. Platform fact-index read + evidence works for report states.
2. Minimal holding fact **schemas** exist for dual-mainnet natives (Phase 1 owns schemas; collectors
   later are writers).
3. Fixture/seed admission via real `FactRecorded` + descriptor admission (test/CI path).
4. Fact-backed report graph replaces pin+observe.
5. Live portfolio pin/observe paths and portfolio-only live balance/head transports are deleted.
6. Public `portfolio_snapshot` means report-only.
7. Portfolio integration/parity tests rewritten off live Reth crawl (fixtures + collectors when
   present).

**Product honesty:** after cutover, `portfolio_snapshot` is useless without admitted facts.
Dual-mainnet demo requires either fixture seed (CI) or collectors (operator). Prefer landing
configured collectors on the same branch before merge so the tip is usable end-to-end; if collectors
lag, document that explicitly and keep CI fixture-based.

### 8. Coverage And Source Status Vocabulary

Typed closed enums with a single pure owner. Near-term report-accept set is minimal.

**CoverageStatus**

| Value | Meaning | May write Platform fact? | Report accept? |
|---|---|---|---|
| `complete_at_anchor` | source proved completeness for the claimed universe | yes | yes |
| `configured_only` | only configured assets; wallet discovery incomplete by design | yes | yes |
| `truncated` | source truncated / incomplete pagination | **prefer fail closed before write** for near-term collectors | no |
| `incomplete` | known incomplete observation | **prefer fail closed before write** for near-term collectors | no |

**SourceStatus**

| Value | Meaning | May write Platform fact? | Report accept? |
|---|---|---|---|
| `ok` | observation succeeded | yes | yes |
| `unsupported` | source cannot serve this subject | no — fail closed before write | n/a |
| `failed` | source error | no — fail closed before write | n/a |

Configured collectors default to `configured_only`. Most JSON-RPC balance reads cannot claim
full-wallet completeness.

### 9. Near-Term Bitcoin Holding Fact

**Decision:** `bitcoin.address_balance_snapshot` (total sats + anchor + coverage + source status),
matching current provider truth (`scantxoutset`-class total balance). Full UTXO set retention is
deferred.

### 10. Report Graph After Cutover

Delete from portfolio report topology and adapters:

- `PinViewsState` live head/anchor reads
- `ObserveBatchState` live balance reads
- `PortfolioReadCapability` / transport-factory paths used only for live pin or balance crawl

Target shape:

```text
ResolveSubjects
  -> ResolveValuations          (pure, config-only fixed placeholders; no PinnedViews input)
  -> QueryHoldingFacts*         (Platform fact-index; static fanout per required symbol)
  -> MergeObservations
  -> AssembleSnapshot           (network_pins from selected fact anchors; hard-fail only)
  -> ProjectReport
```

`ResolveValuations`: pure, config-only. Drop `PinnedViews` dependency. `DirectPrice` /
view-dependent readers: config validation reject until price phase.

`AssembleSnapshot`: no live `views` input. Inputs are subjects, valuations, observation batch (with
anchors), and assembly config. Project `network_pins` per Contract 2.

### 11. Progressive Resume Invariants (Progressive Collectors Only)

For progressive collectors (not required for report cutover, but shape-wide when progressive lands):

1. Data fact records before control cursor advance.
2. Cursor advance derives only from certified observation/data-fact material + prior cursor.
3. Watermarks must not regress.
4. Observe is `ReadExternal` only; record is `ManagedPlatformWrite` only.
5. Snapshot re-reads may append multiple Platform facts for the same subject; report "latest
   acceptable" is totally ordered by the named selection policy.

### 12. Fixture And Test Admission

Facts used by report tests enter through `FactRecorded` + descriptor admission. Prefer a tiny
certified writer graph or a named `test_support` API that emits real claims, response artifacts, and
projections.

**Forbidden:** SQL inserts into `fact_index` / `fact_index_terms` without admission authority.

### 13. Phase 1 Owns Minimal Fact Schemas

Cutover fixtures and report selection require fact kinds before collectors exist. **Schemas are
Phase 1 work**, not Phase 2:

- `bitcoin.address_balance_snapshot` in `states/btc` (or module split thereof)
- `evm.address_native_balance_snapshot` in `states/evm` (new crate when first EVM fact lands)

Collectors (later commits on the same branch) are writers of those kinds. They do not own the first
definition.

---

## Core Decision

### Facts Are Source-Near And Anchored

Portfolio facts record bounded observations close to the source truth, including when they were
true. The report operation converts selected facts into the existing portfolio model.

Examples:

- a Bitcoin balance snapshot fact records the address, **anchor**, total sats, and coverage
- an EVM native balance fact records the account, **block anchor**, raw wei, decimals, and coverage
- an EVM token balances fact records the account, **block anchor**, selected token balances, and
  coverage
- a price snapshot fact records the priced symbol, quote, source id, **anchor/time**, price decimal

The report-facing `Observation` stays a projection:

```text
holding facts (+ later price facts) + symbol config + valuation config
  -> Observation (including per-holding anchor)
network_pins
  <- unique per-network anchors from selected holdings
```

Holdings facts live in family state crates (`states/btc`, `states/evm`, later `states/price`).
They do **not** live in a `portfolio-facts` crate. Portfolio owns report query, selection,
normalization, and assembly only.

### Collectors Own Source IO

Near-term collectors:

- `btc_chain_head_collector` (exists; progressive template)
- `bitcoin_address_balance_collector` (snapshot re-read; total balance)
- `evm_account_balance_collector` (snapshot re-read)
- `evm_token_balance_collector` (snapshot re-read; after cutover natives)

Later collectors:

- `evm_token_discovery_collector` (progressive)
- `token_metadata_collector`
- `price_collector` (usually snapshot re-read)
- protocol position collectors
- optional later: full UTXO snapshot if analytics require it

## Collector Patterns (Minimal)

The shared platform is already the framework. Collectors must not invent a second one.

Two control patterns exist as review heuristics, not a framework crate or shape enum.

### Progressive (cursor / watermark)

```text
QueryControlCursor -> ObserveSource -> RecordDataFact -> RecordControlCursor
```

| Role | Effect | Capability | Audience |
|---|---|---|---|
| QueryControlCursor | `ReadExternal` | Fact-index read (Control) | reads Control |
| ObserveSource | `ReadExternal` | domain source read | none yet |
| RecordDataFact | `ManagedPlatformWrite` | shared fact-record role | Platform |
| RecordControlCursor | `ManagedPlatformWrite` | shared fact-record role | Control |

Existing `btc_chain_head_collector` is the progressive template.

### Snapshot re-read (configured balances / prices)

```text
ObserveSource (anchor + balances in one observation DTO)
  -> RecordDataFact
```

Coverage and source status live on the data fact. Control cursors are not required.

### Shared Vs Specialized

| Share when a second consumer needs it | Specialize (monomorphic) |
|---|---|
| Platform + Control fact-index query runner | data fact types / descriptors |
| Managed fact-record runner (second writer) | progressive cursor types |
| Fact-record capability role | observe config / normalize |
| Coverage / source status enums | domain read caps, transports, family op expand |

Copy-paste thin Operation `expand` until a third real duplicate hurts.

### Explicit Non-Abstractions

1. No generic collector mega-trait.
2. No cross-family holdings soup types.
3. No unified BTC+EVM capability.
4. No soup checkpoint type with optional chain fields.
5. No observe-and-record merged effect class.
6. No dynamic topology from discovery inside one certified run.
7. No ambient source routing in states/ops.
8. No mega collector framework crate.
9. No Platform fact-index reads inside collectors for prior portfolio balances.
10. No dual portfolio live-read path.
11. No separate near-term report pin table parallel to fact anchors.
12. No public-facts CLI/REST as certified report authority.
13. No soft-success zeroed required holdings under `configured_symbols`.
14. No wall-clock staleness as uncertified ambient policy.

### Capability Matrix

| Need | Capability class |
|---|---|
| Source observation | family domain read caps |
| Progressive cursor load | fact-index read (Control) |
| Record data / cursor facts | shared managed fact-record role |
| Portfolio report selection | fact-index read (Platform) + recorded evidence |

## Repository Organization

Organize by durable family and layer monocrates, not by collector recipe name.

### Placement Rules

1. Graph planning → family op crate module / `Operation` type.
2. `State::handle`, fact types, normalize → family state crate.
3. Provider authority → `*-capabilities`.
4. HTTP/RPC → `transports/*`.
5. Bind intent + evidence → `adapters/{protocol}`.
6. Report DTO / wallet config / valuation join → portfolio model/config/state/op.
7. Shared runner mechanics → shared kit, not copy-paste.
8. Workflow names stop at ops.
9. New collector = module + Operation + states + adapter registration lines.
10. Portfolio crates must not grow source-near holding fact types.
11. Collector ops must not depend on portfolio report DTOs.

### Target Tree (Guidance, Not A Phase Gate)

```text
crates/
  fact-capabilities/          # Control + Platform fact-index; shared fact-record role when needed
  states/btc/                 # chain-head + address balance facts
  states/evm/                 # native (+ later token) balance facts
  states/portfolio/           # report only
  ops/btc-collectors-op/      # when second BTC cycle lands
  ops/evm-collectors-op/      # when EVM collectors land
  ops/portfolio-tracker-op/   # fact-backed report only after cutover
  adapters/btc-jsonrpc/
  adapters/evm/               # when EVM collectors land
  adapters/portfolio/         # Platform fact-index + pure report only after cutover
```

Non-critical packaging when cheap: rename chain-head op crate, rehome `collectors/proof`, extract
`adapters/fact`.

### Deletes At Cutover

| Delete | Why |
|---|---|
| Live `PinViewsState` | live chain IO; replaced by fact anchors |
| Live `ObserveBatchState` | live chain IO |
| Portfolio-only live pin/balance transports | dual truth |
| Soft-success zeroed required holdings | false completeness |
| Dual-mode "live if facts missing" | dual truth |

## Asset Universe Policy

Near-term: only configured symbols matter. Missing/unacceptable facts for required symbols fail the
run. Unconfigured tokens are out of scope. The report must not claim complete wallet discovery.

Do not invent `asset_universe.mode` TOML until discovery work starts. Configured-symbols is the only
report mode until then.

## Recommended Fact Families

### `bitcoin.address_balance_snapshot` (cutover)

Subject: `network`, `bitcoin_network`, `semantic_source_identity`, `address`  
Response: anchor height, optional anchor hash, total sats, coverage, source status

### `evm.address_native_balance_snapshot` (cutover)

Subject: `network`, `chain_id`, `account`  
Response: block number, optional block hash, raw wei decimal string, decimals, coverage, source
status

### `evm.address_token_balances_snapshot` (post-cutover)

Subject: `network`, `chain_id`, `account`, reader/policy id, token_set_identity  
Response: block anchor, token balance entries, coverage, source status

### `price.quote_snapshot` (later)

Subject: `priced_symbol_id`, `quote`, `source_id`  
Response: unit price decimal string, anchor/time, freshness metadata, source status

No floats in hashed structures.

## Fact Visibility

Holding/price data facts default to public `Platform` audience. Progressive control cursors are
Control audience. Descriptor field exposure remains deliberate (returnable / query-only / hidden).

## Implementation Plan (One PR, Progressive Commits)

Branch delivery: **one PR**. Sequence by **commit**. Keep the eventual merge tip free of dual public
truth. Intermediate WIP commits may break portfolio while the branch is open.

### Commit series A — Platform fact-index substrate

- Extend `FactIndexReadRequest` for Platform audience (or audience allow-list).
- Evidence/receipt/trust-root/hydrate/replay parity with Control.
- Shared query helper extracted so portfolio will not reimplement BTC Control runner.
- Tests: Platform query, replay without live re-query; no public-facts authority.

### Commit series B — Minimal holding fact schemas + fixtures

- Define `bitcoin.address_balance_snapshot` and `evm.address_native_balance_snapshot` (types,
  descriptors, Platform visibility, anchors, coverage/source enums accept set).
- Fixture admission via real `FactRecorded` path / certified tiny writer / named test_support.
- Pure normalize helpers: fact → `Observation` (including anchor).

### Commit series C — Fact-backed report graph + delete live crawl

- Subject projection + selection policy `latest-acceptable.v1`.
- Report topology: subjects → pure valuations → Platform fact queries → merge → assemble → project.
- `network_pins` from selected fact anchors; hard-fail only.
- Delete `PinViews`, `ObserveBatch`, portfolio live pin/balance transports.
- Public `portfolio_snapshot` = report-only.
- Rewrite portfolio integration/parity tests to fixtures.

### Commit series D — Configured collectors (same branch, preferred before merge)

- BTC address balance snapshot collector (observe → record).
- EVM native balance collector.
- Shared fact-record role/runners when second writer needs them; delete BTC-bound record authority
  if still present.
- Collector replay tests; document multi-run collect-then-report recipe.
- Optional: ERC-20 configured token collector after natives.

### Commit series E — Later on this or follow-up work

- Prices/metadata, freshness policy encoding, discovery, packaging renames.

## Closed Decisions

| Question | Decision |
|---|---|
| Where do holding facts live? | Family state crates. Not `portfolio-facts`. |
| Platform fact-index for report? | Extend existing fact-index read for Platform + evidence parity with Control. |
| Public-facts API as report authority? | No. |
| Separate report pin table (near-term)? | **No.** As-of is the fact anchor. |
| How do `network_pins` form? | Projected only from selected holding fact anchors; fail on inconsistency. |
| Selection policy? | `mfm.portfolio.holding.latest-acceptable.v1` (order + acceptability + limit 1). |
| Fail-closed? | Hard-fail run for required symbols; no soft-success partial snapshots in v1. |
| Snapshot observe? | One observation DTO with anchor + balances; Phase 2 packaging for multi-cap. |
| Public `portfolio_snapshot`? | Report-only after cutover. |
| Composition? | External multi-run recipe; no pin-table handoff. |
| Live PinViews + ObserveBatch? | Deleted at cutover. |
| Near-term BTC fact? | `bitcoin.address_balance_snapshot`. |
| Who owns cutover fact schemas? | Phase 1 / commit series B. Collectors are writers. |
| Cutover ERC-20? | Out of scope; natives only. |
| Delivery shape? | One PR, progressive commits. |
| Mega framework / crate-per-collector / always-4-node? | No. |

## Open Questions

- How should freshness policy be represented when price facts land (global vs per-kind vs
  per-source)? Basis remains fact-carried anchor/time, not ambient process clock.
- Optional certified as-of upper bound filter on anchors (encoding only; not a second pin authority).
- First token discovery provider: local Reth/Erigon, hosted index, or provider-neutral logs.
- When (if ever) full Bitcoin UTXO retention is required.
- Exact packaging of shared fact runners (`adapters/fact` vs internal kit).

## Verification Expectations

On the merge tip of this branch:

- report consumes only Platform facts with recorded query evidence
- report fails if live chain providers are unavailable
- missing / unacceptable / inconsistent-anchor failures for required symbols
- replay without live fact-index or chain reads
- no live PinViews/ObserveBatch registration for portfolio report
- fixture facts admitted through real fact authority
- `network_pins` match selected fact anchors
- no-secret persisted/public surfaces
- when collectors land: collect-then-report dual-mainnet recipe works; collector replay without live
  source

Final merge-readiness uses the repository's normal Nixfied gates.
