# mfm-stream-store-postgres

PostgreSQL run-store implementation. This crate is the only owner of the MFM
run-store PostgreSQL schema.

`PostgresRunStore` is the certified submit/resume surface for durable run
streams. It persists append-only commits, canonical event payload bytes,
artifact blobs/evidence, resource-lane transition authority, and observation
cursor rows through `mfm-store`.
It owns the PostgreSQL migrations, crate-local SQLx query metadata, and runtime
schema compatibility checks for that store.

## Runtime Contract

Runtime connections validate the schema and do not create or alter tables. Apply
the embedded migrations explicitly before starting CLI, REST, or library
callers:

```rust
# async fn example() -> Result<(), mfm_stream_store_postgres::PostgresStoreError> {
mfm_stream_store_postgres::PostgresSchema::migrate_env().await?;
let _store = mfm_stream_store_postgres::PostgresRunStore::connect_env().await?;
# Ok(())
# }
```

## Verification

Use the repository DB gate for this crate:

```sh
nix run .#test-db
```

That gate starts managed Postgres, applies this crate's migrations, verifies
checked SQLx metadata against the migrated schema, and then runs the
Postgres-backed parity tests. The plain `nix run .#test` gate intentionally
remains service-free.

## SQLx Metadata

With `DATABASE_URL` pointing at a disposable local Postgres database, apply this
crate's migrations and regenerate metadata from the crate directory:

```sh
export DATABASE_URL="postgresql://postgres:postgres@127.0.0.1:5432/mfm_test"
nix shell .#sqlx-cli --command \
  sqlx migrate run --source crates/storages/stream-store-postgres/migrations
nix shell .#sqlx-cli --command bash -lc \
  'cd crates/storages/stream-store-postgres && cargo sqlx prepare -- --all-targets --features parity-tests'
```

After regenerating metadata, run `nix run .#test-db`.

## Compatibility

The old dynamic stream-store surface and old `typed_*` schema are removed from
this crate. This is a destructive dev-branch baseline: use a fresh database or
drop/recreate the existing local schema before applying migrations.

## Data Limits

The backend stores event sequence and timestamp fields in PostgreSQL `BIGINT`
columns. Event `u64` values above `i64::MAX` are unsupported by this backend and
are rejected before write; negative persisted sequence or timestamp values are
treated as storage corruption on read.
