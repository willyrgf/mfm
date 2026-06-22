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

1. Append/artifact cutover: `PreparedCommitBundle`, artifact bytes/evidence into Postgres, filesystem
   artifact store removal, the append-only authority schema (partitioned), and the constraint/payload
   simplifications.
2. Resource-lane cutover: the pre-invocation lane lifecycle, per-lane admission, `lane_id`-keyed
   lane-transition authority, the no-deadlock single-claim invariant, and the `docs/saga.md`
   resource-claim rewrite.
3. Observation cutover: the read-model layer, `run_commit_log` epoch-tagged watch cursors, the shared
   CLI/REST list/watch API, and the `typed_` -> target naming baseline.

Within each cutover do not merge a partial state where runtime authority, storage behavior, public
APIs, and docs disagree. Prefer three landable cutovers over one big-bang merge so each new primitive
(bundle append, lanes, watch cursors) is validated independently.

## Target Outcome

The finished refactor has one production persistence story:

```text
Postgres authority tables
  + SQLx-only storage crate access
  + append-only domain rows
  + transaction-bound artifact bytes/evidence
  + Postgres-owned rebuildable read models
  + one app API exposed by CLI/REST
  + no production filesystem/in-memory stores
```

The implementation is complete only when:

- production app/CLI/REST cannot select filesystem or in-memory storage
- artifact bytes and evidence are inserted in the same Postgres transaction as committed events
- plan-only append is gone from production
- old `typed_*` schema surfaces are gone from the final storage baseline
- old storage type names are not kept as compatibility aliases
- CLI/REST do not depend directly on `sqlx`, Postgres storage crates, or filesystem stores
- `docs/design.md` and `docs/architecture.md` describe the new source of truth

## Coordination Model

Use three workstreams, but land commits in dependency order.

- Kernel/runtime authority: `crates/kernel/store`, `crates/kernel/runtime`, `crates/kernel/replay`,
  capability contracts, and in-memory test helpers.
- Postgres storage: Postgres schema, SQLx queries, artifact bytes/evidence, read models, rebuild
  tools, maintenance role, and strict load verification.
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
- Rewrite the `docs/saga.md` "Resource Claims" section for the pre-invocation `ResourceLaneClaimed`
  lifecycle, the claim-kind-to-lane mapping (`Exclusive` only), and the no-deadlock invariant; this
  lands in the resource-lane cutover merge unit (a planned-change callout already points to the RFC).
  partition-detach archival (not production delete), the artifact-size limit, and the dedicated-MFM-
  database requirement driven by cluster-wide `xmin` coupling.
- Document the read-your-writes caveat (list/watch lag the sealed frontier; strict status is
  immediate) and the operator `reseed_store_epoch` step after physical restore/clone.
- Update the storage README/runbook with the destructive fresh-database cutover stance.
- Mark filesystem artifact storage as removed from production architecture.
- Mark in-memory storage as test-only.
- Document the destructive dev-branch cutover posture.
- Create or update the persisted/public surface inventory for Postgres artifact bytes, evidence,
  event payload copies, lane authority rows, observation facts, cursor metadata, and CLI/REST
  outputs.

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
- Persist canonical prepared request material in the commit authority contract:
  commit purpose, prepared request hash, and canonical request bytes.
- Define deterministic `commit_id` derivation before event batch hashing; do not leave commit id as
  a post-staging storage-assigned value.
- Replace `AsyncTypedRunEventStore::append_prepared_commit_plan` with
  `append_prepared_commit_bundle`.
- Rename the durable production run-store trait/type surface in this same cutover; do not keep
  `AsyncTypedRunEventStore` or other replaced `Typed*` production aliases as forwarding wrappers.
- Add resource-lane authority types and event payloads for capability-declared requirements,
  resolved claims, `ResourceLaneClaimIntent`, store-filled committed
  `ResourceLaneClaimed`/`ResourceLaneReleased`, `ResourceLaneClaimBlocked`, held lane sets, and
  holder proofs, plus lane-local transition authority keyed on a fixed-width `lane_id`.
- Only `Exclusive` claims take a lane; `ExactTouchedSet` and `ManualOnly` take none. Encode the
  no-deadlock invariant in the types/contract: an attempt acquires all its lanes in one
  all-or-nothing `ResourceLaneClaimed` commit and never holds a lane while issuing a second blocking
  claim.
- Define the store-filled fencing-token protocol: prepared authority covers claim intent and fill
  policy, while final commit batch hashes cover the store-assigned lane-local token.
- Make artifact evidence identity evidence-keyed: APIs and strict reads must carry
  `(artifact_id, evidence_hash)` or exact event-derived requirements when typed meaning matters.
- Model existing artifact reuse as an exact-evidence reference such as
  `ExistingArtifactAdmission { artifact_id, evidence_hash }`; artifact-id-only reuse is not a
  production authority surface.
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
- Keep `StateAttemptStarted` as a separate first attempt commit.
- Commit `ResourceLaneClaimIntent` as a separate certified pre-invocation commit for the
  already-started open attempt; storage materializes the final `ResourceLaneClaimed` event and then
  returns the committed `HeldResourceLaneSet` for invocation preparation.
- Treat valid lane contention as `ResourceLaneClaimBlocked`, leaving the attempt open and parked
  before invocation; do not convert normal contention into attempt failure, saga failure, run
  failure, or stream corruption. Any cancellation/interruption/deadline terminal path must be
  separately certified and unrelated to ordinary contention.
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
- Add or update type-boundary and redaction contracts proving secret-bearing signer/runtime/raw-
  transaction types cannot become typed persisted/public surfaces.

Verification:

- `cargo test -p mfm-artifact-capabilities`
- artifact read/provider tests
- secret/redaction regression tests

### Commit 6: replace the Postgres storage public surface and schema baseline

Scope:

- Replace public storage type names with target names such as `PostgresRunStore`,
  `PostgresArtifactStore`, `PostgresObservationStore`, and `PostgresStoreError`.
- Delete old `PostgresTyped*` aliases and old `typed.rs` style modules instead of keeping
  forwarding wrappers.
- Delete old `typed_*` production SQL, migrations, query modules, schema validators, and target
  storage aliases in this schema/API baseline commit. Do not defer old-name cleanup to a later
  compatibility-removal pass.
- Split the Postgres implementation into clear run, artifact, read-model, schema, locking, and
  rebuild modules as needed.
- Replace the old `typed_*` migration baseline with target non-`typed_` tables.
- Add append-only authority tables, declared partitioned on a stable per-run key so archival can
  detach partitions later:
  `commits`, `run_events`, `artifact_blobs`, `artifact_admissions`,
  `run_artifact_admissions`, `commit_artifact_evidence`, `resource_lane_claim_events`,
  `resource_lane_release_events`, and `resource_lane_transitions` (lane tables keyed on `lane_id`).
- Keep only the real uniqueness facts (`commits` PK + `commit_id` + `(run_id, commit_key)`); do not
  add redundant composite UNIQUE supersets. `run_events` stores canonical payload bytes only (no
  `payload_json JSONB`). Child FKs target the natural key (`commits(run_id, seq)`/`(commit_id)`,
  `run_events(run_id, seq, ordinal)`).
- Add observation/read tables:
  `run_commit_log`, `run_observation_change_summaries`, observation facts, observation derivation
  tables. Do not add `logical_key_observations` (deferred to a future RFC with its completeness proof).
- Add `store_metadata` with an opaque `store_epoch`. Do not add `cursor_domain_seals` or the
  XID-domain fingerprint/frontier-hash subsystem.
- Use single-enforcer integrity: SQL relational constraints (FK/UNIQUE/CHECK/NOT NULL) plus the
  database-owned `append_xid` assignment and the no-update/no-delete triggers. Do not add deferrable
  constraint triggers that re-implement multi-row folds (contiguity, cardinality, reverse
  commit-to-commit-log completeness, lane completeness/ordering); those are owned by the Rust append
  path and re-checked by strict load. Any SQL check that duplicates a Rust fold needs a parity test.
- Add database-owned `append_xid` assignment for `commits` and `run_commit_log`; production insert
  statements must not be able to override cursor transaction ids.
- Add run-commit-log ordinal constraints (`first_ordinal = 0`,
  `last_ordinal = event_count - 1`) for the one-row-per-commit cursor log.
- Add an explicit `reseed_store_epoch` maintenance entry point (maintenance role only) for use after
  destructive reset, restore, clone, import, or rollback; it rotates `store_epoch` and invalidates
  outstanding cursors. Startup does not auto-detect physical-domain changes.
- Add no-update/no-delete/no-truncate guards for the app role.
- Add maintenance-role-only rebuild/validation/archival/reseed procedure boundaries.
- Make schema validation reject stale old `typed_*` schemas or old migration checksums.

Deletion requirement:

- Do not add compatibility views over old `typed_*` tables.
- Do not read from old `typed_*` tables.

Verification:

- fresh database migration succeeds
- stale schema validation fails
- SQLx prepare/check for the Postgres storage crate
- namespace scan finds no production `PostgresTyped*` storage aliases

### Commit 7: implement transaction-bound Postgres append

Scope:

- Implement `append_prepared_commit_bundle` in the Postgres storage crate.
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
- Enforce terminal lane-release scope rules so attempt, side-effect, saga, and run terminal commits
  cannot leave release-required lane claims active unless the verified prefix or closed same-commit
  terminal-release rule proves release.
- Enforce batch-hash integrity in Rust from persisted canonical bytes; SQL keeps only relational
  constraints. Do not create a SQL canonicalization engine.
- Build logical-key admission by folding authoritative `run_events`; there is no observation index in
  v1, so observation-row absence can never influence admission.
- Let database-owned triggers/functions assign `commits.append_xid` and
  `run_commit_log.append_xid` from PostgreSQL `pg_current_xact_id()` and use it only for
  snapshot-sealed observation cursors.
- Insert exactly one `run_commit_log` row for each commit and fail the transaction if the reverse
  commit-to-commit-log completeness check fails.
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
- artifact mismatch/missing/extra-byte rejection tests
- large-blob pre-commit + short append-transaction test; maintenance-role orphan-blob sweep test
- no-deadlock invariant test: a history holding a lane then committing a further `ResourceLaneClaimed`
  is rejected
- lane and run advisory locks use distinct class ids
- snapshot-sealed watch cursor tests with older slow transactions and newer fast transactions
- SQL scan showing no production domain `UPDATE`, `DELETE`, or `TRUNCATE`

### Commit 8: implement strict Postgres load and artifact authority reads

Scope:

- Load run streams from `commits`/`run_events` and cross-check:
  event count, ordinals, run-local `(run_id, seq, ordinal)` membership, commit key/id, payload
  hash, batch hash, and request hash.
- Load retained artifacts from Postgres through event-derived requirements.
- Implement adapter/public-output artifact read surfaces over Postgres with proof-bearing requests.
- Reject artifact id/digest/byte length/evidence hash inconsistencies.

Verification:

- strict stream load corruption tests
- retained artifact read tests
- public-output authority artifact tests
- adapter artifact request tests

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
- Add `RunObservationSink` only for authority-produced projection-versioned observations with
  provenance and row hashes. It is not a general app/framework write surface.
- Implement `run_observation_change_summaries` separate from immutable `run_commit_log`.
- Add `current_*` views over observation facts.
- Add `build_projection_version`, validate, and drift tooling behind maintenance role boundaries.

Verification:

- observation facts rebuild exactly from authority rows
- SQL-only derivations are mechanical
- observation rows produced by authority-layer projection code match strict projection rebuilds
- `RunObservationSink` rejects rows without producer authority, provenance, projection version, and
  canonical row hash
- corrupt observation facts do not affect strict resume/replay/public-output reads

### Commit 11: expose app-level list/watch/read APIs

Scope:

- Add app-level `list_runs`, `poll_run_changes`, `watch_run_changes`, `run_status`, and
  `run_stream` APIs.
- Implement opaque cursor encoding over `(run_commit_log.append_xid, run_commit_log.commit_sort_key)`
  with cursor format version and store epoch.
- Use sealed/MACed stateless cursors or server-issued opaque tokens with defined key/source,
  rotation, and stable cursor errors. Tampered or future lower-bound cursors must fail closed.
- Keep `append_xid`, `commit_sort_key`, and PostgreSQL transaction ids out of public JSON/text
  output; expose opaque cursors/high-watermarks and domain observation ids instead.
- Make `list_runs` and watch polling use one snapshot-sealed frontier for returned rows, joined
  summaries, projection version, and watch cursor. Page cursors, if added, must be separate from
  watch cursors.
- Reject cursors whose `store_epoch` does not match the live row with `StaleStoreEpoch`.
- Keep `run_status` and `run_stream` strict authority reads (immediate, no frontier lag).
- Keep list/watch as observation reads over Postgres read models. Document the read-your-writes
  caveat: a just-created run appears in list/watch only after the sealed frontier advances past it.

Verification:

- cursor decode/encode tests
- cursor sealing/MAC or server-token tamper tests
- cursor key rotation tests
- no-skip watch cursor tests
- list/watch single-frontier consistency tests
- stale cursor-version tests
- stale store-epoch tests
- `ProjectionUnavailable` tests
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

- Add `mfm run list`.
- Add `mfm run watch --from <cursor>`.
- Preserve strict `mfm run stream <RUN_ID> --from-seq <N> --watch`.
- Define stable JSON output and compact text output.
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

- Add REST `GET /v1/runs`.
- Add REST `GET /v1/runs/watch?from=<cursor>&limit=<n>&wait_ms=<n>`.
- Keep strict status/stream routes strict.
- Define stable JSON schemas and error codes:
  `InvalidCursor`, `StaleCursorVersion`, `StaleStoreEpoch`, `ProjectionUnavailable`,
  `LimitOutOfRange`, `RunNotFound`, `StrictStatusRequired`, `WatchTimedOut`.
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
- Update `bin/cli/README.md`.
- Update `bin/rest-api/README.md`.
- Update storage crate README/runbook with fresh DB reset instructions.
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
- Production appends insert events, artifact bytes/evidence keyed by evidence hash, committed
  resource-lane claim/release/transition rows, commit-log rows, and required observation facts
  atomically.
- Logical-key admission is proven by folding authoritative `run_events`; there is no admission index.
- Resource-lane contention parks open attempts through `ResourceLaneClaimBlocked` without writing
  corrupt or terminal stream authority, and the no-deadlock single-claim invariant holds.
- Ordinary resource-lane contention does not authorize attempt/saga/run failure.
- Opaque epoch-tagged cursors plus database-owned `append_xid` assignment protect list/watch cursors;
  `reseed_store_epoch` invalidates them after restore/clone/import/rollback.
- Read models rebuild from authority rows and can be drift-checked.
- Strict status, stream, replay, and public output do not trust read models.
- CLI/REST expose one shared app list/watch API.
- CLI/REST no longer expose filesystem artifact roots.
- CLI/REST no longer depend directly on `sqlx`, Postgres storage crates, or filesystem stores.
- Filesystem storage is deleted from production.
- In-memory storage exists only behind test-only modules or test-support crates for tests that
  intentionally avoid Postgres.
- `docs/design.md`, `docs/architecture.md`, `docs/saga.md`, CLI docs, and REST docs match the
  implementation.
- No permanent compatibility shims, old aliases, or fallback paths remain.
