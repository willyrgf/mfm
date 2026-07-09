# Implementation Plan: Fact-Backed Portfolio Collectors And Reports

**Source RFC:** `RFC_COLLECTORS_PORTFOLIO.md`  
**Branch:** `collectors-portf` (breaking-change branch)  
**Delivery:** **one PR** to `dev`, progressive **commits** (series A→E)  
**Status:** Ready to implement (architect-hardened)

---

## Session rules (non-negotiable)

0. **Fewer concepts / paths / types / duplicated responsibilities.** Prefer delete over wrap. LOC reduction is valuable; do not make code cryptic.
1. **No BC.** All breaking changes are allowed. Do not preserve soft-success, dual IO, or legacy type names "for compatibility."
2. **No fallbacks.** DELETE old code. No dual public truth on the merge tip. No "live if facts missing." No dual registration window.
3. **Progressive commits** within this branch (series A→E). Intermediate WIP may break portfolio; merge tip must be coherent.
4. Follow **`docs/code-quality.md`** (no hacks, no partial shims, no fragile schema workarounds).
5. If design is unclear mid-implementation, **stop**. Resolve against this plan + RFC. **Do not invent dual paths.**

---

## How to use this plan (implementers)

1. Read **Session rules**, **Forbidden patterns**, and **Locked decisions** before writing code.
2. Treat **series order as hard**: A → B → C → D (preferred) → E (later). Do not start C until A+B green.
3. Within a series, follow the **Commits** list in order. Each commit should be reviewable and preferably green under focused Cargo checks.
4. For every series, complete the **Done when** checklist before moving on.
5. When a file appears in **DELETE**, remove it completely in the named series — do not leave dead re-exports or unused adapters "just in case."
6. Prefer pure unit tests for selection / projection / normalize. Adapter tests prove hydrate + evidence. Integration tests use real fact admission fixtures, not SQL pokes.
7. If two designs both "work," pick the one with fewer types and fewer code paths. Re-read Forbidden patterns.
8. **Series C is atomic for public meaning:** C1–C5 land as progressive commits, but there is **zero dual registration window** — live pin/observe must not remain registered beside SelectHoldings on any commit tip intended as "report cutover complete." Prefer C3 (topology flip + unregister live) and C4 (delete dead types) in rapid succession without shipping a hybrid tip.
9. Do not start series E work on this branch unless A–D (or A–C with documented collector lag) are done.
10. Final merge readiness: `nix run .#check`, `nix run .#test`, `nix run .#test-db` (when store/facts touched), then `nix run .#ci`.

---

## Executive summary

Today `portfolio_snapshot` is a **live crawler** with soft partial success:

```text
ResolveSubjects
  → PinViews              (live head)
  → ResolveValuations     (input: PinnedViews; supports DirectPrice)
  → ObserveBatch*         (live balance fanout per wallet×symbol)
  → MergeObservations
  → AssembleSnapshot      (pins from live views; soft errors accumulate)
  → ProjectReport         (error_count from soft errors)
```

Target after cutover — **select-centric, hard-fail, report-only**:

```text
collectors (separate runs): ObserveSource → RecordDataFact

portfolio_snapshot:
  ResolveSubjects
    → SelectHoldings           (Platform fact-index + pure network-coherent select)
    → ResolveValuations        (pure FixedUnitPrice only; no views)
    → AssembleSnapshot         (network_pins from selected holding anchors)
    → ProjectReport
```

**DELETE entirely at cutover:**

- `PinViews*`, `ObserveBatch*`, `MergeObservations*`
- Live portfolio read capability + transport factory
- Soft-success system: `PortfolioSnapshot.errors`, `PortfolioSnapshotError`, batch/valuation soft `errors`, `PortfolioReport.error_count`
- View-dependent valuation (`DirectPrice`, `DerivedUnitPrice`) from cutover surface
- Any `QueryHoldingFacts*` per-symbol crawl fanout design

**As-of authority forever:** anchors on selected Platform data facts + named selection policy  
`mfm.portfolio.holding.latest-network-coherent.v1`.  
**`network_pins` forever:** pure projection of selected required holding anchors — never config, never live head, never pin-facts.  
**Cutover kinds:** mandatory height+hash; collectors prove balance@anchor before write.

```text
A (Platform FactIndexRead + shared fact query kit)
  └─► B (holding schemas + states/evm + fixtures + pure normalize)
        └─► C (atomic public cut: select + topology flip + DELETE live + soft errors)
              └─► D (configured collectors; family op monocrates; MUST share tip in-batch)
                    └─► E (prices, at-or-before, discovery)
```

**Merge tip:** A+B+C required. D preferred before merge. E later.

---

## Current ground truth (branch start)

| Area | Today | Implication |
|---|---|---|
| `FactIndexReadRequest` | Rejects non-Control (`NonControlAudience`) in `crates/fact-capabilities/src/lib.rs` | Series A widens to Platform |
| BTC Control query | `adapters/btc-jsonrpc` hydrate + evidence for checkpoints | Extract **shared fact kit** helpers; do not copy into portfolio |
| BTC fact record | `BtcFactRecordCapability` + managed write | Shared fact-record role only when second writer lands (D) |
| Portfolio graph | Live `PinViews` + `ObserveBatch*` fanout + `MergeObservations` | **Replace** with single `SelectHoldings`; **DELETE** live + merge |
| Portfolio adapter | Live pin/observe + `PortfolioTransportFactory` | **DELETE** at C; become report-only Platform fact-index runner |
| Soft errors | `PortfolioSnapshot.errors`, `PortfolioSnapshotError`, `ResolvedValuations.errors`, `ObservationBatch.errors`, `MergedObservations.errors`, `PortfolioReport.error_count` | **DELETE entire soft-success system** at C |
| `ResolveValuations` | Input = `PinnedViews`; supports `DirectPrice` | Drop views; **DELETE** view-dependent readers from cutover surface |
| Holding facts | None | B defines schemas; D writes them |
| `states/evm` | **Missing** (only `states/evm-contracts`) | **New monocrate** at B |
| Platform reads | App public-facts CLI/REST only | **Never** report authority |
| Mixed collector+report draft | Not present | **Rejected** for cutover (external multi-run only) |

---

## Forbidden patterns

These are **hard rejects** in review. Do not introduce them "temporarily."

| # | Forbidden | Why |
|---|---|---|
| F1 | Dual live+facts portfolio path / "live if facts missing" | Dual truth |
| F2 | Dual registration of PinViews/ObserveBatch **and** SelectHoldings | Dual truth window |
| F3 | `QueryHoldingFacts*` per-required-symbol crawl fanout graph | Wrong shape; selection is one state + pure fn |
| F4 | Keep `MergeObservations` "because NonEmpty fanout needed" after SelectHoldings | Redundant; SelectHoldings emits the full observation set |
| F5 | Soft-success: successful snapshot with `errors: [...]` / zeroed required holdings | False completeness |
| F6 | Keep `PortfolioSnapshot.errors` / `PortfolioSnapshotError` / `error_count` empty "for BC" | No BC; delete the system |
| F7 | Report pin table / config `network_pins` as selection input | Dual as-of authority |
| F8 | Pin-facts / chain-head facts as report pin authority | Collector ops only |
| F9 | Independent per-holding latest then fail on pin disagreement | Misses coherent older common anchors (A@100/99 + B@99) |
| F10 | `limit=1` latest-per-holding as selection (skip intersection) | Same as F9 |
| F11 | Second `PlatformFactIndexReadCapability` | Widen existing request |
| F12 | Public-facts CLI/REST as certified report authority | Wrong evidence class |
| F13 | Collectors Platform-query prior portfolio holdings for observe | Observe is source IO |
| F14 | Ambient `now()` / wall-clock freshness as uncertified selection | Must be fact-carried later |
| F15 | Height-only anchors for cutover BTC/EVM holding kinds | Hash mandatory |
| F16 | Write Platform fact with `truncated` / `incomplete` / `unsupported` / `failed` | Writers fail closed |
| F17 | SQL insert into `fact_index` without `FactRecorded` admission | Breaks authority |
| F18 | `portfolio-facts` crate / cross-family `HoldingsFact` soup | Family monocrates |
| F19 | `SelectionPolicy` trait / pluggable policy registry | Fixed policy id + pure fn |
| F20 | Mega collector framework / crate-per-collector for thin expand | Family op monocrates |
| F21 | Mixed certified draft: collectors + report in one `portfolio_snapshot` expand | External multi-run only |
| F22 | View-dependent valuation (`DirectPrice` / `DerivedUnitPrice`) on cutover surface | Price phase later |
| F23 | Soft `unacceptable_coverage` / `unacceptable_source_status` as separate public codes when empty set | Collapse to `missing_fact` |
| F24 | `pin_mismatch` error code | Implies pin table |
| F25 | LWW using floats / wall clock / non-deterministic order | Use `store_commit_order` only (v1) |
| F26 | Shared-tip optional for multi-subject same-network batch in D | **MUST** share tip in-batch |
| F27 | Partial dual path "for demos" while fixtures land | Fixtures or collectors only |

---

## Target architecture (normative)

```text
protocol IO (transports)
  -> family adapter binds observe/record runners
  -> family collector Operation (snapshot or progressive)
  -> durable Platform data facts (each with explicit anchor)
     (+ Control cursors only when progressive)
  -> portfolio report Operation (Platform fact-index only; certified selection)
  -> PortfolioSnapshot + PortfolioReport
     (network_pins always projected from selected holding anchors)
```

```text
collectors: observe(source @ joint anchor) → Platform data facts
report:     certified policy → select facts → Observations + network_pins(from selected anchors)
replay:     recorded fact-query evidence only
```

Public surfaces after cutover:

| Surface | Meaning |
|---|---|
| `portfolio_snapshot` | fact-backed **report only** |
| collector entry points / internal ops | source IO that records facts |
| CLI/docs "full refresh" | **external** multi-run orchestration (collect, then report) — not a dual-IO certified op |

---

# Public API Design (minimum surface)

Architect consensus: **minimum public surface**. No traits for selection, hydrate, or normalize. No second fact-index stack. No pin table. Series A must not invent portfolio report APIs.

## 1. Platform FactIndexRead (series A)

**Crate:** `mfm-fact-capabilities` (`crates/fact-capabilities/src/lib.rs`)

| Item | Decision |
|---|---|
| Capability | Keep single `FactIndexReadCapability` (`mfm.fact.index.read`) |
| Provider | Keep `FactIndexReadProvider::read_fact_index` |
| Request | `FactIndexReadRequest::new(plan)` — allow plan audience ∈ `{Control, Platform}` |
| Response / Evidence | Unchanged (`FactIndexReadResponse`, `FactIndexReadEvidence`) |
| Errors | Replace `NonControlAudience` with allowlist failure (`UnsupportedAudience` or rename reason to cover both rejected audiences) |
| Hydration | **Not** on the capability trait. Adapter/shared kit hydrates retained `FactResponse` artifacts |
| Second capability for Platform | **Forbidden** |

**Shared fact kit (A packaging):** extract pure/private helpers following BTC Control runner patterns — not a public `FactHydrator` trait. Prefer `pub(crate)` or pure fns callable from BTC and later portfolio adapters without inventing a mega framework. Packaging may live as:

- shared module under an existing adapter tree temporarily, **or**
- small pure helpers next to fact capability / runtime-adjacent code

**Do not** create `adapters/fact` preemptively unless extraction is forced by a second real consumer (portfolio report in C). Prefer extracting only what BTC already does.

**Who may call Platform index in certified runs:** portfolio report states (C). Collectors must not Platform-query prior portfolio holdings.

## 2. Coverage / source status (series B)

**Single pure owner** (prefer `mfm-facts` / `crates/kernel/facts`; no new crate unless forced):

```text
CoverageStatus:
  complete_at_anchor | configured_only | truncated | incomplete

HoldingSourceStatus:   // NOT BtcSourceStatus (synced/ibd/unknown)
  ok | unsupported | failed
```

| Value | Write Platform fact? | Report accept (v1)? |
|---|---|---|
| `complete_at_anchor` | yes | yes |
| `configured_only` | yes | yes |
| `truncated` / `incomplete` | **fail closed before write** | no (never admitted → never selected) |
| `ok` | yes | yes |
| `unsupported` / `failed` | **fail closed before write** | n/a |

Wire as string tags on fact responses (no floats). Public enums with parse/`as_str`.

**Writers only admit admissible coverage/status.** Report allow-lists then become:

- coverage ∈ `{configured_only, complete_at_anchor}`
- source_status ∈ `{ok}`

If a candidate fails allow-list, it is not acceptable. Empty acceptable set → `missing_fact` (no separate `unacceptable_*` public codes).

## 3. Holding fact types (series B)

### `bitcoin.address_balance_snapshot` — `mfm-states-btc` (`crates/states/btc`)

| Type | Role |
|---|---|
| Subject | `network`, `bitcoin_network`, `semantic_source_identity`, `address` |
| Response | **mandatory** `anchor_height`, **mandatory** `anchor_hash`, `balance_sats`, `coverage`, `source_status` |
| Fact | `MfmFactType`, Platform visibility, descriptor orderings for selection |

Style: private fields + public accessors (match existing `BtcChainHeadFact` style).  
**Never** on fact: `wallet_id`, `symbol_id`, RPC URLs, provider diagnostics.

### `evm.address_native_balance_snapshot` — **new** `mfm-states-evm` (`crates/states/evm`)

| Type | Role |
|---|---|
| Subject | `network`, `chain_id`, `account` |
| Response | **mandatory** `block_number`, **mandatory** `block_hash`, `raw_wei` decimal string, `decimals`, `coverage`, `source_status` |
| Fact | Platform; same discipline |

**Do not** put holding facts in `states/evm-contracts` or invent `portfolio-facts`.

### Normalize split

| Layer | Public API | Responsibility |
|---|---|---|
| Family state | pure fn fact/response → source-near holding (amount, decimals, anchor, coverage) | No portfolio IDs |
| Portfolio state | pure fn requirement + selected material + valuation → `Observation` | Adds wallet/symbol join |

No `ObservationNormalizer` trait. No cross-family `HoldingsFact` enum.

## 4. Selection policy (series C)

**Not a trait. Not a registry. Not a pluggable policy object.**

```text
PORTFOLIO_HOLDING_LATEST_NETWORK_COHERENT_POLICY_ID =
  "mfm.portfolio.holding.latest-network-coherent.v1"

// digest: ContentDigest via sha256 of policy id bytes
// (mirror BTC checkpoint CHECKPOINT_QUERY_SELECTION_POLICY style)
```

Home: `mfm-state-portfolio` as `pub const` + small digest helper.

### Candidate query shape (normative)

For each required holding, the certified plan loads an **Exact / full candidate set** for that subject under Platform audience — **not** `limit=1` latest.

- Query plan must return **all** acceptable candidates needed for intersection (descriptor-supported orderings may help local ranking, but selection correctness must not depend on store-side limit-1).
- Receipt-pinned selection: selected indices / claim refs bound into `FactSelectionEvidence` after pure selection.
- Hydration of retained `FactResponse` artifacts is mandatory for balance payloads.

### Algorithm: `select_network_coherent` (pure, normative)

Inputs (after hydrate + subject projection filters):

```text
for each required holding h:
  candidates[h] = list of Candidate {
    network_id,
    anchor: (height: u64, hash: bytes/hex-normalized),
    store_commit_order: u64,   // from index/receipt ordering material
    fact_claim_id / fact_ref,  // for evidence indices only
    response_material,         // hydrated holding payload
  }
  // candidates already filtered to acceptable coverage + source_status
```

Steps:

1. Group required holdings by `network_id`.
2. For each holding `h` in group `G`:
   - if `candidates[h]` empty → fail `missing_fact`.
3. Compute `common_anchors = intersection over h in G of { candidate.anchor for candidate in candidates[h] }`  
   Anchor identity for cutover kinds: **`(height, hash)`** (hash required).
4. If `common_anchors` empty → fail `no_common_network_anchor`.
5. Choose `chosen_anchor = max(common_anchors)` by:
   - `height` descending
   - then `hash` bytes descending (deterministic tie-break)
6. For each holding `h` in `G`:
   - let `at = filter candidates[h] where anchor == chosen_anchor`
   - if `at` empty → `missing_fact` (should not happen if intersection correct; still fail closed)
   - if multiple: **LWW v1 = order by `store_commit_order` descending only**; accept first  
     - Do **not** require `fact_claim_id` as a second sort key unless store guarantees total order without it; prefer **store_commit_order only** (closed decision).
   - Emit selected candidate + bind selection evidence indices against the receipt-ordered rows for that holding's query.
7. After all groups: if residual same-network selected anchors disagree → fail `inconsistent_network_anchors` (guard / bug).
8. If plan/receipt cardinality violated after policy → `ambiguous_facts`.

**Canonical unit case:**

```text
Holding A candidates: @100, @99
Holding B candidates: @99
Same network
→ common = {@99}
→ chosen = @99
→ select A@99 and B@99
// Independent latest would wrongly pick A@100 + B@99 then fail.
```

### Error codes (minimal, hard-fail run)

| Code | When |
|---|---|
| `missing_fact` | no acceptable Platform fact for a required subject (includes coverage/status filter-empty) |
| `no_common_network_anchor` | network group has acceptable facts but empty intersection of anchors |
| `ambiguous_facts` | selection cardinality violated after policy / receipt |
| `inconsistent_network_anchors` | residual same-network anchor disagreement after selection (guard) |
| `unsupported_requirement` | portfolio requirement has no projection rule (e.g. ERC-20 at cutover) |

**Deleted / not used at cutover:**

- `unacceptable_coverage`, `unacceptable_source_status` → fold into `missing_fact`
- `pin_mismatch`
- soft snapshot error codes that allow run success

**Series E only (later):** `as_of_not_exact`, optional `conflicting_facts`.

## 5. Portfolio report states after cutover (series C)

| State | Status |
|---|---|
| `ResolveSubjectsState` | Keep |
| `SelectHoldingsState` | **New** — Platform fact-index + pure select; emits full selected observation set |
| `ResolveValuationsState` | Keep; **no** `PinnedViews` input; FixedUnitPrice only |
| `AssembleSnapshotState` | Keep; **no** views; pins from selected observations; **no soft errors** |
| `ProjectReportState` | Keep; **no** `error_count` |
| `PinViewsState` + configs/types | **DELETE** |
| `ObserveBatchState` + configs/types | **DELETE** |
| `MergeObservationsState` + configs/types | **DELETE** (redundant under select-centric graph) |
| `PortfolioReadCapability` + intents + `RawBalanceObservation` | **DELETE** |
| Soft error types / fields | **DELETE** |

### Select-centric graph (only)

```text
ResolveSubjects
  → SelectHoldings          // subjects in; selected holdings + anchors out
  → ResolveValuations       // pure fixed prices; no PinnedViews
  → AssembleSnapshot        // subjects + selected holdings + valuations; pins from holdings
  → ProjectReport
```

**Never** invent:

```text
// FORBIDDEN
ResolveSubjects → ResolveValuations → QueryHoldingFacts* → MergeObservations → ...
```

Fanout for candidates may exist **inside** the SelectHoldings adapter runner (N Platform queries), but the **typed program graph** has one selection state, not N observe/query nodes + merge.

### `SelectHoldingsState` (minimum shape)

| Piece | Responsibility |
|---|---|
| Config | portfolio requirements material: wallets/symbols/networks needed for projection; store_scope; fixed policy id; allow-lists; certified constants |
| Input | `ResolvedSubjects` only (no pins, no live views) |
| Caps | `(FactIndexReadCapability,)` only |
| Output | Selected holdings material sufficient to build observations **or hard-fail the state/run** |

**What descriptors / queries must return for selection:**

- Subject filter fields for exact subject match (BTC: network, bitcoin_network, semantic_source_identity, address; EVM: network, chain_id, account)
- Returnable/hydrated response: mandatory height+hash, balance payload, coverage, source_status
- Index/receipt material that exposes **`store_commit_order`** for LWW among same subject+anchor
- Enough rows for full candidate sets (no silent truncation of the candidate universe)

**Evidence shape:**

```text
for each required holding query:
  compile CanonicalFactQueryPlan (Platform audience, subject predicates, no limit-1 selection)
  → FactIndexRead
  → receipt + trust root + rows
  → hydrate FactResponse artifact(s)
  → filter acceptable candidates
  → pure select_network_coherent across the network group
  → FactSelectionEvidence with selected indices bound to receipt order
  → materialize Observation fields (anchor included)
```

Replay: recorded fact-query evidence + retained response artifacts only. Never re-query live frontier. Never construct live chain providers.

### Valuations (cutover)

- Drop `PinnedViews` dependency entirely.
- **DELETE** `DirectPrice` / `DerivedUnitPrice` from the cutover surface (validation path, config decode, public valuation API used by report). FixedUnitPrice only (including intentional dual-mainnet zeros). Reintroduce view-dependent readers in series E as **new work** when price facts land — not as feature-flagged leftovers.
- Valuation resolution is pure and **hard-fail** on missing fixed route — no soft `ResolvedValuations.errors`.

### Assemble / pins

```text
// DELETE AssembleSnapshotInput.views
// DELETE soft error accumulation into PortfolioSnapshot.errors
// ADD pure:
fn project_network_pins_from_observations(
  observations: &[Observation],
) -> Result<Vec<NetworkPin>, AssembleError>
```

- One pin per network among required selected holdings.
- Under network-coherent policy, same-network anchors already match; residual disagreement → `inconsistent_network_anchors`.
- Pins are **never** selection inputs.

### Soft-success system — DELETE list (complete)

| Item | Location |
|---|---|
| `PortfolioSnapshot.errors` field | `crates/portfolio/model/src/portfolio.rs` |
| `PortfolioSnapshotError` type + schema | same |
| `PortfolioReport.error_count` | same |
| `ResolvedValuations.errors` | `crates/states/portfolio/src/lib.rs` |
| `ObservationBatch.errors` | same (type may die with ObserveBatch) |
| `MergedObservations.errors` | same (type dies with MergeObservations) |
| Soft push paths in resolve/observe/assemble | same |
| Tests asserting success with non-empty errors / `error_count > 0` | model tests, CLI, integration |

After cutover: success means **complete required set**; any required failure fails the run with a stable public error code.

## 6. Keep as public report DTOs (reshape, do not soft-delete)

- `PortfolioSnapshot` (**without** `errors`)
- `PortfolioReport` (**without** `error_count`)
- `Observation*`, `NetworkPin`, `ExecutionAnchor`
- Portfolio authored/canonical config (no pin-table / as-of-input fields at cutover)

## 7. Naming that paints corners (forbid)

| Bad | Prefer |
|---|---|
| Residual `PinViews` / input `network_pins` | `SelectHoldings`; pins output-only |
| `QueryHoldingFacts*` fanout + `MergeObservations` | single `SelectHoldings` + pure select |
| `SelectionPolicy` trait | string `policy_id` + fixed functions |
| `ReportAsOf` authority type | later bounds as policy params |
| `pin_mismatch` | `missing_fact` / `no_common_network_anchor` |
| Independent per-holding latest | network-coherent common anchor selection |
| `SourceStatus` without qualifier | `HoldingSourceStatus` (not `BtcSourceStatus`) |
| Cross-family `HoldingsFact` | monomorphic family facts |
| Separate `PlatformFactIndexReadCapability` | widen existing request |
| Soft `errors: []` retention | delete field |

## 8. Must be public vs private

| Item | Visibility | When |
|---|---|---|
| `FactIndexRead*` | Public (`fact-capabilities`) | A |
| Shared fact query/hydrate helpers | Private / `pub(crate)` kit | A |
| Holding fact types + descriptors | Public (family states) | B |
| Coverage / HoldingSourceStatus | Public (single pure owner) | B |
| Normalize pure fns | Public fns, not traits | B |
| Fixture admission | test_support / `#[cfg(test)]` | B |
| `SelectHoldings*` + policy id + `select_network_coherent` | Public (`states/portfolio`) | C |
| `project_network_pins_from_observations` | Crate-public pure fn | C |
| Live pin/observe/read capability / transport factory | **Deleted** | C |
| Soft error types/fields | **Deleted** | C |
| Collector observe/record states | Public (family states) | D |
| Shared `FactRecordCapability` | Public when second writer needs it | D |
| Family collector op monocrates | Public ops | D |
| at-or-before bounds types | **Not yet** | E |

---

# Packaging schedule (intentional — first-class progressive work)

Packaging renames and monocrates are **long-term structure**, not polish. Wrong homes become permanent dual places for future work. They do not block the public-meaning cut (C3), but they **are scheduled progressive work** on this branch and must not be left as temporary forks.

| Series | Packaging intent | Do / Don't |
|---|---|---|
| **A** | Shared fact kit (query plan validation + hydrate/response-ref helpers) | Extract BTC pattern for second consumer; **no** portfolio-private copy of hydrate/evidence; no new mega crate |
| **B** | `states/evm` monocrate for first EVM holding fact | New crate; not `evm-contracts`; not portfolio |
| **C** | `adapters/portfolio` becomes **report-only** (Platform fact-index runner) | Delete transport factory + live runners in the public cut series |
| **D** | Family op monocrates: `ops/btc-collectors-op`, `ops/evm-collectors-op` | Fold/rename chain-head into BTC collectors monocrate when second BTC cycle lands; land EVM monocrate with EVM collectors; no crate-per-recipe forever |

**Follow-on packaging (still intentional, not merge-gate for C meaning flip):**

- Promote shared kit home to `adapters/fact` (or agreed durable home) once two real consumers force a clear boundary.
- Rehome out-of-taxonomy leftovers (e.g. `collectors/proof`) into the placement rules.
- Do **not** leave temporary second hydrate/record implementations "until rename."

---

# Series A — Platform fact-index substrate + shared fact kit

### Goal

Certified states can read **Platform** facts with Control-parity evidence/replay. Shared hydrate / response-artifact helpers exist so series C will not copy the BTC Control runner. **Portfolio public behavior unchanged.**

### Commits (suggested)

1. Allow Platform on `FactIndexReadRequest`; update `FactIndexInvalidRequest` reason + unit tests in `fact-capabilities`.
2. Extract shared hydrate / response-artifact requirement helpers; BTC Control checkpoint runner uses them.
3. Provider/integration smoke: Platform plan against store projection (where available).

### Crates / files

| Path | Action |
|---|---|
| `crates/fact-capabilities/src/lib.rs` | Allow Platform audience; update docs/errors/tests |
| `crates/adapters/btc-jsonrpc/src/lib.rs` (+ tests) | Call shared kit; keep Control checkpoint path green |
| Store fact query path (spot-check) | Confirm audience filtering accepts Platform when authorized |
| Test support in-memory fact-index providers | Rename Control-only assumptions if present |

### ADD

- Platform audience allowlist on `FactIndexReadRequest::new`
- Shared pure helpers for fact-response artifact requirement + hydrate-by-ref (kit)
- Rustdoc: Platform reads require certified evidence obligations; not public-facts authority

### DELETE

- Control-only guard and tests asserting Platform must fail (`NonControlAudience` Platform case)
- Do **not** delete BTC Control checkpoint path
- Do **not** add portfolio report APIs
- Do **not** invent `SelectionPolicy` traits

### Tests

- Platform plan accepted; Control still works
- Unsupported audience rejected
- Evidence/receipt shape parity
- Provider redaction unchanged
- BTC checkpoint path still green
- Shared kit unit tests for hydrate requirement helpers

### Done when

- [ ] Platform `FactIndexReadRequest` works end-to-end against store projection
- [ ] BTC uses shared hydrate path
- [ ] No public-facts used as certified read path
- [ ] Portfolio still compiles (even if later commits break it temporarily)
- [ ] No new portfolio public report types shipped in A

### Risks

- Store path assumes Control-only somewhere — verify early
- Over-extracting a "fact framework" — only extract BTC's existing pattern
- Renaming `NonControlAudience` carefully so error messages stay redaction-safe

---

# Series B — Holding fact schemas + `states/evm` + fixtures

### Goal

Dual-mainnet **native** fact kinds exist with **mandatory height+hash**, coverage/source, Platform visibility (returnable field table per RFC), fixture admission via real `FactRecorded`, and pure normalize helpers. Collectors not required yet. Writers-only admissible coverage/status are encoded in normalize fail-closed rules (even before collectors land).

### Commits (suggested)

1. `CoverageStatus` + `HoldingSourceStatus` pure enums (single owner)
2. `bitcoin.address_balance_snapshot` fact types + descriptors + tests (mandatory hash; fail closed if missing)
3. New `mfm-states-evm` crate + `evm.address_native_balance_snapshot` (mandatory hash)
4. Fixture admission test_support (real claim + artifact + projection — **no SQL index poke**)
5. Pure normalize helpers: fact material → observation fields (unit tests)

### Crates / files

| Path | Action |
|---|---|
| `crates/kernel/facts` (or agreed pure owner) | Coverage / HoldingSourceStatus |
| `crates/states/btc/src/lib.rs` | BTC balance fact + descriptor |
| **New** `crates/states/evm/` | Native balance fact crate |
| Workspace `Cargo.toml` | Register `mfm-states-evm` |
| test_support / integration harness | Real `FactRecorded` admission path |
| `crates/states/portfolio` | Pure projection helpers only if needed for normalize tests (no SelectHoldings yet) |

### ADD

- Fact kinds + descriptors with sortable/queryable anchor height **and** hash fields
- Visibility helpers Platform indexed; classify returnable vs hidden per RFC field table
- Fail-closed normalize when height or hash missing
- Fail-closed normalize when coverage/status not admissible for write (`truncated`/`incomplete`/`unsupported`/`failed`)
- Fixture API e.g. admit holding fact through real admission
- Descriptor orderings that support candidate listing and `store_commit_order` exposure for later LWW

### DELETE

- Nothing portfolio-live yet
- No ERC-20 fact kind at cutover
- No soft portfolio error types yet (deleted in C)

### Tests

- Descriptor field ids / orderings
- Missing height or hash rejected at normalize/admit
- Closed enum parse
- Fixture: Platform query by subject predicates returns fact
- Response JSON round-trip (no floats)
- Pure unit: BTC sats / EVM wei → observation quantity + full anchor
- Inadmissible coverage/status cannot admit as Platform holding fact

### Done when

- [ ] Both native kinds admit + query under Platform in tests
- [ ] Missing hash cannot admit
- [ ] Only admissible coverage/status can be written
- [ ] No raw SQL index poking
- [ ] Report graph may still be live (branch WIP OK until C)

### Risks

- Multi-term ordering support in fact index — confirm `store_commit_order` available to selection
- New crate vs stuffing into `evm-contracts` — **always new monocrate `states/evm`**
- Do not put `wallet_id` / `symbol_id` on facts

---

# Series C — Atomic public cut (report-only + delete live + delete soft errors)

### Goal

Public `portfolio_snapshot` is **report-only**. Live pin/observe **deleted**. Soft-success system **deleted**. Graph is select-centric. `latest-network-coherent.v1` only. Hard-fail. `network_pins` from selected anchors only. Parity tests off live Reth crawl; report succeeds without live chain when facts are present.

**ZERO dual registration window:** do not leave PinViews/ObserveBatch registered beside SelectHoldings on any tip claimed as cutover-complete. Land C3 and C4 tightly.

### Commits (atomic series — order is hard)

| Commit | Name | What lands |
|---|---|---|
| **C1** | Pure select | Subject projection table + `select_network_coherent` + pin projection pure fns + unit tests (incl. A@100/99 + B@99 → both @99) |
| **C2** | SelectHoldings state | `SelectHoldingsState` + config/input/output + adapter Platform fact-index runner using shared kit + evidence |
| **C3** | Topology flip + unregister live | Op expand rewire to select-centric graph; **unregister** live pin/observe/merge from certification + adapter registration; valuations without views; **DELETE** DirectPrice path |
| **C4** | Delete dead types + soft errors | DELETE PinViews/ObserveBatch/MergeObservations/PortfolioRead*/transport factory; DELETE soft error fields/types/paths; fix all compile breakages |
| **C5** | Fixture tests + docs | Rewrite integration/parity to fixtures; report succeeds without live providers; docs: report-only, useless without facts |

### Crates / files

| Path | Action |
|---|---|
| `crates/states/portfolio/src/lib.rs` (+ tests, README) | Add SelectHoldings; reshape valuations/assemble/project; DELETE live + soft |
| `crates/ops/portfolio-tracker-op/src/lib.rs` | Topology cutover; registry update |
| `crates/adapters/portfolio/src/lib.rs` (+ tests, Cargo.toml) | Report-only Platform runner; DELETE transports |
| `crates/portfolio/model/src/portfolio.rs` (+ tests) | DELETE `errors`, `PortfolioSnapshotError`, `error_count` |
| `crates/portfolio/model/src/symbol.rs` | Cutover valuation surface: FixedUnitPrice only |
| `crates/app/src/*` | Registration cleanup for live portfolio transports |
| Integration / parity tests | Fixtures; no live Reth crawl as portfolio truth |
| CLI / docs (`bin/cli/README.md`, portfolio READMEs) | Report-only semantics |

### ADD

```text
ResolveSubjects
  → SelectHoldings           // Platform fact-index + pure network-coherent select
  → ResolveValuations        // pure FixedUnitPrice only
  → AssembleSnapshot         // pins from selected holding anchors; hard-fail only
  → ProjectReport
```

- Policy constants + pure `select_network_coherent(...)` + selection evidence helpers
- Subject projection: BTC + EVM native only; else `unsupported_requirement`
- Hard-fail error mapping: `missing_fact`, `no_common_network_anchor`, `ambiguous_facts`, `inconsistent_network_anchors`, `unsupported_requirement`
- Candidate Exact/full-set query plans (no limit-1 selection)

### DELETE (complete list)

**`states/portfolio`**

- `PinViewsState`, `PinViewsConfig`, `PinnedView`, `PinnedViews`
- `ObserveBatchState`, `ObserveBatchConfig`, `ObserveBatchInput`, `ObserveBatchInputHandles`
- `MergeObservationsState`, `MergeObservationsConfig`, `MergedObservations`
- `ObservationBatch` used only by observe/merge fanout (replace with SelectHoldings output type)
- `PortfolioReadCapability`, `PortfolioNetworkReadIntent`, `PortfolioBalanceReadIntent`
- `RawBalanceObservation`, `PortfolioReadError`
- Live helpers (`pin_view_read_intents`, `pinned_view_for_network`, `observation_batch_from_raw_balance`, …)
- Soft error accumulation helpers (`snapshot_error`, error sort keys used only for soft merge, …)
- `ResolvedValuations.errors` field and soft valuation error paths
- Any residual `QueryHoldingFacts*` design if introduced experimentally

**`portfolio/model`**

- `PortfolioSnapshot.errors`
- `PortfolioSnapshotError` (+ schema)
- `PortfolioReport.error_count`
- Tests that encode soft partial success

**`adapters/portfolio`**

- `PinViewsRunner`, `ObserveBatchRunner`, `MergeObservationsRunner` (if present)
- `PortfolioTransportFactory`, portfolio EVM/BTC live providers / caches
- Live head/balance execution

**`ops/portfolio-tracker-op`**

- Graph edges for pin/observe/merge
- `ViewDomainKey` usage if only for pins
- Registry entries for deleted states
- Re-exports of deleted types

**App / tests / docs**

- Portfolio-only live transport wiring
- Reth live crawl parity as portfolio truth
- Docs implying live crawl or soft success

### KEEP / RESHAPE

- `ResolveSubjectsState`
- `ResolveValuationsState` (input no longer `PinnedViews`)
- `AssembleSnapshotState` (no views; no soft errors)
- `ProjectReportState` (no error_count)
- `PortfolioSnapshot` / `PortfolioReport` / `Observation` / `NetworkPin` / `ExecutionAnchor` (fields as above)

### SelectHoldings algorithm steps (implementer checklist)

1. From config + `ResolvedSubjects`, expand required holdings (wallet×configured native symbols).
2. For each required holding, project fact kind + subject predicates (table below).
3. Build Platform `CanonicalFactQueryPlan` for **full candidate set** (Exact subject match; not limit-1 latest).
4. `FactIndexRead` → validate receipt → hydrate responses via shared kit.
5. Filter each row to acceptable coverage/status; drop non-matching subjects.
6. Run pure `select_network_coherent` per network group.
7. On success, normalize each selected fact → observation fields (join wallet_id/symbol_id + later valuation join at ResolveValuations/Assemble as designed).
8. Record selection evidence (policy digest + selected indices) for each query.
9. On any required failure, fail the state/run with minimal codes — never soft-append.

### Tests (must include)

| Case | Expected |
|---|---|
| Topology | No pin/observe/merge nodes; SelectHoldings present |
| Projection table | BTC native + EVM native OK; ERC-20 → `unsupported_requirement` |
| Network-coherent | A@100 + A@99, B@99 → both selected @99 |
| Empty common set | A@100 only, B@99 only → `no_common_network_anchor` |
| Missing fact | zero candidates → `missing_fact` |
| Coverage filter-empty | only `truncated` candidates → `missing_fact` (not soft success) |
| LWW | same subject+anchor two commits → higher `store_commit_order` wins |
| Pins projection | `network_pins` match selected anchors only |
| Residual disagree | guard → `inconsistent_network_anchors` |
| Adapter Platform query + hydrate → Observation | success |
| Replay | no live index frontier re-query; no live chain providers |
| Fixture dual-mainnet report | **succeeds with chain providers unbound** |
| No facts | report fails hard |
| Soft errors gone | types/fields unreferenced; no `error_count` |
| Certification registry | green without deleted states |

### Done when

- [ ] Public meaning = report-only
- [ ] No live PinViews/ObserveBatch/MergeObservations registration
- [ ] No soft-success system in public DTOs
- [ ] No dual IO truth
- [ ] Report works without live chain when facts present
- [ ] Product honesty: useless without admitted facts
- [ ] Select-centric graph only (no QueryHoldingFacts fanout)

### Risks

- CLI/integration tests expecting soft `error_count > 0` success — rewrite to hard-fail
- NonEmpty fanout assumptions in op expand — SelectHoldings is one node; plan-time still requires non-empty configured symbols
- Temptation to keep MergeObservations — **delete it**
- Do **not** add pin table or at-or-before in C
- Do **not** ship C2 without a plan for C3/C4 in the same cutover sequence

---

# Series D — Configured collectors (preferred before merge)

### Goal

Operators collect dual-mainnet native balances into Platform facts, then report. Snapshot pattern only. **Write-time anchor integrity** (balance proven at mandatory height+hash). Multi-subject same-network batch **MUST share one observe tip**. Collector replay without live RPC. Packaging: family op monocrates.

### Pattern

```text
ObserveSource (joint tip + balance + coverage + source_status)
  → RecordDataFact (ManagedPlatformWrite, Platform)
```

- BTC: resolve tip once per batch, pin-in balance at that tip/hash; fail on tip drift
- EVM: balance at block hash (EIP-1898 or equivalent verification); fail if hash missing/mismatch
- Fail closed before write on unsupported/failed/truncated/incomplete
- Coverage default: `configured_only`
- Shared tip is **required in-batch**, not optional

### Prove-before-write checklist (every collector write path)

- [ ] Observation DTO includes **height and hash**
- [ ] Balance/holding payload was read **at** that hash (or provider response verified against it)
- [ ] Tip drift / hash mismatch → error **before** Platform write
- [ ] `coverage` ∈ `{configured_only, complete_at_anchor}` only (near-term default `configured_only`)
- [ ] `source_status == ok` only; `unsupported`/`failed` never write
- [ ] No secrets, RPC URLs, provider diagnostics in fact response
- [ ] No `wallet_id` / `symbol_id` on fact subject
- [ ] Multi-subject same network: **one joint tip resolved once** and applied to all observes in the batch
- [ ] Normalize fail-closed unit tests cover missing hash, tip drift, hash mismatch
- [ ] Replay path does not require live RPC

### Packaging (D)

| Family | Op monocrate | Notes |
|---|---|---|
| BTC | `ops/btc-collectors-op` | Fold/rename `btc-chain-head-collector-op` when second BTC cycle lands; chain-head remains **not** report pin authority |
| EVM | `ops/evm-collectors-op` | New when EVM collectors land |
| Portfolio | `ops/portfolio-tracker-op` | Report-only (already after C) |

Adapters:

- `adapters/btc-jsonrpc` — observe + record for BTC balance (+ existing chain-head)
- `adapters/evm` — when EVM collectors land
- Shared fact-record role/runners when second writer needs them; delete BTC-bound record exclusivity if still present

### Commits (suggested)

1. BTC address balance snapshot collector (observe → record; prove balance@anchor; joint tip API for multi-address batches)
2. EVM native balance collector (hash-bound reads; joint tip for multi-account same chain)
3. Shared `FactRecordCapability` if second writer requires it; app registration for collectors
4. Collect → report dual-mainnet integration; document external multi-run recipe
5. Optional: ERC-20 configured token collector **after** natives (not merge-gate)

### ADD

- Family observe/record states + thin collector ops in family monocrates
- App registration for collectors
- Optional shared `FactRecordCapability`
- Docs: collect-then-report recipe (external multi-run; **mixed certified draft REJECTED**)

### DELETE

- BTC-bound fact-record exclusivity if shared path lands
- Any residual portfolio live balance path
- Any temptation to call collectors from inside `portfolio_snapshot` expand

### Tests

- Normalize + fail-closed (missing hash, tip drift, hash mismatch, inadmissible coverage/status)
- Adapter mock transport
- Collector fails if providers unavailable or balance@hash unprovable
- Collector replay without live RPC
- Multi-subject batch shares tip → same anchor on all written facts
- Collect → report dual-mainnet → consistent `network_pins`
- Without shared tip simulation: network-coherent may fall back to older common or `no_common_network_anchor` (document operational expectation: always share tip)

### Done when

- [ ] Merge tip usable with RPC (not only SQL fixtures)
- [ ] Report still never live-reads chain
- [ ] Collect-then-report recipe documented
- [ ] In-batch shared tip enforced for multi-subject same network
- [ ] Prove-before-write checklist green in tests

### Risks

- Tip race without shared tip → empty common set / older anchors only — **MUST share tip**
- Op packaging churn when renaming chain-head into `btc-collectors-op`
- ERC-20 optional after natives; not merge-gate
- Do not Platform-query portfolio holdings inside collectors

---

# Series E — Later (same authority model; not cutover)

### In scope later

- Price facts + certified price selection join (not `network_pins` as fake price pins)
- `at-or-before.v1` + certified bounds (filters only; `network_pins` rule unchanged)
- Optional strict equality → `as_of_not_exact`
- Freshness as certified policy over fact-carried times
- Discovery collectors; ERC-20 configured set + `token_set_identity`
- Residual packaging polish only after intentional A–D packaging gates land (`adapters/fact` durable home if not already forced, leftover rehomes)
- Optional `conflicting_facts` if product wants fail-closed on same subject+anchor different response hash

### Still forbidden in E

- Report pin table
- Pin-facts as report authority
- Ambient `now()` selection
- Dual `network_pins` sources
- Soft-success partial snapshots without a real product requirement (if ever: new design, not a silent reintroduction)

---

# Subject projection table (cutover)

| Portfolio requirement | Fact kind | Predicates | Coverage accept |
|---|---|---|---|
| BTC native | `bitcoin.address_balance_snapshot` | `network`, `bitcoin_network`, `semantic_source_identity`, `address` | `configured_only`, `complete_at_anchor` |
| EVM native | `evm.address_native_balance_snapshot` | `network`, `chain_id`, `account` | `configured_only`, `complete_at_anchor` |
| ERC-20 | — | — | `unsupported_requirement` until later |

Source status accept (all cutover holdings): `{ok}` only.

---

# Verification gates

### Per series (while developing)

- Focused `cargo test -p <crate>` / `cargo check -p <crate>`
- Series A/B must not leave permanent dual portfolio truth on merge tip

### Before progressive commits on this branch (when feasible)

- `nix run .#check`
- `nix run .#test`
- `nix run .#test-db` when store/fact projection touched

### Merge tip (A+B+C+preferred D)

- [ ] Report consumes only Platform facts with recorded query evidence
- [ ] Report **succeeds without live chain providers** when required facts/artifacts present
- [ ] Report fails on missing facts / `no_common_network_anchor` / unsupported requirement
- [ ] Replay never constructs live providers / never re-queries live fact-index frontier
- [ ] Network-coherent unit: **A@100/99 + B@99 → both @99**
- [ ] Cutover facts reject missing block hash
- [ ] Writers reject inadmissible coverage/status
- [ ] No PinViews/ObserveBatch/MergeObservations registration
- [ ] No soft `PortfolioSnapshot.errors` / `PortfolioSnapshotError` / `error_count`
- [ ] No QueryHoldingFacts fanout topology
- [ ] Fixtures via real fact admission
- [ ] `network_pins` match selected fact anchors only
- [ ] Platform field exposure matches RFC table; no secrets/routing
- [ ] Mixed collector+report certified draft not present
- [ ] With D: collect-then-report works; shared tip in-batch; prove-before-write; collector replay clean
- [ ] `nix run .#ci` for final merge-readiness

---

# Locked decisions (expanded)

| # | Decision |
|---|---|
| L1 | Single as-of authority: fact anchors + selection policy; `network_pins` are projection only forever |
| L2 | One PR, progressive commits; delete live pin/observe at C with **no fallback** and **no dual registration window** |
| L3 | Holding facts live in family state crates (`states/btc`, `states/evm`); never `portfolio-facts` |
| L4 | Platform fact-index = extend existing capability; not public-facts CLI/REST |
| L5 | Select-centric report graph only: `ResolveSubjects → SelectHoldings → ResolveValuations → AssembleSnapshot → ProjectReport` |
| L6 | DELETE `MergeObservations` (select-centric graph owns the full observation set); never add `QueryHoldingFacts*` crawl fanout |
| L7 | Hard-fail configured symbols; **DELETE soft-success system** (`errors` fields, `PortfolioSnapshotError`, `error_count`) — no empty-field BC |
| L8 | Minimal public error codes; fold `unacceptable_*` → `missing_fact`; keep `no_common_network_anchor` distinct |
| L9 | Coverage / HoldingSourceStatus single pure owner; writers fail closed to admissible set only |
| L10 | Selection = `mfm.portfolio.holding.latest-network-coherent.v1` (not independent per-holding latest) |
| L11 | Candidate sets are Exact/full for intersection; not store `limit=1` latest |
| L12 | LWW v1 = **`store_commit_order` descending only** |
| L13 | Cutover BTC/EVM: mandatory height+hash; write-time balance@anchor integrity |
| L14 | Series A ships shared fact kit only — no portfolio report public APIs |
| L15 | Series B creates `states/evm` monocrate for first EVM holding fact |
| L16 | Series C makes `adapters/portfolio` report-only |
| L17 | Series D uses family collector op monocrates (`btc-collectors-op`, `evm-collectors-op`) |
| L18 | Series D multi-subject same-network **MUST** share tip in-batch |
| L19 | Mixed certified collector+report draft **REJECTED**; external multi-run only |
| L20 | View-dependent valuation (`DirectPrice` / `DerivedUnitPrice`) **DELETED** from cutover surface; E reintroduces as new work only |
| L21 | ERC-20 out of cutover scope (natives only) |
| L22 | Platform visibility intentional for public chain data; explicit field exposure; no secrets/routing |
| L23 | Shared fact-record extraction at second writer (D), not preemptively in A |
| L24 | Progressive chain-head / Control cursors are collector ops only — not report pin authority |
| L25 | Cross-network simultaneity is not required; per-network pins are correct |
| L26 | Series E adds bounds/prices as same authority model extensions — never dual pin paths |

---

# Closed open items (from earlier draft)

| Former open item | Closed decision |
|---|---|
| Keep `PortfolioSnapshot.errors` empty vs remove | **Remove field** (+ `PortfolioSnapshotError` + `error_count`) |
| Collapse `unacceptable_*` vs keep | **Collapse to `missing_fact`** |
| LWW claim_id secondary key | **Prefer `store_commit_order` only** |
| Exact home of shared hydrate helpers | Shared fact kit in A; prefer no new crate until forced; portfolio adapter consumes in C |
| Report `store_scope` constant vs config | Prefer certified default constant unless product forces config field |
| Series E bounds encoding / discovery provider | Deferred to E product work — not dual paths |

---

# Critical files

| File | Role |
|---|---|
| `crates/fact-capabilities/src/lib.rs` | Platform audience allowlist on `FactIndexReadRequest` |
| `crates/adapters/btc-jsonrpc/src/lib.rs` | Control query+hydrate+evidence pattern; shared kit consumer; BTC collector write later |
| Shared fact kit module (extracted in A) | Hydrate/response-ref helpers reused by portfolio report runner |
| `crates/states/btc/src/lib.rs` | `bitcoin.address_balance_snapshot` + later observe/record |
| `crates/states/evm/` (**new**) | `evm.address_native_balance_snapshot` + later observe/record |
| `crates/states/portfolio/src/lib.rs` | DELETE pin/observe/merge/soft errors; ADD `SelectHoldings` + pure select |
| `crates/ops/portfolio-tracker-op/src/lib.rs` | Topology cutover to select-centric report-only graph |
| `crates/adapters/portfolio/src/lib.rs` | DELETE transports; Platform fact-index `SelectHoldings` runner |
| `crates/portfolio/model/src/portfolio.rs` | Snapshot/report DTOs; DELETE soft error surface |
| `crates/portfolio/model/src/symbol.rs` | Valuation reader surface (FixedUnitPrice only at cutover) |
| `crates/ops/btc-collectors-op/` (**D packaging**) | BTC collector family monocrate |
| `crates/ops/evm-collectors-op/` (**D packaging**) | EVM collector family monocrate |
| `crates/app/src/*` | Registration cleanup / collector registration |
| `RFC_COLLECTORS_PORTFOLIO.md` | Normative design contract |
| `docs/code-quality.md` | No hacks / no dual shims |

---

## Anti-patterns (concrete)

| Anti-pattern | Correct action |
|---|---|
| Keep `ObserveBatch` nodes but read facts instead of RPC | Wrong — replace with `SelectHoldings` |
| N `QueryHoldingFacts` nodes + `MergeObservations` | Wrong — one select state + pure fn |
| Succeed with empty wallet + `errors: [{code: missing_fact}]` | Wrong — fail the run |
| Leave `errors: []` field for schema stability | Wrong — delete field (no BC) |
| `limit=1` order by height desc per holding then compare pins | Wrong — intersect full candidate anchors first |
| Register both live and fact runners "until tests migrate" | Wrong — zero dual registration window |
| Call collectors inside portfolio expand "for convenience" | Wrong — external multi-run only |
| Write fact with height only "provider didn't return hash" | Wrong — fail closed before write |
| Multi-address BTC collect without shared tip | Wrong — MUST share tip in-batch |
| Soft-mark `truncated` coverage then let report skip | Wrong — do not write; report never sees it |

---

## Delivery map (one PR)

```text
A  Platform FactIndexRead + shared fact kit
B  Holding schemas + states/evm + fixtures + pure normalize
C  Atomic public cut:
     C1 pure select
     C2 SelectHoldings state + adapter
     C3 topology flip + unregister live
     C4 delete dead types + soft errors
     C5 fixture tests + docs
D  Configured collectors + family op monocrates + shared tip + prove-before-write
E  Later: prices, at-or-before, discovery (same authority model)
```

**Merge tip requires A+B+C.** Prefer D before merge so the tip is operator-usable with RPC, not only fixtures. If D lags, document that explicitly; CI remains fixture-based.

When stuck: re-read Forbidden patterns and Locked decisions. Prefer **delete** over wrap. Prefer **one path** over dual.
