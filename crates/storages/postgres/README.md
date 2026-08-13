# mfm-storage-postgres

The PostgreSQL adapter supplies mechanical exact-head ordering under the admitted
`PrimaryCrashRestart` durability profile. Store remains the semantic owner.

The v8 inventory is deliberately append-only and partitioned by `(store_scope_id, store_epoch,
tenant_scope_id)`: `mfm_run_frames` and `mfm_run_heads` retain run prefixes and heads;
`mfm_fact_heads` and `mfm_fact_publications` retain the dense independent fact frontier and its
publication coordinates; and `mfm_configuration_revisions`/`mfm_configuration_heads` retain the
configuration stream. The writer materializes a zero `mfm_fact_heads` row with
`ON CONFLICT DO NOTHING` and locks it before checking a first publication. The shared backend
conformance race proves that concurrent first publishers linearize to one commit and one
`FactFrontierChanged`, with no partial run or fact row.

Configuration rows remain mechanical canonical bytes. Each row stores the exact typed content
reference and cumulative byte count at its global sequence; the head stores the same count for
bounded preflight before byte allocation. Store alone decodes and validates `MfmConfig` values.
The baseline intentionally rejects retired generic `mfm.configuration` rows and has no legacy
reader.

The database also records one persisted `(store_scope_id, store_epoch)` deployment identity.
Ordinary opens must match it, including simultaneous qualified opens. A trusted restore uses
`PostgresStore::rotate_identity` with a fresh scope or epoch, then opens the new pair; that
rotation makes already-open handles with the previous identity fail closed and leaves old
partitions replay-only.
