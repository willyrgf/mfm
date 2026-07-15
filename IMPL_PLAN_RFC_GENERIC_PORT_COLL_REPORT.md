# Implementation plan: generic portfolio collection and reporting

Status: implementation-ready

Date: 2026-07-15

Planning baseline: 47103d77278a30a9edefa48f021a0211b8f445b4

Source of truth: RFC_GENERIC_PORT_COLL_REPORT.md

## Purpose and authority

This document turns RFC_GENERIC_PORT_COLL_REPORT.md into an execution plan for engineer-agents. It
does not reopen the RFC's product or architecture decisions. The RFC defines the required behavior;
this plan defines dependency order, commit boundaries, deletion obligations, proof, and mandatory
architecture checkpoints.

The implementation is one destructive cutover developed through buildable commits. Intermediate
commits are review points on the implementation branch, not supported product versions. Do not add
temporary compatibility code to make an intermediate commit look releasable.

When documents disagree, use this precedence:

1. AGENTS.md and docs/code-quality.md for repository workflow and quality rules.
2. docs/design.md for platform invariants.
3. RFC_GENERIC_PORT_COLL_REPORT.md for the accepted product and architecture contract.
4. This plan for implementation order and evidence.
5. Existing code only as implementation evidence, never as authority for retaining an old path.

RFC_GENERIC_PORT_COLL_REPORT.md deliberately supersedes the affected public-operation,
portfolio-composition, collector-ingress, and contract-operation clauses of RFC_CONFIG.md. Update
RFC_CONFIG.md during the cutover so it no longer presents superseded behavior as current.

## Objective

Replace the current public-operation topology with one high-level portfolio objective:

~~~text
mfm.portfolio/snapshot@1
  -> resolve one PortfolioConfig at admission
  -> derive exact holding demand
  -> collect every demanded native and ERC-20 source at one shared anchor per network
  -> assemble an exact content-bound collection receipt
  -> select only the facts named by that receipt
  -> value and project one complete snapshot and direct quote-total report
~~~

At the same time:

- remove all eight current public entry points;
- remove standalone collector and report launch paths;
- add complete anchored ERC-20 collection and replay;
- replace count readiness with exact set and fact-content proof;
- remove role/protocol/Aave modeling that has no executable product path;
- delete the contract lifecycle operation and continuation/import paths;
- retain and finish deploy, configure, and validate as reusable contract states;
- reduce packages, public types, dependency edges, code paths, and future edit sites.

## Non-negotiable execution rules

These rules apply to every phase, commit, review, and architecture escalation.

### 0. Optimize for deletion and one authority

Optimize for fewer concepts, fewer code paths, fewer public types, fewer duplicated
responsibilities, and fewer places future changes must touch. LOC reduction is valuable.

Operational consequences:

- Prefer deleting or folding a wrapper over renaming it.
- Prefer one direct closed type over parallel classification and reader types.
- Prefer one exact manifest or receipt over counts plus relational cross-checks.
- Prefer private or crate-private helper types. Make a type public only when a real cross-crate
  contract requires it.
- Do not add a generic framework, registry, dynamic collector, universal receipt, or universal
  observation merely because multiple closed cases exist.
- Do not move planning into the app or binaries to reduce crate count.
- At every architect gate, report package, dependency, Rust LOC, public-declaration, and residue
  deltas. Any increase must be tied to a necessary invariant or the complete ERC-20 capability.

### 1. Maintain no backward compatibility

All breaking changes are allowed. Do not preserve old request shapes, setup values, schemas,
descriptors, output fields, replay formats, entry-point IDs, or persisted branch runs.

Previously certified runs using deleted descriptors intentionally become non-resumable and
non-replayable. Recreate experimental branch data and republish catalog values. Do not add a
migration or decoder for the removed contracts.

### 2. Delete old code when its replacement lands

There are no fallbacks, aliases, deprecated paths, hidden modules, facades, dual schemas, temporary
public operations, or compatibility replay branches. Git history is the recovery mechanism.

Each commit that installs a replacement deletes the superseded implementation, tests, fixtures,
docs, dependencies, and exports in that same commit. There is no final cleanup phase for old code.
If a proposed commit requires an adapter whose only purpose is to keep an old shape compiling,
expand or reorder the commit instead.

### 3. Divide work into coherent commits

Each commit below has one architectural outcome and a lower-case subject. Preserve the stated
boundaries unless an architect approves a smaller or more atomic decomposition. Never combine
unrelated work merely to hide a failing intermediate state.

Every commit that adds or changes a public library item documents the item and all public fields or
methods in rustdoc, and updates the crate entrypoint example when its usage changes. Public API
documentation is part of that behavior commit, not Phase 6 cleanup.

Before every commit:

1. Run focused Cargo checks and tests for the changed crates.
2. Run cargo fmt --all -- --check.
3. Run nix run .#check.
4. Run nix run .#test.
5. Run nix run .#test-db.
6. Review the staged diff and deletion searches.

Run nix run .#ci after each major graph/replay or contract-state phase and once more for final
merge readiness.

### 4. Escalate unclear architecture before coding around it

The accepted RFC has no open product decisions. If repository evidence makes an ownership,
lineage, type, or certification decision unclear, stop before adding an abstraction. Spawn an
architect-agent and pass this plan, the RFC, the conflicting code evidence, and rules 0 through 3
above.

Compatibility, fallback, speculative retention, and a second public path are not candidate
solutions.

## Definition of done

The work is complete only when all of the following are true:

- App discovery returns exactly mfm.portfolio/snapshot@1.
- The request strict-decodes exactly one portfolio CatalogRef and no other policy or child config.
- All eight old IDs are rejected, including unversioned, alias, and latest-like forms.
- Setup publishes PortfolioConfig only within this RFC's domain scope.
- PortfolioConfig has one direct Native or Erc20 holding-source algebra and no parallel role,
  protocol, reader, kind, or underlying-source authority.
- Holding demand is exactly the normalized wallet-to-symbol edge set.
- One network coordinator resolves one joint tip and shares it with every demanded child.
- BTC native, EVM native, and EVM ERC-20 collection produce exact source receipts.
- ERC-20 decimals and balances use exact hash-anchored calls and evidence-only replay.
- PortfolioCollectionReceipt proves exact logical coverage and exact FactContentIdentity values.
- Reporting queries the receipt's exact source and anchor, proves N + 1 exhaustion, hydrates all
  candidates, filters by exact identity, and only then applies deterministic claim ordering.
- The public root returns one direct snapshot/report object with canonical zero preservation,
  direct quote totals, and no role totals or generated_at_ms.
- The report path has no live BTC/EVM capability.
- The portfolio tracker operation crate, EVM contract lifecycle operation crate, and one-purpose
  lifecycle adapter-contract package are absent.
- Deploy, configure, and validate contract states remain executable and replayable without a
  production operation or import/adoption continuation.
- Configure retains an exact block-number/block-hash validation anchor.
- Optional expected contract code hash is actually verified from retained bytecode evidence.
- Old code, tests, fixtures, docs, examples, packages, dependencies, setup kinds, and descriptors
  are deleted, not hidden.
- Focused tests, all repository Nix gates, and final nix run .#ci pass.
- A separate architect-agent has returned APPROVED for every phase gate and the final cutover.

## Target ownership and simplification budget

### Package outcome

Use existing semantic crates. Do not add a crate for ERC-20, receipts, demand compilation, or a
public facade.

The intended package cut is:

| Current package | Final decision |
|---|---|
| mfm-op-portfolio-collect-report | Rename in place to mfm-op-portfolio-snapshot and make it the complete public-objective owner. |
| mfm-op-portfolio-tracker | Fold its internal report expansion into the snapshot crate, then delete the package. |
| mfm-op-evm-contract-lifecycle | Delete the package. Retain state and adapter crates. |
| mfm-adapter-contracts | Fold its minimum binding identity into the retained contract-state boundary, then delete this lifecycle-only package. |
| mfm-op-btc-collectors | Retain as internal BTC network/resource graph ownership; delete standalone root/public helpers. |
| mfm-op-evm-collectors | Retain as internal EVM network/resource graph ownership; delete standalone root/public helpers. |
| mfm-states-btc | Retain source-near BTC observation, fact, and receipt semantics. |
| mfm-states-evm | Retain native semantics and add separate ERC-20 metadata/balance/fact semantics. |
| mfm-facts | Own the one generic FactContentIdentity boundary type. |
| mfm-state-portfolio | Own exact fact selection, valuation, assembly, and projection states. |
| mfm-adapters-portfolio | Bind fact queries/evidence/replay for exact receipt-constrained selection. |
| mfm-state-evm-contracts | Retain and tighten the three contract state primitives. |
| mfm-adapters-evm-contracts | Retain state runner/replay bindings; delete source-run and import/adoption support. |

At implementation start, record the actual package baseline P0. With the planned in-place rename
and three package deletions, the expected final count is:

~~~text
Pfinal = P0 - 3
~~~

The current planning baseline contains 52 packages, so 49 is evidence to check, not a value to
blindly hard-code if the branch moves before implementation. Update the Cargo metadata contract from
the implementation-time baseline and assert the three removed packages are absent.

Any new package or failure to remove any of the three packages requires RFC_DECISION_REQUIRED from an
architect-agent before proceeding.

### Final responsibility map

| Responsibility | Owner |
|---|---|
| Portfolio/network/wallet/symbol/source semantics | crates/portfolio/model |
| Exact demand compilation and complete root graph | portfolio snapshot operation crate |
| BTC joint tip and native at-anchor subgraph | BTC internal operation crate |
| EVM joint tip and native/ERC-20 at-anchor subgraphs | EVM internal operation crate |
| Source observation, normalization, fact writing | BTC/EVM state crates |
| Generic EVM block, balance, and call capability binding | EVM adapter and transport crates |
| Exact fact-content identity | crates/kernel/facts |
| Exact portfolio receipt assembly | operation-local pure state in snapshot crate |
| Receipt-constrained selection and report semantics | portfolio state and adapter crates |
| Catalog admission and single entry-point dispatch | crates/app |
| Runtime provider routes and credentials | runtime config and live app assembly |
| CLI/REST transport and rendering | thin binaries |
| Contract deploy/configure/validate semantics | contract state/model crates |
| Contract live/replay bindings | contract adapter/capability/transport crates |

### Public-type budget

The expected new cross-crate public contracts are narrow:

- HoldingSourceConfig and the reshaped semantic/output values required by model consumers.
- HoldingRequirementKey and HoldingSourceKey only where operation/state boundaries require them.
- Typed BTC, EVM-native, EVM-ERC-20, network, and portfolio receipt values.
- FactContentIdentity.
- Typed ERC-20 observations/facts required by state and adapter boundaries.
- ConfiguredContractAnchor and the minimum contract validation request/evidence values.
- PortfolioPublicOutputs. Keep the strict app request DTO private inside mfm-app unless a concrete
  cross-crate consumer proves otherwise.

Keep compiler indexes, grouping maps, expected-set builders, receipt comparison helpers, ABI
decoders, and operation-local assembly types private or crate-private whenever the framework does
not require public visibility.

With one entry point, do not preserve a public one-variant EntryPoint enum merely to resemble the
old registry. Prefer a direct constant/descriptor and a single dispatch path unless a framework
trait requires a type. Apply the same scrutiny to setup enums and descriptor registries.

## Phase 0: capture the implementation baseline

Phase 0 makes no implementation commit. It records evidence needed to prove that the final design
is smaller and that deleted paths did not move elsewhere.

### Required baseline record

Record:

- HEAD SHA and git status;
- workspace package count;
- direct workspace dependency-pair count;
- Rust source LOC;
- public declaration count, separated by workspace and affected crates;
- all current app entry-point IDs;
- all current setup kinds;
- all current operation descriptors;
- affected package dependency graph;
- focused tests currently covering facts, portfolio model, portfolio collection/reporting,
  BTC/EVM collection, EVM call reads, contract states, replay, app discovery, CLI, and REST;
- directories, symbols, schema IDs, setup kinds, and public IDs required to disappear.

Use reproducible commands and save their command text with the phase handoff. Suitable starting
points are:

~~~sh
git rev-parse HEAD
git status --porcelain=v1
cargo metadata --no-deps --format-version 1
rg -n "mfm\\.(portfolio|bitcoin|evm)" crates bin tests docs examples
rg -n "^pub(\\([^)]*\\))? (struct|enum|trait|type|fn|mod|const|static)" crates bin
rg --files -g "*.rs"
~~~

Do not commit generated logs. Put the baseline in the implementation handoff or PR evidence.

### Baseline interpretation

The baseline is not a compatibility checklist. It is a deletion checklist. If an old symbol has no
consumer after its replacement, delete it even when it was not named individually in this plan.

## Critical implementation contracts

The following sections call out areas where a superficially green implementation could still
violate the RFC.

### Admission and certification boundary

The only public request is strictly:

~~~json
{
  "portfolio": {
    "name": "acme/primary",
    "digest": "content:sha256-jcs-v1:..."
  }
}
~~~

The app resolves and verifies the exact CatalogRef once during admission, calls the model's
validation/normalization path, and passes the normalized concrete PortfolioConfig to
PortfolioSnapshotOperation. Catalog identity remains admission evidence; the catalog is not
consulted by planning after resolution, execution, resume, replay, status, or output rendering.

PortfolioSnapshotOperation::Config is exactly PortfolioConfig. That normalized concrete value is
the sole authored, persisted, certified, runtime, and replay configuration authority. Expansion may
construct one non-wire, ephemeral ValidatedPortfolioConfig index from it for efficient proven
lookups. That wrapper is never independently authored, serialized, cataloged, persisted as a second
config, or exposed in public output. Keep its Rust visibility no wider than real cross-crate
compiler use requires.

Do not introduce a second authored snapshot config, child config vector, request policy, or
app-owned graph builder.

All joint-tip, source-read, and fact-scan policies are closed operation-version policy compiled into
certified state config. The public request contains none of them.

### Version-1 closed policy ledger

Do not let policy values emerge as adapter defaults. Before Commit 6, inventory the exact existing
state constants and record one private set of Portfolio Snapshot v1 constants used directly by
expansion. Do not add an authored policy DTO merely to group them:

| Concern | Version-1 authority |
|---|---|
| BTC joint-tip selection | Existing deterministic best-tip state policy, invoked exactly once per required BTC network/source. |
| EVM joint-tip selection | Existing admissible joint-tip resolve state, invoked exactly once per required EVM network; only this state may resolve a moving head. |
| Joint-tip/source read bounds | Exact state-owned non-zero constants matching the capability calls and required anchor re-verification. Never caller-authored. |
| BTC/EVM native coverage | Existing closed complete configured-source coverage, if architect review confirms it represents the full demanded source at the anchor. |
| ERC-20 coverage | complete_at_anchor. |
| Successful source status | ok. |
| Fact store scope | Existing certified internal Platform-fact scope from execution context; not request/config input. |
| Fact candidate bound N | One private non-zero Portfolio Snapshot v1 constant; query requests N + 1. |

The exact numeric fact candidate bound is an implementation policy the RFC intentionally denotes N.
It must be chosen from actual fact-store/query evidence and approved by the Gate 2 architect before
Commit 6 starts. Record the value and rationale in the gate handoff. Do not invent a default in the
adapter, accept it from the request, or proceed with None/unbounded. If existing query limits cannot
support a defensible fixed value plus N + 1 proof, return RFC_DECISION_REQUIRED.

Likewise, preserve exact state-owned source-read constants rather than assuming every state performs
one call; EVM observations may include a required anchor re-verification read. Expansion copies
these closed values into certified configs and tests their exact values.

### Direct portfolio model and exact demand

The one source authority is:

~~~rust
pub enum HoldingSourceConfig {
    Native,
    Erc20 {
        contract_address: NormalizedEvmAddress,
    },
}
~~~

EVM NetworkConfig owns native_decimals. ERC-20 decimals are never authored; they are observed at
the collection anchor.

Aggregate validation must prove the complete normalized join before expansion:

- every wallet network exists and has the matching family;
- every wallet-to-symbol edge resolves and stays on that network;
- BTC supports Native only;
- EVM supports Native and Erc20 only;
- token addresses are normalized and non-zero;
- at least one explicit wallet-to-symbol edge exists;
- no duplicate semantic wallet subject exists on one network;
- no two logical holdings alias the same physical source and risk double counting;
- quotes and valuation routes are complete and float-free;
- all normalized maps, vectors, and nested demand are deterministically ordered.

Demand is exactly the explicit edge relation. Do not infer native holdings for token-only wallets.
Do not form accounts-by-all-configured-tokens products. Unreferenced symbols create no collection
nodes.

Each logical edge receives one deterministic HoldingRequirementKey and one closed HoldingSourceKey.
Presentation IDs remain in the logical key/mapping; facts remain source-near and do not contain
wallet ID, symbol ID, quote, valuation, or report IDs.

### Shared network anchors and static typed fan-in

Each required network resolves exactly one joint tip. Native and token resource children consume
the same certified tip handle. A resource child never resolves another tip.

EVM reads that affect facts use the joint-tip block hash with EIP-1898
requireCanonical: true. There is no latest, block-number, tag, or provider-default fallback.

The operation binds deterministically sorted configuration-sized Vec<Handle<T>> values using the
program framework's existing certified typed input binding. Do not add erased handles, runtime
variadic state types, dynamic collector registries, or result-driven topology.

Empty family vectors are valid. The combined logical manifest is not empty.

Keep the BTC chain-head checkpoint/cycle only when its existing checkpoint consumer remains. It is
internal Control machinery, not a holding collector and not a public snapshot path.

### Complete ERC-20 behavior

Add ERC-20 as separate reusable states and resource operations, not as branches in a universal
collector.

Token metadata:

- call destination is the normalized token contract;
- calldata is exactly 0x313ce567;
- selector is the shared joint-tip block hash with requireCanonical: true;
- return value is exactly 32 bytes;
- the high 31 bytes are zero;
- the final byte is the observed u8 decimals;
- one metadata read occurs per unique token/network.

Token balance:

- destination is the normalized token contract;
- calldata is 0x70a08231 plus twelve zero bytes plus the normalized 20-byte account;
- return value is exactly one 32-byte unsigned word;
- support the complete uint256 range as a decimal digit string;
- zero is a successful present value;
- one balance read occurs per demanded token source key.

Do not use a generic ABI-to-JSON decoder that accepts trailing bytes or narrows integer output.
Prefer exact typed decoding helpers in the existing EVM core/state boundary.

The EVM adapter extends its existing bound provider with generic EvmCallReadCapability. Reuse the
generic EVM transport. Do not add an ERC-20 transport or public operation.

Retained evidence and replay must independently verify destination, exact calldata, exact hash
selector, canonical requirement, source/network/chain binding, raw bytes, exact result length,
decoding, observation, fact, receipt, and report-query binding. Replay must not construct a live
provider or consult runtime config.

### FactContentIdentity and receipt authority

Add exactly one reusable fact-boundary identity:

~~~text
FactContentIdentity {
  fact_descriptor_hash,
  subject_material_hash,
  response_schema_id,
  response_hash
}
~~~

Its canonical digest domain is mfm.fact.content-identity.v1.

Construct it only inside a verification path that recomputes the complete descriptor hash,
canonical subject-material hash, response schema ID, and canonical hydrated response hash. A
FactClaim or InternalFactRef is occurrence/reference material, not proof that those hashes match the
descriptor, subject, and retained response bytes. Do not implement unchecked From conversions from
either type. A constructor may accept a non-forgeable verified wrapper if the fact layer already
provides one; otherwise recording and report hydration recompute all four components before
construction.

Exclude FactClaimId, run coordinates, artifact/store occurrence metadata, store commit order, query
order, and catalog watermark. Prove that two verified claims with the same semantic fact content
have the same identity and that descriptor, subject, response schema, or response changes alter it.

Source receipts are typed by family/resource. BTC-native, EVM-native, and EVM-ERC-20 entries use
family-specific source key structs so an irrelevant cross-family variant is unrepresentable. Only
the portfolio compiler wraps those proven entries into the closed cross-family HoldingSourceKey
mapping. Do not pass a universal source enum into source-near states and runtime-reject irrelevant
variants.

Receipts contain sorted source keys, exact shared anchor, fixed successful coverage/status, and
exact FactContentIdentity per fact. They contain no portfolio presentation or valuation material.

PortfolioCollectionReceipt is assembled by a pure exact-set state from the expected logical
manifest and typed child receipts. It proves:

- all expected requirements are present exactly once;
- no duplicate or unexpected logical/source result exists;
- each source binding and anchor matches;
- all results on one network share the exact number/height and hash;
- coverage and source status are admissible;
- each logical requirement maps to the exact fact identity recorded for its source;
- at least one logical requirement completed.

Counts may be diagnostics only. They are never readiness authority.

### Receipt-constrained report selection

The report operation consumes the PortfolioCollectionReceipt handle as a real typed graph
dependency. For each holding it queries exact source, fact descriptor, exact receipt anchor,
state-owned coverage/status, certified store scope, and a fixed scan bound N.

Request N + 1 candidates:

- Exact(n), where n is no greater than N, proves exhaustion.
- AtLeast(N + 1) fails candidate_bound_exhausted.
- There is no successful truncated query.

On exact exhaustion:

1. Hydrate every candidate.
2. Recompute every candidate's FactContentIdentity.
3. Filter by equality with the receipt.
4. Require at least one identical-content claim.
5. Apply deterministic claim ordering only among identical-content claims.

Selecting a claim before identity filtering is forbidden. A same-anchor conflicting fact and a
newer fact at another anchor must not replace this run's collected content.

Report states consume retained fact-query evidence only. They declare no live BTC/EVM capability.

### Direct public output

The one root output is PortfolioPublicOutputs with snapshot and report fields.

An observation uses one direct source:

~~~rust
pub struct AnchoredHoldingSource {
    pub holding: HoldingSourceConfig,
    pub anchor: ObservationAnchor,
}
~~~

Delete parallel observation kind, role, protocol, reader, and underlying-source fields.

Every explicit holding remains present, including zero native and token balances. Every wallet and
the portfolio emit one canonical total row for every configured quote, including zero.

Totals are only:

~~~text
{ quote, total_value_dec }
~~~

They are checked direct decimal-string sums. Delete assets, collateral, debt, staked, net, sign
inversion, role dispatch, positive-bucket merging, and generated_at_ms plumbing.

Public output must not leak receipt entries, requirement/source keys, fact identities, claims,
artifacts, scan bounds, provider evidence, routes, or credentials. Snapshot and report network pins
are projections of selected facts and exactly equal receipt anchors.

### Retained contract-state boundary

Delete the lifecycle operation and continuation/import/adoption model. Retain only executable
deploy, configure, and validate state semantics plus their capabilities, adapters, runners, and
replay.

Configure accepts the deployed value type and certifies only
ContextBoundDeployContractState as an allowed producer descriptor. Validate accepts the configured
value type and certifies only ContextBoundConfigureContractState as an allowed producer descriptor.
Rust typestate tests prove wrong stage/value types; graph tests prove an arbitrary state producing
the same Rust value type is rejected by descriptor identity. Do not add producer-specific wrapper
types solely to make Rust distinguish producers. Move those tests to the state boundary before
deleting the operation crate.

Every successful transaction receipt must carry block_number and block_hash. The current generic
EVM receipt read drops blockHash; extend the generic EVM capability/transport response and propagate
the hash through contract receipt verification and replay.

Configure derives:

~~~text
ConfiguredContractAnchor {
  block_number,
  block_hash
}
~~~

Use the last successful configure receipt in certified transaction order, or the deploy receipt
when configuration submits no transaction. Prove the anchor canonical/final under the certified
policy.

When ContractProfile.deployed_code_hash exists, validate reads runtime bytecode for the configured
address at the exact configured block hash with requireCanonical: true. Retain content-addressed
bytecode plus selector and redacted source evidence. Recompute byte length and Keccak-256 during
live verification and replay.

- Authenticated empty code or a hash mismatch yields a successful report with valid: false.
- Malformed bytes, unauthenticated evidence, source mismatch, anchor mismatch, or internally
  inconsistent evidence fails execution.
- When expected code hash is absent, schedule no code-identity read.

Remove inert configure assertion fields and permanently empty provenance/import fields. Keep the
test-only deploy -> configure -> validate graph out of app discovery and setup.

## Mandatory architect readiness gates

An architect readiness gate is blocking after every implementation phase. Do not begin files owned
by the next phase until the current phase is APPROVED.

### Reviewer independence and scope

- Spawn a separate architect-agent. The implementing engineer cannot self-approve.
- Prefer a fresh reviewer at each major gate.
- Tell the reviewer to inspect the actual tree and commit range, not a summary alone.
- The reviewer is read-only and must not edit implementation files.
- Give the reviewer the RFC, this plan, AGENTS.md, docs/code-quality.md, docs/architecture.md, and
  docs/design.md.
- Pass rules 0 through 3 verbatim.

### Required evidence bundle

For each gate provide:

- phase number and outcome;
- base SHA and head SHA;
- commit subjects and diffstat;
- affected package/dependency graph;
- files and symbols deleted;
- focused test commands and results;
- all three per-commit Nix gate results;
- nix run .#ci result when the phase requires it;
- residue-search commands and every remaining match classified;
- package, dependency-pair, Rust LOC, and public-declaration deltas;
- known deviations from this plan and their prior architect approval;
- explicit request to decide whether the next phase may start.

Green CI alone is not proof. The review must inspect topology, exact-set behavior, replay authority,
negative cases, and residue.

### Required architect prompt

Use this prompt, replacing phase and commit range:

~~~text
Act as the architecture readiness reviewer for Phase <N> of
IMPL_PLAN_RFC_GENERIC_PORT_COLL_REPORT.md.

Read:
- AGENTS.md
- docs/code-quality.md
- docs/architecture.md
- docs/design.md
- RFC_GENERIC_PORT_COLL_REPORT.md
- IMPL_PLAN_RFC_GENERIC_PORT_COLL_REPORT.md

Review the actual tree and commit range <BASE>..<HEAD>. Do not edit files.

Non-negotiable evaluation rules:
0. Optimize for fewer concepts, fewer code paths, fewer public types, fewer duplicated
   responsibilities, and fewer places future changes must touch. LOC reduction is valuable.
1. No backward compatibility is maintained. All breaking changes are allowed.
2. Do not accept fallbacks, hidden old code, facades, aliases, or speculative retention.
   Superseded code must be deleted; git history is the recovery mechanism.
3. The work must remain divided into coherent, independently reviewable commits.

Evaluate the phase acceptance evidence and determine whether the next phase may start.
Look especially for:
- RFC boundary violations;
- old concepts or code paths that should have been deleted;
- duplicate authorities or redundant public types;
- abstractions added only to preserve old structure;
- missing negative, replay, determinism, or residue evidence;
- opportunities to delete or merge code without losing an invariant.

Return exactly one verdict:
- APPROVED
- CHANGES_REQUIRED
- RFC_DECISION_REQUIRED

For CHANGES_REQUIRED, list concrete blocking corrections.
For RFC_DECISION_REQUIRED, state the unresolved decision and the smallest viable alternatives.
Approval means this phase is complete and the next phase is architecturally ready to start.
~~~

### Gate handling

- APPROVED: record the commit range and verdict, then begin the next phase.
- CHANGES_REQUIRED: remain in the phase, make focused corrective commits, rerun required gates, and
  request another review over the expanded range.
- RFC_DECISION_REQUIRED: stop implementation. Run the decision escalation below. Do not choose a
  speculative abstraction locally.

### Architecture-decision escalation

Give the decision architect:

- exact conflicting RFC passages;
- concrete current code, type, schema, and dependency evidence;
- affected phase and proposed commit boundary;
- two or three smallest complete alternatives;
- expected package, public-type, code-path, Rust LOC, and future-edit-site deltas;
- rules 0 through 3 verbatim.

The decision must reject compatibility or transitional dual paths as alternatives. If the decision
changes an RFC boundary, update the RFC and this plan first in one dedicated documentation commit,
obtain architecture approval for that decision, and only then resume code.

## Progressive public removal rule

The implementation must never register mfm.portfolio/snapshot@1 while any old public ID remains.
The clean commit sequence removes obsolete public roots as their internal replacements become
usable, then publishes the new root only after all eight old IDs are gone.

This means some intermediate branch commits intentionally expose fewer or even zero public
operations. That is acceptable: they are not release points, and no compatibility promise exists.
Do not keep an old ID by dispatching it to new internals. That would be an alias/facade even if it
made intermediate manual testing more convenient.

The fixed public-ID timeline is:

| Commit | Public surface change |
|---|---|
| 2 | Delete both old portfolio IDs and the standalone EVM-native ID. |
| 5 | Delete the standalone BTC balance ID. |
| 8 | Delete all four contract IDs. |
| 10 | Publish mfm.portfolio/snapshot@1 after every old ID is absent. |

Every commit that removes an ID must update the app registry, strict request decoding, setup kinds,
discovery tests, CLI/REST tests or fixtures that directly enumerate it, and the immediately
affected public documentation. The final commit publishes the one new ID and completes the
repository-wide public contract.

## Phase 1: establish content identity and cut the portfolio vocabulary

### Outcome

The fact layer has one exact semantic content identity, and PortfolioConfig expresses only the
holding sources the platform will execute end to end. No old role/protocol/reader/Aave vocabulary
survives.

### Commit 1: add exact fact content identity

Subject:

~~~text
add exact fact content identity
~~~

Primary ownership:

- crates/kernel/facts/src
- crates/kernel/facts/tests or module-local tests
- crates/kernel/facts/README.md
- fact schema/descriptor inventories directly affected by the new value

Required work:

1. Add FactContentIdentity with exactly:
   - fact_descriptor_hash;
   - subject_material_hash;
   - response_schema_id;
   - response_hash.
2. Add canonical validation/encoding and digest derivation under
   mfm.fact.content-identity.v1.
3. Add one checked construction path that recomputes descriptor, canonical subject material,
   response schema, and canonical hydrated response content. Reuse canonical fact hashing logic.
   Do not trust compact hashes copied from a FactClaim or InternalFactRef without that material.
4. Keep occurrence metadata out of the type and digest.
5. Export only the constructors/accessors required across collection and report crates.
6. Document the distinction between semantic fact content and a stored claim occurrence.

Do not:

- add a portfolio-specific identity;
- include FactClaimId, run ID, artifact ID, evidence location, store ordering, or query order;
- accept caller-authored hashes without verifying their canonical relationship;
- implement unchecked From<&FactClaim> or From<&InternalFactRef>;
- add a second identity wrapper in receipt crates later.

Focused proof:

- same canonical descriptor/subject/response across different claim/store occurrences yields the
  same identity;
- descriptor change changes identity;
- subject change changes identity;
- response schema change changes identity;
- response content change changes identity;
- canonical round trip and digest domain tests;
- malformed hash/schema components fail;
- a structurally valid claim/ref whose hashes disagree with hydrated canonical material fails;
- source recording and report hydration independently recompute the same identity;
- no float or secret-bearing field can enter the hashed value.

Suggested focused commands:

~~~sh
cargo test -p mfm-facts
cargo test -p mfm-integration-tests --test cargo_metadata_contract
~~~

### Commit 2: replace portfolio holding model

Subject:

~~~text
replace portfolio holding model
~~~

Primary ownership:

- crates/portfolio/model/src/symbol.rs
- crates/portfolio/model/src/portfolio.rs
- crates/portfolio/model/src/portfolio_snapshot.rs
- crates/portfolio/model/src/holding.rs
- crates/portfolio/model/src/ids.rs
- crates/portfolio/model/src/lib.rs
- crates/portfolio/model tests
- direct compile consumers in portfolio states/ops/app/setup/examples
- docs/persisted-public-surfaces.md and any model docs that would otherwise become false

Required work:

1. Install HoldingSourceConfig with only Native and Erc20 { contract_address }.
2. Put native_decimals directly on NetworkConfig::Evm.
3. Remove authored ERC-20 decimals.
4. Reshape SymbolConfig and observation source values around the direct source algebra.
5. Strengthen whole-aggregate validation and deterministic normalization.
6. Define or reshape HoldingRequirementKey and HoldingSourceKey only at the narrow owner boundary
   that needs them. Avoid making compiler internals model-public.
7. Replace role totals with direct quote totals.
8. Install the final pure checked direct-total reducer over explicit observation values, including
   one canonical zero row per configured quote. Later phases only wire receipt-selected native and
   token observations into this reducer.
9. Remove generated_at_ms and every assembly/replay/projection argument used only to inject zero.
10. Update all direct consumers to the new model without an old-to-new translation layer.
11. Keep existing setup fixtures schema-valid with the new model, using a minimal native-only
    portfolio while no public portfolio objective exists. Use model-internal fixtures to prove
    ERC-20 config. Defer runnable ERC-20 setup/runbook examples until Commit 10, when the complete
    public path exists.
12. Remove every authored/default EVM native-decimal authority outside NetworkConfig::Evm.
    Certified child/state configs may carry a copied decimal value only when it is deterministically
    derived from that network config and is not independently selectable.
13. Delete the standalone mfm.evm/evm_native_balance@1 request, setup kind, app dispatch, root/public
    launch helpers, examples, and direct public tests in this commit. Its authored collector config
    is incompatible with network-owned native semantics; do not retain it until Phase 3.
14. Delete mfm.portfolio/portfolio_snapshot@1 and
    mfm.portfolio/collect_then_report@1, their app request/dispatch/root launch paths, and current
    public docs/tests in this commit. Their request/output schemas become invalid with the direct
    model, and keeping them would advertise a partial ERC-20 workflow.
15. Delete CollectThenReportRequest, BitcoinCollectorPolicy, and EvmCollectorPolicy rather than
    retaining internal authored-policy versions. Until the final compiler lands, direct internal
    consumers use the same private fixed state constants recorded in the version-1 policy ledger;
    every EVM native decimal copy derives from NetworkConfig::Evm.
16. Update docs/design.md, docs/architecture.md, docs/persisted-public-surfaces.md, RFC_CONFIG.md,
    affected runbooks, and public library rustdoc/examples for the model and three removed IDs in
    this commit.

Delete in this commit:

- SymbolKind;
- SymbolRole;
- BalanceReaderConfig;
- protocol and underlying-symbol fields;
- ProtocolId and ProtocolReaderId when no consumer remains;
- AaveMarketId and AaveReserveId when no consumer remains;
- crates/portfolio/model/src/aave.rs;
- crates/portfolio/model/src/aave_tests.rs;
- Aave exports, descriptors, fixtures, and examples;
- assets/collateral/debt/staked/net output fields and reducers;
- observation kind/role/protocol/reader fields;
- generated_at_ms and its time/zero plumbing;
- default_native_decimals and any serde/default fallback;
- caller-authored EVM native collector decimals/config;
- the standalone EVM-native public ID and root/setup surface;
- both old portfolio public IDs, their request DTOs, and root launch helpers;
- the public CollectThenReport collector policies;
- branches, tests, and dependencies that only support those concepts.

Do not retain serde aliases, default old fields, conversion enums, legacy fixture readers, or a
compatibility SymbolKind derived from HoldingSourceConfig.

Required validation matrix:

| Case | Expected result |
|---|---|
| BTC wallet + BTC Native symbol | accepted |
| BTC wallet + Erc20 symbol | rejected |
| EVM wallet + Native symbol | accepted |
| EVM wallet + Erc20 non-zero normalized token | accepted |
| EVM token-only wallet | accepted; no native demand inferred |
| unreferenced symbol definition | accepted; no demand |
| no wallet-to-symbol edges anywhere | rejected |
| wallet/network family mismatch | rejected |
| symbol/network mismatch | rejected |
| zero or non-normalized token address | rejected |
| duplicate semantic wallet subject on one network | rejected |
| two logical holdings aliasing one physical balance source | rejected |
| incomplete or float-bearing valuation route | rejected |
| normalized equivalent input orderings | identical normalized value and demand order |

Focused proof:

~~~sh
cargo test -p mfm-portfolio-model
cargo test -p mfm-state-portfolio
cargo check -p mfm-app
~~~

If old operation consumers cannot compile without a compatibility translation, widen this commit
to update those direct consumers. Do not reintroduce removed fields. It is acceptable for a public
operation to be removed earlier than planned; it is not acceptable to preserve its schema with a
shim.

### Phase 1 deletion searches

Run code, schema, fixture, example, and current-doc searches for:

~~~text
SymbolKind
SymbolRole
BalanceReaderConfig
ProtocolId
ProtocolReaderId
AaveMarketId
AaveReserveId
balance_reader
underlying_symbol_id
generated_at_ms
assets_value_dec
collateral_value_dec
debt_value_dec
staked_value_dec
net_value_dec
default_native_decimals
mfm.evm/evm_native_balance@1
mfm.portfolio/portfolio_snapshot@1
mfm.portfolio/collect_then_report@1
CollectThenReportRequest
EvmCollectorPolicy
MissingNativeDecimals
~~~

Historical discussion may remain only in RFC_GENERIC_PORT_COLL_REPORT.md, this plan, or explicitly
marked historical architecture records. Classify every match; do not ignore an entire directory.
The field name decimals is expected in derived EVM native facts/state config and observed ERC-20
material; prove that none of those occurrences is an authored fallback.

### Architect Gate 1

In addition to the standard evidence bundle, require the reviewer to answer:

- Is FactContentIdentity the only new fact semantic identity?
- Is HoldingSourceConfig the only holding-source authority?
- Is native scale owned exactly once?
- Can an unsupported protocol, staking, Aave, or BTC-token combination still be represented?
- Does explicit wallet-to-symbol demand remain distinct from physical source observation?
- Did any removed concept survive in a wrapper, serde alias, fixture codec, or public helper?
- Is the package/public-type/LOC delta credible for a simplifying semantic cut?

Do not start Phase 2 until APPROVED.

## Phase 2: add complete anchored ERC-20 execution

### Outcome

The reusable EVM state and adapter boundaries can observe token decimals once, observe every
demanded token balance at the same hash anchor, record typed source-near facts, and replay entirely
from retained evidence. There is still no ERC-20 public operation.

### Commit 3: add anchored erc20 observation states

Subject:

~~~text
add anchored erc20 observation states
~~~

Primary ownership:

- crates/evm-core/src/encoding.rs or a narrowly named existing codec module
- crates/states/evm/src/erc20_balance_collect.rs
- crates/states/evm/src/lib.rs
- crates/states/evm tests
- typed ERC-20 fact descriptors/codecs
- docs/persisted-public-surfaces.md and crate rustdoc/examples for new public values/states/facts

Required work:

1. Add exact decimals result decoding:
   - exactly 32 bytes;
   - high 31 bytes zero;
   - last byte retained as u8.
2. Add exact balance result decoding:
   - exactly 32 bytes;
   - complete unsigned 256-bit value;
   - canonical decimal digit-string projection.
3. Add ObserveErc20TokenMetadataState bound to network, chain, token, shared EvmJointTip, observed
   decimals, and redacted read evidence.
4. Add ObserveErc20BalanceState bound to metadata, token, account, network, chain, shared tip, raw
   units, decimals, and redacted read evidence.
5. Add RecordErc20BalanceFactState and the typed
   evm.address_erc20_balance_snapshot fact.
6. Keep metadata and balance states source-near. Do not include portfolio wallet/symbol IDs,
   valuation, price, display metadata, or runtime route.
7. Keep zero as a successful fact value. Commit 5 adds the final receipt only when the network
   coordinator consumes it; do not add an unused or partial receipt type here.
8. Document every new public item and public field, and update the changed crate entrypoint rustdoc
   example in this commit.

Do not:

- add an ERC-20-specific capability or transport;
- use the permissive generic ABI JSON decoder for these exact words;
- use u64 for token balances;
- add symbol/name token reads;
- make token decimals authored or optional fallback config;
- merge native and ERC-20 observation into a universal dynamic state.

Focused proof:

- exact decimals calldata;
- exact balanceOf calldata and address padding;
- decimals values 0 and 255;
- non-zero decimals high padding rejected;
- short, long, empty, and malformed words rejected;
- balance zero and maximum uint256 retained exactly;
- wrong token/account/network/chain/tip binding rejected;
- zero fact recorded with successful complete-at-anchor semantics;
- fact subject/result contain only the RFC fields and no presentation material.

Suggested focused commands:

~~~sh
cargo test -p mfm-evm-core
cargo test -p mfm-states-evm
~~~

### Commit 4: bind erc20 reads and replay

Subject:

~~~text
bind erc20 reads and replay
~~~

Primary ownership:

- crates/adapters/evm/src/lib.rs and focused modules/tests split from it as needed
- crates/transports/evm
- crates/evm-capabilities
- crates/app/src/evm_collector.rs
- app runner/replay registration
- docs/evm-rpc-routing.md

Required work:

1. Extend the existing EvmBoundProvider binding to generic call reads.
2. Register the existing EvmCallReadCapability for the same certified network/provider binding used
   by block and native-balance reads.
3. Bind metadata and balance state intent to exact EIP-1898 blockHash calls with
   requireCanonical: true.
4. Retain narrowly typed redacted external-read evidence:
   - destination;
   - exact calldata;
   - exact hash selector and canonical flag;
   - raw return bytes;
   - certified provider-source identity.
5. Add managed fact recording and pure receipt runner bindings.
6. Add replay applicability and recomputation for metadata, balance, fact, and receipt outputs.
7. Verify app live assembly exposes call capability through the existing per-network EVM provider.
8. Lock existing transport behavior with focused tests; change transport code only where its
   generic EVM call contract is incomplete.

Failure semantics:

- RPC revert or failed authenticated call: required collection failure;
- unsupported canonical hash selection: failure;
- missing/mismatched network, chain, source, token, account, or anchor: failure;
- malformed return: failure;
- zero balance: success;
- replay evidence tamper: replay failure, never live fallback.

Focused proof:

~~~sh
cargo test -p mfm-evm-capabilities
cargo test -p mfm-transports-evm
cargo test -p mfm-adapters-evm
cargo test -p mfm-app
~~~

Use actual package names from cargo metadata if they differ from the descriptive names above.

### Phase 2 deletion and absence searches

Prove there is:

- no erc20 transport package/module;
- no erc20-specific capability when EvmCallReadCapability suffices;
- no token decimals field in authored portfolio/config setup;
- no latest or block-number fallback in ERC-20 reads;
- no live provider construction in replay;
- no generic JSON fact payload for ERC-20;
- no public ERC-20 entry point or setup kind;
- no raw endpoint, headers, opaque provider message, or credential in retained evidence.

### Architect Gate 2

Run nix run .#ci before this gate because live EVM capability binding and evidence-only ERC-20
replay are a major runtime/replay change.

In addition to the standard evidence bundle, require the reviewer to answer:

- Are calldata and response shapes exact and fail-closed?
- Does every read consume the shared hash anchor and require canonicality?
- Is the full uint256 range preserved without floats or narrowing?
- Is zero indistinguishable from success everywhere except its numeric value?
- Are facts source-near and presentation-free?
- Can replay recompute every binding and output without runtime config or a live provider?
- Did implementation reuse the generic EVM call boundary instead of inventing ERC-20 plumbing?
- Does the closed policy ledger name every source-read/store/coverage authority, and is the exact
  non-zero fact candidate bound N approved for Commit 6?

Do not start Phase 3 until APPROVED.

## Phase 3: replace collectors and pin reporting to exact receipts

### Outcome

BTC and EVM collection are internal deterministic network/resource subgraphs with one shared tip
per network and exact typed source receipts. The portfolio graph derives exact logical demand,
assembles one content-bound receipt, and reporting materially consumes it. Count readiness and
historical latest selection disappear together; there is no ignored receipt or bridge.

### Commit 5: replace standalone collectors with anchored internal receipts

Subject:

~~~text
replace standalone collectors with anchored internal receipts
~~~

Primary ownership:

- crates/states/btc/src/address_balance_collect.rs and tests
- crates/states/evm native/ERC-20 collection modules and tests
- crates/ops/btc-collectors-op
- crates/ops/evm-collectors-op
- BTC/EVM adapter receipt/replay bindings
- remaining BTC public dispatch/setup/discovery, CLI/REST tests, and collector docs
- docs/architecture.md and docs/persisted-public-surfaces.md

Required work:

1. Replace count summaries with final typed BTC-native, EVM-native, and EVM-ERC-20 receipt entries
   containing family-specific source key, exact anchor, fixed status/coverage, and checked
   FactContentIdentity.
2. Bind receipt assembly to every managed fact-record handle. Derive identity from the verified
   typed fact completed by that handle; no claim/store occurrence ID and no receipt before append
   completion.
3. Remove inert BTC observation time context and seed plumbing.
4. Make each family operation a network coordinator plus separate at-anchor resource operations.
5. Resolve one joint tip per required network. EVM native and ERC-20 children consume the same
   typed tip handle.
6. Produce the final ERC-20 receipt here, when its network coordinator immediately consumes it. Do
   not add a universal receipt or cross-family source enum to source-near states.
7. Schedule metadata once per unique token and balance once per demanded token source.
8. Keep node keys, scopes, configs, child vectors, source entries, and edges deterministically
   sorted.
9. Delete the remaining mfm.bitcoin/btc_address_balance@1 request, setup, app dispatch, holding-
   balance root/launch/public-output helpers, examples, docs, and direct tests.
10. Prove the EVM standalone ID/root deleted in Commit 2 remains absent.
11. Retain the BTC chain-head checkpoint/cycle only if its real Control consumer remains.
12. Update changed public library rustdoc and crate examples in this commit.

Delete:

- BTC/EVM standalone holding-balance draft/launch/public-output helpers;
- root constants and seeds used only by those holding-balance roots;
- duplicated batching and independent resource-tip resolution;
- remaining standalone BTC request/policy/setup types;
- count-only batch summaries/readiness;
- residual EVM public-root material.

Topology proof:

| Demand | Required topology |
|---|---|
| BTC only | no EVM nodes |
| EVM only | no BTC nodes |
| EVM native only | one tip, native child, no token child |
| EVM token only | one tip, token children, no native child |
| EVM mixed | one tip shared by native and token children |
| multiple token accounts | metadata once per token; balance once per source |
| unreferenced token | no token node |

Repeated expansion must produce identical keys, edges, bindings, and certified configs.

### Commit 6: replace portfolio readiness with receipt-pinned reporting

Subject:

~~~text
replace portfolio readiness with receipt-pinned reporting
~~~

This is one atomic authority cut. Do not commit exact receipts while report selection ignores them.

Primary ownership:

- crates/ops/portfolio-collect-report-op
- operation-local pure portfolio receipt state
- crates/states/portfolio
- crates/adapters/portfolio
- family exact-anchor fact-query helpers
- current internal tracker/report operation
- crates/app/src/composition.rs removed or reduced to non-planning runner binding
- docs/design.md, docs/architecture.md, docs/persisted-public-surfaces.md, RFC_CONFIG.md, and
  portfolio state/adapter rustdoc

Required work:

1. Set Operation::Config authority to normalized PortfolioConfig and build one ephemeral validated
   index during expansion.
2. Compile the exact sorted logical manifest and logical-to-family-source mapping.
3. Group physical work without losing logical coverage; derive all child configs from the approved
   private version-1 policy constants.
4. Bind sorted typed vectors of BTC and EVM network receipt handles to one pure
   AssemblePortfolioCollectionReceiptState.
5. Prove exact expected/actual equality, no duplicates/unexpected entries, source/network binding,
   exact number/hash anchor, admissible status/coverage, checked fact identity, and non-empty
   completion.
6. Produce one PortfolioCollectionReceipt with sorted CollectedHoldingReceipt entries, exact
   network anchors, and manifest identity.
7. Replace latest-network-coherent selection with
   mfm.portfolio.holding.collection-receipt-anchor.v1.
8. Make SelectHoldings consume the receipt handle as a typed graph dependency.
9. Query exact source, descriptor, anchor, coverage/status, certified store scope, and approved
   fixed N; request N + 1.
10. Accept only Exact(n) no greater than N. Fail AtLeast(N + 1) as
    candidate_bound_exhausted before hydration.
11. Hydrate every candidate, recompute checked FactContentIdentity, filter by receipt equality, then
    deterministically order only identical-content claims.
12. Add BTC-native, EVM-native, and EVM-ERC-20 hydration/projection and preserve zero.
13. Wire the final direct total reducers installed in Commit 2 to selected holdings. Do not create a
    second reducer or reintroduce role/time concepts.
14. Prove output pins exactly equal receipt anchors.
15. Extend retained query evidence and replay through cardinality, hydration, identity filtering,
    ordering, projection, and totals.
16. Keep report states free of live chain, catalog, and runtime-config authority.
17. Update all changed public item/field rustdoc and crate examples in this commit.

Delete:

- caller-authored child vectors and relational builders;
- any residual authored collector policy;
- PortfolioInputsReady and CollectThenReportReadiness config/state/output;
- family-count authority;
- app-owned graph planning;
- historical latest-common-anchor policy in this workflow;
- unbounded family candidate helpers used only here;
- ordering-before-hydration/identity;
- no-common-network-anchor as a normal snapshot path;
- residual role/time/count transforms;
- duplicate manifest/source-key types.
- ValidatedNetworkConfigs, ValidatedWalletConfigs, and ValidatedSymbolConfigs plus isolated-fragment
  state configs when the aggregate compiler/report graph leaves no independent consumer. Keep only
  the aggregate validation authority; require architect evidence for any survivor.

Exact receipt failures must cover missing, duplicate, unexpected, source/network mismatch,
inadmissible status/coverage, number/hash mismatch, fact-identity mismatch, aliases, and empty
combined demand.

Adversarial report proof:

| Situation | Required result |
|---|---|
| exact matching claim | selected |
| identical-content claims | deterministic evidence-backed selection |
| same-anchor conflicting response | cannot replace receipt identity |
| newer fact at another anchor | cannot replace receipt anchor |
| Exact(0) or no identity match | hard failure |
| AtLeast(N + 1) | candidate_bound_exhausted |
| descriptor/subject/response tamper | live/replay failure |
| zero native/token fact | present zero observation |
| one required holding missing | no root output |

Focused proof:

~~~sh
cargo test -p mfm-op-btc-collectors
cargo test -p mfm-op-evm-collectors
cargo test -p mfm-op-portfolio-collect-report
cargo test -p mfm-state-portfolio
cargo test -p mfm-adapters-portfolio
cargo test -p mfm-fact-capabilities
cargo test -p mfm-integration-tests --test cargo_metadata_contract
~~~

### Phase 3 residue and Architect Gate 3

Search source, manifests, tests, fixtures, setup, and current docs for:

~~~text
mfm.bitcoin/btc_address_balance@1
mfm.evm/evm_native_balance@1
BtcAddressBalanceBatchSummary
EvmNativeBalanceBatchSummary
PortfolioInputsReady
CollectThenReportReadiness
ValidatedNetworkConfigs
ValidatedWalletConfigs
ValidatedSymbolConfigs
btc_address_balance_program_draft
evm_native_balance_program_draft
latest-network-coherent
limit: None
~~~

Classify broad latest/limit matches. Snapshot resource children may not resolve latest
independently, and snapshot fact queries may not be unbounded.

Run nix run .#ci, then require the reviewer to answer:

- Does normalized PortfolioConfig alone determine topology?
- Is logical demand distinct from family-specific physical batching?
- Is there exactly one shared tip per required network?
- Are exact typed sets and checked identities authoritative?
- Does report selection materially depend on the receipt in the same authority cut?
- Is N fixed/approved and does N + 1 prove exhaustion?
- Does identity filtering precede ordering?
- Can historical/concurrent content replace this run's facts?
- Are report states free of live chain IO?
- Were standalone public roots, count readiness, latest selection, and duplicate policy deleted?

Do not start Phase 4 until APPROVED.

## Phase 4: assemble the complete internal snapshot objective

### Outcome

The snapshot operation owns the complete graph and one production-quality root draft/launch helper,
but the app does not publish it yet. The tracker package and all old portfolio operation vocabulary
are absent.

### Commit 7: assemble the portfolio snapshot objective

Subject:

~~~text
assemble the portfolio snapshot objective
~~~

Primary ownership:

- crates/ops/portfolio-collect-report-op renamed in place to crates/ops/portfolio-snapshot-op
- crates/ops/portfolio-tracker-op folded and deleted
- workspace Cargo.toml and Cargo.lock
- internal descriptor/runner aggregation
- complete-operation integration tests
- operation ownership docs and rustdoc

Required work:

1. Rename the package in place to mfm-op-portfolio-snapshot.
2. Make PortfolioSnapshotOperation own concrete PortfolioConfig and the complete demand ->
   collection -> exact receipt -> internal report -> PortfolioPublicOutputs graph.
3. Fold only the useful report expansion into this crate as an internal operation/module.
4. Delete mfm-op-portfolio-tracker and its dependency.
5. Delete residual CollectThenReport config/output/type names and old root helpers; both old public
   IDs and requests must already be absent from Commit 2.
6. Remove CatalogRef from operation contracts.
7. Install one final portfolio_snapshot_program_draft/launch builder in the operation crate. The
   same helper is exercised here and called by mfm-app in Commit 10; do not create a parallel
   cfg(test)-only graph.
8. Register only the descriptors required to certify and test this internal root. Do not publish
   mfm.portfolio/snapshot@1 yet.
9. Prove live execution, interruption/resume, and evidence-only replay for BTC, EVM native, ERC-20,
   mixed networks, zero values, source failure, receipt mismatch, query saturation, conflicting
   facts, and catalog/runtime/provider unavailability after admission.
10. Bind exactly one PortfolioPublicOutputs root value.
11. Update docs/architecture.md, docs/design.md, current portfolio workflow docs, and public crate
    rustdoc/examples for the new ownership and deleted tracker package.

Delete:

- crates/ops/portfolio-tracker-op and Cargo.lock entry;
- tracker workflow/root helpers;
- report-only wrapper;
- CollectThenReportConfig and old public-output/root vocabulary;
- old package-path re-exports or forwarding modules;
- dependencies made obsolete by folding.

Package proof:

- mfm-op-portfolio-tracker absent;
- package count is implementation P0 minus one at this point;
- report planning remains operation-owned;
- the final root helper exists once and is not app-public.

### Phase 4 residue and Architect Gate 4

Search for:

~~~text
PortfolioTrackerWorkflowOperation
CollectThenReport
crates/ops/portfolio-tracker-op
mfm-op-portfolio-tracker
generated_at_ms
assets_value_dec
collateral_value_dec
debt_value_dec
staked_value_dec
net_value_dec
~~~

Run nix run .#ci. Require the reviewer to answer:

- Does one operation own the complete objective with one root binding?
- Is the tracker package actually deleted rather than renamed/re-exported?
- Is PortfolioPublicOutputs minimal and free of internal identities?
- Does the exact final root helper exercise the same graph the app will publish?
- Are resume and replay independent of catalog/runtime/live providers after admission?
- Is the internal objective ready to publish after contract cleanup?

Do not start Phase 5 until APPROVED.

## Phase 5: delete contract operations and finish retained states

### Outcome

No contract public operation, setup kind, lifecycle operation package, lifecycle-only adapter
contract package, speculative app wiring, import/adoption continuation, or source-run authority
remains. Deploy, configure, and validate survive as directly composable typed states with exact
receipt anchors, meaningful configuration, reusable adapter-library runners/factories, and
evidence-only replay.

### Commit 8: remove contract operations and continuation paths

Subject:

~~~text
remove contract operations and continuation paths
~~~

Primary ownership:

- crates/ops/evm-contract-lifecycle-op deleted
- crates/adapter-contracts deleted
- crates/states/evm-contracts
- crates/adapters/evm-contracts
- crates/evm-contract-model
- contract-specific app wiring deleted; test support moved to state/adapter/integration boundaries
- app registry/setup/discovery/requests
- CLI/REST tests/docs for contract operations
- workspace Cargo metadata
- docs/design.md, docs/architecture.md, docs/persisted-public-surfaces.md, RFC_CONFIG.md, and
  contract state/adapter rustdoc

Required work:

1. Retain deploy/configure/validate state descriptors and runner bindings in the state/adapter
   libraries before deleting the operation crate.
2. Delete all four contract public IDs, entry configs, request schemas, app dispatch, setup kinds,
   catalog examples, CLI/REST paths, and discovery assertions.
3. Delete the complete mfm-op-evm-contract-lifecycle package.
4. Fold the minimum contract-state adapter kind/version/binding helper into
   mfm-state-evm-contracts using the same direct AdapterBindingSpec pattern as other state crates.
   Rename it around contract states, not lifecycle. Delete the complete mfm-adapter-contracts
   package and its redundant AdapterBindingDescriptor/capability list.
5. Delete ImportDeployedContractState and ImportConfiguredContractState.
6. Delete ImportDeployedSpec, ImportConfiguredSpec, source-run import, external-adoption request,
   evidence, policy, provenance, errors, fixtures, and replay branches.
7. Delete crates/adapters/evm-contracts/src/source_run_support.rs and source-run certification
   registry/factory plumbing.
8. Collapse deployed/configured provenance to the single direct producer lineage.
9. Keep configure's allowed producer descriptor set limited to
   ContextBoundDeployContractState and validate's limited to
   ContextBoundConfigureContractState.
10. Move trybuild typestate tests from the deleted operation crate to
    crates/states/evm-contracts/tests. Trybuild proves wrong stage/value types only.
11. Add graph/certification negative tests with a test state that produces the same Rust output
    type, proving an unapproved producer descriptor is rejected. Do not invent producer-specific
    wrapper types merely to force a Rust type error.
12. Replace the operation-owned lifecycle test root with a test-only typed
    deploy -> configure -> validate graph.
13. Delete production app contract runner/factory registration and dependencies unless a current
    certified non-public graph consumer is proven. Retain reusable adapter-library runtime
    factory/provider abstractions and exercise them from the test graph.
14. Keep state descriptors/runners internal; do not add a replacement contract-states operation or
    setup kind.
15. Update architecture/design/persisted-surface/current contract docs and all changed public
    rustdoc/examples in this commit. Delete the public lifecycle runbook framing now, not later.

Delete operation-owned output wrappers and optional fields that are permanently empty after direct
lineage. Rename a lifecycle-named neutral registration helper only when a direct state-bound name
is clearer; do not keep the old name as an alias.

Required proof:

- wrong stage/value types fail trybuild;
- a same-output-type test producer fails graph certification because its descriptor is not allowed;
- state crate has no dependency on operation/app crates;
- state/adapter crates have no dependency on either deleted package;
- cargo metadata shows mfm-op-evm-contract-lifecycle and mfm-adapter-contracts absent and total
  packages equal implementation P0 minus three;
- test graph certifies and runs without app entry-point registration;
- production app has no speculative contract runner/factory registration or dependency;
- signer/transaction/finality policies and exclusive nonce resource claims remain;
- side-effect saga, ambiguity recovery, resume, and replay tests remain attached to state/adapter
  boundaries.

### Commit 9: complete contract anchor and code identity validation

Subject:

~~~text
complete contract anchor and code identity validation
~~~

Primary ownership:

- crates/evm-capabilities
- crates/transports/evm
- crates/evm-contract-model
- crates/states/evm-contracts
- crates/adapters/evm-contracts
- reusable adapter-library runtime factory/provider and test-graph wiring
- docs/evm-rpc-routing.md
- contract model/state/adapter rustdoc and current retained-state documentation

Required work:

1. Extend generic EvmReceiptReadResponse to retain block_hash in addition to block_number.
2. Parse and verify RPC receipt blockHash instead of discarding it.
3. Propagate block hash through ContractTransactionReceipt, deploy/configure confirmation,
   mutation evidence, fixtures, finality verification, and replay.
4. Introduce ConfiguredContractAnchor { block_number, block_hash } as direct configured instance
   material.
5. Select the last successful configure receipt in certified transaction order, or the deploy
   receipt when configuration submits no transaction.
6. Prove the selected anchor canonical/final under the certified policy.
7. Remove configure confirmation read/event assertion fields and evidence plumbing that no
   implementation consumes.
8. When ContractProfile.deployed_code_hash is present:
   - include expected hash and exact configured block hash in typed validation request;
   - declare/bind the generic code-read capability;
   - call eth_getCode at the exact hash with requireCanonical: true;
   - retain content-addressed raw runtime bytecode, exact selector, and redacted source evidence;
   - recompute byte length and Keccak-256;
   - include presence and exact equality in valid.
9. When expected code hash is absent, schedule no code-identity read and no empty placeholder
   evidence.
10. Keep valid as the conjunction of every retained configured chain/call/log assertion plus the
    optional code-identity assertion. Code identity must neither replace nor short-circuit other
    configured validation semantics.
11. Repeat all content, digest, binding, source, selector, length, and Keccak checks in replay
    without a live provider.
12. Remove capabilities, request fields, report fields, errors, and branches that become unused
    after deleting inert assertions/import paths.
13. Update routing, persisted-surface, design details, and all changed public rustdoc/examples in
    this commit.

Do not broaden unrelated validation reads from Latest unless the RFC requires it. If existing state
invariants make the code-identity hash anchor impossible without changing those other reads, request
RFC_DECISION_REQUIRED rather than silently redesigning the whole state.

Required semantic tests:

| Case | Required result |
|---|---|
| expected hash present, matching non-empty code | successful report, valid true |
| expected hash present, authenticated empty code | successful report, valid false |
| expected hash present, different code hash | successful report, valid false |
| expected hash absent | no code-read capability invocation/evidence |
| malformed code bytes | execution failure |
| unauthenticated response | execution failure |
| wrong address/network/chain/source | execution failure |
| wrong block number or hash | execution failure |
| retained bytecode digest tampered | replay failure |
| recorded length/hash tampered | replay recomputation failure |
| zero configure transactions | deploy receipt anchor retained |
| multiple configure transactions | last certified successful receipt anchor retained |
| receipt missing block hash | failure; no number-only fallback |
| expected hash absent, another retained assertion fails | no code read; report valid false |
| expected hash matches, another retained assertion fails | report valid false |
| expected hash mismatches while other assertions pass | report valid false |

Test-only graph proof:

- live deploy -> configure -> validate;
- interruption and resume at mutation/confirmation boundaries;
- ambiguous submission recovery;
- finality and canonical receipt verification;
- exclusive signer/nonce claims;
- validation valid true and false projections;
- evidence-only side-effect and read replay;
- typestate compile failures and same-type producer descriptor rejection.

### Phase 5 deletion searches

Search code, manifests, schemas, fixtures, examples, current docs, and tests for:

~~~text
mfm.evm.contract/deploy@1
mfm.evm.contract/configure@1
mfm.evm.contract/validate@1
mfm.evm.contract/lifecycle@1
mfm-op-evm-contract-lifecycle
evm-contract-lifecycle-op
mfm-adapter-contracts
evm_contract_lifecycle_adapter
ImportDeployed
ImportConfigured
ExternalAdoption
source_run_support
SourceRun
configured_block_number
DeployProvenance
ConfigurationClaim
~~~

Some generic words may survive for unrelated run provenance. Classify every match and prove no
contract continuation/import authority remains. A compatibility type hidden behind cfg(test) is
still forbidden unless it is solely a compile-fail fixture demonstrating absence.

### Architect Gate 5

Run nix run .#ci before this gate because operation deletion, side-effect recovery, generic receipt
evidence, and validation replay are major changes.

Require the reviewer to answer:

- Are deploy/configure/validate states executable without a production operation?
- Did every contract entry, setup, operation descriptor, operation, import, adoption, and source-run
  path disappear while the three state descriptors remain?
- Are producer restrictions proved by typestate plus allowed-descriptor certification tests?
- Were the lifecycle-only adapter-contract package and speculative app wiring deleted?
- Does every successful receipt carry and verify number plus hash?
- Is ConfiguredContractAnchor selected deterministically from certified receipt order?
- Is code validation anchored by exact hash and backed by retained raw bytecode?
- Does empty/mismatch yield valid false while malformed/binding failure fails execution?
- Does absence of expected hash schedule no read?
- Can replay recompute all code identity checks without trusting recorded length/hash?
- Did inert fields and duplicate provenance disappear?

Do not start Phase 6 until APPROVED.

## Phase 6: publish the sole public portfolio objective

### Outcome

The app, setup surface, CLI, REST API, examples, fixtures, docs, and integration tests expose one
complete objective and one request shape. Every old ID has already been deleted; this phase adds no
alias and reintroduces no low-level root.

### Commit 10: publish portfolio snapshot as sole operation

Subject:

~~~text
publish portfolio snapshot as sole operation
~~~

This commit is intentionally cross-cutting because the externally visible request/discovery
contract must land atomically.

Primary ownership:

- crates/app/src/entry_point.rs
- crates/app/src/config_setup.rs
- crates/app/src/lib.rs
- crates/app/src/replay_verifiers.rs
- crates/app/Cargo.toml
- bin/cli
- bin/rest-api
- tests/integration
- examples/setup
- workspace Cargo.toml, Cargo.lock, and metadata contracts
- nixfied.nix integration task names
- README and required architecture/design/routing/public-surface docs
- RFC_CONFIG.md

Required work:

1. Register exactly mfm.portfolio/snapshot@1.
2. Strict-decode exactly one portfolio: CatalogRef<PortfolioConfig> field using a private mfm-app
   request DTO. Do not expose the DTO across crates when CLI/REST already pass JSON through app
   admission.
3. Reject unknown/missing fields, old IDs, aliases, unversioned forms, latest-like selection, and
   old request shapes.
4. Resolve, verify, normalize, and retain the exact catalog source at admission.
5. Pass concrete PortfolioConfig to PortfolioSnapshotOperation.
6. Keep catalog resolution out of operation planning, runtime execution, resume, replay, status,
   and output rendering.
7. Publish only PortfolioConfig in setup within this RFC's scope.
8. Prove standalone collector, contract context/action/import, and obsolete child policy setup
   kinds and examples are already absent. Do not make this a deferred cleanup step.
9. Simplify discovery and dispatch around one operation. Delete a one-variant public enum if a
   direct constant/descriptor suffices.
10. Register the snapshot graph's internal collector/report/fact/adapter descriptors and runners.
    Do not register retained contract-state runners in the app without a current certified consumer;
    their reusable library factories and test graph are sufficient.
11. Keep binaries thin: parse/transport request, start/resume, and render only.
12. Update CLI and REST start/discovery/setup parity from the same strict request.
13. Add the complete public snapshot integration matrix using the final operation root helper.
    Owner commits must already have deleted old public integration roots.
14. Drop/recreate experimental branch data and regenerate branch fixtures/snapshots. Do not add
    migrations or compatibility codecs for deleted schemas.
15. Publish the new ingress and audit current docs in the same commit:
    - docs/design.md;
    - docs/architecture.md;
    - docs/persisted-public-surfaces.md;
    - docs/evm-rpc-routing.md, verifying prior ERC-20/contract-state updates rather than deferring
      them;
    - docs/portfolio-collect-then-report.md, preferably renamed/replaced with a snapshot document;
    - bin/cli/README.md;
    - bin/rest-api/README.md;
    - affected app/state/adapter READMEs;
    - RFC_CONFIG.md;
    - examples and runbooks.
16. Rename/add the current snapshot integration and Nix task names, and prove no stale old-operation
    names survived their owner commits.

Do not create:

- an @2 alias;
- an old-ID redirect;
- a latest alias;
- a request union;
- a facade operation;
- a child collector request;
- a report-only mode;
- a collect/reuse mode;
- a legacy catalog setup document;
- a replay compatibility registry.

Required public-surface tests:

- discovery returns exactly one row and exact descriptor;
- every one of the eight deleted IDs is rejected;
- aliases, unversioned IDs, and latest-like forms are rejected;
- request with exactly one valid portfolio ref is accepted;
- missing portfolio fails;
- extra request field fails;
- old collector/config/policy fields fail;
- setup inventory contains only PortfolioConfig for this domain;
- old setup kinds fail;
- CLI and REST resolve/start the same catalog ref and retain the same admission evidence;
- catalog mutation/revocation/unavailability after admission does not affect resume/replay;
- runtime config is consulted only for live collector capability binding;
- output JSON/text contains only the new direct contract.

Required end-to-end matrix:

- BTC native only;
- EVM native only;
- EVM token only;
- EVM native plus token;
- multiple wallets and tokens;
- multiple BTC/EVM networks;
- zero native and zero token holdings;
- unsupported BTC token config rejected before expansion;
- empty logical demand rejected before expansion;
- duplicate source alias rejected before expansion;
- source failure yields no partial report;
- exact receipt mismatch yields no report;
- query saturation yields candidate_bound_exhausted;
- live execution and resume;
- evidence-only replay with catalog/runtime/provider access disabled;
- CLI/REST parity against managed Postgres.

Package/dependency proof:

- workspace package count equals implementation P0 minus three;
- mfm-op-portfolio-tracker absent;
- mfm-op-evm-contract-lifecycle absent;
- mfm-adapter-contracts absent;
- app has no dependency on any deleted package or on unused contract state/adapter wiring;
- no binary directly owns planning;
- no new ERC-20/receipt/facade package exists;
- mfm-catalog-model is absent from operation crates unless a real non-admission semantic use is
  architect-approved.

### Final Architect Gate 6

Run all of:

~~~sh
cargo fmt --all -- --check
nix run .#check
nix run .#test
nix run .#test-db
nix run .#ci
~~~

Then run the standard architect prompt over the complete implementation range as well as the Phase
6 range.

Final APPROVED is forbidden while:

- discovery exposes anything other than the one snapshot ID;
- any old ID/request/setup/descriptor/root helper survives;
- any alias, fallback, facade, compatibility codec, or hidden old module survives;
- an RFC acceptance criterion lacks positive and negative proof;
- docs/examples/tests describe superseded behavior as current;
- package/dependency/public-type/LOC evidence is missing or unexplained;
- CI is red;
- the new operation needs a live catalog during resume/replay;
- report states have chain IO;
- contract states no longer execute or replay independently.

## Final cross-phase audit

Phase-local sections own detailed deletion and behavior proof. The final review uses this compact
matrix to catch cross-phase gaps; it must not become a deferred cleanup list.

| Surface | Final absence/invariant proof |
|---|---|
| Public ingress | Eight old IDs, requests, aliases, entry variants, setup kinds, roots, and public outputs absent; one strict snapshot ID/request present. |
| Portfolio model | Old kind/role/reader/protocol/Aave/time/role-total concepts absent; one direct source algebra and one network-owned EVM native scale. |
| Collection | Standalone holding-balance roots, independent child tips, count summaries, dynamic collectors, and authored policies absent; BTC Control cycle retained only with a real consumer. |
| Receipt/report | PortfolioInputsReady, CollectThenReportReadiness, latest-network-coherent selection, unbounded snapshot queries, order-before-identity, partial/fallback/reuse modes absent. |
| Packages | Tracker op, contract lifecycle op, and lifecycle adapter-contract packages plus every dependency/lock entry absent; no replacement facade package. |
| Contract states | Import/adoption/source-run/inert fields/number-only anchor/speculative app wiring absent; three state descriptors, runners, library factories, and test graph retained. |
| Persisted surfaces | Old schemas/examples/snapshots/database fixtures absent; current docs and rustdoc describe only current contracts; historical names limited to explicit RFC/plan history. |
| Proof | Model/topology/set equality, strict ABI, exact identity, N + 1, conflict resistance, zero, resume/replay, side-effect recovery, contract hash, CLI/REST/Postgres parity all asserted materially. |

Use rg first across source, Cargo manifests, lockfile, SQL, fixtures, examples, Nix, tests, and docs.
Search exact wire strings/schema IDs separately from Rust identifiers. Every match is deleted,
replaced, or classified with exact file/line as intentionally historical RFC/plan prose. Never use
a directory-wide allowlist.

Use the smallest relevant Cargo set during edit loops. The cumulative affected packages are
mfm-facts, mfm-portfolio-model, mfm-evm-core, BTC/EVM/portfolio state and adapter crates, BTC/EVM
collector ops, the snapshot op, EVM capability/transport crates, contract model/state/adapter
crates, mfm-app, and mfm-integration-tests. Obtain exact names from cargo metadata. Before the
snapshot rename use mfm-op-portfolio-collect-report; afterward use
mfm-op-portfolio-snapshot. Never add a package alias just so both commands work.

### Required managed gates

Before each commit:

~~~sh
cargo fmt --all -- --check
nix run .#check
nix run .#test
nix run .#test-db
~~~

After Phases 2, 3, 4, and 5, and for final merge readiness:

~~~sh
nix run .#ci
~~~

If a managed service test fails, inspect its actual service/log evidence and rerun the focused Cargo
test with explicit environment variables as described by AGENTS.md. Do not weaken or skip the gate.

## Complexity and edit-site accounting

At each architect gate compare against Phase 0:

- package count;
- direct workspace dependency pairs;
- Rust LOC in affected crates and workspace;
- public struct/enum/trait/type/function count;
- number of app public IDs and request variants;
- number of setup kinds;
- number of operation root/launch helpers;
- number of holding-source classification types;
- number of readiness/receipt authorities;
- number of contract producer/provenance branches;
- number of files a future holding source would need to change.

Expected final direction:

- packages: minus three;
- public IDs: eight to one;
- setup kinds in this domain: portfolio only;
- holding source authorities: one;
- readiness authority: exact portfolio receipt only;
- report anchor authority: collection receipt only;
- contract producer paths: direct deploy -> configure -> validate only;
- standalone collector/report/contract operation roots: zero;
- compatibility paths: zero.

ERC-20 and stronger receipt/replay proof will add necessary code. Judge that addition against the
deleted public roots, old model branches, duplicated readiness, tracker wrapper, contract operation,
and import/adoption machinery. A net LOC increase is not automatically a failure, but unexplained
layers, public types, or repeat edit sites are.

## Commit and agent coordination

### Commit list

The intended commit sequence is:

1. add exact fact content identity
2. replace portfolio holding model
3. add anchored erc20 observation states
4. bind erc20 reads and replay
5. replace standalone collectors with anchored internal receipts
6. replace portfolio readiness with receipt-pinned reporting
7. assemble the portfolio snapshot objective
8. remove contract operations and continuation paths
9. complete contract anchor and code identity validation
10. publish portfolio snapshot as sole operation

Architect corrective commits may be inserted within their phase. Keep them focused and include them
in the next review range. Do not squash away evidence that a meaningful architecture correction was
made unless the repository owner explicitly requests history cleanup.

### Safe multi-agent use

- One integration owner controls the phase branch and commits.
- Parallel engineer-agents may work only on disjoint files with a declared owner and shared type
  contract.
- Do not parallelize dependent schema and consumer changes across commits when either agent would
  add a compatibility bridge.
- Architect-agents are read-only during readiness review.
- No engineer starts the next phase while its predecessor gate is unresolved.
- Before committing, inspect git status and every diff because all agents share the worktree.
- Preserve unrelated user changes and never reset or overwrite them.

Good parallel splits inside a phase include:

- pure fact identity versus portfolio model tests in Phase 1;
- EVM core/state codecs versus adapter replay test inventory in Phase 2, after agreeing exact typed
  contracts;
- BTC internal receipt refactor versus EVM internal receipt refactor in Phase 3;
- family collector receipt work versus portfolio selection test preparation in Phase 3, while the
  integration owner keeps Commit 6's receipt/report authority cut atomic;
- model/state import deletion versus adapter source-run deletion in Phase 5.

The integration owner must resolve exports, dependency edges, descriptor registration, and
cross-crate tests. Subagents do not independently invent public types.

### When a phase cannot stay compile-green

First try to enlarge the affected commit to include direct consumers and deletions. Do not add:

- temporary conversion types;
- feature flags for old/new behavior;
- defaulted legacy fields;
- old package re-exports;
- dual descriptor registries;
- test-only production aliases.

If the necessary atomic commit becomes unreviewably broad, stop and ask an architect-agent for the
smallest complete decomposition. Pass rules 0 through 3. A temporary non-compiling worktree during
the edit is normal; a committed compatibility bridge is not.

## Final implementation handoff

The engineer's final handoff must include:

- the ten commit subjects and SHAs, including any corrective commits;
- architect verdict and reviewed range for every gate;
- final public ID and request/setup schema;
- final package/dependency diagram or concise inventory;
- baseline versus final package, dependency-pair, LOC, and public-type metrics;
- list of deleted packages, roots, IDs, setup kinds, schemas, and major types;
- focused test matrix with commands and results;
- nix run .#check, .#test, .#test-db, and .#ci results;
- residue searches and classified historical exceptions;
- any architect-approved variance from the RFC or this plan;
- explicit confirmation that no compatibility, fallback, alias, facade, or hidden old path was
  introduced.

The work is not ready for merge based on implementation self-assessment alone. Final readiness
requires the Phase 6 architect-agent's APPROVED verdict over the actual final tree.
