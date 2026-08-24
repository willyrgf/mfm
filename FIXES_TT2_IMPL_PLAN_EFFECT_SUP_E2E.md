# TT2 Effect support and EVM lifecycle cleanup implementation plan

## Status

This is the engineer-agent handoff for a second corrective pass over the unmerged
`effect-w-e2e` branch. It targets `0f3e790dc` and the complete thirteen-commit Effect series from
`8f3d215ba` through `0f3e790dc` relative to `origin/dev` at `f493a0ca1`.

This pass is a reduction and boundary-quality change. It does not reopen the durable Effect
protocol, EVM transaction semantics, append-only authority, PostgreSQL durability model, signing
crate ownership, anchored Read, or managed Reth proof. The current implementation passes CI and the
core completion ledger is clean; the remaining problem is retained implementation records,
unnecessary public/test types, parallel test models, and implementation-detail assertions.

This file is a handoff input, not another permanent implementation record. Keep it untracked unless
the user explicitly requests otherwise. The implementation commits delete the two tracked completed
plans, but must not add this file. A final worktree may therefore contain only this intentionally
preserved untracked handoff file.

## Outcome

After this cut:

- `docs/design.md`, `docs/architecture.md`, focused current docs, and crate READMEs remain the only
  maintained design descriptions;
- `crates/signing` remains the small reusable transient recoverable-secp256k1 port;
- the concrete in-process key map named `Keystore` becomes private while `KeystoreOwner`,
  `KeystoreSigner`, and `SecretSecp256k1Scalar` remain the public custody surface;
- negative signing and keystore trait contracts use compact compile-time assertions instead of 22
  Rust/compiler-stderr snapshot files;
- the transaction provider returns the existing `EvmChainInstance` instead of a second
  `ObservedChainInstance` projection;
- live EVM tests retain the safety matrix without reproducing PostgreSQL authority validation or
  freezing complete helper-call traces;
- Runtime tests retain every ambiguity, cancellation, concurrency, retained-history, and error
  mapping regression through fewer Store and assembly fixtures;
- the contract lifecycle e2e remains one independent real-Reth deploy/configure/read/report proof;
  and
- the candidate is materially smaller without deleting durability, cryptographic, persistence, or
  external-chain coverage.

The expected reduction is approximately 3,800-4,200 current-tree lines. This is a review aid, not an
acceptance quota. Acceptance is based on deleting the named concepts and duplicate fixtures while
preserving the exact behavior and tests listed below.

## Reviewed baseline

At handoff, the full branch changes 104 files with 18,141 additions and 1,127 deletions relative to
`origin/dev`:

| Surface | Net lines |
| --- | ---: |
| Completed root implementation plans | +3,275 |
| Production and build code | +6,302 |
| Tests and fixtures | +7,176 |
| Maintained documentation | +261 |
| Total | +17,014 |

The seven first-pass corrective commits change 59 files with 6,113 additions and 2,319 deletions
relative to `origin/effect-w-e2e`. Their net composition is:

| Surface | Net lines |
| --- | ---: |
| Completed root implementation plans | +3,275 |
| Production and build code | -225 |
| Tests and fixtures | +716 |
| Maintained documentation | +28 |
| Total | +3,794 |

The first-pass production design therefore did shrink. The apparent six-thousand-line corrective
expansion is primarily the two plans and added test scaffolding. In `0f3e790dc`, 3,275 of 3,584
additions are the two completed plan files; the remaining code and maintained-doc change is net
negative.

Concrete remaining reduction targets are:

| Target | Current cost or problem |
| --- | --- |
| `IMPL_PLAN_EFFECT_SUP_E2E.md` and `FIXES_IMPL_PLAN_EFFECT_SUP_E2E.md` | 3,275 tracked historical/checklist lines duplicating current docs |
| signing and keystore `tests/ui` trees | 22 source/stderr files and 224 lines for eleven negative-trait exclusions |
| public `Keystore` | public but has no public constructor, method, return path, or consumer |
| `ObservedChainInstance` | duplicate public `(chain_id, genesis hash)` representation |
| live `MemoryAuthority` | repeats authority transition validation already owned by the port/PostgreSQL implementation |
| exact live operation arrays | freeze positive helper counts and incidental complete order |
| Runtime `RetainedStore` | duplicates behavior already present in `ScriptedStore::with_retained` |
| Runtime ambiguity scenarios | four nearly identical assembly/start/resume blocks |
| Runtime retained-history corruption setup | repeats frame parsing, object replacement, ordering, canonicalization, and assembly |

## Frozen architecture decisions

| Concern | Keep | Delete or change |
| --- | --- | --- |
| Generic Effect | Program v3, Journal frame v2, durable prepare, Runtime-owned `EffectId`, `Pending`, caller-driven recovery | no protocol or wire change in TT2 |
| EVM transaction | nonce-free command, account-bound binding, typed settlement, exact retained raw bytes | no transaction-value redesign |
| Signing | `crates/signing` as the implementation-independent object-safe port | do not merge it into keystore, live EVM, capabilities, IDs, or values |
| Keystore | private thread-affine key map, unique owner, least-authority signer handles | public visibility of the unreachable inner `Keystore` type |
| Nonce authority | separate port crate and optional separately gated PostgreSQL handle | no generic event store, Store coupling, or in-memory production authority |
| Chain identity | existing persisted `EvmChainInstance` | transient duplicate `ObservedChainInstance` only |
| Anchored Read | reusable Program-visible context/intent/evidence/State and live JSON-RPC adapter | do not inline it into the e2e or defer it |
| Live reconciliation | one receipt, one canonical equality observation, exact-byte resubmission | complete helper traces and fake-owned validation |
| Runtime tests | observable append ambiguity, cancellation, concurrency, hot/cold, evidence binding | duplicate Store implementations and repeated scenario setup |
| E2E | pinned Solidity/solc, PostgreSQL, Reth, independent observer, bounded cold reconstruction | no reduction that weakens independent external proof |

The anchored Read is not treated as test-only. The requested lifecycle requires a real Read after two
Effects, and `docs/design.md` makes its context-preserving contract part of the current platform
design. Production composition intentionally does not register the development-only transaction
capability yet; lack of a current product consumer is not grounds to replace the Read with a
test-local State.

## Non-goals

Do not use TT2 to:

- change Program v3, Journal frame v2, Runtime frame counts, `EffectId`, or `Pending` semantics;
- change any MFM schema identity, canonical JSON field, transaction command/evidence wire, authority
  marker, SQL relation, ACL, advisory-lock preimage, or migration;
- rename `expected_genesis_hash` in the persisted `EvmChainInstance`; deleting the duplicate live
  type does not justify persisted identity churn;
- change EIP-1559 encoding, Keccak, low-S validation, sender derivation, exact raw limits, nonce
  reservation, receipt validation, or settlement policy;
- add production finality, confirmation depth, reorg handling, replacement, fee bumping, rollback,
  key persistence, or key recovery;
- register development transaction Effects or anchored transaction-route Reads in
  `ComposedRuntime`;
- delete `crates/signing`, `crates/keystore`, `crates/domains/evm-transaction-authority`, or any
  PostgreSQL authority table/gate;
- move test helpers into production crates merely to reduce a test-file line count;
- replace independent Reth assertions with the adapter under test;
- add a generic mock framework, test-support crate, Cargo feature, or new production dependency;
- weaken redaction, secret-surface, thread-affinity, exact schema/ACL, ambiguous-commit, or hostile
  wire tests; or
- optimize only the diff statistic by hiding code behind macros or dense unreadable helpers.

## Preserved safety and ownership invariants

Every commit must preserve all of these:

1. Runtime appends the complete Effect command and derived `EffectId` before adapter entry.
2. `Pending` appends no conclusion, performs no internal retry, and returns Runnable.
3. Adapter errors leave the same retained prepare and preserve their typed Runtime mapping.
4. Hot and cold folds validate the same Program, frame, command, evidence, and outcome contracts.
5. Store remains mechanical and has no Program, domain, reducer, capability, or Effect semantics.
6. One semantic EVM transaction may be submitted repeatedly only as byte-identical retained raw
   bytes under the same Effect, command, nonce, signature, and hash.
7. Local binding, authority epoch, purpose, key-derived sender, retained raw, and nested-domain
   mismatches stop before applicable provider IO or mutation.
8. Nonce reservation is append-only, account/chain/epoch-bound, serialized by the existing
   PostgreSQL authority, and never released or reused.
9. Provider submission occurs only after Prepared raw bytes are durably retained.
10. A null receipt causes at most one submission attempt in one adapter invocation.
11. Settlement requires a matching typed receipt and exact equality with the provider's current
    canonical identity for that receipt block number.
12. `Secp256k1Signer` remains key- and immutable-purpose-bound and object safe.
13. Private scalars, digests, and compact signatures retain their reviewed diagnostic/serde
    exclusions; private scalars never acquire an accessor.
14. The inner key map remains neither `Send` nor `Sync` and stays on its dedicated owner thread.
15. No private key, mnemonic, credential, raw signed transaction, or secret-bearing diagnostic enters
    Program, C0, Journal, Store metadata, RunView, output, failure detail, or logs.
16. PostgreSQL core and optional EVM authority retain separate pools, exact gates, privileges, and
    provisioning calls.
17. The e2e independently proves exactly two wallet transactions with nonces 0 and 1, exact hashes,
    receipts, deployed code, one configuration event, anchored value 42, no third mutation, and
    stable cold terminal replay.
18. Current-only cutovers delete superseded files and symbols; no aliases, deprecated names, legacy
    decoder, compatibility path, or dual representation remains.

## 1. Remove completed implementation records

Delete the tracked files:

- `IMPL_PLAN_EFFECT_SUP_E2E.md`;
- `FIXES_IMPL_PLAN_EFFECT_SUP_E2E.md`.

They explicitly describe themselves as historical/completed, conflict in places, and duplicate facts
now owned by `docs/design.md`, `docs/architecture.md`, focused docs, crate READMEs, tests, and Git
history. Repository precedent includes `9dad9c8cd` (`docs: clean up old impl plan`) and `747c9e3d`
(`remove completed implementation records`).

Do not migrate their prose into another tracked document. Review the maintained docs before deletion
only to ensure that no current contract exists solely in a plan. The reviewed baseline already has
current ownership for Effect semantics, signing/keystore, EVM transaction/Read behavior, PostgreSQL
authority, execution, routing, build tasks, and known limitations. If a genuinely current fact is
missing, add only that fact to its owning current document in this commit, then delete the plans.

Do not stage `FIXES_TT2_IMPL_PLAN_EFFECT_SUP_E2E.md`. It remains the local engineer handoff.

Acceptance:

- neither completed tracked plan is present at `HEAD`;
- no new RFC, audit log, plan summary, or historical compatibility document replaces them;
- maintained docs have one owner per current claim;
- `git diff --check` passes; and
- the docs-only commit runs no unnecessary Rust or broad gate.

## 2. Consolidate signing and keystore ownership assertions

### Keep the signing crate

`crates/signing/src/lib.rs` remains the exact small transient port:

- `SigningError` and `Result`;
- `SigningDigest`;
- `Secp256k1PublicKey`;
- `CompactRecoverableSignature`;
- `recover_public_key`;
- `SigningFuture`; and
- `Secp256k1Signer`.

Do not move this API into keystore. Keystore is one concrete secret-custody implementation while live
EVM is a consumer. Moving the port into either side reverses a dependency and merely relocates LOC.
Do not add persisted identity, MFM values, serde, routes, algorithm registries, or per-call purpose.

### Make the inner key map private

Change only the visibility of the concrete inner `Keystore` in `crates/keystore/src/lib.rs` from
public to private. Keep its name because `AGENTS.md` names the thread-affinity invariant, but expose
no public constructor or alias.

The final public custody surface remains:

- `MAX_KEY_INSTANCES`;
- `KeystoreError`;
- `SecretSecp256k1Scalar`;
- `KeystoreOwner`; and
- `KeystoreSigner`.

`KeystoreOwner::start`, `import_secp256k1`, and `shutdown` remain unchanged. The owner loop still
constructs `Keystore` inside the dedicated OS thread. `KeystoreSigner` remains the cloneable
key/purpose-bound implementation of `Secp256k1Signer`.

Update the crate-level rustdoc and keystore README to describe a private thread-affine key map rather
than link to `Keystore` as public API. `docs/design.md` and `docs/architecture.md` may continue using
“keystore” as the responsibility name; do not imply that the inner map is constructible by callers.

### Replace compiler-stderr snapshot sprawl

Use `static_assertions = "1.1"` as a dev dependency in `mfm-signing` and `mfm-keystore`. This exact
crate/version already exists in the lockfile and is used by PostgreSQL tests for negative trait
boundaries. Keep `serde` as a dev dependency only where required to name `serde::Serialize` in the
assertion.

In `crates/signing/tests/signing_contract.rs`, replace the `trybuild` runner with compile-time
assertions that:

~~~rust
static_assertions::assert_not_impl_any!(
    SigningDigest: std::fmt::Debug, std::fmt::Display, serde::Serialize
);
static_assertions::assert_not_impl_any!(
    CompactRecoverableSignature: std::fmt::Debug, std::fmt::Display, serde::Serialize
);
~~~

Do not add a restriction for `Secp256k1PublicKey` beyond the current design. It is public material;
the design merely declines to add diagnostic/serde implementations until a consumer needs one.

Inside `crates/keystore/src/lib.rs`'s private test module, assert:

~~~rust
static_assertions::assert_not_impl_any!(Keystore: Send, Sync);
static_assertions::assert_not_impl_any!(
    SecretSecp256k1Scalar: std::fmt::Debug, std::fmt::Display, serde::Serialize
);
~~~

The private module is the correct owner of the unexported `Keystore` auto-trait contract. The public
integration test continues exercising import, deterministic signing, purpose binding, capacity,
recovery, and shutdown through `KeystoreOwner` and `KeystoreSigner`.

Delete all 22 files under:

- `crates/signing/tests/ui/`;
- `crates/keystore/tests/ui/`.

Remove `trybuild` from both manifests. Do not remove repository-wide `trybuild`; Program, derive, and
Runtime still use it for exclusions that cannot be expressed by naming a public/private type in a
normal assertion.

### Signing and keystore test contract

Retain all of these observable cases:

- checked generator public key; invalid prefix and off-curve rejection;
- frozen signature recovery and wrong-digest non-equality;
- zero/invalid/high-S scalar and invalid recovery-ID rejection;
- digest/signature diagnostic and serde exclusions;
- `Keystore: !Send + !Sync` and secret-scalar diagnostic/serde exclusions;
- zero/out-of-range/valid secret scalar admission;
- scalar-one public key, deterministic low-S signature, recovery ID, and public recovery;
- duplicate same-key import consuming one key slot;
- same key under two purposes yielding equal public keys and distinct immutable purposes;
- exact 64/65 distinct-key capacity and bounded-channel backpressure;
- clean shutdown, final-sender exit, owner panic, post-shutdown failure; and
- redaction-safe error text with no secret diagnostics.

No runtime behavior or cryptographic contract changes in this commit.

## 3. Use one checked EVM chain-instance value

Delete `ObservedChainInstance` from `crates/live/evm/src/transaction.rs` and its public re-export from
`crates/live/evm/src/lib.rs`.

Change the transaction provider facet to:

~~~rust
fn chain_instance(&self) -> EvmTransactionProviderFuture<'_, EvmChainInstance>;
~~~

`EvmChainInstance` already represents exactly the two checked facts needed by authored binding and
provider observation: a nonzero chain ID and checked genesis hash. The provider trait is the
provenance boundary; a duplicate struct does not add evidence of observation.

Update `JsonRpcEvmProvider::chain_instance` to construct `EvmChainInstance` from the observed chain ID
and genesis anchor and map invalid ingress to `AdapterError::Unavailable`. Update the scripted live
provider and e2e setup to use `EvmChainInstance` directly.

In live transaction preflight, compare the entire observed value with
`command.binding().route().chain_instance()`. Do not separately compare primitives and do not add a
conversion trait or alias.

This is deliberately not a persisted cutover:

- keep the `EvmChainInstance` field name and canonical wire `expected_genesis_hash`;
- keep its MFM semantic/schema identities;
- keep `NonceDomain`, its advisory-lock preimage, and frozen lock vector byte-identical;
- keep PostgreSQL columns and the v2 authority schema byte-identical; and
- add no serde rename, compatibility decoder, or migration.

Affected current consumers include:

- `crates/live/evm/src/transaction.rs`;
- `crates/live/evm/src/json_rpc.rs`;
- `crates/live/evm/src/lib.rs`;
- `crates/live/evm/src/transaction_tests.rs`;
- any JSON-RPC transaction-provider test; and
- `crates/app/tests/evm_contract_effect_e2e.rs`.

The existing wrong chain ID and wrong genesis tests remain. Add no test that merely checks the
deleted type name. Exact domain JSON/schema/lock vectors must remain unchanged and prove that this
commit did not alter durable identity.

## 4. Simplify live EVM transaction test boundaries

Production reconciliation remains unchanged in this commit. Refactor
`crates/live/evm/src/transaction_tests.rs` around observable signer, authority, and provider
boundaries.

### Remove the forwarding call recorder

Delete `RecordingSigner`. Use the real `KeystoreSigner` for ordinary preparation and a small
non-forwarding same-key/same-purpose signer that deliberately fails `sign` for recovery paths where
signing must not occur. The latter stores only copied public key and purpose and implements:

- `public_key` by returning its own checked public key;
- `purpose` by returning its own immutable purpose; and
- `sign` by returning `SigningError::Failed`.

Use this rejecting signer on a retry from retained Prepared state and on the Settled fast path. A
successful result then proves that no new signature was requested without freezing a positive call
count or depending on deterministic ECDSA.

Keep one purpose-specific fault signer for the wrong-returned-signature regression. It is a real
boundary fault, not a generic call recorder. It may delegate to a concrete keystore handle to obtain
a valid signature over the deliberately wrong digest.

### Reduce the in-memory authority to mechanics

Keep one small test-local `MemoryAuthority` because the live adapter needs an independently
controllable implementation of its external port. Delete:

- `AuthorityOperation`;
- the authority operation vector and exact operation-array assertions; and
- fake-owned validation of Effect ID, command ref, raw bytes, or settlement consistency.

The fake should mechanically retain only the latest `AuthorityState`:

- `load` returns its configured/current state;
- `reserve_or_compare` returns the existing reservation or stores one constructed from the call and
  configured pending nonce;
- `retain_prepared` constructs/stores a `PreparedRecord` from the current reservation and supplied
  raw/hash;
- `retain_settlement` constructs/stores a `SettledRecord` from the current Prepared record and
  supplied evidence.

Do not duplicate PostgreSQL locking, append-or-compare conflict classification, command/effect
validation, or transition rejection. Those belong to the authority implementation and its tests.
The live adapter already validates every returned record; tests inject wrong nested state directly
to prove that validation.

### Keep semantic provider observation, not complete traces

Keep the scripted provider and semantic operations because receipt-before-submit and no-IO
preflight are external safety contracts. Replace complete positive arrays with assertions over only
the required facts:

- the first receipt observation precedes the first submission;
- at most one submission occurs in one invocation;
- every repeated submission contains byte-identical raw bytes;
- no pending nonce is observed after a reservation exists;
- canonical-block observation occurs only after a present validated receipt;
- noncanonical evidence never calls authority settlement;
- local mismatch produces zero authority/provider entry; and
- Settled fast path produces no provider operation and succeeds with the rejecting signer.

It is valid to assert zero boundary entry and the explicit at-most-one-submit contract. Do not assert
the exact number of chain-instance, receipt, canonical-block, load, or compare helper calls beyond a
public safety requirement. Do not slice the operation vector by fixed indices.

### Retained live-EVM matrix

Preserve at least these cases, preferably in the current eleven focused scenarios rather than one
test per primitive:

| Owner | Required cases |
| --- | --- |
| Registration | complete binding is the sole adapter key; duplicate registration rejected |
| Preparation | absent state reserves nonce, signs once semantically, retains exact frozen raw/hash, returns Pending |
| Cancellation | cancellation after Prepared leaves one resumable exact transaction |
| Retry | Prepared retry succeeds with rejecting signer, submits retained byte-identical raw, observes no second pending nonce |
| Reserved | preloaded reservation skips pending nonce and reaches Prepared |
| Submission | matching hash is Pending; transport failure and mismatched hash are Unavailable while Prepared remains |
| Receipt | Create success/revert, Call success/revert, wrong CREATE address, wrong target, wrong sender/hash |
| Canonicality | wrong canonical anchor remains Prepared and never retains settlement |
| Signature | valid signature over wrong digest is rejected before Prepared/submission |
| Preflight | wrong binding, epoch, endpoint, chain ID, genesis, purpose, key-derived sender all stop before mutation/provider entry |
| Retained facts | wrong nested domain and corrupt raw stop before provider entry |
| Fast path | Settled returns byte-identical evidence with rejecting signer and no provider call |
| Codec | leading-zero integer, nonempty access list, trailing bytes, malformed/noncanonical list, invalid parity, zero/high-S scalar |
| Transport | bounded loopback request and dropped submission acknowledgement remain covered |

Do not delete the pure hostile codec table, real keystore known-answer, loopback connection-drop
regression, or JSON-RPC ingress tests to meet a line target.

## 5. Consolidate Runtime Effect recovery regressions

Refactor only `crates/kernel/runtime/tests/runtime_contract.rs`. Production Runtime, Program,
Journal, Store, capability, and value code must not change.

### Keep one hostile Store fixture

Delete `RetainedStore`. `ScriptedStore::with_retained` already has the same complete-prefix load and
append-rejection behavior. Use it for every retained invalid-history case.

Keep `FaultStore`; it represents a distinct all-operations-fail boundary used to verify Store-to-
Runtime error authority. Keep `ScriptedStore`; it represents exact append acknowledgement and
retention combinations. Do not introduce another Store implementation.

The final Store fixture set is:

- production `MemoryStore` for ordinary behavior;
- `ScriptedStore` for recording, retained complete prefixes, NotInserted, and Indeterminate cases;
  and
- `FaultStore` for direct load/append error mapping.

### Table-drive the ambiguity matrix

Replace the four repeated bodies in
`ambiguous_effect_appends_recover_from_exact_retained_facts` with one explicit case table. Each case
must state:

- candidate sequence (`2` prepare or `3` conclusion);
- append action (`Indeterminate` or `RetainThenIndeterminate`);
- whether the candidate was retained;
- expected head immediately after the failed start;
- expected adapter entries after start and after resume; and
- expected terminal state/head after resume.

The four required rows are:

| Candidate | Retained before Indeterminate | Adapter entries after start | Recovery |
| --- | --- | ---: | --- |
| prepare | no | 0 | reload genesis, append prepare, enter adapter once, settle |
| prepare | yes | 0 | reload retained prepare, enter adapter once, settle |
| conclusion | no | 1 | reload prepare, re-enter adapter, append conclusion |
| conclusion | yes | 1 | reload retained conclusion, do not re-enter adapter |

Keep exact `EffectId` reuse where it is the public semantic fact. Do not assert an internal reducer
function or declaration index.

Keep the existing two-row `RetainThenNotInserted` convergence test and three-row adapter
Unavailable/Internal/panic test table. Consolidate their repeated adapter construction only when the
helper clearly represents the same external Effect adapter boundary.

### Consolidate hostile retained-frame construction

Retain all four invalid-history cases:

- wrong retained prepare `EffectId`;
- replaced command bytes/ref;
- swapped evidence bound to another `EffectId`; and
- conclusion outcome with the wrong schema.

Introduce one test-local hostile-frame utility that:

1. parses one selected encoded frame to `serde_json::Value`;
2. gives a case-specific closure controlled mutation access;
3. sorts the `objects` array by `(schema_id, content_digest)` when a mutation changes an object ref;
4. canonicalizes the mutated JSON through `PlainCanonicalJsonBytes`; and
5. returns the replacement bytes.

This helper represents malicious Store ingress. It must not implement Journal qualification,
calculate expected Runtime outcomes, or become a generic production encoder. Continue using
`raw_content_digest` and production canonicalization for exact replacement refs.

Build one small Runtime fixture for “read retained history with a counting adapter” and reuse it
across the four corrupt prefixes. The observable assertion remains `InvalidHistory` with zero
adapter entry. Keep the original valid pending and valid settled prefixes as controls.

### Runtime regression inventory

The final file must retain observable coverage for:

- duplicate/wrong-kind Effect registration and missing adapter before Store IO;
- pure and zero-State hot/cold equivalence;
- fused Read replay and resumable failure;
- Read cancellation and runnable prefix;
- durable Effect prepare before adapter entry;
- cancellation after prepare and cold identity reuse;
- Pending yielding exactly one caller invocation and later settlement;
- preparation failure and evidence-binding failure appending no invalid conclusion;
- all four ambiguous prepare/conclusion acknowledgement outcomes;
- NotInserted convergence for prepare and conclusion;
- adapter Unavailable/Internal/panic mappings with one pending prepare;
- all four retained Effect history corruptions with zero adapter IO;
- concurrent pending callers converging on one conclusion; and
- Store load/append error authority mapping.

Do not remove a case solely because another layer tests a related primitive. Runtime owns its fold,
adapter-entry, and append-acknowledgement consequences.

## Documentation ownership

Most maintained docs should not need semantic edits because TT2 preserves behavior. Update only
facts made inaccurate by the visibility/type cleanup:

| Owner | Required update |
| --- | --- |
| `crates/keystore/README.md` and crate rustdoc | inner key map is private; public callers use owner/signer handles |
| `crates/live/evm/README.md` or rustdoc | provider observes and returns `EvmChainInstance` if the type is named |
| `docs/design.md` | only if it currently implies public construction of the inner key map or names `ObservedChainInstance` |
| `docs/architecture.md` | only if its taxonomy names a deleted public/transient type |

Do not edit persisted-surface documentation for the chain consolidation: durable
`EvmChainInstance`, `expected_genesis_hash`, schema IDs, authority domain, and SQL stay unchanged.
Do not add LOC/commit history or TT2-specific commentary to current docs.

## Complete deletion ledger

The series is incomplete while any of these remain in tracked current code/tests/docs, excluding
this intentionally untracked handoff file where the names describe the deletion:

### Files

- `IMPL_PLAN_EFFECT_SUP_E2E.md`;
- `FIXES_IMPL_PLAN_EFFECT_SUP_E2E.md`;
- every file below `crates/signing/tests/ui/`;
- every file below `crates/keystore/tests/ui/`.

### Symbols and fixtures

- `pub struct Keystore` (the private `struct Keystore` remains);
- `ObservedChainInstance`;
- `RecordingSigner`;
- `AuthorityOperation`;
- authority operation recording/accessor code;
- `RetainedStore`;
- complete positive provider/authority operation-array comparisons; and
- `trybuild` dev dependencies in `mfm-signing` and `mfm-keystore`.

### Forbidden replacements

- a renamed observed-chain wrapper or conversion trait;
- a second in-memory authority implementation or generic mock framework;
- another retained-only Store wrapper;
- new compiler `.stderr` snapshots for nameable trait exclusions;
- a tracked TT2 summary or completed-plan status document;
- schema aliases, serde renames, migrations, v3 authority markers, or compatibility readers; and
- production test-support modules used only to move LOC out of test files.

Use scoped searches, because other crates intentionally retain `trybuild` and this handoff file names
the deleted targets:

~~~bash
rg -n 'ObservedChainInstance|RecordingSigner|AuthorityOperation|RetainedStore' \
  crates docs --glob '*.rs' --glob '*.md'
rg -n 'trybuild' crates/signing crates/keystore --glob 'Cargo.toml' --glob '*.rs'
find crates/signing/tests/ui crates/keystore/tests/ui -type f
~~~

The final `find` may report that the directories do not exist; that is success.

## Reduction accounting

The engineer must measure rather than claim simplification. Record before and after:

~~~bash
git diff --stat origin/dev...HEAD
git diff --numstat origin/dev...HEAD
git diff --stat 0f3e790dc...HEAD
git diff --numstat 0f3e790dc...HEAD
~~~

Report at least:

- tracked files removed;
- production/build, test/fixture, maintained-doc, and plan additions/deletions;
- public types removed;
- test fixture types removed;
- exact safety cases retained;
- any helper or type added and the duplicated code/change sites it replaces; and
- final full-branch and TT2-only statistics.

Expected, non-binding review targets are:

- at least 3,800 net current-tree lines removed from `0f3e790dc`;
- no more than roughly 90 changed files relative to `origin/dev` after the UI/plan file deletions;
- full-branch additions reduced from 18,141 to roughly 14,500 or fewer; and
- production behavior LOC flat except for deleting the observed chain projection and public
  visibility surface.

If the implementation misses these ranges, do not delete safety tests to force the statistic.
Explain which named duplication could not be removed and why its replacement would add more concepts
or weaken a boundary.

## Ordered logical commits

Implement five lower-case commits. Each commit must build and leave one coherent current design.

### 1. `remove completed effect implementation records`

- verify current contracts are present in maintained docs;
- delete the two tracked completed plans;
- do not add this TT2 handoff;
- make no Rust, schema, test, or workflow change; and
- run docs-only verification.

### 2. `consolidate signing boundary assertions`

- make the inner `Keystore` private;
- replace signing/keystore trybuild snapshots with compile-time negative trait assertions;
- delete both UI trees;
- replace `trybuild` dev dependencies with the already-locked `static_assertions` dev dependency;
- retain all cryptographic, capacity, purpose, shutdown, panic, redaction, and ownership tests; and
- update keystore rustdoc/README in the same commit.

This is one coherent public/test-surface cut. Do not separate private visibility from its replacement
thread-affinity proof.

### 3. `use one evm chain instance value`

- delete and stop re-exporting `ObservedChainInstance`;
- return `EvmChainInstance` through the provider facet;
- update JSON-RPC, live tests, and e2e consumers;
- compare the complete observed/authored value; and
- freeze all persisted EVM and authority vectors byte-identically.

### 4. `simplify evm transaction test boundaries`

- delete the forwarding call recorder and authority operation model;
- reduce `MemoryAuthority` to mechanical state retention;
- add/use the rejecting same-key/same-purpose signer for no-resign proof;
- replace complete positive operation arrays with required partial-order/absence/byte assertions;
- preserve the complete live/codec/transport matrix; and
- change no production reconciliation behavior.

### 5. `consolidate effect recovery regressions`

- delete `RetainedStore`;
- table-drive the four ambiguous append cases;
- consolidate malicious retained-frame rewriting and repeated retained-runtime assembly;
- retain the complete Runtime regression inventory; and
- change no production Runtime, Journal, Program, or Store code.

Do not fold unrelated formatting, CLI, or pre-existing cleanup into these commits.

## Verification workflow

All direct Rust commands run in the default Nix development shell. Use the narrowest gate while
iterating and do not run broad component gates immediately before final CI.

### Commit 1

~~~bash
git diff --check HEAD^..HEAD
rg -n 'Effect|Secp256k1Signer|PostgresEvmTransactionAuthority|anchored' \
  docs/design.md docs/architecture.md docs/known-gaps.md \
  docs/build-and-verification.md crates/*/README.md crates/*/*/README.md
~~~

Review the search manually for current ownership. Do not run Cargo for this docs-only deletion.

### Commit 2

~~~bash
nix develop -c cargo fmt --all -- --check
nix develop -c cargo test -p mfm-signing -p mfm-keystore --all-targets
nix develop -c cargo clippy -p mfm-signing -p mfm-keystore --all-targets -- -D warnings
~~~

Because this touches cryptographic and ownership contracts, do not omit the negative trait,
known-answer, low-S, capacity, backpressure, shutdown, and panic tests.

### Commit 3

~~~bash
nix develop -c cargo test -p mfm-evm -p mfm-evm-live --all-targets
nix develop -c cargo check \
  -p mfm-storage-postgres -p mfm-app --all-targets
nix develop -c cargo clippy \
  -p mfm-evm -p mfm-evm-live -p mfm-storage-postgres -p mfm-app \
  --all-targets -- -D warnings
~~~

Require existing transaction JSON/schema and nonce-domain lock-vector tests to remain byte-identical.
No managed PostgreSQL task is selected because neither SQL nor authority persistence changes.

### Commit 4

~~~bash
nix develop -c cargo test -p mfm-evm-live --all-targets
nix develop -c cargo clippy -p mfm-evm-live --all-targets -- -D warnings
~~~

Review the test list against the retained live-EVM matrix. A smaller test file with missing rows is a
failure.

### Commit 5

~~~bash
nix develop -c cargo test -p mfm-runtime --all-targets
nix develop -c cargo clippy -p mfm-runtime --all-targets -- -D warnings
~~~

Review all four ambiguity rows, all four hostile retained-history variants, cancellation,
concurrency, and Store error authority explicitly.

### Final candidate

Run focused formatting and scoped static checks while iterating. Once the exact candidate is ready,
run the managed Effect proof and then CI once:

~~~bash
git diff --check origin/dev...HEAD
nix develop -c cargo fmt --all -- --check
nix run .#run -- --task effect-e2e
nix run .#ci
~~~

`effect-e2e` is selected because the public provider return type and e2e compilation path change,
even though transaction behavior does not. CI is selected because the series crosses public crypto,
live adapter, Runtime test, and application boundaries. Do not run `.#check`, `.#test`, and
`.#test-db` immediately before `.#ci`; CI composes them. Do not run `.#model-check` unless this plan
unexpectedly changes `nixfied.nix`, which is outside scope.

## Completion checklist

The TT2 series is complete only when:

- all five commits exist in the specified order with lower-case subjects;
- each commit is coherent and contains its required focused verification;
- the two completed tracked implementation plans are deleted;
- this TT2 handoff is not tracked;
- `crates/signing` remains the thin transient signer port;
- the inner `Keystore` is private and remains `!Send + !Sync`;
- `KeystoreOwner`, `KeystoreSigner`, and secret-scalar APIs retain their behavior;
- all 22 signing/keystore UI snapshot files and both scoped trybuild dependencies are gone;
- compact compile-time assertions preserve every negative trait contract;
- `ObservedChainInstance` and its re-export are gone;
- the provider returns `EvmChainInstance` and complete equality binds observation to authored chain;
- no EVM schema, canonical JSON, authority lock vector, SQL, marker, or migration changes;
- `RecordingSigner`, `AuthorityOperation`, and complete positive operation arrays are gone;
- a Prepared retry and Settled fast path succeed with a signer that cannot sign;
- repeated submissions remain byte-identical and at most one occurs per invocation;
- all local mismatch paths prove zero authority/provider entry;
- the live receipt/action/canonicality/codec/submission/transport matrices remain complete;
- `RetainedStore` is gone and no replacement Store type appears;
- all four ambiguous append cases retain their distinct adapter-entry/head outcomes;
- all four retained Effect corruption cases remain `InvalidHistory` with zero adapter IO;
- Runtime cancellation, concurrency, hot/cold, Pending, NotInserted, adapter error, and Store mapping
  tests remain;
- the managed Reth e2e still proves deploy/configure/anchored-read/report and exactly two mutations;
- production-finality, persistent-key-recovery, and authority-rollback limitations remain documented;
- scoped stale-target searches are clean outside this handoff;
- the reduction accounting and deletion report are supplied;
- `git diff --check` passes;
- focused verification passes; and
- one final `nix run .#ci` passes on the exact candidate.

## Engineer final report

The engineer-agent report must include:

- the five commit hashes and subjects;
- exact files, public types, fixture types, and dependencies deleted;
- before/after full-branch and TT2-only diff statistics classified by production, tests, maintained
  docs, and plans;
- confirmation that all named safety matrices remain;
- exact focused commands and results per commit;
- managed `effect-e2e` and final CI results;
- confirmation that no secret/raw transaction entered diagnostics or persisted public surfaces;
- the intentionally untracked status of this TT2 handoff; and
- every remaining risk, skipped check, blocker, or deviation from this plan.

Do not report “all tests passed” as a substitute for the deletion ledger, public-surface review, and
case-by-case retained coverage.

## Material uncertainties

### TT2 handoff retention

- Assumption: the user wants this detailed file available to the engineer but not committed into the
  final source tree, matching the earlier intentionally preserved untracked-plan workflow.
- Why uncertain: the user explicitly requested the document in the repository workspace, but did not
  explicitly state whether it should be a tracked merge artifact.
- Consequence if wrong: keeping it untracked may not satisfy an external audit/archive process;
  tracking it would reintroduce the exact completed-plan bloat this pass removes.
- Validation: preserve it untracked and state that clearly in the final report; obtain explicit user
  direction before staging, deleting, or moving it.

### Existing CLI assertion commit placement

- Assumption: TT2 adds follow-up commits and does not rewrite the existing thirteen Effect commits.
- Why uncertain: `6d559d5a4` contains the unrelated but correct CLI blank-separator expectation, while
  repository policy says unrelated cleanup belongs in a separate commit.
- Consequence if wrong: normal merge history retains one unrelated line in the original e2e commit;
  rewriting history would change every descendant hash and broaden the engineer's authority beyond
  this cleanup.
- Validation: leave the correct final CLI code unchanged and disclose the historical placement. If
  the user explicitly authorizes a rebase before merge, move that one-line correction into its own
  lower-case commit without changing CLI output or TT2 content.

No architecture or ownership uncertainty remains. The current authoritative design resolves the
signing crate boundary, private custody versus signer handle, anchored Read ownership, nonce
authority, and development-only settlement policy.
