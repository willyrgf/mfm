# mfm-storage-postgres

Real PostgreSQL implementation of structured RunHistory and append-only configured-value history.

## Current schema

`migrations/0001_store.sql` is the one destructive pre-production baseline. It contains immutable
store identity/schema metadata, exact-target authority and fence generation, structured run
batches/records/objects, and configuration stream revisions. Dense tenant fact heads and append-only
transition routes live in `tenant_fact_heads` and `tenant_fact_publications`. Old journal,
projection, mutable-cursor, support, and single-row configuration tables have no compatibility
reader or upgrade path.

Apply the baseline with an owner connection:

```rust,no_run
# async fn migrate() -> Result<(), mfm_storage_postgres::PostgresStoreError> {
mfm_storage_postgres::PostgresSchema::migrate(
    "postgresql://migration-owner@localhost/mfm",
).await?;
# Ok(())
```

A holder of deployment root credentials is inside the TCB.

## Target-session TCB

Production openers consume only opaque deployment-issued session bundles
([`ApplicationTargetSessions`](crate::ApplicationTargetSessions),
[`CombinedTargetSessions`](crate::CombinedTargetSessions),
[`ConfigurationMaintenanceSessions`](crate::ConfigurationMaintenanceSessions)). Ordinary application
code never receives a `PgPool`, URL, connection options, raw fence, writer session, or DML
transaction. Embedding deployment code issues the bundle through
[`issue_application_sessions`](crate::issue_application_sessions) (or the combined/maintenance
variants) after verifying database identity, schema identity, store scope, exact role
OID/membership, release epoch, fence generation, non-superuser status, no `BYPASSRLS`, no unintended
inheritance, and exact grants.

Each target schema owns distinct non-login roles derived from a private 16-hex target key:

- owner
- qualification
- run-reader
- run-writer
- configuration-reader
- configuration-writer

Grants are limited to that schema. Sibling schemas remain ungranted. Physical run-read, run-write,
configuration-read, and configuration-write sessions use distinct logins; a reader is never backed
by a writer-capable pool.

Every transaction obtains a fresh current target permit, pins `search_path`, selects the exact
managed role, and compares fence generation against the locked target authority row before DML. The
private transaction module owns acquisition, role selection, locks, and commit classification. All
run and configuration DML requires the `LockedWriteTx` (or configuration) typestate.

There is no always-successful production fence and no production constructor that accepts a raw
pool. Test issuance uses deployment-private login materials only inside integration tests.

## Qualification openers

- `open_structured_authoritative` — run history from application sessions
- `open_structured_authoritative_application` — run history plus resolve-only configuration
- `open_structured_authoritative_with_configuration` — joint assembly with a split configuration
  store
- `open_configuration_maintenance` — deployment-only configuration append

## Transaction semantics

Each run append uses a real SQL transaction and a transaction-scoped advisory lock derived from
the exact run id, acquired before decision-bearing head reads. Inside the lock it loads the complete
current assigned prefix/object closure, checks the expected head and writer lineage, stores exactly
the store-validated batch, and commits all records/objects/head together. Fact publication and
scanner-authorization appends additionally serialize on the exact tenant frontier.

Positive backend replies include normalized stored content; the store compares them with its
retained candidate. Connection loss while acknowledging commit reports acknowledgement unknown and
is resolved only through the original append identity.

Configuration append uses an exact stream lock and one-based predecessor-linked revision. Exact
request replay returns the original revision; a stale predecessor or changed value conflicts.

## Verification

```sh
nix run .#run -- --task recoverability-postgres-v1
nix run .#run -- --task postgres-sqlx-check
```

The suite covers exact-target role/ACL matrices, sibling-target denial, retained-input hostility,
configuration durability, fresh-process continuation, dense concurrent fact publication, rollback,
malformed rows, and unavailable-database failure without fallback.
