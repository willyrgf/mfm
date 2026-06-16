# mfm-stream-store-postgres

PostgreSQL typed run-event store implementation.

`PostgresTypedRunEventStore` is the certified typed submit/resume surface for
durable run streams. It persists typed event envelopes, commit keys, artifact
evidence, logical-key indexes, and replayable projections through `mfm-store`.
This crate owns the PostgreSQL schema, crate-local SQLx query metadata, and all
PostgreSQL migrations for that store.

Runtime connections validate the schema and do not create or alter tables. Apply
the embedded migrations explicitly before starting CLI, REST, or library
callers:

```rust
# async fn example() -> Result<(), mfm_stream_store_postgres::PostgresTypedStoreError> {
mfm_stream_store_postgres::PostgresSchema::migrate_env().await?;
let _store = mfm_stream_store_postgres::PostgresTypedRunEventStore::connect_env().await?;
# Ok(())
# }
```

Local SQLx metadata lives in `.sqlx/` beside this README and must be regenerated
from a migrated Postgres database whenever checked queries or migrations change:

```sh
nix shell nixpkgs#sqlx-cli --command cargo sqlx --version

export DATABASE_URL="postgresql://postgres:postgres@127.0.0.1:5432/mfm_test"
nix shell nixpkgs#sqlx-cli --command \
  sqlx migrate run --source crates/storages/stream-store-postgres/migrations
nix shell nixpkgs#sqlx-cli --command bash -lc \
  'cd crates/storages/stream-store-postgres && cargo sqlx prepare -- --all-targets --features parity-tests'
cargo clean -p mfm-stream-store-postgres
nix shell nixpkgs#sqlx-cli --command bash -lc \
  'cd crates/storages/stream-store-postgres && cargo sqlx prepare --check -- --all-targets --features parity-tests'
SQLX_OFFLINE=true cargo check -p mfm-stream-store-postgres --all-targets --features parity-tests
```

Repository CI exposes the Postgres-backed subset as `nix run .#test-db`. The
plain `nix run .#test` gate intentionally remains service-free.

The old dynamic stream-store surface has been removed from this crate. New run
submission, resume, and replay paths go through the typed `mfm-store` contract
only.

The backend stores event sequence and timestamp fields in PostgreSQL `BIGINT`
columns. Event `u64` values above `i64::MAX` are unsupported by this backend and
are rejected before write; negative persisted sequence or timestamp values are
treated as storage corruption on read.
