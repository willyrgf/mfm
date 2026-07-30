# mfm-storage-postgres

PostgreSQL authority for the recoverability-v3 committed run journal.

The public runtime entry point is `open_authoritative`. It consumes a writer
pool and a deployment-owned `AuthoritativeWriterFence`, qualifies the exact
database/schema/store lineage, and returns:

```text
(QualifiedRunStore<PostgresRunJournalBackend>, RunAccessAuthorityIssuer)
```

The issuer is the sole non-cloneable issuer paired with that qualified store.
The assembly is not cloneable and exposes neither its backend nor its pool. Qualified support is
admitted before `QualifiedRunStore::split` consumes it into one non-cloneable
`RunHistoryWriter<PostgresRunJournalBackend>` and a cloneable
`RunHistoryReader<PostgresRunJournalBackend>`. Runtime consumes the writer; application and replay
composition retain readers. The internal `RunJournalBackend` accepts only store-created
append/load verifiers; PostgreSQL rows are not a generic reader or a way to mint run authority.

## Destructive baseline

`migrations/0001_store.sql` is the one current, destructive pre-production
baseline. It contains only:

- `store_identity`
- `store_schema_metadata`
- `tenant_fact_order_heads`
- `journal_commits`
- `journal_records`
- `artifact_blobs`
- `artifact_admissions`
- `commit_object_authorities`
- `commit_artifact_bindings`
- `qualified_support_members`
- `fact_scan_attestations`
- `configured_values`

The retired event, projection, global-order, admission-lane, lifecycle, fact
query, and generic reader tables have no compatibility path. A database whose
SQLx ledger has the old `0001` checksum, or whose catalog has missing or extra
authority objects, is rejected. Export or reset that database; do not add an
upgrade shim or a second migration to reinterpret the retired schema.

Apply the baseline with an owner connection before opening the runtime store:

```rust,no_run
# async fn migrate() -> Result<(), mfm_storage_postgres::PostgresStoreError> {
mfm_storage_postgres::PostgresSchema::migrate(
    "postgresql://migration-owner@localhost/mfm",
)
.await?;
# Ok(())
# }
```

## Writer qualification and fencing

Local qualification proves that the connection is a writable primary, can
assume the sealed application role, carries the exact migration checksum and
closed catalog, preserves the immutable store scope/epoch, and has a valid
dense tenant-fact history. Qualification probes those facts both before and
after the external fence. Each full catalog validation pins its
transaction-local search path to the schema named by the adjacent writer
probe, so ambient state on another pooled connection cannot redirect
validation to a different schema.

Those local checks cannot prove that a promoted writer is the sole surviving
lineage. The supplied `AuthoritativeWriterFence` must be implemented by the
deployment system that owns promotion, WAL/backup ancestry, and stale-primary
exclusion. Its success asserts, for the exact database OID, schema, store scope,
and epoch, that:

- stale and sibling writers are permanently fenced;
- every published run suffix and tenant fact head is retained;
- rollback to an older WAL/backup lineage cannot qualify.

There is deliberately no production bypass, caught-up-replica mode, or fence
implementation in this crate. `TestAuthoritativeWriterFence` exists only under
tests or the `parity-tests` feature.

`RunHistoryReader<PostgresRunJournalBackend>::check_ready` is the bounded local readiness proof
for an already qualified process capability. It opens one `READ COMMITTED`,
`READ WRITE` transaction, installs a transaction-local quoted search path and
statement/lock timeouts, and uses one row query to recheck the database name
and OID, schema, recovery/read-only state, permission to assume the application
role, `SELECT`/`INSERT` journal privileges, exact store scope/epoch, and the
singleton schema metadata contract. It always rolls the transaction back.
Connection and query failures remain bounded PostgreSQL error categories;
writer-condition, retained-lineage, and singleton/schema failures are classified as
`WriterRequired`, `WriterFenceRejected`, and `SchemaAuthorityMismatch`
respectively.

Readiness deliberately does not rerun the deployment fence or full catalog
qualification, contact providers, resolve DNS, execute semantic callbacks,
inspect run progress, or claim that any external dependency is healthy.

## Transaction contract

Append transactions run at `READ COMMITTED`. An admission first acquires its
transaction-scoped logical-start advisory lock, resolves an existing run or
selects the proposed run id, and then acquires that run's transaction-scoped
advisory lock before loading any journal or object rows. It holds both locks
through verification and transaction end. A successor acquires only the run
lock; no path acquires the logical-start lock after a run lock.

Each signed 64-bit lock key is computed in Rust from the first eight bytes of a
SHA-256 digest over a domain-separated canonical object:

- admission: tenant, entry-point operation, and invocation identity;
- successor: run id.

No PostgreSQL hash function, session lock, mutable run-head table, or application
`UPDATE` privilege participates. A 64-bit collision can add contention but
cannot merge authority because every logical key, idempotency key, predecessor,
candidate digest, record, and object is rechecked after the lock.

An idempotent retry reloads and verifies the complete authoritative prefix; a
matching request id alone is not treated as proof. A successor reloads the
complete current journal and reachable object closure inside the locked
transaction and invokes the store-owned physical and semantic fold before
assignment. Stale predecessors and closed runs are classified by that sole
fold.

Object authority is keyed by the full canonical `ValueRef`, including producer
binding and evidence-contract reference. A content-deduplicated payload may
therefore carry several distinct logical authorities. Each commit persists the
complete recursive admission-intent closure in `commit_object_authorities`;
record field roots remain in `commit_artifact_bindings`. Loads reconstruct and
verify both sets rather than inferring authority from a content digest or an
artifact/evidence tuple.

Raw content-addressed blobs may be staged before the locked append. Staged bytes
grant no authority and an abandoned staging row is unreachable. Exact object
admissions are verified inside the append transaction before the tenant
allocator is called. The protected tenant head is the final serialization
point; the commit, records, complete object authority, field bindings, fact
routing, optional fact-scan attestation, and tenant-head advance either all
commit or all roll back.

An I/O, protocol, or connection loss while acknowledging `COMMIT` returns
`AppendOutcome::OutcomeUnknown`. Retrying the same append request is the only
safe way to resolve visibility. An already committed authorization never
recreates live execution authority.

Authorized loads use one read-only `REPEATABLE READ` transaction for the
complete immutable commit/record graph and every reachable exact object. The
store-owned verifier rederives the physical journal before returning an opaque
`CommittedRunJournal`.

## Support, configured values, and facts

`QualifiedRunStore::admit_support_graph` binds a producer-free qualified graph to one sealed
deployment authority before the one-shot split. Admission is serialized by qualification scope
and atomically admits or verifies the complete exact field-path keyset, full
producer-bound references, and bytes. A retained graph cannot be extended,
trimmed, or repointed by a later retry.

Admission-source verification accepts only the store-created affine verifier.
For each pending same-store, same-tenant source coordinate, PostgreSQL uses the
same exact-run `REPEATABLE READ` load path described above, then returns the
physically verified journal to the store-owned recursive closure verifier.
Callers cannot supply an arbitrary source load, skip a recursively discovered
run, or turn a source run id into read authority.

Configured values are immutable deployment inputs keyed by:

```text
(store_scope_id, tenant_scope_id, entry_point_id, target)
```

The runtime has no publish, update, target-only lookup, list, or export API.
Deployment tooling provisions the full canonical `ConfiguredValueBinding`,
producer-bound `ValueRef`, and admitted bytes through a migration-owner path.
The application role has `SELECT` only. Runtime resolution is available solely
through `RunHistoryReader::resolve_configured_value` with an exact
`RunAccessAuthority<Admit>` and certified `RetainedValueContract`; the verifier rederives the key,
producer, contract, and byte identity before returning a sealed value.

Fact selection pages use one read-only `REPEATABLE READ` snapshot and scan the
dense tenant publication coordinate range fixed by the store verifier. Each
route reloads and verifies its complete producer run and object closure.
Publication and fact budgets include the consuming run even when policy makes
that run unselectable. A returned fact observation persists its authorization,
full attestation authority, observation, and containing journal head in the
same append transaction. Replay loads and revalidates those exact links.

## Roles and mutation

Migrations create non-login `mfm_store_owner` and `mfm_store_application`
roles. Runtime transactions assume the application role. Immutable identity,
schema metadata, journal, blob, admission, authority, binding, support, fact
attestation, and configured-value rows are protected by privileges and
mutation triggers. The application role has no direct `UPDATE`, `DELETE`, or
`TRUNCATE` path for authority rows, no `INSERT` path for configured values, and
no direct mutation path for `tenant_fact_order_heads`; only the sealed
security-definer allocator may advance or read that head for a coordinate.
The application role has `SELECT`-only access to `_sqlx_migrations` so every
authoritative open can validate the compiled migration checksum. That
operational ledger remains outside run/fact authority catalog semantics and
cannot be mutated by the application role.

## Verification

Use the repository-owned task, which provisions a disposable PostgreSQL schema:

```sh
nix run .#run -- --task recoverability-postgres-v3
```

The dedicated `recoverability-v3` target runs the shared corpus vectors through
a physical blob round trip. Package tests cover the closed catalog,
qualification, privileges, rollback/head integrity, advisory-lock behavior,
idempotency, independent qualified-writer reconciliation, and backend parity.

When SQL, the authoritative schema model, or SQLx query metadata changes, run
the repository-owned narrow task. It performs the online SQLx check and proves
that runtime validation rejects a hostile mutation of the migrated schema:

```sh
nix run .#run -- --task postgres-sqlx-check
```

Choose broader verification only when the affected boundary requires it, as
described in `docs/build-and-verification.md`.
