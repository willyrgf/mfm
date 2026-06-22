# RFC: Postgres Transition Store Refactor

Status: proposed

Compatibility posture: breaking dev-branch refactor.

This RFC intentionally does not preserve backward compatibility with the current `typed_*`
Postgres schema, filesystem artifact roots, CLI flags, REST environment variables, storage type
names, or old local development data. Existing development databases and artifact directories are
disposable for this transition.

Cutover is destructive: developers must drop/recreate the affected database or schema and rerun the
new Postgres storage migrations. The implementation must not provide mixed-version operation,
dual-writes, best-effort backfill, automatic conversion from the current `typed_*` tables, or
compatibility aliases that keep the old storage model alive.

If non-disposable production data appears before this refactor lands, this RFC is blocked until a
separate export/import or historical-inspection plan is written and reviewed.

## Summary

MFM should converge on one production persistence system: Postgres, accessed only through `sqlx`
inside the storage implementation crate.

The target store is an append-only transition store. Production code never updates, deletes, or
truncates MFM domain rows. It only inserts committed facts. The latest state is a read concern,
derived from history through Postgres-owned views, storage-derived read facts, or rebuildable cache
surfaces.

The core authority remains strict: committed run events and artifact evidence are the source of
truth. Postgres-owned read models exist to make platform reads, lists, watches, dashboards, and
search practical. They are never semantic authority for resume, replay, public-output rendering,
side-effect admission, or run completion.

Target naming intentionally avoids the current `typed_` table prefix and avoids carrying `Typed`
into new/refactored public storage/app structures. The typed-core design remains the semantic
foundation, but the persisted schema and public storage APIs should use stable domain names such as
`run_events`, `RunEventStore`, and `RunReadModel`.

## Problems

### 1. Postgres Access Is Not Yet The Only Production Storage Boundary

Run events already have a Postgres implementation, but artifact bytes are currently stored by a
filesystem artifact store in production CLI/REST paths. That creates a split-brain persistence
model:

- Postgres can contain committed event evidence.
- The filesystem can independently lose or corrupt artifact bytes.
- Backup/restore is no longer one operation.
- Replay and public-output reads can fail even when the run stream exists.

The production system should not have a separate filesystem artifact store. The only accepted
non-Postgres persistence exception is a narrow in-memory test store for tests that deliberately do
not spin up Postgres.

### 2. Current Postgres Tables Are Not Strict Transition Tables

The current event-store schema includes mutable helper tables such as run heads and unique
logical-key payload state. Current code updates or upserts those helper rows during append.

That is inconsistent with the desired transition-table rule:

- no production `UPDATE`
- no production `DELETE`
- no production `TRUNCATE`
- no mutable "current" row as persistence authority
- latest state comes from deriving over inserted history

### 3. There Is No Single Platform Read/Watch API For Run Evolution

MFM has per-run status and stream reads, but it does not yet expose one public platform API for:

- active runs
- historical runs
- run evolution over time
- durable watch cursors
- operational dashboards

This API should exist once and be backed by app services over storage traits. CLI and REST should
not implement separate read logic.

### 4. Read Models Must Be Fast Without Becoming Authority

Strict event-log reads are clean, but platform reads can become expensive if every list/watch call
folds many streams in Rust.

The desired hybrid is:

- strict append-only authority, like an event log
- Postgres-owned read models for efficient platform reads
- every read-model row provably derived from committed events
- every read model rebuildable from authority rows
- no app/framework direct writes into read-model tables

## Goals

- Standardize all production MFM persistence on Postgres.
- Standardize all production Postgres access through `sqlx` in storage crates only.
- Make all MFM domain tables insert-only transition tables.
- Move artifact bytes and artifact evidence into Postgres.
- Remove filesystem storage from production and from the main architecture.
- Keep only a narrow in-memory test store exception for tests that intentionally avoid Postgres.
- Add a single app-level read/watch API shared by CLI and REST.
- Put read-model derivation in the Postgres storage layer.
- Prove read models are derived from committed events and can be rebuilt.
- Preserve runtime/store authority boundaries from `docs/design.md`.
- Reduce storage alternatives and overall storage-specific LOC over time.

## Non-Goals

- Do not move private keys, mnemonics, passwords, signer runtime sources, raw signed transactions,
  or other secret-bearing data into the MFM run database.
- Do not replace the Rust store admission/projection proof logic with PL/pgSQL semantic admission.
- Do not make materialized views semantic authority.
- Do not silently migrate uncertified historical runs into certified run history.

Artifact bytes moving into Postgres does not weaken the secret boundary. Artifact admission must
still reject or prevent known secret-bearing classes from entering the run database, including
private keys, mnemonics, passwords, signer runtime sources, raw signed transactions, unredacted
transport credentials, and unredacted diagnostic payloads. SQL/API errors for artifact insertion,
verification, rebuild, and read paths must remain redacted.

## Naming Rules

Target table names must not use the `typed_` prefix. Target Rust storage/app/public structures
introduced or refactored by this work should not use a `Typed` prefix.

Preferred target names:

- `commits`, not `typed_commits`
- `run_events`, not `typed_run_events`
- `artifact_blobs`, not `typed_artifact_blobs`
- `artifact_admissions`, not `typed_artifacts`
- `run_artifact_admissions`, not `typed_run_artifacts`
- `run_read_facts`, not `typed_run_read_models`
- `run_commit_log`, not `typed_run_change_log`
- `run_change_summaries`, not `typed_run_change_log`
- `RunEventStore`, not `TypedRunEventStore`
- `PostgresRunStore`, not `PostgresTypedRunEventStore`
- `RunReadModelStore`, not `TypedRunReadModelStore`
- `RunServices`, not `TypedRunServices`

Current code may still contain old names while this refactor is being implemented. The final cutover
should remove production old-name surfaces rather than keeping compatibility aliases. New RFC text,
new schemas, new public APIs, and refactored storage abstractions should use the target names.

## Proposed Architecture

The storage architecture has three layers.

### 1. Authority Layer

Authority tables are append-only. These rows are the durable facts from which MFM can rebuild
history.

Proposed authority tables:

```text
commits
run_events
artifact_blobs
artifact_admissions
run_artifact_admissions
commit_artifact_evidence
logical_key_observations
```

`commits` records one accepted commit for one run sequence.

Important columns:

```text
commit_pos BIGINT NOT NULL
run_id TEXT NOT NULL
seq BIGINT NOT NULL
commit_key TEXT NOT NULL
commit_purpose TEXT NOT NULL
commit_fingerprint TEXT NOT NULL
prepared_request_hash TEXT NOT NULL
prepared_request_canonical_json BYTEA NOT NULL
commit_batch_hash TEXT NOT NULL
event_count INTEGER NOT NULL
committed_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp()
```

Constraints:

```text
PRIMARY KEY (run_id, seq)
UNIQUE (commit_pos)
UNIQUE (run_id, commit_key)
UNIQUE (run_id, seq, commit_key, commit_pos)
CHECK (seq >= 1)
CHECK (event_count >= 1)
```

`commit_pos` is the durable global order for committed MFM run commits. It is unique and
monotonic, but it is not required to be gapless. It must be assigned only while holding the global
commit-order serialization point described in the write flow. Plain PostgreSQL identity or sequence
values are not valid public watch cursors unless allocation is serialized this way and the
serialization point is held until the transaction commits. `committed_at` is audit/display data, not
ordering authority.

`commit_purpose` is the stable purpose tag for the prepared commit variant. `prepared_request_hash`
and `prepared_request_canonical_json` persist the canonical request material needed to reverify the
commit fingerprint. The persisted canonical request must bind at least the run id, commit key,
purpose, payloads, preconditions, required artifact evidence, and admitted artifact evidence. It
should not include `expected_next_seq`; otherwise a valid idempotent retry after the first commit
cannot be recognized. Artifact byte payloads do not need to be included directly if their evidence
binds digest and byte length and the store verifies bytes before insert.

`commit_fingerprint` is the idempotency digest over the canonical prepared request material. Because
the canonical request bytes are persisted, strict load/rebuild tooling can reverify the fingerprint
instead of treating it as append-time-only metadata.

`commit_batch_hash` binds the commit row to the stored ordered event batch and commit artifact
evidence. It must be computed from canonical MFM bytes over the resulting event envelopes and
artifact evidence bindings. The implementation must enforce before commit, either with Rust
validation plus a deferrable constraint trigger or with equivalent database checks, that:

- event ordinals for a commit are contiguous `0..event_count-1`
- every event row for `(run_id, seq)` carries the commit row's `commit_key` and `commit_pos`
- the number of inserted event rows equals `event_count`
- the ordered event rows rebuild `commit_batch_hash`
- commit artifact evidence rows are bound to the same `(run_id, seq, commit_key)`

Strict load paths must also cross-check these invariants so database corruption is detected before
authority objects are minted.

`run_events` records store-owned event envelopes.

Important columns:

```text
commit_pos BIGINT NOT NULL
run_id TEXT NOT NULL
seq BIGINT NOT NULL
ordinal INTEGER NOT NULL
event_id TEXT NOT NULL
event_schema_id TEXT NOT NULL
spec_hash TEXT NOT NULL
commit_key TEXT NOT NULL
logical_key TEXT NOT NULL
payload_hash TEXT NOT NULL
payload_canonical_json BYTEA NOT NULL
payload_json JSONB NOT NULL
```

Constraints:

```text
PRIMARY KEY (run_id, seq, ordinal)
UNIQUE (commit_pos, ordinal)
UNIQUE (run_id, event_id)
FOREIGN KEY (run_id, seq, commit_key, commit_pos)
  REFERENCES commits(run_id, seq, commit_key, commit_pos)
  ON DELETE RESTRICT
CHECK (seq >= 1)
CHECK (ordinal >= 0)
```

The store must keep validating persisted rows by reconstructing event identity, payload hash,
schema id, spec hash, logical key, ordering, and commit grouping through the Rust store contract.
Global event order is `(commit_pos, ordinal)`. A generated identity event column may exist as a
debug surrogate, but it must not drive public watch cursors or semantic stream order.

`payload_json` is a query copy only. `payload_hash`, `commit_fingerprint`, `commit_batch_hash`, and
read-model row hashes must be computed from MFM canonical JSON bytes, not from `jsonb::text` or any
Postgres JSONB serialization. Hashed structured data must continue to reject floats. The initial
implementation should store `payload_canonical_json` so strict load paths and rebuild tools can
verify the query copy and hashes without trusting JSONB formatting.

`artifact_blobs` stores immutable content-addressed bytes.

Important columns:

```text
artifact_id TEXT PRIMARY KEY
digest TEXT NOT NULL
byte_len BIGINT NOT NULL
bytes BYTEA NOT NULL
inserted_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp()
```

Constraints:

```text
UNIQUE (digest)
UNIQUE (artifact_id, digest)
UNIQUE (artifact_id, digest, byte_len)
CHECK (byte_len >= 0)
CHECK (octet_length(bytes) = byte_len)
```

PostgreSQL documents a 1 GB field-size limit in its current limits appendix. MFM should still
enforce a lower application/schema artifact-byte limit before insertion, because practical limits
such as memory pressure, backup/restore cost, WAL volume, and query latency will matter before the
hard database limit. The exact MFM artifact-byte limit should be a configuration/schema constant
validated before `artifact_blobs` insertion and repeated as a database `CHECK`.

Blob rows are byte authority only. Typed artifact meaning belongs in evidence/admission rows, not in
mutable or competing blob metadata.

`artifact_id` must be derived from `digest` using the canonical MFM artifact identity algorithm.
The storage layer must verify this before insertion, and the schema should enforce it with a
migration-owned check function if the textual id/digest encoding makes that practical. Strict load
paths must reject any blob row where `artifact_id`, `digest`, `byte_len`, and `octet_length(bytes)`
do not mutually verify.

`artifact_admissions` stores the full typed artifact evidence shape:

```text
evidence_hash TEXT PRIMARY KEY
artifact_id TEXT NOT NULL REFERENCES artifact_blobs(artifact_id) ON DELETE RESTRICT
digest TEXT NOT NULL
byte_len BIGINT NOT NULL
media_type TEXT NOT NULL
schema_id TEXT NULL
semantic_type_id TEXT NULL
producer_node_id TEXT NULL
producer_seed_id TEXT NULL
artifact_role TEXT NOT NULL
evidence_canonical_json BYTEA NOT NULL
inserted_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp()
```

Constraints:

```text
UNIQUE (artifact_id, evidence_hash)
UNIQUE (evidence_hash, artifact_id, digest, byte_len)
FOREIGN KEY (artifact_id, digest, byte_len)
  REFERENCES artifact_blobs(artifact_id, digest, byte_len)
  ON DELETE RESTRICT
CHECK (byte_len >= 0)
CHECK (producer_node_id IS NULL OR producer_seed_id IS NULL)
```

The evidence hash is canonical MFM evidence, not a hash of JSONB text. The evidence row must verify
against the blob artifact id, digest, and byte length before insertion. The evidence canonical bytes
must contain the same `artifact_id`, `digest`, and `byte_len` stored in the relational columns, and
strict load paths must recompute `evidence_hash` from `evidence_canonical_json`.

`run_artifact_admissions` and `commit_artifact_evidence` bind evidence to run/commit authority:

```text
run_artifact_admissions
  run_id TEXT NOT NULL
  artifact_id TEXT NOT NULL
  evidence_hash TEXT NOT NULL
  first_commit_pos BIGINT NOT NULL
  first_seq BIGINT NOT NULL
  first_commit_key TEXT NOT NULL

commit_artifact_evidence
  run_id TEXT NOT NULL
  seq BIGINT NOT NULL
  commit_key TEXT NOT NULL
  commit_pos BIGINT NOT NULL
  artifact_id TEXT NOT NULL
  evidence_hash TEXT NOT NULL
  binding_kind TEXT NOT NULL CHECK (binding_kind IN ('required', 'admitted'))
  requirement_source TEXT NULL
```

Both binding tables must use composite foreign keys that bind the artifact id to the exact evidence:

```text
FOREIGN KEY (artifact_id, evidence_hash)
  REFERENCES artifact_admissions(artifact_id, evidence_hash)
  ON DELETE RESTRICT
```

`run_artifact_admissions` must also bind the first admission to the commit that admitted it:

```text
FOREIGN KEY (run_id, first_seq, first_commit_key, first_commit_pos)
  REFERENCES commits(run_id, seq, commit_key, commit_pos)
  ON DELETE RESTRICT
```

`commit_artifact_evidence` must bind each requirement/admission row to its commit:

```text
FOREIGN KEY (run_id, seq, commit_key, commit_pos)
  REFERENCES commits(run_id, seq, commit_key, commit_pos)
  ON DELETE RESTRICT
```

The required/admitted distinction is part of commit authority. It must be possible to rebuild the
commit fingerprint and retained artifact requirements from inserted commit artifact evidence plus
the committed events.

`logical_key_observations` replaces mutable logical-key helper tables with an append-only authority
index. It records exactly what each committed event contributed to logical-key admission.

Important columns:

```text
run_id TEXT NOT NULL
logical_key TEXT NOT NULL
commit_pos BIGINT NOT NULL
seq BIGINT NOT NULL
ordinal INTEGER NOT NULL
event_id TEXT NOT NULL
payload_hash TEXT NOT NULL
is_unique BOOLEAN NOT NULL
unique_observation_kind TEXT NULL
previous_payload_hash TEXT NULL
rewrite_reason TEXT NULL
```

Constraints:

```text
PRIMARY KEY (run_id, logical_key, seq, ordinal)
UNIQUE (run_id, seq, ordinal)
FOREIGN KEY (run_id, seq, ordinal)
  REFERENCES run_events(run_id, seq, ordinal)
  ON DELETE RESTRICT
CHECK (
  (is_unique = FALSE AND unique_observation_kind IS NULL)
  OR (is_unique = TRUE AND unique_observation_kind IS NOT NULL)
)
```

Fold order is `(seq, ordinal)`.

- `LogicalKeySet` is rebuilt by collecting every distinct `(run_id, logical_key)`.
- `UniqueLogicalPayloads` is rebuilt by folding only `is_unique = TRUE` rows.
- Attempt-scoped logical keys remain non-unique observations.
- A repeated unique logical key with the same payload hash is invalid.
- A repeated unique logical key with a different payload hash is invalid unless Rust admission
  proves the recoverable submission-result exception against the projection prefix before that
  event.

The recoverable submission-result exception must be explicit in the observation row:

```text
unique_observation_kind = 'recoverable_rewrite'
rewrite_reason = 'submission_result_unknown_recovery'
previous_payload_hash = <hash being superseded>
```

SQL may mechanically insert observations from event envelopes, but it must not authorize the
recoverable rewrite. The Rust staging path remains responsible for applying the current
`submission_result` rule against authority rows and the projection prefix. Parity tests must prove
that folding `logical_key_observations` produces the same `LogicalKeySet` and
`UniqueLogicalPayloads` as folding the authoritative event stream, including accepted and rejected
recoverable submission-result cases.

### 2. Admission And Proof Layer

Rust remains the semantic proof engine.

Read facts must not feed semantic append admission. The storage implementation may use indexes,
change-log rows, or read-model rows only as non-authoritative hints to locate candidate authority
rows. Any hinted data must be reloaded or revalidated against authority rows before it can influence
append admission, side-effect legality, resource-lane claims, resume, replay, or public-output
authority.

The commit base used by Rust staging must be built from:

- `commits`
- `run_events`
- artifact admission/evidence authority
- append-only logical-key observations
- Rust projections rebuilt or verified from those authority rows inside the same append path

A semantic append must pass through the Rust store staging path:

```text
PreparedCommitBundle
  -> load authority rows and non-authoritative index hints needed for the base
  -> verify pending artifact bytes against admitted evidence
  -> Rust stage/validate commit
  -> insert authority rows
  -> Postgres storage layer records mechanical and Rust-derived read facts
  -> commit transaction
```

The database must not become a second implementation of the runtime/state-machine semantics.
Specifically, SQL triggers must not replace:

- prepared commit purpose validation
- side-effect ledger legal transition checks
- saga terminal proof validation
- manual-resolution proof validation
- replay/public-output authority construction
- certified spec verification

Those remain Rust/kernel/runtime/app responsibilities.

`PreparedCommitBundle` is a breaking replacement or extension of the current plan-only append
surface. It contains the prepared commit plan plus the artifact byte payloads admitted by that plan.
The store must reject missing, extra, or mismatched bytes; it may reuse an existing blob only when
the existing blob and evidence are byte-for-byte/evidence-identical. Production code should not
stage artifact bytes in a filesystem store before append. The only accepted non-Postgres staging
exception remains narrow in-memory test code.

This requires an explicit kernel/runtime/store API cutover:

- `mfm-store` defines `PreparedCommitBundle` as the only production append input.
- `AsyncTypedRunEventStore::append_prepared_commit_plan` is replaced by
  `append_prepared_commit_bundle`.
- `PreparedCommitBundle` carries `PreparedCommitPlan` plus `PreparedArtifactBytes { bytes,
  evidence }` for every admitted artifact that is not already present with identical evidence.
- `PreparedCommitBundle` construction verifies artifact bytes against evidence before the store is
  called; the store verifies again inside the transaction before inserting authority rows.
- `mfm-runtime` commit planning returns bundles or bundle builders for every commit path: run
  admission, attempt start, attempt terminal, side-effect progress, side-effect terminal,
  framework lifecycle output, retention, manual resolution, saga terminal, recovery, and observed
  failure diagnostic commits.
- `RuntimeArtifactStager` and helper functions that stage bytes before append are removed from
  production. Tests may keep in-memory bundle helpers.
- Recovery and resume paths must never append a plan without the artifact bytes required by newly
  admitted evidence.

### 3. Postgres-Owned Read-Model Layer

Read models live in Postgres and are owned by the Postgres storage layer. Ownership may be
implemented by storage migrations, SQL functions/views/triggers, or Rust projection code inside the
Postgres storage crate writing canonical read facts in the same append transaction. App/framework
code does not write these rows directly.

SQL must not become a second semantic projection engine. SQL derivation is limited to mechanical
facts that can be copied or deterministically reshaped from committed scalar/canonical-byte columns
without interpreting certified runtime policy. Any read fact that requires `ProjectionSnapshot`,
certified saga policy, side-effect legality, manual-resolution proof state, retention authority, or
public-output authority must be emitted by the shared Rust projection/admission code inside the
storage transaction, then stored in Postgres with provenance and rebuild support.

Read models are for:

- run list
- run watch
- operational dashboards
- latest run summary
- latest attempt summary
- latest public output summary
- resource-lane observation
- change feeds and cursors

Recommended read fact tables:

```text
run_read_facts
attempt_read_facts
cell_read_facts
side_effect_read_facts
public_output_read_facts
retention_read_facts
run_commit_log
run_change_summaries
projection_derivations
```

Read facts have two provenance classes.

Delta facts derived mechanically from one committed event carry event provenance:

```text
projection_version TEXT NOT NULL
source_run_id TEXT NOT NULL
source_seq BIGINT NOT NULL
source_ordinal INTEGER NOT NULL
source_commit_pos BIGINT NOT NULL
source_event_id TEXT NOT NULL
source_payload_hash TEXT NOT NULL
source_event_schema_id TEXT NOT NULL
derived_key TEXT NOT NULL
derived_row_canonical_json BYTEA NOT NULL
derived_row_hash TEXT NOT NULL
```

The provenance tuple must reference the source event:

```text
FOREIGN KEY (source_run_id, source_seq, source_ordinal)
  REFERENCES run_events(run_id, seq, ordinal)
  ON DELETE RESTRICT
```

Aggregate summaries are not one-event facts. A current run summary, saga mode, latest state,
attempt summary, public-output summary, or dashboard row depends on a stream prefix, a commit
prefix, or multiple source events. Those rows must either remain views over event-sourced delta
facts or carry aggregate provenance:

```text
projection_version TEXT NOT NULL
source_kind TEXT NOT NULL CHECK (source_kind IN ('run_prefix', 'global_prefix', 'multi_event'))
source_run_id TEXT NULL
source_from_seq BIGINT NULL
source_to_seq BIGINT NULL
source_last_ordinal INTEGER NULL
source_high_commit_pos BIGINT NOT NULL
source_event_count BIGINT NOT NULL
source_input_hash TEXT NOT NULL
derived_key TEXT NOT NULL
derived_row_canonical_json BYTEA NOT NULL
derived_row_hash TEXT NOT NULL
```

Sparse multi-source or dashboard rows must additionally write `projection_derivation_sources` rows
that reference each source event, or use a documented prefix hash chain that proves the same source
set. A single `(source_run_id, source_seq, source_ordinal)` tuple is only sufficient for one-event
delta facts.

Current-state views are views over inserted facts, for example:

```text
current_runs
current_attempts
current_cells
current_side_effect_ledgers
current_public_outputs
current_resource_lanes
```

These views use run sequence, source ordinal, `commit_pos`, and aggregate high-watermarks to select
the latest fact. They must not be written by app code.

## Trigger And View Strategy

Start with the simplest derivations as SQL views:

- run admission summary
- run completion summary
- run head by `max(seq)`
- public output produced/render-failed summary
- change feed by `run_commit_log.change_pos`

Use trigger-derived insert-only read facts when:

- a query is too expensive as a pure view
- the derivation is mechanical from one committed event, or aggregate provenance is explicitly
  recorded
- the row can carry event or aggregate provenance
- rebuild functions can recreate the same facts from authority rows

Do not implement saga mode, side-effect ledger legality, manual-resolution state, public-output
authority, or retention proof as independent PL/pgSQL projections. Those are semantic projections
owned by Rust/kernel/runtime code. If list/watch needs those fields, the Postgres storage crate must
call the shared Rust projection code while appending and insert the resulting canonical read facts
inside the same transaction, with rebuild tooling that re-runs the same Rust projection over
authority rows.

Trigger-derived read facts must not hash Postgres `jsonb::text`. Each hash-bearing fact must define
a canonical row schema and canonicalizer identity. The implementation must either provide a
database-side canonicalizer tested against `mfm-canonical`, or store canonical row bytes produced by
the storage layer's canonicalization code and have Postgres derive only from those canonical bytes
and scalar columns. If neither is true for a candidate model, that model must stay a view or cache
without semantic hash authority until rebuild validation is available.

Use materialized views only as rebuildable caches for expensive dashboards. They must live in a
clearly cache-like namespace and must never be used for admission, resume, replay, or public-output
authority.

`LISTEN/NOTIFY` is only a wakeup mechanism. Watch clients must resume from `run_commit_log` or a
durable cursor and never trust notifications as durable data.

## Durable Change Log And Cursors

`run_commit_log` is the durable cursor source for run list/watch observation APIs. It is insert-only
and atomic with the authority rows for the same commit. It is not projection-versioned and does not
store presentation summaries. This separation lets new projection versions insert rebuilt summaries
without mutating cursor authority or conflicting with old cursor rows.

Required columns:

```text
change_pos BIGINT PRIMARY KEY
run_id TEXT NOT NULL
commit_pos BIGINT NOT NULL REFERENCES commits(commit_pos) ON DELETE RESTRICT
seq BIGINT NOT NULL
first_ordinal INTEGER NOT NULL
last_ordinal INTEGER NOT NULL
event_count INTEGER NOT NULL
commit_batch_hash TEXT NOT NULL
created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp()
```

The initial API should use one commit-log row per committed run commit. `change_pos` may equal
`commit_pos`, or it may be a separate value assigned under the same global commit-order
serialization point. If a later design emits multiple changes per commit, the cursor must become
`(commit_pos, change_ordinal)` or `change_pos` must still be assigned under the same global
serialization point before commit.

Projection-versioned summaries live in `run_change_summaries`:

```text
projection_version TEXT NOT NULL
change_pos BIGINT NOT NULL REFERENCES run_commit_log(change_pos) ON DELETE RESTRICT
summary_kind TEXT NOT NULL
summary_row_hash TEXT NOT NULL
summary_row_canonical_json BYTEA NOT NULL
PRIMARY KEY (projection_version, change_pos, summary_kind)
```

Rebuilding a new projection version inserts new summary rows for the same immutable
`run_commit_log.change_pos` values. It must not update old summaries or rewrite cursor authority.
If a requested projection version is unavailable for a visible change, the API must either return a
documented `ProjectionUnavailable` response or run an explicit rebuild path before returning data.

Public watch cursors are opaque exclusive lower-bound cursors over `run_commit_log.change_pos`.
They must include a cursor format version and may include a cursor epoch if the cursor contract
itself changes. Projection version is response/query metadata, not cursor authority. The service
must reject stale incompatible cursor-format versions or explicitly migrate them; it must never
silently continue a cursor across incompatible cursor contracts.

`poll_run_changes(cursor, limit)` returns rows strictly greater than the decoded cursor, ordered by
`change_pos ASC`. If rows are returned, `next_cursor` is the last returned `change_pos`. If no rows
are returned, the response includes the latest visible high-watermark and may advance the cursor to
that high-watermark because `change_pos` is commit-order serialized. Clients get no-skip delivery
when they persist the returned cursor only after processing the page. Duplicate delivery is allowed
after client retries, so change records must be idempotent for consumers.

`watch_run_changes` must poll durable rows before and after waiting. `LISTEN/NOTIFY` can only wake
the waiter; it must not be treated as the data source and may be missed or coalesced.

## Write Flow

The append flow should be:

1. Begin transaction.
2. Check `(run_id, commit_key)` for idempotency before stale sequence checks.
3. Take a transaction-scoped advisory lock for `run_id`.
4. Re-check `(run_id, commit_key)` after acquiring the lock.
5. Take a conservative resource-lane advisory lock. Start with one global lock; refine to sorted
   per-lane locks only after extraction and contention tests are strong.
6. Load committed event history, artifact authority, logical-key authority, and non-authoritative
   index hints needed to build the Rust commit base.
7. Stage and validate the prepared commit bundle in Rust, including artifact byte/evidence checks.
8. Acquire the global commit-order serialization point before assigning `commit_pos`, `change_pos`,
   or any other global cursor/order position. This can be a transaction-scoped advisory lock taken
   before sequence allocation, or another transactional mechanism that serializes allocation and is
   held until commit. This serialization point is independent of resource-lane locking and must
   remain even if resource-lane locking is later refined to per-lane locks.
9. Insert `commits`.
10. Insert `artifact_blobs` and artifact evidence/admission rows as needed.
11. Insert `run_events`.
12. Insert or mechanically derive `logical_key_observations`.
13. Insert `run_commit_log`.
14. Let the Postgres storage layer insert mechanical SQL facts and Rust-derived canonical read facts,
    including projection-versioned `run_change_summaries`.
15. Emit `NOTIFY` after authority rows, read facts, commit-log rows, and summary rows are inserted.
16. Commit.

If synchronous trigger derivation fails, the whole transaction must roll back. That preserves
atomicity between authority rows and read-model facts.

## Read Flow

There are two classes of reads.

### Strict Authority Reads

These must load committed event rows and verified artifact evidence, then rebuild/verify through
Rust authority:

- resume
- replay
- public-output rendering
- strict per-run status
- retention proof
- side-effect recovery
- manual-resolution authority

Read models may be used as hints or for quick existence checks, but they cannot authorize these
operations.

### Platform Observation Reads

These may read Postgres-owned read models:

- run list
- run watch
- dashboard summaries
- active/historical run filtering
- run evolution pages

Public responses must make the distinction clear. A list/watch summary is operational observation;
strict status remains available per run when a verified semantic view is required.

## Public API

Add one shared app-level API used by CLI and REST.

App-facing methods:

```text
list_runs(query) -> RunListResponse
poll_run_changes(cursor, limit) -> RunChangePage
watch_run_changes(cursor, options) -> RunChangePage or stream
run_status(run_id) -> RunResponse
run_stream(run_id, range) -> RunStreamResponse
```

CLI:

```text
mfm run list
mfm run watch --from <cursor>
mfm run stream <RUN_ID> --from-seq <N> --watch
```

REST:

```text
GET /v1/runs
GET /v1/runs/watch?from=<cursor>&limit=<n>&wait_ms=<n>
GET /v1/runs/:run_id/status
GET /v1/runs/:run_id/stream?from_seq=<n>&to_seq=<n>
```

These are public contracts. The implementation must define stable response structs, JSON schemas,
text output, and error codes before the routes/commands are considered complete.

Minimum list/watch JSON fields:

```text
cursor
next_cursor
cursor_version
projection_version
high_watermark
runs[]
changes[]
```

Minimum run summary fields:

```text
run_id
head_seq
commit_pos
status_kind
run_mode
started_at
updated_at
completed_at
public_output_summary
attempt_summary
```

Stable public error codes must include at least:

```text
InvalidCursor
StaleCursorVersion
ProjectionUnavailable
LimitOutOfRange
RunNotFound
StrictStatusRequired
WatchTimedOut
```

CLI text output should be compact and line-oriented for list/watch, while JSON output must use the
same stable schema as REST where practical. The implementation must update `bin/cli/README.md`,
`bin/rest-api/README.md`, and contract tests in the same change that introduces these commands and
routes.

The watch cursor is opaque publicly. Internally it encodes `run_commit_log.change_pos` as an
exclusive lower bound plus a cursor format version. Projection version is response/query metadata,
not cursor authority. The cursor must not encode plain identity sequence values that are allocated
outside the global commit-order serialization point.

`mfm run watch` and `GET /v1/runs/watch` are observation APIs backed by `run_commit_log`,
`run_change_summaries`, and other Postgres-owned read models.

`mfm run stream <RUN_ID> --watch` is a strict authority stream tail, not a read-model watch. It must
emit data loaded from the authoritative run stream and ordered by `(seq, ordinal)`. It may use
`run_commit_log` or `LISTEN/NOTIFY` only as wakeups before re-reading the authoritative stream. The
stream cursor is run-local: `from_seq` is inclusive. After emitting a complete batch, the next watch
read starts at `head_seq + 1`. If event-level limits are introduced, pages must not split a commit
unless the cursor is extended to `(seq, ordinal)`.

`run_status(run_id)` remains a strict authority read by default. List/watch responses may expose
operational summaries, but callers that need semantic saga detail, public-output authority, or
retention/replay proof must use per-run strict status or stream APIs.

## Storage Traits And Crate Boundaries

The target storage API should split semantic append/load from observation reads.

Proposed storage traits:

```text
RunEventStore
  append_prepared_commit_bundle(...)
  load_run_stream(...)
  expected_next_seq(...)

RunProjectionStore
  status_projection_snapshot(...)

RunReadModelStore
  list_run_read_models(...)
  get_run_read_model(...)
  poll_run_read_model_changes(...)
  wait_run_read_model_change(...)

CommitArtifactWriteSet
  verified bytes and evidence carried by PreparedCommitBundle

RetainedArtifactReadProvider
  read_retained_artifact(EventArtifactRequirement)

ArtifactReadProvider
  read_artifact(ArtifactReadRequest)

PublicOutputArtifactReader
  read_public_output_artifact(PublicOutputReadAuthority)
```

There should not be a broad production `ArtifactStore` trait with arbitrary `get_artifact_by_id`,
`has_artifact`, or unscoped read methods. Runtime staging, retained-artifact reads for verified
history, adapter artifact-read capabilities, and public-output artifact reads are distinct authority
surfaces. They may be implemented by one concrete Postgres type, but they must remain separate trait
surfaces with proof-bearing request types.

`mfm-app` should be generic over the run store, read-model store, and narrow artifact read
capabilities it actually needs. Runner registration should receive `Arc<dyn ArtifactReadProvider>`
when that is the only artifact capability a runner needs. Strict status/replay paths should use
`RetainedArtifactReadProvider`. Public-output rendering should use a reader that accepts
`PublicOutputReadAuthority` or event/projection-derived evidence, not an arbitrary artifact id.
CLI and REST should not construct filesystem stores and should not query Postgres directly.

Production wiring:

```text
PostgresRunStore
PostgresArtifactStore
PostgresReadModelStore
```

These storage responsibilities may be implemented as separate crates or as one unified Postgres
storage crate if shared SQLx pool management, migrations, schema validation, advisory locking,
artifact evidence, and read-model derivation are materially simpler that way. The artifact store
must not be hidden inside the run-event implementation as an incidental private detail. It needs
explicit `PostgresArtifactStore` surfaces for commit bundle writes, retained reads, capability
reads, and public-output reads, even if those surfaces are implemented by one concrete Postgres
store type.

The default production app factory should compose the Postgres run, artifact, and read-model
surfaces. CLI and REST should depend on that factory rather than constructing concrete storage
pieces directly.

The current production pre-append artifact stager should disappear from production wiring. Runtime
may still prepare bytes before append, but those bytes must remain in the commit bundle passed to
the Postgres append transaction. Orphan blobs from failed production commits are not part of the
target design.

## Alternative Store Removal

Production alternatives must be removed, not hidden behind fixtures.

Required removals:

- remove filesystem artifact storage from production app/CLI/REST
- remove `--typed-artifact-root` and `MFM_TYPED_ARTIFACT_ROOT` from production docs
- remove filesystem artifact store dependencies from CLI/REST production manifests
- remove production code paths that select non-Postgres storage
- remove old storage dependency allowlist exceptions once callers are refactored

Accepted exception:

- an in-memory store may remain only in tests that intentionally avoid starting Postgres

That exception should be implemented as narrowly as possible:

- `#[cfg(test)]` module, or
- test-support crate/feature used only by tests

It must not be part of production app assembly, CLI, REST, or examples.

Filesystem storage should not remain as an alternative production/local backend. If a test needs
artifact bytes without Postgres, prefer an in-memory artifact store under the same test-only
exception.

## Rebuild And Proof

Read models are correct only if they can be rebuilt and compared.

Required rebuild tools:

- `rebuild_read_models(projection_version)`
- `validate_read_models(projection_version)`
- `read_model_high_watermark()`
- `read_model_drift_report()`

Required properties:

- every delta read fact has source-event provenance
- every aggregate read fact has prefix or multi-source provenance
- every source-event provenance points to an authority event row
- every aggregate high-watermark points to committed authority rows
- rebuilding from authority rows produces the same derived row hashes
- materialized/cache rows can be dropped and recreated
- strict runtime reads ignore read-model corruption

Suggested `projection_derivations` table:

```text
projection_version TEXT NOT NULL
model_name TEXT NOT NULL
derived_key TEXT NOT NULL
source_kind TEXT NOT NULL
source_run_id TEXT NULL
source_seq BIGINT NULL
source_ordinal INTEGER NULL
source_commit_pos BIGINT NULL
source_event_id TEXT NULL
source_payload_hash TEXT NULL
source_event_schema_id TEXT NULL
source_high_commit_pos BIGINT NOT NULL
source_event_count BIGINT NOT NULL
source_input_hash TEXT NOT NULL
derived_row_canonical_json BYTEA NOT NULL
derived_row_hash TEXT NOT NULL
created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp()
```

This table is not authority. It is an audit ledger for read-model derivation.

For `source_kind = 'multi_event'`, `projection_derivation_sources` must enumerate source events or
reference a documented prefix hash chain:

```text
projection_version TEXT NOT NULL
model_name TEXT NOT NULL
derived_key TEXT NOT NULL
source_run_id TEXT NOT NULL
source_seq BIGINT NOT NULL
source_ordinal INTEGER NOT NULL
source_commit_pos BIGINT NOT NULL
```

## Guardrails

Database guardrails:

- deny `UPDATE`, `DELETE`, and `TRUNCATE` on MFM domain tables for the app role
- add database triggers that reject update/delete on authority tables
- add equivalent guards for read fact tables except inside controlled rebuild procedures
- production rebuild/validate procedures should be executable only by an explicit maintenance role,
  not by normal app/CLI/REST runtime credentials
- if local development uses a single physical database role, rebuild functions must still require an
  explicit maintenance entry point and must not be exposed through production app services
- avoid `ON DELETE CASCADE` on authority tables
- validate required triggers/functions/views during schema validation
- validate that global watch/order positions are assigned only under the global commit-order
  serialization point
- validate that hash-bearing rows use canonical MFM bytes and never `jsonb::text`

Code guardrails:

- `sqlx` only in Postgres storage crates
- CLI/REST do not import concrete storage implementations except through production app factory
- CLI/REST production manifests do not depend directly on `sqlx`, Postgres storage crates, or
  filesystem artifact storage crates
- no production reference to filesystem artifact storage
- no production reference to in-memory stores
- scans reject `UPDATE`, `DELETE`, and `TRUNCATE` in MFM domain SQL except migration/admin repair
  code with explicit labels
- scans reject `typed_` target table names in new migrations
- scans reject new public target structs with `Typed` prefixes in storage/app read-model APIs
- scans reject broad production artifact reads by arbitrary artifact id outside proof-bearing
  request types
- scans reject production pre-append filesystem artifact staging
- cargo metadata contract rejects new or remaining dependency allowlist exceptions that let
  `bin/cli`, `bin/rest-api`, or app-facing command glue depend on concrete storage crates directly

## Required Documentation Updates

This RFC changes the design contract, not only an implementation detail. The cutover must update the
authoritative project docs in the same change set as the storage/runtime/API refactor.

`docs/design.md` must be updated to describe:

- Postgres as the only production persistence system
- append-only authority tables without production update/delete/truncate
- `PreparedCommitBundle` as the production append authority carrying artifact bytes plus evidence
- artifact bytes and evidence stored in Postgres, with filesystem artifact storage removed from the
  production design
- Postgres-owned read models as rebuildable observation surfaces, not semantic authority
- strict status/replay/public-output reads rebuilding from committed stream and verified artifacts
- immutable `run_commit_log` cursor authority separate from projection-versioned summaries
- maintenance-role requirements for read-model rebuild/validation procedures
- the breaking dev-branch cutover posture and lack of compatibility with old `typed_*` tables

`docs/architecture.md` must be updated to describe:

- storage crate boundaries for run authority, artifact authority, read models, and SQLx ownership
- the rule that CLI/REST depend on app services/factories, not concrete storage crates or `sqlx`
- the narrow artifact authority surfaces: commit bundle writes, retained reads, capability reads,
  and public-output reads
- removal of filesystem artifact storage from production architecture
- test-only status of in-memory storage
- cargo/dependency guardrails that prevent production binaries from importing concrete storage
  implementations directly

CLI and REST docs must also be updated when the public list/watch routes and commands are added.

## Migration And Cutover Plan

This is a breaking dev-branch cutover, not an online compatibility migration.

1. Update the authoritative docs.
   - Update `docs/design.md` and `docs/architecture.md` with the target storage, artifact,
     read-model, API, and dependency-boundary contracts from this RFC.
   - Update CLI and REST docs when list/watch commands/routes and storage configuration change.

2. Treat the target Postgres schema as a new storage baseline.
   - Replace the current `typed_*` migration baseline with target non-`typed_` authority,
     artifact, read-model, trigger/function, and rebuild migrations.
   - Do not add compatibility views, aliases, triggers, or adapters that keep old `typed_*`
     tables live.
   - Do not read from old `typed_*` tables after the cutover.

3. Require explicit local database reset.
   - Any database that has old `_sqlx_migrations` entries or old `typed_*` tables must be
     dropped/recreated, or its development schema must be dropped/recreated.
   - The new store should fail schema validation when stale old tables or old migration checksums
     are present.
   - Document the reset command in the storage, CLI, and REST docs.

4. Remove filesystem artifact storage from production in the same cutover.
   - Remove `--typed-artifact-root` and `MFM_TYPED_ARTIFACT_ROOT` from production
     CLI/REST paths.
   - Do not migrate filesystem artifact bytes automatically.
   - Existing development artifact roots may be deleted by developers after reset.

5. Implement the target store directly.
   - Add `commits`, `run_events`, `artifact_blobs`, artifact evidence/admission tables,
     `logical_key_observations`, `run_commit_log`, projection-versioned read facts, views,
     triggers/functions, and rebuild/validation tools using target names.
   - Move same-run ordering from mutable run-head rows to advisory locking plus inserted commit
     authority.
   - Add global commit-order serialization before assigning `commit_pos` or `change_pos`.
   - Replace helper upserts that encode current state with append-only observations/facts or
     rebuildable views.
   - Replace plan-only append with commit bundles that carry admitted artifact bytes into the
     Postgres append transaction.

6. Switch app, CLI, and REST to the new production factory.
   - CLI and REST should depend on app-level services, not concrete filesystem stores or direct
     Postgres queries.
   - App services should depend on explicit run, read-model, retained artifact, capability read, and
     public-output artifact surfaces.
   - Old storage type names may be removed or renamed without compatibility aliases.
   - Remove direct `sqlx`, Postgres storage crate, and filesystem artifact store dependencies from
     CLI/REST production manifests.
   - Remove any cargo metadata allowlist exceptions that previously permitted those direct
     dependencies.

7. Regenerate SQLx metadata from a freshly migrated disposable database.
   - Existing `.sqlx` metadata for the old schema is invalid after the baseline replacement.

8. Verify the cutover.
   - Fresh database migration succeeds.
   - Schema validation rejects stale `typed_*` schemas.
   - `docs/design.md` and `docs/architecture.md` describe the new source of truth.
   - No production path references filesystem artifact storage.
   - No production domain SQL uses `UPDATE`, `DELETE`, or `TRUNCATE` except explicitly labeled
     migration/admin rebuild code.
   - Read models rebuild from authority rows and strict runtime reads ignore read-model corruption.

Rollback is code rollback plus database/schema reset to that branch's expected baseline. There is no
downgrade migration.

## Verification

Focused checks:

- `cargo fmt --all -- --check`
- `cargo check --workspace`
- `cargo test -p mfm-store`
- `cargo test -p mfm-stream-store-postgres --features parity-tests`
- `cargo test -p mfm-integration-tests --test cargo_metadata_contract`
- `cargo test -p mfm-integration-tests --test architecture_namespace_contract`
- SQLx prepare/check for the Postgres storage crate

Required new tests:

- authority tables reject update/delete/truncate for app role
- read fact rows have valid source-event provenance
- read facts rebuild exactly from authority rows
- SQL read models match Rust projection rebuild for representative runs
- SQL-only read derivations are limited to mechanical scalar/canonical-byte facts
- corrupt read facts do not affect resume/replay/public-output authority
- read facts are not used as append-admission authority
- hash-bearing rows are computed from canonical MFM bytes, not JSONB text
- commit rows reject or detect event_count, commit_key, ordinal, and batch-hash mismatches
- commit fingerprints reverify from persisted commit purpose and canonical request material
- artifact ids are verified as derived from digests
- artifact evidence rows reject digest/length/id mismatches against blob rows
- global watch cursors cannot skip later-committed rows after concurrent transactions
- watch cursor resumes after disconnect
- stale cursor-format versions are rejected or explicitly migrated
- unavailable projection-version summaries return `ProjectionUnavailable` or trigger an explicit
  rebuild path before data is returned
- empty watch pages return a well-defined high-watermark
- `LISTEN/NOTIFY` is not required for correctness
- strict `run stream --watch` tails authority rows and does not use read-model summaries
- `run_commit_log` cursor rows remain immutable across projection-version rebuilds
- same-run concurrent appends serialize correctly
- idempotent commit-key retry still wins before stale sequence checks
- cross-run resource-lane contention admits only one conflicting claimant
- artifact bytes and evidence are inserted atomically with the commit bundle
- missing, extra, or mismatched artifact bytes are rejected before authority rows commit
- every runtime append path passes a `PreparedCommitBundle`, not a plan-only append
- `logical_key_observations` fold to the same logical-key sets as the authority stream
- recoverable submission-result rewrites match Rust staging parity tests
- retained artifact reads require event-derived requirements
- adapter artifact reads require `ArtifactReadRequest`
- public-output artifact reads require public-output authority or event/projection-derived evidence
- secret-shaped artifact bytes/evidence are rejected or redacted before entering Postgres
- SQL, CLI, and REST errors for artifact/read-model paths do not expose secret-bearing values
- list/watch JSON schemas, text output, cursor errors, and REST/CLI contracts have tests
- production rebuild procedures are not executable through normal app/CLI/REST credentials
- production CLI/REST cannot select filesystem storage
- production CLI/REST do not depend directly on `sqlx`, Postgres storage crates, or filesystem
  artifact storage crates
- in-memory storage is only available in tests

## Resolved Design Decisions

- Artifact storage may live in a separate Postgres artifact crate or in a unified Postgres storage
  crate shared with run events, but the artifact-store surface must remain explicit. It must not be
  hidden as a private implementation detail of the run-event store.
- The production append API should become bundle-based so admitted artifact bytes enter the same
  Postgres transaction as committed events and artifact evidence. The RFC chooses this breaking API
  change instead of preserving pre-commit filesystem staging with possible orphan blobs.
- Artifact APIs should be split by authority surface. A single concrete Postgres store may implement
  several traits, but production code should not expose broad arbitrary artifact-id reads.
- Artifact identity is bound by artifact id derivation from digest, byte-length checks, canonical
  evidence hashes, and composite foreign keys from evidence/admission rows to blob rows.
- SQL derivation is limited to mechanical scalar/canonical-byte facts. Semantic projections required
  for list/watch summaries should be emitted by shared Rust projection code inside the Postgres
  storage transaction and rebuilt by re-running that same code over authority rows.
- Read-model rebuild procedures should use an explicit maintenance role in production. Local
  development can use a single physical database role only if rebuild functions are still hidden
  behind an explicit maintenance entry point and are not exposed through app/CLI/REST runtime paths.
- Artifact bytes should be stored in Postgres `bytea` columns. PostgreSQL's current documented hard
  field-size limit is 1 GB, but MFM should enforce lower artifact-size limits before insert and with
  database checks.
- The first observation models should cover run summary, immutable commit cursor log,
  projection-versioned change summaries, public output summary, and attempt summary. Those are
  enough for the first public list/watch API.
- Per-run strict status should continue to return full saga detail through verified stream and
  artifact authority. List/watch should return operational summaries unless the caller explicitly
  asks for strict per-run status.
- Public watch cursors should be based on `run_commit_log.change_pos` assigned in commit order, not
  plain PostgreSQL identity allocation order.
- Cursor authority and projection summaries are split: `run_commit_log` is immutable cursor
  authority, while `run_change_summaries` is projection-versioned and rebuildable.
- This RFC is a breaking dev-branch cutover. Old `typed_*` data, filesystem artifact roots, CLI
  flags, REST environment variables, and storage names do not need compatibility shims.

## Decision

Adopt the hybrid architecture:

```text
append-only Postgres authority
  + Rust store/runtime proof
  + Postgres-owned rebuildable read models
  + one CLI/REST observation API
  + no production alternative stores
```

This keeps the strictness of a clean event log while giving the platform efficient read and watch
surfaces. It also standardizes persistence, removes split-brain storage, reduces backend choices,
and keeps read models out of the semantic authority path.
