# mfm-stream-store-postgres

PostgreSQL typed run-event store implementation.

`PostgresTypedRunEventStore` is the certified typed submit/resume surface for
durable run streams. It persists typed event envelopes, commit keys, artifact
evidence, logical-key indexes, and replayable projections through `mfm-store`.

The old dynamic stream-store surface has been removed from this crate. New run
submission, resume, and replay paths go through the typed `mfm-store` contract
only.

The backend stores event sequence and timestamp fields in PostgreSQL `BIGINT`
columns. Event `u64` values above `i64::MAX` are unsupported by this backend and
are rejected before write; negative persisted sequence or timestamp values are
treated as storage corruption on read.
