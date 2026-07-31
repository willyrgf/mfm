# Implementation Plan: Runtime History Choke Point

Status: ready for implementation

Branch: `refact-recov`

Authoritative design: [`RFC_RUNTIME_HISTORY_CHOKE_POINT.md`](RFC_RUNTIME_HISTORY_CHOKE_POINT.md)

RFC readiness commit: `09688b975b44e21069657f1fd71013ed220ef9d4`

Code baseline: `1b8fd8ad8` (`make operation completion a typed outcome`). The commits between that
baseline and the RFC readiness commit change documentation only.

## Material uncertainties

none

## Outcome

Replace the arbitrary graph scheduler and generic executor with one declaration-ordered,
statically structured operation program, a callback-free verified history fold, and a Runtime
cursor interpreter. Preserve the five append-only run-history record families and exact-head
atomic append. Move EVM sender/nonce convergence into injected EVM states plus one separate,
narrow wallet-nonce authority. Delete every superseded graph/executor path rather than adapting or
retaining it.

This plan implements one current design. It does not authorize:

- graph-to-structured compatibility lowering;
- a dual graph/structured Runtime;
- a generic executor, outbox, or effect lifecycle under a new name;
- a retired run-history or nonce-authority reader, decoder, migration, or certifier anywhere in
  the current tree;
- dual persisted schemas, a `v2` beside the current recoverability contract, or a second writer;
- EVM or nonce knowledge in generic Runtime, journal, store, or replay crates; or
- deferring required production behavior from the cutover commit to the hardening commit.

## Branch And Commit Discipline

Keep the final work on this same `refact-recov` branch in the following order:

1. `resolve runtime rfc implementation blockers`
2. `add runtime history choke point implementation plan`
3. `replace graph and executor with structured runtime history`
4. `harden structured runtime and wallet authority qualification`

Commits 1 and 2 are documentation commits. Commit 3 is the one inseparable production and
persisted-contract cutover. Commit 4 adds hostile, crash, concurrency, and operational
qualification evidence; commit 3 must already be coherent, documented, package-tested, and usable.

Do not commit intermediate compatibility states. Engineers may use local fixup commits while
working, but must fold them into the owning logical commit before handoff. If commit 4 uncovers a
missing production protocol or a design-contract change, amend commit 3 and, when necessary, the
RFC rather than disguising implementation as test hardening.

## Baseline And Simplification Gates

The baseline was measured from `1b8fd8ad8` with tracked files:

| Measure | Baseline |
| --- | ---: |
| All tracked Rust | 171,477 lines |
| Production-like tracked Rust | 122,273 lines |
| Affected production-like Rust | 109,533 lines |
| Affected SQL | 1,592 lines |
| `crates/kernel/runtime/src` Rust | 6,004 lines |
| `crates/kernel/spec/src` plus `crates/kernel/certify/src` Rust | 5,137 lines |
| Workspace members | 29 |

“Production-like Rust” means tracked `*.rs` under a `src` directory, excluding paths containing
`_tests`, `/tests`, or `test_support`. It deliberately includes inline tests and is a stable
comparison measure, not a claim that every counted line ships.

Known mandatory gross deletion is 27,395 lines before deleting obsolete portions of shared files:

| Required deletion | Gross tracked lines |
| --- | ---: |
| `crates/kernel/executor`, `crates/storages/executor-file`, and `crates/storages/executor-postgres` | 19,904 |
| graph-only planner, terminal, scheduler, materialization, and certified-node files listed below | 3,704 |
| `wallet_executor.rs` and `wallet_executor_tests.rs` | 3,548 |
| `wallet_state.rs` | 123 |
| obsolete qualified-transport UI pass fixture | 15 |
| obsolete `evm_audited_graph.rs` integration fixture | 101 |

Hard gates:

- all mandatory deletion paths are absent;
- workspace membership is 27: delete three executor crates and add one narrow EVM PostgreSQL
  authority crate;
- final tracked Rust is below 171,477 lines;
- affected production-like Rust plus SQL is below the 111,125-line baseline;
- no production generic lifecycle/history/fold replaces the executor under another name; and
- tests, validation, security controls, and required documentation are never removed merely to
  satisfy a line count.

Review targets:

- remove at least 10,000 net affected production-like Rust plus SQL lines, yielding at most
  101,125 lines in that measured scope;
- reduce `crates/kernel/runtime/src` to at most 3,200 Rust lines;
- reduce combined `crates/kernel/spec/src` and `crates/kernel/certify/src` to at most 3,000 Rust
  lines; and
- keep the new wallet-nonce PostgreSQL library and its production SQL below 4,000 lines.

Missing a review target requires an architect re-review before commit 3. The review must identify
each retained or added abstraction, its sole responsibility, why a smaller existing owner cannot
hold it, and the deletion it enabled. Test growth does not justify retaining production
indirection.

## Target Ownership

| Owner | Required responsibility |
| --- | --- |
| `mfm-program` and `mfm-program-derive` | Affine authoring DSL for ordered `State`, exhaustive `Match`, bounded `FanOut`, child calls, lexical values, and root result expressions. |
| `mfm-spec` | Canonical expanded and certified structured-program data, slots, paths, contracts, policies, nominal outcomes, and cursor vocabulary. |
| `mfm-certify` | Pure expansion verification, structural normalization, lexical dominance, exact failure plans, policy coverage, fan-out bounds, manifests, and one canonical `CertifiedProgram` authority. |
| `mfm-journal` | Exactly `RunAdmitted`, `StateTransitionCommitted`, `ExternalAccessAuthorized`, `ExternalAccessObserved`, and `RunClosed`. |
| `mfm-store` | Exact-head candidate validation, atomic object/fact/event append, callback-free fold, verified cursor/frontier, closure, and the non-cloneable `RunHistoryWriter` capability consumed only by Runtime assembly. |
| `mfm-runtime` | Sole run-history writer, admission, one cursor action, callback execution, sealed `QualifiedPhysicalBinding<K>`, and the private affine Read/Effect access bracket. |
| `mfm-replay` | Recorded verification and projection over the same fold; no live IO or second reducer. |
| `mfm-evm` | Canonical chain-instance, wallet-domain registration, store-incarnation, target-fence, nonce, intent, candidate, completion, failure, state, expansion, and domain-port contracts only; no durable registry or PostgreSQL implementation. |
| `mfm-evm-live` | Exact EVM transport and signer/nonce-authority adapter bindings; no lifecycle or scheduler. |
| `mfm-storage-evm-postgres` | Both the separately credentialed append-only wallet-activation registry admin CAS/deployment proof reader/offline verifier and the distinct per-lineage nonce schema/adapter, including current-incarnation checks, linearizable status, and reserve/activate/complete transactions. |
| `mfm-storage-postgres` | EVM-neutral generic RunHistory implementation only. |
| Qualified deployment chain-instance registry | Immutable, never-reused, one-to-one physical-chain inventory and declaration/proof issuance. Deployment supplies the authority; MFM owns only contracts, offline verification, and selected proof assembly. RPC providers and Runtime never own it. |
| Qualified deployment infrastructure | Non-rollback registry and external-fence lineages, non-exportable physical-target key, target-bound session/transaction-permit issuance, irreversible old-target and sender-path fencing, sibling-issuer exclusion, and promotion orchestration. No production bypass lives in a crate. |
| `mfm-app` and binaries | Qualification and capability assembly, with registry administration/fence issuance confined to deployment maintenance; normal start/resume and output rendering receive only the sealed nonce adapter and public verifier. |

## Required Deletion And Replacement

### Delete whole crates

- `crates/kernel/executor/`
- `crates/storages/executor-file/`
- `crates/storages/executor-postgres/`

Remove their workspace members, dependency edges, Cargo lock entries, READMEs, migrations, SQLx
metadata, roles, tests, fixtures, Nix tasks, app configuration, readiness checks, and public error
mapping.

### Delete graph-only kernel files

- `crates/kernel/certify/src/planner.rs`
- `crates/kernel/certify/src/terminal.rs`
- `crates/kernel/runtime/src/decision.rs`
- `crates/kernel/runtime/src/materialization.rs`
- `crates/kernel/spec/src/certified_node.rs`
- `crates/kernel/spec/src/certified_node_tests.rs`

Do not move their topology, ready-node, required-success, dependency-skip, alternative-source, or
global scheduling logic into new modules.

### Delete executor-shaped EVM files and tests

- `crates/live/evm/src/wallet_executor.rs`
- `crates/live/evm/src/wallet_executor_tests.rs`
- `crates/domains/evm/src/wallet_state.rs`
- `crates/app/tests/ui/pass/evm_wallet_executor_uses_qualified_transport.rs`
- replace `tests/integration/tests/evm_audited_graph.rs` with a structured-runtime integration
  fixture whose path and assertions contain no graph compatibility terminology;

Delete the old monolithic lifecycle content from:

- `crates/domains/evm/src/wallet.rs`
- `crates/live/evm/src/wallet_qualification.rs`
- `crates/live/evm/src/wallet_rpc.rs`

Rehome only reusable semantic or exact-transport material. Prefer focused current modules:

- `crates/domains/evm/src/chain.rs`
- `crates/domains/evm/src/nonce.rs`
- `crates/domains/evm/src/expansion.rs`
- `crates/live/evm/src/nonce_adapter.rs`

Do not retain compatibility exports from the old module names.

### Delete obsolete persisted and operational surfaces

- the executor PostgreSQL schema, owner/application roles, views, functions, triggers, fences, and
  qualification binary;
- executor pool, writer-fence, readiness, configuration, and error surfaces in `mfm-app`;
- `executor-postgres-qualification` in `nixfied.nix`;
- executor-specific recoverability predicates, support objects, corpus entries, DTO fields, CLI
  text, REST JSON, trace fields, and documentation;
- the special `FactSelection` execution/access kind, while retaining its purpose-limited scanner
  behind an ordinary certified `Read`; and
- empty or structural-inference fallibility such as the current empty
  `EvmSubmitTransactionFailure`.

### Zero-live-reference scan

At the end of commit 3, the following names must have no live occurrence in current code,
manifests, migrations, build tasks, schemas, or current-design documentation. Historical
discussion in the RFC, this explicit deletion inventory, and explicit old-byte hostile fixtures
are excluded. No current feature, library, application, binary, or maintenance target may parse
the retired schema:

```text
mfm-executor
mfm-storage-executor-file
mfm-storage-executor-postgres
mfm_executor
mfm_storage_executor_file
mfm_storage_executor_postgres
QualifiedExecutorExpansion
RecoverableEffectExecutor
AuthorizedEnsureAccess
TerminalEffectEvidence
ExecutorEnsureResult
EffectRequested
EffectSettled
DependencySkipped
BlockingSource
NodePhase
TransitionSlot
PendingEffectState
required_success_nodes
authorization_count
ordered_sources
RunTerminalContract
EvmWalletInitialNonceDescriptor
VerifiedTerminalEffectView
CapabilityOperation
AuthoredNode
AuthoredSourceSelector
AuthoredInputBinding
AuthoredPublicOutputBinding
CanonicalAuthoredProgram
AuthoredProgramBuilder
AuthoredHandle
CertifiedNodeContract
CertifiedSourceSelector
CertifiedInputBinding
ExpandedCertifiedSpec
CertifiedAdmissionArtifacts
AuthoredBaseKind
CertifiedInputDestination
CertifiedOutputBinding
ExecutorPre
ExecutorProtected
ExecutorPost
EvmSubmitTransactionFailure
EvmWalletExecutor
executor-postgres-qualification
executor_status
executor_operation_id
executor_binding_ref
ensure_result_contract
terminal_evidence_contract
```

The committed scan also rejects the identifier/string fragments `Executor`, `executor_`, and
`executor-` case-insensitively across live code, manifests, migrations, build tasks, schemas, and
generated contracts. Current documentation rejects every exact obsolete symbol or claim that an
executor path survives; prose may mention its deletion only in the RFC, this plan, or an explicit
removal rationale. Any retained executable occurrence must be an individually reviewed negative
fixture.

Likewise remove the SQL names `mfm_executor_owner`, `mfm_executor_application`,
`executor_schema_metadata`, `executor_bindings`, `executor_content_records`,
`executor_effect_frontiers`, `executor_resource_records`, and
`executor_effect_resource_links`.

Any exception must be a bounded negative fixture proving rejection, be named as such, and be
reviewed individually. There is no exception for a dormant API, feature-gated decoder, migration
tool, or offline reader.

## Retain And Reuse

Retain responsibility, not necessarily today’s API:

- `crates/kernel/canonical` JCS-style canonical JSON, hashing, bounds, and no-float enforcement;
- generic identity and content references from `mfm-ids`;
- `RetainedValueContract` as the only inhabited retained-value descriptor;
- object, fact, content-addressing, scanner, `TenantFactFrontier`, affine scan permit, and
  completeness-attestation machinery;
- `mfm-journal` as the five-family contract crate;
- generic exact-head, object, fact, backend, and conformance primitives in `mfm-store`;
- EVM-neutral RunHistory persistence in `mfm-storage-postgres`;
- bounded exact JSON-RPC transport in `crates/live/evm/src/transport`;
- reusable signer, keystore, Bitcoin, app-shell, CLI/REST transport, and redacted-error machinery.

Reuse is conditional on the retained code having one target responsibility. Delete wrappers and
aliases whose only purpose was the graph or executor design.

## Frozen Public Failure Sum

Commit 3 replaces the empty transaction failure type with this closed, payload-free initial sum:

```text
EvmSubmissionFailure =
    TransportUnavailable
  | ProviderUnavailable
  | SignerUnavailable
  | NonceAuthorityUnavailable
  | DestinationRejected
  | ObservationPolicyExhausted
  | ReplacementPolicyExhausted
  | NonceDomainBusy
  | NonceLineageDiverged
  | NonceCapacityExhausted
  | ExecutionReverted
```

Expansion owns one exact finite leaf-failure mapping table into this sum.
`CandidateProgressionConflict` is reconciled through status and is not a public failure variant.
`SupersededBeforeEntry`, `EntryUnknown`, malformed evidence, contract mismatch, and integrity
faults keep their generic non-domain meanings and cannot be mapped into this sum. Changing this
closed enumeration requires an RFC amendment before schema generation.

## Commit 1 — Resolve RFC Readiness Blockers

Subject: `resolve runtime rfc implementation blockers`

State: complete at `09688b975b44e21069657f1fd71013ed220ef9d4`.

Scope:

- freeze structured State/Match/FanOut semantics, typed result expressions, declaration order, and
  depth-two collect-all Pure/Read fan-out;
- freeze certified slot versus resolved reference phases, fragment substitution, nominal
  outcomes, proposed versus committed state outcomes, `Never`, exact failure-plan identity, and
  causal supersession;
- freeze selector-table-derived variant payload type/contract identity, dependent
  `Never | Typed(C)` failure maps, and exact default-route contract flow;
- freeze the non-self-referential tagged closure walk and qualified entry-point trust anchor for
  the exact program contract, certification predicate set, expansion profile, and policy;
- freeze cursor/frontier, five-family history, exact-head closure, access ambiguity, and resource
  rotation rules;
- freeze the EVM-neutral runtime/store boundary and the injected EVM wallet-nonce protocol;
- close current-schema domain activation, virgin pending/high-water, idempotency epoch, retry,
  status, candidate, signer, and completion contracts;
- freeze lineage-serialized atomic composite domain activation issuance, current physical-store
  incarnation, target-bound read/mutation authority, exact promotion/restore/crash ordering, and
  zero-registry-IO normal access; and
- record `Material uncertainties: none`.

Verification:

- final independent formal audit: no blocker or medium finding;
- final whole-RFC readiness audit: no blocker or medium finding;
- Markdown fence/headings and changed claims reviewed;
- no Markdown links were introduced; and
- `git diff --check`.

## Commit 2 — Add This Implementation Plan

Subject: `add runtime history choke point implementation plan`

Scope:

- add only this file;
- bind implementation to the ready RFC and the exact code/LOC baseline;
- freeze the commit sequence, ownership, failure sum, deletion list, cutover order, verification
  allocation, and stop conditions; and
- make code reduction a measured acceptance condition.

Verification:

- confirm every current repository path and package/task claim; separately verify that
  `mfm-storage-evm-postgres`, `wallet-nonce-postgres-sqlx-check`,
  `wallet-nonce-postgres-qualification`, and `runtime-history-choke-point-contract` are explicit
  creation/replacement targets rather than claims that they already exist;
- validate the baseline and deletion line counts with the commands below;
- review Markdown links and command claims; and
- run `git diff --check`.

## Commit 3 — Replace Graph And Executor With Structured Runtime History

Subject: `replace graph and executor with structured runtime history`

This commit is indivisible. It must not be created until the tree implements one complete current
design. The internal order below is a work order, not permission to publish intermediate commits.

### 3.1 Freeze canonical types before behavior

- Implement `FailureContract::{Never, Typed}` and the kernel-only `Never` sentinel with no
  `MfmValue`, codec, decoder, retained slot, or producer.
- Implement `ProposedStateOutcome`, nominal committed `StateOutcome`, `LaneOutcome`, and
  `OperationOutcome`.
- Implement semantic-call paths, occurrence paths, lexical slots, resolved lexical references,
  exact selector/input/fragment-boundary slots, `ProducerBound`, `FailurePlanBound`, and derived
  `FailurePlanIdentity`.
- Make `RunHistoryWriter` non-cloneable and consumable only by Runtime assembly. Seal
  `QualifiedPhysicalBinding<K>` as the exact immutable public certificate plus private invoker
  handle/credential; a public binding reference alone must not prepare access.
- Make state and fragment failure slots bind the exact typed
  `RetainedValueContract` reference, not merely the fact that a failure contract is inhabited.
- Implement sealed structural constructors: arm aliases reuse their source content reference;
  variant payloads derive their type, nominal contract, and canonical payload reference only from
  the selector contract's exact tag/path table; fan-out joins derive the canonical
  declaration-ordered vector reference.
- Implement one canonical `CertifiedProgramRef` whose object closure binds the authored and
  expanded programs, expansion profile and proof, policy coverage, public contracts, bounds,
  manifests, and secret-free implementation closure. Repeated admission fields are equality-only
  audit projections.
- Build its closure digest from canonical `CertifiedProgramComponents` plus the registered tagged
  depth-first object walk, excluding the digest/root, checking type and content references,
  rejecting active-stack cycles, and deduplicating completed DAG objects on first visit.
- Resolve the entry-point admission policy from qualified registry identity. Bind its exact
  program contract, certification predicate set, profile, policy versions, and coverage in the
  root; never accept certification rules selected by the root or caller.
- Implement the closed EVM failure sum above before generating schemas or fixtures.
- Freeze canonical encodings, domain separators, bounds, and no-float assertions for every new
  hashed type.

### 3.2 Replace graph authoring, specification, and certification

- Rewrite the operation builder around ordered blocks containing only `StateBinding`,
  `MatchBinding`, `FanOutBinding`, and expansion-only `FragmentBinding`.
- Make root success/failure helpers typed tail expressions, never instructions.
- Preserve declaration order and separate dense lexical ordinals from stable semantic labels.
- Implement planning-fixed non-empty collect-all fan-out, homogeneous lane contracts, transitive
  Pure/Read restriction, and certified nesting depth two.
- Implement pure expansion in this order: child substitution, capability lowering, policy
  wrapping, failure completion, normalization, certification.
- Make support states final expansion leaves; require explicit composite policies for
  cross-policy support.
- Implement exact pre/post wrapping, affine protected calls, exact failure plans, default handler
  tables, constrained handler routes, and fragment-boundary propagation.
- Make scope failure maps dependent: `Never` has an empty map and cannot construct
  `DefaultPropagation`; `Typed(C)` alone can construct mapper routes and default propagation
  parameterized by that exact `C`.
- Require `.or_default()` to use the exact `Propagate` variant-payload slot from its handler
  output as the scope-failure tail, with the enclosing scope, route entry, payload slot, and tail
  all carrying the same nominal contract and with no intervening binding or same-typed
  substitution.
- Delete cycle detection, arbitrary reachability, alternative-source, required-success,
  dependency-skip, terminal-node, and global ready-set certification.
- Add compile-fail coverage for forward references, cross-arm/lane values, effects in fan-out,
  wrong outcomes, missing handlers, fake `Never`, wrong plan identity, and excessive fan-out.

### 3.3 Replace journal payloads and the authoritative fold

- Keep exactly the five record families while replacing graph/node payloads with structured paths,
  exact inputs, access links, nominal referenced outcomes, and cursor-derived evidence.
- Implement one callback-free fold that verifies every prefix and derives the semantic state,
  outstanding access, cursor, actionable frontier, object/fact closure, and terminal outcome.
- Require exact-head compare-and-append and all-or-nothing object/fact/event binding.
- Make the store author the public physical-binding reference in authorization records only after
  its purpose-limited certificate verifier proves the exact implementation binding and admitted
  stable lineage. On refresh, require the recorded head or a monotonic descendant and reject
  rollback/sibling lineages without receiving private invoker/resource authority.
- Atomically append `RunAdmitted + RunClosed` or
  `StateTransitionCommitted + RunClosed` exactly when the root outcome first becomes derivable.
- Bind exactly one root outcome and its complete structural object closure; never append control
  records for Match, FanOut, joins, fragments, or outcomes.
- Reject a substituted certification component or audit projection even when the substituted
  object is independently valid.
- Re-evaluate the complete closure under the exact qualified entry-point predicate set and reject
  a stale/unqualified policy, weaker set, or proof valid only under a different set.
- Preserve durable `BlockedIntegrity` only for committed
  `ExternalAccessObserved::IntegrityFault`; leave callback/codec/contract/rejected-candidate faults
  uncommitted and repeatably attributable.
- Prove a definite `Returned` or `SafeFailure` can commit the exact typed state failure, traverse
  its one default or explicit handler, and atomically close
  `OperationOutcome::Failure`; that closed run contributes no admission or cursor block to a
  later distinct `run_id`.
- Update memory and PostgreSQL conformance together so no backend owns a different fold.

### 3.4 Replace Runtime and replay

- Make Runtime an interpreter over the verified structured cursor, not a graph scanner.
- Outside fan-out, expose at most one current state. Inside fan-out, derive the RFC’s exact
  declaration-ordered actionable frontier and minimum action path.
- Keep `drive_once` bounded to one semantic transition or one audited access operation.
- Preserve the private `Prepared -> Authorized -> PendingObservation -> CommittedObservation`
  Read/Effect bracket and affine invoker authority. Keep every field/constructor private, require a
  newly appended authorization to mint `Authorized`, and make settlement consume only the exact
  committed observation.
- Inventory every registered Read/Effect invoker, including fact scanning and all EVM operations.
  Qualify each as one bounded primitive operation: one affine `Authorized<K>` entry, no hidden
  retry/duplication or lower-level semantic loop, exhaustive totalization to the closed
  `AccessCompletion<K>`, and no outer normal error after accepting authority.
- Make `PendingObservation<K>` own one immutable logical key, authorization/request/completion,
  and append identity. Resolve only exact-identical content; on a definite predecessor-only
  stale-head race, rebuild only the envelope after proving the authorization remains outstanding,
  without reinvocation. Resolve ambiguous acknowledgement for the unchanged original append
  identity before any rebase, and never return normal success while pending material remains
  uncommitted or the store is unavailable.
- Permit refresh only from qualified `SupersededBeforeEntry`; leave unmatched Read and
  possible-entry Effect occurrences parked under the RFC’s initial policy.
- Commit only accepted proposed state outcomes; no callback or adapter constructs nominal
  committed outcome authority.
- Replace the special fact-selection access path with an ordinary Read invoker backed by the
  retained purpose-limited scanner.
- Rewrite replay, trace, and export against the same fold and structured provenance. No replay
  callback, live IO, legacy reducer, or second cursor is allowed.

### 3.5 Rewrite every production operation

Compile all three current `Operation` implementations into the new DSL:

- `PortfolioSnapshotOperation`;
- `EvmBalanceCollectionOperation`; and
- `EvmSubmitTransactionOperation`.

Preserve the portfolio network fan-out containing each child EVM read fan-out at certified depth
two. Make every conditional an explicit closed-sum Match, every retry/poll/replacement a distinct
bounded state occurrence, and every fallible state/fragment boundary own one exact handler.

Add registered pure expansions and profiles for semantic security policy, default failure
handling, EVM read support, and EVM submission. Runtime and store must remain unable to
distinguish authored from injected states. Keep best-effort logs, metrics, and spans as redacted
read-only observers outside semantic execution; only a genuinely required acknowledgement may be
an explicit state/fact/effect.

### 3.6 Implement current EVM domain and live boundaries

In `mfm-evm`, implement:

- `ChainInstanceDeclaration`, `QualifiedChainInstanceId`, `EvmChainLineageId`, route-membership
  proof contracts, and offline validation of the deployment-issued immutable, never-reused,
  one-to-one chain-instance registry proof;
- wallet-nonce domain/store-lineage/writer-epoch identifiers,
  `WalletNonceStoreIncarnation`, and sealed
  `QualifiedCurrentWalletNonceStoreIncarnation` contracts;
- current-schema `WalletNonceDomainActivationRecord`,
  `QualifiedWalletNonceDomainActivation`, qualified canonical finalized-block sender-nonce floor,
  exact replay-exclusion and terminal-prior-resource dispositions, and activation-record identity
  versus permanent-binding and producer evidence; current-incarnation proof is exclusively
  assembly-private physical binding, never canonical semantic request data;
- canonical wallet-activation registry request/proof contracts: the wallet domain is the primary
  key, activation-record identity is globally unique, the store lineage is a separate primary key
  with one monotonically advancing current-incarnation head, exact replay returns the original
  proof, and multiple domains on one lineage share that head;
- one `issue_domain_activation` contract that serializes with promotion on the lineage key,
  atomically creates or validates the lineage head plus inserts the domain/activation row, and
  returns a stable composite proof over both keys with exact ambiguous-ack resolution;
- target-bound read-session and fresh affine mutation-permit contracts tied to exact physical
  target/database identity, lineage, writer epoch, and transaction, with no serializable,
  cloneable, or static-token substitute;
- authenticated issuer, `SubmissionIntentId`, stable reservation/candidate/completion keys,
  nonce-free intent, bounded mutation-equivalent candidate family, and exact semantic signer;
- reservation, activation permit, contiguous prefix, status, terminal outcome, completion, and
  producer-bound current-run types;
- injected pending observation, status, binding, attestation, activation, observation,
  reconciliation, completion, and projection states;
- a canonical `ReadEvmWalletNonceStatusRequest` that carries the exact stable
  `ProducerBound<QualifiedWalletNonceDomainActivation>` and never a current-incarnation proof;
- a canonical `ReserveEvmNonceRequest` that carries the exact
  `ProducerBound<QualifiedWalletNonceDomainActivation>` and fresh
  `ProducerBound<QualifiedPendingNonceFloor>`; and
- the exact structured EVM submission expansion with mandatory status reads at every RFC
  reconciliation point.

In `mfm-evm-live`:

- extend the existing exact transport for qualified chain/genesis/finalized-anchor observations
  plus `eth_getTransactionCount(sender, exact_finalized_block)` during deployment qualification
  and `eth_getTransactionCount(sender, "pending")` for every reservation;
- require every new Runtime-authorized reserve Effect invocation—first use, later use, and a new
  caller whose semantic key may already exist—to consume a newly committed current-run qualified
  pending observation before entering the nonce authority;
- implement direct Runtime-authorized EVM Read/Effect invokers, binding wallet-nonce invokers only
  to the sealed current-target adapter, immutable activation proof closure, and offline verifier;
- attest deterministic candidate identity without retaining bearer bytes;
- sign and submit the exact activated candidate once; and
- reject signers whose attestation consumes quota, approval, anti-replay, billing, rate-limit, or
  other semantic state.

### 3.7 Add the separate wallet-nonce PostgreSQL authority

Add exactly:

```text
crates/storages/evm-postgres/
package: mfm-storage-evm-postgres
```

Its domain port and canonical request/response types come from `mfm-evm`. It owns:

- distinct append-only activation-registry and per-lineage nonce schemas in the one current `0001`
  migration;
- separate registry-admin, registry-public, nonce-application, owner, and qualification
  roles/pools, with explicit cross-write denial;
- a deployment-only activation-registry issuer/promoter and proof reader that produces an
  immutable closure for an offline public verifier;
- one lineage-key-serialized `issue_domain_activation` registry transaction that atomically
  creates or validates the current lineage head, inserts the permanent domain row and unique
  activation-record identity, and returns/resolves one composite proof over both keys;
- a lineage-head promotion transaction using that same serialization point, so issuance cannot
  validate one head and commit after promotion advances it;
- non-rollback registry, external-fence, and nonce-store lineages; irreversible session
  revocation and sibling-fence-issuer exclusion; writer epoch, exact physical-target/database
  identity, and public non-exportable-target-key binding;
- an open/assembly boundary that consumes the deployment-supplied external fence issuer and
  returns a sealed non-`Clone`, non-serializable target-bound adapter; no production fence bypass
  or in-crate self-attestation;
- exact verification and first-successful-reservation binding of the current-schema
  `WalletNonceDomainActivationRecord`;
- one transactionally consistent linearizable status read whose canonical request carries the
  stable permanent domain binding and whose exact database transaction/snapshot is atomically
  opened through the live target-bound session before returning even `Absent`;
- per-mutation acquisition and under-lock revalidation of a fresh non-replayable external-fence
  permit bound to the exact PostgreSQL session and transaction;
- target-fence, key-first, lock, incarnation-revalidate, re-resolve, validate, mutate, and
  permanent-result transactions for reserve, activate, and complete;
- pending-floor allocation, one-incomplete-intent exclusion, pairwise candidate identity,
  contiguous activation, older-candidate completion, full object closures, and exact
  creation-witness equality rules;
- validation that each reserve request carries the exact fresh qualified pending observation
  captured once for that affine invocation; the authority may treat its reference as
  creation-only evidence, while internal lost-ack key resolution/re-execution reuses the unchanged
  captured request and performs no RPC call or new Runtime authorization;
- bounded retry-transparent resolution of ambiguous database acknowledgements; and
- rejection of every old schema byte sequence without any retained reader or decoder.

The deployment-only registry plane issues one immutable, secret-free current-schema domain
activation record binding the permanent store lineage, initial store incarnation,
chain/domain/sender, issuer namespace, exact replay exclusion, exhaustive sender-path inventory,
definite terminality of every prior allocation/submitted candidate, no unresolved possible entry,
a qualified canonical finalized-block sender-nonce floor, and fences. The wallet domain is the
primary key and activation-record identity is globally unique. The store lineage is a separate
primary key with one monotonic current-incarnation head; multiple domains may use that lineage and
share its head. An exact same issuance returns the original proof. Any different lineage or
activation identity for that domain, or sibling/non-next lineage head, conflicts.

The registry transaction locks the exact lineage key, creates the initial head only when absent or
validates the exact existing head, inserts the domain binding, and makes both visible
all-or-nothing. A crash or ambiguous acknowledgement resolves both keys to the original composite
proof; it can never leave a domain row without its admitted lineage head or vice versa. Concurrent
domains proposing sibling initial targets conflict, and issuance cannot interleave with promotion.

For the first domain on a new lineage, first qualify the closed physical target and its
non-exportable target-bound capability, then execute composite issuance, then allow the first local
reservation. A domain joining an existing lineage must bind its exact already-current head and
cannot propose another target. A crash after registry issuance pins the domain to that lineage even
when no local reservation exists. On the first successful reservation, the authority requires
fresh pending to equal the finalized floor, validates and atomically binds the exact record with
the virgin pending-floor reservation, and retains the composite registry proof/producer references
only as creation evidence. A no-mutation response binds nothing. Concurrent first reservations
serialize under the domain lock; pre-lock and under-lock resolution both compare the exact
activation-record identity, while independently valid fresh proofs may differ.

Normal status/reserve/activate/complete calls perform zero activation-registry IO and acquire no
cross-authority transaction or lock. They use the sealed target-bound session, immutable registry
proof closure, and offline verifier. A copied database, configuration, lineage/epoch, public proof,
client credential, static signature, replayed permit, rolled-back fence head, sibling fence issuer,
or Rust-constructible token is insufficient without the non-exportable physical-target key and a
live deployment fence.

Promotion is the only current-incarnation head advance and uses the same lineage-key serialization
point as issuance. Its exact order is:

1. irrevocably fence and drain the old physical target, every old writer/signer/relayer/operator
   and direct-submit path, and resolve or abort every outstanding old-epoch transaction;
2. prove the final complete nonce prefix, including activation bindings, reservations, candidate
   activations, completions, active-intent/high-water state, permanent operation results, and
   object closures, bound with the fence proof to the same post-quiescence final old head;
3. hydrate the still-closed replacement from that exact prefix and qualify its non-exportable
   target-bound capability;
4. compare-and-append the next writer epoch/current-incarnation lineage head against the exact
   prior head; and
5. open the replacement for reads and mutations.

A crash before step 1 leaves the old head current. A crash after fencing and before registry CAS is
unavailable-safe and retries the same promotion. A crash after CAS retries exact and never reopens
the old target. Any unprovable fence, complete prefix, registry non-rollback lineage, or stale- or
sibling-writer exclusion forbids same-domain promotion and requires a new sender/domain. The
post-quiescence proof covers every domain on the promoted lineage. Restore preserves the exact
permanent binding and complete prefix; it is never a second virgin lineage.

There is no bootstrap operation, credential, role, result, certifier, migration reader, or
high-water import. A fully drained and fenced reused sender starts from fresh pending only when it
equals the qualified finalized nonce floor. If terminality, floor provenance, replay exclusion,
or exhaustive fencing cannot be qualified, activation uses a new sender/domain. Compatible
upgrades retain the one current schema and full readable lineage; an incompatible
persisted-schema or issuer-namespace change always uses a new sender/domain.

`mfm-storage-postgres` must not depend on `mfm-evm`, Alloy, or the new nonce crate. The two stores
may share a PostgreSQL server and low-level SQLx helpers or the deployment-fence pattern, but never
a role, pool, transaction, lock, writer credential, semantic registry, or cross-write path.

Extend `postgres-sqlx-offline-check` to compile both `mfm-storage-postgres` and
`mfm-storage-evm-postgres` with `SQLX_OFFLINE=true`. Add
`wallet-nonce-postgres-sqlx-check` to migrate the new current schema in an isolated managed
PostgreSQL namespace, run `cargo sqlx prepare --check`, validate both registry and nonce catalogs
plus role grants, and prove hostile schema mutation is rejected. Replace
`executor-postgres-qualification` with `wallet-nonce-postgres-qualification`, make qualification
depend on the new online SQLx/schema check, keep the expanded offline check in `.#check`, and put
the online check plus qualification in `.#test-db`, therefore in `.#ci`.

### 3.8 Cut over application and public surfaces

- Replace executor assembly with Runtime, RunHistory, qualified EVM invokers, the chain-instance
  registry, and one already sealed target-bound wallet-nonce adapter plus its public
  activation-registry proof closure/offline verifier.
- Put activation-registry issuance/promotion and external target-fence issuance behind a separate
  deployment-maintenance assembly path. Its proof reader may export the immutable public closure
  used at assembly; Runtime, normal application state, CLI/REST request paths, and registered state
  callbacks must not receive registry roles/pools, fence credentials, or raw pools.
- Qualify a one-to-one deployment chain registry and every sender-capable signer, relayer,
  operator, stale deployment, and direct-submit path.
- Require deployment maintenance to supply the immutable physical-chain inventory and
  declaration/proof closure, reject namespace reuse or non-bijective inventory, and assemble only
  the selected offline-verified declaration into normal application state. No RPC provider,
  Runtime path, or normal request may issue or mutate chain-instance identity.
- Implement the exact fence/drain, complete-prefix, registry-CAS, then replacement-open promotion
  sequence and fail closed to a new sender/domain when any proof is unavailable.
- Add the incompatible-release gate for incomplete intents, qualify the exact current-schema
  wallet-domain activation record, and require a new sender/domain for an incompatible schema or
  issuer namespace.
- Keep CLI and REST as start/resume/render wrappers; update their one current status, output,
  failure, and redaction contracts if projections change.
- Remove every executor-specific pool/fence/readiness/configuration/error field, retain only the
  independently required run-history fence, add the target-bound wallet fence, and prevent generic
  PostgreSQL pools from escaping assembly.

### 3.9 Reset persisted contracts once

Reset in place:

- `contracts/recoverability/v1/annex.json`;
- `contracts/recoverability/v1/corpus.json`;
- `contracts/recoverability/v1/README.md`;
- `tests/support/recoverability_v1.rs`;
- journal, trace, export, DTO, and object schemas;
- `crates/storages/postgres/migrations/0001_store.sql`; and
- the new wallet-activation-registry/wallet-nonce authority’s initial migration and qualification
  corpus.

Reject every old byte sequence. Do not add `v2`, a second migration path, a fallback decoder, or a
mixed-version reader/writer.

Update in the same commit:

- `docs/design.md`;
- `docs/architecture.md`;
- `docs/run-execution.md`;
- `docs/evm-transactions.md`;
- `docs/recoverability-predicate-owners-v1.md`;
- `docs/recoverability-app-surface-v1.md`;
- affected crate READMEs;
- `bin/cli/README.md` and `bin/rest-api/README.md` when their current surface changes;
- `docs/build-and-verification.md` for the expanded/new SQLx checks, replaced qualification, and
  `runtime-history-choke-point-contract` tasks; and
- every current fixture, example, trace, schema inventory, and generated contract.

### 3.10 Delete, scan, and measure

- Perform every deletion listed in this plan.
- Remove unused dependencies and features from all manifests and regenerate `Cargo.lock`.
- Add the `runtime-history-choke-point-contract` Nixfied leaf. It must run the zero-live-reference
  and SQL-name scans over tracked current surfaces, enforce an explicit reviewed allowlist
  containing only negative fixtures, verify workspace membership, and emit/fail the exact LOC
  gates and review targets below. Compose it into `.#check`, and therefore `.#ci`.
- Update the SQLx leaves and gate edges exactly as specified in 3.7; do not leave the new
  registry/nonce migration outside offline compilation, online prepare/schema mutation checks, or
  `.#test-db`.
- Recount workspace members and every LOC gate from the baseline commands.
- Inspect the largest surviving modules and delete pass-through wrappers, duplicate enums,
  parallel folds, stale aliases, and graph/executor terminology.
- If a review target is missed, complete the architect re-review before creating the commit.

### Commit 3 minimum tests

Commit 3 must already include:

- DSL unit and compile-fail tests for order, scope, Match, FanOut, fragments, `Never`, failure
  plans, exact failure contracts, default-handler payload flow, structural content references,
  selector-derived nominal payload contracts, canonical certification closure, qualified
  predicate-set binding, and nominal outcomes;
- compile-fail authority tests proving no crate outside the owning modules can construct or clone
  `RunHistoryWriter`, `QualifiedPhysicalBinding<K>`, `Prepared<K>`, `Authorized<K>`,
  `PendingObservation<K>`, `CommittedObservation<K>`,
  `QualifiedCurrentWalletNonceStoreIncarnation`, the target-bound live session, or a mutation
  permit; prove the latter three also have no serde path;
- registered-invoker contract tests covering one affine entry, lower-level call counts, total
  completion, no outer normal error, and no hidden retry/duplication for every inventory member;
- deterministic expansion tests for registry/map iteration variation and every production
  operation;
- memory-store and PostgreSQL happy-path/conformance tests for the five-family fold, exact-head
  append, closure, access bracket, and resume;
- Runtime/replay tests for sequential cursor advancement, depth-two fan-out, parked access, and
  atomic closure, including definite typed operational failure followed by successful admission
  and execution of an independent later `run_id`;
- pending-observation tests for exact-content resolution, stale-head envelope-only rebase,
  ambiguous-ack-before-rebase ordering, no reinvocation, and no successful escape while the store
  is unavailable;
- EVM canonical encoding, key, pending, status, candidate, completion, and projection unit tests;
- deployment chain-instance registry tests for immutable proof verification, one-to-one inventory,
  never-reused namespaces, selected declaration assembly, and rejection of RPC-authored identity;
- wallet-activation registry tests for lineage-serialized all-or-nothing issuance, exact composite
  proof replay/resolution, sibling initial-target conflict, and issuance/promotion exclusion;
- wallet-nonce PostgreSQL success, idempotent replay, basic contention, current-schema
  activation/virgin-first-reservation, registry exact replay/conflict, current-incarnation status
  gating with definite stale-Read `SafeFailure` and no `Refreshable`, rejection of `Absent`
  without the exact permanent domain binding, fresh transaction-bound mutation fencing,
  rejection, and upgrade-gate tests;
- a core copied-target test proving identical database/lineage/epoch/configuration/public proof
  cannot return semantic status or mutate without the one non-exportable target-bound capability;
- normal-access instrumentation proving status/reserve/activate/complete issue zero
  activation-registry calls and still operate with the registry database unavailable from the
  request path;
- app assembly tests proving registry-admin/public, nonce, and run-history role separation,
  maintenance credential confinement, and all three production operations; and
- old-byte, old-schema, and old-public-API rejection tests.

These are correctness tests, not the exhaustive qualification matrix assigned to commit 4.

### Commit 3 focused verification

Run direct Cargo/Rust commands only inside the default Nix development shell.

During implementation, check/test affected packages in this target work order:

```text
mfm-values
mfm-capabilities
mfm-program
mfm-program-derive
mfm-spec
mfm-certify
mfm-journal
mfm-store
mfm-qualified-run-test-support
mfm-runtime
mfm-replay
mfm-evm
mfm-portfolio
mfm-evm-live
mfm-storage-postgres
mfm-storage-evm-postgres
mfm-app
mfm
mfm-rest-api
mfm-integration-tests
```

The final Cargo metadata determines actual dependency scheduling; this list is a focused
change-surface checklist, not a hand-maintained substitute for the dependency graph.

Before creating commit 3:

```text
nix develop -c cargo fmt --all -- --check
nix run .#model-check
nix run .#run -- --task cargo-metadata-contract
nix run .#run -- --task postgres-sqlx-offline-check
nix run .#run -- --task recoverability-postgres-v1
nix run .#run -- --task wallet-nonce-postgres-sqlx-check
nix run .#run -- --task wallet-nonce-postgres-qualification
nix run .#run -- --task runtime-history-choke-point-contract
git diff --check
```

Run the new/changed Nixfied task directly only after confirming its exact id in `nixfied.nix`.
Use focused package and test filters while iterating. Do not run broad gates merely because the
commit is about to be created; commit 4 owns the final `.#ci` run.

## Commit 4 — Harden Qualification

Subject: `harden structured runtime and wallet authority qualification`

This commit adds adversarial evidence around the already complete design. It must not add a
missing scheduler, recovery protocol, authority, schema, compatibility path, or production
operation.

### Structured program and history attacks

- Reject forged lexical slots/references, wrong producer roles, cross-plan
  `FailurePlanBound`, non-dominating values, inactive arms, cross-lane values, wrong fragment
  roots/boundaries, same-type/different-contract failure slots, correct structural provenance with
  a wrong content reference, byte-identical variant payloads under a distinct nominal contract,
  default-handler payload substitution, certification-component or predicate-policy
  substitution, and nominal outcome substitution.
- Exercise every root provenance kind and admission-only structural closure.
- Property-test byte-identical expansion, stable identities, exact wrapper nesting, hard bounds,
  canonical closure order/DAG deduplication/cycle rejection, and every depth-two fan-out physical
  completion permutation.
- Crash and restart at admission, authorization, invocation, observation, settlement, handler,
  Match, fan-out lane/join, terminal transition, and closure boundaries.
- Race exact-head appends, duplicate observations, competing lane actions, physical refresh, and
  stale workers.
- Mutate every five-family payload, referenced object, fact, contract, and cursor relationship and
  prove callback-free rejection.

### Access, fact, and replay attacks

- Prove one unresolved occurrence cannot receive overlapping authorization.
- Prove non-owning crates cannot construct/clone writer, physical-binding, or access-bracket
  authority, including through serde, public fields, aliases, and feature combinations.
- Attack public binding certificates with a wrong implementation, stable lineage, head,
  descendant proof, rollback, or sibling lineage; prove the store's purpose-limited verifier
  authors the persisted public reference and cannot invoke/sign/mutate the resource.
- Fault-inject every registered invoker below its declared primitive boundary and prove one affine
  entry, bounded lower-level calls, no hidden retry/duplication, exhaustive completion, and no
  outer normal error.
- Prove only `SupersededBeforeEntry` creates `Refreshable`, unmatched Read remains waiting, and
  possible-entry Effect remains parked.
- Qualify each `Returned`/`SafeFailure` settlement and reject evidence laundering.
- Prove prior-run fact selection uses its exact authorization-captured frontier, complete scanner,
  and no generic query/append authority.
- Reproduce traces and projections from records alone and reject old or mixed persisted bytes.
- Race and lose acknowledgements for generic observation append; prove exact logical/content
  identity, ambiguous-ack resolution before envelope-only rebase, no reinvocation, and no normal
  success while observation persistence is unavailable.

### Chain, nonce, candidate, and completion attacks

- Distinguish independently operated fork clones while collapsing redundant routes to one
  qualified chain instance; reject aliases, namespace reuse, wrong anchors, and dual membership.
- Race first and later reservations across same and different intents with independently fresh
  lagging, equal, and provider-ahead pending observations; reject a retained, foreign-run, or
  synthesized observation for every new reserve invocation, while proving internal lost-ack
  resolution uses exactly the original invocation's captured observation.
- Lose database acknowledgements for applied and no-mutation responses; prove key-first resolution,
  retry transparency, one escaped disposition, and no extra Runtime authorization.
- Race activation at the same and different ordinals; reject skips, duplicates, family changes,
  wrong permits, wrong witnesses, and non-contiguous prefixes.
- Race activation with completion and prove any activated older candidate may win.
- Reproduce deterministic candidate hashes and signed envelopes across processes and qualified
  signer generations; reject semantic signer drift and any retained bearer bytes.
- Race status with activation/completion; reject stale/torn snapshots and prove current-run
  producer binding.
- Complete from different compatible finality witnesses and reject a conflicting canonical
  inclusion/result.

### Activation, restore, and upgrade attacks

- Prove no current feature, library, application, binary, or maintenance target can parse retired
  bytes; the old database remains opaque audit material.
- Reject a wrong activation contract/registry lineage, permanent domain binding, store lineage,
  initial/current store incarnation, physical-target key binding, writer epoch, issuer namespace,
  chain/domain/sender, replay-exclusion disposition, prior-effect/resource terminal disposition,
  finalized block/nonce floor, sender inventory, or stale fence.
- Race different store lineages for one unclaimed domain through the registry admin CAS; prove
  exactly one permanent domain/activation binding, globally unique activation-record identity,
  exact composite-proof replay resolution, permanent conflict for a different binding, and a
  shared lineage-key current-incarnation head for multiple domains.
- Race two new domains proposing sibling initial targets for one lineage, race issuance against
  promotion, crash between every internal registry statement, and lose acknowledgement; prove one
  lineage serialization point, all-or-nothing lineage-head/domain-row visibility, no partial
  binding, and resolution of the original composite proof over both keys.
- Crash after registry issuance and before local first reservation; prove the domain remains pinned
  to that lineage and only exact same-lineage retry or qualified promotion can proceed.
- Before that first local reservation, omit the permanent domain-binding proof or present a proof
  for another lineage and prove status cannot return `Absent`.
- Race concurrent virgin first reservations after issuance; prove only one exact local activation
  record and reservation commit, no local binding on a no-mutation response, fresh pending equality
  with the finalized floor, and rejection of a conflicting activation record both before and after
  the lock; accept fresh independently valid registry/producer evidence without changing the
  permanent record or original proof.
- Give two physical targets the same copied database, store-lineage ID, epoch, configuration,
  public registry/incarnation proof, client credential, and static attestation. Prove only the
  target holding the non-exportable key and live external-fence session can return semantic status
  or execute reserve, activate, and complete.
- Prove every authoritative status read actively validates the target-bound live session, every
  mutation uses a distinct non-replayable permit bound to the exact database session/transaction,
  a stale Read settles through reviewed `SafeFailure` and never `Refreshable`, stale Effect
  capabilities produce only the RFC disposition, and no normal operation calls or requires
  availability of the activation registry.
- Prove a reused sender requires definite prior-effect terminality, exhaustive fences, and replay
  exclusion or forces a new sender/domain; prove no bootstrap/high-water-import path exists.
- Race old/new target reads and mutations and crash at every promotion cut point: before fencing,
  after old-target/sender-path admission is revoked, after old transactions drain, after the final
  prefix is captured, after replacement hydration, after lineage-head CAS, and before replacement
  opening. Commit between a premature snapshot and the fence and prove that snapshot is rejected;
  accepted fence and prefix proofs must bind the same post-quiescence final old head and include
  every domain on the lineage.
- Restore/promote the nonce store and reject registry rollback, sibling registry/store writers,
  stale epoch, old-target re-entry, or any omission/rewrite/rebinding of an immutable historical
  activation binding, incarnation, target-key record, reservation, candidate activation,
  completion, active marker, high-water mark, permanent operation result, or object-closure
  member. Permit a different current incarnation/target key only as the newly appended certified
  promotion successor that retains the entire immutable prefix.
- Roll back the external-fence lineage, start a sibling fence issuer, replay a consumed transaction
  permit, and attempt to resurrect a stale target session after promotion; each must fail closed
  even when the old physical target still holds its private key.
- Prove a crash before promotion CAS keeps no replacement writable, a crash after old fencing is
  unavailable-safe and exactly retryable, a crash after CAS never reopens the old target, and any
  unprovable fence/prefix/hydration forces a new sender/domain.
- Reject incompatible activation with an incomplete intent; qualify exact-compatible old-release
  completion, preserve complete current-schema history for compatible upgrades, and require the
  new-sender escape for incompatible schema or issuer changes.

### Security and operational evidence

- Use canary credentials and provider diagnostics to prove secrets never reach programs, history,
  objects, facts, errors, DTOs, traces, exports, or logs.
- Fail, block, duplicate, and reorder every best-effort log/metric/span sink and prove no semantic
  record, cursor, outcome, retry, or run availability changes.
- Prove RunHistory, nonce, activation-registry admin, and activation-registry public roles cannot
  cross-write; prove registry-admin and external-fence-issuer credentials cannot escape deployment
  maintenance, and no generic pool escapes assembly.
- Inventory every removed predicate and assign its target owner or deletion.
- Run the zero-live-reference scan, SQL-name scan, workspace count, and LOC report as testable
  release evidence.
- Update qualification docs and corpora with the exact evidence and no stale executor claims.

### Commit 4 final verification

Use focused commands to diagnose failures, then run once on the final revision:

```text
nix run .#ci
```

Do not immediately precede that run with `nix run .#check`, `nix run .#test`, and
`nix run .#test-db`; `.#ci` already composes them. Also run:

```text
git diff --check
```

Review changed documentation links, examples, command claims, the negative-name scans, final LOC
report, and final four-commit sequence.

## RFC Verification Crosswalk

Every bullet under the RFC’s `Implementation Acceptance Verification` heading is mandatory. This
table allocates them; it does not replace or weaken them.

| RFC verification area | Commit 3 owner | Commit 4 owner |
| --- | --- | --- |
| Production-operation coverage and no arbitrary DAG need | Compile all three operations and their core fixtures | Re-run under registry/order variation and hostile bounds |
| Child substitution, exact slot contracts, sealed structural references, nominal outcomes, and root closure | Type/compile-fail tests plus fold happy paths | Cross-kind/source/role/contract/content substitution, including byte-identical distinct-contract payloads, and admission-only closure attacks |
| Canonical `CertifiedProgram` authority | Construct and admit one exact tagged component closure under the qualified entry-point predicate set | Cycle/order/dedup attacks plus independently valid component, predicate-policy, proof, and audit-projection substitutions |
| Pure expansion, wrappers, policy coverage, and bounds | Deterministic examples and exact profile tests | Property tests, permutation tests, and bound exhaustion |
| `Never`, fallibility, handlers, exact default payload, mapping chains, causal supersession, and later-run independence | Positive/local-negative contracts plus definite failure closure followed by a new `run_id` | Same-type/different-contract, payload, cross-plan, region, and stranded-run hostile cases |
| Match, fan-out, declaration order, and actionable frontier | Sequential/depth-two conformance | All completion orders, crashes, and concurrent observations |
| Five-family append, exact-head fold, and atomic closure | Memory/PostgreSQL conformance | Payload/object mutation and append races |
| Read/Effect bracket, ambiguity, refresh, and integrity | Core state-machine tests | Crash at every boundary and stale-worker races |
| Physical-binding qualification and invoker totalization | Sealed authority/compile-fail tests plus complete invoker inventory | Wrong-lineage verifier attacks and lower-level call-count/fault injection |
| Pending-observation persistence | Exact-content, stale-head, and acknowledgement-order tests | Lost-ack/rebase races, store unavailability, and no-reinvocation attacks |
| Ordinary Read fact selection and replay | Core scanner/replay tests | Frontier completeness, authority isolation, and old-byte attacks |
| Pending allocation, stable intent, and reservation | Canonical/unit/basic concurrency tests | Provider/race/lost-ack/foreign-intent matrix |
| Candidate family, signer, activation, and broadcast | Core bounded expansion and transaction tests | Cross-process, ordinal, witness, hash, and bearer attacks |
| Status, terminal equality, completion, and reconciliation | Core object-closure and projection tests | Torn/racing status, older winner, and witness-equivalence matrix |
| Qualified chain-instance registry | Core immutable-inventory proof and assembly tests | Fork clone, alias, namespace-reuse, and dual-membership matrix |
| Permanent wallet-domain/store-lineage registration | Core lineage-serialized composite issuance, two-key exact resolution, and promotion-exclusion tests | Competing domain/lineage/initial-target, internal-statement crash, lost-ack, issuance/promotion race, and registry rollback/sibling-writer matrix |
| Physical store incarnation and target-bound fencing | Core permanent-domain-proof/live-session status gate, per-transaction mutation permit, copied-target, offline-verifier, and zero-registry-IO tests | Stale four-operation, clone, fence-authority rollback/sibling/replay, old/new race, promotion cut-point, final-head/prefix, and credential-escape matrix |
| Domain activation, virgin pending, replay exclusion, and old-byte rejection | First-successful-reservation binding and local rejection tests | Hostile activation, concurrent-first-use, no-reader, restore, and upgrade drills |
| Failure/redaction/public projection | Closed mapper and DTO tests | Canary-secret and diagnostic-redaction matrix |
| Best-effort telemetry | Observer isolation and sink-failure smoke tests | Blocking/duplicate/reordered sink fault matrix with invariant run outcomes |
| Ownership/deletion/no executor | Dependency and compile boundaries plus deletion | Negative-name, role-crosswrite, predicate, and LOC evidence |

The commit 4 review must check off every RFC bullet against one row and cite its exact test or
qualification evidence.

## Stop And Amend Conditions

Stop implementation and amend the RFC and this plan before proceeding if any current requirement
needs:

- runtime-data-dependent fan-out, fan-out deeper than two, or an Effect anywhere inside fan-out;
- non-local success, arbitrary jumps, cycles, or graph scheduling;
- automatic same-occurrence recovery after possible target entry;
- EVM/nonce inspection by generic Runtime, journal, store, or replay;
- chain-instance identity issued or mutated by an RPC provider, Runtime, or normal request path
  instead of the qualified deployment inventory;
- a generic executor/outbox/lifecycle abstraction;
- cross-authority database transactions, roles, pools, or locks;
- wallet-domain issuance that can expose a domain row and lineage head separately or does not
  serialize with promotion on the lineage key;
- activation-registry IO or availability in ordinary status/reserve/activate/complete execution;
- a copyable/static target credential, a read snapshot not atomically gated by the target session,
  or a mutation permit not bound to its exact database transaction;
- same-lineage promotion before irreversible old-target/sender-path fencing, old-transaction
  quiescence, and capture of the complete final head for every domain on that lineage;
- a second persisted reader/writer or compatibility schema;
- a same-sender incompatible persisted-schema or issuer-namespace cutover;
- widening or changing the frozen EVM failure sum; or
- missing the LOC targets without the required architect re-review.

Exact generated schema hashes, canonical sentinel bytes, and stable key byte encodings remain
implementation outputs constrained by the RFC. Producing them does not reopen architecture.

## Reproducible Baseline Commands

Run from repository root:

```bash
git ls-files '*.rs' | xargs wc -l | tail -n 1

LC_ALL=C git ls-files '*.rs' \
  | awk '$0 ~ /\/src\// && $0 !~ /(_tests|\/tests|test_support)/ { print }' \
  | xargs wc -l \
  | tail -n 1

LC_ALL=C git ls-files '*.rs' \
  | awk '$0 ~ /\/src\// && $0 !~ /(_tests|\/tests|test_support)/ { print }' \
  | rg '^(crates/(kernel/(program|program-derive|spec|certify|journal|runtime|store|replay|capabilities|values|executor)|storages/(executor-file|executor-postgres|postgres)|domains/(evm|portfolio)|live/evm|app)|bin/(cli|rest-api)|tests/qualified-run-support)/' \
  | xargs wc -l \
  | tail -n 1

LC_ALL=C git ls-files '*.sql' \
  | rg '^(crates/(storages/(executor-file|executor-postgres|postgres)|domains/evm|live/evm|app)|bin/(cli|rest-api)|tests/qualified-run-support)/' \
  | xargs wc -l \
  | tail -n 1

git ls-files \
  crates/kernel/executor \
  crates/storages/executor-file \
  crates/storages/executor-postgres \
  | xargs wc -l \
  | tail -n 1

sed -n '/^members = \[/,/^\]/p' Cargo.toml \
  | rg -c '^[[:space:]]+"'

git ls-files crates/kernel/runtime/src \
  | rg '\.rs$' \
  | xargs wc -l \
  | tail -n 1

git ls-files crates/kernel/spec/src crates/kernel/certify/src \
  | rg '\.rs$' \
  | xargs wc -l \
  | tail -n 1

git ls-files \
  crates/kernel/certify/src/planner.rs \
  crates/kernel/certify/src/terminal.rs \
  crates/kernel/runtime/src/decision.rs \
  crates/kernel/runtime/src/materialization.rs \
  crates/kernel/spec/src/certified_node.rs \
  crates/kernel/spec/src/certified_node_tests.rs \
  | xargs wc -l \
  | tail -n 1

git ls-files \
  crates/live/evm/src/wallet_executor.rs \
  crates/live/evm/src/wallet_executor_tests.rs \
  | xargs wc -l \
  | tail -n 1

git ls-files crates/domains/evm/src/wallet_state.rs \
  | xargs wc -l \
  | tail -n 1

git ls-files \
  crates/app/tests/ui/pass/evm_wallet_executor_uses_qualified_transport.rs \
  | xargs wc -l \
  | tail -n 1

git ls-files tests/integration/tests/evm_audited_graph.rs \
  | xargs wc -l \
  | tail -n 1

git ls-files crates/storages/evm-postgres \
  | awk '$0 ~ /\/src\/.*\.rs$/ && $0 !~ /(_tests|\/tests|test_support)/ \
      || $0 ~ /\/migrations\/.*\.sql$/ { print }' \
  | xargs wc -l \
  | tail -n 1
```

After the cutover, replace the deleted executor paths in the affected-scope expression with
`crates/storages/evm-postgres`, while retaining the deleted paths as zero-line members of the
baseline comparison. The new-wallet-store command counts production-like tracked Rust under its
`src` tree plus tracked production migration SQL; this is the exact 4,000-line target scope.
Record both gross deletion and net added/deleted production-like lines.

The `runtime-history-choke-point-contract` leaf is the executable authority for negative-name,
SQL-name, workspace, and LOC enforcement. Its report must include all baseline and final values,
gross mandatory deletion, net affected-scope deletion, every reviewed negative-fixture exception,
and the largest surviving production modules. Its mandatory-path inventory includes every row in
the deletion table, including `tests/integration/tests/evm_audited_graph.rs`. The ad hoc commands
above are reproducibility aids, not a substitute for that committed gate.

## Definition Of Done

The implementation is done only when:

- commits 1–4 exist in the declared order on `refact-recov`;
- commit 3 is the sole complete code/schema/docs cutover and commit 4 is qualification hardening;
- all three production operations compile and execute through the structured program;
- Runtime is a cursor interpreter and the store is the sole callback-free append validator;
- the five run-history families and separate wallet-nonce authority satisfy the RFC;
- permanent domain/lineage registration, target-bound physical fencing, offline normal access, and
  post-quiescence promotion pass their qualification matrix;
- old persisted bytes are rejected and no compatibility or executor path remains;
- every mandatory deletion and negative scan passes;
- workspace and LOC gates pass;
- every RFC acceptance bullet has cited evidence;
- `nix run .#ci` passes once on the final revision; and
- `git diff --check` is clean.
