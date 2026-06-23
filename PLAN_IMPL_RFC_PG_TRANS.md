# Implementation Plan: Postgres Transition Store Refactor

Status: implementation planning guide

Source RFC: [`RFC_REFAC_PG_TRANS.md`](RFC_REFAC_PG_TRANS.md)

## Non-Negotiables

This plan is for a breaking dev-branch cutover.

- Do not maintain backward compatibility.
- All breaking changes are allowed.
- Do not create fallbacks, compatibility shims, legacy feature flags, aliases, or hidden old paths.
- Delete old code when its replacement lands. If needed, recover old code from git history.
- Do not defer deletion of replaced production surfaces to a late cleanup commit. The commit that
  introduces a target replacement must remove the old production path, alias, selector, flag, or
  dependency it replaces.
- Divide work per commit before executing.
- If a design decision is unclear, spawn an architect-agent to review it. Pass these same
  non-negotiables to the agent so everyone stays aligned.

Every implementation PR/branch should include an explicit commit plan before coding. Each commit
should be reviewable, have a narrow purpose, and either compile on its own or clearly state why it
is part of an agreed short-lived compile-breaking sequence. Prefer compile-clean commits.

Commits below are a reviewable execution stack, not permission to merge contradictory intermediate
states. Any commit that would make `docs/design.md`, runtime authority, storage behavior, public
APIs, or binary wiring disagree must stay in an unmerged stack until the matching replacement and
deletion commits are present.

This is not a storage-only refactor, but it does not have to land as one monolithic merge. It
decomposes into three separable, individually coherent cutovers that land in dependency order, each
with no compatibility shims:

1. Append/artifact cutover: `PreparedCommitBundle`, Postgres artifact blob/evidence authority,
   filesystem artifact store removal, the append-only authority schema, and the constraint/payload
   simplifications. Small artifact bytes may be inserted in the append transaction; large immutable
   blobs may be content-addressed in a short preceding transaction and then verified by the append
   transaction before authority evidence/admission/event rows commit.
2. Resource-lane cutover: the pre-invocation lane lifecycle, per-lane admission, `lane_id`-keyed
   lane-transition authority, the no-deadlock single-claim invariant, and the `docs/saga.md`
   resource-claim rewrite.
3. Observation cutover: the read-model layer, `run_commit_log` epoch-tagged watch cursors, the shared
   CLI/REST run observation API, and the `typed_` -> target naming baseline.

Within each cutover do not merge a partial state where runtime authority, storage behavior, public
APIs, and docs disagree. Prefer three landable cutovers over one big-bang merge so each new primitive
(bundle append, lanes, watch cursors) is validated independently.

The numbered commits below are implementation slices, not merge boundaries. If engineers execute the
work as three PRs, move any resource-lane bullets into the resource-lane cutover and any
read-model/cursor/API/global-name bullets into the observation/naming cutover. If engineers execute
the numbered list as one linear branch, keep it unmerged until the applicable cutover gate below is
complete.

Cutover gates:

- Append/artifact gate: plan-only durable append is gone; artifact bytes/evidence authority is in
  Postgres; authority evidence/admission/event rows and artifact bytes are append-transaction-bound;
  no production blob precommit or orphan-sweep path exists; filesystem artifact production paths are
  deleted; logical-key admission folds `run_events`; strict loads verify hashes from canonical bytes.
- Resource-lane gate: `docs/saga.md`, runtime lifecycle, capability resolvers, store admission,
  Postgres lane schema, strict load, and recovery all describe the same pre-invocation
  `ResourceLaneClaimed` lifecycle with no global lane locks and no terminal failure from ordinary
  contention.
- Observation/naming gate: read models, `run_commit_log`, epoch-tagged opaque cursors, shared
  `read_run_observations`, CLI/REST list-watch contracts, and the final `typed_`/`Typed*` naming
  baseline land together. Do not merge a repository-wide old-name cleanup before this gate unless the
  whole observation/naming replacement is in the same merge unit.

## Target Outcome

The finished refactor has one production persistence story:

```text
Postgres authority tables
  + SQLx-only storage crate access
  + append-only domain rows
  + transaction-bound artifact evidence/admissions/events
  + Postgres-owned rebuildable read models
  + one app API exposed by CLI/REST
  + no production filesystem/in-memory stores
```

The implementation is complete only when:

- production app/CLI/REST cannot select filesystem or in-memory storage
- artifact bytes, artifact evidence, run/commit admissions, and committed event rows are inserted in
  the same Postgres transaction
- plan-only append is gone from production
- old `typed_*` schema surfaces are gone from the final storage baseline
- old storage type names are not kept as compatibility aliases
- CLI/REST do not depend directly on `sqlx`, Postgres storage crates, or filesystem stores
- `docs/design.md` and `docs/architecture.md` describe the new source of truth

## Coordination Model

Use three workstreams, but land commits in dependency order.

- Kernel/runtime authority: `crates/kernel/store`, `crates/kernel/runtime`, `crates/kernel/replay`,
  capability contracts, and in-memory test helpers.
- Postgres storage: Postgres schema, SQLx queries, artifact bytes/evidence, append-owned read
  models, private read-only proof helpers, and strict load verification.
- App/API/binary boundaries: `crates/app`, `bin/cli`, `bin/rest-api`, docs, cargo metadata, and
  architecture tests.

Architect-agent escalation is expected when a choice affects authority boundaries, append
semantics, read-model correctness, or crate ownership. The prompt to the architect-agent must say:

```text
We are implementing RFC_REFAC_PG_TRANS.md. This is a breaking dev-branch cutover.
Do not preserve backward compatibility. Do not create fallbacks. Old code must be deleted.
Recommend the cleanest target architecture and commit-sized implementation steps.
```

## Commit Plan

### Commit 1: update authoritative docs first

Scope:

- Draft the `docs/design.md` and `docs/architecture.md` changes first so engineers share the target
  contract while coding.
- Do not merge these authoritative doc changes by themselves while the implementation still
  contradicts them. They are a merge gate for the cutover stack, not standalone truth before code
  catches up.
- Update `docs/design.md` with the new Postgres-only production persistence contract in the same
  merge unit as the storage/runtime/app/API cutover.
- Update `docs/architecture.md` with storage boundaries, SQLx ownership, artifact authority
  surfaces, and CLI/REST dependency rules in the same merge unit as the dependency rewiring.
- Document that artifact `BYTEA` safety comes from typed/signer/keystore/transient adapter
  boundaries, not storage-side secret scanning.
- Document resource lanes as capability-declared, runtime-preflight-resolved authority acquired
  before live IO and enforced by the store without global locks.
- Document lane-local transition authority, store-assigned fencing tokens, and
  `ResourceLaneClaimBlocked` as a parked-attempt outcome that cannot itself authorize terminal
  failure.
- Document the v1 lane liveness contract: capped exponential backoff with jitter, wakeups from
  `LISTEN/NOTIFY` on lane release, a periodic floor poll, and fairness/queueing still deferred.
- Document resolver purity and placement: lane resolvers may use certified config, typed inputs,
  non-secret binding metadata, side-effect identity, and already materialized non-secret artifacts,
  but not live transports, signers, keystores, clocks, storage, app orchestration, CLI, or REST.
- Rewrite the `docs/saga.md` "Resource Claims" section for the pre-invocation `ResourceLaneClaimed`
  lifecycle, the claim-kind-to-lane mapping (`Exclusive` only), and the no-deadlock invariant; this
  lands in the resource-lane cutover merge unit (a planned-change callout already points to the RFC).
- Document the dedicated-MFM-database requirement driven by cluster-wide `xmin` coupling.
- Document the read-your-writes caveat (list/watch lag the sealed frontier; strict status is
  immediate) and that restore/clone procedures are destructive with respect to issued cursors until a
  real epoch-reseed design exists.
- Document that list mode is capped by `limit` in v1 and is not separately paginated; any future list
  page cursor must be separate from the watch cursor.
- Document the cursor key contract if sealed stateless cursors are used: key id, durable key source
  across app/CLI/REST restarts, accepted old-key rotation window, and stable unknown/missing/retired
  key errors.
- Document rollback as code rollback plus database/schema reset to that branch's expected baseline;
  there is no downgrade migration.
- Update the storage README/runbook with the destructive fresh-database cutover stance.
- Mark filesystem artifact storage as removed from production architecture.
- Mark in-memory storage as test-only.
- Document the destructive dev-branch cutover posture.
- Create or update the persisted/public surface inventory for Postgres artifact bytes, evidence,
  event payload copies, lane authority rows, observation facts, cursor metadata, and CLI/REST
  outputs. Each inventory entry must classify secret-boundary status and authority role: strict
  authority, observation, rebuildable cache, operational telemetry, or public output.

Verification:

- `cargo test -p mfm-integration-tests --test architecture_namespace_contract`
- documentation review against `RFC_REFAC_PG_TRANS.md`

### Commit 2: add dependency and namespace guardrails

Scope:

- Add or update cargo metadata/architecture tests that will enforce the final boundary:
  `sqlx` only in Postgres storage crates; CLI/REST no direct concrete storage dependencies.
- Add scans for new `typed_` target table names and public `Typed` target storage/app names.
- Add scans rejecting production references to filesystem artifact storage and in-memory stores.
- Add scans rejecting non-test in-memory store factories, config/env selectors, examples, features,
  or manifest dependencies. In-memory helpers must live behind `#[cfg(test)]` or a test-support
  crate used only by tests.
- Add guardrails rejecting broad production artifact byte writes and arbitrary-id artifact reads
  outside proof-bearing request types.
- Add guardrails rejecting production pre-append filesystem artifact staging.
- Add guardrails rejecting global commit-order locks, global counter rows, table-level append
  serialization, global resource-lane locks, and `commit_pos`/`change_pos` allocation in production
  append paths.
- Add scans proving resource-lane resolvers do not live in storage, app orchestration, CLI, REST,
  signer, keystore, or live transport/client modules.
- Add scans proving hash-bearing storage/read-model code uses canonical MFM bytes and never
  `jsonb::text` as hash input.
- Add guardrails that `RunObservationStore` remains an internal one-page read boundary and has no
  sink writes, projection rebuild/repair methods, active-lane inspection, status-lite
  `get_run_observation`, or cursor internals.
- Add dependency checks that keep signer providers below app/runtime wiring and out of states,
  operations, storage, CLI/REST semantic surfaces, and typed model crates.

Execution note:

- If these tests would fail against current code, land them in the same commit that removes the
  violating dependencies, or mark them as target assertions in docs until the removal commit. Do not
  create permanent allowlist exceptions.

Verification:

- `cargo test -p mfm-integration-tests --test cargo_metadata_contract`
- `cargo test -p mfm-integration-tests --test architecture_namespace_contract`

### Commit 3: replace plan-only append with commit bundles in `mfm-store`

Scope:

- Add `PreparedCommitBundle` and `PreparedArtifactBytes` to `crates/kernel/store`.
- Persist the split canonical request material in the commit authority contract:
  commit purpose, `commit_idempotency_hash`/canonical bytes excluding only closed retry-volatile
  fields such as `expected_next_seq`, and `prepared_authority_hash`/canonical bytes including the full
  accepted authority record.
- Pair every commit-level hash (`commit_idempotency_hash`, `prepared_authority_hash`,
  `commit_batch_hash`) with domain/hash version and canonicalizer identity; strict load must reject
  unknown or mismatched canonicalizer identities.
- Define deterministic `commit_id` derivation before event batch hashing; do not leave commit id as
  a post-staging storage-assigned value.
- Define duplicate `(run_id, commit_key)` behavior: compare idempotency canonical bytes before stale
  sequence checks, return the persisted commit/result on exact idempotent retry, and return
  `IdempotencyConflict` for divergent request material instead of request-computed hashes/results.
- Replace `AsyncTypedRunEventStore::append_prepared_commit_plan` with
  `append_prepared_commit_bundle`.
- Rename the touched durable production run-store trait/type surface to target names in this same
  replacement; do not keep `AsyncTypedRunEventStore` as a forwarding wrapper for the new append API.
  Defer repository-wide old `Typed*` cleanup to the observation/naming gate unless the whole stack is
  landing as one unmerged merge unit.
- Split target store traits into semantic append/load and observation reads:
  `RunEventStore`, `RunObservationStore`, `RetainedArtifactReadProvider`, and `ArtifactReadProvider`.
  Do not add a standalone public commit artifact write-set trait.
- Add resource-lane authority types and event payloads for capability-declared requirements,
  resolved claims, `ResourceLaneClaimIntent`, store-filled committed
  `ResourceLaneClaimed`/`ResourceLaneReleased`, held lane sets, and holder proofs, plus lane-local
  transition authority keyed on a fixed-width `lane_id`.
- Model `ResourceLaneClaimBlocked` only as a non-persisted store outcome for valid contention. It is
  not a run event payload, not lane-transition authority, and not a persisted read-model fact.
- Specify `lane_id` derivation as the fixed-width 32-byte id over canonical
  `(namespace, key_schema_id, key_canonical_json, mode)` and keep the repeated descriptor out of
  keys, unique constraints, and foreign keys except where stored once for audit.
- Only `Exclusive` claims take a lane; `ExactTouchedSet` and `ManualOnly` take none. Encode the
  no-deadlock invariant in the types/contract: an attempt acquires all its lanes in one
  all-or-nothing `ResourceLaneClaimed` commit and never holds a lane while issuing a second blocking
  claim.
- Define the store-filled fencing-token protocol: prepared authority covers claim intent and fill
  policy, while final commit batch hashes cover the store-assigned lane-local token.
- Define final claim/release identity rules: runtime does not supply arbitrary `claim_id`; the final
  claim identity is derived from committed holder identity, lane identity, fencing token, and
  `commit_id`, or otherwise covered by the materialized canonical event payload.
- Make artifact evidence identity evidence-keyed: APIs and strict reads must carry
  `(artifact_id, evidence_hash)` or exact event-derived requirements when typed meaning matters.
- Model existing artifact reuse as an exact-evidence reference such as
  `ExistingArtifactAdmission { artifact_id, evidence_hash }`; artifact-id-only reuse is not a
  production authority surface.
- Make `PreparedArtifactBytes` private-field/proof-object based, not a public `{ bytes, evidence }`
  bag. Bundle construction verifies evidence before the store call; the Postgres append path verifies
  again inside the transaction.
- Update in-memory test store helpers to use bundles.
- Delete the plan-only production append API.

Deletion requirement:

- Do not keep `append_prepared_commit_plan` as a deprecated alias.

Verification:

- `cargo test -p mfm-store`
- targeted compile of crates that depend on `mfm-store`

### Commit 4: refactor runtime commit planning to produce bundles

Scope:

- Update all runtime commit paths to produce/pass `PreparedCommitBundle`:
  run admission, attempt start, attempt terminal, side-effect progress, side-effect terminal,
  framework lifecycle output, retention, manual resolution, saga terminal, recovery, and observed
  failure diagnostics.
- Split side-effect lifecycle so lane-bearing capabilities can resolve concrete resource-lane claims
  during pure preflight after `StateAttemptStarted` and before invocation construction or live IO.
- Put resolver implementations in pure capability/adapter contract code. They may inspect certified
  config, typed inputs, non-secret runtime binding metadata, side-effect identity, and already
  materialized non-secret artifacts; they must not call live transports, signers, network, keystore
  providers, clocks, or storage.
- If an exclusive lane cannot be resolved without live IO or secrets, reject that capability/state
  lifecycle design instead of discovering the lane after invocation starts.
- Keep `StateAttemptStarted` as a separate first attempt commit.
- Commit `ResourceLaneClaimIntent` as a separate certified pre-invocation commit for the
  already-started open attempt; storage materializes the final `ResourceLaneClaimed` event and then
  returns the committed `HeldResourceLaneSet` for invocation preparation.
- Treat valid lane contention as `ResourceLaneClaimBlocked`, leaving the attempt open and parked
  before invocation; do not convert normal contention into attempt failure, saga failure, run
  failure, or stream corruption. Any cancellation/interruption/deadline terminal path must be
  separately certified and unrelated to ordinary contention.
- Retry parked lane claims with capped exponential backoff plus jitter; wake attempts on lane-release
  `LISTEN/NOTIFY` and a periodic floor poll, never treating the notification as the durable signal.
- Keep `ResourceLaneClaimBlocked` out of MFM domain tables and persisted read-model tables; at most
  emit it as ephemeral logs/metrics or non-authoritative operational telemetry outside the transition
  and read-model schema.
- Require side-effect output to echo held lane claims and reject mismatches before append.
- Verify the same committed held lane, or commit a certified recovery claim/release, before any
  recovery path that may touch live IO.
- Remove production `RuntimeArtifactStager` and `stage_prepared_artifacts` flows.
- Keep only narrow in-memory test bundle helpers.
- Ensure recovery/resume never appends newly admitted artifact evidence without bytes in the bundle.

Deletion requirement:

- Delete production pre-append artifact staging code instead of hiding it behind a trait.

Verification:

- `cargo test -p mfm-runtime`
- runtime tests covering artifact-producing commit paths
- runtime tests proving same account/chain EVM mutations cannot both reach nonce read/signing
- runtime tests proving read-only capabilities do not acquire exclusive lanes

### Commit 5: define narrow artifact authority surfaces

Scope:

- Keep artifact reads behind proof-bearing request types:
  retained run-history reads, exact adapter `ArtifactReadAuthority`, and public-output authority
  reads.
- Remove broad production artifact APIs such as arbitrary `get_artifact_by_id`, `has_artifact`,
  or unscoped reads from app-facing surfaces.
- Remove broad production byte insertion APIs such as `insert_artifact_bytes(Vec<u8>)`; bytes may
  enter Postgres only through verified `PreparedCommitBundle` members inside the append transaction.
- Enforce artifact role/schema/semantic/producer contracts and same-commit admission/event bindings.
  Storage verifies content addressing and evidence; it must not rely on secret-content scanning of
  arbitrary `BYTEA`.
- Add or update type-boundary and redaction contracts proving secret-bearing signer/runtime/raw-
  transaction types cannot become typed persisted/public surfaces.
- Ensure SQL/API errors on artifact insertion, verification, rebuild, and read paths are redacted and
  never include secret-bearing values, raw bytes, signer runtime material, or unredacted diagnostics.

Verification:

- `cargo test -p mfm-artifact-capabilities`
- artifact read/provider tests
- secret/redaction regression tests
- compile-fail tests proving signer runtime config, secret wrappers, private-key wrappers,
  mnemonic/password wrappers, raw signed transaction types, and signed payload types cannot implement
  typed persisted/public surfaces such as `MfmValue`, `MfmConfig`, public output, state input,
  operation output, or artifact value contracts
- artifact `BYTEA` tests proving content-addressing, evidence, role, producer, same-commit, and
  commit-bundle binding without broad secret-content scanning

### Commit 6: replace the Postgres storage public surface and schema baseline

Scope:

- Replace public storage type names with target names such as `PostgresRunStore`,
  `PostgresArtifactStore`, `PostgresObservationStore`, and `PostgresStoreError`.
- For the surfaces replaced by this schema/API baseline, delete old `PostgresTyped*` aliases and old
  `typed.rs` style modules instead of keeping forwarding wrappers.
- If executing as one unmerged stack, delete old `typed_*` production SQL, migrations, query modules,
  schema validators, and target storage aliases in this schema/API baseline commit. If executing as
  three landable cutovers, use target names for new/replaced tables but defer repository-wide
  old-name cleanup to the observation/naming cutover; do not merge both old and new paths as
  compatibility aliases.
- Split the Postgres implementation into clear run, artifact, read-model, schema, locking, and
  rebuild modules as needed.
- Replace the old `typed_*` migration baseline with target non-`typed_` tables.
- Add append-only authority tables:
  `commits`, `run_events`, `artifact_blobs`, `artifact_admissions`,
  `run_artifact_admissions`, `commit_artifact_evidence`, `resource_lane_claim_events`,
  `resource_lane_release_events`, and `resource_lane_transitions` (lane tables keyed on `lane_id`).
- Keep only the real uniqueness facts (`commits` PK + `commit_id` + `(run_id, commit_key)`); do not
  add redundant composite UNIQUE supersets. `run_events` stores canonical payload bytes only (no
  `payload_json JSONB`). Child FKs target the natural key (`commits(run_id, seq)`/`(commit_id)`,
  `run_events(run_id, seq, ordinal)`).
- Include commit authority columns for `commit_idempotency_hash`,
  `idempotency_canonical_json`, `prepared_authority_hash`,
  `prepared_authority_canonical_json`, `commit_batch_hash`, `append_xid`, and hash/canonicalizer
  version metadata.
- Enforce an MFM artifact-byte size limit before insert and as a database `CHECK`, well below
  PostgreSQL's hard field-size limit. `artifact_blobs` rows are immutable content-addressed bytes
  only, with `artifact_id` derived from digest and verified by storage.
- Model artifact evidence as evidence-keyed authority: `artifact_admissions` stores canonical
  evidence bytes and query-copy evidence columns; `run_artifact_admissions` and
  `commit_artifact_evidence` bind exact `(artifact_id, evidence_hash)` rows. Run-level admission
  must prove the first exact `binding_kind = 'admitted'` commit evidence row, preferably with an
  ordinary composite FK using a checked `first_binding_kind = 'admitted'` column.
- Add resource-lane constraints for `UNIQUE (lane_id, fencing_token)`, `UNIQUE (claim_id)` releases,
  `resource_lane_transitions` primary key `(lane_id, lane_transition_seq)`, unique transition hashes,
  `UNIQUE (claim_id, transition_kind)`, `UNIQUE (release_id)`, `CHECK (lane_transition_seq >= 1)`,
  transition-kind/source-event-shape checks, and source-event FKs to
  `run_events(run_id, seq, ordinal)`. Keep holder/lane agreement and transition hash-chain semantics
  in Rust append staging plus strict load, not wide composite FKs.
- Add observation/read tables:
  `run_commit_log`, `run_observation_change_summaries`, observation facts, observation derivation
  tables. Do not add `logical_key_observations` (deferred to a future RFC with its completeness proof).
- Add `store_metadata` with singleton row, opaque `store_epoch`, `schema_contract_version`, and
  creation timestamp. Do not add `cursor_domain_seals` or the XID-domain fingerprint/frontier-hash
  subsystem.
- Add `run_commit_log.commit_sort_key BYTEA`, reserve the all-zero 32-byte sentinel, require real
  rows to carry the versioned leading byte, and enforce `UNIQUE (append_xid, commit_sort_key)`.
  Sort-key collisions fail closed as storage errors; do not fall back to text ordering.
- Keep `run_commit_log` immutable cursor authority and keep `run_observation_change_summaries`
  projection-versioned and rebuildable; rebuilding projections must not update or rewrite cursor
  rows.
- Use single-enforcer integrity: SQL relational constraints (FK/UNIQUE/CHECK/NOT NULL) plus the
  database-owned `append_xid` assignment and the no-update/no-delete triggers. Do not add deferrable
  constraint triggers that re-implement multi-row folds (contiguity, cardinality, reverse
  commit-to-commit-log completeness, lane completeness/ordering); those are owned by the Rust append
  path and re-checked by strict load. Any SQL check that duplicates a Rust fold needs a parity test.
- Add database-owned `append_xid` assignment for `commits` and `run_commit_log`; production insert
  statements must not be able to override cursor transaction ids.
- Add run-commit-log ordinal constraints (`first_ordinal = 0`,
  `last_ordinal = event_count - 1`) for the one-row-per-commit cursor log.
- Add no-update/no-delete/no-truncate guards for the app role.
- Deny app/CLI/REST runtime credentials mutation of `store_metadata` and cursor codec/key metadata.
- Avoid `ON DELETE CASCADE` on authority tables.
- Add mutation guards for read fact tables. Do not expose production rebuild/repair, epoch-reseed, or
  orphan-blob-sweep entry points until a Postgres role/ownership/credential/runbook design exists.
- Make schema validation reject stale old `typed_*` schemas or old migration checksums.
- Validate required triggers/functions/views during schema validation, including no-update/delete
  guards, database-owned `append_xid`, and cursor key metadata if stateless sealed cursors are used.

Deletion requirement:

- Do not add compatibility views over old `typed_*` tables.
- Do not read from old `typed_*` tables.

Verification:

- fresh database migration succeeds
- stale schema validation fails
- SQLx prepare/check for the Postgres storage crate
- namespace scan finds no production `PostgresTyped*` storage aliases
- schema validation proves `append_xid` cannot be overridden by production insert statements
- schema validation proves no cursor-domain seal/fingerprint subsystem or `logical_key_observations`
  index exists in the target baseline

### Commit 7: implement transaction-bound Postgres append

Scope:

- Implement `append_prepared_commit_bundle` in the Postgres storage crate.
- Validate bundle shape before opening the transaction where possible, including artifact
  byte/evidence checks, canonical request material, payload shape, and large-byte hashing unless the
  check needs transaction-visible authority rows.
- Inside the transaction, check `(run_id, commit_key)` for idempotency before stale sequence checks,
  take the transaction-scoped run advisory lock, and re-check idempotency after the run lock.
- Verify artifact bytes/evidence inside the transaction before inserting authority rows.
- Insert `commits`, `artifact_blobs`, artifact evidence/admission rows, `run_events`,
  resource-lane claim/release/transition rows, and `run_commit_log` atomically. Large artifact bytes
  may be content-addressed in a short preceding transaction; the append transaction then verifies the
  blob digest/length and inserts only the small evidence/admission/event rows.
- For lane claim/release commits, require committed lane claim/release intents and holder proofs in
  the prepared bundle. Use sorted per-`lane_id` transaction advisory locks (two-argument
  `pg_advisory_xact_lock(classid, objid)` with a lane class id distinct from the run-lock class id)
  while admitting those commit-bound rows, assign store-filled lane-local transition sequences and
  fencing tokens under the lock, and return `ResourceLaneClaimBlocked` without appending authority
  rows or persisted read-model facts for valid contention. Do not expose standalone lane
  acquire/release APIs, derive lanes in storage, or add a global commit-order lock, global
  resource-lane lock, global counter row, table-level append serialization, or
  `commit_pos`/`change_pos` allocation.
- For admitted lane claims/releases, assign lane-local transition sequence, lane-local fencing token
  for claims, final claim/release ids when store-filled, and final committed lane event payload while
  holding the concrete lane locks. Compute the final `commit_batch_hash` after materializing
  store-filled fields.
- Enforce terminal lane-release scope rules so attempt, side-effect, saga, and run terminal commits
  cannot leave release-required lane claims active unless the verified prefix or closed same-commit
  terminal-release rule proves release.
- Enforce the closed same-commit terminal-release batch shape: only exact in-scope active-claim
  releases, releases appear before the terminal disposition event in `(seq, ordinal)` order, no
  unrelated releases, and no new lane acquisition in the same terminal commit.
- Enforce batch-hash integrity in Rust from persisted canonical bytes; SQL keeps only relational
  constraints. Do not create a SQL canonicalization engine.
- Enforce event ordinals, event count, source-event mirror completeness, commit-artifact evidence
  completeness, one-to-one lane event/mirror/transition completeness, and reverse
  commit-to-commit-log completeness in Rust staging before `COMMIT`.
- Build logical-key admission by folding authoritative `run_events`; there is no observation index in
  v1, so observation-row absence can never influence admission.
- Let database-owned triggers/functions assign `commits.append_xid` and
  `run_commit_log.append_xid` from PostgreSQL `pg_current_xact_id()` and use it only for
  snapshot-sealed observation cursors.
- Insert exactly one `run_commit_log` row for each commit and fail the transaction if the reverse
  commit-to-commit-log completeness check fails.
- Insert synchronous mechanical observation facts required by the public observation API atomically
  with authority. If synchronous observation derivation fails, roll back the whole transaction.
  Expensive dashboard/cache models may be rebuilt asynchronously, but app/CLI/REST handlers must
  surface `ObservationUnavailable` or omit optional summaries rather than running privileged rebuilds.
- Emit `NOTIFY` only after authority, commit-log, and synchronous observation rows are inserted;
  notification is a wakeup delivered at commit, never the data source.
- Remove mutable helper writes: no run-head updates, no unique logical-key upserts, no current-state
  table updates.

Verification:

- same-run concurrent append tests
- cross-run resource-lane contention tests
- valid lane contention returns `ResourceLaneClaimBlocked` and leaves the attempt open before
  invocation
- valid lane contention does not insert authority rows or persisted read-model facts claiming
  derivation from committed history
- independent cross-run append tests proving unrelated runs do not block each other
- crash after lane-bearing `StateAttemptStarted` but before `ResourceLaneClaimed` retries pure
  preflight and claim admission, parks, or applies a separate certified cancellation/interruption
  policy; ordinary contention does not become failure proof
- crash after `ResourceLaneClaimed` but before invocation preparation recovers the same committed
  held lane or releases it through a certified cleanup/interruption commit
- Postgres rejects prepared invocation resource keys without a committed held lane
- Postgres rejects prepared invocation keys that differ from preflight claims
- SQL/code scans prove no global resource-lane table lock or single `global` lock row remains
- lane-transition sequence/hash continuity and terminal-release scope tests
- idempotent commit-key retry tests
- idempotency conflict tests proving duplicate `(run_id, commit_key)` returns persisted results on
  matching canonical bytes and rejects divergent material before stale sequence checks
- commit hash tests proving `commit_id`, prepared authority, idempotency, final batch hash, hash
  domain version, and canonicalizer identity reverify from persisted canonical bytes
- artifact mismatch/missing/extra-byte rejection tests
- artifact byte-size limit and append-transaction-bound blob insert tests
- no-deadlock invariant test: a history holding a lane then committing a further `ResourceLaneClaimed`
  is rejected
- lane and run advisory locks use distinct class ids
- lane id derivation tests proving keys/indexes use fixed-width `lane_id`, not repeated
  `key_canonical_json` descriptors
- same-commit terminal-release tests for ordering, exact scope, unrelated release rejection, and no
  same-batch new acquisition
- snapshot-sealed watch cursor tests with older slow transactions and newer fast transactions
- `LISTEN/NOTIFY` wakeup tests proving missed/coalesced notifications do not affect correctness
- SQL scan showing no production domain `UPDATE`, `DELETE`, or `TRUNCATE`

### Commit 8: implement strict Postgres load and artifact authority reads

Scope:

- Load run streams from `commits`/`run_events` and cross-check:
  event count, ordinals, run-local `(run_id, seq, ordinal)` membership, commit key/id, payload
  hash, event identity, schema id, spec hash, logical key, commit id derivation, idempotency hash,
  prepared authority hash, batch hash, canonicalizer identity, and canonical request bytes.
- Strict-load lane authority from `resource_lane_claim_events`, `resource_lane_release_events`, and
  `resource_lane_transitions`; verify fixed-width `lane_id` derivation, source event bindings,
  transition hash chains, contiguous lane-local transition sequences, active-holder fold, release
  legality, claim-token monotonicity, and no-deadlock history shape before trusting any lane.
- Strict-load terminal scope invariants so side-effect/attempt/saga/run terminal histories cannot
  leave release-required lane claims active unless the verified prefix or same terminal commit proves
  the allowed release rule.
- Load retained artifacts from Postgres through event-derived requirements.
- Implement adapter/public-output artifact read surfaces over Postgres with proof-bearing requests.
- Reject artifact id/digest/byte length/evidence hash inconsistencies.
- Reject artifact evidence mismatches where evidence canonical bytes do not bind the stored artifact
  id, digest, byte length, role/schema/semantic metadata, producer identity, certified
  spec/descriptor context, or requirement provenance expected by the request.
- Ensure strict resume, replay, public-output rendering, retention proof, side-effect recovery,
  manual-resolution authority, and strict per-run status load authority rows and verified artifacts;
  read models may be hints only and cannot mint authority.

Verification:

- strict stream load corruption tests
- lane strict-load corruption and active-holder tests
- terminal scope release invariant tests
- canonicalizer/hash-domain mismatch tests
- retained artifact read tests
- public-output authority artifact tests
- adapter artifact request tests
- tests proving read-model corruption does not affect strict status/resume/replay/public-output reads

### Commit 9: implement logical-key admission folding

Scope:

- Fold authoritative `run_events` into `LogicalKeySet` and `UniqueLogicalPayloads` for admission and
  strict load. There is no `logical_key_observations` table in v1.
- Keep the recoverable submission-result exception in the Rust staging rule, applied against the
  folded `run_events` prefix.
- A future RFC may add an observation index together with its prefix-completeness proof; not here.

Verification:

- accepted recoverable submission-result rewrite test
- rejected repeated unique logical-key tests
- tests proving logical-key admission is derived only from `run_events` folds (no index to be absent)

### Commit 10: implement Postgres-owned read models

Scope:

- Implement mechanical SQL facts only for scalar/canonical-byte derivations.
- Persist only mechanical observation rows derived from scalar/canonical-byte authority columns in
  the initial implementation. Do not call runtime/app semantic projection code from storage.
- Start with simple SQL views where possible: run admission summary, run completion summary, run head
  by `max(seq)`, mechanical observed public-output event-label summary, and change feed over
  `run_commit_log.append_xid`.
- Use trigger-derived insert-only read facts only when a query is too expensive as a pure view, the
  derivation is mechanical from one committed event or explicitly recorded aggregate provenance, and
  rebuild functions can recreate the same facts from authority rows.
- Do not implement saga mode, side-effect ledger legality, manual-resolution state, retention proof,
  or public-output authority as independent PL/pgSQL projections.
- Every delta observation fact must carry source-event provenance; every aggregate fact must carry
  prefix or multi-source provenance. Sparse multi-source rows must enumerate sources or use a
  documented prefix hash chain.
- Hash-bearing facts must store canonical row bytes produced by MFM canonicalization code, or use a
  database-side canonicalizer tested against `mfm-canonical`; never hash PostgreSQL `jsonb::text`.
- Implement `run_observation_change_summaries` separate from immutable `run_commit_log`.
- Add `current_*` views over observation facts.
- Keep materialized views/cache rows in clearly cache-like namespaces only; they must never be cursor
  authority and never strict semantic authority.
- Keep read-model proof as private read-only validation: append-created rows must match rows
  materialized from authority, and drift/corruption tests must detect mismatches without exposing a
  production repair API.
- Do not add a production-public observation sink trait in this RFC. Observation rows are written only
  by the append pipeline that owns the authority and provenance for the row.
- Semantic-looking observations must record `producer_authority`, `projection_version`, source
  provenance, and row hash. Storage may persist such rows from the owning authority layer through the
  append/rebuild pipeline, but must not call runtime/app projection code internally.

Verification:

- observation facts materialize exactly from authority rows in private read-only validation
- drift/corruption tests detect missing, extra, and mismatched read rows without public repair APIs
- delta and aggregate provenance tests, including full watch frontier tuple recovery for
  platform-prefix high-watermarks
- SQL-only derivations are mechanical
- hash-bearing read facts use canonical MFM bytes and not `jsonb::text`
- materialized/cache models are never cursor authority and never strict semantic authority
- observation rows produced by authority-layer projection code match strict projection rebuilds
- no public/app observation sink can write rows outside the append/rebuild pipeline
- corrupt observation facts do not affect strict resume/replay/public-output reads
- required observation gaps return `ObservationUnavailable`; optional summaries are omitted by
  public contract

### Commit 11: expose one app-level run observation API

Scope:

- Add one app-level `read_run_observations(query) -> RunObservationPage` API.
- Do not add separate `list_runs`, `poll_run_changes`, or `watch_run_changes` app contracts.
- Reuse the existing strict `run_status`, strict `run_stream`, and public-output authority paths;
  rename them only as part of removing `typed` prefixes.
- Implement opaque cursor encoding over `(run_commit_log.append_xid, run_commit_log.commit_sort_key)`
  with cursor format version and store epoch.
- Use snapshot-sealed polling with `safe_before_xid = pg_snapshot_xmin(pg_current_snapshot())`.
  Returned rows and `next_cursor` must be bounded by one sealed frontier per request.
- Use sealed/MACed stateless cursors or server-issued opaque tokens. If stateless cursors are used,
  define `cursor_key_id`, durable key source across app/CLI/REST restarts, accepted old-key rotation
  window, and stable errors for unknown, missing, retired, mismatched, tampered, malformed, and
  future cursor keys or formats, and decide whether projection/query compatibility metadata is part
  of the MACed payload. If server-issued opaque tokens are used instead, define an insert-only token
  authority table or equivalent durable token authority, token expiry/retention behavior, and the
  same stable error cases before exposing the API. Tampered or future lower-bound cursors must fail
  closed.
- Keep `append_xid`, `commit_sort_key`, and PostgreSQL transaction ids out of public JSON/text
  output; expose only `next_cursor` and, if necessary, an opaque `change_id`.
- Keep `cursor_version`, `store_epoch`, `projection_version`, high-watermarks, frontier lag,
  commit ids, observation ids, artifact ids, evidence hashes, lane keys, holder proofs, and fencing
  tokens out of public JSON/text output.
- Make `read_run_observations` use one snapshot-sealed frontier for returned rows and the next
  cursor. Page cursors, if added later, must be a separate design from watch cursors.
- Implement list mode and watch mode as one API: no cursor returns the current matching runs bounded
  by one sealed frontier, with `next_cursor` at that frontier; a cursor returns changes after that
  lower bound and at or before a fresh sealed frontier. v1 list mode is capped by `limit` and is not
  separately paginated.
- Minimum public page fields are `next_cursor` and `runs[]`, with optional opaque `change_id`.
  Minimum run fields are `run_id`, `head_seq`, `observed_status`, `started_at`, `updated_at`, and
  `completed_at`. Semantic-looking fields must be omitted or clearly observation-only.
- Do not expose bare saga mode, public output summaries, attempt summaries, manual-resolution
  authority, retention proof, or public-output authority in list/watch; callers use strict per-run
  APIs for those.
- Return long-poll timeout as a successful empty page with a fresh cursor, not as an error.
- Poll durable `run_commit_log` rows before and after any long-poll wait. `LISTEN/NOTIFY` is only a
  wakeup and may be missed or coalesced.
- Reject cursors whose `store_epoch` does not match the live row with the public `CursorExpired`
  error.
- Keep `run_status` and `run_stream` strict authority reads (immediate, no frontier lag).
- Keep list/watch as observation reads over Postgres read models. Document the read-your-writes
  caveat: a just-created run appears in list/watch only after the sealed frontier advances past it.

Verification:

- cursor decode/encode tests
- cursor sealing/MAC or server-token tamper tests
- cursor key-id, durable-key-source, restart, accepted old-key rotation window, unknown-key, missing
  key, retired-key, and stale-format tests
- no-skip watch cursor tests
- list/watch single-frontier consistency tests
- tests proving `RunObservationStore` does not join unbounded `current_*` views that can race ahead
  of the returned frontier
- expired cursor tests
- `ObservationUnavailable` tests
- successful empty-page long-poll timeout tests
- durable poll-before/after-wait tests proving `LISTEN/NOTIFY` is not required for correctness
- strict stream watch tests proving authority stream reads

### Commit 12: refactor app construction around production Postgres factory

Scope:

- Make app services generic over run store, observation store, and narrow artifact read surfaces.
- Add one production factory that composes the Postgres run/artifact/read-model surfaces.
- Remove app construction that accepts filesystem artifact stores for production.
- Remove app construction, examples, configuration, feature gates, and environment selectors that
  accept in-memory stores in production. Move any remaining in-memory helper behind `#[cfg(test)]`
  or a test-support crate used only by tests.
- Remove direct concrete storage wiring from CLI/REST paths.

Verification:

- app tests compile without filesystem artifact store production dependencies
- app tests compile without production in-memory store selectors
- cargo metadata guardrails for app/CLI/REST boundaries

### Commit 13: implement CLI list/watch and remove old storage flags

Scope:

- Add `mfm run list [--cursor <opaque>] [--limit <n>] [--wait-ms <n>] [--watch]`.
- Do not add a separate first-class `mfm run watch` command in this RFC. If ergonomics require it
  later, add it as an alias over `mfm run list --watch` in a separate change.
- Preserve strict `mfm run stream <RUN_ID> --from-seq <N> --watch`.
- Define stable JSON output and compact text output.
- Expose only the minimal observation page fields: `next_cursor`, `runs[]`, and optionally opaque
  `change_id`; each run carries at least `run_id`, `head_seq`, `observed_status`, `started_at`,
  `updated_at`, and `completed_at`.
- Update `bin/cli/README.md` and CLI JSON/text contract tests in this same commit. Do not defer CLI
  docs for the new command, storage flags, or read-your-writes caveat to the final docs pass.
- Document the explicit local database/schema reset command in the CLI README in this same commit.
- Remove `--typed-artifact-root` and `MFM_TYPED_ARTIFACT_ROOT`.
- Remove direct `sqlx`, Postgres storage crate, and filesystem artifact store dependencies from
  CLI production manifest.

Deletion requirement:

- Delete old CLI store-selection helpers instead of hiding them.

Verification:

- CLI JSON contract tests
- CLI text smoke tests
- CLI dependency guardrail tests

### Commit 14: implement REST list/watch and remove direct storage wiring

Scope:

- Add REST `GET /v1/runs?cursor=<opaque>&limit=<n>&wait_ms=<n>`.
- Do not add `GET /v1/runs/watch` in this RFC. Future SSE/WebSocket streaming must be a separate
  proposal if long-poll pages are insufficient.
- Keep strict status/stream routes strict.
- Define stable JSON schemas and error codes:
  `InvalidCursor`, `CursorExpired`, `LimitOutOfRange`, `RunNotFound`,
  `ObservationUnavailable`.
- Expose the same public observation response shape as the app/CLI contract: `next_cursor`, `runs[]`,
  optional opaque `change_id`, and run fields `run_id`, `head_seq`, `observed_status`, `started_at`,
  `updated_at`, and `completed_at`.
- Treat long-poll timeout as HTTP 200 with an empty page and fresh cursor.
- Update `bin/rest-api/README.md` and REST schema/route contract tests in this same commit. Do not
  defer REST docs for the new route, storage env changes, or cursor errors to the final docs pass.
- Document the explicit local database/schema reset command in the REST README in this same commit.
- Remove direct concrete storage and filesystem artifact store dependencies from REST production
  manifest.

Verification:

- REST route contract tests
- REST cursor/error tests
- REST dependency guardrail tests

### Commit 15: delete filesystem artifact store production crate/path

Scope:

- Delete `crates/storages/artifact-store-fs` and every production constructor/path that depends on
  it, unless the implementation has already removed the crate entirely in an earlier replacement
  commit.
- Delete production references, docs, examples, flags, environment variables, and dependency
  entries.
- Replace tests that need non-Postgres artifacts with narrow in-memory test helpers.
- Remove old artifact root docs and environment variables.

Deletion requirement:

- Do not keep a filesystem artifact store fixture crate. Tests that need non-Postgres artifact
  bytes must use narrow in-memory test helpers.
- This deletion is required before any cutover merge if the filesystem store still exists at this
  point in the stack.

Verification:

- `rg "artifact-store-fs|FsTypedArtifactStore|typed-artifact-root|MFM_TYPED_ARTIFACT_ROOT"`
  returns no production hits
- workspace compile

### Commit 16: verify old names and compatibility surfaces are gone

Scope:

- Run final scans proving old public `Typed*` storage/app names were removed in their replacement
  commits.
- Run final scans proving old `typed_*` SQL references are gone from production migrations and
  queries.
- Remove any remaining dependency allowlist exceptions only if a previous replacement commit could
  not do so atomically; otherwise this commit should only verify they are already absent.
- Delete dead comments/TODOs that described temporary compile-breaking stack state. Do not use this
  commit to remove production compatibility paths that should have been deleted with their
  replacements.

Verification:

- namespace scans
- cargo metadata contract
- architecture namespace contract

### Commit 17: update docs and runbooks for the final user surface

Scope:

- Reconcile `docs/design.md`, `docs/architecture.md`, and `docs/saga.md` against the implemented
  cutover before the stack merges. This is final consistency review, not permission to delay
  authoritative doc updates until after merge.
- Verify `bin/cli/README.md` and `bin/rest-api/README.md` already changed in the same commits as
  their command/route changes; this commit is only the final consistency pass.
- Verify the storage crate README/runbook, CLI README, and REST README all document the same explicit
  fresh database/schema reset command.
- Document that old local databases/artifact roots are disposable.

Verification:

- docs review against implementation
- CLI/REST examples run against fresh Postgres where applicable

### Commit 18: full verification and cleanup

Scope:

- Run the focused and workspace-level gates.
- Regenerate SQLx metadata against a freshly migrated disposable database.
- Remove temporary TODOs, transitional comments, and dead helper modules.
- Confirm the final branch has no old production storage path.

Required checks:

- `cargo fmt --all -- --check`
- `cargo check --workspace`
- `cargo test --workspace`
- `cargo test -p mfm-integration-tests --test cargo_metadata_contract`
- `cargo test -p mfm-integration-tests --test architecture_namespace_contract`
- SQLx prepare/check for the Postgres storage crate
- focused Postgres parity/concurrency/cursor/artifact tests with a fresh database

If the Postgres storage crate is renamed during the refactor, update the package names in these
commands in the same commit that renames the crate. Do not keep an old crate name only to preserve
old command examples.

## Parallel Work Boundaries

Parallelize only where write sets are disjoint.

- Kernel/runtime work can proceed before Postgres implementation, but it must break and then repair
  callers by replacing plan-only append with bundles.
- Postgres schema work can proceed after the bundle contract is clear.
- CLI/REST work should wait for the app API and production factory.
- Docs/design updates should start first and be finalized again after code lands.

Avoid parallel edits to the same file families:

- `crates/kernel/store/src/lib.rs`
- `crates/kernel/runtime/src/*`
- Postgres storage crate migrations and SQLx query files
- `crates/app/src/lib.rs`
- CLI/REST command/router modules

## Engineer Checklist For Each Commit

Before coding:

- State the commit title and exact write scope.
- State which old code will be deleted.
- State which tests prove the new behavior.
- State whether an architect-agent review is needed.

During coding:

- Delete replaced code in the same commit.
- Avoid compatibility wrappers.
- Avoid feature flags for old behavior.
- Keep app/CLI/REST out of direct storage implementation details.
- Keep strict reads strict.

Before committing:

- Run the focused tests for the touched crates.
- Run `cargo fmt --all -- --check` when Rust files changed.
- Run architecture/cargo metadata checks when dependency boundaries changed.
- Record any architectural decision in docs, not only in commit messages.

## Architect-Agent Escalation Triggers

Spawn an architect-agent before proceeding when:

- a change would make read models influence append admission
- SQL starts interpreting saga, side-effect, manual-resolution, retention, or public-output semantics
- a proposal keeps old filesystem/in-memory production storage alive
- a proposal adds fallback behavior or compatibility aliases
- a cursor/order change could skip or duplicate durable watch events
- artifact reads can happen by arbitrary id without proof-bearing request authority
- resource-lane claims are derived in storage, discovered after live IO, created outside prepared
  commit authority, or guarded by a global lock
- CLI/REST need a direct dependency on a concrete storage crate

The expected output from the architect-agent is a clear recommendation and a commit-sized path
forward.

## Final Definition Of Done

The refactor is done when all of the following are true:

- A fresh Postgres database can migrate to the target schema.
- A stale old `typed_*` database is rejected.
- Production appends insert artifact bytes, events, artifact evidence/admissions keyed by evidence
  hash, committed resource-lane claim/release/transition rows, commit-log rows, and required
  observation facts atomically.
- Commit idempotency, prepared authority, commit id, final batch hash, hash-domain versions, and
  canonicalizer identities reverify from persisted canonical bytes; `run_events` stores canonical
  payload bytes only.
- Logical-key admission is proven by folding authoritative `run_events`; there is no admission index.
- Resource-lane contention parks open attempts through `ResourceLaneClaimBlocked` without writing
  corrupt or terminal stream authority, and the no-deadlock single-claim invariant holds.
- Ordinary resource-lane contention does not authorize attempt/saga/run failure.
- Opaque epoch-tagged cursors plus database-owned `append_xid` assignment, bytewise
  `commit_sort_key`, snapshot-sealed frontiers, and cursor key rotation rules protect list/watch
  cursors. Restore/clone/import/rollback procedures are destructive with respect to issued cursors
  until an authorized epoch-reseed design exists.
- Read models materialize from authority rows, carry event/prefix/multi-source provenance, can be
  drift-checked in private read-only validation, and never hash PostgreSQL `jsonb::text`.
- Strict status, stream, replay, and public output do not trust read models.
- CLI/REST expose one shared app run observation API with the stable minimal response shape
  (`next_cursor`, `runs[]`, optional opaque `change_id`; each run has `run_id`, `head_seq`,
  `observed_status`, `started_at`, `updated_at`, `completed_at`).
- Secret-bearing signer/runtime/raw-transaction types are compile-fail blocked from typed
  persisted/public surfaces, and artifact/read-model SQL/API errors are redacted.
- CLI/REST no longer expose filesystem artifact roots.
- CLI/REST no longer depend directly on `sqlx`, Postgres storage crates, or filesystem stores.
- Filesystem storage is deleted from production.
- In-memory storage exists only behind test-only modules or test-support crates for tests that
  intentionally avoid Postgres.
- `docs/design.md`, `docs/architecture.md`, `docs/saga.md`, CLI docs, and REST docs match the
  implementation.
- No permanent compatibility shims, old aliases, or fallback paths remain.
