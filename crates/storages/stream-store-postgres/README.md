# mfm-stream-store-postgres

PostgreSQL typed run-event store implementation.

`PostgresTypedRunEventStore` is the certified typed submit/resume surface for
durable run streams. It persists typed event envelopes, commit keys, artifact
evidence, logical-key indexes, and replayable projections through `mfm-store`.

The legacy `PostgresStreamStore` remains a parity-lane implementation for the
old stream-store traits only. It uses separate `mfm_*` tables and is not a
certified typed run submission or resume surface.

The backend stores stream sequence and timestamp fields in PostgreSQL `BIGINT`
columns. Stream `u64` values above `i64::MAX` are unsupported by this backend and
are rejected before write; negative persisted sequence or timestamp values are
treated as storage corruption on read.
