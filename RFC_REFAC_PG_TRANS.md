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

This is not a storage-only refactor. It touches the lane lifecycle, `PreparedCommitBundle`, artifact
evidence identity, strict-vs-observation store traits, app factory, CLI/REST contracts, schema
validation, and the docs contract. It does not have to land as one monolithic merge. The work
decomposes into three separable cutovers that land in dependency order, each internally coherent and
each with no compatibility shims:

1. Append/artifact cutover: `PreparedCommitBundle`, artifact bytes/evidence into Postgres, removal of
   the filesystem artifact store, and the append-only authority schema.
2. Resource-lane cutover: pre-invocation lane lifecycle, per-lane admission, lane-transition
   authority, and the `saga.md` resource-claim reconciliation.
3. Observation cutover: the read-model layer, `run_commit_log` watch cursors, and the shared
   CLI/REST run observation API, plus the `typed_` -> target naming baseline.

Within each cutover, reviewable commit slices are required, but partial merges that leave runtime
authority, storage behavior, public APIs, and docs disagree are not an acceptable intermediate state.
Splitting into three landable cutovers is explicitly preferred over one big-bang merge so each new
primitive (bundle append, lanes, watch cursors) can be validated independently. The naming baseline
travels with the observation cutover; it does not force the other two to merge simultaneously.

## Summary

MFM should converge on one production persistence system: Postgres, accessed only through `sqlx`
inside the storage implementation crate.

The target store is an append-only transition store. Production code never updates, deletes, or
truncates MFM domain rows. It only inserts committed facts. The latest state is a read concern,
derived from history through Postgres-owned views, storage-derived observation facts, or rebuildable cache
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
- Put read-model materialization in the Postgres storage layer.
- Prove read models are derived from committed events and can be rebuilt.
- Preserve runtime/store authority boundaries from `docs/design.md`.
- Reduce storage alternatives and overall storage-specific LOC over time.

## Non-Goals

- Do not move private keys, mnemonics, passwords, signer runtime sources, raw signed transactions,
  or other secret-bearing data into the MFM run database.
- Do not replace the Rust store admission/projection proof logic with PL/pgSQL semantic admission.
- Do not make materialized views semantic authority.
- Do not silently migrate uncertified historical runs into certified run history.
- Do not solve data lifecycle in this RFC. Archival, table partitioning, and growth-bounding for the
  append-only authority tables are deferred to a future RFC. v1 is append-only; growth is managed
  operationally (provisioning plus the artifact-size limit) until that RFC lands.

Artifact bytes moving into Postgres does not move the secret boundary into Postgres. Private keys,
mnemonics, passwords, decrypted wallet material, signer runtime sources, signature scalars, raw
signed transactions, unredacted transport credentials, and unredacted diagnostics must never become
typed values, typed configs, event payloads, facts, artifact evidence, artifact bytes, public
outputs, fixtures, or error details.

The target guarantee is provenance and type confinement, not best-effort insert-time content
scanning. Keystore-backed signers and signer providers may handle secret material only inside
narrowly scoped closures/primitives and return signatures plus public identity metadata. Mutation
adapters may materialize raw signed transactions only as transient submit-time bytes below the typed
semantic boundary. The Postgres artifact store verifies content-addressing, byte length, evidence,
role, and commit bindings; it is not responsible for proving arbitrary `BYTEA` is non-secret.
SQL/API errors for artifact insertion, verification, rebuild, and read paths must remain redacted.

## Naming Rules

Target table names must not use the `typed_` prefix. Target Rust storage/app/public structures
introduced or refactored by this work should not use a `Typed` prefix.

Preferred target names:

- `commits`, not `typed_commits`
- `run_events`, not `typed_run_events`
- `artifact_blobs`, not `typed_artifact_blobs`
- `artifact_admissions`, not `typed_artifacts`
- `run_artifact_admissions`, not `typed_run_artifacts`
- `run_observation_facts`, not `typed_run_read_models`
- `run_commit_log`, not `typed_run_change_log`
- `run_observation_change_summaries`, not `typed_run_change_log`
- `RunEventStore`, not `TypedRunEventStore`
- `PostgresRunStore`, not `PostgresTypedRunEventStore`
- `RunObservationStore`, not `TypedRunReadModelStore`
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
resource_lane_claim_events
resource_lane_release_events
resource_lane_transitions
```

This refactor adds no derived admission index. Logical-key admission folds `run_events` directly (see
"Logical-Key Admission" below); an append-only observation index plus its prefix-completeness proof
is deferred to a future RFC and is not part of this baseline.

`commits` records one accepted commit for one run sequence.

Important columns:

```text
commit_id TEXT NOT NULL
run_id TEXT NOT NULL
seq BIGINT NOT NULL
commit_key TEXT NOT NULL
commit_purpose TEXT NOT NULL
commit_idempotency_hash TEXT NOT NULL
idempotency_canonical_json BYTEA NOT NULL
prepared_authority_hash TEXT NOT NULL
prepared_authority_canonical_json BYTEA NOT NULL
commit_batch_hash TEXT NOT NULL
event_count INTEGER NOT NULL
append_xid XID8 NOT NULL DEFAULT pg_current_xact_id()
committed_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp()
```

Constraints:

```text
PRIMARY KEY (run_id, seq)
UNIQUE (commit_id)
UNIQUE (run_id, commit_key)
CHECK (seq >= 1)
CHECK (event_count >= 1)
```

Only three uniqueness facts are real: stream position `(run_id, seq)`, global commit identity
`commit_id`, and per-run commit-key idempotency `(run_id, commit_key)`. Earlier drafts added wide
composite UNIQUE supersets (e.g. `(commit_id, run_id, seq, event_count, commit_batch_hash,
append_xid)`) only so child tables could hang composite foreign keys off them. That is rejected here:
each redundant UNIQUE is a separate index maintained on every append, which is write amplification the
append path cannot afford. Child tables instead reference the natural key — `commits(run_id, seq)` or
`commits(commit_id)` — and the agreement of the remaining columns (`commit_key`, `event_count`,
`commit_batch_hash`, `append_xid`) is verified by Rust strict load and append staging, not by an
index. Relational integrity stays in SQL only where a single key expresses it; multi-column
agreement is a fold check in Rust.

`commit_id` is the stable identity for one accepted run commit. It is not a global ordering value
and must not be used as platform-wide semantic order. The initial algorithm is closed and
non-circular:

```text
commit_id =
  hash("mfm.commit.id.v1" || canonical(run_id, seq, commit_key, commit_purpose,
       prepared_authority_hash))
```

`commit_id` is known before final event-envelope hashing. Event envelopes and
`commit_batch_hash` may therefore include `commit_id` without a store-assigned identity cycle.
Storage must not assign `commit_id` after Rust staging unless a future RFC defines an explicit
store-filled commit-id protocol and updates every affected hash contract.

`append_xid` is the PostgreSQL top-level transaction id that inserted the commit. It is assigned by
Postgres with `pg_current_xact_id()` and is used only for snapshot-sealed observation cursors in
`run_commit_log`. It is not semantic authority, not commit-time order, and not a substitute for
run-local stream order. Strict authority remains `(run_id, seq, ordinal)` plus committed artifact
evidence and Rust verification.

`append_xid` must be database-owned. The `DEFAULT pg_current_xact_id()` shown in the schema is not
enough by itself, because production inserts could otherwise override it. The migration must install
a `BEFORE INSERT` trigger or equivalent database-owned mechanism that overwrites `commits.append_xid`
and `run_commit_log.append_xid` with the inserting top-level transaction id. Production insert
statements must not provide `append_xid`, and schema validation must verify the trigger/function is
present.

The target architecture must not introduce a global commit-order advisory lock, table lock, global
counter row, or equivalent serialization point to assign platform-wide commit positions. Independent
runs must be able to commit concurrently unless they contend on the same run id or on explicitly
held resource-lane claims.

`commit_purpose` is the stable purpose tag for the prepared commit variant. The persisted request
material is split into two canonical records:

- `commit_idempotency_hash` and `idempotency_canonical_json` exclude `expected_next_seq` so a valid
  retry of an already accepted `(run_id, commit_key)` can be recognized before stale sequence
  checks.
- `prepared_authority_hash` and `prepared_authority_canonical_json` include the full accepted
  authority record, including `expected_next_seq`, purpose, payloads, preconditions, required
  artifact evidence, admitted artifact evidence, and the accepted base stream identity or base
  stream hash.

For commits with store-filled fields, the persisted prepared authority record must distinguish
prepared intent from materialized event payload. `prepared_authority_hash` covers
`prepared_payload_intent`, store-fill policy ids, holder identities, preconditions, and evidence
requirements. It does not pretend to hash values that runtime cannot know yet, such as a
store-assigned resource-lane fencing token. The materialized event payloads, including every
store-filled value, are persisted in `run_events.payload_canonical_json` and bound by
`commit_batch_hash`. Strict load verifies both records: prepared intent proves what runtime asked
storage to admit, while the materialized event payload proves what storage committed.

Artifact byte payloads do not need to be included directly if their evidence binds digest and byte
length and the store verifies bytes before insert. Strict load/rebuild tooling must reverify both
hashes from persisted canonical bytes instead of treating them as append-time-only metadata.

The idempotency record must include every field that determines the committed event batch, artifact
evidence set, semantic authority, or public outcome *given a fixed accepted sequence*, excluding only
a closed list of retry-volatile precondition fields needed to recognize an already accepted
`(run_id, commit_key)` before stale sequence checks. `expected_next_seq` is the canonical excluded
field. Note this is deliberately not "every field in `commit_batch_hash`": `seq` is bound by
`commit_id` and therefore by the batch, yet it is excluded from the idempotency record. This is sound
because `seq` is frozen at first acceptance — a true retry of an already accepted `(run_id,
commit_key)` resolves by returning the persisted commit at its accepted `seq`, never by recomputing a
divergent batch from the retry's (possibly stale) `expected_next_seq`. On duplicate `(run_id,
commit_key)`, the store must compare the request's
idempotency canonical bytes with the persisted bytes. A mismatch is `IdempotencyConflict`. A match
returns the persisted commit/result from stored rows; it must not return request-computed
`commit_id`, request-computed batch hashes, or request-local authority objects. If
`prepared_authority_hash` differs for a matching idempotency record, the store may still return the
persisted result only when the difference is in the closed retry-volatile field set and the
persisted canonical authority proves the accepted commit. Any other difference is
`IdempotencyConflict`.

`commit_batch_hash` binds the commit row to the stored ordered event batch and commit artifact
evidence. It must be computed from canonical MFM bytes over the resulting event envelopes and
artifact evidence bindings. Rust staging computes and validates this hash before commit for commits
whose final event payloads are fully known before storage admission. For store-filled fields such as
resource-lane fencing tokens, Rust staging validates the claim intent and store-fill policy, and the
Postgres append path computes the final `commit_batch_hash` after materializing the store-filled
event payload. Strict load recomputes the final hash from persisted canonical bytes before minting
authority. Database checks should remain structural unless a DB-side MFM canonicalizer is promoted
to a first-class maintained component.

All commit-level hashes must be domain-separated and versioned. The persisted canonical bytes for
`commit_idempotency_hash`, `prepared_authority_hash`, and `commit_batch_hash` must include or be
paired with the canonicalizer identity and hash-domain version used to compute them. Strict load must
reject unknown or mismatched canonicalizer identities instead of silently recomputing with a different
schema.

The implementation must enforce before commit that:

- event ordinals for a commit are contiguous `0..event_count-1`
- every event row for `(run_id, seq)` carries the commit row's `commit_key` and `commit_id`
- the number of inserted event rows equals `event_count`
- commit artifact evidence rows are bound to the same `(run_id, seq, commit_key, commit_id)`
- every commit has exactly one `run_commit_log` row bound to the same commit
- any commit containing `ResourceLaneClaimed` or `ResourceLaneReleased` has exact one-to-one
  event-to-mirror-to-transition completeness
- the canonical bytes needed for Rust strict-load hash verification are persisted without relying on
  Postgres `jsonb::text`

These invariants have **one primary enforcer: the Rust append path.** Every production insert flows
through `append_prepared_commit_bundle`, which builds and validates the full row set inside one
transaction before `COMMIT`. The app role has no `UPDATE`/`DELETE`/`TRUNCATE` and no other production
write surface, so the only way malformed rows reach disk is a bug in that single staging path. The
correct place to prevent that is the staging path plus its tests, not a second implementation.

Earlier drafts mandated deferrable constraint triggers (or "equivalent transaction-final validation")
that re-derive the same contiguity, cardinality, reverse-completeness, and lane-completeness folds in
PL/pgSQL. That is rejected. Multi-row folds reimplemented in PL/pgSQL are hard to write correctly,
fire per-row, and drift from the Rust definition over time — they add a maintenance and divergence
cost without adding an independent guarantee, because the same crate already owns the only writer.

The division of labor is therefore:

- **SQL relational integrity** carries what a single key, foreign key, `CHECK`, or `NOT NULL`
  expresses directly: existence of the parent `commits` row (`run_events`/evidence/lane/commit-log FKs
  to `commits(run_id, seq)` or `commits(commit_id)`), `seq >= 1`, `ordinal >= 0`, `event_count >= 1`,
  byte-length checks, and the enum `CHECK`s. The `run_commit_log -> commits` FK rejects orphan cursor
  rows.
- **Rust append staging** is the single authority for the multi-row folds above (contiguity,
  cardinality, reverse one-to-one commit-to-commit-log completeness, and lane event/mirror/transition
  completeness with lane-local sequence continuity, previous-transition-hash chaining, active-holder
  release legality, and claim-token monotonicity).
- **Rust strict load** re-runs the same fold checks on read as the corruption guard, so out-of-band
  damage or a staging bug is detected before authority objects are minted.

A cheap `CHECK` or trigger that adds genuine defense-in-depth for a single-row property is allowed,
but it is not required, and any SQL check that duplicates a Rust fold must ship with a parity test
that proves the two agree. The schema must not grow a second semantic engine.

`run_events` records store-owned event envelopes.

Important columns:

```text
commit_id TEXT NOT NULL
run_id TEXT NOT NULL
seq BIGINT NOT NULL
ordinal INTEGER NOT NULL
event_id TEXT NOT NULL
event_type TEXT NOT NULL
event_schema_id TEXT NOT NULL
spec_hash TEXT NOT NULL
commit_key TEXT NOT NULL
logical_key TEXT NOT NULL
payload_hash TEXT NOT NULL
payload_canonical_json BYTEA NOT NULL
```

Constraints:

```text
PRIMARY KEY (run_id, seq, ordinal)
UNIQUE (commit_id, ordinal)
UNIQUE (run_id, event_id)
FOREIGN KEY (run_id, seq)
  REFERENCES commits(run_id, seq)
  ON DELETE RESTRICT
CHECK (seq >= 1)
CHECK (ordinal >= 0)
```

`run_events` stores only `payload_canonical_json`. Earlier drafts also stored `payload_json JSONB` as
a "query copy," doubling payload storage on the hottest table. The canonical bytes are the only
authority, and human-/SQL-queryable JSON belongs in the observation facts that already exist for
exactly that purpose. Strict load decodes the canonical bytes; nothing reads JSON off `run_events`.
The three wide composite UNIQUEs from earlier drafts are also removed: they existed only to be foreign
key targets for `logical_key_observations` and the lane mirror tables. `logical_key_observations` is
deferred (see below), and the lane mirror tables reference the `run_events` primary key
`(run_id, seq, ordinal)`; the agreement of `event_id`, `event_type`, `payload_hash`, `commit_key`,
`commit_id`, and `logical_key` is a Rust strict-load fold, not a wide index on the append hot path.

The store must keep validating persisted rows by reconstructing event identity, payload hash,
schema id, spec hash, logical key, ordering, and commit grouping through the Rust store contract.
Semantic event order is run-local `(run_id, seq, ordinal)`. There is intentionally no global
semantic event order across independent runs. A generated identity event column may exist as a debug
surrogate, but it must not drive public watch cursors or semantic stream order.

`payload_hash`, `commit_idempotency_hash`, `prepared_authority_hash`, `commit_batch_hash`, and
read-model row hashes must be computed from MFM canonical JSON bytes, not from `jsonb::text` or any
Postgres JSONB serialization. Hashed structured data must continue to reject floats. `run_events`
stores `payload_canonical_json` so strict load paths and rebuild tools verify hashes from canonical
bytes without trusting JSONB formatting. Observation facts that need queryable JSON derive it from
those canonical bytes (see the read-model layer); the authority table carries bytes only.

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

#### Artifact BYTEA Secret Boundary

`artifact_blobs.bytes` is allowed to contain only bytes that already crossed a typed,
role-specific artifact construction boundary. It is not a generic byte dump.

The storage layer must not expose `insert_artifact_bytes(Vec<u8>)` or equivalent broad production
APIs. Artifact bytes enter Postgres only through `PreparedCommitBundle` artifact members whose
evidence was produced by closed runtime/adapter constructors and whose role is one of the allowed
`ArtifactRole` contracts.

Storage validates:

- digest, `artifact_id`, and `byte_len`
- evidence canonical bytes and evidence hash
- role/schema/semantic/producer contract
- same-commit admission and event/reference binding

Storage does not validate by scanning arbitrary bytes for private keys, mnemonics, passwords, raw
signed transactions, or credentials. If a change needs such scanning to be safe, the design has
already placed secret material on the wrong side of the boundary.

`artifact_admissions` stores the full typed artifact evidence shape:

```text
evidence_hash TEXT PRIMARY KEY
artifact_id TEXT NOT NULL REFERENCES artifact_blobs(artifact_id) ON DELETE RESTRICT
digest TEXT NOT NULL
byte_len BIGINT NOT NULL
evidence_schema_version TEXT NOT NULL
media_type TEXT NOT NULL
schema_id TEXT NULL
semantic_type_id TEXT NULL
producer_node_id TEXT NULL
producer_seed_id TEXT NULL
producer_descriptor_hash TEXT NULL
certified_spec_hash TEXT NULL
requirement_provenance_hash TEXT NULL
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

At minimum, canonical evidence must bind the evidence schema version, artifact content identity
(`artifact_id`, digest, byte length), media/schema/semantic type, artifact role, producer identity,
certified spec or descriptor context needed to interpret producer ids, and requirement provenance
when the artifact satisfies a run/event requirement. Nullable relational convenience columns are
only query copies of this canonical evidence. If a column such as `producer_node_id` or
`producer_seed_id` is present, the canonical evidence must also bind the certified descriptor/spec
context that gives that id meaning.

`run_artifact_admissions` and `commit_artifact_evidence` bind evidence to run/commit authority:

```text
run_artifact_admissions
  run_id TEXT NOT NULL
  artifact_id TEXT NOT NULL
  evidence_hash TEXT NOT NULL
  first_commit_id TEXT NOT NULL
  first_seq BIGINT NOT NULL
  first_commit_key TEXT NOT NULL
  first_binding_kind TEXT NOT NULL CHECK (first_binding_kind = 'admitted')

commit_artifact_evidence
  run_id TEXT NOT NULL
  seq BIGINT NOT NULL
  commit_key TEXT NOT NULL
  commit_id TEXT NOT NULL
  artifact_id TEXT NOT NULL
  evidence_hash TEXT NOT NULL
  binding_kind TEXT NOT NULL CHECK (binding_kind IN ('required', 'admitted'))
  requirement_source TEXT NULL
```

Binding tables must enforce set semantics in the database, matching the Rust commit contract:

```text
run_artifact_admissions:
  PRIMARY KEY (run_id, artifact_id, evidence_hash)

commit_artifact_evidence:
  PRIMARY KEY (run_id, seq, binding_kind, artifact_id, evidence_hash)
  UNIQUE (run_id, seq, commit_key, commit_id, binding_kind, artifact_id, evidence_hash)
```

Artifact identity is not evidence identity. `artifact_id` is digest-derived content identity, while
`evidence_hash` binds role, schema, semantic type, producer, and other typed evidence. The same bytes
may legitimately be admitted under distinct evidence records. All authority, retained reads,
requirements, and rebuilds that need typed meaning must use `(artifact_id, evidence_hash)` or exact
event-derived requirements, not `artifact_id` alone. If a future commit format permits multiple
bindings for the same exact `(binding_kind, artifact_id, evidence_hash)`, it must add an explicit
`binding_ordinal` and include it in the canonical evidence set and primary key. Duplicates must never
be allowed to distort rebuild counts, retained-artifact requirements, or `commit_batch_hash`.

Both binding tables must use composite foreign keys that bind the artifact id to the exact evidence:

```text
FOREIGN KEY (artifact_id, evidence_hash)
  REFERENCES artifact_admissions(artifact_id, evidence_hash)
  ON DELETE RESTRICT
```

`run_artifact_admissions` must also bind the first admission to the commit that admitted it:

```text
FOREIGN KEY (run_id, first_seq)
  REFERENCES commits(run_id, seq)
  ON DELETE RESTRICT
```

(`first_commit_key`/`first_commit_id` agreement with that `commits` row is a strict-load/append-staging
check, consistent with the trimmed `commits` keys.)

That commit binding is not sufficient by itself. The first admission must also be bound to the
exact `commit_artifact_evidence` row that admitted the same `(artifact_id, evidence_hash)`. This is a
real relational constraint (a composite foreign key into an existing `commit_artifact_evidence`
unique), so it is enforced with the checked/generated `first_binding_kind = 'admitted'` column rather
than a fold-reimplementing trigger:

```text
FOREIGN KEY (
  run_id,
  first_seq,
  first_commit_key,
  first_commit_id,
  first_binding_kind,
  artifact_id,
  evidence_hash
)
  REFERENCES commit_artifact_evidence(
    run_id,
    seq,
    commit_key,
    commit_id,
    binding_kind,
    artifact_id,
    evidence_hash
  )
  ON DELETE RESTRICT

CHECK (first_binding_kind = 'admitted')
```

The checked-column composite foreign key is the preferred pattern because it is ordinary relational
integrity, not a re-implemented Rust fold. A deferrable trigger is acceptable only if PostgreSQL null
semantics make the composite key impractical, and then it must reject any run-level admission whose
first commit does not contain the exact `binding_kind = 'admitted'` evidence row.

`commit_artifact_evidence` must bind each requirement/admission row to its commit:

```text
FOREIGN KEY (run_id, seq)
  REFERENCES commits(run_id, seq)
  ON DELETE RESTRICT
```

(`commit_key`/`commit_id` agreement is a strict-load/append-staging check.)

The required/admitted distinction is part of commit authority. It must be possible to rebuild the
commit idempotency hash, full prepared authority hash, `commit_batch_hash`, and retained artifact
requirements from the deduplicated inserted commit artifact evidence plus the committed events.

#### Logical-Key Admission (No Observation Index In v1)

The current schema's mutable logical-key helper tables (`typed_logical_keys`,
`typed_unique_logical_payloads`) are removed. They are **not** replaced with a new
`logical_key_observations` index in this refactor.

Earlier drafts proposed an append-only `logical_key_observations` table with full per-event
provenance, an eight-column foreign key into `run_events`, and parity tests. But the same drafts also
required that the initial implementation **must not** use it for admission — append must fold
`run_events`, because a missing observation row is indistinguishable from an absent key without a
separate completeness proof. So in v1 the table would be pure cost: write amplification on every
append, a wide composite FK that forces an extra index on the `run_events` hot path, and a parity
harness — for zero admission benefit. Its only payoff (skipping full-stream folds) arrives only once
the prefix-completeness proof is designed, and that proof does not exist yet.

Therefore v1 derives logical-key admission directly:

- Append admission and strict semantic load fold authoritative `run_events` rows in `(seq, ordinal)`
  order to prove logical-key presence, absence, uniqueness, and recoverable rewrite legality.
- `LogicalKeySet` is the set of distinct `(run_id, logical_key)`; `UniqueLogicalPayloads` folds the
  unique-marked events.
- A repeated unique logical key with the same payload hash is invalid. A repeated unique logical key
  with a different payload hash is invalid unless Rust admission proves the recoverable
  submission-result exception against the projection prefix before that event. That rule
  (`submission_result_unknown_recovery`, superseding a known previous payload hash via a stable Rust
  admission rule id) stays entirely in the Rust staging path; it never needed a persisted index.

A future RFC may reintroduce an append-only observation index **together with** its prefix-completeness
proof (a per-run prefix seal recording observed event count, source range, ordered observation hash,
and authority prefix hash, inserted atomically with the observations, verified before admission, and
invalidated by any missing/extra/mismatched row). The index and the proof that makes it usable must
land together, not the index first. Until then there is no `logical_key_observations` table to keep
rebuildable or drift-check.

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
- Rust projections rebuilt or verified from those authority rows inside the same append path

Logical-key presence, absence, and uniqueness are proven by folding `run_events`; v1 has no
admission index to load as a hint. Should a future RFC add such an index, it may be loaded beside the
base only as a hint and only proves absence/legality once the append path verifies that index's
explicit prefix-completeness proof.

A semantic append must pass through the Rust store staging path:

```text
PreparedCommitBundle
  -> load authority rows needed for the base (commits, run_events, artifact authority)
  -> verify pending artifact bytes against admitted evidence
  -> Rust stage/validate commit
  -> insert authority rows
  -> Postgres storage layer records mechanical observation facts
  -> future authority-computed observation rows enter only through explicit authority-layer APIs
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

`PreparedCommitBundle` is a breaking replacement for the current plan-only append surface. It
contains the prepared commit plan plus the artifact byte payloads admitted by that plan.
The store must reject missing, extra, or mismatched bytes; it may reuse an existing blob only when
the existing blob and evidence are byte-for-byte/evidence-identical. Production code should not
stage artifact bytes in a filesystem store before append. The only accepted non-Postgres staging
exception remains narrow in-memory test code.

This requires an explicit kernel/runtime/store API cutover:

- `mfm-store` defines `PreparedCommitBundle` as the only production append input.
- `AsyncTypedRunEventStore::append_prepared_commit_plan` is deleted and replaced by
  `append_prepared_commit_bundle`; it must not remain as a deprecated alias or blanket adapter.
- Every runtime append path passes a `PreparedCommitBundle`, including zero-artifact commits with an
  empty artifact set. `PreparedCommitPlan` may remain as an internal planning value, but it must not
  be directly appendable through a durable production store trait.
- `PreparedCommitBundle` carries opaque `PreparedArtifactBytes` proof objects for every admitted
  artifact that is not already present with identical evidence. These objects must have private
  fields and verified constructors rather than being a public `{ bytes, evidence }` bag.
- Reuse without bytes must be represented by an explicit exact-evidence reference, such as
  `ExistingArtifactAdmission { artifact_id, evidence_hash }`; the store must verify the exact blob
  and evidence rows already exist before accepting it.
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

### 3. Resource-Lane Authority

Resource lanes are declared by capability/IO contracts and resolved by runtime preflight before state
execution. Storage never derives resource lanes from arbitrary payloads and never takes global
resource-lane locks. Storage only admits, records, and enforces explicit lane claims supplied by the
runtime lifecycle.

This is a lifecycle change from the model in `docs/saga.md`, which today derives exclusive lanes from
`SideEffectInvocationPrepared.resource_key` evidence recorded at invocation preparation. The target
model acquires the lane *before* invocation through a separate committed `ResourceLaneClaimed` event.
`docs/saga.md` is part of this RFC's documentation merge gate and must be updated in the resource-lane
cutover; the two documents must not describe different lane lifecycles.

**Claim taxonomy.** `mfm-spec` defines three resource-claim kinds (`ResourceClaimSpec::Exclusive`,
`ExactTouchedSet`, `ManualOnly`). Only `Exclusive` takes a lane in this design, and the lane schema is
`mode = 'exclusive'` for exactly that reason. `ExactTouchedSet` records touched-key evidence only
*after* execution, so it cannot resolve a pre-invocation lane and takes none; the kernel makes no
cross-run concurrency claim for it beyond schema checks, exactly as today. `ManualOnly` takes no lane.
Capabilities whose exclusive resource identity is not deterministically derivable before live IO
cannot use the exclusive lane lifecycle (see the resolver purity rules below).

**No-deadlock invariant.** Deadlock detection and global scheduling policy remain outside the saga
contract. The design structurally excludes the deadlock class instead of detecting it: **an attempt
acquires every exclusive lane it needs in a single `ResourceLaneClaimed` commit, all-or-nothing,
under sorted per-lane admission locks; it never holds a committed lane while issuing a second,
separately blocking claim.** Sorted locks make the single multi-lane claim free of lock-ordering
deadlock, and all-or-nothing admission (return `ResourceLaneClaimBlocked` if any lane in the set
conflicts, holding nothing) makes hold-and-wait impossible. Strict load and append admission must
reject any history in which an open attempt holds an active lane and then commits a further
`ResourceLaneClaimed`, because that shape can deadlock across runs. In v1 a side effect needs at most
one lane, so the common path is a single-lane claim; the invariant is what keeps multi-lane
requirements safe if they appear.

**Single-lane FIFO liveness.** For the practical v1 case of one exclusive lane in a claim commit,
Postgres admission maintains a lane-local FIFO waiter queue under the same per-lane advisory
transaction lock used for authority admission. Waiter rows are mutable operational admission state;
they are not semantic replay authority, do not grant ownership, and do not replace
`ResourceLaneClaimed`, `ResourceLaneReleased`, or `resource_lane_transitions` as the only durable lane
ownership/order authority. A single-lane exclusive claim may materialize only when no active
authoritative holder exists and either no live waiter exists for the lane or this claim is the oldest
live waiter. A later live waiter may bypass an earlier waiter only after the earlier waiter is marked
`claimed`, `cancelled`, or `expired`. FIFO is therefore guaranteed only among live non-expired
waiters for single-lane exclusive claims; multi-lane claims must not be described as fair until a
separate atomic multi-lane FIFO design lands.

`ResourceLaneClaimBlocked` may insert or refresh one non-authoritative waiter row so the claimant can
retain its FIFO position on retry. It still persists no run event, commit, resource-lane claim,
resource-lane release, lane-transition, or other MFM domain authority row. Claim retries use bounded
backoff and refresh the waiter lease; expired waiters can be skipped. A retry for a deterministic
claim fingerprint reuses the same operational waiter id; if its prior row was `expired` or
`cancelled`, the row is reactivated with a fresh lane-local ticket rather than preserving stale
priority. The store stages and validates the prepared commit before enqueueing any waiter, so
malformed or otherwise semantically invalid claims cannot occupy the FIFO head. `LISTEN/NOTIFY` on
release is a wake hint only, never a grant and never required for correctness, because waiters must
re-read and pass store-side FIFO admission before ownership materializes. Ordinary contention never
converts the parked attempt into a failure by itself.

The capability contract should expose lane requirements as typed authority, for example:

```text
ResourceLaneRequirement
  namespace
  key_schema_id
  mode                  # initially Exclusive
  acquisition           # initially PreStateInvocation
  hold                  # initially UntilSideEffectTerminal

ResolvedResourceLaneClaim
  requirement_digest
  namespace
  key_schema_id
  key_canonical_json
  mode
  resolved_by_capability_impl

ResourceLaneClaimIntent
  resolved_claim
  run_id
  node_id
  attempt_id
  ledger_key
  invocation_epoch
  store_fill_policy_id      # initially resource_lane_fencing_v1
```

Resolvers are pure. They may inspect certified config, typed inputs, non-secret runtime binding
metadata, side-effect identity, and already materialized non-secret artifacts. They must not call
live transports, signers, network, keystore providers, clocks, or other secret/live systems. If a
lane cannot be resolved without live IO or secrets, the state cannot execute through this lifecycle.
Resolver implementations belong in pure capability/adapter contract code. They must not live in
live transport clients, signer/keystore code, storage crates, app orchestration, CLI, or REST
layers. Transports may expose non-live descriptor constants and schema ids that resolvers consume,
but they must not perform the resolution.

Runtime lifecycle for lane-bearing states:

1. Scheduler selects a node/attempt candidate.
2. Runtime appends `StateAttemptStarted` from certified attempt authority as a separate commit.
3. Runtime materializes a pure preflight context from certified config, typed inputs, capability
   bindings, side-effect identity, and the committed open attempt.
4. Runtime derives intent/idempotency and ledger identity using pure callbacks.
5. Runtime asks capability/adapter lane resolvers for required `ResolvedResourceLaneClaim`s.
6. Runtime builds a separate certified pre-invocation commit containing `ResourceLaneClaimIntent`
   for that already-started attempt.
7. The store admits the `ResourceLaneClaimed` commit only if the attempt has a prior committed
   `StateAttemptStarted`, the attempt is still open, the claim matches certified capability
   requirements and preflight evidence, and no active conflicting lane claim exists. While holding
   the per-lane admission lock, the store assigns the next lane-local fencing token, materializes the
   final `ResourceLaneClaimed` event, and inserts the run event, relational lane claim row, and
   lane-transition row atomically with that commit.
8. Only after that claim commit succeeds does runtime receive a `HeldResourceLaneSet` and build or
   run the state invocation.
9. Runtime validates that any `SideEffectInvocationPrepared.resource_key` exactly matches the held
   preflight claim.
10. Runtime releases the lane only through terminal side-effect, recovery, cleanup, or another
    certified release prepared commit that emits `ResourceLaneReleased` and is admitted by the
    store with the exact release mirror row and lane-transition row.

State or adapter output must echo the held lane claim. A mismatch is a runtime/store admission error,
not a recoverable state result. Recovery paths that may touch live IO for a lane-bearing side effect
must first verify an active committed held lane or commit a certified recovery claim for the same
lane before IO. If a prepared resource key already exists, recovery uses that exact key; otherwise it
reruns deterministic preflight and records the claim in the stream before IO.

`ResourceLaneClaimed` is an attempt-bound, stream-authoritative, pre-invocation event materialized
by the store from a runtime-provided `ResourceLaneClaimIntent`. It is not pre-attempt authority and
it must not be combined with `StateAttemptStarted` in the same commit. Strict load and append
admission must reject `ResourceLaneClaimed` unless it is preceded by a separate committed
`StateAttemptStarted` for the same `(run_id, node_id, attempt_id)`.

Normal contention is not corruption and not an attempt failure. If the claim intent is valid but an
active conflicting lane or earlier live waiter exists, the store returns `ResourceLaneClaimBlocked`
without inserting a commit, run event, lane mirror row, lane-transition row, or terminal attempt
event. For single-lane exclusive claims, Postgres may insert or refresh one mutable operational
waiter row used only for FIFO admission; that row is not MFM domain authority and cannot authorize
ownership, replay, status, or terminal outcome. The attempt remains open and parked before
invocation. Scheduler/recovery may retry claim admission when the conflicting lane releases or its
waiter lease is refreshed, run independent work with a scoped independence witness, or keep the
attempt parked. A blocked lane claim by itself must never authorize `StateAttemptFailed`, saga
failure, run failure, or any terminal attempt disposition. Termination requires separate certified
cancellation, interruption, timeout, or operator policy unrelated to ordinary lane contention,
committed through the normal prepared authority path. Because no authority row is committed, this
blocked outcome must not be persisted in MFM domain tables or Postgres read-model tables as if it were
derived from committed history. It may be emitted as ephemeral logs/metrics or stored only in
non-authoritative operational coordination/telemetry outside the MFM transition/read-model authority
schema. Public list/watch may derive lane-blocked display from verified open-attempt history plus
active lane transition authority at read time, or omit that display.

Crash behavior is stream-driven:

- before `ResourceLaneClaimed`, including after a `ResourceLaneClaimBlocked` outcome, recovery
  retries pure preflight and claim admission, parks the open attempt as lane-blocked, or applies a
  separate certified cancellation/interruption/deadline policy if one exists; ordinary contention is
  never itself the failure proof
- after `ResourceLaneClaimed` but before `SideEffectInvocationPrepared`, recovery continues the same
  attempt using the committed held claim or releases it through a certified cleanup/interruption
  commit
- after `SideEffectInvocationPrepared`, the side-effect lifecycle owns recovery and must verify the
  same committed claim before live IO

Existing side-effect pre-prepare phases such as `SideEffectIntentPersisted` and `SideEffectClaimed`
remain non-lane-holding phases. Only `ResourceLaneClaimed` creates release-required resource-lane
authority. `docs/design.md` must be updated with this explicit lifecycle distinction in the same
implementation change.

For EVM transaction preparation that consumes an account nonce, the exclusive lane is
`mfm.evm.account_nonce` keyed by canonical `{ chain_id, account }`. It is not keyed by RPC URL or
runtime source ref. The lane must be acquired before nonce read, transaction construction, signing,
submission, or submission recovery. Read-only EVM capabilities do not require this exclusive lane.

The store API should expose lane authority only through prepared commit admission and strict reads:

```text
append_prepared_commit_bundle(bundle_with_lane_intents_or_releases)
  -> CommitOutcome | ResourceLaneClaimBlocked
load_active_resource_lanes(...)
load_lane_history(...)
```

The initial Postgres implementation must use sorted per-lane transaction-scoped advisory locks while
mutating lane authority rows. The lock key is derived from `lane_id` (the fixed-width derived lane
identity). Advisory locks must use the two-argument form `pg_advisory_xact_lock(classid, objid)` with
a dedicated lane class id, distinct from the class id used for the per-`run_id` advisory lock, so a
`run_id` hash and a `lane_id` hash cannot collide in a shared 64-bit space and accidentally serialize
or interleave unrelated work. The store must acquire those locks before checking active conflicting
claims, and it must perform the active-claim check plus claim/release row and lane-transition row
insertions in the same transaction while holding the locks. Within a commit that claims more than one
lane, the locks are taken in sorted `lane_id` order. A future implementation may replace advisory
locks only with a reviewed DB-enforced mechanism that proves the same no-double-claim property, such
as serializable admission plus mandatory retry on serialization failure.

These locks serialize admission only. They are not the execution-duration resource lock.
Execution-duration authority is represented by committed lane claim/release events and their
relational rows. Where operational leases with fencing tokens are needed, they must be tied to a
committed claim and remain operational observations; mutable lease rows are not semantic replay
authority.

Fencing tokens are store-assigned, lane-local, and monotonic. Runtime must not assign or guess them.
While holding the concrete lane admission lock, storage computes the next token from append-only
lane-transition history for that `lane_id` and inserts exactly one claim row with that token. This is
a lane-local sequence, not a global counter and not a mutable resource-lane state row.

Claim/release ordering also needs durable lane-local authority. The per-lane admission lock is only
an admission mechanism; it is not replayable evidence. Storage must therefore assign a
`lane_transition_seq` under the same concrete lane lock for every admitted `ResourceLaneClaimed` and
`ResourceLaneReleased` transition. This sequence is lane-local, transactionally derived from
committed `resource_lane_transitions` rows, and must not use PostgreSQL sequences or identity
columns. Strict load folds `resource_lane_transitions` by `(lane_id, lane_transition_seq)` to
prove that a claim only occurs while the lane is unheld, a release is by the active holder, and claim
fencing tokens are the unique next lane-local claim tokens. `append_xid` and run-local `(run_id, seq,
ordinal)` must not be used as semantic lane order across runs.

The prepared claim commit uses a store-filled-field protocol:

- `prepared_authority_hash` covers the `ResourceLaneClaimIntent`, holder identity, certified
  requirement digest, preflight evidence, and `store_fill_policy_id`
- `prepared_authority_hash` does not include the final fencing token, because the runtime cannot
  know it
- `commit_id` is derived from the prepared authority hash before store fill
- while admitting the commit, storage assigns the fencing token, materializes the final
  `ResourceLaneClaimed` event payload, and computes `commit_batch_hash` over the final event
  envelope and evidence set
- strict load recomputes both the prepared authority hash and final batch hash, and verifies that
  the stored token is the unique next lane-local token under the same append-only claim prefix

`claim_id` should be derived from the committed holder identity, lane identity, fencing token, and
`commit_id`, or otherwise be covered by the final `ResourceLaneClaimed` canonical payload. It must
not be a runtime-supplied arbitrary identifier.

Lane identity is a fixed-width derived id, not a repeated tuple. The canonical lane descriptor
`(namespace, key_schema_id, key_canonical_json, mode)` is stored once, on the claim event, as the
audit source. Everywhere a key, unique, foreign key, or transition order needs the lane, it uses:

```text
lane_id =
  0x01 || sha256(
    "mfm.resource_lane.id.v1" || canonical(namespace, key_schema_id, key_canonical_json, mode)
  )[0..31]
```

`lane_id` is a 32-byte `BYTEA` of fixed width. This keeps the large `key_canonical_json BYTEA` out of
every index and foreign key; it appears in exactly one column on one table. The store derives and
verifies `lane_id` from the descriptor before insert, and strict load reverifies it.

Required lane authority rows:

```text
resource_lane_claim_events
  claim_id TEXT PRIMARY KEY
  lane_id BYTEA NOT NULL
  run_id TEXT NOT NULL
  node_id TEXT NOT NULL
  attempt_id TEXT NOT NULL
  ledger_key TEXT NULL
  invocation_epoch INTEGER NULL
  namespace TEXT NOT NULL
  key_schema_id TEXT NOT NULL
  key_canonical_json BYTEA NOT NULL
  mode TEXT NOT NULL CHECK (mode IN ('exclusive'))
  requirement_digest TEXT NOT NULL
  resolved_by_capability_impl TEXT NOT NULL
  fencing_token BIGINT NOT NULL
  commit_id TEXT NOT NULL REFERENCES commits(commit_id) ON DELETE RESTRICT
  source_seq BIGINT NOT NULL
  source_ordinal INTEGER NOT NULL
  source_event_id TEXT NOT NULL
  source_event_type TEXT NOT NULL CHECK (source_event_type = 'ResourceLaneClaimed')
  source_event_payload_hash TEXT NOT NULL
  created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp()

resource_lane_release_events
  release_id TEXT PRIMARY KEY
  claim_id TEXT NOT NULL REFERENCES resource_lane_claim_events(claim_id) ON DELETE RESTRICT
  lane_id BYTEA NOT NULL
  run_id TEXT NOT NULL
  node_id TEXT NOT NULL
  attempt_id TEXT NOT NULL
  ledger_key TEXT NULL
  invocation_epoch INTEGER NULL
  claim_fencing_token BIGINT NOT NULL
  release_reason TEXT NOT NULL
  commit_id TEXT NOT NULL REFERENCES commits(commit_id) ON DELETE RESTRICT
  source_seq BIGINT NOT NULL
  source_ordinal INTEGER NOT NULL
  source_event_id TEXT NOT NULL
  source_event_type TEXT NOT NULL CHECK (source_event_type = 'ResourceLaneReleased')
  source_event_payload_hash TEXT NOT NULL
  created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp()

resource_lane_transitions
  lane_id BYTEA NOT NULL
  lane_transition_seq BIGINT NOT NULL
  transition_kind TEXT NOT NULL CHECK (transition_kind IN ('claim', 'release'))
  claim_id TEXT NOT NULL
  release_id TEXT NULL
  claim_fencing_token BIGINT NOT NULL
  previous_transition_hash TEXT NULL
  transition_hash TEXT NOT NULL
  run_id TEXT NOT NULL
  commit_id TEXT NOT NULL REFERENCES commits(commit_id) ON DELETE RESTRICT
  source_seq BIGINT NOT NULL
  source_ordinal INTEGER NOT NULL
  source_event_id TEXT NOT NULL
  source_event_type TEXT NOT NULL
  source_event_payload_hash TEXT NOT NULL
  created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp()
```

Each lane claim/release row is a relational mirror of a committed lane event.
`resource_lane_transitions` is the durable lane-local ordering authority over those mirrored events.
Schema constraints or strict-load validation must bind
`(run_id, source_seq, source_ordinal, source_event_id, commit_id)` to the corresponding `run_events`
row and must verify that the event payload carries the same lane claim/release material.

Required lane constraints include:

```text
resource_lane_claim_events:
  UNIQUE (lane_id, fencing_token)

resource_lane_release_events:
  UNIQUE (claim_id)

resource_lane_transitions:
  PRIMARY KEY (lane_id, lane_transition_seq)
  UNIQUE (transition_hash)
  UNIQUE (claim_id, transition_kind)
  UNIQUE (release_id)
  CHECK (lane_transition_seq >= 1)
  CHECK (
    (transition_kind = 'claim' AND release_id IS NULL AND source_event_type = 'ResourceLaneClaimed')
    OR
    (transition_kind = 'release' AND release_id IS NOT NULL AND source_event_type = 'ResourceLaneReleased')
  )
```

`UNIQUE (lane_id, fencing_token)` is the no-double-claim guarantee at the same lane-local token.
`UNIQUE (claim_id)` on releases enforces at most one release per claim. The earlier "everything"
composite uniques existed only to be foreign-key targets for cross-table holder/lane matching; with
`lane_id` and `claim_id` as keys, the holder fields (`run_id`, `node_id`, `attempt_id`, `ledger_key`,
`invocation_epoch`) and `claim_fencing_token` agreement between a release and its claim are verified
in Rust strict load and append staging rather than carried in a wide BYTEA-bearing index.

The transition hash is canonical MFM bytes over the lane identity, transition sequence, transition
kind, holder identity, claim/release id, claim fencing token, source event binding, and
`previous_transition_hash`. For `lane_transition_seq = 1`, `previous_transition_hash` is null.
For later transitions, it must equal the previous transition's hash for the same lane. Strict load
must verify the transition hash chain, sequence contiguity, claim-token monotonicity, and active
holder fold before any lane authority is trusted.

Foreign keys bind each mirror/transition row to its source event by the `run_events` primary key, and
releases to their claim by `claim_id`:

```text
resource_lane_claim_events:
  FOREIGN KEY (run_id, source_seq, source_ordinal)
    REFERENCES run_events(run_id, seq, ordinal)

resource_lane_release_events:
  FOREIGN KEY (run_id, source_seq, source_ordinal)
    REFERENCES run_events(run_id, seq, ordinal)
  FOREIGN KEY (claim_id)
    REFERENCES resource_lane_claim_events(claim_id)

resource_lane_transitions:
  FOREIGN KEY (run_id, source_seq, source_ordinal)
    REFERENCES run_events(run_id, seq, ordinal)
```

The FK proves the source event exists; Rust strict load and append staging prove the rest of the
binding — that the referenced `run_events` row has the expected `event_id`, `event_type`,
`payload_hash`, and `commit_id`, that a claim transition references the exact claim by `lane_id` +
`fencing_token`, that a release references the exact active holder it releases (`claim_id`, holder
fields, `claim_fencing_token`), and that the lane descriptor hashes to the stored `lane_id`. This
removes the seven- and eleven-column composite foreign keys (and the nullable-holder sentinel/trigger
workarounds they forced) without weakening the guarantee: the mirror tables are written only by the
same append path that wrote the source events, in the same transaction, and strict load fails closed
on any mismatch.

`current_resource_lanes` is a view or observation table derived from lane transitions. It is useful
for operational dashboards, wakeups, and scheduling hints, but it is not scheduling authority.
Transition selection, independence witnesses, append admission, and recovery must use strict
active-lane reads verified from `resource_lane_transitions`, claim/release event mirrors, and
store-enforced holder checks.

At append time, the store must reject:

- `SideEffectInvocationPrepared` with an exclusive resource key but no active committed held lane
- a prepared resource key that differs from the preflight claim
- terminal/recovery commits from a holder that does not own the lane
- an attempt or side-effect terminal commit that would leave active release-required lane claims for
  the exact holder
- a saga terminal commit that would leave active release-required lane claims in that saga scope
- a run terminal commit that would leave any active release-required lane claim for that run
- standalone resource-lane acquire/release APIs that mint authority outside prepared commits
- any storage-derived global lane, single `global` lock row, or payload-inferred lane claim

For terminal scopes, the required release may already exist earlier in the verified prefix or be
included in the same terminal commit under a documented terminal-release rule. Otherwise append
admission rejects the terminal commit.

The same-commit terminal-release rule has a closed batch shape. It may include only releases for
active release-required claims in the terminal scope, every release must bind the exact holder and
lane transition authority for that claim, and unrelated releases are rejected. Batch validation must
fold the existing lane-transition prefix plus same-commit release transitions in `(seq, ordinal)`
order before evaluating terminal closure. Every `ResourceLaneReleased` event that satisfies a
terminal disposition must appear at an earlier ordinal than the terminal disposition event it
satisfies. Strict load must enforce the same ordering rule; the terminal event observes only prior
same-commit releases plus the verified prefix. The rule does not allow acquiring a new lane in the
same terminal commit.

The store must distinguish invalid lane claims from normal contention. A malformed claim, stale
attempt, unknown requirement, mismatched holder, or illegal phase is an admission error. A valid
claim intent blocked by an active conflicting lane is `ResourceLaneClaimBlocked` and leaves the
stream unchanged.

Strict load must reject histories where an attempt/side-effect holder, saga scope, or run reaches a
terminal state while release-required claims remain active for that scope, unless the verified prefix
or same terminal commit proves the corresponding release rule. This is a strict-load invariant, not
just a scheduler cleanup preference.

The old global resource-lane mutex/table shape must be deleted, not hidden behind a compatibility
path.

### 4. Postgres-Owned Read-Model Layer

Read models live in Postgres and are owned by the Postgres storage layer only as observation
materialization surfaces. Storage owns tables, views, indexes, rebuild procedures, drift validation,
and durable observation cursors. It does not own runtime projection semantics.

Strict semantics stay below the app assembly boundary. `mfm-store` owns spec-independent stream
contracts, projection fold contracts, and strict stream fold rules. `mfm-runtime` owns
spec-aware `ProjectionSnapshot` construction, validation, recovery, side-effect lifecycle,
manual-resolution use, retention use, resume, replay, and strict status behavior. `mfm-app`
assembles stores, registries, artifacts, and capabilities, orchestrates verified reads, and mints
public read authorities only after store/runtime verification. App and framework code do not write
observation rows directly and must not own workflow or projection semantics.

Authority-produced observation rows are written only by the append/rebuild pipeline that already
owns the verified input and provenance for the row. This RFC does not add a production-public
observation sink trait. Initial allowed producers are `mfm-store` for spec-independent stream
projection contracts and `mfm-runtime` for spec-aware lifecycle/recovery observations. `mfm-app` may
orchestrate these writes as part of verified services but must not construct semantic observation
contents itself. Every semantic-looking observation row must record `producer_authority`,
`projection_version`, source provenance, and row hash.

SQL must not become a second semantic projection engine. SQL derivation is limited to mechanical
facts that can be copied or deterministically reshaped from committed scalar/canonical-byte columns
without interpreting certified runtime policy.

Read facts are split from semantic authority:

- Observation facts may be stored in Postgres for list/watch/dashboard/search. They are
  projection-versioned, provenance-bearing, rebuildable, and never authority for execution.
- Semantic authority is still minted by Rust app/runtime/store strict paths from committed stream
  rows plus verified artifacts. This includes `ProjectionSnapshot`, certified saga policy,
  side-effect legality, manual-resolution proof state, retention authority, public-output
  authority, resume, replay, and strict status.

If an observation row displays semantic-looking data such as saga/run mode, public-output status,
manual-resolution state, retention status, or side-effect state, it must be labeled and documented
as projection-versioned observation data. No storage read-model trait may return `ProjectionSnapshot`
or hydrate observation rows into `ProjectionSnapshot`, `VerifiedRunHistoryView`,
`PublicOutputReadAuthority`, manual-resolution authority, retention authority, side-effect recovery
authority, or strict resume/replay/status authority. Those remain app/runtime/store strict-read
responsibilities outside the read-model layer.

Read models are for:

- run list
- run watch
- operational dashboards
- latest run summary
- latest attempt summary
- latest public output summary
- resource-lane observation
- change feeds and cursors

Recommended observation tables:

```text
run_observation_facts
attempt_observation_facts
cell_observation_facts
side_effect_observation_facts
public_output_observation_facts
retention_observation_facts
run_commit_log
run_observation_change_summaries
observation_derivations
```

Read facts have two provenance classes.

Delta facts derived mechanically from one committed event carry event provenance:

```text
projection_version TEXT NOT NULL
source_run_id TEXT NOT NULL
source_seq BIGINT NOT NULL
source_ordinal INTEGER NOT NULL
source_commit_id TEXT NOT NULL
source_event_id TEXT NOT NULL
source_logical_key TEXT NOT NULL
source_payload_hash TEXT NOT NULL
source_event_schema_id TEXT NOT NULL
derived_key TEXT NOT NULL
derived_row_canonical_json BYTEA NOT NULL
derived_row_hash TEXT NOT NULL
```

The provenance tuple must reference the source event by the `run_events` primary key:

```text
FOREIGN KEY (source_run_id, source_seq, source_ordinal)
  REFERENCES run_events(run_id, seq, ordinal)
  ON DELETE RESTRICT
```

The stored `source_event_id`, `source_payload_hash`, `source_logical_key`, and `source_commit_id` must
match the referenced authority event row; that agreement is verified by the read-model rebuild/drift
tooling (which already reads authority rows), not by a wide composite foreign key. A fact row carries
the redundant source metadata for rebuild verification, and rebuild fails closed on any mismatch.

Aggregate summaries are not one-event facts. Mechanical aggregates such as run head, commit count,
or change-feed frontiers may remain SQL views over event-sourced delta facts when they do not
interpret certified runtime policy. Semantic-looking summaries such as saga mode, latest state,
attempt state, side-effect state, manual-resolution state, retention state, or public-output status
must be produced by the authority layer that owns the corresponding semantics and then persisted as
projection-versioned observation rows with aggregate provenance, or omitted from the observation
API. SQL views may present those persisted observations, but SQL must not derive the semantic
meaning independently.

Aggregate rows that are not pure mechanical views depend on a stream prefix, a commit prefix, or
multiple source events. Those rows must carry aggregate provenance:

```text
projection_version TEXT NOT NULL
source_kind TEXT NOT NULL CHECK (source_kind IN ('run_prefix', 'platform_prefix', 'multi_event'))
source_run_id TEXT NULL
source_from_seq BIGINT NULL
source_to_seq BIGINT NULL
source_last_ordinal INTEGER NULL
source_high_append_xid XID8 NULL
source_high_commit_id TEXT NULL
source_high_commit_sort_key BYTEA NULL
source_event_count BIGINT NOT NULL
source_input_hash TEXT NOT NULL
derived_key TEXT NOT NULL
derived_row_canonical_json BYTEA NOT NULL
derived_row_hash TEXT NOT NULL
```

Sparse multi-source or dashboard rows must additionally write `observation_derivation_sources` rows
that reference each source event, or use a documented prefix hash chain that proves the same source
set. A single `(source_run_id, source_seq, source_ordinal)` tuple is only sufficient for one-event
delta facts. For platform-prefix summaries that reference a watch frontier, provenance must either
store the full cursor tuple `(source_high_append_xid, source_high_commit_sort_key)` or reference a
`run_commit_log(commit_id)` row from which that tuple is recovered. `source_high_commit_id` alone is
not an ordering value.

Current-state views are views over inserted facts, for example:

```text
current_runs
current_attempts
current_cells
current_side_effect_ledgers
current_public_outputs
current_resource_lanes
```

These views use run sequence, source ordinal, source commit id, and aggregate high-watermarks to
select the latest fact. Global observation freshness is described by snapshot-sealed append-XID
frontiers, not by a semantic global commit position. Current-state views must not be written by app
code.

## Trigger And View Strategy

Start with the simplest derivations as SQL views:

- run admission summary
- run completion summary
- run head by `max(seq)`
- observed public output produced/render-failed event-label summary, mechanical only and not
  public-output authority
- change feed by `run_commit_log.append_xid`

Use trigger-derived insert-only read facts when:

- a query is too expensive as a pure view
- the derivation is mechanical from one committed event, or aggregate provenance is explicitly
  recorded
- the row can carry event or aggregate provenance
- rebuild functions can recreate the same facts from authority rows

Do not implement saga mode, side-effect ledger legality, manual-resolution state, public-output
authority, or retention proof as independent PL/pgSQL projections. Those are semantic projections
owned by the authority layers, not by the Postgres storage implementation. The initial observation
layer must be limited to mechanical facts derived from scalar/canonical-byte authority columns and
views over those facts. If list/watch needs semantic-looking summaries later, the projection code
must live in the crate that owns that contract: `mfm-store` for spec-independent stream projection
contracts, `mfm-runtime` for spec-aware lifecycle/recovery semantics, and `mfm-app` only for
orchestration and public authority construction. The Postgres storage crate may persist
projection-versioned rows handed to it by the append/rebuild pipeline, but it must not call
runtime/app projection code internally or become the owner of those semantics. Such rows must be
labeled as observations and must not mint or substitute for strict semantic authority.

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

`store_metadata` defines the durable store instance that public cursors belong to:

```text
singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton)
store_epoch TEXT NOT NULL
schema_contract_version TEXT NOT NULL
created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp()
```

`store_epoch` is a random opaque id generated once at fresh migration. Public watch cursors carry it
(see the cursor codec below); a cursor whose `store_epoch` does not match the live row is rejected
with the public `CursorExpired` error. Internal diagnostics may record the stale epoch reason, but
the public API does not expose store epochs or epoch-specific error names. That is the entire
cursor-domain contract.

Earlier drafts added a `cursor_domain_seals` table, an XID-domain fingerprint computed from the
PostgreSQL system identifier and timeline through privileged functions, a frontier hash over every
cursor row, and an offline-reseal path that rewrites `commits`/`run_commit_log` into a new XID domain.
That subsystem is removed. It coupled the app role to physical Postgres internals and a maintenance
function, and it made every clone/restore fail closed until a privileged reseal ran — a large amount
of machinery whose only deliverable is "watch cursors survive a physical restore," which v1 does not
need.

The cursor-domain rule is intentionally narrow:

- Fresh migration generates a new `store_epoch`. Cursors from any prior epoch are rejected with
  `CursorExpired`, and the client re-lists to obtain a fresh cursor.
- Observation rebuilds, cache refreshes, and new projection/observation versions do not change the
  epoch; the cursor domain is unchanged.
- Any operation that changes the PostgreSQL XID ordering domain behind existing cursor rows —
  physical restore, `pg_basebackup` clone, logical dump/restore, point-in-time rollback, row discard —
  invalidates outstanding cursors by operator/runbook contract. MFM v1 does not expose a
  `reseed_store_epoch` entry point until Postgres role ownership, credentials, and restore/clone
  runbook semantics are designed.

The store does not auto-detect physical-domain changes, because doing so reliably is exactly the
machinery being removed. The contract is therefore explicit and documented in the runbook: **a
physically restored or cloned database served with old cursor state can mis-order or skip watch
cursors.** Until a maintenance-role design lands, restore/clone procedures must be destructive with
respect to issued cursors. If a deployment ever genuinely needs cursor survival across physical
restore, a reviewed RFC can reintroduce either a fingerprint/seal or a properly authorized
epoch-rotation procedure, justified against this simpler default.

`run_commit_log` is the durable cursor source for run list/watch observation APIs. It is insert-only
and atomic with the authority rows for the same commit. It is not projection-versioned and does not
store presentation summaries. This separation lets new projection versions insert rebuilt summaries
without mutating cursor authority or conflicting with old cursor rows.

Required columns:

```text
commit_id TEXT PRIMARY KEY
commit_sort_key BYTEA NOT NULL
append_xid XID8 NOT NULL DEFAULT pg_current_xact_id()
run_id TEXT NOT NULL
seq BIGINT NOT NULL
first_ordinal INTEGER NOT NULL
last_ordinal INTEGER NOT NULL
event_count INTEGER NOT NULL
commit_batch_hash TEXT NOT NULL
created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp()
```

Constraints:

```text
CHECK (octet_length(commit_sort_key) = 32)
CHECK (get_byte(commit_sort_key, 0) = 1)
CHECK (first_ordinal = 0)
CHECK (last_ordinal = event_count - 1)
UNIQUE (append_xid, commit_sort_key)
FOREIGN KEY (commit_id)
  REFERENCES commits(commit_id)
  ON DELETE RESTRICT
```

The cursor log references `commits(commit_id)` (its unique identity). Agreement of `run_id`, `seq`,
`event_count`, `commit_batch_hash`, and `append_xid` with the parent `commits` row, plus the
one-row-per-commit reverse completeness, is enforced by the Rust append path and re-checked by strict
load — not by a wide composite foreign key. `run_commit_log.append_xid` is assigned by the same
database-owned mechanism as `commits.append_xid` in the same transaction, so the two are equal by
construction.

The initial API should use one commit-log row per committed run commit. There is intentionally no
global `commit_pos`, no global `change_pos`, and no global commit-order lock. Public list/watch
delivery is ordered by the observation tuple `(append_xid, commit_sort_key)`, where `append_xid` is
the PostgreSQL top-level transaction id that inserted the row and `commit_sort_key` is a fixed-width
bytewise tie-breaker derived from the commit identity. The cursor must not order by raw `TEXT`
without an explicit bytewise collation; a `BYTEA` sort key avoids database-locale-dependent text
ordering.

The initial `commit_sort_key` derivation is:

```text
commit_sort_key =
  0x01 || sha256(
    "mfm.run_commit_log.sort_key.v1" ||
    canonical(run_id, seq, commit_key, commit_id, commit_batch_hash)
  )[0..31]
```

The leading `0x01` reserves the all-zero 32-byte value for cursor sentinels and makes real rows
structurally unable to use the sentinel. A collision on `(append_xid, commit_sort_key)` is a fatal
`CommitSortKeyCollision` storage error. The store must not resolve a collision by changing the sort
key algorithm ad hoc or by falling back to text ordering.

This observation order is not semantic commit order. It exists only to provide no-skip list/watch
delivery for operational APIs. Per-run strict order remains `(run_id, seq, ordinal)`. Resource-lane
conflict order is defined by held lane claims and store-enforced lane-transition authority. Resume,
replay, strict status, manual-resolution authority, retention proof, and public-output authority
must ignore observation cursor order.

Plain PostgreSQL sequences or identity columns are not valid durable public watch cursors. They are
not transactional: sequence changes are visible immediately and are not rolled back when a
transaction aborts. A transaction can therefore allocate a later sequence value, commit first, and
cause a watcher to advance past an earlier sequence value whose transaction commits later. The
target design avoids that failure mode without serializing all writers.

Snapshot-sealed watch polling uses PostgreSQL transaction snapshots:

```text
safe_before_xid = pg_snapshot_xmin(pg_current_snapshot())

SELECT ...
FROM run_commit_log
WHERE append_xid < safe_before_xid
  AND (
    append_xid > cursor.append_xid
    OR (append_xid = cursor.append_xid AND commit_sort_key > cursor.commit_sort_key)
  )
ORDER BY append_xid ASC, commit_sort_key ASC
LIMIT $limit
```

PostgreSQL defines snapshot `xmin` as the lowest transaction id still active; transaction ids below
that frontier are closed, meaning committed-visible or rolled back/dead for the current snapshot.
Therefore a watcher may safely advance across absent rows below `safe_before_xid`: no still-active
older transaction can later commit a row with `append_xid < safe_before_xid`.

Public watch cursors are opaque exclusive lower-bound cursors over `(append_xid, commit_sort_key)`.
They must include at least:

```text
cursor_version
store_epoch
append_xid
commit_sort_key
```

Cursor bytes must be integrity-protected. The implementation should use either server-issued
opaque tokens or a versioned sealed/MACed encoding that covers `cursor_version`, `store_epoch`,
`append_xid`, `commit_sort_key`, and projection/query compatibility metadata. Forged, tampered, or
future lower-bound cursors must be rejected with stable cursor errors rather than allowing clients
to silently skip observation rows.

The initial implementation should prefer a sealed/MACed stateless cursor codec. The cursor contract
must define:

- `cursor_key_id`
- durable key source for app/CLI/REST restarts
- accepted old-key window during rotation
- stable errors for unknown, missing, retired, or mismatched cursor keys
- whether projection/query compatibility metadata is part of the MACed payload

If a future implementation chooses server-issued opaque tokens instead, it must define an
insert-only token table or equivalent durable token authority, token expiry/retention behavior, and
the same stable error cases before exposing the public API.

The empty/default `commit_sort_key` is the all-zero 32-byte value and sorts before real commit sort
keys. Real rows cannot use the sentinel because of the `commit_sort_key` prefix check. When a page
is empty, the service may advance the cursor to `(safe_before_xid, zero_sort_key)` and report that
value as the sealed high-watermark. That means every MFM change with `append_xid < safe_before_xid`
has either been delivered or proven absent/aborted. Rows with `append_xid = safe_before_xid` remain
eligible for later pages once a future snapshot seals them.

If rows are returned, `next_cursor` is the last returned `(append_xid, commit_sort_key)`. A page may
split rows with the same `append_xid`; the bytewise tie-breaker prevents skipped rows. Duplicate
delivery is allowed after client retries, so change records must be idempotent for consumers.

Long-running transactions with old assigned XIDs delay watch high-watermark advancement, but they
must not block independent run appends. Two consequences need explicit handling:

- **The frontier is coupled to cluster-wide `xmin`, not just MFM appends.** `pg_snapshot_xmin`
  reflects the oldest XID-assigned transaction anywhere in the database instance — a `pg_dump`, a
  vacuum, an analytics query, or a co-tenant application can hold it back and stall the watch frontier
  for every watcher. The MFM store must therefore run on a database instance dedicated to MFM (no
  co-tenant write workloads), and that requirement is part of the operational contract, not just a
  recommendation. Keep append transactions short, set bounded statement and
  idle-in-transaction timeouts, and expose frontier lag only as an internal/maintenance metric, not
  as a public list/watch field.

- **Large artifact bytes vs. short append transactions are in direct tension.** The `PreparedCommitBundle`
  inserts artifact `BYTEA` inside the append transaction, and a multi-megabyte insert is inherently a
  long transaction holding a young XID — which is exactly what stalls the frontier. Until a real
  Postgres maintenance-role design exists, MFM does not pre-commit large blobs in a separate
  transaction and does not expose an orphan sweep. The v1 contract is therefore conservative:
  artifact blobs enter Postgres only as part of the append transaction, under an MFM byte-size limit
  enforced before insert and by the database. If larger blobs need a pre-commit path later, that path
  needs a separate role/privilege/runbook RFC before it lands.

There is also a **read-your-writes caveat** for list/watch: because the frontier only serves rows with
`append_xid < safe_before_xid`, a run you just started does not appear in `read_run_observations`
until `xmin` advances past its commit. This is expected and must be documented in the public API: strict
per-run `run_status(run_id)` is immediate and authoritative, while list/watch are sealed-frontier
observation surfaces that lag by the current oldest in-flight transaction.

Projection-versioned summaries live in `run_observation_change_summaries`:

```text
projection_version TEXT NOT NULL
commit_id TEXT NOT NULL REFERENCES run_commit_log(commit_id) ON DELETE RESTRICT
summary_kind TEXT NOT NULL
summary_row_hash TEXT NOT NULL
summary_row_canonical_json BYTEA NOT NULL
PRIMARY KEY (projection_version, commit_id, summary_kind)
```

Rebuilding a new projection version inserts new summary rows for the same immutable
`run_commit_log.commit_id` values. It must not update old summaries or rewrite cursor authority. If
a requested projection version is unavailable for a sealed visible change, normal app/CLI/REST
handlers must omit optional summaries or return `ObservationUnavailable` if required observation
rows are unavailable. Projection builds are explicit out-of-band maintenance operations; public
list/watch handlers must not silently run rebuild procedures with runtime credentials before
returning data.

Projection version is response/query metadata, not cursor authority. The service must reject stale
incompatible cursor-format versions or explicitly migrate them; it must never silently continue a
cursor across incompatible cursor contracts.

`read_run_observations` with a wait time must poll durable rows before and after waiting.
`LISTEN/NOTIFY` can only wake the waiter; it must not be treated as the data source and may be
missed or coalesced.

## Write Flow

The append flow should be:

1. Validate the prepared commit bundle in Rust before opening the database transaction where
   possible, including artifact byte/evidence checks, canonical request material, and payload
   shape. Large byte hashing should happen before the transaction unless the check specifically
   needs transaction-visible authority rows.
2. Begin transaction.
3. Check `(run_id, commit_key)` for idempotency using `commit_idempotency_hash` before stale
   sequence checks.
4. Take a transaction-scoped advisory lock for `run_id`.
5. Re-check `(run_id, commit_key)` after acquiring the run lock.
6. For commits that acquire, use, or release resource lanes, require the prevalidated bundle to
   carry concrete lane claim intents, release intents, and holder proofs already resolved by runtime
   preflight. Take sorted per-lane transaction-scoped advisory locks for those concrete lane keys
   before checking active conflicting claims, and hold them through lane-transition sequence
   assignment plus claim/release/transition row insertion. These locks serialize admission only; they
   are not the execution-duration resource lock. A global resource-lane lock is not acceptable in the
   target design because it serializes independent runs and still arrives too late for nonce-like
   live IO.
7. Load committed event history, artifact authority, and non-authoritative index hints needed to
   build the Rust commit base. Logical-key absence and uniqueness must be proven by folding
   `run_events` unless an explicit completeness proof for the exact prefix is verified.
8. Stage and validate the prepared commit in Rust against the loaded authority base. Reverify
   artifact byte/evidence matches inside the transaction before inserting authority rows. For a
   valid lane claim intent that conflicts with an active held lane, return `ResourceLaneClaimBlocked`
   without inserting rows.
9. For an admitted lane claim or release, assign store-filled fields while holding the lane lock:
   lane-local transition sequence, lane-local fencing token for claims, final claim/release id when
   store-filled, and final `ResourceLaneClaimed`/`ResourceLaneReleased` event payload. Compute the
   final `commit_batch_hash` over the materialized event envelopes and evidence set.
10. Insert or reuse `artifact_blobs` and artifact evidence/admission rows as needed. Large blobs may
    already have been content-addressed in a short preceding transaction to keep this transaction
    short; verify they exist with the expected digest/length and insert only evidence/admission rows.
11. Insert `commits`. The database-owned trigger overwrites `append_xid` with
    `pg_current_xact_id()`; this value is observation cursor metadata only.
12. Insert `run_events`. Logical-key admission was already proven by folding `run_events` in staging;
    no observation index is written (none exists in v1).
13. Insert `commit_artifact_evidence` and `run_artifact_admissions`.
14. Insert resource-lane claim/release and lane-transition authority rows when the bundle carries
    admitted lane operations.
15. Insert `run_commit_log` with the same `append_xid` and `commit_id` bound to the commit row.
16. Insert bounded mechanical observation facts and any explicitly authority-provided observation
    rows that are required to be atomic with the append. The initial implementation should keep this
    mechanical only.
17. Emit `NOTIFY` after authority rows, commit-log rows, and synchronous observation rows are
    inserted. `NOTIFY` remains only a wakeup and is delivered at commit.
18. Commit.

There is no global commit-order lock, global counter row, table-level append serialization, or
global `commit_pos`/`change_pos` allocation step in this flow. Any design or implementation that
needs such a point to make watch cursors correct is outside this RFC.

If synchronous trigger/observation derivation fails, the whole transaction must roll back. That
preserves atomicity for the minimal observation rows the public API depends on. Expensive dashboard
models may be rebuilt asynchronously from authority rows, but then list/watch must surface
`ObservationUnavailable` or omit those optional summaries until the requested projection version is
available. Public app/CLI/REST request handlers must not invoke privileged rebuild procedures as an
implicit fallback.

## Read Flow

There are two classes of reads.

### Strict Authority Reads

These must load committed event rows, verified artifact evidence, and lane-transition authority for
lane-bearing histories, then rebuild/verify through Rust authority:

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

Add one new shared app-level capability used by CLI and REST: cross-run operational observation.
Everything else should reuse existing strict authority APIs or be deleted during the breaking
refactor. If a doubtful surface is later needed, recover it from git history or introduce it in a
separate RFC with proof that the existing surfaces cannot carry it.

App-facing API:

```text
read_run_observations(query) -> RunObservationPage
```

`read_run_observations` is the only new public app method in this RFC. The query carries filters,
limit, an optional opaque cursor, and optional long-poll wait time. List, poll, and watch are modes
of this one observation read. There must not be separate `list_runs`, `poll_run_changes`, or
`watch_run_changes` app contracts.

The presence of the cursor selects the mode, and both modes seal exactly one frontier per request:

- **List mode (no cursor):** the response is the current set of runs matching the query, read from
  the `current_*` observation views bounded by one snapshot-sealed frontier, up to `limit` rows.
  `next_cursor` is that sealed frontier's exclusive lower bound.
- **Poll/watch mode (with cursor):** the response is the runs whose `run_commit_log` changes fall
  after the cursor and at or before a freshly sealed frontier, up to `limit` rows, with `next_cursor`
  advanced to the new frontier. An empty page (including a long-poll timeout) returns the advanced
  `next_cursor`, not an error.

Because every request seals one frontier and `next_cursor` is always that frontier's exclusive lower
bound, a client can list and then watch from the returned cursor without skipping changes at the
boundary XID.

Known limitation: the list-mode snapshot is capped at `limit` and is not separately paginated.
Listing an active or historical set larger than `limit`, or filtering by a historical time window
beyond what the forward change feed carries, is deferred to a separate list-pagination design. That
design must use its own page cursor and must not reuse the watch cursor, so "next page of this list"
and "next change after this sealed point" never alias. v1 deliberately ships the smaller capability:
the most recent matching runs plus a coherent forward watch.

Existing strict APIs remain the way callers obtain authority-bearing per-run state:

```text
run_status(run_id)      // strict authority read, existing surface
run_stream(run_id, ...) // strict authority stream, existing surface
typed_public_output(...) or its renamed replacement // public-output authority path
```

Those methods may be renamed as part of removing `typed` prefixes, but they must not be duplicated
as observation APIs.

CLI:

```text
mfm run list [--cursor <opaque>] [--limit <n>] [--wait-ms <n>] [--watch]
mfm run stream <RUN_ID> --from-seq <N> --watch
```

`mfm run list --watch` is only CLI sugar over repeated/long-poll observation reads. Do not add a
separate first-class `mfm run watch` command in this RFC.

REST:

```text
GET /v1/runs?cursor=<opaque>&limit=<n>&wait_ms=<n>
GET /v1/runs/:run_id/status
GET /v1/runs/:run_id/stream?from_seq=<n>&to_seq=<n>
```

Do not add `GET /v1/runs/watch` in this RFC. Future SSE/WebSocket streaming can be proposed later
if long-poll pages over `GET /v1/runs` are not enough.

These are public contracts. The implementation must define stable response structs, JSON schemas,
text output, and error codes before the routes/commands are considered complete.

Minimum run-observation page fields:

```text
next_cursor
runs[]
```

`next_cursor` is an opaque exclusive lower-bound cursor. A long-poll timeout is a successful empty
page with a fresh cursor, not an error.

Minimum run observation fields:

```text
run_id
head_seq
observed_status
started_at
updated_at
completed_at
```

If clients need idempotent change de-duplication, expose an opaque `change_id`. Do not expose
`projection_version`, `sealed_high_watermark`, `watch_frontier_lag`, `commit_id`,
`last_observation_id`, `projection_summary`, `append_xid`, `commit_sort_key`, artifact ids,
evidence hashes, lane keys, fencing tokens, or holder proofs in the public list/watch contract.

Fields that look semantic must either be omitted from the observation API or nested under a clearly
observation-only envelope. List/watch must not expose bare `run_mode`, `public_output_summary`,
`attempt_summary`, manual-resolution authority, retention proof, or public-output authority.
Callers that need semantic saga status, public-output authority, retention/replay proof, or
manual-resolution authority must call strict per-run status/stream/public-output APIs.

`read_run_observations` must use one snapshot-sealed frontier for the whole request. Rows in the
response are bounded by the sealed frontier, and `next_cursor` resumes from the sealed lower bound
without skipping rows at the frontier XID. If list pagination is added later, it must be a separate
explicit design from watch cursors so clients cannot confuse "next page of this list" with "next
change after this sealed observation point."

Stable public error codes must include at least:

```text
InvalidCursor
CursorExpired
LimitOutOfRange
RunNotFound
ObservationUnavailable
```

CLI text output should be compact and line-oriented for list/watch, while JSON output must use the
same stable schema as REST where practical. The implementation must update `bin/cli/README.md`,
`bin/rest-api/README.md`, and contract tests in the same change that introduces these commands and
routes.

The watch cursor is opaque publicly. Internally it encodes `(run_commit_log.append_xid,
run_commit_log.commit_sort_key)` as an exclusive lower bound plus a cursor format version and
`store_epoch`.
Projection version is response/query metadata, not cursor authority. The cursor must not encode
plain identity sequence values, global counters, wall-clock timestamps, or values that require a
global commit-order lock. Cursor decode must reject mismatched `store_epoch` with
`CursorExpired`, distinct from malformed cursor bytes.

Public list/watch responses must not expose `append_xid`, `commit_sort_key`, PostgreSQL transaction
ids, text-collation-dependent ordering keys, cursor versions, store epochs, projection versions, or
read-model watermarks. If clients need a visible progress token, expose only `next_cursor` or an
opaque `change_id`.

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

Proposed public or semi-public crate traits:

```text
RunEventStore
  append_prepared_commit_bundle(...)
  load_run_stream(...)
  expected_next_seq(...)

RunObservationStore
  read_run_observations(query) -> RunObservationPage

RetainedArtifactReadProvider
  read_retained_artifact(EventArtifactRequirement)

ArtifactReadProvider
  read_artifact(ArtifactReadAuthority)
```

`RunObservationStore` is an internal app/storage boundary, not a CLI/REST dependency and not an
external extension point. It must expose one page read only. It must not expose `get_run_observation`
as a status-lite API, separate poll/watch methods, sink writes, active-lane inspection, projection
maintenance, cursor internals, or Postgres implementation details.

Do not add a production-public observation sink trait in this RFC. Observation rows are
written only by the append/rebuild pipeline that already owns the authority and provenance for the
row. If a future projection system needs an explicit sink abstraction, it must be introduced as a
sealed internal trait or a separate RFC.

Do not add a standalone public commit artifact write-set trait. Verified bytes and evidence are
part of `PreparedCommitBundle`; splitting them back out risks recreating the plan-only append path.

There should not be a broad production `ArtifactStore` trait with arbitrary `get_artifact_by_id`,
`has_artifact`, or unscoped read methods. Runtime staging, retained-artifact reads for verified
history, adapter artifact-read capabilities, and public-output artifact reads are distinct authority
surfaces. They may be implemented by one concrete Postgres type, but artifact reads must remain
proof-bearing request surfaces.

`mfm-app` should be generic over the run store, observation store, and narrow artifact read
capabilities it actually needs. Runner registration should receive an artifact reader only when the
capability contract mints an exact `ArtifactReadAuthority` from verified run history, certified
config/seed authority, or side-effect/public-output evidence. Production artifact-read requests must
carry exact `(artifact_id, evidence_hash)` authority or exact event-derived requirements before they
can reach a reader. Partial artifact-read constructors must not exist on production read surfaces;
if tests or private authority-minting code need builders, those builders must be unable to issue a
readable request until verified history, config, and evidence fill every proof field. Strict
status/replay paths should use retained artifact authority derived from committed event
requirements. Public-output rendering must reuse the existing `PublicOutputReadAuthority` path or
its renamed replacement. Do not add a new public-output artifact reader trait in this RFC. It
must not accept arbitrary artifact ids or projection-versioned observation rows from list/watch.
Observation/read-model projection data cannot authorize artifact reads. CLI and REST should not
construct filesystem stores and should not query Postgres directly.

No observation store trait may return `ProjectionSnapshot`. Strict status must rebuild or verify the
projection from authority rows and verified artifacts through the strict store/runtime path.

Production wiring:

```text
PostgresRunStore
PostgresArtifactStore
PostgresObservationStore
```

These storage responsibilities may be implemented as separate crates or as one unified Postgres
storage crate if shared SQLx pool management, migrations, schema validation, advisory locking,
artifact evidence, and read-model derivation are materially simpler that way. The artifact store
must not be hidden inside the run-event implementation as an incidental private detail. It needs
explicit `PostgresArtifactStore` surfaces for commit bundle writes, retained reads, capability
reads, and public-output reads, even if those surfaces are implemented by one concrete Postgres
store type.

The default production app factory should compose the Postgres run, artifact, and observation
surfaces. CLI and REST should depend on that factory rather than constructing concrete storage
pieces directly.

The current production *filesystem* artifact stager disappears from production wiring. Artifact bytes
live only in Postgres `artifact_blobs`. Bytes are inserted inside the append transaction, and the
append transaction admits evidence/admission/event rows atomically. The bytes never touch a
filesystem store. Runtime does not own a separate filesystem stager, and v1 does not expose a
separate large-blob precommit or orphan-sweep path.

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

Required proof surface:

- append-owned read rows are derived from authority rows in the append path
- tests can materialize the same read rows from authority and compare hashes
- drift/corruption tests can prove mismatches are detected without exposing a production repair API
- schema validation verifies required read-model tables, views, triggers, columns, and constraints

Production `validate_read_models`, `build_projection_version`, `read_model_high_watermark`, and
`read_model_drift_report` entry points are deferred until a real maintenance-role model is designed.

Required properties:

- every delta read fact has source-event provenance
- every aggregate read fact has prefix or multi-source provenance
- every source-event provenance points to an authority event row
- every aggregate high-watermark points to committed authority rows
- rebuilding from authority rows produces the same derived row hashes
- append-synchronous observation facts and `run_commit_log` rows are immutable once served
- rebuilding the same projection version may insert missing rows only when hashes match existing rows
- rebuilding the same projection version with different hashes returns drift/corruption, not update
- materialized/cache rows can be dropped and recreated only in cache namespaces that are never cursor
  authority and never strict semantic authority
- strict runtime reads ignore read-model corruption

Suggested `observation_derivations` table:

```text
projection_version TEXT NOT NULL
model_name TEXT NOT NULL
derived_key TEXT NOT NULL
source_kind TEXT NOT NULL
source_run_id TEXT NULL
source_seq BIGINT NULL
source_ordinal INTEGER NULL
source_commit_id TEXT NULL
source_event_id TEXT NULL
source_payload_hash TEXT NULL
source_event_schema_id TEXT NULL
source_high_append_xid XID8 NULL
source_high_commit_id TEXT NULL
source_high_commit_sort_key BYTEA NULL
source_event_count BIGINT NOT NULL
source_input_hash TEXT NOT NULL
derived_row_canonical_json BYTEA NOT NULL
derived_row_hash TEXT NOT NULL
created_at TIMESTAMPTZ NOT NULL DEFAULT statement_timestamp()
```

This table is not authority. It is an audit ledger for read-model derivation.

For `source_kind = 'multi_event'`, `observation_derivation_sources` must enumerate source events or
reference a documented prefix hash chain:

```text
projection_version TEXT NOT NULL
model_name TEXT NOT NULL
derived_key TEXT NOT NULL
source_run_id TEXT NOT NULL
source_seq BIGINT NOT NULL
source_ordinal INTEGER NOT NULL
source_commit_id TEXT NOT NULL
```

## Guardrails

Database guardrails:

- deny `UPDATE`, `DELETE`, and `TRUNCATE` on MFM domain tables for the app role
- deny app/CLI/REST role mutation of `store_metadata` and cursor codec/key metadata
- add database triggers that reject update/delete on authority tables (this single-row guard is the
  one trigger class that earns its keep; multi-row structural invariants are owned by the Rust append
  path, not by PL/pgSQL re-implementations)
- add equivalent guards for read fact tables
- production rebuild/repair/epoch-reseed/orphan-sweep procedures are not part of v1; do not expose
  public maintenance entry points until Postgres roles, ownership, credentials, and runbooks are
  designed
- avoid `ON DELETE CASCADE` on authority tables
- validate required functions/views during schema validation; do not mandate deferrable constraint
  triggers that duplicate Rust multi-row folds, and require a parity test for any SQL check that does
  duplicate a Rust fold
- reject global commit-order locks, global counter rows, table-level append serialization, or any
  global `commit_pos`/`change_pos` allocation logic in production append paths
- require sorted per-lane admission locks (two-argument `pg_advisory_xact_lock(classid, objid)` with a
  dedicated lane class id distinct from the run-lock class id), serializable admission with mandatory
  retry, or a reviewed equivalent DB-enforced mechanism before active resource-lane claim checks
- enforce `UNIQUE (lane_id, fencing_token)` for no-double-claim, and reject any history where an open
  attempt holds an active lane and then commits a further `ResourceLaneClaimed` (the no-deadlock
  invariant)
- validate that opaque watch cursor internals use snapshot-sealed `run_commit_log.append_xid` plus
  `commit_sort_key`, not PostgreSQL identity/sequence values or text-collated ids, and that public
  responses do not expose those implementation fields
- validate that real `run_commit_log.commit_sort_key` rows cannot use the all-zero cursor sentinel
  and that sort-key collisions fail closed
- confirm v1 has no logical-key admission index: logical-key presence/absence/uniqueness is proven by
  folding `run_events`, and observation-row absence can never influence admission
- reseed `store_epoch` after restore, clone, import, rollback, destructive reset, or any operation
  that changes the `append_xid` ordering domain; reject cursors from a prior `store_epoch` with
  public `CursorExpired`
- validate that hash-bearing rows use canonical MFM bytes and never `jsonb::text`

Code guardrails:

- `sqlx` only in Postgres storage crates
- CLI/REST do not import concrete storage implementations except through production app factory
- CLI/REST production manifests do not depend directly on `sqlx`, Postgres storage crates, or
  filesystem artifact storage crates
- no production reference to filesystem artifact storage
- no production reference to in-memory stores
- no production partial artifact-read constructor can issue a read without exact
  `(artifact_id, evidence_hash)` authority or exact event-derived requirements
- scans reject `UPDATE`, `DELETE`, and `TRUNCATE` in MFM domain SQL except migration/admin repair
  code with explicit labels
- scans reject `typed_` target table names in new migrations
- scans reject new public target structs with `Typed` prefixes in storage/app read-model APIs
- scans reject broad production artifact reads by arbitrary artifact id outside proof-bearing
  request types
- scans reject production pre-append filesystem artifact staging
- compile-fail tests reject signer runtime config, secret wrappers, private-key wrappers,
  mnemonic/password wrappers, raw signed transaction types, and signed payload types from
  implementing `MfmValue`, `MfmConfig`, public output, state input, operation output, or artifact
  value surfaces
- review scans may flag names such as `password`, `mnemonic`, `private_key`, `raw_transaction`, and
  `signed_payload`, but scans are tripwires only; they are not architectural proof and must not
  become broad artifact `BYTEA` insertion gates
- cargo metadata checks keep signer providers below app/runtime wiring and out of states,
  operations, storage, CLI/REST semantic surfaces, and typed model crates
- storage APIs reject broad artifact-byte writes and broad arbitrary-id reads outside proof-bearing
  request types
- cargo metadata contract rejects new or remaining dependency allowlist exceptions that let
  `bin/cli`, `bin/rest-api`, or app-facing command glue depend on concrete storage crates directly

## Required Documentation Updates

This RFC changes the design contract, not only an implementation detail. The cutover must update the
authoritative project docs in the same change set as the storage/runtime/API refactor.
Those doc updates are a merge gate for the implementation cutover. The implementation must not land
with `docs/design.md`, `docs/architecture.md`, or `docs/saga.md` still describing filesystem artifact
production storage, plan-only durable appends, mutable helper authority, the old
invocation-prepared-derived lane model, or concrete storage dependencies that this RFC removes.
`docs/saga.md` is the authoritative companion for cross-run resource claims and changes materially
here; it is part of this merge gate, not optional follow-up.

`docs/design.md` must be updated to describe:

- Postgres as the only production persistence system
- append-only authority tables without production update/delete/truncate
- `PreparedCommitBundle` as the production append authority carrying artifact bytes plus evidence
- artifact bytes and evidence stored in Postgres, with filesystem artifact storage removed from the
  production design
- the artifact `BYTEA` secret boundary: storage verifies bytes/evidence/bindings, while typed,
  signer, keystore, and transient adapter boundaries prevent secret material from becoming artifacts
- Postgres-owned read models as rebuildable observation surfaces, not semantic authority
- strict status/replay/public-output reads rebuilding from committed stream and verified artifacts
- resource lanes as capability-declared, runtime-preflight-resolved authority acquired before live IO
  through a pre-invocation `ResourceLaneClaimed` event, with the no-deadlock single-claim invariant
- snapshot-sealed `run_commit_log.append_xid` observation cursors separate from
  projection-versioned summaries, with opaque epoch-tagged cursors and an operator reseed (no
  cursor-domain seal/fingerprint subsystem)
- the read-your-writes caveat: list/watch lag the sealed frontier while strict per-run status is
  immediate
- append-only authority with a dedicated-database requirement driven by cluster-wide `xmin` coupling
- no production maintenance entry points for read-model rebuild/validation or epoch reseed until
  Postgres roles, ownership, credentials, and runbooks are designed
- the breaking dev-branch cutover posture and lack of compatibility with old `typed_*` tables

`docs/architecture.md` must be updated to describe:

- storage crate boundaries for run authority, artifact authority, read models, and SQLx ownership
- the rule that CLI/REST depend on app services/factories, not concrete storage crates or `sqlx`
- the narrow artifact authority surfaces: commit bundle writes, retained reads, capability reads,
  and public-output reads
- resource-lane ownership across capability contracts, runtime preflight, store enforcement, and
  Postgres per-lane admission locking
- the rule that storage must not derive resource lanes from arbitrary payloads or protect them with
  global locks
- removal of filesystem artifact storage from production architecture
- test-only status of in-memory storage
- cargo/dependency guardrails that prevent production binaries from importing concrete storage
  implementations directly

`docs/saga.md` must be updated to describe:

- the pre-invocation `ResourceLaneClaimed` lifecycle replacing "exclusive lanes are derived from
  recorded invocation-prepared resource key evidence"
- how the three claim kinds map onto lanes: `Exclusive` takes a lane, `ExactTouchedSet` and
  `ManualOnly` take none
- store-assigned lane-local fencing tokens and `resource_lane_transitions` as lane order authority
- `ResourceLaneClaimBlocked` as a parked-attempt outcome that never authorizes terminal failure and
  may persist only non-authoritative operational waiter state
- the no-deadlock single-claim invariant and the v1 liveness story: bounded retry/backoff, optional
  wake hints, and Postgres FIFO only among live non-expired waiters for single-lane exclusive claims;
  multi-lane fairness remains deferred

CLI and REST docs must also be updated when the public list/watch routes and commands are added.

The persisted/public surface inventory must be created or updated as part of the same cutover. It
must list the new Postgres artifact bytes, artifact evidence, event payload canonical bytes, lane
authority rows, observation facts, cursor metadata, and CLI/REST output surfaces, with their
secret-boundary and authority classifications.

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
     resource-lane claim/release and lane-transition authority tables (keyed on `lane_id`),
     `run_commit_log`, projection-versioned observation facts, views, functions, and
     build/validation tools using target names. Do not add a `logical_key_observations` index
     (deferred) and do not add the
     cursor-domain seal/fingerprint subsystem (replaced by epoch + operator reseed).
   - Move same-run ordering from mutable run-head rows to advisory locking plus inserted commit
     authority.
   - Use runtime-preflight resolved resource-lane claims and sorted per-lane admission locks only for
     those concrete lanes. Do not derive lanes in storage and do not add a global lane lock.
   - Use snapshot-sealed `append_xid` observation cursors for list/watch. Do not add global
     commit-order serialization, global counter rows, or `commit_pos`/`change_pos` allocation.
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
- mechanical SQL observations match authority-derived rebuilds for representative runs
- semantic-looking observation rows are produced by the owning authority layer and match strict
  projection rebuilds
- SQL-only read derivations are limited to mechanical scalar/canonical-byte facts
- corrupt read facts do not affect resume/replay/public-output authority
- read facts are not used as append-admission authority
- hash-bearing rows are computed from canonical MFM bytes, not JSONB text
- commit rows reject or detect event_count, commit_key, ordinal, and batch-hash mismatches
- commit idempotency hashes and full prepared-authority hashes reverify from persisted commit
  purpose and canonical request material
- commit ids are deterministically derived before event batch hashing and cannot be assigned
  post-hash by storage
- `append_xid` is assigned by database-owned insert triggers/functions and cannot be overridden by
  production insert statements
- a commit without exactly one matching `run_commit_log` row rolls back before commit
- a mismatched `run_commit_log` binding or ordinal range rolls back before commit
- artifact ids are verified as derived from digests
- artifact evidence rows reject digest/length/id mismatches against blob rows
- snapshot-sealed XID watch cursors cannot skip rows when an older transaction commits after a
  newer transaction
- aborted older transactions do not block watch cursor advancement after the snapshot frontier
  passes them
- public watch tests prove no global commit-order advisory lock, global counter row, or table-level
  append serialization is needed
- watch cursor resumes after disconnect
- watch cursors carrying a prior `store_epoch` are rejected with `CursorExpired` after fresh
  migration or destructive reset
- restore/clone procedures are destructive with respect to issued cursors until an authorized
  epoch-reseed procedure is designed
- forged, tampered, or future public watch cursors are rejected by the sealed/MACed cursor
  contract instead of silently skipping rows
- cursor codec tests cover key id, durable key source across restarts, accepted old-key rotation
  window, unknown-key rejection, and retired-key rejection
- stale cursor-format versions are rejected or explicitly migrated
- unavailable required observation rows return `ObservationUnavailable`; optional summaries are
  omitted according to the public contract; normal app/CLI/REST handlers do not run privileged
  rebuild procedures as implicit fallbacks
- `read_run_observations` returns list rows and `next_cursor` from one consistent snapshot-sealed
  frontier, with any future page cursor kept separate from watch cursors
- `RunObservationStore` observation reads do not join against unbounded `current_*` views that can
  race ahead of the returned frontier
- no public/app observation sink can write rows outside the append/rebuild pipeline
- aggregate observation provenance stores or recovers the full watch frontier tuple when it claims a
  platform-prefix high watermark
- empty watch pages return a fresh `next_cursor`
- `LISTEN/NOTIFY` is not required for correctness
- strict `run stream --watch` tails authority rows and does not use read-model summaries
- `run_commit_log` cursor rows remain immutable across projection-version rebuilds and are ordered
  by `(append_xid, commit_sort_key)` only for observation delivery
- same-run concurrent appends serialize correctly
- idempotent commit-key retry still wins before stale sequence checks
- cross-run resource-lane contention admits only one conflicting claimant
- valid resource-lane contention returns `ResourceLaneClaimBlocked`, leaves the stream unchanged,
  and parks the already-started attempt before invocation; for single-lane exclusive claims it may
  insert or refresh one mutable operational waiter row that carries no semantic authority
- `ResourceLaneClaimBlocked` does not insert MFM domain rows, lane-transition rows, or persisted
  Postgres read-model facts that claim derivation from committed history
- single-lane exclusive claims are admitted by live-waiter FIFO under the lane advisory transaction
  lock: a later live waiter cannot materialize while an earlier live waiter remains waiting
- independent cross-run appends that do not share resource lanes can commit concurrently
- same chain/account EVM mutation runs cannot both reach nonce read, signing, or submission
- different chain/account EVM mutation runs can execute concurrently
- read-only EVM capabilities acquire no exclusive nonce lane
- store rejects prepared invocation resource keys without a committed held lane
- store rejects prepared invocation keys that differ from preflight claims
- recovery verifies the same committed held lane or records a certified recovery claim/release
  before live IO
- Postgres code has no global resource-lane table lock or single `global` lock row
- Postgres admission proves active resource-lane conflicts under concurrent transactions cannot
  double-admit the same exclusive lane
- the no-deadlock invariant holds: an attempt acquires all its lanes in a single `ResourceLaneClaimed`
  commit, and a history where an open attempt holds a lane and then commits a further
  `ResourceLaneClaimed` is rejected by append admission and strict load
- `lane_id` is derived/verified from the lane descriptor, and lane keys/indexes use `lane_id` not the
  repeated `key_canonical_json` tuple
- lane and run advisory locks use distinct `pg_advisory_xact_lock(classid, objid)` class ids
- `ExactTouchedSet` and `ManualOnly` claims acquire no lane
- resource-lane fencing tokens are store-assigned, lane-local, monotonic, and verified by strict
  load from append-only lane-transition history
- lane-transition rows prove durable claim/release order, transition hash chains, contiguous
  lane-local transition sequences, exact source events, and exact holder/lane ownership
- lane claim/release events cannot commit without exact mirror and lane-transition rows, and
  mirror/transition rows cannot commit without exact source events
- terminal side-effect/attempt/saga/run commits cannot leave release-required lane claims active
  in their respective scopes unless the verified authority prefix proves an explicit terminal
  release rule
- same-commit terminal release batches contain only exact in-scope active-claim releases, reject
  unrelated releases, and cannot acquire new lanes
- artifact evidence/admission/event rows are inserted atomically with the commit bundle
- artifact bytes are inserted inside the append transaction under the MFM byte-size limit
- `artifact_blobs` are append-only; there is no production orphan-sweep delete path in v1
- missing, extra, or mismatched artifact bytes are rejected before authority rows commit
- `run_events` stores only canonical payload bytes (no `payload_json` query copy)
- run-level artifact admissions prove the exact first `binding_kind = 'admitted'`
  `commit_artifact_evidence` row
- every runtime append path passes a `PreparedCommitBundle`, not a plan-only append
- logical-key presence, absence, and uniqueness are proven by folding `run_events` in staging; there
  is no admission index in v1
- recoverable submission-result rewrites are applied by the Rust staging rule against the folded
  `run_events` prefix (accepted and rejected cases)
- retained artifact reads require event-derived requirements
- adapter artifact reads require exact `ArtifactReadAuthority`
- public-output artifact reads require `PublicOutputReadAuthority` or strict
  `VerifiedPublicOutputArtifactRequirement`; observation/read-model projection rows cannot authorize
  artifact reads
- artifact read helpers cannot construct production-readable requests from partial artifact ids
- compile-fail tests prove secret-bearing signer/runtime/raw-transaction types cannot become typed
  persisted or public surfaces
- artifact BYTEA insertion tests verify content-addressing, evidence, role, producer, same-commit,
  and commit-bundle binding; they do not attempt broad secret-content scanning
- signer/provider/adapter tests prove secret material and raw signed transactions remain transient
  and redacted, and only non-secret evidence crosses into typed artifacts/events
- architecture scans prove resource-lane resolvers do not live in storage, app, CLI, REST, signer,
  keystore, or live transport/client modules
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
- SQL derivation is limited to mechanical scalar/canonical-byte facts in the initial
  implementation. Semantic-looking fields displayed by list/watch must be projection-versioned
  observation rows produced by the authority layer that owns the relevant semantics, then persisted
  by storage through an explicit API. Storage must not call runtime/app projection code internally,
  and observation rows must not mint strict semantic authority.
- v1 has no logical-key admission index. Append admission folds `run_events` directly. An append-only
  observation index plus its prefix-completeness proof is deferred to a future RFC and must land
  together, not index-first.
- Schema integrity is split by a single-enforcer rule: SQL carries what one key/FK/CHECK expresses
  directly, and the Rust append path is the sole enforcer of multi-row folds (contiguity, cardinality,
  reverse commit-log completeness, lane completeness/ordering), re-checked by strict load on read.
  Redundant composite UNIQUE supersets and deferrable triggers that duplicate Rust folds are removed.
- `run_events` stores only canonical payload bytes; the `payload_json JSONB` query copy is removed and
  queryable JSON lives in observation facts.
- Resource lanes use only the `Exclusive` claim kind; `ExactTouchedSet` and `ManualOnly` take no lane.
  Lane identity is a fixed-width derived `lane_id`, not a repeated descriptor tuple, keeping the large
  `key_canonical_json` out of every key/index/FK.
- Resource-lane contention is represented as `ResourceLaneClaimBlocked`, which leaves the stream
  unchanged and parks the already-started attempt before invocation. For single-lane exclusive claims,
  Postgres may persist/refresh a mutable operational waiter row for FIFO admission; the row is not
  stream or lane ownership authority. The no-deadlock invariant is structural: an attempt claims all
  its lanes in one all-or-nothing commit and never holds a lane while issuing a second blocking claim.
  v1 liveness uses capped backoff, optional release wakeups, and store-enforced FIFO only among live
  non-expired single-lane waiters; multi-lane fairness remains out of scope.
- Resource-lane fencing tokens are store-assigned, lane-local, monotonic values filled during claim
  admission under the concrete lane lock.
- Resource-lane claim/release order is durable lane-local authority in `resource_lane_transitions`,
  not advisory-lock timing, `append_xid`, wall-clock time, or run-local sequence from independent
  runs.
- Commit ids are deterministic pre-batch identities derived from prepared authority material, so
  event envelopes and `commit_batch_hash` have no post-hash storage-assignment cycle.
- Read-model rebuild/repair procedures are not production entry points in v1. Read-model correctness
  is proven by append-time derivation, strict authority loads, schema validation, and private
  read-only tests until a maintenance-role model is designed.
- Artifact bytes should be stored in Postgres `bytea` columns. PostgreSQL's current documented hard
  field-size limit is 1 GB, but MFM should enforce lower artifact-size limits before insert and with
  database checks. V1 inserts artifact blobs inside the append transaction and has no production
  orphan-sweep path.
- The MFM store runs on a database instance dedicated to MFM, because watch-frontier liveness is
  coupled to cluster-wide `xmin`. Data lifecycle (archival, partitioning, growth-bounding) is
  deferred to a future RFC; v1 is append-only with growth managed operationally.
- The first observation models should cover observed run summary, immutable commit cursor log,
  projection-versioned change summaries, observed public-output summary, and observed attempt
  summary. Those are enough for the first public list/watch API.
- Per-run strict status should continue to return full saga detail through verified stream and
  artifact authority. List/watch should return operational summaries unless the caller explicitly
  asks for strict per-run status.
- Public watch cursors should be based on snapshot-sealed `(run_commit_log.append_xid,
  commit_sort_key)`, not PostgreSQL identity allocation order, text collation, wall-clock
  timestamps, or globally serialized commit positions. Cursors are opaque, MACed, and epoch-tagged;
  the cursor-domain seal/fingerprint subsystem is removed. Physical restore/clone procedures are
  destructive with respect to issued cursors until an authorized epoch-reseed procedure is designed.
  Watch is a lagging observation surface (read-your-writes gap); strict per-run status is immediate.
- Cursor authority and projection summaries are split: `run_commit_log` is immutable observation
  cursor authority, while `run_observation_change_summaries` is projection-versioned and
  rebuildable.
- The target architecture intentionally has no global commit-order lock, global counter row, or
  global `commit_pos`/`change_pos` allocation. Cross-run semantic order does not exist unless
  created by explicit resource-lane authority.
- This RFC is a breaking dev-branch cutover. Old `typed_*` data, filesystem artifact roots, CLI
  flags, REST environment variables, and storage names do not need compatibility shims.
- The work lands as three separable, individually coherent cutovers (append/artifact, resource-lane,
  observation/naming) in dependency order, not one big-bang merge. No compatibility shims within any
  cutover.

## Appendix: Deferred Postgres Maintenance Roles

Earlier drafts required production maintenance entry points for read-model rebuild/repair, store
epoch reseed, and orphan artifact blob sweep. That requirement is removed from v1 because the RFC did
not specify the required Postgres role model: role names, object ownership, GRANT/REVOKE posture,
credential separation, SECURITY DEFINER policy, restore/clone runbook, or tests that prove app,
CLI, and REST credentials cannot use those surfaces.

V1 must not expose public Postgres maintenance entry points that merely document "use maintenance
credentials" while accepting an ordinary `PgPool`. The implemented guarantees are append-time
authority writes, append-owned read-model derivation, strict-load/read-only validation, schema
validation, checked SQL where practical, and architecture guardrails that reject fake maintenance
boundaries. Any future maintenance role work needs its own reviewed design before code lands.

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
