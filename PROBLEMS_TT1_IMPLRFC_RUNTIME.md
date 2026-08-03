# Problems Found in the TT1 Runtime-History RFC Implementation

## Status and scope

This document is the handoff problem ledger for the first implementation attempt reviewed at:

- branch: refact-runtime
- reviewed head: 31aced23a, harden structured runtime and wallet authority qualification
- implementation commit: 33e301c43, replace graph and executor with structured runtime history
- pre-implementation parent: b76d77a09
- normative design: [RFC_RUNTIME_HISTORY_CHOKE_POINT.md](RFC_RUNTIME_HISTORY_CHOKE_POINT.md)
- implementation scope: [IMPL_PLAN_RFC_RUNTIME_HISTORY_CHOKE_POINT.md](IMPL_PLAN_RFC_RUNTIME_HISTORY_CHOKE_POINT.md)

The implementation was reviewed for the three attributes expected from this platform core:

- correctness: invalid authority, provenance, recovery, and settlement states must be rejected or
  made unrepresentable;
- dexterity: operations must remain easy to compose, expand, wrap, observe, and extend without
  bypassing guarantees;
- simplicity: each responsibility must have one owner and one current path, with the minimum
  concepts, public authority surfaces, duplicated logic, future change sites, and LOC.

The verdict is that the implementation is substantial but is not a safe or complete RFC end state.
The problems below are the minimum known problem set. Passing the current tests or broad CI does not
resolve them because several failures sit exactly at boundaries that the test suite does not
exercise.

This is intentionally a problem statement, not a fix plan. “Required resolution properties” state
the guarantees that a solution must establish without choosing its mechanism in advance.

## Review evidence

The engineer reported:

- nix run .#model-check passed with model hash
  cea674342a54b4310a3ec46238474288655aff6a2ac4486b56b4ae01379cea59;
- nix run .#ci passed all 12 tasks in run 3282277-1785750529539613768;
- git diff --check, repository-integrity checks, and focused gates passed;
- the worktree was clean at the reported revision;
- the RFC and implementation plan were unchanged by the implementation.

The review treated that broad CI evidence as credible rather than rerunning the same expensive gate.
It then audited source and tests independently across:

- authored, expanded, and certified program semantics;
- Runtime access typestate, ambiguity handling, and callback-free replay;
- memory and PostgreSQL history storage, roles, fencing, and exact-head behavior;
- EVM transport, signing, wallet authority, nonce allocation, recovery, and completion;
- application admission, authorization, export, replay, CLI/REST, and keystore boundaries.

Focused suites passed in the audited areas. The findings below are therefore predominantly missing
contracts, missing boundary tests, and behaviors that existing tests incorrectly accept or avoid,
not ordinary test failures.

## Severity

- Blocker: invalidates a central RFC guarantee, permits an authority/provenance bypass, breaks a
  required production path, or can produce a wrong irreversible semantic result.
- High: violates a required contract or leaves a serious correctness, security, operability, or
  dexterity gap.
- Medium: important quality, security-hardening, conformance, or maintainability problem that should
  be resolved in the same cutover.
- Low: latent correctness or documentation debt that is not currently known to affect a shipped
  path.

## Executive problem index

| ID | Severity | Area | Problem |
| --- | --- | --- | --- |
| AUTH-01 | Blocker | Runtime/store | Runtime is not the sole run-history writer |
| AUTH-02 | Blocker | Certification/store | Certified-program verification can be forged |
| AUTH-03 | Blocker | PostgreSQL | A retained raw pool bypasses all validated mutation |
| AUTH-04 | Blocker | PostgreSQL | Writer and maintenance roles are not store-target scoped |
| LANG-01 | Blocker | State fallibility | SafeFailure disposition is neither exhaustive nor enforced |
| LANG-02 | Blocker | Child composition | Typed child failure cannot cross its fragment boundary |
| PROV-01 | Blocker | Match/FanOut | Retained provenance is flattened away |
| PROV-02 | High | Hostile input | Normalized provenance can exhaust the process stack |
| STORE-01 | High | Exact-head append | Contention can produce generic availability failure |
| EVM-01 | Blocker | Keystore/signing | A fresh production submission cannot use the qualified signer |
| EVM-02 | Blocker | Configuration | Persisted submission semantics are not requalified |
| EVM-03 | Blocker | Recovery | A later run does not observe retained activated candidates |
| EVM-04 | Blocker | Replacement | Replacement eligibility self-certifies |
| EVM-05 | Blocker | Broadcast | Possible target entry is classified as definite rejection |
| EVM-06 | Blocker | Nonce allocation | The allocator can permanently reserve u64::MAX |
| APP-01 | Blocker | Authorization | Export does not authorize prior-run source dependencies |
| REPLAY-01 | Blocker | Portable evidence | Exported history is not independently verifiable |
| AUTH-05 | High | Read authority | Purpose-limited read capabilities were removed |
| EVM-07 | High | Signing faults | Signer integrity failures become ordinary unavailability |
| EVM-08 | High | Durable identity | Persisted identity does not freeze all execution behavior |
| EVM-09 | High | Completion | Completion does not retain the full public-result closure |
| EVM-10 | High | Wallet storage | Status and mutation cost grows with the complete lifetime |
| EVM-11 | High | Physical rotation | Release evidence is not bound to the authority’s current target |
| APP-02 | High | Replay | compare_current is permanently stubbed |
| APP-03 | Medium | Run identity | Store admission accepts an independently supplied run ID |
| LANG-03 | Medium | DSL dexterity | Structured aggregate values do not compose through all constructs |
| LANG-04 | Medium | Certification | Hostile input can retain duplicate arm labels |
| LANG-05 | Low | Scheduling | Structural ordering does not follow declared ordinals |
| STORE-02 | High | Writer fence | An always-successful test fence is production-compilable |
| STORE-03 | High | Conformance | Required cross-process and fault-injection proofs are absent |
| STORE-04 | Medium | Backend parity | Memory and PostgreSQL accept different histories |
| STORE-05 | Medium | SQL assurance | The SQLx metadata gate does not cover dynamic queries |
| STORE-06 | Medium | Fact performance | Fact-frontier validation repeatedly scans retained history |
| STORE-07 | Medium | Persistence ownership | Transaction authority setup is duplicated |
| SEC-01 | Medium | Keystore | Decrypted key bytes cross an ordinary stack copy |
| REPLAY-02 | Medium | Ownership | Portable evidence has the wrong implementation owner |
| REPLAY-03 | Medium | Public contracts | Replay-result identity is not annex-backed |
| QUALITY-01 | High | Simplicity | Core responsibility is concentrated into oversized modules |
| VERIFY-01 | High | Verification | Important conformance and hostile-boundary proofs are absent |
| PROCESS-01 | High | Delivery | The claimed checkpoint process is not represented in the branch |

## Authority and choke-point failures

### AUTH-01 — Runtime is not the sole run-history writer

Severity: Blocker

Violated guarantee:

The RFC requires Runtime to be the only component able to append semantic run history. The store
must validate Runtime proposals but must not expose an independently usable semantic writer.

Current behavior:

- [StructuredRunStore::split](crates/kernel/store/src/structured/backend.rs#L237) is public and
  returns a StructuredRunHistoryWriter.
- The writer’s mutation entry points are public in
  [mutation.rs](crates/kernel/store/src/structured/mutation.rs#L432).
- Proposal constructors for admissions, transitions, facts, access authorizations, and observations
  are public.
- PostgreSQL qualification returns the raw structured store, so this is reachable from production
  assembly rather than being an internal test fixture.

Failure scenario:

Any library consumer holding the store can load a verified cursor, construct a schema-valid
proposal, and append a state result, access observation, or fact without Runtime dispatch, the
qualified state callback, the adapter, or affine invocation authority. The callback-free fold
accepts the forged domain truth because its structural schema remains valid.

Consequence:

The central “history choke point” is a convention rather than an authority boundary. This also
undermines reasoning about effect attempts, provenance, recovery, and audit results because those
records no longer prove that Runtime performed the required sequence.

Required resolution properties:

- Production consumers outside Runtime cannot acquire semantic append authority.
- Store persistence remains independently testable without publishing production-forgeable
  semantic proposal authority.
- Compile-fail or visibility tests prove that application, adapter, and unrelated crate code cannot
  append a transition, authorization, observation, fact, or closure directly.
- Tests prove that every committed semantic record is causally linked to the required Runtime
  authority transition.

### AUTH-02 — Certified-program verification can be forged

Severity: Blocker

Violated guarantee:

Persisted execution and replay must use the one expansion certified by the admitted root. A verifier
must not be able to substitute a different expanded program or schema set.

Current behavior:

- [VerifiedProgramData::new](crates/kernel/store/src/structured/fold.rs#L103) is public.
- [StructuredProgramVerifier](crates/kernel/store/src/structured/fold.rs#L137) is a public,
  caller-implementable verification boundary.
- [StructuredRunStore::new](crates/kernel/store/src/structured/backend.rs#L221) accepts that
  verifier.
- The fold checks the returned document/root and operation ID at
  [fold.rs](crates/kernel/store/src/structured/fold.rs#L518), but does not prove that the returned
  expansion is the root-certified expansion.

Failure scenario:

A custom verifier admits a legitimate certified document but returns an unrelated expanded program
and schemas with the same operation ID. The store then evaluates history against the substituted
program.

Consequence:

Certified provenance is not actually certified across the store boundary. Replay, cursor
derivation, transition validation, and output interpretation can all be redirected.

Required resolution properties:

- The verified program used by the fold must be inseparably derived from the admitted certified
  root.
- No production caller can construct or substitute verified program data.
- A hostile test attempts same-operation-ID substitution and receives a typed integrity rejection.

### AUTH-03 — A retained PostgreSQL pool bypasses validated mutation

Severity: Blocker

Violated guarantee:

No generic database pool or direct mutation authority may survive production assembly. All history
mutation must pass through the one validated store path.

Current behavior:

- Public openers accept a cloneable PgPool at
  [qualification.rs](crates/storages/postgres/src/qualification.rs#L93).
- Qualification deliberately requires the application role to be settable.
- That role receives direct INSERT and UPDATE privileges over history heads, batches, objects, and
  fact routing at [0001_store.sql](crates/storages/postgres/migrations/0001_store.sql#L509).
- The compile-fail test proves only that the backend has no pool getter. It does not prevent the
  caller retaining a clone before qualification.

Failure scenario:

The caller clones the pool, passes one clone to qualification, retains another, selects the required
application role, and mutates the raw history tables without ValidatedBatch, fold validation,
object-closure validation, exact-head classification, or Runtime.

Consequence:

The database permission model independently defeats both the store and Runtime choke points.

Required resolution properties:

- The production history credential cannot directly mutate the raw representation outside the
  qualified storage boundary.
- No cloneable raw connection authority survives assembly.
- A production-shaped hostile test retains every input authority available before qualification
  and proves that raw mutation remains impossible.

### AUTH-04 — PostgreSQL roles are not store-target scoped

Severity: Blocker

Violated guarantee:

Writer and maintenance authority must be bound to the exact store target and must reject sibling or
copied targets.

Current behavior:

- The migration creates fixed cluster-global roles such as mfm_store_application at
  [0001_store.sql](crates/storages/postgres/migrations/0001_store.sql#L28).
- Every migrated schema grants those same roles schema usage and table access at
  [0001_store.sql](crates/storages/postgres/migrations/0001_store.sql#L473).
- Qualification checks membership in those fixed role names rather than a store-bound privilege
  set at [qualification.rs](crates/storages/postgres/src/qualification.rs#L274).
- Tests create isolated schemas but grant their logins the same global roles; they do not test
  cross-schema denial.

Failure scenario:

A credential qualified for schema A selects the cluster-global application or maintenance role and
explicitly reads or mutates schema B.

Consequence:

Target-bound fencing and tenant/store isolation fail unless the platform introduces a
one-store-per-PostgreSQL-cluster deployment invariant. No such invariant currently exists.

Required resolution properties:

- Authority for one store cannot access a sibling or copied target in the same cluster.
- Qualification validates the effective target-specific privilege set, not only role membership.
- Cross-schema read, write, maintenance, and copied-credential hostile tests deny access.

### AUTH-05 — Purpose-limited read capabilities were removed

Severity: High

Violated guarantee:

Public read, trace, audit, replay, and export must receive distinct purpose authority rather than
generic access to the complete history.

Current behavior:

- [StructuredRunHistoryReader::load_verified](crates/kernel/store/src/structured/backend.rs#L337)
  accepts only a run ID and returns the complete verified run.
- Production retains one cloneable reader in
  [production_structured.rs](crates/app/src/production_structured.rs#L69).
- AuthorizedRunCall does not retain the approved grant at
  [application.rs](crates/app/src/application.rs#L852).
- Facade methods currently call the intended projection, but the type boundary cannot prevent one
  purpose from projecting another.

Consequence:

Purpose isolation is enforced only by current call-site discipline. Future code can accidentally
turn ReadPublic authority into trace, audit, replay, or export authority without a type error.

Required resolution properties:

- Downstream code receives authority specific to the approved purpose.
- A consumer holding one purpose cannot load or project another purpose’s data.
- Compile-fail and policy tests cover cross-purpose substitution.

## Structured language, composition, and provenance failures

### LANG-01 — SafeFailure disposition is neither exhaustive nor enforced

Severity: Blocker

Violated guarantee:

For an infallible state or a state declaring SafeFailureSuccessOnly, every admitted safe-failure
variant and every valid instance must deterministically produce success. This guarantee was intended
to make invalid settlement states unrepresentable.

Current behavior:

- [StructuredStateCallbacks](crates/kernel/program/src/structured.rs#L698) accepts an unrestricted
  observation-settlement closure regardless of the declared disposition.
- Qualification evaluates only caller-supplied reviewed_safe_failures at
  [structured.rs](crates/kernel/certify/src/structured.rs#L2806).
- Registry construction cannot prove that the corpus covers every capability-valid variant or
  instance at [structured.rs](crates/kernel/certify/src/structured.rs#L3657).
- Runtime accepts the callback result without enforcing the declared disposition at
  [structured.rs](crates/kernel/certify/src/structured.rs#L3920).

Failure scenario:

A capability admits safe failures A and B. The registration corpus includes only A mapped to
Success. The callback maps B to Failure or InvalidEvidence. Qualification succeeds, and a valid B
observation later violates the certified disposition.

Consequence:

An allegedly infallible state can fail or strand a run. The core guarantee depends on examples
selected by the implementation being qualified.

Required resolution properties:

- Exhaustiveness comes from a closed contract, type, or equally strong proof rather than a finite
  self-selected example corpus.
- A disposition-specific state cannot return an incompatible settlement.
- Compile-fail and runtime hostile tests cover omitted variants and valid values within each
  variant.

### LANG-02 — Typed child failure cannot cross its fragment boundary

Severity: Blocker

Violated guarantee:

A child operation’s typed failure must cross the child fragment boundary and enter the call site’s
declared failure handling path.

Current behavior:

- Certification creates a new FragmentBoundary::TypedFailure source slot around the child body’s
  failure merge at [structured.rs](crates/kernel/certify/src/structured.rs#L6690).
- The fold passes the old child binding into resolve_failure at
  [fold.rs](crates/kernel/store/src/structured/fold.rs#L2702).
- bind_exact compares that old lexical source to the new boundary slot at
  [fold.rs](crates/kernel/store/src/structured/fold.rs#L2800), so the references differ.
- Existing certification tests write JSON into arbitrary slots and do not enforce the lexical
  identity transition exercised by a real child failure.

Failure scenario:

A nested child reaches a legitimate typed failure. Instead of producing the child-call failure
output and following its handler, transition preparation rejects the lexical binding.

Consequence:

One of the central operation-composition paths is broken precisely where custom failure handling is
needed.

Required resolution properties:

- Every child success and failure exit has an explicit, verifiable boundary mapping.
- The mapping preserves source identity and cannot accept substitution from another child or exit.
- Execution tests drive a real nested child to failure and prove the exact call-site handler and
  durable provenance.

### PROV-01 — Match and FanOut retained provenance is flattened away

Severity: Blocker

Violated guarantee:

Selected Match arms and FanOut lanes must retain nominal provenance. Two lanes with identical value
bytes must remain distinguishable by group path, lane key, ordinal, contracts, and source
LexicalValueRef.

Current behavior:

- Match aliases the selected arm value directly into the merge slot at
  [fold.rs](crates/kernel/store/src/structured/fold.rs#L2652), discarding selected-arm identity.
- FanOut creates raw Success/Failure JSON wrappers at
  [fold.rs](crates/kernel/store/src/structured/fold.rs#L2717).
- Those wrappers omit the group path, lane key, ordinal, value/failure contract references, and
  source reference.
- The current store test explicitly requires byte-identical lanes to share the same wrapper object
  at [tests.rs](crates/kernel/store/src/structured/tests.rs#L800), contradicting the RFC’s
  lane-bound provenance requirement.

Failure scenario:

Two lanes produce identical bytes. Their retained wrapper and content reference are identical, so a
consumer cannot prove which lane, ordinal, or contract produced the value. Match has the analogous
problem when different arms return the same value.

Consequence:

Provenance substitution cannot be detected. Composition, replay, audit, and future policy wrappers
cannot rely on nominal origin.

Required resolution properties:

- Retained Match and FanOut products bind the structural origin independently of payload equality.
- Identical payloads in distinct arms or lanes produce distinguishable nominal provenance.
- Hostile tests substitute identical values between lanes/arms and receive exact rejection.

### PROV-02 — Normalized provenance can exhaust the process stack

Severity: High

Violated guarantee:

Hostile persisted or certified input must fail with bounded typed rejection rather than aborting the
process.

Current behavior:

- Denormalization admits up to 65,536 table entries at
  [structured.rs](crates/kernel/spec/src/structured.rs#L2292).
- resolve_slot recursively follows producer-slot references without a depth budget at
  [structured.rs](crates/kernel/spec/src/structured.rs#L2373).
- A shallow normalized JSON table can encode a long acyclic reference chain.
- Persisted verification expands that object before comparing it with recomputed closure data.

Failure scenario:

A hostile document provides a tens-of-thousands-deep acyclic slot chain. Deserialization succeeds
within the entry-count bound, then recursive resolution overflows the process stack before semantic
rejection.

Consequence:

Untrusted persisted or admission material can terminate the process rather than yielding a typed
integrity error.

Required resolution properties:

- Traversal has a strict, reviewed resource bound independent of process stack size.
- Near-limit valid input succeeds and one-past-limit input fails deterministically.
- Hostile tests run with normal production stack settings.

### LANG-03 — Structured aggregate values do not compose through all constructs

Severity: Medium

Violated goal:

Expansion should be able to wrap and compose any state, including states consuming or returning
FanOutResults.

Current behavior:

- State inputs accept StructuredValue, but Match output is restricted to MfmValue at
  [structured.rs](crates/kernel/program/src/structured.rs#L1367).
- Child outputs and policy-recipe inputs have equivalent restrictions.
- A policy therefore cannot transparently wrap an aggregate-consuming state, a Match cannot merge a
  fan-out join, and a child cannot return one.

Consequence:

The DSL does not yet provide the promised general expansion/wrapping dexterity. New telemetry,
security, failure, or orchestration policies may require special cases or new change sites.

Required resolution properties:

- The architect must define one coherent value algebra for state, Match, child, policy, and FanOut
  boundaries.
- Tests demonstrate policy wrapping, Match merging, and child return for aggregate values without
  alternate execution paths.

### LANG-04 — Hostile input can retain duplicate arm labels

Severity: Medium

The typed authoring builder rejects duplicate Match labels at
[structured.rs](crates/kernel/program/src/structured.rs#L1992), but hostile authored validation
checks canonical tags without independently proving label uniqueness at
[structured.rs](crates/kernel/certify/src/structured.rs#L7923). Empty duplicate arms can avoid the
later semantic-call collision checks. Custom recovery has the same omission around route validation
at [structured.rs](crates/kernel/certify/src/structured.rs#L8115).

Consequence:

Persisted hostile authored data can certify a structure the public builder cannot create, weakening
stable arm identity and future Match/recovery evolution.

Required resolution properties:

- Certification revalidates every invariant established by the public authoring path.
- Duplicate Match and recovery labels are rejected even when arms are empty or semantically
  identical.

### LANG-05 — Structural ordering does not follow declared ordinals

Severity: Low

StructuralPath derives ordering that compares declaration labels before declaration ordinals and
lane keys before lane ordinals at
[structured.rs](crates/kernel/spec/src/structured.rs#L415). The current fan-out frontier walks
declaration-ordered lanes and returns on the first action, so no reachable misordering was
demonstrated. Runtime nevertheless later selects a minimum occurrence path.

Consequence:

This is latent scheduling debt: a future multi-action frontier or small walker change could silently
replace declaration order with lexical label/key order.

Required resolution properties:

- Ordering semantics are explicit and agree with declaration/lane ordinals.
- Reverse-label/ordinal tests protect the intended order before a multi-action frontier is exposed.

Documentation follow-up:

The public documentation around whether a registered authored program can be replaced is ambiguous
relative to the intentional support-envelope API. This trust boundary should be documented
precisely when certification authority is repaired.

## Store and exact-head problems

### STORE-01 — Exact-head contention can produce generic availability failure

Severity: High

Violated guarantee:

Concurrent exact-head appends must deterministically classify the winner and loser as appended,
ExistingSame, or StaleHead. Availability failure is not a semantic contention result.

Current behavior:

- The run backend begins a SERIALIZABLE transaction and performs snapshot-producing validation at
  [structured.rs](crates/storages/postgres/src/structured.rs#L209).
- It acquires the run or tenant advisory lock later at
  [structured.rs](crates/storages/postgres/src/structured.rs#L591).
- A waiter can retain a snapshot predating the lock winner’s commit, then receive a serialization
  or uniqueness failure.
- Insert conflicts are flattened to BackendUnavailable.
- Race tests explicitly accept BackendUnavailable at
  [structured_history.rs](crates/storages/postgres/tests/structured_history.rs#L1564).

Consequence:

Callers cannot distinguish ordinary concurrency from database unavailability, weakening idempotent
recovery and exact-head reasoning.

Required resolution properties:

- A waiter observes the winning append and returns the exact semantic result.
- Same-run cross-process tests assert one precise winner/loser result rather than a set containing
  availability failure.
- Run-history and configuration exact-head paths share one authority/transaction setup where their
  requirements are the same.

### STORE-02 — An always-successful test fence is production-compilable

Severity: High

TestAuthoritativeWriterFence is publicly exported behind the ordinary parity-tests Cargo feature at
[lib.rs](crates/storages/postgres/src/lib.rs#L16) and always approves qualification at
[qualification.rs](crates/storages/postgres/src/qualification.rs#L444). A Cargo feature is not by
itself a test-only authority boundary.

Consequence:

A production artifact built with that feature can acquire authoritative writer qualification
without deployment fence evidence.

Required resolution properties:

- No production feature combination can construct an always-successful authoritative fence.
- Production-feature matrix or compile-fail tests prove that the fixture is unreachable outside
  tests.

### STORE-03 — Required cross-process and fault-injection proofs are absent

Severity: High

The suite covers sequential fresh-process continuation, but it does not cover same-run
cross-process exact-head/idempotency contention, copied/sibling-target generic writer fencing, or
acknowledgment fault injection. Transaction rollback injection covers object insertion rather than
every material atomic-write stage.

Consequence:

The most important authority, concurrency, and ambiguity guarantees are inferred from component
tests rather than proved at the physical boundary.

Required resolution properties:

- One shared conformance corpus runs against memory and PostgreSQL.
- PostgreSQL adds exact cross-process, target-isolation, privilege, acknowledgment, and
  stage-by-stage rollback tests.
- Assertions accept only the exact semantic outcome allowed by the RFC.

### STORE-04 — Memory and PostgreSQL accept different histories

Severity: Medium

Memory accepts object/envelope sizes that PostgreSQL rejects at 16 MiB or 65,536 objects. The shared
fold also starts from already-deserialized RawRunHistory and therefore cannot reject noncanonical
original bytes; PostgreSQL maintains a separate strict decoding path.

Consequence:

Backend-independent tests can approve a history or byte representation that the authoritative
backend rejects, and canonical-byte enforcement has more than one owner.

Required resolution properties:

- Memory and PostgreSQL agree on all semantic and resource acceptance bounds.
- Original-byte canonicality is owned at one explicit boundary and exercised by both conformance
  paths where applicable.

### STORE-05 — The SQLx metadata gate does not cover dynamic queries

Severity: Medium

The PostgreSQL crate uses roughly fifty runtime sqlx::query calls, with no checked query macros or
tracked metadata for them. The online cargo sqlx prepare --check task therefore does not provide
compile-time metadata validation for those queries. The catalog validator is useful but proves a
different property. See [structured.rs](crates/storages/postgres/src/structured.rs#L256) and
[nixfied.nix](nixfied.nix#L386).

Required resolution properties:

- Documentation and gate claims accurately describe what query/schema properties are checked.
- Every query-shape guarantee needed for release has an executable owner.

### STORE-06 — Fact-frontier validation repeatedly scans retained history

Severity: Medium

Every publication/barrier append computes count, minimum, and maximum over retained publications at
[structured.rs](crates/storages/postgres/src/structured.rs#L958). The query omits the leading store
scope and epoch columns of the primary key declared at
[0001_store.sql](crates/storages/postgres/migrations/0001_store.sql#L268).

Consequence:

Fact-bearing append cost grows with retained history and risks cumulative quadratic behavior.

Required resolution properties:

- Frontier validation has bounded or appropriately indexed cost while preserving append-only
  verification.
- Long-history performance tests cover the authoritative query plan.

### STORE-07 — Transaction authority setup is duplicated

Severity: Medium

Run history and configuration independently implement pool begin, role selection, search-path
pinning, identity/schema verification, and commit classification at
[structured.rs](crates/storages/postgres/src/structured.rs#L209) and
[configuration.rs](crates/storages/postgres/src/configuration.rs#L198). Their inconsistent isolation
choices already contribute to STORE-01.

Required resolution properties:

- Shared authority and transaction guarantees have one implementation owner.
- Run-specific and configuration-specific behavior remains explicit without duplicating the common
  security boundary.

## EVM production, recovery, and wallet-authority failures

### EVM-01 — A fresh production submission cannot use the qualified keystore signer

Severity: Blocker

Violated guarantee:

At least one production-qualified path must execute Runtime through the live EVM adapter, guarded
real keystore signer, PostgreSQL wallet authority, broadcast, finality observation, and completion.

Current behavior:

- Keystore qualification requires a complete public-key-plus-account binding at
  [signer.rs](crates/keystore/src/signer.rs#L165).
- EVM builds its guarded signing request with account identity only at
  [signing.rs](crates/domains/evm/src/signing.rs#L236).
- Exact optional-field comparison at [lib.rs](crates/signing/src/lib.rs#L751) rejects the request
  as BindingMismatch before the guard or signer runs.

Why current tests pass:

- Direct broadcast/completion integration phases use DeterministicTestSigner at
  [evm_postgres_submission.rs](tests/integration/tests/evm_postgres_submission.rs#L2250).
- The later “production” phases reuse the same already-completed semantic intent at
  [evm_postgres_submission.rs](tests/integration/tests/evm_postgres_submission.rs#L436), so they
  converge without invoking the keystore signer.
- The focused keystore test at
  [evm_postgres_submission.rs](tests/integration/tests/evm_postgres_submission.rs#L468) checks only
  the declared identity.

Consequence:

The claimed real production end-to-end proof is false for a fresh submission.

Required resolution properties:

- A fresh, unique semantic intent reaches the actual QualifiedKeystoreSigner and completes.
- The test proves that signing occurred and cannot converge from previously completed authority
  state.
- Identity mismatch tests still fail before signing and without leaking secret material.

### EVM-02 — Persisted submission semantics are not requalified

Severity: Blocker

Violated guarantee:

Every configured submission revision admitted for execution must match the sealed deployment’s
exact signer, signing profile, broadcast capability, submission contract, expansion contract, and
terminal assurance policy.

Current behavior:

- Deployment assembly correctly invokes deployment_semantics at
  [application.rs](crates/app/src/application.rs#L173).
- ProductionBackend discards most of those exact semantics and retains mainly route, chain, and
  activation data at
  [production_structured.rs](crates/app/src/production_structured.rs#L69).
- Admission later performs submission_configuration.validate plus route/chain/activation
  comparisons at
  [production_structured.rs](crates/app/src/production_structured.rs#L585), rather than recomputing
  deployment_semantics against the sealed signer.
- Live attestation and broadcast proof construction echo request-supplied contract references at
  [structured.rs](crates/live/evm/src/structured.rs#L289) and
  [structured.rs](crates/live/evm/src/structured.rs#L395).

Failure scenario:

A configuration revision for the same target contains syntax-valid substituted signing-profile,
submission, or terminal-assurance references. Admission accepts it, and real adapter activity emits
provenance claiming those substituted contracts.

Consequence:

Durable history can falsely claim that real signing and broadcast were performed under contracts
that the sealed deployment never qualified.

Required resolution properties:

- Every resolved configuration revision is checked against the exact sealed deployment semantics
  before admission.
- Syntax-valid substitutions for each code-owned contract are rejected before any history or wallet
  mutation.
- Attestation and broadcast evidence derives contract identity from qualified authority rather than
  echoing untrusted configuration.

### EVM-03 — A later run does not observe retained activated candidates

Severity: Blocker

Violated guarantee:

After one run ends definitively, a later run with a different run ID must converge on the existing
wallet reservation. It must observe the entire retained activated-candidate prefix before deciding
whether replacement or exhaustion is appropriate.

Current behavior:

- Reserved status sets next_candidate_ordinal to activated_candidates.len() at
  [submission_process.rs](crates/domains/evm/src/submission_process.rs#L293).
- Candidate selection begins after the retained prefix at
  [submission_process.rs](crates/domains/evm/src/submission_process.rs#L607).
- The flow activates the next replacement or declares ReplacementPolicyExhausted without observing
  retained transaction hashes.

Failure scenario:

Run A activates the only candidate and terminates before finality. The transaction later finalizes.
Run B starts with a different run ID, sees a one-candidate retained prefix, never observes that hash,
and closes with ReplacementPolicyExhausted.

Consequence:

Independent-run recovery produces the wrong semantic result and cannot complete already-finalized
work.

Required resolution properties:

- A later run reads and observes every retained candidate in certified order.
- It can complete from any retained candidate that becomes canonical.
- Exact tests use different run IDs and cover restart before receipt, after receipt, before
  finality, after finality, and candidate-family exhaustion.

### EVM-04 — Replacement eligibility self-certifies

Severity: Blocker

Violated guarantee:

A replacement may be activated only from committed EVM evidence accepted by the frozen replacement
policy.

Current behavior:

- CandidateActivationPermit::Replacement contains predecessor references, ordinal, policy data, and
  an eligibility reference.
- Permit derivation at
  [wallet_authority.rs](crates/domains/evm/src/wallet_authority.rs#L1111) hashes the reservation,
  activated prefix, predecessor, and next ordinal.
- It contains no committed transaction lookup, receipt, finalized head, current status, or typed
  CandidateReplacementEligibility evidence.
- Reserve, activate, and complete requests remain raw clonable value products rather than the RFC’s
  producer-bound wallet algebra.

Consequence:

The same layer requesting replacement effectively manufactures the proof that replacement is
allowed. Current-reservation and current-chain-state freshness are not carried into authority.

Required resolution properties:

- Replacement authority consumes exact committed evidence from the qualified observation states.
- The replacement policy and all evidence it evaluates are frozen into durable identity.
- Stale, cross-run, cross-reservation, cross-candidate, and caller-fabricated eligibility are
  rejected.

### EVM-05 — Possible target entry is classified as definite rejection

Severity: Blocker

Violated guarantee:

Once a broadcast request may have crossed the physical-entry boundary, uncertainty must remain
EntryUnknown until observation resolves it. Ordinary safe failure requires proof that target entry
did not occur.

Current behavior:

- HTTP transport classifies non-200 after send as BoundaryFailure::AfterEntry at
  [transport/mod.rs](crates/live/evm/src/transport/mod.rs#L820).
- Mapping later converts post-entry HTTP status and generic JSON-RPC error to DestinationRejected at
  [transport/mod.rs](crates/live/evm/src/transport/mod.rs#L1045).
- The live adapter turns DestinationRejected into ordinary SafeFailure at
  [structured.rs](crates/live/evm/src/structured.rs#L383).

Failure scenario:

A proxy forwards the raw transaction to a node, the node accepts it, and the proxy then returns a
non-200 response or an unrecognized error. Runtime records definite rejection and may close the run
even though the transaction entered the network.

Consequence:

The platform can make unsafe subsequent decisions from a false non-entry claim.

Required resolution properties:

- Every post-send outcome without exact proof of non-entry remains EntryUnknown.
- Node-specific “already known” and definite-rejection classifications are explicit, reviewed, and
  tested across supported transports.
- Ambiguity tests simulate acceptance followed by HTTP/JSON-RPC failure and prove no false safe
  failure.

### EVM-06 — The allocator can permanently reserve u64::MAX

Severity: Blocker

Violated guarantee:

Every reserved nonce must be protocol-valid for a transaction.

Current behavior:

- First allocation accepts a provider pending nonce of u64::MAX at
  [authority.rs](crates/storages/evm-postgres/src/authority.rs#L1419).
- Later allocation accepts high-water-plus-one equal to u64::MAX.
- SQL permits values through 18446744073709551615 at
  [0001_wallet_authority.sql](crates/storages/evm-postgres/migrations/0001_wallet_authority.sql#L177).
- [EIP-2681](https://eips.ethereum.org/EIPS/eip-2681) makes transaction nonce greater than or equal
  to 2^64 - 1 invalid.

Failure scenario:

The provider returns the maximum value, or the local high-water mark reaches one below it. Authority
creates an incomplete reservation that can never be broadcast or completed.

Consequence:

The nonce domain can remain permanently busy, recreating the poisoned-resource condition the RFC is
intended to eliminate.

Required resolution properties:

- Protocol-invalid values are rejected before reservation.
- Boundary tests cover maximum-valid, first-invalid, provider-ahead, and local-high-water cases.
- Existing domain state cannot be advanced into an unbroadcastable reservation.

### EVM-07 — Signer integrity failures become ordinary unavailability

Severity: High

Current behavior:

Wrong result identity, invalid signature, wrong recovered address, deterministic-profile mismatch,
signed-hash mismatch, and several binding integrity faults are all mapped to SignerUnavailable at
[structured.rs](crates/live/evm/src/structured.rs#L273). Bounded component invocation similarly
maps integrity failure into the same domain failure.

Consequence:

A compromised, misbound, or contract-violating signer is indistinguishable from an operationally
unavailable signer. The run can take normal failure handling rather than blocking on an integrity
fault.

Required resolution properties:

- Operational availability and integrity/contract violations remain distinct through adapter,
  Runtime, persisted history, and public redaction.
- Hostile signer tests cover each integrity class and prove it cannot become SafeFailure.

### EVM-08 — Durable identity does not freeze all execution behavior

Severity: High

Current behavior:

The observation-round bound and complete expansion contract are not included in the permanent
transaction-intent or candidate-family digest. The same authenticated token and reservation can
therefore resume under changed observation or expansion behavior.
See [submission.rs](crates/domains/evm/src/submission.rs#L130),
[wallet_authority.rs](crates/domains/evm/src/wallet_authority.rs#L388), and
[wallet_authority.rs](crates/domains/evm/src/wallet_authority.rs#L441).

Consequence:

Persisted authority identity does not fully determine the algorithm that later runs use.

Required resolution properties:

- Every behavior-affecting policy required for safe continuation is frozen into the durable intent
  or referenced immutable contract closure.
- Resume under any changed behavior is rejected before adapter or wallet mutation.

### EVM-09 — Completion does not retain the full public-result closure

Severity: High

Current behavior:

Wallet completion stores a small hard-coded execution-disposition JSON value, while the complete
CompletedWalletNonce public output is synthesized later. The durable completion authority therefore
does not retain the full content-addressed result closure that consumers observe.
See [wallet_authority.rs](crates/domains/evm/src/wallet_authority.rs#L1274) and
[submission_process.rs](crates/domains/evm/src/submission_process.rs#L1369).

Consequence:

The public output is not wholly proven by the canonical wallet completion record.

Required resolution properties:

- Completion binds the exact canonical public result and all provenance needed to reconstruct it.
- Replay reconstructs the same output without synthesizing uncommitted semantic fields.

### EVM-10 — Wallet status and mutation cost grows with the complete lifetime

Severity: High

Current behavior:

Status and mutation reload every historical reservation, candidate family, and completion using
N+1 queries beginning at
[authority.rs](crates/storages/evm-postgres/src/authority.rs#L174). Mutation paths perform this
while holding serialization resources.

Consequence:

Latency and lock duration grow without bound for a long-lived nonce domain, increasing contention
and eventual availability risk.

Required resolution properties:

- Current status and mutation validation have a bounded or explicitly indexed cost independent of
  the complete historical lifetime.
- Historical auditability remains append-only without making each new mutation replay all history
  through N+1 queries.
- Performance/concurrency tests cover long-lived domains.

### EVM-11 — Release evidence is not bound to the authority’s current target

Severity: High

Current behavior:

Initial wallet release target validation is bound to initial_store_incarnation_ref, but successor
releases may name another physical target while operations continue using the same concrete
authority. The authority exposes its actual current incarnation, but no relation to the successor
release target is enforced.
See [application.rs](crates/app/src/application.rs#L327),
[structured_wallet.rs](crates/live/evm/src/structured_wallet.rs#L49),
[physical_release.rs](crates/live/evm/src/physical_release.rs#L105), and
[authority.rs](crates/storages/evm-postgres/src/authority.rs#L122).

Consequence:

Persisted access provenance can claim a rotated physical target that the invoked authority does not
represent.

Required resolution properties:

- Every release selected for an invocation is proven current for the exact physical authority used.
- Rotation tests cover successor, stale predecessor, copied target, and mismatched authority
  incarnation.

Additional EVM/wallet problems:

- Provider validation permits proof collections whose hex-encoded JSON cannot fit its own 1 MiB
  protocol frame. See [provider.rs](crates/storages/evm-postgres/src/provider.rs#L30) and
  [provider.rs](crates/storages/evm-postgres/src/provider.rs#L680).
- Reserve validates pending-floor policy and route references mainly syntactically rather than
  against the exact qualified policy and route catalog. See
  [submission_registry.rs](crates/domains/evm/src/submission_registry.rs#L1248).
- RPC/signer/broadcast currentness is represented partly by process-local release histories rather
  than one enforced per-access live fence. The deployment coupling that revokes the old physical
  path is not explicit in the types.

## Application authorization and replay failures

### APP-01 — Export does not authorize prior-run source dependencies

Severity: Blocker

Violated guarantee:

Export authority for a consumer run must not disclose material from a prior source run without
separate authority for that source.

Current behavior:

- export_run authorizes only the requested root run at
  [application.rs](crates/app/src/application.rs#L781).
- Production then loads and exports it directly at
  [production_structured.rs](crates/app/src/production_structured.rs#L509).
- Selected prior-run facts retain source subject, response, claim, and canonical bytes at
  [fact_scan.rs](crates/kernel/store/src/structured/fact_scan.rs#L484), and those become part of the
  root run’s exported object closure.
- The baseline recursively discovered and authorized every source dependency.
- SourceRunExportDenied remains defined but is unreachable in production.

Failure scenario:

A principal has Export authority for a consumer run but not for its prior source run. Exporting the
consumer succeeds and includes the source fact material.

Consequence:

Cross-run material can be disclosed without the source run’s export grant.

Required resolution properties:

- Every exported dependency is authorized under the correct source target and purpose.
- Denial produces the exact redacted public error before export bytes are emitted.
- Tests cover nested source dependencies, tenant/principal substitution, and indistinguishable
  missing/denied sources.

### REPLAY-01 — Portable history is not independently verifiable

Severity: Blocker

Violated guarantee:

A portable export must contain enough canonical evidence to verify the recorded append-only history
offline without access to the current live store.

Current behavior:

- The export DTO contains flattened records and objects at
  [production_structured.rs](crates/app/src/production_structured.rs#L919).
- It omits CommittedBatch envelopes containing predecessor linkage, append identity, candidate and
  commit digests, per-append object membership, fact coordinates, writer epoch, and atomic record
  grouping.
- Replay “validation” regenerates the current online export and compares bytes at
  [production_structured.rs](crates/app/src/production_structured.rs#L1008).
- A semantic terminal export advertises the complete append head while omitting adjacent RunClosed;
  the current integration test explicitly expects this inconsistent projection.

Consequence:

The artifact cannot independently prove history completeness, exact-head linkage, append atomicity,
or object membership. Even adding a verifier later would not make the current semantic-head
projection internally consistent.

Required resolution properties:

- The portable format carries the exact canonical append envelopes and closure needed by the sole
  callback-free verifier.
- Verification works offline and does not consult or regenerate from the live store.
- Physical and semantic fixation are both explicit and internally reproducible.
- Tampering, truncation, append reordering, object substitution, missing closure, and false-head
  tests fail with exact typed results.

### APP-02 — compare_current is permanently stubbed

Severity: High

Current behavior:

Both non-verify replay branches validate the export and return a fixed unavailable result at
[production_structured.rs](crates/app/src/production_structured.rs#L467). ReplayRequest has no route
for supplying a candidate. The baseline implementation invoked the qualified current registry and
returned a real comparison report.

Consequence:

A retained public capability was silently removed while its route and DTO remained, creating an
always-unavailable compatibility-shaped surface rather than one current implementation.

Required resolution properties:

- The architect must decide whether comparison belongs to the current design.
- If retained, the request, qualified candidate resolution, comparison, authorization, and tests
  must be complete.
- If removed, its route, DTOs, errors, documentation, and tests must be deleted in the same cutover.

### APP-03 — Store admission accepts an independently supplied run ID

Severity: Medium

Current behavior:

StructuredAdmissionRequest accepts a caller-supplied RunId independently of store scope, tenant,
operation, and invocation at
[mutation.rs](crates/kernel/store/src/structured/mutation.rs#L123). Admission persists it verbatim,
and replay checks only equality with the containing envelope. Production application code happens
to derive the expected ID correctly.

Consequence:

Other legitimate Runtime/library assembly paths can persist a run identity unrelated to the
authoritative derivation preimage.

Required resolution properties:

- One owner derives or verifies run identity from the complete frozen preimage.
- No public admission path can persist an unrelated typed digest.
- Tests invoke store admission rather than independently reimplementing and comparing the formula.

### REPLAY-02 — Portable evidence has the wrong implementation owner

Severity: Medium

Portable export serialization, digesting, and validation live in mfm-app at
[production_structured.rs](crates/app/src/production_structured.rs#L919), while
[mfm-replay portable.rs](crates/kernel/replay/src/portable.rs#L1) owns only a media-type constant and
enum.

Consequence:

Projection and verification ownership is split, creating a second change site and helping explain
why the portable format does not use the sole recorded verifier.

Required resolution properties:

- The architecture assigns portable evidence and recorded verification one owner.
- Application code requests the projection through that owner rather than defining a second format
  and validator.

### REPLAY-03 — Replay-result identity is not annex-backed

Severity: Medium

StructuredReplayResult invents a schema identity from a hard-coded name at
[structured.rs](crates/kernel/replay/src/structured.rs#L522). The generated recoverability registry
does not define a replay-result contract at
[generate.py](contracts/recoverability/generate.py#L554).

Consequence:

The public replay DTO claims a current schema identity that is not generated from the authoritative
annex.

Required resolution properties:

- Every retained public DTO identity has one authoritative generated contract.
- If replay result is not part of the current public contract, its schema claim and surface are
  removed rather than synthesized ad hoc.

Verification follow-up:

The current model-check task does not execute all claimed generated-contract drift checks. Gate
documentation and the Nixfied task graph must agree on which annex/corpus artifacts are regenerated
or rejected on drift.

## Security and secret-lifetime problem

### SEC-01 — Decrypted key bytes cross an ordinary stack copy

Severity: Medium

Current behavior:

[operations.rs](crates/keystore/src/keystore/operations.rs#L106) copies a
Zeroizing<[u8; 32]> into an ordinary by-value array before
[secure_key.rs](crates/keystore/src/keystore/secure_key.rs#L17) wraps it again. Source and final
destination are zeroized, but compiler or ABI temporaries created by the copy are outside either
zeroizing owner. Qualification, handoff, and each sign exercise this path.

Consequence:

Plaintext private-key material may remain in stack copies after the owning values are dropped.

Required resolution properties:

- Decrypted key ownership moves directly between zeroizing containers without an unprotected
  by-value intermediate.
- Tests witness decrypted-key ownership and cleanup, not only unlock-buffer cleanup.
- Redaction tests cover qualification, handoff, signing, and Debug/error rendering with sentinel
  credentials and paths.

No direct persisted, logged, Debug, or public-error secret leak was otherwise found.

## Simplicity, verification, and delivery problems

### QUALITY-01 — Core responsibility is concentrated into oversized modules

Severity: High

The overall production Rust/SQL LOC reduction is real and valuable, but important responsibilities
have been concentrated:

- crates/kernel/certify/src/structured.rs is approximately 11,400 lines and combines certification,
  process qualification, physical binding, authority construction, and live invocation support.
- crates/kernel/store/src/structured/fold.rs is approximately 4,700 lines.
- crates/kernel/spec/src/structured.rs is approximately 3,300 lines.
- crates/kernel/program/src/structured.rs is approximately 2,700 lines.
- PostgreSQL run and configuration paths duplicate authority and transaction setup.

Consequence:

The number of files fell, but future change sites and coupled responsibilities remain high. Several
defects above occur at seams hidden inside these large modules or between duplicated transaction
setups.

Required resolution properties:

- Responsibility boundaries follow the repository taxonomy without adding parallel paths or public
  indirection.
- A change to certification, fold semantics, physical binding, or persistence transaction setup has
  one clear owner.
- Refactoring reduces coupled concepts and LOC while preserving readability, validation, tests, and
  the one current design.

### VERIFY-01 — Important boundary proofs are absent

Severity: High

The reported CI run passed, and focused certifier, store, PostgreSQL, wallet, keystore, signing, and
integration tests are substantial. The missing proofs correspond directly to the defects above:

- no compile-fail proof that only Runtime can append semantic history;
- no hostile custom-verifier program-substitution test;
- no real nested-child typed-failure execution test;
- no omitted SafeFailure variant or value test;
- no identical-payload cross-lane/arm substitution test;
- no near-limit normalized-provenance depth test;
- no sibling-schema or copied-target PostgreSQL denial test;
- no same-run cross-process exact-head test with one exact loser result;
- no fresh unique-intent production test through QualifiedKeystoreSigner;
- no different-run retained-candidate recovery test;
- no accepted-then-HTTP-failed broadcast ambiguity test;
- no u64::MAX nonce boundary test;
- no configured semantic-contract substitution test at application admission;
- no source-run export authorization test;
- no offline portable-history verification test.

Several current tests mask defects by accepting generic BackendUnavailable, checking only identity,
using a deterministic signer, reusing already-completed wallet state, or asserting the inconsistent
semantic export head.

Required resolution properties:

- Every problem fixed by the next implementation has a boundary-focused regression test that fails
  against 31aced23a.
- Tests assert the exact semantic result, durable prefix, callback count, authority denial, or
  rejected hostile state.
- Broad CI remains a final gate, not a substitute for the missing semantic assertions.

### PROCESS-01 — The claimed checkpoint process is not represented in the branch

Severity: High for delivery confidence

Current evidence:

- Commit f12c4586 added four ordered implementation checkpoints and required independent review at
  each checkpoint.
- f12c4586 is not an ancestor of reviewed HEAD 31aced23a.
- The plan at reviewed HEAD is unchanged from b76d77a09 and does not contain those checkpoints.
- The engineer reported that all four checkpoint reviews approved, but no durable branch artifact
  identifies those reviews, their findings, or their resolutions.
- Commit 31aced23a contains only a small set of hostile regression/support files and does not repair
  many of the core, application, or keystore defects found in review.

Consequence:

The claimed review process cannot be audited and evidently did not catch failures at the intended
checkpoint boundaries.

Required resolution properties for the next delivery:

- The architect-approved problem resolution and implementation plan are ancestors of the
  implementation.
- Each required checkpoint leaves durable review evidence identifying reviewed revision, blockers,
  and resolution revision.
- The final report distinguishes proved production paths from test-only paths, placeholders,
  externally required deployment authority, and remaining material uncertainties.

## Foundations worth preserving

The next architecture and fix plan should preserve the parts that are genuinely strong unless a
problem above requires changing them:

- The retired graph/executor path and crates are genuinely removed.
- There is one structured authored/expanded/certified model rather than a compatibility path.
- The journal has exactly five record families.
- Adjacent root-atomic RunClosed validation is strong.
- Memory mutation is atomic under one mutex.
- PostgreSQL writes the batch envelope, objects, fact route, CAS head, and adjacent closure in one
  transaction; injected object failure rolls the transaction back.
- Fresh-process PostgreSQL continuation is real and a closed pool does not fall back to memory.
- Replay folds persisted history without callbacks into states or adapters.
- Runtime’s internal physical-access sequence uses private affine typestates.
- Ambiguous authorization acknowledgment does not mint invocation authority.
- Observation retry does not reinvoke an effect.
- Effect refresh after committed SupersededBeforeEntry is separated from ordinary observation
  continuation.
- Wallet registry promotion, lineage locking, immutable SQL shape, and sealed lease concepts are
  substantial.
- Admission policy dispatch correctly binds store, operation, configured target, invocation,
  tenant, and principal.
- The internal prior-run fact scanner remains bounded, tenant-scoped, and purpose-limited.
- CLI and REST route/command sets remain present.
- Secret and public-error redaction is generally careful.

## Deliberate boundaries that are not classified as defects

The next architect should not treat these observations as implementation bugs without changing the
design contract:

- Standalone CLI and REST startup fail closed when no deployment-qualified authoritative writer
  fence and physical authorities are supplied. The current plan explicitly leaves that deployment
  binding external. The defect is only if the product claims those wrappers are independently live.
- Production reproduce was already unavailable in the baseline through an unavailable resolver.
  compare_current is classified separately because it was a working retained capability and is now
  permanently stubbed.
- The review found no direct defect in five-family atomic closure, memory-store atomicity, or the
  basic PostgreSQL batch/object/head transaction itself.

## Material uncertainties for the architect

### PostgreSQL deployment topology

Choice or assumption:

The review assumes multiple independently fenced stores or schemas may exist in one PostgreSQL
cluster.

Why uncertain:

The implementation uses cluster-global roles as though one store per cluster might be intended, but
the RFC and architecture do not declare or certify that invariant.

Consequence if wrong:

If one store per cluster is mandatory, AUTH-04 becomes an undocumented deployment/qualification
contract rather than an immediate cross-store breach. AUTH-03 remains a blocker regardless.

How to resolve:

Make one explicit architecture decision, encode it in qualification and deployment documentation,
and prove it with copied/sibling-target tests.

### Physical release currentness

Choice or assumption:

The review assumes the core must prove that the exact physical target invoked is the target named
by the current release.

Why uncertain:

Some currentness and revocation may be expected from external deployment infrastructure, but that
coupling is not expressed in the public types or certified contracts.

Consequence if wrong:

Release rotation can produce provenance naming a target different from the authority actually used,
or the platform may silently depend on an operational revocation step that recovery cannot verify.

How to resolve:

Specify the owner of currentness, the evidence consumed per invocation, and the revocation/rotation
sequence; then test old, successor, copied, and mismatched targets.

## Acceptance threshold for a replacement implementation

The implementation must not be declared complete merely because broad CI passes or production LOC
falls. Before acceptance:

1. Every Blocker and High problem above is either resolved in code or deliberately removed from the
   current RFC/design contract together with all affected APIs, tests, and documentation.
2. Invalid writer, verifier, settlement, provenance, replacement, and physical-binding states are
   made unrepresentable where practical and are otherwise rejected at one owning boundary.
3. A fresh production submission completes through the real qualified EVM RPC, keystore signer,
   wallet authority, broadcast, observation, and completion path.
4. A later, different run converges on every valid retained wallet state without reinvoking an
   uncertain effect or declaring false exhaustion.
5. Portable exports verify offline through the sole callback-free history verifier.
6. Memory and PostgreSQL pass the same exact conformance corpus, with PostgreSQL-specific
   cross-process, target-fence, privilege, and fault-injection coverage.
7. The final code has one writer path, one certified-program authority, one transaction-authority
   setup per shared requirement, and no compatibility or placeholder path.
8. The final review reports material uncertainties honestly and provides durable evidence for each
   checkpoint and production claim.
