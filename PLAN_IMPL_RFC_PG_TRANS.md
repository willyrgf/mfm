# Implementation Plan: Postgres Transition Store Refactor

Status: implementation planning guide

Source RFC: [`RFC_REFAC_PG_TRANS.md`](RFC_REFAC_PG_TRANS.md)

## Non-Negotiables

This plan is for a breaking dev-branch cutover.

- Do not maintain backward compatibility.
- All breaking changes are allowed.
- Do not create fallbacks, compatibility shims, legacy feature flags, aliases, or hidden old paths.
- Delete old code when its replacement lands. If needed, recover old code from git history.
- Divide work per commit before executing.
- If a design decision is unclear, spawn an architect-agent to review it. Pass these same
  non-negotiables to the agent so everyone stays aligned.

Every implementation PR/branch should include an explicit commit plan before coding. Each commit
should be reviewable, have a narrow purpose, and either compile on its own or clearly state why it
is part of an agreed short-lived compile-breaking sequence. Prefer compile-clean commits.

This is not a storage-only refactor. The lane lifecycle, `PreparedCommitBundle`, artifact evidence
identity, strict-vs-observation store traits, app factory, CLI/REST contracts, schema validation, and
docs contract must land as one coherent cutover. Do not merge a partial state where runtime
authority, storage behavior, public APIs, and docs disagree.

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

- Update `docs/design.md` with the new Postgres-only production persistence contract.
- Update `docs/architecture.md` with storage boundaries, SQLx ownership, artifact authority
  surfaces, and CLI/REST dependency rules.
- Document that artifact `BYTEA` safety comes from typed/signer/keystore/transient adapter
  boundaries, not storage-side secret scanning.
- Document resource lanes as capability-declared, runtime-preflight-resolved authority acquired
  before live IO and enforced by the store without global locks.
- Update the storage README/runbook with the destructive fresh-database cutover stance.
- Mark filesystem artifact storage as removed from production architecture.
- Mark in-memory storage as test-only.
- Document the destructive dev-branch cutover posture.

Verification:

- `cargo test -p mfm-integration-tests --test architecture_namespace_contract`
- documentation review against `RFC_REFAC_PG_TRANS.md`

### Commit 2: add dependency and namespace guardrails

Scope:

- Add or update cargo metadata/architecture tests that will enforce the final boundary:
  `sqlx` only in Postgres storage crates; CLI/REST no direct concrete storage dependencies.
- Add scans for new `typed_` target table names and public `Typed` target storage/app names.
- Add scans rejecting production references to filesystem artifact storage and in-memory stores.

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
- Add resource-lane authority types and event payloads for capability-declared requirements,
  resolved claims, `ResourceLaneClaimIntent`, store-filled committed
  `ResourceLaneClaimed`/`ResourceLaneReleased`, `ResourceLaneClaimBlocked`, held lane sets, and
  holder proofs.
- Define the store-filled fencing-token protocol: prepared authority covers claim intent and fill
  policy, while final commit batch hashes cover the store-assigned lane-local token.
- Make artifact evidence identity evidence-keyed: APIs and strict reads must carry
  `(artifact_id, evidence_hash)` or exact event-derived requirements when typed meaning matters.
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
  before invocation; do not convert normal contention into attempt failure or stream corruption.
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
- Split the Postgres implementation into clear run, artifact, read-model, schema, locking, and
  rebuild modules as needed.
- Replace the old `typed_*` migration baseline with target non-`typed_` tables.
- Add append-only authority tables:
  `commits`, `run_events`, `artifact_blobs`, `artifact_admissions`,
  `run_artifact_admissions`, `commit_artifact_evidence`, `resource_lane_claim_events`,
  `resource_lane_release_events`.
- Add observation/read tables:
  `logical_key_observations`, `run_commit_log`, `run_observation_change_summaries`,
  observation facts, observation derivation tables.
- Add schema constraints/validation triggers for `event_type` source-event bindings, exact first
  artifact admission bindings, `run_commit_log` commit bindings, and lane holder ownership proofs.
- Add `store_epoch` handling that changes on destructive reset, restore, clone, import, rollback, or
  any operation that changes the PostgreSQL XID ordering domain behind public cursors.
- Add no-update/no-delete/no-truncate guards.
- Add maintenance-role-only rebuild/validation procedure boundaries.
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
  `logical_key_observations`, resource-lane claim/release rows, and `run_commit_log` atomically.
- For lane claim/release commits, require committed lane claim/release intents and holder proofs in
  the prepared bundle. Use sorted per-lane transaction locks while admitting those commit-bound
  rows, assign store-filled lane-local fencing tokens under the lock, and return
  `ResourceLaneClaimBlocked` without appending rows for valid contention. Do not expose standalone
  lane acquire/release APIs, derive lanes in storage, or add a global commit-order lock, global
  resource-lane lock, global counter row, table-level append serialization, or `commit_pos`/
  `change_pos` allocation.
- Enforce batch-hash integrity structurally in SQL and semantically in Rust from persisted canonical
  bytes; do not create a SQL canonicalization engine.
- Build logical-key admission from authoritative `run_events`; use `logical_key_observations` only
  as non-authoritative hints unless a prefix-completeness proof is implemented and verified.
- Insert `run_commit_log.append_xid` from PostgreSQL `pg_current_xact_id()` and use it only for
  snapshot-sealed observation cursors.
- Remove mutable helper writes: no run-head updates, no unique logical-key upserts, no current-state
  table updates.

Verification:

- same-run concurrent append tests
- cross-run resource-lane contention tests
- valid lane contention returns `ResourceLaneClaimBlocked` and leaves the attempt open before
  invocation
- independent cross-run append tests proving unrelated runs do not block each other
- crash after lane-bearing `StateAttemptStarted` but before `ResourceLaneClaimed` retries pure
  preflight and claim admission or interrupts/fails without live IO
- crash after `ResourceLaneClaimed` but before invocation preparation recovers the same committed
  held lane or releases it through a certified cleanup/interruption commit
- Postgres rejects prepared invocation resource keys without a committed held lane
- Postgres rejects prepared invocation keys that differ from preflight claims
- SQL/code scans prove no global resource-lane table lock or single `global` lock row remains
- idempotent commit-key retry tests
- artifact mismatch/missing/extra-byte rejection tests
- snapshot-sealed watch cursor tests with older slow transactions and newer fast transactions
- SQL scan showing no production domain `UPDATE`, `DELETE`, or `TRUNCATE`

### Commit 8: implement strict Postgres load and artifact authority reads

Scope:

- Load run streams from `commits`/`run_events` and cross-check:
  event count, ordinals, commit key, commit position, payload hash, batch hash, and request hash.
- Load retained artifacts from Postgres through event-derived requirements.
- Implement adapter/public-output artifact read surfaces over Postgres with proof-bearing requests.
- Reject artifact id/digest/byte length/evidence hash inconsistencies.

Verification:

- strict stream load corruption tests
- retained artifact read tests
- public-output authority artifact tests
- adapter artifact request tests

### Commit 9: implement logical-key observation rebuild parity

Scope:

- Fold authoritative `run_events` into `LogicalKeySet` and `UniqueLogicalPayloads` for admission.
- Keep `logical_key_observations` as rebuildable, non-authoritative hints unless a reviewed
  prefix-completeness proof is added.
- Preserve the recoverable submission-result exception through explicit observation fields.
- Prove parity with folding the authoritative event stream.

Verification:

- accepted recoverable submission-result rewrite test
- rejected repeated unique logical-key tests
- parity tests comparing observation fold to stream fold
- tests proving missing observation rows cannot make admission believe a logical key is absent

### Commit 10: implement Postgres-owned read models

Scope:

- Implement mechanical SQL facts only for scalar/canonical-byte derivations.
- Persist only mechanical observation rows derived from scalar/canonical-byte authority columns in
  the initial implementation. Do not call runtime/app semantic projection code from storage.
- Implement `run_observation_change_summaries` separate from immutable `run_commit_log`.
- Add `current_*` views over observation facts.
- Add `build_projection_version`, validate, and drift tooling behind maintenance role boundaries.

Verification:

- observation facts rebuild exactly from authority rows
- SQL-only derivations are mechanical
- observation rows produced by authority-layer projection code match strict projection rebuilds
- corrupt observation facts do not affect strict resume/replay/public-output reads

### Commit 11: expose app-level list/watch/read APIs

Scope:

- Add app-level `list_runs`, `poll_run_changes`, `watch_run_changes`, `run_status`, and
  `run_stream` APIs.
- Implement opaque cursor encoding over `(run_commit_log.append_xid, run_commit_log.commit_sort_key)`
  with cursor format version and store epoch.
- Keep `append_xid`, `commit_sort_key`, and PostgreSQL transaction ids out of public JSON/text
  output; expose opaque cursors/high-watermarks and domain observation ids instead.
- Keep `run_status` and `run_stream` strict authority reads.
- Keep list/watch as observation reads over Postgres read models.

Verification:

- cursor decode/encode tests
- no-skip watch cursor tests
- stale cursor-version tests
- stale store-epoch tests
- `ProjectionUnavailable` tests
- strict stream watch tests proving authority stream reads

### Commit 12: refactor app construction around production Postgres factory

Scope:

- Make app services generic over run store, observation store, and narrow artifact read surfaces.
- Add one production factory that composes the Postgres run/artifact/read-model surfaces.
- Remove app construction that accepts filesystem artifact stores for production.
- Remove direct concrete storage wiring from CLI/REST paths.

Verification:

- app tests compile without filesystem artifact store production dependencies
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

- Remove `crates/storages/artifact-store-fs` from production workspace usage.
- Delete production references, docs, examples, and dependency entries.
- Replace tests that need non-Postgres artifacts with narrow in-memory test helpers.
- Remove old artifact root docs and environment variables.

Deletion requirement:

- Do not keep the filesystem store as a fixture crate unless it is completely outside production and
  explicitly justified. Prefer in-memory test helpers.

Verification:

- `rg "artifact-store-fs|FsTypedArtifactStore|typed-artifact-root|MFM_TYPED_ARTIFACT_ROOT"`
  returns no production hits
- workspace compile

### Commit 16: remove old names and compatibility surfaces

Scope:

- Remove old public `Typed*` storage/app names that were replaced by target names.
- Remove old `typed_*` SQL references from production migrations and queries.
- Remove old dependency allowlist exceptions.
- Remove compatibility aliases and dead modules.

Verification:

- namespace scans
- cargo metadata contract
- architecture namespace contract

### Commit 17: update docs and runbooks for the final user surface

Scope:

- Finalize `docs/design.md` and `docs/architecture.md` after code lands.
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
  resource-lane claim/release rows, logical-key observation hints, commit-log rows, and required
  observation facts atomically.
- Logical-key admission is proven from authoritative `run_events`, not from observation-row absence.
- Resource-lane contention parks open attempts through `ResourceLaneClaimBlocked` without writing
  corrupt or terminal stream authority.
- Read models rebuild from authority rows and can be drift-checked.
- Strict status, stream, replay, and public output do not trust read models.
- CLI/REST expose one shared app list/watch API.
- CLI/REST no longer expose filesystem artifact roots.
- CLI/REST no longer depend directly on `sqlx`, Postgres storage crates, or filesystem stores.
- Filesystem storage is deleted from production.
- In-memory storage exists only for tests that intentionally avoid Postgres.
- `docs/design.md`, `docs/architecture.md`, CLI docs, and REST docs match the implementation.
- No permanent compatibility shims, old aliases, or fallback paths remain.
