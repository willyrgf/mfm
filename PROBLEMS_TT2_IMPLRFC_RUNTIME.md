# Problems Found in the TT2 Runtime-History Remediation

## Status and scope

This document began as the handoff problem ledger for the remediation implementation reviewed at
`a4dada89`. Its historical findings remain useful for traceability, but the current disposition
is the re-audit recorded below.

- branch: `refact-runtime`;
- current implementation/evidence source tip: `0a4b02e1b947609bb0b9dc09be3606e8aa513ebf`;
- current evidence/documentation head before this refresh: `886fee0941c42838705f1f29662d43cc80ee7b72`;
- normative proposal: [RFC_RUNTIME_HISTORY_CHOKE_POINT.md](RFC_RUNTIME_HISTORY_CHOKE_POINT.md);
- first implementation problem ledger:
  [PROBLEMS_TT1_IMPLRFC_RUNTIME.md](PROBLEMS_TT1_IMPLRFC_RUNTIME.md);
- remediation plan:
  [IMPL_PLAN_PROBLEMS_TT1_IMPLRFC_RUNTIME.md](IMPL_PLAN_PROBLEMS_TT1_IMPLRFC_RUNTIME.md);
- implementation review record:
  [TT1_RUNTIME_REMEDIATION_REVIEW.md](TT1_RUNTIME_REMEDIATION_REVIEW.md).

The current production code being assessed is the source tree at `0a4b02e1`; the detailed finding
sections below retain the original `a4dada89` observations as historical traceability.

The implementation was reviewed for the three properties required of this platform core:

- correctness: authority, ordering, persistence, recovery, and settlement must reject invalid
  states or make them unrepresentable;
- dexterity: operations, injected states, capabilities, transports, storage, and replay must remain
  composable without bypassing their guarantees;
- simplicity: each responsibility must have one owner and one current path, with minimal public
  authority, duplicated decisions, change sites, concepts, and LOC.

This is a problem statement for an architect, not an implementation plan. The required resolution
properties below describe outcomes. They deliberately do not freeze a mechanism before the
architecture resolves the material uncertainties.

## Historical review verdict at `a4dada89`

The recorded final `GATE: PASS` is not supported by the reviewed tree. The implementation contains
substantial and valuable remediation, but it is not a complete or safe end state for the RFC or the
TT1 problem ledger.

The reported model check and 12/12 composed CI result are treated as credible. Rerunning the same
broad gates would not answer the findings below because most sit at authority, concurrency,
cross-process, recovery, and offline-verification boundaries absent from those gates.

The conservative disposition of the 40 original TT1 problems is:

- 12 closed;
- 13 partial;
- 15 open.

None of the eight replacement-implementation acceptance thresholds in the TT1 ledger can be signed
off. In particular, public live-access authority remains forgeable, the generic PostgreSQL target
has no external non-rollback fence, EVM recovery can broadcast retained candidates before
chain-state reobservation, replacement eligibility is not producer-bound, the
required fresh production keystore signing/broadcast proof was not exercised, and portable source
relationships cannot be verified offline.

## Current re-audit at `0a4b02e1`

The later implementation revisions and focused evidence supersede the historical disposition
above. Runtime access, PostgreSQL snapshot/head fixation, contention recovery, EVM recovery and
authority qualification, replay reproduction removal, semantic/audit suffix handling, portable
byte budgets, purpose-bound projections, key-cleanup witnesses, and recursive export closure now
have implementation and focused regression coverage. The current evidence ledger records those
scopes as closed or conditional, with remaining work including deployment trust,
cross-process/fault matrices, complete generated/hostile/high-scale corpus breadth, live application multi-hop and
production evidence, independent provider/public-result verification, external
termination/OOM/resource-failure coverage, and the full simplicity/isolation acceptance matrix.

The current matrix is authoritative for this re-audit; the detailed sections below preserve the
original finding text and line references for traceability.

## Classification

### Severity

- Blocker: invalidates a central authority or recovery guarantee, defeats a required fence, permits
  an unsafe irreversible action, or makes a required production/replay path semantically unsound.
- High: violates a central frozen contract or leaves a serious correctness, security, operability,
  dexterity, or acceptance gap.
- Medium: violates a lower-risk conformance or leaves important performance, hardening, or
  maintainability debt. Medium findings still belong in this core cutover.

### Disposition

- Open: the required contract is absent or contradicted.
- Partial: meaningful remediation exists, but the complete property is not established.
- Proof gap: the implementation might satisfy the property, but the required boundary evidence is
  absent or the cited test does not exercise it.
- Contract conflict: the implementation relies on a different trust, identity, or ownership model
  from the RFC or frozen remediation plan.
- Closed: both the implementation property and its exact hostile/regression proof are present.

## Historical material uncertainties at `a4dada89`

These choices require an architect decision before implementation planning. They do not make the
current `GATE: PASS` acceptable.

Several surrounding contracts are already frozen and are not reopened here: Runtime owns the live
access bracket, deployment infrastructure owns the external non-rollback fence, each PostgreSQL
load observes one snapshot, recovery observes retained work before conditionally resubmitting it,
purpose types restrict obtainable data, and `SubmissionIntentId` contains only nonce domain,
authenticated issuer, and bounded caller token while a separate digest binds semantic meaning. The
uncertainties below concern the minimal representation or the unresolved deployment trust boundary.

### PostgreSQL credential trust boundary

Choice or assumption:

The remediation plan says ordinary assembly receives only an opaque session bundle and requires
proof that retained input credentials cannot bypass qualification. The implementation instead
publishes cloneable raw login URLs and declares their holder part of the deployment TCB.

Why uncertain:

It is unclear whether expanding the deployment TCB to include direct table-mutation authority was
an intentional contract change or an implementation shortcut.

Consequence if wrong:

A retained writer login bypasses the store fold, exact-head classifier, and Runtime choke point.

How to resolve:

Freeze one threat boundary. Either make raw credentials inaccessible after a one-shot deployment
broker issues non-exportable capabilities, or deliberately amend the plan and security model to
state that the credential issuer can forge all history and explain what guarantee remains.

### Portable source evidence

Choice or assumption:

Offline verification of prior-run facts may require either recursively bundled source histories or
an independently authenticated publication proof that is sufficient without those histories.

Why uncertain:

The current artifact contains only root batches and source IDs that are not checked against folded
evidence. The current design and remediation plan require portable source relationships but do not
reduce that requirement to one concrete wire representation.

Consequence if wrong:

Artifacts can claim false source relationships or cannot reproduce the facts that affected the
root run.

How to resolve:

Specify the minimal complete offline proof, its trust snapshot, recursive bounds, fixation, and
tamper rules before changing the format.

### Wallet incarnation currentness

Choice or assumption:

The live wallet binding needs an authority-owned value or permit naming the current promoted
physical incarnation; the immutable activation's initial incarnation is not that value.

Why uncertain:

Promotion exists in the PostgreSQL wallet authority, but the public authority interface exposes
only the original activation attestation to application/live binding assembly.

Consequence if wrong:

Either legitimate promoted authorities cannot be assembled or stale releases can be mistaken for
current authority.

How to resolve:

Assign one owner for current incarnation and make every protected access consume its fresh,
non-cloneable proof. Exercise initial, promoted, stale, copied, and mismatched targets end to end.

### Purpose-limited data authority

Choice or assumption:

Purpose limitation requires each holder to be structurally unable to inspect data outside its
projection. The unresolved choice is the minimum exact field/object set for public, trace, audit,
replay, and export projections.

Why uncertain:

The plan freezes data isolation but does not enumerate every field required by all five current
product projections, while several current wrappers expose complete record/object collections.

Consequence if wrong:

An over-broad projection preserves the current authority defect; an under-broad projection breaks a
retained product or forces consumers to regain generic history access.

How to resolve:

Inventory the retained consumers, define the minimum projection for each purpose, then expose only
those values and add cross-purpose compile-fail plus data-isolation tests.

### Runtime/store access-proof representation

Choice or assumption:

Runtime ownership is frozen, but the minimum crate-private representation connecting an exact
store append result to one physical invocation has not been selected.

Why uncertain:

The current design put the token and invocation in certification to avoid a dependency problem;
simply changing visibility may make the production store adapter or isolated Runtime tests
impossible to assemble.

Consequence if wrong:

An overly public port recreates TT2-AUTH-01; an overly coupled port creates a crate cycle or a
second test-only authority path.

How to resolve:

The architect must choose one dependency direction and private affine proof flow, including how the
store mints it and how Runtime tests use caller-owned fake resources without exposing production
authority.

### PostgreSQL snapshot and fixation mechanism

Choice or assumption:

One load must observe a single exact prefix and its indexed head. The unresolved mechanism is a
repeatable-read snapshot, one-statement projection, explicit lock, or another equivalent design.

Why uncertain:

The choice affects contention, streaming/size behavior, configuration parity, cache semantics, and
how an external fence is exercised during a read.

Consequence if wrong:

The store can retain torn reads, serialize unrelated readers/writers, or make the index an
unintended source of semantic authority.

How to resolve:

Choose one snapshot protocol for run and configuration reads, specify its linearization point, and
prove old-or-new prefix behavior under deterministic interleavings.

### EVM retained-candidate and replacement proof shape

Choice or assumption:

Recovery is observation-first and replacement needs independent producer evidence. The unresolved
choice is the smallest set of typed states and committed evidence that proves those facts without
duplicating the operation scheduler inside wallet authority.

Why uncertain:

The current count/digest design is too weak, while copying every provider object into every permit
could create unnecessary schemas, storage, and change sites.

Consequence if wrong:

The design either remains self-certifying or becomes too complex and inflexible for new transports
and terminal policies.

How to resolve:

Define one affine producer-bound eligibility value, its exact producing states, validation owner,
freshness frontier, and consumption rule before changing the operation expansion.

### Canonical annex and transport budget ownership

Choice or assumption:

One generated annex should own each canonical frame/document byte, item, depth, and numeric bound,
but the correct split between shared canonical limits and transport/product-specific limits has not
been selected.

Why uncertain:

Forcing every portable document under a per-value 16 MiB limit may be too restrictive, while
allowing an outer 512 MiB document whose inner canonical decoder rejects it is internally
contradictory. Provider proofs and configuration values introduce related but not identical risks.

Consequence if wrong:

The encoder can produce undecodable artifacts, or the fix can create one oversized universal limit
that permits avoidable memory/CPU denial of service across unrelated surfaces.

How to resolve:

Inventory every encoded boundary, assign exactly one generated owner to each whole-document and
nested-frame budget, and prove compositional limits plus bounded preallocation before decoding.

## Executive problem index

| ID | Severity | Disposition | Problem | Primary TT1 mapping |
| --- | --- | --- | --- | --- |
| TT2-AUTH-01 | Blocker | Closed | Live physical access is assembled behind Runtime-owned, marker-sealed authority | AUTH-01 |
| TT2-AUTH-02 | Blocker | Closed | Production PostgreSQL login material is issued through an opaque deployment admission | AUTH-03 |
| TT2-STORE-01 | Blocker | Conditional | The generic PostgreSQL target still needs concrete production external-fence integration | AUTH-04, STORE-02 |
| TT2-STORE-02 | High | Conditional | Snapshot/head validation passes, but cross-process acknowledgement and fault matrices remain | STORE-03, STORE-04 |
| TT2-STORE-03 | High | Closed | Full-history loads verify the indexed head inside the same backend snapshot | STORE-03 |
| TT2-STORE-04 | High | Closed | Contention classification rolls back before bounded identity reconciliation | STORE-01 |
| TT2-STORE-05 | Medium | Conditional | Shared canonical ingress aligns bounded memory/PostgreSQL inputs; bounded shape and sequential-scale corpus passes, while the complete generated/hostile/high-scale corpus remains unverified | STORE-04 |
| TT2-STORE-06 | Medium | Conditional | AST inventory covers generic and builder forms; complete ownership/scale audit remains | STORE-05 |
| TT2-STORE-07 | High | Closed | Prior-fact verification loads each required producer prefix once per scan | STORE-06 |
| TT2-EVM-01 | High | Closed | The production path drives a fresh semantic intent through the qualified keystore signer | EVM-01, VERIFY-01 |
| TT2-EVM-02 | Blocker | Closed | Recovery reobserves retained candidates before any conditional broadcast | EVM-03 |
| TT2-EVM-03 | Blocker | Closed | Replacement activation consumes producer-authorized retained-prefix evidence | EVM-04 |
| TT2-EVM-04 | High | Closed | Candidate-family exhaustion reconciles final wallet status before closure | EVM-03, EVM-04 |
| TT2-EVM-05 | High | Closed | Stable caller intent and separate semantic digest conflict behavior are implemented | EVM-08 |
| TT2-EVM-06 | High | Closed | Wallet release currentness follows registered historical incarnations and promotion | EVM-11 |
| TT2-EVM-07 | High | Conditional | Normal status uses bounded projections; latency/scale and strict plan breadth remain | EVM-10 |
| TT2-EVM-08 | High | Closed | Signer integrity failures remain integrity outcomes | EVM-07 |
| TT2-EVM-09 | High | Conditional | Completion closure and reload pass; independent provider/public-result verification remains | EVM-09 |
| TT2-EVM-10 | Medium | Closed | `u64::MAX` is rejected before pending-nonce observation and persistence | EVM-06 |
| TT2-EVM-11 | High | Closed | Pending-floor route, policy, and semantics are authority-qualified | EVM-02, EVM-08 |
| TT2-EVM-12 | High | Conditional | Release/restart qualification passes; crash, ambiguity, promotion, replacement, and scale matrices remain | EVM-05, VERIFY-01 |
| TT2-REPLAY-01 | Blocker | Conditional | Recursive source proof, fixation, trust, bounds, and parity are implemented; live app multi-hop proof remains | PROV-01, APP-01, REPLAY-01 |
| TT2-REPLAY-02 | High | Closed | Production reproduction/current-history comparison is deleted | APP-02, REPLAY-01 |
| TT2-REPLAY-03 | High | Conditional | Exact semantic cutoff and kind-aware suffix rejection pass; live production evidence remains | REPLAY-01 |
| TT2-REPLAY-04 | High | Closed | Encoder/decoder frame and total budgets share annex limits | REPLAY-01 |
| TT2-REPLAY-05 | High | Conditional | Generated schema/corpus and online/offline parity pass; complete live matrix remains | REPLAY-03 |
| TT2-APP-01 | High | Conditional | Purpose products expose bounded status/projection data; the complete isolation matrix remains | AUTH-05 |
| TT2-APP-02 | High | Conditional | Recursive closure is retained and graph-checked; live production multi-hop proof remains | PROV-01, APP-01 |
| TT2-SEC-01 | High | Conditional | Protected allocation and cleanup witnesses pass; external termination/OOM/resource classes remain | SEC-01 |
| TT2-QUALITY-01 | High | Conditional | Duplicate paths were removed, but core ownership and LOC remain concentrated | QUALITY-01 |
| TT2-VERIFY-01 | High | Conditional | Broad/focused gates pass; the complete acceptance matrix remains | LANG-05, VERIFY-01 |
| TT2-PROCESS-01 | High | Conditional | Current evidence preserves residuals and exact provenance; strict PASS remains withheld | PROCESS-01 |

## Runtime and storage authority problems

### TT2-AUTH-01 — Live physical access can bypass Runtime and committed authorization

Severity: Blocker

Disposition: Open

Violated contract:

Runtime must own the complete private access bracket: prepare an exact physical binding, append the
authorization through the store, receive a non-forgeable affine append result, invoke once, and
commit the observation. The application and unrelated crates must not be able to split the process
authority or mint either half of this protocol. The remediation plan requires this at
[lines 275-285](IMPL_PLAN_PROBLEMS_TT1_IMPLRFC_RUNTIME.md#L275) and explicitly deletes exposed
authorization tokens and certify/store coupling at
[lines 299-309](IMPL_PLAN_PROBLEMS_TT1_IMPLRFC_RUNTIME.md#L299). The RFC describes the authorization,
invocation, and observation bracket at
[lines 122-148](RFC_RUNTIME_HISTORY_CHOKE_POINT.md#L122), the append-bound proof at
[lines 2644-2669](RFC_RUNTIME_HISTORY_CHOKE_POINT.md#L2644), the affine exactly-one-entry guarantee
at [lines 1161-1167](RFC_RUNTIME_HISTORY_CHOKE_POINT.md#L1161), and prepared-access observation at
[lines 2711-2742](RFC_RUNTIME_HISTORY_CHOKE_POINT.md#L2711).

Current implementation:

- [`CommittedAccessAuthorization`](crates/kernel/certify/src/structured.rs#L138) has private fields,
  but its alleged store mint, [`from_committed_successor`](crates/kernel/certify/src/structured.rs#L157), is
  public and requires no private store capability.
- [`ExternalAccessAuthorized`](crates/kernel/journal/src/structured.rs#L902) is publicly
  constructible because all fields are public.
- [`QualifiedProgramRegistry::into_runtime_parts`](crates/kernel/certify/src/structured.rs#L3877)
  publicly gives a caller the live `RuntimeProcessRegistry` half.
- That registry publicly exposes
  [`prepare_access`](crates/kernel/certify/src/structured.rs#L4030) and
  [`invoke_authorized_physical_binding`](crates/kernel/certify/src/structured.rs#L4234).
- The opaque binding publicly reveals its frozen request and certificate at
  [lines 1197-1206](crates/kernel/certify/src/structured.rs#L1197).
- [`ExpectedAuthorization::matches`](crates/kernel/certify/src/structured.rs#L1645) compares
  caller-constructible values but no proof that the record was appended. It omits the attempt ID,
  attempt ordinal, occurrence path, semantic call ID, semantic head, and exact persisted request
  reference.
- The compile-fail fixture's expected compiler output actually suggests calling
  `CommittedAccessAuthorization::from_committed_successor` in
  [construct_committed_access_authorization.stderr](crates/kernel/certify/tests/ui/structured/fail/construct_committed_access_authorization.stderr).

Minimal failure scenario:

1. A consuming crate builds and splits a qualified registry.
2. It asks the process registry to prepare a binding for values it supplied and can recompute.
3. It constructs a matching `ExternalAccessAuthorized` and arbitrary `RecordRef` with the expected
   run ID.
4. It calls the public `from_committed_successor(..., None)` without appending any history.
5. It consumes that value in the public invocation method and enters the effect.

Consequence:

The physical effect can occur with no committed authorization, no exact-head serialization, no
Runtime cursor transition, and no guaranteed observation. The central history choke point is
therefore forgeable even though direct history writer APIs were narrowed.

Required resolution properties:

- Only a successful store append result can mint the affine authorization value.
- Only Runtime can own the process registry, binding preparation, append result, and invocation
  entry point in production assembly.
- The append proof binds every field relied on by invocation, including exact attempt identity and
  committed record reference.
- Certification contains no live process/invocation authority.

Required proof:

An external consuming-crate compile-fail matrix must attempt registry splitting, store-mint calls,
binding preparation, binding invocation, fabricated authorization, reuse, and cross-binding
substitution. A runtime test must show that one newly committed authorization permits exactly one
matching invocation and no other path can enter it.

### TT2-AUTH-02 — Raw PostgreSQL credentials survive opaque-bundle qualification

Severity: Blocker

Disposition: Contract conflict

This severity applies under the remediation plan's frozen retained-credential threat model. If that
model is deliberately amended, the architect must reclassify the issue against the replacement
security model rather than silently treating the current implementation as proof.

Violated contract:

The remediation plan requires production openers to consume opaque deployment-issued bundles,
remove URL/pool inputs, and prove that retained credentials cannot bypass target authority at
[lines 476-503](IMPL_PLAN_PROBLEMS_TT1_IMPLRFC_RUNTIME.md#L476).

Current implementation:

- [`SessionLoginMaterial`](crates/storages/postgres/src/session.rs#L245) is public, `Clone`, and has
  a public raw `database_url` field.
- Public issuance functions accept those values directly, including
  [`issue_application_sessions`](crates/storages/postgres/src/session.rs#L285).
- A valid login is `NOINHERIT` at
  [session.rs lines 640-650](crates/storages/postgres/src/session.rs#L640) but retains `SET`
  authority over the managed run-writer role as checked at
  [lines 653-700](crates/storages/postgres/src/session.rs#L653); transaction setup uses that authority
  through `SET LOCAL ROLE` at
  [transaction.rs line 250](crates/storages/postgres/src/transaction.rs#L250).
- The managed role grants `INSERT, UPDATE` on `run_history_heads` and `INSERT` on batches and batch
  objects at [0001_store.sql lines 613-621](crates/storages/postgres/migrations/0001_store.sql#L613).
- Source documentation changes the premise by declaring every holder of the login material inside
  the deployment TCB at [session.rs lines 240-244](crates/storages/postgres/src/session.rs#L240).

Failure scenario under the plan's stated threat model:

A deployment caller clones the run-writer URL, passes one copy to session issuance, and retains the
other. The retained login can issue DML directly without the store fold, canonical append contract,
exact-head classification, or Runtime.

Consequence:

The claimed opaque session does not revoke or contain its input authority. Whether this is a direct
vulnerability depends on the unresolved TCB decision, but it unquestionably fails the plan's
required retained-input proof.

Required resolution properties and proof:

- Freeze the credential issuer's authority in the architecture and threat model.
- If callers are outside semantic-history trust, issuance must consume a non-exportable or
  one-shot broker capability and leave no usable raw login with them.
- A production-shaped hostile test must retain every issuance input and prove it cannot perform raw
  DML, select another role, or open a copied/stale target.

### TT2-STORE-01 — The generic PostgreSQL target lacks the external non-rollback fence

Severity: Blocker

Disposition: Open

Violated contract:

The current generic store contract gives RunHistory responsibility for writer generation and
non-rollback lineage at [RFC lines 4247-4268](RFC_RUNTIME_HISTORY_CHOKE_POINT.md#L4247). The current
architecture requires opaque deployment-issued sessions and a per-transaction fence-generation
check at [docs/architecture.md lines 120-127](docs/architecture.md#L120). The remediation plan makes
deployment infrastructure the authoritative external-fence boundary and requires every transaction
to acquire fresh external currentness at
[lines 476-500](IMPL_PLAN_PROBLEMS_TT1_IMPLRFC_RUNTIME.md#L476).

The RFC's more concrete wallet-nonce design separates a target-held non-exportable signing key from
a deployment-owned non-rollback fence authority at
[lines 3155-3186](RFC_RUNTIME_HISTORY_CHOKE_POINT.md#L3155); it should not be misread as saying that
the generic RunHistory store must use that exact key mechanism.

Current implementation:

- [`TargetPermit`](crates/storages/postgres/src/transaction.rs#L13) is only a clone of fields already
  present in `TargetBinding`; `from_binding` performs no external issuance.
- Per-transaction validation reads `target_authority` from the same database being trusted at
  [transaction.rs lines 269-347](crates/storages/postgres/src/transaction.rs#L269).
- The writer advisory-lock key includes the locally asserted fence generation at
  [transaction.rs lines 274-284](crates/storages/postgres/src/transaction.rs#L274). Old- and
  successor-generation writers therefore lock different keys, while all unrelated writes under one
  generation serialize on one target-wide key. The authority query itself does not lock an
  independent fence record.
- There is no independent fence service, externally signed current-generation proof, or
  irreversible external checkpoint in this generic store/configuration path.
- Role identity is derived only from `md5(schema_name)` in
  [roles.rs](crates/storages/postgres/src/roles.rs#L30) and the migration
  [baseline](crates/storages/postgres/migrations/0001_store.sql#L7). A restored same-schema database
  recreates the same purported authority names and carries its local authority row with it.
- The regression named
  [`coordinated_configuration_rollback_is_visible_to_fresh_sessions`](crates/storages/postgres/tests/structured_history.rs#L855)
  rewinds both the configuration head and its revision, then explicitly expects fresh sessions to
  reopen successfully at [lines 920-927](crates/storages/postgres/tests/structured_history.rs#L920).

Failure scenario:

A database snapshot is restored or copied together with `target_authority`, configuration/history
heads, ACLs, and schema-local data. Because all evidence is inside that copied rollback domain, the
copy certifies itself as current. Application sessions can read stale state and accept new writes.

Consequence:

The platform cannot distinguish the authoritative target from a coordinated rollback or copied
target. Append-only history, configuration currentness, exact-head authority, and physical
rotation guarantees do not survive the failure the fence was introduced to handle. The lock shape
also imposes target-wide write serialization without proving mutual exclusion across generations.

Required resolution properties:

- Every read or write transaction obtains a fresh proof from authority outside the rollback domain.
- The proof binds exact database identity, schema, store scope/epoch, target key, release epoch, and
  fence generation.
- Restores and copies cannot mint or replay current authority.
- Rotation serializes old and successor writers through one irreversible lineage.

Required proof:

Copied-target, coordinated-rollback, stale-generation, wrong-database, same-schema/different-
database, and promotion crash tests must fail closed unless the independent authority explicitly
opens the exact successor.

### TT2-STORE-02 — PostgreSQL full reads are not one consistent snapshot

Severity: High

Disposition: Open

Violated contract:

One verified history load must represent one database prefix. It must not combine rows observed at
different heads or manufacture `InvalidHistory` from a healthy concurrent append.

Current implementation:

- Run/configuration backend reads opened through `begin_base` use `READ COMMITTED` at
  [transaction.rs lines 222-248](crates/storages/postgres/src/transaction.rs#L222), so each SQL
  statement obtains a new snapshot. Qualification separately uses a read-only `REPEATABLE READ`
  transaction; this finding concerns the ordinary full-load path after session issuance.
- A run load selects batches first and objects second at
  [structured.rs lines 244-267](crates/storages/postgres/src/structured.rs#L244), then rejects any
  object rows without a batch at [lines 269-272](crates/storages/postgres/src/structured.rs#L269).
- Configuration load likewise selects revisions and then its head in separate statements at
  [configuration.rs lines 199-208](crates/storages/postgres/src/configuration.rs#L199).
- The memory backends take one mutex snapshot, so this is also a backend semantic difference.

Minimal interleaving:

1. Reader statement one observes batches through head H.
2. A writer atomically commits H+1 and its objects.
3. Reader statement two observes objects through H+1.
4. Reconstruction sees H+1 objects with no H+1 batch and returns `InvalidHistory` for healthy data.

Configuration can similarly combine old revisions with a new head.

Consequence:

Fresh Runtime loads, cache misses, replay, audit, export, and configuration resolution can fail
spuriously under normal concurrent writes. Treating this as integrity failure can park otherwise
healthy runs and makes behavior timing-dependent.

Required resolution properties and proof:

- A multi-statement read uses one transaction snapshot, or one statement returns the complete
  prefix and its authoritative fixation.
- The loaded rows and exact indexed head are compared inside that same snapshot.
- Deterministic barriers must append between every read phase and prove the result is either the
  complete old prefix or complete new prefix, never a torn combination or false integrity fault.

### TT2-STORE-03 — Post-issuance full loads ignore the materialized run head

Severity: High

Disposition: Partial

Violated contract:

The final remediation defines the indexed head as the snapshot point for a non-mutating load at
[docs/design.md lines 148-155](docs/design.md#L148). A materialized index is not semantic authority,
but a full load cannot claim that snapshot point without comparing its folded prefix with the head
inside the same snapshot.

Current implementation:

- PostgreSQL full load reads only batch and object tables at
  [structured.rs lines 244-279](crates/storages/postgres/src/structured.rs#L244).
- [`current_head`](crates/storages/postgres/src/structured.rs#L282) separately reads only sequence
  and digest from `run_history_heads`.
- The runtime adapter uses that projection only when deciding whether to reuse its one-entry cache
  at [adapter.rs lines 200-209](crates/kernel/store/src/structured/adapter.rs#L200).
- On a cache miss or fresh process, `load_verified` folds the batch/object prefix without
  cross-checking the materialized head.
- Session issuance does call `validate_authoritative_schema_at`, whose prefix-integrity query rejects
  divergent heads and batches lacking heads at
  [schema.rs lines 881-894](crates/storages/postgres/src/schema.rs#L881). That is useful qualification
  evidence, but it occurs before subsequent loads and is not their same-snapshot fixation.

Consequence:

A missing, rewound, or divergent `run_history_heads` row introduced after session issuance is
ignored by an uncached full load. Session qualification and cache reuse have integrity checks, but
the general reader, replay, audit, or export load does not establish the claimed same-snapshot
fixation.

Required resolution properties and proof:

- Fresh and cached loads share one snapshot-and-fixation rule.
- Missing, ahead, behind, or digest-divergent head projections are rejected consistently.
- Tests use an already-issued session with an empty adapter cache, mutate each head field between
  issuance and load, and prove rejection. Separate tests interleave mutation with each read phase.

### TT2-STORE-04 — Contention recovery queries an aborted transaction

Severity: High

Disposition: Open

Violated contract:

Healthy exact-head contention and idempotent retry must return `ExistingSame`, `AppendConflict`, or
`StaleHead`; it must not collapse into generic availability because the classifier itself uses an
invalid PostgreSQL transaction.

Current implementation:

- A failed batch insert is recognized as contention at
  [structured.rs lines 750-770](crates/storages/postgres/src/structured.rs#L750).
- The SQLSTATE set includes unique violation, serialization failure, and deadlock at
  [lines 1193-1201](crates/storages/postgres/src/structured.rs#L1193).
- The code then calls `classify_existing_under_lock` on the same transaction, which immediately
  issues `SELECT` queries at [lines 1204-1224](crates/storages/postgres/src/structured.rs#L1204).

PostgreSQL marks a transaction failed after those errors. Further SQL cannot classify anything
until rollback.

Consequence:

The exact classifier advertised by the code is unreachable for the errors it claims to recover
from and can return `BackendUnavailable` instead of a domain result. Acknowledgement uncertainty
and retry behavior remain under-specified at precisely the concurrency boundary.

Required resolution properties and proof:

- Roll back the failed transaction, reacquire current target authority and the canonical lock, then
  re-read and classify.
- Add deterministic unique, serialization, deadlock, and commit-acknowledgement fault injection.
- Assert the exact durable prefix and semantic outcome for both contenders and every retry.

### TT2-STORE-05 — Memory/PostgreSQL canonical acceptance still diverges

Severity: Medium

Disposition: Partial

Violated contract:

Both backends must consume the same store-owned bounded canonical append/configuration values and
accept or reject an identical corpus.

Current implementation and evidence:

- `CanonicalRunAppend` validates the complete bounded batch envelope before either backend
  receives it; database envelope checks remain defense in depth.
- `CanonicalConfigurationAppend` now canonicalizes and bounds the complete serialized revision
  at `MAX_CONFIGURATION_REVISION_BYTES` before dispatch, so memory and PostgreSQL share the same
  ingress decision.
- Managed `run-2138056-1785988778488975922` from clean source tip `0a4b02e1` passes 18
  structured-history tests, including a memory/PostgreSQL corpus for positive, exact-limit,
  one-byte-over, stale-predecessor, idempotent-replay, 32 deterministic JSON shape, and 32
  sequential-successor outcomes. Numeric ordering of loaded PostgreSQL prefixes is fixed at
  the same source tip; the complete generated/hostile/high-scale corpus remains unverified.

Consequence:

The previously observed memory/production acceptance divergence is closed for the shared
boundary and covered vector set. A complete generated, hostile, and scale acceptance corpus is
still not independently reproduced, so the item remains Partial.

Required resolution properties and proof:

- Construct one bounded canonical batch envelope and one bounded configuration revision before
  backend dispatch.
- Make both backends accept only those values; database constraints remain defense in depth.
- Run one exact positive/negative/max-boundary corpus against both backends and compare outcomes.

### TT2-STORE-06 — SQL inventory has systematic false negatives

Severity: Medium

Disposition: Partial

Current implementation and evidence:

The inventory is now an AST visitor that recognizes direct and aliased SQLx calls, generic
scalar/query-as forms, checked SQLx macros, wrapped helpers, and `QueryBuilder` construction and
fragment methods. `7e467952` adds explicit syntax fixtures for each of those forms, and the source
inventory plus fixture tests pass 2/2. The dedicated `postgres-sql-inventory-check` task remains
the executable source gate; SQLx offline still does not type-check dynamically assembled SQL.

Consequence:

The former generic/builder false-negative class is covered by the scanner and fixtures. A complete
independent ownership and scale audit for every dynamic statement is still required before this
item can be closed.

Required resolution properties and proof:

- Use a syntactic/metadata mechanism that covers every SQL construction form, or forbid forms the
  gate cannot inspect.
- Maintain one explicit inventory of unavoidable dynamic SQL and its reason.
- Add fixture queries for generic scalar, query-as, builder, macro, and wrapped helper forms and
  prove the gate rejects each unowned form.

### TT2-STORE-07 — Prior-fact verification repeatedly refolds growing prefixes

Severity: High

Disposition: Partial

Current implementation:

The scanner pages a dense publication route, but for every publication it loads the complete
producer prefix at
[fact_scan.rs lines 360-378](crates/kernel/store/src/structured/fact_scan.rs#L360) and runs the full
callback-free verifier again at [lines 406-411](crates/kernel/store/src/structured/fact_scan.rs#L406).
The configured publication bound is as high as one million.

Failure shape:

If one producer run publishes facts at successively longer prefixes, scanning N publications can
rehydrate and refold prefixes 1 through N. Work and data transfer become quadratic even though the
route scan itself is paged.

Consequence:

A contractually admitted request can exhaust CPU, memory bandwidth, or database IO without
violating any byte/item bound. This is a denial-of-service and dexterity problem in a core reusable
read.

Required resolution properties and proof:

- Verify each producer prefix at most once per required fixation, or consume an indexed proof that
  does not refold every predecessor repeatedly.
- Bound total history bytes and verification work, not only publication count and result bytes.
- Add an adversarial same-run long-prefix benchmark/test that asserts asymptotic query and fold
  counts.

## EVM submission and wallet-authority problems

### TT2-EVM-01 — Fresh production keystore signing and broadcast were not exercised

Severity: High

Disposition: Proof gap

Required acceptance proof:

The TT1 acceptance threshold requires one fresh unique semantic intent to traverse the real
production application, qualified RPC, PostgreSQL wallet authority, qualified keystore signer,
broadcast, observation, canonical completion, restart, and callback-free projection.

Current test:

- The qualification first runs direct worker phases at
  [lines 346-368](tests/integration/tests/evm_postgres_submission.rs#L346).
- Direct assembly uses a `DeterministicTestSigner` constructed by
  [`deterministic_signer`](tests/integration/tests/evm_postgres_submission.rs#L2297), called at
  [lines 2038-2044](tests/integration/tests/evm_postgres_submission.rs#L2038).
- Those phases broadcast and complete the only wallet reservation before production application
  phases start.
- Production does construct a real qualified keystore signer at
  [lines 1016-1022](tests/integration/tests/evm_postgres_submission.rs#L1016), but submits the same
  `SUBMISSION_TOKEN`, tenant, principal, domain, and transaction intent.
- The final assertion explicitly expects no new broadcast because production converges on the
  “already completed semantic intent” at
  [lines 442-463](tests/integration/tests/evm_postgres_submission.rs#L442).
- The separate
  [`production_keystore_fixture_matches_declared_signer_identity`](tests/integration/tests/evm_postgres_submission.rs#L475)
  proves identity wiring only; it does not sign or broadcast.

Consequence:

The test does exercise real-keystore identity qualification and production application assembly.
It does not exercise signing or broadcasting with that qualified signer: production reads and
projects a completion created by the deterministic direct path. It therefore does not close EVM-01
or the required production end-to-end proof.

Required proof:

- Use a new caller token and empty wallet-authority state that no setup phase completes first.
- Admit and drive exclusively through `connect_production_application`.
- Prove the qualified keystore signs the exact candidate, one raw transaction is broadcast, the
  provider observations prove completion, the wallet closes, and a fresh process projects it.
- Add signer identity/account mismatch, stale release, locked/wrong key, and integrity-failure cases
  around the same production path.

### TT2-EVM-02 — Recovery broadcasts retained candidates before chain-state reobservation

Severity: Blocker

Disposition: Open

Violated contract:

The RFC requires a later run to inspect every retained activated candidate in order at
[lines 4117-4124](RFC_RUNTIME_HISTORY_CHOKE_POINT.md#L4117). It may reproduce and resubmit the exact
current retained candidate, but only after observation shows that another submission is still
needed at [lines 3754-3759](RFC_RUNTIME_HISTORY_CHOKE_POINT.md#L3754) and
[lines 4189-4203](RFC_RUNTIME_HISTORY_CHOKE_POINT.md#L4189). Recovery must not depend on repeating an
effect before discovering durable chain truth.

Current implementation:

- A retained ordinal receives
  [`CandidateActivationPermit::Reobservation`](crates/domains/evm/src/wallet_authority.rs#L1194)
  from the recovery walk at [lines 1256-1266](crates/domains/evm/src/wallet_authority.rs#L1256).
- The PostgreSQL authority resolves an existing candidate as ordinary
  `ActivateCandidateResponse::Activated` at
  [authority.rs lines 1790-1814](crates/storages/evm-postgres/src/authority.rs#L1790).
- The `activated` branch immediately executes `BroadcastExactCandidateState`, then begins
  transaction/receipt observation at
  [submission_expansion.rs lines 439-460](crates/domains/evm/src/submission_expansion.rs#L439).

Minimal failure scenario:

1. Run A broadcasts candidate zero and dies before retaining the final receipt/completion in wallet
   authority.
2. The transaction finalizes on chain.
3. Run B starts from retained candidate zero.
4. `Reobservation` is converted to ordinary `Activated`.
5. Run B invokes broadcast before its first chain lookup can discover finality.

Consequence:

An unnecessary broadcast failure or changed physical binding can prevent recovery from discovering
an already-finalized result. More fundamentally, “observe retained work” and “newly activated work
may be submitted” are represented by the same response variant, so the invalid order is easy to
express.

Required resolution properties:

- Retained/reobservation and newly activated candidates are distinct typed states.
- A retained candidate enters transaction/receipt/finality lookup before any resubmission.
- Any permit to resubmit a retained current candidate is affine and derived from exact committed
  observations proving submission is still needed; a newly activated candidate has a distinct
  initial-broadcast permit.
- Recovery completes, conditionally resubmits, or advances replacement solely from committed
  producer observations.

Required proof:

For crashes before/after provider receipt, receipt visibility, finality, and wallet completion, a
different run must observe first. It may resubmit the exact current retained candidate only when the
committed observation authorizes that action, and it must converge to the one canonical result.

### TT2-EVM-03 — Replacement eligibility is self-certified

Severity: Blocker

Disposition: Open

Violated contract:

Replacement must consume producer-bound evidence from the current wallet-status snapshot and the
RFC's committed bounded transaction, receipt, and head reads. The state requesting replacement
cannot certify its own eligibility. The RFC defines this at
[lines 3688-3744](RFC_RUNTIME_HISTORY_CHOKE_POINT.md#L3688), and the remediation plan repeats it at
[lines 803-823](IMPL_PLAN_PROBLEMS_TT1_IMPLRFC_RUNTIME.md#L803).

Current implementation:

- [`derive_exact_candidate_activation_permit`](crates/domains/evm/src/wallet_authority.rs#L1228)
  publicly derives a permit from a reservation, activated-candidate vector available through status,
  candidate ordinal, and numeric observed-prefix length.
- Replacement `eligibility_ref` is a digest over those caller-visible values, a policy reference,
  and the literal string claiming independent observation at
  [lines 1297-1323](crates/domains/evm/src/wallet_authority.rs#L1297).
- It contains no record reference or complete value from a committed transaction, receipt, head,
  or status producer.
- The request carries the next candidate and derived permit. The PostgreSQL authority reloads the
  retained prefix but merely calls the same pure derivation with
  `observed_prefix_len == requested ordinal` at
  [authority.rs lines 2694-2709](crates/storages/evm-postgres/src/authority.rs#L2694).

Failure scenario:

A caller reads the reservation and retained prefix from status, constructs the next candidate, and
derives the same replacement permit without executing the required chain observations. The
authority reloads the prefix but cannot distinguish this request from one produced by the certified
recovery program because the requested ordinal substitutes for the missing observation frontier.

Consequence:

Replacement can be activated while an earlier candidate is successful or without evidence that the
certified policy permits replacing an ambiguous predecessor. Two mutation-equivalent candidates
share a nonce, but wrong replacement timing can still cause unsafe fee escalation, recovery
failure, or a result justified by fictional evidence.

Required resolution properties:

- Eligibility is a private, affine value constructed only from exact committed producer outputs.
- It binds the current reservation, run/occurrence, full retained prefix, predecessor, next ordinal,
  exact policy, freshness/status frontier, and evidence identities.
- The wallet authority independently validates those producer relationships rather than trusting a
  count or self-authored digest.

Required proof:

Hostile direct calls must reject missing, fabricated, stale, cross-run, cross-reservation,
cross-candidate, reordered, or already-consumed evidence. A positive test must show that exact
request replay resolves the original activation without a second mutation or different activation,
while a consumed permit cannot authorize changed input or survive a status/prefix change.

### TT2-EVM-04 — Definite failure can close without final authoritative status

Severity: High

Disposition: Partial

Violated contract:

Every definite run failure must linearize after a fresh `ReadWalletNonceStatus`. A completion or
activation visible at or before that final snapshot must win over the local failure. A later change
does not retroactively change this run's bounded result. The RFC requires this reconciliation at
[lines 4217-4235](RFC_RUNTIME_HISTORY_CHOKE_POINT.md#L4217).

Current implementation:

- Candidate-family exhaustion directly records `ReplacementPolicyExhausted` in
  [`mark_candidate_family_exhausted`](crates/domains/evm/src/submission_process.rs#L644).
- [`select_submission_terminal`](crates/domains/evm/src/submission_process.rs#L1392) turns that local
  progress value into `Exhausted`/`Failed` without a wallet read.
- [`author_submission_terminal`](crates/domains/evm/src/submission_expansion.rs#L656) immediately
  closes the operation from that decision.
- Reservation/candidate leaf failures already have meaningful final-status reconciliation at
  [submission_expansion.rs lines 176-252](crates/domains/evm/src/submission_expansion.rs#L176); the
  uncovered candidate-family exhaustion branch is the remaining defect.

Consequence:

A completion visible after the last candidate observation but at or before the missing final status
snapshot can be hidden by candidate-family exhaustion. The next run may recover the durable truth,
but this run has published the wrong semantic result for the snapshot it was required to take.

Required resolution properties and proof:

- Every definite failure/exhaustion branch must pass through an explicit fresh status state.
- Completed status projects completion; only unchanged incomplete status may authorize the exact
  reviewed exhaustion failure.
- Deterministic races place completion immediately before the final status read and immediately
  after its linearization and assert the contract's respective winner.

### TT2-EVM-05 — Submission identity violates the stable idempotency contract

Severity: High

Disposition: Open

Required contract:

`SubmissionIntentId` is derived from wallet nonce domain, authenticated issuer, and bounded caller
token only at [RFC lines 3626-3633](RFC_RUNTIME_HISTORY_CHOKE_POINT.md#L3626). The transaction and
candidate-family digest is separate; reusing that stable ID with changed semantics is a permanent
integrity conflict at [lines 3646-3659](RFC_RUNTIME_HISTORY_CHOKE_POINT.md#L3646).

The current domain contract preserves the same formula and changed-semantics conflict at
[docs/evm-transactions.md lines 90-106](docs/evm-transactions.md#L90). Because the remediation plan
declares current design documents authoritative, this identity is not an open architecture choice.

The remediation plan's looser statement that intent identity binds behavior-affecting policy at
[lines 198-201](IMPL_PLAN_PROBLEMS_TT1_IMPLRFC_RUNTIME.md#L198), and its associated proof wording,
must be corrected with the implementation; it cannot override the explicit current domain contract.

Current implementation:

- [`derive_submission_intent_id`](crates/domains/evm/src/wallet_authority.rs#L391) additionally
  hashes observation rounds, candidate-family digest, and expansion contract.
- The candidate-family digest itself covers transaction intent at
  [wallet_authority.rs lines 643-681](crates/domains/evm/src/wallet_authority.rs#L643).
- The call site supplies those behavior values at
  [submission_process.rs lines 104-110](crates/domains/evm/src/submission_process.rs#L104).
- The only PostgreSQL uniqueness constraint keyed by submission intent is
  `(wallet_nonce_domain_id, submission_intent_id)` at
  [0001_wallet_authority.sql lines 188-206](crates/storages/evm-postgres/migrations/0001_wallet_authority.sql#L188).

Failure scenario:

The same authenticated caller reuses one token while changing destination, value, calldata,
candidate fees, or observation policy. Because the semantic change also changes the ID, the
authority sees a new reservation rather than a conflict for the stable caller operation.

Consequence:

Durable exactly-once/idempotency semantics differ from the unchanged RFC. Depending on current
incomplete state, the request may receive `Busy` or later allocate and broadcast another nonce
instead of exposing token reuse with changed meaning as integrity failure.

Required resolution properties and proof:

- Restore the stable caller identity formula and persist a separate complete semantic digest with a
  permanent uniqueness/conflict rule.
- Correct the inconsistent remediation-plan wording and delete the behavior-policy identity path;
  do not retain a legacy identifier or compatibility lookup.
- Test same token/same semantics, same token/each changed semantic field, different token/same
  semantics, cross-tenant/principal, and explicit idempotency-epoch rotation.

### TT2-EVM-06 — Promotion cannot be represented by live wallet binding currentness

Severity: High

Disposition: Open

Violated contract:

Every protected wallet access must name and prove the concrete authority's current physical
incarnation. Legitimate promotion must replace the initial target without weakening stale-target
rejection. The RFC distinguishes activation from current release and promotion at
[lines 3221-3252](RFC_RUNTIME_HISTORY_CHOKE_POINT.md#L3221); the remediation plan requires an
authority-owned current-incarnation proof at
[lines 723-729](IMPL_PLAN_PROBLEMS_TT1_IMPLRFC_RUNTIME.md#L723).

Current implementation:

- [`EvmStructuredWalletBindings::new`](crates/live/evm/src/structured_wallet.rs#L52) derives the
  authority target from immutable `initial_store_incarnation_ref` and requires every root and
  successor release to equal it at [lines 58-89](crates/live/evm/src/structured_wallet.rs#L58).
- [`authority_release_is_current`](crates/live/evm/src/structured_wallet.rs#L169) likewise defines
  “current” as the initial activation value.
- Application validation repeats the same equality at
  [application.rs lines 327-366](crates/app/src/application.rs#L327).
- Activation is explicitly permanent and stores an initial incarnation at
  [wallet_authority.rs lines 852-889](crates/domains/evm/src/wallet_authority.rs#L852).
- PostgreSQL tests can promote and open a successor authority using a different incarnation at
  [wallet_authority.rs test lines 2514-2543](crates/storages/evm-postgres/tests/wallet_authority.rs#L2514).

Consequence:

The storage layer can promote, but application/live assembly cannot truthfully represent that
successor: naming the new target fails initial-ref equality, while naming the initial target lies
about the authority actually used.

Required resolution properties and proof:

- The authority exposes or consumes a fresh permit for its current incarnation; initial and current
  are distinct concepts.
- Release histories bind each successor and reject stale predecessors after promotion.
- An end-to-end app test promotes, restarts, reads retained status, performs a protected successor
  operation, and rejects the old target at every access boundary.

### TT2-EVM-07 — Wallet status and mutation cost is lifetime-dependent

Severity: High

Disposition: Open

Violated contract:

Normal status and mutation must use an atomically maintained bounded current projection whose cost
does not grow with completed reservation history. The remediation plan requires that at
[lines 757-794](IMPL_PLAN_PROBLEMS_TT1_IMPLRFC_RUNTIME.md#L757).

Current implementation:

[`load_validated_domain_aggregate`](crates/storages/evm-postgres/src/authority.rs#L174) executes:

- `COUNT(*)` and `MAX(nonce)` across every reservation at
  [lines 191-205](crates/storages/evm-postgres/src/authority.rs#L191); and
- an unbounded `fetch_all` of every incomplete reservation row at
  [lines 272-285](crates/storages/evm-postgres/src/authority.rs#L272).

The database does maintain `local_high_water_nonce` in one bounded domain row, which is useful
progress. Reservation count/maximum validation and incomplete-reservation identity are nevertheless
recomputed from lifetime rows. There is no long-history `EXPLAIN` or query-count proof.

Consequence:

Every status read and mutation becomes more expensive as a sender's lifetime history grows. A core
authority intended to survive indefinitely has unbounded latency and operational risk.

Required resolution properties and proof:

- Retain the existing high-water projection and atomically maintain active reservation identity and
  any validation counters needed by normal-path checks.
- Normal paths read a constant number of bounded rows and do not scan completed reservations.
- Long-history tests and reviewed `EXPLAIN` plans assert index use, rows visited, query count, and
  latency-independent shape at large lineage sizes.

### TT2-EVM-08 — A registered signer path converts integrity into availability

Severity: High

Disposition: Open

Violated contract:

Signer identity/binding/integrity failure is permanent invalid evidence and must never be retried or
settled as ordinary `SignerUnavailable`.

Current implementation:

The structured read attestation path has an `IntegrityFault` variant, but a second
[`BoundedComponentInvoker<EvmCandidateSigner>`](crates/live/evm/src/structured.rs#L909) catches
`LiveInvocationFailure::Integrity` and returns `EvmSubmissionFailure::SignerUnavailable` at
[lines 914-924](crates/live/evm/src/structured.rs#L914). The same registration function retains that
signer component at [lines 989-999](crates/live/evm/src/structured.rs#L989).

Consequence:

If this path is reachable now or becomes reachable through future expansion, an identity violation
can enter retry/failure handling as an operational outage. If it is unreachable, it is a duplicate
wrong implementation path in the most sensitive part of the core.

Required resolution properties and proof:

- Delete the superseded signer invocation path if the read attestation is the sole current design,
  or give every retained path an exhaustive integrity-capable result type.
- Repository-wide reachability/API tests must prove no signer integrity error maps to availability.

### TT2-EVM-09 — The completion value is not the complete public-result closure

Severity: High

Disposition: Partial

Violated contract:

The RFC requires `CompletedWalletNonce` to retain the canonical terminal outcome object closure and
enough public preimage to recover and rehash the result without ambient reconstruction at
[lines 3875-3915](RFC_RUNTIME_HISTORY_CHOKE_POINT.md#L3875) and
[lines 3994-4027](RFC_RUNTIME_HISTORY_CHOKE_POINT.md#L3994).

Current implementation:

- [`CompletedWalletNonce`](crates/domains/evm/src/wallet_authority.rs#L1578) now retains terminal
  witnesses and the activated candidate prefix, which is meaningful progress.
- Its canonical public result is nevertheless a hard-coded JSON string containing only execution
  disposition, created at [lines 1402-1406](crates/domains/evm/src/wallet_authority.rs#L1402).
- The completion retains `CanonicalTerminalOutcome.transaction_intent_digest`, but not the full
  transaction intent or a candidate-family digest/value. The surrounding
  `WalletNonceStatus::Completed` carries the reservation's candidate-family reference and the full
  family at [lines 1681-1695](crates/domains/evm/src/wallet_authority.rs#L1681).
- Application projection selects the result channel from disposition, but on success it wraps the
  entire cloned `CompletedWalletNonce` at
  [submission_process.rs lines 1413-1427](crates/domains/evm/src/submission_process.rs#L1413).

Consequence:

The completion object described as the permanent complete recovery preimage is not independently
complete. Consumers must retain a larger status response or reconstruct meaning outside the
completion contract, splitting ownership and weakening portable verification.

Required resolution properties and proof:

- One content-addressed completion closure contains the full canonical public output plus every
  required transaction, candidate, signer, route/release, observation, and decision preimage.
- The application projects only from that closure.
- Remove or rename narrower digest/summary values so they cannot be mistaken for complete evidence.
- A later-run/offline test starts with the persisted closure alone, rehashes every reference, and
  produces byte-identical public output.

### TT2-EVM-10 — Invalid maximum nonce reaches committed provider observation

Severity: Medium

Disposition: Partial

Current implementation:

`TransactionNonce` correctly rejects `u64::MAX`, and wallet SQL/mutation boundaries prevent it from
being reserved. However:

- [`ObservedPendingNonceFloor::pending_nonce`](crates/domains/evm/src/wallet_authority.rs#L1084)
  remains a raw `u64`;
- capability validation checks domain/reference syntax but not the nonce at
  [submission_registry.rs lines 1001-1008](crates/domains/evm/src/submission_registry.rs#L1001);
- live transport converts any `U256` fitting `u64`, including `u64::MAX`, into `Returned` at
  [structured.rs lines 547-553](crates/live/evm/src/structured.rs#L547); and
- pure qualification copies that value into `QualifiedPendingNonceFloor` without constructing
  `TransactionNonce` at
  [submission_process.rs lines 500-518](crates/domains/evm/src/submission_process.rs#L500).

Consequence:

An invalid protocol nonce can be committed as successful provider history and only fail later at
wallet mutation. Storage poisoning is prevented, but the claimed “reject at every decode/provider
boundary” invariant and backend agreement are false.

Required resolution properties and proof:

- Use the valid nonce type at transport decoding and every observed/qualified floor boundary.
- `u64::MAX` produces a reviewed integrity/capacity outcome before a successful returned
  observation can be committed.
- Boundary tests cover zero, `u64::MAX - 1` (the valid maximum), `u64::MAX`, and values above
  `u64::MAX`.

### TT2-EVM-11 — Pending-floor route and policy are not authority-qualified

Severity: High

Disposition: Open

Violated contract:

The pending observation used to allocate a nonce must be bound to the current activated chain,
sender, qualified route membership, code-owned pending-floor policy, and current physical release.
Syntax-valid references are not authority.

Current implementation:

- Pending request/observation validation checks only nonce-domain structure and reference syntax at
  [submission_registry.rs lines 1001-1008](crates/domains/evm/src/submission_registry.rs#L1001).
- Reserve validation verifies that the reference is shaped correctly and that the domain matches,
  but does not compare the route and policy to authority-owned current values at
  [lines 1202-1223](crates/domains/evm/src/submission_registry.rs#L1202).
- PostgreSQL reserve revalidates the activation but then accepts the capability validator's request
  at [authority.rs lines 1327-1350](crates/storages/evm-postgres/src/authority.rs#L1327).

Consequence:

A syntactically valid observation from the wrong route generation or a substituted policy can
become nonce-allocation evidence. This undermines first-use and double-check semantics even though
the scalar nonce arithmetic is guarded later.

Required resolution properties and proof:

- The owning live/provider authority issues a producer-bound observation for the exact activated
  domain, sender, route membership, release, and code-owned policy.
- The wallet authority independently checks equality to its current activation/currentness proof.
- Hostile tests substitute chain, sender, route generation, membership, release, policy, source
  run, and observation record independently.

### TT2-EVM-12 — The required recovery and authority proof matrices are absent

Severity: High

Disposition: Proof gap

The review record itself carries these residuals before later declaring a complete pass:

- fresh unique-intent keystore E2E at
  [review lines 808-814](TT1_RUNTIME_REMEDIATION_REVIEW.md#L808);
- multi-run crash recovery, provider ambiguity, long-history plans, and promotion at
  [review lines 944-950](TT1_RUNTIME_REMEDIATION_REVIEW.md#L944).

Missing boundary proofs include:

- different-run recovery after every broadcast/receipt/finality/completion crash point;
- observation-first traversal of multiple retained candidates;
- accepted-by-provider followed by timeout, disconnect, invalid JSON, or process loss for every
  supported transport outcome;
- direct hostile replacement requests lacking producer evidence;
- final-status races at every failure/exhaustion branch;
- actual wallet promotion through app/live binding;
- long-lineage query-plan/row-count bounds;
- stable-token semantic-conflict cases; and
- full completion-closure offline recovery.

Required resolution property:

Each matrix must assert exact wallet rows, chain-call counts, run records, semantic outcome, and
absence of unsafe repeated effects. Broad CI or construction-only unit tests cannot substitute for
these production-shaped boundaries.

## Application, export, and replay problems

### TT2-REPLAY-01 — Portable source relationships have no offline proof

Severity: Blocker

Disposition: Open

Violated contract:

The portable format must retain and validate source-run relationships, prior-fact provenance,
semantic and physical fixation, and complete object closure using only artifact bytes plus an
explicit trust snapshot. The current design requires portable source relationships at
[docs/design.md lines 257-265](docs/design.md#L257), and the remediation plan requires their complete
offline proof at [lines 942-976](IMPL_PLAN_PROBLEMS_TT1_IMPLRFC_RUNTIME.md#L942).

Current implementation:

- [`PortableRunExport`](crates/kernel/replay/src/portable.rs#L82) contains root-run batches and a
  vector of `source_run_ids`.
- Export construction records only the root evidence's direct source IDs at
  [lines 116-120](crates/kernel/replay/src/portable.rs#L116). It does not bundle source histories,
  fact-publication proofs, or a recursively authenticated source closure.
- Offline fold constructs `RawRunHistory` from the root batches only and invokes the store fold at
  [lines 207-234](crates/kernel/replay/src/portable.rs#L207).
- After validating the envelope's version, bounds, batch linkage, object count, store fixation, and
  source-ID types, cross-run validation of `source_run_ids` checks only count and strict
  ordering/uniqueness at [lines 255-319](crates/kernel/replay/src/portable.rs#L255). It never derives
  source relationships from folded evidence, compares them to the vector, or verifies source
  histories.
- The app does authorize and load the recursive source graph at
  [production_structured.rs lines 830-890](crates/app/src/production_structured.rs#L830), but returns
  only the root evidence and discards every loaded source value.
- [`ReplayTrustSnapshot`](crates/kernel/replay/src/portable.rs#L323) carries program and physical
  binding verifiers only; it has no authenticated source-publication snapshot.
- `PortableFixation` contains root journal/semantic heads and store scope/epoch but no explicit
  physical target/release fixation at [portable.rs lines 65-77](crates/kernel/replay/src/portable.rs#L65).

Minimal false-proof scenario:

1. Start from a structurally valid root portable artifact.
2. Omit, add, or substitute sorted source IDs.
3. Recompute the unkeyed top-level digest.
4. `verify_offline` folds the unchanged root batches and returns the same verified projection
   because the source vector is unused by the offline fold/projection and is not checked against
   folded evidence. It does affect the artifact digest and resulting `ContentRef`, which the attacker
   recomputes in step 3.

The same verifier also cannot independently re-evaluate prior-run facts that influenced the root.

Consequence:

The artifact's claimed dependency closure is not proved. A third party cannot establish which
source histories affected execution, whether the source facts were valid at the recorded frontier,
or whether the root and sources shared the required target/tenant/purpose fixation.

Required resolution properties:

- Choose the complete offline source-proof model identified in Material uncertainties.
- Source relationships are derived from verified root/source evidence, not trusted as a self-listed
  vector.
- The artifact proves recursive bounds, cycles/shared dependencies, exact fact publication routes,
  producer transitions, source heads, target/tenant/purpose equality, and physical fixation.
- Offline verification performs no live store/provider/callback access.

Required proof:

A golden online/offline projection must match byte-for-byte. Omitted, extra, substituted, reordered,
wrong-tenant, wrong-target, stale-head, false-frontier, false-publication, cyclic, over-budget, and
tampered nested source cases must each fail.

### TT2-REPLAY-02 — Production `Reproduce` is a hidden current-history comparison

Severity: High

Disposition: Open

Violated contract:

The remediation plan deletes `compare_current` completely and retains reproduction only as an
explicitly unavailable mode. No comparison-shaped surface or hidden candidate input may remain at
[lines 907-934](IMPL_PLAN_PROBLEMS_TT1_IMPLRFC_RUNTIME.md#L907).

Current implementation:

- `ReplayRequest::Reproduce` first loads the current live export evidence at
  [production_structured.rs lines 475-480](crates/app/src/production_structured.rs#L475).
- [`validate_replay_export`](crates/app/src/production_structured.rs#L1087) decodes the caller
  artifact, generates a new expected semantic artifact from current live evidence, and requires
  object, bytes, and `ContentRef` equality at
  [lines 1108-1123](crates/app/src/production_structured.rs#L1108).
- It does not call `PortableRunExport::verify_offline`.
- Only after that current-state comparison does it return the frozen reproduction-unavailable
  result.

Failure scenario:

A valid historical semantic artifact is retained for an open run. The run later commits another
semantic transition. Reproduction rejects the earlier artifact because it is not byte-equal to a
newly generated current artifact, even though the earlier prefix remains independently valid for
its own fixation.

Consequence:

The deleted `compare_current` behavior survives under another name, and production does not use the
offline verification path whose correctness the portable contract is supposed to guarantee.

Required resolution properties and proof:

- Delete the hidden candidate/artifact input and current comparison from unavailable reproduction,
  as required by the frozen plan. If retaining a no-input `Reproduce` request adds no product value,
  delete that request variant too.
- Tests prove the unavailable response consumes no artifact and performs no live history comparison;
  offline artifact validation remains exclusively in the verify path.

### TT2-REPLAY-03 — Semantic portable validation accepts a later audit suffix

Severity: High

Disposition: Open

Violated contract:

A semantic export must end at the full atomic batch containing the semantic cutoff. Audit history
after that batch belongs only to an audit export.

Current implementation:

The encoder correctly selects batches with sequence at or before the semantic cutoff at
[portable.rs lines 374-404](crates/kernel/replay/src/portable.rs#L374). The decoder's validator is
weaker: at [lines 286-309](crates/kernel/replay/src/portable.rs#L286), it rejects only a last sequence
below the semantic cutoff. A last sequence above the cutoff passes once the artifact digest and
fixation are recomputed.

Consequence:

The same logical semantic export kind admits multiple physical suffixes. Canonical artifact
identity, historical comparison, and the distinction between semantic and audit export are no
longer exact.

Required resolution properties and proof:

- Decoder and encoder share one selection predicate.
- Semantic fixation requires the last included batch to be exactly the semantic cutoff's atomic
  batch, including adjacent `RunClosed` when applicable.
- Hostile fixtures append one or more authorization/observation audit batches and prove semantic
  decode rejects them while audit decode accepts the exact full prefix.

### TT2-REPLAY-04 — Portable byte limits are internally contradictory

Severity: High

Disposition: Open

Current implementation:

- Portable encode/decode advertises a 512 MiB maximum at
  [`MAX_PORTABLE_EXPORT_BYTES`](crates/kernel/replay/src/portable.rs#L33).
- Both `to_canonical_bytes` and the outer decoder accept that limit at
  [portable.rs lines 145-181](crates/kernel/replay/src/portable.rs#L145).
- The decoder then calls `RecoverabilityContract::strict_decode`, which rejects any canonical value
  above the annex-wide maximum at
  [recoverability.rs lines 340-350](crates/kernel/canonical/src/recoverability.rs#L340).
- The annex owner fixes that maximum at 16,777,216 bytes in
  [generate.py](contracts/recoverability/generate.py#L717).
- Application replay aliases the same 512 MiB portable constant at
  [production_structured.rs line 62](crates/app/src/production_structured.rs#L62) and applies it to
  buffering at [lines 1094-1101](crates/app/src/production_structured.rs#L1094).

Consequence:

The encoder can emit an artifact larger than 16 MiB through 512 MiB that the sole decoder rejects.
Public limits, memory allocation, the replay unit limit fixture, and generated schema authority
disagree.

Required resolution properties and proof:

- One annex-owned byte/depth/count budget is consumed by encoder, decoder, app spool, CLI/REST, and
  tests.
- Encoding a value guarantees that the same current decoder can accept it.
- Exact-limit and one-byte-over fixtures cover both a single large batch and many small batches,
  with bounded allocation before full deserialization.

### TT2-REPLAY-05 — Replay schema authority and corpus remain incomplete

Severity: High

Disposition: Partial

Current implementation:

- [`StructuredCanonicalProjection::strict_decode`](crates/kernel/replay/src/structured.rs#L51)
  accepts any caller-provided schema name admitted by `SchemaId::new` and validates only generic
  canonical JSON.
- Projection IDs are synthesized from a hard-coded string formula at
  [structured.rs lines 512-519](crates/kernel/replay/src/structured.rs#L512), not obtained from an
  annex-owned replay-result schema.
- The recoverability annex defines the portable envelope but no structured replay-result contract
  or typed projection shapes.
- The generated corpus has no portable positive/negative vector matrix required by the plan.
- Documentation is already stale: the portable schema ID in
  [contracts/recoverability/v1/README.md](contracts/recoverability/v1/README.md#L27) has digest prefix
  `6438e621...`, while the current generated
  [annex.json](contracts/recoverability/v1/annex.json) value has prefix `cba79da4...`.

Consequence:

Public replay values can claim schema identities with no authoritative shape, drift checks do not
cover the portable semantics they advertise, and documentation cannot be trusted as the generated
contract summary.

Required resolution properties and proof:

- Every retained public replay/export DTO has one annex-owned generated schema identity and exact
  decoder.
- Remove arbitrary-name schema construction from public paths.
- Generate portable and replay positive/negative/tamper vectors from the same owner and make README
  metadata generated or checked byte-for-byte.

### TT2-APP-01 — Purpose evidence types do not enforce purpose-limited data

Severity: High

Disposition: Open

Violated contract:

The remediation plan says each target-bound reader exposes only its exact projection and that one
purpose cannot be used to obtain another purpose's data at
[lines 94-97](IMPL_PLAN_PROBLEMS_TT1_IMPLRFC_RUNTIME.md#L94) and
[lines 286-289](IMPL_PLAN_PROBLEMS_TT1_IMPLRFC_RUNTIME.md#L286). Its same-purpose pairing and export
authorization rules appear at [lines 205-212](IMPL_PLAN_PROBLEMS_TT1_IMPLRFC_RUNTIME.md#L205) and
[lines 868-890](IMPL_PLAN_PROBLEMS_TT1_IMPLRFC_RUNTIME.md#L868).

Current implementation:

The wrappers are nominally distinct and cannot be converted directly, which is progress. Their
data accessors remain broad:

- public evidence can resolve arbitrary retained history objects at
  [purpose.rs lines 155-163](crates/kernel/store/src/structured/purpose.rs#L155);
- trace evidence exposes every physical record at
  [lines 190-198](crates/kernel/store/src/structured/purpose.rs#L190);
- audit evidence exposes every physical record at
  [lines 225-232](crates/kernel/store/src/structured/purpose.rs#L225); and
- replay evidence exposes every record and committed batch at
  [lines 275-283](crates/kernel/store/src/structured/purpose.rs#L275).

Consequence:

The types label a purpose but often carry the complete verified run and expose enough raw data for
the holder to construct another projection itself. The frozen least-purpose data boundary is not
enforced by nominal method separation alone.

Required resolution properties and proof:

- Resolve the purpose-data Material uncertainty.
- Define and construct minimum data products per purpose rather than wrapping one all-powerful
  verified run where that leaks another projection.
- Cross-purpose tests must cover both type conversion and information access: a public/trace/status
  holder must be unable to enumerate audit/export-only data even by manual projection.

### TT2-APP-02 — Recursive source authorization is not retained as export evidence

Severity: High

Disposition: Partial

Current implementation:

Application export now recursively invokes the dependency authorizer and loads every source under
the export reader at
[production_structured.rs lines 830-878](crates/app/src/production_structured.rs#L830). This closes
the original direct-disclosure failure in part.

After validating the graph, however, it returns only the root at
[lines 880-890](crates/app/src/production_structured.rs#L880). It discards:

- each source's verified evidence;
- the exact grant/decision that authorized that source;
- source head and physical target/release fixation; and
- the verified recursive relationship needed by portable construction.

Consequence:

Authorization happens transiently but does not become a property of the emitted bytes. Portable
construction sees only direct IDs from the root and cannot prove that the app actually authorized
or verified the same recursive closure it claims.

Required resolution properties and proof:

- Export construction consumes an opaque, bounded authorized source-closure value rather than root
  evidence plus recomputed IDs.
- That value binds target, tenant, principal, purpose, heads, physical fixation, and recursive
  relationships through serialization or an independently verifiable proof.
- Denied, missing, wrong-target, stale, cyclic, shared, and over-budget dependencies emit zero bytes
  and one redacted error.

## Security, simplicity, verification, and delivery problems

### TT2-SEC-01 — The same-allocation key handoff is not represented

Severity: High

Disposition: Open

Required contract:

Decrypted private-key ownership must move between protected owners without leaving an unzeroized
by-value stack copy. The remediation plan specifically requires a same-allocation transfer and a
witnessed cleanup proof at
[lines 633-647](IMPL_PLAN_PROBLEMS_TT1_IMPLRFC_RUNTIME.md#L633).

Current implementation:

- `ProtectedKeyMaterial` stores an inline `Zeroizing<[u8; 32]>` at
  [secure_key.rs lines 20-29](crates/keystore/src/keystore/secure_key.rs#L20).
- Decryption copies from a heap `Vec` into that fixed array at
  [lines 31-51](crates/keystore/src/keystore/secure_key.rs#L31).
- `into_secure_key` uses `mem::replace` and moves the inline wrapper by value again at
  [lines 65-76](crates/keystore/src/keystore/secure_key.rs#L65).
- `SecureKey` stores another inline `Zeroizing<[u8; 32]>` at
  [lines 90-108](crates/keystore/src/keystore/secure_key.rs#L90).
- Tests record logical transfer/cleanup flags at
  [lines 252-281](crates/keystore/src/keystore/secure_key.rs#L252); they do not observe stable
  address identity or stale moved-from stack storage.

Rust move semantics do not guarantee a stable allocation for inline arrays or guarantee erasure of
every compiler/ABI temporary. This finding does not assert an observed leak; it states that the
promised invariant is false as documented and unproved by representation.

Required resolution properties and proof:

- Use heap-stable protected ownership or another representation whose move does not copy secret
  bytes between stack locations.
- Zeroize the decrypt source and final protected allocation on every success/error/panic path.
- Tests witness allocation/address continuity where meaningful, source/final cleanup, wrong-length
  cleanup, and production qualification/signing call graphs. Redaction tests remain mandatory.

### TT2-QUALITY-01 — Simplification reduced old code but concentrated new authority

Severity: High

Disposition: Open

Observed structure at the reviewed tree:

- `crates/kernel/certify/src/structured.rs`: 11,474 lines;
- `crates/kernel/store/src/structured/fold.rs`: 4,649 lines;
- `crates/kernel/spec/src/structured.rs`: 3,780 lines;
- `crates/kernel/program/src/structured.rs`: 2,948 lines;
- `crates/storages/evm-postgres/src/authority.rs`: 3,065 lines.

Certification still owns the store-mint token at
[structured.rs lines 133-167](crates/kernel/certify/src/structured.rs#L133), physical-binding
selection and exposure at [lines 1050-1206](crates/kernel/certify/src/structured.rs#L1050), process
selection and preparation at [lines 4025-4227](crates/kernel/certify/src/structured.rs#L4025), and
physical invocation at [lines 4230-4250](crates/kernel/certify/src/structured.rs#L4230). The plan
assigns live process/access authority to Runtime at
[lines 77-94](IMPL_PLAN_PROBLEMS_TT1_IMPLRFC_RUNTIME.md#L77). The public bypass in TT2-AUTH-01 is a
direct consequence of that ownership mismatch.

The remediation delta from `31aced23a` to `a4dada89` adds 14,105 and deletes 9,804 Rust/SQL lines: a
net increase of 4,301 lines. That is not automatically wrong, but it does not support the claim that
the remediation completed the LOC/simplicity objective. Several comments also encode plan-commit
history rather than enduring invariants, for example
[transaction.rs lines 232-234](crates/storages/postgres/src/transaction.rs#L232).

Consequence:

Core authority remains difficult to audit, change, and test in isolation. Responsibilities that
should be enforced by module ownership are exposed as public conventions, while duplicated or
misplaced paths survive.

Required resolution properties:

- Move deterministic certification, Runtime live authority, store fold/persistence, replay, and
  domain authority into their documented owners.
- Delete duplicate paths and public types instead of wrapping them in more capability vocabulary.
- Split modules only along real ownership boundaries; avoid thin indirection that preserves the
  same coupled change site.
- The final change should reduce public authority surface, future change sites, and production LOC
  while retaining validation, readable code, and boundary tests.

### TT2-VERIFY-01 — The acceptance proof matrix remains materially incomplete

Severity: High

Disposition: Proof gap

Required missing proofs across this ledger include:

- external crate cannot forge or invoke Runtime physical access;
- retained PostgreSQL issuance inputs cannot mutate raw storage;
- copied/restored target and coordinated rollback are rejected by an external fence;
- same-run cross-process exact-head contenders return exact semantic outcomes;
- acknowledgement-loss, serialization, deadlock, and object/head fault injection;
- snapshot-consistent run/configuration reads under deterministic interleavings;
- fresh-load indexed-head removal/rewind/divergence;
- exact memory/PostgreSQL shared acceptance corpus;
- fresh unique-intent real-keystore production submission;
- different-run observation-first recovery and complete ambiguity matrix;
- producer-bound replacement hostile matrix and final-status races;
- promoted wallet authority through application/live binding;
- long-history wallet and fact-scan query/verification bounds;
- recursive offline source proof and online/offline golden projection;
- portable tamper, semantic/audit suffix, bounds, and generated corpus matrix;
- purpose data isolation, not only wrapper conversion; and
- stable-allocation key-lifetime evidence.

Required resolution property:

Every Blocker/High fix must have a focused regression that fails on `a4dada89` and asserts the exact
authority denial, durable prefix, external callback count, semantic result, or bounded resource
cost. Broad CI is the closing integration gate, not the evidence for an unexercised invariant.

### TT2-PROCESS-01 — The final pass contradicts its own durable review record

Severity: High

Disposition: Partial

This severity concerns delivery confidence rather than a separate runtime semantic transition.

Current evidence:

- The review record explicitly admits missing PostgreSQL cross-process/fault/acknowledgement proofs
  at [lines 615-629](TT1_RUNTIME_REMEDIATION_REVIEW.md#L615).
- It admits the fresh keystore path is not proved at
  [lines 808-814](TT1_RUNTIME_REMEDIATION_REVIEW.md#L808).
- It carries EVM crash, replacement-evidence, fault, scale, and promotion residuals at
  [lines 944-950](TT1_RUNTIME_REMEDIATION_REVIEW.md#L944).
- It carries offline golden/tamper/corpus and live multi-hop export residuals at
  [lines 1138-1142](TT1_RUNTIME_REMEDIATION_REVIEW.md#L1138).
- The final section then claims the production qualification closes the fresh-keystore E2E at
  [lines 1310-1311](TT1_RUNTIME_REMEDIATION_REVIEW.md#L1310), even though the test reused a
  deterministic-path completion as shown in TT2-EVM-01.
- The final exact-tree reviewer checked revision ancestry, documentation-only scope, and recorded
  command evidence at [lines 1335-1349](TT1_RUNTIME_REMEDIATION_REVIEW.md#L1335); that is not a
  substantive re-audit of the claims.
- `ca01d81a` and `67f113ed` change only this review document, not production or regression code.
- `git diff --check 31aced23a..HEAD` fails on trailing whitespace in
  [docs/code-quality.md lines 36-37](docs/code-quality.md#L36). The reported plain
  `git diff --check` on a clean worktree checks no committed range and was therefore vacuous.

Consequence:

Durable checkpoint evidence records unresolved acceptance work but the final gate reclassifies it
as non-blocking without fixing it or changing the contract. This makes revision pinning precise but
does not make the semantic conclusion reliable.

Required resolution properties:

- A new skeptical review evaluates the exact production tree against every TT2 item and required
  proof, not only the existence of evidence files.
- Every residual is classified as resolved, deliberately removed from the normative contract, or a
  material uncertainty/blocker. Proof gaps required by acceptance cannot be relabeled optional.
- Range-based hygiene and repository searches examine committed changes, while worktree-only checks
  are reported accurately.

## Disposition of every TT1 problem

This matrix preserves traceability to the first ledger. “Closed” means the original defect's
substance and focused proof were found; a new TT2 defect in the same area is listed separately
rather than silently rewriting the old ID.

| TT1 ID | Disposition | Current evidence or remaining gap |
| --- | --- | --- |
| AUTH-01 | Partial | Direct public history writer/split surfaces were narrowed, but TT2-AUTH-01 leaves the live effect bracket forgeable outside Runtime. |
| AUTH-02 | Closed | Production store verification uses the concrete certified-root registry; the old caller-substitutable program verifier path is no longer available to app assembly. |
| AUTH-03 | Open | Opaque sessions exist, but cloneable public raw login URLs retain DML authority; see TT2-AUTH-02. |
| AUTH-04 | Open | Schema roles are more specific, but same-schema copied targets self-qualify and no external fence exists; see TT2-STORE-01. |
| AUTH-05 | Partial | Purpose-specific reader/evidence types exist, but their data projections remain broadly permissive; see TT2-APP-01. |
| LANG-01 | Closed | Fallibility is represented through closed nominal outcomes rather than optional convention. |
| LANG-02 | Closed | Typed child failure can cross structured fragment boundaries under the current child contract. |
| LANG-03 | Closed | Recursive structured aggregate values now compose through retained constructs. |
| LANG-04 | Closed | Certification rejects duplicate arm labels and related hostile structural input. |
| LANG-05 | Partial | Ordinal-first behavior is implemented, but the dedicated label-rename/ordinal invariance proof remains thin; see TT2-VERIFY-01. |
| PROV-01 | Partial | Lexical/provenance structure is stronger, but completion/source projection does not preserve the full proof downstream; see TT2-APP-02 and TT2-REPLAY-01. |
| PROV-02 | Closed | Normalized provenance traversal is bounded/iterative under the current certified representation. |
| STORE-01 | Partial | Canonical writer locking and domain outcomes exist, but the post-error classifier uses an aborted transaction and cross-process/ack proofs are absent. |
| STORE-02 | Closed | The original always-successful production-compilable test-fence surface is removed. The replacement generic fence is independently defective under TT2-STORE-01. |
| STORE-03 | Open | Deterministic cross-process, copied-target, snapshot, acknowledgement, and fault-injection conformance remains absent. |
| STORE-04 | Partial | Some append bounds are shared, but envelope/configuration acceptance and read snapshots still differ between memory and PostgreSQL. |
| STORE-05 | Partial | A SQL inventory and offline gate exist, but the inventory misses generic and builder queries. |
| STORE-06 | Partial | Dense publication routing exists, but each publication can reload and refold a growing producer prefix. |
| STORE-07 | Closed | Run/configuration transaction acquisition and DML typestate are substantially centralized in one PostgreSQL transaction module. |
| EVM-01 | Open | Production signer wiring exists, but the required fresh unique-intent keystore sign/broadcast/complete proof was not exercised. |
| EVM-02 | Closed | Admission requalifies configured EVM semantics and rejects the tested cross-chain/route substitution before wallet mutation. |
| EVM-03 | Open | Retained candidates are iterated, but reobservation enters the broadcast branch before chain-state reobservation. |
| EVM-04 | Open | Replacement is ordered, but authority eligibility contains no independent producer evidence and can be self-derived. |
| EVM-05 | Partial | Transport mapping preserves `EntryUnknown` for important post-entry failures, but the full transport/crash ambiguity matrix is absent; see TT2-EVM-12. |
| EVM-06 | Partial | `TransactionNonce` and wallet SQL reject `u64::MAX`; provider/observation history still admits it as successful returned data. |
| EVM-07 | Open | The structured attestation path preserves integrity, but a retained registered signer path still maps integrity to `SignerUnavailable`. |
| EVM-08 | Open | Behavior-affecting values are frozen, but putting them into `SubmissionIntentId` conflicts with the RFC's stable caller key and removes changed-semantics conflict. |
| EVM-09 | Partial | Completion retains terminal witnesses and activated prefix, but is not itself the complete transaction/family/public-output object closure. |
| EVM-10 | Open | Status/mutation still scans lifetime reservations and lacks a bounded projection/plan proof. |
| EVM-11 | Open | Per-access release checks exist, but wallet currentness is equated to immutable initial incarnation and cannot represent promotion. |
| APP-01 | Partial | Recursive authorization is attempted before serialization, but source evidence/fixation is discarded and portable output cannot prove it. |
| APP-02 | Closed | The explicit `compare_current` request/value surface was deleted. TT2-REPLAY-02 records a new hidden current comparison in `Reproduce`. |
| APP-03 | Closed | Store admission derives the authoritative run ID from the frozen preimage instead of accepting a caller-selected ID. |
| REPLAY-01 | Open | Root batches can fold offline, but source relationships/facts, physical fixation, limits, and the hostile corpus are incomplete. |
| REPLAY-02 | Closed | `mfm-replay` now owns the sole portable envelope/encoder/decoder surface. |
| REPLAY-03 | Open | Portable envelope has an annex schema, but replay-result projection is synthesized and generated corpus/docs drift remains. |
| SEC-01 | Open | Plaintext-array public construction was removed, but inline moves do not establish the promised same protected allocation. |
| QUALITY-01 | Open | Old executor LOC was removed, but authority remains misplaced and concentrated in very large core modules; remediation Rust/SQL grew net LOC. |
| VERIFY-01 | Open | Required authority, cross-process, crash, promotion, scale, offline, and security proof matrices remain absent. |
| PROCESS-01 | Partial | Checkpoint evidence is now durable and revision-pinned, but its final pass contradicts acknowledged residuals and did not substantively re-audit them. |

Count check:

| Disposition | Count |
| --- | ---: |
| Closed | 12 |
| Partial | 13 |
| Open | 15 |
| Total | 40 |

## Why the previous eight acceptance thresholds are not met

| TT1 threshold | Disposition at reviewed tree |
| --- | --- |
| 1. Every Blocker/High resolved or contract deliberately removed | Not met: multiple Blocker/High implementation defects and proof gaps remain, without matching normative deletion. |
| 2. Invalid writer/verifier/settlement/provenance/replacement/binding states excluded | Not met: live authorization is forgeable and replacement eligibility self-certifies. |
| 3. Fresh production EVM submission through real RPC, keystore, wallet, broadcast, and completion | Not met: production reused a completion produced by the deterministic direct worker. |
| 4. A later run converges on all retained wallet states without unsafe reinvocation | Not met: retained candidate reobservation enters broadcast before chain-state reobservation, and exhaustion lacks mandatory final reconciliation. |
| 5. Portable exports verify offline through the sole callback-free verifier | Not met: the root folds, but source relationships/facts are unused by that fold and not compared with folded evidence; production `Reproduce` compares live state. |
| 6. Memory/PostgreSQL exact conformance plus cross-process/fence/fault proofs | Not met: canonical acceptance and snapshot behavior differ; copied-target, contention, acknowledgement, and fault proofs are missing. |
| 7. One writer/certified authority/transaction owner with no placeholder path | Not met: Runtime live invocation authority remains in certification and is publicly splittable/forgeable; duplicate signer and incomplete authority paths remain. |
| 8. Honest durable final review with uncertainties and production claims distinguished | Not met: admitted acceptance residuals were reclassified as a complete pass, and the final reviewer checked evidence pinning rather than semantic truth. |

## Foundations worth preserving

The next architecture and implementation should preserve these genuine improvements unless a
specific TT2 resolution requires changing their representation:

- the superseded graph/executor crates and compatibility path remain deleted;
- the structured language uses declaration-ordered `State`, `Match`, and bounded `FanOut`, with
  result/failure represented as outcomes rather than instructions;
- fallibility, child failure, recursive aggregate values, and several hostile certification cases
  are materially stronger;
- the run journal retains exactly the five append-only record families;
- adjacent root-atomic `RunClosed` and callback-free fold validation remain strong;
- memory append is atomic under one mutex;
- PostgreSQL batch, object, fact-route, head, and adjacent closure writes are in one transaction;
- the old public semantic history writer and caller-supplied run ID are removed from application
  assembly;
- a one-entry verified-successor cache limits retained runtime memory and checks an indexed head
  before cache reuse;
- PostgreSQL role separation and shared transaction typestates are substantial, even though the
  external fence and raw-credential boundary are incomplete;
- EVM configuration requalification, complete signer identity construction, entry ambiguity,
  `TransactionNonce`, contiguous candidate-prefix validation, and terminal witness validation are
  valuable foundations;
- application export discovers and authorizes recursive sources before emitting bytes, even though
  it does not yet carry that proof into the artifact;
- `mfm-replay` owns the one portable format and has a callback-free offline root-fold entry;
- explicit `compare_current` APIs and caller-selected comparison values remain deleted;
- admission derives run identity from the authoritative preimage contract; and
- secret/public-error redaction remains generally careful.

Preserving a foundation does not preserve its current public API when that API causes a TT2
authority defect. Breaking changes and deletion remain preferable to compatibility paths.

## Required architect decisions before a fix plan

1. What single crate owns preparation, append-result minting, and invocation of a physical access,
   and what is the minimum private port between Runtime and store?
2. What exactly is inside the deployment PostgreSQL TCB, and how does an external non-rollback
   authority prove currentness without exporting a reusable writer credential?
3. What is the one snapshot/fixation contract shared by cached loads, fresh loads, configuration,
   replay, audit, and export?
4. Which typed EVM states distinguish newly activated broadcast authority, the mandatory
   observation-first retained path, and evidence-conditioned resubmission?
5. What complete producer evidence authorizes replacement, and which authority validates and
   consumes it?
6. What complete cutover restores the stable caller-token `SubmissionIntentId`, moves behavior
   semantics into the separate conflict digest, and deletes the superseded identity path?
7. Which value names the wallet authority's current promoted incarnation, and how is it refreshed
   per protected access?
8. Does portable verification bundle recursive histories or consume an authenticated fact/source
   publication proof, and what exact trust snapshot is sufficient?
9. What data, not merely wrapper type, belongs to each public/trace/audit/replay/export purpose?
10. Which annex-owned bounds govern batch frames, complete portable documents, configuration,
    provider proofs, and replay projections?

A dedicated architect agent is required now, before fix planning. It must choose one target design,
name every responsibility owner, define the complete cutover and deletion scope, identify affected
contracts and tests, and provide an ordered logical commit sequence. Its design must minimize
concepts, code paths, public types, duplicated responsibilities, future change sites, and LOC;
breaking changes are allowed, and superseded paths must be deleted without compatibility fallbacks.
Each decision above must state the invalid states it excludes and the exact hostile proof that will
establish it.

## Acceptance threshold for the TT2 remediation

The next implementation must not be declared complete from a model hash, broad CI, checkpoint prose,
or aggregate LOC alone. Acceptance requires all of the following:

1. Every TT2 item is resolved in production code or deliberately removed from the normative
   RFC/design/plan together with every affected API, schema, test, and document. Blocker and High
   findings prevent any core acceptance; Medium findings remain part of the same cutover.
2. An external crate cannot split production process authority, mint committed authorization,
   invoke a physical binding, append semantic history, or reuse/cross-wire an affine permit.
3. Generic PostgreSQL history/configuration sessions consume a fresh external non-rollback proof;
   copied, restored, stale, wrong-database, and coordinated-rollback targets fail closed.
4. Fresh and cached history/configuration reads are snapshot-consistent, verify indexed heads, and
   return exact domain outcomes under deterministic contention and acknowledgement faults.
5. Memory and PostgreSQL accept the same bounded canonical run/configuration corpus.
6. A fresh unique production submission signs with the real qualified keystore, broadcasts once,
   observes, completes, restarts, and projects solely through the product application path.
7. A different run observes every retained candidate before any new broadcast/replacement;
   replacement consumes independent producer evidence; every definite failure reconciles status.
8. Stable intent identity, semantic-conflict behavior, completion closure, pending-floor authority,
   promotion currentness, and bounded status cost match one explicit normative contract.
9. A third party verifies a portable root and every source/fact relationship offline using only the
   artifact and explicit trust snapshot; semantic/audit fixation and limits are exact.
10. Purpose capabilities restrict both operations and obtainable data to their exact projection.
11. Key ownership has a representation-backed zeroization invariant rather than an inline-move
    claim that tests cannot observe.
12. Each fix has a boundary regression that fails on `a4dada89`; the complete focused proof matrix
    and scope-selected final gates pass on the exact reviewed revision.
13. Runtime, certification, store, EVM, replay, and app responsibilities have one documented owner;
    duplicate paths/public authority are deleted, core change sites and LOC are reduced, and no
    compatibility fallback is added.
14. A new independent skeptical review audits behavior and source at the exact code revision,
    records all residuals/material uncertainties honestly, and reaches `PASS` only after the
    acceptance evidence itself is complete.
