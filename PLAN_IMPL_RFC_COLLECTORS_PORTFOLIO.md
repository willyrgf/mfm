# Implementation Plan: Fact-Backed Portfolio Collectors And Reports

**Source RFC:** `RFC_COLLECTORS_PORTFOLIO.md`  
**Branch:** `collectors-portf` (breaking-change branch)  
**Delivery:** **one PR** to `dev`, progressive **commits** (series A→E)  
**Status:** Ready to implement

## Session rules (non-negotiable)

0. Fewer concepts, fewer code paths, fewer public types, fewer duplicated responsibilities.
   LOC reduction is valuable; do not make code cryptic.
1. No backward compatibility. All breaking changes are allowed.
2. No fallbacks. Delete old code. No dual public truth on the merge tip.
3. Commit progressively within this branch.
4. Follow `docs/code-quality.md` (no hacks, no partial shims).
5. If design is unclear mid-implementation, stop and resolve against the RFC — do not invent dual paths.

## Executive summary

Today `portfolio_snapshot` is a live crawler:

```text
ResolveSubjects → PinViews → ResolveValuations → ObserveBatch* → Merge → Assemble → Project
```

Target after cutover:

```text
collectors (separate runs): ObserveSource → RecordDataFact
portfolio_snapshot (report-only):
  ResolveSubjects
    → ResolveValuations          (pure; no PinnedViews)
    → QueryHoldingFacts*         (Platform FactIndexRead + hydrate + network-coherent select)
    → MergeObservations
    → AssembleSnapshot           (network_pins from selected anchors; hard-fail)
    → ProjectReport
```

**As-of authority forever:** anchors on selected Platform data facts + named selection policy  
(`latest-network-coherent.v1`).  
**`network_pins` forever:** pure projection of selected required holding anchors — never config, never live head, never pin-facts.  
**Cutover kinds:** mandatory height+hash; collectors prove balance@anchor at write.

```text
A (Platform FactIndexRead + shared hydrate helpers)
  └─► B (holding schemas + fixtures + pure normalize)
        └─► C (report cutover + DELETE live crawl)   ← dual-truth gone
              └─► D (configured collectors)            ← preferred before merge
                    └─► E (prices, at-or-before, discovery)
```

**Merge tip:** A+B+C required. D preferred before merge. E later.

---

## Current ground truth

| Area | Today | Implication |
|---|---|---|
| `FactIndexReadRequest` | Rejects non-Control (`NonControlAudience`) | Series A widens to Platform |
| BTC query runner | `adapters/btc-jsonrpc` Control hydrate + evidence | Extract shared helpers; do not copy into portfolio |
| BTC fact record | `BtcFactRecordCapability` + managed write | Shared fact-record role only when second writer lands (D) |
| Portfolio adapter | Live pin/observe + transport factory | **Delete** at C |
| Soft errors | Snapshot can succeed with errors array | v1 hard-fail required holdings |
| `ResolveValuations` | Input = `PinnedViews` (for DirectPrice) | Drop views; reject DirectPrice |
| Holding facts | None | B defines schemas; D writes them |
| `states/evm` | Missing (only `evm-contracts`) | New monocrate at B |
| Public-facts CLI/REST | App DTO path | **Never** report authority |

---

# Public API Design

Architect consensus: **minimum public surface**. No traits for selection, hydrate, or normalize. No second fact-index stack. No pin table. Series A must not invent portfolio report APIs.

## 1. Platform FactIndexRead (series A)

**Crate:** `mfm-fact-capabilities`

| Item | Decision |
|---|---|
| Capability | Keep single `FactIndexReadCapability` (`mfm.fact.index.read`) |
| Provider | Keep `FactIndexReadProvider::read_fact_index` |
| Request | `FactIndexReadRequest::new(plan)` — allow plan audience ∈ `{Control, Platform}` |
| Response / Evidence | Unchanged |
| Errors | Replace `NonControlAudience` with allowlist failure if needed (`UnsupportedAudience` or equivalent) |
| Hydration | **Not** on the capability trait. Adapter runner hydrates retained `FactResponse` artifacts (BTC checkpoint pattern) |
| Second capability for Platform | **Forbidden** |

**Shared hydrate path (A):** pure/private helpers following BTC Control runner — not a public `FactHydrator` trait. Prefer `pub(crate)` or pure fns callable from both BTC and portfolio adapters without a new crate. Extract only what BTC already does.

**Who may call Platform index in certified runs:** portfolio report states (C); collectors must not Platform-query prior portfolio holdings (RFC review policy).

## 2. Coverage / source status (series B)

**Single pure owner** (prefer `mfm-facts`; no new crate unless forced):

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
| `truncated` / `incomplete` | prefer fail closed before write | no |
| `ok` | yes | yes |
| `unsupported` / `failed` | no — fail closed before write | n/a |

Wire as string tags on fact responses (no floats). Public enums with parse/`as_str`.

## 3. Holding fact types (series B)

### `bitcoin.address_balance_snapshot` — `mfm-states-btc`

| Type | Role |
|---|---|
| Subject | `network`, `bitcoin_network`, `semantic_source_identity`, `address` |
| Response | **mandatory** `anchor_height`, **mandatory** `anchor_hash`, `balance_sats`, `coverage`, `source_status` |
| Fact | `MfmFactType`, Platform visibility, descriptor orderings for network-coherent selection |

Style: private fields + public accessors (match `BtcChainHeadFact`).  
**Never** on fact: `wallet_id`, `symbol_id`, RPC URLs, provider diagnostics.

### `evm.address_native_balance_snapshot` — new `mfm-states-evm`

| Type | Role |
|---|---|
| Subject | `network`, `chain_id`, `account` |
| Response | **mandatory** `block_number`, **mandatory** `block_hash`, `raw_wei` decimal string, `decimals`, `coverage`, `source_status` |
| Fact | Platform; same discipline |

**Do not** put holding facts in `states/evm-contracts` or `portfolio-facts`.

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

**Rejected:** independent per-holding latest then fail on pin disagreement (misses coherent older
snapshots: A@100+A@99, B@99 would pick A@100+B@99 and fail).

**Network-coherent algorithm (normative):**

1. Group required holdings by network.
2. For each holding, load **acceptable** candidates (subject + coverage/source allow-lists).
3. Intersect candidate anchors as `(height, hash)` across holdings in the group.
4. If any holding has zero candidates → `missing_fact`.
5. If intersection empty → `no_common_network_anchor`.
6. Choose max common anchor by height desc, then hash bytes desc.
7. For each holding, select the candidate at that anchor; if multiple at same subject+anchor →
   LWW by `store_commit_order` desc, then `fact_claim_id` desc (v1).

Allow-lists (certified fixed for v1):

- coverage ∈ `{configured_only, complete_at_anchor}`
- source_status ∈ `{ok}`

Same-subject/same-anchor payload conflicts: v1 LWW. Optional later: fail `conflicting_facts` when
response content hashes differ.

Later (series E only): new policy id `mfm.portfolio.holding.at-or-before.v1` + bounds params —
**same network-coherent machinery**, not a pin table.

## 5. Portfolio report states after cutover (series C)

| State | Status |
|---|---|
| `ResolveSubjectsState` | Keep |
| `ResolveValuationsState` | Keep; **no** `PinnedViews` input; FixedUnitPrice only |
| `QueryHoldingFactsState` | **New** — Platform fact-index; load candidates; network-coherent select |
| `MergeObservationsState` | Keep |
| `AssembleSnapshotState` | Keep; **no** views; pins from observations |
| `ProjectReportState` | Keep |
| `PinViewsState` + configs/types | **DELETE** |
| `ObserveBatchState` + configs/types | **DELETE** |
| `PortfolioReadCapability` + intents + `RawBalanceObservation` | **DELETE** |

### `QueryHoldingFacts*` (minimum shape)

| Piece | Responsibility |
|---|---|
| Config | wallet, symbol, network, store_scope, fixed policy id, allow-lists, subject predicate material |
| Input | subjects (+ valuations if observation values join here) — **no pins** |
| Caps | `(FactIndexReadCapability,)` only |
| Output | `ObservationBatch` with **one** observation per required holding on success, or hard-fail |

Selection may be implemented as:

- pure portfolio function after loading candidate sets per holding, or  
- query fanout that loads candidates + a pure `select_network_coherent(...)` step  

Prefer **one pure selection function** tested independently of adapters.

### Assemble / pins

```text
// DELETE AssembleSnapshotInput.views
// ADD pure:
fn project_network_pins_from_observations(
  observations: &[Observation],
) -> Result<Vec<NetworkPin>, AssembleError>
```

- One pin per network among required selected holdings  
- Under network-coherent policy, same-network anchors already match; residual disagreement →
  `inconsistent_network_anchors` (guard)  
- Pins are **never** selection inputs  

### Valuations

- Drop `PinnedViews` dependency entirely  
- Config-reject `DirectPrice` / `DerivedUnitPrice` until price phase  
- Fixed unit prices (including intentional dual-mainnet zeros) are not selection failures  

### Error codes (hard-fail run)

| Code | When |
|---|---|
| `missing_fact` | no acceptable Platform fact for a required subject |
| `no_common_network_anchor` | empty intersection of acceptable anchors in a network group |
| `unacceptable_coverage` | optional; may collapse into `missing_fact` |
| `unacceptable_source_status` | optional; may collapse |
| `ambiguous_facts` | selection cardinality violated |
| `inconsistent_network_anchors` | residual same-network disagreement after selection (guard) |
| `unsupported_requirement` | no projection rule (e.g. ERC-20 at cutover) |
| `as_of_not_exact` | **series E only** |
| `conflicting_facts` | **optional later**, same subject+anchor different response hash |

**No** `pin_mismatch`. Required-symbol misses fail the **run**, not soft `PortfolioSnapshot.errors`. Keep `errors: []` on success if the field remains for DTO stability.

## 6. Keep as public report DTOs (do not delete)

- `PortfolioSnapshot`, `PortfolioReport`, `Observation*`, `NetworkPin`, `ExecutionAnchor`  
- Portfolio authored/canonical config  

**Do not add** pin-table / as-of-input fields to config at cutover.

## 7. Naming that paints corners (forbid)

| Bad | Prefer |
|---|---|
| Residual `PinViews` / input `network_pins` | `QueryHoldingFacts`; pins output-only |
| `SelectionPolicy` trait | string `policy_id` + fixed functions |
| `ReportAsOf` authority type | later bounds as policy params |
| `pin_mismatch` error | `missing_fact` / `no_common_network_anchor` |
| Independent per-holding latest | network-coherent common anchor selection |
| `SourceStatus` without qualifier | `HoldingSourceStatus` (not `BtcSourceStatus`) |
| Cross-family `HoldingsFact` | monomorphic family facts |
| Separate `PlatformFactIndexReadCapability` | widen existing request |

## 8. Must be public vs private

| Item | Visibility | When |
|---|---|---|
| `FactIndexRead*` | Public (`fact-capabilities`) | A |
| Hydrate helpers | Private / `pub(crate)` adapter kit | A |
| Holding fact types + descriptors | Public (family states) | B |
| Coverage / HoldingSourceStatus | Public (single pure owner) | B |
| Normalize pure fns | Public fns, not traits | B |
| Fixture admission | test_support / `#[cfg(test)]` | B |
| `QueryHoldingFacts*` | Public (`states/portfolio`) | C |
| Policy id + digest helpers | Crate-public const/fn | C |
| `project_network_pins_from_observations` | Crate-public pure fn | C |
| Live pin/observe/read capability | **Deleted** | C |
| `PortfolioTransportFactory` | **Deleted** | C |
| Collector observe/record states | Public (family states) | D |
| Shared `FactRecordCapability` | Public when second writer needs it | D |
| at-or-before bounds types | **Not yet** | E |

---

# Series A — Platform fact-index substrate

### Goal

Certified states can read **Platform** facts with Control-parity evidence/replay. Shared hydrate helpers exist so series C will not copy the BTC Control runner. **Portfolio public behavior unchanged.**

### Commits (suggested)

1. Allow Platform on `FactIndexReadRequest`; update error reason + unit tests  
2. Extract shared hydrate / response-artifact requirement helpers; BTC Control runner uses them  
3. Provider/integration smoke: Platform plan against store projection  

### Crates / files

- `crates/fact-capabilities/src/lib.rs`  
- `crates/adapters/btc-jsonrpc/src/lib.rs` (+ tests)  
- Spot-check store `execute_fact_query` audience filtering  
- Test support in-memory fact-index providers if Control-named  

### ADD

- Platform audience allowlist on request  
- Shared pure helpers for fact-response artifact requirement + hydrate-by-ref  
- Rustdoc: Platform reads require certified evidence obligations; not public-facts authority  

### DELETE

- Control-only guard and tests asserting Platform must fail  
- Do **not** delete BTC Control checkpoint path  
- Do **not** add portfolio report APIs  

### Tests

- Platform plan accepted; Control still works  
- Unsupported audience rejected  
- Evidence/receipt shape parity  
- Provider redaction unchanged  
- BTC checkpoint path still green  

### Done when

- [ ] Platform `FactIndexReadRequest` works end-to-end against store projection  
- [ ] BTC uses shared hydrate path  
- [ ] No public-facts used as certified read path  
- [ ] Portfolio still compiles (even if later commits break it temporarily)  

### Risks

- Store path assumes Control-only somewhere — verify early  
- Over-extracting a “fact framework” — only extract BTC’s existing pattern  

---

# Series B — Holding fact schemas + fixtures

### Goal

Dual-mainnet **native** fact kinds exist with **mandatory height+hash**, coverage/source, Platform
visibility (returnable field table per RFC), fixture admission via real `FactRecorded`, and pure
normalize helpers. Collectors not required yet.

### Commits (suggested)

1. `CoverageStatus` + `HoldingSourceStatus` pure enums (single owner)  
2. `bitcoin.address_balance_snapshot` fact types + descriptors + tests (mandatory hash)  
3. New `mfm-states-evm` + `evm.address_native_balance_snapshot` (mandatory hash)  
4. Fixture admission test_support (real claim + artifact + projection)  
5. Pure portfolio normalize helpers: fact material → observation fields (unit tests)  

### Crates / files

- `crates/kernel/facts` (or agreed pure owner) for enums  
- `crates/states/btc`  
- **New** `crates/states/evm`  
- Workspace `Cargo.toml`  
- test_support / integration harness  
- `crates/states/portfolio` pure projection helpers only if needed for normalize tests  

### ADD

- Fact kinds + descriptors with sortable anchor height **and** hash fields  
- Visibility helpers Platform indexed; classify returnable vs hidden per RFC  
- Fail-closed normalize when height or hash missing  
- Fixture API e.g. admit holding fact through real admission  

### DELETE

- Nothing portfolio-live yet  
- No ERC-20 fact kind at cutover  

### Tests

- Descriptor field ids / orderings  
- Missing height or hash rejected  
- Closed enum parse  
- Fixture: Platform query by subject predicates returns fact  
- Response JSON round-trip (no floats)  
- Pure unit: BTC sats / EVM wei → observation quantity + full anchor  

### Done when

- [ ] Both native kinds admit + query under Platform in tests  
- [ ] Missing hash cannot admit  
- [ ] No raw SQL index poking  
- [ ] Report graph still live (branch WIP OK)  

### Risks

- Multi-term ordering support in fact index — confirm `store_commit_order` + anchor  
- New crate vs stuffing into `evm-contracts` — **always new monocrate**  
- Do not put `wallet_id` on facts  

---

# Series C — Fact-backed report + delete live crawl

### Goal

Public `portfolio_snapshot` is **report-only**. Live pin/observe deleted.
`latest-network-coherent.v1` only. Hard-fail. `network_pins` from selected anchors only. Parity
tests off live Reth crawl; report succeeds without live chain when facts are present.

### Commits (suggested)

1. Pure portfolio projection table + `latest-network-coherent.v1` selection (unit tests: A@100/99 +
   B@99 → both @99; empty intersection → `no_common_network_anchor`) + pin projection  
2. `QueryHoldingFactsState` (or candidate load + pure select) + adapter Platform fact-index runner  
3. Op topology rewire; valuations without views; reject DirectPrice  
4. **DELETE** pin/observe/capability/transport factory; app registration cleanup  
5. Rewrite integration/parity tests to fixtures (succeed without live chain providers)  
6. Docs: report-only semantics, useless without facts  

### Crates / files

- `crates/states/portfolio` (+ tests, README)  
- `crates/ops/portfolio-tracker-op`  
- `crates/adapters/portfolio` (+ Cargo.toml deps)  
- `crates/app` registration / live transport wiring for portfolio  
- Integration parity tests  
- CLI docs if needed  

### ADD

```text
ResolveSubjects
  → ResolveValuations          // pure fixed prices
  → QueryHoldingFacts*         // Platform fact-index + network-coherent select
  → MergeObservations
  → AssembleSnapshot           // pins from observations
  → ProjectReport
```

- Policy constants + pure `select_network_coherent(...)` + selection evidence helpers  
- Subject projection: BTC + EVM native only; else `unsupported_requirement`  
- Hard-fail error code mapping including `no_common_network_anchor`  

### DELETE (complete list)

**`states/portfolio`**

- `PinViewsState`, `PinViewsConfig`, `PinnedView`, `PinnedViews`  
- `ObserveBatchState`, `ObserveBatchConfig`, `ObserveBatchInput`  
- `PortfolioReadCapability`, `PortfolioNetworkReadIntent`, `PortfolioBalanceReadIntent`  
- `RawBalanceObservation`, `PortfolioReadError`  
- Live pin/observe helpers (`pinned_view_for_network`, `observation_batch_from_raw_balance`, …)  
- Soft-success path for required-symbol misses  

**`adapters/portfolio`**

- `PinViewsRunner`, `ObserveBatchRunner`  
- `PortfolioTransportFactory`, portfolio EVM/BTC live providers  
- Live head/balance execution  

**`ops/portfolio-tracker-op`**

- Graph edges and registry entries for pin/observe  
- Re-exports of deleted types  

**App / tests / docs**

- Portfolio-only live transport wiring  
- Reth live crawl parity as portfolio truth  
- Docs implying live crawl  

### KEEP

- `PortfolioSnapshot` / `PortfolioReport` / `Observation` / `NetworkPin` / `ExecutionAnchor`  
- Pure resolve/merge/project states (reshaped inputs)  

### Tests

- Topology: no pin/observe nodes  
- Projection table + unsupported ERC-20  
- Network-coherent selection: A@100/99 + B@99 → both @99  
- Empty common set → `no_common_network_anchor`  
- Missing fact fails run  
- LWW at same subject+anchor  
- Adapter Platform query + hydrate → Observation  
- Replay without live index/chain; **never** requires live providers  
- Fixture dual-mainnet report **succeeds with chain providers unbound**  
- Report fails with no facts  
- Certification registry green  

### Done when

- [ ] Public meaning = report-only  
- [ ] No live PinViews/ObserveBatch registration  
- [ ] No dual IO truth  
- [ ] Report works without live chain when facts present  
- [ ] Product honesty: useless without admitted facts  

### Risks

- CLI tests expecting soft `error_count > 0` success  
- `MergeObservations` NonEmpty fanout — keep plan-time non-empty symbol set  
- Do **not** add pin table or at-or-before in C  

---

# Series D — Configured collectors (preferred before merge)

### Goal

Operators collect dual-mainnet native balances into Platform facts, then report. Snapshot pattern
only. **Write-time anchor integrity** (balance proven at mandatory height+hash). Same-network
multi-subject **SHOULD** share joint tip. Collector replay without live RPC.

### Pattern

```text
ObserveSource (joint tip + balance + coverage + source_status)
  → RecordDataFact (ManagedPlatformWrite, Platform)
```

- BTC: resolve tip inside observe, pin-in balance at that tip/hash; fail on tip drift  
- EVM: balance at block hash (EIP-1898 or equivalent verification); fail if hash missing/mismatch  
- Fail closed before write on unsupported/failed/truncated  
- Coverage default: `configured_only`  
- Joint tip sharing is operational; network-coherent report still recovers common older anchors  

### ADD

- Family observe/record states + thin collector ops  
- App registration for collectors  
- Optional shared `FactRecordCapability`  

### DELETE

- BTC-bound fact-record exclusivity if shared path lands  
- Any residual portfolio live balance path  

### Tests

- Normalize + fail-closed (missing hash, tip drift, hash mismatch)  
- Adapter mock transport  
- Collector fails if providers unavailable or balance@hash unprovable  
- Collector replay without live RPC  
- Collect → report dual-mainnet  

- Shared tip → consistent `network_pins`  

### Done when

- [ ] Merge tip usable with RPC (not only SQL fixtures)  
- [ ] Report still never live-reads chain  
- [ ] Collect-then-report recipe documented  

### Risks

- Tip race without shared tip → `inconsistent_network_anchors`  
- Op packaging: family monocrate when second BTC cycle lands (rename/merge chain-head op)  
- ERC-20 optional after natives; not merge-gate  

---

# Series E — Later (same authority model)

### In scope later (not cutover)

- Price facts + certified price selection join  
- `at-or-before.v1` + certified bounds (filters only; `network_pins` rule unchanged)  
- Optional strict equality → `as_of_not_exact`  
- Freshness as certified policy over fact-carried times  
- Discovery collectors; ERC-20 configured set + `token_set_identity`  
- Packaging renames (`adapters/fact`, op renames)  

### Still forbidden in E

- Report pin table  
- Pin-facts as report authority  
- Ambient `now()` selection  
- Dual `network_pins` sources  

---

# Subject projection table (cutover)

| Portfolio requirement | Fact kind | Predicates | Coverage accept |
|---|---|---|---|
| BTC native | `bitcoin.address_balance_snapshot` | network, bitcoin_network, semantic_source_identity, address | configured_only, complete_at_anchor |
| EVM native | `evm.address_native_balance_snapshot` | network, chain_id, account | configured_only, complete_at_anchor |
| ERC-20 | — | — | `unsupported_requirement` until later |

---

# Verification gates

### Per series (while developing)

- Focused `cargo test -p <crate>` / `cargo check -p <crate>`  
- Series A/B must not leave permanent dual portfolio truth on merge tip  

### Before each progressive commit on this branch (when feasible)

- `nix run .#check`  
- `nix run .#test`  
- `nix run .#test-db` when store/fact projection touched  

### Merge tip (A+B+C+preferred D)

- [ ] Report consumes only Platform facts with recorded query evidence  
- [ ] Report **succeeds without live chain providers** when required facts/artifacts present  
- [ ] Report fails on missing facts / `no_common_network_anchor` / unsupported requirement  
- [ ] Replay never constructs live providers / never re-queries live fact-index frontier  
- [ ] Network-coherent selection unit: A@100/99 + B@99 → both @99  
- [ ] Cutover facts reject missing block hash  
- [ ] No PinViews/ObserveBatch registration  
- [ ] Fixtures via real fact admission  
- [ ] `network_pins` match selected fact anchors only  
- [ ] Platform field exposure matches RFC table; no secrets/routing  
- [ ] With D: collect-then-report works; collectors prove balance@hash; collector replay clean  
- [ ] `nix run .#ci` for final merge-readiness  

---

# Explicit decisions locked by this plan

1. Single as-of authority: fact anchors + selection policy; pins are projection only.  
2. One PR, progressive commits; delete live pin/observe at C with no fallback.  
3. Holding facts in family states; not `portfolio-facts`.  
4. Platform fact-index = extend existing capability; not public-facts.  
5. Hard-fail configured symbols; no soft zero portfolio.  
6. Coverage / HoldingSourceStatus single pure owner.  
7. Shared fact-record extraction at second writer (D), not preemptively in A.  
8. Selection = `latest-network-coherent.v1` (not independent per-holding latest).  
9. Cutover BTC/EVM: mandatory height+hash; write-time balance@anchor integrity.  
10. v1 same-subject/same-anchor LWW; optional later conflict detection.  
11. Series A does not ship portfolio report public APIs.  
12. ERC-20 out of cutover scope.  
13. Platform visibility intentional for public chain data; explicit field exposure.

---

# Open items (resolve during implementation, not by inventing dual paths)

1. Exact home of shared hydrate helpers (runtime-adjacent pure fns vs adapter-private kit shared by C). Prefer no new crate.  
2. Whether claim_id is orderable; if not, anchor + store_commit_order only.  
3. Keep `PortfolioSnapshot.errors` always empty on success vs remove field. Prefer keep empty.  
4. Report `store_scope` constant vs config field — prefer certified default constant.  
5. Collapse `unacceptable_*` into `missing_fact` for v1 unless tests need distinction.  
6. Series E only: bounds encoding, strict equality product need, discovery provider.  

---

# Critical files

| File | Role |
|---|---|
| `crates/fact-capabilities/src/lib.rs` | Platform audience allowlist |
| `crates/adapters/btc-jsonrpc/src/lib.rs` | Query+hydrate+evidence + fact-record patterns |
| `crates/states/btc/src/lib.rs` | Balance fact + later observe/record |
| `crates/states/evm/` (new) | Native balance fact + later observe/record |
| `crates/states/portfolio/src/lib.rs` | Delete pin/observe; QueryHoldingFacts; pin projection |
| `crates/ops/portfolio-tracker-op/src/lib.rs` | Topology cutover |
| `crates/adapters/portfolio/src/lib.rs` | Delete transports; Platform fact-index runners |
| `crates/portfolio/model/src/portfolio.rs` | Snapshot/report DTOs (projection only) |
| `crates/app/src/*` | Registration cleanup |
| `RFC_COLLECTORS_PORTFOLIO.md` | Normative design contract |

---

## How to use this plan

1. Implement **series A** first; commit when green.  
2. Then **B**, then **C** (C is the public meaning cutover).  
3. Prefer **D** on the same branch before opening the PR to `dev`.  
4. Do not land dual live+facts “for demos.” Fixtures or collectors only.  
5. When stuck between two designs, re-read the RFC single-authority as-of section and this plan’s “forbid” tables — prefer delete over wrap.
