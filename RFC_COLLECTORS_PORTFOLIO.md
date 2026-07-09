# RFC: Fact-Backed Portfolio Collectors And Reports

Status: Draft (architect-hardened)

## Summary

`portfolio_snapshot` must not be a live crawler.

The current portfolio workflow pins chains, reads wallet balances, values positions, and assembles a
report in one run. That shape works for narrow configured demos and fails for real portfolios. A
portfolio view may need Bitcoin balances, EVM native balances, ERC-20 balances, token discovery,
protocol positions, prices, metadata, freshness, and coverage proofs. Doing that on the fly makes
the report slow, incomplete, hard to replay, and easy to misrepresent as complete.

Target architecture:

```text
protocol IO (transports)
  -> family adapter binds observe/record runners
  -> family collector Operation (snapshot re-read near-term; progressive later)
  -> durable source-near Platform data facts (each with an explicit anchor)
  -> portfolio report Operation
     (Platform fact-index only; certified selection policy)
  -> PortfolioSnapshot + PortfolioReport public outputs
     (network_pins always projected from selected fact anchors; never a pin table)
```

Collectors produce reusable facts. Portfolio reporting consumes facts only.

**As-of is a single authority model forever:**

1. Every Platform data fact carries its own source-near anchor (chain height/hash, oracle/time, etc.).
2. The certified report selects facts under a named selection policy family
   (`latest-network-coherent` now; optional `at-or-before` bounds later as **filters**, not a second
   pin store). Per-network selection is **network-coherent**: it does not pick independent latests
   that disagree on anchor when a common older anchor exists.
3. Public `network_pins` are **always** a pure projection of selected required holding anchors —
   descriptive output, not selection authority and not report config.
4. Same-network selected holdings share one anchor by construction of the selection policy (or the
   run fails if no common acceptable anchor exists).
5. No report pin table. No pin-facts as report authority. No live `PinViews`.
6. Cutover BTC/EVM holding facts require height **and** block hash. Collectors must prove balance
   was read at that anchor before write.

```text
collectors: observe(source @ joint anchor) → Platform data facts
report:     certified policy → SelectHoldings → Observations + network_pins(from selected anchors)
replay:     recorded fact-query evidence only
```

Public surfaces after cutover:

| Surface | Meaning |
|---|---|
| `portfolio_snapshot` | fact-backed report only |
| collector entry points / internal ops | source IO that records facts |
| CLI/docs "full refresh" | external multi-run orchestration (collect, then report); never a mixed certified draft |

This RFC is a breaking redesign on a breaking-change branch. Dual live+facts paths, silent
fallbacks, soft-success partial snapshots, and crate-per-collector scaffolding are rejected. Live
portfolio pin and observe code is deleted when the fact-backed report lands. Delivery is **one PR**
with progressive **commits**; git history is recovery.

## Project rules

These rules govern every commit on this branch:

0. Prefer fewer concepts, paths, types, and duplicated responsibilities. Reduce LOC without cryptic
   code.
1. No backward compatibility. Do not preserve live crawl APIs, dual IO paths, or soft-success
   snapshot shapes for callers of the old design.
2. No fallbacks. Delete old code when the replacement lands. Git history is recovery.
3. Progressive commits within one PR. Keep each commit reviewable and local.
4. Follow `docs/code-quality.md` for every code, test, documentation, build, and workflow change.
5. No dual public truth on the merge tip. Intermediate WIP commits may break portfolio; the tip
   intended for merge must not keep live pin/observe beside facts.

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
Collection and reporting must distinguish:

- complete at anchor
- configured assets only
- discovery coverage incomplete
- missing facts
- stale facts
- source unsupported
- source failed

Coverage is a first-class output of collection. It is not a UI detail. Public report success under
`configured_symbols` must remain visibly incomplete discovery: success means “all configured
requirements were selected,” not “the wallet was fully scanned.”

### Reporting And Collection Have Different Lifecycles

Collection is source-facing and operational:

- collectors may run periodically
- collectors may fan out by address, chain, token, price source, or protocol at planning time
- collectors may use different providers over time
- collectors may retry or lag behind the latest chain head
- progressive collectors (later) may need control cursors; progressive chain-head / Control cursor
  facts are **never** report pin authority

Reporting is deterministic and compositional:

- it selects facts by subject, coverage, and a certified selection policy over fact anchors
- it materializes fact response artifacts under recorded query evidence
- it normalizes facts into the existing portfolio `Observation` view (including anchors)
- it produces stable public outputs (`network_pins` as projection of selected anchors)
- replay uses recorded fact-query evidence, not live IO

Putting both lifecycles in one certified state graph creates a workflow that is hard to certify,
hard to resume, and hard to explain. Mixed certified collectors+report drafts are **rejected**.

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

Both `PinViews` and `ObserveBatch` are live chain IO. Cutover deletes **both**, plus
`MergeObservations` when selection owns the full observation set.

**Migration of as-of authority:**

| Era | How as-of works | How `network_pins` form |
|---|---|---|
| Live today | `PinViews` live head → observe at pin | From live `PinnedViews` |
| Cutover / long-term | Anchors on Platform data facts + certified selection policy | **Always** projected from selected holding fact anchors |

There is no intermediate dual path where pins come from either config or facts. Optional historical
as-of is later expressed as selection **bounds** (filters), not as a pin table that owns
`network_pins`.

### Collector Surface Area Will Explode Without A Standard

Near-term collectors already include chain-head, Bitcoin balance, EVM native balance, and later EVM
token balance. Without a minimal standard:

- N nearly-identical op crates appear for thin topology glue
- checkpoint/control machinery is copied with wrong watermark semantics
- portfolio grows a second live truth path beside collectors
- a mega "collector framework" freezes tomorrow's discovery into today's balance reads

The expensive mistakes are dual IO truth paths and framework soup. The cheap mistake is pasting a
small `expand` again. Optimize for the expensive mistakes. Prefer family monocrates and shared
runners only when a second real consumer exists.

## Goals

- Move portfolio reporting to a fact-backed workflow with no live chain/source reads.
- Keep collectors independently useful and reusable across reports, audits, and future workflows.
- Keep `PortfolioSnapshot`, `PortfolioReport`, and `Observation` as report-facing DTOs only.
- Add durable source-near facts for holdings, prices, metadata, and coverage; each data fact carries
  an explicit time/chain anchor.
- Use a **single long-term as-of authority**: fact anchors + certified selection policy;
  `network_pins` are projection only forever.
- Make missing, incomplete, or unsupported required data explicit report **run** failures.
- Use recorded fact-query evidence for report replay.
- Keep chain/indexer/oracle IO in collectors, adapters, transports, and provider backends.
- Keep operation crates deterministic and states free of ambient IO.
- Organize collectors by family monocrates so growth is O(families), not O(collectors).
- Prefer fewer concepts: no report pin table, no pin-facts as report authority, no dual pin paths,
  no soft-success error vectors on public snapshots.
- Support configured symbols before open-ended token discovery, with public honesty that success is
  not full discovery.
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
- Do not keep `PortfolioSnapshot.errors`, `PortfolioSnapshotError`, or intermediate soft error
  vectors that enable partial public success.
- Do not certify a mixed collectors+report draft.
- Do not keep view-dependent valuation readers (`DirectPrice` and peers) on the cutover surface.
- Do not treat `limit=1` fact queries as selection authority.

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

Normative sequence for report fact selection:

```text
compile canonical fact query plan (from certified report config + subject projection + policy)
  -> FactIndexRead (Platform audience) over the requirement set
  -> authenticated receipt + trust root + selection evidence
  -> hydrate retained FactResponse artifact(s)
  -> validate selection cardinality and acceptability
  -> SelectHoldings (network-coherent policy over full acceptable candidate sets)
  -> normalize into Observation (including fact anchor)
```

Prefer **one selection frontier/receipt** for the requirement set when the capability allows it.
Do not invent N independent authority receipts when one receipt can pin the candidate inputs for
the whole select step.

Replay rules:

- replay uses recorded fact-query evidence and retained response artifacts only
- replay must not re-query the live index frontier and accept newer facts
- public-facts API and ambient store SQL are forbidden as semantic authority for report states
- selection inputs must be receipt-pinned or fully retained so network-coherent selection is
  deterministic on replay

Hydration is mandatory for report holding facts: receipt alone is not enough when the balance
payload lives in the retained response artifact.

Who may call Platform fact-index in certified runs:

- portfolio report states (required)
- other certified readers only when they have the same evidence obligations
- collectors must not Platform-query prior portfolio holdings (review policy; observe is source IO)

Commit series A extracts a shared fact-index query path (audience-agnostic request, hydrate,
evidence) so portfolio does not copy the BTC Control runner. That shared fact kit is intentional
packaging work at A / second consumer — not “when cheap.”

### 2. Anchors On Facts (As-Of Model) — Single Authority

Two different concepts must not be conflated:

| Concept | Role | Authority for as-of? |
|---|---|---|
| Store read frontier (projection generation, commit watermark) | Authenticates **which index snapshot** the query read; bound into the query receipt | Query evidence only |
| Fact anchor | Source-near chain/oracle as-of on every Platform data fact response | **Yes — claim content** |
| Selection policy | Hash-defining rule choosing which acceptable fact wins per requirement | **Yes — certified** |
| Optional as-of bounds | Certified **filters** inside selection policy (later) | Yes as policy params, **not** as pins |
| Public `network_pins` | Projection of selected required holding anchors | **No — descriptive output only** |
| Progressive chain-head / Control cursor facts | Collector ops only | **Not** report pin authority |

**Decision (long-term and near-term):** there is **one** as-of authority for report truth:
**anchors on selected facts**, under a certified selection policy.

- No separate hash-defining report pin table.
- No pin-facts as report pin authority.
- No live `PinViews`.
- Progressive chain-head / Control cursor facts remain collector ops only and **must never** become
  report pin authority.
- Optional historical bounds are parameters of the **same selection policy family**, not a parallel
  path where `network_pins` may come from either config or facts.

```text
collectors: observe(source @ joint anchor) → Platform data facts
report:     certified policy → SelectHoldings → Observations + network_pins(from selected anchors)
replay:     recorded fact-query evidence only
```

#### Anchor requirements (every Platform holding / price data fact)

- Anchor material is part of the fact **response** (content-addressed with the claim).
- **Cutover BTC/EVM holding kinds:** height/block number **and** block hash are **mandatory**.
  Height-only is not allowed for these kinds. A later fact kind may document a weaker source only if
  it is a distinct kind with its own descriptor and honesty bar.
- Price facts (later): source timestamp or on-chain oracle anchor; never process wall clock alone as
  the sole anchor.
- Observe normalize **fails closed before write** if required anchor material is missing.
- Hashed structures: no floats (integer heights; decimal strings for quantities/prices).

#### Report selection — policy family

**Near-term cutover (required): network-coherent latest**

Independent per-holding "pick latest" then fail on anchor disagreement is **rejected**. It fails even
when a coherent older common snapshot exists (e.g. A has facts at 100 and 99, B only at 99 →
per-holding latest selects A@100 and B@99 then fails).

```text
policy_id = mfm.portfolio.holding.latest-network-coherent.v1

group required holdings by network

for each network group G:
  for each holding h in G:
    load the full acceptable Platform candidate set for h
      (subject predicates + coverage allow-list + source_status allow-list)
    // Candidate completeness is mandatory: full acceptable sets.
    // limit=1 is never selection authority.
    if any holding has zero candidates -> missing_fact

  let common_anchors = intersection over h in G of
    { (height, hash) of acceptable candidates of h }
    // height+hash identity for cutover kinds

  if common_anchors is empty -> no_common_network_anchor

  let chosen_anchor = max(common_anchors) by
    height desc, then hash bytes desc (deterministic tie-break)

  for each holding h in G:
    select the unique acceptable candidate at chosen_anchor
    if multiple candidates at same subject+chosen_anchor:
      order by store_commit_order desc
      then, only if clean and available, fact_claim_id desc  // secondary only if clean
      accept first
    if zero at chosen_anchor after filters -> missing_fact (should not happen if intersection correct)

if plan/receipt cardinality violated after policy -> ambiguous_facts
```

Acceptability filters apply before intersection and selection. Do **not** take raw latest-per-holding
with `limit=1` and only then check network coherence.

**Same-subject / same-anchor conflicts (cutover):**

- Selection is deterministic last-write-wins via `store_commit_order`.
- Secondary key (`fact_claim_id` or equivalent) is used **only if clean and available** on the
  retained selection inputs; do not invent a dual conflict mode.
- Different `semantic_source_identity` (or family equivalent) is a different subject — not a conflict.
- Same subject + same anchor + different response payload: cutover LWW is allowed and documented.
  Optional later `conflicting_facts` (fail closed on content-hash mismatch) is series E work and is
  **not** a cutover error code.

**Long-term extension (same authority model; not dual path):**

```text
policy_id = mfm.portfolio.holding.at-or-before.v1
  + certified per-network (and later price-axis) upper bounds

same network-coherent selection, restricted to candidates with
  fact.anchor ≤ bound (height/block or oracle time; family-defined comparison)
  and still using (height, hash) identity for chain holdings
```

Optional later strict mode: selected anchor must equal bound or fail `as_of_not_exact`. Default is
upper-bound filter with **actual** selected anchors projected to `network_pins`. Bounds do not invent
missing history; missing data still fails `missing_fact` / `no_common_network_anchor`.
`as_of_not_exact` is **not** a cutover error code.

Recollect-at-bound is a **collector** recipe. Report stays facts-only and never live-pins.

#### Public `network_pins` projection (single rule forever)

- Project **only** from selected **required holding** facts' anchors.
- One pin per network that appears in those selections.
- Under `latest-network-coherent.v1`, same-network selected holdings already share one
  `(height, hash)` by construction; project that pin.
- If any residual disagreement appears (bug / wrong policy) → run fails
  `inconsistent_network_anchors` (guard only; not a product soft path).
- Do **not** invent pins from live chain head, report config, or chain-head facts.
- Do **not** maintain a parallel pin table that can desync from selected facts.
- Do **not** use `network_pins` as input to selection (no feedback loop).
- Observations also carry the per-holding anchor from the selected fact (existing
  `ExecutionAnchor`-shaped material on the report DTO path).

#### Multi-chain coherence (normative honesty)

- **Per-network coherence is a selection invariant:** same-network required holdings are selected at
  one common anchor, or the run fails with `no_common_network_anchor`.
- **Cross-network simultaneity is not a single block.** Independent networks yield independent pins.
  That is correct, not a bug.
- Full-refresh orchestration and shared-tip collectors reduce empty common-anchor sets operationally;
  they are **not** a certified pin authority layer.

#### Reorgs and tip race

| Hazard | Handling |
|---|---|
| Tip race same network (collect A at H, B at H+1) | Network-coherent selection picks the newest **common** anchor if one exists (e.g. both at H), else `no_common_network_anchor`. Collectors that write multiple subjects on one network in one batch **MUST** share one observe tip so the newest tip is common. |
| Tip race across networks | Allowed; pins differ by network. |
| Reorg after fact write | Facts remain historical observations. New collectors write superseding higher/finalized anchors. Network-coherent latest moves forward when all holdings have the new anchor. Do not silently rewrite old facts. |
| Same height, different hash | Distinct anchors; intersection requires matching hash. Cutover kinds mandate hash so fork siblings cannot be silently mixed. |
| Finality | Collectors may encode finality metadata on progressive facts later. Report acceptability may later filter by fact-carried finality — never via live head checks in the report. |

#### Multi-run full refresh (no pin handoff)

```text
collectors write Platform facts with observed anchors
  -> portfolio_snapshot selects facts under certified policy
  -> network_pins projected from those anchors
```

There is no collect→pin_table→report glue. Collectors and report share the store's fact index only.
Composition is external multi-run only.

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
- selection policy id (`mfm.portfolio.holding.latest-network-coherent.v1` at cutover; later
  `at-or-before.v1` + bounds when product needs historical reports)
- coverage allow-list
- source status allow-list
- optional later: as-of bounds as policy parameters

These are hash-defining certified config or fixed policy digests. They are not adapter defaults.

**v1 `configured_symbols` is hard-fail only.** Any required-symbol selection or normalization
failure fails the **state/run**. Do not emit a successful public snapshot with empty wallets or
zeroed required holdings.

**Soft-success DELETE (cutover and merge tip):**

- Delete `PortfolioSnapshot.errors` as a public soft-success channel.
- Delete `PortfolioSnapshotError` and intermediate soft error vectors used to green-path partial
  holdings.
- Delete assembly paths that zero required holdings and still report success.
- Partial-success product modes are out of scope until a real requirement exists; they are not
  reintroduced as a silent default.

**Minimal cutover error codes** (map to CLI/public error surface):

| Code | When |
|---|---|
| `missing_fact` | no acceptable Platform fact for a required subject after filters (includes former unacceptable coverage / source status outcomes) |
| `no_common_network_anchor` | network group has acceptable facts but empty intersection of anchors |
| `unsupported_requirement` | portfolio requirement has no projection rule |
| `ambiguous_facts` | selection cardinality violated after policy / receipt plan |

Fold former `unacceptable_coverage` and `unacceptable_source_status` into `missing_fact` at
cutover. Keep `no_common_network_anchor` distinct from `missing_fact` so operators can tell "no
data" from "data exists but not cohered". Do **not** introduce `pin_mismatch`.

**Not cutover codes** (series E or guards only if needed later):

| Code | Status |
|---|---|
| `as_of_not_exact` | series E strict historical mode only |
| `conflicting_facts` | optional series E content-hash conflict mode only |
| `inconsistent_network_anchors` | residual guard after selection (bug), not product soft path |

Valuation at cutover: fixed unit prices (including intentional dual-mainnet zeros) are pure config
placeholders, not selection failures. View-dependent valuation readers (`DirectPrice` and peers) are
**deleted from the cutover surface**. They may be reintroduced in series E as new work when price
facts land — not preserved under feature flags.

### 5. Snapshot Collector Observe Anchor Integrity (Writers)

Snapshot pattern: `ObserveSource → RecordDataFact`.

**Decision:** the collector's observe path produces **one observation DTO** that includes anchor +
balances + coverage + source status. That material is what becomes the Platform data fact.

**Anchor integrity is a write-time invariant, not packaging.**

The observation is valid only if the balance (or holding payload) was actually read **at** the
recorded `(height, hash)`. Multi-cap composition (head read then balance read) vs a single combined
RPC is packaging; **proving balance@anchor** is not.

**Prove-before-write monomorphic must-rules (no mega trait):**

- **EVM:** balance/read path **MUST** bind to block hash (prefer EIP-1898 block-hash selectors, or
  explicit hash verification around the read). Missing hash, tip drift, or hash mismatch fails closed
  before Platform write.
- **BTC:** pin-in balance requests **MUST** carry/verify the best block used (height **and** hash).
  Tip mismatch or missing hash fails closed before Platform write.
- Encode these as monomorphic family rules on the observe/normalize path. Do **not** invent a
  generic prove-before-write mega trait or cross-family capability soup.

Normative write rules for cutover kinds:

- Recorded anchor height **and** block hash are mandatory on the observation and the Platform fact.
- Fail closed before Platform write if tip drifts, hash mismatches, or hash is missing.
- Report trusts admitted facts + retained evidence; it does **not** re-verify against live chain.

Collectors that write multiple subjects on one network in one batch **MUST** resolve tip once and
share that joint anchor across the batch so the newest tip is a common anchor for network-coherent
selection.

Report never supplies pins.

### 6. Public Entry And Composition Model

| Surface | Meaning after cutover |
|---|---|
| `portfolio_snapshot` | **report-only**: resolve subjects, select Platform holdings under policy, project report |
| collector ops | source IO only; record Platform data facts (+ Control cursors if progressive later) |
| full refresh | **external** multi-run recipe: run required collectors, then run `portfolio_snapshot` |

A single certified draft that mixes collectors and report is **rejected**. Kernel multi-run
orchestration is not a new authority layer; CLI/docs/app recipes are non-authority glue.

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
4. Fact-backed report graph replaces pin+observe with `SelectHoldings` under
   `latest-network-coherent.v1`.
5. Live portfolio pin/observe paths and portfolio-only live balance/head transports are deleted.
6. Soft-success snapshot error channels and zeroed-required-holdings success paths are deleted.
7. Public `portfolio_snapshot` means report-only.
8. Portfolio integration/parity tests rewritten off live Reth crawl (fixtures + collectors when
   present).
9. `network_pins` projected only from selected fact anchors (single projection rule).
10. Public success honesty: configured mode/coverage is visible so success ≠ full discovery.

**Product honesty:** after cutover, `portfolio_snapshot` is useless without admitted facts.
Dual-mainnet demo requires either fixture seed (CI) or collectors (operator). Prefer landing
configured collectors on the same branch before merge so the tip is usable end-to-end; if collectors
lag, document that explicitly and keep CI fixture-based.

### 8. Coverage And Source Status Vocabulary

Typed closed enums with a single pure owner. Near-term report-accept set is minimal.

**CoverageStatus**

| Value | Meaning | May write Platform fact? | Report accept? |
|---|---|---|---|
| `complete_at_anchor` | source proved completeness for the claimed universe | **yes** | yes |
| `configured_only` | only configured assets; wallet discovery incomplete by design | **yes** | yes |
| `truncated` | source truncated / incomplete pagination | **no** — fail closed before write | n/a |
| `incomplete` | known incomplete observation | **no** — fail closed before write | n/a |

**SourceStatus**

| Value | Meaning | May write Platform fact? | Report accept? |
|---|---|---|---|
| `ok` | observation succeeded | **yes** | yes |
| `unsupported` | source cannot serve this subject | **no** — fail closed before write | n/a |
| `failed` | source error | **no** — fail closed before write | n/a |

Writers **MUST** fail closed: `truncated`, `incomplete`, `unsupported`, and `failed` never write
Platform facts. Only `configured_only|complete_at_anchor` combined with `ok` may write.

Configured collectors default to `configured_only`. Most JSON-RPC balance reads cannot claim
full-wallet completeness. Public report success under configured mode must expose that coverage so
callers cannot confuse success with full discovery.

### 9. Near-Term Bitcoin Holding Fact

**Decision:** `bitcoin.address_balance_snapshot` (total sats + anchor + coverage + source status),
matching current provider truth (`scantxoutset`-class total balance). Full UTXO set retention is
deferred.

### 10. Report Graph After Cutover

Delete from portfolio report topology and adapters:

- `PinViewsState` live head/anchor reads
- `ObserveBatchState` live balance reads
- `MergeObservations` (select-centric graph owns the full observation set)
- `PortfolioReadCapability` / transport-factory paths used only for live pin or balance crawl
- soft-success error fields and zeroed-required-holdings assembly paths
- view-dependent valuation readers (`DirectPrice` and peers) from the cutover surface

Target shape (select-centric; **not** a QueryHoldingFacts* + Merge crawl isomorphism):

```text
ResolveSubjects
  -> SelectHoldings             (Platform fact-index + network-coherent selection over full
                                 acceptable candidate sets; produces Observations)
  -> ResolveValuations          (pure, config-only fixed placeholders; no PinnedViews input)
  -> AssembleSnapshot           (network_pins from selected fact anchors; hard-fail only)
  -> ProjectReport
```

`SelectHoldings` is the named selection state. It owns:

- subject projection into fact queries
- Platform fact-index read under recorded evidence (prefer one frontier/receipt for the requirement
  set)
- acceptability filters
- network-coherent selection over **full** acceptable candidate sets
- normalization into the complete observation set for required holdings

Do not reintroduce a crawl-shaped fanout + merge topology that re-creates live observe batching
under new names.

`ResolveValuations`: pure, config-only. Drop `PinnedViews` dependency. No view-dependent readers at
cutover.

`AssembleSnapshot`: no live `views` input. Inputs are subjects, valuations, the selected observation
set (with anchors), and assembly config. Project `network_pins` per Contract 2 single rule forever.
Hard-fail only; no soft error vectors.

### 11. Fixture And Test Admission

Facts used by report tests enter through `FactRecorded` + descriptor admission. Prefer a tiny
certified writer graph or a named `test_support` API that emits real claims, response artifacts, and
projections.

**Forbidden:** SQL inserts into `fact_index` / `fact_index_terms` without admission authority.

### 12. Phase 1 Owns Minimal Fact Schemas

Cutover fixtures and report selection require fact kinds before collectors exist. **Schemas first**;
collectors are writers:

- `bitcoin.address_balance_snapshot` in `states/btc` (or module split thereof)
- `evm.address_native_balance_snapshot` in `states/evm` (new crate when first EVM fact lands;
  intentional packaging at series B)

Collectors (later commits on the same branch) write those kinds. They do not own the first
definition.

### 13. Collectors Vs Report Config

| Collectors write | Report certifies |
|---|---|
| Joint observation DTO → Platform data fact | Subject projection table |
| Explicit anchor on every data fact | Selection policy id (+ optional bounds later) |
| Coverage + source status (write only ok + configured_only/complete_at_anchor) | Coverage / source status allow-lists |
| Shared joint tip for in-batch multi-subject same network | Fail-closed cardinality and consistency rules |
| Progressive cursors (Control) when progressive later | Public `network_pins` projection rule (fixed forever) |

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
- `bitcoin_address_balance` (snapshot re-read; total balance)
- `evm_native_balance` (snapshot re-read)
- `evm_token_balance_collector` (snapshot re-read; after cutover natives)

Later collectors:

- `evm_token_discovery_collector` (progressive)
- `token_metadata_collector`
- `price_collector` (usually snapshot re-read)
- protocol position collectors
- optional later: full UTXO snapshot if analytics require it

## Collector Patterns (Minimal)

The shared platform is already the framework. Collectors must not invent a second one.

Two control patterns exist as review heuristics, not a framework crate or shape enum. Progressive
collectors are **not** co-specified as a peer chapter of report cutover; one forbid sentence
suffices: progressive chain-head / Control cursor facts are never report pin authority.

### Snapshot re-read (configured balances / prices) — cutover default

```text
ObserveSource (anchor + balances in one observation DTO)
  -> RecordDataFact
```

Coverage and source status live on the data fact. Control cursors are not required. Only
`configured_only|complete_at_anchor` + `ok` may write Platform facts.

### Progressive (cursor / watermark) — later; not report authority

```text
QueryControlCursor -> ObserveSource -> RecordDataFact -> RecordControlCursor
```

When progressive collectors land later:

1. Data fact records before control cursor advance.
2. Cursor advance derives only from certified observation/data-fact material + prior cursor.
3. Watermarks must not regress.
4. Observe is `ReadExternal` only; record is `ManagedPlatformWrite` only.
5. Snapshot re-reads may append multiple Platform facts for the same subject; report selection is
   totally ordered by the named selection policy.

Progressive chain-head facts are **not** report pins. They stay collector/control ops.

### Shared Vs Specialized

| Share when a second consumer needs it | Specialize (monomorphic) |
|---|---|
| Platform + Control fact-index query runner | data fact types / descriptors |
| Managed fact-record runner (second writer) | progressive cursor types |
| Fact-record capability role | observe config / normalize |
| Coverage / source status enums | domain read caps, transports, family op expand |
| Prove-before-write rules as family monomorphic musts | no mega prove trait |

Copy-paste thin Operation `expand` until a third real duplicate hurts. Shared fact kit extraction is
intentional progressive packaging (series A / second consumer), not opportunistic cleanup.

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
16. No `PortfolioSnapshot.errors` / soft error vectors as public success channel.
17. No wall-clock staleness as uncertified ambient policy.
18. No fake global multi-chain block identity for portfolio truth.
19. No mixed certified collectors+report draft.
20. No view-dependent valuation readers on the cutover surface.
21. No `limit=1` as selection authority; full acceptable candidate sets only.
22. No dual conflict mode at cutover (LWW via `store_commit_order`; secondary only if clean).
23. No mega prove-before-write trait; monomorphic EVM/BTC must-rules only.

### Capability Matrix

| Need | Capability class |
|---|---|
| Source observation | family domain read caps |
| Progressive cursor load (later) | fact-index read (Control) |
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
7. Shared runner mechanics → shared kit when a second consumer needs it.
8. Workflow names stop at ops.
9. New collector = module + Operation + states + adapter registration lines.
10. Portfolio crates must not grow source-near holding fact types.
11. Collector ops must not depend on portfolio report DTOs.

### Target Tree (Guidance With Intentional Packaging Gates)

```text
crates/
  fact-capabilities/          # Control + Platform fact-index; shared fact-record role when needed
  states/btc/                 # chain-head + address balance facts
  states/evm/                 # native (+ later token) balance facts
  states/portfolio/           # report only
  ops/btc-collectors-op/      # family monocrate when BTC collectors consolidate
  ops/evm-collectors-op/      # family monocrate when EVM collectors land
  ops/portfolio-tracker-op/   # fact-backed report only after cutover
  adapters/btc-jsonrpc/
  adapters/evm/               # when EVM collectors land
  adapters/portfolio/         # report-only after cutover (Platform fact-index + pure report)
```

**Intentional progressive packaging (not “when cheap”):**

| Gate | Packaging work |
|---|---|
| Series A / second consumer | Shared fact-index query kit (audience-agnostic request, hydrate, evidence) |
| Series B | `states/evm` monocrate when first EVM fact kind lands |
| Series C | Portfolio adapter becomes report-only (delete live pin/balance transports) |
| Series D | Family op monocrates for collectors (`btc-collectors-op`, `evm-collectors-op` as needed) |

### Deletes At Cutover

| Delete | Why |
|---|---|
| Live `PinViewsState` | live chain IO; replaced by fact anchors + selection policy |
| Live `ObserveBatchState` | live chain IO |
| `MergeObservations` | crawl isomorphism; select-centric graph owns full observation set |
| Portfolio-only live pin/balance transports | dual truth |
| Soft-success zeroed required holdings | false completeness |
| `PortfolioSnapshot.errors`, `PortfolioSnapshotError`, soft error vectors | soft-success channel |
| Dual-mode "live if facts missing" | dual truth |
| View-dependent valuation readers (`DirectPrice` etc.) | not cutover surface; reintroduce in E as new work |
| Any report pin table field that owns `network_pins` | dual as-of authority |
| Mixed certified collectors+report drafts | composition is external multi-run only |

## Asset Universe Policy

Near-term: only configured symbols matter. Missing/unacceptable facts for required symbols fail the
run. Unconfigured tokens are out of scope. The report must not claim complete wallet discovery.

Public success honesty: configured mode and selected coverage (`configured_only` vs
`complete_at_anchor`) must remain visible on public outputs or stable report metadata so success is
not misread as full wallet discovery.

Do not invent `asset_universe.mode` TOML until discovery work starts. Configured-symbols is the only
report mode until then.

## Recommended Fact Families

### `bitcoin.address_balance_snapshot` (cutover)

Subject: `network`, `bitcoin_network`, `semantic_source_identity`, `address`  
Response: **mandatory** anchor height, **mandatory** anchor block hash, total sats, coverage, source
status

### `evm.address_native_balance_snapshot` (cutover)

Subject: `network`, `chain_id`, `account`  
Response: **mandatory** block number, **mandatory** block hash, raw wei decimal string, decimals,
coverage, source status

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

### Audience decision (intentional)

Holding and price **data** facts default to public `Platform` audience. Progressive control cursors
are Control audience.

**Threat model for this project:** portfolio collectors observe **public chain state** (addresses and
balances already visible on-chain or via public RPCs) and public market data. MFM treats that
observation substrate as openly discoverable through the durable fact layer where descriptor policy
allows it. This is **not** a private bank ledger product mode.

Still forbidden on every surface (facts, artifacts, query evidence, public outputs, errors):

- secrets, mnemonics, private keys
- RPC URLs, credentials, authorization headers
- private source routing and raw provider diagnostics

If a future private-portfolio product is required, that is a new audience/config design — not a silent
change of these cutover kinds.

### Field exposure (cutover holding kinds)

| Field class | Exposure | Why |
|---|---|---|
| network / chain ids, address/account, anchors (height+hash), balances, coverage, source_status | **returnable** (and queryable where useful for selection) | public chain observations; needed for honest inspection and report debugging |
| subject filter fields used only for indexing | may be **query-only** if returnable rows should stay compact | filtering without row bloat |
| internal fact claim refs, artifact ids, evidence hashes, predecessor refs | **hidden** | retained evidence / store internals, not public product |
| RPC endpoints, credentials, provider diagnostics | **never stored** | secrets / routing |

Descriptor field exposure remains deliberate: every field must be classified. Public Platform
audience does not mean every internal detail is returnable.

## Implementation Plan (One PR, Progressive Commits)

Engineer-facing commit order, delete lists, tests, and packaging gates live in
`PLAN_IMPL_RFC_COLLECTORS_PORTFOLIO.md`. That plan is the implementer checklist; this RFC is the
normative design contract.

Branch delivery: **one PR**. Sequence by **commit**. Keep the eventual merge tip free of dual public
truth. Intermediate WIP commits may break portfolio while the branch is open. No backward
compatibility; no fallbacks; delete old code; git history is recovery.

### Commit series A — Platform fact-index substrate

- Extend `FactIndexReadRequest` for Platform audience (or audience allow-list).
- Evidence/receipt/trust-root/hydrate/replay parity with Control.
- Shared fact-index query kit extracted intentionally (second consumer / portfolio will not
  reimplement BTC Control runner).
- Prefer one selection frontier/receipt shape usable for a requirement set.
- Tests: Platform query, replay without live re-query; no public-facts authority.
- Packaging: shared fact kit at A / second consumer (intentional, not “when cheap”).

### Commit series B — Minimal holding fact schemas + fixtures

- Define `bitcoin.address_balance_snapshot` and `evm.address_native_balance_snapshot` (types,
  descriptors, Platform visibility, **mandatory anchors**, coverage/source enums accept set).
- Stand up `states/evm` when first EVM fact kind lands (intentional packaging at B).
- Fixture admission via real `FactRecorded` path / certified tiny writer / named test_support.
- Pure normalize helpers: fact → `Observation` (including anchor).
- Writers-side vocabulary enforces only `configured_only|complete_at_anchor` + `ok` may write.

### Commit series C — Fact-backed report graph + delete live crawl

- Subject projection + selection policy `latest-network-coherent.v1` only.
- Report topology:
  `ResolveSubjects → SelectHoldings → ResolveValuations → AssembleSnapshot → ProjectReport`.
- `SelectHoldings` owns full acceptable candidate sets, network-coherent selection, and observation
  normalization; delete crawl-shaped QueryHoldingFacts* + Merge isomorphism.
- `network_pins` from selected fact anchors only; hard-fail `no_common_network_anchor` / `missing_fact`.
- Minimal cutover codes only: `missing_fact`, `no_common_network_anchor`, `unsupported_requirement`,
  `ambiguous_facts`.
- Delete `PinViews`, `ObserveBatch`, portfolio live pin/balance transports.
- Delete soft-success channels: `PortfolioSnapshot.errors`, `PortfolioSnapshotError`, intermediate
  soft error vectors, zeroed required holdings on success.
- Delete view-dependent valuation readers from cutover surface.
- Public `portfolio_snapshot` = report-only; portfolio adapter is report-only (intentional at C).
- Rewrite portfolio integration/parity tests to fixtures (report succeeds **without** live chain
  providers when facts are present).
- Public success honesty: configured mode/coverage visible.
- **Do not** add report pin table fields or `at-or-before` yet (strict subset of long-term model).

### Commit series D — Configured collectors (same branch, preferred before merge)

- BTC address balance snapshot collector (observe → record; **mandatory height+hash**; monomorphic
  prove balance@anchor).
- EVM native balance collector (EIP-1898 / hash-bound reads; **mandatory height+hash**; monomorphic
  prove).
- In-batch multi-subject same-network writers **MUST** share one observe tip.
- Writers fail closed for truncated/incomplete/unsupported/failed — never write Platform facts.
- Shared fact-record role/runners when second writer needs them; delete BTC-bound record authority
  if still present.
- Family op monocrates intentional packaging at D (`btc-collectors-op` / `evm-collectors-op` as
  needed).
- Collector replay tests; document multi-run collect-then-report recipe (external only).
- Optional: ERC-20 configured token collector after natives.

### Commit series E — Later extensions (same model; no dual authority)

- Prices/metadata with oracle anchors; certified price selection join.
- Reintroduce view-dependent valuation readers **as new work** only if product needs them; not as
  restoration of deleted cutover surface under a flag.
- Optional `at-or-before.v1` + certified bounds encoding (filters only; `network_pins` rule unchanged).
- Optional strict equality historical mode (`as_of_not_exact`).
- Optional `conflicting_facts` if product wants content-hash fail-closed instead of pure LWW.
- Freshness thresholds as certified policy over fact-carried times.
- Discovery; progressive collectors (still never report pin authority).

Adding bounds later is a new policy id / config fields, **not** a rewrite of `network_pins`
semantics.

## Closed Decisions

| Question | Decision |
|---|---|
| Where do holding facts live? | Family state crates. Not `portfolio-facts`. |
| Platform fact-index for report? | Extend existing fact-index read for Platform + evidence parity with Control. |
| Public-facts API as report authority? | No. |
| Separate report pin table (any phase)? | **No.** As-of lives on fact anchors; optional later bounds are selection filters only. |
| How do `network_pins` form? | **Always** projected from selected required holding fact anchors. Never from config pin tables, live head, or pin-facts. |
| Is `network_pins` semantic authority? | **No.** Descriptive projection only forever. |
| Selection policy? | Near-term: `mfm.portfolio.holding.latest-network-coherent.v1` (newest common acceptable anchor per network). Long-term: add `at-or-before.v1` (+ optional strict equality) in the same family. |
| Independent per-holding latest then fail? | **No** — rejects coherent older snapshots. |
| Report graph shape? | Select-centric: `ResolveSubjects → SelectHoldings → ResolveValuations → AssembleSnapshot → ProjectReport`. Not QueryHoldingFacts* + Merge crawl isomorphism. |
| Candidate completeness? | Full acceptable sets; `limit=1` is never selection authority. Selection inputs receipt-pinned or fully retained for replay. Prefer one frontier/receipt for the requirement set. |
| Cutover BTC/EVM anchor hash? | **Mandatory** height + hash. |
| Same-subject/same-anchor conflict? | Cutover LWW via `store_commit_order`; secondary only if clean. No dual conflict mode at cutover. |
| Pin-as-fact for report? | **No** as report authority. Progressive chain-head facts stay collector/control and never report pin authority. |
| Multi-chain point-in-time? | Per-network pins; no fake global block across independent chains. |
| Fail-closed report? | Hard-fail run for required symbols; soft-success channels deleted. |
| Soft-success DTOs? | Delete `PortfolioSnapshot.errors`, `PortfolioSnapshotError`, intermediate soft error vectors, zeroed required holdings on success. |
| Cutover error codes? | `missing_fact`, `no_common_network_anchor`, `unsupported_requirement`, `ambiguous_facts`. Fold unacceptable_* into `missing_fact`. Omit `as_of_not_exact` / `conflicting_facts` from cutover. |
| Writer fail-closed? | Only `configured_only\|complete_at_anchor` + `ok` may write Platform facts; truncated/incomplete/unsupported/failed never write. |
| Snapshot observe integrity? | Prove-before-write monomorphic must-rules (EVM hash-bound, BTC tip/hash); no mega trait. |
| In-batch multi-subject same network? | **MUST** share joint observe tip. |
| Platform holding facts? | Intentional for public chain data; field exposure table required; no secrets/routing. |
| Public `portfolio_snapshot`? | Report-only after cutover. |
| Composition? | External multi-run recipe only; mixed certified collectors+report draft **rejected**. |
| Live PinViews + ObserveBatch? | Deleted at cutover. |
| View-dependent valuation readers? | Deleted from cutover surface; reintroduce in E as new work if needed. |
| Near-term BTC fact? | `bitcoin.address_balance_snapshot`. |
| Who owns cutover fact schemas? | Commit series B. Collectors are writers. |
| Cutover ERC-20? | Out of scope; natives only. |
| Delivery shape? | One PR, progressive commits; no backward compatibility; no fallbacks; no dual public truth on merge tip. |
| Packaging? | Intentional progressive work: shared fact kit at A/second consumer; `states/evm` at B; portfolio adapter report-only at C; family op monocrates at D. |
| Mega framework / crate-per-collector / always-4-node? | No. |
| Public success honesty? | Configured mode/coverage visible so success ≠ full discovery. |

## Open Questions

- Encoding of certified as-of upper bounds (per-network map vs global) for `at-or-before.v1`.
- Whether product needs strict equality historical mode vs upper-bound-only.
- Freshness policy representation when price facts land (global vs per-kind vs per-source); basis
  remains fact-carried anchor/time, not ambient process clock.
- Price↔holding join rule parameters (still selection over price facts, not a second pin table).
- First token discovery provider: local Reth/Erigon, hosted index, or provider-neutral logs.
- When (if ever) full Bitcoin UTXO retention is required.

## Verification Expectations

On the merge tip of this branch:

- report consumes only Platform facts with recorded query evidence
- report **succeeds without live chain providers** when required facts, response artifacts, and
  query evidence are present
- report **fails** with minimal cutover codes when required facts are missing, no common network
  anchor exists, requirement is unsupported, or selection is ambiguous
- replay never constructs live chain providers and never re-queries the live fact-index frontier
- selection inputs are receipt-pinned or fully retained; network-coherent selection is deterministic
  on replay
- no live PinViews/ObserveBatch/MergeObservations registration for portfolio report
- no soft-success public snapshot path (`PortfolioSnapshot.errors` gone; no zeroed required holdings
  on success)
- no view-dependent valuation readers on cutover surface
- fixture facts admitted through real `FactRecorded` authority
- `network_pins` match selected fact anchors only (no config pin table)
- network-coherent selection recovers common older anchors (A@100+A@99, B@99 → select both @99)
- candidate selection uses full acceptable sets (not `limit=1` authority)
- public success under configured mode does not claim full discovery (coverage/mode visible)
- collectors that write multi-subject same-network batches share one joint tip
- writers never admit Platform facts with truncated/incomplete/unsupported/failed status
- collectors prove balance@hash before write (EVM hash-bound; BTC tip/hash)
- no mixed certified collectors+report draft; composition is external multi-run only
- no dual public truth: live portfolio crawl deleted when fact-backed report lands
