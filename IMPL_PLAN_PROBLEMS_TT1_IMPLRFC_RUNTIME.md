# Implementation Plan: TT1 Runtime-History Remediation

Status: superseded by the [TT2 runtime-history remediation
plan](IMPL_PLAN_PROBLEMS_TT2_IMPLRFC_RUNTIME.md)

This document is retained as the historical TT1 remediation plan. Do not implement it. Its
implementation and review exposed the TT2 findings; the linked TT2 plan owns the one current
remediation direction.

This plan defined the implementation direction that followed
`PROBLEMS_TT1_IMPLRFC_RUNTIME.md`. It targeted every problem in that ledger against the contracts
then current in `docs/design.md` and `docs/architecture.md`, and applied the correct-by-construction
policy in `docs/code-quality.md`. The RFC remains the historical source of the runtime-history
objective; the current design documents own the target contract.

That implementation started from the revision containing this plan and targeted a complete cutover:
no compatibility API, old persisted-format reader, dual authority path, fallback writer, or
best-effort PostgreSQL mode was to be retained.

## Material uncertainties

none

The following are frozen design choices, not open questions:

- One PostgreSQL cluster may host multiple independently fenced store targets.
- Deployment infrastructure is the trusted credential and external-fence boundary. Ordinary MFM
  assembly receives only opaque, exact-target capabilities and never receives raw PostgreSQL
  credentials, a `PgPool`, connection options, or a writer-capable backend.
- Physical release currentness is checked at every protected access or transaction. A
  process-start check is not sufficient.
- `compare_current` is deleted. Recorded verification remains supported; the separately selected
  reproduction mode remains explicitly unavailable until its own contract is designed.
- Breaking the first implementation's public and persisted contracts is allowed. Each affected
  format is replaced with one current version and its superseded baseline is deleted.

If implementation discovers a new material choice involving architecture, ownership, or a design
contract, work stops at the preceding coherent commit and the repository's architect-agent rule is
applied before code continues.

## Outcome

After this sequence:

1. Runtime is the only production path that can request semantic run-history mutation.
2. The store owns the sole fold, sole append transaction authority, certified-root verification,
   canonical bounds, and exact append classification.
3. Certification cannot be forged or substituted by callers.
4. The structured language preserves recursive aggregate values and exact lexical provenance while
   making invalid settlement states and empty fan-outs unrepresentable.
5. PostgreSQL authority is exact-target, role-separated, current at every transaction, and
   semantically equivalent to the memory backend under faults and contention.
6. EVM submission uses a real qualified signing identity, preserves entry ambiguity, advances every
   activated candidate from producer evidence, and retains the complete public recovery closure.
7. Read authority is purpose-scoped, export authorizes every recursively referenced run before
   emitting bytes, and portable replay is independently verifiable with the same store fold.
8. Every ledger item has a regression proof, and independent checkpoint reviews are durable in the
   branch.

## Foundations that must remain intact

The remediation must preserve the already-correct foundations called out by the problem ledger:

- one structured `State` / `Match` / `FanOut` execution model, with the superseded graph executor
  remaining deleted;
- exactly the five run-record families and append-only `run:*` streams;
- per-append atomicity and root-atomic close;
- canonical, float-free structured hashing and content-addressed manifests, facts, snapshots, and
  outputs;
- callback-free replay;
- no memory fallback for a failed production store;
- affine access brackets, authorization ambiguity, no unsafe reinvocation, and effect refresh;
- the bounded prior-fact scanner and the current CLI/REST transport sets, except for the explicit
  `compare_current` deletion;
- redaction of secrets and internal diagnostics on every persisted and public surface.

No remediation commit may reintroduce a second language, fold, verifier, transaction wrapper,
export schema, or EVM decision engine.

## Fixed target architecture

### Runtime, certification, and store authority

The dependency direction becomes:

```text
mfm-certify <- mfm-runtime <- mfm-store
```

Here `A <- B` means that `B` depends on `A`.

- `mfm-certify` owns deterministic program expansion, predicates, proof construction, and the
  concrete callback-free `AdmissionVerificationRegistry`. It has no store dependency and no live
  process authority.
- `mfm-runtime` owns `Runtime`, live process/access-bracket authority, prior-run scanner invocation,
  private semantic command/attempt states, and the consumer-side `RuntimeHistoryPort` contract.
- `mfm-store` implements that port with a private production adapter around the sole fold and
  backend, and depends directly on `mfm-certify` only for its concrete registry/certified-program
  types. Its production assembly returns a `Runtime` plus target/purpose-specific readers, never a
  store, run-history writer, proposal constructor, verified mutation permit, append attempt, pool,
  or concrete backend. Each reader exposes only its sealed projection. The app facade pairs it with
  an app-owned `AuthorizedRunCall<P>` that retains the exact approved grant for the same purpose.

Configuration history remains a separate append-only stream, not a sixth run-record family.
Deployment/maintenance may receive an exact-target configuration append capability from the
deployment TCB; ordinary application assembly receives resolve-only authority. It shares the
PostgreSQL transaction security boundary where appropriate but does not pass through Runtime.

The cross-crate port is necessarily visible to `mfm-store` and may be implementable for isolated
tests or custom stores. Constructing a Runtime with a caller-owned port conveys authority only over
resources that port already owns. Callers cannot combine such a port with MFM's production backend:
the concrete production adapter and backend stay private, cannot be extracted from Runtime, and are
created only by store-owned production assembly. The security property is structural ownership of
the production adapter, not a convention attached to a public writer method.

On a certified-root cache miss, the store calls the concrete registry's `verify_root` and consumes
the resulting private-field `CertifiedProgram`. The cache key is the certified content identity.
There is no verifier trait, public verified-program constructor, expanded-program injection path,
or second verification algorithm.

Run identifiers are derived inside the authoritative store adapter from the exact annex-backed
`mfm.run-id-preimage.v1`: store scope, tenant scope, entry-point operation ID, and invocation
identity. A caller supplies invocation material, never a digest that is trusted as a run
identifier. Other admission material remains excluded exactly as the current identity contract
specifies.

### Structured value, failure, and provenance model

Every admission, state, child-success, Match-result, policy-success, fan-out-lane, and
operation-root success boundary uses one sealed recursive algebra:

```text
StructuredValue =
    Retained<MfmValue>
  | NonEmptyFanOutResults<StructuredValue, FailureValue>
```

Failures, `Never`, Match selectors and variant payloads, facts, configurations, and capability
request/returned/safe-failure values remain scalar. `FanOutResults` has a head plus tail, so an
empty aggregate cannot be constructed or decoded.

Qualification owns one recursive `StructuredValueDefinition` and one schema/contract encoder.
Store validation dispatches through that definition; it does not grow a parallel aggregate append
path. Exact producer identity remains in certified slots and folded `LexicalValueRef` values.
Labels are diagnostic only; canonical ordering and identity use qualification-assigned ordinals.

Safe-failure settlement is split from returned-value settlement. The safe-failure callback can
construct only a success proposal. It cannot express `Failure` or `InvalidEvidence` for any
inhabited safe-failure value. Adapter unavailability may leave an attempt uncommitted, but it cannot
be recorded as a semantic failure. This removes the need for a self-selected example corpus.

### PostgreSQL authority and transaction model

Each target has a distinct non-login owner plus qualification, run-reader, run-writer,
configuration-reader, and configuration-writer roles. Grants are limited to that target's schema
and function set. Run roles have no configuration DML; configuration roles have no run/fact DML;
readers and qualification have no DML.

Production openers consume an opaque deployment-issued target-session bundle. The bundle binds
database identity, target schema, store scope, role identities, release epoch, and external fence
generation. Readers and writers receive physically distinct session capabilities; a reader is not
a wrapper around a writer-capable `Arc`.

One private PostgreSQL transaction module owns:

- session acquisition and exact-role selection;
- database/schema/store identity validation;
- pinned `search_path` and target ACL checks;
- per-transaction external-currentness validation;
- canonical advisory-lock ordering;
- commit/acknowledgement classification and SQLSTATE mapping.

Every write follows:

```text
current target permit
  -> READ COMMITTED transaction
  -> canonical target/run or target/configuration lock
  -> LockedWriteTx
  -> current append identity and head read
  -> classify or mutate
```

The `LockedWriteTx` typestate is required by all DML helpers, so reading a decision snapshot before
the lock is impossible through the internal API.

### EVM submission and wallet recovery

Configuration carries deployer-selected semantic transaction material only. Code-owned execution
contracts, provider routes, issuer identities, observation policy, candidate iteration, release
identity, and signing identity are qualified facts or content-addressed code contracts; an adapter
never echoes them from untrusted configuration.

A signing request carries the full public signing identity and expected account derivation.
Keystore qualification proves the requested account and public key belong to the same live key.
Signer-integrity violations are deterministic invalid evidence, not retriable backend
unavailability.

The operation has explicit pre-entry, entered-unknown, proved-rejected, and completed states. Once
submission may have crossed the external entry boundary, loss of acknowledgement remains
`EntryUnknown` unless producer evidence proves a narrower outcome.

Every retained activated candidate is examined in certified order on every recovery run before a
replacement is admitted. Candidate eligibility comes from provider-produced observations and
wallet status, never from the state that wants to replace the candidate. `SubmissionIntentId` binds
only nonce domain, authenticated issuer, and bounded caller token. A separate
`SubmissionSemanticsDigest` binds all behavior-affecting policy, including observation rounds and
candidate-expansion rules.

### Read authority, export, and replay

Store assembly returns distinct target-bound purpose readers. The application facade privately
pairs each reader with its same-purpose authorized call and never receives a generic verified-run
handle.

Export first discovers and authorizes the entire bounded dependency closure, including source runs,
without emitting output. Missing and denied dependencies produce the same reviewed redacted
failure. Serialization begins only after every dependency is authorized for the same target,
tenant, principal, and export purpose.

`mfm-replay` owns one strict portable format and validator. A bundle contains exact committed batch
envelopes, complete referenced objects, fact publication routes/frontiers, source-run relationships,
and explicit semantic and physical fixation. The offline verifier parses canonical bytes, checks
all content identities, and invokes the store's read-only entry point to the sole fold with a
concrete trust snapshot. It performs no live IO and invokes no operation callback.

## Commit sequence

The subjects below are normative and lower case. Tests, contract fixtures, documentation, and
deletions belong to the commit that changes their behavior. A checkpoint commit is made only after
an independent reviewer has evaluated the exact preceding implementation revision.

| Order | Commit subject | Primary scope |
| --- | --- | --- |
| 0 | `plan tt1 runtime history remediation` | This plan |
| 1 | `seal runtime history mutation and program verification` | Runtime/store/certify authority |
| 2 | `make safe failure settlement total` | Safe-failure language contract |
| 3 | `preserve structured values and lexical provenance` | Aggregate composition and provenance |
| 4 | `record kernel remediation checkpoint` | Independent kernel review evidence |
| 5 | `seal postgres store target authority` | Exact-target sessions, roles, transaction ownership |
| 6 | `classify postgres exact head races precisely` | Locking, bounds, and contention semantics |
| 7 | `bound postgres fact publication and query assurance` | Fact cost, SQLx, conformance, faults |
| 8 | `record postgres remediation checkpoint` | Independent PostgreSQL review evidence |
| 9 | `move decrypted keys without plaintext copies` | Keystore secret ownership |
| 10 | `bind evm submission semantics to the qualified signer` | Signer and deployment meaning |
| 11 | `preserve evm entry ambiguity and physical currentness` | External-entry and release proofs |
| 12 | `bound evm nonce values and wallet status cost` | Nonce domain and status projection |
| 13 | `make evm candidate recovery evidence driven` | Candidate iteration and recovery closure |
| 14 | `record evm remediation checkpoint` | Independent EVM/keystore review evidence |
| 15 | `authorize recursive export before serialization` | Recursive source-run export authority |
| 16 | `remove current candidate replay comparison` | Complete `compare_current` contract deletion |
| 17 | `make portable replay independently verifiable` | Replay ownership and portable format |
| 18 | `record application remediation checkpoint` | Final independent design review evidence |

## Detailed commit plan

### Commit 0: `plan tt1 runtime history remediation`

Add this file as the reviewed execution contract. Do not mix implementation into this commit.

Required checks:

- every ID in `PROBLEMS_TT1_IMPLRFC_RUNTIME.md` appears in the traceability table below;
- every path and task named here exists at the baseline or is explicitly marked as a new path;
- `git diff --check` passes.

### Commit 1: `seal runtime history mutation and program verification`

Closes: AUTH-01, AUTH-02, AUTH-05, APP-03. Starts QUALITY-01 and VERIFY-01.

This is one inseparable authority cutover. Splitting the dependency inversion, backend enclosure,
certified-program consumption, or production read/write assembly across commits would leave an
intermediate public route to production history.

Construction:

- Invert the kernel dependency direction so `mfm-runtime` no longer depends on `mfm-store` and
  `mfm-certify` no longer depends on `mfm-store`. Make `mfm-store` depend on `mfm-runtime`.
- Move concrete-store/replay integration cases out of Runtime's development dependency graph into
  store or workspace integration tests. Runtime unit tests use a caller-owned test port, so no
  development-dependency cycle weakens the production boundary.
- Move process bindings, access-bracket invocation, and prior-run scanner authority from
  `mfm-certify` into `mfm-runtime`. Leave only deterministic certification and concrete
  verification in `mfm-certify`.
- Define the minimal consumer-side `RuntimeHistoryPort` around semantic admission and continuation
  outcomes. Keep semantic commands and append attempts private to Runtime.
- Make application assembly pass one non-cloneable complete qualified registry into
  `mfm-store` production assembly. That one consuming call installs the concrete verification
  registry in the store adapter and the matching process registry in Runtime; application code
  cannot split, omit, substitute, or reuse either half.
- Implement the port in `mfm-store` with a private adapter that owns the backend and the only route
  to the fold. Expose production assembly that returns `Runtime` and purpose readers only.
- Replace the generic complete-history reader with sealed, target-bound public-read, trace, audit,
  replay, and export readers, each exposing only its exact projection.
- Make the app's `AuthorizedRunCall<P>` retain the approved grant and make each facade method accept
  only the call type matching its private purpose reader.
- Put the concrete `AdmissionVerificationRegistry` inside that adapter. Consume private-field
  `CertifiedProgram` values on cache miss and cache only by certified root identity.
- Derive `RunId` in the store through the authoritative `mfm.run-id-preimage.v1` contract from exact
  store scope, tenant scope, entry-point operation ID, and invocation identity. Do not reimplement
  the formula in app/runtime code or add fields that the annex deliberately excludes. Return the
  derived identifier in the admission result.
- Make the fold entry point take only store-private verified commands. Backend code continues to
  accept only validated batches and cannot construct semantic records.

Delete completely:

- public `StructuredRunHistoryWriter` and writer-returning `split`;
- public semantic proposal constructors and writer methods;
- Runtime APIs that expose `StructuredAppendAttempt`, mutation leases, or authorization tokens;
- `StructuredProgramVerifier`, `StoreProgramVerifier`, public `VerifiedProgramData` construction,
  `split_qualified_registry`, and every expanded-program/schema injection path;
- generic `StructuredRunHistoryReader::load_verified`, cloneable complete-history readers, and
  `AuthorizedRunCall` shapes that discard their approved grant;
- app-supplied run identifiers and all tests/fixtures that choose them;
- certify-to-store authorization and scanner coupling.

Module split:

- split certification by program verification, registry, predicates, and proof construction;
- split Runtime by port, admission, process/access, drive, and fault mapping;
- keep one store fold entry and divide its private implementation by admission, transition, access,
  facts, provenance, and closure validation.

Required proofs:

- compile-fail tests show app/live/unrelated crates cannot obtain a production writer, backend,
  semantic command, proposal constructor, mutation permit, append attempt, or Runtime port;
- compile-fail and policy tests show a public-read, trace, audit, replay, or export capability
  cannot load or project another purpose's data;
- a caller-implemented test port cannot be attached to a production backend;
- hostile reuse of an operation ID with substituted expansion or schemas fails certification;
- malformed certified roots cannot populate the cache;
- a complete registry is consumed exactly once, and missing, extra, duplicated, split, or
  mismatched verifier/process entries cannot assemble;
- identical contract preimages derive identical IDs, each of the four identity-bearing fields
  changes the ID, non-member admission material remains excluded, and callers cannot force an
  unrelated ID by supplying a digest;
- Pure, Read, Effect, fact, authorization, observation, ambiguity, stale-head, and root-close causal
  tests drive mutation only through Runtime;
- Cargo metadata proves the new acyclic dependency direction.

Documentation:

- update `docs/architecture.md` ownership/dependency diagrams and affected crate READMEs;
- update `docs/design.md` only where it still names a superseded public authority shape;
- define precisely when a registered authored program may be replaced through the support-envelope
  API and why that does not grant certified-root substitution;
- document every remaining public port/outcome item without exposing production mutation types.

Focused verification:

- Nix-shell package checks/tests for `mfm-certify`, `mfm-runtime`, `mfm-store`, and `mfm-app`;
- compile-fail/API-surface tests;
- `nix run .#run -- --task cargo-metadata-contract`.

Exit condition: repository-wide search finds no alternate semantic writer/verifier path, and a
production backend is reachable only from the private store-owned Runtime adapter.

### Commit 2: `make safe failure settlement total`

Closes: LANG-01. Continues VERIFY-01.

Construction:

- Replace the single unrestricted settlement callback with distinct returned-value and
  safe-failure callback types.
- Make the safe-failure callback return a success-only proposal whose type has no `Failure` or
  `InvalidEvidence` variant.
- Route every inhabited safe-failure value through that same typed callback. Remove all
  code-selected “reviewed corpus” or sample-list claims.
- Preserve adapter faults as no-append infrastructure outcomes. They must not be translated into a
  semantic failure record.
- Keep `Never` uninhabited and ensure it has no fabricated sample or fallback value.

Required proofs:

- compile-fail tests show a safe-failure callback cannot construct Failure or InvalidEvidence;
- property/table tests drive arbitrary valid safe-failure values through the success-only path;
- returned-value callbacks retain the full declared typed settlement behavior;
- an adapter fault during safe-failure settlement appends no semantic failure and resumes safely;
- qualification rejects mismatched returned/safe-failure callback contracts.

Documentation: replace sample-corpus language in the design, rustdoc, and fixtures with the typed
totality guarantee.

Focused verification: Nix-shell checks/tests for `mfm-program`, `mfm-certify`, `mfm-runtime`, and
`mfm-store`, including consuming-crate compile-fail tests.

Exit condition: the invalid safe-failure settlement state is unrepresentable and no exhaustive-test
claim depends on author-selected examples.

### Commit 3: `preserve structured values and lexical provenance`

Closes: LANG-02, LANG-03, LANG-04, LANG-05, PROV-01, PROV-02. Continues QUALITY-01 and VERIFY-01.

Construction:

- Introduce the sealed recursive `StructuredValue` algebra and use it uniformly at admission roots,
  state input/output, child success, Match result, policy proceed/success, fan-out lane, and
  operation root boundaries.
- Replace vector construction of `FanOutResults` with a non-empty head-plus-tail representation and
  strict decoding.
- Add one recursive `StructuredValueDefinition` for qualification closure. Recursively register
  retained schemas, lane contracts, join contracts, and nested definitions through one encoder.
- Generalize the store's existing value-contract validator to the recursive definition. Do not add
  aggregate-specific semantic proposals or append logic.
- Derive a child failure reference from the declared child failure exit and invocation boundary,
  never from its success input or a parent placeholder.
- Preserve selected Match-arm and fan-out-lane producer segments in `LexicalValueRef`. Aggregate
  data does not carry authority; certified slots and fold-derived references do. The retained
  reference binds group path, arm/lane key, ordinal, value/failure contract, and original source
  reference independently of payload bytes.
- Reject duplicate state, arm, lane, child, and other same-scope labels during qualification.
  Labels remain diagnostics.
- Assign stable ordinals during qualification and use ordinal tuples for canonical path ordering.
- Replace recursive normalization/validation walks over hostile input with an explicit bounded
  work stack. Enforce the same maximum nesting/node budget at construction, decoding,
  qualification, and replay.
- Replace the affected current hash/schema baseline and delete the old representation; do not add a
  compatibility decoder.

Module split:

- divide `mfm-program` structured code into value, state, block, Match, fan-out, policy, and
  operation modules;
- divide `mfm-spec` structured code into contracts, paths, normalization, program, and document
  modules;
- retain one public language and re-export only its intended entry points.

Required proofs:

- a state returns an aggregate and a later state consumes it;
- Match merges aggregate results while preserving the exact selected arm;
- child success returns an aggregate and child failure points at the declared failure exit;
- policy wrapping and operation roots accept aggregates;
- nested fan-outs preserve lane/join provenance through encode, fold, export, and replay;
- empty aggregates, duplicate labels, wrong lane tags, wrong schemas/contracts, missing recursive
  closure, fabricated refs, and excessive depth are rejected;
- `Never` cannot appear as a fan-out failure value;
- hostile maximum-depth and over-depth inputs complete without stack overflow;
- renaming diagnostic labels does not reorder paths, while changing qualified ordinals changes the
  canonical identity as specified.

Documentation: update the language grammar, path/provenance contract, canonical ordering rules,
depth limits, rustdoc examples, and affected schema fixtures in the same commit.

Focused verification: Nix-shell checks/tests for `mfm-program`, `mfm-program-derive`, `mfm-spec`,
`mfm-certify`, `mfm-runtime`, `mfm-store`, and replay fixtures affected by the reset.

Exit condition: all success boundaries compose over one algebra, aggregate emptiness and duplicate
labels are structurally rejected, and no recursive hostile-input walk remains.

### Commit 4: `record kernel remediation checkpoint`

An independent reviewer examines the exact revision after Commit 3 against AUTH-01/02/05, APP-03,
LANG-01..05, PROV-01/02, and the kernel portions of QUALITY-01/VERIFY-01.

Append a checkpoint to the new `TT1_RUNTIME_REMEDIATION_REVIEW.md` containing:

- reviewed revision and reviewer identity;
- commands and generated evidence reviewed;
- every requested proof with a pass/fail disposition;
- repository-wide searches for deleted authority surfaces;
- findings, their fixing revisions, and the final re-review revision;
- an explicit gate decision.

Do not create this commit while a blocker is open. Fix a blocker in the smallest owning
implementation commit, rerun focused verification, obtain re-review, then record the passed
checkpoint.

### Commit 5: `seal postgres store target authority`

Closes: AUTH-03, AUTH-04, STORE-02, STORE-07. Starts STORE-03.

Construction:

- Replace fixed cluster-wide application roles with exact-target owner, qualification, run-reader,
  run-writer, configuration-reader, and configuration-writer roles.
- Generate role/schema identifiers from validated target identities through one private naming
  function. Provision only the required target grants; sibling schemas remain ungranted and are
  exercised by explicit denial tests.
- Make production openers consume opaque deployment-issued session bundles. Remove all production
  inputs that accept `PgPool`, URLs, connection options, or a caller-provided fence.
- Delete application-side PostgreSQL connection construction. Embedding deployment code supplies
  the opaque bundle; missing deployment authority fails closed without a memory fallback.
- Split physical run-read, run-write, configuration-read, and configuration-write session
  capabilities. No shared writer-capable pool backs a reader.
- Verify database identity, schema identity, store scope, expected role OID/membership, release
  epoch, external fence generation, non-superuser status, no `BYPASSRLS`, no unintended
  inheritance, and exact grants before issuing a session.
- Require a fresh current target permit for every transaction. Copied or stale target credentials
  fail closed even when PostgreSQL ACLs would otherwise accept them. Compare the permit generation
  with the locked target-fence row inside the transaction before DML.
- Centralize transaction acquisition, role selection, `search_path`, target checks, lock helpers,
  commit classification, and SQLSTATE mapping in one private PostgreSQL transaction module.
- Make every DML function require a `LockedWriteTx` or the corresponding configuration typestate.
- Move the always-successful test fence and raw-pool fixtures into integration-test support that is
  not compiled into production under any ordinary feature.
- Replace the PostgreSQL schema baseline destructively and delete old role/opening paths.

Required proofs:

- a complete role-operation matrix covers run, fact, configuration, qualification, and schema DDL;
- each role is denied every sibling target in the same cluster;
- retained input pools/credentials cannot be used after qualification to bypass target authority;
- copied, expired, wrong-release, wrong-schema, wrong-database, and stale-generation bundles fail;
- read authority cannot perform DML and run authority cannot mutate configuration;
- no public API exposes a concrete backend, pool, writer session, test fence, or DML transaction;
- compile-fail/API tests cover each removed constructor;
- all run and configuration write paths are forced through the one transaction module.

Documentation:

- update `docs/architecture.md` with the target-session TCB and role separation;
- update store/deployment READMEs with provisioning and threat-boundary requirements;
- update the authoritative PostgreSQL schema and qualification contract;
- explicitly state that a holder of deployment root credentials is inside the TCB.

Focused verification:

- Nix-shell store/runtime package checks;
- PostgreSQL role and hostile-authority integration tests;
- `nix run .#run -- --task postgres-sqlx-check`;
- `nix run .#run -- --task recoverability-postgres-v1`.

Exit condition: ordinary code cannot obtain or retain PostgreSQL mutation authority outside an
exact-target, current transaction capability.

### Commit 6: `classify postgres exact head races precisely`

Closes: STORE-01, STORE-04. Continues STORE-03 and VERIFY-01.

Construction:

- Begin write transactions at `READ COMMITTED`, acquire canonical advisory locks before any
  decision-bearing head or append-identity read, and expose those reads only on `LockedWriteTx`.
- Use one classification function after the lock:
  - same append identity and same canonical bytes -> `ExistingSame`;
  - same append identity and different canonical bytes -> `AppendConflict`;
  - different append identities from the same predecessor -> one `NewlyCommitted` and one
    `StaleHead`.
- On unique/serialization/acknowledgement ambiguity, reacquire authority and re-read under the lock
  before classifying. Healthy contention never maps to `BackendUnavailable`.
- Introduce one store-owned bounded canonical append representation. It validates byte sizes,
  counts, depth, canonical JSON, absence of floats, hashes, and full object closure before either
  backend sees data.
- Make memory and PostgreSQL backends consume that identical representation. Keep database checks
  as defense in depth, not as an alternate contract.
- Remove backend-specific acceptance differences and old serialized-transaction retry heuristics.

Required proofs:

- deterministic multi-process barriers produce each exact-head outcome above;
- same-ID/different-bytes can never return `ExistingSame`;
- acknowledgement loss followed by retry returns the exact committed outcome;
- stale-head races do not return unavailable;
- genuine connection loss before classification remains a redacted unavailable error;
- memory and PostgreSQL accept and reject the same boundary corpus at every minimum/maximum and one
  step beyond;
- non-canonical JSON, floats, oversized frames/objects/batches, missing closure, wrong hashes, and
  predecessor discontinuity are rejected before DML;
- lock-order tests cover run, tenant/fact, and configuration keys and show no inversion.

Documentation: specify exact contention outcomes, lock order, canonical ingress ownership, and
shared bounds.

Focused verification: Nix-shell store tests plus cross-process PostgreSQL contention and
acknowledgement-loss suites.

Exit condition: all healthy races have domain outcomes, and both backends receive the same already
validated canonical append.

### Commit 7: `bound postgres fact publication and query assurance`

Closes: STORE-03, STORE-05, STORE-06. Continues QUALITY-01 and VERIFY-01.

Construction:

- Store the current per-run fact count, minimum/maximum publication coordinates, and frontier in
  the locked run head/projection row and update them atomically with publication. Enforce checked
  increments and per-append bounds without `COUNT`/`MIN`/`MAX` over lifetime history.
- Serve bounded fact scans from an indexed cursor/range contract; no query cost grows with the full
  run lifetime when the requested page/budget is fixed.
- Convert fixed SQL to checked `sqlx` macros and checked metadata. Put unavoidable dynamic SQL
  behind one reviewed builder/allowlist and make the verification task inventory that exact list.
  Regenerate the checked offline metadata in the same commit. Remove any documentation claim
  broader than the executable check.
- Build one backend-conformance suite around Runtime-observable outcomes. Run it against memory and
  PostgreSQL rather than testing two semantic implementations.
- Add PostgreSQL cross-process scheduling and stage-by-stage fault injection at: before lock, after
  lock, after decision read, after each DML class, before commit, after server commit/before
  acknowledgement, and retry.
- Assert rollback atomicity, exact retry classification, root-atomic close, fact frontier
  continuity, and no partial artifacts at every stage.
- Add or update Nixfied leaf tasks so SQL metadata, schema qualification, conformance, and hostile
  mutation probes are executable and accurately composed by the existing gates.

Required proofs:

- a long-history run publishes and scans a bounded fact page with a stable indexed query plan and
  no lifetime aggregate;
- concurrent publishers preserve exact count/frontier and append atomicity;
- every injected stage either leaves no append or leaves the one complete classifiable append;
- a fresh process resumes PostgreSQL history with no memory fallback;
- conformance covers all five record families, every transition, duplicate/conflict/stale
  outcomes, closure and bounds failures, and redacted backend faults;
- offline and online SQLx checks cover every fixed query and fail on an unowned raw query;
- the Nix task graph names what is actually checked.

Documentation: update `docs/build-and-verification.md`, the store README, schema comments, and task
descriptions with the exact assurance boundary.

Focused verification:

- `nix run .#run -- --task postgres-sqlx-offline-check`;
- `nix run .#run -- --task postgres-sqlx-check`;
- `nix run .#run -- --task recoverability-postgres-v1`;
- the new/updated conformance and fault-injection leaf task.

Exit condition: fact work is bounded by the requested append/page, SQL assurance is executable and
truthfully described, and the two backends pass one semantic/fault contract.

### Commit 8: `record postgres remediation checkpoint`

An independent reviewer examines the exact revision after Commit 7 against AUTH-03/04,
STORE-01..07, and related QUALITY-01/VERIFY-01 requirements. Record the revision, role/ACL matrix,
cross-process traces, fault-stage matrix, SQL inventory, query-plan evidence, findings and fixing
revisions, re-review, and gate decision in `TT1_RUNTIME_REMEDIATION_REVIEW.md`.

No EVM or application work begins while a PostgreSQL authority, atomicity, contention, or
cross-process blocker remains open.

### Commit 9: `move decrypted keys without plaintext copies`

Closes: SEC-01. Continues VERIFY-01.

Construction:

- Change the decrypt path to write into or return an owned zeroizing protected allocation, then
  transfer ownership of that same allocation into `SecureKey`.
- Remove dereference-copy construction such as `SecureKey::new(*key_bytes)` and any API that takes a
  plaintext array by value.
- Make protected-buffer ownership and zeroization on every error path explicit in private types.
  Do not make `Keystore` `Send` or `Sync`.
- Keep bounded-input validation, AAD anti-swap binding, constant-time identity comparison,
  fail-closed format handling, and redacted diagnostics.

Required proofs:

- the production call graph has no ordinary `[u8; 32]` plaintext copy between decryption and
  `SecureKey`;
- an instrumented test witnesses decrypted-key allocation ownership transfer and cleanup on
  success and every failure path, rather than observing only unlock-buffer cleanup;
- tampered ciphertext, AAD swap, wrong account/public identity, corrupt format, and oversized input
  fail closed without secret-bearing output or errors;
- compile/API tests prevent reintroduction of a plaintext-taking `SecureKey` constructor;
- sentinel credentials, key material, and paths remain absent from qualification, handoff, signing,
  `Debug`, and error rendering;
- existing zeroization and non-`Send`/non-`Sync` assertions remain.

Documentation: document the ownership invariant and the security rationale, not implementation
history.

Focused verification: `nix develop -c cargo test -p mfm-keystore` plus the affected signer package
tests.

Exit condition: plaintext key material moves by protected allocation ownership and cannot be
copied through the public or internal constructor API.

### Commit 10: `bind evm submission semantics to the qualified signer`

Closes: EVM-01, EVM-02, EVM-07. Continues QUALITY-01 and VERIFY-01.

Construction:

- Separate `AccountAddress`, full `PublicSigningIdentity`, and secret signing authority types.
  Qualification proves address derivation from the exact public key and binds both to the live
  keystore entry.
- Have the app's sealed EVM deployment operation build the signing request from qualified
  deployment semantics. Do not interpret an account identifier as public-key material.
- Reduce configuration to deployer-selected semantic transaction inputs. Derive provider route,
  issuer, code contract, intent references, candidate family, and release identity from qualified
  registries and content-addressed contracts.
- Recompute the sealed deployment semantics for every resolved configuration revision and require
  exact signer, signing profile, broadcast capability, submission contract, expansion contract,
  and terminal assurance policy identity before history or wallet mutation.
- Require the live adapter to return producer evidence binding route, issuer, request identity,
  full public signing identity, verified signed-hash/transaction identity, and release. Raw
  signature bytes remain transient and are not retained.
- Verify signature and address binding before accepting evidence. Map mismatch/corruption to the
  reviewed deterministic invalid-evidence or typed integrity outcome, never `SafeFailure`; reserve
  unavailable for genuine capability/transport absence.
- Delete echo-based qualification, cached/prewarmed signer assumptions, and the old persisted
  deployment baseline.

Required proofs:

- a fresh process, fresh unique real keystore, and empty store can complete the production signing
  path without fixture cache state;
- wrong account, public key, signature, issuer, route, release, request, and intent bindings are
  independently rejected;
- untrusted configuration cannot choose or echo a qualified reference;
- signer integrity violations are not retried as backend unavailability;
- no secret, signature preimage, key bytes, or internal diagnostic enters events, facts, artifacts,
  snapshots, CLI/REST output, or errors;
- the operation remains usable as a library without CLI construction.

Documentation: update EVM operation contracts, adapter README, configuration schema, public output
schema, and redacted error mapping.

Focused verification: Nix-shell checks/tests for keystore, EVM adapters, app, CLI, and REST, plus
the fresh-keystore end-to-end case.

Exit condition: production submission semantics are derived from a qualified live signer and
cannot be assembled from caller-echoed identities.

### Commit 11: `preserve evm entry ambiguity and physical currentness`

Closes: EVM-05, EVM-11 and the ledger's additional provider-frame and per-access-currentness
findings. Continues VERIFY-01.

Construction:

- Model submission progress with private phase types: pre-entry, entry-may-have-occurred,
  proved-rejected, and completed. Only pre-entry failures or explicit producer rejection proof can
  construct `EntryRejected`.
- Once request bytes may have reached the provider, timeout, disconnect, decode failure,
  acknowledgement loss, and process crash produce/resume `EntryUnknown`.
- Replace process-local release checks with a consumed physical-release permit on every provider,
  signer, and protected storage access. Bind it to the exact target, provider route, signer
  release, code release, epoch, and fence generation.
- Revalidate currentness immediately before the protected side effect and when opening every
  protected transaction; reject a release change between accesses.
- Require each initial and successor release target to equal the actual incarnation exposed by the
  concrete wallet/provider authority used for that invocation.
- Introduce a bounded provider-proof byte type at ingress. Account for the complete canonical
  record envelope so an accepted proof can never exceed the store frame limit after encoding.
- Qualify route/reservation semantics, not merely syntax: exact chain, account, provider,
  candidate, release, and pending-entry contract must match the request.

Required proofs:

- accepted-by-provider then timeout/disconnect/decode-error is `EntryUnknown`, never rejected;
- an explicit pre-entry validation rejection remains rejected;
- every supported transport has a closed reviewed table for “already known”, authenticated
  non-entry rejection, generic HTTP failure, and JSON-RPC failure; unknown cases preserve
  ambiguity;
- crashes at each boundary resume without duplicate unsafe reinvocation;
- stale, mismatched, or replaced physical release fails on the next access even in a long-lived
  process;
- a release change between qualification and submission prevents entry;
- maximum provider proof plus envelope is accepted and one byte beyond is rejected before append;
- syntactically valid but semantically wrong pending/route reservations are rejected.

Documentation: specify the entry boundary, ambiguity table, exact release binding, permit lifetime,
and provider-proof size calculation.

Focused verification: Nix-shell EVM/runtime/store tests and cross-process provider fault fixtures.

Exit condition: post-entry uncertainty cannot be narrowed without producer proof, and no protected
access relies on startup-time release currentness.

### Commit 12: `bound evm nonce values and wallet status cost`

Closes: EVM-06, EVM-10. Continues STORE-03 and VERIFY-01.

Construction:

- Introduce a validated transaction-nonce newtype whose values always have a representable checked
  successor. Reject `u64::MAX` at every decode/configuration/provider/storage boundary.
- Make nonce reservation/advance APIs consume and return the validated type. Remove sentinel and
  wrapping arithmetic.
- Add matching database constraints and canonical decode validation. Invalid values perform no
  mutation.
- Preserve the append-only wallet audit stream while adding one current, lock-protected wallet
  status/head projection maintained atomically with each mutation.
- Make status and mutation load the bounded current projection and an explicitly bounded evidence
  window, never the wallet's lifetime lineage or one query per historical candidate.
- Add exact indexes and query-plan assertions for target/account/current-candidate access.

Required proofs:

- zero, normal maximum, `u64::MAX - 1`, and rejected `u64::MAX` cases agree in memory,
  PostgreSQL, configuration, provider evidence, export, and replay;
- rejected maximum leaves no reservation, fact, status, or partial append;
- concurrent reservations advance exactly once under the target lock;
- a very long wallet lineage has constant/bounded query count and stable indexed query plans for
  status and mutation;
- the current projection always reconciles with a full audit fold in test-only verification;
- fault injection proves the audit append and current projection are atomic.

Documentation: state the nonce domain and the audit-versus-current projection ownership.

Focused verification:

- Nix-shell wallet/runtime/store tests;
- `nix run .#run -- --task wallet-nonce-postgres-storage-qualification`;
- `nix run .#run -- --task wallet-nonce-postgres-qualification`.

Exit condition: every admitted nonce has a successor and wallet status cost is bounded independently
of lineage length.

### Commit 13: `make evm candidate recovery evidence driven`

Closes: EVM-03, EVM-04, EVM-08, EVM-09. Continues QUALITY-01 and VERIFY-01.

Construction:

- Freeze the ordered set of all retained activated candidates into the certified operation
  invocation. On every recovery run, iterate every candidate in that order before replacement is
  considered; no “current candidate” shortcut may skip an activated predecessor.
- Represent candidate status/eligibility as producer-bound provider and wallet evidence. A state
  that proposes a replacement cannot also certify the evidence permitting it.
- Make the replacement activation permit affine and consumptive. It binds the current reservation,
  run, predecessor candidate, next ordinal, evidence identities, policy, and freshness frontier;
  raw clonable reserve/activate/complete request products are not authority.
- Make replacement admission require complete evidence for every earlier candidate: completed,
  proved rejected, or still ambiguous according to the certified policy. Preserve ambiguous
  candidates rather than silently replacing them.
- Keep `SubmissionIntentId` stable over nonce domain, authenticated issuer, and bounded caller
  token. Put every behavior-affecting value into a separate canonical
  `SubmissionSemanticsDigest`: observation rounds, backoff/refresh policy where semantic,
  candidate-expansion rules, ordering, provider and release contracts, and algorithm version.
- Make completion retain a content-addressed public closure containing every candidate, request,
  public signing identity, observation/status item, route, release, predecessor, decision, and
  final result needed for later recovery and offline verification. The output references this
  object; it is not a digest with missing preimages.
- Have later runs project the retained closure through authorized source-run reads rather than
  reconstructing it from ambient provider state.
- Delete self-certified eligibility, incomplete completion payloads, and old intent identities.

Required proofs:

- later runs visit each activated candidate after crashes at every candidate/submission/observation
  boundary;
- a missing, ambiguous, or contradictory earlier candidate blocks unsafe replacement;
- a state-authored replacement claim without producer evidence is rejected;
- with the same `SubmissionIntentId`, changing observation rounds or any branching/expansion policy
  changes `SubmissionSemanticsDigest` and produces a permanent conflict;
- the full completion closure rehashes to its reference and contains every public preimage;
- a fresh process can recover from the closure without invoking a completed effect or relying on
  ambient memory;
- tampered candidate order, omitted candidate, substituted evidence, incomplete closure, and wrong
  source run are rejected;
- long candidate histories remain within explicit per-run bounds.

Documentation: update the certified EVM state machine, candidate ordering, evidence producers,
intent preimage, completion schema, recovery examples, and persisted baselines.

Focused verification:

- Nix-shell EVM/app/runtime/store/replay tests;
- `nix run .#run -- --task evm-postgres-submission-qualification`.

Exit condition: replacement is justified only by independent retained evidence and the complete
public recovery preimage survives process and run boundaries.

### Commit 14: `record evm remediation checkpoint`

An independent reviewer examines the exact revision after Commit 13 against EVM-01..11, SEC-01,
the extra provider-frame/currentness findings, and related QUALITY-01/VERIFY-01 requirements.
Record the revision, fresh-keystore trace, secret-surface audit, phase/fault matrix, release-permit
tests, nonce/status query evidence, candidate recovery matrix, findings/fixes/re-review, and gate
decision in `TT1_RUNTIME_REMEDIATION_REVIEW.md`.

No export/replay completion work begins with an open signing-integrity, secret-handling,
external-entry, nonce, or recovery blocker.

### Commit 15: `authorize recursive export before serialization`

Closes: APP-01. Continues VERIFY-01.

Construction:

- Use the sealed export reader and retained grant introduced by Commit 1. Its authorization permit
  carries target, tenant/principal, export purpose, scope, and current physical fixation; app code
  receives no generic `VerifiedRun` or raw store read handle.
- Make export accept that authorized run call rather than a caller-selected generic run reference.
- Split export into two phases. Phase one traverses the bounded source/dependency graph using
  identifier-only internal metadata, detects cycles/over-budget graphs, and authorizes every exact
  dependency. Phase two reads and serializes only after phase one succeeds.
- Require the same target/tenant/principal/export purpose for root and recursively referenced
  source runs unless an explicit design-owned cross-target grant exists; this plan introduces no
  such grant.
- Return one reviewed redacted error for missing, denied, wrong-target, stale-fixation, and
  otherwise inaccessible dependencies. Emit zero bytes and no partial artifact on failure.
- Keep dependency identifiers and authorization diagnostics out of public errors.

Required proofs:

- compile-fail tests show app code cannot obtain a generic reader, change a reader's purpose, or
  use a status/replay reader for export;
- direct, multi-hop, shared, cyclic, too-deep, missing, denied, wrong-tenant, wrong-target, and
  stale source graphs are covered;
- every denied case emits exactly zero bytes/artifacts and the same public error contract;
- authorization occurs before object or record content is read for serialization;
- allowed recursive export is deterministic regardless of traversal encounter order;
- purpose permits cannot be retained across physical-fixation invalidation.

Documentation: update app, CLI, REST, store, and authorization contracts with purpose-specific
inputs and the two-phase export rule.

Focused verification: Nix-shell checks/tests for `mfm-store`, `mfm-app`, CLI, REST, and
authorization integrations, including consuming-crate compile-fail tests.

Exit condition: read authority is least-purpose, and no export byte can be emitted before the full
source closure is authorized.

### Commit 16: `remove current candidate replay comparison`

Closes: APP-02. Continues VERIFY-01.

Construction:

- Delete `compare_current` as a capability, not as an unavailable implementation.
- Remove its replay-mode and request/result variants, application dispatch, candidate/configuration
  resolution, authorization branch, CLI flag/value, REST wire value and route behavior, error
  codes, generated schema members, annex entries, fixtures, documentation, and tests in this one
  cutover.
- Retain recorded verification unchanged.
- Retain reproduction as its separately selected explicit unavailable result. Do not route it
  through comparison and do not introduce a candidate callback, registry lookup, compatibility
  alias, ignored field, or fallback evaluator.
- Regenerate the current transport and annex contracts and delete their superseded baselines.

Required proofs:

- repository-wide symbol, string, generated-contract, CLI-help, and OpenAPI/REST searches find no
  comparison mode or wire value;
- old comparison requests fail ordinary current-schema/CLI parsing rather than returning a
  compatibility-shaped unavailable result;
- recorded verification remains callback-free and produces the same current result;
- reproduction remains explicitly unavailable and accepts no hidden candidate input.

Documentation: remove comparison claims and examples from design, replay/app/CLI/REST READMEs, and
the recoverability annex in the same commit.

Focused verification: Nix-shell replay/app/CLI/REST checks, parser and generated-contract tests, and
the affected recoverability corpus drift check.

Exit condition: the public contract exposes only implemented recorded verification and the
explicitly specified reproduction result; no comparison-shaped surface remains.

### Commit 17: `make portable replay independently verifiable`

Closes: REPLAY-01, REPLAY-02, REPLAY-03. Completes QUALITY-01 and VERIFY-01.

Construction:

- Move the portable envelope, encoder/decoder, canonical validation, digest rules, and export
  validator from `mfm-app` into `mfm-replay`. Delete the duplicate app structs and validator.
- Define one strict current format containing:
  - exact canonical committed-batch envelopes, predecessor/head linkage, append identity,
    candidate and commit digests, writer epoch, and atomic grouping;
  - all records in an atomic batch whenever any member is selected;
  - the adjacent `RunClosed` record required by semantic selection;
  - complete certified program/schema, manifest, snapshot, fact, output, EVM closure, and other
    referenced objects;
  - per-append object membership, fact coordinates/routes/frontiers, and source-run relationships;
  - explicit semantic fixation and physical target/release fixation;
  - a canonical top-level digest over every required member.
- Parse through bounded byte/count/depth types before allocating the full bundle, using limits owned
  by the annex and shared with export.
- Give `mfm-store` one read-only replay-verification entry point to the existing sole fold. It
  accepts already parsed exact evidence and a concrete callback-free trust snapshot and returns
  verified projections; it cannot append or expose mutation authority.
- Make `mfm-replay` validate canonical encoding, digest, object closure, predecessor continuity,
  batch atomicity, record order, certified roots, fact routes/frontiers, source relationships,
  semantic fixation, and physical fixation before invoking that fold.
- Make offline verification use only bundle bytes and the explicit trust snapshot. It performs no
  network/filesystem access, store lookup, provider call, signer call, operation callback, or
  current application evaluation.
- Add the portable request/result/media schema to the authoritative recoverability annex and
  generated contract corpus. Generate Rust and transport fixtures from the same schema owner.
- Replace the portable-format baseline and delete all old readers/fixtures.

Required proofs:

- an isolated verifier with no store/provider access accepts a valid export and reproduces the same
  verified projection as online verification;
- byte tampering, non-canonical encoding, top-level digest change, truncation, record reordering,
  batch slicing, false predecessor/head, missing/extra object, wrong object hash, omitted
  `RunClosed`, false fact frontier/route, wrong source relationship, and fixation substitution each
  fail;
- oversized bytes/counts/depth fail at the bounded decoder without unbounded allocation or stack
  recursion;
- semantic export of any record includes its entire committed batch;
- the exported EVM completion object exposes every public preimage required to rehash it;
- generated Rust/CLI/REST schemas and annex corpus agree byte-for-byte on the current version;
- recorded verification remains callback-free and reproduction remains explicitly unavailable.

Documentation:

- update `docs/design.md` replay/export contracts and generated annex ownership;
- update `docs/architecture.md` to make `mfm-replay` the format/projection owner;
- update replay/app/CLI/REST READMEs and examples;
- document trust snapshot contents and distinguish semantic from physical verification.

Focused verification:

- Nix-shell checks/tests for `mfm-replay`, `mfm-store`, `mfm-app`, CLI, REST, and generated-contract
  tooling;
- recoverability corpus generation/check task;
- `nix run .#run -- --task cargo-metadata-contract`.

Exit condition: a third party can verify the portable artifact without MFM's live store or
callbacks, and no duplicate portable-format surface remains.

### Commit 18: `record application remediation checkpoint`

An independent reviewer examines the exact revision after Commit 17 against AUTH-05, APP-01..03,
REPLAY-01..03, all cross-layer QUALITY-01/VERIFY-01 requirements, and the preservation list at the
top of this plan.

Record in `TT1_RUNTIME_REMEDIATION_REVIEW.md`:

- exact reviewed revision and reviewer;
- full problem-to-proof audit;
- export authorization and portable tamper matrices;
- repository-wide deletion searches;
- generated-contract and documentation consistency evidence;
- all findings and fixing/re-review revisions;
- a production-claim table distinguishing real production proofs, test-only fixtures, deliberately
  unavailable behavior, and authority supplied by the external deployment TCB;
- the final `Material uncertainties` assessment, using `none` only if no material uncertainty
  remains;
- the final “ready for merge verification” decision.

This checkpoint is evidence, not a substitute for the final executable gate.

## Problem traceability

| Problem | Owning commit(s) | Required proof |
| --- | --- | --- |
| AUTH-01 | 1 | No public production writer/proposal/attempt; compile-fail boundary |
| AUTH-02 | 1 | Concrete registry and opaque certified program; substitution rejection |
| AUTH-03 | 5 | No retained pool/raw credentials/backend; hostile retained-input test |
| AUTH-04 | 5 | Exact-target role/ACL matrix and sibling-target denial |
| AUTH-05 | 1 | Sealed purpose readers and cross-purpose compile failures |
| LANG-01 | 2 | Success-only safe-failure settlement for every valid value |
| LANG-02 | 3 | Child failure ref derives from the declared failure exit |
| LANG-03 | 3 | Aggregates compose at every success/input boundary |
| LANG-04 | 3 | Duplicate labels rejected during qualification |
| LANG-05 | 3 | Qualified ordinals, not labels, define canonical order |
| PROV-01 | 3 | Match-arm and fan-out-lane provenance survive fold/export/replay |
| PROV-02 | 3 | Iterative bounded normalization rejects hostile depth safely |
| STORE-01 | 6 | Cross-process exact-head races yield domain outcomes |
| STORE-02 | 5 | Production cannot compile an always-successful test fence |
| STORE-03 | 5, 6, 7 | Shared conformance, cross-process schedules, staged fault injection |
| STORE-04 | 6 | One bounded canonical append contract for both backends |
| STORE-05 | 7 | Checked fixed SQL plus executable dynamic-query allowlist |
| STORE-06 | 7 | Atomic fact frontier/count and bounded indexed scans |
| STORE-07 | 5 | One private PostgreSQL transaction authority |
| EVM-01 | 10 | Fresh real-keystore production signing end to end |
| EVM-02 | 10 | Qualified code-owned semantics; no adapter echo |
| EVM-03 | 13 | Every activated candidate visited on later runs |
| EVM-04 | 13 | Replacement requires independent producer evidence |
| EVM-05 | 11 | Post-entry faults remain EntryUnknown absent rejection proof |
| EVM-06 | 12 | Valid nonce newtype rejects `u64::MAX` without mutation |
| EVM-07 | 10 | Signer-integrity faults are not backend unavailability |
| EVM-08 | 13 | `SubmissionSemanticsDigest` binds all behavior-affecting policy |
| EVM-09 | 13, 17 | Complete public recovery closure retained and exported |
| EVM-10 | 12 | Bounded current status projection and query-count proof |
| EVM-11 | 11 | Exact physical release permit checked per access |
| APP-01 | 15 | Full recursive authorization before zero-or-complete export |
| APP-02 | 16 | `compare_current` deleted from every surface |
| APP-03 | 1 | Store derives run ID from the complete frozen preimage |
| REPLAY-01 | 17 | Independent exact-batch/object/fixation verification |
| REPLAY-02 | 17 | `mfm-replay` owns the only portable format |
| REPLAY-03 | 17 | Authoritative annex and generated corpus own the schema |
| SEC-01 | 9 | Protected allocation ownership transfer with no plaintext copy API |
| QUALITY-01 | 1, 3, 5, 7, 10, 13, 17 | Responsibility-based module splits without parallel logic |
| VERIFY-01 | 1-3, 5-7, 9-13, 15-17 | Boundary, hostile, fault, compile-fail, and E2E proofs |
| PROCESS-01 | 4, 8, 14, 18 | Independent revision-pinned checkpoint evidence in branch |

The extra ledger findings are owned as follows:

- provider evidence larger than the store frame after envelope encoding: Commit 11;
- reservation/route qualification that checks syntax but not meaning: Commits 10 and 11;
- process-local rather than per-access physical currentness: Commit 11.

## Required deletion audit

The final reviewer must confirm absence of all superseded paths, not merely that callers stopped
using them:

- graph/executor compatibility code remains absent;
- public structured run-history writer, writer split, semantic proposal constructors, append
  attempts, mutation permits, caller-supplied run IDs, verifier traits, public verified-program
  construction, and expanded program injection;
- certifier-owned live invocation/store tokens and the old runtime-to-store dependency;
- pool-taking or combined-role PostgreSQL openers, public concrete backends, global application
  roles, duplicated transaction setup, and feature-gated production test fences;
- scalar-only success bounds, empty aggregate constructors, label-based ordering, recursive hostile
  normalization, and flattened Match/fan-out provenance;
- keystore APIs accepting ordinary plaintext key arrays;
- configuration-echoed EVM authority, self-certified replacement, process-local release checks,
  sentinel nonce arithmetic, lifetime/N+1 wallet status loads, and incomplete completion payloads;
- generic application run readers, serialize-before-authorize export, app-owned portable structs,
  and every `compare_current` symbol or wire value;
- all superseded persisted schemas, SQL migrations/baselines, media versions, generated fixtures,
  and compatibility readers.

Repository-wide negative checks become executable tests or inventory tasks where practical; a
review note containing only a manual search is not enough for an authority surface that can recur.

## Verification strategy

Verification is scope-driven and follows `docs/build-and-verification.md`. Each implementation
commit runs the narrow commands named in its section. A commit does not run broad gates merely
because it is about to be created.

At the four checkpoints:

- run the narrow package, compile-fail, generated-contract, PostgreSQL, and integration tasks needed
  by the affected boundary;
- for every regression, retain evidence that the assertion fails against baseline `31aced23a` (or
  that the removed API/contract exists there) and passes on the remediation revision;
- preserve machine-readable fault matrices, role matrices, query plans, and tamper corpora under
  test fixtures or generated evidence where stable;
- have an independent reviewer inspect the exact revision, not an uncommitted successor.

On the final reviewed revision:

1. Run `nix run .#model-check` because the executable verification graph changes.
2. Run targeted commands only for diagnosis if model admission or a leaf task fails.
3. Run `nix run .#ci` once as the final merge-readiness gate.
4. Run `git diff --check` and the documentation link/generated-contract consistency checks.

Do not run `.#check`, `.#test`, and `.#test-db` immediately before `.#ci` on the same tree;
`.#ci` already composes them.

## Completion criteria

The remediation is complete only when:

- all eighteen post-plan commits/checkpoints are present in order or their final history preserves
  the same logical boundaries and lower-case subjects;
- every row in the traceability table has its required executable proof;
- `TT1_RUNTIME_REMEDIATION_REVIEW.md` contains four passed, revision-pinned independent reviews;
- authoritative design, architecture, crate/transport documentation, generated contracts, schema
  baselines, and code describe one current design;
- the final review distinguishes real production paths, test-only support, explicit unavailable
  contracts, and externally supplied deployment authority, and reports material uncertainties;
- all required-deletion searches pass;
- no secret-bearing surface or redaction regression is present;
- the final Nixfied CI gate passes after the last checkpoint, with no code or contract change after
  the implementation revision named by that checkpoint.
