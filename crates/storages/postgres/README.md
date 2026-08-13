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
