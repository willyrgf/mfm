# Implementation plan: establish the Runtime, Journal, and Store proof path

Status: approved implementation handoff for
[`RFC_RUNTIME_STORE_JOURNAL_TYPED_PROOF.md`](RFC_RUNTIME_STORE_JOURNAL_TYPED_PROOF.md)

Audience: engineer-agents and architect reviewers implementing the complete MFM cutover

---

## 0. Authority, mandate, and stopping rule

The RFC is the approved platform architecture. This plan fixes its implementation order, package
ownership, public Rust surfaces, deletion ownership, review gates, tests, and commit boundaries.
The RFC owns semantics and persisted contracts; this plan owns execution of that decision.

The current code, migrations, fixtures, tests, task graph, and current-design documentation describe
the implementation being replaced wherever they conflict with the RFC. The following are not
implementation inputs:

- [`RFC_REFACTOR_SINGLE_TRUST_BOUNDARY.md`](RFC_REFACTOR_SINGLE_TRUST_BOUNDARY.md),
  [`IMPL_PLAN_RFC_REFACTOR_SINGLE_TRUST_BOUNDARY.md`](IMPL_PLAN_RFC_REFACTOR_SINGLE_TRUST_BOUNDARY.md),
  [`REQUIRED_FIXES_FOR_RFC.md`](REQUIRED_FIXES_FOR_RFC.md), and
  [`FINISH_EVM_PORTF_REFACTOR.md`](FINISH_EVM_PORTF_REFACTOR.md) are archival;
- the archival sections of
  [`RFC_FOLLOWUPS_FROM_REFACT_RUNTIME.md`](RFC_FOLLOWUPS_FROM_REFACT_RUNTIME.md) are not active
  contracts; and
- the active part of that follow-up is only a future rebase source. It does not reopen this core
  Program wire, control-index contract, Pure/Read execution algebra, or the retirement of EVM
  submission.

Implementation may break every Rust API, persisted format, SQL baseline, fixture, CLI/REST schema,
and documented behavior in scope. There is no retained-data compatibility claim. Do not add a
legacy decoder, migration from the superseded schema, compatibility type alias, deprecated export,
dual write, fallback, feature-gated old path, or temporary public bridge. Git history is the
archive.

If implementation discovers that an RFC invariant is impossible, unsafe, or materially more
complex than specified, stop that chunk. Do not improvise another architecture. Record the concrete
counterexample, ask a dedicated architect for the smallest complete target, amend the RFC and this
plan, and obtain approval before continuing.

### Material uncertainties

none. Product ownership approved retiring `mfm.evm/submit-transaction@1` and generic Effect. The
Rust ownership, wire formats, Store/Runtime boundary, PostgreSQL durability contract, fixed limits,
and two logical commits are frozen by the RFC.

Routine private implementation choices remain with the engineer only when they do not add a public
type, registry, persisted field, error variant, dependency edge, lifecycle, or second owner.

## 1. Completion rule and complexity budget

The work is complete only when all of the following are true on one tree:

1. both logical commits below exist with the exact lower-case subjects and each commit is coherent;
2. all RFC acceptance criteria pass;
3. every concept in RFC Section 11 is absent from supported source, exports, schemas, fixtures,
   manifests, tasks, and current-design documentation, except where the RFC explicitly transfers
   the responsibility to one new owner;
4. the replacement absence manifest checks symbols, reexports, stable/schema identities, wire
   fields/tags, SQL objects, dependency edges, fixtures, and documentation claims—not just spelling;
5. every code chunk has a blocking architect approval under Section 3 of this plan;
6. Runtime is the sole semantic reducer, Journal the sole run-wire qualifier, and Store purely
   mechanical;
7. no intermediate compatibility surface survives the final tree;
8. the final public API, dependency-edge, tracked source-tree LOC, and State-change-site counts are lower than
   the baseline; and
9. scope-selected checks and the one final composed CI gate pass as required by
   [`docs/build-and-verification.md`](docs/build-and-verification.md).

Deletion is a design result, not optional cleanup. A test that passes while an obsolete public
wrapper, branch, migration, or alternate reducer remains does not satisfy this plan.

Measure the parent of Commit 1, the Commit 1 boundary, and the final tree with the identical
reported commands in Section 9, counting:

- workspace-wide top-level public declarations/reexports in every library crate, backed by a
  reviewed crate-root export ledger; additionally retain the ten proof-path crate count as a
  diagnostic for `mfm-ids`, `mfm-values`, `mfm-capabilities`, `mfm-program`, `mfm-journal`,
  `mfm-store`, `mfm-runtime`, `mfm-evm`, `mfm-evm-live`, and `mfm-app`;
- non-dev inter-crate dependency edges from Cargo metadata;
- tracked crate/bin `src` Rust LOC, including inline tests but excluding external tests and
  generated build output; and
- source files an engineer must edit to add one Pure State and one Read State with an existing
  capability/binding.

The workspace-wide public count, dependency edges, tracked source-tree LOC, and change-site count
must decrease; the ten-crate diagnostic must decrease too. Do not meet the LOC objective by
removing validation, typed errors, contract tests, rustdoc, or readability. Remove duplicate
owners, generic extension points without consumers, wrappers, and obsolete branches.

## 2. Final dependency and ownership shape

The table below freezes normal internal MFM dependency edges. External crates such as serde,
thiserror, SQLx, and Tokio are listed in their owning chunks and do not loosen these edges.

| Consumer | May depend on |
| --- | --- |
| IDs | no internal crate |
| Canonical | IDs |
| Values | Canonical and IDs |
| Program derive | Canonical; IDs/Values dev-only for consuming tests |
| Capabilities | IDs and Values |
| Program | Canonical, Capabilities, IDs, and Values |
| Journal | Canonical, IDs, and Values |
| Store/Memory | IDs and Journal; no Canonical |
| PostgreSQL Store | Canonical, IDs, Journal, and Store |
| Runtime | Canonical, Capabilities, IDs, Journal, Program, Store, and Values |
| EVM domain | Capabilities, IDs, Program, Program derive, and Values |
| Portfolio domain | EVM, IDs, Program, Program derive, and Values |
| EVM live adapter | EVM, IDs, and Runtime |
| App | IDs, Runtime, EVM, and Portfolio; never live EVM |
| Signing | IDs |
| Keystore | IDs |
| CLI/REST | no internal edge while they remain metadata-only |

Trusted process embedding—not another workspace architecture crate—constructs live adapters and
Runtime, then supplies the finished Runtime to App.

Exact ownership:

| Owner | Only retained responsibility |
| --- | --- |
| `mfm-values` | `MfmValue`, schema/canonical qualification primitives, and the shared 8 MiB run-object ceiling |
| `mfm-capabilities` | the Read capability identity and pure intent/evidence binding relation |
| `mfm-program` | checked Program, State behavior traits, root contracts, State/Match graph, and exact associations |
| `mfm-journal` | sealed frame construction, opaque retained-byte transfer, structural history qualification, wire/hashes/limits |
| `mfm-store` | object-safe mechanical Store trait and ephemeral Memory implementation |
| `mfm-storage-postgres` | one checked production constructor, physical readiness, snapshot load, exact-head append, static SQL baseline |
| `mfm-runtime` | finalized assembly, the only semantic fold, typed Pure/Read execution, and `RunView` |
| domains | deterministic Program/C0 authoring and reusable State semantics; no ambient IO |
| live adapters | bind exact typed Read intent to an explicit target/client and return typed evidence or `ReadAdapterError` |
| App/transports | parse, select an entry point, require explicit `RunId`, invoke Runtime, and render redacted results |

Forbidden edges and duplicate owners include:

- Store to Program, capabilities, facts, configuration, Runtime selection, or domain types;
- Journal to Runtime or Store semantics;
- a domain crate to Runtime, Journal, Store, or a live adapter;
- Runtime to App or a transport;
- App-owned frame decoding, reduction, sessions, pending work, or replay;
- a second Program document/builder/catalog lifecycle; and
- `mfm-facts`, `mfm-replay`, or `mfm-storage-evm-postgres` in the final workspace.

## 3. Mandatory architect review protocol

Every chunk named in Sections 5 and 6 requires one named architect reviewer who did not implement
that chunk. The implementer may not self-approve. Several chunks may be developed concurrently, but
none is integration-complete until its own review is `APPROVE`.

The reviewer receives:

1. this plan, the approved RFC, `AGENTS.md`, `docs/design.md`, `docs/architecture.md`, and
   `docs/code-quality.md`;
2. the path-scoped diff and the cumulative diff on which it depends;
3. focused test/compile evidence or an explicit explanation that the inseparable core cutover has
   not yet reached a compilable integration point;
4. before/after public exports, dependency edges, tracked source-tree LOC, and ownership/change-site
   notes relevant to the chunk; and
5. the chunk's deletion checklist with source-search evidence.

The architect must review for all of these objectives:

- semantic correctness, hostile-input behavior, race behavior, durability, and cold/hot agreement;
- one clear owner per invariant and no second reducer, qualifier, registry, DTO, or lifecycle;
- Rust API feasibility, private-field construction authority, lifetimes, object safety, and typed,
  redaction-safe errors;
- the smallest number of concepts, public types, code paths, dependency edges, future change sites,
  and tracked source-tree LOC;
- complete deletion with no alias, fallback, old decoder, renamed equivalent, or speculative
  extension point;
- absence of unnecessary cloning, allocation, reload/refold, blocking-runtime work, or detached
  authority; and
- tests at the owning boundary rather than duplicate end-to-end assertions for local invariants.

The review response has this exact decision shape:

```text
Decision: APPROVE | BLOCK

Correctness findings:
- ...

Simplicity and LOC findings:
- concepts/public types/branches/change sites deleted
- any smaller complete implementation the engineer must use

Dexterity findings:
- files/registrations/change sites required for one new Pure State and one Read State
- whether a domain/schema/binding change remains local to its owning layer

Deletion evidence:
- required names/fields/tables/dependencies absent

Verification evidence reviewed:
- ...

Material uncertainties:
none | exact unresolved item, consequence, and resolution
```

`BLOCK` is mandatory for an unresolved correctness issue, a second owner/path, a compatibility
surface, a needless public abstraction, a material uncertainty, or failure to delete the chunk's
old surface. Tests passing does not override a block. The engineer fixes the same design, reruns the
narrow checks, and obtains a new approval before integration. Review outcomes belong in the PR/task
handoff; do not add a permanent progress log or approval database to the repository.

For Commit 2, reviewers inspect path-scoped chunks in one accumulating worktree. These chunks are
not separately published commits and may temporarily cross a compiler seam. After every chunk is
approved, a final architect who did not implement the integration performs a cross-chunk review for
duplicate ownership, stale exports, dependency cycles, and unnecessary LOC before the one core
commit is created.

## 4. Logical commit graph

There are exactly two implementation commits:

```text
1. remove unsafe evm transaction submission
   |
   v
2. establish the typed runtime journal and store proof path
```

Commit 1 is independently buildable and removes the unsafe product surface before core refactoring.
The current generic Effect implementation may still compile in this commit but has no production
registration or consumer. It is not blessed as an extension point and is deleted in Commit 2.

Commit 2 is one inseparable workspace-wide cutover. The current Store reducer and exported
`SuspendedRun`/preparation/pending-conclusion lifecycle cannot coexist coherently with the target
Runtime reducer and fused Read semantics. Do not create intermediate commits with zero reducers,
two reducers, dual wire formats, adapter shims, or old/new Store traits. Use the reviewed chunks in
Section 6 as work and review boundaries, then create the one commit only after the integrated tree
is coherent.

## 5. Commit 1 — `remove unsafe evm transaction submission`

### 5.1 Result at the commit boundary

The repository exposes Portfolio snapshot planning/execution and its observational EVM Reads only.
`mfm.evm/submit-transaction@1`, nonce reservation, signing/broadcast composition, and submission
status are absent. The generic core Effect types still exist only until Commit 2 and have no
production consumer or registration in App/live composition.

Standalone `mfm-signing` and `mfm-keystore` remain reusable and unchanged unless removing their
now-dead dependency edges requires documentation/manifest cleanup. Do not weaken their existing
security contracts. Rewrite the stale `mfm-signing` crate/type rustdoc that says a Runtime binding
retains `PublicSignerKeyInstance`; describe it only as a reusable platform signing identity after
the sole production binding consumer is deleted.

### 5.2 Chunk 1A — EVM domain product deletion

Primary paths:

- `crates/domains/evm/src/lib.rs`
- `crates/domains/evm/tests/{unit,balance_matrix}.rs`
- `crates/domains/evm/{Cargo.toml,README.md}`
- `crates/domains/portfolio/{src,tests,Cargo.toml,README.md}` for mechanical submission deletion
  fallout and surviving-planner preservation

Delete, rather than deprecate or leave unregistered:

- `EVM_SUBMIT_TRANSACTION_ENTRY_POINT_ID` and every `mfm.evm/submit-transaction@1` identity;
- `EvmConfig`, `EvmSubmissionRoute`, `EvmTransactionTarget`, `EvmSubmissionSelector`,
  `EvmSubmissionRequest`, `EvmSubmissionProgress`, `EvmSubmissionOutput`,
  `EvmSubmissionFailure`, `EvmSubmissionBindings`, `EvmSubmissionPlan`, `plan_submission`, and
  `submission_closure_documents`;
- every nonce-reservation, transaction-candidate, broadcast, receipt/finalization/canonicality, and
  submission failure State/contract/identity;
- `NonceReservationIntent`, `NonceReservationEvidence`, `BroadcastIntent`, and
  `BroadcastEvidence`;
- the `EvmCapability<0>`, `EvmCapability<1>`, and submission-status `EvmCapability<3>`
  implementations and stable/schema ID branches;
- `EvmReadSubject::{TransactionReceipt, FinalizedHead, CanonicalInclusionBlock}` and
  `EvmReadValue::{Receipt, FinalizedHead, CanonicalBlock}` plus their codecs/interpreters;
- `ReadCapabilityFamily::SubmissionStatus`; and
- submission-only tests, helpers, fixtures, error branches, and capacity assertions.

The Commit 1 absence inventory names these exact retired identity families in addition to their
Rust owners:

```text
mfm.evm/submit-transaction@1
mfm.evm.capability.reserve-wallet-nonce@1
mfm.evm.capability.broadcast-transaction@1
mfm.evm.capability.read-submission-status@1
mfm.evm.state.reserve-wallet-nonce@1
mfm.evm.state.derive-candidate@1
mfm.evm.state.broadcast-transaction@1
mfm.evm.state.read-transaction-receipt@1
mfm.evm.state.read-finalized-head@1
mfm.evm.state.read-canonical-inclusion-block@1
mfm.evm.state.consolidate-execution-disposition@1
mfm.evm.read-transaction-receipt@1
mfm.evm.read-finalized-head@1
mfm.evm.read-canonical-inclusion-block@1
mfm.evm.broadcast-transaction@1
mfm.evm.candidate/
mfm.evm.nonce/
mfm.evm.nonce-domain/
mfm_evm_nonce_schema
mfm_evm_nonce_domains
mfm_evm_nonce_operations
```

Retain the chain-identity/anchor/balance Read capability families used by Portfolio (currently K2,
K6, and K7), the balance State graph, `EvmReadEvidence::{Returned,Rejected,SafeFailure,
IntegrityBlocked}`, and the typed `EvmBalanceFailure` mapping behavior.

Deleting `EvmSubmissionProgress` also removes the public default type parameter on `EvmState`.
Retain the current numeric semantic identities in this core RFC and make the surviving shape
explicit:

```rust
pub struct EvmState<const FAMILY: u8, const STAGE: u8, K: MfmValue>(...);
```

Supply the concrete surviving balance context at the few identity-derivation call sites. Do not
renumber surviving stable identities or import the follow-up RFC's semantic-type rename into this
cutover.

After status Reads are deleted, every surviving `EvmReadIntent` is route-bound. Make `route_ref` a
required `ContentRef`, collapse the status/balance constructor split to one checked private
constructor, and retain only `route_ref(&self) -> &ContentRef`. Do not preserve an `Option` or
status-only constructor for a removed case.

Refactor the current private `access_state` authoring helper to Read-only in this commit: constrain
the current capability mode to `ReadMode`, always author `ExecutionMode::Read`, and remove its
Effect branch/import. Generic Effect may remain temporarily inside the kernel, but no EVM/domain
helper may still author it.

Preserve the surviving Portfolio planner behavior at this deletion boundary except for mechanical
changes required by submission removal. Do not implement the known multi-source graph correction
against the address-based Program that Commit 2 deletes; Chunk 2G owns the single implementation
against final implicit indices. Commit 1 remains a coherent product-deletion commit without adding
or worsening that pre-existing planner defect.

Tests for this chunk must prove:

- surviving balance/anchor/chain Program fixtures are byte-for-byte deterministic for this commit;
- no surviving Read intent/value can represent transaction-submission status;
- the submission entry-point and every retired stable/schema identity are absent; and
- Portfolio's child EVM failure still reaches its mapper.

Update the forged-non-prefix balance-context regression from a fully valid baseline containing the
now-required route ref, mutate only the work item, and assert that intended invariant. A fixture
missing route_ref must not make the regression pass for the wrong reason.

Architect gate `C1-A`: domain architect review for semantic completeness, stable-identity deletion,
preservation of the surviving Portfolio planner, and a smaller surviving EVM type graph.

### 5.3 Chunk 1B — live EVM and nonce-store deletion

Primary paths:

- `crates/live/evm/{src/lib.rs,tests/adapter_closure.rs,Cargo.toml,README.md}`
- `crates/storages/evm-postgres/**`
- root `Cargo.toml` and `Cargo.lock`

Delete:

- `wallet_nonce_effect_domain`, `public_signer_key_instance_ref`, and every submission-only live
  identity/helper;
- `WalletNonceAuthority`, `EvmAdapterHandle::{ReserveNonce,Broadcast}`, signer/broadcast handles,
  nonce Store fields, and their registration paths;
- `EvmProviderResponse::{Broadcast,PossibleEntry}` and submission-only `EvmAdapterError` branches;
- `PostgresWalletNonceAuthority`, `PostgresWalletNonceStore`, `WalletNonceDomain`,
  `WalletAuthorityError`, their public `Result`, all nonce activation/reservation/operation code,
  the `crates/storages/evm-postgres` crate, migration, build script, tests, features, and README;
- the `mfm-storage-evm-postgres` workspace member/dependency and now-unused live dependencies on
  signing/nonce storage; and
- every test fake or fixture that can allocate a nonce or broadcast.

Keep the current live Read adapter path working for the surviving Portfolio capabilities until the
Commit 2 direct-`EvmPhysicalTarget`/borrowed-intent cutover. `EvmProviderResponse::{Read,Rejected,
SafeFailure,IntegrityBlocked}` remains. Collapse one-variant `EvmAdapterHandle::Read` into a direct
`provider: Arc<dyn EvmProvider>` field and delete `EvmAdapterError::Unresolved`, which has no
surviving Read use. The interim live surface is only the existing-core-compatible
`EvmLiveAssembly::install(builder, balance_bindings, adapters)` plus read-only planning bindings;
Commit 2 replaces that wrapper completely. At this boundary any generic Effect code is core-only
and has no live construction path.

Tests for this chunk must prove:

- the live assembly has no signer, nonce authority, broadcast, or status capability registration;
- each surviving provider response maps to the same typed EVM Read evidence as before;
- Cargo metadata contains no `mfm-storage-evm-postgres` package or dependency edge; and
- keystore/signing tests remain unchanged or pass if their manifests were touched.

Architect gate `C1-B`: live-IO/storage architect review for complete authority deletion, no hidden
submission handle, and no weakening of standalone signing/keystore.

### 5.4 Chunk 1C — App, product contract, task, and documentation deletion

Primary paths:

- `crates/app/{src/lib.rs,tests,Cargo.toml,README.md}`
- `README.md`, affected domain/live/bin READMEs, and current product docs
- `docs/contracts/evm-portfolio/`
- `docs/evm-transactions.md`
- `nixfied.nix`, root manifests, and affected workflows/scripts

Delete App fields, constructors, dispatch arms, assembly contributions, request/response handling,
and tests that select or drive EVM submission. An invocation using the retired entry-point ID must
fail as an unknown/unsupported entry point before Runtime work. Do not keep a disabled route or a
special `SubmissionRetired` compatibility response.

Remove App's submission-test-only `mfm-signing` dev-dependency as well as every nonce/signer fake;
the standalone signing/keystore crates remain, but App has no reason to name either.

The interim `Application` constructor loses EvmConfig/config-head/submission arguments and fields.
Its temporary catalog registers only the Portfolio snapshot root (use `PortfolioSnapshotInput` for
the old zero-State placeholder) and its live composition accepts only balance Read bindings.

Delete these six fixtures:

```text
evm-submission-destination-rejected.json
evm-submission-failure.json
evm-submission-nonce-lineage-diverged.json
evm-submission-provider-unavailable.json
evm-submission-reverted.json
evm-submission-succeeded.json
```

Delete `docs/evm-transactions.md`. Rewrite product inventories, known gaps, EVM/Portfolio contract
freeze text, capacity-app coverage, and integration tests so they advertise Portfolio only and say
that submission requires a future durable transaction-authority/outbox RFC. Do not design that
authority in this commit.

The exact current-authority paths requiring Commit 1 edits are `README.md`,
`docs/{design,architecture,run-execution,build-and-verification,evm-portfolio-contract-freeze,
evm-rpc-routing,known-gaps}.md`, `crates/{app,domains/evm,live/evm}/README.md`, and both binary
READMEs. Remove EVM-config/plural-entry-point and nonce/signer/submission composition claims while
leaving generic old-core preparation/configuration prose for the Commit 2 rewrite.

The approved RFC assigns physical deletion of the three `docs/preflight/*` artifacts and
`docs/btc-rpc-routing.md` to Commit 2. At the Commit 1 boundary, prepend a short status to each
classifying it as superseded, retained only as a one-time pre-cutover artifact, and not current
product authority. Do not rewrite their stale two-entry-point/submission bodies merely to delete
them in the next commit.

The surviving App may still use the old core lifecycle until Commit 2. This chunk must not begin a
second App API cutover or leave an unused EvmConfig parameter.

Tests and checks:

- surviving Portfolio App integration passes;
- the retired entry-point ID is rejected and absent from public inventories;
- fixture/task discovery has no submission branch;
- category-scoped evidence finds no retired production export/registration, stable/schema identity,
  SQL object, task, fixture, package, or dependency; only the explicitly allowed negative test,
  retirement prose, active RFC/plan, and status-marked pre-cutover archives may name retired input;
  and
- because a workspace package and dependency graph are removed, run focused affected-package tests
  while iterating and `nix run .#ci` once on the final Commit 1 tree.

Update the current cutover scanner only enough to exclude this active implementation plan from its
old-manifest scan, matching its existing exclusion of implementation plans. Do not otherwise
rewrite the scanner/manifest until Commit 2.

Add `pkgs.jq` and `pkgs.ripgrep` to `flake.nix::mkDevTools` in this commit so every deletion and
complexity measurement uses pinned tools. To recheck the frozen parent baseline, invoke the Commit
1 dev shell while the command's working directory is a separate parent-revision worktree; never
fall back to host `jq`/`rg`.

Correct the old manifest's status header only: it is superseded as design authority but remains the
temporary legacy negative-scan input through Commit 1 and will be replaced in Commit 2. Its old
patterns are not sufficient proof of submission deletion, so run the explicit retired path/export/
stable-ID/schema/SQL/dependency scans named above as separate Commit 1 evidence.

The exact retired entry-point literal may remain only in its negative App regression and reviewed
retirement prose. Absence checks use narrow path/category allowlists and prove there is no production
export, registration, schema/wire, fixture, task, or dependency occurrence; do not use a blanket
literal ban that would delete the regression.

Architect gate `C1-C`: product/App architect review for one surviving entry-point inventory, no
compatibility response, and less composition code/LOC.

### 5.5 Commit 1 integration gate

After `C1-A` through `C1-C` approve, assign a fourth architect to inspect the combined diff. The
architect must verify that no code path can reserve a nonce, request signing for submission,
broadcast, or poll submission status, while Portfolio Reads still work. Then create exactly:

```text
remove unsafe evm transaction submission
```

Record focused checks and the one final CI result in the handoff. Start Commit 2 only from this
coherent commit.

## 6. Commit 2 — `establish the typed runtime journal and store proof path`

### 6.0 How to execute the inseparable cutover

Prepare Chunks 2A through 2I in the order below unless an immediate producer/consumer must be
edited together to keep the worktree compilable. Do not preserve a superseded API merely to make an
intermediate chunk compile. A path may appear in more than one review slice only when ownership
actually crosses the boundary; the later reviewer must inspect the cumulative changes.

The final commit includes source, tests, migration reset, generated/checked fixtures, manifests,
Cargo.lock, Nix tasks, absence manifest, and current-design documentation. There is no follow-up
cleanup commit.

### 6.1 Chunk 2A — ids, values, derive, and Read capabilities

Primary paths:

- `crates/kernel/ids/{src,Cargo.toml,README.md}`
- `crates/kernel/values/{src,Cargo.toml,README.md}`
- `crates/kernel/program-derive/{src,Cargo.toml,README.md}`
- `crates/kernel/capabilities/{src,Cargo.toml,README.md}`

#### Retained public capability surface

Replace the current mode/facts/evidence-marker abstraction with exactly:

```rust
pub type Result<T> = std::result::Result<T, CapabilityError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CapabilityError {
    #[error("capability contract is invalid")]
    InvalidContract,
    #[error("capability evidence does not bind to intent")]
    EvidenceBinding,
}

pub trait ReadCapabilityContract: Send + Sync + 'static {
    type Intent: MfmValue;
    type Evidence: MfmValue;

    fn contract_id() -> Result<StableId>;
    fn bind_evidence(intent: &Self::Intent, evidence: &Self::Evidence) -> Result<()>;
}
```

`mfm-capabilities` must export only that trait, error, and alias. Delete its dependency on
`mfm-facts`; do not retain a prelude, marker-trait module, mode module, default retry policy, fact
hook, or outcome type.

Delete from capabilities and all reexports/tests:

- `AccessCapabilityContract`, `AccessMode`, `ReadMode`, `EffectMode`, `AccessEvidenceValue`,
  `FactSelectionMode`, `NoPriorFacts`, `PriorRunFacts`;
- `total_attempt_bound`, `prior_fact_selection`, `requires_prior_facts`, and mode validation;
- capability-owned `ProposedStateOutcome`, including fact fields; and
- `QualifiedRecordedEvidence`, fact marker/proposal types, and every associated dependency.

#### IDs and value/schema surface

Delete the unused identity families and control/authority identities:

```text
EffectKind / EffectKindKind / EffectVersion / EffectVersionKind
CapabilityKind / CapabilityKindKind / CapabilityVersion / CapabilityVersionKind
SequentialControlAddress
StoreScopeId / StoreEpoch / TenantScopeId
AppendRequestId
FieldSegment / FieldPath
SemanticDigest / FactContentIdentityDigest / FactLogicalIdentityDigest / FactQueryDigest
JournalCommitDigest / JournalRecordHash / RequestDigest / RunSemanticStateDigest
the semantic-identity/branded-digest macros left with no caller
StableAuthorKey / LocalPublicId / RuntimeEnvName / short_stable_id_fragment
```

Do not replace them with generic `StateAddress`, `ScopeId`, `IncarnationId`, or an untyped identity
bag. Retain strict `StableId`, `EntryPointId`, `RunId`, schema/content identities, and the digest
types required by the approved wire.

In `mfm-values`:

- retain one `MfmValue` path and the existing float-free canonical/schema machinery;
- export the single shared
  `pub const MAX_RUN_OBJECT_CANONICAL_BYTES: usize = 8_388_608;`;
- delete `MfmConfig`, `ValidatedConfig`, `PublicOutputDescriptor`, `StateInput`,
  `OperationOutput`, `PublicOutputs`, and their derives;
- delete `ConfigError`, `ValueError::Config`,
  `SchemaKind::{PlanningConfig,StateInput,OperationOutput,PublicOutput}`,
  `StringGrammar::{SemanticDigest,StoreScopeId,TenantScopeId}`,
  `CanonicalJsonProfile::UnsignedNative`, and the now-dead field-path support;
- keep generic foreign and nested empty enums invalid; and
- add only the exact root `mfm.kernel/never@1` SchemaIdentity exception frozen in RFC Section 3.1.

Use one checked value-instance helper for the only shared canonicalization/hash operation:

```rust
pub fn canonicalize_mfm_value<T: MfmValue>(
    value: &T,
) -> std::result::Result<(PlainCanonicalJsonBytes, ContentRef), ValueError>;
```

It proves descriptor/schema agreement, encodes exact no-float canonical JSON, applies the known
secret-marker policy, enforces `MAX_RUN_OBJECT_CANONICAL_BYTES`, and derives the instance
`ContentRef` from the exact schema plus SHA-256 of those bytes. Runtime immediately wraps this tuple
in its private typed qualification; `EvmPhysicalTarget::binding_ref` uses the returned ref; Journal
rechecks ref/bytes at frame construction. This is not a public proof wrapper or a second value
registry. Delete the weaker Program-owned `canonical_value` helper so no second serialization/hash
path remains.

Add one redaction-safe `ValueError::Capacity` variant. Only this shared helper returns it for the
8 MiB object ceiling; descriptor, schema, serialization, and shape failures retain their existing
non-capacity classifications. Runtime maps it to `RuntimeError::Capacity` for C0, intent, evidence,
and outcome qualification; a static adapter binding that exceeds it is
`IncompatibleAssembly`; `EvmPhysicalTarget::binding_ref` maps it into its existing domain-authoring
error. Program and Journal perform their own byte-limit checks and return their own `Capacity`
variants; they do not depend on this Value error. This one variant is smaller and more accurate
than a wrapper error or misclassifying a valid-but-oversized value as an internal schema fault.

The implementation obtains one descriptor, requires its semantic identity to equal
`T::semantic_id()` and its derived schema to equal `T::schema_id()`, encodes once, checks the 8 MiB
bound, validates those exact bytes against that descriptor, and builds the instance ref with
`ContentRef::new(schema_id, ContentDigest::from_digest(DigestAlgorithm::Sha256V1,
canonical.digest_bytes()))`. Do not call `PlainCanonicalJsonBytes::content_digest()`, which uses
the identity-oriented `Sha256JcsV1`, or expose a second helper/reexport.

Do not add `SchemaShape::Never`, a generic `allow_empty`, or encode Never as Unit. The exception is
for the one complete externally tagged, zero-variant root identity. All values fail validation
against it.

Keep descriptor provenance truthful by changing the hidden framework helper to accept the owning
crate explicitly; Program's Never registration must not claim it was defined by mfm-values:

```rust
#[doc(hidden)]
pub fn framework_value_descriptor(
    owner_crate: &str,
    semantic_type_id: SemanticTypeId,
    schema_name: &str,
    shape: SchemaShape,
    rust_type_path: &str,
) -> std::result::Result<SchemaDescriptor, ValueError>;
```

Update `mfm-program-derive` to remove the four obsolete derives (`MfmConfig`, `StateInput`,
`OperationOutput`, and `PublicOutputs`) and every parser/diagnostic branch used only by them. The
surviving derives are `MfmValue` and `PersistedSchema`. Move `mfm-ids` and `mfm-values` from regular
to dev dependencies after StateInput's macro-time use disappears; keep `mfm-canonical` regular.
Fix retained generated StableId and EntryPointId bounds to their actual 512-byte grammars. Add
consuming-crate tests for changed expansion.

#### Error ownership

Capability identity/ref derivation during checked Program authoring maps to
`ProgramError::InvalidContract`. The same static association failure during Runtime assembly maps
to `RuntimeError::IncompatibleAssembly`. A hot `bind_evidence` failure maps to `Internal` without an
append; the same failure over retained history maps to `InvalidHistory`. Do not expose raw
`CapabilityError` through Runtime or transport errors.

#### Tests and evidence

- exact identity and schema goldens for Never, including the three RFC digests and complete
  canonical SchemaIdentity bytes;
- every JSON value rejected against Never; foreign/nested empty enums still rejected;
- compile tests show only `MfmValue` and `PersistedSchema` derives remain;
- capability bind success/failure and invalid contract tests;
- public API tests prove no mode, facts, attempts, capability kind/version, configuration, or
  outcome wrapper remains; and
- Cargo metadata proves `mfm-capabilities` no longer depends on `mfm-facts`.

Architect gate `C2-A`: foundations architect. Require explicit confirmation that one small reserved
Never exception replaced—not generalized—the invalid empty-enum path, and that deleting identifiers
did not create an untyped replacement.

### 6.2 Chunk 2B — checked Program and State behavior

Primary paths:

- `crates/kernel/program/{src,tests,Cargo.toml,README.md}`
- domain Program fixtures affected by the wire reset

#### Public State behavior

Move the sole typed outcome and behavior contracts beside `State` in `mfm-program`:

```rust
pub trait State: Send + Sync + 'static {
    type Input: MfmValue;
    type Output: MfmValue;
    type Failure: MfmValue;

    fn state_id() -> Result<StableId>;
}

#[derive(Debug, PartialEq, Eq)]
pub enum ProposedStateOutcome<O, F> {
    Success { output: O },
    Failure { failure: F },
}

pub trait PureState: State {
    fn evaluate(
        input: Self::Input,
    ) -> ProposedStateOutcome<Self::Output, Self::Failure>;
}

pub trait ReadState<C>: State
where
    C: ReadCapabilityContract,
{
    fn prepare(
        input: &Self::Input,
    ) -> std::result::Result<C::Intent, ReadPreparationError>;

    fn interpret(
        input: Self::Input,
        evidence: &C::Evidence,
    ) -> ProposedStateOutcome<Self::Output, Self::Failure>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("read preparation failed")]
pub struct ReadPreparationError;
```

`ReadPreparationError` has no public detail/source/message field. It is an invariant failure in
trusted deterministic preparation, not a domain rejection. Expected rejection is a preceding Pure
guard or a typed evidence-to-`S::Failure` interpretation.

`mfm-program` owns and exports the uninhabited Rust `Never` type. Implement its MfmValue/schema
registration manually using the exact reserved identity; there is no constructible value and no
second Never type in `mfm-values`.

Delete `FailureValue`, `State::integrity_failure`, optional failure, framework integrity routing,
domain-local `EvmPureState`, `EvmAccessState`, and `PortfolioPureState` traits. Update the ordinary
EVM Read interpreter to map `EvmReadEvidence::IntegrityBlocked` to the exact typed
`EvmBalanceFailure::IntegrityBlocked` path before deleting the framework projection.

#### Sole checked Program type

The public API has one private-field checked `Program`. `ProgramDocumentV2` is an internal wire
projection, not a public DTO. Freeze the public construction surface to:

```rust
impl Program {
    pub fn new(
        entry_point_id: EntryPointId,
        admitted_context_contract_ref: ContentRef,
        root_success_contract_ref: ContentRef,
        root_failure_contract_ref: ContentRef,
        declarations: Vec<Declaration>,
    ) -> Result<Self>;

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self>;

    pub fn entry_point_id(&self) -> &EntryPointId;
    pub fn admitted_context_contract_ref(&self) -> &ContentRef;
    pub fn root_success_contract_ref(&self) -> &ContentRef;
    pub fn root_failure_contract_ref(&self) -> &ContentRef;
    pub fn declarations(&self) -> &[Declaration];
    pub fn canonical_bytes(&self) -> &[u8];
    pub fn content_ref(&self) -> &ContentRef;
}

pub enum Declaration {
    State(StateDeclaration),
    Match(MatchDeclaration),
}

impl Execution {
    pub fn pure() -> Self;
    pub fn read(
        capability_contract_ref: ContentRef,
        intent_contract_ref: ContentRef,
        evidence_contract_ref: ContentRef,
        binding_ref: ContentRef,
    ) -> Self;
}

impl StateDeclaration {
    pub fn new(
        state_implementation_ref: ContentRef,
        input_contract_ref: ContentRef,
        output_contract_ref: ContentRef,
        failure_contract_ref: ContentRef,
        execution: Execution,
        next_index: Option<u16>,
        failure_next_index: Option<u16>,
    ) -> Result<Self>;

    pub fn state_implementation_ref(&self) -> &ContentRef;
    pub fn input_contract_ref(&self) -> &ContentRef;
    pub fn output_contract_ref(&self) -> &ContentRef;
    pub fn failure_contract_ref(&self) -> &ContentRef;
    pub fn execution(&self) -> &Execution;
    pub fn next_index(&self) -> Option<u16>;
    pub fn failure_next_index(&self) -> Option<u16>;
}

impl MatchVariant {
    pub fn new(tag: StableId, entry_index: u16) -> Self;
}

impl MatchDeclaration {
    pub fn new(
        selector_contract_ref: ContentRef,
        variants: Vec<MatchVariant>,
    ) -> Result<Self>;

    pub fn selector_contract_ref(&self) -> &ContentRef;
    pub fn variants(&self) -> &[MatchVariant];
}
```

`Execution` also exposes exact `is_pure()`, `capability_contract_ref()`,
`intent_contract_ref()`, `evidence_contract_ref()`, and `binding_ref()` accessors, with each Read
getter returning `Option<&ContentRef>`; sealed construction makes them all-or-none. `MatchVariant`
exposes exact `tag()` and `entry_index()`. Do not add `Program::declaration(index)`—the
declarations slice and Runtime's private pre-resolved index table already own that lookup.

Declaration/Execution constructors check only invariants local to their fields. `Program::new` and
`Program::decode_canonical` call one shared whole-Program validator for limits, root/index/forward
edge/target-kind/reachability and contract-continuity rules. Domain planners and Runtime do not
duplicate that graph validator; RuntimeAssembly association adds only registry-dependent typed
descriptor checks.

The exact persisted projection is:

```text
ProgramDocumentV2 {
  entry_point_id,
  admitted_context_contract_ref,
  root_success_contract_ref,
  root_failure_contract_ref,
  declarations: [
    { kind: "state", value: {
        state_implementation_ref,
        input_contract_ref,
        output_contract_ref,
        failure_contract_ref,
        execution,
        next_index: null | u16,
        failure_next_index: null | u16
    }}
    | { kind: "match", value: {
        selector_contract_ref,
        variants: [{ tag, entry_index: u16 }]
    }}
  ]
}

execution =
    { kind: "pure" }
  | { kind: "read", capability_contract_ref, intent_contract_ref,
      evidence_contract_ref, binding_ref }
```

All fields shown are mandatory; `null` is literal and never omission. JCS owns object-key order.
Match arms are sorted unique by raw ASCII/UTF-8 tag bytes. The declarations array is
identity-bearing; constructor order is preserved, not normalized.

Index zero is the only non-empty Program entry. Every successor/arm index is in bounds, strictly
forward, and reachable from zero. Match arms target State declarations, and Runtime association
proves descriptor payload contract equals target State input. Empty declarations are allowed only
for admitted-context == root-success and root-failure == Never. The Program admits at most 65,536
declarations, at most 65,535 States, 256 arms per Match, and 8 MiB canonical bytes.

Do not build a whole 65,536-declaration fixture if it exceeds 8 MiB. Test the u16 parser/constructor
boundary independently (`65_535` accepted, JSON integer `65_536` rejected) and use representable
whole graphs for structural tests.

Retain the v1 State/capability reference algorithms exactly as frozen by the RFC. Keep explicit
input/output/failure and intent/evidence refs because those raw-ID digests do not commit their ABI.
Map every static ID-return error to `InvalidContract` before hashing. The preimage is exact raw
UTF-8 stable ID—no prefix, separator, newline, JCS envelope, or ABI. Do not replace it with a new
descriptor layer. Freeze complete refs for `mfm.test.identity/state@1` and
`mfm.test.identity/read@1` to
`content:sha256-v1:fab16c95f74ec061a9482105a20937043dd820e983a35ae63d235d69e53c540a`
and
`content:sha256-v1:70c931a7ae3e52ce943296347e6a3f0ab04425cfb8a390428145ff34753ced54`.

`ProgramError` is exactly:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ProgramError {
    #[error("program canonical bytes are invalid")]
    Canonical,
    #[error("program contract is invalid")]
    InvalidContract,
    #[error("program capacity exceeded")]
    Capacity,
}
```

Delete `InvalidValue` and `InvalidCatalog`; do not add a per-constructor error family.

`Program` and the authoring types do not implement public `Serialize`/`Deserialize`. Decode through
private raw DTOs with owned raw ContentRef/identity strings, verify strict canonical re-encoding,
then call checked construction. Map malformed/noncanonical/unknown JSON and u16 parse overflow to
`Canonical`; identity conversion, graph/Never/ordering/contract violations to `InvalidContract`;
and count or byte ceilings to `Capacity`. A typed Serde derive must not create fragment construction
paths or collapse malformed identity syntax into the wrong error.

The crate root exports exactly these 17 target names and no public `single_trust`/wire/constants
module: `State`, `PureState`, `ReadState`, `ProposedStateOutcome`, `ReadPreparationError`, `Never`,
`Program`, `ProgramError`, `Result`, `Declaration`, `Execution`, `StateDeclaration`, `MatchVariant`,
`MatchDeclaration`, `state_implementation_ref`, `capability_contract_ref`, and
`nominal_contract_ref`.

#### Complete Program deletions

Delete `ProgramCatalog`, `ProgramCatalogBuilder`, `ProgramCatalogInner`, `ProgramBuilder`,
`ProgramIngress`, the public `ProgramDocument`, `ProgramRef` wrapper, `ValueAssociation`,
`CapabilityAssociation`, `ReifyValue`, public `canonical_value`, public `QualifiedValue`/
`QualifiedTypedValue`, public downcast/erase helpers, catalog brands, and catalog closure documents.

Delete from the wire/model:

- `SequentialControlAddress`, declaration address maps/sorting/root inference/cycle DFS;
- `root_contract_ref`, `terminal`, `maximum_conclusion_bytes`, `total_attempt_bound`,
  `fact_selection_required`, `effect_domain`, signer fields, and optional execution binding;
- `AccessBindingV2`, old `BindingDescriptor`, and their schema IDs;
- Match payload/continuation contract refs; and
- every `FailureValue`/optional-failure/default-failure path.

Do not replace catalog validation with another dummy Program/closure registry. Each real Program is
associated once by RuntimeAssembly.

#### Program tests and goldens

- exact `mfm-program-document@2` canonical bytes/content ref; v1 and unknown fields rejected;
- empty identity Program; State-root Program; root Match; branch/rejoin; common failure mapper;
- forward edge, self/back edge, out-of-bounds, unreachable declaration, State-count, arm-count, and
  Program-byte bounds;
- raw tag comparator prefix/punctuation/digit ordering and duplicate/out-of-order retained arms;
- all success/failure/root contract continuities and Never placement/reachability;
- Match descriptor payload-to-target input checks in Runtime association tests;
- exact v1 State/capability ref preimage goldens from the RFC; and
- compile/public API tests proving the catalog/document/address/binding wrappers are gone.

Architect gate `C2-B`: Program architect. Require a hostile-wire review and explicit confirmation
that implicit forward indices remove more concepts/LOC than they add, array order is the only
control identity, and no second Program lifecycle survives.

### 6.3 Chunk 2C — exact Journal wire and history qualification

Primary paths:

- `crates/kernel/journal/{src,tests,Cargo.toml,README.md}`
- Journal wire fixtures/goldens owned by affected domain/runtime tests

Replace the current Journal implementation rather than adapting its open DTO graph. The retained
public surface is one redaction-safe `JournalError`, the four Journal-owned constants, opaque
`EncodedRunFrame`, opaque unqualified `StoredRunBytes`, qualified `JournalHistory`, and only the
borrowed qualified record/object views Runtime needs, plus the sole non-qualifying
`frame_head_digest` helper used by Journal and physical Store validation.

```rust
pub const MAX_FRAME_BYTES: usize = 25_231_360;
pub const MAX_FRAME_NON_PAYLOAD_ENVELOPE: usize = 65_536;
pub const MAX_RUN_FRAMES: u64 = 65_536;
pub const MAX_RUN_BYTES: u64 = 536_870_912;
// MAX_RUN_OBJECT_CANONICAL_BYTES is imported from mfm-values, not reexported.

pub fn frame_head_digest(frame_bytes: &[u8]) -> ContentDigest;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum JournalError {
    #[error("journal capacity exceeded")]
    Capacity,
    #[error("journal frame is invalid")]
    InvalidFrame,
    #[error("journal history is invalid")]
    InvalidHistory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutcomeKind { Success, Failure }

pub struct JournalObject<'a> { /* private ref + canonical bytes */ }

pub enum JournalRecord<'a> {
    RunAdmitted {
        program: JournalObject<'a>,
        admitted_context: JournalObject<'a>,
    },
    StateConcludedPure {
        kind: OutcomeKind,
        outcome: JournalObject<'a>,
    },
    StateConcludedRead {
        intent: JournalObject<'a>,
        evidence: JournalObject<'a>,
        kind: OutcomeKind,
        outcome: JournalObject<'a>,
    },
}

impl EncodedRunFrame {
    pub fn admission(
        run_id: &RunId,
        program_ref: &ContentRef,
        program: &[u8],
        context_ref: &ContentRef,
        context: &[u8],
    ) -> Result<Self, JournalError>;

    pub fn run_id(&self) -> &RunId;
    pub fn run_sequence(&self) -> u64;
    pub fn previous_head_digest(&self) -> Option<&ContentDigest>;
    pub fn head_digest(&self) -> &ContentDigest;
    pub fn canonical_bytes(&self) -> &[u8];
}

impl StoredRunBytes {
    pub fn new(ordered_frames: Vec<Vec<u8>>) -> Result<Self, JournalError>;
    // No raw getter, iterator, frame accessor, serde, or unchecked constructor.
}

impl JournalHistory {
    pub fn qualify(
        expected_run_id: &RunId,
        stored: StoredRunBytes,
    ) -> Result<Self, JournalError>;

    pub fn from_genesis(frame: EncodedRunFrame) -> Result<Self, JournalError>;

    pub fn encode_pure_conclusion(
        &self,
        kind: OutcomeKind,
        value_ref: &ContentRef,
        value: &[u8],
    ) -> Result<EncodedRunFrame, JournalError>;

    pub fn encode_read_conclusion(
        &self,
        intent_ref: &ContentRef,
        intent: &[u8],
        evidence_ref: &ContentRef,
        evidence: &[u8],
        kind: OutcomeKind,
        value_ref: &ContentRef,
        value: &[u8],
    ) -> Result<EncodedRunFrame, JournalError>;

    pub fn extend_inserted(&mut self, inserted: EncodedRunFrame)
        -> Result<JournalRecord<'_>, JournalError>;
    pub fn records(&self) -> impl ExactSizeIterator<Item = JournalRecord<'_>> + '_;
    pub fn run_id(&self) -> &RunId;
    pub fn head_sequence(&self) -> u64;
    pub fn head_digest(&self) -> &ContentDigest;
}
```

`JournalRecord<'_>` and `JournalObject<'_>` are borrowed, read-only qualified views with private
representation. They expose only the record kind/content refs and each frame-local object's
content ref/canonical bytes required by Runtime. They are not serializable or accepted by any
constructor. Do not reintroduce open durable `RunRecord`, `StateOutcome`, `ValueRef`, or object DTOs
under these names.

`JournalObject` has only `content_ref()` and `canonical_bytes()` accessors. The large owning types
are not `Clone`; their `Debug` implementations redact canonical content. Private wire parsing may
use `serde_json::value::RawValue` to preserve nested canonical object bytes without a
serialize/decode/serialize path.

`StoredRunBytes::new` accepts exactly `1..=MAX_RUN_FRAMES` buffers and checks only that nonempty raw
count plus per-frame-byte and cumulative-byte bounds. It deliberately does not claim canonical or
structural validity. Store implementations can construct it; only `JournalHistory::qualify`
consumes it. An empty `StoredRunBytes` is impossible. Absence is only `load_run -> None`: a private
empty Memory synchronization shell (`frames.is_empty()` and no head), like PostgreSQL with no head
and no frames, is merely absent and needs no cleanup lifecycle. Any physical frame without a head,
or any head without a complete nonempty prefix, is CorruptPhysicalState. This asymmetry avoids a
Store/Journal dependency cycle and avoids exposing raw retained bytes to Runtime.

Local construction uses `Capacity` for fixed bounds and `InvalidFrame` for an invariant defect;
retained qualification reports `InvalidHistory`. `StoredRunBytes::new` uses `InvalidHistory` for an
empty transfer and `Capacity` for its raw ceilings; a Store loading persisted rows maps either to
`CorruptPhysicalState` after its own physical checks. Diagnostics never include frame/object bytes.

#### Exact wire

Encode only:

```text
RunFrameV1 {
  domain: "mfm.run.frame.v1",
  run_id,
  run_sequence,
  previous_head_digest: null | digest,
  record,
  objects: sorted-unique [{ content_ref, canonical: <raw canonical JSON value> }]
}

record =
    { kind: "run_admitted", program_ref, admitted_context }
  | { kind: "state_concluded_pure", outcome }
  | { kind: "state_concluded_read", intent, evidence, outcome }

outcome =
    { kind: "success", value }
  | { kind: "failure", value }
```

Construction canonicalizes and validates each raw object, enforces the shared 8 MiB object bound,
verifies the supplied instance ref, coalesces an identical ref only when its bytes are identical,
sorts by the complete content ref, and enforces exact frame-local closure. It then measures
non-object envelope bytes, enforces frame bounds, serializes exact JCS, and computes the recursive
head only through `frame_head_digest`. That infallible helper returns
`ContentDigest::from_digest(DigestAlgorithm::Sha256V1,
mfm_canonical::sha256_digest_bytes(frame_bytes))`; it hashes exact bytes and does not parse,
canonicalize, or qualify a frame. Successor constructors derive sequence and predecessor only from
`JournalHistory`; callers do not supply them. `extend_inserted` consumes a known-inserted frame,
rechecks RunId, sequence, and predecessor before extending the same qualified accumulator.

Qualification consumes `StoredRunBytes`; for every frame it strictly decodes/re-encodes, rejects
unknowns/floats/noncanonical input, verifies closure/object refs/sorting/bounds, verifies genesis and
all predecessor recurrence, and requires one identical RunId throughout the chain equal to
`expected_run_id`. Runtime supplies that requested identity and its semantic fold still checks the
genesis association; Store separately checks each physical row key against its sealed frame RunId.
Journal exposes no public single-raw-frame decoder that Store could use to create a parallel
qualification path.

#### Journal deletions

Delete or wholly replace the current `single_trust.rs` open model and all exports for:

```text
RunFrame / RunRecord / RunAdmitted / StatePrepared / StateConcluded
StateOutcome / ImmutableObject / ValueRef / RecordLogicalKey / PreparationRef
PreparationMode / StateConcludedAccess / occurrence / preparation_sequence
entry_point in admission / record_digest / first-reference state
facts / configuration / portable codecs / request or authority identities
open constructors followed by validate
public RunPosition / raw-history and raw-frame inspection
```

There is no configuration or fact journal, first-reference dictionary, global object count,
semantic record digest, call/preparation ID, or Effect tag.

#### Journal verification

- exact canonical/frame-head goldens for admission, Pure success/failure, and fused Read
  success/failure;
- hard-coded complete `content:sha256-v1:<64 lowercase hex>` head strings for a genesis and its
  successor, exact equality with `frame_head_digest(frame.canonical_bytes())`, and rejection of the
  `sha256-jcs-v1` tag even for identical digest bytes;
- strict unknown-field/tag/version/v1-old-wire rejection;
- missing/extra/duplicate/out-of-order/conflicting object closure tests;
- wrong object hash/schema, wrong RunId, gap, predecessor, sequence, and recursive-head tests;
- object 8 MiB, envelope 65,536 theorem fixture, exact 25,231,360 frame derivation, frame count,
  and cumulative run-byte boundaries;
- three maximum objects fit one fused Read frame; and
- API/negative tests prove no open DTO/raw-history/preparation/Effect/config/fact surface remains.

Architect gate `C2-C`: Journal/persisted-wire architect. Require a byte-level hostile-input review,
proof that one encode/one qualify path exists, and confirmation that the borrowed views did not
become a second construction API.

### 6.4 Chunk 2D — mechanical Store and Memory

Primary paths:

- `crates/kernel/store/{src,tests,Cargo.toml,README.md}`

Replace `backend.rs`, `single_trust.rs`, the semantic backend facade, and their tests. The target
library surface is:

```rust
use std::{future::Future, pin::Pin};

pub trait Store: Send + Sync {
    fn load_run<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> Pin<Box<dyn Future<Output = Result<Option<StoredRunBytes>, StoreError>>
                  + Send + 'a>>;

    fn append_run<'a>(
        &'a self,
        frame: &'a EncodedRunFrame,
    ) -> Pin<Box<dyn Future<Output = Result<AppendResult, StoreError>>
                  + Send + 'a>>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppendResult { Inserted, NotInserted }

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum StoreError {
    #[error("store capacity exceeded")]
    Capacity,
    #[error("store physical state is corrupt")]
    CorruptPhysicalState,
    #[error("store operation is unavailable")]
    Unavailable,
    #[error("store append outcome is indeterminate")]
    Indeterminate,
}

pub struct MemoryStore { /* private per-RunId state */ }
impl MemoryStore { pub fn new() -> Self; }
impl Default for MemoryStore { /* new */ }
```

Use a small physical model that permits corruption injection only inside tests:

```rust
struct MemoryStore {
    runs: tokio::sync::Mutex<
        HashMap<RunId, Arc<tokio::sync::Mutex<MemoryRun>>>,
    >,
}

struct MemoryRun {
    frames: Vec<Arc<StoredFrame>>, // dense by construction
    head: Option<Head>,
}

struct StoredFrame { bytes: Vec<u8>, head_digest: ContentDigest }
struct Head { sequence: u64, total_bytes: u64 }
```

Keep the borrowing Store API and make the async/CPU boundary private. Tokio is a normal
`mfm-store` dependency. Never add `Clone` to `EncodedRunFrame`, expose an owned canonical-byte
accessor, use `block_in_place`/`block_on`, or move a Memory guard/state into `spawn_blocking`.

Use one private pure-job helper that first calls `tokio::runtime::Handle::try_current`; polling a
Store outside Tokio returns `Unavailable` rather than panicking. Join failure is likewise
`Unavailable` before mutation. Bridge the borrowed candidate without blocking the executor:

```rust
async fn own_candidate_bytes(source: &[u8]) -> Result<Vec<u8>, StoreError> {
    let length = source.len();
    let mut owned = run_pure_blocking(move || {
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(length).map_err(|_| StoreError::Unavailable)?;
        Ok(bytes)
    }).await?;

    for chunk in source.chunks(256 * 1024) {
        owned.extend_from_slice(chunk); // infallible after exact reserve
        tokio::task::yield_now().await;
    }
    Ok(owned)
}
```

The 256 KiB chunk is private scheduling policy. Clone only small RunId/sequence/predecessor/digest
projections separately. Cancellation during this cooperative snapshot drops the private buffer and
mutates nothing.

Use full borrowing boxed-future signatures; expose no public `StoreFuture`, backend future/result
alias, async-trait dependency, generic Store parameter propagated through Runtime/App, opening
method, readiness method, run list, selector, or pagination API. Runtime holds `Arc<dyn Store>`.

Memory serializes one RunId append in one in-process critical section. It performs every fallible
validation and required allocation before publishing anything, then publishes immutable frame,
head, and total bytes together. No returned-error path exists after the first mutation. It derives
target sequence/predecessor/head/bytes from `EncodedRunFrame`; an exact target returns
`NotInserted`, a noncurrent predecessor returns `NotInserted`, and one caller at most receives
`Inserted`.

Capacity checks apply only to an insertion candidate and use checked subtraction:

```text
frame_len <= MAX_FRAME_BYTES
candidate_sequence <= MAX_RUN_FRAMES
current_total <= MAX_RUN_BYTES - frame_len
```

`NotInserted` bypasses insertion capacity checks. There is no pre-provider or future-conclusion
preflight because no preparation exists; Store accounts only actual bytes.

For load, lock one run only long enough to capture its small head and an ordered vector of immutable
`Arc<StoredFrame>` snapshots, then release the lock. Move that snapshot into one pure blocking job
that hashes, validates, copies up to 512 MiB, and constructs `StoredRunBytes`. Yield while copying a
large pointer vector if necessary. Return one complete prefix or an error. Memory never
manufactures `Indeterminate`.

For append, cooperatively own the candidate bytes before locking. Acquire an owned per-run guard,
clone only the current head and target Arc observations, and await a pure blocking planner while the
async future retains the guard. The job receives no guard or mutable Store state; it validates
digests, exact bytes, predecessor, and capacity and returns either `NotInserted` or the insert
material. Back in the future, fallibly reserve the one dense frame slot, then publish immutable
frame/head/total in one small non-awaiting section. Dropping the future while the planner runs drops
the guard and result without mutation.

Do not put an `Option` slot or arbitrary interior-gap state in the production Memory model merely
for tests. Same-crate tests may corrupt bytes/digest/head/total or truncate rows; PostgreSQL owns the
interior-gap hostile-row case because its physical representation can express it.

#### Store deletion

Delete all semantic and split-backend surfaces, including:

```text
StoreBrand / StructuredStore / OpenedStructuredStore / StoreParts
QualifiedHistoryPort / HistoryReader / StoreAuditPort / QualifiedRun
RunReducer / RunSelection / RunAction / SelectedRun
PreparedAdmission / PreparedConclusion / SelectedConclusion / FactContinuation
SemanticStore / replay_terminality / retained_program / validate_prefix
StructuredStoreBackend / MemoryStructuredBackend
AccessActionMode / AccessPreparationOutcome / AdmissionOutcome
SelectedConclusionOutcome / SelectedConclusionPreparationOutcome
BackendAppendCommand / BackendAppendOutcome / AppendDisposition
AppendResult::{Existing,Stale} / RunAppend / RunPosition
StoreWorkLimits / RawHistoryLoadLimit / raw selectors/prefixes/facts/configuration
AppendRequestId / expected_position / command digest / receipt / reservation
AcknowledgementUnknown / UnavailableBeforeSubmission / InvalidPhysicalCommand
AlreadyConcludedSame / idempotency-conflict result types
Store-owned ReadyError / StoreOpenError / check_ready
```

Remove Store's production dependencies on Canonical, capabilities, facts, Program, values, and
serde_json. It depends only on IDs, Journal transfer/frame/hash surface, `thiserror`, and Tokio for
its private async mutex/cooperative-copy/pure-job boundary. Memory stores
`EncodedRunFrame::head_digest()` on insertion and calls only
`mfm_journal::frame_head_digest(frame_bytes)` when recomputing derived physical metadata on load.
It never selects a digest algorithm or parses JSON; frame construction and wire qualification
remain Journal-owned.

#### Store conformance

Replace the broad old conformance harness with one small scenario matrix used by Memory and
PostgreSQL tests without becoming a production Store API. Keep it under integration-test support
and include it from both test targets by source path or another test-only mechanism; do not retain
`backend-conformance`, `parity-tests`, or `test-support` Cargo features/public exports. It covers absent load, exact retry,
competing candidate, later exact historical retry, absent-head orphan, corrupt target/head,
capacity/count/byte limits, atomic fault boundary, complete load, and error taxonomy. If a dev-only
helper is required, keep it inside tests rather than exposing backend commands or production
constructors.

The matrix also proves absence is `None`, a private empty Memory synchronization shell loads as
`None`, no backend can return `Some` with an empty transfer, frames without a head and a head without
a complete nonempty prefix are CorruptPhysicalState, independently corrupted bytes/digest metadata
is detected through Journal's helper, and mfm-store has no mfm-canonical dependency or call site.

Architect gate `C2-D`: storage-interface architect. Require object-safety/lifetime review, an
explicit search showing Store has no semantic/domain dependency, and proof that Memory publishes
atomically with fewer branches/types than the old backend. Current-thread Tokio heartbeat,
barrier-controlled job cancellation, candidate-copy equality, and join-failure tests must prove no
executor block or mutation after a discarded pure result. PostgreSQL alone tests an interior row
gap; do not add an impossible optional slot to Memory solely for fault injection.

### 6.5 Chunk 2E — PostgreSQL physical Store

Primary paths:

- `crates/storages/postgres/{src/lib.rs,migrations,build.rs,Cargo.toml,README.md}`
- PostgreSQL parity/integration tests and SQLx checks

#### Exact production API

Expose only:

```rust
pub struct PostgresStore {
    pool: sqlx::PgPool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum StoreOpenError {
    #[error("postgres store is incompatible")]
    Incompatible,
    #[error("postgres store is unavailable")]
    Unavailable,
}

impl PostgresStore {
    pub async fn connect(
        database_url: &str,
    ) -> std::result::Result<Self, StoreOpenError>;
}
```

`mfm-storage-postgres` owns `StoreOpenError`; `mfm-store` does not reexport it. Delete public
`DurabilityProfile`, `PostgresError`, its broad `Result`, `from_pool`, `migrate`, `rotate_identity`,
`check_ready`, `profile`, pool/schema accessors, and any unchecked/test-support constructor.
Same-crate tests may use a private `#[cfg(test)] install_fresh_schema` and checked pool helper.
Production schema installation is the static SQL migration artifact, never a Store method.
Delete the current build script if it exists only to track/embed the migration; `include_str!` owns
that compile dependency. Remove `parity-tests`/`test-support` features and serde/serde_json. Keep
Tokio as a direct normal dependency for cooperative candidate snapshots and pure byte jobs; SQLx's
transitive runtime dependency is not an owned API contract.

#### Fresh schema baseline

Replace the existing migration in place; do not add a migration from the old schema. The logical
shape is only:

```sql
CREATE TABLE public.mfm_store_schema (
    schema_contract TEXT COLLATE "C" PRIMARY KEY
        CHECK (schema_contract = 'mfm.run-history-postgres.v1')
);
INSERT INTO public.mfm_store_schema (schema_contract)
VALUES ('mfm.run-history-postgres.v1');

CREATE TABLE public.mfm_run_frames (
    run_id TEXT COLLATE "C" NOT NULL
        CHECK (run_id ~ '^run:sha256-jcs-v1:[0-9a-f]{64}$'),
    run_sequence BIGINT NOT NULL CHECK (run_sequence BETWEEN 1 AND 65536),
    frame_bytes BYTEA NOT NULL CHECK (octet_length(frame_bytes) BETWEEN 1 AND 25231360),
    head_digest TEXT COLLATE "C" NOT NULL
        CHECK (head_digest ~ '^content:sha256-v1:[0-9a-f]{64}$'),
    PRIMARY KEY (run_id, run_sequence)
);

CREATE TABLE public.mfm_run_heads (
    run_id TEXT COLLATE "C" PRIMARY KEY
        CHECK (run_id ~ '^run:sha256-jcs-v1:[0-9a-f]{64}$'),
    head_sequence BIGINT NOT NULL CHECK (head_sequence BETWEEN 1 AND 65536),
    total_bytes BIGINT NOT NULL CHECK (total_bytes BETWEEN 1 AND 536870912),
    FOREIGN KEY (run_id, head_sequence)
        REFERENCES public.mfm_run_frames (run_id, run_sequence)
        ON UPDATE NO ACTION
        ON DELETE NO ACTION
);
```

Add exact checked grammar constraints for `RunId` and head digest, and require all authority tables
to be permanent/logged. These are exactly the three MFM-owned Store tables; the database may
contain unrelated operator-owned objects. The marker table must contain exactly the inserted row.
The primary keys are sufficient; do not add predecessor, length, head
digest to `mfm_run_heads`, command/digest/receipt/reservation/global-object indexes, or a sentinel
lock row.

Delete every old tenant/scope/epoch/identity/rotation, object/membership, request/receipt,
configuration, fact/proof, reservation, and global-sequence table, column, query, test helper, and
schema ID.

#### Checked connection lifecycle

`connect` owns its pool. Before returning, and in the pool's `after_connect` hook for every new
physical connection/reconnect, require:

- exact static schema marker and expected table/column/constraint baseline;
- `pg_is_in_recovery() = false`;
- `fsync = on` and `full_page_writes = on`; and
- every authority table is logged.

Initial wrong baseline/unsafe durability is `StoreOpenError::Incompatible`; inability to observe it
is `Unavailable`. After Runtime exists, a replacement connection failing this gate is rejected and
the affected Store operation returns `StoreError::Unavailable`; an append candidate has not yet
been submitted. Store reconstruction is required. Do not add `StoreError::Incompatible` or a
post-open public health method.

#### Complete load transaction

Implement one `REPEATABLE READ READ ONLY` transaction whose isolation/read-only mode is established
before its first snapshot statement:

```text
begin repeatable read read only
head = select head left-joined to its immutable frame
if no head:
    if exists(any frame for RunId): CorruptPhysicalState
    else: commit and return None
reject head.total_bytes > MAX_RUN_BYTES before allocation
aggregate count, min(sequence), max(sequence), sum(octet_length(frame_bytes))
require min(sequence) == 1, max(sequence) == head_sequence,
        count == head_sequence, and sum == head.total_bytes
fetch all frame rows ordered by sequence
move the owned Vec<PgRow> into one pure spawn_blocking job
inside that job decode borrowed BYTEA slices from the owned rows, verify RunId/key/order,
    raw length, stored digest using mfm_journal::frame_head_digest(exact retained frame_bytes),
    and checked accumulated sum;
    copy final buffers and construct StoredRunBytes::new
commit read-only transaction and return Some(...)
```

Do not use `query_as::<_, Vec<u8>>` for the large BYTEA column; its decode copies on the executor.
Fetch owned `PgRow`s and move only those rows—not the transaction/connection—into the pure job.
Compile-assert that the configured SQLx `PgRow` is `Send + 'static`.

A gap, duplicate, orphan, broken head join, wrong key/digest, or accounting mismatch is
`CorruptPhysicalState`. Database/pool failure to produce a complete snapshot is `Unavailable` and
returns no partial bytes. Load never returns `Indeterminate` or `Capacity`.

#### Exact-head append transaction

Implement this order exactly:

```text
BEGIN READ COMMITTED READ WRITE
SET LOCAL synchronous_commit = on
lock_key = first 8 bytes, big-endian two's-complement i64, of
           SHA-256(JCS({domain:"mfm.store.run-lock.v1",run_id}))
SELECT pg_advisory_xact_lock(lock_key)       -- before every state-observing read

read head left-joined to its frame
read candidate target (run_id, candidate sequence)
when head absent, indexed EXISTS(any frame for RunId)

validate all observed physical rows first:
  absent head implies no frame
  present head joins exact frame and has valid key/digest/total
  observed target has exact key/self-consistent digest and sequence <= head
  candidate sequence <= head implies target exists
  any violation => rollback + CorruptPhysicalState

if exact target bytes exist: rollback + NotInserted
if candidate-derived predecessor != current validated head: rollback + NotInserted
if actual frame/count/cumulative bytes exceed limits: rollback + Capacity

insert immutable frame
insert/update head and checked total
COMMIT
```

Before `BEGIN`, create the cooperative owned candidate snapshot specified in Chunk 2D. Fetch
head/target as owned `PgRow`s. Move only those rows and candidate bytes into one pure blocking
planner that checks physical digests/equality/predecessor/capacity and, only for an insert, builds
the private owned `PgArguments` BYTEA buffer off the executor. Compile-assert that
`PgArguments` is `Send + 'static`. Execute `query_with`, the small head write, and COMMIT
asynchronously. The connection, transaction, advisory-lock authority, and all DML remain outside
the blocking job. Planner results preserve `CorruptPhysicalState`, `Capacity`, and `NotInserted`;
only blocking-job infrastructure or SQL-argument encoding failure maps to `Unavailable`. All occur
before the first DML and never map to `Indeterminate`.

The natural `(run_id, run_sequence)` target and exact frame bytes are the sole receipt. Hash
collisions may only over-serialize. Do not add a second lock, target digest index, command ID,
expected position, result position, or full-history scan on append.

PostgreSQL retains its direct mfm-canonical dependency only for the separate advisory-lock JCS
preimage above. It never uses Canonical to select or brand a frame-head algorithm; insertion uses
the sealed frame projection and physical reload/append validation uses Journal's helper.

Classify errors at the commit boundary:

- pre-COMMIT connection/query failure or confirmed rollback/rejection: `Unavailable` when it is an
  availability failure and the candidate definitely did not commit;
- proved physical invariant/constraint corruption before mutation or after rollback:
  `CorruptPhysicalState`;
- capacity rejection before mutation: `Capacity`;
- a PostgreSQL response that proves COMMIT rejection/rollback: `Unavailable`; and
- IO/protocol/no response after COMMIT may have reached PostgreSQL: `Indeterminate`.

Do not classify every definitely-noncommitted error as Unavailable—Capacity and corruption retain
their own variants. Never return `Indeterminate` merely because an earlier statement failed.

#### PostgreSQL tests

- run the shared Store conformance matrix against PostgreSQL;
- repeatable-snapshot append race, no torn prefix, absent-head orphan, broken join/target/gap,
  exact historical retry, and concurrent competing append;
- connection/open/reconnect checks for marker, recovery, durability settings, and unlogged tables;
- current-thread executor heartbeat during large row hashing/copy, exact `query_with` BYTEA
  persistence, and configured-SQLx `PgRow`/`PgArguments` Send assertions;
- synchronous-commit assertion and advisory-lock hash golden;
- head-digest constraint accepts exact `content:sha256-v1:<64 lowercase hex>` and rejects the
  `sha256-jcs-v1` tag without changing the advisory-lock JCS golden;
- fault injection before COMMIT submission, confirmed rollback, COMMIT rejection, and unknown
  acknowledgement;
- exact sequence/frame/run-byte SQL boundaries and hostile grammar/constraint rows; and
- schema inventory proves exactly the three tables and no old column/table/index.

Keep non-database classifier/schema unit tests unignored. Put managed-DB tests in private
same-crate `#[cfg(test)]` modules and mark them ignored with an explicit managed-service reason;
`postgres-test` runs `cargo test -p mfm-storage-postgres --lib -- --include-ignored` with its
managed `DATABASE_URL`. Do not silently skip DB tests by inspecting whether an environment variable
happens to exist.

Architect gate `C2-E`: PostgreSQL/durability architect. Require transaction-order and ambiguous-
commit review, one-snapshot load proof, a fresh-schema inventory, and rejection of any duplicate
semantic/idempotency owner.

### 6.6 Chunk 2F — Runtime assembly, sole reducer, and progression

Primary paths:

- `crates/kernel/runtime/{src,tests,Cargo.toml,README.md}`

Delete `lifecycle.rs` and replace the current `single_trust.rs` ownership graph. Keep modules small
enough that assembly/association, fold, typed drivers, and public API tests have one obvious owner;
do not preserve old module names or types merely to reduce diff size.

#### Exact public surface

```rust
pub type Result<T> = std::result::Result<T, RuntimeError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RuntimeError {
    #[error("run is absent")]
    Absent,
    #[error("run admission conflicts with retained history")]
    AdmissionConflict,
    #[error("append outcome is indeterminate")]
    Indeterminate,
    #[error("retained run history is invalid")]
    InvalidHistory,
    #[error("runtime assembly is incompatible")]
    IncompatibleAssembly,
    #[error("runtime capacity exceeded")]
    Capacity,
    #[error("runtime dependency is unavailable")]
    Unavailable,
    #[error("runtime internal failure")]
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ReadAdapterError {
    #[error("read adapter is unavailable")]
    Unavailable,
    #[error("read adapter failed")]
    Internal,
}

pub struct RuntimeAssemblyBuilder { /* private registries */ }
pub struct RuntimeAssembly { /* finalized immutable Arc-owned inner */ }

impl RuntimeAssemblyBuilder {
    pub fn new() -> Self;
    pub fn register_value<T: MfmValue>(&mut self) -> Result<()>;
    pub fn register_pure<S: PureState>(&mut self) -> Result<()>;
    pub fn register_read<S, C>(&mut self) -> Result<()>
    where
        S: ReadState<C>,
        C: ReadCapabilityContract;

    pub fn register_adapter<C, B, F>(
        &mut self,
        binding: B,
        callback: F,
    ) -> Result<()>
    where
        C: ReadCapabilityContract,
        B: MfmValue,
        F: for<'a> Fn(&'a C::Intent)
                -> Pin<Box<dyn Future<
                    Output = std::result::Result<C::Evidence, ReadAdapterError>,
                > + Send + 'a>>
            + Send
            + Sync
            + 'static;

    pub fn finish(self) -> Result<RuntimeAssembly>;
}

pub struct Runtime { /* RuntimeAssembly + Arc<dyn Store> */ }

impl Runtime {
    pub fn new(assembly: RuntimeAssembly, store: Arc<dyn Store>) -> Self;

    pub async fn start<T: MfmValue>(
        &self,
        run_id: RunId,
        program: Program,
        c0: T,
    ) -> Result<RunView>;

    pub async fn resume(&self, run_id: &RunId) -> Result<RunView>;
    pub async fn read(&self, run_id: &RunId) -> Result<RunView>;
}
```

Do not expose public assembly lookup/associate methods, codec registrations, driver traits,
`BoxFuture`, `Any`, Store getters, Program registries, session handles, or per-method error/response
types.

`RunView`, `RetainedValueView`, and `RunViewState` are private-field, non-Serde trusted-core types:

```rust
pub struct RunView { /* private durable head + state */ }
pub struct RetainedValueView { /* private qualified retained value */ }

pub enum RunViewState {
    Runnable,
    Succeeded(RetainedValueView),
    Failed(RetainedValueView),
}

impl RunView {
    pub fn run_id(&self) -> &RunId;
    pub fn head_sequence(&self) -> u64;
    pub fn head_digest(&self) -> &ContentDigest;
    pub fn state(&self) -> &RunViewState;
}

impl RetainedValueView {
    pub fn contract_ref(&self) -> &ContentRef;
    pub fn value_ref(&self) -> &ContentRef;
    pub fn canonical_bytes(&self) -> &[u8];
}
```

No constructor is public. Every view names a real qualified durable head. A zero-State success is
derived from the durable genesis/C0; every other terminal value is from a durable conclusion.

#### Assembly contribution and association

`register_pure` automatically registers or exact-deduplicates `S::{Input,Output,Failure}`.
`register_read` does the same plus `C::{Intent,Evidence}`. `register_value` is only for otherwise
unreferenced roots, Match selectors, and zero-State values; Never is automatic. Exact repeated
value/static State registrations are no-ops; a same stable identity with a different schema/Rust
association is `IncompatibleAssembly`. Registering one State reference with both Pure and Read
meaning is invalid. Across the entire builder, one capability contract ref has exactly one Rust
`C` plus Intent/Evidence association; a second Rust capability under the same ref is incompatible
even when registered through a different State.

`register_adapter` runs `canonicalize_mfm_value` on `B`, derives the binding ref internally, and
stores one callback under private `(capability_contract_ref, binding_ref)`. Every duplicate adapter
key is rejected because closures cannot be equality-deduplicated. The method never accepts a
caller-asserted ref. `finish` validates all static contributions and yields immutable Arc-owned
state.

Associating one checked Program produces a Runtime-private `ExecutableProgram` containing one Arc
to assembly inner plus compact pre-resolved codec, State-driver, adapter, and Match-projection
handles. Association must:

- resolve every reachable State implementation by exact v1 ref and compare Input/Output/Failure,
  Pure/Read kind, and Read C/intent/evidence signature;
- resolve every Read adapter by exact capability/binding tuple with no partial/fallback key;
- validate Match closed-sum tagging, exhaustive sorted tags, payload contracts, and target State
  inputs; and
- fail `IncompatibleAssembly` before fold/provider IO when any association is missing or differs.

Declaration index/routes/root fields are not part of reusable driver identity. State is not part of
adapter identity. Fold receives only `ExecutableProgram`, never RuntimeAssembly again, and performs
no registry lookup or association-time downcast. Only the already pre-resolved monomorphic driver
downcasts its exact private `QualifiedValue` members to its registered State/capability types; there
is no generic, fallback, or second downcast path.

#### One private erasure boundary

Use one private object-safe `RegisteredState` boundary. Its exact private spelling may follow this
shape:

```rust
trait RegisteredState: Send + Sync {
    fn associate(
        &self,
        declaration: &StateDeclaration,
        adapters: &AdapterRegistry,
    ) -> Result<Arc<dyn RegisteredState>>;

    fn validate_retained_read(
        &self,
        intent: &QualifiedValue,
        evidence: &QualifiedValue,
    ) -> Result<()>;

    fn start<'a>(
        &'a self,
        input: QualifiedValue,
        context: DriverContext<'a>,
    ) -> DriverFuture<'a>;
}

type DriverFuture<'a> = Pin<Box<
    dyn Future<Output = Result<DriverDisposition>> + Send + 'a
>>;
```

Passing `DriverContext<'a>` by value avoids tying a mutable reference's borrow to the context's
entire inner lifetime. The same trait/driver implementation owns its private association-time
adapter binding and retained evidence-binding validation; do not create a second erased
association trait.

`QualifiedValue`, `DriverContext`, `DriverDisposition`, `ExecutableProgram`, codec holders, and
adapter holders are Runtime-private. There is one private canonical value representation containing
`Box<dyn Any + Send + Sync>`, exact contract, instance ref, and the canonical bytes returned by the
sole qualifier. `Box<dyn Any + Send + Sync>` is permitted only inside this association/driver
boundary; no second supported erase/downcast workflow exists. Its typed member is mandatory on
every path: cold
qualification decodes exactly once from the Journal-owned canonical bytes and retains those exact
bytes beside the typed value, hot qualification retains the exact typed value, and Match projection
produces the same complete representation. There is no bytes-only variant, reserialization during
cold decoding, or serialize-then-decode handoff between adjacent hot States.

`DriverContext` represents its accumulator borrow as a private takeable
`&mut Option<Accumulator>`. Before a `'static` pure blocking job, take the owned `Send`
accumulator/value, move it into the job, immediately await, and replace the slot from the result.
If the outer future is dropped while the slot is empty, that invocation ends and durable history
remains the only authority. Do not replace this move schedule with `Arc<Mutex<_>>`, an owner cell,
or a detached-result path.

Reusable `PureDriver<S>` and unbound `ReadDriver<S,C>` registrations are monomorphic. Association
downcasts/binds the exact adapter once and produces the bound driver retained by
`ExecutableProgram`. `RegisteredState::start` owns the complete attempt. A private
`DriverContext` borrows Store and the one accumulator and exposes only append-and-fold-next,
reload-required, and stop behavior. The driver returns a small durable disposition, never a typed
owner/result smuggled through erasure.

#### Sole fold and hot accumulator

The private fold input is exactly one `ExecutableProgram` plus one `JournalHistory`. It yields:

```text
Runnable { declaration_index, pre-resolved driver, qualified input }
Succeeded { exact root-success qualified value }
Failed { exact root-failure qualified value }
InvalidHistory
```

It checks requested/genesis RunId, retained Program/C0 association, index-zero/zero-State rules,
selected declaration order, exact Pure/Read conclusion kind, fused Read intent/evidence/outcome,
cold `C::bind_evidence`, success/failure continuities, Match projection, common failure mapper, and
root terminality. Match extracts the selected canonical payload through the pre-resolved schema
projection and feeds the same `QualifiedValue`; it emits no frame.

For an acknowledged genesis `Inserted`, `start` retains the checked Program and exact qualified C0,
constructs `JournalHistory::from_genesis`, compares both refs/bytes to its sole admission record,
and builds the same private `QualifiedJournalRecord` used by cold fold. For every acknowledged
successor `Inserted`, move the exact local `EncodedRunFrame` into
`JournalHistory::extend_inserted`. The hot driver retains the exact qualified outcome, and for a
Read the intent/evidence/outcome, across the append. Compare their kinds, refs, and canonical bytes
against the returned borrowed record view, then build the same private qualified record that cold
codecs construct. End the borrowed Journal-record scope before mutating the accumulator in
`fold_next`. This preserves typed values without a bytes-to-typed decode while still proving that
the acknowledged frame is exactly the proposed record. Never reload
after known Inserted. On `NotInserted`, discard the local accumulator, complete-load, qualify, and
full-fold the winner. On `Indeterminate`, return immediately without local advancement. This is
linear in loaded/appended frames, not quadratic full-history refolding.

#### Entry and progression algorithms

`start`:

```text
move checked Program + typed C0 + immutable assembly handle into one pure spawn_blocking job
validate/associate Program + C0 contract, canonicalize/qualify C0, and encode genesis
immediately await the exact genesis + qualified admission values; do not pre-load
append_run(genesis)
Inserted    -> JournalHistory::from_genesis + fold_next + advance_until_stable
NotInserted -> complete load; exact genesis continues, any structurally qualified different genesis AdmissionConflict
Indeterminate/error -> mapped RuntimeError; retain no owner
```

Admission `NotInserted` has deliberate collision precedence. Complete-load and Journal-qualify the
winner, strictly decode its retained Program, verify the Program ref and admitted-C0 contract/ref,
then compare the complete Program and C0 refs plus canonical bytes to the proposed checked genesis.
Any different genesis that passes Journal structural qualification, strict Program decode/ref
verification, and the retained C0 schema-ref check is `AdmissionConflict`, even when this assembly
does not support that foreign Program. Runtime does not claim that the foreign C0 was decoded or
typed-qualified: that would require the deliberately unavailable foreign assembly codec. An exact
genesis reuses the already-associated proposed Program/C0 and full-folds; subsequent semantic
defects are `InvalidHistory`. Do not require foreign assembly association merely to classify a
RunId collision. `NotInserted` followed by an absent load is an `Internal` Store-contract
violation.

`resume`:

```text
complete load or Absent
move requested RunId + StoredRunBytes + immutable assembly handle into one pure spawn_blocking job
JournalHistory::qualify(requested RunId, stored bytes)
decode retained Program from genesis and verify ref
associate ExecutableProgram
qualify retained values and full-fold once
immediately await and recover the qualified accumulator/view
advance_until_stable
```

`read` performs the same complete load, qualification, association, and fold but never calls
`RegisteredState::start`, an adapter, or a provider. Cold fold may call the associated driver's
pure retained `C::bind_evidence` validator; that is proof validation, not external execution. It
may return `Runnable`.

`advance_until_stable` handles one selected State at a time:

- Pure: `spawn_blocking` evaluate + qualify; encode one Pure conclusion; append/fold locally;
- Read: use one pure blocking job to prepare and qualify the intent; call the exact borrowed-intent
  adapter once; then use one immediately awaited pure blocking job to qualify the returned typed
  evidence, run `C::bind_evidence`, interpret it with the retained typed State input, qualify the
  outcome, and encode one fused Read conclusion; append/fold locally; and
- Match: project/fold synchronously or through bounded pure blocking work, with no record.

Cold entry may inspect 512 MiB/65,536 frames, so Journal qualification, retained Program decode and
association, retained value qualification, and the full fold must not execute inline on the async
executor. Use the one immediately-awaited pure `spawn_blocking` job above rather than splitting it
into repeated parse passes. Apply the same rule to hot canonical/value qualification and any
substantial Match projection, Journal encoding/exact-byte comparison, or `fold_next` work. Move the
owned accumulator/value into the job and back; do not clone history. An immutable
`ExecutableProgram` may carry pre-resolved bound-driver/callback handles through the closure as
inert association data, but the closure never invokes `RegisteredState::start`, an adapter
callback, a provider/client method, or an async future. No Store handle, connection, mutation, or
append authority enters a Runtime blocking closure.

`ReadPreparationError` becomes `Internal`, enters no adapter, and appends nothing.
`ReadAdapterError::Unavailable` becomes Runtime `Unavailable`; `ReadAdapterError::Internal` becomes
Runtime `Internal`. Both append nothing and end that invocation—there is no immediate retry loop.
Every accepted evidence variant, including authenticated external domain-integrity evidence, goes
through ordinary State interpretation. A local intent/target mismatch is the adapter `Internal`
case and cannot mint durable evidence. Concurrent Reads may observe twice; exact-head append selects
one durable winner and every `NotInserted` caller reloads it.

Contain panics only at the trusted adapter boundary. A small Runtime-private future wrapper applies
`catch_unwind` both when constructing the callback future and around every poll of that future.
Either panic becomes redacted `RuntimeError::Internal`, appends nothing, and the returned error
carries no panic payload or detail. Registered callbacks remain subject to the repository's
no-secret panic/log contract; `catch_unwind` does not suppress an installed panic hook. This wrapper
adds no retry, timeout, detached work, public lifecycle, or new owner; test construction-time and
poll-time panics separately.

A successful start/resume progresses until Succeeded or Failed. Capacity, invalid/incompatible
state, infrastructure failure, or indeterminate append returns an error. Program acyclicity plus
run frame/byte limits are the only progression bounds; do not add a State-start counter/yield
setting.

#### Error mapping

Implement one conversion table, not scattered `From` guesses:

| Source | Runtime error |
| --- | --- |
| absent resume/read | `Absent` |
| structurally qualified different genesis under one RunId | `AdmissionConflict` |
| Store append ambiguity | `Indeterminate` |
| retained Store corruption, Journal/Program/limit/semantic violation | `InvalidHistory` |
| retained Read bind failure | `InvalidHistory` |
| inconsistent registration/finalization or unsupported Program | `IncompatibleAssembly` |
| local admission/outcome/frame or append capacity | `Capacity` |
| `ReadAdapterError::Unavailable` or definite Store unavailability | `Unavailable` |
| `ReadAdapterError::Internal`, supplied Program/C0 mismatch, hot bind, trusted driver/join/codec fault | `Internal` |

Private in-flight faults retain their sources only until classification. The public Copy
`RuntimeError` carries only the redacted code/message above. Do not turn errors into RunView states.

Map Store errors by operation, exhaustively:

| Store result | Load mapping | Append mapping |
| --- | --- | --- |
| `Capacity` | `Internal` (impossible Store contract result) | `Capacity` |
| `CorruptPhysicalState` | `InvalidHistory` | `InvalidHistory` |
| `Unavailable` | `Unavailable` | `Unavailable` |
| `Indeterminate` | `Internal` (loads never mutate) | `Indeterminate` |

An append `NotInserted` is not an error; it triggers the complete reload path above. Tests inject
every cell in this table rather than asserting one undifferentiated Store conversion.

#### Cancellation and blocking

Dropping `start`/`resume` is supported. It may add no candidate frame, or one in-flight append may
commit atomically; all earlier acknowledged prefix frames remain. The next complete load resolves
the durable state. Pure recomputes and observational Read may repeat.

All potentially substantial trusted synchronous work enters `spawn_blocking`; the closure owns no
Store or persistence authority, invokes no adapter/provider/async IO, and contains no `block_on`.
Runtime immediately awaits it. A
closure that finishes after outer-future drop has its result discarded and cannot perform IO.
Runtime owns no timeout, semaphore, cancellation token, cleanup, detached task, pending cell, or
completion finalizer.

#### Runtime deletion and tests

Delete all old public/private lifecycle families listed in RFC Section 11, including `Dynamic*`,
`Accepted*`, `Unresolved*`, implementation-holder structs, `PreparedExecution`, `CommittedCall`,
`QualifiedAdapter`, public static `BoxFuture`, `RunSession`, `SuspendedRun`, `ParkedRun`,
`PendingConclusion`, step enums, limits/semaphores, pending/acknowledgement owners, cancellation
branches, Store semantic ports, and catalog accessors.

Rewrite API/compile tests and add:

- exact value/State idempotent registration, inconsistent association, and duplicate adapter key;
- unsupported Program/ABI and Match association: `start` rejects before any Store call, while
  `resume`/`read` necessarily load but reject before adapter/provider/append IO;
- hot == cold RunView for Pure, Read, Match, mapped failure, both roots, Never, and zero-State;
- fused Read bind hot/cold, preparation failure, both ReadAdapterError mappings, one ingress per
  selected Read declaration occurrence per invocation, concurrent winner, and authenticated
  external domain-integrity evidence;
- wrong intent chain and wrong route independently produce adapter `Internal`, enter no provider,
  append nothing, and never manufacture `EvmReadEvidence::IntegrityBlocked`;
- a synthetic child-success path and handler-success path rejoin the same successor under hot and
  cold folding;
- an already returned RunView remains an unchanged valid snapshot when an indeterminate append is
  later observed committed, while a later read may return the advanced view;
- adapter callback panic during future construction and during future polling maps to `Internal`
  with no append and no panic detail in the returned Runtime error;
- known Inserted performs zero loads; NotInserted performs one complete reload;
- start exact retry, collision precedence without foreign assembly association, NotInserted followed
  by absent load, wrong RunId/history, and every Store error mapping;
- drop before/during blocking, provider await, and append await, including a detached
  `spawn_blocking` closure that can do no dependent IO; and
- small compile-fail tests only for surviving private construction/authority boundaries; prove
  broad old lifecycle deletion through the replacement export/manifest scan rather than one
  compile test per retired name.

Architect gate `C2-F`: Runtime/concurrency architect. Require cold/hot proof review, cancellation at
every await, single-erasure/single-qualifier evidence, load-count evidence, and an explicit simpler-
than-current lifecycle/LOC assessment.

### 6.7 Chunk 2G — Portfolio, surviving EVM Reads, and direct live bindings

Primary paths:

- `crates/domains/evm/{src,tests,Cargo.toml,README.md}`
- `crates/domains/portfolio/{src,tests,Cargo.toml,README.md}`
- `crates/live/evm/{src,tests,Cargo.toml,README.md}`

#### Domain-owned physical target

Move the sole `EvmPhysicalTarget` definition from live EVM to `mfm-evm`:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "physical-target",
    version = "1",
    schema = "mfm.evm-physical-target"
)]
pub struct EvmPhysicalTarget {
    chain_id: u64,
    endpoint_ref: ContentRef,
}

impl EvmPhysicalTarget {
    pub fn new(
        chain_id: u64,
        endpoint_ref: ContentRef,
    ) -> Result<Self, EvmDomainError>;

    pub const fn chain_id(&self) -> u64;
    pub const fn endpoint_ref(&self) -> &ContentRef;
    pub fn binding_ref(&self) -> Result<ContentRef, EvmDomainError>;
}
```

Use checked custom Deserialize with unknown-field rejection; constructor and decoder reject chain
ID zero. `binding_ref` calls the one `mfm_values::canonicalize_mfm_value` and returns its instance
ref. It does not duplicate schema/Serde/hash logic.

Freeze exact semantic shorthand `mfm.evm/physical-target@1`, schema
`mfm.evm-physical-target@1`, canonical field names, and a complete value/content-ref golden.
Credentials, client handles, DSNs, and signers are never target fields.

#### Portfolio planning

Replace the wrapper/catalog/configuration flow with:

```rust
pub fn plan_snapshot(
    selector: PortfolioSnapshotSelector,
    config: &PortfolioConfig,
    targets: &[EvmPhysicalTarget],
) -> Result<(Program, PortfolioSnapshotInput), PortfolioError>;
```

Return Program first and typed C0 second. `PortfolioConfig` and its private collection authoring
config become ordinary process-local Serde input; remove MfmConfig/MfmValue derives. The selector
also need not be a persistable MfmValue. The exact secret-free selected demand/route material needed
by execution remains in Program or `PortfolioSnapshotInput`.

Keep PortfolioConfig fields private and replace its plain derived Deserialize with checked custom
Deserialize (unknown fields rejected) that calls the existing config validator, or an equally small
checked constructor used by every ingress. Invalid trusted configuration must not remain
representable after MfmConfig is deleted.

Require `targets` strictly sorted and unique by `chain_id` (`windows(2)` with left < right). Reject
duplicate chain IDs even when endpoints differ. For each collection, select one exact target and put
its derived binding ref in all corresponding child Read declarations and typed C0 route. Delete
`PortfolioAdmissionPlan`, `source_refs`, `snapshot_closure_document`, `EvmBalanceBindings`, and
dummy catalog/closure validation.

Error ownership distinguishes caller selector rejection from trusted composition/planner failure:
selector validation/mismatch is `PortfolioError::InvalidValue`; invalid target coverage/order,
impossible continuation, or Program authoring is `PortfolioError::Program`/internal. Do not collapse
both into one caller-error branch.

Refactor `append_balance_fragment` to use `&EvmPhysicalTarget`/its derived ref and vector-position
u16 successors. It constructs the exact Program declarations directly; no `BindingDescriptor`,
address helper/map, or live-owned planner wrapper remains.

Implement the complete validated `EvmBalanceRequest` unrolling once, against the final implicit
indices. For each source emit the reusable
check-chain -> anchor -> select -> Match -> native-or-(decimals -> token) -> confirm group. A
confirm-success edge targets the next source's check; only the final confirm targets one collection
consolidate State. This repeats occurrences, not State registrations or adapter callbacks. With S
sources the current shape is eight declarations per source plus one consolidate per collection and
remains well inside Program/run bounds.

Implement Program-owned `PureState`/`ReadState` directly for surviving EVM and Portfolio States.
Delete `EvmPureState`, `EvmAccessState`, `PortfolioPureState`, `FailureValue`, and the integrity
projection. Preserve current surviving State/capability/operation stable IDs; semantic numeric-type
renames remain deferred to the follow-up RFC.

The final normal `mfm-evm` manifest is capabilities, IDs, Program, program-derive, Values, serde,
and thiserror. Remove facts and every Runtime/live/Store/IO edge. Canonical/serde_json may remain
dev-only for exact goldens when used.

The ordinary EVM Read interpreter must distinguish
`EvmReadEvidence::IntegrityBlocked` and produce the exact
`EvmBalanceFailure::IntegrityBlocked` before Portfolio's existing mapper. Do not let a generic
`read_returned`/Option helper collapse it into `SourceUnavailable`.

#### Live provider and registration

Remove Runtime call ownership from the provider API:

```rust
pub trait EvmProvider: Send + Sync + 'static {
    fn request<'a>(
        &'a self,
        operation: StableId,
        request_bytes: Vec<u8>,
    ) -> Pin<Box<dyn Future<
        Output = std::result::Result<EvmProviderResponse, ReadAdapterError>,
    > + Send + 'a>>;
}

pub enum EvmProviderResponse {
    Read(EvmReadValue),
    Rejected,
    SafeFailure,
    IntegrityBlocked,
}
```

Delete the public static BoxFuture alias, every `call_id`, echoed operation, redacted code field,
and response-size reserialization. The request future itself correlates the supplied operation; a
provider echo would not independently authenticate it. Delete `Broadcast`/`PossibleEntry` and
`EvmAdapterError` completely. Raw-wire bounds/authentication remain inside the trusted
provider/adapter ingress before it returns this closed typed response. An authentication/integrity
failure safe to conclude is `IntegrityBlocked` only when it is an authenticated external
observation. Timeout, transport failure, and malformed/unauthenticated raw ingress return
`ReadAdapterError::Unavailable`; a trusted local invariant returns `ReadAdapterError::Internal`.
Installer/assembly failure already has the exact Runtime error owner; do not wrap it in a
live-adapter error.

Use one direct installer, not a contribution object:

```rust
pub fn register_evm_reads(
    builder: &mut RuntimeAssemblyBuilder,
    target: EvmPhysicalTarget,
    provider: Arc<dyn EvmProvider>,
) -> mfm_runtime::Result<()>;
```

The installer registers only the three surviving capability callbacks for the target. Trusted
composition separately registers all eight concrete EVM State implementations for
`PortfolioContinuation`: six Read States with `register_read` and the two Pure select-asset/
consolidate States with `register_pure`, plus the Portfolio Pure States. Live IO does not own domain
State registration or a generic cumulative-context parameter. One target may
serve all six current Read States.
Each callback captures the exact provider/target pair, checks typed intent chain and route against
that target before IO, and returns `ReadAdapterError::Internal` with no provider call or append when
they differ. Otherwise it serializes bounded provider request bytes, authenticates/qualifies one
response, and returns typed evidence or the narrow `ReadAdapterError`. Runtime alone applies
`C::bind_evidence` and State interpretation.

Trusted composition is the TCB for pairing an opaque provider handle with its public target.
Changing endpoint target identity changes binding ref and Program; replacing a provider client
under the same public target does not. The installer accepts no asserted binding ref.

Delete `EvmAdapterBinding`, `EvmLiveAssembly`, `EvmBalanceBindings`,
`evm_live_adapter_implementation_ref`, `EVM_LIVE_ADAPTER_ID`, live-owned target/schema/hash helpers,
descriptor/role maps, and live EVM's `mfm-portfolio` and signing dependencies. App registers
no assembly contribution. Trusted embedding/composition registers the Portfolio Pure States,
invokes this EVM installer for each target/provider, finishes Runtime, and only then passes Runtime
plus process-local planning inputs into App.

The final normal live-EVM manifest uses only `mfm-evm`, `mfm-ids`, `mfm-runtime`, and `serde_json`
for bounded request construction. Remove direct canonical, capabilities, Program, Portfolio,
signing, nonce-store, values, serde, and thiserror edges unless the architect identifies a concrete
surviving source use and a smaller owner is impossible.

#### Domain/live verification

- exact physical-target schema/value/ref; zero chain and unknown fields rejected;
- target order/duplicate chain rejection and exact target ref in Program plus C0;
- endpoint change changes target ref and Program; handle replacement does not;
- one target reused across three capabilities/six Read States without duplicate callback keys;
- raw asserted ref cannot compile; missing/wrong/duplicate adapter association is rejected;
- wrong intent route/chain never enters provider, appends nothing, and returns Runtime Internal;
- provider unavailable appends nothing; adapter Internal remains distinct; every accepted evidence
  maps through the ordinary interpreter, and only authenticated external integrity evidence becomes
  durable;
- Portfolio native/token success, hot/cold equivalence, and child failure mapper/root failure; and
- mixed native+token two-source progression/output, second-source failure mapping, 64-source
  authoring/capacity, and 65-source rejection; and
- no old binding/live wrapper, call ID, domain behavior trait, config lifecycle, or submission
  variant remains.

Architect gate `C2-G`: domain/live-boundary architect. Require stable-identity/contract review,
Cargo-graph evidence that live depends on domain + Runtime while the domain has only its frozen
IDs/Values/Program/derive/capability/Serde/error dependencies and no Runtime/live/Store/IO edge,
exact target/provider TCB analysis, and evidence that the direct installer reduces wrapper/maps/
change sites and LOC.

### 6.8 Chunk 2H — thin Application/transports and crate deletion

Primary paths:

- `crates/app/{src,tests,Cargo.toml,README.md}`
- `bin/{cli,rest-api}/{src,Cargo.toml,README.md}`
- `crates/kernel/facts/**`
- `crates/kernel/replay/**`
- root `Cargo.toml` and `Cargo.lock`

#### Minimal retained App

Keep one Portfolio-only facade with no execution lifecycle:

```rust
pub type Result<T> = std::result::Result<T, ApplicationError>;

#[derive(Debug, thiserror::Error)]
pub enum ApplicationError {
    #[error("application request is invalid")]
    InvalidRequest,
    #[error("application internal failure")]
    Internal,
    #[error("runtime operation failed")]
    Runtime(#[source] RuntimeError),
}

pub struct Application {
    runtime: Runtime,
}

impl Application {
    pub fn new(runtime: Runtime) -> Self;

    pub async fn start_portfolio(
        &self,
        run_id: RunId,
        selector: PortfolioSnapshotSelector,
        config: &PortfolioConfig,
        targets: &[EvmPhysicalTarget],
    ) -> Result<RunView>;

    pub async fn resume(&self, run_id: &RunId) -> Result<RunView>;
    pub async fn read(&self, run_id: &RunId) -> Result<RunView>;
}
```

`start_portfolio` calls `plan_snapshot`. Selector rejection or selector/config mismatch maps to
`InvalidRequest`; trusted target coverage/order, impossible continuation, Program authoring, or a
wrong fixed entry-point invariant maps to the App-owned redacted `Internal`. Only an error actually
returned by Runtime is wrapped as `ApplicationError::Runtime(error)`; do not add a blanket
conversion or manufacture `Runtime(RuntimeError::Internal)` for an App fault. It verifies the fixed
`PORTFOLIO_SNAPSHOT_ENTRY_POINT_ID` equals
`Program.entry_point_id`, and calls
`Runtime::start` with the caller-supplied RunId. `resume` and `read` delegate directly. App never
generates a RunId, accepts a separate Runtime entry selector, opens Store, registers an assembly,
or inspects Journal frames. Application owns only Runtime; checked PortfolioConfig and targets are
borrowed for start and need not be reconstructed or retained for later resume/read. A cold Runtime
still requires the exact target/provider associations registered in its assembly.

The normal App dependency target is only IDs, EVM domain target, Portfolio, Runtime, and error
support. Remove direct dependencies on canonical, capabilities, Journal, Program, replay, Store,
values, live EVM, and signing unless one is proven necessary by this exact facade. Trusted process
composition constructs Runtime and its live adapter registrations before Application.
Integration tests may retain live EVM, Memory/PostgreSQL Store, and Tokio as dev-only composition
dependencies; they must not leak into normal App ownership.

Delete all current App lifecycle/transport DTOs and helpers:

```text
MAX_ADMISSION_BYTES / parse_admission_json
AdmitRunRequest without explicit RunId / AdmitRunResponse / DriveResponse
App RunStatus / PublicRunView
ReplayResponse / TraceResponse / TraceRecord
AccessAuditResponse / AccessAuditEntry / ExportedRun
reader/config/audit ports and configuration heads
suspended/pending owner map and retained-owner resolution
frame-derived status/reducer/terminality
export/replay/trace/audit methods
application_catalog / register_catalog_values / validate_runtime_closure
dummy Program/closure document validation
```

Do not create new App response/status wrappers around `RunView`. Transport rendering can match the
trusted view and map the one redacted App/Runtime error contract at its boundary.

#### Facts, replay, and workspace

Delete `crates/kernel/facts/**` and `crates/kernel/replay/**` wholesale, their workspace members,
features, tests, READMEs, every normal/dev dependency and Cargo.lock package edge. Delete every
fact selection/publication/proof/storage/schema concept from capabilities, Program, Journal, Store,
Runtime, domains, App, SQL, fixtures, and tasks; do not leave an empty traits crate.

Runtime `read` is the one Store-backed semantic inspection path. Do not replace mfm-replay with a
portable bundle, trace reducer, offline inspector, raw Store port, or App frame projection.

The workspace member count moves from 21 at the original baseline to 20 after Commit 1 and 18 after
facts/replay deletion. Any other package addition is outside this plan.

#### CLI and REST

The current binaries have no supported run-progression route. Keep them honest transport-only
metadata binaries or remove stale commands; do not invent a partially composed live start route in
this cutover. Their docs must not advertise nonce/signer/submission composition, replay/export,
implicit RunId generation, or App-owned statuses.

Any future start route must parse and echo an explicit RunId before execution, select a typed entry
point, and call the Application contract above. It must not recover the deleted generic
Admit/Drive JSON DTOs.

#### App/workspace verification

- start requires explicit RunId and dispatch EntryPointId equals retained Program field;
- invalid selector maps to `ApplicationError::InvalidRequest`, trusted target/Program/planner
  failure maps to `ApplicationError::Internal`, and an actual Runtime Absent remains exactly
  `ApplicationError::Runtime(RuntimeError::Absent)`;
- exact admission retry and actual Runtime errors pass through the redacted App error mapping;
- Application is constructed from Runtime alone; after a successful start/composition test drops
  every authoring Config/target slice and proves cold resume/read still succeeds;
- resume/read perform no frame inspection and return Runtime RunView directly;
- no App method can hold a session/pending owner or invoke replay/export/trace/audit;
- CLI/REST command/help/output contracts match the retained metadata-only surface;
- Cargo metadata has exactly 18 workspace members and no facts/replay package or edge; and
- production export/reexport/manifest scans prove the complete old App DTO/lifecycle surface is
  absent; do not add a large compile-test harness that only repeats the deletion inventory.

Architect gate `C2-H`: App/boundary architect. Require a dependency and public-export audit, proof
that App is a thin call facade rather than a lifecycle, and a challenge of whether every retained
method/field is needed.

### 6.9 Chunk 2I — schemas, fixtures, documentation, tasks, and absence proof

Primary paths:

- `AGENTS.md`, `README.md`, root manifests, Cargo.lock, `flake.nix`, and `nixfied.nix`
- `.github/`, `.config/`, `nix/`, and `scripts/check-cutover-manifest.sh`
- current-design docs and every affected crate/bin README
- stale current-source contract comments, including `crates/kernel/canonical/src/limits.rs`
- contract and wire/schema golden fixtures
- old/new cutover manifests

#### Current-design documentation

Rewrite, in the same core commit:

```text
AGENTS.md
README.md
docs/design.md
docs/architecture.md
docs/build-and-verification.md
docs/run-execution.md
docs/persisted-public-surfaces.md
docs/known-gaps.md
docs/evm-rpc-routing.md
docs/evm-portfolio-contract-freeze.md
docs/portfolio-snapshot.md
crates/kernel/{ids,values,program-derive,capabilities,program,journal,store,runtime}/README.md
crates/domains/{evm,portfolio}/README.md
crates/live/evm/README.md
crates/storages/postgres/README.md
crates/app/README.md
bin/{cli,rest-api}/README.md
```

`AGENTS.md` remains the repository agent source of truth but must stop requiring deleted facts,
configuration, replay, manifest/snapshot, or old Store/Runtime owners. Preserve the general
append-only, atomicity, content-addressing, no-float, no-ambient-IO, secret-exclusion, Nix, code-
quality, and architect-review rules under their new owners.

The docs must describe only the final Pure/Read Runtime, index-zero State/Match Program, exact root
failures, fused Read frame, one Runtime fold, two-method Store, three-table PostgreSQL baseline,
direct EvmPhysicalTarget binding, Portfolio-only App, explicit RunId contract, cancellation safety,
and unsupported writable rewind. Do not document an old type as an alternative or migration path.

Delete:

```text
docs/preflight/capability-binding-inventory.md
docs/preflight/capacity-envelope.md
docs/preflight/cumulative-context-abi.md
docs/btc-rpc-routing.md
```

Commit 1 already deleted `docs/evm-transactions.md` and the six submission result fixtures. Preserve
the three surviving public result fixtures byte-for-byte in both commits:

```text
docs/contracts/evm-portfolio/evm-balance-collection.json
docs/contracts/evm-portfolio/portfolio-snapshot.json
docs/contracts/evm-portfolio/portfolio-snapshot-failure.json
```

These are public result projections, not Program/Journal wire. Reset only Program/C0/Journal
execution goldens and add the physical-target golden.

#### Authority files and replacement scanner

Delete only:

```text
SINGLE_TRUST_BOUNDARY_CUTOVER_MANIFEST.md
IMPL_PLAN_RFC_REFACTOR_SINGLE_TRUST_BOUNDARY.log
```

The second path is an absence target even if already missing. Keep the header-marked archival prior
RFC, old implementation plan, REQUIRED_FIXES, FINISH handoff, and
`RFC_FOLLOWUPS_FROM_REFACT_RUNTIME.md` as tracked archives. They are not implementation inputs and
are outside the supported-current scan.

Add `RUNTIME_STORE_JOURNAL_CUTOVER_MANIFEST.md`, generated/reviewed from RFC Section 11 and this
plan's per-chunk additions. Rewrite `scripts/check-cutover-manifest.sh` in place and keep task ID
`negative-scan`.

The scanner examines explicit supported-current roots:

```text
Cargo.toml  Cargo.lock  README.md  AGENTS.md  flake.nix  nixfied.nix
.github/  .config/  nix/  bin/  crates/  docs/  scripts/
```

It uses category-specific scopes:

- exact path absence for deleted crates, docs, fixtures, migrations, and authority files;
- package/dependency patterns over Cargo.toml and Cargo.lock;
- stable/schema/wire/SQL/task/current-claim patterns over production `src`, Cargo package
  descriptions, migrations/build files, current docs, Nix, scripts, and workflows;
- public export/reexport patterns over production library roots; and
- explicit partial-enum/field/SQL-column checks where a surviving type/table loses variants.

Tests and hostile fixtures may name retired types/tags/fields to prove rejection. Keep those in
external test/fixture roots or narrowly allowlisted locations; they may not export/register them.
The retired submission entry-point negative App regression exists only at the Commit 1 boundary;
delete it with generic dispatch in Commit 2 because the final typed App accepts no arbitrary entry
ID. Reviewed retirement prose may still name the retired ID. Do not weaken the production scan
with a repository-wide blanket exclusion or force deletion of hostile old-wire evidence.

The manifest includes every RFC Section 11 literal plus the additional exact current exports found
by the implementation inventory: dead identity/schema families, Journal open DTOs, Store backend
outcomes, App DTOs, EVM partial variants/IDs, concrete PostgreSQL constructors/errors, fact/replay
packages, and configuration constants. Renaming an equivalent owner still fails architect review
even if a literal scanner cannot recognize it.

Transferred names are scoped to the removed owner or variant, not globally banned. For example,
the scanner removes capability-owned `ProposedStateOutcome` but permits the one Program-owned type,
removes Store-trait `StoreOpenError` but permits the concrete PostgreSQL open error, removes the
live-owned/wrapper target but permits domain-owned `EvmPhysicalTarget`, and rejects only
`RunView::Waiting` plus `AppendResult::{Existing,Stale}` while retaining the smaller final enums.
Apply this owner/variant rule to every transferred RFC Section 11 entry.

Test the scanner by creating then removing one temporary supported-root fixture for each class:
path/package, public export, stable/wire, SQL/dependency, and current-doc claim. Each injection must
fail and the clean tree must pass. Do not check in stale canary files.

#### Nixfied and task graph

Keep public task IDs but rewrite their work:

- `capacity-app`: Portfolio-only public result/Program/C0 coverage, no submission filter;
- `capacity-runtime`: hot/cold Pure/fused-Read/Match/failure/zero-State coverage and load counts;
- `capacity-store`: actual frame/count/cumulative Store limits, no configuration/preflight owner;
- `capacity-envelope`: compose the three current capacity owners and exact Journal theorem fixture;
- `postgres-test`: managed DB, private ignored PostgreSQL lib tests with `--include-ignored`, no
  `test-support` feature;
- `negative-scan`: new manifest/scanner; and
- `ci`: the one final composed graph with the current model.

Run `nix run .#model-check` early after task edits. Update `docs/build-and-verification.md` to match
the executable graph rather than listing deleted capacity/configuration tasks.

Retain the `pkgs.jq` and `pkgs.ripgrep` default development tools added in Commit 1. Section 9's
metadata/surface evidence must remain pinned. These are development-only tools and create no
Rust/package ownership edge.

Do not parameterize production limits for cheap tests. Exercise exact arithmetic/bound-plus-one in
unit tests and analytical/golden fixtures, then use small integration histories to prove wiring.
The routine CI need not allocate a 512 MiB/65,536-frame run. Keep any one-time full-bound benchmark
manual/ignored and report it separately; this is not a reason to weaken or duplicate constants.

#### Chunk verification

- every changed link/example/command claim resolves;
- all three surviving public result fixtures remain exact;
- Program/Journal/target/schema goldens match the approved wire;
- Cargo metadata and workspace member/dependency counts match the final graph;
- new negative-scan injection tests and clean scan pass;
- model-check admits the new task graph; and
- `git diff --check` is clean.

Architect gate `C2-I`: documentation/build/cutover architect. Require source-of-truth consistency,
manifest scope/adversarial injection review, exact archive-vs-current boundary, and proof that task
and documentation rewrites removed duplicate authority rather than recording another lifecycle.

### 6.10 Commit 2 integration and final architect gate

After `C2-A` through `C2-I` approve:

1. run a clean public-export/dependency/schema/deletion inventory over the integrated tree;
2. have a non-author architect trace one zero-State run, one Pure path, one multi-source Read path,
   one mapped child failure, one concurrent append race, one cancellation at append, and one cold
   resume from PostgreSQL through every owner;
3. require that architect to challenge every new public type, dependency edge, registry/map,
   wrapper, clone, reload, and production line added by the cutover;
4. resolve every block without a compatibility path;
5. run the focused checks in Section 9, then the one final `nix run .#ci`; and
6. create exactly:

```text
establish the typed runtime journal and store proof path
```

No chunk is a cherry-pickable deployment unit. The commit contains all docs, migration, task,
fixture, manifest, and Cargo.lock changes.

## 7. Mandatory deletion ledger

RFC Section 11 is the literal authoritative deletion list. The new cutover manifest must import
every item from it; an item is not optional because this plan groups it under an owner instead of
repeating its spelling. The path ledger below assigns deletion and review responsibility so no item
falls between chunks.

| Owner/chunk | Delete completely | Replacement, if any |
| --- | --- | --- |
| 2A IDs | Effect/capability kind/version, control address, Store scope/epoch/tenant, append/request, field-path, semantic-digest and dormant checked-string families | retained general IDs only |
| 2A Values/derive | MfmConfig/ValidatedConfig/config errors, old State/input/output markers and four derives, planning/fact schema variants, duplicated value canonicalizer | one MfmValue path, Never exception, one tuple qualifier |
| 2A Capabilities | Access/Read/Effect modes, attempts, facts, evidence markers, capability outcome/qualified evidence | exact ReadCapabilityContract only |
| 2B Program | catalog/builder/document/ingress/ref/reifier/brand/qualified-value APIs; addresses/maps/cycle/root inference; optional/default failure; binding/config/fact/effect fields; public wire DTO | one checked Program v2 plus direct State traits |
| 2C Journal | open frame/record/object/outcome DTOs; preparation/Access/occurrence/call fields; record digest; dictionary/object-count; config/fact/portable codecs | sealed frame, opaque bytes, qualified history/views |
| 2D Store | semantic reducer/selection/ports/brands/owners; backend command/result layers; readiness/list/audit/config/fact APIs; expected position/request/receipt/reservation/global object/accounting | two-method Store plus Memory |
| 2E PostgreSQL | broad error/profile/open/migrate/rotate/check APIs; every old table/query/column/index for identity/config/fact/request/receipt/reservation/object membership | checked connect and three-table baseline |
| 2F Runtime | Dynamic/Access/preparation/CommittedCall/owner/session/step/pending/Waiting/limits/semaphore/cancellation/finalizer types; ReadUnavailable; public implementation/driver/BoxFuture/downcast paths | one private driver/fold, ReadAdapterError, and RunView |
| 2G domains/live | duplicate behavior traits; BindingDescriptor/binding/live assembly wrappers; live-owned target; call IDs/provider submission branches; domain config lifecycle | standard State traits, direct target, callback-only installer |
| 2H App/workspace | generic Admit/Drive/status/replay/trace/audit/export DTOs; suspended map/frame decoder/catalog validation; facts/replay crates | Runtime-only three-method Portfolio App facade with borrowed start inputs |
| 2I docs/build | old cutover manifest/log, stale preflight/BTC docs, old schemas/goldens/tasks/current claims | one absence manifest/scanner and current docs |

The following absent responsibilities must not reappear under a different name:

- Effect or mutating-provider semantics, durable preparation/replacement/call authority, retries,
  or a synthetic outbox;
- independent configuration publication/latest lookup/stream/head/revision;
- fact selection/publication/proof/tree/log/storage;
- portable run bundle/export/offline inspection or another reducer;
- tenant/scope/epoch/incarnation/active identity/rotation;
- random append/request ID, command/receipt/result-position table, reservation/liability state, or
  global object membership;
- Store-side Program decode/reduction/reification or App-side frame interpretation;
- a public address, Program catalog/document wrapper, binding descriptor wrapper, implementation
  holder, lifecycle/session, raw history, or second value qualification path; and
- EVM nonce allocation, candidate/sign/broadcast/status ownership.

Deletion proof is structural. Private test fakes may inject corruption or retired wire spellings,
but they cannot be exported, registered, compiled into production paths, or described as supported.

## 8. Activation, rollback, and data boundary

Commit 1 rollback is whole-commit only. Re-enabling EVM submission requires renewed product
authorization and a separately approved durable transaction-authority RFC; do not keep a disabled
route, tombstone DTO, or feature flag to make rollback easy.

Commit 2 is a fresh-baseline release cutover:

1. quiesce all writers using the old authority;
2. provision a fresh database with only `mfm.run-history-postgres.v1`;
3. deploy the new binary/assembly against that database; and
4. admit only new RunIds under the new Program/Journal wire.

There is no in-place converter, old-byte reader, dual read/write, rolling mixed-version deployment,
feature flag, or per-chunk activation. An old database may be retained out of band as an unsupported
read-only archive, not attached to the new product.

A release rollback uses the old binary with its old database snapshot, or the new binary with its
fresh new database. Never run old code on new rows or new code on the old baseline. Never copy
acknowledged new rows into an older writable timeline. Internal Chunks 2A–2I are review slices, not
deployment or rollback units.

The supported new durability timeline remains no-loss and monotonic. Writable rewind, PITR to an
older acknowledged state, data-loss failover, or a writable historical clone is outside the safety
claim and must not be presented as recovery.

## 9. Verification, measurable simplification, and handoff evidence

Verification is scope-driven. Use focused Cargo commands in the default Nix shell while a chunk is
changing, run model admission early when the task graph changes, and run the one composed CI gate
once on each completed commit. Do not run `.#check`, `.#test`, `.#test-db`, or all of
`negative-scan`/`capacity-envelope` immediately before `.#ci` on the same revision merely to repeat
its work.

### 9.1 Freeze the baseline and use identical measurements

Capture evidence at exactly three revisions:

1. the parent of Commit 1;
2. Commit 1; and
3. Commit 2/final.

Use the same commands and roots at all three revisions. Run the following after entering
`nix develop`; the Cargo command must not use a host toolchain. For the parent measurement, run
these commands in a parent-revision worktree from Commit 1's pinned dev shell as specified in
Section 5.4:

```bash
metrics_dir=$(mktemp -d)
cargo metadata --format-version 1 --no-deps > "$metrics_dir/metadata.json"

jq -r '.workspace_members | length' "$metrics_dir/metadata.json"

jq -r '
  [.packages[] as $package
   | $package.dependencies[]
   | select(.source == null and .kind == null)
   | select(.name | startswith("mfm-"))
   | [$package.name, .name]]
  | unique | sort | .[] | @tsv
' "$metrics_dir/metadata.json" \
  | tee "$metrics_dir/normal-edges.tsv"
wc -l < "$metrics_dir/normal-edges.tsv"

git ls-files \
  ':(glob)crates/**/src/**/*.rs' \
  ':(glob)bin/**/src/**/*.rs' \
  | xargs wc -l

git ls-files \
  ':(glob)crates/**/tests/**/*.rs' \
  ':(glob)bin/**/tests/**/*.rs' \
  | xargs wc -l

git ls-files ':(glob)crates/**/src/**/*.rs' \
  | xargs rg -n --no-heading \
      '^pub(?:\s+async)?\s+(?:struct|enum|trait|type|fn|const|static|use)\b' \
  | rg -v ':pub mod ' \
  | tee "$metrics_dir/workspace-public-api.txt"
wc -l < "$metrics_dir/workspace-public-api.txt"

proof_roots=(
  crates/kernel/ids/src
  crates/kernel/values/src
  crates/kernel/capabilities/src
  crates/kernel/program/src
  crates/kernel/journal/src
  crates/kernel/store/src
  crates/kernel/runtime/src
  crates/domains/evm/src
  crates/live/evm/src
  crates/app/src
)

git ls-files "${proof_roots[@]}" \
  | rg '\.rs$' \
  | xargs wc -l

rg -n --glob '*.rs' \
  '^pub(?:\s+async)?\s+(?:struct|enum|trait|type|fn|const|static|use)\b' \
  "${proof_roots[@]}" \
  | rg -v ':pub mod ' \
  | tee "$metrics_dir/proof-public-api.txt"
wc -l < "$metrics_dir/proof-public-api.txt"
```

The normal-edge evidence is the sorted unique pair list, not only its count. Both public-surface
commands are mechanical cross-checks for top-level public declarations/reexports, including
`pub async fn` and excluding `pub mod`. The workspace-wide count is the blocking public-surface
metric; the ten-root count localizes the proof-path change. At every revision the architect must
also record and review the logical exported crate-root API ledger because formatting, private-module
placement, or moving a wrapper into an uncounted crate must not game either metric. Source-tree LOC
deliberately includes inline `#[cfg(test)]` modules. Report external `tests/` LOC separately so
moving tests inline cannot look like product simplification. Remove `metrics_dir` after recording
the redaction-safe counts/lists.

The approved starting baseline is:

| Measure | Parent of Commit 1 |
| --- | ---: |
| Workspace members | 21 |
| Normal internal MFM dependency edges | 82 |
| Tracked `crates/**/src` + `bin/**/src` Rust LOC | 32,401 |
| Tracked external crate/bin test Rust LOC | 7,424 |
| Workspace-wide library top-level public declarations/reexports, excluding `pub mod` | 392 |
| Ten proof-path crate source LOC | 25,249 |
| Ten proof-path top-level public declarations/reexports, excluding `pub mod` | 314 |

If rerunning the exact commands on the implementation base disagrees, stop and explain the base
revision or command drift before coding; do not silently replace the baseline.

Required reduction evidence:

- Commit 1 has exactly 20 workspace members, removes at least 24 inventoried source-level exports,
  deletes the 292-LOC nonce PostgreSQL crate, and removes at least the three normal edges
  `mfm-evm-live -> mfm-signing`, `mfm-evm-live -> mfm-storage-evm-postgres`, and
  `mfm-storage-evm-postgres -> mfm-ids` without adding a replacement submission edge.
- Commit 2/final has exactly 18 workspace members. Deleting facts and replay removes another 763
  source-tree LOC and 19 top-level public declarations. Across both commits, whole-crate deletion
  alone is at least 1,055 source-tree LOC. All whole-crate and in-place owner cuts directly
  inventory at least 43 public-declaration deletions before counting the larger
  kernel/Runtime/App surface reduction.
- The final source-tree LOC, workspace-wide public count, ten-crate proof-path LOC, ten-crate public
  count, and normal internal edge count are each strictly lower than both the original and Commit 1
  values. Tracked source-tree Rust additions across the two implementation commits must be fewer
  than tracked source-tree Rust deletions.
  Report test, documentation, SQL, and fixture churn separately; do not hide generated or moved
  code from the narrative.
- A manually separated default-production Journal + Store + PostgreSQL Rust/SQL total above 3,000
  LOC is an architect review trigger, not a mechanically gamed test-placement gate: the reviewer
  must identify the production/test split and justify every excess owner/path. More than 18 logical
  top-level public items is an architect `BLOCK` until an indispensable invariant is identified.
- Every final normal dependency edge must occur in the exact per-crate target manifests in this
  plan. A strict aggregate decrease is necessary but does not excuse one unowned edge.

For dexterity, record the exact files and registrations changed to add a representative Pure State
and a Read State using an existing capability/target. The final path should be domain behavior plus
trusted composition registration; it must not require Journal, Store, Runtime reducer, App,
schema, or binding-registry edits. Also record the files changed for one new Read adapter target;
that change must stay in the domain target/live constructor/composition path.

### 9.2 Commit 1 focused verification and definition of done

Run the narrow package checks while the three chunks are being reviewed:

```bash
nix develop -c cargo fmt --all -- --check
nix develop -c cargo test -p mfm-evm --all-targets
nix develop -c cargo test -p mfm-portfolio --all-targets
nix develop -c cargo test -p mfm-evm-live --all-targets
nix develop -c cargo test -p mfm-app --all-targets
```

Run `mfm-signing`/`mfm-keystore` focused tests only if their code or manifests changed. After the
Nixfied filter/workspace graph changes, run `nix run .#model-check` and
`nix flake check --no-build` early. While iterating, run the Portfolio-only `capacity-app`,
temporary legacy `negative-scan`, and the explicit production-scope
submission identity/path/schema/SQL/dependency checks from Sections 5.2–5.4. Then run
`git diff --check` and exactly one final:

```bash
nix run .#ci
```

Commit 1 is ready only when:

- `C1-A`, `C1-B`, `C1-C`, and the non-author combined-diff architect all say `APPROVE` against the
  same proposed commit tree;
- existing surviving Portfolio native/token/failure behavior remains unchanged;
- no production source, export, registration, fixture, task, SQL object, or dependency can submit,
  allocate nonce, sign for submission, broadcast, or poll submission status;
- all listed current product docs describe Portfolio-only operation and the four superseded
  pre-cutover artifacts (three preflight files plus the BTC routing note) carry their
  non-authoritative status;
- the retired literal occurs only in narrow negative-test/retirement allowlists;
- member/edge/export/LOC evidence meets the Commit 1 floor; and
- the focused and final commands, exit status, and revision are recorded.

### 9.3 Commit 2 focused verification matrix

Exercise each reviewed slice in producer-to-consumer order. Prefer an individual failing test
filter during development; before that slice's architect approval, run its affected package set:

```bash
# 2A — identities, values, derives, capabilities
nix develop -c cargo test \
  -p mfm-ids -p mfm-values -p mfm-program-derive -p mfm-capabilities \
  --all-targets

# 2B–2C — checked Program and exact Journal
nix develop -c cargo test -p mfm-program -p mfm-journal --all-targets

# 2D–2E — Memory/Store and non-DB PostgreSQL tests
nix develop -c cargo test -p mfm-store --all-targets
nix develop -c cargo test -p mfm-storage-postgres --lib

# 2F — sole Runtime fold and cancellation/error behavior
nix develop -c cargo test -p mfm-runtime --all-targets

# 2G — surviving domains and live Read adapters
nix develop -c cargo test \
  -p mfm-evm -p mfm-portfolio -p mfm-evm-live \
  --all-targets

# 2H — thin App plus metadata-only binaries
nix develop -c cargo test -p mfm-app --all-targets
nix develop -c cargo check -p mfm -p mfm-rest-api --all-targets
```

After the task-model edit, run `nix run .#model-check` early. Use the managed database lane for the
ignored same-crate PostgreSQL integration tests; it must actually execute them and must not silently
pass when no database exists:

```bash
nix run .#run -- --task postgres-test
```

While iterating, run the rewritten capacity and absence contracts:

```bash
nix run .#run -- --task capacity-envelope
nix run .#run -- --task negative-scan
```

The absence task's own temporary-canary tests must demonstrate failure for every scoped class and
then pass on the clean tree. The PostgreSQL lane must cover the fresh schema/readiness/reconnect,
snapshot corruption, exact-row receipt, concurrency, and COMMIT classification cases. Arithmetic
and exact fixtures own the 8/25/512 MiB and frame-count boundaries; routine CI need not allocate a
full maximum run.

Before final approval, run `nix develop -c cargo fmt --all -- --check`, regenerate the exact
metadata/LOC/export/schema evidence, and run `git diff --check`. When focused failures are resolved,
do not immediately repeat broad component gates; run exactly one final composed gate on the exact
candidate commit tree:

```bash
nix run .#ci
```

### 9.4 Commit 2 definition of done

Commit 2 is ready only when:

- every `C2-A` through `C2-I` review is `APPROVE`, every `BLOCK` has a linked correction and
  rereview, and a different non-author architect approves the whole cumulative diff;
- the exact public signatures, Program/Journal wires, limits, schema marker/tables, Runtime mapping,
  and domain target goldens match the RFC and this plan;
- hot/cold Pure, fused Read, Match, mapped failure, Never, zero-State, concurrent winner,
  cancellation, and Store ambiguity/corruption tests all pass;
- the synthetic child-success/handler-success hot-and-cold rejoin, immutable earlier-RunView after
  late indeterminate commit, and cold App resume/read after dropping authoring Config/targets tests
  all pass explicitly;
- the replacement manifest covers every deletion-ledger item, its scoped scanner self-tests pass,
  and supported current docs contain no alternate owner or stale claim;
- Cargo metadata has exactly 18 members, only justified target dependency edges, and no facts,
  replay, or nonce-store package/edge;
- the new PostgreSQL baseline has only the three specified MFM Store tables, reconnect gating and
  exact transaction semantics pass, and no old baseline/migration path is reachable;
- original/Commit 1/final complexity evidence shows strict net simplification and the architect's
  dexterity trace finds no cross-owner State change site; and
- all focused checks, `git diff --check`, and the single final CI result are recorded against the
  final revision.

### 9.5 Architect and engineer handoff record

Do not add permanent review-log machinery to Runtime or Store. In the implementation handoff/PR,
record for each chunk:

```text
Chunk:
Author revision:
Architect reviewer:
Decision: APPROVE | BLOCK
Correctness findings:
Simplicity / net-LOC findings:
Dexterity findings:
Deletion evidence:
Verification reviewed:
Material uncertainties: none | <blocking item>
```

The final engineer report leads with the two commit SHAs and outcomes, then lists:

1. major owners refactored and superseded paths/types physically deleted;
2. architect decisions and resolved blocks by chunk;
3. focused and final verification commands/results;
4. original/Commit 1/final members, edges, LOC, public surfaces, storage budget, and State-change
   sites using the frozen measurement method;
5. final Program/Journal/schema/golden identities and reset data boundary; and
6. any unrun check or external limitation.

Do not call the work complete with an unrun required gate, unexplained metric regression, open
architect block, or compatibility residue. Any implementation discovery that changes architecture,
ownership, the persisted contract, product scope, or the two commit boundaries reopens the single
Material uncertainties section and triggers the stopping rule in Section 0 before more code is
written.
