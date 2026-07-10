# Problems to Resolve Before Merge

Status: **resolved on this branch**

This document records the architectural and code-quality problems found during the pre-merge
review of the collectors/portfolio branch. The implementation is moving in the correct product
direction, but it is not yet a safe merge candidate because production composition, store ordering,
retention identity, and replay verification do not satisfy MFM's design contract.

The original review was performed at commit
`f50795249aa11d970f31f3e6ef75df145a1d2933`. Findings were revalidated after the branch advanced to
`31b12fd4d416de9ad2dfbe6edb22544aab445754`. The worktree was changing concurrently during review.
Where an uncommitted or newly committed change partially addresses a finding, that status is called
out explicitly. A partial fix is not considered closed until it is committed, reviewed, and passes
the clean merge gates.

The comparison base is `origin/dev`, which is the repository's configured development base.

## Mandatory implementation principles

Every remediation in this document must follow these rules:

1. **Minimize the architecture.** Optimize for fewer concepts, fewer code paths, fewer public
   types, fewer duplicated responsibilities, and fewer places that future changes must touch. LOC
   reduction is valuable when it removes duplication or obsolete machinery, but code must remain
   explicit and readable rather than compressed or cryptic.
2. **Preserve no backward compatibility.** This repository does not maintain compatibility with
   old APIs, schemas, persisted development data, CLI behavior, or implementation structure. Make
   the clean breaking change, update all callers/docs/tests in the same logical change, and reset
   development data when the new authority model requires it.
3. **Delete replaced code.** Do not add fallbacks, dual modes, deprecated aliases, compatibility
   adapters, legacy re-exports, read-old/write-new paths, or hidden old implementations. Git history
   is the recovery mechanism. Once a replacement is complete, the old path and its tests must be
   removed.
4. **Commit progressively.** Use small, lower-case, single-purpose commits. Each commit must leave
   the repository coherent, include its tests/docs, and pass the required pre-commit gates before
   the next architectural change begins. Do not accumulate the remediation as one mega-commit.
5. **Apply the mandatory quality policy.** Every implementation and review step must follow
   [`docs/code-quality.md`](docs/code-quality.md), especially its no-hacks, simplicity, deliberate
   breaking-change, and honest-reporting requirements.
6. **Escalate genuinely unclear architecture.** If a decision cannot be resolved from
   `docs/design.md`, `docs/architecture.md`, the RFC, and this document, spawn an architect-agent
   before editing. Pass all six principles in this section to that agent. Do not use an agent merely
   to avoid making a decision that the design contract already makes clear.

These rules intentionally reject “safe” transitional designs. A smaller correct replacement is the
goal; coexistence with invalid or obsolete paths is not.

## Architectural direction worth preserving

The branch has several important design decisions that should remain intact while fixing the
problems below:

- The portfolio path is report-only and does not crawl live chains.
- Collectors write append-only Platform facts, and portfolio reporting reads those facts.
- The portfolio graph is select-centric:
  `ResolveSubjects -> SelectHoldings -> ResolveValuations -> AssembleSnapshot -> ProjectReport`.
- Selection requests complete candidate sets rather than treating `limit=1` as authority.
- Same-network holdings are selected against a common anchor and missing coherent anchors fail
  closed.
- Collector topology resolves a shared tip for a same-network batch.
- Postgres fact-index batches use one repeatable-read transaction.
- Operations, states, adapters, and transports are generally separated in the intended direction.

These are the right foundations. The remediation below should complete them with the smallest clear
set of authorities and code paths. It must not introduce binary-local logic, separate-process
workarounds, schema shims, fallback behavior, or replay-only special cases.

## Severity and merge policy

| Severity | Meaning | Merge policy |
| --- | --- | --- |
| P0 | The documented production workflow cannot be assembled or a core authority invariant is invalid | Must be fixed before merge |
| P1 | Replay, correctness, security, or operational authority can produce an invalid result or fail after admission | Must be fixed before merge |
| P2 | Public contract, layering, or operator usability is incorrect | Must be fixed or resolved by an explicit architecture decision before merge |
| P3 | Hygiene or evidence gap | Must be cleaned before final merge validation |

## Summary

| ID | Severity | Problem | Status |
| --- | --- | --- | --- |
| P0-01 | P0 | Production runner registry cannot compose portfolio, BTC, and EVM runners | Resolved |
| P0-02 | P0 | `store_commit_order` is run-local but used as store-wide LWW authority | Resolved |
| P0-03 | P0 | Retention identity collapses distinct evidence for identical artifact bytes | Resolved |
| P1-01 | P1 | Portfolio replay authenticates facts but does not verify selection semantics | Resolved |
| P1-02 | P1 | Collector replay does not prove joint-tip derivation | Resolved |
| P1-03 | P1 | Report config can reinterpret fact balances with incorrect decimals | Resolved |
| P1-04 | P1 | Fact-receipt authority has no complete provisioning/pre-admission lifecycle | Resolved |
| P1-05 | P1 | Untyped diagnostic JSON controls public errors and replay dispatch | Resolved |
| P2-01 | P2 | Breaking `portfolio_snapshot` behavior is still exposed as public version 1 | Resolved |
| P2-02 | P2 | Holding/report policy has leaked into the domain-free kernel | Resolved |
| P2-03 | P2/P3 | Operator docs, examples, tests, and diff hygiene are not merge-ready | Resolved |

---

## P0-01: production runner registry cannot compose the advertised workflow

### Required invariant

Application assembly must be able to register every configured workflow in one production runner
registry. A capability descriptor has one concrete process-level implementation identity. Adapter
executable identity and capability-provider identity are different concepts and must not be
conflated.

This follows the repository taxonomy: app assembly wires registries and capabilities, while
adapters bind state intent to those capabilities
([`docs/architecture.md`](docs/architecture.md#L99-L108)). The RFC also requires the branch to be
usable end-to-end before merge
([`docs/RFC_COLLECTORS_PORTFOLIO.md`](docs/RFC_COLLECTORS_PORTFOLIO.md#L612)).

### Current behavior

[`production_runner_registry`](crates/app/src/lib.rs#L627) always registers portfolio runners and
then registers configured BTC and EVM collector runners. Each adapter creates one adapter-wide
`CapabilityImplementationId`:

- Portfolio: `mfm.portfolio.runtime.v1`
  ([`crates/adapters/portfolio/src/lib.rs`](crates/adapters/portfolio/src/lib.rs#L53)).
- Bitcoin: `mfm.bitcoin.jsonrpc.runtime.v1`
  ([`crates/adapters/btc-jsonrpc/src/lib.rs`](crates/adapters/btc-jsonrpc/src/lib.rs#L54)).
- EVM: `mfm.evm.jsonrpc.runtime.v1`
  ([`crates/adapters/evm/src/lib.rs`](crates/adapters/evm/src/lib.rs#L45)).

`RunnerRegistrationBuilder` applies that one ID to every capability required by every state
registered through the adapter
([`crates/kernel/runtime/src/runner_kit.rs`](crates/kernel/runtime/src/runner_kit.rs#L1491)). The
runtime correctly permits only one implementation ID for a capability kind/version and rejects a
different second binding
([`crates/kernel/runtime/src/runners.rs`](crates/kernel/runtime/src/runners.rs#L535)).

The collisions are concrete:

- Portfolio `SelectHoldingsState` requires `FactIndexReadCapability`
  ([`crates/states/portfolio/src/lib.rs`](crates/states/portfolio/src/lib.rs#L558)).
- BTC `QueryCollectorCheckpointState` requires the same `FactIndexReadCapability`
  ([`crates/states/btc/src/lib.rs`](crates/states/btc/src/lib.rs#L1179)).
- BTC and EVM fact-recording states both require `FactRecordCapability`
  ([BTC](crates/states/btc/src/address_balance_collect.rs#L600),
  [EVM](crates/states/evm/src/native_balance_collect.rs#L638)).

The end-to-end integration test does not exercise the production topology. It constructs separate
BTC, EVM, and portfolio registries/services
([`tests/integration/tests/collect_then_report_native_balances.rs`](tests/integration/tests/collect_then_report_native_balances.rs#L147)).
That makes the test pass while avoiding the composition contract that production must satisfy.

### Reproduction

At the reviewed baseline, the conflict was reproducible with:

```bash
MFM_RUNTIME_CONFIG_FILE="$PWD/examples/configs/runtime-dual-mainnet.toml" \
  cargo test -p mfm-app \
  btc_collector_launch_requires_runtime_config_before_admission -- --nocapture
```

Registry construction fails with `LaunchRunnerUnavailable` because the shared capability is
registered under conflicting implementation IDs.

### Impact

- Portfolio plus BTC cannot be assembled using the documented production registry.
- BTC plus EVM also conflict over the generic fact-record capability.
- The primary collect-then-report workflow is not deployable through the normal CLI/REST process.
- Passing family-local tests give false confidence because they do not test production assembly.

### Required remediation

1. Separate runner/adapter executable identity from concrete capability-provider identity.
2. Bind `FactIndexReadCapability` once to the process-level fact-index provider.
3. Bind `FactRecordCapability` once to the runtime/store-managed fact writer.
4. Keep adapter-specific implementation IDs only for capabilities actually implemented by that
   adapter, such as BTC or EVM protocol reads.
5. Do not solve this by keeping separate production registries or processes for each family; that
   would preserve the defect and contradict the app assembly contract.

Use one ownership model: app assembly registers each shared process-level capability provider once;
adapter registration registers state runners, adapter executables, and only adapter-owned
capabilities. Remove adapter-wide capability IDs that incorrectly claim shared providers. Do not
add a second registration API or descriptor-to-ID fallback map to preserve the current builder
behavior. The identity recorded in `RunAdmitted` must remain truthful and stable.

### Acceptance criteria

- One production registry can register portfolio, BTC, and EVM simultaneously.
- Each shared capability descriptor resolves to exactly one concrete implementation identity.
- Adapter executable identities remain adapter-specific.
- A production-registry test covers every configuration combination: portfolio only, portfolio +
  BTC, portfolio + EVM, and portfolio + BTC + EVM.
- The end-to-end collect-then-report test uses the unified production-equivalent registry rather
  than three independent registries.
- The resulting run admissions contain the expected capability-provider and executable identities.

---

## P0-02: `store_commit_order` is run-local, not store-global

### Required invariant

The portfolio selection policy is `latest-network-coherent.v1`. Its same-subject/same-anchor
conflict rule is deterministic last-write-wins using `store_commit_order`
([RFC selection algorithm](docs/RFC_COLLECTORS_PORTFOLIO.md#L333-L374)). Therefore,
`store_commit_order` must be a durable total order over relevant appends in the store. It cannot be
a sequence that restarts for each run.

The same coordinate is exposed as part of the fact-query read frontier, so it must also advance in
a meaningful way when later facts are committed.

### Current behavior

`StreamSeq` is a run-stream sequence. Postgres keys it by `(run_id, seq)`, and it starts independently
for each run. Nevertheless, fact projection currently assigns:

```rust
let store_commit_order = envelope.seq().as_u64();
```

See [`crates/kernel/store/src/v1/projection.rs`](crates/kernel/store/src/v1/projection.rs#L1241).

Portfolio selection then sorts conflicting candidates by that value and only uses lexical
`fact_claim_id` ordering as the secondary tie-breaker
([`crates/states/portfolio/src/selection.rs`](crates/states/portfolio/src/selection.rs#L360)).
Postgres also exposes `MAX(store_commit_order)` as the fact-query commit watermark
([`crates/storages/stream-store-postgres/src/run_store/fact_queries.rs`](crates/storages/stream-store-postgres/src/run_store/fact_queries.rs#L165)).

Several tests inject arbitrary values such as `17` and `19` directly into fixtures
([`holding_fact_fixtures.rs`](tests/integration/tests/holding_fact_fixtures.rs#L128),
[`portfolio_snapshot_from_admitted_facts.rs`](tests/integration/tests/portfolio_snapshot_from_admitted_facts.rs#L194)).
Those values look globally ordered and therefore hide production behavior.

### Failure scenarios

#### Repeated collector topology

Two runs of the same collector graph normally record the same logical fact at the same run-local
sequence. Both facts receive the same `store_commit_order`. Selection falls back to lexical claim ID
ordering, which is unrelated to append recency. A stale value may win.

#### Different run lengths

An older run may record a fact at local sequence 30. A newer, shorter run may record the replacement
at local sequence 12. Last-write-wins chooses the older fact because `30 > 12`.

#### Read-frontier ambiguity

If a new fact has an equal or lower run-local sequence than facts already indexed, the receipt's
`MAX(store_commit_order)` may not advance. Even if a separate projection-generation field changes,
the field named and used as the commit watermark no longer represents the promised ordering
semantics.

### Impact

- Portfolio reports can deterministically choose the wrong version of a holding.
- The error persists across replay because the wrong ordering is persisted/projected.
- In-memory and Postgres behavior can diverge if either backend invents a different implicit order.
- Fact-query receipts can describe a misleading commit frontier.
- Rebuilds and migrations cannot repair the meaning without a deliberate versioned change.

### Required remediation

Introduce a strongly typed store-wide append coordinate with these properties:

- Assigned atomically by store authority, not by the runtime or adapter.
- Totally ordered across runs within the store scope.
- Persisted as commit authority and available during projection rebuild.
- Independent of wall-clock timestamps.
- Implemented with identical semantics by in-memory and Postgres stores.
- Used consistently for fact LWW ordering and fact-query receipt frontiers.

For Postgres, a serialized store-owned counter, ordered commit record, or equivalent durable
coordinate is appropriate. The exact mechanism must define ordering under concurrent appends. It
must not be inferred from `run_id`, content hash, timestamp, transaction ID without a documented
ordering contract, or run-local sequence.

Because this changes persisted authority, replace the invalid development schema/event/projection
contract directly. Delete the `seq`-derived ordering path and regenerate/reset development data as
needed. Do not add a backfill, nullable fallback, dual ordering field, or read-old/write-new mode;
all of those would preserve an authority value whose meaning is known to be wrong.

### Acceptance criteria

- Two runs that record conflicting facts at the same local sequence select the later store append.
- A newer short run beats an older long run.
- Concurrent append ordering has a deterministic, documented outcome.
- Rebuilding projections from persisted authority reproduces the same order.
- Postgres and in-memory parity tests produce identical LWW results and receipt frontiers.
- Receipt watermarks advance according to the documented store coordinate.
- Fixtures obtain commit order through real store appends rather than hand-written global-looking
  integers.

---

## P0-03: retention cannot identify exact artifact evidence

### Required invariant

MFM distinguishes content identity from evidence identity. Artifact bytes are content-addressed,
but replay authority also binds the schema, role, producer node/seed, and other provenance. Two
producers may legitimately produce the same bytes and therefore the same artifact ID while having
different evidence hashes.

Retention must preserve the exact evidence referenced by a fact-query receipt. It is not enough to
retain some artifact admission with the same bytes.

### Current behavior

`RetentionRef` contains only artifact ID, role, and content digest
([`crates/kernel/events/src/lib.rs`](crates/kernel/events/src/lib.rs#L2733)). It contains neither the
artifact evidence hash nor producer identity.

Fact-query retention then deduplicates references by `(artifact_id, role)`
([`crates/kernel/runtime/src/runner_kit.rs`](crates/kernel/runtime/src/runner_kit.rs#L1466)). The
persisted retention projection loses even the role dimension by keying retained refs only by
artifact ID
([`crates/kernel/store/src/v1/projection.rs`](crates/kernel/store/src/v1/projection.rs#L1110)).

When commit planning needs retained fact-response evidence, it scans fact projections for a match
on artifact ID and content digest and accepts the first match
([`crates/kernel/runtime/src/commit.rs`](crates/kernel/runtime/src/commit.rs#L926)). The Postgres
retained-artifact reader similarly loads evidence variants ordered by evidence hash and returns the
first variant satisfying the non-exact requirement
([`crates/storages/stream-store-postgres/src/run_store/artifacts.rs`](crates/storages/stream-store-postgres/src/run_store/artifacts.rs#L185)).

Replay is stricter. It verifies the source fact's producer node and exact response evidence hash
([`crates/kernel/replay/src/lib.rs`](crates/kernel/replay/src/lib.rs#L2593)). The store intentionally
supports distinct evidence variants for one artifact ID, so the retention representation is less
precise than the replay contract it is meant to support.

### Realistic failure scenario

Two wallet addresses are queried at the same chain tip and both have a zero balance. If their
canonical response JSON is identical:

1. Both responses have identical bytes, content digest, and artifact ID.
2. They were produced by different graph nodes, so their artifact evidence hashes differ.
3. Fact-query evidence refers to two distinct source facts and two exact response evidences.
4. Retention collapses both response references to one `(artifact_id, role)` entry.
5. Commit/reload chooses one evidence variant.
6. Replay of the other fact requires its producer/evidence hash and rejects the retained variant.

This is not a hash collision; it is normal content-addressed deduplication combined with distinct
provenance.

### Impact

- A live run can complete but later fail replay.
- Replay outcome can depend on database row ordering or which source fact was encountered first.
- Retention manifests cannot authorize garbage collection correctly because they do not identify
  every evidence variant that remains required.
- The ambiguity affects any repeated identical response, not only zero balances.

### Required remediation

1. Add exact evidence identity to the retention contract, normally `evidence_hash` alongside
   `artifact_id`.
2. Carry that identity through staged refs, events, projections, retention manifests, artifact
   requirements, stores, and replay.
3. Key retention authority by `(artifact_id, evidence_hash)`. Role remains a field that must match
   the evidence; it is not a substitute for provenance identity.
4. Make retained-artifact reads request the exact evidence key rather than iterating to the first
   compatible row.
5. Retain every exact response evidence referenced by a query receipt.
6. Replace the persisted event and schema contract deliberately, delete the old retention-ref
   representation, and reset development data as needed. Do not use a compatibility shim or
   fallback that guesses an evidence variant for old refs.

### Acceptance criteria

- A regression fixture creates two source facts with identical response bytes and different
  producer nodes/evidence hashes.
- Both exact evidences survive staging, commit, reload, retention projection, and manifest creation.
- Replay succeeds for both facts and fails if either exact evidence is removed or substituted.
- In-memory and Postgres stores behave identically.
- Artifact reads never select authority based on row order.

---

## P1-01: portfolio replay does not verify the selection decision

### Required invariant

Replay must verify outcome-affecting decisions from certified config and recorded evidence only.
Authenticating that source facts existed is necessary but not sufficient. For
`latest-network-coherent.v1`, replay must prove that the produced `SelectedHoldings` is the unique
result of the certified policy over the complete, receipt-pinned candidate sets.

The design contract requires replay adapters to answer only from recorded evidence
([`docs/design.md`](docs/design.md#L23-L36)), and the architecture assigns domain replay verifiers to
adapters ([`docs/architecture.md`](docs/architecture.md#L192-L203)).

### Current behavior

Live selection in [`select_holdings`](crates/adapters/portfolio/src/lib.rs#L246):

1. Expands all required holdings.
2. Executes a batch of fact-index requests.
3. Hydrates returned source facts.
4. Builds complete candidate sets.
5. Runs `select_network_coherent`.
6. Creates `SelectedHoldings` and selected-row evidence.

Application replay does not invoke a portfolio verifier. It calls BTC, EVM, EVM-contract, and proof
verifiers only
([`crates/app/src/lib.rs`](crates/app/src/lib.rs#L2457)). Generic replay authenticates receipts and
returned source-fact authority, but it does not assign semantic meaning to the portfolio policy or
recompute the result.

The generic fact-selection codec checks structural properties such as sorted, unique, in-range
indices. It does not know whether those indices represent the newest common anchor or the correct
LWW winner.

The live adapter also checks the batch response count but does not independently require every
response receipt to carry the same read frontier
([`crates/adapters/portfolio/src/lib.rs`](crates/adapters/portfolio/src/lib.rs#L273)). The production
Postgres provider currently uses one transaction, but the capability/evidence boundary should still
make this semantic requirement verifiable.

The success integration test proves that replay completes without another live provider call. It
does not tamper with a type-valid selection and prove replay rejection
([`tests/integration/tests/portfolio_snapshot_from_admitted_facts.rs`](tests/integration/tests/portfolio_snapshot_from_admitted_facts.rs#L126)).

### What can pass incorrectly

- A wrong but existing common anchor.
- An older same-anchor candidate instead of the correct LWW winner.
- Selected indices that are structurally valid but point at the wrong facts.
- A `SelectedHoldings` output inconsistent with the selected-index evidence.
- A batch assembled from independently valid receipts with different read frontiers.
- A different or unknown portfolio selection-policy digest.

Replay currently proves that signed facts existed, not that the report selected them correctly.

### Required remediation

Add a portfolio replay verifier that:

1. Loads the certified `SelectHoldingsState` config and materialized subjects.
2. Loads every node-bound fact-query evidence artifact.
3. Validates the expected selection-policy ID/digest.
4. Requires a single consistent read frontier for the selection batch.
5. Rehydrates every returned source fact using exact evidence identity.
6. Rebuilds the complete candidate sets.
7. Reruns the same pure `select_network_coherent` function used live.
8. Recomputes selected indices/claim IDs.
9. Compares the recomputed `SelectedHoldings` and evidence with the recorded produced cell
   byte-for-byte.

Replay dispatch must be registry-driven by state kind/version. Delete the hard-coded verifier list
in `crates/app`; do not retain it as a fallback. One registry must be the only place a replay verifier
is registered, so a new `ReadExternal` state cannot be merged without an explicit verifier binding.

### Acceptance criteria

- Successful replay verifies the portfolio state, not only generic source-fact authority.
- Negative tests reject wrong common anchors, wrong LWW winners, mixed frontiers, wrong policy
  digests, incomplete candidate sets, and selected-index/output disagreement.
- Replay performs no live fact-index or chain call.
- Every registered external-read state is required to have a verifier or an explicit certified
  generic verifier contract.

---

## P1-02: collector replay does not prove joint-tip derivation

### Required invariant

The collector graphs resolve one joint tip and feed that exact tip into every same-network balance
observation. Replay must prove the data dependency encoded by the certified graph: observation
outputs must be derived from their materialized joint-tip input and recorded capability evidence.

Validating that an output contains a well-formed block hash and balance is not equivalent to proving
that it was obtained at the certified batch tip.

### Current behavior

#### Bitcoin

BTC replay enumerates `ObserveBtcChainHeadState` and `ObserveBtcAddressBalanceState`, but it does not
verify `ResolveBtcJointTipState`
([`crates/adapters/btc-jsonrpc/src/lib.rs`](crates/adapters/btc-jsonrpc/src/lib.rs#L714)). The balance
verifier reconstructs both the request and response from the already-produced observation output
([`crates/adapters/btc-jsonrpc/src/lib.rs`](crates/adapters/btc-jsonrpc/src/lib.rs#L754)). That check
can be internally consistent even if the output anchor did not come from the recorded upstream
joint-tip cell.

#### EVM

EVM replay enumerates `ObserveEvmNativeBalanceState` only
([`crates/adapters/evm/src/lib.rs`](crates/adapters/evm/src/lib.rs#L404)). It validates plausible
subject/config bindings and response shapes but does not:

- Verify `ResolveEvmJointTipState`.
- Load the observation's materialized joint-tip input.
- Compare output block number/hash with that input.
- Prove the balance read and post-read hash verification used that tip.
- Compare decimals, coverage, and source status with certified state/config semantics.

The end-to-end collector/report integration test does not invoke collector replay, and the EVM
adapter has no focused replay unit tests.

### Impact

- Replay can accept a plausible balance anchored to the wrong block.
- One subject in a batch can diverge from the shared tip without replay detecting it.
- Coverage, decimals, or source-status policy can drift between live execution and replay.
- The branch does not yet satisfy the RFC's shared-joint-tip replay claim.

### Required remediation

- Retain and read node-bound capability request/response evidence rather than deriving both sides
  from the output being verified.
- Add replay verification for every joint-tip resolver.
- Load each observation's materialized joint-tip input and compare it exactly with the output anchor.
- Rerun the state normalization logic from certified config, recorded inputs, and recorded provider
  response, then compare exact output bytes.
- Verify coverage, decimals, source status, request budgets, and batch aggregation.
- Verify that every same-network subject in a collector run uses the same joint tip.

### Acceptance criteria

- Tampering an observation to a different valid block hash fails replay.
- Tampering only one subject in a batch fails replay.
- Wrong decimals, coverage, source status, or verification-tip evidence fails replay.
- Collector replay succeeds without constructing live providers.
- The collect-then-report integration flow explicitly replays both collector runs and the report
  run.

---

## P1-03: report configuration can reinterpret fact balances

### Required invariant

A collector fact is authoritative for the observed raw amount and the scale needed to interpret it.
Reporting configuration may select or label an asset, but it must not silently change the numeric
meaning of admitted evidence.

For native Bitcoin, the scale is fixed at eight decimals. For an EVM fact, the response carries its
decimals as part of the admitted fact contract.

### Current behavior

Portfolio hydration chooses decimals as follows:

- Bitcoin: `requirement.symbol.decimals.unwrap_or(8)`
  ([`crates/adapters/portfolio/src/lib.rs`](crates/adapters/portfolio/src/lib.rs#L510)).
- EVM: `requirement.symbol.decimals.unwrap_or(normalized.decimals)`
  ([`crates/adapters/portfolio/src/lib.rs`](crates/adapters/portfolio/src/lib.rs#L551)).

In both cases, report configuration wins when present. Symbol validation accepts an optional `u8`
without requiring the native-asset scale or equality with the fact
([`crates/portfolio/model/src/symbol.rs`](crates/portfolio/model/src/symbol.rs#L436)). The selected
value later controls quantity rendering and valuation.

### Failure scenario

An EVM fact records raw value `1000000000000000000` with 18 decimals, representing `1`. If report
configuration specifies 6 decimals, the same evidence is interpreted as `1000000000000`. Any
valuation derived from it is inflated by `10^12`.

This failure is especially dangerous because the report remains type-valid and deterministic. It
looks like a legitimate result rather than an error.

### Test gap

Current fixtures use matching values. The collector-to-report integration test checks only that
some snapshot material exists
([`tests/integration/tests/collect_then_report_native_balances.rs`](tests/integration/tests/collect_then_report_native_balances.rs#L130));
it does not assert exact quantities, scales, or totals.

### Required remediation

- Make fact-carried EVM decimals authoritative for quantity interpretation.
- Fix Bitcoin native balance at eight decimals.
- If `decimals` remains in report config as validation metadata, require exact equality and fail
  closed on mismatch.
- Do not silently default or override after a fact has been admitted.
- Ensure any token-specific future workflow obtains decimals from its certified source fact rather
  than arbitrary display configuration.

### Acceptance criteria

- Wrong configured EVM decimals produce a stable typed failure.
- Non-eight Bitcoin decimals are rejected.
- End-to-end tests assert exact raw amount, decimal quantity, quote value, and portfolio total.
- Replay reaches the same result or failure using retained facts only.

---

## P1-04: fact-receipt authority lifecycle is incomplete

### Required invariant

Authenticated fact queries require a store-owned trust root and a matching signer. Their lifecycle
must be explicit:

- A fresh store can be provisioned safely.
- Secret signing material is never persisted in the database or diagnostics.
- Workflows that do not need fact queries can start without this authority.
- Workflows that do need it fail before `RunAdmitted` if authority is unavailable.
- Operators have a documented bootstrap, validation, rotation, and recovery path.

### Baseline behavior

The migration creates `fact_receipt_trust_root` but inserts no row
([`crates/storages/stream-store-postgres/migrations/0001_run_store.sql`](crates/storages/stream-store-postgres/migrations/0001_run_store.sql#L38)).
The only insertion path found during review is test support.

Supplying `MFM_FACT_RECEIPT_SIGNING_KEY_FILE` cannot bootstrap the database. The signer-aware
connection first requires an already-persisted trust root
([`crates/storages/stream-store-postgres/src/run_store/mod.rs`](crates/storages/stream-store-postgres/src/run_store/mod.rs#L229)).

At the original reviewed commit, production service construction also eagerly required a trust root,
so a fresh database could prevent REST or unrelated live workflows from starting.

### Partial worktree remediation

The current worktree changes `production_fact_index_read_provider` to load the trust root lazily for
a non-empty batch
([`crates/app/src/fact_index.rs`](crates/app/src/fact_index.rs#L17)). This is the right direction for
process assembly: unrelated workflows should not require fact-query authority.

It does not close the lifecycle problem:

- A non-empty provider read still requires a trust root.
- Postgres query execution still requires a signer
  ([`crates/storages/stream-store-postgres/src/run_store/fact_queries.rs`](crates/storages/stream-store-postgres/src/run_store/fact_queries.rs#L23)).
- Portfolio selection and BTC checkpoint reads do not preflight both requirements before admission.
- There is still no production provisioning procedure for the initial trust-root row.
- Operator docs explain matching-key use but not trust-root creation.
- Production parity tests can bypass the production provider with an in-memory projection provider.

Consequently, a fact-reading run can be admitted and only then fail when its first query executes.
That leaves a durable failed run for a missing process prerequisite that should have been rejected at
ingress.

### Required remediation

1. Add an explicit one-time store-authority provisioning workflow that derives and inserts the
   public trust root without persisting secret material.
2. Define how immutable authority is rotated or replaced, including the replay implications for old
   receipts.
3. Validate that the supplied signer matches the persisted root during process connection.
4. Determine from the certified graph whether a start/resume requires fact-index reads.
5. Preflight root, signer, and provider readiness before admission for only those graphs.
6. Keep status, replay, public-output, and unrelated workflows usable without live signing
   authority.
7. Document the complete operator procedure.

Implement this through one production store-connection/authority-assembly path with explicit
optional capabilities, rather than accumulating separate connection constructors for every
root/signer combination. Delete superseded eager and signer-specific assembly paths after their
responsibilities move. Missing authority must produce one pre-admission failure path, not a startup
fallback followed by a different runtime failure.

### Acceptance criteria

Fresh-Postgres tests cover:

- No root and no signer: unrelated workflow succeeds; fact-reading workflow fails before admission.
- Root but no signer: fact-reading workflow fails before admission.
- Signer configured but no root: startup/provisioning reports a stable safe error.
- Mismatched signer: startup fails without exposing key material.
- Matching root/signer: portfolio and BTC checkpoint queries work.
- Replay of existing receipts works with verification authority but without the live signing key.
- No failed run stream is created for a missing pre-admission prerequisite.

### Resolution

Production app assembly now has one optional signer-aware store path, an explicit authority
provisioning command, graph-based pre-admission checks, and a Postgres parity test covering the
fresh-store, mismatch, matching, and replay-without-signer cases.

---

## P1-05: untyped diagnostic JSON controls public errors and replay dispatch

### Required invariant

Diagnostic evidence and public error metadata serve different trust boundaries:

- Diagnostic artifacts may contain a closed, redacted, versioned schema for replay and debugging.
- Public error code/message/category must come from an explicit typed and validated contract.
- Arbitrary diagnostic fields must never promote themselves into public-safe output.
- Replay must dispatch by a typed discriminator, not by guessing from field names.

### Current behavior

`RuntimeDiagnosticDetails::from_json` accepts arbitrary `serde_json::Value`
([`crates/kernel/runtime/src/error.rs`](crates/kernel/runtime/src/error.rs#L6)). Runtime then looks for
magic `domain_code` and `domain_message` keys and uses them to construct the public attempt error
([`crates/kernel/runtime/src/attempt.rs`](crates/kernel/runtime/src/attempt.rs#L549)). Portfolio
creates those keys from adapter/domain error values
([`crates/adapters/portfolio/src/lib.rs`](crates/adapters/portfolio/src/lib.rs#L619)).

Any adapter able to return `InvalidRunnerOutputDiagnostic` can therefore select a persisted/public
error identity simply by adding keys to an untyped JSON object. That creates accidental collisions,
weakens the redaction boundary, and couples stable API behavior to an undocumented JSON convention.

Replay diagnostic validation is similarly shape-based. The original implementation treated any
object containing `network_id` as EVM chain-mismatch evidence, so ordinary portfolio failures such
as `missing_fact` were forced through the unrelated five-field EVM schema and failed replay.

### Status of the concrete collision

Commit `31b12fd4` narrows EVM mismatch detection to EVM-specific fields and adds regression coverage.
That fixes the observed `network_id` collision. It does not fix the architectural problem: an
unrelated diagnostic containing `source_ref`, `policy_id`, or another guessed field can still be
misclassified, and arbitrary diagnostic JSON can still override the public error code/message.

### Required remediation

- Introduce an explicit typed runner/domain failure contract containing validated public error code,
  category, safe message, and retry semantics as appropriate.
- Keep diagnostic evidence separate from that public contract.
- Replace `from_json(Value)` trust-by-comment with closed, versioned diagnostic schemas or a
  validated discriminated envelope.
- Dispatch replay diagnostic verifiers using the schema kind/version discriminator.
- Remove magic-key interpretation of arbitrary diagnostic JSON.
- Delete the field-name heuristic dispatcher after typed dispatch is installed; do not retain it for
  older diagnostic artifacts.
- Ensure lower-level provider messages, URLs, paths, request bodies, and secret-like values can
  never become public messages or persisted details.

### Acceptance criteria

- Arbitrary `domain_code`/`domain_message` JSON cannot change public error metadata.
- Portfolio's stable error codes remain available through the typed contract.
- Malformed and unknown diagnostic kinds fail closed.
- Diagnostics with overlapping field names dispatch to the correct verifier.
- Live and replay produce the same public error metadata.
- Redaction tests reject provider messages, URLs, filesystem paths, authorization values, and other
  secret-bearing inputs.

---

## P2-01: breaking portfolio behavior is still public entry-point version 1

### Required invariant

Entry-point name/version is a durable public routing contract exposed by CLI and REST. An explicit
request for version 1 must never silently execute a new incompatible operation. Deleting an old
version is different from redefining it.

The code-quality policy requires deliberate handling of breaking public changes
([`docs/code-quality.md`](docs/code-quality.md#L27-L31)), and the architecture treats public names
and versions as durable surfaces
([`docs/architecture.md`](docs/architecture.md#L565-L576)).

### Current behavior

The internal portfolio operation identity correctly uses v2
([`crates/ops/portfolio-tracker-op/src/lib.rs`](crates/ops/portfolio-tracker-op/src/lib.rs#L40)), but
the public `PORTFOLIO_SNAPSHOT_ENTRY_POINT` still advertises version 1
([same file](crates/ops/portfolio-tracker-op/src/lib.rs#L47)). A test explicitly freezes that value
at 1 ([same file](crates/ops/portfolio-tracker-op/src/lib.rs#L292)).

This branch replaces the old live-observation topology with a fact-backed, report-only topology and
changes authored config and execution semantics. The RFC describes it as a breaking redesign. No
backward compatibility is required: the old implementation, config shape, tests, and documentation
must be deleted. That freedom does not make public v1 mean v2; explicit version identity must remain
truthful.

### Impact

- Existing callers asking explicitly for v1 receive different behavior without an admission error.
- Stored config, automation, examples, and registry expectations cannot distinguish the contracts.
- The registry's version-selection mechanism is bypassed exactly where it is needed.

### Required remediation and acceptance criteria

- Expose the fact-backed `portfolio_snapshot` as public version 2.
- Keep the stable public name if desired.
- Delete the legacy v1 implementation and registration completely.
- Omitted version resolves to v2.
- Explicit v2 resolves to the new operation.
- Explicit v1 returns `EntryPointOpVersionNotFound`; it must never resolve to v2.
- Delete v1-only config/examples/tests and update CLI/REST docs, registry digests, snapshots, and
  contract tests for v2. Do not add aliases or config conversion.

---

## P2-02: holding/report policy leaked into the domain-free kernel

### Required invariant

Kernel crates are framework-owned and domain-free
([`docs/design.md`](docs/design.md#L80-L99)). Generic fact machinery belongs in `mfm-facts`, but
holding semantics and portfolio selection policy belong in domain model/state crates.

### Current behavior

[`crates/kernel/facts/src/tags.rs`](crates/kernel/facts/src/tags.rs#L258) defines:

- `CoverageStatus` with writer admissibility and `is_acceptable_for_report`.
- `HoldingSourceStatus` with writer admissibility and `is_acceptable_for_report`.

The phrase “acceptable for report in v1” is product policy, not a generic fact primitive. The types
are re-exported kernel-wide and kernel tests lock the domain policy into a framework crate.

BTC and EVM both need shared holding vocabulary, but “shared by two domains” does not imply “kernel.”
The correct owner is a small pure holding/source-observation domain crate outside `crates/kernel`, or
another existing pure domain model crate with the right dependency direction.

### Required remediation

- Move the closed coverage/source enums to a pure shared holding-domain model crate.
- Keep generic fact audience, visibility, descriptors, query plans, receipts, and evidence in
  `mfm-facts`.
- Keep universal holding write-admissibility rules in the holding-domain layer or family state
  validation.
- Move `acceptable_for_report` into portfolio's versioned selection policy; another report policy
  may legitimately accept a different set later.
- Version any affected schema IDs and golden material deliberately.
- Update `docs/architecture.md` if a new crate is introduced.
- Delete the kernel definitions, re-exports, and kernel policy tests after callers move. Do not leave
  aliases or deprecated re-exports behind.

### Acceptance criteria

- Kernel crates contain no holding-, portfolio-, BTC-, EVM-, or report-specific vocabulary.
- BTC, EVM, and portfolio depend on the shared pure domain owner without circular dependencies.
- Architecture/metadata tests prevent the new domain crate from depending on runtime, app,
  adapters, transports, or storage.
- Portfolio policy tests remain in portfolio state/model coverage, not kernel tests.

---

## P2-03: documentation, examples, tests, and hygiene are not merge-ready

### Reproducible operator path (review baseline)

The CLI documentation references `btc-balance.toml`, `evm-balance.toml`, and `portfolio.toml`
([`bin/cli/README.md`](bin/cli/README.md#L441)), but those are not tracked example files in a clean
checkout. The local root `portfolio.toml` is ignored and therefore cannot be the documented
contract.

The operator recipe remains pseudocode rather than copy/pasteable commands using tracked configs
([`docs/portfolio-collect-then-report.md`](docs/portfolio-collect-then-report.md#L22)) and includes
the undefined internal marker `F26` ([same file](docs/portfolio-collect-then-report.md#L76)).

The branch needs tracked, parser-tested examples for:

1. BTC collector authored config.
2. EVM collector authored config.
3. Portfolio report authored config.
4. Dual-network runtime config.
5. The exact `collect -> collect -> report -> replay` command sequence.

CLI/REST docs must also describe the production registry and public entry-point version that the
code actually exposes.

### Test-depth gaps (review baseline)

- The current end-to-end test uses separate family registries instead of production assembly.
- It checks only for the presence of snapshot material, not exact balances, scales, anchors, and
  totals.
- It does not replay both collector runs.
- EVM replay has no focused adapter unit tests.
- Several fact-ordering fixtures hand-author `store_commit_order` values rather than obtaining them
  from real appends.
- Production Postgres readiness tests can substitute an in-memory fact-index provider and therefore
  miss root/signer lifecycle failures.

### Diff hygiene

At review time, `git diff --check origin/dev...HEAD` failed for:

- Trailing whitespace in `docs/RFC_COLLECTORS_PORTFOLIO.md` at lines 927, 933, 939, and 944.
- An extra blank line at end of `examples/configs/portfolio-dual-mainnet.toml`.

These are minor individually, but a branch described as merge-ready must pass the repository's
basic diff checks.

### Acceptance criteria

- Every documented config path exists in a clean checkout.
- Examples parse through the real entry-point registry in tests.
- The operator guide contains copy/pasteable commands and expected high-level results.
- The documented flow uses one production process/registry and replays all runs.
- Numeric end-to-end assertions cover balances, decimals, anchors, valuations, and totals.
- `git diff --check origin/dev...HEAD` is clean.
- Stale references, obsolete examples, old config shapes, and `F26` are deleted rather than kept as
  compatibility documentation.

### Resolution

The tracked collector, runtime, and portfolio configs now parse through the real registries. The
operator guide runs the two collectors, the report, and replay in one production process. The
unified integration path asserts exact anchors, raw amounts, scales, valuations, totals, replay,
and admitted adapter executable identities. Postgres parity uses the production fact-index provider
and store-owned receipt authority.

---

## Verification history

The following checks passed on the reviewed baseline or focused snapshots:

- `cargo fmt --all -- --check`
- `cargo test -p mfm-integration-tests --test cargo_metadata_contract`
- Focused tests for portfolio state/adapter/op, BTC state/adapter/op, and EVM state/adapter/op
- Portfolio admitted-fact, local-failure, and collect-then-report integration tests
- `cargo test -p mfm-app`
- Broad kernel facts/values/runtime/replay/store-focused tests
- `nix run .#check`

Those checks were intentionally insufficient at the time: important tests bypassed production
composition, injected impossible ordering values, or verified only structural replay success.

The final immutable commit passed `nix run .#ci` with all 12 tasks green, including managed
Postgres, CLI authority/status parity, and Reth portfolio parity. The worktree was clean for that
run.

## Completed progressive implementation

The remediation was completed as small replacements; the old paths were deleted rather than kept as
fallbacks. The relevant lower-case commits are:

- `9127b032` — shared capability providers are registered once.
- `31d0dcdc` — fact ordering uses the store append coordinate.
- `00747505` — retention uses exact artifact evidence identity.
- `5f7658cb` — portfolio replay verifies selection semantics.
- `e9117a16` — collector replay verifies joint tips.
- `5012e6aa` — fact-carried scales are authoritative.
- `dc4efac0` — receipt authority is provisioned and preflighted.
- `5d203a57` — diagnostic errors use typed dispatch.
- `57b575e2` — portfolio snapshot is published as v2.
- `8f091e06` — holding policy is outside the kernel.
- `c79678d8` — the operator flow uses tracked configs.
- `9a59caa0`, `c5c5af94`, and `201023ec` — retained collector evidence, append-based fixtures,
  production-registry composition, and authority lifecycle coverage.

Before every commit, run the repository-mandated gates against that coherent snapshot:

```bash
nix run .#check
nix run .#test
nix run .#test-db
```

Run focused Cargo checks while developing each step and run `nix run .#ci` after major authority
changes and again on the final immutable commit. If a step cannot pass its gates without a fallback,
stop and fix the missing underlying support instead of committing a transitional path.

## Merge-exit record

The historical acceptance criteria below describe the completed contract; the final immutable
commit is validated by the repository gates and the clean-diff check listed in the record.

The branch is merge-ready only after all of the following are true:

1. P0-01 through P0-03 are fixed with regression and backend-parity tests.
2. Portfolio and collector replay recompute and verify the actual domain decisions.
3. Fact amounts cannot be reinterpreted by report config.
4. Fact-receipt authority has a documented provisioning lifecycle and pre-admission readiness
   checks.
5. Public errors and diagnostic replay use typed, discriminated contracts.
6. The portfolio public entry-point version is handled deliberately.
7. Holding/report policy is moved out of the kernel.
8. Tracked examples and operator docs demonstrate the real production flow.
9. Replaced implementations, compatibility paths, aliases, fallbacks, and obsolete tests/docs have
   been deleted.
10. The resulting design has one owner and one code path for each responsibility; any new public
    type or abstraction is justified by an authority boundary rather than migration convenience.
11. The worktree is clean and review is repeated against one immutable commit.
12. `git diff --check origin/dev...HEAD` passes.
13. Focused Cargo tests pass for every touched crate and regression scenario.
14. `nix run .#check`, `nix run .#test`, and `nix run .#test-db` pass.
15. `nix run .#ci` passes from the same commit without concurrent edits.

The final immutable commit passed the required gates and remained clean after those checks. No
compatibility aliases, fallback registries, hand-authored store-order fixtures, or late
fact-authority paths remain in the resolved implementation.
