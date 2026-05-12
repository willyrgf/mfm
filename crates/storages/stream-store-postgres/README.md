# mfm-stream-store-postgres

PostgreSQL `StreamStore` implementation (parity lane).

The backend stores stream sequence and timestamp fields in PostgreSQL `BIGINT`
columns. Stream `u64` values above `i64::MAX` are unsupported by this backend and
are rejected before write; negative persisted sequence or timestamp values are
treated as storage corruption on read.
