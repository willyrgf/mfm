# mfm-storage-postgres

Real PostgreSQL implementation of structured RunHistory and append-only configured-value history.

## Current schema

`migrations/0001_store.sql` is the one destructive pre-production baseline. It contains immutable
store identity/schema metadata, structured run batches/records/objects, and configuration stream
revisions. Dense tenant fact heads and append-only transition routes live in
`tenant_fact_heads` and `tenant_fact_publications`. Old journal, projection, mutable-cursor,
support, and single-row configuration tables have no compatibility reader or upgrade path.

Apply the baseline with an owner connection:

```rust,no_run
# async fn migrate() -> Result<(), mfm_storage_postgres::PostgresStoreError> {
mfm_storage_postgres::PostgresSchema::migrate(
    "postgresql://migration-owner@localhost/mfm",
).await?;
# Ok(())
```

## Qualification

`open_structured_authoritative` consumes a pool, deployment-owned `AuthoritativeWriterFence`, exact
program verifier, and purpose-limited public physical-binding verifier. It validates the migration
checksum and ledger shape, closed catalog manifests, writable primary, exact roles and ACLs,
database/schema identity, stable store scope, and writer epoch before returning a
`StructuredRunStore`. Catalog manifests bind relation owners, columns and defaults, complete
constraint definitions and validation state, indexes, functions, triggers, policies, rules, and
schema/table/column/default ACLs. A retained constraint name or effective privilege is not accepted
as a substitute for the exact physical contract.

Local SQL checks cannot prove non-rollback promotion or exclusion of stale/sibling writers. The
deployment fence must prove those properties for the exact database, schema, store scope, and
epoch. No production bypass or in-crate self-attestation exists. Test fence fixtures are available
only under test/parity features. A fence that reads retained store rows through the supplied pool
must use a transaction-scoped `SET LOCAL ROLE mfm_store_qualification` after pinning the qualified
schema.

`open_structured_authoritative_application` additionally returns resolve-only configuration
history. Deployment uses the separate qualification entry point that yields a configuration
writer. App assembly cannot append configured values.

## Transaction semantics

Each run append uses a real SQL transaction and a transaction-scoped advisory lock derived from
the exact run id. Inside the lock it loads the complete current assigned prefix/object closure,
checks the expected head and writer lineage, stores exactly the store-validated batch, and commits
all records/objects/head together. Object-row or record-row failure rolls back the whole batch.
Fact publication and scanner-authorization appends additionally serialize on the exact tenant
frontier. The same transaction checks dense route integrity, persists the run batch, inserts an
exact publication route when applicable, and advances the tenant head; barriers check and retain
the current head without advancing it.

Positive backend replies include normalized stored content; the store compares them with its
retained candidate. Connection loss while acknowledging commit reports acknowledgement unknown and
is resolved only through the original append identity.

Loads use a consistent read-only snapshot, reconstruct numeric sequence order, validate exact
canonical rows, and invoke the sole store fold. A fresh process/pool can refold and continue the
same run. No path delegates to memory.

Configuration append uses an exact stream lock and one-based predecessor-linked revision. Exact
request replay returns the original revision; a stale predecessor or changed value conflicts.
`configuration_heads` is an independent local exact-head CAS over the append-only revision stream;
qualification and append detect local head removal, rollback, or divergence. That local check cannot
by itself prove that the database and configuration authority were not rolled back together, so a
production deployment must combine it with the admitted external non-rollback writer-fence
authority.

## Roles

The migration creates fixed NOLOGIN owner, qualification, application, and configuration-maintenance
roles. Migration credentials are separate from ordinary credentials and are rejected by every
production opener. Provision each ordinary LOGIN as NOINHERIT, without superuser, role/database
creation, replication, row-security bypass, database ownership, role configuration, direct schema
ACLs, or direct table ACLs. Grant only the entry point's exact memberships with non-inheriting,
settable membership options:

```sql
-- open_structured_authoritative and open_structured_authoritative_application
GRANT mfm_store_qualification, mfm_store_application TO mfm_application_login
WITH INHERIT FALSE, SET TRUE;

-- open_configuration_maintenance
GRANT mfm_store_qualification, mfm_store_configuration_maintenance TO mfm_maintenance_login
WITH INHERIT FALSE, SET TRUE;

-- open_structured_authoritative_with_configuration
GRANT mfm_store_qualification, mfm_store_application,
      mfm_store_configuration_maintenance TO mfm_combined_assembly_login
WITH INHERIT FALSE, SET TRUE;
```

Qualification requires `session_user = current_user` and rejects extra memberships, membership
admin options, inherited membership authority, owner membership, and privileged `SET ROLE` session
substitution. Backends use transaction-scoped role selection only after qualification. The
application role cannot update/delete/truncate immutable authority rows and configuration access is
SELECT-only. `_sqlx_migrations` is SELECT-only through the qualification role and is never mutable
through an ordinary pool.

## Verification

```sh
nix run .#run -- --task recoverability-postgres-v1
nix run .#run -- --task postgres-sqlx-check
```

The qualification covers configuration durability, fresh-process continuation, dense concurrent
fact publication and hostile head removal, rollback, malformed rows, numeric ordering beyond
sequence nine, named `CHECK (TRUE)` replacement, public and hostile grants, membership/session
escalation, SQLx metadata mutation, and unavailable-database failure without fallback.
