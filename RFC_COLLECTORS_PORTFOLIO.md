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
  -> portfolio report Operation
     (Platform fact-index only; certified selection policy)
  -> PortfolioSnapshot + PortfolioReport public outputs
     (network_pins always projected from selected fact anchors; never a pin table)
```

Collectors produce reusable facts. Portfolio reporting consumes facts only.

**As-of is a single authority model (long-term and near-term):**

1. Every Platform data fact carries its own source-near anchor (chain height/hash, oracle/time, etc.).
2. The certified report selects facts under a named selection policy family (`latest-acceptable` now;
   optional `at-or-before` bounds later as **filters**, not a second pin store).
3. Public `network_pins` are **always** a pure projection of selected required holding anchors —
   descriptive output, not selection authority and not report config.
4. Same-network selected holdings must agree on anchor or the run fails.
5. No report pin table. No pin-facts as report authority. No live `PinViews`.

```text
collectors: observe(source @ joint anchor) → Platform data facts
report:     certified policy → select facts → Observations + network_pins(from selected anchors)
replay:     recorded fact-query evidence only
```

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

- it should select facts by subject, coverage, and a certified selection policy over fact anchors
- it should materialize fact response artifacts
- it should normalize facts into the existing portfolio `Observation` view (including anchors)
- it should produce stable public outputs (`network_pins` as projection of selected anchors)
- replay should use recorded fact-query evidence, not live IO

Putting both lifecycles in one state graph creates a workflow that is hard to certify, hard to
resume, and hard to explain.

### Live Portfolio Graph Is Dual IO Today

Current topology:

```text
ResolveSubjects
  -> PinViews          (live chain head / execution anchor)
  -> ResolveValuations
  -> ObserveBatch*     (live balances at those pins)
  -> MergeObservations
  -> AssembleSnapshot  (network_pins from live views)
  -> ProjectReport
```

Both `PinViews` and `ObserveBatch` are live chain IO. Cutover deletes **both**.

**Migration of as-of authority:**

| Era | How as-of works | How `network_pins` form |
|---|---|---|
| Live today | `PinViews` live head → observe at pin | From live `PinnedViews` |
| Cutover / long-term | Anchors on Platform data facts + certified selection policy | **Always** projected from selected holding fact anchors |

There is no intermediate dual path where pins come from either config or facts. Optional historical
as-of is later expressed as selection **bounds** (filters), not as a pin table that owns
`network_pins`.

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
- Use a **single long-term as-of authority**: fact anchors + certified selection policy;
  `network_pins` are projection only.
- Make missing, stale, incomplete, or unsupported source data explicit report **run** failures.
- Use recorded fact-query evidence for report replay.
- Keep chain/indexer/oracle IO in collectors, adapters, transports, and provider backends.
- Keep operation crates deterministic and states free of ambient IO.
- Use two collector control patterns without inventing a collector framework.
- Organize collectors by family monocrates so growth is O(families), not O(collectors).
- Prefer fewer concepts: no report pin table, no pin-facts as report authority, no dual pin paths.
- Support configured symbols before open-ended token discovery.
- Delete live portfolio pin and observe with the fact-backed report on this branch.
- Deliver as one PR with progressive commits; do not leave dual public truth on the merge tip.

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
- Do not introduce a separate report pin table (near-term or long-term) parallel to fact anchors.
- Do not use progressive chain-head / pin-facts as portfolio report pin authority.
- Do not treat public `network_pins` as operator-chosen or config-authoritative as-of.
- Do not require a global multi-chain block identity for portfolio truth (independent L1s yield
  independent pins).
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
compile canonical fact query plan (from certified report config + subject projection + policy)
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

Commit series A extracts a shared fact-index query path (audience-agnostic request, hydrate,
evidence) so portfolio does not copy the BTC Control runner. Packaging may stay in an existing
adapter tree temporarily; the shared functions must exist on the branch before report cutover
commits.

### 2. Anchors On Facts (As-Of Model) — Single Authority

Two different concepts must not be conflated:

| Concept | Role | Authority for as-of? |
|---|---|---|
| Store read frontier (projection generation, commit watermark) | Authenticates **which index snapshot** the query read; bound into the query receipt | Query evidence only |
| Fact anchor | Source-near chain/oracle as-of on every Platform data fact response | **Yes — claim content** |
| Selection policy | Hash-defining rule choosing which acceptable fact wins per requirement | **Yes — certified** |
| Optional as-of bounds | Certified **filters** inside selection policy (later) | Yes as policy params, **not** as pins |
| Public `network_pins` | Projection of selected required holding anchors | **No — descriptive output only** |
| Progressive chain-head / Control cursor facts | Collector ops | **Not** report pin authority |

**Decision (long-term and near-term):** there is **one** as-of authority for report truth:
**anchors on selected facts**, under a certified selection policy.

- No separate hash-defining report pin table.
- No pin-facts as report pin authority.
- No live `PinViews`.
- Progressive chain-head / Control cursor facts remain collector ops only.
- Optional historical bounds are parameters of the **same selection policy family**, not a parallel
  path where `network_pins` may come from either config or facts.

```text
collectors: observe(source @ joint anchor) → Platform data facts
report:     certified policy → select facts → Observations + network_pins(from selected anchors)
replay:     recorded fact-query evidence only
```

#### Anchor requirements (every Platform holding / price data fact)

- Anchor material is part of the fact **response** (content-addressed with the claim).
- Chain holdings: height/block number **and** block hash when the provider can prove it; height alone
  only when the fact kind documents that hash is unavailable.
- Price facts (later): source timestamp or on-chain oracle anchor; never process wall clock alone as
  the sole anchor.
- Observe normalize **fails closed before write** if required anchor material is missing.
- Hashed structures: no floats (integer heights; decimal strings for quantities/prices).

#### Report selection — policy family

**Near-term cutover (required):**

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

Acceptability filters apply in the ordered candidate scan (or as query predicates that preserve the
same total order). Do **not** take raw latest-by-height with `limit=1` and then reject coverage, or
an older acceptable fact will be missed.

**Long-term extension (same authority model; not dual path):**

```text
policy_id = mfm.portfolio.holding.at-or-before.v1
  + certified per-network (and later price-axis) upper bounds

same ordering/acceptability as latest-acceptable, plus:
  fact.anchor ≤ bound (numeric height/block or oracle time; family-defined comparison)
```

Optional later strict mode: selected anchor must equal bound or fail `as_of_not_exact`. Default is
upper-bound filter with **actual** selected anchors projected to `network_pins` (honest when only
older facts exist). Bounds do not invent missing history; missing data still fails `missing_fact`.

Recollect-at-bound is a **collector** recipe (observe at tip/bound, write facts, then report). Report
stays facts-only and never live-pins.

#### Public `network_pins` projection (single rule forever)

- Project **only** from selected **required holding** facts' anchors.
- One pin per network that appears in those selections.
- If multiple selected holdings share a network but disagree on anchor height/hash (per kind rules)
  → run fails `inconsistent_network_anchors`.
- If all selected holdings on a network share the same anchor → that becomes the network pin.
- Do **not** invent pins from live chain head, report config, or chain-head facts.
- Do **not** maintain a parallel pin table that can desync from selected facts.
- Do **not** use `network_pins` as input to selection (no feedback loop).
- Observations also carry the per-holding anchor from the selected fact (existing
  `ExecutionAnchor`-shaped material on the report DTO path).

#### Multi-chain coherence (normative honesty)

- **Per-network coherence is certified:** same-network required holdings share one anchor or the run
  fails.
- **Cross-network simultaneity is not a single block.** Independent networks yield independent pins.
  That is correct, not a bug.
- Full-refresh orchestration may reduce tip skew operationally; it is **not** a certified pin
  authority layer.

#### Reorgs and tip race

| Hazard | Handling |
|---|---|
| Tip race same network (collect A at H, B at H+1) | Report fails `inconsistent_network_anchors`. Collectors that write multiple subjects on one network **SHOULD** share one observe tip (batch joint anchor). |
| Tip race across networks | Allowed; pins differ by network. |
| Reorg after fact write | Facts remain historical observations. New collectors write superseding higher/finalized anchors. Report latest-acceptable moves forward. Do not silently rewrite old facts. |
| Same height, different hash | Fail closed when projecting network pins / mixing fork siblings; prefer kinds that require hash. |
| Finality | Collectors may encode finality metadata on progressive facts. Report acceptability may later filter by fact-carried finality — never via live head checks in the report. |

#### Multi-run full refresh (no pin handoff)

```text
collectors write Platform facts with observed anchors
  -> portfolio_snapshot selects facts under certified policy
  -> network_pins projected from those anchors
```

There is no collect→pin_table→report glue. Collectors and report share the store's fact index only.

#### Prices / second time axis (same model, two anchor families)

Do **not** invent a second pin authority for prices.

- Holdings: chain anchors → feed `network_pins` and observation sources.
- Prices: oracle/source time anchors on `price.quote_snapshot` (later).
- Join is selection over price facts under a certified price policy (e.g. latest acceptable with
  `price.anchor ≤ certified_price_as_of`), never ambient `now()`.
- Price as-of appears on valuation/observation source refs, **not** as a `network_pins` entry unless
  the price is truly chain-anchored on that network.

One conceptual rule forever: **time lives on facts; report only selects and projects.**

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
- selection policy id (`mfm.portfolio.holding.latest-acceptable.v1` at cutover; later
  `at-or-before.v1` + bounds when product needs historical reports)
- coverage allow-list
- source status allow-list
- optional later: as-of bounds as policy parameters

These are hash-defining certified config or fixed policy digests. They are not adapter defaults.

**v1 `configured_symbols` is hard-fail only.** Any required-symbol selection or normalization
failure fails the **state/run**. Do not emit a successful public snapshot with empty wallets or
zeroed required holdings. Soft `PortfolioSnapshot.errors` partial success is **out of scope** until
a real partial-success product requirement exists.

Stable run failure codes (map to CLI/public error surface):

| Code | When |
|---|---|
| `missing_fact` | no acceptable Platform fact for a required subject |
| `unacceptable_coverage` | only candidates failed coverage allow-list (if distinguished before empty set) |
| `unacceptable_source_status` | only candidates failed source status allow-list |
| `ambiguous_facts` | selection cardinality violated after policy |
| `inconsistent_network_anchors` | selected holdings on one network disagree on anchor |
| `unsupported_requirement` | portfolio requirement has no projection rule |
| `as_of_not_exact` | **later only**, strict equality historical mode |

Prefer collapsing filter-empty outcomes to `missing_fact` when simpler; keep distinct codes only when
the failure path is unambiguous. Do **not** introduce `pin_mismatch` (that code implies a pin table).

Valuation placeholders: fixed unit prices (including intentional dual-mainnet zeros) are not
selection failures. View-dependent valuation readers (`DirectPrice`) are config-rejected until price
facts land.

### 5. Snapshot Collector Observe Anchor (Writers)

Snapshot pattern: `ObserveSource → RecordDataFact`.

**Decision:** the collector's observe path produces **one observation DTO** that includes anchor +
balances + coverage + source status. That material is what becomes the Platform data fact.

How the observe state obtains the tip may be adapter multi-cap composition (e.g. head read + balance
at tip) or a combined capability. Packaging detail. The fact always records the joint observation
anchor.

Collectors that write multiple subjects on one network **SHOULD** resolve tip once and share that
joint anchor across the batch so reports do not fail `inconsistent_network_anchors` from tip races.

Note on current BTC API: `BtcBalanceReadRequest` is pin-in today; observe may resolve tip inside the
same observe state, then read balance at that tip, then emit one observation. Report never supplies
pins.

### 6. Public Entry And Composition Model

| Surface | Meaning after cutover |
|---|---|
| `portfolio_snapshot` | **report-only**: resolve subjects, query Platform facts under selection policy, project report |
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
2. Minimal holding fact **schemas** exist for dual-mainnet natives (schemas first; collectors later
   are writers).
3. Fixture/seed admission via real `FactRecorded` + descriptor admission (test/CI path).
4. Fact-backed report graph replaces pin+observe with `latest-acceptable.v1`.
5. Live portfolio pin/observe paths and portfolio-only live balance/head transports are deleted.
6. Public `portfolio_snapshot` means report-only.
7. Portfolio integration/parity tests rewritten off live Reth crawl (fixtures + collectors when
   present).
8. `network_pins` projected only from selected fact anchors (single projection rule).

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
anchors), and assembly config. Project `network_pins` per Contract 2 single rule forever.

### 11. Progressive Resume Invariants (Progressive Collectors Only)

For progressive collectors (not required for report cutover, but shape-wide when progressive lands):

1. Data fact records before control cursor advance.
2. Cursor advance derives only from certified observation/data-fact material + prior cursor.
3. Watermarks must not regress.
4. Observe is `ReadExternal` only; record is `ManagedPlatformWrite` only.
5. Snapshot re-reads may append multiple Platform facts for the same subject; report selection is
   totally ordered by the named selection policy.

Progressive chain-head facts are **not** report pins. They stay collector/control ops.

### 12. Fixture And Test Admission

Facts used by report tests enter through `FactRecorded` + descriptor admission. Prefer a tiny
certified writer graph or a named `test_support` API that emits real claims, response artifacts, and
projections.

**Forbidden:** SQL inserts into `fact_index` / `fact_index_terms` without admission authority.

### 13. Phase 1 Owns Minimal Fact Schemas

Cutover fixtures and report selection require fact kinds before collectors exist. **Schemas first**;
collectors are writers:

- `bitcoin.address_balance_snapshot` in `states/btc` (or module split thereof)
- `evm.address_native_balance_snapshot` in `states/evm` (new crate when first EVM fact lands)

Collectors (later commits on the same branch) write those kinds. They do not own the first
definition.

### 14. Collectors Vs Report Config

| Collectors write | Report certifies |
|---|---|
| Joint observation DTO → Platform data fact | Subject projection table |
| Explicit anchor on every data fact | Selection policy id (+ optional bounds later) |
| Coverage + source status | Coverage / source status allow-lists |
| Progressive cursors (Control) when progressive | Fail-closed cardinality and consistency rules |
| Prefer shared tip for multi-subject same network | Public `network_pins` projection rule (fixed forever) |

Collectors **must not** Platform-query prior portfolio holdings for observe. Report **must not**
live-read chain heads or balances.

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
  <- unique per-network anchors from selected required holdings (projection only)
```

Holdings facts live in family state crates (`states/btc`, `states/evm`, later `states/price`).
They do **not** live in a `portfolio-facts` crate. Portfolio owns report query, selection,
normalization, and assembly only.

### Collectors Own Source IO

Near-term collectors:

- `btc_chain_head_collector` (exists; progressive template — **not** report pin authority)
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

Existing `btc_chain_head_collector` is the progressive template. Its Platform/Control outputs are
not report pin inputs.

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
11. No separate report pin table parallel to fact anchors (near-term or long-term).
12. No pin-facts as report selection authority (chain-head facts are collector ops only).
13. No dual rule where `network_pins` may come from either config or facts.
14. No public-facts CLI/REST as certified report authority.
15. No soft-success zeroed required holdings under `configured_symbols`.
16. No wall-clock staleness as uncertified ambient policy.
17. No fake global multi-chain block identity for portfolio truth.

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
| Live `PinViewsState` | live chain IO; replaced by fact anchors + selection policy |
| Live `ObserveBatchState` | live chain IO |
| Portfolio-only live pin/balance transports | dual truth |
| Soft-success zeroed required holdings | false completeness |
| Dual-mode "live if facts missing" | dual truth |
| Any report pin table field that owns `network_pins` | dual as-of authority |

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

Price anchors are a separate anchor family (oracle/source time). They join via certified price
selection, not via `network_pins` as fake chain pins.

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
  descriptors, Platform visibility, **mandatory anchors**, coverage/source enums accept set).
- Fixture admission via real `FactRecorded` path / certified tiny writer / named test_support.
- Pure normalize helpers: fact → `Observation` (including anchor).

### Commit series C — Fact-backed report graph + delete live crawl

- Subject projection + selection policy `latest-acceptable.v1` only.
- Report topology: subjects → pure valuations → Platform fact queries → merge → assemble → project.
- `network_pins` from selected fact anchors only; hard-fail inconsistency; hard-fail required misses.
- Delete `PinViews`, `ObserveBatch`, portfolio live pin/balance transports.
- Public `portfolio_snapshot` = report-only.
- Rewrite portfolio integration/parity tests to fixtures.
- **Do not** add report pin table fields or `at-or-before` yet (strict subset of long-term model).

### Commit series D — Configured collectors (same branch, preferred before merge)

- BTC address balance snapshot collector (observe → record; joint anchor).
- EVM native balance collector.
- Multi-subject same-network writers **SHOULD** share one observe tip.
- Shared fact-record role/runners when second writer needs them; delete BTC-bound record authority
  if still present.
- Collector replay tests; document multi-run collect-then-report recipe.
- Optional: ERC-20 configured token collector after natives.

### Commit series E — Later extensions (same model; no dual authority)

- Prices/metadata with oracle anchors; certified price selection join.
- Optional `at-or-before.v1` + certified bounds encoding (filters only; `network_pins` rule unchanged).
- Optional strict equality historical mode (`as_of_not_exact`).
- Freshness thresholds as certified policy over fact-carried times.
- Discovery; packaging renames.

Adding bounds later is a new policy id / config fields, **not** a rewrite of `network_pins`
semantics.

## Closed Decisions

| Question | Decision |
|---|---|
| Where do holding facts live? | Family state crates. Not `portfolio-facts`. |
| Platform fact-index for report? | Extend existing fact-index read for Platform + evidence parity with Control. |
| Public-facts API as report authority? | No. |
| Separate report pin table (any phase)? | **No.** As-of lives on fact anchors; optional later bounds are selection filters only. |
| How do `network_pins` form? | **Always** projected from selected required holding fact anchors; fail on same-network inconsistency. Never from config pin tables, live head, or pin-facts. |
| Is `network_pins` semantic authority? | **No.** Descriptive projection only. |
| Selection policy? | Near-term: `mfm.portfolio.holding.latest-acceptable.v1`. Long-term: add `at-or-before.v1` (+ optional strict equality) in the same family. |
| Pin-as-fact for report? | **No** as report authority. Progressive chain-head facts stay collector/control. |
| Multi-chain point-in-time? | Per-network pins; no fake global block across independent chains. |
| Fail-closed? | Hard-fail run for required symbols; no soft-success partial snapshots in v1. |
| Snapshot observe? | One observation DTO with anchor + balances; multi-subject same network SHOULD share tip. |
| Public `portfolio_snapshot`? | Report-only after cutover. |
| Composition? | External multi-run recipe; no pin-table handoff. |
| Live PinViews + ObserveBatch? | Deleted at cutover. |
| Near-term BTC fact? | `bitcoin.address_balance_snapshot`. |
| Who owns cutover fact schemas? | Commit series B. Collectors are writers. |
| Cutover ERC-20? | Out of scope; natives only. |
| Delivery shape? | One PR, progressive commits. |
| Mega framework / crate-per-collector / always-4-node? | No. |

## Open Questions

- Encoding of certified as-of upper bounds (per-network map vs global) for `at-or-before.v1`.
- Whether product needs strict equality historical mode vs upper-bound-only.
- Freshness policy representation when price facts land (global vs per-kind vs per-source); basis
  remains fact-carried anchor/time, not ambient process clock.
- Price↔holding join rule parameters (still selection over price facts, not a second pin table).
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
- `network_pins` match selected fact anchors only (no config pin table)
- no-secret persisted/public surfaces
- when collectors land: collect-then-report dual-mainnet recipe works; collector replay without live
  source; multi-subject same-network shared tip avoids inconsistent anchors when expected

Final merge-readiness uses the repository's normal Nixfied gates.
