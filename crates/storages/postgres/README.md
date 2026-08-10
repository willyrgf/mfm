# mfm-storage-postgres

Real PostgreSQL implementation of structured RunHistory and append-only configured-value history.

## Current schema

`migrations/0001_store.sql` is the one destructive pre-production baseline. It contains immutable
store identity/schema metadata, exact-target authority and fence generation, structured run
batches/records/objects, and configuration stream revisions. Dense tenant fact heads and append-only
transition routes live in `tenant_fact_heads` and `tenant_fact_publications`. Old journal,
projection, mutable-cursor, support, and single-row configuration tables have no compatibility
reader or upgrade path.

The deployment-owned migration job applies this baseline with its private owner connection. The
crate does not expose raw migration URLs or pools to application callers.

## Target-session TCB

Production openers consume only opaque deployment-issued session bundles
([`PostgresApplicationSessions`](crate::PostgresApplicationSessions),
[`PostgresCombinedSessions`](crate::PostgresCombinedSessions),
[`PostgresConfigurationSessions`](crate::PostgresConfigurationSessions)). Ordinary application
code never receives a `PgPool`, URL, connection options, raw fence, writer session, or DML
transaction. Embedding deployment code consumes a one-shot
[`PostgresTargetAdmission`](crate::PostgresTargetAdmission) from its
[`DeploymentCredentialBroker`](crate::DeploymentCredentialBroker) after verifying database identity, schema identity, store scope, exact role
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
pool. Test issuance uses deployment-private login materials only behind the `test-support` feature.

Append acknowledgement is paired with an [`ExternalCheckpointAuthority`](crate::ExternalCheckpointAuthority)
owned by deployment infrastructure. Prepared and acknowledged successor state is not read from
the database itself, so a copied database cannot make an unacknowledged append current.

## Qualification openers

- `open_structured_authoritative` — run history from application sessions
- `open_structured_authoritative_application` — run history plus resolve-only configuration
- `open_structured_authoritative_with_configuration` — joint assembly with a split configuration
  store
- `open_configuration_maintenance` — deployment-only configuration append

## Transaction semantics

Each run append begins `READ COMMITTED`, acquires the canonical run advisory lock before any
decision-bearing head or append-identity read, then classifies under that lock:

- same append identity and same canonical bytes -> `ExistingSame`
- same append identity and different canonical bytes -> `AppendConflict`
- different append identities from the same predecessor -> one `NewlyCommitted` and one `StaleHead`

On unique/serialization ambiguity the store re-reads under the lock before classifying. Healthy
contention never maps to `BackendUnavailable`. Object counts, frame sizes, and object closure bounds
are owned by the shared store canonical-append module and enforced identically for memory and
PostgreSQL before DML.

Positive backend replies include normalized stored content; the store compares them with its
retained candidate. Connection loss while acknowledging commit reports acknowledgement unknown and
is resolved only through the original append identity.

Configuration append uses an exact stream lock and one-based predecessor-linked revision. Exact
request replay returns the original revision; a stale predecessor or changed value conflicts.

## Verification

```sh
nix run .#run -- --task structured-history-postgres-qualification
nix run .#run -- --task postgres-sqlx-check
nix run .#run -- --task postgres-sqlx-offline-check
nix run .#run -- --task postgres-sql-inventory-check
```

Fact frontiers and publication counts live on the locked `tenant_fact_heads` row and advance
atomically with each publication. Bounded fact scans use the primary-key cursor
`(store_scope_id, store_epoch, tenant_scope_id, fact_order)` with a fixed `LIMIT`; they do not
`COUNT`/`MIN`/`MAX` over lifetime history.

The SQLx offline/online checks cover migration metadata and the schema probe. Dynamic runtime
`sqlx::query` families are inventoried by `postgres-sql-inventory-check`; documentation claims no
broader compile-time SQL coverage than those executable owners.

The suite covers exact-target role/ACL matrices, sibling-target denial, retained-input hostility,
configuration durability, fresh-process continuation, dense concurrent fact publication, rollback,
malformed rows, and unavailable-database failure without fallback.
