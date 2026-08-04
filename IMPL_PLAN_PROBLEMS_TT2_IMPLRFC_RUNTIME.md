# Implementation Plan: TT2 Runtime-History Remediation

Status: ready for implementation

This remediation is a plan-contract commit followed by 12 ordered implementation commits and an
independent exact-revision review. It resolves all 32 TT2 findings without compatibility paths or
alternate authorities. Implementation starts from the revision containing this plan.

Planning baseline: current `refact-runtime` head
`b66250c706239b217c619b6896db0b1646ca7a40`; regression baseline: implementation revision
`a4dada89a5b01cf2bf97f9e16c398fda0a5c1e42`.

Primary sources:

- [TT2 problem ledger](PROBLEMS_TT2_IMPLRFC_RUNTIME.md);
- [design contract](docs/design.md);
- [architecture contract](docs/architecture.md);
- [code-quality policy](docs/code-quality.md); and
- [build and verification contract](docs/build-and-verification.md).

## Material uncertainties

none

One external prerequisite is intentionally fixed rather than uncertain: production PostgreSQL
opening requires a deployment-provided credential broker and non-rollback checkpoint authority.
The repository defines and tests that contract but does not provide an always-successful production
implementation. Standalone binaries continue to fail closed.

## Target architecture

The Cargo and authority direction remains:

```text
mfm-certify <- mfm-runtime <- mfm-store
```

- `mfm-certify` owns only deterministic certification and program verification.
- `mfm-runtime` owns callbacks, physical-binding selection, access preparation, the affine
  invocation bracket, and one-action driving.
- `mfm-store` owns the sole fold, canonical append construction, persistence, snapshot fixation,
  external store checkpoint integration, and purpose projections.
- `mfm-app` receives an assembled `Runtime` plus purpose-specific services; it never receives a
  registry half, store port, backend, pool, or invocation handle.

### Frozen architect decisions

| Area | Target decision | Invalid state excluded | Required hostile proof |
| --- | --- | --- | --- |
| Runtime access | Move process and binding authority from certification into `Runtime`. The store returns a lineage-sealed verified successor, not a publicly constructible token. `Runtime` privately derives `Authorized<K>` only after matching the exact committed record. | Invocation from fabricated records, fake heads, reused permits, or a split registry. | External-crate compile failures and one-commit/one-invocation runtime tests. |
| PostgreSQL TCB | Raw credentials exist only inside a deployment credential broker. Ordinary assembly consumes a non-cloneable, one-shot target admission and receives opaque role-separated sessions. | Retained URLs, pools, role switching, and direct DML outside the store. | Retained-input, role-escalation, stale-ticket, and sibling-target denial tests. |
| Non-rollback authority | An external checkpoint authority retains an acknowledged head and at most one exact prepared successor for each affected run, configuration, and fact key set. | Copied or restored databases certifying their own rollback state. | Copy, restore, rollback, wrong-target, stale-generation, promotion, and prepared-successor crash tests. |
| Read snapshot | Run and configuration loads use `REPEATABLE READ`; an external short-lived read fixation pins the expected indexed head until the first head query establishes the snapshot. | Torn batch/object or revision/head reads and false integrity errors. | Deterministic append barriers before and between every SQL read phase. |
| EVM recovery | Newly activated candidates, retained candidates needing observation, and retained candidates eligible for resubmission are distinct typed paths. | Broadcasting retained work before discovering chain truth. | Crash/restart tests at every broadcast, receipt, finality, and completion boundary. |
| Replacement | A fresh final wallet-status read plus exact committed transaction, receipt, and head evidence is converted by the qualified adapter into a non-cloneable replacement permit. The raw wallet port no longer accepts caller-authored activation permits. | Self-certified, stale, reordered, cross-run, or cross-reservation replacement. | Direct hostile port calls and consumed/cross-wired permit tests. |
| Stable intent | `SubmissionIntentId` hashes only nonce domain, authenticated issuer, and bounded caller token. A separate `SubmissionSemanticsDigest` binds transaction, family, expansion, observation, replacement, signing, and terminal policy. | Changed semantics becoming a new ID instead of a permanent conflict. | Same-token field-by-field conflict matrix and explicit new-domain idempotency-epoch test. |
| Wallet currentness | Immutable activation retains the initial incarnation. Every protected access separately consumes a provider-issued current-incarnation lease or transaction permit. | Equating initial activation with current promoted physical authority. | Initial, promoted, stale-old, copied, and mismatched target tests through app, live adapter, and storage. |
| Portable evidence | Bundle the exact root and required producer prefixes, dense fact publication routes through every recorded frontier, and recursive dependency relationships. Fold each producer once at its maximum required head. | Self-listed or unverifiable source IDs and unproved fact completeness. | Offline golden parity plus omitted, extra, substituted, cyclic, and frontier-tamper corpus. |
| Purpose authority | The store constructs minimum data products, not wrappers around `VerifiedStructuredRun`. Export evidence is opaque and consumable only by the encoder. | Public, trace, or replay holders manually reconstructing audit or export data. | Compile-fail conversion tests and data-isolation assertions. |
| Bounds | Use named generated budgets for canonical values/frames, run batches, configuration envelopes, portable frames/bundles, replay projections, and retained provider proofs. Portable output is a bounded sequence of canonical frames. | Encoders producing bytes rejected by their current decoders or universal oversized limits. | Exact-limit and one-over tests for individual frames, many-frame bundles, configuration, and provider evidence. |

## Runtime/store access protocol

The new protocol is:

```text
Runtime-private Prepared<K>
  -> store append
  -> store-lineage-sealed VerifiedSuccessor
  -> Runtime verifies the exact authorization record
  -> Runtime-private Authorized<K>
  -> exactly one invocation
  -> Runtime-private PendingObservation<K>
  -> store append/resolution
  -> committed observation
```

The verified authorization comparison must include:

- run and committed record references;
- predecessor and successor journal heads;
- access attempt ID and ordinal;
- occurrence ID and structural path;
- semantic call and semantic-head anchor;
- state input reference;
- access kind;
- capability and adapter semantic/implementation identities;
- persisted request reference and digest;
- physical binding and stable lineage;
- store scope and writer epoch.

A process-local paired assembly seal binds the production process registry to its private store
adapter. Custom `Runtime` instances may create their own seal and resources, but cannot attach a
fake history port to MFM's production invokers or backend.

## PostgreSQL checkpoint protocol

The deployment checkpoint uses stable keys independent of fence generation:

```text
Run(run_id)
Configuration(store, tenant, entry, target)
TenantFacts(tenant)
```

One run append may update both its run key and tenant-fact key as a single sorted multi-key operation.

Write flow:

1. Acquire a fresh target/session lease from the external authority.
2. Begin the SQL transaction and acquire stable advisory locks in canonical key order.
3. Read exact predecessor heads under lock.
4. Construct the complete canonical successor.
5. Externally prepare the exact predecessor/successor key set.
6. Perform SQL DML.
7. Commit SQL.
8. Acknowledge the external checkpoint.
9. On ambiguity, reconnect, reacquire authority and locks, inspect the database, then acknowledge
   the exact successor, retry the identical predecessor, or reject divergence.

Read flow:

1. Acquire a short external read fixation for the requested stream key and current target.
2. Begin `REPEATABLE READ READ ONLY`.
3. Read the indexed head as the first snapshot-establishing query.
4. Require exact equality with the fixation, then release its short barrier.
5. Read batches/objects or revisions within the same snapshot.
6. Fold and compare the derived head with the indexed head.

The `Runtime` cache check and cache-miss load become one backend operation returning either
`Unchanged(exact_head)` or `RawPrefix(exact_head, rows)` from that same snapshot.

## Implementation contract

The commit subjects below are normative and lower case. Each implementation commit must leave one
coherent current design. Tests, generated contracts, fixtures, deletions, and documentation belong
to the commit that changes their behavior.

In particular, each contract-changing commit must update the affected parts of
[`docs/design.md`](docs/design.md), [`docs/architecture.md`](docs/architecture.md), recoverability
annexes and schemas, CLI/REST documentation, binary READMEs, and public rustdoc in that same commit.
It must delete every superseded contract and generated artifact without a compatibility path.

The implementation agent owns commits 1 through 12. It must then stop and report the exact
candidate implementation revision. A separate skeptical reviewer owns commit 13; the implementation
agent must not self-review or create that commit.

## Ordered commit plan

### 0. plan tt2 runtime history remediation

Add `IMPL_PLAN_PROBLEMS_TT2_IMPLRFC_RUNTIME.md` containing this target, traceability, deletion
inventory, and verification contract.

Also mark the TT1 plan as superseded and correct its EVM identity wording: behavior belongs in the
separate `SubmissionSemanticsDigest`, not `SubmissionIntentId`.

No implementation belongs in this commit.

### 1. seal runtime physical access behind committed history

Closes: TT2-AUTH-01; begins TT2-QUALITY-01 and TT2-VERIFY-01.

Changes:

- Move process registry, physical-binding source traits, prepared binding, expected authorization,
  invoker dispatch, and `QualifiedAccessCompletion` from `mfm-certify` to `mfm-runtime`.

- Leave certification with data-only implementation manifests and program verification.
- Replace `NewlyAppendedAuthorization` with a `Runtime`-private `Authorized<K>`.
- Change `RuntimeHistoryPort::authorize_access` to return a verified successor/disposition.
  `Runtime` inspects the successor and privately mints invocation authority.

- Introduce the paired production assembly seal and an opaque store-owned
  `ProductionRegistryBundle`.
- Make production assembly consume the complete bundle once and return `Runtime` plus purpose
  services.
- Split `runtime/src/structured.rs` by real ownership: registry, access bracket, drive, faults, and
  port.
- Split the touched certification module so no live process code remains there.

Delete:

- `NewlyAppendedAuthorization::from_store_mint`;
- public `prepare_access` and `invoke_qualified_physical_binding`;
- public registry splitting and `Runtime::new(history, processes)` for production types;
- public request/certificate accessors on live bindings;
- certification-owned invocation and store-mint code;
- obsolete certify UI fixtures that currently advertise the mint function.

Proofs:

- External crate cannot obtain or split a production process registry.
- It cannot construct a production `Runtime` from a fake port.
- Fabricated `ExternalAccessAuthorized` data grants no live authority.
- Cross-binding, cross-attempt, reuse, serialization, cloning, and acknowledgement-resolution
  attempts fail.
- One newly committed exact authorization enters one matching invoker exactly once.
- Existing-same or ambiguous resolution never mints a second invocation.

Focused verification: runtime/certify/store/app package checks and tests, Trybuild API tests, and
the `cargo-metadata-contract` task, all through Nix.

### 2. bind postgres sessions to external deployment authority

Closes: TT2-AUTH-02 and TT2-STORE-01.

Changes:

- Replace `SessionLoginMaterial`, `TargetSessionMaterials`, and public issuance functions with a
  consumed deployment-issued `PostgresTargetAdmission`.
- Define the deployment broker and external checkpoint ports.
- Keep raw URLs, pools, authentication material, and target proof keys inside the deployment TCB.
- Produce distinct run-reader, run-writer, configuration-reader, and configuration-writer sessions.
- Add transaction-bound target leases and connection challenges.
- Add external acknowledged/prepared checkpoint state for run, configuration, and tenant-fact keys.
- Use advisory-lock keys independent of fence generation.
- Bind every proof to database identity, schema, store scope/epoch, target public-key identity,
  release epoch, and fence generation.
- Replace the current schema baseline; add no migration reader.
- Extend the integration provider fixture to hold checkpoint state outside the database process.

Delete:

- public raw database URL fields;
- `issue_application_sessions`, `issue_combined_sessions`, and equivalent raw-material openers;
- locally cloned `TargetPermit::from_binding`;
- same-database-only `target_authority` claims as sufficient currentness;
- production-compilable permissive fence fixtures.

Proofs:

- Retained application inputs cannot open connections, perform DML, or `SET ROLE`.
- Copied/restored databases, coordinated configuration rollback, stale generation, wrong database,
  same schema on another database, and replayed tickets fail closed.

- Prepared predecessor permits only byte-identical retry.
- Prepared successor is acknowledged after restart.
- A sibling successor or database-ahead state rejects readiness.
- Promotion drains old leases and excludes all old-generation commits.

### 3. make postgres loads and races exact

Closes: TT2-STORE-02, TT2-STORE-03, and TT2-STORE-04.

Changes:

- Add one snapshot load API for cached and uncached run reads.
- Use `REPEATABLE READ` for run and configuration loads.
- Make indexed-head read the first snapshot query.
- Compare loaded/folded prefix with the same-snapshot head.
- Rework write contention handling so any failed SQL transaction is rolled back before classification.
- Reacquire a fresh target lease and canonical locks before rereading.
- Integrate unique, serialization, deadlock, and acknowledgement ambiguity with the external
  prepared-successor protocol.
- Remove classification SQL from aborted transactions.

Proofs:

- Append between each batch/object/configuration query produces a complete old or new prefix.
- Empty-cache loads reject missing, ahead, behind, rewound, and digest-divergent head rows after session issuance.
- Cached and fresh loads use the same fixation rule.
- Two processes obtain exact `ExistingSame`, `AppendConflict`, or `StaleHead` results.
- Commit-acknowledgement loss never reinvokes an external operation or produces a partial append.

### 4. share canonical storage ingress and inventory every sql query

Closes: TT2-STORE-05 and TT2-STORE-06.

Changes:

- Introduce store-owned `CanonicalRunAppend` and `CanonicalConfigurationAppend`.
- Validate complete envelope bytes, object closure, counts, ordering, hashes, floats, depth, and
  generated limits before backend dispatch.
- Make memory and PostgreSQL accept only those values.
- Keep database constraints as defense in depth.
- Replace line-based SQL scanning with a Rust-syntax inventory that recognizes:
    - generic `query`, `query_as`, and `query_scalar`;
    - SQLx macros;
    - `QueryBuilder`;
    - imports, aliases, and wrapped helpers.

- Use checked SQLx macros for fixed SQL.
- Isolate unavoidable dynamic construction behind one private catalog with statement ID, owner, and reason.
- Move the inventory checker to test/tool code; a `syn` dev dependency is justified here because
  textual scanning cannot establish the claimed boundary.

Proofs:

- One positive/negative/max/one-over corpus yields identical memory/PostgreSQL results for run and
  configuration appends.
- Fixture files exercise every SQL construction form and fail when unowned.
- Offline SQLx metadata and the dynamic catalog together cover every production query.
- Repository searches reject new raw SQL constructors outside the catalog.

### 5. bound prior fact verification by unique producer prefix

Closes: TT2-STORE-07.

Changes:

- Page the dense publication route first and group required publications by producer run.
- For each producer, determine one maximum required head.
- Load and fold that prefix once; validate every earlier publication from the same verified result.
- Carry one bounded memoization session through recursive fact verification.
- Add total producer-history bytes, distinct producer count, route count, and fold-work limits.
- Store exact producer-prefix byte metadata with publication routes for bounded preflight; verify it
  against loaded bytes.
- Reuse incremental fold state only within the bounded verification session; do not create a second
  reducer or durable cursor.

Proofs:

- One producer with `N` successive publications performs one load/fold rather than `N` growing
  folds.
- Work is linear in route entries plus unique retained bytes.
- Recursive/shared producers deduplicate; cycles and over-budget requests fail deterministically.
- PostgreSQL query and fold counters are asserted, not inferred from elapsed time.

### 6. restrict run evidence to its exact purpose

Closes: TT2-APP-01.

Minimum products:

- Public: run ID, tenant, invocation, entry operation, journal/semantic heads, fold-derived status,
  and resolved terminal public outcome.
- Trace: run/head header plus state-transition projection rows only.
- Audit: run/head header plus authorization/observation pairs only.
- Recorded replay: run/head/semantic head/status/count summary only.
- Export discovery: identifiers, required heads, tenant/target fixation, direct source and fact-frontier metadata only.
- Export content: opaque fragments consumable only by `mfm-replay`'s encoder.

Changes:

- Stop storing `VerifiedStructuredRun` inside public evidence newtypes.
- Project the minimum value immediately after the sole fold.
- Remove arbitrary object resolution and raw record/batch enumeration outside export.
- Keep export content opaque, non-cloneable, non-serializable by callers, and without public accessors.
- Retain tenant checks through a narrow projection header.

Proofs:

- Cross-purpose conversion and accessor compile-fail matrix.
- Public and trace products contain no authorization request, physical binding, audit outcome,
  arbitrary object, or raw batch.
- Audit contains no state outputs beyond reviewed identifiers.
- Replay summary cannot reconstruct trace/audit/export.
- Only the export encoder consumes export fragments.

### 7. move decrypted keys through one protected allocation

Closes: TT2-SEC-01.

Changes:

- Allocate the final zeroizing heap buffer before decryption.
- Use in-place authenticated decryption into that allocation.
- Represent ownership as `Zeroizing<Box<[u8; 32]>>` or an equivalent stable heap allocation.
- Move only the owning pointer from `ProtectedKeyMaterial` to `SecureKey`.
- Remove all plaintext-array-taking constructors and inline `Zeroizing<[u8; 32]>` fields.
- Preserve AAD, bounded input, constant-time identity checks, non-`Send`/non-`Sync`, and redacted
  errors.

Proofs:

- Instrumented address continuity across decrypt, qualification, handoff, and signing.
- Cleanup on success, authentication failure, wrong length, identity mismatch, and unwind.
- Compile/API tests reject plaintext `[u8; 32]` constructors.
- Canary key bytes, paths, and credentials never appear in `Debug`, errors, logs, histories,
  artifacts, or outputs.

### 8. qualify evm signer and pending nonce evidence

Closes: TT2-EVM-08, TT2-EVM-10, and TT2-EVM-11; begins TT2-EVM-12.

Changes:

- Delete the retained `BoundedComponentInvoker<EvmCandidateSigner>` path and its
  integrity-to-availability mapping.
- Keep one `Read`-qualified signer-attestation path; integrity remains `IntegrityFault`.
- Change `ObservedPendingNonceFloor.pending_nonce` and every qualified floor to
  `TransactionNonce`.
- Decode provider quantities directly into that newtype.
- Treat `u64::MAX` or larger as committed integrity/capacity evidence, never `Returned`.
- Define producer-bound pending-floor evidence containing exact chain binding, sender, route
  generation/membership, physical release, policy, request, and authorization origin.

- Have the live adapter convert committed origin plus provider result into an opaque
  `QualifiedPendingNonceObservation`.
- Make the wallet reserve port accept only that opaque value and recheck it against activation plus
  the current wallet-authority lease.

Proofs:

- Zero and `u64::MAX - 1` succeed; `u64::MAX` and larger cannot become returned observations.
- Substitute chain, sender, route, membership, release, policy, source run, and observation independently.
- No signer integrity branch maps to `SignerUnavailable`.
- Wrong key, locked key, stale generation, wrong address/public key, and malformed signature remain
  distinctly classified.

### 9. restore stable evm intent and bounded wallet currentness

Closes: TT2-EVM-05, TT2-EVM-06, and TT2-EVM-07.

Changes:

- Restore the three-field `SubmissionIntentId` formula.
- Introduce one complete `SubmissionSemanticsDigest`.
- Persist both and make `(domain, submission_intent_id)` resolve only when the semantic digest and
  retained preimages match exactly.
- Replace lifetime aggregate queries with an atomically maintained domain projection containing:
    - local high-water nonce;
    - reservation count/chain head;
    - exact active reservation key;
    - current resource frontier;
    - current incarnation/release reference.

- Normal status loads the domain row, at most one exact reservation, the bounded candidate prefix,
  and optional completion.
- Keep historical reservations append-only behind indexed exact-key lookup.
- Separate immutable activation from current incarnation.
- Require a fresh provider-issued current-incarnation lease for status and a non-cloneable
  transaction permit for reserve/activate/complete.
- Update live/app assembly to accept a promoted successor release whose history descends from the initial release.
- Replace the wallet SQL baseline with no old reader.

Proofs:

- Same token/same semantics resolves; changing each semantic field conflicts.
- Different token/same semantics is a distinct intent.
- Tenant/principal separation follows authenticated issuer identity.
- Issuer namespace rotation is allowed only as the documented new-sender/domain cutover.
- Large completed lineage keeps constant bounded query count and indexed `EXPLAIN` shape.
- App assembles and uses a promoted authority, while the old authority fails every subsequent protected access.
- Current projection and audit lineage reconcile in a test-only full verifier.

### 10. make evm recovery observation first and producer authorized

Closes: TT2-EVM-01 through TT2-EVM-04, TT2-EVM-09, and the remaining TT2-EVM-12 matrix.

State model:

```text
RetainedCandidate
  -> observe transaction
  -> observe receipt
  -> optionally observe finality/inclusion
  -> fresh wallet status
  -> Completed
     | EligibleForExactResubmission
     | EligibleForNextReplacement
     | ContinueWaiting
     | ExactReviewedFailure

NewCandidate
  -> attest
  -> activate
  -> NewlyActivated        -> initial broadcast
     | AlreadyRetained     -> retained observation path
     | Reconcile           -> fresh status
```

Changes:

- Remove `Reobservation -> Activated -> Broadcast`.
- Give initial broadcast and retained resubmission distinct request/permit types.
- Permit retained resubmission only after committed "submission still needed" observations and an
  unchanged final status.
- Create `CandidateReplacementEvidence` from exact status, transaction, receipt, head, prefix,
  policy, and decision-chain producers.
- `Runtime` passes the committed access-origin proof to the adapter.
- The adapter validates certified producer relationships and mints a non-cloneable
  `QualifiedReplacementActivation`.
- PostgreSQL consumes it under the domain lock and rechecks current reservation, full prefix,
  predecessor, next ordinal, policy, and frontier.
- Exact operation-key retry resolves the original activation; changed input conflicts.
- Route every definite EVM submission failure, including family exhaustion, through an explicit final status read.
- Replace `CompletedWalletNonce` with one complete content-addressed closure containing full intent,
  family, reservation, activated prefix, public signer/route/release evidence, terminal
  observations, decisions, canonical public output, and every required preimage.

- Project application output solely from that closure.

Proof matrix:

- Different-run recovery after crashes before/after provider receipt, receipt visibility, finality,
  completion SQL commit, and acknowledgement.
- Observation-first traversal of multiple retained candidates, including an older winning hash.
- Accepted-by-provider followed by timeout, disconnect, invalid JSON, or process loss.
- Missing, fabricated, stale, cross-run, cross-reservation, reordered, changed-prefix, and consumed
  replacement evidence.
- Completion immediately before and after every final status linearization.
- Completion closure alone rehashes every member and produces byte-identical public output.
- Fresh unique-intent production test uses only `connect_production_application`, an empty wallet
  authority, the real qualified keystore, one broadcast, provider observations, completion, process
  restart, and callback-free projection. No deterministic worker may pre-complete that intent.

### 11. delete replay reproduction and generate exact projection contracts

Closes: TT2-REPLAY-02 and the projection portion of TT2-REPLAY-05.

Changes:

- Delete `ReplayRequest::Reproduce`, its input, unavailable result, dispatch, CLI/REST values,
  error paths, tests, and documentation.
- `replay_run` becomes recorded verification only.
- Replace generic `StructuredCanonicalProjection` and caller-provided schema names with concrete
  generated DTOs:
    - replay result;
    - transition trace entry/page;
    - access audit entry/page;
    - public run view.

- Obtain every schema identity and decoder from the recoverability annex.
- Generate or byte-check the annex README.
- Delete hard-coded projection schema formulas and old fixtures.

Proofs:

- Repository-wide search finds no reproduce/comparison input, mode, wire value, or unavailable result.
- Old requests fail current parsing.
- Arbitrary schema names cannot be decoded or emitted.
- Generated schemas, Rust types, CLI JSON, REST bodies, corpus, and README agree byte-for-byte.

### 12. export recursively verified portable evidence

Closes: TT2-REPLAY-01, TT2-REPLAY-03, TT2-REPLAY-04, remaining TT2-REPLAY-05, and TT2-APP-02.

Portable format:

- Replace the monolithic `+json` artifact with one current bounded frame-stream media type.
- Each frame is canonical JSON within the generated frame limit.
- Frames carry ordinal, kind, previous-frame digest, and payload.
- A final seal binds total counts/bytes, root run, export kind, every run fixation,
  authorization-closure reference, and the final frame-chain digest.

- Deterministic order is root first, then dependency category, run ID, append sequence, object
  reference, and publication coordinate.
- Old monolithic bytes are rejected; there is no compatibility decoder.

Export flow:

1. Fold root metadata and derive semantic sources plus completeness-witness producer prefixes.
2. Recursively discover the bounded graph, including dense fact publication routes through every captured frontier.
3. Authorize every root, semantic source, and completeness producer for the same target, tenant,
   principal, and export purpose.
4. Freeze exact required heads and physical fixation in an opaque `AuthorizedExportClosure`.
5. Open no output spool until the closure is complete.
6. Load exact prefixes and emit deterministic frames.
7. Commit the spool only after the final seal succeeds.

Offline verification:

- Parse frame lengths before allocation.
- Verify canonical bytes, chain digests, counts, exact closure, and final seal.
- Fold each run once at its maximum required head.
- Re-evaluate prior-fact selection against bundled dense routes and producer transitions.
- Derive source relationships from folded observations; reject self-listed disagreement.
- Verify tenant/target/purpose equality, writer lineage, retained physical release histories, source
  heads, publication coordinates, and frontiers.
- Use only bundle bytes plus an explicit trust snapshot containing program trust, retained
  physical-release trust, store/checkpoint trust, and the exact authorized-closure reference.

- Perform no store, provider, signer, state callback, or application-policy call.

Selection rules:

- Semantic export ends at exactly the batch containing the semantic head, including every adjacent
  record such as `RunClosed`.
- Audit export ends at exactly its declared journal head.
- Encoder and decoder use the same cutoff function.

Proofs:

- Online and isolated offline projections match byte-for-byte.
- Omitted, extra, substituted, reordered, stale-head, false-frontier, false-publication,
  wrong-tenant, wrong-target, cyclic, shared, and over-budget dependency cases fail.

- A semantic bundle with one later audit batch fails; the corresponding audit bundle succeeds.
- Exact frame and total bundle limits succeed; one-byte-over fails before full allocation.
- Single-large-frame and many-small-frame cases are covered.
- Denied dependencies emit zero bytes and one reviewed redacted error.

### 13. record tt2 runtime remediation review

Closes: TT2-QUALITY-01, TT2-VERIFY-01, and TT2-PROCESS-01 only after every implementation proof is present.

The engineer must stop at the candidate implementation revision and request an independent
skeptical review. The reviewer records:

- exact implementation and review revisions;
- every TT2 item and its production-code disposition;
- every focused regression and whether it fails against `a4dada89`;
- authority/API deletion searches;
- PostgreSQL role, snapshot, checkpoint, contention, and fault matrices;
- EVM production, crash, replacement, promotion, and scale matrices;
- portable golden/tamper/budget corpus;
- secret/redaction audit;
- hand-written production LOC and public authority changes;
- unverified risks and `Material uncertainties`;
- a `PASS` only if all Blocker/High requirements and their evidence are complete.

Any finding is fixed in its owning implementation area before the review is finalized. Evidence
prose cannot waive a required proof.

## Required deletion audit

The final tree must contain none of the following:

- `NewlyAppendedAuthorization`, `from_store_mint`, public binding invocation, or production
  registry splitting;
- certification-owned live process authority;
- raw PostgreSQL login/session materials, pool-taking openers, local self-fence permits, or
  generation-specific writer locks;
- aborted-transaction classifiers or multi-snapshot full loads;
- backend-specific canonical acceptance;
- line-based SQL inventory;
- repeated full producer-prefix folds;
- broad `VerifiedStructuredRun` purpose wrappers;
- inline movable plaintext key arrays;
- duplicate signer invocation or integrity-to-availability mapping;
- behavior-bearing `SubmissionIntentId`;
- initial-incarnation-as-current comparisons;
- lifetime reservation scans;
- public/self-derived candidate activation permits;
- retained-candidate broadcast-before-observation;
- direct exhaustion/failure without final status;
- incomplete completion summaries;
- `ReplayRequest::Reproduce`, generic replay schema names, or hard-coded schema hashes;
- monolithic portable `+json`, caller-listed unproved source relationships, or later-audit semantic
  suffixes;
- old schema readers, migrations, aliases, fallbacks, and fixtures.

## Traceability

| Commit | TT2 findings |
| --- | --- |
| 1 | `TT2-AUTH-01` |
| 2 | `TT2-AUTH-02`, `TT2-STORE-01` |
| 3 | `TT2-STORE-02`, `TT2-STORE-03`, `TT2-STORE-04` |
| 4 | `TT2-STORE-05`, `TT2-STORE-06` |
| 5 | `TT2-STORE-07` |
| 6 | `TT2-APP-01` |
| 7 | `TT2-SEC-01` |
| 8 | `TT2-EVM-08`, `TT2-EVM-10`, `TT2-EVM-11`, part of `TT2-EVM-12` |
| 9 | `TT2-EVM-05`, `TT2-EVM-06`, `TT2-EVM-07`, part of `TT2-EVM-12` |
| 10 | `TT2-EVM-01`, `TT2-EVM-02`, `TT2-EVM-03`, `TT2-EVM-04`, `TT2-EVM-09`, remainder of `TT2-EVM-12` |
| 11 | `TT2-REPLAY-02`, part of `TT2-REPLAY-05` |
| 12 | `TT2-REPLAY-01`, `TT2-REPLAY-03`, `TT2-REPLAY-04`, remainder of `TT2-REPLAY-05`, `TT2-APP-02` |
| 1–12 | `TT2-QUALITY-01`, `TT2-VERIFY-01` |
| 13 | `TT2-PROCESS-01` and final `TT2-QUALITY-01`/`TT2-VERIFY-01` audit |

## Verification strategy

During implementation:

- Run direct Cargo/Rust tooling only through `nix develop`.
- Use package-scoped formatting, Clippy/checks, named tests, and compile-fail suites while iterating.
- Run `cargo-metadata-contract` after the runtime/certify/store dependency and public API cutover.
- Run focused PostgreSQL leaves after commits 2–5:

  - `postgres-sql-inventory-check`;
  - `postgres-sqlx-offline-check`;
  - `postgres-sqlx-check`;
  - `recoverability-postgres-v1`;
  - new external-fence/snapshot/fault leaves.

- Run wallet and EVM leaves after commits 8–10:

  - `wallet-nonce-postgres-storage-qualification`;
  - `evm-postgres-submission-qualification`;
  - `wallet-nonce-postgres-qualification`.

- Add a no-service portable corpus leaf for commits 11–12.
- Run `nix run .#model-check` after finalizing the changed Nixfied task graph.
- On the exact final reviewed revision, run `nix run .#ci` once. Do not precede it with separate
  `.#check`, `.#test`, and `.#test-db` runs on the same tree.
- Run range-based hygiene, specifically `git diff --check a4dada89..HEAD`, plus generated-contract
  and documentation-link checks.

Completion requires every regression to assert an exact denial, durable prefix, callback count,
semantic outcome, or resource bound—not merely that a broad gate passed.
